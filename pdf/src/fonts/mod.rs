//! Font loading, coverage itemization, and advance-width metrics.
//!
//! **No fonts are embedded in the binary.** All faces are loaded at runtime
//! from the directories in `RenderOptions::font_dirs` (CLI `--font-dir`). The
//! template's `fonts: [...]` field names the family to use as the primary text
//! face; its Regular/Bold/Italic/BoldItalic styles are resolved from the loaded
//! files. A monospaced font (e.g. JetBrains Mono) supplies the `code`/mono
//! faces, and every other loaded font (e.g. a CJK font) becomes a coverage
//! fallback for characters the primary can't cover.
//!
//! Text is split into runs by which face covers each character (the requested
//! styled face → fallback faces, in load order). Advance widths come from
//! `ttf-parser`; printpdf does the actual glyph encoding + embedding.

mod subset;

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ttf_parser::Face;

use crate::error::{PdfError, RenderWarning, Result};
use crate::template::FontWeight;

/// A primary/monospace face style: which (monospace? × bold? × italic?)
/// combination to draw with. `FaceKey::default()` is the plain Regular face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FaceKey {
    pub mono: bool,
    pub bold: bool,
    pub italic: bool,
}

impl From<FontWeight> for FaceKey {
    fn from(w: FontWeight) -> Self {
        FaceKey {
            mono: false,
            bold: matches!(w, FontWeight::Bold),
            italic: false,
        }
    }
}

/// Which face a run of text should be drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontSlot {
    /// A primary-family or monospace styled face.
    Styled(FaceKey),
    /// A coverage fallback face, by load index.
    Fallback(usize),
}

impl FontSlot {
    /// The plain Regular slot (always present in the registry).
    pub const REGULAR: FontSlot = FontSlot::Styled(FaceKey {
        mono: false,
        bold: false,
        italic: false,
    });
}

/// One loaded face. Advance widths and coverage are precomputed once at load
/// into `advances` (char → advance in em fractions), so metric lookups during
/// layout are a hashmap hit rather than a fresh `Face::parse` + cmap/hmtx walk
/// per character — layout used to re-parse the sfnt table directory ~900×/render
/// (see `benches/render.rs`). The raw `bytes` are kept for subsetting/embedding.
#[derive(Clone)]
struct LoadedFace {
    bytes: Vec<u8>,
    /// Lowercased file stem, used for family matching (e.g. "titilliumweb-bold").
    key: String,
    is_bold: bool,
    is_italic: bool,
    is_mono: bool,
    ascent: f32,
    descent: f32,
    cap_height: f32,
    /// Covered chars → advance width as an em fraction (`advance_units / upem`).
    /// A char is "covered" iff present here; missing chars fall back to the
    /// default advance in [`LoadedFace::advance`].
    advances: HashMap<char, f32>,
}

impl LoadedFace {
    fn load(bytes: Vec<u8>, key: String) -> Result<LoadedFace> {
        let (ascent, descent, cap_height, is_bold, is_italic, is_mono);
        let mut advances: HashMap<char, f32> = HashMap::new();
        {
            let face = Face::parse(&bytes, 0)
                .map_err(|e| PdfError::Font(format!("ttf-parser failed for {key}: {e}")))?;
            let u = face.units_per_em() as f32;
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
            // Style detection: trust the font's OS/2 / post flags, but fall back
            // to the filename (e.g. "...-BoldItalic", "JetBrainsMono-...") since
            // not every face sets the flags reliably.
            is_bold = face.is_bold() || key.contains("bold");
            is_italic = face.is_italic() || key.contains("italic") || key.contains("oblique");
            is_mono = face.is_monospaced() || key.contains("mono");

            // Precompute the advance for every char the (Unicode) cmap maps to a
            // glyph. This mirrors the old per-char path exactly: coverage =
            // "glyph_index present", advance = `glyph_hor_advance / upem`.
            if let Some(cmap) = face.tables().cmap {
                for subtable in cmap.subtables {
                    if !subtable.is_unicode() {
                        continue;
                    }
                    subtable.codepoints(|cp| {
                        if let Some(ch) = char::from_u32(cp) {
                            if let Some(gid) = face.glyph_index(ch) {
                                if let Some(adv) = face.glyph_hor_advance(gid) {
                                    advances.entry(ch).or_insert(adv as f32 / u);
                                }
                            }
                        }
                    });
                }
            }
        }
        Ok(LoadedFace {
            bytes,
            key,
            is_bold,
            is_italic,
            is_mono,
            ascent,
            descent,
            cap_height,
            advances,
        })
    }

