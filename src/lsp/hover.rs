//! Hover provider for LSP.

use crate::lsp::symbols::SymbolTable;
use crate::span::Span;
use lsp_types::{Hover, HoverContents, HoverParams, MarkedString, Position, Range};

pub fn get_hover(source: &str, position: Position) -> Option<Hover> {
    let offset = position_to_offset(source, position)?;
    let table = crate::lsp::symbols::build_symbol_table(source)?;

    let symbol = table.find_at_position(offset)?;

    let mut contents = Vec::new();

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

    if let Some(docs) = get_builtin_docs(&symbol.symbol.name) {
        contents.push(MarkedString::String(docs));
    }

    Some(Hover {
        contents: HoverContents::Array(contents),
        range: Some(lsp_range_from_span(symbol.symbol.span)),
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
        "abs" => "Returns the absolute value.\n\n```\nabs(n: Int|Float): Int|Float\n```",
        "min" => "Returns the minimum of two values.\n\n```\nmin(a: Any, b: Any): Any\n```",
        "max" => "Returns the maximum of two values.\n\n```\nmax(a: Any, b: Any): Any\n```",
        "pow" => "Returns base raised to the power of exponent.\n\n```\npow(base: Any, exp: Any): Any\n```",
        "sqrt" => "Returns the square root.\n\n```\nsqrt(n: Any): Float\n```",
        "push" => "Appends an element to an array.\n\n```\npush(array: Array, element: Any): Void\n```",
        "pop" => "Removes and returns the last element of an array.\n\n```\npop(array: Array): Any\n```",
        "keys" => "Returns the keys of a hash.\n\n```\nkeys(hash: Hash): Array\n```",
        "values" => "Returns the values of a hash.\n\n```\nvalues(hash: Hash): Array\n```",
        "has_key" => "Checks if a hash contains a key.\n\n```\nhas_key(hash: Hash, key: Any): Bool\n```",
        "delete" => "Deletes a key from a hash.\n\n```\ndelete(hash: Hash, key: Any): Any\n```",
        "merge" => "Merges two hashes.\n\n```\nmerge(hash1: Hash, hash2: Hash): Hash\n```",
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

fn position_to_offset(source: &str, position: Position) -> Option<usize> {
    let mut line = 0;
    let mut offset = 0;

    for line_str in source.lines() {
        if line == position.line {
            let col = position.character as usize;
            let mut char_offset = 0;
            for (i, c) in line_str.char_indices() {
                if char_offset >= col {
                    return Some(offset + i);
                }
                char_offset += 1;
            }
            return Some(offset + line_str.len().min(col));
        }
        line += 1;
        offset += line_str.len() + 1;
    }
    None
}

fn lsp_range_from_span(span: Span) -> Range {
    Range {
        start: Position {
            line: (span.line.saturating_sub(1)) as u32,
            character: (span.column.saturating_sub(1)) as u32,
        },
        end: Position {
            line: (span.line.saturating_sub(1)) as u32,
            character: ((span.column + (span.end - span.start)) as u32),
        },
    }
}
