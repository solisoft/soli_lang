//! Position helpers shared by the LSP providers.

use crate::span::Span;
use tower_lsp::lsp_types::{Position, Range};

pub(super) fn extract_word_at_offset(source: &str, offset: usize) -> Option<String> {
    let mut start = offset;
    let mut end = offset;

    let chars: Vec<char> = source.chars().collect();
    if start >= chars.len() {
        return None;
    }

    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }

    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }

    if start == end {
        return None;
    }

    Some(chars[start..end].iter().collect())
}

pub(super) fn position_to_offset(source: &str, position: Position) -> Option<usize> {
    let mut offset = 0;

    for (line, line_str) in source.lines().enumerate() {
        if line as u32 == position.line {
            let col = position.character as usize;
            for (char_offset, (i, _)) in line_str.char_indices().enumerate() {
                if char_offset >= col {
                    return Some(offset + i);
                }
            }
            return Some(offset + line_str.len().min(col));
        }
        offset += line_str.len() + 1;
    }
    None
}

pub(super) fn lsp_range_from_span(span: Span) -> Range {
    Range {
        start: Position {
            line: span.line.saturating_sub(1),
            character: span.column.saturating_sub(1),
        },
        end: Position {
            line: span.line.saturating_sub(1),
            character: span.column + (span.end - span.start),
        },
    }
}
