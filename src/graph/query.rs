//! `soli graph query` — retrieval over the code graph for agents.
//!
//! One call returns the code most relevant to a natural-language task, *with*
//! its immediate graph relationships — the graph-RAG payoff (semantic seed →
//! graph expansion) without the agent writing any AQL.
//!
//! Pipeline: embed the question → ANN search the `node_vec` vector index for
//! seed nodes → for each seed, one traversal for its neighbours (callers,
//! callees, routes, views, …). If the graph was built with `--no-embed` (no
//! vector index) or no embedding key is configured, it falls back to a
//! keyword-ranked scan so the command still works.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::Value;

use crate::graph::model::{NODE_COLLECTION, VECTOR_INDEX};
use crate::solidb_http::SoliDBClient;

/// Cap on neighbours returned per seed, to keep output bounded.
const MAX_NEIGHBORS: usize = 50;

/// Max characters of node `text` included as a seed snippet (embeddings never
/// appear in query output).
const SNIPPET_MAX: usize = 400;

/// When a `--path` or `--kind` filter is active the vector index can't
/// pre-filter, so we over-fetch and filter afterwards.
const PATH_OVERFETCH: usize = 20;
const PATH_OVERFETCH_CAP: usize = 500;

/// Preferential order for neighbour edge kinds (lower = higher priority).
const EDGE_KIND_RANK: &[&str] = &[
    "routes_to",
    "calls",
    "renders",
    "redirects",
    "relates",
    "instantiates",
    "inherits",
    "imports",
    "implements",
    "defines",
];

pub struct QueryOptions {
    pub database: Option<String>,
    /// Number of seed (most-relevant) nodes to return.
    pub limit: usize,
    /// Neighbour-expansion depth (1 = direct relationships).
    pub hops: usize,
    /// Keep only seeds whose `file` starts with this path prefix (e.g. `api/`
    /// or `app/src/`). `None` (or empty) = no path constraint. Lets an agent
    /// scope retrieval to one side of a mono-repo without post-processing.
    pub path: Option<String>,
    /// Keep only seeds whose `kind` is one of these (e.g. `method,controller`).
    /// Empty = no kind constraint.
    pub kinds: Vec<String>,
}

/// A neighbour of a seed node, reached over one edge.
#[derive(Serialize)]
pub struct Neighbor {
    /// `out` (this edge points away from the seed) or `in` (points at it).
    pub direction: String,
    pub edge_kind: String,
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub signature: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
    pub line: u32,
}

/// A seed node (relevant to the query) plus its graph neighbours.
#[derive(Serialize)]
pub struct SeedHit {
    pub score: f64,
    pub kind: String,
    pub qualified_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
    pub line: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub signature: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub doc: String,
    /// Truncated node `text` for agent context (never the embedding vector).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub snippet: String,
    pub neighbors: Vec<Neighbor>,
}

/// The full query answer.
#[derive(Serialize)]
pub struct QueryResult {
    /// `semantic` (vector search) or `keyword` (fallback scan).
    pub mode: String,
    pub query: String,
    pub results: Vec<SeedHit>,
}

