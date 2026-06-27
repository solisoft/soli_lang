//! Database CRUD operations and JSON conversion utilities.

use crate::interpreter::builtins::http_class::get_http_client;
use crate::interpreter::value::{Class, Instance, Value};
use crate::serve::get_tokio_handle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::core::{
    db_url, force_refresh_jwt_token, get_api_key, get_basic_auth, get_cursor_url,
    get_database_name, get_jwt_token,
};
#[allow(unused_imports)]
use super::registry::{clear_model_classes, get_model_class, register_model_class};

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

/// Send a SolidB request through `apply_db_auth`. On a 401 response we
/// invalidate the cached JWT (so the next `get_jwt_token()` re-logs in)
/// and retry the request once. The closure must rebuild the request from
/// scratch so the retry attaches the freshly-refreshed Authorization
/// header instead of the dead one.
///
/// SolidB JWTs expire after 24h (see `db_config::CachedJwt`). The pre-
/// emptive refresh in `get_jwt_token()` covers the common case; this
/// retry covers the corner cases where the token *was* still valid at
/// request-build time but the request itself comes back unauthorised
/// (server-side revocation, clock skew between client and DB, etc.).
async fn send_with_db_auth_retry<F>(make_request: F) -> Result<reqwest::Response, reqwest::Error>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let resp = apply_db_auth(make_request()).send().await?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        force_refresh_jwt_token();
        return apply_db_auth(make_request()).send().await;
    }
    Ok(resp)
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
///
/// If called from within an async runtime context, creates a dedicated single-thread
/// runtime to avoid blocking the I/O driver and causing potential deadlocks.
fn run_db_future<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>> + 'static,
{
    if let Some(rt) = get_tokio_handle() {
        if tokio::runtime::Handle::try_current().is_ok() {
            // Already inside async runtime — create a dedicated single-thread runtime
            // so we don't block the caller's I/O driver thread
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(future)
        } else {
            // Outside async context — safe to block
            rt.block_on(future)
        }
    } else {
        FALLBACK_RT.with(|rt| rt.block_on(future))
    }
}

// Thread-local transaction state for managing database transactions.
thread_local! {
    static CURRENT_TX: RefCell<Option<TransactionState>> = const { RefCell::new(None) };
}

// Thread-local mock storage for testing.
thread_local! {
    static QUERY_MOCKS: RefCell<HashMap<String, Vec<serde_json::Value>>> = RefCell::new(HashMap::new());
}

/// Register a mock response for a specific query.
pub fn register_query_mock(query: String, results: Vec<serde_json::Value>) {
    QUERY_MOCKS.with(|mocks| {
        mocks.borrow_mut().insert(query, results);
    });
}

/// Clear all registered query mocks.
pub fn clear_query_mocks() {
    QUERY_MOCKS.with(|mocks| mocks.borrow_mut().clear());
}

