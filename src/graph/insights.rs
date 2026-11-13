//! What agents learn about the code, kept beside the graph.
//!
//! The graph is rebuilt from the source; what an agent concluded while
//! working (why a method exists, which flow a question really maps to) is
//! not in the source, so it was lost after every session. Two collections
//! keep it:
//!
//! - `soli_graph_insights` — notes written with `soli graph note`, each
//!   anchored to the graph nodes it is about. A later `soli graph query`
//!   returns the notes that match the question or touch its results, and
//!   brings their anchors in as results. A note whose anchor's code changed
//!   since it was written is reported `stale`.
//! - `soli_graph_queries` — every `soli graph query` question with the code it
//!   returned, so a later question that shares its words sees what was asked
//!   before and where it led.
//!
//! `soli graph build` (even `--fresh`) never touches either collection: node
//! keys are derived from the code's names, so an anchor survives a rebuild.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::Value;

use crate::graph::model::{text_fingerprint, NODE_COLLECTION};
use crate::solidb_http::SoliDBClient;

/// One document per note.
pub const INSIGHT_COLLECTION: &str = "soli_graph_insights";
/// Vector index over the notes' `embedding`.
pub const INSIGHT_INDEX: &str = "insight_vec";
/// One document per `soli graph query` question.
pub const QUERY_COLLECTION: &str = "soli_graph_queries";

/// The question log keeps this many entries, newest first.
const QUERY_LOG_CAP: usize = 1000;
/// Notes returned with a query, at most.
const MAX_INSIGHTS: usize = 5;
/// Past questions returned with a query, at most.
const MAX_PAST_QUERIES: usize = 3;
/// Cosine similarity below which a note does not match a question.
const MIN_INSIGHT_SCORE: f64 = 0.4;

/// A note, as a query or `soli graph notes` returns it.
#[derive(Serialize, Clone, Debug)]
pub struct InsightHit {
    pub key: String,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub question: String,
    /// The qualified names of the nodes it is about.
    pub anchors: Vec<String>,
    /// An anchor's code changed (or disappeared) since the note was written.
    pub stale: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    /// Similarity to the question, when it matched semantically.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Graph keys of the anchors (to bring them in as results).
    #[serde(skip)]
    pub anchor_keys: Vec<String>,
}

/// An earlier question that shares words with this one.
#[derive(Serialize, Clone, Debug)]
pub struct PastQuery {
    pub question: String,
    pub at: String,
    /// The qualified names it returned.
    pub results: Vec<String>,
}

/// Write a note about `anchors` (qualified names or graph keys). The same text
/// on the same anchors updates the earlier note instead of adding one.
pub fn add_note(
    text: &str,
    anchors: &[String],
    question: Option<&str>,
    database: Option<&str>,
) -> Result<InsightHit, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a note needs text".to_string());
    }
    if anchors.is_empty() {
        return Err(
            "a note needs at least one anchor: --on <qualified name>, e.g. --on PostsController#create"
                .to_string(),
        );
    }
    let (client, _db) = crate::graph::sync::connect(database)?;
    let resolved = resolve_anchors(&client, anchors)?;
    let question = question.map(str::trim).unwrap_or("");

    let mut anchor_keys: Vec<&str> = resolved.iter().map(|a| a.key.as_str()).collect();
    anchor_keys.sort_unstable();
    let key = format!(
        "i{}",
        text_fingerprint(&format!("{}\u{1f}{}", text, anchor_keys.join(",")))
    );
    let created_at = chrono::Utc::now().to_rfc3339();
    let mut doc = serde_json::json!({
        "_key": key,
        "text": text,
        "question": question,
        "anchors": resolved
            .iter()
            .map(|a| serde_json::json!({ "key": a.key, "name": a.name, "fingerprint": a.fingerprint }))
            .collect::<Vec<_>>(),
        "created_at": created_at,
    });
    let embedding = crate::embedding::generate_embedding(&embed_text(question, text));
    if let Some(vector) = &embedding {
        doc["embedding"] = serde_json::json!(vector);
    }

    let _ = client.create_collection(INSIGHT_COLLECTION, None);
    // The client answers a missing document with an error, not `None`.
    let exists = matches!(client.get(INSIGHT_COLLECTION, &key), Ok(Some(_)));
    if exists {
        client
            .update(INSIGHT_COLLECTION, &key, doc, false)
            .map_err(|e| format!("update note: {}", e))?;
    } else {
        client
            .insert(INSIGHT_COLLECTION, Some(&key), doc)
            .map_err(|e| format!("write note: {}", e))?;
    }
    if let Some(vector) = &embedding {
        // A no-op once the index exists.
        let _ = client.create_vector_index(
            INSIGHT_COLLECTION,
            INSIGHT_INDEX,
            "embedding",
            vector.len(),
            "cosine",
            None,
        );
    }

    Ok(InsightHit {
        key,
        text: text.to_string(),
        question: question.to_string(),
        anchors: resolved.iter().map(|a| a.name.clone()).collect(),
        stale: false,
        created_at,
        score: None,
        anchor_keys: resolved.into_iter().map(|a| a.key).collect(),
    })
}

