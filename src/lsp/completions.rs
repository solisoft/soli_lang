//! Completions provider for LSP.
use crate::lsp::symbols::SymbolTable;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, CompletionResponse, Position};

const KEYWORDS: &[&str] = &[
    "let",
    "var",
    "const",
    "fn",
    "def",
    "class",
    "if",
    "else",
    "elif",
    "while",
    "for",
    "in",
    "return",
    "break",
    "continue",
    "next",
    "match",
    "case",
    "default",
    "try",
    "catch",
    "throw",
    "import",
    "from",
    "as",
    "new",
    "self",
    "super",
    "true",
    "false",
    "nil",
    "and",
    "or",
    "not",
    "is",
    "isnt",
    "yield",
    "async",
    "await",
    "static",
    "public",
    "private",
    "protected",
    "extends",
    "implements",
    "interface",
    "trait",
    "module",
    "describe",
    "context",
    "test",
    "it",
    "specify",
    "before_each",
    "after_each",
    "before_all",
    "after_all",
    "expect",
    "to",
    "eq",
    "be",
    "a",
    "an",
    "have",
];

const TYPES: &[&str] = &[
    "Int", "Float", "String", "Bool", "Void", "Any", "Null", "Array", "Hash", "Object", "Number",
    "Function", "Class", "Module", "Result", "Option", "Future", "Promise", "DateTime", "Duration",
    "Regex", "JSON", "I18n", "HTTP", "Cache", "KV",
];

pub fn get_completions(
    source: &str,
    position: Position,
    table: Option<&SymbolTable>,
) -> Option<CompletionResponse> {
    let offset = position_to_offset(source, position)?;
    let (leading, _) = split_at_offset(source, offset);

    let prefix = extract_word_prefix(leading);
    if prefix.is_empty() {
        return None;
    }

    let mut items = Vec::new();

    for keyword in KEYWORDS.iter().filter(|k| k.starts_with(&prefix)) {
        items.push(CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            insert_text: Some(keyword.to_string()),
            ..Default::default()
        });
    }

    for type_name in TYPES.iter().filter(|t| t.starts_with(&prefix)) {
        items.push(CompletionItem {
            label: type_name.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            insert_text: Some(type_name.to_string()),
            ..Default::default()
        });
    }

    if let Some(table) = table {
        for scoped in table
            .symbols
            .iter()
            .filter(|s| s.symbol.name.starts_with(&prefix))
        {
            let kind = match scoped.symbol.kind {
                crate::lsp::symbols::SymbolKind::Variable => CompletionItemKind::VARIABLE,
                crate::lsp::symbols::SymbolKind::Function => CompletionItemKind::FUNCTION,
                crate::lsp::symbols::SymbolKind::Class => CompletionItemKind::CLASS,
                crate::lsp::symbols::SymbolKind::Parameter => CompletionItemKind::VARIABLE,
                crate::lsp::symbols::SymbolKind::Property => CompletionItemKind::PROPERTY,
                crate::lsp::symbols::SymbolKind::Method => CompletionItemKind::METHOD,
                crate::lsp::symbols::SymbolKind::Constant => CompletionItemKind::CONSTANT,
            };

            let mut item = CompletionItem {
                label: scoped.symbol.name.clone(),
                kind: Some(kind),
                insert_text: Some(scoped.symbol.name.clone()),
                detail: scoped.symbol.type_name.clone(),
                ..Default::default()
            };

            if let Some(type_name) = &scoped.symbol.type_name {
                item.documentation = Some(tower_lsp::lsp_types::Documentation::String(
                    type_name.clone(),
                ));
            }

            items.push(item);
        }
    }

    Some(CompletionResponse::Array(items))
}

fn split_at_offset(source: &str, offset: usize) -> (&str, &str) {
    let mut current_offset = 0;

    for (idx, c) in source.char_indices() {
        if current_offset >= offset {
            return (&source[..idx], &source[idx..]);
        }
        current_offset += c.len_utf8();
    }

    (source, "")
}

fn extract_word_prefix(leading: &str) -> String {
    let mut prefix = String::new();
    for c in leading.chars().rev() {
        if c.is_alphanumeric() || c == '_' {
            prefix.push(c);
        } else {
            break;
        }
    }
    prefix.chars().rev().collect()
}

fn position_to_offset(source: &str, position: Position) -> Option<usize> {
    let mut offset = 0;

    for (line, line_str) in source.lines().enumerate() {
        if line as u32 == position.line {
            let col = position.character as usize;
            for (char_offset, (i, _)) in line_str.char_indices().enumerate() {
                if char_offset >= col {
                    return Some(offset + i);
                }
            }
            return Some(offset + line_str.len().min(col));
        }
        offset += line_str.len() + 1;
    }
    None
}
