//! Hover provider for LSP.
use super::goto::{local_declaration, project_targets, ProjectScope};
use super::util::{doc_comment_above, offset_to_position, position_to_offset, word_bounds};
use crate::lsp::symbols::{SymbolKind, SymbolTable};
use tower_lsp_server::ls_types::{Hover, HoverContents, MarkedString, Position};

/// What the identifier under the cursor is: its declaration's kind or type when
/// the file declares it (at the `def` or at any use); else the project
/// declaration it names — its signature and the file it is in; else the
/// builtin's doc for a builtin.
///
/// `table` is the document's cached symbol table; without one (the buffer
/// never parsed) only the project and the builtins answer.
pub fn get_hover(
    source: &str,
    position: Position,
    table: Option<&SymbolTable>,
    project: ProjectScope<'_>,
) -> Option<Hover> {
    let offset = position_to_offset(source, position)?;
    let (word_start, word_end) = word_bounds(source, offset)?;
    let word = &source[word_start..word_end];
    let declaration = table.and_then(|table| local_declaration(table, source, word, word_start));

    let mut contents = Vec::new();
    if let Some(symbol) = declaration {
        let type_info = match &symbol.symbol.type_name {
            Some(t) => format!("**{}** : {}", symbol.symbol.name, t),
            None => {
                let kind_str = match symbol.symbol.kind {
                    SymbolKind::Variable => "variable",
                    SymbolKind::Function => "function",
                    SymbolKind::Class => "class",
                    SymbolKind::Parameter => "parameter",
                    SymbolKind::Property => "property",
                    SymbolKind::Method => "method",
                    SymbolKind::Constant => "constant",
                };
                format!("**{}** : {}", symbol.symbol.name, kind_str)
            }
        };
        let mut text = type_info;
        let documented = matches!(
            symbol.symbol.kind,
            SymbolKind::Function | SymbolKind::Method | SymbolKind::Class | SymbolKind::Constant
        );
        if documented {
            let line = offset_to_position(source, symbol.symbol.span.start_usize()).line;
            if let Some(doc) = doc_comment_above(source, line) {
                text.push_str(&format!("\n\n{doc}"));
            }
        }
        contents.push(MarkedString::String(text));
    } else if let Some((index, namespace)) = project {
        for symbol in project_targets(source, word_start, word_end, index, namespace) {
            let place = symbol
                .path
                .strip_prefix(&index.root)
                .unwrap_or(&symbol.path)
                .to_string_lossy()
                .replace('\\', "/");
            let owner = symbol
                .owner
                .as_ref()
                .map(|owner| format!(" of {owner}"))
                .unwrap_or_default();
            let mut text = format!("**{}** : {}{owner}", symbol.name, symbol.kind.label());
            if let Some(signature) = &symbol.signature {
                text.push_str(&format!("\n\n```soli\n{signature}\n```"));
            }
            if let Some(doc) = &symbol.doc {
                text.push_str(&format!("\n\n{doc}"));
            }
            text.push_str(&format!("\n\n{place}:{}", symbol.range.start.line + 1));
            contents.push(MarkedString::String(text));
        }
    }
    // A name the project declares is that declaration, not the builtin.
    if contents.is_empty() {
        if let Some(docs) = get_builtin_docs(word) {
            contents.push(MarkedString::String(docs));
        }
    }
    if contents.is_empty() {
        return None;
    }
    Some(Hover {
        contents: HoverContents::Array(contents),
        range: None,
    })
}