    fn parse(&self) -> Face<'_> {
        Face::parse(&self.bytes, 0).expect("face was validated at load")
    }

    fn matches_family(&self, family: &str) -> bool {
        let fam = family.to_ascii_lowercase().replace([' ', '-', '_'], "");
        self.key.replace([' ', '-', '_'], "").contains(&fam)
    }

    /// Whether this face can draw `ch` (whitespace is always considered covered,
    /// matching the drawing path, which simply advances the cursor for it).
    fn covers(&self, ch: char) -> bool {
        ch.is_whitespace() || self.advances.contains_key(&ch)
    }

    /// Advance width (pt) of `ch` at `size`. Uncovered chars fall back to a
    /// nominal width (a narrow space, else half an em) — identical to the
    /// pre-cache per-glyph path.
    fn advance(&self, ch: char, size: f32) -> f32 {
        if let Some(em) = self.advances.get(&ch) {
            em * size
        } else if ch == ' ' {
            0.25 * size
        } else {
            0.5 * size
        }
    }
}

/// A maximal run of characters drawn with one face.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub slot: FontSlot,
    pub text: String,
}

/// The loaded font set: styled faces (primary family + monospace) keyed by
/// style, plus coverage fallback faces.
pub struct FontRegistry {
    faces: HashMap<FaceKey, LoadedFace>,
    fallbacks: Vec<LoadedFace>,
}

/// Key for the process-wide [`FontRegistry::cached`] map.
#[derive(PartialEq, Eq, Hash)]
struct CacheKey {
    dirs: Vec<PathBuf>,
    families: Vec<String>,
}

