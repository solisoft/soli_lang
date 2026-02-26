//! Database CRUD operations and JSON conversion utilities.

use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::builtins::http_class::get_http_client;
use crate::interpreter::value::{HashKey, Value};
use crate::serve::get_tokio_handle;

use super::core::{get_api_key, get_basic_auth, get_cursor_url, get_database_name};

/// Apply DB authentication headers (API key or basic auth) to a request.
fn apply_db_auth(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Some(key) = get_api_key() {
        builder.header("X-API-Key", key)
    } else if let Some(auth) = get_basic_auth() {
        builder.header("Authorization", auth)
    } else {
        builder
    }
}

// Fallback tokio runtime for DB operations outside of a server context
// (e.g., REPL, scripts). Uses a lightweight current-thread runtime instead
// of a full multi-thread runtime to save ~1-2MB of RSS.
thread_local! {
    static FALLBACK_RT: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create fallback tokio runtime");
}

/// Run a future using the worker's thread-local tokio handle,
/// falling back to a lightweight per-thread runtime (e.g. from the REPL).
fn run_db_future<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    if let Some(rt) = get_tokio_handle() {
        rt.block_on(future)
    } else {
        FALLBACK_RT.with(|rt| rt.block_on(future))
    }
}

/// Execute DB operation that returns serde_json::Value directly.
/// This skips the double JSON conversion (Value -> String -> Value).
pub fn exec_db_json<F>(f: F) -> Value
where
    F: FnOnce() -> Result<serde_json::Value, String>,
{
    match f() {
        Ok(json) => json_to_value(&json),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

pub fn json_to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            let values: Vec<Value> = arr.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        serde_json::Value::Object(obj) => {
            let mut pairs: IndexMap<HashKey, Value> = IndexMap::new();
            for (k, v) in obj.iter() {
                pairs.insert(HashKey::String(k.clone()), json_to_value(v));
            }
            Value::Hash(Rc::new(RefCell::new(pairs)))
        }
    }
}

/// Fast async query execution - uses server's tokio runtime.
/// Uses same HTTP client as HTTP.request for consistency.
pub fn exec_async_query_with_binds(
    sdbql: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
) -> Result<Vec<serde_json::Value>, String> {
    // Get cached values (initialized on first use after .env is loaded)
    let url = get_cursor_url();

    let client = get_http_client().clone();

    let future = async move {
        let mut payload = serde_json::json!({ "query": sdbql });
        if let Some(bv) = bind_vars {
            payload["bindVars"] = serde_json::json!(bv);
        }

        let request = apply_db_auth(
            client
                .post(url)
                .header("Content-Type", "application/json")
                .body(payload.to_string()),
        );

        let resp = request
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Query failed: {} - {}", status, body));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("JSON error: {}", e))?;
        Ok(json
            .get("result")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default())
    };

    run_db_future(future)
}

