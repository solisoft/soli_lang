//! Database CRUD operations and JSON conversion utilities.

use crate::interpreter::builtins::http_class::get_http_client;
use crate::interpreter::value::{Class, Instance, Value};
use crate::serve::get_tokio_handle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::core::{get_api_key, get_basic_auth, get_cursor_url, get_database_name, get_jwt_token};

/// Apply DB authentication headers.
/// Priority: JWT (fastest) > API key > Basic auth fallback.
fn apply_db_auth(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Some(jwt) = get_jwt_token() {
        builder.header("Authorization", format!("Bearer {}", jwt))
    } else if let Some(key) = get_api_key() {
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

// Thread-local transaction state for managing database transactions.
thread_local! {
    static CURRENT_TX: RefCell<Option<TransactionState>> = const { RefCell::new(None) };
}

pub struct TransactionState {
    pub tx_id: String,
    pub database: String,
    pub host: String,
}

/// Get the current transaction ID if one is active.
pub fn get_current_tx_id() -> Option<String> {
    CURRENT_TX.with(|tx| tx.borrow().as_ref().map(|t| t.tx_id.clone()))
}

/// Begin a new transaction.
pub fn begin_transaction(isolation_level: Option<&str>) -> Result<String, String> {
    let host = super::core::DB_CONFIG.host.clone();
    let database = get_database_name().to_string();
    let url = format!(
        "http://{}/_api/database/{}/transaction/begin",
        host, database
    );

    let body = serde_json::json!({
        "database": database,
        "isolationLevel": isolation_level.unwrap_or("read_committed")
    });

    run_db_future(async move {
        let client = get_http_client().clone();
        let request = apply_db_auth(
            client
                .request(reqwest::Method::POST, &url)
                .header("Content-Type", "application/json")
                .body(body.to_string()),
        );

        let response = request
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Begin transaction failed: {} - {}", status, body));
        }

        let json: serde_json::Value = serde_json::from_str(
            &response
                .text()
                .await
                .map_err(|e| format!("Read error: {}", e))?,
        )
        .map_err(|e| format!("JSON error: {}", e))?;

        let tx_id = json
            .get("tx_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No tx_id in response".to_string())?
            .to_string();

        CURRENT_TX.with(|tx| {
            *tx.borrow_mut() = Some(TransactionState {
                tx_id: tx_id.clone(),
                database: database.clone(),
                host: host.clone(),
            });
        });

        Ok(tx_id)
    })
}

/// Commit the current transaction.
pub fn commit_transaction() -> Result<(), String> {
    let tx_id = get_current_tx_id().ok_or_else(|| "No active transaction".to_string())?;

    let host = super::core::DB_CONFIG.host.clone();
    let database = get_database_name().to_string();
    let url = format!(
        "http://{}/_api/database/{}/transaction/{}/commit",
        host, database, tx_id
    );

    run_db_future(async move {
        let client = get_http_client().clone();
        let request = apply_db_auth(client.request(reqwest::Method::POST, &url));

        let response = request
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Commit transaction failed: {} - {}", status, body));
        }

        CURRENT_TX.with(|tx| {
            tx.borrow_mut().take();
        });
        Ok(())
    })
}

/// Rollback the current transaction.
pub fn rollback_transaction() -> Result<(), String> {
    let tx_id = get_current_tx_id().ok_or_else(|| "No active transaction".to_string())?;

    let host = super::core::DB_CONFIG.host.clone();
    let database = get_database_name().to_string();
    let url = format!(
        "http://{}/_api/database/{}/transaction/{}/rollback",
        host, database, tx_id
    );

    run_db_future(async move {
        let client = get_http_client().clone();
        let request = apply_db_auth(client.request(reqwest::Method::POST, &url));

        let response = request
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "Rollback transaction failed: {} - {}",
                status, body
            ));
        }

        CURRENT_TX.with(|tx| {
            tx.borrow_mut().take();
        });
        Ok(())
    })
}

