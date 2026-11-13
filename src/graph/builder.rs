//! Walk a Soli project's source and extract a [`ProjectGraph`].
//!
//! The build is a static analysis over the parsed AST (via [`solilang::parse`])
//! plus the executed route table (via [`crate::serve::route_listing`]), with no
//! database or embedding I/O — that happens later in [`crate::graph::sync`].
//!
//! Passes:
//! 1. **nodes** — one node per file / class / method / function / enum /
//!    interface, plus `defines` edges (containment) and the raw import /
//!    inherit / implement / relation records. Views record `partial("…")`
//!    string targets for a post-pass.
//! 2. **resolve structure** — turn inherit/implement/relation records into
//!    edges (creating `external` stub nodes for out-of-project bases); link
//!    view → partial `renders` edges once every view node exists.
//! 3. **calls** — walk method/function bodies for high-confidence `calls`,
//!    `instantiates`, `renders` (incl. `partial`), and deferred `redirects`
//!    edges. Local class bindings (`new User()`, typed lets, model finders)
//!    unlock precision-first instance method edges.
//! 4. **routes** — one `route` node per registered route + `routes_to` edges
//!    into the controller action (or the controller unit as a fallback);
//!    then resolve deferred `redirects` against route paths.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::expr::{Argument, Expr, ExprKind};
use crate::ast::stmt::{
    ClassDecl, EnumDecl, FunctionDecl, InterfaceDecl, MethodDecl, Program, Stmt, StmtKind,
};
use crate::ast::types::TypeKind;
use crate::graph::model::{sanitize_key, Edge, Node, ProjectGraph};

/// Model / class static methods whose return value is treated as an instance
/// of the receiver class for local type tracking.
const CLASS_INSTANCE_FACTORIES: &[&str] = &[
    "find",
    "find_by",
    "first",
    "first_by",
    "create",
    "new",
    "find_or_create_by",
];

/// Model statics that read the model's collection (`Post.where(…)`).
const MODEL_READS: &[&str] = &[
    "all",
    "find",
    "find_by",
    "first",
    "first_by",
    "last",
    "where",
    "count",
    "exists",
    "sum",
    "avg",
    "min",
    "max",
    "pluck",
    "order",
    "limit",
    "offset",
    "includes",
    "search",
    "rag",
    "paginate",
    "group_by",
    "distinct",
    "select",
    "find_or_create_by",
];

/// Model statics that write the model's collection.
const MODEL_WRITES: &[&str] = &[
    "create",
    "create_many",
    "insert",
    "upsert",
    "update",
    "update_all",
    "delete",
    "delete_all",
    "destroy",
    "destroy_all",
    "increment",
    "decrement",
    "import",
];

/// Instance methods that write the record's collection (`post.save`).
const RECORD_WRITES: &[&str] = &[
    "save",
    "update",
    "delete",
    "destroy",
    "increment",
    "decrement",
    "touch",
];

