//! Rename symbol provider for LSP.

use crate::lsp::symbols::SymbolTable;
use lsp_types::{Position, Range, TextEdit, WorkspaceEdit};

pub fn rename_symbol(
    source: &str,
    position: Position,
    new_name: &str,
    table: &SymbolTable,
) -> Option<WorkspaceEdit> {
    let offset = position_to_offset(source, position)?;

    let word = extract_word_at_offset(source, offset)?;

    let mut edits = Vec::new();

    for scoped in table.symbols.iter() {
        if scoped.symbol.name == word {
            edits.push(TextEdit {
                range: lsp_range_from_span(scoped.symbol.span),
                new_text: new_name.to_string(),
            });
        }
    }

    if edits.is_empty() {
        return None;
    }

    Some(WorkspaceEdit {
        changes: Some(std::collections::HashMap::from([(
            lsp_types::Url::from_file_path("")
                .unwrap_or_else(|_| lsp_types::Url::parse("file:///").unwrap()),
            edits,
        )])),
        change_annotations: None,
        document_changes: None,
    })
}

fn extract_word_at_offset(source: &str, offset: usize) -> Option<String> {
    let chars: Vec<char> = source.chars().collect();
    if offset >= chars.len() {
        return None;
    }

    let mut start = offset;
    let mut end = offset;

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
