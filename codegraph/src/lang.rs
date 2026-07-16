//! Per-language tree-sitter extraction. Each language walks its parse tree by
//! node kind + field name (more robust across grammar versions than `.scm`
//! query strings) and emits language-agnostic [`Def`]/[`EdgeRef`] records.
//!
//! Every walker extracts *definitions* (class/module/method/function/…),
//! *inheritance* (`inherits`/`implements`), and *imports*. Call resolution is
//! left to the caller (best-effort name matching), so calls are not emitted
//! here.

use tree_sitter::{Node, Parser};

use crate::{Def, EdgeRef, Extraction};

/// A source language with a tree-sitter grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Ruby,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Rust,
    CSharp,
}

pub fn language_for_extension(ext: &str) -> Option<Language> {
    match ext {
        "rb" | "rake" | "gemspec" => Some(Language::Ruby),
        "py" | "pyi" => Some(Language::Python),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
        "ts" | "mts" | "cts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        "rs" => Some(Language::Rust),
        "cs" => Some(Language::CSharp),
        _ => None,
    }
}

pub fn extract(language: Language, source: &str) -> Extraction {
    let (lang, walk): (tree_sitter::Language, WalkFn) = match language {
        Language::Ruby => (tree_sitter_ruby::LANGUAGE.into(), walk_ruby),
        Language::Python => (tree_sitter_python::LANGUAGE.into(), walk_python),
        Language::JavaScript => (tree_sitter_javascript::LANGUAGE.into(), walk_js),
        Language::TypeScript => (tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), walk_js),
        Language::Tsx => (tree_sitter_typescript::LANGUAGE_TSX.into(), walk_js),
        Language::Rust => (tree_sitter_rust::LANGUAGE.into(), walk_rust),
        Language::CSharp => (tree_sitter_c_sharp::LANGUAGE.into(), walk_csharp),
    };
    extract_with(source, lang, walk)
}

type WalkFn = fn(Node, &[u8], &mut Vec<String>, &mut Extraction);

fn extract_with(source: &str, language: tree_sitter::Language, walk: WalkFn) -> Extraction {
    let mut out = Extraction::default();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(source, None) else {
        return out;
    };
    let mut scope: Vec<String> = Vec::new();
    walk(tree.root_node(), source.as_bytes(), &mut scope, &mut out);
    out
}

// ---- shared helpers --------------------------------------------------------

fn text<'a>(node: Node, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn field_text(node: Node, field: &str, bytes: &[u8]) -> String {
    node.child_by_field_name(field)
        .map(|n| text(n, bytes).to_string())
        .unwrap_or_default()
}

fn line_of(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

#[allow(clippy::too_many_arguments)]
fn push_def(
    out: &mut Extraction,
    kind: &str,
    name: &str,
    qualified_name: String,
    node: Node,
    signature: String,
    superclass: Option<String>,
) {
    out.defs.push(Def {
        kind: kind.to_string(),
        name: name.to_string(),
        qualified_name,
        line: line_of(node),
        signature,
        superclass,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    });
}

fn qualify(scope: &[String], name: &str, sep: &str) -> String {
    match scope.last() {
        Some(owner) => format!("{}{}{}", owner, sep, name),
        None => name.to_string(),
    }
}

/// Method vs function by enclosing scope, with the language's method separator.
fn member(scope: &[String], name: &str, sep: &str) -> (&'static str, String) {
    match scope.last() {
        Some(owner) => ("method", format!("{}{}{}", owner, sep, name)),
        None => ("function", name.to_string()),
    }
}

// ---- Ruby ------------------------------------------------------------------

fn walk_ruby(node: Node, bytes: &[u8], scope: &mut Vec<String>, out: &mut Extraction) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "class" | "module" => {
                let name = field_text(child, "name", bytes);
                if name.is_empty() {
                    walk_ruby(child, bytes, scope, out);
                    continue;
                }
                let is_class = child.kind() == "class";
                let superclass = ruby_superclass(child, bytes);
                let qn = if scope.is_empty() {
                    name.clone()
                } else {
                    format!("{}::{}", scope.join("::"), name)
                };
                let signature = match (is_class, &superclass) {
                    (true, Some(s)) => format!("class {} < {}", qn, s),
                    (true, None) => format!("class {}", qn),
                    (false, _) => format!("module {}", qn),
                };
                push_def(
                    out,
                    if is_class { "class" } else { "module" },
                    &name,
                    qn.clone(),
                    child,
                    signature,
                    superclass.clone(),
                );
                if let Some(s) = superclass {
                    out.edges.push(edge("inherits", &s, &qn, line_of(child)));
                }
                scope.push(name);
                walk_ruby(child, bytes, scope, out);
                scope.pop();
            }
            "method" | "singleton_method" => {
                let name = field_text(child, "name", bytes);
                if name.is_empty() {
                    continue;
                }
                let (kind, qn) = member(scope, &name, "#");
                let params = field_text(child, "parameters", bytes);
                push_def(
                    out,
                    kind,
                    &name,
                    qn,
                    child,
                    format!("def {}{}", name, params),
                    None,
                );
            }
            "call" => {
                ruby_call(child, bytes, scope, out);
                walk_ruby(child, bytes, scope, out);
            }
            _ => walk_ruby(child, bytes, scope, out),
        }
    }
}

