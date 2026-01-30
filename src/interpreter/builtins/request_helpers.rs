//! Request helper functions for Rails-like E2E controller testing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};

use super::test_server::get_test_server_port;

type HashPairs = IndexMap<HashKey, Value>;

thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static REQUEST_HEADERS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    #[allow(clippy::missing_const_for_thread_local)]
    static AUTH_HEADERS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static COOKIES: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn register_request_helpers(env: &mut Environment) {
    env.define(
        "get".to_string(),
        Value::NativeFunction(NativeFunction::new("get", Some(1), |args| {
            let path = extract_string(&args[0], "get(path)")?;
            http_request("GET", &path, None, None)
        })),
    );

    env.define(
        "post".to_string(),
        Value::NativeFunction(NativeFunction::new("post", Some(2), |args| {
            let path = extract_string(&args[0], "post(path, data)")?;
            let data = args[1].clone();
            http_request("POST", &path, None, Some(data))
        })),
    );

    env.define(
        "put".to_string(),
        Value::NativeFunction(NativeFunction::new("put", Some(2), |args| {
            let path = extract_string(&args[0], "put(path, data)")?;
            let data = args[1].clone();
            http_request("PUT", &path, None, Some(data))
        })),
    );

    env.define(
        "patch".to_string(),
        Value::NativeFunction(NativeFunction::new("patch", Some(2), |args| {
            let path = extract_string(&args[0], "patch(path, data)")?;
            let data = args[1].clone();
            http_request("PATCH", &path, None, Some(data))
        })),
    );

    env.define(
        "delete".to_string(),
        Value::NativeFunction(NativeFunction::new("delete", Some(1), |args| {
            let path = extract_string(&args[0], "delete(path)")?;
            http_request("DELETE", &path, None, None)
        })),
    );

    env.define(
        "head".to_string(),
        Value::NativeFunction(NativeFunction::new("head", Some(1), |args| {
            let path = extract_string(&args[0], "head(path)")?;
            http_request("HEAD", &path, None, None)
        })),
    );

    env.define(
        "options".to_string(),
        Value::NativeFunction(NativeFunction::new("options", Some(1), |args| {
            let path = extract_string(&args[0], "options(path)")?;
            http_request("OPTIONS", &path, None, None)
        })),
    );

    env.define(
        "request".to_string(),
        Value::NativeFunction(NativeFunction::new("request", Some(2), |args| {
            let method = extract_string(&args[0], "request(method, path)")?;
            let path = extract_string(&args[1], "request(method, path)")?;
            let body = args.get(2).cloned();
            http_request(&method, &path, None, body)
        })),
    );

    env.define(
        "set_header".to_string(),
        Value::NativeFunction(NativeFunction::new("set_header", Some(2), |args| {
            let name = extract_string(&args[0], "set_header(name, value)")?;
            let value = extract_string(&args[1], "set_header(name, value)")?;
            REQUEST_HEADERS.with(|cell| {
                let mut headers = cell.borrow_mut();
                headers.insert(name, value);
            });
            Ok(Value::Null)
        })),
    );

    env.define(
        "clear_authorization".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "clear_authorization",
            Some(0),
            |_args| {
                clear_authorization_inner();
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "set_authorization".to_string(),
        Value::NativeFunction(NativeFunction::new("set_authorization", Some(1), |args| {
            let token = extract_string(&args[0], "set_authorization(token)")?;
            AUTH_HEADERS.with(|cell| {
                let mut headers = cell.borrow_mut();
                headers.insert("Authorization".to_string(), format!("Bearer {}", token));
            });
            Ok(Value::Null)
        })),
    );

    env.define(
        "clear_authorization".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "clear_authorization",
            Some(0),
            |_args| {
                AUTH_HEADERS.with(|cell| {
                    *cell.borrow_mut() = HashMap::new();
                });
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "set_cookie".to_string(),
        Value::NativeFunction(NativeFunction::new("set_cookie", Some(2), |args| {
            let name = extract_string(&args[0], "set_cookie(name, value)")?;
            let value = extract_string(&args[1], "set_cookie(name, value)")?;
            COOKIES.with(|cell| {
                let mut cookies = cell.borrow_mut();
                if !cookies.is_empty() {
                    cookies.push(';');
                }
                cookies.push_str(&format!("{}={}", name, value));
            });
            Ok(Value::Null)
        })),
    );

    env.define(
        "clear_cookies".to_string(),
        Value::NativeFunction(NativeFunction::new("clear_cookies", Some(0), |_args| {
            clear_cookies_inner();
            Ok(Value::Null)
        })),
    );
}

pub fn clear_authorization_inner() {
    AUTH_HEADERS.with(|cell| {
        *cell.borrow_mut() = HashMap::new();
    });
}

pub fn clear_cookies_inner() {
    COOKIES.with(|cell| {
        *cell.borrow_mut() = String::new();
    });
}

pub fn set_authorization_inner(token: String) {
    AUTH_HEADERS.with(|cell| {
        let mut headers = cell.borrow_mut();
        headers.insert("Authorization".to_string(), format!("Bearer {}", token));
    });
}

pub fn set_cookie_inner(name: String, value: String) {
    COOKIES.with(|cell| {
        let mut cookies = cell.borrow_mut();
        if !cookies.is_empty() {
            cookies.push(';');
        }
        cookies.push_str(&format!("{}={}", name, value));
    });
}

fn http_request(
    method: &str,
    path: &str,
    _query: Option<HashMap<String, Value>>,
    body: Option<Value>,
) -> Result<Value, String> {
    let port = match get_test_server_port() {
        Some(p) => p,
        None => {
            return Err("Test server is not running. Call test_server_start() first.".to_string())
        }
    };

    let base_url = format!("http://127.0.0.1:{}", port);
    let full_url = format!("{}{}", base_url, path);

    let client = reqwest::blocking::Client::new();

    let mut request = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
        &full_url,
    );

    let mut all_headers: HashMap<String, String> = HashMap::new();

    REQUEST_HEADERS.with(|cell| {
        let headers = cell.borrow();
        for (name, value) in headers.iter() {
            all_headers.insert(name.clone(), value.clone());
        }
    });

    AUTH_HEADERS.with(|cell| {
        let headers = cell.borrow();
        for (name, value) in headers.iter() {
            all_headers.insert(name.clone(), value.clone());
        }
    });

    let cookies = COOKIES.with(|cell| cell.borrow().clone());

    for (name, value) in all_headers.iter() {
        request = request.header(name, value);
    }

    if !cookies.is_empty() {
        request = request.header("Cookie", cookies.as_str());
    }

    if let Some(body_val) = body {
        let body_str = value_to_string(body_val)?;
        request = request.header("Content-Type", "application/json");
        request = request.body(body_str);
    }

    let response = request.send().map_err(|e| e.to_string())?;

    let status = response.status().as_u16();
    let response_headers: HashMap<String, String> = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            Some((name.to_string(), value.to_str().unwrap_or("").to_string()))
        })
        .collect();

    let body = response.text().map_err(|e| e.to_string())?;

    let mut header_pairs: HashPairs = IndexMap::new();
    for (name, value) in response_headers {
        header_pairs.insert(HashKey::String(name), Value::String(value));
    }

    let mut response_hash: HashPairs = IndexMap::new();
    response_hash.insert(
        HashKey::String("status".to_string()),
        Value::Int(status as i64),
    );
    response_hash.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(header_pairs))),
    );
    response_hash.insert(
        HashKey::String("body".to_string()),
        Value::String(body.clone()),
    );
    response_hash.insert(HashKey::String("url".to_string()), Value::String(full_url));
    response_hash.insert(
        HashKey::String("method".to_string()),
        Value::String(method.to_string()),
    );

    Ok(Value::Hash(Rc::new(RefCell::new(response_hash))))
}

fn extract_string(value: &Value, context: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        _ => Err(format!("{} expects string argument", context)),
    }
}

fn value_to_string(value: Value) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s),
        Value::Int(n) => Ok(n.to_string()),
        Value::Float(f) => Ok(f.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Null => Ok("null".to_string()),
        _ => Err(format!("Cannot convert {} to string", value.type_name())),
    }
}
