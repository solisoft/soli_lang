//! Walk a Soli project's source and extract a [`ProjectGraph`].
//!
//! The build is a static analysis over the parsed AST (via [`solilang::parse`])
//! plus the executed route table (via [`crate::serve::route_listing`]), with no
//! database or embedding I/O — that happens later in [`crate::graph::sync`].
//!
//! Passes:
//! 1. **nodes** — one node per file / class / method / function / enum /
//!    interface, plus `defines` edges (containment) and the raw import /
//!    inherit / implement / relation records.
//! 2. **resolve structure** — turn inherit/implement/relation records into
//!    edges (creating `external` stub nodes for out-of-project bases).
//! 3. **calls** — walk method/function bodies for high-confidence `calls`,
//!    `instantiates` and `renders` edges.
//! 4. **routes** — one `route` node per registered route + `routes_to` edges
//!    into the controller action (or the controller unit as a fallback).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::expr::{Argument, Expr, ExprKind};
use crate::ast::stmt::{
    ClassDecl, EnumDecl, FunctionDecl, InterfaceDecl, MethodDecl, Program, Stmt, StmtKind,
};
use crate::graph::model::{sanitize_key, Edge, Node, ProjectGraph};

const MAX_SNIPPET: usize = 1200;
const MAX_TEXT: usize = 2000;

/// One route as plain strings.
pub struct RouteRef {
    pub method: String,
    pub path: String,
    pub handler: String,
}

/// A `Send`-safe snapshot of the route table. `Route` itself is `!Send` (it
/// holds `Value`s in `middleware`), so the dev reindex thread — and any caller
/// crossing a thread boundary — takes this string-only form instead.
pub struct RouteSnapshot {
    pub routes: Vec<RouteRef>,
    pub websockets: Vec<RouteRef>,
}

impl RouteSnapshot {
    /// Build from the route lister's output (used by the standalone CLI path).
    pub fn from_listing(listing: &crate::serve::route_listing::RouteListing) -> Self {
        RouteSnapshot {
            routes: listing
                .routes
                .iter()
                .map(|r| RouteRef {
                    method: r.method.clone(),
                    path: r.path_pattern.clone(),
                    handler: r.handler_name.clone(),
                })
                .collect(),
            websockets: listing
                .websockets
                .iter()
                .map(|w| RouteRef {
                    method: "WS".to_string(),
                    path: w.path_pattern.clone(),
                    handler: w.handler_name.clone(),
                })
                .collect(),
        }
    }
}

/// A parsed `.sl` file kept around for the body-walk pass.
struct ParsedFile {
    relpath: String,
    program: Program,
}

/// A controller class id paired with its `(action, method_id)` list.
type ClassActions = (String, Vec<(String, String)>);

/// Source text of a file plus its pre-split lines, threaded through the pass-1
/// extractors (source for snippets, lines for leading-doc comments).
#[derive(Clone, Copy)]
struct Src<'a> {
    source: &'a str,
    lines: &'a [&'a str],
}

/// A controller's routable unit: the node routes point at (a controller class,
/// or the file itself for function-style controllers) and its action → node map.
struct ControllerUnit {
    unit_id: String,
    actions: HashMap<String, String>,
}

/// Read-side context threaded through the body walker in pass 3.
struct WalkCtx {
    caller_id: String,
    enclosing_class: Option<String>,
    /// Controller key used to expand bare `render("index")` calls.
    render_prefix: Option<String>,
    relpath: String,
}

#[derive(Default)]
struct GraphBuilder {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    node_ids: HashSet<String>,
    id_to_key: HashMap<String, String>,
    used_keys: HashSet<String>,

    // Resolution indexes.
    class_by_name: HashMap<String, String>,
    functions_by_name: HashMap<String, Vec<String>>,

    // Deferred structural records resolved in pass 2.
    inherits: Vec<(String, String, String, u32)>, // (child_id, super_name, relpath, line)
    implements: Vec<(String, String, String, u32)>, // (child_id, iface_name, relpath, line)
    relations: Vec<(String, String, String, String, u32)>, // (class_id, target, dsl, relpath, line)

    // Per-file bookkeeping for controller-unit resolution.
    controller_files: Vec<String>,
    file_class_actions: HashMap<String, Vec<ClassActions>>,
    file_functions: HashMap<String, Vec<(String, String)>>,

    controllers: HashMap<String, ControllerUnit>,

    unresolved_calls: usize,
    resolved_imports: usize,
}

/// Build the code-graph for the Soli app rooted at `app_path`.
pub fn build_graph(app_path: &Path) -> Result<ProjectGraph, String> {
    build_graph_inner(app_path, None, &mut |_, _| {})
}

