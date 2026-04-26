//! `respond_to` — Rails-style content negotiation primitive.
//!
//! Used as `respond_to(req, fn(format) { format.html(...); format.json(...); })`
//! or `respond_to(req, {"html": fn() ..., "json": fn() ...})`.
//!
//! This module exposes only helpers; the actual call interception lives in
//! `executor::calls::function::try_evaluate_respond_to`, where `&mut Interpreter`
//! is in scope (a NativeFunction can't invoke a `Value::Function`).

use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, StrKey, Value};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    /// Per-worker stack of registration vecs. Pushed on `respond_to` entry,
    /// popped on exit so nested calls don't clobber each other.
    pub static RESPOND_TO_BUILDER: RefCell<Vec<Vec<(String, Value)>>> = const { RefCell::new(Vec::new()) };
}

/// All format tokens the DSL exposes as methods on the `format` object.
const FORMAT_TOKENS: &[&str] = &[
    "html", "json", "xml", "csv", "pdf", "excel", "htmx", "xhr", "text", "any",
];

/// Build the `format` hash passed to the user's `fn(format) { ... }` block.
/// Each entry is a NativeFunction that records its argument under the format name.
pub fn make_format_hash() -> Value {
    let mut pairs = HashPairs::default();
    for &token in FORMAT_TOKENS {
        let name = token.to_string();
        let recorder = NativeFunction::new(format!("format.{}", token), Some(1), move |args| {
            let handler = args
                .into_iter()
                .next()
                .ok_or_else(|| format!("format.{} expects 1 argument", name))?;
            if !matches!(
                handler,
                Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
            ) {
                return Err(format!(
                    "format.{} expects a function argument (got {})",
                    name,
                    handler.type_name()
                ));
            }
            RESPOND_TO_BUILDER.with(|stack| {
                if let Some(top) = stack.borrow_mut().last_mut() {
                    // Last-write-wins: replace any prior registration for this token.
                    top.retain(|(k, _)| k != &name);
                    top.push((name.clone(), handler));
                }
            });
            Ok(Value::Null)
        });
        pairs.insert(
            HashKey::String(token.to_string()),
            Value::NativeFunction(recorder),
        );
    }
    Value::Hash(Rc::new(RefCell::new(pairs)))
}

/// Parse an Accept header into ranked `(token, q)` pairs sorted by descending q.
///
/// Handles `text/html;q=0.5,application/json;q=0.9,*/*;q=0.1` style headers,
/// suffix forms like `application/vnd.api+json`, and unknown mime types
/// (silently dropped).
pub fn parse_accept_header(header: &str) -> Vec<(String, f32)> {
    let mut entries: Vec<(String, f32)> = Vec::new();
    for raw in header.split(',') {
        let segment = raw.trim();
        if segment.is_empty() {
            continue;
        }
        let mut parts = segment.split(';').map(str::trim);
        let media = match parts.next() {
            Some(m) if !m.is_empty() => m.to_ascii_lowercase(),
            _ => continue,
        };
        let mut q: f32 = 1.0;
        for param in parts {
            if let Some(rest) = param.strip_prefix("q=") {
                if let Ok(parsed) = rest.parse::<f32>() {
                    q = parsed.clamp(0.0, 1.0);
                }
            }
        }
        if let Some(token) = mime_to_token(&media) {
            entries.push((token, q));
        }
    }
    // Stable sort by descending q so first-source-wins on ties.
    entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    entries
}

fn mime_to_token(mime: &str) -> Option<String> {
    let m = mime;
    let token = match m {
        "text/html" | "application/xhtml+xml" => "html",
        "application/json" | "text/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "text/csv" => "csv",
        "application/pdf" => "pdf",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.ms-excel" => "excel",
        "text/plain" => "text",
        "*/*" => "*",
        _ => {
            // application/<anything>+json or +xml suffix forms
            if m.starts_with("application/") && m.ends_with("+json") {
                "json"
            } else if m.starts_with("application/") && m.ends_with("+xml") {
                "xml"
            } else {
                return None;
            }
        }
    };
    Some(token.to_string())
}

/// Map a URL path's trailing extension to a format token.
fn extension_to_token(path: &str) -> Option<String> {
    let last_segment = path.rsplit('/').next().unwrap_or("");
    let dot = last_segment.rfind('.')?;
    let ext = &last_segment[dot + 1..];
    if ext.is_empty() {
        return None;
    }
    let lower = ext.to_ascii_lowercase();
    Some(match lower.as_str() {
        "html" | "htm" => "html".to_string(),
        "json" => "json".to_string(),
        "xml" => "xml".to_string(),
        "csv" => "csv".to_string(),
        "pdf" => "pdf".to_string(),
        "xlsx" | "xls" => "excel".to_string(),
        "txt" => "text".to_string(),
        _ => return None,
    })
}

