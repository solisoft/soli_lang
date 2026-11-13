//! Per-language tree-sitter extraction. Each language walks its parse tree by
//! node kind + field name (more robust across grammar versions than `.scm`
//! query strings) and emits language-agnostic [`Def`]/[`EdgeRef`] records.
//!
//! Every walker extracts *definitions* (class/module/method/function/…),
//! *inheritance* (`inherits`/`implements`), and *imports*. The C# walker also
//! emits by-name `calls` and `instantiates` references from method bodies; the
//! caller resolves each name against the project's definitions (best-effort,
//! precision-first). The Python walker emits them too, with a receiver when it
//! is known (`self.m`, `super.m`, `Class.m`, a local whose class is certain);
//! see [`python_scan_body`]. Other languages still emit structure only.

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
                    qn.clone(),
                    child,
                    format!("def {}{}", name, params),
                    None,
                );
                let mut locals = std::collections::HashMap::new();
                if let Some(parameters) = child.child_by_field_name("parameters") {
                    python_typed_params(parameters, bytes, &mut locals);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    python_scan_body(body, bytes, &qn, &mut locals, out);
                }
            }
            "decorated_definition" => {
                python_routes(child, bytes, scope, out);
                walk_python(child, bytes, scope, out);
            }
            "expression_statement" => {
                if let Some(assignment) = child.named_child(0).filter(|n| n.kind() == "assignment") {
                    python_declaration(assignment, bytes, scope, out);
                }
                walk_python(child, bytes, scope, out);
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

/// HTTP verbs a Flask `MethodView` answers with methods of that name.
const HTTP_VERBS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options"];

/// Module- and class-level declarations the graph reads:
/// `bp = Blueprint("name", __name__, url_prefix="/p")` → `blueprint`
/// (`bp\t/p`), and `__tablename__ = "t"` in a class → `table` (`t`).
fn python_declaration(assignment: Node, bytes: &[u8], scope: &[String], out: &mut Extraction) {
    let (Some(left), Some(right)) = (
        assignment.child_by_field_name("left"),
        assignment.child_by_field_name("right"),
    ) else {
        return;
    };
    if left.kind() != "identifier" {
        return;
    }
    let name = text(left, bytes);
    let line = line_of(assignment);
    if name == "__tablename__" && !scope.is_empty() {
        if let Some(table) = python_string(right, bytes) {
            out.edges
                .push(edge("table", &table, &scope.join("."), line));
        }
        return;
    }
    if right.kind() != "call" {
        return;
    }
    let callee = right
        .child_by_field_name("function")
        .map(|f| text(f, bytes))
        .unwrap_or("");
    if callee.rsplit('.').next() != Some("Blueprint") {
        return;
    }
    let prefix = right
        .child_by_field_name("arguments")
        .and_then(|args| python_keyword(args, "url_prefix", bytes))
        .and_then(|value| python_string(value, bytes))
        .unwrap_or_default();
    out.edges.push(edge(
        "blueprint",
        &format!("{}\t{}", name, prefix),
        "",
        line,
    ));
}

/// `@bp.route("/path", methods=[…])` on a function or a `MethodView` class:
/// one `route` reference per verb (`bp\tVERB\t/path`), from the handler.
fn python_routes(decorated: Node, bytes: &[u8], scope: &[String], out: &mut Extraction) {
    let Some(definition) = decorated.child_by_field_name("definition") else {
        return;
    };
    let def_name = field_text(definition, "name", bytes);
    if def_name.is_empty() {
        return;
    }
    let mut cursor = decorated.walk();
    for decorator in decorated.named_children(&mut cursor) {
        if decorator.kind() != "decorator" {
            continue;
        }
        let Some(call) = decorator.named_child(0).filter(|n| n.kind() == "call") else {
            continue;
        };
        let Some(func) = call.child_by_field_name("function").filter(|f| f.kind() == "attribute")
        else {
            continue;
        };
        if func.child_by_field_name("attribute").map(|a| text(a, bytes)) != Some("route") {
            continue;
        }
        let blueprint = func
            .child_by_field_name("object")
            .map(|o| text(o, bytes).to_string())
            .unwrap_or_default();
        let Some(args) = call.child_by_field_name("arguments") else {
            continue;
        };
        let mut arg_cursor = args.walk();
        let Some(path) = args
            .named_children(&mut arg_cursor)
            .next()
            .and_then(|first| python_string(first, bytes))
        else {
            continue;
        };
        let line = line_of(decorator);
        if definition.kind() == "class_definition" {
            // A `MethodView`: each verb method is a handler.
            let Some(body) = definition.child_by_field_name("body") else {
                continue;
            };
            let mut body_cursor = body.walk();
            for item in body.named_children(&mut body_cursor) {
                let function = if item.kind() == "decorated_definition" {
                    item.child_by_field_name("definition")
                } else {
                    Some(item)
                };
                let Some(function) = function.filter(|f| f.kind() == "function_definition") else {
                    continue;
                };
                let verb = field_text(function, "name", bytes);
                if HTTP_VERBS.contains(&verb.as_str()) {
                    out.edges.push(edge(
                        "route",
                        &format!("{}\t{}\t{}", blueprint, verb.to_uppercase(), path),
                        &format!("{}.{}", def_name, verb),
                        line,
                    ));
                }
            }
        } else {
            let verbs: Vec<String> = python_keyword(args, "methods", bytes)
                .map(|list| {
                    let mut list_cursor = list.walk();
                    list.named_children(&mut list_cursor)
                        .filter_map(|item| python_string(item, bytes))
                        .map(|verb| verb.to_uppercase())
                        .collect()
                })
                .filter(|verbs: &Vec<String>| !verbs.is_empty())
                .unwrap_or_else(|| vec!["GET".to_string()]);
            let (_, handler) = member(scope, &def_name, ".");
            for verb in verbs {
                out.edges.push(edge(
                    "route",
                    &format!("{}\t{}\t{}", blueprint, verb, path),
                    &handler,
                    line,
                ));
            }
        }
    }
}

/// The value of keyword argument `name` in an argument list.
fn python_keyword<'t>(arguments: Node<'t>, name: &str, bytes: &[u8]) -> Option<Node<'t>> {
    let mut cursor = arguments.walk();
    let found = arguments.named_children(&mut cursor).find(|arg| {
        arg.kind() == "keyword_argument"
            && arg.child_by_field_name("name").map(|n| text(n, bytes)) == Some(name)
    });
    found.and_then(|arg| arg.child_by_field_name("value"))
}

