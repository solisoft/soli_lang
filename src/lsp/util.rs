//! Position helpers shared by the LSP providers.

use crate::lexer::TokenKind;
use crate::span::Span;
use tower_lsp::lsp_types::{Position, Range};

/// The identifier around byte `offset` — the unit of `Span::start` and of
/// `position_to_offset`. It used to index a `Vec<char>` with that byte offset,
/// which picked the wrong word after any non-ASCII text.
pub(super) fn extract_word_at_offset(source: &str, offset: usize) -> Option<String> {
    if offset >= source.len() || !source.is_char_boundary(offset) {
        return None;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let start = source[..offset]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(offset, |(i, _)| i);
    let end = source[offset..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(source.len(), |(i, _)| offset + i);
    (start < end).then(|| source[start..end].to_string())
}

/// The byte offset of an LSP position. `character` counts UTF-16 code units,
/// as LSP does by default, and a position past the end of its line clamps to
/// the end of the line, before any `\r\n` (which used to cost one byte per
/// line: `lines()` drops the `\r` and the offset added back only the `\n`).
pub(super) fn position_to_offset(source: &str, position: Position) -> Option<usize> {
    let mut line_start = 0;
    for (line, text) in source.split_inclusive('\n').enumerate() {
        if line as u32 == position.line {
            let content = text.trim_end_matches('\n').trim_end_matches('\r');
            let mut units = 0;
            for (i, c) in content.char_indices() {
                if units >= position.character as usize {
                    return Some(line_start + i);
                }
                units += c.len_utf16();
            }
            return Some(line_start + content.len());
        }
        line_start += text.len();
    }
    None
}

/// A span as an LSP range on its first line. The end is the start plus the
/// span's length (it was one past it); both are exact for an ASCII
/// identifier, since the span's column counts chars and its length bytes.
pub(super) fn lsp_range_from_span(span: Span) -> Range {
    let line = span.line.saturating_sub(1);
    let character = span.column.saturating_sub(1);
    Range {
        start: Position { line, character },
        end: Position {
            line,
            character: character + (span.end - span.start),
        },
    }
}

/// The LSP position of byte `offset`: zero-based line, column in UTF-16 units
/// (the inverse of `position_to_offset`).
pub(super) fn offset_to_position(source: &str, offset: usize) -> Position {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &source[..offset];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let character = before[line_start..].encode_utf16().count() as u32;
    Position { line, character }
}

/// The LSP range of the byte range `start..end`.
pub(super) fn range_of(source: &str, start: usize, end: usize) -> Range {
    Range {
        start: offset_to_position(source, start),
        end: offset_to_position(source, end),
    }
}

/// Byte ranges of every identifier spelled `name`, in source order: in the
/// code and inside `#{…}` interpolations (strings and `@sdbql` blocks), never
/// in a plain string or a comment. References and rename are built on it —
/// they used to list only the *declarations* bearing the name, so a rename
/// left every call site behind.
pub(super) fn identifier_occurrences(source: &str, name: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    collect_identifiers(source, 0, name, &mut found, 0);
    found.sort_unstable();
    found.dedup();
    found
}

fn collect_identifiers(
    text: &str,
    base: usize,
    name: &str,
    found: &mut Vec<(usize, usize)>,
    depth: usize,
) {
    // Interpolations nest (`"#{"#{x}"}"`); the bound only guards pathological input.
    if depth > 16 {
        return;
    }
    let Ok(tokens) = crate::lexer::Scanner::new(text).scan_tokens() else {
        return;
    };
    for token in &tokens {
        let (start, end) = (token.span.start as usize, token.span.end as usize);
        match &token.kind {
            TokenKind::Identifier(word) if word == name => found.push((base + start, base + end)),
            TokenKind::InterpolatedString(_) | TokenKind::SdqlBlock { .. } => {
                let Some(raw) = text.get(start..end) else {
                    continue;
                };
                for (inner_start, inner) in interpolation_bodies(raw) {
                    collect_identifiers(inner, base + start + inner_start, name, found, depth + 1);
                }
            }
            _ => {}
        }
    }
}

/// The body of each `#{…}` in a string or query token, with its byte offset in
/// that token's text. Braces nest, so `#{ {"a": 1}["a"] }` is one body.
fn interpolation_bodies(raw: &str) -> Vec<(usize, &str)> {
    let bytes = raw.as_bytes();
    let mut bodies = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b'#' || bytes[i + 1] != b'{' {
            i += 1;
            continue;
        }
        let body_start = i + 2;
        let mut depth = 1;
        let mut j = body_start;
        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        if depth != 0 {
            break;
        }
        bodies.push((body_start, &raw[body_start..j - 1]));
        i = j;
    }
    bodies
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn a_word_after_non_ascii_text_is_found_by_its_byte_offset() {
        // `x` is byte 7 but char 4: indexing chars by the byte offset landed
        // on the space after `=` and found no word.
        let source = "ééé x = yy";
        assert_eq!(extract_word_at_offset(source, 7).as_deref(), Some("x"));
        assert_eq!(extract_word_at_offset(source, 12).as_deref(), Some("yy"));
        assert_eq!(extract_word_at_offset(source, 9), None);
    }

    #[test]
    fn character_counts_utf16_units() {
        // 😀 is 4 bytes, 2 UTF-16 units, 1 char.
        assert_eq!(position_to_offset("😀x", at(0, 2)), Some(4));
        assert_eq!(position_to_offset("éx", at(0, 1)), Some(2));
    }

    #[test]
    fn crlf_line_starts_do_not_drift() {
        let source = "a\r\nbb\r\ncc";
        assert_eq!(position_to_offset(source, at(1, 0)), Some(3));
        assert_eq!(position_to_offset(source, at(2, 1)), Some(8));
        // Past the end of a line clamps before its `\r\n`.
        assert_eq!(position_to_offset(source, at(1, 9)), Some(5));
        assert_eq!(position_to_offset(source, at(3, 0)), None);
    }

    #[test]
    fn a_range_ends_at_the_identifier_not_one_past_it() {
        // `foo` at column 5 (1-based), bytes 4..7.
        let range = lsp_range_from_span(Span::new(4, 7, 2, 5));
        assert_eq!(range.start, at(1, 4));
        assert_eq!(range.end, at(1, 7));
    }
    #[test]
    fn occurrences_are_identifiers_in_code_and_interpolations_only() {
        let source = "# greet in a comment\ndef greet(name)\n  \"greet: #{greet_count} #{name}\"\nend\n\nprint(greet(\"é\"))\nx = \"#{greet(1)} and greet\"\n";
        let found: Vec<&str> = identifier_occurrences(source, "greet")
            .into_iter()
            .map(|(start, end)| &source[start..end])
            .collect();
        // The declaration, the call, and the call inside an interpolation —
        // not the comment, not the plain words in strings, not `greet_count`.
        assert_eq!(found, vec!["greet", "greet", "greet"]);
        let positions: Vec<Position> = identifier_occurrences(source, "greet")
            .into_iter()
            .map(|(start, _)| offset_to_position(source, start))
            .collect();
        assert_eq!(positions, vec![at(1, 4), at(5, 6), at(6, 7)]);
    }

    #[test]
    fn a_position_after_non_ascii_text_counts_utf16_units() {
        let source = "x = \"é😀\"\ny";
        let y = source.rfind('y').unwrap();
        assert_eq!(offset_to_position(source, y), at(1, 0));
        let quote = source.rfind('"').unwrap();
        assert_eq!(offset_to_position(source, quote), at(0, 8));
    }
}
