//! Idempotent reconciliation of class-body index declarations (`index`,
//! `vector_index`, `fulltext_index`, `geo_index`) against the database.
//!
//! Declarations are metadata-only at class-load time. This module lists the
//! existing indexes per collection and creates only what's missing BY NAME —
//! list-first avoids depending on server 409 semantics, and concurrent
//! creates across workers surface as non-fatal warnings.
//!
//! Invoked (1) at dev-server boot after models load, and (2) by
//! `soli db:indexes` for production/deploy pipelines. Migrations remain the
//! recommended production DDL path; this is the DSL reconciler.

use super::crud::exec_db_api_request;

/// Sync every declared index. Returns human-readable lines describing what
/// was created (empty = everything already existed). Failures are collected
/// as warning lines rather than aborting the sweep.
pub fn sync_declared_indexes() -> Vec<String> {
    let mut report = Vec::new();

    for (collection, secondary, vector, fulltext, geo) in super::registry::all_declared_indexes() {
        // --- secondary + fulltext (same route family)
        let existing = list_names(&format!("/index/{}", collection), "indexes");
        for def in &secondary {
            if existing.contains(&def.name) {
                continue;
            }
            let body = serde_json::json!({
                "name": def.name,
                "fields": def.fields,
                "type": def.index_type,
                "unique": def.unique,
            });
            report.push(create(
                &format!("/index/{}", collection),
                body,
                &collection,
                &def.name,
                &def.index_type,
            ));
        }
        for def in &fulltext {
            if existing.contains(&def.name) {
                continue;
            }
            let body = serde_json::json!({
                "name": def.name,
                "fields": def.fields,
                "type": "fulltext",
                "unique": false,
            });
            report.push(create(
                &format!("/index/{}", collection),
                body,
                &collection,
                &def.name,
                "fulltext",
            ));
        }

        // --- vector (own route)
        if !vector.is_empty() {
            let existing = list_names(&format!("/vector/{}", collection), "indexes");
            for def in &vector {
                if existing.contains(&def.name) {
                    continue;
                }
                let mut body = serde_json::json!({
                    "name": def.name,
                    "field": def.field,
                    "dimension": def.dimension,
                });
                if let Some(metric) = &def.metric {
                    body["metric"] = serde_json::json!(metric);
                }
                if let Some(m) = def.m {
                    body["m"] = serde_json::json!(m);
                }
                if let Some(ef) = def.ef_construction {
                    body["ef_construction"] = serde_json::json!(ef);
                }
                if let Some(q) = &def.quantization {
                    body["quantization"] = serde_json::json!(q);
                }
                report.push(create(
                    &format!("/vector/{}", collection),
                    body,
                    &collection,
                    &def.name,
                    "vector",
                ));
            }
        }

        // --- geo (own route)
        if !geo.is_empty() {
            let existing = list_names(&format!("/geo/{}", collection), "indexes");
            for def in &geo {
                if existing.contains(&def.name) {
                    continue;
                }
                let body = serde_json::json!({ "name": def.name, "field": def.field });
                report.push(create(
                    &format!("/geo/{}", collection),
                    body,
                    &collection,
                    &def.name,
                    "geo",
                ));
            }
        }
    }

    report
}

/// Existing index names at a list endpoint. A missing collection (or any
/// error) reads as "no indexes" — creation will surface the real problem.
fn list_names(path: &str, key: &str) -> Vec<String> {
    match exec_db_api_request(reqwest::Method::GET, path, None) {
        Ok(resp) => resp
            .get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|i| {
                        i.get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn create(path: &str, body: serde_json::Value, collection: &str, name: &str, kind: &str) -> String {
    let result = match exec_db_api_request(reqwest::Method::POST, path, Some(body.clone())) {
        // At boot time the collection may not exist yet (first deploy, empty
        // dev DB) — create it (typed, when the model declared a type) and
        // retry the index once.
        Err(e) if e.to_lowercase().contains("collectionnotfound") || e.contains("404") => {
            let mut coll_body = serde_json::json!({ "name": collection });
            if let Some(ctype) = super::registry::get_collection_type(collection) {
                coll_body["type"] = serde_json::Value::String(ctype);
            }
            let _ = exec_db_api_request(reqwest::Method::POST, "/collection", Some(coll_body));
            exec_db_api_request(reqwest::Method::POST, path, Some(body))
        }
        other => other,
    };
    match result {
        Ok(_) => format!("created {} index {} on {}", kind, name, collection),
        // Concurrent worker boots may race the create; already-exists is fine.
        Err(e) if e.contains("409") || e.to_lowercase().contains("exists") => {
            format!("{} index {} on {} already exists", kind, name, collection)
        }
        Err(e) => format!(
            "WARNING: could not create {} index {} on {}: {}",
            kind, name, collection, e
        ),
    }
}