fn ruby_superclass(class_node: Node, bytes: &[u8]) -> Option<String> {
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        if child.kind() == "superclass" {
            let mut inner = child.walk();
            return child
                .named_children(&mut inner)
                .last()
                .map(|n| text(n, bytes).to_string())
                .filter(|s| !s.is_empty());
        }
    }
    None
}

fn ruby_call(call: Node, bytes: &[u8], scope: &[String], out: &mut Extraction) {
    let method = call
        .child_by_field_name("method")
        .map(|n| text(n, bytes))
        .unwrap_or("");
    let from = scope.last().cloned().unwrap_or_default();
    let line = line_of(call);
    match method {
        "require" | "require_relative" => {
            if let Some(arg) = ruby_arg(call, bytes, "string") {
                out.edges.push(edge_from("imports", &arg, &from, line));
            }
        }
        "include" | "extend" | "prepend" => {
            if let Some(arg) = ruby_arg(call, bytes, "constant") {
                out.edges.push(edge_from("implements", &arg, &from, line));
            }
        }
        _ => {}
    }
}

fn ruby_arg(call: Node, bytes: &[u8], want: &str) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for arg in args.named_children(&mut cursor) {
        if want == "string" && arg.kind() == "string" {
            let mut inner = arg.walk();
            for part in arg.named_children(&mut inner) {
                if part.kind() == "string_content" {
                    return Some(text(part, bytes).to_string());
                }
            }
        } else if want == "constant" && matches!(arg.kind(), "constant" | "scope_resolution") {
            return Some(text(arg, bytes).to_string());
        }
    }
    None
}

// ---- Python ----------------------------------------------------------------

fn walk_python(node: Node, bytes: &[u8], scope: &mut Vec<String>, out: &mut Extraction) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "class_definition" => {
                let name = field_text(child, "name", bytes);
                if name.is_empty() {
                    continue;
                }
                let bases = python_bases(child, bytes);
                let qn = qualify(scope, &name, ".");
                let signature = if bases.is_empty() {
                    format!("class {}", name)
                } else {
                    format!("class {}({})", name, bases.join(", "))
                };
                push_def(
                    out,
                    "class",
                    &name,
                    qn.clone(),
                    child,
                    signature,
                    bases.first().cloned(),
                );
                for base in &bases {
                    out.edges.push(edge("inherits", base, &qn, line_of(child)));
                }
                scope.push(name);
                walk_python(child, bytes, scope, out);
                scope.pop();
            }
            "function_definition" => {
                let name = field_text(child, "name", bytes);
                if name.is_empty() {
                    continue;
                }
                let (kind, qn) = member(scope, &name, ".");
                let params = field_text(child, "parameters", bytes);
                push_def(
                    out,
                    kind,
                    &name,
                    qn,
                    child,
                    format!("def {}{}", name, params),
                    None,
                );
            }
            "import_statement" | "import_from_statement" => {
                let target = field_text(child, "module_name", bytes);
                let target = if target.is_empty() {
                    // plain `import a.b` — take the first dotted_name child
                    child_of_kind(child, "dotted_name", bytes)
                } else {
                    target
                };
                if !target.is_empty() {
                    out.edges
                        .push(edge_from("imports", &target, "", line_of(child)));
                }
            }
            _ => walk_python(child, bytes, scope, out),
        }
    }
}

