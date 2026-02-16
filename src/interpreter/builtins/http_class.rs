//! HTTP built-in class for SoliLang.
//!
//! Provides the HTTP class with static methods for making HTTP requests:
//! - HTTP.get(url, options?) -> Future<String>
//! - HTTP.post(url, body, options?) -> Future<String>
//! - HTTP.put(url, body, options?) -> Future<String>
//! - HTTP.delete(url, options?) -> Future<String>
//! - HTTP.patch(url, body, options?) -> Future<String>
//! - HTTP.head(url, options?) -> Future<String>
//! - HTTP.get_json(url) -> Future<Value>
//! - HTTP.post_json(url, data) -> Future<Value>
//! - HTTP.put_json(url, data) -> Future<Value>
//! - HTTP.patch_json(url, data) -> Future<Value>
//! - HTTP.request(method, url, options?, body?) -> Future<HTTPResponse>
//! - HTTP.get_all(urls) -> Array<Future<String>>
//! - HTTP.parallel(requests) -> Array<Future<HTTPResponse>>
//!
//! Also provides helpers for working with HTTP responses.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;

use indexmap::IndexMap;
use reqwest::Client;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{
    hash_from_pairs, Class, FutureState, HashKey, HttpFutureKind, NativeFunction, Value,
};
use crate::serve::get_tokio_handle;

const BLOCKED_SCHEMES: &[&str] = &["javascript", "file", "ftp", "ssh", "telnet", "gopher"];

pub fn validate_url_for_ssrf(url: &str) -> Result<(), String> {
    let url = url.trim();

    if url.is_empty() {
        return Err("URL cannot be empty".to_string());
    }

    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_lowercase(), r),
        None => {
            return Err("URL must have a scheme (e.g., http:// or https://)".to_string());
        }
    };

    if scheme.is_empty() {
        return Err("URL scheme cannot be empty".to_string());
    }

    if BLOCKED_SCHEMES.contains(&scheme.as_str()) {
        return Err(format!(
            "URL scheme '{}:' is not allowed for security reasons",
            scheme
        ));
    }

    if scheme != "http" && scheme != "https" {
        return Err("Only HTTP and HTTPS URLs are allowed".to_string());
    }

    let host = if let Some((h, _)) = rest.split_once('/') {
        if let Some((_, h2)) = h.split_once('@') {
            h2
        } else {
            h
        }
    } else if let Some((_, h)) = rest.split_once('@') {
        h
    } else {
        rest
    };

    let host = if let Some((h, _)) = host.split_once(':') {
        h
    } else {
        host
    };

    if host.is_empty() {
        return Err("URL host cannot be empty".to_string());
    }

    if is_blocked_host(host) {
        return Err("Access to private/localhost addresses is not allowed".to_string());
    }

    Ok(())
}

fn is_blocked_host(_host: &str) -> bool {
    // Allow all hosts including localhost and private networks
    false
}

static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

/// Get the shared async HTTP client (used by HTTP class and Model queries)
pub fn get_http_client() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .pool_max_idle_per_host(32)
            .build()
            .expect("Failed to create HTTP client")
    })
}

#[allow(clippy::arc_with_non_send_sync)]
fn spawn_http_future<F>(f: F, kind: HttpFutureKind) -> Value
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });
    Value::Future(Arc::new(Mutex::new(FutureState::Pending {
        receiver: rx,
        kind,
    })))
}

fn value_to_json(value: &Value) -> Result<String, String> {
    let json = crate::interpreter::value::value_to_json(value)?;
    serde_json::to_string(&json).map_err(|e| format!("JSON serialization error: {}", e))
}

fn json_to_value(json: &serde_json::Value) -> Result<Value, String> {
    crate::interpreter::value::json_to_value(json)
}

