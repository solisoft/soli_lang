//! View-introspection helpers for Rails-like E2E controller testing.
//!
//! These run in the **test-runner** process. The view path + locals are
//! rendered in the spawned `soli serve` child and shipped back as the
//! `x-soli-test-*` response headers (see `serve::mod::finalize_response`).
//! `request_helpers::http_request` decodes those headers after every
//! `get`/`post`/... and stores them here via [`set_last_render`], so the
//! `assigns()` / `assign(key)` / `view_path()` / `render_template()` helpers
//! reflect the most recent response.

use std::cell::RefCell;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{empty_hash, json_to_value, HashKey, NativeFunction, Value};

thread_local! {
    static LAST_ASSIGNS_JSON: RefCell<Option<String>> = const { RefCell::new(None) };
    static LAST_VIEW_PATH: RefCell<Option<String>> = const { RefCell::new(None) };
    static LAST_RENDERED: RefCell<bool> = const { RefCell::new(false) };
}

/// Record the render captured from the most recent test HTTP response.
/// `view_path`/`assigns_json` are `None` when the response rendered no
/// template (a redirect or a JSON/text response), which makes
/// `render_template()` report `false`. `assigns_json` is the already-decoded
/// JSON string (the transport-level base64 is undone by the caller).
pub fn set_last_render(view_path: Option<String>, assigns_json: Option<String>) {
    let rendered = view_path.is_some();
    LAST_VIEW_PATH.with(|cell| *cell.borrow_mut() = view_path);
    LAST_ASSIGNS_JSON.with(|cell| *cell.borrow_mut() = assigns_json);
    LAST_RENDERED.with(|cell| *cell.borrow_mut() = rendered);
}

/// Reset the captured render (no template). Mostly covered by `set_last_render`
/// running on every response, but kept for explicit teardown.
pub fn clear_last_render() {
    set_last_render(None, None);
}

pub fn register_assigns_helpers(env: &mut Environment) {
    env.define(
        "assigns".to_string(),
        Value::NativeFunction(NativeFunction::new("assigns", Some(0), |_args| {
            Ok(parsed_assigns())
        })),
    );

    env.define(
        "assign".to_string(),
        Value::NativeFunction(NativeFunction::new("assign", Some(1), |args| {
            let key = extract_string(&args[0], "assign(key)")?;
            Ok(get_assign(&key))
        })),
    );

    env.define(
        "view_path".to_string(),
        Value::NativeFunction(NativeFunction::new("view_path", Some(0), |_args| {
            get_view_path()
        })),
    );

    // Both spellings: `render_template()` (the form the scaffold + docs use)
    // and the `render_template?()` predicate.
    env.define(
        "render_template".to_string(),
        Value::NativeFunction(NativeFunction::new("render_template", Some(0), |_args| {
            Ok(Value::Bool(was_rendered()))
        })),
    );
    env.define(
        "render_template?".to_string(),
        Value::NativeFunction(NativeFunction::new("render_template?", Some(0), |_args| {
            Ok(Value::Bool(was_rendered()))
        })),
    );
}

fn was_rendered() -> bool {
    LAST_RENDERED.with(|cell| *cell.borrow())
}

/// Parse the captured locals JSON into a Soli hash. Returns an empty hash when
/// nothing was captured or the JSON is unparseable.
fn parsed_assigns() -> Value {
    LAST_ASSIGNS_JSON.with(|cell| match &*cell.borrow() {
        Some(json) => match serde_json::from_str::<serde_json::Value>(json) {
            Ok(parsed) => json_to_value(parsed).unwrap_or_else(|_| empty_hash()),
            Err(_) => empty_hash(),
        },
        None => empty_hash(),
    })
}

fn get_assign(key: &str) -> Value {
    if let Value::Hash(hash) = parsed_assigns() {
        for (k, v) in hash.borrow().iter() {
            if let HashKey::String(name) = k {
                if name.as_ref() == key {
                    return v.clone();
                }
            }
        }
    }
    Value::Null
}

fn get_view_path() -> Result<Value, String> {
    LAST_VIEW_PATH.with(|cell| match &*cell.borrow() {
        Some(path) => Ok(Value::String(path.clone().into())),
        None => Ok(Value::String(String::new().into())),
    })
}

fn extract_string(value: &Value, context: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone().to_string()),
        _ => Err(format!("{} expects string argument", context)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_render_populates_helpers() {
        set_last_render(
            Some("posts/index".to_string()),
            Some(r#"{"title":"Hi","count":3}"#.to_string()),
        );
        assert!(was_rendered());
        match get_view_path().unwrap() {
            Value::String(s) => assert_eq!(s.to_string(), "posts/index"),
            _ => panic!("view_path should be a string"),
        }
        match get_assign("title") {
            Value::String(s) => assert_eq!(s.to_string(), "Hi"),
            _ => panic!("assign(title) should be a string"),
        }
        match get_assign("count") {
            Value::Int(n) => assert_eq!(n, 3),
            _ => panic!("assign(count) should be an int"),
        }
        assert!(matches!(get_assign("missing"), Value::Null));
    }

    #[test]
    fn no_render_reports_false_and_empty() {
        set_last_render(None, None);
        assert!(!was_rendered());
        match get_view_path().unwrap() {
            Value::String(s) => assert_eq!(s.to_string(), ""),
            _ => panic!("view_path should be the empty string"),
        }
        assert!(matches!(get_assign("anything"), Value::Null));
        match parsed_assigns() {
            Value::Hash(h) => assert!(h.borrow().iter().next().is_none()),
            _ => panic!("assigns should be an (empty) hash"),
        }
    }

    #[test]
    fn malformed_assigns_json_degrades_to_empty_hash() {
        // A render happened (view path present) but the locals JSON is junk.
        set_last_render(Some("x".to_string()), Some("{not valid json".to_string()));
        assert!(was_rendered());
        match parsed_assigns() {
            Value::Hash(h) => assert!(h.borrow().iter().next().is_none()),
            _ => panic!("assigns should degrade to an empty hash"),
        }
    }
}
