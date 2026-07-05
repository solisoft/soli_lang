//! Search pushdown for the Model layer: DB-side vector (HNSW), fulltext, and
//! geo queries. All endpoints ride on `crud::exec_db_api_request`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::value::{Class, Value};

use super::crud::{exec_db_api_request, json_doc_to_instance, json_to_value};

/// One ANN hit: (doc_key, score, document).
pub struct VectorHit {
    pub doc_key: String,
    pub score: f64,
    pub document: serde_json::Value,
}

/// POST /_api/database/{db}/vector/{coll}/{index}/search
pub fn exec_vector_search(
    collection: &str,
    index_name: &str,
    vector: &[f64],
    limit: usize,
    ef_search: Option<usize>,
) -> Result<Vec<VectorHit>, String> {
    let mut body = serde_json::json!({ "vector": vector, "limit": limit });
    if let Some(ef) = ef_search {
        body["ef_search"] = serde_json::json!(ef);
    }
    let resp = exec_db_api_request(
        reqwest::Method::POST,
        &format!("/vector/{}/{}/search", collection, index_name),
        Some(body),
    )?;
    let hits = resp
        .get("results")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|hit| {
                    Some(VectorHit {
                        doc_key: hit.get("doc_key")?.as_str()?.to_string(),
                        score: hit.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0),
                        document: hit
                            .get("document")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(hits)
}

/// Fulltext search via the SDBQL FULLTEXT() function. Returns ranked
/// instances with `_search_score` (and `_highlighted` when requested).
/// The query text is always a bind var; collection/field/distance are
/// validated literals.
#[allow(clippy::too_many_arguments)]
pub fn exec_fulltext_search(
    collection: &str,
    class: &Rc<Class>,
    field: &str,
    query_text: &str,
    distance: usize,
    limit: usize,
    highlight: bool,
    drop_soft_deleted: bool,
) -> Result<Value, String> {
    let return_expr = if highlight {
        format!(
            "MERGE(r.doc, {{_search_score: r.score, _highlighted: HIGHLIGHT(r.doc.{}, [@__soli_q])}})",
            field
        )
    } else {
        "MERGE(r.doc, {_search_score: r.score})".to_string()
    };
    // FOR-over-function-call isn't in the DB grammar; bind the hits with a
    // LET first (verified shape).
    let sdbql = format!(
        "LET __soli_hits = FULLTEXT(\"{}\", \"{}\", @__soli_q, {}) FOR r IN __soli_hits \
         LIMIT {} RETURN {}",
        collection, field, distance, limit, return_expr
    );
    let mut binds = std::collections::HashMap::new();
    binds.insert(
        "__soli_q".to_string(),
        serde_json::Value::String(query_text.to_string()),
    );

    let rows = super::crud::exec_with_auto_collection(sdbql, Some(binds), collection)?;
    let instances: Vec<Value> = rows
        .iter()
        .filter(|doc| {
            // FULLTEXT bypasses the FILTER pipeline, so soft-deleted rows are
            // dropped client-side for soft-delete models.
            !(drop_soft_deleted && doc.get("deleted_at").map(|v| !v.is_null()).unwrap_or(false))
        })
        .map(|doc| json_doc_to_instance(class, doc))
        .collect();
    Ok(Value::Array(Rc::new(RefCell::new(instances))))
}

/// Geo near/within. `endpoint` is "near" or "within"; the third body key is
/// `limit` (near) or `radius` (within). Results become instances with a
/// `_distance` field (meters), mirroring the `_similarity_score` precedent.
pub fn exec_geo_query(
    collection: &str,
    class: &Rc<Class>,
    field: &str,
    endpoint: &str,
    lat: f64,
    lon: f64,
    third: (&str, f64),
    drop_soft_deleted: bool,
) -> Result<Value, String> {
    // `limit` deserializes as usize server-side; `radius` is a float.
    let third_val = if third.0 == "limit" {
        serde_json::json!(third.1 as u64)
    } else {
        serde_json::json!(third.1)
    };
    let body = serde_json::json!({ "lat": lat, "lon": lon, third.0: third_val });
    let resp = exec_db_api_request(
        reqwest::Method::POST,
        &format!("/geo/{}/{}/{}", collection, field, endpoint),
        Some(body),
    )?;
    let results = resp
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let instances: Vec<Value> = results
        .iter()
        .filter_map(|hit| {
            let doc = hit.get("document")?;
            if drop_soft_deleted && doc.get("deleted_at").map(|v| !v.is_null()).unwrap_or(false) {
                return None;
            }
            let distance = hit.get("distance").and_then(|d| d.as_f64()).unwrap_or(0.0);
            let inst = json_doc_to_instance(class, doc);
            if let Value::Instance(ref i) = inst {
                i.borrow_mut()
                    .set("_distance".to_string(), Value::Float(distance));
            }
            Some(inst)
        })
        .collect();
    Ok(Value::Array(Rc::new(RefCell::new(instances))))
}

/// Attach `_similarity_score` to an instance value (fresh copy, mirroring
/// the client-side path's behavior).
pub fn attach_score(value: Value, score: f64) -> Value {
    if let Value::Instance(inst) = &value {
        inst.borrow_mut()
            .set("_similarity_score".to_string(), Value::Float(score));
    }
    value
}

/// json_to_value re-export point for callers materializing raw rows.
pub fn raw_rows_to_values(rows: &[serde_json::Value]) -> Vec<Value> {
    rows.iter().map(json_to_value).collect()
}