/// Like [`build_graph`], but reports parse progress as `(files_done, total)`
/// after each source file — the CPU-bound pass that scales with project size.
pub fn build_graph_with_progress(
    app_path: &Path,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<ProjectGraph, String> {
    build_graph_inner(app_path, None, on_progress)
}

/// Like [`build_graph`], but with the route table supplied by the caller
/// instead of executing `config/routes.sl`. Used by the dev-server auto-reindex
/// so it never re-runs the routing DSL (which would append to the process-global
/// WebSocket registry the live server depends on).
pub fn build_graph_with_routes(
    app_path: &Path,
    routes: &RouteSnapshot,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<ProjectGraph, String> {
    build_graph_inner(app_path, Some(routes), on_progress)
}

fn build_graph_inner(
    app_path: &Path,
    routes: Option<&RouteSnapshot>,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<ProjectGraph, String> {
    if !app_path.is_dir() {
        return Err(format!("Folder '{}' does not exist", app_path.display()));
    }

    let mut builder = GraphBuilder::default();
    let mut parsed: Vec<ParsedFile> = Vec::new();
    let mut file_hashes: HashMap<String, String> = HashMap::new();

    // Pass 1: nodes. `.slv` views are added inline; `.sl` files are parsed and
    // walked, then retained for the body pass.
    let files = gather_source_files(app_path);
    let total = files.len();
    for (index, path) in files.into_iter().enumerate() {
        on_progress(index + 1, total);
        let relpath = relpath(app_path, &path);
        let role = role_for_path(&relpath);
        let is_view = path.extension().and_then(|e| e.to_str()) == Some("slv");
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        file_hashes.insert(relpath.clone(), md5_hex(&source));
        if is_view {
            builder.add_view(&relpath, &source);
            continue;
        }
        let program = match crate::parse(&source) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Warning: skipping {} (parse error: {})", relpath, e);
                builder.ensure_file_node(&relpath, &role, &source);
                continue;
            }
        };
        builder.ensure_file_node(&relpath, &role, &source);
        if role == "controller" {
            builder.controller_files.push(relpath.clone());
        }
        let lines: Vec<&str> = source.lines().collect();
        let src = Src {
            source: &source,
            lines: &lines,
        };
        for stmt in &program.statements {
            builder.extract_top_stmt(stmt, &relpath, &role, src, None);
        }
        parsed.push(ParsedFile { relpath, program });
    }

    // Pass 2: resolve inherit/implement/relation records into edges.
    builder.resolve_structure();

    // Controller units (needs all nodes from pass 1).
    builder.build_controller_units();

    // Pass 3: call / instantiate / render edges from bodies.
    for file in &parsed {
        let prefix = controller_key_from_relpath(&file.relpath);
        builder.walk_bodies(&file.program, &file.relpath, prefix.as_deref());
    }

    // Pass 4: routes. Use the caller-supplied table when given (dev reindex);
    // otherwise execute `config/routes.sl` via the route lister.
    match routes {
        Some(snapshot) => builder.extract_routes_from(snapshot),
        None => {
            // Executing config/routes.sl runs app code that may print to stdout
            // (the `soli new` scaffold ends routes.sl with `print("Routes
            // loaded!")`). Capture and discard that output so the command's
            // stdout stays clean — critical for `--dry-run` JSON and so it
            // never collides with the progress bar.
            let listing = {
                let _capture = crate::interpreter::builtins::StdoutCaptureGuard::start();
                crate::serve::route_listing::collect_routes(app_path)
            };
            if let Ok(listing) = listing {
                builder.extract_routes_from(&RouteSnapshot::from_listing(&listing));
            }
        }
    }

    Ok(ProjectGraph {
        nodes: builder.nodes,
        edges: builder.edges,
        unresolved_calls: builder.unresolved_calls,
        resolved_imports: builder.resolved_imports,
        file_hashes,
    })
}

/// Hex-encoded MD5 of a source file's content, used for the incremental-build
/// manifest (skip a re-build when no file changed).
pub(crate) fn md5_hex(source: &str) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(source.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

impl GraphBuilder {
    /// Intern a node under `id`, assigning a unique SolidB key. Duplicate ids
    /// are ignored (first definition wins). Returns the assigned key.
    fn intern(&mut self, id: &str, mut node: Node) -> String {
        if let Some(existing) = self.id_to_key.get(id) {
            return existing.clone();
        }
        let mut key = sanitize_key(id);
        if self.used_keys.contains(&key) {
            let base = key.clone();
            let mut n = 2;
            while self.used_keys.contains(&key) {
                key = format!("{}_{}", base, n);
                n += 1;
            }
        }
        node.key = key.clone();
        self.used_keys.insert(key.clone());
        self.node_ids.insert(id.to_string());
        self.id_to_key.insert(id.to_string(), key.clone());
        self.nodes.push(node);
        key
    }

    fn has_node(&self, id: &str) -> bool {
        self.node_ids.contains(id)
    }

    /// Push an edge, resolving both endpoint ids to keys. Silently dropped if
    /// either endpoint is unknown.
    fn push_edge(
        &mut self,
        from_id: &str,
        to_id: &str,
        kind: &str,
        relation: &str,
        file: &str,
        line: u32,
    ) {
        let (Some(from), Some(to)) = (self.id_to_key.get(from_id), self.id_to_key.get(to_id))
        else {
            return;
        };
        self.edges.push(Edge {
            from: from.clone(),
            to: to.clone(),
            edge_kind: kind.to_string(),
            relation: relation.to_string(),
            file: file.to_string(),
            line,
        });
    }

    /// Create (once) an `external` stub node for an out-of-project name so that
    /// `inherits`/`implements` edges to framework bases (Model, Object, …)
    /// still land on a queryable node. Returns its id.
    fn ensure_external(&mut self, name: &str) -> String {
        let id = format!("external:{}", name);
        if !self.has_node(&id) {
            self.intern(
                &id,
                Node {
                    key: String::new(),
                    kind: "external".to_string(),
                    name: name.to_string(),
                    qualified_name: name.to_string(),
                    file: String::new(),
                    line: 0,
                    signature: String::new(),
                    superclass: None,
                    role: String::new(),
                    doc: String::new(),
                    text: format!("external {}", name),
                    embedding: vec![],
                },
            );
        }
        id
    }

    fn ensure_file_node(&mut self, relpath: &str, role: &str, source: &str) {
        let id = format!("file:{}", relpath);
        if self.has_node(&id) {
            return;
        }
        let name = relpath.rsplit('/').next().unwrap_or(relpath).to_string();
        let snippet = snippet(source, 0, source.len());
        let text = compose_text("file", relpath, "", "", &snippet);
        self.intern(
            &id,
            Node {
                key: String::new(),
                kind: "file".to_string(),
                name,
                qualified_name: relpath.to_string(),
                file: relpath.to_string(),
                line: 0,
                signature: String::new(),
                superclass: None,
                role: role.to_string(),
                doc: String::new(),
                text,
                embedding: vec![],
            },
        );
    }

    fn add_view(&mut self, relpath: &str, source: &str) {
        let logical = view_logical_name(relpath);
        let id = format!("view:{}", logical);
        if self.has_node(&id) {
            return;
        }
        let name = logical.rsplit('/').next().unwrap_or(&logical).to_string();
        let snippet = snippet(source, 0, source.len());
        let text = compose_text("view", &logical, "", "", &snippet);
        self.intern(
            &id,
            Node {
                key: String::new(),
                kind: "view".to_string(),
                name,
                qualified_name: logical,
                file: relpath.to_string(),
                line: 0,
                signature: String::new(),
                superclass: None,
                role: "view".to_string(),
                doc: String::new(),
                text,
                embedding: vec![],
            },
        );
    }

    /// Extract a top-level declaration (unwrapping `export`).
    fn extract_top_stmt(
        &mut self,
        stmt: &Stmt,
        relpath: &str,
        role: &str,
        src: Src,
        class_prefix: Option<&str>,
    ) {
        match &stmt.kind {
            StmtKind::Export(inner) => {
                self.extract_top_stmt(inner, relpath, role, src, class_prefix)
            }
            StmtKind::Class(decl) => self.extract_class(decl, relpath, role, src, class_prefix),
            StmtKind::Function(decl) => self.extract_function(decl, relpath, role, src),
            StmtKind::Enum(decl) => self.extract_enum(decl, relpath, role, src),
            StmtKind::Interface(decl) => self.extract_interface(decl, relpath, src),
            StmtKind::Import(decl) => {
                self.resolve_import(relpath, &decl.path, decl.span.line);
            }
            _ => {}
        }
    }

    fn extract_class(
        &mut self,
        decl: &ClassDecl,
        relpath: &str,
        role: &str,
        src: Src,
        class_prefix: Option<&str>,
    ) {
        let qualified = match class_prefix {
            Some(p) => format!("{}::{}", p, decl.name),
            None => decl.name.clone(),
        };
        let id = format!("class:{}", qualified);
        let file_id = format!("file:{}", relpath);
        let kind = match role {
            "model" => "model",
            "controller" => "controller",
            _ => "class",
        };
        let signature = class_signature(decl);
        let doc = leading_doc(src.lines, decl.span.line);
        let snip = snippet(src.source, decl.span.start_usize(), decl.span.end_usize());
        let text = compose_text(kind, &qualified, &signature, &doc, &snip);
        self.intern(
            &id,
            Node {
                key: String::new(),
                kind: kind.to_string(),
                name: decl.name.clone(),
                qualified_name: qualified.clone(),
                file: relpath.to_string(),
                line: decl.span.line,
                signature,
                superclass: decl.superclass.clone(),
                role: role.to_string(),
                doc,
                text,
                embedding: vec![],
            },
        );
        self.class_by_name
            .entry(decl.name.clone())
            .or_insert_with(|| id.clone());
        self.push_edge(&file_id, &id, "defines", "", relpath, decl.span.line);

        if let Some(sc) = &decl.superclass {
            self.inherits
                .push((id.clone(), sc.clone(), relpath.to_string(), decl.span.line));
        }
        for iface in &decl.interfaces {
            self.implements.push((
                id.clone(),
                iface.clone(),
                relpath.to_string(),
                decl.span.line,
            ));
        }

        // Model relationship DSL calls (has_many, belongs_to, edge, …).
        for stmt in &decl.class_statements {
            self.extract_relation(&id, stmt, relpath);
        }

        // Methods (+ constructor as `new`).
        let mut actions: Vec<(String, String)> = Vec::new();
        if let Some(ctor) = &decl.constructor {
            let mid = format!("method:{}#new", qualified);
            let sig = format!("new({})", params_sig(&ctor.params));
            let mdoc = leading_doc(src.lines, ctor.span.line);
            let msnip = snippet(src.source, ctor.span.start_usize(), ctor.span.end_usize());
            let mtext = compose_text("method", &format!("{}#new", qualified), &sig, &mdoc, &msnip);
            self.intern(
                &mid,
                Node {
                    key: String::new(),
                    kind: "method".to_string(),
                    name: "new".to_string(),
                    qualified_name: format!("{}#new", qualified),
                    file: relpath.to_string(),
                    line: ctor.span.line,
                    signature: sig,
                    superclass: None,
                    role: role.to_string(),
                    doc: mdoc,
                    text: mtext,
                    embedding: vec![],
                },
            );
            self.push_edge(&id, &mid, "defines", "", relpath, ctor.span.line);
        }
        for m in &decl.methods {
            let mid = self.extract_method(m, &qualified, &id, relpath, role, src);
            actions.push((m.name.clone(), mid));
        }
        self.file_class_actions
            .entry(relpath.to_string())
            .or_default()
            .push((id.clone(), actions));

        for nested in &decl.nested_classes {
            self.extract_class(nested, relpath, role, src, Some(&qualified));
        }
    }

    fn extract_method(
        &mut self,
        m: &MethodDecl,
        class_qualified: &str,
        class_id: &str,
        relpath: &str,
        role: &str,
        src: Src,
    ) -> String {
        let qualified = format!("{}#{}", class_qualified, m.name);
        let id = format!("method:{}", qualified);
        let signature = method_signature(m);
        let doc = leading_doc(src.lines, m.span.line);
        let snip = snippet(src.source, m.span.start_usize(), m.span.end_usize());
        let text = compose_text("method", &qualified, &signature, &doc, &snip);
        self.intern(
            &id,
            Node {
                key: String::new(),
                kind: "method".to_string(),
                name: m.name.clone(),
                qualified_name: qualified,
                file: relpath.to_string(),
                line: m.span.line,
                signature,
                superclass: None,
                role: role.to_string(),
                doc,
                text,
                embedding: vec![],
            },
        );
        self.push_edge(class_id, &id, "defines", "", relpath, m.span.line);
        id
    }

    fn extract_function(&mut self, decl: &FunctionDecl, relpath: &str, role: &str, src: Src) {
        // File-qualified id: top-level `def index` repeats across controllers.
        let id = format!("function:{}#{}", relpath, decl.name);
        let file_id = format!("file:{}", relpath);
        // For function-style controllers, show the action as `posts#index` so
        // it reads like the route handler; helpers/services keep the bare name.
        let qualified_name = match controller_key_from_relpath(relpath) {
            Some(ckey) => format!("{}#{}", ckey, decl.name),
            None => decl.name.clone(),
        };
        let signature = function_signature(decl);
        let doc = leading_doc(src.lines, decl.span.line);
        let snip = snippet(src.source, decl.span.start_usize(), decl.span.end_usize());
        let text = compose_text("function", &qualified_name, &signature, &doc, &snip);
        self.intern(
            &id,
            Node {
                key: String::new(),
                kind: "function".to_string(),
                name: decl.name.clone(),
                qualified_name,
                file: relpath.to_string(),
                line: decl.span.line,
                signature,
                superclass: None,
                role: role.to_string(),
                doc,
                text,
                embedding: vec![],
            },
        );
        self.push_edge(&file_id, &id, "defines", "", relpath, decl.span.line);
        self.functions_by_name
            .entry(decl.name.clone())
            .or_default()
            .push(id.clone());
        self.file_functions
            .entry(relpath.to_string())
            .or_default()
            .push((decl.name.clone(), id));
    }

    fn extract_enum(&mut self, decl: &EnumDecl, relpath: &str, role: &str, src: Src) {
        let id = format!("enum:{}", decl.name);
        let file_id = format!("file:{}", relpath);
        let doc = leading_doc(src.lines, decl.span.line);
        let snip = snippet(src.source, decl.span.start_usize(), decl.span.end_usize());
        let text = compose_text("enum", &decl.name, "", &doc, &snip);
        self.intern(
            &id,
            Node {
                key: String::new(),
                kind: "enum".to_string(),
                name: decl.name.clone(),
                qualified_name: decl.name.clone(),
                file: relpath.to_string(),
                line: decl.span.line,
                signature: format!("enum {}", decl.name),
                superclass: None,
                role: role.to_string(),
                doc,
                text,
                embedding: vec![],
            },
        );
        self.class_by_name
            .entry(decl.name.clone())
            .or_insert_with(|| id.clone());
        self.push_edge(&file_id, &id, "defines", "", relpath, decl.span.line);
        for m in &decl.methods {
            self.extract_method(m, &decl.name, &id, relpath, role, src);
        }
    }

    fn extract_interface(&mut self, decl: &InterfaceDecl, relpath: &str, src: Src) {
        let id = format!("interface:{}", decl.name);
        let file_id = format!("file:{}", relpath);
        let doc = leading_doc(src.lines, decl.span.line);
        let snip = snippet(src.source, decl.span.start_usize(), decl.span.end_usize());
        let text = compose_text("interface", &decl.name, "", &doc, &snip);
        self.intern(
            &id,
            Node {
                key: String::new(),
                kind: "interface".to_string(),
                name: decl.name.clone(),
                qualified_name: decl.name.clone(),
                file: relpath.to_string(),
                line: decl.span.line,
                signature: format!("interface {}", decl.name),
                superclass: None,
                role: String::new(),
                doc,
                text,
                embedding: vec![],
            },
        );
        self.push_edge(&file_id, &id, "defines", "", relpath, decl.span.line);
    }

    /// Record a model relationship DSL call as a deferred `relates` record.
    fn extract_relation(&mut self, class_id: &str, stmt: &Stmt, relpath: &str) {
        let StmtKind::Expression(expr) = &stmt.kind else {
            return;
        };
        let ExprKind::Call { callee, arguments } = &expr.kind else {
            return;
        };
        let ExprKind::Variable(dsl) = &callee.kind else {
            return;
        };
        let line = expr.span.line;
        match dsl.as_str() {
            "has_many" | "has_one" | "belongs_to" | "has_and_belongs_to_many" => {
                let Some(assoc) = first_string_or_symbol(arguments) else {
                    return;
                };
                let target = named_string_arg(arguments, "class_name")
                    .unwrap_or_else(|| pascalize(&crate::inflect::singularize(&assoc)));
                self.relations.push((
                    class_id.to_string(),
                    target,
                    dsl.clone(),
                    relpath.to_string(),
                    line,
                ));
            }
            "edge" => {
                for key in ["from", "to"] {
                    if let Some(coll) = named_string_arg(arguments, key) {
                        let target = pascalize(&crate::inflect::singularize(&coll));
                        self.relations.push((
                            class_id.to_string(),
                            target,
                            "edge".to_string(),
                            relpath.to_string(),
                            line,
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    /// Resolve a local `import "..."` into a file→file edge when it lands on a
    /// project file node.
    fn resolve_import(&mut self, importer_relpath: &str, import_path: &str, line: u32) {
        // Only relative imports point at project files; builtin modules
        // (`import "erb"`) and stdlib absolutes are skipped.
        if !(import_path.starts_with("./") || import_path.starts_with("../")) {
            return;
        }
        let importer_dir = Path::new(importer_relpath)
            .parent()
            .unwrap_or(Path::new(""));
        let joined = importer_dir.join(import_path);
        let normalized = normalize_rel(&joined);
        let target_id = format!("file:{}", normalized);
        if self.has_node(&target_id) {
            let from = format!("file:{}", importer_relpath);
            self.push_edge(&from, &target_id, "imports", "", importer_relpath, line);
            self.resolved_imports += 1;
        }
    }

    fn resolve_structure(&mut self) {
        let inherits = std::mem::take(&mut self.inherits);
        for (child, super_name, relpath, line) in inherits {
            let to = self
                .class_by_name
                .get(&super_name)
                .cloned()
                .unwrap_or_else(|| self.ensure_external(&super_name));
            self.push_edge(&child, &to, "inherits", "", &relpath, line);
        }
        let implements = std::mem::take(&mut self.implements);
        for (child, iface, relpath, line) in implements {
            let to = format!("interface:{}", iface);
            let to = if self.has_node(&to) {
                to
            } else {
                self.ensure_external(&iface)
            };
            self.push_edge(&child, &to, "implements", "", &relpath, line);
        }
        let relations = std::mem::take(&mut self.relations);
        for (class_id, target, dsl, relpath, line) in relations {
            if let Some(to) = self.class_by_name.get(&target).cloned() {
                self.push_edge(&class_id, &to, "relates", &dsl, &relpath, line);
            }
        }
    }

    fn build_controller_units(&mut self) {
        let files = std::mem::take(&mut self.controller_files);
        for relpath in files {
            let ckey = controller_key_from_relpath(&relpath).unwrap_or_else(|| relpath.clone());
            let (unit_id, actions) = if let Some(classes) = self.file_class_actions.get(&relpath) {
                if let Some((class_id, acts)) = classes.first() {
                    (class_id.clone(), acts.iter().cloned().collect())
                } else {
                    (format!("file:{}", relpath), HashMap::new())
                }
            } else if let Some(fns) = self.file_functions.get(&relpath) {
                (format!("file:{}", relpath), fns.iter().cloned().collect())
            } else {
                (format!("file:{}", relpath), HashMap::new())
            };
            self.controllers
                .insert(ckey, ControllerUnit { unit_id, actions });
        }
    }

    // ---- Pass 3: body walk -------------------------------------------------

    fn walk_bodies(&mut self, program: &Program, relpath: &str, render_prefix: Option<&str>) {
        for stmt in &program.statements {
            self.walk_top_for_bodies(stmt, relpath, render_prefix);
        }
    }

    fn walk_top_for_bodies(&mut self, stmt: &Stmt, relpath: &str, render_prefix: Option<&str>) {
        match &stmt.kind {
            StmtKind::Export(inner) => self.walk_top_for_bodies(inner, relpath, render_prefix),
            StmtKind::Function(decl) => {
                let ctx = WalkCtx {
                    caller_id: format!("function:{}#{}", relpath, decl.name),
                    enclosing_class: None,
                    render_prefix: render_prefix.map(str::to_string),
                    relpath: relpath.to_string(),
                };
                self.walk_block(&decl.body, &ctx);
            }
            StmtKind::Class(decl) => self.walk_class_bodies(decl, None, relpath, render_prefix),
            StmtKind::Enum(decl) => {
                for m in &decl.methods {
                    let ctx = WalkCtx {
                        caller_id: format!("method:{}#{}", decl.name, m.name),
                        enclosing_class: Some(decl.name.clone()),
                        render_prefix: render_prefix.map(str::to_string),
                        relpath: relpath.to_string(),
                    };
                    self.walk_block(&m.body, &ctx);
                }
            }
            _ => {}
        }
    }

    fn walk_class_bodies(
        &mut self,
        decl: &ClassDecl,
        prefix: Option<&str>,
        relpath: &str,
        render_prefix: Option<&str>,
    ) {
        let qualified = match prefix {
            Some(p) => format!("{}::{}", p, decl.name),
            None => decl.name.clone(),
        };
        if let Some(ctor) = &decl.constructor {
            let ctx = WalkCtx {
                caller_id: format!("method:{}#new", qualified),
                enclosing_class: Some(qualified.clone()),
                render_prefix: render_prefix.map(str::to_string),
                relpath: relpath.to_string(),
            };
            self.walk_block(&ctor.body, &ctx);
        }
        for m in &decl.methods {
            let ctx = WalkCtx {
                caller_id: format!("method:{}#{}", qualified, m.name),
                enclosing_class: Some(qualified.clone()),
                render_prefix: render_prefix.map(str::to_string),
                relpath: relpath.to_string(),
            };
            self.walk_block(&m.body, &ctx);
        }
        for nested in &decl.nested_classes {
            self.walk_class_bodies(nested, Some(&qualified), relpath, render_prefix);
        }
    }

    fn walk_block(&mut self, stmts: &[Stmt], ctx: &WalkCtx) {
        for stmt in stmts {
            self.walk_stmt(stmt, ctx);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt, ctx: &WalkCtx) {
        match &stmt.kind {
            StmtKind::Expression(e) => self.walk_expr(e, ctx),
            StmtKind::Let {
                initializer: Some(e),
                ..
            } => self.walk_expr(e, ctx),
            StmtKind::Const { initializer, .. } => self.walk_expr(initializer, ctx),
            StmtKind::Block(stmts) => self.walk_block(stmts, ctx),
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(condition, ctx);
                self.walk_stmt(then_branch, ctx);
                if let Some(e) = else_branch {
                    self.walk_stmt(e, ctx);
                }
            }
            StmtKind::While { condition, body } => {
                self.walk_expr(condition, ctx);
                self.walk_stmt(body, ctx);
            }
            StmtKind::For { iterable, body, .. } => {
                self.walk_expr(iterable, ctx);
                self.walk_stmt(body, ctx);
            }
            StmtKind::Return(Some(e)) => self.walk_expr(e, ctx),
            StmtKind::Throw(e) => self.walk_expr(e, ctx),
            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                self.walk_stmt(try_block, ctx);
                for c in catch_clauses {
                    self.walk_stmt(&c.body, ctx);
                }
                if let Some(f) = finally_block {
                    self.walk_stmt(f, ctx);
                }
            }
            // Nested function/class declarations inside a body are rare; their
            // own bodies are handled where declared at the top level.
            _ => {}
        }
    }

    fn walk_expr(&mut self, expr: &Expr, ctx: &WalkCtx) {
        match &expr.kind {
            ExprKind::Call { callee, arguments } => {
                self.resolve_call(callee, arguments, expr.span.line, ctx);
                self.walk_expr(callee, ctx);
                for a in arguments {
                    self.walk_argument(a, ctx);
                }
            }
            ExprKind::New {
                class_expr,
                arguments,
            } => {
                if let Some(cname) = simple_name(class_expr) {
                    if let Some(to) = self.class_by_name.get(&cname).cloned() {
                        self.push_edge(
                            &ctx.caller_id,
                            &to,
                            "instantiates",
                            "",
                            &ctx.relpath,
                            expr.span.line,
                        );
                    }
                }
                for a in arguments {
                    self.walk_argument(a, ctx);
                }
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::LogicalAnd { left, right }
            | ExprKind::LogicalOr { left, right }
            | ExprKind::NullishCoalescing { left, right }
            | ExprKind::Pipeline { left, right } => {
                self.walk_expr(left, ctx);
                self.walk_expr(right, ctx);
            }
            ExprKind::Unary { operand, .. } => self.walk_expr(operand, ctx),
            ExprKind::Grouping(e)
            | ExprKind::Spread(e)
            | ExprKind::Throw(e)
            | ExprKind::PostfixIncrement(e)
            | ExprKind::PostfixDecrement(e) => self.walk_expr(e, ctx),
            ExprKind::Member { object, .. } | ExprKind::SafeMember { object, .. } => {
                self.walk_expr(object, ctx)
            }
            ExprKind::QualifiedName { qualifier, .. } => self.walk_expr(qualifier, ctx),
            ExprKind::Index { object, index } => {
                self.walk_expr(object, ctx);
                self.walk_expr(index, ctx);
            }
            ExprKind::Assign { target, value } => {
                self.walk_expr(target, ctx);
                self.walk_expr(value, ctx);
            }
            ExprKind::CompoundAssign { target, value, .. } => {
                self.walk_expr(target, ctx);
                self.walk_expr(value, ctx);
            }
            ExprKind::Array(items) => {
                for e in items {
                    self.walk_expr(e, ctx);
                }
            }
            ExprKind::Hash(pairs) => {
                for (k, v) in pairs {
                    self.walk_expr(k, ctx);
                    self.walk_expr(v, ctx);
                }
            }
            ExprKind::Block(stmts) => self.walk_block(stmts, ctx),
            ExprKind::Lambda { body, .. } => self.walk_block(body, ctx),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(condition, ctx);
                self.walk_expr(then_branch, ctx);
                if let Some(e) = else_branch {
                    self.walk_expr(e, ctx);
                }
            }
            ExprKind::Match { expression, arms } => {
                self.walk_expr(expression, ctx);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.walk_expr(g, ctx);
                    }
                    self.walk_expr(&arm.body, ctx);
                }
            }
            ExprKind::ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                self.walk_expr(element, ctx);
                self.walk_expr(iterable, ctx);
                if let Some(c) = condition {
                    self.walk_expr(c, ctx);
                }
            }
            ExprKind::HashComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                self.walk_expr(key, ctx);
                self.walk_expr(value, ctx);
                self.walk_expr(iterable, ctx);
                if let Some(c) = condition {
                    self.walk_expr(c, ctx);
                }
            }
            ExprKind::Rescue { expr, fallback } => {
                self.walk_expr(expr, ctx);
                self.walk_expr(fallback, ctx);
            }
            _ => {}
        }
    }

    fn walk_argument(&mut self, arg: &Argument, ctx: &WalkCtx) {
        match arg {
            Argument::Positional(e) | Argument::Block(e) => self.walk_expr(e, ctx),
            Argument::Named(named) => self.walk_expr(&named.value, ctx),
        }
    }

    /// Resolve a single call site into a `calls` or `renders` edge, when it is
    /// high-confidence. Everything else is left untouched.
    fn resolve_call(&mut self, callee: &Expr, arguments: &[Argument], line: u32, ctx: &WalkCtx) {
        match &callee.kind {
            // `render("view", ...)` inside a controller action.
            ExprKind::Variable(name) if name == "render" => {
                if let Some(view) = render_target(arguments, ctx.render_prefix.as_deref()) {
                    let to = format!("view:{}", view);
                    if self.has_node(&to) {
                        self.push_edge(&ctx.caller_id, &to, "renders", "", &ctx.relpath, line);
                    }
                }
            }
            // Bare `foo(...)` — a project function if there's exactly one.
            ExprKind::Variable(name) => {
                if let Some(ids) = self.functions_by_name.get(name) {
                    match ids.len() {
                        1 => {
                            let to = ids[0].clone();
                            self.push_edge(&ctx.caller_id, &to, "calls", "", &ctx.relpath, line);
                        }
                        n if n > 1 => self.unresolved_calls += 1,
                        _ => {}
                    }
                }
            }
            // `Klass.method(...)` (static) or `this.method(...)`.
            ExprKind::Member { object, name } => match &object.kind {
                // Call on a known class → its method (or the class as a fallback
                // when the method isn't one we extracted).
                ExprKind::Variable(recv) if self.class_by_name.contains_key(recv) => {
                    let target = format!("method:{}#{}", recv, name);
                    if self.has_node(&target) {
                        self.push_edge(&ctx.caller_id, &target, "calls", "", &ctx.relpath, line);
                    } else if let Some(class_id) = self.class_by_name.get(recv).cloned() {
                        self.push_edge(&ctx.caller_id, &class_id, "calls", "", &ctx.relpath, line);
                    }
                }
                ExprKind::This => {
                    if let Some(class) = &ctx.enclosing_class {
                        let target = format!("method:{}#{}", class, name);
                        if self.has_node(&target) {
                            self.push_edge(
                                &ctx.caller_id,
                                &target,
                                "calls",
                                "",
                                &ctx.relpath,
                                line,
                            );
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    // ---- Pass 4: routes ----------------------------------------------------

    fn extract_routes_from(&mut self, snapshot: &RouteSnapshot) {
        let routes_file = "config/routes.sl".to_string();
        let has_routes_file = self.has_node(&format!("file:{}", routes_file));

        for route in &snapshot.routes {
            self.add_route_node(
                &route.method,
                &route.path,
                &route.handler,
                &routes_file,
                has_routes_file,
            );
        }
        for ws in &snapshot.websockets {
            self.add_route_node("WS", &ws.path, &ws.handler, &routes_file, has_routes_file);
        }
    }

    fn add_route_node(
        &mut self,
        method: &str,
        path: &str,
        handler: &str,
        routes_file: &str,
        link_file: bool,
    ) {
        let name = format!("{} {}", method, path);
        let id = format!("route:{}", name);
        let text = compose_text("route", &name, handler, "", "");
        self.intern(
            &id,
            Node {
                key: String::new(),
                kind: "route".to_string(),
                name: name.clone(),
                qualified_name: handler.to_string(),
                file: routes_file.to_string(),
                line: 0,
                signature: handler.to_string(),
                superclass: None,
                role: "route".to_string(),
                doc: String::new(),
                text,
                embedding: vec![],
            },
        );
        if link_file {
            let file_id = format!("file:{}", routes_file);
            self.push_edge(&file_id, &id, "defines", "", routes_file, 0);
        }

        // routes_to → controller action (or the controller unit as a fallback).
        if let Some((ckey, action)) = handler.split_once('#') {
            if let Some(unit) = self.controllers.get(ckey) {
                let target = unit
                    .actions
                    .get(action)
                    .cloned()
                    .unwrap_or_else(|| unit.unit_id.clone());
                self.push_edge(&id, &target, "routes_to", "", routes_file, 0);
            }
        }
    }
}

// ---- Free helpers ----------------------------------------------------------

/// Collect `.sl`/`.slv` files under `app/`, `config/`, and `lib/`. Mirrors the
/// linter's `collect_lint_files` walk (skips dot/underscore dirs) but lives in
/// the library crate, which can't reach the binary-crate `cli` module.
fn gather_source_files(app_path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for sub in ["app", "config", "lib"] {
        let dir = app_path.join(sub);
        if dir.is_dir() {
            collect_source_files(&dir, &mut files);
        }
    }
    files.sort();
    files.dedup();
    files
}

fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let lintable = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|ext| ext == "sl" || ext == "slv")
                .unwrap_or(false);
            if lintable {
                out.push(path);
            }
        } else if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name.starts_with('_') {
                continue;
            }
            collect_source_files(&path, out);
        }
    }
}

fn relpath(app_path: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(app_path).unwrap_or(path);
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_rel(path: &Path) -> String {
    // Collapse `.`/`..` segments without touching the filesystem.
    let mut parts: Vec<&str> = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::Normal(s) => {
                if let Some(s) = s.to_str() {
                    parts.push(s);
                }
            }
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::CurDir => {}
            _ => {}
        }
    }
    parts.join("/")
}

fn role_for_path(relpath: &str) -> String {
    let role = if relpath.starts_with("app/models/") {
        "model"
    } else if relpath.starts_with("app/controllers/") {
        "controller"
    } else if relpath.starts_with("app/helpers/") {
        "helper"
    } else if relpath.starts_with("app/services/") {
        "service"
    } else if relpath.starts_with("app/jobs/") {
        "job"
    } else if relpath.starts_with("app/middleware/") {
        "middleware"
    } else if relpath.starts_with("app/mailers/") {
        "mailer"
    } else if relpath.starts_with("app/views/") {
        "view"
    } else {
        ""
    };
    role.to_string()
}

/// Controller key for a file under `app/controllers/` — mirrors
/// `app_loader::controller_key_from_path` on the already-normalized relpath
/// (`app/controllers/admin/users_controller.sl` → `admin/users`). `None` for
/// files outside `app/controllers/`.
fn controller_key_from_relpath(relpath: &str) -> Option<String> {
    let rest = relpath.strip_prefix("app/controllers/")?;
    let rest = rest.strip_suffix(".sl").unwrap_or(rest);
    Some(rest.trim_end_matches("_controller").to_string())
}

/// `app/views/posts/index.html.slv` → `posts/index`.
fn view_logical_name(relpath: &str) -> String {
    let stripped = relpath.strip_prefix("app/views/").unwrap_or(relpath);
    let (dir, file) = match stripped.rsplit_once('/') {
        Some((d, f)) => (Some(d), f),
        None => (None, stripped),
    };
    let base = file.split('.').next().unwrap_or(file);
    match dir {
        Some(d) => format!("{}/{}", d, base),
        None => base.to_string(),
    }
}

/// Normalize a `render(...)` string argument into a view logical name.
fn render_target(arguments: &[Argument], prefix: Option<&str>) -> Option<String> {
    let arg = first_string_or_symbol(arguments)?;
    // Strip template suffixes: `home/index.html` / `x.slv` → `home/index` / `x`.
    let mut name = arg.as_str();
    for suffix in [".slv", ".html", ".erb"] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    if name.contains('/') {
        Some(name.to_string())
    } else {
        match prefix {
            Some(p) => Some(format!("{}/{}", p, name)),
            None => Some(name.to_string()),
        }
    }
}

fn first_string_or_symbol(arguments: &[Argument]) -> Option<String> {
    let first = arguments.first()?;
    let Argument::Positional(expr) = first else {
        return None;
    };
    match &expr.kind {
        ExprKind::StringLiteral(s) | ExprKind::Symbol(s) => Some(s.clone()),
        _ => None,
    }
}

fn named_string_arg(arguments: &[Argument], key: &str) -> Option<String> {
    for arg in arguments {
        if let Argument::Named(named) = arg {
            if named.name == key {
                if let ExprKind::StringLiteral(s) | ExprKind::Symbol(s) = &named.value.kind {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

/// The class name for `new X(...)` / `new Outer::Inner(...)`.
fn simple_name(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Variable(name) => Some(name.clone()),
        ExprKind::QualifiedName { name, .. } => Some(name.clone()),
        _ => None,
    }
}

fn class_signature(decl: &ClassDecl) -> String {
    match &decl.superclass {
        Some(sc) => format!("class {} < {}", decl.name, sc),
        None => format!("class {}", decl.name),
    }
}

fn method_signature(m: &MethodDecl) -> String {
    let statik = if m.is_static { "static " } else { "" };
    let ret = m
        .return_type
        .as_ref()
        .map(|t| format!(" -> {}", t))
        .unwrap_or_default();
    format!("{}def {}({}){}", statik, m.name, params_sig(&m.params), ret)
}

fn function_signature(f: &FunctionDecl) -> String {
    let ret = f
        .return_type
        .as_ref()
        .map(|t| format!(" -> {}", t))
        .unwrap_or_default();
    format!("def {}({}){}", f.name, params_sig(&f.params), ret)
}

fn params_sig(params: &[crate::ast::stmt::Parameter]) -> String {
    params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.type_annotation))
        .collect::<Vec<_>>()
        .join(", ")
}

fn pascalize(word: &str) -> String {
    word.split(['_', '-'])
        .filter(|s| !s.is_empty())
        .map(|seg| {
            let mut chars = seg.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Collect a contiguous run of `#`/`//` comment lines immediately above `line`.
fn leading_doc(lines: &[&str], line: u32) -> String {
    if line == 0 {
        return String::new();
    }
    let mut idx = line as usize; // 1-based; lines[line-1] is the decl line
    let mut collected: Vec<String> = Vec::new();
    while idx >= 2 {
        let candidate = lines[idx - 2].trim();
        let stripped = candidate
            .strip_prefix("///")
            .or_else(|| candidate.strip_prefix("//"))
            .or_else(|| candidate.strip_prefix('#'));
        match stripped {
            Some(text) => {
                collected.push(text.trim().to_string());
                idx -= 1;
            }
            None => break,
        }
    }
    collected.reverse();
    let doc = collected.join(" ");
    truncate_on_boundary(&doc, 400)
}

/// Byte-range source slice, clamped and truncated on a char boundary.
fn snippet(source: &str, start: usize, end: usize) -> String {
    let end = end.min(source.len());
    let start = start.min(end);
    let slice = source.get(start..end).unwrap_or("");
    truncate_on_boundary(slice, MAX_SNIPPET)
}

fn compose_text(kind: &str, qualified: &str, signature: &str, doc: &str, snippet: &str) -> String {
    let mut parts = vec![format!("{} {}", kind, qualified)];
    if !signature.is_empty() {
        parts.push(signature.to_string());
    }
    if !doc.is_empty() {
        parts.push(doc.to_string());
    }
    if !snippet.is_empty() {
        parts.push(snippet.to_string());
    }
    truncate_on_boundary(&parts.join("\n"), MAX_TEXT)
}

fn truncate_on_boundary(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn build_fixture() -> (tempfile::TempDir, ProjectGraph) {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/models/user.sl",
            "# A user record.\nclass User < Model\n  has_many \"posts\"\n  def authenticate(password: String) -> Bool {\n    return true;\n  }\nend\n",
        );
        write(
            dir.path(),
            "app/models/post.sl",
            "class Post < Model\n  belongs_to \"user\"\nend\n",
        );
        write(
            dir.path(),
            "app/controllers/sessions_controller.sl",
            // `new User()` → instantiates; `User.authenticate(...)` (call on a
            // known class) → calls; `render(...)` → renders. An instance call on
            // a local variable is deliberately NOT resolved (no type inference).
            "def create(req: Any) -> Any {\n  let u = new User();\n  User.authenticate(\"x\");\n  return render(\"sessions/new\");\n}\n",
        );
        write(
            dir.path(),
            "app/views/sessions/new.html.slv",
            "<h1>Login</h1>\n",
        );
        write(
            dir.path(),
            "config/routes.sl",
            "post(\"/login\", \"sessions#create\");\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        (dir, graph)
    }

    fn has_edge(graph: &ProjectGraph, from_q: &str, to_q: &str, kind: &str) -> bool {
        let key_of = |q: &str| {
            graph
                .nodes
                .iter()
                .find(|n| n.qualified_name == q)
                .map(|n| n.key.clone())
        };
        let (Some(from), Some(to)) = (key_of(from_q), key_of(to_q)) else {
            return false;
        };
        graph
            .edges
            .iter()
            .any(|e| e.from == from && e.to == to && e.edge_kind == kind)
    }

    #[test]
    fn extracts_class_model_and_method_nodes() {
        let (_dir, graph) = build_fixture();
        let user = graph
            .nodes
            .iter()
            .find(|n| n.qualified_name == "User")
            .expect("User node");
        assert_eq!(user.kind, "model");
        assert_eq!(user.role, "model");
        assert_eq!(user.superclass.as_deref(), Some("Model"));
        assert!(user.doc.contains("A user record"));
        assert!(graph
            .nodes
            .iter()
            .any(|n| n.qualified_name == "User#authenticate" && n.kind == "method"));
    }

    #[test]
    fn inherits_edge_points_at_external_model_stub() {
        let (_dir, graph) = build_fixture();
        assert!(has_edge(&graph, "User", "Model", "inherits"));
        assert!(graph
            .nodes
            .iter()
            .any(|n| n.kind == "external" && n.name == "Model"));
    }

    #[test]
    fn relates_edge_from_has_many_and_belongs_to() {
        let (_dir, graph) = build_fixture();
        assert!(has_edge(&graph, "User", "Post", "relates"));
        assert!(has_edge(&graph, "Post", "User", "relates"));
    }

    #[test]
    fn routes_to_edge_targets_the_controller_action() {
        let (_dir, graph) = build_fixture();
        let route = graph
            .nodes
            .iter()
            .find(|n| n.kind == "route" && n.name == "POST /login")
            .expect("route node");
        // The route resolves to the function-based controller action `create`.
        let create = graph
            .nodes
            .iter()
            .find(|n| n.kind == "function" && n.name == "create")
            .expect("create fn");
        assert!(graph
            .edges
            .iter()
            .any(|e| e.from == route.key && e.to == create.key && e.edge_kind == "routes_to"));
    }

    #[test]
    fn instantiates_calls_and_renders_edges() {
        let (_dir, graph) = build_fixture();
        // create() instantiates User, calls User#authenticate, renders the view.
        let create_key = graph
            .nodes
            .iter()
            .find(|n| n.kind == "function" && n.name == "create")
            .unwrap()
            .key
            .clone();
        let user_key = graph
            .nodes
            .iter()
            .find(|n| n.qualified_name == "User")
            .unwrap()
            .key
            .clone();
        assert!(graph
            .edges
            .iter()
            .any(|e| e.from == create_key && e.to == user_key && e.edge_kind == "instantiates"));
        assert!(has_edge(
            &graph,
            "sessions#create",
            "User#authenticate",
            "calls"
        ));
        let view_key = graph
            .nodes
            .iter()
            .find(|n| n.kind == "view")
            .unwrap()
            .key
            .clone();
        assert!(graph
            .edges
            .iter()
            .any(|e| e.from == create_key && e.to == view_key && e.edge_kind == "renders"));
    }

    #[test]
    fn view_node_logical_name() {
        let (_dir, graph) = build_fixture();
        assert!(graph
            .nodes
            .iter()
            .any(|n| n.kind == "view" && n.qualified_name == "sessions/new"));
    }
}
