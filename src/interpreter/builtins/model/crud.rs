//! Database CRUD operations and JSON conversion utilities.

use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::builtins::http_class::get_http_client;
use crate::interpreter::value::{HashKey, Value};
use crate::serve::get_tokio_handle;

use super::core::{get_api_key, get_cursor_url};

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
    let api_key = get_api_key();

    let rt = get_tokio_handle().ok_or("No tokio runtime available")?;
    let client = get_http_client().clone();

    let future = async move {
        let mut payload = serde_json::json!({ "query": sdbql });
        if let Some(bv) = bind_vars {
            payload["bindVars"] = serde_json::json!(bv);
        }

        let mut request = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(payload.to_string());

        if let Some(key) = api_key {
            request = request.header("X-API-Key", key);
        }

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

    rt.block_on(future)
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
    let api_key = get_api_key();
    let body = format!(r#"{{"query":"{}"}}"#, sdbql.replace('"', r#"\"#));

    let Some(rt) = get_tokio_handle() else {
        return Value::String("Error: No tokio runtime available".to_string());
    };

    let client = get_http_client().clone();
    match rt.block_on(async move {
        let mut request = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body);

        if let Some(key) = api_key {
            request = request.header("X-API-Key", key);
        }

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

    let Some(rt) = get_tokio_handle() else {
        return Value::String("Error: No tokio runtime available".to_string());
    };

    let client = get_http_client().clone();
    match rt.block_on(async move {
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
