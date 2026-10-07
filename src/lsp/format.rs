//! Document formatting provider for LSP.
use tower_lsp::lsp_types::{Position, Range, TextEdit};

/// Format the whole document with `soli fmt`'s formatter, as one edit.
///
/// This used to be a re-indenter of its own that counted only `{ } ( ) [ ]`:
/// on `def … end` code — the house style — it moved every body line to column
/// 0, so format-on-save flattened the file. A file that does not parse is left
/// alone, as `soli fmt` leaves it.
pub fn format_document(source: &str) -> Vec<TextEdit> {
    let Ok(formatted) = crate::fmt::format_source(source) else {
        return Vec::new();
    };
    if formatted == source {
        return Vec::new();
    }
    vec![TextEdit {
        range: Range {
            start: Position::new(0, 0),
            end: super::util::offset_to_position(source, source.len()),
        },
        new_text: formatted,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_keeps_a_def_body_indented() {
        let source = "def greet(name)\n  \"Hello #{name}\"\nend\n";
        assert!(
            format_document(source).is_empty(),
            "already formatted: no edit"
        );
        let messy = "def greet(name)\n\"Hello #{name}\"\nend\n";
        let edits = format_document(messy);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, source);
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        assert!(format_document("def broken(x\n  x +\nend\n").is_empty());
    }
}