fn font_cache() -> &'static Mutex<HashMap<CacheKey, Arc<FontRegistry>>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Arc<FontRegistry>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl FontRegistry {
    /// Build a registry from the fonts found in `dirs`. `families` (from the
    /// template's `fonts` field) selects the primary family; its Regular/Bold/
    /// Italic/BoldItalic faces — and any monospaced face — are bucketed by style,
    /// while every other loaded font becomes a coverage fallback. Errors if no
    /// usable font is found.
    pub fn from_font_dirs(dirs: &[PathBuf], families: &[String]) -> Result<FontRegistry> {
        let mut loaded: Vec<LoadedFace> = Vec::new();
        for dir in dirs {
            for path in font_files(dir) {
                if let Ok(bytes) = fs::read(&path) {
                    let key = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("font")
                        .to_ascii_lowercase();
                    if let Ok(face) = LoadedFace::load(bytes, key) {
                        loaded.push(face);
                    }
                }
            }
        }
        if loaded.is_empty() {
            return Err(PdfError::Font(format!(
                "no usable fonts found in {dirs:?}; provide a font directory (RenderOptions::font_dirs / --font-dir)"
            )));
        }

        // If any non-mono face matches a requested family, only those become the
        // primary family; otherwise treat every non-mono face as primary so
        // bold/italic still resolve even when the family name doesn't match.
        let any_family_match = loaded
            .iter()
            .any(|f| !f.is_mono && families.iter().any(|fam| f.matches_family(fam)));

        let mut faces: HashMap<FaceKey, LoadedFace> = HashMap::new();
        let mut fallbacks: Vec<LoadedFace> = Vec::new();
        for f in loaded {
            if f.is_mono {
                faces
                    .entry(FaceKey {
                        mono: true,
                        bold: f.is_bold,
                        italic: f.is_italic,
                    })
                    .or_insert(f);
            } else if !any_family_match || families.iter().any(|fam| f.matches_family(fam)) {
                faces
                    .entry(FaceKey {
                        mono: false,
                        bold: f.is_bold,
                        italic: f.is_italic,
                    })
                    .or_insert(f);
            } else {
                fallbacks.push(f);
            }
        }

        // Guarantee a plain Regular face — `face_of` uses it as the final
        // fallback, so it must exist.
        if !faces.contains_key(&FaceKey::default()) {
            let regular = faces
                .values()
                .find(|f| !f.is_bold && !f.is_italic && !f.is_mono)
                .cloned()
                .or_else(|| fallbacks.first().cloned())
                .or_else(|| faces.values().next().cloned());
            match regular {
                Some(f) => {
                    faces.insert(FaceKey::default(), f);
                }
                None => {
                    return Err(PdfError::Font(
                        "no usable primary font found in the font directories".to_string(),
                    ))
                }
            }
        }

        Ok(FontRegistry { faces, fallbacks })
    }

    /// Like [`from_font_dirs`], but memoized process-wide, keyed by
    /// canonicalized `dirs` + `families`. Font files don't change while a
    /// server is running, so repeated renders with the same font
    /// configuration reuse one already-loaded registry instead of paying a
    /// full directory scan + read + parse of every font file on every call —
    /// this was the dominant per-render cost (see `benches/render.rs`).
    pub fn cached(dirs: &[PathBuf], families: &[String]) -> Result<Arc<FontRegistry>> {
        let key = CacheKey {
            dirs: dirs
                .iter()
                .map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone()))
                .collect(),
            families: families.to_vec(),
        };
        if let Some(hit) = font_cache().lock().unwrap().get(&key) {
            return Ok(hit.clone());
        }
        let registry = Arc::new(FontRegistry::from_font_dirs(dirs, families)?);
        font_cache().lock().unwrap().insert(key, registry.clone());
        Ok(registry)
    }

    /// Raw bytes of every loaded face (primary/mono styles + coverage
    /// fallbacks), for feeding to a font source that wants bytes rather than
    /// directories — e.g. SVG `<text>` rendering, which would otherwise
    /// re-scan and re-read `font_dirs` from disk on its own.
    pub fn all_font_bytes(&self) -> Vec<&[u8]> {
        self.faces
            .values()
            .map(|f| f.bytes.as_slice())
            .chain(self.fallbacks.iter().map(|f| f.bytes.as_slice()))
            .collect()
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

    /// The guaranteed plain Regular face.
    fn regular(&self) -> &LoadedFace {
        self.faces
            .get(&FaceKey::default())
            .expect("a Regular face is guaranteed at load")
    }

    /// Resolve a styled face, degrading gracefully when an exact face isn't
    /// loaded: drop italic, then bold — *keeping* monospace as long as possible
    /// — then drop monospace, ultimately the plain Regular.
    fn resolve_styled(&self, key: FaceKey) -> &LoadedFace {
        let candidates = [
            FaceKey {
                mono: key.mono,
                bold: key.bold,
                italic: key.italic,
            },
            FaceKey {
                mono: key.mono,
                bold: key.bold,
                italic: false,
            },
            FaceKey {
                mono: key.mono,
                bold: false,
                italic: key.italic,
            },
            FaceKey {
                mono: key.mono,
                bold: false,
                italic: false,
            },
            FaceKey {
                mono: false,
                bold: key.bold,
                italic: key.italic,
            },
            FaceKey {
                mono: false,
                bold: key.bold,
                italic: false,
            },
            FaceKey {
                mono: false,
                bold: false,
                italic: key.italic,
            },
            FaceKey::default(),
        ];
        for k in candidates {
            if let Some(f) = self.faces.get(&k) {
                return f;
            }
        }
        self.regular()
    }

    fn face_of(&self, slot: FontSlot) -> &LoadedFace {
        match slot {
            FontSlot::Styled(key) => self.resolve_styled(key),
            FontSlot::Fallback(i) => self.fallbacks.get(i).unwrap_or_else(|| self.regular()),
        }
    }

    /// Ascent (em fraction) for the primary face of `weight`.
    pub fn ascent(&self, weight: FontWeight) -> f32 {
        self.resolve_styled(FaceKey::from(weight)).ascent
    }

    /// Descent (em fraction, negative) for the primary face of `weight`.
    pub fn descent(&self, weight: FontWeight) -> f32 {
        self.resolve_styled(FaceKey::from(weight)).descent
    }

    /// Cap height (em fraction) for the primary face of `weight`. Used to
    /// optically center cell text rather than centering the full ascent box.
    pub fn cap_height(&self, weight: FontWeight) -> f32 {
        self.resolve_styled(FaceKey::from(weight)).cap_height
    }

    /// Split `text` into runs by font coverage: the requested styled face
    /// (`key`, which accepts a `FontWeight` or a full `FaceKey`), then fallback
    /// faces in load order. Characters covered by no loaded font are dropped
    /// (keeping the PDF free of `.notdef` references) and raise a single warning.
    pub fn itemize(
        &self,
        text: &str,
        key: impl Into<FaceKey>,
        warnings: &mut Vec<RenderWarning>,
    ) -> Vec<Run> {
        let key = key.into();
        let primary_slot = FontSlot::Styled(key);
        let primary = self.resolve_styled(key);

        let mut runs: Vec<Run> = Vec::new();
        let mut missing = false;
        for ch in text.chars() {
            let slot = if primary.covers(ch) {
                primary_slot
            } else if let Some(i) = self.fallbacks.iter().position(|f| f.covers(ch)) {
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
    pub fn measure(&self, text: &str, key: impl Into<FaceKey>, size: f32) -> f32 {
        let mut sink = Vec::new();
        self.itemize(text, key, &mut sink)
            .iter()
            .map(|run| self.measure_run(run, size))
            .sum()
    }

    /// Width (pt) of a single run's text at `size`.
    pub fn measure_run(&self, run: &Run, size: f32) -> f32 {
        let lf = self.face_of(run.slot);
        run.text.chars().map(|c| lf.advance(c, size)).sum()
    }

    /// Advance width (pt) of a single character in `slot` at `size`. Used by the
    /// styled (inline-rich-text) wrapper — a cached hashmap lookup.
    pub fn char_advance(&self, slot: FontSlot, ch: char, size: f32) -> f32 {
        self.face_of(slot).advance(ch, size)
    }
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
        // ./fonts holds Titillium R/B/I/BI + JetBrains Mono R/B/I — all are either
        // the primary family or monospaced, so none become coverage fallbacks.
        assert_eq!(r.fallback_count(), 0);
    }

    #[test]
    fn latin_is_single_run() {
        let r = pdf_fonts();
        let mut w = Vec::new();
        let runs = r.itemize("Invoice", FontWeight::Normal, &mut w);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].slot, FontSlot::REGULAR);
        assert!(w.is_empty());
    }

    #[test]
    fn bold_resolves_to_bold_face() {
        let r = pdf_fonts();
        assert!(
            r.faces[&FaceKey {
                mono: false,
                bold: true,
                italic: false
            }]
                .is_bold
        );
        assert!(!r.faces[&FaceKey::default()].is_bold);
    }

    #[test]
    fn italic_and_mono_faces_load_and_route() {
        let r = pdf_fonts();
        for key in [
            FaceKey {
                mono: false,
                bold: false,
                italic: true,
            },
            FaceKey {
                mono: false,
                bold: true,
                italic: true,
            },
            FaceKey {
                mono: true,
                bold: false,
                italic: false,
            },
        ] {
            assert!(r.faces.contains_key(&key), "missing face {key:?}");
        }
        let mut w = Vec::new();
        let mono = FaceKey {
            mono: true,
            bold: false,
            italic: false,
        };
        let runs = r.itemize("x", mono, &mut w);
        assert_eq!(runs[0].slot, FontSlot::Styled(mono));
    }

    #[test]
    fn missing_style_degrades_without_panicking() {
        // mono-bold-italic isn't shipped (only mono R/B/I) → resolves to a loaded
        // face via degradation rather than panicking.
        let r = pdf_fonts();
        let key = FaceKey {
            mono: true,
            bold: true,
            italic: true,
        };
        let w = r.char_advance(FontSlot::Styled(key), 'x', 12.0);
        assert!(w > 0.0);
    }

    #[test]
    fn cjk_without_fallback_warns_gracefully() {
        let r = pdf_fonts();
        let mut w = Vec::new();
        let runs = r.itemize("Invoice こんにちは", FontWeight::Normal, &mut w);
        assert!(runs.iter().all(|run| run.slot == FontSlot::REGULAR));
        assert_eq!(w.len(), 1);
        assert!(matches!(w[0], RenderWarning::MissingGlyph { .. }));
    }

    #[test]
    fn cjk_fallback_when_available() {
        // The lang app's font folder carries Titillium + JetBrains Mono + Noto
        // Sans JP. Skip if absent (crate built in isolation).
        let dirs = [PathBuf::from("../font")];
        let r = match FontRegistry::from_font_dirs(&dirs, &["titillium".to_string()]) {
            Ok(r) if r.fallback_count() > 0 => r,
            _ => return,
        };
        let mut w = Vec::new();
        let runs = r.itemize("Invoice こんにちは世界", FontWeight::Normal, &mut w);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].slot, FontSlot::REGULAR);
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
