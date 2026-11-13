//! Generic (non-Soli) extractor: walk a repository by file extension, pull
//! structural nodes/edges from tree-sitter (via the optional `soli-codegraph`
//! crate) for supported languages, and chunk-embed everything else. Produces
//! the same [`ProjectGraph`] the Soli extractor does, so all of the sync /
//! embedding / query / incremental machinery is reused unchanged.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::graph::builder::md5_hex;
use crate::graph::config::GraphConfig;
use crate::graph::model::{sanitize_key, Edge, Node, ProjectGraph};

const MAX_SNIPPET: usize = 1200;
const MAX_TEXT: usize = 2000;

/// A definition extracted from a source file (mirror of
/// `soli_codegraph::Def`, defined locally so this module compiles without the
/// `codegraph` feature).
struct RawDef {
    kind: String,
    name: String,
    qualified_name: String,
    line: u32,
    signature: String,
    superclass: Option<String>,
    start_byte: usize,
    end_byte: usize,
}

/// A by-name reference (mirror of `soli_codegraph::EdgeRef`).
struct RawEdge {
    kind: String,
    target: String,
    from_qualified: String,
    line: u32,
}

/// Build the code graph for an arbitrary repository.
pub fn build_generic_graph(
    dir: &Path,
    config: &GraphConfig,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<ProjectGraph, String> {
    if !dir.is_dir() {
        return Err(format!("Folder '{}' does not exist", dir.display()));
    }
    let mut files = Vec::new();
    gather(dir, dir, config, &mut files);
    files.sort();

    let mut builder = GenericBuilder::default();
    let total = files.len();
    for (index, path) in files.iter().enumerate() {
        on_progress(index + 1, total);
        let rel = relpath(dir, path);
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        builder.file_hashes.insert(rel.clone(), md5_hex(&source));
        builder.add_file(&rel, &source);

        let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match extract_defs(&ext, &source) {
            Some((defs, edges)) if !defs.is_empty() => {
                builder.add_structure(&rel, &source, defs, edges);
            }
            // A parsed file with no definitions (a package `__init__.py` that
            // re-exports, a module of constants) is chunk-embedded, but its
            // imports are still edges.
            Some((_, edges)) => {
                builder.add_chunks(&rel, &source, config.chunk_lines);
                builder.add_structure(&rel, &source, Vec::new(), edges);
            }
            None => builder.add_chunks(&rel, &source, config.chunk_lines),
        }
    }

    builder.resolve_edges();
    Ok(builder.into_graph())
}

/// Language extraction, isolated behind the `codegraph` feature. Returns `None`
/// (→ chunk-embed) for unsupported extensions or when the feature is off.
fn extract_defs(ext: &str, source: &str) -> Option<(Vec<RawDef>, Vec<RawEdge>)> {
    #[cfg(feature = "codegraph")]
    {
        let lang = soli_codegraph::language_for_extension(ext)?;
        let ex = soli_codegraph::extract(lang, source);
        let defs = ex
            .defs
            .into_iter()
            .map(|d| RawDef {
                kind: d.kind,
                name: d.name,
                qualified_name: d.qualified_name,
                line: d.line,
                signature: d.signature,
                superclass: d.superclass,
                start_byte: d.start_byte,
                end_byte: d.end_byte,
            })
            .collect();
        let edges = ex
            .edges
            .into_iter()
            .map(|e| RawEdge {
                kind: e.kind,
                target: e.target,
                from_qualified: e.from_qualified,
                line: e.line,
            })
            .collect();
        Some((defs, edges))
    }
    #[cfg(not(feature = "codegraph"))]
    {
        let _ = (ext, source);
        None
    }
}

#[derive(Default)]
struct GenericBuilder {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    node_ids: HashSet<String>,
    id_to_key: HashMap<String, String>,
    used_keys: HashSet<String>,
    file_hashes: HashMap<String, String>,

    // Resolution indexes.
    class_by_name: HashMap<String, String>, // name -> node id (classes/modules)
    defs_by_name: HashMap<String, Vec<String>>, // name -> node ids (methods/functions)
    /// Simple class name -> how many classes carry it (a name two files
    /// declare is not a receiver the graph can trust).
    class_name_count: HashMap<String, usize>,
    /// Simple class name -> its base classes' simple names, from `inherits`.
    class_bases: HashMap<String, Vec<String>>,
    /// Simple model class name -> its table (`__tablename__`).
    model_tables: HashMap<String, String>,
    /// Blueprint variable -> the `url_prefix`es declared under that name.
    blueprint_prefixes: HashMap<String, Vec<String>>,

    // Deferred by-name edges: (from_id, RawEdge, relpath).
    deferred: Vec<(String, RawEdge, String)>,
}

impl GenericBuilder {
    fn intern(&mut self, id: &str, mut node: Node) {
        if self.node_ids.contains(id) {
            return;
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
        self.id_to_key.insert(id.to_string(), key);
        self.nodes.push(node);
    }

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

    fn ensure_external(&mut self, name: &str) -> String {
        let id = format!("external:{}", name);
        if !self.node_ids.contains(&id) {
            self.intern(
                &id,
                node(
                    "external",
                    name,
                    name,
                    "",
                    0,
                    "",
                    None,
                    &format!("external {}", name),
                ),
            );
        }
        id
    }

    fn add_file(&mut self, rel: &str, source: &str) {
        let id = format!("file:{}", rel);
        let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
        let text = compose_text("file", rel, "", &snippet(source, 0, source.len()));
        self.intern(&id, node("file", &name, rel, rel, 0, "", None, &text));
    }

    fn add_structure(&mut self, rel: &str, source: &str, defs: Vec<RawDef>, edges: Vec<RawEdge>) {
        let file_id = format!("file:{}", rel);
        for def in &defs {
            let id = def_id(def, rel);
            let snip = snippet(source, def.start_byte, def.end_byte);
            let text = compose_text(&def.kind, &def.qualified_name, &def.signature, &snip);
            self.intern(
                &id,
                node(
                    &def.kind,
                    &def.name,
                    &def.qualified_name,
                    rel,
                    def.line,
                    &def.signature,
                    def.superclass.clone(),
                    &text,
                ),
            );
            // Containment edge + resolution indexes.
            match def.kind.as_str() {
                "class" | "module" | "interface" | "enum" => {
                    self.push_edge(&file_id, &id, "defines", "", rel, def.line);
                    self.class_by_name
                        .entry(def.name.clone())
                        .or_insert_with(|| id.clone());
                    *self.class_name_count.entry(def.name.clone()).or_default() += 1;
                }
                _ => {
                    // method/function: attach to its enclosing class when the
                    // qualified name carries one (`User#authenticate`).
                    // `User#authenticate` (Ruby), `User.authenticate` (Python).
                    let owner = def
                        .qualified_name
                        .split_once('#')
                        .or_else(|| def.qualified_name.rsplit_once('.'));
                    if let Some((owner, _)) = owner {
                        let owner_id = format!("class:{}", owner);
                        if self.node_ids.contains(&owner_id) {
                            self.push_edge(&owner_id, &id, "defines", "", rel, def.line);
                        } else {
                            self.push_edge(&file_id, &id, "defines", "", rel, def.line);
                        }
                    } else {
                        self.push_edge(&file_id, &id, "defines", "", rel, def.line);
                    }
                    self.defs_by_name
                        .entry(def.name.clone())
                        .or_default()
                        .push(id.clone());
                }
            }
        }
        // Defer by-name edges (need the full project def index). The enclosing
        // def was just interned above, so resolve `from` against real node ids
        // rather than guessing the kind from the name's shape.
        for edge in &edges {
            if edge.kind == "table" {
                let class = edge
                    .from_qualified
                    .rsplit('.')
                    .next()
                    .unwrap_or(&edge.from_qualified);
                self.model_tables
                    .insert(class.to_string(), edge.target.clone());
            }
            if edge.kind == "blueprint" {
                if let Some((var, prefix)) = edge.target.split_once('\t') {
                    self.blueprint_prefixes
                        .entry(var.to_string())
                        .or_default()
                        .push(prefix.to_string());
                }
            }
            if edge.kind == "inherits" {
                let class = edge
                    .from_qualified
                    .rsplit(['.', ':', '#'])
                    .next()
                    .unwrap_or(&edge.from_qualified)
                    .to_string();
                let base = edge.target.rsplit('.').next().unwrap_or(&edge.target);
                self.class_bases
                    .entry(class)
                    .or_default()
                    .push(base.to_string());
            }
        }
        for edge in edges {
            let from_id = if edge.from_qualified.is_empty() {
                file_id.clone()
            } else {
                self.resolve_from_id(&edge.from_qualified, rel)
            };
            self.deferred.push((from_id, edge, rel.to_string()));
        }
    }

    fn add_chunks(&mut self, rel: &str, source: &str, chunk_lines: usize) {
        let file_id = format!("file:{}", rel);
        let lines: Vec<&str> = source.lines().collect();
        if lines.is_empty() {
            return;
        }
        let step = chunk_lines.max(1);
        let mut start = 0usize;
        while start < lines.len() {
            let end = (start + step).min(lines.len());
            let body = lines[start..end].join("\n");
            let start_line = start as u32 + 1;
            let id = format!("chunk:{}#{}", rel, start_line);
            let qn = format!("{}:{}", rel, start_line);
            let text = compose_text("chunk", &qn, "", &truncate(&body, MAX_SNIPPET));
            self.intern(
                &id,
                node("chunk", &qn, &qn, rel, start_line, "", None, &text),
            );
            self.push_edge(&file_id, &id, "defines", "", rel, start_line);
            start = end;
        }
    }

    fn resolve_edges(&mut self) {
        let deferred = std::mem::take(&mut self.deferred);
        for (from_id, edge, rel) in deferred {
            match edge.kind.as_str() {
                "inherits" | "implements" => {
                    let to = self
                        .class_by_name
                        .get(&edge.target)
                        .cloned()
                        .unwrap_or_else(|| self.ensure_external(&edge.target));
                    self.push_edge(&from_id, &to, &edge.kind, "", &rel, edge.line);
                }
                "imports" => {
                    // Resolve a relative require to a project file; otherwise an
                    // external stub (a gem/library dependency).
                    if let Some(target_file) = self.resolve_import(&rel, &edge.target) {
                        self.push_edge(&from_id, &target_file, "imports", "", &rel, edge.line);
                    } else {
                        let ext = self.ensure_external(&edge.target);
                        self.push_edge(&from_id, &ext, "imports", "", &rel, edge.line);
                    }
                }
                // A model's table: `__tablename__`.
                "table" => {
                    let collection = self.ensure_collection(&edge.target);
                    self.push_edge(&from_id, &collection, "stored_in", "", &rel, edge.line);
                }
                // `@bp.route(path)`: `bp\tVERB\tpath`, from the handler.
                "route" => {
                    let mut parts = edge.target.splitn(3, '\t');
                    let (Some(bp), Some(verb), Some(path)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    // The blueprint's prefix when one declaration carries the name.
                    let prefix = match self.blueprint_prefixes.get(bp).map(Vec::as_slice) {
                        Some([only]) => only.as_str(),
                        _ => "",
                    };
                    let full = join_route(prefix, path);
                    let route = self.ensure_route(verb, &full, &rel, edge.line);
                    let file_id = format!("file:{}", rel);
                    self.push_edge(&file_id, &route, "defines", "", &rel, edge.line);
                    self.push_edge(&route, &from_id, "routes_to", "", &rel, edge.line);
                }
                // SQLAlchemy access: `Model\top`.
                "db_read" | "db_write" => {
                    let Some((model, op)) = edge.target.split_once('\t') else {
                        continue;
                    };
                    // `self` inside a model method: the method's class.
                    let model = if model == "self" {
                        edge.from_qualified
                            .rsplit_once('.')
                            .map(|(owner, _)| owner.rsplit('.').next().unwrap_or(owner))
                            .unwrap_or("")
                    } else {
                        model
                    };
                    if let Some(table) = self.table_of(model) {
                        let collection = self.ensure_collection(&table);
                        let kind = if edge.kind == "db_read" {
                            "reads"
                        } else {
                            "writes"
                        };
                        self.push_edge(&from_id, &collection, kind, op, &rel, edge.line);
                    }
                }
                // A receiver-qualified Python call: `self.m`, `super.m`,
                // `Class.m`.
                "calls" if edge.target.contains('.') => {
                    if let Some(to) = self.resolve_member_call(&edge.from_qualified, &edge.target) {
                        self.push_edge(&from_id, &to, "calls", "", &rel, edge.line);
                    }
                }
                "calls" => {
                    // Precision-first: only link an unambiguous name. A bare
                    // Python call reaches a function, never a method.
                    let python = rel.ends_with(".py") || rel.ends_with(".pyi");
                    let to = self.defs_by_name.get(&edge.target).and_then(|ids| {
                        let mut candidates = ids
                            .iter()
                            .filter(|id| !python || id.starts_with("function:"));
                        match (candidates.next(), candidates.next()) {
                            (Some(only), None) => Some(only.clone()),
                            _ => None,
                        }
                    });
                    if let Some(to) = to {
                        self.push_edge(&from_id, &to, "calls", "", &rel, edge.line);
                    }
                }
                "instantiates" => {
                    // Link `new Foo()` only to a known project class — never an
                    // external stub, so framework types (`new List<T>()`) don't
                    // flood the graph with noise.
                    let unique = self.class_name_count.get(&edge.target).copied() == Some(1);
                    if let Some(to) = self
                        .class_by_name
                        .get(&edge.target)
                        .cloned()
                        .filter(|_| unique)
                    {
                        self.push_edge(&from_id, &to, "instantiates", "", &rel, edge.line);
                    }
                }
                _ => {}
            }
        }
    }

    /// Map an enclosing def's qualified name to its interned node id, trying the
    /// real candidates in id space (`method:` / `class:` / file-scoped
    /// `function:`) and returning the first that exists. Unlike
    /// [`def_id_for_qualified`], this doesn't guess the kind from the name's
    /// shape — so a `.`/`::`-separated method name (C#, Rust) resolves to its
    /// `method:` node instead of a nonexistent `class:` one. Falls back to the
    /// shape heuristic when nothing matches, preserving prior behaviour.
    /// The table of `model`, or of the nearest base that declares one (a
    /// subclass without `__tablename__` shares its parent's). Nothing for a
    /// class name two files declare.
    fn table_of(&self, model: &str) -> Option<String> {
        let mut queue: std::collections::VecDeque<String> = vec![model.to_string()].into();
        let mut seen = HashSet::new();
        while let Some(class) = queue.pop_front() {
            if !seen.insert(class.clone()) || seen.len() > 32 {
                continue;
            }
            if self.class_name_count.get(&class).copied().unwrap_or(0) != 1 {
                continue;
            }
            if let Some(table) = self.model_tables.get(&class) {
                return Some(table.clone());
            }
            if let Some(bases) = self.class_bases.get(&class) {
                queue.extend(bases.iter().cloned());
            }
        }
        None
    }

    /// The `collection:<table>` node, created on first use.
    fn ensure_collection(&mut self, table: &str) -> String {
        let id = format!("collection:{}", table);
        if !self.node_ids.contains(&id) {
            let text = format!("collection {} (database table / collection)", table);
            self.intern(
                &id,
                node("collection", table, table, "", 0, "", None, &text),
            );
        }
        id
    }

    /// The `route:VERB /path` node, created on first use.
    fn ensure_route(&mut self, verb: &str, path: &str, rel: &str, line: u32) -> String {
        let name = format!("{} {}", verb, path);
        let id = format!("route:{}", name);
        if !self.node_ids.contains(&id) {
            let text = format!("route {}", name);
            self.intern(&id, node("route", &name, &name, rel, line, "", None, &text));
        }
        id
    }

    /// `self.m` / `super.m` / `Class.m` called from `from_qualified`
    /// (`Owner.caller`): the method `m` of that class or its nearest ancestor
    /// that defines it. Nothing when the class is unknown or its name is
    /// declared twice in the project.
    fn resolve_member_call(&self, from_qualified: &str, target: &str) -> Option<String> {
        let (receiver, method) = target.rsplit_once('.')?;
        let owner = from_qualified
            .rsplit_once('.')
            .map(|(owner, _)| owner.rsplit('.').next().unwrap_or(owner));
        let start: Vec<String> = match receiver {
            "self" => vec![owner?.to_string()],
            "super" => self.class_bases.get(owner?).cloned().unwrap_or_default(),
            class => vec![class.to_string()],
        };
        // Breadth-first up the hierarchy: the nearest definition wins.
        let mut queue: std::collections::VecDeque<String> = start.into();
        let mut seen = HashSet::new();
        while let Some(class) = queue.pop_front() {
            if !seen.insert(class.clone()) || seen.len() > 32 {
                continue;
            }
            if self.class_name_count.get(&class).copied().unwrap_or(0) != 1 {
                // Unknown (a library base) or ambiguous: stop on this branch.
                continue;
            }
            let id = format!("method:{}.{}", class, method);
            if self.node_ids.contains(&id) {
                return Some(id);
            }
            if let Some(bases) = self.class_bases.get(&class) {
                queue.extend(bases.iter().cloned());
            }
        }
        None
    }

    fn resolve_from_id(&self, qualified: &str, rel: &str) -> String {
        let candidates = [
            format!("method:{}", qualified),
            format!("class:{}", qualified),
            format!("function:{}#{}", rel, qualified),
        ];
        for candidate in candidates {
            if self.node_ids.contains(&candidate) {
                return candidate;
            }
        }
        def_id_for_qualified(qualified, rel)
    }

    fn resolve_import(&self, importer: &str, target: &str) -> Option<String> {
        if importer.ends_with(".py") || importer.ends_with(".pyi") {
            return self.resolve_python_import(importer, target);
        }
        if !(target.starts_with("./") || target.starts_with("../") || target.contains('/')) {
            return None;
        }
        let dir = Path::new(importer).parent().unwrap_or(Path::new(""));
        // Try the path as-is and with common source extensions appended
        // (Ruby's `require_relative`, JS/TS relative imports).
        let candidates = [
            target.to_string(),
            format!("{}.rb", target),
            format!("{}.js", target),
            format!("{}.ts", target),
            format!("{}.tsx", target),
            format!("{}.jsx", target),
            format!("{}/index.ts", target),
            format!("{}/index.js", target),
        ];
        for candidate in candidates {
            let joined = normalize_rel(&dir.join(&candidate));
            let id = format!("file:{}", joined);
            if self.node_ids.contains(&id) {
                return Some(id);
            }
        }
        None
    }

    /// A Python module path to its project file: `a.b.c` from the indexed
    /// root, `.c` / `..b.c` from the importer's package, each as `c.py` or the
    /// package's `c/__init__.py`. `from a.b import c` names the module `a.b`;
    /// when `c` is itself a submodule the import still lands on `a/b`.
    fn resolve_python_import(&self, importer: &str, target: &str) -> Option<String> {
        let dots = target.chars().take_while(|&c| c == '.').count();
        let mut path = if dots == 0 {
            std::path::PathBuf::new()
        } else {
            let mut dir = Path::new(importer)
                .parent()
                .unwrap_or(Path::new(""))
                .to_path_buf();
            for _ in 1..dots {
                dir.pop();
            }
            dir
        };
        for segment in target[dots..].split('.').filter(|s| !s.is_empty()) {
            path.push(segment);
        }
        let module = normalize_rel(&path);
        let candidates = if module.is_empty() {
            vec!["__init__.py".to_string()]
        } else {
            vec![
                format!("{}.py", module),
                format!("{}.pyi", module),
                format!("{}/__init__.py", module),
            ]
        };
        candidates
            .into_iter()
            .map(|candidate| format!("file:{}", candidate))
            .find(|id| self.node_ids.contains(id))
    }

    fn into_graph(self) -> ProjectGraph {
        ProjectGraph {
            nodes: self.nodes,
            edges: self.edges,
            unresolved_calls: 0,
            resolved_imports: 0,
            file_hashes: self.file_hashes,
        }
    }
}

// ---- helpers ---------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn node(
    kind: &str,
    name: &str,
    qualified_name: &str,
    file: &str,
    line: u32,
    signature: &str,
    superclass: Option<String>,
    text: &str,
) -> Node {
    Node {
        key: String::new(),
        kind: kind.to_string(),
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        file: file.to_string(),
        line,
        signature: signature.to_string(),
        superclass,
        role: String::new(),
        doc: String::new(),
        text: text.to_string(),
        embedding: vec![],
    }
}

/// A blueprint prefix and a route path as one URL path: `/actions` + `` →
/// `/actions`, `/actions` + `/<int:id>` → `/actions/<int:id>`.
fn join_route(prefix: &str, path: &str) -> String {
    let joined = format!(
        "{}/{}",
        prefix.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let trimmed = joined.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    }
}

fn def_id(def: &RawDef, rel: &str) -> String {
    match def.kind.as_str() {
        "class" | "module" | "interface" | "enum" => format!("class:{}", def.qualified_name),
        "method" => format!("method:{}", def.qualified_name),
        _ => format!("function:{}#{}", rel, def.name),
    }
}

fn def_id_for_qualified(qualified: &str, rel: &str) -> String {
    if qualified.contains('#') {
        format!("method:{}", qualified)
    } else if qualified.contains("::") || qualified.chars().next().is_some_and(|c| c.is_uppercase())
    {
        format!("class:{}", qualified)
    } else {
        format!("function:{}#{}", rel, qualified)
    }
}

fn gather(root: &Path, dir: &Path, config: &GraphConfig, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if config.allows_dir(name) {
                gather(root, &path, config, out);
            }
        } else if path.is_file() {
            let rel = relpath(root, &path);
            if config.matches(&rel) {
                out.push(path);
            }
        }
    }
}

