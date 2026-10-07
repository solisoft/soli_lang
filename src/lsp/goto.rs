//! Go-to definition provider for LSP.
use super::util::{extract_word_at_offset, identifier_occurrences, position_to_offset, range_of};
use crate::lsp::symbols::{ScopedSymbol, SymbolTable};
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Url};

/// Where the identifier under the cursor is declared in this document (`uri`).
///
/// The cursor's *word* decides, not the symbol whose span contains the cursor:
/// a function's span covers its whole body, so the old lookup sent every
/// position inside a function to that function's own `def`.
pub fn goto_definition(
    source: &str,
    position: Position,
    table: &SymbolTable,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    let offset = position_to_offset(source, position)?;
    let word = extract_word_at_offset(source, offset)?;
    let declaration = declaration_of(table, &word, offset)?;
    let (start, end) = name_in_declaration(source, declaration, &word);
    Some(GotoDefinitionResponse::Scalar(Location {
        uri: uri.clone(),
        range: range_of(source, start, end),
    }))
}

/// The declaration of `name` visible at `offset`: the innermost one whose scope
/// contains it, else the first one in the file (a call before its `def`).
pub(super) fn declaration_of<'a>(
    table: &'a SymbolTable,
    name: &str,
    offset: usize,
) -> Option<&'a ScopedSymbol> {
    let candidates = table.find_by_name(name);
    candidates
        .iter()
        .filter(|s| s.scope_start <= offset && offset <= s.scope_end)
        .max_by_key(|s| s.scope_start)
        .or_else(|| candidates.first())
        .copied()
}

/// The byte range of the name itself inside a declaration's span (`def greet(…)`
/// → `greet`), so the editor lands on the name, not the start of the line.
fn name_in_declaration(source: &str, declaration: &ScopedSymbol, name: &str) -> (usize, usize) {
    let span = declaration.symbol.span;
    let (start, end) = (span.start_usize(), span.end_usize());
    identifier_occurrences(source, name)
        .into_iter()
        .find(|&(s, e)| s >= start && e <= end)
        .unwrap_or((start, start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::symbols::build_symbol_table;

    fn target(source: &str, line: u32, character: u32) -> Option<(Url, Position)> {
        let uri = Url::parse("file:///app/greet.sl").unwrap();
        let table = build_symbol_table(source)?;
        match goto_definition(source, Position::new(line, character), &table, &uri)? {
            GotoDefinitionResponse::Scalar(location) => Some((location.uri, location.range.start)),
            _ => None,
        }
    }

    #[test]
    fn a_call_jumps_to_the_name_in_its_def_in_this_document() {
        let source = "def greet(name)\n  name\nend\n\nprint(greet(\"x\"))\n";
        let (uri, start) = target(source, 4, 7).unwrap();
        assert_eq!(uri.as_str(), "file:///app/greet.sl");
        assert_eq!(start, Position::new(0, 4));
    }

    #[test]
    fn a_call_inside_another_function_jumps_to_the_callee_not_the_enclosing_def() {
        let source = "def helper\n  1\nend\n\ndef main\n  helper()\nend\n";
        let (_, start) = target(source, 5, 3).unwrap();
        assert_eq!(start, Position::new(0, 4));
    }
}