/// Every note, newest first, with its staleness.
pub fn list_notes(database: Option<&str>) -> Result<Vec<InsightHit>, String> {
    let (client, _db) = crate::graph::sync::connect(database)?;
    let rows = client
        .query(
            &format!(
                "FOR i IN {} SORT i.created_at DESC RETURN UNSET(i, \"embedding\")",
                INSIGHT_COLLECTION
            ),
            None,
        )
        .unwrap_or_default();
    let mut hits: Vec<InsightHit> = rows.iter().map(|row| hit_from(row, None)).collect();
    mark_stale(&client, &rows, &mut hits);
    Ok(hits)
}

/// Delete a note by its key.
pub fn forget_note(key: &str, database: Option<&str>) -> Result<(), String> {
    let (client, _db) = crate::graph::sync::connect(database)?;
    let found = matches!(client.get(INSIGHT_COLLECTION, key), Ok(Some(_)));
    if !found {
        return Err(format!(
            "no note with key {} (soli graph notes lists them)",
            key
        ));
    }
    client
        .delete(INSIGHT_COLLECTION, key)
        .map_err(|e| format!("delete note: {}", e))
}

/// The notes a query should see: those similar to the question (`vector`),
/// sharing its words when there is no vector, and those anchored on one of
/// `seed_keys`. Errors (no collection yet) read as no notes.
pub(crate) fn related_insights(
    client: &SoliDBClient,
    vector: Option<&[f64]>,
    terms: &[String],
    seed_keys: &[String],
) -> Vec<InsightHit> {
    let mut rows: Vec<(Value, Option<f64>)> = Vec::new();
    match vector {
        Some(vector) => {
            if let Ok(hits) = client.vector_search(
                INSIGHT_COLLECTION,
                INSIGHT_INDEX,
                vector,
                MAX_INSIGHTS,
                None,
            ) {
                for hit in hits {
                    let score = hit.get("score").and_then(Value::as_f64).unwrap_or(0.0);
                    if score < MIN_INSIGHT_SCORE {
                        continue;
                    }
                    if let Some(doc) = hit.get("document") {
                        rows.push((doc.clone(), Some(score)));
                    }
                }
            }
        }
        None if !terms.is_empty() => {
            let query = format!(
                "FOR i IN {} \
                 LET s = LENGTH(FOR t IN @terms FILTER CONTAINS(LOWER(i.text), t) \
                 OR CONTAINS(LOWER(i.question), t) RETURN 1) \
                 FILTER s >= @min SORT s DESC LIMIT @lim RETURN UNSET(i, \"embedding\")",
                INSIGHT_COLLECTION
            );
            let mut binds = HashMap::new();
            binds.insert("terms".to_string(), serde_json::json!(terms));
            binds.insert("min".to_string(), serde_json::json!(terms.len().min(2)));
            binds.insert("lim".to_string(), serde_json::json!(MAX_INSIGHTS));
            if let Ok(found) = client.query(&query, Some(binds)) {
                rows.extend(found.into_iter().map(|row| (row, None)));
            }
        }
        None => {}
    }
    if !seed_keys.is_empty() {
        let query = format!(
            "FOR i IN {} FILTER LENGTH(FOR a IN i.anchors FILTER a.key IN @keys RETURN 1) > 0 \
             SORT i.created_at DESC LIMIT @lim RETURN UNSET(i, \"embedding\")",
            INSIGHT_COLLECTION
        );
        let mut binds = HashMap::new();
        binds.insert("keys".to_string(), serde_json::json!(seed_keys));
        binds.insert("lim".to_string(), serde_json::json!(MAX_INSIGHTS));
        if let Ok(found) = client.query(&query, Some(binds)) {
            rows.extend(found.into_iter().map(|row| (row, None)));
        }
    }

    let mut seen = HashSet::new();
    rows.retain(|(row, _)| seen.insert(str_field(row, "_key")));
    rows.truncate(MAX_INSIGHTS);
    let docs: Vec<Value> = rows.iter().map(|(row, _)| row.clone()).collect();
    let mut hits: Vec<InsightHit> = rows
        .iter()
        .map(|(row, score)| hit_from(row, *score))
        .collect();
    mark_stale(client, &docs, &mut hits);
    hits
}