/// A plain string literal's value (`"x"`, `'x'`, `r"x"`); none for an
/// f-string or a concatenation.
fn python_string(node: Node, bytes: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let raw = text(node, bytes);
    let unprefixed = raw.trim_start_matches(['r', 'R', 'b', 'B', 'u', 'U']);
    if unprefixed.starts_with(['f', 'F']) {
        return None;
    }
    let quote = unprefixed.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    Some(unprefixed.trim_matches(quote).to_string())
}

/// The model class an SQLAlchemy argument names: `Order`, or `Order.id`
/// (a column of it).
fn python_model_head(node: Node, bytes: &[u8]) -> Option<String> {
    let head = match node.kind() {
        "identifier" => text(node, bytes),
        "attribute" => {
            let mut object = node.child_by_field_name("object")?;
            while object.kind() == "attribute" {
                object = object.child_by_field_name("object")?;
            }
            if object.kind() != "identifier" {
                return None;
            }
            text(object, bytes)
        }
        _ => return None,
    };
    (head.chars().next().is_some_and(char::is_uppercase) && is_simple_ident(head))
        .then(|| head.to_string())
}

/// SQLAlchemy reads and writes in one call or attribute, as `db_read` /
/// `db_write` references (`Model\top`):
/// `select(M, M.col…)`, `.join(M)`, `session.query(M…)`, `session.get(M, …)`,
/// `M.query…` read; `insert(M)`, `update(M)`, `delete(M)` and
/// `session.add/delete/merge(x)` on an instance of a known class write.
fn python_db_refs(
    node: Node,
    bytes: &[u8],
    from_qn: &str,
    locals: &std::collections::HashMap<String, String>,
    out: &mut Extraction,
) {
    let line = line_of(node);
    let mut push = |kind: &str, model: &str, op: &str| {
        out.edges
            .push(edge(kind, &format!("{}\t{}", model, op), from_qn, line));
    };
    if node.kind() == "attribute" {
        if node.child_by_field_name("attribute").map(|a| text(a, bytes)) == Some("query") {
            if let Some(object) = node.child_by_field_name("object") {
                if let Some(model) = python_model_head(object, bytes).filter(|_| object.kind() == "identifier") {
                    push("db_read", &model, "query");
                }
            }
        }
        return;
    }
    if node.kind() != "call" {
        return;
    }
    let (Some(func), Some(args)) = (
        node.child_by_field_name("function"),
        node.child_by_field_name("arguments"),
    ) else {
        return;
    };
    let mut cursor = args.walk();
    let positional: Vec<Node> = args
        .named_children(&mut cursor)
        .filter(|a| a.kind() != "keyword_argument")
        .collect();
    let (name, on_session) = match func.kind() {
        "identifier" => (text(func, bytes), false),
        "attribute" => {
            let attribute = func
                .child_by_field_name("attribute")
                .map(|a| text(a, bytes))
                .unwrap_or("");
            let object = func
                .child_by_field_name("object")
                .map(|o| text(o, bytes))
                .unwrap_or("");
            (attribute, object.ends_with("session"))
        }
        _ => return,
    };
    match (name, on_session) {
        ("select", false) => {
            for arg in &positional {
                if let Some(model) = python_model_head(*arg, bytes) {
                    push("db_read", &model, "select");
                }
            }
        }
        ("insert" | "update" | "delete", false) if func.kind() == "identifier" => {
            if let Some(model) = positional.first().and_then(|a| python_model_head(*a, bytes)) {
                push("db_write", &model, name);
            }
        }
        ("join" | "outerjoin" | "select_from", false) => {
            if let Some(model) = positional
                .first()
                .filter(|a| a.kind() == "identifier")
                .and_then(|a| python_model_head(*a, bytes))
            {
                push("db_read", &model, name);
            }
        }
        ("query", true) => {
            for arg in &positional {
                if let Some(model) = python_model_head(*arg, bytes) {
                    push("db_read", &model, "query");
                }
            }
        }
        ("get", true) => {
            if let Some(model) = positional
                .first()
                .filter(|a| a.kind() == "identifier")
                .and_then(|a| python_model_head(*a, bytes))
            {
                push("db_read", &model, "get");
            }
        }
        ("add" | "delete" | "merge", true) => {
            let class = positional.first().and_then(|arg| match arg.kind() {
                // Inside a model method: the builder reads `self` as its class.
                "identifier" if text(*arg, bytes) == "self" => Some("self".to_string()),
                "identifier" => locals.get(text(*arg, bytes)).cloned(),
                "call" => {
                    let value = python_value_class(*arg, bytes);
                    (!value.is_empty()).then_some(value)
                }
                _ => None,
            });
            if let Some(class) = class {
                push("db_write", &class, name);
            }
        }
        _ => {}
    }
}