/// Get mock response for a query if one is registered.
pub fn get_mock_for_query(query: &str) -> Option<Vec<serde_json::Value>> {
    QUERY_MOCKS.with(|mocks| mocks.borrow().get(query).cloned())
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

/// True when a transaction is open on this thread.
pub fn has_active_tx() -> bool {
    CURRENT_TX.with(|tx| tx.borrow().is_some())
}

/// Forcibly drop the thread-local transaction state without touching the
/// server. `commit_transaction`/`rollback_transaction` already clear it on
/// success; this is the defensive backstop the block-form runner calls after a
/// commit/rollback that itself errored, so a half-finished transaction can
/// never leak into the next request handled by a reused worker thread.
pub fn clear_current_tx() {
    CURRENT_TX.with(|tx| {
        tx.borrow_mut().take();
    });
}

/// Begin a new transaction.
pub fn begin_transaction(isolation_level: Option<&str>) -> Result<String, String> {
    let host = super::core::DB_CONFIG.host.clone();
    let database = get_database_name().to_string();
    // SEC-027: use the configured scheme; was forcing http:// regardless.
    let url = db_url(&format!("/_api/database/{}/transaction/begin", database));

    let body = serde_json::json!({
        "database": database,
        "isolationLevel": isolation_level.unwrap_or("read_committed")
    });

    run_db_future(async move {
        let client = get_http_client().clone();
        let body_str = body.to_string();
        let response = send_with_db_auth_retry(|| {
            client
                .request(reqwest::Method::POST, &url)
                .header("Content-Type", "application/json")
                .body(body_str.clone())
        })
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;
        let status = response.status();
        if !status.is_success() {
            let body = crate::interpreter::builtins::http_class::read_capped_text_async(response)
                .await
                .unwrap_or_default();
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

    let database = get_database_name().to_string();
    // SEC-027: use the configured scheme.
    let url = db_url(&format!(
        "/_api/database/{}/transaction/{}/commit",
        database, tx_id
    ));

    run_db_future(async move {
        let client = get_http_client().clone();
        let response = send_with_db_auth_retry(|| client.request(reqwest::Method::POST, &url))
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;
        let status = response.status();
        if !status.is_success() {
            let body = crate::interpreter::builtins::http_class::read_capped_text_async(response)
                .await
                .unwrap_or_default();
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

    let database = get_database_name().to_string();
    // SEC-027: use the configured scheme.
    let url = db_url(&format!(
        "/_api/database/{}/transaction/{}/rollback",
        database, tx_id
    ));

    run_db_future(async move {
        let client = get_http_client().clone();
        let response = send_with_db_auth_retry(|| client.request(reqwest::Method::POST, &url))
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;
        let status = response.status();
        if !status.is_success() {
            let body = crate::interpreter::builtins::http_class::read_capped_text_async(response)
                .await
                .unwrap_or_default();
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
        Err(e) => Value::String(format!("Error: {}", e).into()),
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

/// Extract class name from _id field: "default:organisations/UUID" → "Organisation"
fn class_name_from_id(id: &str) -> String {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() >= 2 {
        let collection = parts[0].split(':').nth(1).unwrap_or(parts[0]);
        super::relations::classify(collection)
    } else {
        "Instance".to_string()
    }
}

/// Convert a JSON document to a class instance with all fields set.
/// For included relations, the correct class is derived from the _id field.
pub fn json_doc_to_instance(class: &Rc<Class>, json: &serde_json::Value) -> Value {
    let target_class = resolve_instance_class(class, json);
    let mut instance = Instance::new(target_class.clone());
    if let serde_json::Value::Object(map) = json {
        for (k, v) in map {
            instance.set(k.clone(), json_to_value(v));
        }
    }
    Value::Instance(Rc::new(RefCell::new(instance)))
}

/// Resolve the correct class for a JSON document based on its _id field.
/// Falls back to the given class if no _id or can't determine class.
fn resolve_instance_class(default_class: &Rc<Class>, json: &serde_json::Value) -> Rc<Class> {
    if let serde_json::Value::Object(map) = json {
        if let Some(serde_json::Value::String(id)) = map.get("_id") {
            let class_name = class_name_from_id(id);
            if let Some(class) = super::registry::get_model_class(&class_name) {
                return class;
            }
        }
    }
    default_class.clone()
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
        Err(e) => Value::String(format!("Error: {}", e).into()),
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
        Err(e) => Value::String(format!("Error: {}", e).into()),
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

    // Capture inputs for the dev-mode query log before the future moves them.
    let log_enabled = super::query_log::is_enabled();
    let log_query = if log_enabled {
        Some(sdbql.clone())
    } else {
        None
    };
    let log_binds = if log_enabled { bind_vars.clone() } else { None };
    let started = if log_enabled {
        Some(std::time::Instant::now())
    } else {
        None
    };

    let future = async move {
        let mut payload = serde_json::json!({ "query": sdbql });
        if let Some(bv) = bind_vars {
            payload["bindVars"] = serde_json::json!(bv);
        }
        let body_str = payload.to_string();

        let resp = send_with_db_auth_retry(|| {
            client
                .post(&url)
                .header("Content-Type", "application/json")
                .body(body_str.clone())
        })
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = crate::interpreter::builtins::http_class::read_capped_text_async(resp)
                .await
                .unwrap_or_default();
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

    let result = run_db_future(future);

    let db_duration = if let (Some(q), Some(t0)) = (log_query, started) {
        let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
        let dur_us = (elapsed * 1000.0).max(0.0) as u64;
        let span_name: String = q.chars().take(80).collect();
        crate::serve::span_log::record(
            &span_name,
            crate::serve::span_log::SpanKind::Db,
            t0,
            dur_us,
            None,
        );
        super::query_log::record(q, log_binds, elapsed);
        std::time::Duration::from_millis(elapsed as u64)
    } else {
        std::time::Duration::ZERO
    };

    // Always feed the coarse production Prometheus counter (Phase A).
    // The rich per-query log stays gated to --dev.
    crate::metrics::Metrics::global().record_db_queries(db_duration);

    result
}

/// Simple async query without bind variables - convenience wrapper.
pub fn exec_async_query(sdbql: String) -> Value {
    match exec_async_query_with_binds(sdbql, None) {
        Ok(results) => {
            // Direct conversion without intermediate Array wrapper
            let values: Vec<Value> = results.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        Err(e) => Value::String(format!("Error: {}", e).into()),
    }
}

/// Async query returning raw JSON string (no Value conversion - fastest).
/// Uses same HTTP client as HTTP.request for consistency.
pub fn exec_async_query_raw(sdbql: String) -> Value {
    // Get cached values (initialized on first use after .env is loaded)
    let url = get_cursor_url();
    // SEC-036: build the JSON body with serde_json so the SDBQL value is
    // escaped correctly. The previous `format!` used a `r#"\"#` replacement
    // (a single backslash), which produced malformed JSON for any quoted
    // SDBQL and let a `"` in the input inject sibling fields into the body.
    let body = match serde_json::to_string(&serde_json::json!({ "query": sdbql })) {
        Ok(b) => b,
        Err(e) => return Value::String(format!("Error: {}", e).into()),
    };

    let log_enabled = super::query_log::is_enabled();
    let log_query = if log_enabled {
        Some(sdbql.clone())
    } else {
        None
    };
    let started = if log_enabled {
        Some(std::time::Instant::now())
    } else {
        None
    };

    let client = get_http_client().clone();
    let result = match run_db_future(async move {
        let resp = send_with_db_auth_retry(|| {
            client
                .post(&url)
                .header("Content-Type", "application/json")
                .body(body.clone())
        })
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = crate::interpreter::builtins::http_class::read_capped_text_async(resp)
                .await
                .unwrap_or_default();
            return Err(format!("Query failed: {} - {}", status, body));
        }

        crate::interpreter::builtins::http_class::read_capped_text_async(resp)
            .await
            .map_err(|e| format!("Read error: {}", e))
    }) {
        Ok(text) => Value::String(text.into()),
        Err(e) => Value::String(format!("Error: {}", e).into()),
    };

    let db_duration = if let (Some(q), Some(t0)) = (log_query, started) {
        let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
        let dur_us = (elapsed * 1000.0).max(0.0) as u64;
        let span_name: String = q.chars().take(80).collect();
        crate::serve::span_log::record(
            &span_name,
            crate::serve::span_log::SpanKind::Db,
            t0,
            dur_us,
            None,
        );
        super::query_log::record(q, None, elapsed);
        std::time::Duration::from_millis(elapsed as u64)
    } else {
        std::time::Duration::ZERO
    };

    // Always feed the coarse production Prometheus counter (Phase A).
    // The rich per-query log stays gated to --dev.
    crate::metrics::Metrics::global().record_db_queries(db_duration);

    result
}

/// Hardcoded query - exact same pattern as HTTP.request for comparison.
pub fn exec_query_hardcoded(sdbql: String) -> Value {
    // Use environment variables for credentials
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
    let api_key = std::env::var("SOLIDB_API_KEY").unwrap_or_else(|_| {
        eprintln!("WARNING: SOLIDB_API_KEY not set, database queries will fail");
        String::new()
    });
    // SEC-036: same hand-rolled-JSON anti-pattern as exec_async_query_raw —
    // build the body with serde_json so backslashes, control chars, and
    // unicode in the SDBQL are escaped correctly.
    let body = match serde_json::to_string(&serde_json::json!({ "query": sdbql })) {
        Ok(b) => b,
        Err(e) => return Value::String(format!("Error: {}", e).into()),
    };

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
        crate::interpreter::builtins::http_class::read_capped_text_async(resp)
            .await
            .map_err(|e| e.to_string())
    }) {
        Ok(text) => Value::String(text.into()),
        Err(e) => Value::String(format!("Error: {}", e).into()),
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

fn is_database_not_found_error(error: &str) -> bool {
    let error_lower = error.to_lowercase();
    error_lower.contains("database")
        && (error_lower.contains("not found")
            || error_lower.contains("not exist")
            || error_lower.contains("does not exist")
            || error_lower.contains("unknown database"))
}

/// True when an operation failed because the target collection *or* its parent
/// database does not exist yet. Both are auto-created on first use, so the
/// document/query call sites retry through `create_collection_sync` (which now
/// also creates the database) on either condition.
fn is_missing_collection_or_database_error(error: &str) -> bool {
    is_collection_not_found_error(error) || is_database_not_found_error(error)
}

/// Create the configured database. Mirrors `create_collection_sync`: a 409
/// (already exists) is treated as success so concurrent first-callers race
/// cleanly, and the database is created on first use the same way collections
/// are created on demand.
fn create_database_sync(name: &str) -> Result<(), String> {
    let raw = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
    // SEC-027: preserve the operator-set scheme (or pick https for
    // remote, http for loopback) instead of forcing http://.
    let (scheme, host) = super::db_config::parse_solidb_host(&raw);
    // Note: `/_api/databases` (plural) is GET-only (list); creation is a POST
    // to the singular `/_api/database`.
    let url = format!("{}{}/_api/database", scheme, host);

    let body = serde_json::json!({ "name": name }).to_string();

    let client = get_http_client().clone();

    run_db_future(async move {
        let resp = send_with_db_auth_retry(|| {
            client
                .post(&url)
                .header("Content-Type", "application/json")
                .body(body.clone())
        })
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            // 409 = database already exists — exactly the state we want.
            if status == reqwest::StatusCode::CONFLICT {
                return Ok(());
            }
            let body = crate::interpreter::builtins::http_class::read_capped_text_async(resp)
                .await
                .unwrap_or_default();
            return Err(format!("Create database failed: {} - {}", status, body));
        }

        Ok(())
    })
}

/// Create a collection, auto-creating the parent database first if it does not
/// exist yet. The collection endpoint returns 404 (or a database-not-found
/// message) when the database path is missing; in that case we create the
/// database and retry the collection creation once.
fn create_collection_sync(name: &str) -> Result<(), String> {
    match try_create_collection_once(name) {
        Err(e) if is_database_not_found_error(&e) => {
            create_database_sync(&get_database_name())?;
            try_create_collection_once(name)
        }
        other => other,
    }
}

fn try_create_collection_once(name: &str) -> Result<(), String> {
    let raw = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
    // SEC-027: preserve the operator-set scheme (or pick https for
    // remote, http for loopback) instead of forcing http://.
    let (scheme, host) = super::db_config::parse_solidb_host(&raw);
    let database = get_database_name();
    let url = format!("{}{}/_api/database/{}/collection", scheme, host, database);

    let body = serde_json::json!({ "name": name }).to_string();

    let client = get_http_client().clone();

    run_db_future(async move {
        let resp = send_with_db_auth_retry(|| {
            client
                .post(&url)
                .header("Content-Type", "application/json")
                .body(body.clone())
        })
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            // 409 = collection already exists — that's exactly the state the
            // auto-create path wants to reach. Treat as success so the caller
            // retries the original query against the now-known-to-exist
            // collection instead of bubbling a fake failure.
            if status == reqwest::StatusCode::CONFLICT {
                return Ok(());
            }
            let body = crate::interpreter::builtins::http_class::read_capped_text_async(resp)
                .await
                .unwrap_or_default();
            // A 404 on the collection endpoint means the parent database path
            // is missing. Surface it as a database-not-found so the caller
            // creates the database first and retries.
            if status == reqwest::StatusCode::NOT_FOUND || is_database_not_found_error(&body) {
                return Err(format!("Database not found: {} - {}", status, body));
            }
            return Err(format!("Create collection failed: {} - {}", status, body));
        }

        Ok(())
    })
}

/// Execute query with automatic collection creation on collection-related error.
/// This is used for Model operations to auto-create collections when they don't exist.
/// Returns mock data if registered for the query.
pub fn exec_with_auto_collection(
    sdbql: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
    collection_name: &str,
) -> Result<Vec<serde_json::Value>, String> {
    // Check for mock response first
    if let Some(mock_results) = get_mock_for_query(&sdbql) {
        return Ok(mock_results);
    }

    let result = exec_async_query_with_binds(sdbql.clone(), bind_vars.clone());

    if let Err(ref e) = result {
        if is_missing_collection_or_database_error(e) {
            if let Err(create_err) = create_collection_sync(collection_name) {
                return Err(format!(
                    "Collection '{}' (or its database) not found, and failed to create it: {}",
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
        Err(e) => Value::String(format!("Error: {}", e).into()),
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
        Err(e) => Value::String(format!("Error: {}", e).into()),
    }
}

/// Build the base URL for document operations: {scheme}{host}/_api/database/{db}/document/{collection}.
/// SEC-027: use the configured scheme; was forcing http:// regardless.
fn document_base_url(collection: &str) -> String {
    db_url(&format!(
        "/_api/database/{}/document/{}",
        get_database_name(),
        collection
    ))
}

/// Execute a single SDBQL query in transactional context.
///
/// SEC-035: previously wrapped the SDBQL in a server-side JavaScript function
/// (`function() { return AQL_QUERY('...', {}); }`) and POSTed it to
/// `/_api/transaction`. Single-quote escaping was insufficient — backslashes,
/// newlines, comments, and template literals could all break out of the JS
/// string literal and execute attacker-controlled JS.
///
/// We now route through the cursor endpoint with `{query, bindVars}`, so the
/// SDBQL string is sent as a query parameter and is never interpolated into
/// JavaScript source. A single AQL query on the cursor endpoint is atomic
/// (server-side single-statement transaction), preserving the previous
/// transactional semantics for the one-shot string form.
pub fn exec_transaction_sdbql(sdbql: &str) -> Result<serde_json::Value, String> {
    let results = exec_async_query_with_binds(sdbql.to_string(), None)?;
    Ok(serde_json::Value::Array(results))
}

/// Execute a direct HTTP document operation using the shared runtime and client.
fn exec_document_request(
    method: reqwest::Method,
    url: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    exec_document_request_with_headers(method, url, body, &[])
}

/// Same as `exec_document_request` but allows passing extra request headers
/// (used for `If-Match` on conditional PUTs).
fn exec_document_request_with_headers(
    method: reqwest::Method,
    url: String,
    body: Option<serde_json::Value>,
    extra_headers: &[(&'static str, String)],
) -> Result<serde_json::Value, String> {
    let client = get_http_client().clone();
    let extra: Vec<(&'static str, String)> =
        extra_headers.iter().map(|(k, v)| (*k, v.clone())).collect();

    let body_str = body.as_ref().map(|b| b.to_string());

    run_db_future(async move {
        let resp = send_with_db_auth_retry(|| {
            let mut r = client.request(method.clone(), &url);
            if let Some(ref bs) = body_str {
                r = r
                    .header("Content-Type", "application/json")
                    .body(bs.clone());
            }
            for (k, v) in &extra {
                r = r.header(*k, v.as_str());
            }
            r
        })
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let body = crate::interpreter::builtins::http_class::read_capped_text_async(resp)
                .await
                .unwrap_or_default();
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
    // When a transaction is open on this thread, route the write through the
    // transaction endpoint so it participates in (and rolls back with) the tx.
    // `exec_insert_tx` delegates straight back here when no tx is active, so
    // this never recurses.
    if get_current_tx_id().is_some() {
        return exec_insert_tx(collection, key, document);
    }
    if let Some(k) = key {
        if let Some(obj) = document.as_object_mut() {
            obj.insert("_key".to_string(), serde_json::json!(k));
        }
    }
    let url = document_base_url(collection);
    let result = exec_document_request(reqwest::Method::POST, url.clone(), Some(document.clone()));

    if let Err(ref e) = result {
        if is_missing_collection_or_database_error(e) {
            create_collection_sync(collection)?;
            return exec_document_request(reqwest::Method::POST, url, Some(document));
        }
    }
    result
}

/// Execute a get with automatic collection creation.
pub fn exec_get(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    // Read through the active transaction so it sees uncommitted writes.
    if get_current_tx_id().is_some() {
        return exec_get_tx(collection, key);
    }
    let url = format!("{}/{}", document_base_url(collection), normalize_key(key));
    let result = exec_document_request(reqwest::Method::GET, url.clone(), None);

    if let Err(ref e) = result {
        if is_missing_collection_or_database_error(e) {
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
    // Route the update through the active transaction when one is open.
    if get_current_tx_id().is_some() {
        return exec_update_tx(collection, key, document);
    }
    let url = format!("{}/{}", document_base_url(collection), normalize_key(key));
    let result = exec_document_request(reqwest::Method::PUT, url.clone(), Some(document.clone()));

    if let Err(ref e) = result {
        if is_missing_collection_or_database_error(e) {
            create_collection_sync(collection)?;
            return exec_document_request(reqwest::Method::PUT, url, Some(document));
        }
    }
    result
}

/// Conditional PUT with an `If-Match: <expected_rev>` header. SoliDB returns
/// HTTP 409 with a "has been modified" message when the revision no longer
/// matches; callers use that to drive a CAS retry loop.
pub fn exec_update_if_match(
    collection: &str,
    key: &str,
    document: serde_json::Value,
    expected_rev: &str,
) -> Result<serde_json::Value, String> {
    let url = format!("{}/{}", document_base_url(collection), normalize_key(key));
    let headers = [("If-Match", expected_rev.to_string())];
    exec_document_request_with_headers(reqwest::Method::PUT, url, Some(document), &headers)
}

/// True if `err` is the rev-mismatch surface of `exec_update_if_match`.
pub fn is_rev_mismatch_error(err: &str) -> bool {
    err.contains("HTTP 409") && err.contains("has been modified")
}

/// Atomically apply `delta` to a numeric `field` on document `key` in
/// `collection`, using a fetch + If-Match CAS retry loop. Returns the new
/// integer value and the resulting `_rev` so the caller can refresh the
/// in-memory instance.
pub fn cas_field_delta(
    collection: &str,
    key: &str,
    field: &str,
    delta: i64,
) -> Result<(i64, String), String> {
    const MAX_ATTEMPTS: usize = 10;

    for _ in 0..MAX_ATTEMPTS {
        let current_doc = exec_get(collection, key)?;
        let rev = current_doc
            .get("_rev")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("Document '{}/{}' has no _rev field", collection, key))?
            .to_string();
        let current_value = current_doc
            .get(field)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .unwrap_or(0);
        let new_value = current_value + delta;

        let mut body = serde_json::Map::new();
        body.insert(
            field.to_string(),
            serde_json::Value::Number(serde_json::Number::from(new_value)),
        );

        match exec_update_if_match(collection, key, serde_json::Value::Object(body), &rev) {
            Ok(resp) => {
                let new_rev = resp
                    .get("_rev")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&rev)
                    .to_string();
                return Ok((new_value, new_rev));
            }
            Err(e) if is_rev_mismatch_error(&e) => continue,
            Err(e) => return Err(e),
        }
    }

    Err(format!(
        "Atomic update of {}.{} failed after {} attempts (too much contention)",
        collection, field, MAX_ATTEMPTS
    ))
}

/// Execute a delete with automatic collection creation.
pub fn exec_delete(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    // Route the delete through the active transaction when one is open.
    if get_current_tx_id().is_some() {
        return exec_delete_tx(collection, key);
    }
    let url = format!("{}/{}", document_base_url(collection), normalize_key(key));
    let result = exec_document_request(reqwest::Method::DELETE, url.clone(), None);

    if let Err(ref e) = result {
        if is_missing_collection_or_database_error(e) {
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
        let database = get_database_name().to_string();
        // SEC-027: use the configured scheme.
        let url = db_url(&format!(
            "/_api/database/{}/transaction/{}/document/{}",
            database, tx_id, collection
        ));
        exec_document_request(reqwest::Method::POST, url, Some(document))
    } else {
        exec_insert(collection, key, document)
    }
}

/// Transaction-aware document get.
/// If a transaction is active, uses the transaction endpoint.
pub fn exec_get_tx(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    if let Some(tx_id) = get_current_tx_id() {
        let database = get_database_name().to_string();
        // SEC-027: use the configured scheme.
        let url = db_url(&format!(
            "/_api/database/{}/transaction/{}/document/{}/{}",
            database,
            tx_id,
            collection,
            normalize_key(key)
        ));
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
        let database = get_database_name().to_string();
        // SEC-027: use the configured scheme.
        let url = db_url(&format!(
            "/_api/database/{}/transaction/{}/document/{}/{}",
            database,
            tx_id,
            collection,
            normalize_key(key)
        ));
        exec_document_request(reqwest::Method::PUT, url, Some(document))
    } else {
        exec_update(collection, key, document, false)
    }
}

/// Transaction-aware document delete.
/// If a transaction is active, uses the transaction endpoint.
pub fn exec_delete_tx(collection: &str, key: &str) -> Result<serde_json::Value, String> {
    if let Some(tx_id) = get_current_tx_id() {
        let database = get_database_name().to_string();
        // SEC-027: use the configured scheme.
        let url = db_url(&format!(
            "/_api/database/{}/transaction/{}/document/{}/{}",
            database,
            tx_id,
            collection,
            normalize_key(key)
        ));
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
                assert_eq!(inst_ref.get("_key"), Some(Value::String("abc-xyz".into())));
                assert_eq!(inst_ref.get("name"), Some(Value::String("Alice".into())));
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

    #[test]
    fn test_class_name_from_id() {
        assert_eq!(
            class_name_from_id("default:organisations/abc123"),
            "Organisation"
        );
        assert_eq!(class_name_from_id("default:users/xyz789"), "User");
        assert_eq!(class_name_from_id("default:blog_posts/def456"), "BlogPost");
        assert_eq!(class_name_from_id("no_slash"), "Instance");
    }

    #[test]
    fn test_json_doc_to_instance_with_id_derives_correct_class() {
        clear_model_classes();

        let contact_class = Rc::new(Class {
            name: "Contact".to_string(),
            ..Default::default()
        });
        let organisation_class = Rc::new(Class {
            name: "Organisation".to_string(),
            ..Default::default()
        });

        register_model_class("Contact", contact_class.clone());
        register_model_class("Organisation", organisation_class.clone());

        let json = serde_json::json!({
            "_id": "default:organisations/019da674-194a-713d-94c7-47228ed73f90",
            "_key": "019da674-194a-713d-94c7-47228ed73f90",
            "name": "solisoft",
            "email": "contact@solisoft.net"
        });

        let val = json_doc_to_instance(&contact_class, &json);
        match val {
            Value::Instance(inst) => {
                assert_eq!(inst.borrow().class.name, "Organisation");
                assert_eq!(
                    inst.borrow().get("name"),
                    Some(Value::String("solisoft".into()))
                );
                assert_eq!(
                    inst.borrow().get("email"),
                    Some(Value::String("contact@solisoft.net".into()))
                );
            }
            _ => panic!("Expected Value::Instance, got {:?}", val),
        }

        clear_model_classes();
    }

    #[test]
    fn test_json_doc_to_instance_falls_back_when_class_not_found() {
        clear_model_classes();

        let contact_class = Rc::new(Class {
            name: "Contact".to_string(),
            ..Default::default()
        });

        let json = serde_json::json!({
            "_id": "default:organisations/some-uuid",
            "_key": "some-uuid",
            "name": "solisoft"
        });

        let val = json_doc_to_instance(&contact_class, &json);
        match val {
            Value::Instance(inst) => {
                assert_eq!(inst.borrow().class.name, "Contact");
            }
            _ => panic!("Expected Value::Instance"),
        }

        clear_model_classes();
    }
}