/// Earlier questions sharing at least two of `terms` (one, for a one-word
/// question), the current question aside.
pub(crate) fn past_queries(
    client: &SoliDBClient,
    question: &str,
    terms: &[String],
) -> Vec<PastQuery> {
    if terms.is_empty() {
        return Vec::new();
    }
    let query = format!(
        "FOR q IN {} FILTER q.question != @question \
         LET s = LENGTH(FOR t IN q.terms FILTER t IN @terms RETURN 1) \
         FILTER s >= @min SORT s DESC, q.at DESC LIMIT @lim \
         RETURN {{ question: q.question, at: q.at, results: q.results }}",
        QUERY_COLLECTION
    );
    let mut binds = HashMap::new();
    binds.insert("question".to_string(), serde_json::json!(question));
    binds.insert("terms".to_string(), serde_json::json!(terms));
    binds.insert("min".to_string(), serde_json::json!(terms.len().min(2)));
    binds.insert("lim".to_string(), serde_json::json!(MAX_PAST_QUERIES));
    let Ok(rows) = client.query(&query, Some(binds)) else {
        return Vec::new();
    };
    rows.iter()
        .map(|row| PastQuery {
            question: str_field(row, "question"),
            at: str_field(row, "at"),
            results: string_list(row, "results"),
        })
        .collect()
}

/// Record a question and the code it returned, keeping the log bounded. A
/// failure to record never fails the query.
pub(crate) fn log_query(
    client: &SoliDBClient,
    question: &str,
    terms: &[String],
    results: &[String],
) {
    if results.is_empty() {
        return;
    }
    let _ = client.create_collection(QUERY_COLLECTION, None);
    let doc = serde_json::json!({
        "question": question,
        "terms": terms,
        "results": results,
        "at": chrono::Utc::now().to_rfc3339(),
    });
    if client.insert(QUERY_COLLECTION, None, doc).is_err() {
        return;
    }
    let _ = client.query(
        &format!(
            "FOR q IN {coll} SORT q.at DESC LIMIT {cap}, 100000 REMOVE q IN {coll}",
            coll = QUERY_COLLECTION,
            cap = QUERY_LOG_CAP
        ),
        None,
    );
}

/// An anchor resolved against the graph.
struct Resolved {
    key: String,
    name: String,
    fingerprint: String,
}

/// Each name (a node's qualified name or its key) to its node; an error names
/// the ones the graph does not hold.
fn resolve_anchors(client: &SoliDBClient, names: &[String]) -> Result<Vec<Resolved>, String> {
    let query = format!(
        "FOR n IN {} FILTER n.qualified_name IN @names OR n._key IN @names \
         RETURN {{ key: n._key, name: n.qualified_name, text: n.text }}",
        NODE_COLLECTION
    );
    let mut binds = HashMap::new();
    binds.insert("names".to_string(), serde_json::json!(names));
    let rows = client.query(&query, Some(binds)).map_err(|e| {
        format!(
            "look up anchors: {} (has the graph been built? soli graph build)",
            e
        )
    })?;
    let mut resolved: Vec<Resolved> = Vec::new();
    for name in names {
        // A qualified name can belong to several nodes (`posts` the collection
        // and the view `posts`); a key is unique. Keep each match once.
        for row in rows
            .iter()
            .filter(|row| &str_field(row, "name") == name || &str_field(row, "key") == name)
        {
            let key = str_field(row, "key");
            if resolved.iter().any(|r| r.key == key) {
                continue;
            }
            resolved.push(Resolved {
                key,
                name: str_field(row, "name"),
                fingerprint: text_fingerprint(&str_field(row, "text")),
            });
        }
    }
    let missing: Vec<&str> = names
        .iter()
        .filter(|name| {
            !rows
                .iter()
                .any(|row| &&str_field(row, "name") == name || &&str_field(row, "key") == name)
        })
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "not in the code graph: {} (use the qualified_name `soli graph query` prints, e.g. PostsController#create)",
            missing.join(", ")
        ));
    }
    Ok(resolved)
}

