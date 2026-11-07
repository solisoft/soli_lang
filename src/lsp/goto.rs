//! Go-to definition provider for LSP.
use super::project::{IndexedSymbol, Namespace, ProjectIndex};
use super::receiver::{member_access, Receiver};
use super::util::{identifier_occurrences, position_to_offset, range_of, word_bounds};
use crate::lsp::symbols::{ScopedSymbol, SymbolKind, SymbolTable};
use tower_lsp_server::ls_types::{GotoDefinitionResponse, Location, Position, Uri};

/// The project and the namespace of the file asking (see
/// [`ProjectIndex::top_level`]).
pub type ProjectScope<'a> = Option<(&'a ProjectIndex, Option<Namespace>)>;

/// Where the identifier under the cursor is declared: in this document
/// (`uri`) first, then anywhere in the project — `Post` in a controller lands
/// on `class Post` in `app/models/post.sl`, `post.summary` on the `def` of
/// the class the receiver resolves to. A name declared in several places
/// answers with every one.
///
/// The cursor's *word* decides, not the symbol whose span contains the cursor:
/// a function's span covers its whole body, so the old lookup sent every
/// position inside a function to that function's own `def`.
pub fn goto_definition(
    source: &str,
    position: Position,
    table: Option<&SymbolTable>,
    uri: &Uri,
    project: ProjectScope<'_>,
) -> Option<GotoDefinitionResponse> {
    let offset = position_to_offset(source, position)?;
    let (word_start, word_end) = word_bounds(source, offset)?;
    let word = &source[word_start..word_end];
    if let Some(declaration) =
        table.and_then(|table| local_declaration(table, source, word, word_start))
    {
        let (start, end) = name_in_declaration(source, declaration, word);
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: range_of(source, start, end),
        }));
    }
    let (index, namespace) = project?;
    let locations: Vec<Location> = project_targets(source, word_start, word_end, index, namespace)
        .into_iter()
        .filter_map(|symbol| {
            Some(Location {
                uri: Uri::from_file_path(&symbol.path)?,
                range: symbol.range,
            })
        })
        .collect();
    match locations.len() {
        0 => None,
        1 => locations
            .into_iter()
            .next()
            .map(GotoDefinitionResponse::Scalar),
        _ => Some(GotoDefinitionResponse::Array(locations)),
    }
}

/// The declaration in this file of the word at `word_start`. After a `.` only
/// a method or a field counts: `post.title` is not the local `title`.
pub(super) fn local_declaration<'a>(
    table: &'a SymbolTable,
    source: &str,
    word: &str,
    word_start: usize,
) -> Option<&'a ScopedSymbol> {
    let declaration = declaration_of(table, word, word_start)?;
    let after_dot = source[..word_start].ends_with('.') || source[..word_start].ends_with('@');
    if after_dot
        && !matches!(
            declaration.symbol.kind,
            SymbolKind::Method | SymbolKind::Property
        )
    {
        return None;
    }
    Some(declaration)
}

/// The project declarations the word at `word_start..word_end` refers to: a
/// member of the receiver's class (or, the receiver unresolved, every member
/// of that name) after a `.` or `@`, else a top-level name in reach.
pub(super) fn project_targets<'a>(
    source: &str,
    word_start: usize,
    word_end: usize,
    index: &'a ProjectIndex,
    namespace: Option<Namespace>,
) -> Vec<&'a IndexedSymbol> {
    let word = &source[word_start..word_end];
    let Some(access) = member_access(source, word_end) else {
        return index.top_level(word, namespace);
    };
    match &access.receiver {
        Receiver::Class(class)
        | Receiver::Instance(class)
        | Receiver::SelfInstance(Some(class))
            if index.class(class).is_some() =>
        {
            index.member(class, word)
        }
        Receiver::Builtin(_) => Vec::new(),
        _ => index.members_named(word),
    }
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

    fn target(source: &str, line: u32, character: u32) -> Option<(Uri, Position)> {
        let uri = "file:///app/greet.sl".parse().unwrap();
        let table = build_symbol_table(source)?;
        match goto_definition(
            source,
            Position::new(line, character),
            Some(&table),
            &uri,
            None,
        )? {
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