fn python_bases(class_node: Node, bytes: &[u8]) -> Vec<String> {
    let Some(args) = class_node.child_by_field_name("superclasses") else {
        return Vec::new();
    };
    let mut cursor = args.walk();
    args.named_children(&mut cursor)
        .filter(|n| matches!(n.kind(), "identifier" | "attribute"))
        .map(|n| text(n, bytes).to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ---- JavaScript / TypeScript -----------------------------------------------

fn walk_js(node: Node, bytes: &[u8], scope: &mut Vec<String>, out: &mut Extraction) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "abstract_class_declaration" | "class" => {
                let name = field_text(child, "name", bytes);
                if name.is_empty() {
                    walk_js(child, bytes, scope, out);
                    continue;
                }
                let superclass = js_superclass(child, bytes);
                let signature = match &superclass {
                    Some(s) => format!("class {} extends {}", name, s),
                    None => format!("class {}", name),
                };
                push_def(
                    out,
                    "class",
                    &name,
                    name.clone(),
                    child,
                    signature,
                    superclass.clone(),
                );
                if let Some(s) = superclass {
                    out.edges.push(edge("inherits", &s, &name, line_of(child)));
                }
                scope.push(name);
                walk_js(child, bytes, scope, out);
                scope.pop();
            }
            "interface_declaration" | "enum_declaration" => {
                let name = field_text(child, "name", bytes);
                if !name.is_empty() {
                    let kind = if child.kind() == "enum_declaration" {
                        "enum"
                    } else {
                        "interface"
                    };
                    push_def(
                        out,
                        kind,
                        &name,
                        name.clone(),
                        child,
                        format!("{} {}", kind, name),
                        None,
                    );
                }
            }
            "method_definition" => {
                let name = field_text(child, "name", bytes);
                if name.is_empty() {
                    continue;
                }
                let (kind, qn) = member(scope, &name, "#");
                let params = field_text(child, "parameters", bytes);
                push_def(
                    out,
                    kind,
                    &name,
                    qn,
                    child,
                    format!("{}{}", name, params),
                    None,
                );
            }
            "function_declaration" | "generator_function_declaration" => {
                let name = field_text(child, "name", bytes);
                if !name.is_empty() {
                    let params = field_text(child, "parameters", bytes);
                    push_def(
                        out,
                        "function",
                        &name,
                        name.clone(),
                        child,
                        format!("function {}{}", name, params),
                        None,
                    );
                }
            }
            "import_statement" => {
                let src = child
                    .child_by_field_name("source")
                    .map(|n| js_string(n, bytes));
                if let Some(s) = src.flatten() {
                    out.edges.push(edge_from("imports", &s, "", line_of(child)));
                }
            }
            _ => walk_js(child, bytes, scope, out),
        }
    }
}

