//! Data model for the project code-graph.
//!
//! A [`ProjectGraph`] is a set of [`Node`]s (files, classes, methods,
//! functions, routes, views, …) and [`Edge`]s between them (defines, inherits,
//! imports, calls, renders, routes_to, relates). It is produced by
//! [`crate::graph::builder`] and written to SolidB by [`crate::graph::sync`].
//!
//! Both structs derive `Serialize` so the whole graph can be emitted as JSON
//! (`soli graph build --dry-run`) and so individual node/edge documents can be
//! sent to SolidB verbatim.

use serde::Serialize;

/// SolidB collection holding one document per graph node.
pub const NODE_COLLECTION: &str = "soli_graph_nodes";
/// SolidB edge collection holding one document per graph edge.
pub const EDGE_COLLECTION: &str = "soli_graph_edges";
/// Small collection holding the build manifest (per-file content hashes) so a
/// re-run can skip when nothing changed.
pub const META_COLLECTION: &str = "soli_graph_meta";
/// Name of the vector index created over `soli_graph_nodes.embedding`.
pub const VECTOR_INDEX: &str = "node_vec";

/// A single graph node. `key` is a SolidB-safe `_key`; the human-readable
/// identity lives in `kind` + `qualified_name`.
#[derive(Debug, Clone, Serialize)]
pub struct Node {
    /// SolidB `_key` (sanitized, unique).
    pub key: String,
    /// Node kind: file, class, model, controller, method, function, route,
    /// view, enum, interface, external.
    pub kind: String,
    /// Short name (e.g. `authenticate`, `User`, `GET /login`).
    pub name: String,
    /// Fully-qualified name (e.g. `User#authenticate`, `posts#index`).
    pub qualified_name: String,
    /// Project-relative source path (`/`-separated), empty for synthetic nodes.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// 1-based source line (0 when not applicable).
    pub line: u32,
    /// A readable signature line for methods/functions.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub signature: String,
    /// Superclass name for classes (`Model`, `ApplicationController`, …).
    /// Always serialized (never skipped) so an incremental UPDATE overwrites it
    /// to `null` when a class drops its `< Base`.
    pub superclass: Option<String>,
    /// MVC role: model, controller, helper, service, job, middleware, mailer.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// Leading `#`/`//` doc comment, if any. Always serialized so an incremental
    /// UPDATE clears it when the comment is removed.
    pub doc: String,
    /// The text embedded for semantic search (`kind`, name, signature, doc,
    /// source snippet).
    pub text: String,
    /// Embedding vector; empty until [`crate::graph::sync`] fills it (or when
    /// building with `--no-embed`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub embedding: Vec<f64>,
}

/// A single directed graph edge between two node keys.
#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    /// Source node key.
    pub from: String,
    /// Target node key.
    pub to: String,
    /// defines, inherits, implements, imports, calls, instantiates, renders,
    /// redirects, routes_to, relates.
    pub edge_kind: String,
    /// Association name for `relates` edges (has_many, belongs_to, …).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub relation: String,
    /// Project-relative file the edge originates from.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// 1-based source line of the edge (0 when not applicable).
    pub line: u32,
}

/// The full extracted graph plus build statistics.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProjectGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Call sites whose target could not be resolved to a node (reported, not
    /// stored as edges).
    #[serde(skip)]
    pub unresolved_calls: usize,
    /// Local `import` statements that resolved to a project file (edges),
    /// tracked for the summary line.
    #[serde(skip)]
    pub resolved_imports: usize,
    /// `relpath -> content hash` for every source file, so an incremental
    /// re-build can skip when nothing changed.
    #[serde(skip)]
    pub file_hashes: std::collections::HashMap<String, String>,
}

impl ProjectGraph {
    /// Node document as written to `soli_graph_nodes` (`_key` + fields).
    pub fn node_document(node: &Node) -> serde_json::Value {
        let mut doc = serde_json::to_value(node).unwrap_or(serde_json::Value::Null);
        if let Some(obj) = doc.as_object_mut() {
            // `key` becomes the SolidB `_key`; drop the redundant field.
            let key = obj.remove("key");
            if let Some(key) = key {
                obj.insert("_key".to_string(), key);
            }
            // Stamp the content hash so the next incremental sync can detect an
            // unchanged node and skip re-writing it.
            obj.insert(
                "chash".to_string(),
                serde_json::Value::String(node_content_hash(node)),
            );
        }
        doc
    }

