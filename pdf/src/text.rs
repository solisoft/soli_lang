//! Word wrapping and horizontal alignment over font-aware measurements.

use crate::color::Rgb;
use crate::error::RenderWarning;
use crate::fonts::{FaceKey, FontRegistry, FontSlot};
use crate::template::{Alignment, FontWeight};

/// Line height multiplier applied to the font size.
pub const LINE_HEIGHT_FACTOR: f32 = 1.2;

/// Line height (pt) for a given font size.
pub fn line_height(size: f32) -> f32 {
    size * LINE_HEIGHT_FACTOR
}

/// Wrap `text` to fit `max_width` pt. Explicit `\n` always break. Words wider
/// than `max_width` are hard-broken by character. Returns at least one line.
pub fn wrap(
    reg: &FontRegistry,
    text: &str,
    weight: FontWeight,
    size: f32,
    max_width: f32,
) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        wrap_paragraph(reg, para, weight, size, max_width, &mut lines);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_paragraph(
    reg: &FontRegistry,
    para: &str,
    weight: FontWeight,
    size: f32,
    max_width: f32,
    out: &mut Vec<String>,
) {
    if max_width <= 0.0 {
        out.push(para.to_string());
        return;
    }
    let mut current = String::new();
    for word in para.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if reg.measure(&candidate, weight, size) <= max_width || current.is_empty() {
            if reg.measure(word, weight, size) > max_width && current.is_empty() {
                // The word alone is too wide: hard-break it.
                hard_break(reg, word, weight, size, max_width, out, &mut current);
            } else {
                current = candidate;
            }
        } else {
            out.push(std::mem::take(&mut current));
            // Re-process this word on a fresh line.
            if reg.measure(word, weight, size) > max_width {
                hard_break(reg, word, weight, size, max_width, out, &mut current);
            } else {
                current = word.to_string();
            }
        }
    }
    out.push(current);
}

