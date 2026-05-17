//! Go-to definition provider for LSP.

use crate::lsp::symbols::SymbolTable;
use lsp_types::{GotoDefinitionResponse, Location, Position, Range, Url};

pub fn goto_definition(
    source: &str,
    position: Position,
    table: &SymbolTable,
) -> Option<GotoDefinitionResponse> {
    let offset = position_to_offset(source, position)?;

    if let Some(scoped) = table.find_at_position(offset) {
        let definition = Location {
            uri: Url::from_file_path("").unwrap_or_else(|_| Url::parse("file:///").unwrap()),
            range: lsp_range_from_span(scoped.symbol.span),
        };
        return Some(GotoDefinitionResponse::Scalar(definition));
    }

    let word = extract_word_at_offset(source, offset)?;
    let symbols = table.find_by_name(&word);

    if let Some(first) = symbols.first() {
        let definition = Location {
            uri: Url::from_file_path("").unwrap_or_else(|_| Url::parse("file:///").unwrap()),
            range: lsp_range_from_span(first.symbol.span),
        };
        return Some(GotoDefinitionResponse::Scalar(definition));
    }

    None
}

pub fn goto_type_definition(
    source: &str,
    position: Position,
    table: &SymbolTable,
) -> Option<GotoDefinitionResponse> {
    let offset = position_to_offset(source, position)?;

    if let Some(scoped) = table.find_at_position(offset) {
        if let Some(type_name) = &scoped.symbol.type_name {
            let type_symbols = table.find_by_name(type_name);
            if let Some(first) = type_symbols.first() {
                let definition = Location {
                    uri: Url::from_file_path("")
                        .unwrap_or_else(|_| Url::parse("file:///").unwrap()),
                    range: lsp_range_from_span(first.symbol.span),
                };
                return Some(GotoDefinitionResponse::Scalar(definition));
            }
        }
    }

    None
}

fn extract_word_at_offset(source: &str, offset: usize) -> Option<String> {
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

fn position_to_offset(source: &str, position: Position) -> Option<usize> {
    let mut line = 0;
    let mut offset = 0;

    for line_str in source.lines() {
        if line == position.line {
            let col = position.character as usize;
            let mut char_offset = 0;
            for (i, c) in line_str.char_indices() {
                if char_offset >= col {
                    return Some(offset + i);
                }
                char_offset += 1;
            }
            return Some(offset + line_str.len().min(col));
        }
        line += 1;
        offset += line_str.len() + 1;
    }
    None
}

fn lsp_range_from_span(span: crate::span::Span) -> Range {
    Range {
        start: Position {
            line: (span.line.saturating_sub(1)) as u32,
            character: (span.column.saturating_sub(1)) as u32,
        },
        end: Position {
            line: (span.line.saturating_sub(1)) as u32,
            character: ((span.column + (span.end - span.start)) as u32),
        },
    }
}