/// `x: Order` / `x: Order = …` parameters: the local's class.
fn python_typed_params(
    parameters: Node,
    bytes: &[u8],
    locals: &mut std::collections::HashMap<String, String>,
) {
    let mut cursor = parameters.walk();
    for param in parameters.named_children(&mut cursor) {
        if !matches!(param.kind(), "typed_parameter" | "typed_default_parameter") {
            continue;
        }
        let name = match param.child_by_field_name("name") {
            Some(n) => text(n, bytes).to_string(),
            None => child_of_kind(param, "identifier", bytes),
        };
        let class = param
            .child_by_field_name("type")
            .map(|t| python_class_name(text(t, bytes)))
            .unwrap_or_default();
        if !name.is_empty() && !class.is_empty() {
            locals.insert(name, class);
        }
    }
}

/// A type annotation's class, when it names exactly one: `Order`,
/// `Optional[Order]`, `"Order"`, `models.Order` → `Order`; `list[Order]` or
/// `Order | None` style unions of several classes → none.
fn python_class_name(annotation: &str) -> String {
    let raw = annotation.trim().trim_matches(|c| c == '"' || c == '\'');
    let raw = raw
        .strip_prefix("Optional[")
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or(raw);
    let raw = raw.trim_end_matches("| None").trim_end_matches(" ").trim();
    let last = raw.rsplit('.').next().unwrap_or(raw);
    if is_simple_ident(last) && last.chars().next().is_some_and(char::is_uppercase) {
        last.to_string()
    } else {
        String::new()
    }
}