fn relpath(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_rel(path: &Path) -> String {
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
            _ => {}
        }
    }
    parts.join("/")
}

fn snippet(source: &str, start: usize, end: usize) -> String {
    let end = end.min(source.len());
    let start = start.min(end);
    truncate(source.get(start..end).unwrap_or(""), MAX_SNIPPET)
}

fn compose_text(kind: &str, qualified: &str, signature: &str, snippet: &str) -> String {
    let mut parts = vec![format!("{} {}", kind, qualified)];
    if !signature.is_empty() {
        parts.push(signature.to_string());
    }
    if !snippet.is_empty() {
        parts.push(snippet.to_string());
    }
    truncate(&parts.join("\n"), MAX_TEXT)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(all(test, feature = "codegraph"))]
mod tests {
    use super::*;
    use std::fs;

    fn cs_config() -> GraphConfig {
        GraphConfig {
            extensions: vec!["cs".to_string()],
            ..Default::default()
        }
    }

    fn key_of(graph: &ProjectGraph, kind: &str, qn: &str) -> Option<String> {
        graph
            .nodes
            .iter()
            .find(|n| n.kind == kind && n.qualified_name == qn)
            .map(|n| n.key.clone())
    }

    fn has_edge(
        graph: &ProjectGraph,
        from: (&str, &str),
        to: (&str, &str),
        edge_kind: &str,
    ) -> bool {
        let (Some(f), Some(t)) = (key_of(graph, from.0, from.1), key_of(graph, to.0, to.1)) else {
            return false;
        };
        graph
            .edges
            .iter()
            .any(|e| e.from == f && e.to == t && e.edge_kind == edge_kind)
    }