pub fn register_http_class(env: &mut Environment) {
    let mut http_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    http_static_methods.insert(
        "get".to_string(),
        Rc::new(NativeFunction::new("HTTP.get", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.get() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::get(&url).call() {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "post".to_string(),
        Rc::new(NativeFunction::new("HTTP.post", Some(2), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.post() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let body = match &args[1] {
                Value::String(s) => s.clone(),
                Value::Hash(_) => value_to_json(&args[1])?,
                other => {
                    return Err(format!(
                        "HTTP.post() expects string or hash body, got {}",
                        other.type_name()
                    ))
                }
            };

            let content_type = if args[1].type_name() == "Hash" {
                "application/json".to_string()
            } else {
                "text/plain".to_string()
            };

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client
                            .post(&url)
                            .header("Content-Type", content_type)
                            .body(body)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::post(&url)
                        .set("Content-Type", &content_type)
                        .send_string(&body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "put".to_string(),
        Rc::new(NativeFunction::new("HTTP.put", Some(2), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.put() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let body = match &args[1] {
                Value::String(s) => s.clone(),
                Value::Hash(_) => value_to_json(&args[1])?,
                other => {
                    return Err(format!(
                        "HTTP.put() expects string or hash body, got {}",
                        other.type_name()
                    ))
                }
            };

            let content_type = if args[1].type_name() == "Hash" {
                "application/json".to_string()
            } else {
                "text/plain".to_string()
            };

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client
                            .put(&url)
                            .header("Content-Type", content_type)
                            .body(body)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::put(&url)
                        .set("Content-Type", &content_type)
                        .send_string(&body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "patch".to_string(),
        Rc::new(NativeFunction::new("HTTP.patch", Some(2), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.patch() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let body = match &args[1] {
                Value::String(s) => s.clone(),
                Value::Hash(_) => value_to_json(&args[1])?,
                other => {
                    return Err(format!(
                        "HTTP.patch() expects string or hash body, got {}",
                        other.type_name()
                    ))
                }
            };

            let content_type = if args[1].type_name() == "Hash" {
                "application/json".to_string()
            } else {
                "text/plain".to_string()
            };

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client
                            .patch(&url)
                            .header("Content-Type", content_type)
                            .body(body)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::patch(&url)
                        .set("Content-Type", &content_type)
                        .send_string(&body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("HTTP.delete", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.delete() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client
                            .delete(&url)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::delete(&url).call() {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "head".to_string(),
        Rc::new(NativeFunction::new("HTTP.head", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.head() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client.head(&url).send().await.map_err(|e| e.to_string())?;
                        let status = resp.status().as_u16();
                        Ok(format!(
                            "{} {}",
                            status,
                            resp.status().canonical_reason().unwrap_or("")
                        ))
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::head(&url).call() {
                        Ok(response) => {
                            let status = response.status();
                            Ok(format!("{} {}", status, response.status_text()))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "get_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_json", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.get_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client
                            .get(&url)
                            .header("Accept", "application/json")
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = resp.text().await.map_err(|e| e.to_string())?;
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json_to_value(&json),
                            Err(e) => Err(format!("Failed to parse JSON: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::get(&url).set("Accept", "application/json").call() {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "post_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.post_json", Some(2), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.post_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let json_body = value_to_json(&args[1])?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client
                            .post(&url)
                            .header("Content-Type", "application/json")
                            .body(json_body)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = resp.text().await.map_err(|e| e.to_string())?;
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json_to_value(&json),
                            Err(e) => Err(format!("Failed to parse JSON: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::post(&url)
                        .set("Content-Type", "application/json")
                        .send_string(&json_body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "put_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.put_json", Some(2), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.put_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let json_body = value_to_json(&args[1])?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client
                            .put(&url)
                            .header("Content-Type", "application/json")
                            .body(json_body)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = resp.text().await.map_err(|e| e.to_string())?;
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json_to_value(&json),
                            Err(e) => Err(format!("Failed to parse JSON: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::put(&url)
                        .set("Content-Type", "application/json")
                        .send_string(&json_body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "patch_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.patch_json", Some(2), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.patch_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let json_body = value_to_json(&args[1])?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let resp = client
                            .patch(&url)
                            .header("Content-Type", "application/json")
                            .body(json_body)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = resp.text().await.map_err(|e| e.to_string())?;
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json_to_value(&json),
                            Err(e) => Err(format!("Failed to parse JSON: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq::patch(&url)
                        .set("Content-Type", "application/json")
                        .send_string(&json_body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "request".to_string(),
        Rc::new(NativeFunction::new("HTTP.request", None, |args| {
            if args.len() < 2 {
                return Err("HTTP.request() requires at least method and URL".to_string());
            }

            let method = match &args[0] {
                Value::String(s) => s.to_uppercase(),
                other => {
                    return Err(format!(
                        "HTTP.request() method must be string, got {}",
                        other.type_name()
                    ))
                }
            };

            let url = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.request() URL must be string, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let mut headers_vec: Vec<(String, String)> = Vec::new();
            if args.len() > 2 {
                if let Value::Hash(headers) = &args[2] {
                    for (key, value) in headers.borrow().iter() {
                        let key_str = match key {
                            HashKey::String(s) => s.clone(),
                            _ => continue,
                        };
                        let value_str = match value {
                            Value::String(s) => s.clone(),
                            _ => format!("{}", value),
                        };
                        headers_vec.push((key_str, value_str));
                    }
                }
            }

            let body_opt: Option<String> = if args.len() > 3 {
                Some(match &args[3] {
                    Value::String(s) => s.clone(),
                    Value::Hash(_) => value_to_json(&args[3])?,
                    Value::Null => String::new(),
                    other => format!("{}", other),
                })
            } else {
                None
            };

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    let method_clone = method.clone();
                    let body_opt_clone = body_opt.clone();
                    let headers_vec_clone = headers_vec.clone();
                    match rt.block_on(async move {
                        let mut request = match method_clone.as_str() {
                            "GET" => client.get(&url),
                            "POST" => client.post(&url),
                            "PUT" => client.put(&url),
                            "DELETE" => client.delete(&url),
                            "PATCH" => client.patch(&url),
                            "HEAD" => client.head(&url),
                            _ => return Err(format!("Unsupported HTTP method: {}", method_clone)),
                        };

                        for (key, value) in &headers_vec_clone {
                            request = request.header(key.as_str(), value.as_str());
                        }

                        if let Some(body) = body_opt_clone {
                            request = request.body(body);
                        }

                        let resp = request.send().await.map_err(|e| e.to_string())?;

                        let status = resp.status().as_u16();
                        let status_text =
                            resp.status().canonical_reason().unwrap_or("").to_string();

                        let mut headers_map = serde_json::Map::new();
                        for (name, value) in resp.headers().iter() {
                            if let Ok(v) = value.to_str() {
                                headers_map.insert(
                                    name.to_string(),
                                    serde_json::Value::String(v.to_string()),
                                );
                            }
                        }

                        let body = resp.text().await.map_err(|e| e.to_string())?;

                        create_http_response(status, status_text, headers_map, body)
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => {
                    let method_clone = method.clone();
                    let body_opt_clone = body_opt.clone();
                    let headers_vec_clone = headers_vec.clone();
                    Ok(spawn_http_future(
                        move || {
                            let mut request = match method_clone.as_str() {
                                "GET" => ureq::get(&url),
                                "POST" => ureq::post(&url),
                                "PUT" => ureq::put(&url),
                                "DELETE" => ureq::delete(&url),
                                "PATCH" => ureq::patch(&url),
                                "HEAD" => ureq::head(&url),
                                _ => {
                                    return Err(format!(
                                        "Unsupported HTTP method: {}",
                                        method_clone
                                    ))
                                }
                            };

                            for (key, value) in &headers_vec_clone {
                                request = request.set(key, value);
                            }

                            let response = if let Some(body) = body_opt_clone {
                                request.send_string(&body)
                            } else {
                                request.call()
                            };

                            match response {
                                Ok(resp) => {
                                    let status = resp.status();
                                    let status_text = resp.status_text().to_string();

                                    let mut headers_map = serde_json::Map::new();
                                    for name in resp.headers_names() {
                                        if let Some(value) = resp.header(&name) {
                                            headers_map.insert(
                                                name,
                                                serde_json::Value::String(value.to_string()),
                                            );
                                        }
                                    }

                                    let body = resp.into_string().map_err(|e| {
                                        format!("Failed to read response body: {}", e)
                                    })?;

                                    let result = serde_json::json!({
                                        "status": status,
                                        "status_text": status_text,
                                        "headers": headers_map,
                                        "body": body
                                    });

                                    Ok(result.to_string())
                                }
                                Err(ureq::Error::Status(code, resp)) => {
                                    let status_text = resp.status_text().to_string();
                                    let body = resp.into_string().unwrap_or_default();

                                    let result = serde_json::json!({
                                        "status": code,
                                        "status_text": status_text,
                                        "headers": {},
                                        "body": body
                                    });

                                    Ok(result.to_string())
                                }
                                Err(e) => Err(format!("HTTP request failed: {}", e)),
                            }
                        },
                        HttpFutureKind::FullResponse,
                    ))
                }
            }
        })),
    );

    http_static_methods.insert(
        "json_parse".to_string(),
        Rc::new(NativeFunction::new("HTTP.json_parse", Some(1), |args| {
            let json_str = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.json_parse() expects string, got {}",
                        other.type_name()
                    ))
                }
            };

            match serde_json::from_str::<serde_json::Value>(&json_str) {
                Ok(json) => json_to_value(&json),
                Err(e) => Err(format!("Failed to parse JSON: {}", e)),
            }
        })),
    );

    http_static_methods.insert(
        "json_stringify".to_string(),
        Rc::new(NativeFunction::new(
            "HTTP.json_stringify",
            Some(1),
            |args| value_to_json(&args[0]).map(Value::String),
        )),
    );

    http_static_methods.insert(
        "get_all".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_all", Some(1), |args| {
            let urls = match &args[0] {
                Value::Array(arr) => {
                    let mut url_strings = Vec::new();
                    for item in arr.borrow().iter() {
                        match item {
                            Value::String(s) => url_strings.push(s.clone()),
                            other => {
                                return Err(format!(
                                    "HTTP.get_all() expects array of strings, got {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    url_strings
                }
                other => {
                    return Err(format!(
                        "HTTP.get_all() expects array of URLs, got {}",
                        other.type_name()
                    ))
                }
            };

            let results = run_parallel_gets(urls);

            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(body) => Value::String(body),
                    Err(e) => hash_from_pairs([("error".to_string(), Value::String(e))]),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    http_static_methods.insert(
        "parallel".to_string(),
        Rc::new(NativeFunction::new("HTTP.parallel", Some(1), |args| {
            let requests = match &args[0] {
                Value::Array(arr) => {
                    let mut req_configs = Vec::new();
                    for item in arr.borrow().iter() {
                        let config = parse_request_config(item)?;
                        req_configs.push(config);
                    }
                    req_configs
                }
                other => {
                    return Err(format!(
                        "HTTP.parallel() expects array of request configs, got {}",
                        other.type_name()
                    ))
                }
            };

            let results = run_parallel_requests(requests);

            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(response) => response_to_value(response),
                    Err(e) => hash_from_pairs([("error".to_string(), Value::String(e))]),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    let http_class = Class {
        name: "HTTP".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: http_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("HTTP".to_string(), Value::Class(Rc::new(http_class)));
}

fn create_http_response(
    status: u16,
    status_text: String,
    headers_map: serde_json::Map<String, serde_json::Value>,
    body: String,
) -> Result<Value, String> {
    let response_headers: IndexMap<HashKey, Value> = headers_map
        .into_iter()
        .map(|(k, v)| {
            (
                HashKey::String(k),
                Value::String(v.as_str().unwrap_or("").to_string()),
            )
        })
        .collect();

    let mut result: IndexMap<HashKey, Value> = IndexMap::new();
    result.insert(
        HashKey::String("status".to_string()),
        Value::Int(status as i64),
    );
    result.insert(
        HashKey::String("status_text".to_string()),
        Value::String(status_text),
    );
    result.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(response_headers))),
    );
    result.insert(HashKey::String("body".to_string()), Value::String(body));

    Ok(Value::Hash(Rc::new(RefCell::new(result))))
}

#[derive(Clone)]
struct RequestConfig {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
}

struct HttpResponse {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: String,
}

fn parse_request_config(value: &Value) -> Result<RequestConfig, String> {
    match value {
        Value::String(url) => Ok(RequestConfig {
            method: "GET".to_string(),
            url: url.clone(),
            headers: vec![],
            body: None,
        }),
        Value::Hash(hash) => {
            let hash = hash.borrow();
            let mut url = None;
            let mut method = "GET".to_string();
            let mut headers = vec![];
            let mut body = None;

            for (k, v) in hash.iter() {
                if let HashKey::String(key) = k {
                    match key.as_str() {
                        "url" => {
                            if let Value::String(s) = v {
                                url = Some(s.clone());
                            }
                        }
                        "method" => {
                            if let Value::String(s) = v {
                                method = s.to_uppercase();
                            }
                        }
                        "headers" => {
                            if let Value::Hash(h) = v {
                                for (hk, hv) in h.borrow().iter() {
                                    if let (HashKey::String(k), Value::String(v)) = (hk, hv) {
                                        headers.push((k.clone(), v.clone()));
                                    }
                                }
                            }
                        }
                        "body" => match v {
                            Value::String(s) => body = Some(s.clone()),
                            Value::Hash(_) => body = Some(value_to_json(v)?),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }

            let url = url.ok_or("Request config must have 'url' field")?;
            Ok(RequestConfig {
                method,
                url,
                headers,
                body,
            })
        }
        other => Err(format!(
            "Request config must be string URL or hash, got {}",
            other.type_name()
        )),
    }
}

fn run_parallel_gets(urls: Vec<String>) -> Vec<Result<String, String>> {
    let handles: Vec<_> = urls
        .into_iter()
        .map(|url| {
            thread::spawn(move || match ureq::get(&url).call() {
                Ok(response) => response
                    .into_string()
                    .map_err(|e| format!("Failed to read response: {}", e)),
                Err(ureq::Error::Status(code, response)) => {
                    let body = response.into_string().unwrap_or_default();
                    Err(format!("HTTP {} error: {}", code, body))
                }
                Err(e) => Err(format!("Request failed: {}", e)),
            })
        })
        .collect();

    handles
        .into_iter()
        .map(|h| {
            h.join()
                .unwrap_or_else(|_| Err("Thread panicked".to_string()))
        })
        .collect()
}

fn run_parallel_requests(requests: Vec<RequestConfig>) -> Vec<Result<HttpResponse, String>> {
    let handles: Vec<_> = requests
        .into_iter()
        .map(|config| thread::spawn(move || execute_request(config)))
        .collect();

    handles
        .into_iter()
        .map(|h| {
            h.join()
                .unwrap_or_else(|_| Err("Thread panicked".to_string()))
        })
        .collect()
}

fn execute_request(config: RequestConfig) -> Result<HttpResponse, String> {
    let mut request = match config.method.as_str() {
        "GET" => ureq::get(&config.url),
        "POST" => ureq::post(&config.url),
        "PUT" => ureq::put(&config.url),
        "DELETE" => ureq::delete(&config.url),
        "PATCH" => ureq::patch(&config.url),
        "HEAD" => ureq::head(&config.url),
        _ => return Err(format!("Unsupported HTTP method: {}", config.method)),
    };

    for (key, value) in &config.headers {
        request = request.set(key, value);
    }

    let response = if let Some(body) = config.body {
        request.send_string(&body)
    } else {
        request.call()
    };

    match response {
        Ok(resp) => {
            let status = resp.status();
            let status_text = resp.status_text().to_string();
            let mut headers = vec![];
            for name in resp.headers_names() {
                if let Some(value) = resp.header(&name) {
                    headers.push((name, value.to_string()));
                }
            }
            let body = resp
                .into_string()
                .map_err(|e| format!("Failed to read response: {}", e))?;

            Ok(HttpResponse {
                status,
                status_text,
                headers,
                body,
            })
        }
        Err(ureq::Error::Status(code, resp)) => {
            let status_text = resp.status_text().to_string();
            let body = resp.into_string().unwrap_or_default();
            Ok(HttpResponse {
                status: code,
                status_text,
                headers: vec![],
                body,
            })
        }
        Err(e) => Err(format!("Request failed: {}", e)),
    }
}

fn response_to_value(response: HttpResponse) -> Value {
    let headers: IndexMap<HashKey, Value> = response
        .headers
        .into_iter()
        .map(|(k, v)| (HashKey::String(k), Value::String(v)))
        .collect();

    let mut result: IndexMap<HashKey, Value> = IndexMap::new();
    result.insert(
        HashKey::String("status".to_string()),
        Value::Int(response.status as i64),
    );
    result.insert(
        HashKey::String("status_text".to_string()),
        Value::String(response.status_text),
    );
    result.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(headers))),
    );
    result.insert(
        HashKey::String("body".to_string()),
        Value::String(response.body),
    );

    Value::Hash(Rc::new(RefCell::new(result)))
}