impl QueryResult {
    pub fn to_pretty_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Run a retrieval query against the code graph in SolidB.
pub fn run_query(question: &str, opts: &QueryOptions) -> Result<QueryResult, String> {
    let (client, _db) = crate::graph::sync::connect(opts.database.as_deref())?;
    let limit = opts.limit.max(1);
    let hops = opts.hops.clamp(1, 3);
    // Treat an empty `--path` as no filter so callers can pass through a blank
    // value without accidentally excluding every node.
    let path = opts.path.as_deref().filter(|p| !p.is_empty());
    let kinds: Option<&[String]> = if opts.kinds.is_empty() {
        None
    } else {
        Some(opts.kinds.as_slice())
    };

    // Seeds: semantic first, keyword fallback.
    let (seeds, mode) = match semantic_seeds(&client, question, limit, path, kinds) {
        Some(v) if !v.is_empty() => (v, "semantic"),
        _ => (
            keyword_seeds(&client, question, limit, path, kinds)?,
            "keyword",
        ),
    };

    let mut results = Vec::with_capacity(seeds.len());
    for (doc, score) in seeds {
        let key = field(&doc, "_key");
        let neighbors = if key.is_empty() {
            Vec::new()
        } else {
            fetch_neighbors(&client, &key, hops)?
        };
        results.push(SeedHit {
            score,
            kind: field(&doc, "kind"),
            qualified_name: field(&doc, "qualified_name"),
            file: field(&doc, "file"),
            line: uint(&doc, "line"),
            signature: field(&doc, "signature"),
            doc: field(&doc, "doc"),
            snippet: truncate_snippet(&field(&doc, "text")),
            neighbors,
        });
    }

    Ok(QueryResult {
        mode: mode.to_string(),
        query: question.to_string(),
        results,
    })
}

/// ANN seeds via the `node_vec` vector index. `None` when embeddings are
/// unavailable or the index doesn't exist (built with `--no-embed`).
///
/// With a `path` or `kind` filter the vector index can't pre-filter, so we
/// over-fetch and drop non-matching hits afterwards — this keeps `limit`
/// matching seeds when the top-ranked nodes fall outside the filter.
fn semantic_seeds(
    client: &SoliDBClient,
    question: &str,
    limit: usize,
    path: Option<&str>,
    kinds: Option<&[String]>,
) -> Option<Vec<(Value, f64)>> {
    let vector = crate::embedding::generate_embedding(question)?;
    let needs_overfetch = path.is_some() || kinds.is_some();
    let fetch = if needs_overfetch {
        (limit * PATH_OVERFETCH).min(PATH_OVERFETCH_CAP)
    } else {
        limit
    };
    let hits = client
        .vector_search(NODE_COLLECTION, VECTOR_INDEX, &vector, fetch, None)
        .ok()?;
    let seeds: Vec<(Value, f64)> = hits
        .into_iter()
        .filter_map(|hit| {
            let doc = hit.get("document")?.clone();
            let score = hit.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0);
            Some((doc, score))
        })
        .filter(|(doc, _)| path_matches(doc, path) && kind_matches(doc, kinds))
        .take(limit)
        .collect();
    Some(seeds)
}

/// Keyword-ranked fallback: weighted term hits on name fields beat body text.
fn keyword_seeds(
    client: &SoliDBClient,
    question: &str,
    limit: usize,
    path: Option<&str>,
    kinds: Option<&[String]>,
) -> Result<Vec<(Value, f64)>, String> {
    let terms = tokenize(question);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    // Restrict before scoring so `LIMIT` counts only matching nodes.
    let mut filters = String::new();
    if path.is_some() {
        filters.push_str("FILTER STARTS_WITH(n.file, @path) ");
    }
    if kinds.is_some() {
        filters.push_str("FILTER n.kind IN @kinds ");
    }
    // Weighted score: qualified_name > name > signature > full text.
    let query = format!(
        "FOR n IN soli_graph_nodes {filters}\
         LET qn = LENGTH(FOR t IN @terms FILTER CONTAINS(LOWER(n.qualified_name), t) RETURN 1) \
         LET nm = LENGTH(FOR t IN @terms FILTER CONTAINS(LOWER(n.name), t) RETURN 1) \
         LET sg = LENGTH(FOR t IN @terms FILTER CONTAINS(LOWER(n.signature), t) RETURN 1) \
         LET tx = LENGTH(FOR t IN @terms FILTER CONTAINS(LOWER(n.text), t) RETURN 1) \
         LET s = (qn * 4) + (nm * 3) + (sg * 2) + tx \
         FILTER s > 0 SORT s DESC LIMIT @lim RETURN MERGE(n, {{ _score: s }})"
    );
    let mut binds = HashMap::new();
    binds.insert("terms".to_string(), serde_json::json!(terms));
    binds.insert("lim".to_string(), serde_json::json!(limit));
    if let Some(p) = path {
        binds.insert("path".to_string(), serde_json::json!(p));
    }
    if let Some(k) = kinds {
        binds.insert("kinds".to_string(), serde_json::json!(k));
    }
    let rows = client
        .query(&query, Some(binds))
        .map_err(|e| format!("keyword search failed: {}", e))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let score = row.get("_score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            (row, score)
        })
        .collect())
}

