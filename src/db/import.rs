//! SoliDB → SQL document-table import (Phase 3).
//!
//! Reads collections from a live SoliDB (SOLIDB_* env) and upserts each
//! document into the configured SQL adapter (`SOLI_DB_ADAPTER` +
//! `DATABASE_URL`) as `_key` + `doc` rows.

use super::sql;
use crate::solidb_http::SoliDBClient;

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

    let client = solidb_client()?;
    let names: Vec<String> = if collections.is_empty() {
        list_solidb_collections(&client)?
    } else {
        collections.to_vec()
    };

    let mut results = Vec::new();
    for name in names {
        if name.starts_with('_') {
            continue;
        }
        results.push(import_one(&client, &name)?);
    }
    Ok(results)
}

fn import_one(client: &SoliDBClient, collection: &str) -> Result<ImportCollectionResult, String> {
    let docs = fetch_solidb_docs(client, collection)?;
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

/// Build a SoliDB client from the same `SOLIDB_*` env the Model layer uses.
/// Auth priority mirrors `SoliDBClient::apply_auth`: JWT > API key > basic.
fn solidb_client() -> Result<SoliDBClient, String> {
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".into());
    let mut client = SoliDBClient::connect(&host).map_err(|e| format!("solidb connect: {e}"))?;
    if let Ok(jwt) = std::env::var("SOLIDB_JWT") {
        client = client.with_jwt_token(&jwt);
    }
    if let Ok(api_key) = std::env::var("SOLIDB_API_KEY") {
        client = client.with_api_key(&api_key);
    }
    if let (Ok(user), Ok(pass)) = (
        std::env::var("SOLIDB_USERNAME"),
        std::env::var("SOLIDB_PASSWORD"),
    ) {
        client = client.with_basic_auth(&user, &pass);
    }
    let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".into());
    client.set_database(&database);
    Ok(client)
}

fn list_solidb_collections(client: &SoliDBClient) -> Result<Vec<String>, String> {
    let items = client
        .list_collections()
        .map_err(|e| format!("list collections: {e}"))?;
    // Items are either bare names or {"name": …} objects.
    Ok(items
        .iter()
        .filter_map(|x| {
            x.as_str().map(|s| s.to_string()).or_else(|| {
                x.get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
        })
        .collect())
}

/// Full collection scan. `SoliDBClient::query` drains the cursor
/// (`has_more` + `PUT /_api/cursor/{id}`), so collections larger than one
/// batch (default 1000 docs) come back complete.
fn fetch_solidb_docs(
    client: &SoliDBClient,
    collection: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let query = format!("FOR doc IN {collection} RETURN doc");
    client
        .query(&query, None)
        .map_err(|e| format!("fetch {collection}: {e}"))
}
