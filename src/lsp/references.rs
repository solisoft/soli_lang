//! Find references provider for LSP.
use super::util::{extract_word_at_offset, identifier_occurrences, position_to_offset, range_of};
use tower_lsp::lsp_types::{Location, Position, Url};

/// Every use of the identifier under the cursor in this document — declaration
/// and call sites alike, `#{…}` interpolations included, at `uri`.
pub fn find_references(source: &str, position: Position, uri: &Url) -> Option<Vec<Location>> {
    let offset = position_to_offset(source, position)?;
    let word = extract_word_at_offset(source, offset)?;
    let references: Vec<Location> = identifier_occurrences(source, &word)
        .into_iter()
        .map(|(start, end)| Location {
            uri: uri.clone(),
            range: range_of(source, start, end),
        })
        .collect();
    (!references.is_empty()).then_some(references)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_site_and_the_declaration_are_both_references_in_this_document() {
        let uri = Url::parse("file:///app/greet.sl").unwrap();
        let source = "def greet(name)\n  name\nend\n\nprint(greet(\"x\"))\n";
        let found = find_references(source, Position::new(4, 7), &uri).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|location| location.uri == uri));
        assert_eq!(found[0].range.start, Position::new(0, 4));
        assert_eq!(found[1].range.start, Position::new(4, 6));
    }
}
