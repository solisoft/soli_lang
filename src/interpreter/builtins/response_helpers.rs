//! Response helper functions for Rails-like E2E controller testing.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

type HashPairs = Vec<(Value, Value)>;

pub fn register_response_helpers(env: &mut Environment) {
    env.define(
        "res_status".to_string(),
        Value::NativeFunction(NativeFunction::new("res_status", Some(1), |args| {
            let response = &args[0];
            extract_status(response)
        })),
    );

    env.define(
        "res_body".to_string(),
        Value::NativeFunction(NativeFunction::new("res_body", Some(1), |args| {
            let response = &args[0];
            extract_body(response)
        })),
    );

    env.define(
        "res_json".to_string(),
        Value::NativeFunction(NativeFunction::new("res_json", Some(1), |args| {
            let response = &args[0];
            extract_json(response)
        })),
    );

    env.define(
        "res_header".to_string(),
        Value::NativeFunction(NativeFunction::new("res_header", Some(2), |args| {
            let response = &args[0];
            let name = extract_string(&args[1], "res_header(response, name)")?;
            extract_header(response, &name)
        })),
    );

    env.define(
        "res_headers".to_string(),
        Value::NativeFunction(NativeFunction::new("res_headers", Some(1), |args| {
            let response = &args[0];
            extract_all_headers(response)
        })),
    );

    env.define(
        "res_redirect?".to_string(),
        Value::NativeFunction(NativeFunction::new("res_redirect?", Some(1), |args| {
            let response = &args[0];
            Ok(Value::Bool(is_redirect(response)))
        })),
    );

    env.define(
        "res_location".to_string(),
        Value::NativeFunction(NativeFunction::new("res_location", Some(1), |args| {
            let response = &args[0];
            extract_location(response)
        })),
    );

    env.define(
        "res_ok?".to_string(),
        Value::NativeFunction(NativeFunction::new("res_ok?", Some(1), |args| {
            let response = &args[0];
            Ok(Value::Bool(is_success(response)))
        })),
    );

    env.define(
        "res_client_error?".to_string(),
        Value::NativeFunction(NativeFunction::new("res_client_error?", Some(1), |args| {
            let response = &args[0];
            Ok(Value::Bool(is_client_error(response)))
        })),
    );

    env.define(
        "res_server_error?".to_string(),
        Value::NativeFunction(NativeFunction::new("res_server_error?", Some(1), |args| {
            let response = &args[0];
            Ok(Value::Bool(is_server_error(response)))
        })),
    );

    env.define(
        "res_not_found?".to_string(),
        Value::NativeFunction(NativeFunction::new("res_not_found?", Some(1), |args| {
            let response = &args[0];
            Ok(Value::Bool(is_not_found(response)))
        })),
    );

    env.define(
        "res_unauthorized?".to_string(),
        Value::NativeFunction(NativeFunction::new("res_unauthorized?", Some(1), |args| {
            let response = &args[0];
            Ok(Value::Bool(is_unauthorized(response)))
        })),
    );

    env.define(
        "res_forbidden?".to_string(),
        Value::NativeFunction(NativeFunction::new("res_forbidden?", Some(1), |args| {
            let response = &args[0];
            Ok(Value::Bool(is_forbidden(response)))
        })),
    );

    env.define(
        "res_unprocessable?".to_string(),
        Value::NativeFunction(NativeFunction::new("res_unprocessable?", Some(1), |args| {
            let response = &args[0];
            Ok(Value::Bool(is_unprocessable(response)))
        })),
    );

    env.define(
        "render_template?".to_string(),
        Value::NativeFunction(NativeFunction::new("render_template?", Some(0), |_args| {
            Ok(Value::Bool(false))
        })),
    );

    env.define(
        "view_path".to_string(),
        Value::NativeFunction(NativeFunction::new("view_path", Some(0), |_args| {
            Ok(Value::String(String::new()))
        })),
    );
}