/// Execute DB operation that returns serde_json::Value directly.
/// This skips the double JSON conversion (Value -> String -> Value).
pub fn exec_db_json<F>(f: F) -> Value
where
    F: FnOnce() -> Result<serde_json::Value, String>,
{
    match f() {
        Ok(json) => crate::interpreter::value::json_to_value(json).unwrap_or(Value::Null),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Convert a serde_json::Value reference to a Soli Value (infallible wrapper).
pub fn json_to_value(json: &serde_json::Value) -> Value {
    crate::interpreter::value::json_to_value_ref(json).unwrap_or(Value::Null)
}

/// Normalize a document key: "default:users/UUID" → "UUID" (strip everything up to last '/').
pub fn normalize_key(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// Convert a JSON document to a class instance with all fields set.
pub fn json_doc_to_instance(class: &Rc<Class>, json: &serde_json::Value) -> Value {
    let mut instance = Instance::new(class.clone());
    if let serde_json::Value::Object(map) = json {
        for (k, v) in map {
            instance.set(k.clone(), json_to_value(v));
        }
    }
    Value::Instance(Rc::new(RefCell::new(instance)))
}

/// Execute query returning class instances with automatic collection creation.
pub fn exec_auto_collection_as_instances(
    sdbql: String,
    collection_name: &str,
    class: &Rc<Class>,
) -> Value {
    match exec_with_auto_collection(sdbql, None, collection_name) {
        Ok(results) => {
            let values: Vec<Value> = results
                .iter()
                .map(|json| json_doc_to_instance(class, json))
                .collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Execute query with binds returning class instances with automatic collection creation.
pub fn exec_auto_collection_as_instances_with_binds(
    sdbql: String,
    bind_vars: HashMap<String, serde_json::Value>,
    collection_name: &str,
    class: &Rc<Class>,
) -> Value {
    match exec_with_auto_collection(sdbql, Some(bind_vars), collection_name) {
        Ok(results) => {
            let values: Vec<Value> = results
                .iter()
                .map(|json| json_doc_to_instance(class, json))
                .collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        Err(e) => Value::String(format!("Error: {}", e)),
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
    // Use environment variables for credentials
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
    let api_key = std::env::var("SOLIDB_API_KEY").unwrap_or_else(|_| {
        eprintln!("WARNING: SOLIDB_API_KEY not set, database queries will fail");
        String::new()
    });
    let body = format!(r#"{{"query":"{}"}}"#, sdbql.replace('"', r#"\""#));

    let client = get_http_client().clone();
    let db_name = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "solipay".to_string());
    let url = format!(
        "{}/_api/database/{}/cursor",
        host.trim_end_matches('/'),
        db_name
    );
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

/// Build the base URL for document operations: http://{host}/_api/database/{db}/document/{collection}
fn document_base_url(collection: &str) -> String {
    format!(
        "http://{}/_api/database/{}/document/{}",
        DB_CONFIG.host,
        get_database_name(),
        collection
    )
}

/// Execute a database transaction with SDBQL queries.
/// The action is a string containing SDBQL statements to execute within the transaction.
pub fn exec_transaction(action: &str) -> Result<serde_json::Value, String> {
    let client = get_http_client().clone();
    let host = DB_CONFIG.host.clone();
    let _database = get_database_name();
    let url = format!("http://{}/_api/transaction", host);

    let body = serde_json::json!({
        "collections": {
            "allow": [],
            "exclusive": [],
            "write": []
        },
        "action": action
    });

    run_db_future(async move {
        let mut request = apply_db_auth(client.request(reqwest::Method::POST, &url));
        request = request.header("Content-Type", "application/json");

        let response = request
            .body(serde_json::to_string(&body).map_err(|e| e.to_string())?)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("Transaction failed: {} - {}", status, error_text));
        }

        let json: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        Ok(json)
    })
}

/// Execute a transaction with a single SDBQL query string.
pub fn exec_transaction_sdbql(sdbql: &str) -> Result<serde_json::Value, String> {
    let action = format!(
        "function() {{ return AQL_QUERY('{}', {{}}); }}",
        sdbql.replace("'", "\\'")
    );
    exec_transaction(&action)
}

/// Execute a direct HTTP document operation using the shared runtime and client.
fn exec_document_request(
    method: reqwest::Method,
    url: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let client = get_http_client().clone();

    run_db_future(async move {
        let mut request = apply_db_auth(client.request(method, &url));
        if let Some(b) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(b.to_string());
        }

        let resp = request
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {} {}: {}", status, url, body));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| format!("Read error: {}", e))?;
        if text.is_empty() {
            Ok(serde_json::Value::Null)
        } else {
            serde_json::from_str(&text).map_err(|e| format!("JSON error: {}", e))
        }
    })
}

/// Execute an insert with automatic collection creation.
pub fn exec_insert(
    collection: &str,
    key: Option<&str>,
    mut document: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if let Some(k) = key {
        if let Some(obj) = document.as_object_mut() {
            obj.insert("_key".to_string(), serde_json::json!(k));
        }
    }
    let url = document_base_url(collection);
    let result = exec_document_request(reqwest::Method::POST, url.clone(), Some(document.clone()));

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            return exec_document_request(reqwest::Method::POST, url, Some(document));
        }
    }
    result
}

/// Execute a get with automatic collection creation.
pub fn exec_get(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    let url = format!("{}/{}", document_base_url(collection), normalize_key(key));
    let result = exec_document_request(reqwest::Method::GET, url.clone(), None);

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            return exec_document_request(reqwest::Method::GET, url, None);
        }
    }
    result
}

/// Execute an update (PUT) with automatic collection creation.
pub fn exec_update(
    collection: &str,
    key: &str,
    document: serde_json::Value,
    _merge: bool,
) -> Result<serde_json::Value, String> {
    let url = format!("{}/{}", document_base_url(collection), normalize_key(key));
    let result = exec_document_request(reqwest::Method::PUT, url.clone(), Some(document.clone()));

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            return exec_document_request(reqwest::Method::PUT, url, Some(document));
        }
    }
    result
}

