//! Rename symbol provider for LSP.
use super::util::{extract_word_at_offset, identifier_occurrences, position_to_offset, range_of};
use tower_lsp::lsp_types::{Position, TextEdit, Url, WorkspaceEdit};

/// Rename every use of the identifier under the cursor in this document —
/// the declaration and its call sites, `#{…}` interpolations included. It used
/// to rename the declaration only, which left every call broken.
pub fn rename_symbol(
    source: &str,
    position: Position,
    new_name: &str,
    uri: &Url,
) -> Option<WorkspaceEdit> {
    if !is_identifier(new_name) {
        return None;
    }
    let offset = position_to_offset(source, position)?;
    let word = extract_word_at_offset(source, offset)?;
    let edits: Vec<TextEdit> = identifier_occurrences(source, &word)
        .into_iter()
        .map(|(start, end)| TextEdit {
            range: range_of(source, start, end),
            new_text: new_name.to_string(),
        })
        .collect();
    if edits.is_empty() {
        return None;
    }
    Some(WorkspaceEdit {
        changes: Some(std::collections::HashMap::from([(uri.clone(), edits)])),
        change_annotations: None,
        document_changes: None,
    })
}

/// A name the rename may write: letters, digits and `_`, not starting with a
/// digit, optionally ending in `?` or `!` like `valid?` / `save!`.
fn is_identifier(name: &str) -> bool {
    let body = name.strip_suffix(['?', '!']).unwrap_or(name);
    let mut chars = body.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_the_declaration_and_every_call() {
        let uri = Url::parse("file:///app/greet.sl").unwrap();
        let source =
            "def greet(name)\n  name\nend\n\nprint(greet(\"x\"))\ns = \"#{greet(\"y\")}\"\n";
        let edit = rename_symbol(source, Position::new(0, 5), "welcome", &uri).unwrap();
        let edits = &edit.changes.unwrap()[&uri];
        assert_eq!(edits.len(), 3);
        assert!(edits.iter().all(|e| e.new_text == "welcome"));
    }

    #[test]
    fn refuses_a_name_that_is_not_an_identifier() {
        let uri = Url::parse("file:///app/greet.sl").unwrap();
        let source = "def greet\n  1\nend\n";
        assert!(rename_symbol(source, Position::new(0, 5), "not a name", &uri).is_none());
        assert!(rename_symbol(source, Position::new(0, 5), "1st", &uri).is_none());
        assert!(rename_symbol(source, Position::new(0, 5), "ready?", &uri).is_some());
    }
}