    /// Pretty-printed JSON of the whole graph, for `soli graph build --dry-run`.
    pub fn to_pretty_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Edge document as written to `soli_graph_edges` (`_from`/`_to` refs). The
    /// `_key` is deterministic (a hash of the endpoints/kind/line/relation) so a
    /// re-build upserts the same edge instead of duplicating it — enabling the
    /// non-destructive diff sync.
    pub fn edge_document(edge: &Edge) -> serde_json::Value {
        let mut doc = serde_json::json!({
            "_key": edge_key(edge),
            "_from": format!("{}/{}", NODE_COLLECTION, edge.from),
            "_to": format!("{}/{}", NODE_COLLECTION, edge.to),
            "edge_kind": edge.edge_kind,
        });
        if !edge.relation.is_empty() {
            doc["relation"] = serde_json::json!(edge.relation);
        }
        if !edge.file.is_empty() {
            doc["file"] = serde_json::json!(edge.file);
        }
        if edge.line != 0 {
            doc["line"] = serde_json::json!(edge.line);
        }
        doc
    }
}

/// Turn a readable node id (`method:User#authenticate`, `route:GET /login`)
/// into a valid, stable SolidB `_key`.
///
/// SolidB keys allow `[A-Za-z0-9_:.@()+,=;$!*'%-]` (up to 254 bytes). We map the
/// characters our ids use that fall outside that set — `/`, `#`, whitespace —
/// to allowed ones, replace anything else disallowed with `_`, and, if the
/// result would be too long, truncate and append a short deterministic hash so
/// distinct ids never collide.
pub fn sanitize_key(id: &str) -> String {
    fn is_allowed(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.' | '@' | '-')
    }
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        match c {
            '/' => out.push(':'),
            '#' => out.push('.'),
            c if c.is_whitespace() => out.push('_'),
            c if is_allowed(c) => out.push(c),
            _ => out.push('_'),
        }
    }
    if out.len() > 240 {
        let hash = fnv1a(id);
        out.truncate(200);
        out.push_str(&format!("__{:016x}", hash));
    }
    out
}

/// Deterministic `_key` for an edge, so re-builds upsert (not duplicate) it.
/// Endpoints + kind + line + relation fully identify an edge, including two
/// distinct call sites to the same target (they differ by line).
pub fn edge_key(edge: &Edge) -> String {
    let sig = format!(
        "{}|{}|{}|{}|{}",
        edge.from, edge.to, edge.edge_kind, edge.line, edge.relation
    );
    format!("e{:016x}", fnv1a(&sig))
}

/// Tiny FNV-1a hash — used to disambiguate over-long truncated keys and to
/// derive deterministic edge keys.
fn fnv1a(s: &str) -> u64 {
    fnv1a_bytes(0xcbf2_9ce4_8422_2325, s.as_bytes())
}

