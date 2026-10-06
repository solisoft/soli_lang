//! Rename symbol provider for LSP.
use super::util::{extract_word_at_offset, lsp_range_from_span, position_to_offset};
use crate::lsp::symbols::SymbolTable;
use tower_lsp::lsp_types::{Position, TextEdit, WorkspaceEdit};

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
            tower_lsp::lsp_types::Url::from_file_path("")
                .unwrap_or_else(|_| tower_lsp::lsp_types::Url::parse("file:///").unwrap()),
            edits,
        )])),
        change_annotations: None,
        document_changes: None,
    })
}
