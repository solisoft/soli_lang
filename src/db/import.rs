//! SoliDB → SQL document-table import (Phase 3).
//!
//! Reads collections from a live SoliDB (SOLIDB_* env) and upserts each
//! document into the configured SQL adapter (`SOLI_DB_ADAPTER` +
//! `DATABASE_URL`) as `_key` + `doc` rows.

use super::sql;

/// Result summary for one collection.
#[derive(Debug, Clone)]
pub struct ImportCollectionResult {
    pub collection: String,
    pub imported: usize,
    pub errors: Vec<String>,
}

/// Import `collections` from SoliDB into the active SQL backend.
///
/// When `collections` is empty, lists SoliDB collections (excluding `_`-prefixed
/// system ones) and imports each.
pub fn import_collections(collections: &[String]) -> Result<Vec<ImportCollectionResult>, String> {
    if !sql::is_sql() {
        return Err(
            "soli db:import requires SOLI_DB_ADAPTER=postgres or mysql and DATABASE_URL. \
             See docs/sql-adapter-design.md."
                .into(),
        );
    }
    sql::ensure_connected()?;

    let names: Vec<String> = if collections.is_empty() {
        list_solidb_collections()?
    } else {
        collections.to_vec()
    };

    let mut results = Vec::new();
    for name in names {
        if name.starts_with('_') {
            continue;
        }
        results.push(import_one(&name)?);
    }
    Ok(results)
}

fn import_one(collection: &str) -> Result<ImportCollectionResult, String> {
    let docs = fetch_solidb_docs(collection)?;
    let mut imported = 0usize;
    let mut errors = Vec::new();
    sql::ensure_table(collection)?;
    for doc in docs {
        let key = doc
            .get("_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                doc.get("id").and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
            });
        match sql::insert(collection, key.as_deref(), doc) {
            Ok(_) => imported += 1,
            Err(e) => errors.push(e),
        }
    }
    Ok(ImportCollectionResult {
        collection: collection.to_string(),
        imported,
        errors,
    })
}

struct SolidbEndpoint {
    host: String,
    basic: Option<(String, String)>,
    api_key: Option<String>,
}

fn solidb_base() -> Result<SolidbEndpoint, String> {
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".into());
    let host = host.trim_end_matches('/').to_string();
    let user = std::env::var("SOLIDB_USERNAME").ok();
    let pass = std::env::var("SOLIDB_PASSWORD").ok();
    let basic = match (user, pass) {
        (Some(u), Some(p)) => Some((u, p)),
        _ => None,
    };
    let api_key = std::env::var("SOLIDB_API_KEY").ok();
    Ok(SolidbEndpoint {
        host,
        basic,
        api_key,
    })
}

fn solidb_database() -> String {
    std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".into())
}

fn list_solidb_collections() -> Result<Vec<String>, String> {
    let ep = solidb_base()?;
    let db = solidb_database();
    let url = format!("{}/_api/database/{db}/collection", ep.host);
    let body = http_get(&url, ep.basic.as_ref(), ep.api_key.as_deref())?;
    // Response shapes: { "result": ["posts", …] } or [ "posts", … ]
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(&body) {
        return Ok(arr);
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("list collections json: {e}: {body}"))?;
    if let Some(arr) = v.get("result").and_then(|x| x.as_array()) {
        return Ok(arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect());
    }
    if let Some(arr) = v.get("collections").and_then(|x| x.as_array()) {
        return Ok(arr
            .iter()
            .filter_map(|x| {
                x.as_str().map(|s| s.to_string()).or_else(|| {
                    x.get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                })
            })
            .collect());
    }
    Err(format!("unexpected collections response: {body}"))
}

fn fetch_solidb_docs(collection: &str) -> Result<Vec<serde_json::Value>, String> {
    let ep = solidb_base()?;
    let db = solidb_database();
    let url = format!("{}/_api/database/{db}/cursor", ep.host);
    let query = format!("FOR doc IN {collection} RETURN doc");
    let payload = serde_json::json!({ "query": query });
    let body = http_post_json(&url, &payload, ep.basic.as_ref(), ep.api_key.as_deref())?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("cursor json: {e}: {body}"))?;
    // { result: [ … ] } or { result: { result: […] } }
    if let Some(arr) = v.get("result").and_then(|x| x.as_array()) {
        return Ok(arr.clone());
    }
    Err(format!(
        "unexpected cursor response for {collection}: {body}"
    ))
}

fn http_get(
    url: &str,
    basic: Option<&(String, String)>,
    api_key: Option<&str>,
) -> Result<String, String> {
    let agent = ureq::Agent::new();
    let mut req = agent.get(url);
    if let Some((u, p)) = basic {
        req = req.set("Authorization", &basic_auth_header(u, p));
    }
    if let Some(k) = api_key {
        req = req.set("X-API-Key", k);
    }
    let resp = req.call().map_err(|e| format!("GET {url}: {e}"))?;
    resp.into_string()
        .map_err(|e| format!("GET {url} read: {e}"))
}

fn http_post_json(
    url: &str,
    body: &serde_json::Value,
    basic: Option<&(String, String)>,
    api_key: Option<&str>,
) -> Result<String, String> {
    let agent = ureq::Agent::new();
    let mut req = agent.post(url).set("Content-Type", "application/json");
    if let Some((u, p)) = basic {
        req = req.set("Authorization", &basic_auth_header(u, p));
    }
    if let Some(k) = api_key {
        req = req.set("X-API-Key", k);
    }
    let resp = req
        .send_string(&body.to_string())
        .map_err(|e| format!("POST {url}: {e}"))?;
    resp.into_string()
        .map_err(|e| format!("POST {url} read: {e}"))
}

fn basic_auth_header(user: &str, pass: &str) -> String {
    use base64::Engine;
    let raw = format!("{user}:{pass}");
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
    )
}