/// Emit `calls` / `instantiates` references from a Python body, attributed to
/// `from_qn`. A call's target names its receiver when it is known, and the
/// builder resolves each against the project, dropping what it cannot place:
///
/// - `name(…)` → `calls name` (a lowercase function) or `instantiates Name`;
/// - `self.m(…)` / `cls.m(…)` → `calls self.m`; `super().m(…)` → `calls super.m`;
/// - `Name.m(…)` → `calls Name.m` (a class's static or class method);
/// - `x.m(…)` where `x` holds a class for certain → `calls Class.m`. A local
///   gets one from a typed parameter, `x = Class(…)`, `x: Class = …`, or
///   `x = Registry.get(Class)` (a service locator handing out an instance).
///   Any other assignment clears it.
fn python_scan_body(
    node: Node,
    bytes: &[u8],
    from_qn: &str,
    locals: &mut std::collections::HashMap<String, String>,
    out: &mut Extraction,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "assignment" => {
                // Walk the value first: `x = Foo(x.bar())` reads the old `x`.
                if let Some(right) = child.child_by_field_name("right") {
                    python_scan_body_node(right, bytes, from_qn, locals, out);
                }
                if let Some(left) = child.child_by_field_name("left") {
                    if left.kind() == "identifier" {
                        let name = text(left, bytes).to_string();
                        let annotated = child
                            .child_by_field_name("type")
                            .map(|t| python_class_name(text(t, bytes)))
                            .unwrap_or_default();
                        let inferred = if annotated.is_empty() {
                            child
                                .child_by_field_name("right")
                                .map(|r| python_value_class(r, bytes))
                                .unwrap_or_default()
                        } else {
                            annotated
                        };
                        if inferred.is_empty() {
                            locals.remove(&name);
                        } else {
                            locals.insert(name, inferred);
                        }
                    }
                }
            }
            // Nested definitions are not walked as defs; their calls still
            // belong to the enclosing function, with their own locals.
            "function_definition" | "class_definition" => {
                let mut nested = locals.clone();
                python_scan_body(child, bytes, from_qn, &mut nested, out);
            }
            _ => python_scan_body_node(child, bytes, from_qn, locals, out),
        }
    }
}

/// [`python_scan_body`] on one node: a call is recorded, then its children
/// (callee chain, arguments) are scanned.
fn python_scan_body_node(
    node: Node,
    bytes: &[u8],
    from_qn: &str,
    locals: &mut std::collections::HashMap<String, String>,
    out: &mut Extraction,
) {
    if node.kind() == "call" {
        if let Some(func) = node.child_by_field_name("function") {
            if let Some((kind, target)) = python_call_target(func, bytes, locals) {
                out.edges.push(edge(kind, &target, from_qn, line_of(node)));
            }
        }
    }
    python_db_refs(node, bytes, from_qn, locals, out);
    python_scan_body(node, bytes, from_qn, locals, out);
}

/// The reference a call's callee makes, when its receiver is known.
fn python_call_target(
    func: Node,
    bytes: &[u8],
    locals: &std::collections::HashMap<String, String>,
) -> Option<(&'static str, String)> {
    match func.kind() {
        "identifier" => {
            let name = text(func, bytes);
            if name.chars().next().is_some_and(char::is_uppercase) {
                Some(("instantiates", name.to_string()))
            } else {
                Some(("calls", name.to_string()))
            }
        }
        "attribute" => {
            let object = func.child_by_field_name("object")?;
            let method = text(func.child_by_field_name("attribute")?, bytes);
            let receiver = match object.kind() {
                "identifier" => {
                    let name = text(object, bytes);
                    match name {
                        "self" | "cls" => "self".to_string(),
                        _ => match locals.get(name) {
                            Some(class) => class.clone(),
                            None if name.chars().next().is_some_and(char::is_uppercase) => {
                                name.to_string()
                            }
                            None => return None,
                        },
                    }
                }
                // `super().m(…)`
                "call"
                    if object
                        .child_by_field_name("function")
                        .is_some_and(|f| text(f, bytes) == "super") =>
                {
                    "super".to_string()
                }
                _ => return None,
            };
            is_simple_ident(method).then(|| ("calls", format!("{}.{}", receiver, method)))
        }
        _ => None,
    }
}

