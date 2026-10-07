//! Hover provider for LSP.
use super::util::{extract_word_at_offset, position_to_offset};
use tower_lsp::lsp_types::{Hover, HoverContents, MarkedString, Position};

/// What the identifier under the cursor is: its declaration's kind or type when
/// the file declares it (at the `def` or at any use), and the builtin's doc for
/// a builtin. It used to answer only on a declaration itself — never on a call.
pub fn get_hover(source: &str, position: Position) -> Option<Hover> {
    let offset = position_to_offset(source, position)?;
    let word = extract_word_at_offset(source, offset)?;
    let table = crate::lsp::symbols::build_symbol_table(source);
    let declaration = table
        .as_ref()
        .and_then(|table| crate::lsp::goto::declaration_of(table, &word, offset));

    let mut contents = Vec::new();
    if let Some(symbol) = declaration {
        let type_info = match &symbol.symbol.type_name {
            Some(t) => format!("**{}** : {}", symbol.symbol.name, t),
            None => {
                let kind_str = match symbol.symbol.kind {
                    crate::lsp::symbols::SymbolKind::Variable => "variable",
                    crate::lsp::symbols::SymbolKind::Function => "function",
                    crate::lsp::symbols::SymbolKind::Class => "class",
                    crate::lsp::symbols::SymbolKind::Parameter => "parameter",
                    crate::lsp::symbols::SymbolKind::Property => "property",
                    crate::lsp::symbols::SymbolKind::Method => "method",
                    crate::lsp::symbols::SymbolKind::Constant => "constant",
                };
                format!("**{}** : {}", symbol.symbol.name, kind_str)
            }
        };
        contents.push(MarkedString::String(type_info));
    }
    // A name the file declares itself is that declaration, not the builtin.
    if declaration.is_none() {
        if let Some(docs) = get_builtin_docs(&word) {
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
        match get_hover(source, Position::new(line, character))?.contents {
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
}