fn js_superclass(class_node: Node, bytes: &[u8]) -> Option<String> {
    let mut cursor = class_node.walk();
    let heritage: Vec<Node> = class_node
        .children(&mut cursor)
        .filter(|c| c.kind() == "class_heritage")
        .collect();
    for h in heritage {
        let mut inner = h.walk();
        let parts: Vec<Node> = h.named_children(&mut inner).collect();
        for part in parts {
            // extends_clause (TS) wraps the type; JS puts the expr directly.
            let target = if part.kind() == "extends_clause" {
                let mut c2 = part.walk();
                let inner_nodes: Vec<Node> = part.named_children(&mut c2).collect();
                inner_nodes.first().map(|n| text(*n, bytes).to_string())
            } else {
                Some(text(part, bytes).to_string())
            };
            if let Some(s) = target {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn js_string(node: Node, bytes: &[u8]) -> Option<String> {
    // strip surrounding quotes
    let raw = text(node, bytes);
    let trimmed = raw.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// ---- Rust ------------------------------------------------------------------

fn walk_rust(node: Node, bytes: &[u8], scope: &mut Vec<String>, out: &mut Extraction) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "struct_item" | "enum_item" | "union_item" => {
                let name = field_text(child, "name", bytes);
                if !name.is_empty() {
                    let kind = if child.kind() == "enum_item" {
                        "enum"
                    } else {
                        "class"
                    };
                    let keyword = child.kind().trim_end_matches("_item");
                    push_def(
                        out,
                        kind,
                        &name,
                        name.clone(),
                        child,
                        format!("{} {}", keyword, name),
                        None,
                    );
                }
            }
            "trait_item" => {
                let name = field_text(child, "name", bytes);
                if !name.is_empty() {
                    push_def(
                        out,
                        "interface",
                        &name,
                        name.clone(),
                        child,
                        format!("trait {}", name),
                        None,
                    );
                }
            }
            "impl_item" => {
                let type_name = field_text(child, "type", bytes);
                let trait_name = field_text(child, "trait", bytes);
                if !trait_name.is_empty() && !type_name.is_empty() {
                    out.edges
                        .push(edge("implements", &trait_name, &type_name, line_of(child)));
                }
                if type_name.is_empty() {
                    walk_rust(child, bytes, scope, out);
                } else {
                    scope.push(type_name);
                    walk_rust(child, bytes, scope, out);
                    scope.pop();
                }
            }
            "function_item" => {
                let name = field_text(child, "name", bytes);
                if !name.is_empty() {
                    let (kind, qn) = member(scope, &name, "::");
                    let params = field_text(child, "parameters", bytes);
                    push_def(
                        out,
                        kind,
                        &name,
                        qn,
                        child,
                        format!("fn {}{}", name, params),
                        None,
                    );
                }
            }
            "use_declaration" => {
                let arg = field_text(child, "argument", bytes);
                if !arg.is_empty() {
                    out.edges
                        .push(edge_from("imports", &arg, "", line_of(child)));
                }
            }
            _ => walk_rust(child, bytes, scope, out),
        }
    }
}

// ---- C# --------------------------------------------------------------------

fn walk_csharp(node: Node, bytes: &[u8], scope: &mut Vec<String>, out: &mut Extraction) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "record_declaration" | "struct_declaration" => {
                let name = field_text(child, "name", bytes);
                if name.is_empty() {
                    walk_csharp(child, bytes, scope, out);
                    continue;
                }
                let bases = csharp_bases(child, bytes);
                push_def(
                    out,
                    "class",
                    &name,
                    name.clone(),
                    child,
                    format!("class {}", name),
                    bases.first().cloned(),
                );
                // First base = superclass (inherits); the rest = interfaces.
                for (i, base) in bases.iter().enumerate() {
                    let kind = if i == 0 { "inherits" } else { "implements" };
                    out.edges.push(edge(kind, base, &name, line_of(child)));
                }
                scope.push(name);
                walk_csharp(child, bytes, scope, out);
                scope.pop();
            }
            "interface_declaration" | "enum_declaration" => {
                let name = field_text(child, "name", bytes);
                if !name.is_empty() {
                    let kind = if child.kind() == "enum_declaration" {
                        "enum"
                    } else {
                        "interface"
                    };
                    push_def(
                        out,
                        kind,
                        &name,
                        name.clone(),
                        child,
                        format!("{} {}", kind, name),
                        None,
                    );
                }
            }
            "method_declaration" | "constructor_declaration" => {
                let name = field_text(child, "name", bytes);
                if !name.is_empty() {
                    let (kind, qn) = member(scope, &name, ".");
                    let params = field_text(child, "parameters", bytes);
                    push_def(
                        out,
                        kind,
                        &name,
                        qn,
                        child,
                        format!("{}{}", name, params),
                        None,
                    );
                }
            }
            "using_directive" => {
                let target = csharp_using(child, bytes);
                if !target.is_empty() {
                    out.edges
                        .push(edge_from("imports", &target, "", line_of(child)));
                }
            }
            _ => walk_csharp(child, bytes, scope, out),
        }
    }
}

fn csharp_bases(class_node: Node, bytes: &[u8]) -> Vec<String> {
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        if child.kind() == "base_list" {
            let mut inner = child.walk();
            return child
                .named_children(&mut inner)
                .map(|n| text(n, bytes).to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    Vec::new()
}

fn csharp_using(node: Node, bytes: &[u8]) -> String {
    // `using System.Text;` — the namespace is the qualified_name/identifier child.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(child.kind(), "qualified_name" | "identifier") {
            return text(child, bytes).to_string();
        }
    }
    String::new()
}

// ---- edge helpers ----------------------------------------------------------

fn edge(kind: &str, target: &str, from_qualified: &str, line: u32) -> EdgeRef {
    EdgeRef {
        kind: kind.to_string(),
        target: target.to_string(),
        from_qualified: from_qualified.to_string(),
        line,
    }
}

fn edge_from(kind: &str, target: &str, from_qualified: &str, line: u32) -> EdgeRef {
    edge(kind, target, from_qualified, line)
}

fn child_of_kind(node: Node, kind: &str, bytes: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == kind {
            return text(child, bytes).to_string();
        }
    }
    String::new()
}