/// Execute a delete with automatic collection creation.
pub fn exec_delete(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    let url = format!("{}/{}", document_base_url(collection), normalize_key(key));
    let result = exec_document_request(reqwest::Method::DELETE, url.clone(), None);

    if let Err(ref e) = result {
        if is_collection_not_found_error(e) {
            create_collection_sync(collection)?;
            return exec_document_request(reqwest::Method::DELETE, url, None);
        }
    }
    result
}

/// Execute a query with automatic collection creation.
pub fn exec_query(collection: &str, sdbql: String) -> Result<Vec<serde_json::Value>, String> {
    exec_with_auto_collection(sdbql, None, collection)
}

/// Transaction-aware document insert.
/// If a transaction is active, uses the transaction endpoint.
pub fn exec_insert_tx(
    collection: &str,
    key: Option<&str>,
    mut document: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if let Some(k) = key {
        if let Some(obj) = document.as_object_mut() {
            obj.insert("_key".to_string(), serde_json::json!(k));
        }
    }

    if let Some(tx_id) = get_current_tx_id() {
        let host = super::core::DB_CONFIG.host.clone();
        let database = get_database_name().to_string();
        let url = format!(
            "http://{}/_api/database/{}/transaction/{}/document/{}",
            host, database, tx_id, collection
        );
        exec_document_request(reqwest::Method::POST, url, Some(document))
    } else {
        exec_insert(collection, key, document)
    }
}

/// Transaction-aware document get.
/// If a transaction is active, uses the transaction endpoint.
pub fn exec_get_tx(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    if let Some(tx_id) = get_current_tx_id() {
        let host = super::core::DB_CONFIG.host.clone();
        let database = get_database_name().to_string();
        let url = format!(
            "http://{}/_api/database/{}/transaction/{}/document/{}/{}",
            host,
            database,
            tx_id,
            collection,
            normalize_key(key)
        );
        exec_document_request(reqwest::Method::GET, url, None)
    } else {
        exec_get(collection, key)
    }
}

