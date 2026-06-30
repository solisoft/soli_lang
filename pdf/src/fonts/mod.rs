//! Font loading, coverage itemization, and advance-width metrics.
//!
//! **No fonts are embedded in the binary.** All faces are loaded at runtime
//! from the directories in `RenderOptions::font_dirs` (CLI `--font-dir`). The
//! template's `fonts: [...]` field names the family to use as the primary text
//! face; its Regular/Bold styles are resolved from the loaded files, and every
//! other loaded font (e.g. a CJK font) becomes a fallback for characters the
//! primary can't cover.
//!
//! Text is split into runs by which face covers each character (primary weight
//! → fallback faces, in load order). Advance widths come from `ttf-parser`;
//! printpdf does the actual glyph encoding + embedding.

mod subset;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ttf_parser::Face;

use crate::error::{PdfError, RenderWarning, Result};
use crate::template::FontWeight;

/// Which face a run of text should be drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontSlot {
    Regular,
    Bold,
    /// A fallback face, by load index.
    Fallback(usize),
}

/// One loaded face. The `ttf-parser` `Face` is re-parsed on demand (cheap; it
/// only reads the table directory) so the struct isn't self-referential.
struct LoadedFace {
    bytes: Vec<u8>,
    /// Lowercased file stem, used for family matching (e.g. "titilliumweb-bold").
    key: String,
    is_bold: bool,
    units_per_em: f32,
    ascent: f32,
    descent: f32,
    cap_height: f32,
}

impl LoadedFace {
    fn load(bytes: Vec<u8>, key: String) -> Result<LoadedFace> {
        let (upem, ascent, descent, cap_height, is_bold);
        {
            let face = Face::parse(&bytes, 0)
                .map_err(|e| PdfError::Font(format!("ttf-parser failed for {key}: {e}")))?;
            let u = face.units_per_em() as f32;
            upem = u;
            ascent = face.ascender() as f32 / u;
            descent = face.descender() as f32 / u;
            // Cap height is what makes uppercase/digits look optically centered.
            // Many fonts (e.g. Titillium) have an ascender far taller than the
            // caps, so falling back to the ascent would re-introduce the bias we
            // correct for; use a typical 0.7em instead when the font omits it.
            cap_height = face
                .capital_height()
                .filter(|c| *c > 0)
                .map(|c| c as f32 / u)
                .unwrap_or(0.7);
            is_bold = face.is_bold() || key.contains("bold");
        }
        Ok(LoadedFace {
            bytes,
            key,
            is_bold,
            units_per_em: upem,
            ascent,
            descent,
            cap_height,
        })
    }

    fn parse(&self) -> Face<'_> {
        Face::parse(&self.bytes, 0).expect("face was validated at load")
    }

    fn matches_family(&self, family: &str) -> bool {
        let fam = family.to_ascii_lowercase().replace([' ', '-', '_'], "");
        self.key.replace([' ', '-', '_'], "").contains(&fam)
    }
}

fn covers(face: &Face, ch: char) -> bool {
    ch.is_whitespace() || face.glyph_index(ch).is_some()
}

fn advance(face: &Face, units_per_em: f32, ch: char, size: f32) -> f32 {
    if let Some(gid) = face.glyph_index(ch) {
        if let Some(adv) = face.glyph_hor_advance(gid) {
            return adv as f32 / units_per_em * size;
        }
    }
    if ch == ' ' {
        return 0.25 * size;
    }
    0.5 * size
}

/// A maximal run of characters drawn with one face.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub slot: FontSlot,
    pub text: String,
}

/// The loaded font set: a primary family (Regular/Bold) plus fallback faces.
pub struct FontRegistry {
    regular: LoadedFace,
    bold: LoadedFace,
    fallbacks: Vec<LoadedFace>,
}

impl FontRegistry {
    /// Build a registry from the fonts found in `dirs`. `families` (from the
    /// template's `fonts` field) selects the primary face; any other loaded
    /// font becomes a fallback. Returns an error if no usable font is found.
    pub fn from_font_dirs(dirs: &[PathBuf], families: &[String]) -> Result<FontRegistry> {
        let mut faces: Vec<LoadedFace> = Vec::new();
        for dir in dirs {
            for path in font_files(dir) {
                if let Ok(bytes) = fs::read(&path) {
                    let key = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("font")
                        .to_ascii_lowercase();
                    if let Ok(face) = LoadedFace::load(bytes, key) {
                        faces.push(face);
                    }
                }
            }
        }
        if faces.is_empty() {
            return Err(PdfError::Font(format!(
                "no usable fonts found in {dirs:?}; provide a font directory (RenderOptions::font_dirs / --font-dir)"
            )));
        }

        // Choose Regular and Bold, preferring a requested family.
        let regular_idx = pick(&faces, families, false)
            .or_else(|| faces.iter().position(|f| !f.is_bold))
            .unwrap_or(0);
        let bold_idx =
            pick(&faces, families, true).or_else(|| faces.iter().position(|f| f.is_bold));

        // Extract regular and bold (bold defaults to a copy of regular).
        let regular = faces[regular_idx].clone_face();
        let bold = match bold_idx {
            Some(i) if i != regular_idx => faces[i].clone_face(),
            _ => faces[regular_idx].clone_face(),
        };

        // Everything not chosen becomes a fallback, preserving order.
        let mut fallbacks = Vec::new();
        for (i, f) in faces.into_iter().enumerate() {
            if i == regular_idx || Some(i) == bold_idx {
                continue;
            }
            fallbacks.push(f);
        }

        Ok(FontRegistry {
            regular,
            bold,
            fallbacks,
        })
    }