fn extract_status(response: &Value) -> Result<Value, String> {
    match response {
        Value::Hash(h) => {
            let hash = h.borrow();
            for (k, v) in hash.iter() {
                if let Value::String(key) = k {
                    if key == "status" {
                        return Ok(v.clone());
                    }
                }
            }
            Err("Response missing status field".to_string())
        }
        _ => Err("res_status() expects hash argument".to_string()),
    }
}

fn extract_body(response: &Value) -> Result<Value, String> {
    match response {
        Value::Hash(h) => {
            let hash = h.borrow();
            for (k, v) in hash.iter() {
                if let Value::String(key) = k {
                    if key == "body" {
                        return Ok(v.clone());
                    }
                }
            }
            Err("Response missing body field".to_string())
        }
        _ => Err("res_body() expects hash argument".to_string()),
    }
}

fn extract_json(response: &Value) -> Result<Value, String> {
    let body = extract_body(response)?;
    match body {
        Value::String(s) => match serde_json::from_str(&s) {
            Ok(json) => json_to_value(json),
            Err(e) => Err(format!("Invalid JSON: {}", e)),
        },
        _ => Err("Body is not a string".to_string()),
    }
}

fn extract_header(response: &Value, name: &str) -> Result<Value, String> {
    match response {
        Value::Hash(h) => {
            let hash = h.borrow();
            for (k, v) in hash.iter() {
                if let Value::String(key) = k {
                    if key == "headers" {
                        if let Value::Hash(headers) = v {
                            let headers_hash = headers.borrow();
                            for (hk, hv) in headers_hash.iter() {
                                if let Value::String(header_name) = hk {
                                    if header_name == name {
                                        if let Value::String(s) = hv {
                                            return Ok(Value::String(s.clone()));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(Value::Null)
        }
        _ => Err("res_header() expects hash argument".to_string()),
    }
}

fn extract_all_headers(response: &Value) -> Result<Value, String> {
    match response {
        Value::Hash(h) => {
            let hash = h.borrow();
            for (k, v) in hash.iter() {
                if let Value::String(key) = k {
                    if key == "headers" {
                        return Ok(v.clone());
                    }
                }
            }
            Ok(Value::Hash(Rc::new(RefCell::new(Vec::new()))))
        }
        _ => Err("res_headers() expects hash argument".to_string()),
    }
}

fn is_redirect(response: &Value) -> bool {
    matches!(extract_status(response), Ok(Value::Int(s)) if (300..400).contains(&s))
}

fn is_success(response: &Value) -> bool {
    matches!(extract_status(response), Ok(Value::Int(s)) if (200..300).contains(&s))
}

fn is_client_error(response: &Value) -> bool {
    matches!(extract_status(response), Ok(Value::Int(s)) if (400..500).contains(&s))
}

fn is_server_error(response: &Value) -> bool {
    matches!(extract_status(response), Ok(Value::Int(s)) if (500..600).contains(&s))
}

fn is_not_found(response: &Value) -> bool {
    matches!(extract_status(response), Ok(Value::Int(404)))
}

fn is_unauthorized(response: &Value) -> bool {
    matches!(extract_status(response), Ok(Value::Int(401)))
}

fn is_forbidden(response: &Value) -> bool {
    matches!(extract_status(response), Ok(Value::Int(403)))
}

fn is_unprocessable(response: &Value) -> bool {
    matches!(extract_status(response), Ok(Value::Int(422)))
}

fn extract_location(response: &Value) -> Result<Value, String> {
    extract_header(response, "Location")
}

fn extract_string(value: &Value, context: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        _ => Err(format!("{} expects string argument", context)),
    }
}

fn json_to_value(json: serde_json::Value) -> Result<Value, String> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Ok(Value::Float(n.as_f64().unwrap_or_default()))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s)),
        serde_json::Value::Array(arr) => {
            let values: Result<Vec<Value>, String> = arr.into_iter().map(json_to_value).collect();
            Ok(Value::Array(Rc::new(RefCell::new(values?))))
        }
        serde_json::Value::Object(obj) => {
            let mut pairs: HashPairs = Vec::new();
            for (k, v) in obj {
                pairs.push((Value::String(k), json_to_value(v)?));
            }
            Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
        }
    }
}
