//! HTTP client built-in functions.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{FutureState, HttpFutureKind, NativeFunction, Value};

/// Create a Future value from a closure that will be run in a background thread.
/// The closure should return raw String data that will be converted to Value based on kind.
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

/// Register HTTP client functions in the given environment.
pub fn register_http_builtins(env: &mut Environment) {
    // http_get(url) - Make a GET request, returns Future that resolves to response body
    env.define(
        "http_get".to_string(),
        Value::NativeFunction(NativeFunction::new("http_get", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_get() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(spawn_http_future(
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
            ))
        })),
    );

    // http_post(url, body) - Make a POST request with string body, returns Future
    env.define(
        "http_post".to_string(),
        Value::NativeFunction(NativeFunction::new("http_post", Some(2), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_post() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            let body = match &args[1] {
                Value::String(s) => s.clone(),
                Value::Hash(_) => {
                    // Convert hash to JSON
                    value_to_json(&args[1])?
                }
                other => {
                    return Err(format!(
                        "http_post() expects string or hash body, got {}",
                        other.type_name()
                    ))
                }
            };

            let content_type = if args[1].type_name() == "Hash" {
                "application/json".to_string()
            } else {
                "text/plain".to_string()
            };

            Ok(spawn_http_future(
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
            ))
        })),
    );

    // http_post_json(url, data) - Make a POST request with JSON body, returns Future
    env.define(
        "http_post_json".to_string(),
        Value::NativeFunction(NativeFunction::new("http_post_json", Some(2), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_post_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            let json_body = value_to_json(&args[1])?;

            Ok(spawn_http_future(
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
            ))
        })),
    );

    // http_get_json(url) - Make a GET request and parse response as JSON, returns Future
    env.define(
        "http_get_json".to_string(),
        Value::NativeFunction(NativeFunction::new("http_get_json", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_get_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(spawn_http_future(
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
            ))
        })),
    );

    // http_request(method, url, headers?, body?) - Generic HTTP request, returns Future
    env.define(
        "http_request".to_string(),
        Value::NativeFunction(NativeFunction::new("http_request", None, |args| {
            if args.len() < 2 {
                return Err("http_request() requires at least method and URL".to_string());
            }

            let method = match &args[0] {
                Value::String(s) => s.to_uppercase(),
                other => {
                    return Err(format!(
                        "http_request() method must be string, got {}",
                        other.type_name()
                    ))
                }
            };

            let url = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_request() URL must be string, got {}",
                        other.type_name()
                    ))
                }
            };

            // Extract headers into thread-safe Vec
            let mut headers_vec: Vec<(String, String)> = Vec::new();
            if args.len() > 2 {
                if let Value::Hash(headers) = &args[2] {
                    for (key, value) in headers.borrow().iter() {
                        let key_str = match key {
                            Value::String(s) => s.clone(),
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

            // Extract body if provided
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

            Ok(spawn_http_future(
                move || {
                    // Build request
                    let mut request = match method.as_str() {
                        "GET" => ureq::get(&url),
                        "POST" => ureq::post(&url),
                        "PUT" => ureq::put(&url),
                        "DELETE" => ureq::delete(&url),
                        "PATCH" => ureq::patch(&url),
                        "HEAD" => ureq::head(&url),
                        _ => return Err(format!("Unsupported HTTP method: {}", method)),
                    };

                    // Add headers
                    for (key, value) in &headers_vec {
                        request = request.set(key, value);
                    }

                    // Send request
                    let response = if let Some(body) = body_opt {
                        request.send_string(&body)
                    } else {
                        request.call()
                    };

                    // Build response as JSON string (will be converted to Value on resolve)
                    match response {
                        Ok(resp) => {
                            let status = resp.status();
                            let status_text = resp.status_text().to_string();

                            // Collect headers as JSON object
                            let mut headers_map = serde_json::Map::new();
                            for name in resp.headers_names() {
                                if let Some(value) = resp.header(&name) {
                                    headers_map
                                        .insert(name, serde_json::Value::String(value.to_string()));
                                }
                            }

                            let body = resp
                                .into_string()
                                .map_err(|e| format!("Failed to read response body: {}", e))?;

                            // Create JSON response object
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
        })),
    );

    // http_ok(response) - Check if response status is 2xx (success)
    env.define(
        "http_ok".to_string(),
        Value::NativeFunction(NativeFunction::new("http_ok", Some(1), |args| {
            let status = extract_status(&args[0])?;
            Ok(Value::Bool((200..300).contains(&status)))
        })),
    );

    // http_success(response) - Alias for http_ok
    env.define(
        "http_success".to_string(),
        Value::NativeFunction(NativeFunction::new("http_success", Some(1), |args| {
            let status = extract_status(&args[0])?;
            Ok(Value::Bool((200..300).contains(&status)))
        })),
    );

    // http_redirect(response) - Check if response status is 3xx (redirect)
    env.define(
        "http_redirect".to_string(),
        Value::NativeFunction(NativeFunction::new("http_redirect", Some(1), |args| {
            let status = extract_status(&args[0])?;
            Ok(Value::Bool((300..400).contains(&status)))
        })),
    );

    // http_client_error(response) - Check if response status is 4xx (client error)
    env.define(
        "http_client_error".to_string(),
        Value::NativeFunction(NativeFunction::new("http_client_error", Some(1), |args| {
            let status = extract_status(&args[0])?;
            Ok(Value::Bool((400..500).contains(&status)))
        })),
    );

    // http_server_error(response) - Check if response status is 5xx (server error)
    env.define(
        "http_server_error".to_string(),
        Value::NativeFunction(NativeFunction::new("http_server_error", Some(1), |args| {
            let status = extract_status(&args[0])?;
            Ok(Value::Bool((500..600).contains(&status)))
        })),
    );

    // json_parse(string) - Parse JSON string into Soli value
    env.define(
        "json_parse".to_string(),
        Value::NativeFunction(NativeFunction::new("json_parse", Some(1), |args| {
            let json_str = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "json_parse() expects string, got {}",
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

    // json_stringify(value) - Convert Soli value to JSON string
    env.define(
        "json_stringify".to_string(),
        Value::NativeFunction(NativeFunction::new("json_stringify", Some(1), |args| {
            value_to_json(&args[0]).map(Value::String)
        })),
    );

    // http_get_all(urls) - Make multiple GET requests in parallel
    env.define(
        "http_get_all".to_string(),
        Value::NativeFunction(NativeFunction::new("http_get_all", Some(1), |args| {
            let urls = match &args[0] {
                Value::Array(arr) => {
                    let mut url_strings = Vec::new();
                    for item in arr.borrow().iter() {
                        match item {
                            Value::String(s) => url_strings.push(s.clone()),
                            other => {
                                return Err(format!(
                                    "http_get_all() expects array of strings, got {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    url_strings
                }
                other => {
                    return Err(format!(
                        "http_get_all() expects array of URLs, got {}",
                        other.type_name()
                    ))
                }
            };

            // Run requests in parallel using threads
            let results = run_parallel_gets(urls);

            // Convert results to Value array
            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(body) => Value::String(body),
                    Err(e) => Value::Hash(Rc::new(RefCell::new(vec![(
                        Value::String("error".to_string()),
                        Value::String(e),
                    )]))),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    // http_parallel(requests) - Make multiple HTTP requests in parallel
    env.define(
        "http_parallel".to_string(),
        Value::NativeFunction(NativeFunction::new("http_parallel", Some(1), |args| {
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
                        "http_parallel() expects array of request configs, got {}",
                        other.type_name()
                    ))
                }
            };

            // Run requests in parallel using threads
            let results = run_parallel_requests(requests);

            // Convert results to Value array
            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(response) => response_to_value(response),
                    Err(e) => Value::Hash(Rc::new(RefCell::new(vec![(
                        Value::String("error".to_string()),
                        Value::String(e),
                    )]))),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );
}

/// Convert a Soli Value to a JSON string.
fn value_to_json(value: &Value) -> Result<String, String> {
    let json = crate::interpreter::value::value_to_json(value)?;
    serde_json::to_string(&json).map_err(|e| format!("JSON serialization error: {}", e))
}

/// Extract status code from a response hash or integer.
/// Auto-resolves Futures before extracting.
fn extract_status(value: &Value) -> Result<i64, String> {
    // Auto-resolve Futures
    let resolved = value.clone().resolve()?;

    match &resolved {
        Value::Int(n) => Ok(*n),
        Value::Hash(hash) => {
            for (k, v) in hash.borrow().iter() {
                if let Value::String(key) = k {
                    if key == "status" {
                        if let Value::Int(status) = v {
                            return Ok(*status);
                        }
                    }
                }
            }
            Err("Response hash does not contain 'status' field".to_string())
        }
        other => Err(format!(
            "Expected response hash or status code, got {}",
            other.type_name()
        )),
    }
}

// Use centralized json_to_value from value module
use crate::interpreter::value::json_to_value;

// ========== Parallel HTTP Execution ==========

/// Configuration for a single HTTP request (thread-safe)
#[derive(Clone)]
struct RequestConfig {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
}

/// Response from an HTTP request (thread-safe)
struct HttpResponse {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// Parse a Value (hash or string) into a RequestConfig
fn parse_request_config(value: &Value) -> Result<RequestConfig, String> {
    match value {
        // Simple string URL = GET request
        Value::String(url) => Ok(RequestConfig {
            method: "GET".to_string(),
            url: url.clone(),
            headers: vec![],
            body: None,
        }),
        // Hash with url, method, headers, body
        Value::Hash(hash) => {
            let hash = hash.borrow();
            let mut url = None;
            let mut method = "GET".to_string();
            let mut headers = vec![];
            let mut body = None;

            for (k, v) in hash.iter() {
                if let Value::String(key) = k {
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
                                    if let (Value::String(k), Value::String(v)) = (hk, hv) {
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

/// Run multiple GET requests in parallel
fn run_parallel_gets(urls: Vec<String>) -> Vec<Result<String, String>> {
    use std::thread;

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

/// Run multiple HTTP requests in parallel
fn run_parallel_requests(requests: Vec<RequestConfig>) -> Vec<Result<HttpResponse, String>> {
    use std::thread;

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

/// Execute a single HTTP request
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

    // Add headers
    for (key, value) in &config.headers {
        request = request.set(key, value);
    }

    // Send request
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

/// Convert HttpResponse to a Soli Value (hash)
fn response_to_value(response: HttpResponse) -> Value {
    let headers: Vec<(Value, Value)> = response
        .headers
        .into_iter()
        .map(|(k, v)| (Value::String(k), Value::String(v)))
        .collect();

    let result: Vec<(Value, Value)> = vec![
        (
            Value::String("status".to_string()),
            Value::Int(response.status as i64),
        ),
        (
            Value::String("status_text".to_string()),
            Value::String(response.status_text),
        ),
        (
            Value::String("headers".to_string()),
            Value::Hash(Rc::new(RefCell::new(headers))),
        ),
        (
            Value::String("body".to_string()),
            Value::String(response.body),
        ),
    ];

    Value::Hash(Rc::new(RefCell::new(result)))
}