    /// Append a fallback face from raw font bytes (TTF/OTF).
    pub fn add_fallback(&mut self, bytes: Vec<u8>) -> Result<()> {
        let face = LoadedFace::load(bytes, "fallback".to_string())?;
        self.fallbacks.push(face);
        Ok(())
    }

    /// Number of loaded fallback faces.
    pub fn fallback_count(&self) -> usize {
        self.fallbacks.len()
    }

    /// Raw bytes for a slot (used by the backend to embed the face).
    pub fn bytes(&self, slot: FontSlot) -> &[u8] {
        &self.face_of(slot).bytes
    }

    /// Bytes for a slot subset to just the glyphs needed for `used_chars`,
    /// suitable for embedding. Glyph IDs and the `cmap` are retained, so this is
    /// transparent to the PDF backend (see [`subset`]). Falls back to the full
    /// font bytes if subsetting fails.
    pub fn subset_bytes(&self, slot: FontSlot, used_chars: &BTreeSet<char>) -> Vec<u8> {
        let lf = self.face_of(slot);
        let face = lf.parse();
        // Always keep .notdef (the subsetter retains it regardless; explicit here
        // for clarity). Whitespace chars without a glyph are simply not drawn.
        let mut gids: BTreeSet<u16> = BTreeSet::from([0]);
        for &ch in used_chars {
            if let Some(g) = face.glyph_index(ch) {
                gids.insert(g.0);
            }
        }
        let gids: Vec<u16> = gids.into_iter().collect();
        subset::subset_face(&lf.bytes, 0, &gids)
    }

    fn primary_slot(weight: FontWeight) -> FontSlot {
        match weight {
            FontWeight::Bold => FontSlot::Bold,
            FontWeight::Normal => FontSlot::Regular,
        }
    }

    fn primary_face(&self, weight: FontWeight) -> &LoadedFace {
        match weight {
            FontWeight::Bold => &self.bold,
            FontWeight::Normal => &self.regular,
        }
    }

    fn face_of(&self, slot: FontSlot) -> &LoadedFace {
        match slot {
            FontSlot::Regular => &self.regular,
            FontSlot::Bold => &self.bold,
            FontSlot::Fallback(i) => self.fallbacks.get(i).unwrap_or(&self.regular),
        }
    }

    /// Ascent (em fraction) for the primary face of `weight`.
    pub fn ascent(&self, weight: FontWeight) -> f32 {
        self.primary_face(weight).ascent
    }

    /// Descent (em fraction, negative) for the primary face of `weight`.
    pub fn descent(&self, weight: FontWeight) -> f32 {
        self.primary_face(weight).descent
    }

    /// Cap height (em fraction) for the primary face of `weight`. Used to
    /// optically center cell text rather than centering the full ascent box.
    pub fn cap_height(&self, weight: FontWeight) -> f32 {
        self.primary_face(weight).cap_height
    }

    /// Split `text` into runs by font coverage: the primary face for `weight`,
    /// then fallback faces in load order. Characters covered by no loaded font
    /// are dropped (keeping the PDF free of `.notdef` references) and raise a
    /// single warning.
    pub fn itemize(
        &self,
        text: &str,
        weight: FontWeight,
        warnings: &mut Vec<RenderWarning>,
    ) -> Vec<Run> {
        let primary_slot = Self::primary_slot(weight);
        let primary = self.primary_face(weight).parse();
        let fallbacks: Vec<Face> = self.fallbacks.iter().map(|f| f.parse()).collect();

        let mut runs: Vec<Run> = Vec::new();
        let mut missing = false;
        for ch in text.chars() {
            let slot = if covers(&primary, ch) {
                primary_slot
            } else if let Some(i) = fallbacks.iter().position(|f| covers(f, ch)) {
                FontSlot::Fallback(i)
            } else {
                // No loaded font covers this character. Drop it (rather than
                // emit a .notdef glyph, which breaks PDF/A font-width rules) and
                // record a warning.
                missing = true;
                continue;
            };
            match runs.last_mut() {
                Some(r) if r.slot == slot => r.text.push(ch),
                _ => runs.push(Run {
                    slot,
                    text: ch.to_string(),
                }),
            }
        }

        if missing {
            warnings.push(RenderWarning::MissingGlyph {
                text: text.to_string(),
            });
        }
        runs
    }

