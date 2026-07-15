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

pub struct QueryOptions {
    pub database: Option<String>,
    /// Number of seed (most-relevant) nodes to return.
    pub limit: usize,
    /// Neighbour-expansion depth (1 = direct relationships).
    pub hops: usize,
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

    // Seeds: semantic first, keyword fallback.
    let (seeds, mode) = match semantic_seeds(&client, question, limit) {
        Some(v) if !v.is_empty() => (v, "semantic"),
        _ => (keyword_seeds(&client, question, limit)?, "keyword"),
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
fn semantic_seeds(
    client: &SoliDBClient,
    question: &str,
    limit: usize,
) -> Option<Vec<(Value, f64)>> {
    let vector = crate::embedding::generate_embedding(question)?;
    let hits = client
        .vector_search(NODE_COLLECTION, VECTOR_INDEX, &vector, limit, None)
        .ok()?;
    let seeds: Vec<(Value, f64)> = hits
        .into_iter()
        .filter_map(|hit| {
            let doc = hit.get("document")?.clone();
            let score = hit.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0);
            Some((doc, score))
        })
        .collect();
    Some(seeds)
}

/// Keyword-ranked fallback: score nodes by how many query terms their embedded
/// `text` contains.
fn keyword_seeds(
    client: &SoliDBClient,
    question: &str,
    limit: usize,
) -> Result<Vec<(Value, f64)>, String> {
    let terms = tokenize(question);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let query = "FOR n IN soli_graph_nodes \
         LET s = LENGTH(FOR t IN @terms FILTER CONTAINS(LOWER(n.text), t) RETURN 1) \
         FILTER s > 0 SORT s DESC LIMIT @lim RETURN MERGE(n, { _score: s })";
    let mut binds = HashMap::new();
    binds.insert("terms".to_string(), serde_json::json!(terms));
    binds.insert("lim".to_string(), serde_json::json!(limit));
    let rows = client
        .query(query, Some(binds))
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
    // `efrom` (not `from`, a reserved AQL object key) carries the edge origin so
    // we can label direction relative to the seed.
    let query = format!(
        "FOR v, e IN 1..{} ANY \"{}\" soli_graph_edges LIMIT {} \
         RETURN {{ efrom: e._from, edge_kind: e.edge_kind, kind: v.kind, \
         name: v.qualified_name, file: v.file, line: v.line }}",
        hops, start, MAX_NEIGHBORS
    );
    let rows = client
        .query(&query, None)
        .map_err(|e| format!("neighbour traversal failed: {}", e))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let efrom = field(&row, "efrom");
            let direction = if efrom == start { "out" } else { "in" };
            Neighbor {
                direction: direction.to_string(),
                edge_kind: field(&row, "edge_kind"),
                kind: field(&row, "kind"),
                name: field(&row, "name"),
                file: field(&row, "file"),
                line: uint(&row, "line"),
            }
        })
        .collect())
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
}
