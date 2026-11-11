//! Find references provider for LSP, and the occurrence search rename shares.
//!
//! A name the project declares — a class, a top-level `def`, a constant, a
//! member — is looked up in every file of the application (open buffers as the
//! editor has them, the rest from disk), views included: in a `.slv` only the
//! code inside `<% %>` tags counts. A local variable or a parameter stays in
//! its document.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::goto::{local_declaration, project_targets, ProjectScope};
use super::project::IndexedKind;
use super::util::{identifier_occurrences, position_to_offset, range_of, word_bounds};
use crate::lsp::symbols::{SymbolKind, SymbolTable};
use tower_lsp_server::ls_types::{Location, Position, Range, Uri};

/// Where the occurrences of a name are looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// This document only: a local, a parameter, or a name the project does
    /// not declare.
    Document,
    /// Every file of the application.
    Project {
        /// Only members (methods, fields, scopes, relations) answer to the
        /// name: two classes may each declare one, so a rename cannot tell
        /// whose call a `x.name` is.
        member: bool,
        /// How many project declarations carry the name.
        declarations: usize,
    },
}

/// The document a request comes from.
pub struct Request<'a> {
    pub uri: &'a Uri,
    pub source: &'a str,
    pub position: Position,
    /// A `.slv` view: only its `<% %>` code is Soli.
    pub template: bool,
    pub table: Option<&'a SymbolTable>,
}

/// The word under the cursor, and where its occurrences live.
pub fn word_and_scope(request: &Request<'_>, project: ProjectScope<'_>) -> Option<(String, Scope)> {
    let offset = position_to_offset(request.source, request.position)?;
    let (start, end) = word_bounds(request.source, offset)?;
    let word = request.source[start..end].to_string();
    let Some((index, namespace)) = project else {
        return Some((word, Scope::Document));
    };
    if index.loose {
        return Some((word, Scope::Document));
    }
    let local = request
        .table
        .and_then(|table| local_declaration(table, request.source, &word, start));
    if let Some(local) = local {
        let declared_here = matches!(
            local.symbol.kind,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::Class
                | SymbolKind::Constant
                | SymbolKind::Property
        );
        // A file outside the auto-loaded directories (a script, a test) keeps
        // its own top-level names to itself.
        let shared = namespace.is_some();
        if !declared_here || !shared {
            return Some((word, Scope::Document));
        }
        let member = matches!(local.symbol.kind, SymbolKind::Method | SymbolKind::Property);
        let declarations = if member {
            index.members_named(&word).len()
        } else {
            index.top_level(&word, namespace).len()
        };
        return Some((
            word,
            Scope::Project {
                member,
                declarations: declarations.max(1),
            },
        ));
    }
    let targets = project_targets(request.source, start, end, index, namespace);
    if targets.is_empty() {
        return Some((word, Scope::Document));
    }
    let member = targets.iter().all(|symbol| {
        symbol.owner.is_some() && !matches!(symbol.kind, IndexedKind::Class | IndexedKind::Module)
    });
    let declarations = if member {
        index.members_named(&word).len()
    } else {
        targets.len()
    };
    Some((
        word,
        Scope::Project {
            member,
            declarations,
        },
    ))
}

/// The byte ranges of `name` in `source`; in a template, only inside its
/// `<% %>` tags.
pub fn occurrences(source: &str, name: &str, template: bool) -> Vec<(usize, usize)> {
    if !template {
        return identifier_occurrences(source, name);
    }
    template_code_regions(source)
        .into_iter()
        .flat_map(|(start, end)| {
            identifier_occurrences(&source[start..end], name)
                .into_iter()
                .map(move |(s, e)| (start + s, start + e))
        })
        .collect()
}

/// The code of each `<% %>` tag of a template: after the `<%` and its `=` /
/// `-` marker, before the `-%>` / `%>`. A `<%#` comment and a `<%%` escape
/// hold no code. An unclosed tag runs to the end.
pub fn template_code_regions(source: &str) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    let mut from = 0;
    while let Some(open) = source[from..].find("<%").map(|i| from + i) {
        let mut start = open + 2;
        let rest = &source[start..];
        let skip = rest.starts_with('#') || rest.starts_with('%');
        if rest.starts_with('=') || rest.starts_with('-') {
            start += 1;
        }
        let close = source[start..]
            .find("%>")
            .map_or(source.len(), |i| start + i);
        let end = if source[..close].ends_with('-') && close > start {
            close - 1
        } else {
            close
        };
        if !skip && end > start {
            regions.push((start, end));
        }
        from = (close + 2).min(source.len());
        if close >= source.len() {
            break;
        }
    }
    regions
}