/// FNV-1a over raw bytes, resumable from a seed so multiple fields can be
/// folded into one hash without allocating an intermediate string.
fn fnv1a_bytes(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Content hash over a node's *stored* fields (everything except its `key`
/// identity). Written into the document as `chash` and compared against the
/// stored value on the next sync so an incremental re-sync can skip UPDATE-ing
/// nodes whose content is byte-identical — the main lever for avoiding SolidB's
/// per-write-batch vector-index re-serialization. Deterministic: folds the
/// fields in a fixed order with a separator so distinct field boundaries can't
/// collide.
pub fn node_content_hash(node: &Node) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for part in [
        node.kind.as_str(),
        node.name.as_str(),
        node.qualified_name.as_str(),
        node.file.as_str(),
        node.signature.as_str(),
        node.superclass.as_deref().unwrap_or(""),
        node.role.as_str(),
        node.doc.as_str(),
        node.text.as_str(),
    ] {
        hash = fnv1a_bytes(hash, part.as_bytes());
        hash = fnv1a_bytes(hash, &[0xff]); // field separator
    }
    hash = fnv1a_bytes(hash, &node.line.to_le_bytes());
    for &value in &node.embedding {
        hash = fnv1a_bytes(hash, &value.to_le_bytes());
    }
    format!("{:016x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_maps_slash_and_hash() {
        assert_eq!(
            sanitize_key("method:User#authenticate"),
            "method:User.authenticate"
        );
        assert_eq!(sanitize_key("route:GET /login"), "route:GET_:login");
        assert_eq!(
            sanitize_key("file:app/models/user.sl"),
            "file:app:models:user.sl"
        );
    }

    #[test]
    fn sanitize_is_deterministic_and_disambiguates_long_ids() {
        let long = format!("function:{}", "x".repeat(300));
        let a = sanitize_key(&long);
        let b = sanitize_key(&long);
        assert_eq!(a, b);
        assert!(a.len() <= 240);
        // A different long id must not collide after truncation.
        let other = format!("function:{}", "y".repeat(300));
        assert_ne!(sanitize_key(&long), sanitize_key(&other));
    }

    #[test]
    fn node_document_promotes_key_to_underscore_key() {
        let node = Node {
            key: "class:User".to_string(),
            kind: "model".to_string(),
            name: "User".to_string(),
            qualified_name: "User".to_string(),
            file: "app/models/user.sl".to_string(),
            line: 1,
            signature: String::new(),
            superclass: Some("Model".to_string()),
            role: "model".to_string(),
            doc: String::new(),
            text: "model User".to_string(),
            embedding: vec![],
        };
        let doc = ProjectGraph::node_document(&node);
        assert_eq!(doc["_key"], "class:User");
        assert!(doc.get("key").is_none());
        assert_eq!(doc["superclass"], "Model");
        assert!(doc.get("embedding").is_none(), "empty embedding omitted");
    }

    #[test]
    fn node_document_stamps_content_hash() {
        let node = Node {
            key: "class:User".to_string(),
            kind: "model".to_string(),
            name: "User".to_string(),
            qualified_name: "User".to_string(),
            file: "app/models/user.sl".to_string(),
            line: 1,
            signature: String::new(),
            superclass: Some("Model".to_string()),
            role: "model".to_string(),
            doc: String::new(),
            text: "model User".to_string(),
            embedding: vec![],
        };
        let doc = ProjectGraph::node_document(&node);
        assert_eq!(doc["chash"], node_content_hash(&node));
        assert!(doc["chash"].as_str().is_some_and(|s| !s.is_empty()));
    }

    #[test]
    fn content_hash_is_deterministic_and_change_sensitive() {
        let base = Node {
            key: "method:User.authenticate".to_string(),
            kind: "method".to_string(),
            name: "authenticate".to_string(),
            qualified_name: "User#authenticate".to_string(),
            file: "app/models/user.sl".to_string(),
            line: 12,
            signature: "def authenticate(pw)".to_string(),
            superclass: None,
            role: "model".to_string(),
            doc: String::new(),
            text: "method authenticate".to_string(),
            embedding: vec![0.1, 0.2, 0.3],
        };
        // Identical content → identical hash.
        assert_eq!(node_content_hash(&base), node_content_hash(&base.clone()));

        // A changed line, doc, text, or embedding each perturbs the hash.
        let mut moved = base.clone();
        moved.line = 13;
        assert_ne!(node_content_hash(&base), node_content_hash(&moved));

        let mut re_embedded = base.clone();
        re_embedded.embedding = vec![0.1, 0.2, 0.4];
        assert_ne!(node_content_hash(&base), node_content_hash(&re_embedded));

        let mut redoc = base.clone();
        redoc.doc = "authenticates a user".to_string();
        assert_ne!(node_content_hash(&base), node_content_hash(&redoc));

        // Field-boundary safety: shifting a character across two fields must
        // still change the hash (the separator prevents a silent collision).
        let mut shifted = base.clone();
        shifted.name = "authenticat".to_string();
        shifted.qualified_name = "eUser#authenticate".to_string();
        assert_ne!(node_content_hash(&base), node_content_hash(&shifted));
    }

    #[test]
    fn edge_document_builds_from_to_refs() {
        let edge = Edge {
            from: "route:GET_:login".to_string(),
            to: "method:SessionsController.create".to_string(),
            edge_kind: "routes_to".to_string(),
            relation: String::new(),
            file: "config/routes.sl".to_string(),
            line: 3,
        };
        let doc = ProjectGraph::edge_document(&edge);
        assert_eq!(doc["_from"], "soli_graph_nodes/route:GET_:login");
        assert_eq!(
            doc["_to"],
            "soli_graph_nodes/method:SessionsController.create"
        );
        assert_eq!(doc["edge_kind"], "routes_to");
        assert_eq!(doc["line"], 3);
    }
}
