//! Rename symbol provider for LSP.
use std::collections::HashMap;
use std::path::PathBuf;

use super::goto::ProjectScope;
use super::references::{ranges_by_document, word_and_scope, Request, Scope};
use tower_lsp_server::ls_types::{TextEdit, WorkspaceEdit};

/// Why a rename was refused, for the editor to show.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    NotAnIdentifier,
    /// Several classes declare a member with this name: `x.name` cannot be
    /// told apart, so renaming every one would rewrite the other classes'.
    AmbiguousMember(usize),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotAnIdentifier => write!(f, "not a valid Soli name"),
            Refusal::AmbiguousMember(count) => write!(
                f,
                "{count} classes declare a member with this name; rename them one at a time by hand"
            ),
        }
    }
}

/// Rename every use of the identifier under the cursor — the declaration and
/// its call sites, `#{…}` interpolations included: in its document for a
/// local, across the application (views' `<% %>` code too) for a name the
/// project declares.
pub fn rename_symbol(
    request: &Request<'_>,
    new_name: &str,
    project: ProjectScope<'_>,
    texts: impl FnOnce() -> Vec<(PathBuf, String)>,
) -> Result<Option<WorkspaceEdit>, Refusal> {
    if !is_identifier(new_name) {
        return Err(Refusal::NotAnIdentifier);
    }
    let Some((word, scope)) = word_and_scope(request, project) else {
        return Ok(None);
    };
    if let Scope::Project {
        member: true,
        declarations,
    } = scope
    {
        if declarations > 1 {
            return Err(Refusal::AmbiguousMember(declarations));
        }
    }
    let changes: HashMap<_, Vec<TextEdit>> = ranges_by_document(request, &word, scope, texts)
        .into_iter()
        .map(|(uri, ranges)| {
            let edits = ranges
                .into_iter()
                .map(|range| TextEdit {
                    range,
                    new_text: new_name.to_string(),
                })
                .collect();
            (uri, edits)
        })
        .collect();
    if changes.is_empty() {
        return Ok(None);
    }
    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        change_annotations: None,
        document_changes: None,
    }))
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
    use tower_lsp_server::ls_types::{Position, Uri};

    fn rename(
        source: &str,
        position: Position,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>, Refusal> {
        let uri: Uri = "file:///app/greet.sl".parse().unwrap();
        let request = Request {
            uri: &uri,
            source,
            position,
            template: false,
            table: None,
        };
        rename_symbol(&request, new_name, None, Vec::new)
    }

    #[test]
    fn renames_the_declaration_and_every_call() {
        let uri: Uri = "file:///app/greet.sl".parse().unwrap();
        let source =
            "def greet(name)\n  name\nend\n\nprint(greet(\"x\"))\ns = \"#{greet(\"y\")}\"\n";
        let edit = rename(source, Position::new(0, 5), "welcome")
            .unwrap()
            .unwrap();
        let edits = &edit.changes.unwrap()[&uri];
        assert_eq!(edits.len(), 3);
        assert!(edits.iter().all(|e| e.new_text == "welcome"));
    }

    #[test]
    fn refuses_a_name_that_is_not_an_identifier() {
        let source = "def greet\n  1\nend\n";
        assert_eq!(
            rename(source, Position::new(0, 5), "not a name"),
            Err(Refusal::NotAnIdentifier)
        );
        assert_eq!(
            rename(source, Position::new(0, 5), "1st"),
            Err(Refusal::NotAnIdentifier)
        );
        assert!(rename(source, Position::new(0, 5), "ready?")
            .unwrap()
            .is_some());
    }
}