/// The class a value certainly holds: `Class(…)`; `X.get(Class)` /
/// `X.resolve(Class)` — a registry handing out an instance of its argument
/// (SQLAlchemy's `session.get(Model, id)` included); or one row of a model
/// query, `Model.query….get(…)` / `.first()` / `.one()` /
/// `.one_or_none()` / `.first_or_404()` / `.get_or_404(…)`.
fn python_value_class(value: Node, bytes: &[u8]) -> String {
    if value.kind() != "call" {
        return String::new();
    }
    if let Some(model) = python_query_row_model(value, bytes) {
        return model;
    }
    let Some(func) = value.child_by_field_name("function") else {
        return String::new();
    };
    match func.kind() {
        "identifier" => {
            let name = text(func, bytes);
            if name.chars().next().is_some_and(char::is_uppercase) && is_simple_ident(name) {
                name.to_string()
            } else {
                String::new()
            }
        }
        "attribute" => {
            let method = func
                .child_by_field_name("attribute")
                .map(|a| text(a, bytes))
                .unwrap_or("");
            if !matches!(method, "get" | "resolve") {
                return String::new();
            }
            let Some(args) = value.child_by_field_name("arguments") else {
                return String::new();
            };
            let mut cursor = args.walk();
            let positional: Vec<Node> = args.named_children(&mut cursor).collect();
            let on_session = func
                .child_by_field_name("object")
                .is_some_and(|o| text(o, bytes).ends_with("session"));
            match positional.as_slice() {
                // SQLAlchemy's `session.get(Model, id)`.
                [model, _, ..] if on_session && method == "get" && model.kind() == "identifier" => {
                    let name = text(*model, bytes);
                    if name.chars().next().is_some_and(char::is_uppercase) {
                        name.to_string()
                    } else {
                        String::new()
                    }
                }
                [only] if only.kind() == "identifier" => {
                    let name = text(*only, bytes);
                    if name.chars().next().is_some_and(char::is_uppercase) {
                        name.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        }
        _ => String::new(),
    }
}

/// `Model.query.filter_by(…).first()` → `Model`: a call whose method returns
/// one row, on a chain rooted at `Model.query`.
fn python_query_row_model(call: Node, bytes: &[u8]) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    if func.kind() != "attribute" {
        return None;
    }
    let method = text(func.child_by_field_name("attribute")?, bytes);
    if !matches!(
        method,
        "get" | "first" | "one" | "one_or_none" | "first_or_404" | "get_or_404"
    ) {
        return None;
    }
    // Walk down the receiver chain (`a.b(…).c(…)`) to `Model.query`.
    let mut node = func.child_by_field_name("object")?;
    loop {
        match node.kind() {
            "call" => node = node.child_by_field_name("function")?,
            "attribute" => {
                let object = node.child_by_field_name("object")?;
                if text(node.child_by_field_name("attribute")?, bytes) == "query"
                    && object.kind() == "identifier"
                {
                    let model = text(object, bytes);
                    return (model.chars().next().is_some_and(char::is_uppercase)
                        && is_simple_ident(model))
                    .then(|| model.to_string());
                }
                node = object;
            }
            _ => return None,
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
                        qn.clone(),
                        child,
                        format!("{}{}", name, params),
                        None,
                    );
                    // Scan the body (block or expression-bodied) for calls and
                    // `new` instantiations, attributed to this member.
                    csharp_scan_body(child, bytes, &qn, out);
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

/// Recursively emit `calls` / `instantiates` by-name references found anywhere
/// under `node`, attributed to the enclosing member `from_qn`. Text-based name
/// extraction keeps it robust across grammar versions: the simple (unqualified,
/// non-generic) name is what the caller resolves against project defs.
fn csharp_scan_body(node: Node, bytes: &[u8], from_qn: &str, out: &mut Extraction) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "invocation_expression" => {
                if let Some(func) = child.child_by_field_name("function") {
                    let name = csharp_last_name(func, bytes);
                    if is_simple_ident(&name) {
                        out.edges
                            .push(edge("calls", &name, from_qn, line_of(child)));
                    }
                }
                // Arguments can hold further calls / `new`s.
                csharp_scan_body(child, bytes, from_qn, out);
            }
            "object_creation_expression" => {
                if let Some(ty) = child.child_by_field_name("type") {
                    let name = csharp_last_name(ty, bytes);
                    if is_simple_ident(&name) {
                        out.edges
                            .push(edge("instantiates", &name, from_qn, line_of(child)));
                    }
                }
                csharp_scan_body(child, bytes, from_qn, out);
            }
            _ => csharp_scan_body(child, bytes, from_qn, out),
        }
    }
}

/// The simple name a C# type/callee reduces to: strip generic args (`<…>`) and
/// namespace/receiver qualifiers (`a.b.C` → `C`, `obj.Method` → `Method`).
fn csharp_last_name(node: Node, bytes: &[u8]) -> String {
    let raw = text(node, bytes);
    let base = raw.split('<').next().unwrap_or(raw);
    base.rsplit('.').next().unwrap_or(base).trim().to_string()
}

/// A plausible bare identifier — filters out empty or punctuation-bearing
/// leftovers from the text-based name reduction.
fn is_simple_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
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