/// Calls whose first string argument is raw SDBQL (`db.query("FOR …")`).
const RAW_QUERY_CALLS: &[&str] = &["query", "sdbql", "execute", "raw_query"];

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
    /// Simple method name (`authenticate`) for bare `super(...)` resolution.
    enclosing_method: Option<String>,
    /// Superclass name of the enclosing class (`ApplicationController`).
    superclass: Option<String>,
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
    /// Exact route path → route node id (first registration wins).
    route_by_path: HashMap<String, String>,

    // Deferred structural records resolved in pass 2.
    inherits: Vec<(String, String, String, u32)>, // (child_id, super_name, relpath, line)
    implements: Vec<(String, String, String, u32)>, // (child_id, iface_name, relpath, line)
    relations: Vec<(String, String, String, String, u32)>, // (class_id, target, dsl, relpath, line)
    /// View partial targets collected during pass 1: (view_id, target, file, line).
    view_partials: Vec<(String, String, String, u32)>,
    /// `redirect(path)` sites resolved after routes exist: (caller, path, file, line).
    deferred_redirects: Vec<(String, String, String, u32)>,

    // Per-file bookkeeping for controller-unit resolution.
    controller_files: Vec<String>,
    file_class_actions: HashMap<String, Vec<ClassActions>>,
    file_functions: HashMap<String, Vec<(String, String)>>,

    controllers: HashMap<String, ControllerUnit>,

    unresolved_calls: usize,
    resolved_imports: usize,

    /// Simple class name → its superclass, for model detection and STI roots.
    class_super: HashMap<String, Option<String>>,
    /// Model class name → the collection it is stored in.
    model_collections: HashMap<String, String>,
    /// View id → the `@x` its code reads (its own assignments aside).
    view_reads: HashMap<String, BTreeSet<String>>,
    /// The callable being walked in pass 3: the `@x` it assigns and the views
    /// it renders by name.
    walk_assigned: BTreeSet<String>,
    walk_rendered: Vec<String>,
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
    // Models → the collections they live in (`stored_in`), so pass 3 can
    // point a model's queries at its collection.
    builder.index_models();
    // View → partial `renders` edges once every view node is interned.
    builder.link_view_partials();

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
    // `redirect(path)` edges need the route index built above.
    builder.resolve_redirects();

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
        let (reads, assigned) = crate::template::ivars::ivar_reads(source);
        let reads: BTreeSet<String> = reads
            .into_iter()
            .map(|read| read.name)
            .filter(|name| !assigned.contains(name))
            .collect();
        if !reads.is_empty() {
            self.view_reads.insert(id.clone(), reads);
        }
        // Record partial("…") targets; resolved after every view is interned.
        for (target, line) in scan_partial_calls(source) {
            self.view_partials
                .push((id.clone(), target, relpath.to_string(), line));
        }
    }

    /// Emit view → partial `renders` edges for every recorded `partial("…")`.
    fn link_view_partials(&mut self) {
        let partials = std::mem::take(&mut self.view_partials);
        for (view_id, target, file, line) in partials {
            let normalized = normalize_view_name(&target);
            // A bare `partial("form")` resolves relative to the including view's
            // own directory first (like controller `render` with a prefix),
            // then falls back to the name as given.
            let resolved = if normalized.contains('/') {
                self.resolve_view_id(&normalized)
            } else {
                view_dir(&view_id)
                    .and_then(|dir| self.resolve_view_id(&format!("{}/{}", dir, normalized)))
                    .or_else(|| self.resolve_view_id(&normalized))
            };
            let Some(to) = resolved else {
                continue;
            };
            // Avoid self-loops when a template names itself by mistake.
            if to == view_id {
                continue;
            }
            self.push_edge(&view_id, &to, "renders", "", &file, line);
        }
    }

    /// Resolve a deferred `redirect(path)` against the route path index.
    fn resolve_redirects(&mut self) {
        let redirects = std::mem::take(&mut self.deferred_redirects);
        for (caller_id, path, relpath, line) in redirects {
            let clean = path.split('?').next().unwrap_or(path.as_str());
            if let Some(route_id) = self.route_by_path.get(clean).cloned() {
                self.push_edge(&caller_id, &route_id, "redirects", "", &relpath, line);
            }
        }
    }

    /// Map a logical view name (`posts/form` or `posts/_form`) onto an existing
    /// view node id, trying underscore and non-underscore variants.
    fn resolve_view_id(&self, logical: &str) -> Option<String> {
        let direct = format!("view:{}", logical);
        if self.has_node(&direct) {
            return Some(direct);
        }
        if let Some((dir, base)) = logical.rsplit_once('/') {
            if let Some(stripped) = base.strip_prefix('_') {
                let without = format!("view:{}/{}", dir, stripped);
                if self.has_node(&without) {
                    return Some(without);
                }
            } else {
                let with = format!("view:{}/_{}", dir, base);
                if self.has_node(&with) {
                    return Some(with);
                }
            }
        } else if let Some(stripped) = logical.strip_prefix('_') {
            let without = format!("view:{}", stripped);
            if self.has_node(&without) {
                return Some(without);
            }
        } else {
            let with = format!("view:_{}", logical);
            if self.has_node(&with) {
                return Some(with);
            }
        }
        None
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
            _ if decl.is_module => "module",
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
        self.class_super
            .entry(decl.name.clone())
            .or_insert_with(|| decl.superclass.clone());
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
                    enclosing_method: Some(decl.name.clone()),
                    superclass: None,
                    render_prefix: render_prefix.map(str::to_string),
                    relpath: relpath.to_string(),
                };
                let action = render_prefix.is_some() && !decl.name.starts_with('_');
                self.walk_callable(&decl.body, &ctx, action);
            }
            StmtKind::Class(decl) => self.walk_class_bodies(decl, None, relpath, render_prefix),
            StmtKind::Enum(decl) => {
                for m in &decl.methods {
                    let ctx = WalkCtx {
                        caller_id: format!("method:{}#{}", decl.name, m.name),
                        enclosing_class: Some(decl.name.clone()),
                        enclosing_method: Some(m.name.clone()),
                        superclass: None,
                        render_prefix: render_prefix.map(str::to_string),
                        relpath: relpath.to_string(),
                    };
                    let mut locals = HashMap::new();
                    self.walk_block(&m.body, &ctx, &mut locals);
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
        let superclass = decl.superclass.clone();
        if let Some(ctor) = &decl.constructor {
            let ctx = WalkCtx {
                caller_id: format!("method:{}#new", qualified),
                enclosing_class: Some(qualified.clone()),
                enclosing_method: Some("new".to_string()),
                superclass: superclass.clone(),
                render_prefix: render_prefix.map(str::to_string),
                relpath: relpath.to_string(),
            };
            self.walk_callable(&ctor.body, &ctx, false);
        }
        for m in &decl.methods {
            let ctx = WalkCtx {
                caller_id: format!("method:{}#{}", qualified, m.name),
                enclosing_class: Some(qualified.clone()),
                enclosing_method: Some(m.name.clone()),
                superclass: superclass.clone(),
                render_prefix: render_prefix.map(str::to_string),
                relpath: relpath.to_string(),
            };
            // A public, non-`_` instance method of a controller is an action:
            // it renders its own view when it names none.
            let action = render_prefix.is_some()
                && !m.is_static
                && !m.name.starts_with('_')
                && matches!(m.visibility, crate::ast::stmt::Visibility::Public);
            self.walk_callable(&m.body, &ctx, action);
        }
        for nested in &decl.nested_classes {
            self.walk_class_bodies(nested, Some(&qualified), relpath, render_prefix);
        }
    }

    /// Walk one method or function body, then link an action to the views it
    /// renders: its own view when it names none (`renders`, relation
    /// `implicit`), and an `assigns` edge per `@x` it sets that the view reads.
    fn walk_callable(&mut self, body: &[Stmt], ctx: &WalkCtx, action: bool) {
        self.walk_assigned.clear();
        self.walk_rendered.clear();
        let mut locals = HashMap::new();
        self.walk_block(body, ctx, &mut locals);
        if !action {
            return;
        }
        let mut views = std::mem::take(&mut self.walk_rendered);
        if views.is_empty() {
            if let (Some(prefix), Some(method)) = (&ctx.render_prefix, &ctx.enclosing_method) {
                let own = format!("view:{}/{}", prefix, method);
                if self.has_node(&own) {
                    self.push_edge(&ctx.caller_id, &own, "renders", "implicit", &ctx.relpath, 0);
                    views.push(own);
                }
            }
        }
        let assigned = std::mem::take(&mut self.walk_assigned);
        for view in views {
            let Some(reads) = self.view_reads.get(&view).cloned() else {
                continue;
            };
            for name in reads.intersection(&assigned) {
                self.push_edge(
                    &ctx.caller_id,
                    &view,
                    "assigns",
                    &format!("@{}", name),
                    &ctx.relpath,
                    0,
                );
            }
        }
    }

    /// A method `name` of `class` or of one of its project ancestors.
    fn method_in_hierarchy(&self, class: &str, name: &str) -> Option<String> {
        let mut current = class.to_string();
        for _ in 0..16 {
            let id = format!("method:{}#{}", current, name);
            if self.has_node(&id) {
                return Some(id);
            }
            let simple = current.rsplit("::").next().unwrap_or(&current).to_string();
            current = self.class_super.get(&simple)?.clone()?;
        }
        None
    }

    /// Whether `class` (a simple name) descends from `Model`.
    fn is_model(&self, class: &str) -> bool {
        let mut current = class.to_string();
        for _ in 0..16 {
            match self.class_super.get(&current) {
                Some(Some(parent)) if parent == "Model" => return true,
                Some(Some(parent)) => current = parent.clone(),
                _ => return false,
            }
        }
        false
    }

    /// A model's collection: the one of its STI root (`Admin < User` →
    /// `users`), named as the ORM names it.
    fn index_models(&mut self) {
        let mut names: Vec<String> = self.class_super.keys().cloned().collect();
        names.sort();
        for name in names {
            if !self.is_model(&name) {
                continue;
            }
            let mut root = name.clone();
            while let Some(Some(parent)) = self.class_super.get(&root) {
                if parent == "Model" || !self.is_model(parent) {
                    break;
                }
                root = parent.clone();
            }
            let collection =
                crate::interpreter::builtins::model::core::class_name_to_collection_with_engine(
                    &root, None,
                );
            let coll_id = self.ensure_collection(&collection);
            if let Some(class_id) = self.class_by_name.get(&name).cloned() {
                self.push_edge(&class_id, &coll_id, "stored_in", "", "", 0);
            }
            self.model_collections.insert(name, collection);
        }
    }

    /// The `collection:<name>` node, created on first use.
    fn ensure_collection(&mut self, name: &str) -> String {
        let id = format!("collection:{}", name);
        if !self.has_node(&id) {
            self.intern(
                &id,
                Node {
                    key: String::new(),
                    kind: "collection".to_string(),
                    name: name.to_string(),
                    qualified_name: name.to_string(),
                    file: String::new(),
                    line: 0,
                    signature: String::new(),
                    superclass: None,
                    role: String::new(),
                    doc: String::new(),
                    text: format!("collection {} (database table / collection)", name),
                    embedding: vec![],
                },
            );
        }
        id
    }

    /// A `reads` / `writes` edge from the current callable to `collection`,
    /// with the operation (`where`, `save`, `FOR`, …) as its relation.
    fn db_edge(&mut self, ctx: &WalkCtx, collection: &str, write: bool, op: &str, line: u32) {
        let to = self.ensure_collection(collection);
        let kind = if write { "writes" } else { "reads" };
        self.push_edge(&ctx.caller_id, &to, kind, op, &ctx.relpath, line);
    }

    /// The collections a raw SDBQL query touches.
    fn sdbql_edges(&mut self, query: &str, ctx: &WalkCtx, line: u32) {
        for (collection, write, op) in sdbql_collections(query) {
            self.db_edge(ctx, &collection, write, op, line);
        }
    }

    fn walk_block(&mut self, stmts: &[Stmt], ctx: &WalkCtx, locals: &mut HashMap<String, String>) {
        for stmt in stmts {
            self.walk_stmt(stmt, ctx, locals);
        }
    }

    fn walk_stmt(&mut self, stmt: &Stmt, ctx: &WalkCtx, locals: &mut HashMap<String, String>) {
        match &stmt.kind {
            StmtKind::Expression(e) => self.walk_expr(e, ctx, locals),
            StmtKind::Let {
                name,
                type_annotation,
                initializer,
            } => {
                if let Some(e) = initializer {
                    self.walk_expr(e, ctx, locals);
                }
                // Prefer an explicit Named type annotation; fall back to
                // inferring from the initializer (`new User()`, `User.find`).
                // A re-declaration with no inferable class clears any prior
                // binding, so a shadowed name never keeps a stale type.
                let inferred = type_annotation
                    .as_ref()
                    .and_then(named_class_type)
                    .filter(|c| self.class_by_name.contains_key(c))
                    .or_else(|| initializer.as_ref().and_then(|e| self.class_from_expr(e)));
                bind_local(locals, name, inferred);
            }
            StmtKind::Const {
                name,
                type_annotation,
                initializer,
            } => {
                self.walk_expr(initializer, ctx, locals);
                let inferred = type_annotation
                    .as_ref()
                    .and_then(named_class_type)
                    .filter(|c| self.class_by_name.contains_key(c))
                    .or_else(|| self.class_from_expr(initializer));
                bind_local(locals, name, inferred);
            }
            // Nested scopes: clone so bindings inside don't leak outward.
            StmtKind::Block(stmts) => {
                let mut nested = locals.clone();
                self.walk_block(stmts, ctx, &mut nested);
            }
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            }
            | StmtKind::Unless {
                condition,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(condition, ctx, locals);
                let mut then_locals = locals.clone();
                self.walk_stmt(then_branch, ctx, &mut then_locals);
                if let Some(e) = else_branch {
                    let mut else_locals = locals.clone();
                    self.walk_stmt(e, ctx, &mut else_locals);
                }
            }
            StmtKind::While { condition, body } => {
                self.walk_expr(condition, ctx, locals);
                let mut nested = locals.clone();
                self.walk_stmt(body, ctx, &mut nested);
            }
            StmtKind::For { iterable, body, .. } => {
                self.walk_expr(iterable, ctx, locals);
                let mut nested = locals.clone();
                self.walk_stmt(body, ctx, &mut nested);
            }
            StmtKind::Return(Some(e)) => self.walk_expr(e, ctx, locals),
            StmtKind::Throw(e) => self.walk_expr(e, ctx, locals),
            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                let mut try_locals = locals.clone();
                self.walk_stmt(try_block, ctx, &mut try_locals);
                for c in catch_clauses {
                    let mut catch_locals = locals.clone();
                    self.walk_stmt(&c.body, ctx, &mut catch_locals);
                }
                if let Some(f) = finally_block {
                    let mut finally_locals = locals.clone();
                    self.walk_stmt(f, ctx, &mut finally_locals);
                }
            }
            // Nested function/class declarations inside a body are rare; their
            // own bodies are handled where declared at the top level.
            _ => {}
        }
    }

    fn walk_expr(&mut self, expr: &Expr, ctx: &WalkCtx, locals: &mut HashMap<String, String>) {
        match &expr.kind {
            ExprKind::Call { callee, arguments } => {
                // `User.new(...)` instantiates, as `new User(...)` does.
                if let ExprKind::Member { object, name } = &callee.kind {
                    if name == "new" {
                        if let Some(to) = simple_name(object)
                            .and_then(|cname| self.class_by_name.get(&cname).cloned())
                        {
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
                }
                self.resolve_call(callee, arguments, expr.span.line, ctx, locals);
                let called = match &callee.kind {
                    ExprKind::Variable(name) | ExprKind::Member { name, .. } => Some(name.as_str()),
                    _ => None,
                };
                if called.is_some_and(|name| RAW_QUERY_CALLS.contains(&name)) {
                    if let Some(query) = arguments.iter().find_map(|a| match a {
                        Argument::Positional(e) => literal_text(e),
                        _ => None,
                    }) {
                        self.sdbql_edges(&query, ctx, expr.span.line);
                    }
                }
                // The callee was resolved above as a call: walk only its
                // receiver, so the member arm below does not resolve it again
                // as a parenthesis-less call.
                match &callee.kind {
                    ExprKind::Member { object, .. } | ExprKind::SafeMember { object, .. } => {
                        self.walk_expr(object, ctx, locals)
                    }
                    _ => self.walk_expr(callee, ctx, locals),
                }
                for a in arguments {
                    self.walk_argument(a, ctx, locals);
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
                    self.walk_argument(a, ctx, locals);
                }
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::LogicalAnd { left, right }
            | ExprKind::LogicalOr { left, right }
            | ExprKind::NullishCoalescing { left, right }
            | ExprKind::Pipeline { left, right } => {
                self.walk_expr(left, ctx, locals);
                self.walk_expr(right, ctx, locals);
            }
            ExprKind::Unary { operand, .. } => self.walk_expr(operand, ctx, locals),
            ExprKind::Grouping(e)
            | ExprKind::Spread(e)
            | ExprKind::Throw(e)
            | ExprKind::PostfixIncrement(e)
            | ExprKind::PostfixDecrement(e) => self.walk_expr(e, ctx, locals),
            ExprKind::Member { object, .. } => {
                // `Post.count`, `post.save`, `@publish`: a member read with no
                // parentheses calls a zero-argument method.
                self.resolve_call(expr, &[], expr.span.line, ctx, locals);
                self.walk_expr(object, ctx, locals)
            }
            ExprKind::SafeMember { object, .. } => self.walk_expr(object, ctx, locals),
            ExprKind::QualifiedName { qualifier, .. } => self.walk_expr(qualifier, ctx, locals),
            ExprKind::Index { object, index } => {
                self.walk_expr(object, ctx, locals);
                self.walk_expr(index, ctx, locals);
            }
            ExprKind::SdqlBlock { query, .. } => self.sdbql_edges(query, ctx, expr.span.line),
            ExprKind::Assign { target, value } => {
                if let ExprKind::Member { object, name } = &target.kind {
                    if matches!(object.kind, ExprKind::This) {
                        self.walk_assigned.insert(name.clone());
                    }
                }
                self.walk_expr(target, ctx, locals);
                self.walk_expr(value, ctx, locals);
                // Reassigning a bare local updates (or clears) its tracked type
                // so a later `local.method()` never resolves against a class the
                // variable no longer holds.
                if let ExprKind::Variable(name) = &target.kind {
                    let inferred = self.class_from_expr(value);
                    bind_local(locals, name, inferred);
                }
            }
            ExprKind::CompoundAssign { target, value, .. } => {
                self.walk_expr(target, ctx, locals);
                self.walk_expr(value, ctx, locals);
            }
            ExprKind::Array(items) => {
                for e in items {
                    self.walk_expr(e, ctx, locals);
                }
            }
            ExprKind::Hash(pairs) => {
                for (k, v) in pairs {
                    self.walk_expr(k, ctx, locals);
                    self.walk_expr(v, ctx, locals);
                }
            }
            ExprKind::Block(stmts) => {
                let mut nested = locals.clone();
                self.walk_block(stmts, ctx, &mut nested);
            }
            ExprKind::Lambda { body, .. } => {
                let mut nested = locals.clone();
                self.walk_block(body, ctx, &mut nested);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(condition, ctx, locals);
                // Clone per branch so a binding made in one branch doesn't leak
                // into the other branch or past the expression (matches the
                // statement-level `if`).
                let mut then_locals = locals.clone();
                self.walk_expr(then_branch, ctx, &mut then_locals);
                if let Some(e) = else_branch {
                    let mut else_locals = locals.clone();
                    self.walk_expr(e, ctx, &mut else_locals);
                }
            }
            ExprKind::Match { expression, arms } => {
                self.walk_expr(expression, ctx, locals);
                for arm in arms {
                    // Each arm gets its own scope; bindings don't leak across arms.
                    let mut arm_locals = locals.clone();
                    if let Some(g) = &arm.guard {
                        self.walk_expr(g, ctx, &mut arm_locals);
                    }
                    self.walk_expr(&arm.body, ctx, &mut arm_locals);
                }
            }
            ExprKind::ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                self.walk_expr(element, ctx, locals);
                self.walk_expr(iterable, ctx, locals);
                if let Some(c) = condition {
                    self.walk_expr(c, ctx, locals);
                }
            }
            ExprKind::HashComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                self.walk_expr(key, ctx, locals);
                self.walk_expr(value, ctx, locals);
                self.walk_expr(iterable, ctx, locals);
                if let Some(c) = condition {
                    self.walk_expr(c, ctx, locals);
                }
            }
            ExprKind::Rescue { expr, fallback } => {
                self.walk_expr(expr, ctx, locals);
                self.walk_expr(fallback, ctx, locals);
            }
            _ => {}
        }
    }

    fn walk_argument(
        &mut self,
        arg: &Argument,
        ctx: &WalkCtx,
        locals: &mut HashMap<String, String>,
    ) {
        match arg {
            Argument::Positional(e) | Argument::Block(e) => self.walk_expr(e, ctx, locals),
            Argument::Named(named) => self.walk_expr(&named.value, ctx, locals),
        }
    }

    /// High-confidence class type of an expression for local binding.
    /// Only `new Class(...)` and known-class factory statics (`User.find`).
    fn class_from_expr(&self, expr: &Expr) -> Option<String> {
        match &expr.kind {
            ExprKind::New { class_expr, .. } => {
                let name = simple_name(class_expr)?;
                if self.class_by_name.contains_key(&name) {
                    Some(name)
                } else {
                    None
                }
            }
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::Member { object, name }
                    if CLASS_INSTANCE_FACTORIES.contains(&name.as_str()) =>
                {
                    if let ExprKind::Variable(recv) = &object.kind {
                        if self.class_by_name.contains_key(recv) {
                            return Some(recv.clone());
                        }
                    }
                    None
                }
                _ => None,
            },
            ExprKind::Grouping(inner) => self.class_from_expr(inner),
            _ => None,
        }
    }

    /// Resolve a single call site into a `calls` / `renders` / deferred
    /// `redirects` edge, when it is high-confidence. Everything else is left
    /// untouched.
    fn resolve_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        line: u32,
        ctx: &WalkCtx,
        locals: &HashMap<String, String>,
    ) {
        match &callee.kind {
            // `render("view", ...)` / `partial("view", ...)` inside a controller.
            ExprKind::Variable(name) if name == "render" || name == "partial" => {
                if let Some(view) = render_target(arguments, ctx.render_prefix.as_deref()) {
                    if let Some(to) = self.resolve_view_id(&view) {
                        self.push_edge(&ctx.caller_id, &to, "renders", "", &ctx.relpath, line);
                        self.walk_rendered.push(to);
                    }
                }
            }
            // `redirect("/path")` — deferred until routes are indexed.
            ExprKind::Variable(name) if name == "redirect" => {
                if let Some(path) = first_string_or_symbol(arguments) {
                    self.deferred_redirects.push((
                        ctx.caller_id.clone(),
                        path,
                        ctx.relpath.clone(),
                        line,
                    ));
                }
            }
            // Bare `super(...)` → parent method of the same name.
            ExprKind::Super => {
                if let (Some(super_name), Some(method)) =
                    (ctx.superclass.as_ref(), ctx.enclosing_method.as_ref())
                {
                    let target = format!("method:{}#{}", super_name, method);
                    if self.has_node(&target) {
                        self.push_edge(&ctx.caller_id, &target, "calls", "", &ctx.relpath, line);
                    }
                }
            }
            // Bare `foo(...)` — a project function if there's exactly one.
            ExprKind::Variable(name) => {
                // A bare call inside a class reaches its own (or an inherited)
                // method first, as at run time.
                let own = ctx
                    .enclosing_class
                    .as_ref()
                    .and_then(|class| self.method_in_hierarchy(class, name));
                if let Some(to) = own {
                    self.push_edge(&ctx.caller_id, &to, "calls", "", &ctx.relpath, line);
                } else if let Some(ids) = self.functions_by_name.get(name) {
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
            // `Klass.method(...)` (static), `this.method(...)`,
            // `local.method(...)` (typed local), or `super.method(...)`.
            ExprKind::Member { object, name } => match &object.kind {
                ExprKind::Variable(recv) if self.class_by_name.contains_key(recv) => {
                    let target = format!("method:{}#{}", recv, name);
                    if self.has_node(&target) {
                        self.push_edge(&ctx.caller_id, &target, "calls", "", &ctx.relpath, line);
                    } else if let Some(class_id) = self.class_by_name.get(recv).cloned() {
                        self.push_edge(&ctx.caller_id, &class_id, "calls", "", &ctx.relpath, line);
                    }
                    if let Some(collection) = self.model_collections.get(recv).cloned() {
                        if MODEL_WRITES.contains(&name.as_str()) {
                            self.db_edge(ctx, &collection, true, name, line);
                        } else if MODEL_READS.contains(&name.as_str())
                            || name.starts_with("find_by_")
                        {
                            self.db_edge(ctx, &collection, false, name, line);
                        }
                    }
                }
                ExprKind::Variable(recv) => {
                    // Instance call on a locally-typed variable — method only
                    // (no class fallback; avoids inventing edges for Hashes).
                    if let Some(class) = locals.get(recv) {
                        if RECORD_WRITES.contains(&name.as_str()) {
                            if let Some(collection) = self.model_collections.get(class).cloned() {
                                self.db_edge(ctx, &collection, true, name, line);
                            }
                        }
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
                ExprKind::This => {
                    if let Some(class) = ctx.enclosing_class.clone() {
                        if let Some(target) = self.method_in_hierarchy(&class, name) {
                            self.push_edge(
                                &ctx.caller_id,
                                &target,
                                "calls",
                                "",
                                &ctx.relpath,
                                line,
                            );
                        }
                        // `@save` / `this.update(…)` inside a model writes
                        // its own collection.
                        let simple = class.rsplit("::").next().unwrap_or(&class).to_string();
                        if RECORD_WRITES.contains(&name.as_str()) {
                            if let Some(collection) = self.model_collections.get(&simple).cloned() {
                                self.db_edge(ctx, &collection, true, name, line);
                            }
                        }
                    }
                }
                ExprKind::Super => {
                    if let Some(super_name) = &ctx.superclass {
                        let target = format!("method:{}#{}", super_name, name);
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
        // Index the path for `redirect(path)` matching. A redirect targets a
        // GET route, so prefer GET when several verbs share a path; otherwise
        // the first registration wins.
        let is_get = method.eq_ignore_ascii_case("GET");
        match self.route_by_path.get(path) {
            None => {
                self.route_by_path.insert(path.to_string(), id.clone());
            }
            Some(existing) if is_get && !existing.starts_with("route:GET ") => {
                self.route_by_path.insert(path.to_string(), id.clone());
            }
            Some(_) => {}
        }
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

/// Normalize a `render(...)` / `partial(...)` string argument into a view
/// logical name (`posts/index`, `sessions/_form`).
fn render_target(arguments: &[Argument], prefix: Option<&str>) -> Option<String> {
    let arg = first_string_or_symbol(arguments)?;
    let name = normalize_view_name(&arg);
    if name.contains('/') {
        Some(name)
    } else {
        match prefix {
            Some(p) => Some(format!("{}/{}", p, name)),
            None => Some(name),
        }
    }
}

/// Directory of a `view:<dir>/<name>` id, if any (`view:posts/index` → `posts`).
fn view_dir(view_id: &str) -> Option<&str> {
    view_id
        .strip_prefix("view:")?
        .rsplit_once('/')
        .map(|(dir, _)| dir)
}

/// Strip template suffixes from a view/partial path argument.
fn normalize_view_name(arg: &str) -> String {
    let mut name = arg;
    for suffix in [".slv", ".html", ".erb"] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    name.to_string()
}

/// Record `name`'s tracked class, or clear a stale one when the value has no
/// inferable class (e.g. reassigned to a hash or DB row). Keeps local type
/// tracking precise across re-declaration and reassignment.
fn bind_local(locals: &mut HashMap<String, String>, name: &str, class: Option<String>) {
    match class {
        Some(c) => {
            locals.insert(name.to_string(), c);
        }
        None => {
            locals.remove(name);
        }
    }
}

/// Extract a Named class type from an annotation (`User`, `User?`).
fn named_class_type(ann: &crate::ast::types::TypeAnnotation) -> Option<String> {
    match &ann.kind {
        TypeKind::Named(name) => Some(name.clone()),
        TypeKind::Nullable(inner) => named_class_type(inner),
        _ => None,
    }
}

/// Scan a template/source string for `partial("…")` / `partial('…')` call sites.
/// Line numbers are 1-based. Deliberately lightweight — no full template parse.
fn scan_partial_calls(source: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = source[search_from..].find("partial(") {
        let abs = search_from + rel;
        search_from = abs + "partial(".len();
        // Require a word boundary before `partial` so a method or helper whose
        // name merely ends in `partial(` (`render_partial(`, `my_partial(`)
        // doesn't masquerade as the `partial(...)` builtin.
        if abs > 0 {
            let prev = bytes[abs - 1];
            if prev == b'_' || prev.is_ascii_alphanumeric() {
                continue;
            }
        }
        // 1-based line of the match start.
        let line = source[..abs].bytes().filter(|&b| b == b'\n').count() as u32 + 1;
        let mut i = search_from;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let quote = bytes[i];
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote && bytes[i] != b'\n' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == quote {
                let name = &source[start..i];
                if !name.is_empty() {
                    out.push((name.to_string(), line));
                }
            }
        }
    }
    out
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
/// The text of a string literal, or of an interpolated string's literal parts
/// (`"FOR p IN posts FILTER p.id == #{id}"` → its query shape).
fn literal_text(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::StringLiteral(text) => Some(text.clone()),
        ExprKind::InterpolatedString(parts) => Some(
            parts
                .iter()
                .map(|part| match part {
                    crate::ast::expr::InterpolatedPart::Literal(text) => text.as_str(),
                    crate::ast::expr::InterpolatedPart::Expression(_) => " _ ",
                })
                .collect(),
        ),
        _ => None,
    }
}

/// SDBQL keywords that can follow `IN` without naming a collection.
const SDBQL_KEYWORDS: &[&str] = &[
    "FOR",
    "IN",
    "INTO",
    "LET",
    "FILTER",
    "RETURN",
    "SORT",
    "LIMIT",
    "COLLECT",
    "INSERT",
    "UPDATE",
    "REPLACE",
    "REMOVE",
    "UPSERT",
    "WITH",
    "OUTBOUND",
    "INBOUND",
    "ANY",
    "GRAPH",
    "DISTINCT",
    "AGGREGATE",
    "OPTIONS",
    "SEARCH",
    "WINDOW",
    "TRUE",
    "FALSE",
    "NULL",
    "NOT",
    "AND",
    "OR",
    "LIKE",
    "ASC",
    "DESC",
];

/// The collections a raw SDBQL query reads (`FOR x IN posts`, a traversal's
/// edge collection) and writes (`INSERT … INTO`, `UPDATE`/`REPLACE`/`REMOVE`/
/// `UPSERT … IN`), each with the keyword that touches it. Names bound by
/// `LET` or `FOR`, bind parameters (`@@coll`) and expressions are skipped.
fn sdbql_collections(query: &str) -> Vec<(String, bool, &'static str)> {
    // Tokens: identifiers (dotted paths split), strings as one opaque token,
    // everything else one character each.
    let mut tokens: Vec<String> = Vec::new();
    let chars: Vec<char> = query.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(chars[start..i].iter().collect());
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            let quote = c;
            i += 1;
            while i < chars.len() && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            tokens.push("\"".to_string());
            i += 1;
            continue;
        }
        if !c.is_whitespace() {
            tokens.push(c.to_string());
        }
        i += 1;
    }

    let upper = |t: &str| t.to_ascii_uppercase();
    let is_name = |t: &str| {
        t.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && !SDBQL_KEYWORDS.contains(&upper(t).as_str())
    };
    let mut bound: HashSet<String> = HashSet::new();
    let mut out: Vec<(String, bool, &'static str)> = Vec::new();
    let mut add = |name: &str, write: bool, op: &'static str, bound: &HashSet<String>| {
        if bound.contains(name) || out.iter().any(|(c, w, _)| c == name && *w == write) {
            return;
        }
        out.push((name.to_string(), write, op));
    };
    let mut k = 0;
    while k < tokens.len() {
        // A bind parameter (`@name`, `@@coll`) is never a collection name.
        if tokens[k] == "@" {
            k += 1;
            while k < tokens.len() && tokens[k] == "@" {
                k += 1;
            }
            k += 1;
            continue;
        }
        match upper(&tokens[k]).as_str() {
            "LET" => {
                if let Some(name) = tokens.get(k + 1) {
                    bound.insert(name.clone());
                }
            }
            "FOR" => {
                let mut j = k + 1;
                while j < tokens.len() && upper(&tokens[j]) != "IN" {
                    if is_name(&tokens[j]) {
                        bound.insert(tokens[j].clone());
                    }
                    j += 1;
                }
                if let Some(next) = tokens.get(j + 1) {
                    if is_name(next) {
                        add(next, false, "FOR", &bound);
                    } else {
                        // A traversal: `IN 1..2 OUTBOUND start edges`.
                        if let Some(d) = ((j + 1)..tokens.len().min(j + 12)).find(|&d| {
                            matches!(upper(&tokens[d]).as_str(), "OUTBOUND" | "INBOUND" | "ANY")
                        }) {
                            // The start vertex: a string, a name or `@param`.
                            let mut e = d + 1;
                            while tokens.get(e).is_some_and(|t| t == "@") {
                                e += 1;
                            }
                            if let Some(edges) = tokens.get(e + 1).filter(|t| is_name(t)) {
                                add(edges, false, "FOR", &bound);
                            }
                        }
                    }
                }
                k = j;
            }
            op @ ("INSERT" | "UPDATE" | "REPLACE" | "REMOVE" | "UPSERT") => {
                let op: &'static str = match op {
                    "INSERT" => "INSERT",
                    "UPDATE" => "UPDATE",
                    "REPLACE" => "REPLACE",
                    "REMOVE" => "REMOVE",
                    _ => "UPSERT",
                };
                if let Some(j) = ((k + 1)..tokens.len())
                    .find(|&j| matches!(upper(&tokens[j]).as_str(), "IN" | "INTO"))
                {
                    if let Some(name) = tokens.get(j + 1).filter(|t| is_name(t)) {
                        add(name, true, op, &bound);
                    }
                    // `UPSERT … INSERT … UPDATE … IN coll`: one write.
                    if op == "UPSERT" {
                        k = j;
                    }
                }
            }
            _ => {}
        }
        k += 1;
    }
    out
}

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
    fn klass_new_is_an_instantiates_edge() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/services/receipt.sl",
            "class Receipt\n  new(total)\n    @total = total\n  end\nend\n",
        );
        write(
            dir.path(),
            "app/services/checkout.sl",
            "def checkout(total)\n  Receipt.new(total)\nend\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(has_edge(&graph, "checkout", "Receipt", "instantiates"));
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

    #[test]
    fn instance_call_via_new_binding() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/models/user.sl",
            "class User < Model\n  def authenticate(password: String) -> Bool {\n    return true;\n  }\nend\n",
        );
        write(
            dir.path(),
            "app/controllers/sessions_controller.sl",
            "def create(req: Any) -> Any {\n  let u = new User();\n  u.authenticate(\"x\");\n  return null;\n}\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(has_edge(
            &graph,
            "sessions#create",
            "User#authenticate",
            "calls"
        ));
    }

    #[test]
    fn instance_call_via_type_annotation() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/models/user.sl",
            "class User < Model\n  def save() -> Bool {\n    return true;\n  }\nend\n",
        );
        write(
            dir.path(),
            "app/controllers/users_controller.sl",
            "def update(req: Any) -> Any {\n  let u: User = null;\n  u.save();\n  return null;\n}\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(has_edge(&graph, "users#update", "User#save", "calls"));
    }

    #[test]
    fn instance_call_via_model_finder() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/models/user.sl",
            "class User < Model\n  def authenticate(password: String) -> Bool {\n    return true;\n  }\nend\n",
        );
        write(
            dir.path(),
            "app/controllers/sessions_controller.sl",
            "def create(req: Any) -> Any {\n  let u = User.find(1);\n  u.authenticate(\"x\");\n  return null;\n}\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(has_edge(
            &graph,
            "sessions#create",
            "User#authenticate",
            "calls"
        ));
    }

    #[test]
    fn unbound_instance_call_stays_unlinked() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/models/user.sl",
            "class User < Model\n  def save() -> Bool {\n    return true;\n  }\nend\n",
        );
        write(
            dir.path(),
            "app/controllers/users_controller.sl",
            // `u` is never typed — precision-first: no edge.
            "def update(req: Any) -> Any {\n  let u = req[\"user\"];\n  u.save();\n  return null;\n}\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(!has_edge(&graph, "users#update", "User#save", "calls"));
    }

    #[test]
    fn partial_and_redirect_edges_from_controller() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/controllers/sessions_controller.sl",
            "def create(req: Any) -> Any {\n  partial(\"sessions/form\");\n  return redirect(\"/login\");\n}\n",
        );
        write(
            dir.path(),
            "app/views/sessions/_form.html.slv",
            "<form></form>\n",
        );
        write(
            dir.path(),
            "config/routes.sl",
            "post(\"/login\", \"sessions#create\");\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(has_edge(
            &graph,
            "sessions#create",
            "sessions/_form",
            "renders"
        ));
        // redirect → route: match by path; route qualified_name is the handler.
        let create_key = graph
            .nodes
            .iter()
            .find(|n| n.kind == "function" && n.name == "create")
            .unwrap()
            .key
            .clone();
        let route_key = graph
            .nodes
            .iter()
            .find(|n| n.kind == "route" && n.name == "POST /login")
            .unwrap()
            .key
            .clone();
        assert!(graph
            .edges
            .iter()
            .any(|e| { e.from == create_key && e.to == route_key && e.edge_kind == "redirects" }));
    }

    #[test]
    fn super_call_links_parent_method() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/models/person.sl",
            "class Person < Model\n  def greet() -> String {\n    return \"hi\";\n  }\nend\n",
        );
        write(
            dir.path(),
            "app/models/employee.sl",
            "class Employee < Person\n  def greet() -> String {\n    return super();\n  }\nend\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(has_edge(&graph, "Employee#greet", "Person#greet", "calls"));
    }

    #[test]
    fn view_partial_edge() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/views/posts/index.html.slv",
            "<% partial(\"posts/form\") %>\n",
        );
        write(
            dir.path(),
            "app/views/posts/_form.html.slv",
            "<form></form>\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(has_edge(&graph, "posts/index", "posts/_form", "renders"));
    }

    #[test]
    fn scan_partial_calls_finds_quoted_names() {
        let hits = scan_partial_calls("before\n<% partial(\"posts/form\") %>\npartial('x')");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "posts/form");
        assert_eq!(hits[0].1, 2);
        assert_eq!(hits[1].0, "x");
    }

    #[test]
    fn scan_partial_calls_ignores_suffix_matches() {
        // `render_partial(` ends in `partial(` but is a different call.
        let hits = scan_partial_calls("render_partial(\"x\")\npartial(\"y\")");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "y");
    }

    #[test]
    fn reassigned_local_drops_stale_class_edge() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/models/user.sl",
            "class User < Model\n  def save() -> Bool {\n    return true;\n  }\nend\n",
        );
        write(
            dir.path(),
            "app/controllers/users_controller.sl",
            // `u` is a User, then reassigned to a non-class value; the later
            // `u.save()` must NOT resolve against User#save.
            "def update(req: Any) -> Any {\n  let u = User.find(1);\n  u = req[\"user\"];\n  u.save();\n  return null;\n}\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(!has_edge(&graph, "users#update", "User#save", "calls"));
    }

    #[test]
    fn view_bare_partial_resolves_in_same_dir() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/views/posts/index.html.slv",
            // No directory in the name — resolves against posts/.
            "<% partial(\"form\") %>\n",
        );
        write(
            dir.path(),
            "app/views/posts/_form.html.slv",
            "<form></form>\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        assert!(has_edge(&graph, "posts/index", "posts/_form", "renders"));
    }

    #[test]
    fn redirect_prefers_get_route_over_post() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/controllers/sessions_controller.sl",
            "def create(req: Any) -> Any {\n  return redirect(\"/login\");\n}\n",
        );
        write(
            dir.path(),
            "config/routes.sl",
            // POST is registered first, but the redirect must target GET /login.
            "post(\"/login\", \"sessions#create\");\nget(\"/login\", \"sessions#new\");\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        let create_key = graph
            .nodes
            .iter()
            .find(|n| n.kind == "function" && n.name == "create")
            .unwrap()
            .key
            .clone();
        let get_route_key = graph
            .nodes
            .iter()
            .find(|n| n.kind == "route" && n.name == "GET /login")
            .unwrap()
            .key
            .clone();
        assert!(graph
            .edges
            .iter()
            .any(|e| e.from == create_key && e.to == get_route_key && e.edge_kind == "redirects"));
    }

    fn edge_relation(graph: &ProjectGraph, from_q: &str, to_q: &str, kind: &str) -> Vec<String> {
        let key_of = |q: &str| {
            graph
                .nodes
                .iter()
                .find(|n| n.qualified_name == q)
                .map(|n| n.key.clone())
        };
        let (Some(from), Some(to)) = (key_of(from_q), key_of(to_q)) else {
            return Vec::new();
        };
        let mut relations: Vec<String> = graph
            .edges
            .iter()
            .filter(|e| e.from == from && e.to == to && e.edge_kind == kind)
            .map(|e| e.relation.clone())
            .collect();
        relations.sort();
        relations
    }

    fn build_db_fixture() -> (tempfile::TempDir, ProjectGraph) {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "app/models/post.sl",
            "class Post < Model\n  def publish\n    @published = true\n    @save\n  end\nend\n",
        );
        write(
            dir.path(),
            "app/models/featured_post.sl",
            "class FeaturedPost < Post\nend\n",
        );
        write(
            dir.path(),
            "app/controllers/posts_controller.sl",
            concat!(
                "class PostsController < Controller\n",
                "  # GET /posts\n",
                "  def index(req)\n",
                "    @posts = Post.where({\"published\": true}).order(\"title\").all\n",
                "    @count = FeaturedPost.count\n",
                "    @unused = 1\n",
                "  end\n",
                "\n",
                "  # POST /posts\n",
                "  def create(req)\n",
                "    post = Post.create({\"title\": \"x\"})\n",
                "    post.save\n",
                "    _audit(post)\n",
                "    redirect(\"/posts\")\n",
                "  end\n",
                "\n",
                "  # GET /posts/stats\n",
                "  def stats(req)\n",
                "    rows = @sdbql{ FOR p IN posts FOR c IN comments FILTER c.post == p._key RETURN p }\n",
                "    db.query(\"FOR v IN 1..2 OUTBOUND @start follows RETURN v\")\n",
                "    render(\"posts/index\", {})\n",
                "  end\n",
                "\n",
                "  def _audit(post)\n",
                "    db.query(\"INSERT {at: 1} INTO audit_log\")\n",
                "  end\n",
                "end\n",
            ),
        );
        write(
            dir.path(),
            "app/views/posts/index.html.slv",
            "<% for post in @posts %><%= post.title %><% end %> <%= @count %> <% @local = 1 %><%= @local %>\n",
        );
        let graph = build_graph(dir.path()).unwrap();
        (dir, graph)
    }

    #[test]
    fn a_model_is_stored_in_its_collection_sti_in_its_roots() {
        let (_dir, graph) = build_db_fixture();
        let posts = graph
            .nodes
            .iter()
            .find(|n| n.qualified_name == "posts")
            .expect("posts collection node");
        assert_eq!(posts.kind, "collection");
        assert!(has_edge(&graph, "Post", "posts", "stored_in"));
        assert!(has_edge(&graph, "FeaturedPost", "posts", "stored_in"));
    }

    #[test]
    fn orm_calls_read_and_write_the_models_collection() {
        let (_dir, graph) = build_db_fixture();
        assert_eq!(
            edge_relation(&graph, "PostsController#index", "posts", "reads"),
            vec!["count", "where"]
        );
        assert_eq!(
            edge_relation(&graph, "PostsController#create", "posts", "writes"),
            vec!["create", "save"]
        );
        // `@save` inside the model writes its own collection.
        assert_eq!(
            edge_relation(&graph, "Post#publish", "posts", "writes"),
            vec!["save"]
        );
    }

    #[test]
    fn raw_sdbql_names_its_collections() {
        let (_dir, graph) = build_db_fixture();
        assert_eq!(
            edge_relation(&graph, "PostsController#stats", "posts", "reads"),
            vec!["FOR"]
        );
        assert_eq!(
            edge_relation(&graph, "PostsController#stats", "comments", "reads"),
            vec!["FOR"]
        );
        assert_eq!(
            edge_relation(&graph, "PostsController#stats", "follows", "reads"),
            vec!["FOR"]
        );
        assert_eq!(
            edge_relation(&graph, "PostsController#_audit", "audit_log", "writes"),
            vec!["INSERT"]
        );
    }

    #[test]
    fn sdbql_collections_skips_bound_names_and_parameters() {
        assert_eq!(
            sdbql_collections("LET ids = [1] FOR x IN ids FOR u IN @@coll FOR p IN posts RETURN p"),
            vec![("posts".to_string(), false, "FOR")]
        );
        assert_eq!(
            sdbql_collections("UPSERT {k: 1} INSERT {k: 1} UPDATE {n: 2} IN counters RETURN NEW"),
            vec![("counters".to_string(), true, "UPSERT")]
        );
        assert_eq!(
            sdbql_collections(
                "FOR u IN users FILTER u.name == \"FOR x IN fake\" REMOVE u IN users"
            ),
            vec![
                ("users".to_string(), false, "FOR"),
                ("users".to_string(), true, "REMOVE")
            ]
        );
    }

    #[test]
    fn an_action_renders_its_own_view_and_assigns_what_it_reads() {
        let (_dir, graph) = build_db_fixture();
        assert_eq!(
            edge_relation(&graph, "PostsController#index", "posts/index", "renders"),
            vec!["implicit"]
        );
        // `@unused` is set but not read; the view's own `@local` is not the
        // action's.
        assert_eq!(
            edge_relation(&graph, "PostsController#index", "posts/index", "assigns"),
            vec!["@count", "@posts"]
        );
        // An explicit render is not doubled by an implicit one; `create`
        // redirects and has no view of its own.
        assert_eq!(
            edge_relation(&graph, "PostsController#stats", "posts/index", "renders"),
            vec![""]
        );
        assert!(!has_edge(
            &graph,
            "PostsController#create",
            "posts/index",
            "renders"
        ));
        // `_audit` is a helper, not an action.
        assert!(!has_edge(
            &graph,
            "PostsController#_audit",
            "posts/index",
            "renders"
        ));
    }

    #[test]
    fn a_bare_call_reaches_the_classs_own_method() {
        let (_dir, graph) = build_db_fixture();
        assert!(has_edge(
            &graph,
            "PostsController#create",
            "PostsController#_audit",
            "calls"
        ));
    }
}