/// Read `req[outer][inner]` as a string. Returns None if any layer is missing
/// or not a hash/string.
fn nested_string(req: &Value, outer: &str, inner: &str) -> Option<String> {
    let Value::Hash(h) = req else { return None };
    let h = h.borrow();
    let inner_hash_rc = match h.get(&StrKey(outer))? {
        Value::Hash(inner_h) => inner_h.clone(),
        _ => return None,
    };
    drop(h);
    let inner_h = inner_hash_rc.borrow();
    match inner_h.get(&StrKey(inner))? {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Read `req[key]` as a string.
fn top_string(req: &Value, key: &str) -> Option<String> {
    let Value::Hash(h) = req else { return None };
    let h = h.borrow();
    match h.get(&StrKey(key))? {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Determine the request's preferred formats in priority order.
/// Order: HTMX header, XHR header, URL extension, ?format= query, Accept header.
pub fn detect_request_format(req: &Value) -> Vec<(String, f32)> {
    let mut detected: Vec<(String, f32)> = Vec::new();

    // HTMX takes precedence among classifiers.
    if matches!(
        nested_string(req, "headers", "hx-request").as_deref(),
        Some("true")
    ) {
        detected.push(("htmx".to_string(), 1.0));
    } else if matches!(
        nested_string(req, "headers", "x-requested-with").as_deref(),
        Some("XMLHttpRequest")
    ) {
        detected.push(("xhr".to_string(), 1.0));
    }

    if let Some(path) = top_string(req, "path") {
        if let Some(token) = extension_to_token(&path) {
            push_unique(&mut detected, token, 1.0);
        }
    }

    if let Some(fmt) = nested_string(req, "query", "format") {
        if !fmt.is_empty() {
            push_unique(&mut detected, fmt.to_ascii_lowercase(), 1.0);
        }
    }

    if let Some(accept) = nested_string(req, "headers", "accept") {
        for (tok, q) in parse_accept_header(&accept) {
            push_unique(&mut detected, tok, q);
        }
    }

    detected
}

fn push_unique(into: &mut Vec<(String, f32)>, token: String, q: f32) {
    if into.iter().any(|(t, _)| t == &token) {
        return;
    }
    into.push((token, q));
}

/// Pick the matching handler. Falls back to `any` when registered, else None.
pub fn pick_handler(
    detected: &[(String, f32)],
    registrations: &[(String, Value)],
) -> Option<Value> {
    for (token, _) in detected {
        if token == "*" {
            // Wildcard: first non-`any` registration.
            if let Some((_, fn_val)) = registrations.iter().find(|(k, _)| k != "any") {
                return Some(fn_val.clone());
            }
            continue;
        }
        if let Some((_, fn_val)) = registrations.iter().find(|(k, _)| k == token) {
            return Some(fn_val.clone());
        }
    }
    // No detection match → fall back to `any` if registered.
    if let Some((_, fn_val)) = registrations.iter().find(|(k, _)| k == "any") {
        return Some(fn_val.clone());
    }
    // Empty detection (e.g. no Accept header at all) and no `any`: serve the
    // first registered handler — Rails' "default to first" behavior.
    if detected.is_empty() {
        if let Some((_, fn_val)) = registrations.first() {
            return Some(fn_val.clone());
        }
    }
    None
}

/// Build the canonical 406 Not Acceptable response hash.
pub fn not_acceptable_response() -> Value {
    let mut headers = HashPairs::default();
    headers.insert(
        HashKey::String("Content-Type".to_string()),
        Value::String("text/plain; charset=utf-8".to_string()),
    );
    let mut body = HashPairs::default();
    body.insert(HashKey::String("status".to_string()), Value::Int(406));
    body.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(headers))),
    );
    body.insert(
        HashKey::String("body".to_string()),
        Value::String("Not Acceptable".to_string()),
    );
    Value::Hash(Rc::new(RefCell::new(body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req(headers: &[(&str, &str)], path: &str, query: &[(&str, &str)]) -> Value {
        let mut h = HashPairs::default();
        if !headers.is_empty() {
            let mut hm = HashPairs::default();
            for (k, v) in headers {
                hm.insert(
                    HashKey::String((*k).to_string()),
                    Value::String((*v).to_string()),
                );
            }
            h.insert(
                HashKey::String("headers".to_string()),
                Value::Hash(Rc::new(RefCell::new(hm))),
            );
        }
        h.insert(
            HashKey::String("path".to_string()),
            Value::String(path.to_string()),
        );
        if !query.is_empty() {
            let mut q = HashPairs::default();
            for (k, v) in query {
                q.insert(
                    HashKey::String((*k).to_string()),
                    Value::String((*v).to_string()),
                );
            }
            h.insert(
                HashKey::String("query".to_string()),
                Value::Hash(Rc::new(RefCell::new(q))),
            );
        }
        Value::Hash(Rc::new(RefCell::new(h)))
    }

    #[test]
    fn parses_simple_accept() {
        let got = parse_accept_header("text/html");
        assert_eq!(got, vec![("html".to_string(), 1.0)]);
    }

    #[test]
    fn parses_q_values_and_sorts_descending() {
        let got = parse_accept_header("text/html;q=0.5,application/json;q=0.9");
        assert_eq!(got[0].0, "json");
        assert_eq!(got[1].0, "html");
    }

    #[test]
    fn parses_wildcard_and_suffix_forms() {
        let got = parse_accept_header("application/vnd.api+json,*/*;q=0.1");
        assert_eq!(got[0].0, "json");
        assert_eq!(got.last().unwrap().0, "*");
    }

    #[test]
    fn drops_unknown_mimes() {
        let got = parse_accept_header("foo/bar,text/html");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "html");
    }

    #[test]
    fn extension_from_path() {
        assert_eq!(extension_to_token("/posts/1.json").as_deref(), Some("json"));
        assert_eq!(
            extension_to_token("/posts/1.xlsx").as_deref(),
            Some("excel")
        );
        assert_eq!(extension_to_token("/posts/1").as_deref(), None);
        assert_eq!(extension_to_token("/foo.bar/baz").as_deref(), None);
    }

    #[test]
    fn detects_htmx_first() {
        let req = make_req(
            &[("hx-request", "true"), ("accept", "text/html")],
            "/posts/1",
            &[],
        );
        let got = detect_request_format(&req);
        assert_eq!(got[0].0, "htmx");
    }

    #[test]
    fn detects_xhr_when_no_htmx() {
        let req = make_req(
            &[
                ("x-requested-with", "XMLHttpRequest"),
                ("accept", "text/html"),
            ],
            "/posts/1",
            &[],
        );
        let got = detect_request_format(&req);
        assert_eq!(got[0].0, "xhr");
    }

    #[test]
    fn extension_beats_accept() {
        let req = make_req(&[("accept", "text/html")], "/posts/1.json", &[]);
        let got = detect_request_format(&req);
        assert_eq!(got[0].0, "json");
    }

    #[test]
    fn query_format_present() {
        let req = make_req(&[("accept", "text/html")], "/posts/1", &[("format", "xml")]);
        let got = detect_request_format(&req);
        // extension absent → query takes the front (after htmx/xhr which are absent).
        assert_eq!(got[0].0, "xml");
    }

    #[test]
    fn pick_handler_exact_match() {
        let regs = vec![
            ("html".to_string(), Value::Null),
            ("json".to_string(), Value::Int(1)),
        ];
        let detected = vec![("json".to_string(), 1.0)];
        let picked = pick_handler(&detected, &regs).unwrap();
        assert!(matches!(picked, Value::Int(1)));
    }

    #[test]
    fn pick_handler_wildcard_picks_first() {
        let regs = vec![
            ("html".to_string(), Value::Int(7)),
            ("json".to_string(), Value::Int(1)),
        ];
        let detected = vec![("*".to_string(), 0.5)];
        let picked = pick_handler(&detected, &regs).unwrap();
        assert!(matches!(picked, Value::Int(7)));
    }

    #[test]
    fn pick_handler_falls_back_to_any() {
        let regs = vec![
            ("html".to_string(), Value::Int(1)),
            ("any".to_string(), Value::Int(99)),
        ];
        let detected = vec![("pdf".to_string(), 1.0)];
        let picked = pick_handler(&detected, &regs).unwrap();
        assert!(matches!(picked, Value::Int(99)));
    }

    #[test]
    fn pick_handler_no_match_returns_none() {
        let regs = vec![("html".to_string(), Value::Int(1))];
        let detected = vec![("pdf".to_string(), 1.0)];
        assert!(pick_handler(&detected, &regs).is_none());
    }

    #[test]
    fn pick_handler_empty_detection_picks_first() {
        let regs = vec![
            ("html".to_string(), Value::Int(1)),
            ("json".to_string(), Value::Int(2)),
        ];
        let picked = pick_handler(&[], &regs).unwrap();
        assert!(matches!(picked, Value::Int(1)));
    }

    #[test]
    fn not_acceptable_shape() {
        let resp = not_acceptable_response();
        let Value::Hash(h) = resp else {
            panic!("expected hash")
        };
        let h = h.borrow();
        assert!(matches!(
            h.get(&HashKey::String("status".to_string())),
            Some(Value::Int(406))
        ));
    }
}
