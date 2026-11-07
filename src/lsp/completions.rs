//! Completions provider for LSP.
use super::project::{IndexedKind, IndexedSymbol, Namespace, ProjectIndex};
use super::receiver::{MemberAccess, Receiver};
use super::util::position_to_offset;
use crate::interpreter::executor::calls::method_registry;
use crate::lsp::symbols::SymbolTable;
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, Position,
};

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

/// The completions at `position`.
///
/// After a `.` or an `@` they are the receiver's members (see
/// [`super::receiver`]): a project class's static methods and scopes, an
/// instance's methods, fields and relations (inherited ones included, and
/// `Model`'s when the class is a model), a literal's built-in methods. A
/// receiver that cannot be resolved gets every method name in the project,
/// marked as such. Elsewhere they are keywords, types, the file's names and
/// the project's top-level classes and functions.
pub fn get_completions(
    source: &str,
    position: Position,
    table: Option<&SymbolTable>,
    project: Option<(&ProjectIndex, Option<Namespace>)>,
) -> Option<CompletionResponse> {
    let offset = position_to_offset(source, position)?;
    let (leading, _) = split_at_offset(source, offset);

    if let Some(access) = super::receiver::member_access(leading, leading.len()) {
        // A `.` that ends a sentence in a comment, or an `@` in a string
        // (`"me@"`), is not a member access: no list of every method.
        if in_comment_or_string(current_code_line(leading)) {
            return None;
        }
        let index = project.map(|(index, _)| index);
        return Some(CompletionResponse::Array(member_completions(
            &access, index,
        )));
    }

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
                item.documentation = Some(tower_lsp_server::ls_types::Documentation::String(
                    type_name.clone(),
                ));
            }

            items.push(item);
        }
    }

    if let Some((index, namespace)) = project {
        for symbol in index.reachable_top_level(namespace) {
            if symbol.name.starts_with(&prefix) && !items.iter().any(|i| i.label == symbol.name) {
                items.push(indexed_item(symbol));
            }
        }
    }

    Some(CompletionResponse::Array(items))
}

/// The members a receiver offers, filtered by the part already typed.
fn member_completions(access: &MemberAccess, index: Option<&ProjectIndex>) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = Vec::new();
    let class_receiver = match &access.receiver {
        Receiver::Class(class) => Some((class.as_str(), true)),
        Receiver::Instance(class) | Receiver::SelfInstance(Some(class)) => {
            Some((class.as_str(), false))
        }
        _ => None,
    };
    let resolved = index
        .zip(class_receiver)
        .filter(|(index, (class, _))| index.class(class).is_some());
    if let Some((index, (class, static_side))) = resolved {
        items.extend(
            index
                .members(class, static_side)
                .into_iter()
                .map(indexed_item),
        );
        let model = index.is_model(class);
        if static_side {
            items.push(builtin_item(
                "new",
                "constructor",
                CompletionItemKind::CONSTRUCTOR,
            ));
            if model {
                for name in crate::interpreter::builtins::model::model_static_method_names() {
                    items.push(builtin_item(
                        &name,
                        "Model class method",
                        CompletionItemKind::METHOD,
                    ));
                }
            }
        } else if model {
            for name in crate::interpreter::builtins::model::model_instance_method_names() {
                items.push(builtin_item(
                    &name,
                    "Model instance method",
                    CompletionItemKind::METHOD,
                ));
            }
        }
    } else if let Receiver::Builtin(type_name) = access.receiver {
        for method in method_registry::known_methods(type_name) {
            let detail = match (method.zero_arg, method.ret) {
                (true, "") => format!("{type_name} method, no arguments"),
                (true, ret) => format!("{type_name} method, no arguments -> {ret}"),
                (false, "") => format!("{type_name} method"),
                (false, ret) => format!("{type_name} method -> {ret}"),
            };
            items.push(builtin_item(
                method.name,
                &detail,
                CompletionItemKind::METHOD,
            ));
        }
    } else if let Some(index) = index {
        for name in index.all_method_names() {
            items.push(builtin_item(
                &name,
                "a method in this project (receiver not resolved)",
                CompletionItemKind::METHOD,
            ));
        }
    }
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| item.label.starts_with(&access.prefix) && seen.insert(item.label.clone()));
    items
}

fn builtin_item(name: &str, detail: &str, kind: CompletionItemKind) -> CompletionItem {
    CompletionItem {
        label: name.to_string(),
        kind: Some(kind),
        insert_text: Some(name.to_string()),
        detail: Some(detail.to_string()),
        ..Default::default()
    }
}

fn indexed_item(symbol: &IndexedSymbol) -> CompletionItem {
    let kind = match symbol.kind {
        IndexedKind::Class | IndexedKind::Module => CompletionItemKind::CLASS,
        IndexedKind::Function => CompletionItemKind::FUNCTION,
        IndexedKind::Constant => CompletionItemKind::CONSTANT,
        IndexedKind::Method | IndexedKind::StaticMethod | IndexedKind::Scope => {
            CompletionItemKind::METHOD
        }
        IndexedKind::Field | IndexedKind::Relation => CompletionItemKind::FIELD,
    };
    let detail = match (&symbol.signature, &symbol.owner) {
        (Some(signature), _) => signature.clone(),
        (None, Some(owner)) => format!("{} of {owner}", symbol.kind.label()),
        (None, None) => symbol.kind.label().to_string(),
    };
    CompletionItem {
        label: symbol.name.clone(),
        kind: Some(kind),
        insert_text: Some(symbol.name.clone()),
        detail: Some(detail),
        ..Default::default()
    }
}

/// Whether the cursor at `position` of a `.slv` template is inside a
/// `<% %>` tag — the only place a template holds Soli code. Prose and markup
/// around the tags get no completion (a `.` ending an HTML sentence is not a
/// member access).
pub fn in_template_code(source: &str, position: Position) -> bool {
    let Some(offset) = position_to_offset(source, position) else {
        return false;
    };
    let (leading, _) = split_at_offset(source, offset);
    match (leading.rfind("<%"), leading.rfind("%>")) {
        (Some(open), Some(close)) => open > close,
        (Some(_), None) => true,
        _ => false,
    }
}

/// The code on the cursor's line before it: from the line start, or from the
/// `<%` that opens a template tag on that line.
fn current_code_line(leading: &str) -> &str {
    let line_start = leading.rfind('\n').map_or(0, |i| i + 1);
    let tag_start = leading.rfind("<%").map_or(0, |i| i + 2);
    &leading[line_start.max(tag_start).min(leading.len())..]
}

/// Whether the end of `line` (one line of code) is inside a comment or a
/// string literal. A `#{…}` interpolation is code again.
fn in_comment_or_string(line: &str) -> bool {
    enum State {
        Code { braces: usize },
        Text(char),
    }
    let mut stack = vec![State::Code { braces: 0 }];
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        let nested = stack.len() > 1;
        match stack.last_mut() {
            Some(State::Code { braces }) => match c {
                '#' => return true,
                '"' | '\'' => stack.push(State::Text(c)),
                '{' => *braces += 1,
                '}' if *braces == 0 && nested => {
                    stack.pop();
                }
                '}' => *braces = braces.saturating_sub(1),
                _ => {}
            },
            Some(State::Text(quote)) => {
                let quote = *quote;
                if c == '\\' {
                    chars.next();
                } else if c == quote {
                    stack.pop();
                } else if quote == '"' && c == '#' && chars.peek() == Some(&'{') {
                    chars.next();
                    stack.push(State::Code { braces: 0 });
                }
            }
            None => return false,
        }
    }
    !matches!(stack.last(), Some(State::Code { .. }))
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