    /// Width (pt) of `text` at `size`, accounting for font fallback.
    pub fn measure(&self, text: &str, weight: FontWeight, size: f32) -> f32 {
        let mut sink = Vec::new();
        self.itemize(text, weight, &mut sink)
            .iter()
            .map(|run| self.measure_run(run, size))
            .sum()
    }

    /// Width (pt) of a single run's text at `size`.
    pub fn measure_run(&self, run: &Run, size: f32) -> f32 {
        let lf = self.face_of(run.slot);
        let face = lf.parse();
        run.text
            .chars()
            .map(|c| advance(&face, lf.units_per_em, c, size))
            .sum()
    }

    /// Advance width (pt) of a single character in `slot` at `size`. Used by the
    /// styled (inline-rich-text) wrapper. `parse()` is zero-copy/lazy, so the
    /// per-character cost is small.
    pub fn char_advance(&self, slot: FontSlot, ch: char, size: f32) -> f32 {
        let lf = self.face_of(slot);
        advance(&lf.parse(), lf.units_per_em, ch, size)
    }
}

impl LoadedFace {
    fn clone_face(&self) -> LoadedFace {
        LoadedFace {
            bytes: self.bytes.clone(),
            key: self.key.clone(),
            is_bold: self.is_bold,
            units_per_em: self.units_per_em,
            ascent: self.ascent,
            descent: self.descent,
            cap_height: self.cap_height,
        }
    }
}

/// Pick the first face matching one of `families` with the requested boldness.
fn pick(faces: &[LoadedFace], families: &[String], want_bold: bool) -> Option<usize> {
    for fam in families {
        if let Some(i) = faces
            .iter()
            .position(|f| f.is_bold == want_bold && f.matches_family(fam))
        {
            return Some(i);
        }
    }
    None
}

/// List `.ttf`/`.otf`/`.ttc` files in `dir`, sorted by name. Missing dir → empty.
fn font_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
        Err(_) => return Vec::new(),
    };
    files.retain(|p| {
        matches!(
            p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()),
            Some(ref e) if e == "ttf" || e == "otf" || e == "ttc"
        )
    });
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf_fonts() -> FontRegistry {
        FontRegistry::from_font_dirs(&[PathBuf::from("fonts")], &["titillium".to_string()])
            .expect("load fonts from ./fonts")
    }

    #[test]
    fn loads_primary_family() {
        let r = pdf_fonts();
        // Only Titillium R/B in ./fonts → no fallbacks.
        assert_eq!(r.fallback_count(), 0);
    }

    #[test]
    fn latin_is_single_run() {
        let r = pdf_fonts();
        let mut w = Vec::new();
        let runs = r.itemize("Invoice", FontWeight::Normal, &mut w);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].slot, FontSlot::Regular);
        assert!(w.is_empty());
    }

    #[test]
    fn bold_resolves_to_bold_face() {
        let r = pdf_fonts();
        // The bold face differs from regular (separate file).
        assert!(r.bold.is_bold);
        assert!(!r.regular.is_bold);
    }

    #[test]
    fn cjk_without_fallback_warns_gracefully() {
        let r = pdf_fonts();
        let mut w = Vec::new();
        let runs = r.itemize("Invoice こんにちは", FontWeight::Normal, &mut w);
        assert!(runs.iter().all(|run| run.slot == FontSlot::Regular));
        assert_eq!(w.len(), 1);
        assert!(matches!(w[0], RenderWarning::MissingGlyph { .. }));
    }

    #[test]
    fn cjk_fallback_when_available() {
        // The lang app's font folder carries Titillium + Noto Sans JP. Skip if
        // absent (crate built in isolation).
        let dirs = [PathBuf::from("../lang/font")];
        let r = match FontRegistry::from_font_dirs(&dirs, &["titillium".to_string()]) {
            Ok(r) if r.fallback_count() > 0 => r,
            _ => return,
        };
        let mut w = Vec::new();
        let runs = r.itemize("Invoice こんにちは世界", FontWeight::Normal, &mut w);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].slot, FontSlot::Regular);
        assert!(matches!(runs[1].slot, FontSlot::Fallback(_)));
        assert!(w.is_empty());
    }

    #[test]
    fn measure_is_positive() {
        let r = pdf_fonts();
        let w = r.measure("Hello", FontWeight::Normal, 12.0);
        assert!(w > 0.0 && w < 100.0);
    }
}