    #[test]
    fn python_calls_resolve_through_receivers_and_bases() {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, body: &str| fs::write(dir.path().join(rel), body).unwrap();
        write(
            "base.py",
            "class BaseService:\n    def validate(self, x):\n        pass\n    def insert(self, x):\n        pass\n",
        );
        write(
            "repo.py",
            "class ActionRepository:\n    def add(self, x):\n        pass\n    def save(self):\n        pass\n",
        );
        write("dup_a.py", "class Twin:\n    def go(self):\n        pass\n");
        write("dup_b.py", "class Twin:\n    def go(self):\n        pass\n");
        write("util.py", "def notify(x):\n    pass\n");
        write(
            "service.py",
            concat!(
                "class ActionService(BaseService):\n",
                "    def insert(self, x):\n",
                "        self.validate(x)\n",
                "        super().insert(x)\n",
                "    def run(self, repo: ActionRepository, twin: Twin):\n",
                "        self.insert(1)\n",
                "        repo.add(1)\n",
                "        twin.go()\n",
                "        notify(1)\n",
                "        save()\n",
                "        ActionRepository()\n",
            ),
        );
        let config = GraphConfig {
            extensions: vec!["py".to_string()],
            ..Default::default()
        };
        let graph = build_generic_graph(dir.path(), &config, &mut |_, _| {}).unwrap();
        let insert = ("method", "ActionService.insert");
        let run = ("method", "ActionService.run");
        // `self.validate` is inherited; `super().insert` skips the override.
        assert!(has_edge(
            &graph,
            insert,
            ("method", "BaseService.validate"),
            "calls"
        ));
        assert!(has_edge(
            &graph,
            insert,
            ("method", "BaseService.insert"),
            "calls"
        ));
        // `self.insert` finds the class's own override first.
        assert!(has_edge(&graph, run, insert, "calls"));
        assert!(has_edge(
            &graph,
            run,
            ("method", "ActionRepository.add"),
            "calls"
        ));
        assert!(has_edge(&graph, run, ("function", "notify"), "calls"));
        assert!(has_edge(
            &graph,
            run,
            ("class", "ActionRepository"),
            "instantiates"
        ));
        // A Python method is defined by its class, not only its file.
        assert!(has_edge(&graph, ("class", "ActionService"), run, "defines"));
        // Two classes named `Twin`: no edge. A bare `save()` is not the method.
        assert!(!graph
            .edges
            .iter()
            .any(|e| e.edge_kind == "calls" && e.to.contains("Twin.go")));
        assert!(!has_edge(
            &graph,
            run,
            ("method", "ActionRepository.save"),
            "calls"
        ));
    }