fn get_builtin_docs(name: &str) -> Option<String> {
    let docs = match name {
        "print" | "println" => "Prints values to stdout.\n\n```\nprint(value: Any): Void\n```",
        "input" => "Reads a line of input from stdin.\n\n```\ninput(prompt: String?): String\n```",
        "len" => "Returns the length of an array, string, or hash.\n\n```\nlen(collection: Array|String|Hash): Int\n```",
        "str" => "Converts a value to its string representation.\n\n```\nstr(value: Any): String\n```",
        "int" => "Converts a value to an integer.\n\n```\nint(value: Any): Int\n```",
        "float" => "Converts a value to a float.\n\n```\nfloat(value: Any): Float\n```",
        "type" => "Returns the type name of a value.\n\n```\ntype(value: Any): String\n```",
        "clock" => "Returns the current time in seconds since Unix epoch.\n\n```\nclock(): Float\n```",
        "range" => "Creates a range of integers.\n\n```\nrange(start: Int, end: Int): Array<Int>\n```",
        "has_key" => "Checks if a hash contains a key.\n\n```\nhas_key(hash: Hash, key: Any): Bool\n```",
        "delete" => "Deletes a key from a hash.\n\n```\ndelete(hash: Hash, key: Any): Any\n```",
        "json_parse" => "Parses a JSON string.\n\n```\njson_parse(json: String): Any\n```",
        "json_stringify" => "Converts a value to JSON.\n\n```\njson_stringify(value: Any): String\n```",
        "HTTP" => "HTTP client class.\n\n```\nHTTP.get(url, options?)\nHTTP.post(url, body, options?)\nHTTP.put / HTTP.patch / HTTP.delete / HTTP.head\nHTTP.get_json / HTTP.post_json / HTTP.put_json / HTTP.patch_json\nHTTP.request(method, url, options?)\nHTTP.get_all(urls) / HTTP.parallel(requests)\n```",
        "DateTime" => "DateTime class for date and time manipulation.\n\n```\nDateTime.now(): DateTime\nDateTime.parse(s: String): DateTime\nDateTime.from_unix(ts: Int): DateTime\n```",
        "Duration" => "Duration class for time differences.\n\n```\nDuration.between(start: DateTime, end: DateTime): Duration\nDuration.of_seconds(s: Float): Duration\nDuration.of_minutes(m: Float): Duration\n```",
        "Regex" => "Regex class for pattern matching.\n\n```\nRegex.matches(pattern: String, string: String): Bool\nRegex.find(pattern: String, string: String): Any\nRegex.replace(pattern: String, string: String, replacement: String): String\n```",
        "JSON" => "JSON class for JSON operations.\n\n```\nJSON.parse(json: String): Any\nJSON.stringify(value: Any): String\n```",
        _ => return None,
    };

    Some(docs.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hover_text(source: &str, line: u32, character: u32) -> Option<String> {
        let table = crate::lsp::symbols::build_symbol_table(source);
        match get_hover(source, Position::new(line, character), table.as_ref(), None)?.contents {
            HoverContents::Array(items) => Some(
                items
                    .into_iter()
                    .map(|item| match item {
                        MarkedString::String(text) => text,
                        MarkedString::LanguageString(code) => code.value,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        }
    }

    #[test]
    fn a_call_shows_what_it_calls_and_a_builtin_shows_its_doc() {
        let source = "def greet(name)\n  name\nend\n\nprint(greet(\"x\"))\n";
        assert_eq!(
            hover_text(source, 4, 7).as_deref(),
            Some("**greet** : function")
        );
        assert!(hover_text(source, 4, 1)
            .unwrap()
            .contains("Prints values to stdout"));
        assert!(hover_text(source, 1, 3).unwrap().contains("**name**"));
    }

    #[test]
    fn a_def_shows_the_comment_written_above_it() {
        let source = "x = 1\n\n# Greets someone.\n#\n# Returns the greeting.\ndef greet(name)\n  name\nend\n\nprint(greet(\"x\"))\n";
        assert_eq!(
            hover_text(source, 9, 7).as_deref(),
            Some("**greet** : function\n\nGreets someone.\n\nReturns the greeting.")
        );
        // A blank line ends the block: the comment is not about the `def`.
        let source = "# not about greet\n\ndef greet\n  1\nend\n\nprint(greet)\n";
        assert_eq!(
            hover_text(source, 6, 7).as_deref(),
            Some("**greet** : function")
        );
    }
}