/// Mark each hit whose anchor no longer exists, or whose code changed since
/// the note was written, as stale.
fn mark_stale(client: &SoliDBClient, docs: &[Value], hits: &mut [InsightHit]) {
    let keys: Vec<String> = hits
        .iter()
        .flat_map(|h| h.anchor_keys.iter().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if keys.is_empty() {
        return;
    }
    let query = format!(
        "FOR n IN {} FILTER n._key IN @keys RETURN {{ key: n._key, text: n.text }}",
        NODE_COLLECTION
    );
    let mut binds = HashMap::new();
    binds.insert("keys".to_string(), serde_json::json!(keys));
    let Ok(rows) = client.query(&query, Some(binds)) else {
        return;
    };
    let current: HashMap<String, String> = rows
        .iter()
        .map(|row| {
            (
                str_field(row, "key"),
                text_fingerprint(&str_field(row, "text")),
            )
        })
        .collect();
    for (hit, doc) in hits.iter_mut().zip(docs) {
        let anchors = doc.get("anchors").and_then(Value::as_array);
        hit.stale = anchors.is_some_and(|anchors| {
            anchors.iter().any(|anchor| {
                let key = str_field(anchor, "key");
                current.get(&key) != Some(&str_field(anchor, "fingerprint"))
            })
        });
    }
}

fn hit_from(row: &Value, score: Option<f64>) -> InsightHit {
    let anchors = row.get("anchors").and_then(Value::as_array);
    InsightHit {
        key: str_field(row, "_key"),
        text: str_field(row, "text"),
        question: str_field(row, "question"),
        anchors: anchors
            .map(|list| list.iter().map(|a| str_field(a, "name")).collect())
            .unwrap_or_default(),
        stale: false,
        created_at: str_field(row, "created_at"),
        score,
        anchor_keys: anchors
            .map(|list| list.iter().map(|a| str_field(a, "key")).collect())
            .unwrap_or_default(),
    }
}

/// The text a note is embedded from: its question, then its finding.
fn embed_text(question: &str, text: &str) -> String {
    if question.is_empty() {
        text.to_string()
    } else {
        format!("{}\n{}", question, text)
    }
}

fn str_field(doc: &Value, key: &str) -> String {
    doc.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn string_list(doc: &Value, key: &str) -> Vec<String> {
    doc.get(key)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_embedded_with_its_question_first() {
        assert_eq!(embed_text("", "uses Stripe"), "uses Stripe");
        assert_eq!(
            embed_text("how is a refund paid?", "Refund#perform calls Stripe"),
            "how is a refund paid?\nRefund#perform calls Stripe"
        );
    }

    #[test]
    fn a_hit_reads_its_anchors_names_and_keys() {
        let row = serde_json::json!({
            "_key": "iabc",
            "text": "the refund flow",
            "anchors": [
                { "key": "method:Refund.perform", "name": "Refund#perform", "fingerprint": "1" },
                { "key": "collection:refunds", "name": "refunds", "fingerprint": "2" }
            ],
            "created_at": "2026-11-13T10:00:00Z"
        });
        let hit = hit_from(&row, Some(0.7));
        assert_eq!(hit.anchors, vec!["Refund#perform", "refunds"]);
        assert_eq!(
            hit.anchor_keys,
            vec!["method:Refund.perform", "collection:refunds"]
        );
        assert_eq!(hit.score, Some(0.7));
        assert!(hit.question.is_empty());
    }
}