    #[test]
    fn python_flask_routes_and_tables_reach_the_graph() {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, body: &str| fs::write(dir.path().join(rel), body).unwrap();
        write(
            "models.py",
            "class Base:\n    pass\nclass Action(Base):\n    __tablename__ = \"todo_item\"\nclass SubAction(Action):\n    pass\n",
        );
        write(
            "resources.py",
            concat!(
                "bp = Blueprint(\"bp_actions\", __name__, url_prefix=\"/actions\")\n",
                "@bp.route(\"\")\n",
                "class ActionsResource(MethodView):\n",
                "    def get(self):\n",
                "        return Action.query.all()\n",
                "    def post(self):\n",
                "        sub = SubAction()\n",
                "        db.session.add(sub)\n",
            ),
        );
        let config = GraphConfig {
            extensions: vec!["py".to_string()],
            ..Default::default()
        };
        let graph = build_generic_graph(dir.path(), &config, &mut |_, _| {}).unwrap();
        let get = ("method", "ActionsResource.get");
        let post = ("method", "ActionsResource.post");
        assert!(has_edge(
            &graph,
            ("route", "GET /actions"),
            get,
            "routes_to"
        ));
        assert!(has_edge(
            &graph,
            ("route", "POST /actions"),
            post,
            "routes_to"
        ));
        assert!(has_edge(
            &graph,
            ("class", "Action"),
            ("collection", "todo_item"),
            "stored_in"
        ));
        assert!(has_edge(&graph, get, ("collection", "todo_item"), "reads"));
        // `SubAction` has no table of its own: its parent's.
        assert!(has_edge(
            &graph,
            post,
            ("collection", "todo_item"),
            "writes"
        ));
    }