/// One traversal per seed for its neighbours. The start vertex is inlined as a
/// quoted literal (SolidB's verified form); the `_key` charset excludes quotes,
/// so this is injection-safe.
fn fetch_neighbors(client: &SoliDBClient, key: &str, hops: usize) -> Result<Vec<Neighbor>, String> {
    let start = format!("{}/{}", NODE_COLLECTION, key);
    // Fetch a bit more than MAX then rank in-process so edge-kind priority
    // wins over arbitrary traversal order.
    let fetch_cap = MAX_NEIGHBORS * 2;
    // `efrom` (not `from`, a reserved AQL object key) carries the edge origin so
    // we can label direction relative to the seed.
    let query = format!(
        "FOR v, e IN 1..{} ANY \"{}\" soli_graph_edges LIMIT {} \
         RETURN {{ efrom: e._from, edge_kind: e.edge_kind, kind: v.kind, \
         name: v.qualified_name, signature: v.signature, file: v.file, line: v.line }}",
        hops, start, fetch_cap
    );
    let rows = client
        .query(&query, None)
        .map_err(|e| format!("neighbour traversal failed: {}", e))?;
    let mut neighbors: Vec<Neighbor> = rows
        .into_iter()
        .map(|row| {
            let efrom = field(&row, "efrom");
            let direction = if efrom == start { "out" } else { "in" };
            Neighbor {
                direction: direction.to_string(),
                edge_kind: field(&row, "edge_kind"),
                kind: field(&row, "kind"),
                name: field(&row, "name"),
                signature: field(&row, "signature"),
                file: field(&row, "file"),
                line: uint(&row, "line"),
            }
        })
        .collect();
    neighbors.sort_by_key(|n| edge_kind_rank(&n.edge_kind));
    neighbors.truncate(MAX_NEIGHBORS);
    Ok(neighbors)
}

fn edge_kind_rank(kind: &str) -> usize {
    EDGE_KIND_RANK
        .iter()
        .position(|&k| k == kind)
        .unwrap_or(EDGE_KIND_RANK.len())
}

/// Common natural-language noise dropped from keyword queries (kept short and
/// code-safe — no `get`/`set`/`new`/`run` etc. that carry meaning in code).
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "was", "how", "why", "what", "where", "which", "who", "does",
    "did", "has", "have", "this", "that", "with", "from", "into", "your", "you", "our", "its",
    "but", "not",
];

/// Lowercased, de-duplicated, non-stopword query terms of length ≥ 3 (max 12).
fn tokenize(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.len() >= 3 && !STOPWORDS.contains(&word.as_str()) && seen.insert(word.clone()) {
            out.push(word);
            if out.len() >= 12 {
                break;
            }
        }
    }
    out
}

/// True when `doc`'s `file` starts with `path` (or no path filter is set).
/// A missing/empty `file` never matches a non-empty prefix.
fn path_matches(doc: &Value, path: Option<&str>) -> bool {
    match path {
        None => true,
        Some(prefix) => field(doc, "file").starts_with(prefix),
    }
}

/// True when `doc`'s `kind` is in `kinds` (or no kind filter is set).
fn kind_matches(doc: &Value, kinds: Option<&[String]>) -> bool {
    match kinds {
        None | Some([]) => true,
        Some(list) => {
            let k = field(doc, "kind");
            list.iter().any(|want| want == &k)
        }
    }
}

/// First `SNIPPET_MAX` chars of embedded text for agent context.
fn truncate_snippet(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut end = SNIPPET_MAX.min(text.len());
    // Don't split a UTF-8 codepoint.
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut s = text[..end].to_string();
    if text.len() > end {
        s.push('…');
    }
    s
}