/// The text of every file in the search: the open buffer when the editor
/// holds one, else the file on disk. Files that cannot be read are left out.
pub fn project_texts(
    files: Vec<PathBuf>,
    open: &HashMap<PathBuf, String>,
) -> Vec<(PathBuf, String)> {
    let mut texts: Vec<(PathBuf, String)> = files
        .into_iter()
        .filter_map(|path| {
            let text = match open.get(&path) {
                Some(text) => text.clone(),
                None => std::fs::read_to_string(&path).ok()?,
            };
            Some((path, text))
        })
        .collect();
    // An open buffer not on disk yet (or outside the walked directories) is
    // still searched.
    for (path, text) in open {
        if !texts.iter().any(|(known, _)| known == path) {
            texts.push((path.clone(), text.clone()));
        }
    }
    texts
}

fn is_template_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "slv")
}

/// The range of every occurrence of `word` in the request's scope, grouped
/// by document.
pub fn ranges_by_document(
    request: &Request<'_>,
    word: &str,
    scope: Scope,
    texts: impl FnOnce() -> Vec<(PathBuf, String)>,
) -> Vec<(Uri, Vec<Range>)> {
    let ranges = |text: &str, template: bool| -> Vec<Range> {
        occurrences(text, word, template)
            .into_iter()
            .map(|(start, end)| range_of(text, start, end))
            .collect()
    };
    let documents = match scope {
        Scope::Document => vec![(
            request.uri.clone(),
            ranges(request.source, request.template),
        )],
        Scope::Project { .. } => texts()
            .into_iter()
            .filter_map(|(path, text)| {
                let uri = Uri::from_file_path(&path)?;
                Some((uri, ranges(&text, is_template_path(&path))))
            })
            .collect(),
    };
    documents
        .into_iter()
        .filter(|(_, found)| !found.is_empty())
        .collect()
}

/// Every use of the identifier under the cursor: in its document for a local,
/// across the application for a name the project declares.
pub fn find_references(
    request: &Request<'_>,
    project: ProjectScope<'_>,
    texts: impl FnOnce() -> Vec<(PathBuf, String)>,
) -> Option<Vec<Location>> {
    let (word, scope) = word_and_scope(request, project)?;
    let references: Vec<Location> = ranges_by_document(request, &word, scope, texts)
        .into_iter()
        .flat_map(|(uri, found)| {
            found.into_iter().map(move |range| Location {
                uri: uri.clone(),
                range,
            })
        })
        .collect();
    (!references.is_empty()).then_some(references)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(uri: &'a Uri, source: &'a str, position: Position) -> Request<'a> {
        Request {
            uri,
            source,
            position,
            template: false,
            table: None,
        }
    }

    #[test]
    fn a_call_site_and_the_declaration_are_both_references_in_this_document() {
        let uri = "file:///app/greet.sl".parse().unwrap();
        let source = "def greet(name)\n  name\nend\n\nprint(greet(\"x\"))\n";
        let found =
            find_references(&request(&uri, source, Position::new(4, 7)), None, Vec::new).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|location| location.uri == uri));
        assert_eq!(found[0].range.start, Position::new(0, 4));
        assert_eq!(found[1].range.start, Position::new(4, 6));
    }

    #[test]
    fn a_template_holds_code_only_inside_its_tags() {
        let source = "<p>post title</p>\n<%= post.title %> <%# post %> <%- f.post -%>\n<% post";
        let found: Vec<&str> = occurrences(source, "post", true)
            .into_iter()
            .map(|(s, e)| &source[s..e])
            .collect();
        assert_eq!(found, ["post", "post", "post"]);
        let starts: Vec<usize> = occurrences(source, "post", true)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        // The markup's "post" (offset 3) and the comment tag's are not code.
        assert!(!starts.contains(&3));
    }
}