/// Break a single over-long word across multiple lines by character.
fn hard_break(
    reg: &FontRegistry,
    word: &str,
    weight: FontWeight,
    size: f32,
    max_width: f32,
    out: &mut Vec<String>,
    current: &mut String,
) {
    for ch in word.chars() {
        let candidate = format!("{current}{ch}");
        if !current.is_empty() && reg.measure(&candidate, weight, size) > max_width {
            out.push(std::mem::take(current));
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
}

/// A run of text with one resolved inline style — input to [`layout_styled_lines`].
pub struct StyledSeg {
    pub text: String,
    pub size: f32,
    pub weight: FontWeight,
    pub italic: bool,
    pub mono: bool,
    pub color: Rgb,
    /// Index into the caller's link table, if this segment is a clickable link.
    pub link: Option<usize>,
}

/// One positioned character with its resolved style (output of itemization).
#[derive(Clone, Copy)]
pub struct StyledChar {
    pub ch: char,
    pub slot: FontSlot,
    pub size: f32,
    pub color: Rgb,
    pub link: Option<usize>,
}

enum Tok {
    Word(Vec<StyledChar>, f32),
    Space(Vec<StyledChar>, f32),
    Newline,
}

/// Itemize styled segments by font coverage, then greedy-wrap to `max_width`,
/// measuring each character at its own size. Returns one line of styled
/// characters per output line (always ≥1). Explicit `\n` forces a break;
/// over-wide words are hard-broken by character.
pub fn layout_styled_lines(
    reg: &FontRegistry,
    segs: &[StyledSeg],
    max_width: f32,
    warnings: &mut Vec<RenderWarning>,
) -> Vec<Vec<StyledChar>> {
    // Flatten to styled chars, assigning a font slot per char via itemization.
    let mut chars: Vec<StyledChar> = Vec::new();
    for seg in segs {
        let mut first = true;
        for part in seg.text.split('\n') {
            if !first {
                chars.push(StyledChar {
                    ch: '\n',
                    slot: FontSlot::REGULAR,
                    size: seg.size,
                    color: seg.color,
                    link: seg.link,
                });
            }
            first = false;
            let key = FaceKey {
                mono: seg.mono,
                bold: matches!(seg.weight, FontWeight::Bold),
                italic: seg.italic,
            };
            for run in reg.itemize(part, key, warnings) {
                for ch in run.text.chars() {
                    chars.push(StyledChar {
                        ch,
                        slot: run.slot,
                        size: seg.size,
                        color: seg.color,
                        link: seg.link,
                    });
                }
            }
        }
    }
    if chars.is_empty() {
        return vec![Vec::new()];
    }

    let adv = |c: &StyledChar| reg.char_advance(c.slot, c.ch, c.size);

    // Tokenize into words, whitespace runs, and explicit newline markers.
    let mut toks: Vec<Tok> = Vec::new();
    let mut buf: Vec<StyledChar> = Vec::new();
    let mut buf_w = 0.0f32;
    let mut buf_space = false;
    for c in &chars {
        if c.ch == '\n' {
            if !buf.is_empty() {
                toks.push(make_tok(std::mem::take(&mut buf), buf_w, buf_space));
                buf_w = 0.0;
            }
            toks.push(Tok::Newline);
            continue;
        }
        let is_space = c.ch.is_whitespace();
        if buf.is_empty() {
            buf_space = is_space;
        } else if is_space != buf_space {
            toks.push(make_tok(std::mem::take(&mut buf), buf_w, buf_space));
            buf_w = 0.0;
            buf_space = is_space;
        }
        buf.push(*c);
        buf_w += adv(c);
    }
    if !buf.is_empty() {
        toks.push(make_tok(buf, buf_w, buf_space));
    }

    // Greedy pack tokens into lines.
    let mut lines: Vec<Vec<StyledChar>> = Vec::new();
    let mut line: Vec<StyledChar> = Vec::new();
    let mut line_w = 0.0f32;
    let mut space: Vec<StyledChar> = Vec::new();
    let mut space_w = 0.0f32;

    for tok in toks {
        match tok {
            Tok::Newline => {
                space.clear();
                space_w = 0.0;
                lines.push(std::mem::take(&mut line));
                line_w = 0.0;
            }
            Tok::Space(chars, w) => {
                space = chars;
                space_w = w;
            }
            Tok::Word(word, w) => {
                if line.is_empty() {
                    space.clear();
                    space_w = 0.0;
                    if w <= max_width || max_width <= 0.0 {
                        line.extend(word);
                        line_w = w;
                    } else {
                        hard_break_styled(
                            reg,
                            &word,
                            max_width,
                            &mut lines,
                            &mut line,
                            &mut line_w,
                        );
                    }
                } else if line_w + space_w + w <= max_width {
                    line.append(&mut space);
                    line_w += space_w;
                    space_w = 0.0;
                    line.extend(word);
                    line_w += w;
                } else {
                    space.clear();
                    space_w = 0.0;
                    lines.push(std::mem::take(&mut line));
                    line_w = 0.0;
                    if w <= max_width {
                        line.extend(word);
                        line_w = w;
                    } else {
                        hard_break_styled(
                            reg,
                            &word,
                            max_width,
                            &mut lines,
                            &mut line,
                            &mut line_w,
                        );
                    }
                }
            }
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn make_tok(chars: Vec<StyledChar>, w: f32, is_space: bool) -> Tok {
    if is_space {
        Tok::Space(chars, w)
    } else {
        Tok::Word(chars, w)
    }
}

fn hard_break_styled(
    reg: &FontRegistry,
    word: &[StyledChar],
    max_width: f32,
    lines: &mut Vec<Vec<StyledChar>>,
    line: &mut Vec<StyledChar>,
    line_w: &mut f32,
) {
    for &c in word {
        let cw = reg.char_advance(c.slot, c.ch, c.size);
        if !line.is_empty() && *line_w + cw > max_width {
            lines.push(std::mem::take(line));
            *line_w = 0.0;
        }
        line.push(c);
        *line_w += cw;
    }
}

/// Left x of a line of width `line_width` aligned within `[region_left, +region_width]`.
pub fn align_x(region_left: f32, region_width: f32, line_width: f32, alignment: Alignment) -> f32 {
    match alignment {
        Alignment::Left => region_left,
        Alignment::Right => region_left + (region_width - line_width).max(0.0),
        Alignment::Center => region_left + ((region_width - line_width).max(0.0)) / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn reg() -> FontRegistry {
        FontRegistry::from_font_dirs(&[PathBuf::from("fonts")], &["titillium".to_string()]).unwrap()
    }

    #[test]
    fn short_text_one_line() {
        let reg = reg();
        let lines = wrap(&reg, "Hello world", FontWeight::Normal, 12.0, 500.0);
        assert_eq!(lines, vec!["Hello world".to_string()]);
    }

    #[test]
    fn wraps_long_text() {
        let reg = reg();
        let text = "The quick brown fox jumps over the lazy dog again and again";
        let lines = wrap(&reg, text, FontWeight::Normal, 12.0, 80.0);
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(reg.measure(l, FontWeight::Normal, 12.0) <= 80.0 + 0.01 || !l.contains(' '));
        }
    }

    #[test]
    fn alignment_offsets() {
        assert_eq!(align_x(10.0, 100.0, 40.0, Alignment::Left), 10.0);
        assert_eq!(align_x(10.0, 100.0, 40.0, Alignment::Right), 70.0);
        assert_eq!(align_x(10.0, 100.0, 40.0, Alignment::Center), 40.0);
    }
}