/// Parse a comma-separated `--kind` list into trimmed, non-empty kinds.
pub fn parse_kinds(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn field(doc: &Value, key: &str) -> String {
    doc.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn uint(doc: &Value, key: &str) -> u32 {
    doc.get(key).and_then(|v| v.as_u64()).unwrap_or(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_filters_short_and_stopwords_and_dedupes() {
        let terms = tokenize("Where is the User authentication handled?");
        // "is" too short; "where"/"the" are stopwords; lowercased, order kept.
        assert_eq!(terms, vec!["user", "authentication", "handled"]);
    }

    #[test]
    fn tokenize_dedupes_repeats_and_caps() {
        let terms = tokenize("user User USER order");
        assert_eq!(terms, vec!["user", "order"]);
    }

    #[test]
    fn path_matches_honours_prefix_and_none() {
        let back = serde_json::json!({ "file": "api/Edifice/Invoice.cs" });
        let front = serde_json::json!({ "file": "app/src/views/Invoice.vue" });
        // No filter → everything matches.
        assert!(path_matches(&back, None));
        assert!(path_matches(&front, None));
        // Prefix keeps only its side of the mono-repo.
        assert!(path_matches(&back, Some("api/")));
        assert!(!path_matches(&front, Some("api/")));
        assert!(path_matches(&front, Some("app/")));
        assert!(!path_matches(&back, Some("app/")));
        // Deeper prefixes narrow further.
        assert!(path_matches(&front, Some("app/src/views/")));
        // A missing `file` never matches a non-empty prefix.
        assert!(!path_matches(&serde_json::json!({}), Some("api/")));
    }

    #[test]
    fn kind_matches_filters_kinds() {
        let method = serde_json::json!({ "kind": "method" });
        let route = serde_json::json!({ "kind": "route" });
        assert!(kind_matches(&method, None));
        let kinds = vec!["method".to_string(), "controller".to_string()];
        assert!(kind_matches(&method, Some(&kinds)));
        assert!(!kind_matches(&route, Some(&kinds)));
    }

    #[test]
    fn keyword_score_prefers_name_over_body() {
        // Mirrors the AQL weighted LET expression used in keyword_seeds.
        let score = |terms: &[String], qn: &str, name: &str, sig: &str, text: &str| {
            let count = |hay: &str| {
                let lower = hay.to_lowercase();
                terms.iter().filter(|t| lower.contains(t.as_str())).count() as u32
            };
            count(qn) * 4 + count(name) * 3 + count(sig) * 2 + count(text)
        };
        let terms = tokenize("authentication");
        // Name field carries the term once; body-only also once — name weight (3)
        // beats body weight (1).
        let name_hit = score(&terms, "Other#x", "authentication", "", "body");
        let body_only = score(
            &terms,
            "Other#other",
            "other",
            "",
            "talks about authentication somewhere",
        );
        assert!(name_hit > body_only);
        assert_eq!(name_hit, 3);
        assert_eq!(body_only, 1);
    }

    #[test]
    fn edge_kind_rank_orders_routes_before_defines() {
        assert!(edge_kind_rank("routes_to") < edge_kind_rank("defines"));
        assert!(edge_kind_rank("calls") < edge_kind_rank("imports"));
        assert!(edge_kind_rank("redirects") < edge_kind_rank("defines"));
    }

    #[test]
    fn parse_kinds_splits_and_trims() {
        assert_eq!(
            parse_kinds("method, controller,route"),
            vec!["method", "controller", "route"]
        );
        assert!(parse_kinds("  ,  ").is_empty());
    }

    #[test]
    fn truncate_snippet_caps_length() {
        let long = "a".repeat(500);
        let s = truncate_snippet(&long);
        assert!(s.ends_with('…'));
        assert!(s.chars().count() <= SNIPPET_MAX + 1);
    }
}
