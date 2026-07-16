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
            _ => builder.add_chunks(&rel, &source, config.chunk_lines),
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
                }
                _ => {
                    // method/function: attach to its enclosing class when the
                    // qualified name carries one (`User#authenticate`).
                    if let Some((owner, _)) = def.qualified_name.split_once('#') {
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
                "calls" => {
                    // Precision-first: only link an unambiguous name.
                    if let Some(ids) = self.defs_by_name.get(&edge.target) {
                        if ids.len() == 1 {
                            let to = ids[0].clone();
                            self.push_edge(&from_id, &to, "calls", "", &rel, edge.line);
                        }
                    }
                }
                "instantiates" => {
                    // Link `new Foo()` only to a known project class — never an
                    // external stub, so framework types (`new List<T>()`) don't
                    // flood the graph with noise.
                    if let Some(to) = self.class_by_name.get(&edge.target).cloned() {
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
        if !(target.starts_with("./") || target.starts_with("../") || target.contains('/')) {
            return None;
        }
        let dir = Path::new(importer).parent().unwrap_or(Path::new(""));
        // Try the path as-is and with common source extensions appended.
        for candidate in [target.to_string(), format!("{}.rb", target)] {
            let joined = normalize_rel(&dir.join(&candidate));
            let id = format!("file:{}", joined);
            if self.node_ids.contains(&id) {
                return Some(id);
            }
        }
        None
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
