//! Go-to definition provider for LSP.
use super::util::{extract_word_at_offset, lsp_range_from_span, position_to_offset};
use crate::lsp::symbols::SymbolTable;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Url};

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