/// Transaction-aware document update.
/// If a transaction is active, uses the transaction endpoint.
pub fn exec_update_tx(
    collection: &str,
    key: &str,
    document: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if let Some(tx_id) = get_current_tx_id() {
        let host = super::core::DB_CONFIG.host.clone();
        let database = get_database_name().to_string();
        let url = format!(
            "http://{}/_api/database/{}/transaction/{}/document/{}/{}",
            host,
            database,
            tx_id,
            collection,
            normalize_key(key)
        );
        exec_document_request(reqwest::Method::PUT, url, Some(document))
    } else {
        exec_update(collection, key, document, false)
    }
}

/// Transaction-aware document delete.
/// If a transaction is active, uses the transaction endpoint.
pub fn exec_delete_tx(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    if let Some(tx_id) = get_current_tx_id() {
        let host = super::core::DB_CONFIG.host.clone();
        let database = get_database_name().to_string();
        let url = format!(
            "http://{}/_api/database/{}/transaction/{}/document/{}/{}",
            host,
            database,
            tx_id,
            collection,
            normalize_key(key)
        );
        exec_document_request(reqwest::Method::DELETE, url, None)
    } else {
        exec_delete(collection, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_key_composite_id() {
        assert_eq!(normalize_key("default:users/abc123"), "abc123");
    }

    #[test]
    fn test_normalize_key_with_multiple_slashes() {
        assert_eq!(normalize_key("default:some/nested/path/uuid"), "uuid");
    }

    #[test]
    fn test_normalize_key_plain_key() {
        assert_eq!(normalize_key("abc123"), "abc123");
    }

    #[test]
    fn test_normalize_key_empty() {
        assert_eq!(normalize_key(""), "");
    }

    #[test]
    fn test_normalize_key_trailing_slash() {
        assert_eq!(normalize_key("default:users/"), "");
    }

    #[test]
    fn test_json_doc_to_instance_creates_instance() {
        let class = Rc::new(Class::default());
        let json = serde_json::json!({
            "_key": "abc-xyz",
            "_id": "default:test/abc-xyz",
            "name": "Alice",
            "active": true
        });
        let val = json_doc_to_instance(&class, &json);
        match val {
            Value::Instance(inst) => {
                let inst_ref = inst.borrow();
                assert_eq!(
                    inst_ref.get("_key"),
                    Some(Value::String("abc-xyz".to_string()))
                );
                assert_eq!(
                    inst_ref.get("name"),
                    Some(Value::String("Alice".to_string()))
                );
                assert_eq!(inst_ref.get("active"), Some(Value::Bool(true)));
            }
            _ => panic!("Expected Value::Instance"),
        }
    }

    #[test]
    fn test_json_doc_to_instance_with_nested_values() {
        let class = Rc::new(Class::default());
        let json = serde_json::json!({
            "count": 42,
            "score": std::f64::consts::PI
        });
        let val = json_doc_to_instance(&class, &json);
        match val {
            Value::Instance(inst) => {
                let inst_ref = inst.borrow();
                assert_eq!(inst_ref.get("count"), Some(Value::Int(42)));
                assert_eq!(
                    inst_ref.get("score"),
                    Some(Value::Float(std::f64::consts::PI))
                );
            }
            _ => panic!("Expected Value::Instance"),
        }
    }

    #[test]
    fn test_json_doc_to_instance_with_null_json() {
        let class = Rc::new(Class::default());
        let json = serde_json::Value::Null;
        let val = json_doc_to_instance(&class, &json);
        match val {
            Value::Instance(inst) => {
                assert!(inst.borrow().fields.is_empty());
            }
            _ => panic!("Expected Value::Instance"),
        }
    }

    #[test]
    fn test_json_doc_to_instance_preserves_class() {
        let class = Rc::new(Class {
            name: "User".to_string(),
            ..Default::default()
        });
        let json = serde_json::json!({ "name": "Bob" });
        let val = json_doc_to_instance(&class, &json);
        match val {
            Value::Instance(inst) => {
                assert_eq!(inst.borrow().class.name, "User");
            }
            _ => panic!("Expected Value::Instance"),
        }
    }
}