    #[test]
    fn python_imports_resolve_to_project_modules() {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, body: &str| {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        };
        write("contexts/__init__.py", "");
        write("contexts/risk/__init__.py", "");
        write(
            "contexts/risk/domain/measure_risk.py",
            "def measure():\n    pass\n",
        );
        write("contexts/risk/resources/risk_scale.py", "SCALE = 1\n");
        write(
            "contexts/risk/services/register.py",
            "import os\nfrom contexts.risk.domain.measure_risk import measure\nfrom ..resources.risk_scale import SCALE\nfrom .. import resources\nimport contexts.risk\n",
        );
        let config = GraphConfig {
            extensions: vec!["py".to_string()],
            ..Default::default()
        };
        let graph = build_generic_graph(dir.path(), &config, &mut |_, _| {}).unwrap();
        let importer = ("file", "contexts/risk/services/register.py");
        for target in [
            "contexts/risk/domain/measure_risk.py",
            "contexts/risk/resources/risk_scale.py",
            "contexts/risk/__init__.py",
        ] {
            assert!(
                has_edge(&graph, importer, ("file", target), "imports"),
                "{target}"
            );
        }
        // A library stays an external stub.
        assert!(has_edge(&graph, importer, ("external", "os"), "imports"));
    }

    #[test]
    fn csharp_instantiates_and_calls_resolve_to_the_enclosing_method() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("account.cs"),
            "class Logger {\n  public void Write(string s) {}\n}\n\
             class Account {\n  public void Save() {\n    var log = new Logger();\n    log.Write(\"x\");\n    var noise = new System.Text.StringBuilder();\n  }\n}\n",
        )
        .unwrap();
        let graph = build_generic_graph(dir.path(), &cs_config(), &mut |_, _| {}).unwrap();

        // `new Logger()` → instantiates, attributed to `Account.Save` (the method
        // node, not a nonexistent `class:Account.Save`).
        assert!(has_edge(
            &graph,
            ("method", "Account.Save"),
            ("class", "Logger"),
            "instantiates",
        ));
        // `log.Write(...)` resolves by unambiguous name to `Logger.Write`.
        assert!(has_edge(
            &graph,
            ("method", "Account.Save"),
            ("method", "Logger.Write"),
            "calls",
        ));
        // A framework type (`new StringBuilder()`) is not a project class, so it
        // yields no node and no instantiates edge — `Logger` is the only one.
        assert!(!graph
            .nodes
            .iter()
            .any(|n| n.qualified_name.contains("StringBuilder")));
        let instantiates = graph
            .edges
            .iter()
            .filter(|e| e.edge_kind == "instantiates")
            .count();
        assert_eq!(instantiates, 1, "only `new Logger()` should resolve");
    }
}