/// Simple async query without bind variables - convenience wrapper.
pub fn exec_async_query(sdbql: String) -> Value {
    match exec_async_query_with_binds(sdbql, None) {
        Ok(results) => {
            // Direct conversion without intermediate Array wrapper
            let values: Vec<Value> = results.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Async query returning raw JSON string (no Value conversion - fastest).
/// Uses same HTTP client as HTTP.request for consistency.
pub fn exec_async_query_raw(sdbql: String) -> Value {
    // Get cached values (initialized on first use after .env is loaded)
    let url = get_cursor_url();
    let body = format!(r#"{{"query":"{}"}}"#, sdbql.replace('"', r#"\"#));

    let client = get_http_client().clone();
    match run_db_future(async move {
        let request = apply_db_auth(
            client
                .post(url)
                .header("Content-Type", "application/json")
                .body(body),
        );

        let resp = request
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Query failed: {} - {}", status, body));
        }

        resp.text().await.map_err(|e| format!("Read error: {}", e))
    }) {
        Ok(text) => Value::String(text),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Hardcoded query - exact same pattern as HTTP.request for comparison.
pub fn exec_query_hardcoded(sdbql: String) -> Value {
    // Hardcoded values - exactly like test-cursor does
    let url = "http://localhost:6745/_api/database/solipay/cursor";
    let api_key = "sk_8bc935c8fc837e147a0ab100747d197b354d5cdf80635bbd5951bc1a313a1ab8";
    let body = format!(r#"{{"query":"{}"}}"#, sdbql.replace('"', r#"\"#));

    let client = get_http_client().clone();
    match run_db_future(async move {
        let request = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-API-Key", api_key)
            .body(body);

        let resp = request.send().await.map_err(|e| e.to_string())?;
        resp.text().await.map_err(|e| e.to_string())
    }) {
        Ok(text) => Value::String(text),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

fn is_collection_not_found_error(error: &str) -> bool {
    let error_lower = error.to_lowercase();
    error_lower.contains("collection")
        && (error_lower.contains("not found")
            || error_lower.contains("not exist")
            || error_lower.contains("does not exist")
            || error_lower.contains("unknown collection"))
}

fn create_collection_sync(name: &str) -> Result<(), String> {
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
    let host = host
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .to_string();
    let database = get_database_name();
    let url = format!("http://{}/_api/database/{}/collection", host, database);

    let body = serde_json::json!({ "name": name }).to_string();

    let client = get_http_client().clone();

    run_db_future(async move {
        let request = apply_db_auth(
            client
                .post(&url)
                .header("Content-Type", "application/json")
                .body(body),
        );

        let resp = request
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Create collection failed: {} - {}", status, body));
        }

        Ok(())
    })
}

/// Execute query with automatic collection creation on collection-related error.
/// This is used for Model operations to auto-create collections when they don't exist.
pub fn exec_with_auto_collection(
    sdbql: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
    collection_name: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let result = exec_async_query_with_binds(sdbql.clone(), bind_vars.clone());

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            if let Err(create_err) = create_collection_sync(collection_name) {
                return Err(format!(
                    "Collection '{}' not found, and failed to create it: {}",
                    collection_name, create_err
                ));
            }
            return exec_async_query_with_binds(sdbql, bind_vars);
        }
    }

    result
}

/// Execute query returning Value with automatic collection creation.
pub fn exec_auto_collection(sdbql: String, collection_name: &str) -> Value {
    match exec_with_auto_collection(sdbql, None, collection_name) {
        Ok(results) => {
            let values: Vec<Value> = results.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Execute query with binds returning Value with automatic collection creation.
pub fn exec_auto_collection_with_binds(
    sdbql: String,
    bind_vars: HashMap<String, serde_json::Value>,
    collection_name: &str,
) -> Value {
    match exec_with_auto_collection(sdbql, Some(bind_vars), collection_name) {
        Ok(results) => {
            let values: Vec<Value> = results.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

use crate::interpreter::builtins::model::core::DB_CONFIG;
use crate::solidb_http::SoliDBClient;

/// Create a SoliDBClient configured with database and auth from the environment.
fn create_db_client() -> Result<SoliDBClient, String> {
    let mut client =
        SoliDBClient::connect(&DB_CONFIG.host).map_err(|e| format!("Failed to connect: {}", e))?;
    client.set_database(get_database_name());
    if let Some(key) = get_api_key() {
        client = client.with_api_key(key);
    } else if let (Ok(user), Ok(pass)) = (
        std::env::var("SOLIDB_USERNAME"),
        std::env::var("SOLIDB_PASSWORD"),
    ) {
        client = client.with_basic_auth(&user, &pass);
    }
    Ok(client)
}

/// Execute a SoliDBClient insert operation with automatic collection creation.
pub fn exec_insert(
    collection: &str,
    key: Option<&str>,
    document: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = create_db_client()?;

    let result = client
        .insert(collection, key, document.clone())
        .map_err(|e| format!("Create failed: {}", e));

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            let client = create_db_client()?;
            return client
                .insert(collection, key, document)
                .map_err(|e| format!("Create failed: {}", e));
        }
    }

    result
}

/// Execute a SoliDBClient get operation with automatic collection creation.
pub fn exec_get(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    let client = create_db_client()?;

    let result = client
        .get(collection, key)
        .map_err(|e| format!("Find failed: {}", e))
        .map(|doc| doc.unwrap_or(serde_json::Value::Null));

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            let client = create_db_client()?;
            return client
                .get(collection, key)
                .map_err(|e| format!("Find failed: {}", e))
                .map(|doc| doc.unwrap_or(serde_json::Value::Null));
        }
    }

    result
}

/// Execute a SoliDBClient update operation with automatic collection creation.
pub fn exec_update(
    collection: &str,
    key: &str,
    document: serde_json::Value,
    merge: bool,
) -> Result<serde_json::Value, String> {
    let client = create_db_client()?;

    let result = client
        .update(collection, key, document.clone(), merge)
        .map_err(|e| format!("Update failed: {}", e));

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            let client = create_db_client()?;
            return client
                .update(collection, key, document, merge)
                .map_err(|e| format!("Update failed: {}", e))
                .map(|_| serde_json::Value::String("Updated".to_string()));
        }
    }

    result.map(|_| serde_json::Value::String("Updated".to_string()))
}

/// Execute a SoliDBClient delete operation with automatic collection creation.
pub fn exec_delete(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    let client = create_db_client()?;

    let result = client
        .delete(collection, key)
        .map_err(|e| format!("Delete failed: {}", e));

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            let client = create_db_client()?;
            return client
                .delete(collection, key)
                .map_err(|e| format!("Delete failed: {}", e))
                .map(|_| serde_json::Value::String("Deleted".to_string()));
        }
    }

    result.map(|_| serde_json::Value::String("Deleted".to_string()))
}

/// Execute a SoliDBClient query operation with automatic collection creation.
pub fn exec_query(collection: &str, sdbql: String) -> Result<Vec<serde_json::Value>, String> {
    let client = create_db_client()?;

    let result = client
        .query(&sdbql, None)
        .map_err(|e| format!("Query failed: {}", e));

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            let client = create_db_client()?;
            return client
                .query(&sdbql, None)
                .map_err(|e| format!("Query failed: {}", e));
        }
    }

    result
}
