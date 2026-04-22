//! Request helper functions for Rails-like E2E controller testing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};

use super::test_server::get_test_server_port;

thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static REQUEST_HEADERS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    #[allow(clippy::missing_const_for_thread_local)]
    static AUTH_HEADERS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static COOKIES: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn register_request_helpers(env: &mut Environment) {
    // Test HTTP helpers accept variadic args so callers can tack on an
    // options hash (with a "headers" sub-hash) — scaffolded tests use
    // `get(path, null, { "headers": { ... } })` and
    // `post(path, body, { "headers": { ... } })`.
    env.define(
        "get".to_string(),
        Value::NativeFunction(NativeFunction::new("get", None, |args| {
            let path = extract_string(&args[0], "get(path)")?;
            let options = args.get(2).cloned();
            http_request("GET", &path, None, None, options)
        })),
    );

    env.define(
        "post".to_string(),
        Value::NativeFunction(NativeFunction::new("post", None, |args| {
            let path = extract_string(&args[0], "post(path, data)")?;
            let data = args.get(1).cloned();
            let options = args.get(2).cloned();
            http_request("POST", &path, None, data, options)
        })),
    );

    env.define(
        "put".to_string(),
        Value::NativeFunction(NativeFunction::new("put", None, |args| {
            let path = extract_string(&args[0], "put(path, data)")?;
            let data = args.get(1).cloned();
            let options = args.get(2).cloned();
            http_request("PUT", &path, None, data, options)
        })),
    );

    env.define(
        "patch".to_string(),
        Value::NativeFunction(NativeFunction::new("patch", None, |args| {
            let path = extract_string(&args[0], "patch(path, data)")?;
            let data = args.get(1).cloned();
            let options = args.get(2).cloned();
            http_request("PATCH", &path, None, data, options)
        })),
    );

    env.define(
        "delete".to_string(),
        Value::NativeFunction(NativeFunction::new("delete", None, |args| {
            let path = extract_string(&args[0], "delete(path)")?;
            let options = args.get(1).cloned();
            http_request("DELETE", &path, None, None, options)
        })),
    );

    env.define(
        "head".to_string(),
        Value::NativeFunction(NativeFunction::new("head", None, |args| {
            let path = extract_string(&args[0], "head(path)")?;
            let options = args.get(1).cloned();
            http_request("HEAD", &path, None, None, options)
        })),
    );

    env.define(
        "options".to_string(),
        Value::NativeFunction(NativeFunction::new("options", None, |args| {
            let path = extract_string(&args[0], "options(path)")?;
            let options = args.get(1).cloned();
            http_request("OPTIONS", &path, None, None, options)
        })),
    );

    env.define(
        "request".to_string(),
        Value::NativeFunction(NativeFunction::new("request", None, |args| {
            let method = extract_string(&args[0], "request(method, path)")?;
            let path = extract_string(&args[1], "request(method, path)")?;
            let body = args.get(2).cloned();
            let options = args.get(3).cloned();
            http_request(&method, &path, None, body, options)
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

    env.define(
        "clear_headers".to_string(),
        Value::NativeFunction(NativeFunction::new("clear_headers", Some(0), |_args| {
            REQUEST_HEADERS.with(|cell| {
                *cell.borrow_mut() = HashMap::new();
            });
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
        // Parse current jar, drop any existing entry for this name, then
        // either append the new entry (if non-empty) or drop it.
        let mut pairs: Vec<(String, String)> = Vec::new();
        for pair in cookies.split(';') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            if let Some((k, v)) = pair.split_once('=') {
                if k.trim() == name {
                    continue;
                }
                pairs.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        if !value.is_empty() {
            pairs.push((name, value));
        }
        *cookies = pairs
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
    });
}

pub fn current_cookies() -> String {
    COOKIES.with(|cell| cell.borrow().clone())
}

fn http_request(
    method: &str,
    path: &str,
    _query: Option<HashMap<String, Value>>,
    body: Option<Value>,
    options: Option<Value>,
) -> Result<Value, String> {
    let port = match get_test_server_port() {
        Some(p) => p,
        None => {
            return Err("Test server is not running. Call test_server_start() first.".to_string())
        }
    };

    let full_url = format!("http://127.0.0.1:{}{}", port, path);

    let mut all_headers: HashMap<String, String> = HashMap::new();
    REQUEST_HEADERS.with(|cell| {
        for (name, value) in cell.borrow().iter() {
            all_headers.insert(name.clone(), value.clone());
        }
    });
    AUTH_HEADERS.with(|cell| {
        for (name, value) in cell.borrow().iter() {
            all_headers.insert(name.clone(), value.clone());
        }
    });
    // Extract per-call options, e.g. { "headers": { "HX-Request": "true" } }.
    if let Some(Value::Hash(opts)) = options {
        if let Some(Value::Hash(headers)) = opts
            .borrow()
            .get(&HashKey::String("headers".to_string()))
            .cloned()
        {
            for (k, v) in headers.borrow().iter() {
                if let HashKey::String(name) = k {
                    if let Value::String(s) = v {
                        all_headers.insert(name.clone(), s.clone());
                    }
                }
            }
        }
    }
    let cookies = COOKIES.with(|cell| cell.borrow().clone());

    let body_str = match body {
        None | Some(Value::Null) => None,
        Some(body_val) => Some(value_to_string(body_val)?),
    };

    let (status, response_headers, set_cookies, body_text) = raw_http_request(
        port,
        method,
        path,
        &all_headers,
        &cookies,
        body_str.as_deref(),
    )?;

    // Update the thread-local cookie jar with any Set-Cookie values so the
    // next request carries them (session continuity across get/post calls).
    for raw in &set_cookies {
        let kv = raw.split(';').next().unwrap_or(raw).trim();
        if let Some((name, value)) = kv.split_once('=') {
            set_cookie_inner(name.trim().to_string(), value.trim().to_string());
        }
    }

    // Logout convention: hitting any path ending in "/logout" that returns
    // success or redirect should tear down the client's session state.
    // Soli controllers typically delete the user from the session but don't
    // send a clearing cookie, so the cookie jar alone can't tell us whether
    // the user is still signed in. Pattern-matching the path keeps the
    // test DSL's `signed_in()` honest across `logout()` helper and raw
    // `post("/logout")` calls.
    if (200..400).contains(&status) {
        let path_only = path.split('?').next().unwrap_or(path);
        if path_only == "/logout" || path_only.ends_with("/logout") {
            clear_cookies_inner();
            super::session_helpers::clear_test_user_public();
        }
    }

    let mut header_pairs: HashPairs = HashPairs::default();
    for (name, value) in response_headers {
        header_pairs.insert(HashKey::String(name), Value::String(value));
    }

    let mut response_hash: HashPairs = HashPairs::default();
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
        Value::String(body_text.clone()),
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
        Value::Hash(_) | Value::Array(_) => Ok(value_to_json(&value)),
        _ => Err(format!("Cannot convert {} to string", value.type_name())),
    }
}

fn value_to_json(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        Value::Hash(h) => {
            let hash = h.borrow();
            let pairs: Vec<String> = hash
                .iter()
                .map(|(k, v)| {
                    let key = match k {
                        HashKey::String(s) => s.clone(),
                        HashKey::Symbol(s) => s.clone(),
                        HashKey::Int(i) => i.to_string(),
                        HashKey::Bool(b) => b.to_string(),
                        HashKey::Decimal(d) => d.to_string(),
                        HashKey::Null => "null".to_string(),
                    };
                    format!("\"{}\":{}", key, value_to_json(v))
                })
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        Value::Array(arr) => {
            let arr = arr.borrow();
            let items: Vec<String> = arr.iter().map(value_to_json).collect();
            format!("[{}]", items.join(","))
        }
        _ => format!("\"{}\"", value),
    }
}

#[allow(clippy::type_complexity)]
fn raw_http_request(
    port: u16,
    method: &str,
    path: &str,
    all_headers: &HashMap<String, String>,
    cookies: &str,
    body: Option<&str>,
) -> Result<(u16, HashMap<String, String>, Vec<String>, String), String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let addr = format!("127.0.0.1:{}", port);
    let stream =
        TcpStream::connect_timeout(&addr.parse().unwrap(), std::time::Duration::from_secs(5))
            .map_err(|e| format!("connect to test server failed: {}", e))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    let mut stream = stream;

    let mut req_headers = String::new();
    req_headers.push_str(&format!("{} {} HTTP/1.1\r\n", method, path));
    req_headers.push_str(&format!("Host: 127.0.0.1:{}\r\n", port));
    req_headers.push_str("Connection: close\r\n");
    req_headers.push_str("User-Agent: soli-test/1.0\r\n");

    let has_ct = all_headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("content-type"));
    if body.is_some() && !has_ct {
        req_headers.push_str("Content-Type: application/json\r\n");
    }
    for (name, value) in all_headers {
        req_headers.push_str(&format!("{}: {}\r\n", name, value));
    }
    if !cookies.is_empty() {
        req_headers.push_str(&format!("Cookie: {}\r\n", cookies));
    }
    let body_bytes = body.unwrap_or("");
    req_headers.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
    req_headers.push_str("\r\n");

    stream
        .write_all(req_headers.as_bytes())
        .map_err(|e| format!("write headers failed: {}", e))?;
    if !body_bytes.is_empty() {
        stream
            .write_all(body_bytes.as_bytes())
            .map_err(|e| format!("write body failed: {}", e))?;
    }
    stream.flush().map_err(|e| format!("flush failed: {}", e))?;

    let mut raw = Vec::with_capacity(4096);
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) => return Err(format!("read response failed: {}", e)),
        }
    }

    let header_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "malformed response: no header terminator".to_string())?;
    let (head_bytes, rest) = raw.split_at(header_end + 4);
    let head =
        std::str::from_utf8(head_bytes).map_err(|e| format!("invalid response headers: {}", e))?;
    let body_text = String::from_utf8_lossy(rest).to_string();

    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("missing status line")?;
    let mut status_parts = status_line.splitn(3, ' ');
    status_parts.next();
    let status_code: u16 = status_parts
        .next()
        .ok_or("missing status code")?
        .parse()
        .map_err(|e| format!("bad status code: {}", e))?;

    let mut response_headers: HashMap<String, String> = HashMap::new();
    let mut set_cookies: Vec<String> = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let value = v.trim();
            if key.eq_ignore_ascii_case("set-cookie") {
                set_cookies.push(value.to_string());
            }
            response_headers.insert(key.to_string(), value.to_string());
            // Hyper lowercases response header names on the wire, but CRM
            // scaffolded tests directly read `response["headers"]["HX-Redirect"]`
            // (case-sensitive hash lookup). Publish a canonical Title-Case
            // alias so both forms work. e.g. `hx-redirect` → also stored
            // as `Hx-Redirect`, and — special-cased — `HX-Redirect` for the
            // `hx-*` family that HTMX docs canonicalize as uppercase prefix.
            let title = canonicalize_header_name(key);
            if title != key {
                response_headers
                    .entry(title.clone())
                    .or_insert_with(|| value.to_string());
            }
            if let Some(upper_alias) = uppercase_prefix_alias(key) {
                if upper_alias != key && upper_alias != title {
                    response_headers
                        .entry(upper_alias)
                        .or_insert_with(|| value.to_string());
                }
            }
        }
    }
    Ok((status_code, response_headers, set_cookies, body_text))
}

/// Convert an HTTP header name to canonical Title-Case form, e.g.
/// `hx-redirect` → `Hx-Redirect`, `content-type` → `Content-Type`.
fn canonicalize_header_name(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => {
                    c.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// For well-known uppercase-prefix families (HTMX uses `HX-*`, WebDAV uses
/// `DAV`, etc.), also expose the uppercase-prefix form so direct hash
/// lookups like `response["headers"]["HX-Redirect"]` hit.
fn uppercase_prefix_alias(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("hx-") {
        let mut out = String::from("HX-");
        out.push_str(&canonicalize_header_name(rest));
        return Some(out);
    }
    None
}
