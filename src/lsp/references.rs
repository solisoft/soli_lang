//! Find references provider for LSP.
use super::util::{extract_word_at_offset, lsp_range_from_span, position_to_offset};
use crate::lsp::symbols::SymbolTable;
use tower_lsp::lsp_types::{Location, Position, Url};

pub fn find_references(
    source: &str,
    position: Position,
    table: &SymbolTable,
) -> Option<Vec<Location>> {
    let offset = position_to_offset(source, position)?;

    let word = extract_word_at_offset(source, offset)?;

    let mut references = Vec::new();

    for scoped in table.symbols.iter() {
        if scoped.symbol.name == word {
            references.push(Location {
                uri: Url::from_file_path("").unwrap_or_else(|_| Url::parse("file:///").unwrap()),
                range: lsp_range_from_span(scoped.symbol.span),
            });
        }
    }

    if references.is_empty() {
        None
    } else {
        Some(references)
    }
}
