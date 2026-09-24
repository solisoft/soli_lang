//! What counts as a secret, in one place.
//!
//! Three separate rules for "this key holds a secret" grew up independently:
//! the request-param list in `serve::error_pages`, the header list beside it,
//! and `interpreter::value::is_sensitive_field_name` for model serialisation.
//! The first two guard the request snapshot in an error log; the third guards
//! model output. Nothing guarded the `env:` line of that same error log, which
//! is where a handler's **local variables** are written by value — so a local
//! called `api_key` was redacted as a request param and printed in full three
//! lines below.
//!
//! This module owns the param-style rule so the snapshot and the environment
//! dump cannot disagree. The header list stays where it is: header names are
//! matched exactly, not by substring, and that is a different rule rather than
//! a drifted copy of this one.

/// Substrings that signal a secret-bearing key. Matched case-insensitively
/// anywhere in the key, so `csrf_token`, `access_token` and `user_password` are
/// all caught along with the bare names.
///
/// Substring matching over-redacts a little — `author` contains `auth`,
/// `secretary` contains `secret` — and that is the intended direction. A
/// redacted local costs a debugging session some context; a logged credential
/// costs a rotation, and logs get shipped, retained and shared.
pub(crate) const SECRET_KEY_SUBSTRINGS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "privatekey",
    "authorization",
    "auth",
    "session_id",
    "sessionid",
    "csrf",
    // The `Cookie` header is the session credential in transit. It reached the
    // `env:` line through the `req`/`cookies` globals, whose own names say
    // nothing about secrets, so matching the key itself is what closes it.
    "cookie",
    "credential",
    "passphrase",
];

/// The marker written in place of a redacted value. One spelling, so a log
/// reader (or a grep for leaks) only has to know about one.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// Does this key look like it holds a secret?
///
/// Separators are ignored on both sides, so `api_key`, `api-key`, `apiKey`,
/// `x-api-key` and `X_API_KEY` are one name. The list is spelled with
/// underscores, but header names and many query strings use hyphens, and a
/// match that only knew one spelling let `X-Api-Key` into the error tracker's
/// stored locals and `?api-key=` into its replay line.
pub(crate) fn looks_sensitive(key: &str) -> bool {
    let folded = fold_separators(key);
    SECRET_KEY_SUBSTRINGS
        .iter()
        .any(|sub| folded.contains(fold_separators(sub).as_str()))
}

/// Lower-case, with `_`, `-`, `.` and spaces removed.
fn fold_separators(key: &str) -> String {
    key.chars()
        .filter(|c| !matches!(c, '_' | '-' | '.' | ' '))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Header names that carry a credential, matched exactly after lower-casing.
/// [`header_is_secret`] also applies [`looks_sensitive`], so a custom header
/// such as `X-Stripe-Api-Key` is caught without being listed; this list keeps
/// the well-known names explicit, whatever the substring rule becomes.
pub(crate) const SECRET_HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "x-auth-token",
    "x-csrf-token",
    "x-xsrf-token",
    "x-session-token",
    "x-coverage-token",
];

/// Does this header name carry a secret?
pub(crate) fn header_is_secret(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_HEADER_NAMES.iter().any(|n| *n == lower) || looks_sensitive(&lower)
}

/// Redact secret-bearing values in a URL's query string, keeping the rest
/// readable.
///
/// The `http` log channel prints the full outgoing URL of every `HTTP.*` call,
/// and a query string is a normal place to carry a credential —
/// `?api_key=…`, `?access_token=…`. Only the *values* of sensitive-looking
/// parameters are replaced, so the endpoint and its ordinary parameters stay
/// legible, which is the whole point of having the channel.
///
/// Deliberately simple string surgery rather than a URL parse: this runs on a
/// logging path, must not allocate a parser per call, and must never fail on a
/// malformed URL — an unparseable URL is logged unchanged rather than dropped.
pub(crate) fn redact_url_query(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let mut out = String::with_capacity(url.len());
    out.push_str(base);
    out.push('?');
    for (i, pair) in query.split('&').enumerate() {
        if i > 0 {
            out.push('&');
        }
        match pair.split_once('=') {
            Some((key, _)) if looks_sensitive(key) => {
                out.push_str(key);
                out.push('=');
                out.push_str(REDACTED);
            }
            _ => out.push_str(pair),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::looks_sensitive;

    #[test]
    fn catches_the_names_that_carry_credentials() {
        for key in [
            "password",
            "user_password",
            "passwd",
            "api_key",
            "apiKey",
            "private_key",
            "access_token",
            "csrf_token",
            "session_id",
            "Authorization",
            "session_secret",
            "SECRET",
            // Hyphenated and dotted spellings are the same names.
            "api-key",
            "x-api-key",
            "X-Api-Key",
            "private-key",
            "session-id",
            "session.id",
        ] {
            assert!(looks_sensitive(key), "{key} must be redacted");
        }
    }

    #[test]
    fn leaves_ordinary_names_alone() {
        for key in ["user_name", "email", "id", "count", "created_at", "title"] {
            assert!(!looks_sensitive(key), "{key} must not be redacted");
        }
    }

    #[test]
    fn url_query_secrets_are_redacted_and_the_rest_survives() {
        use super::redact_url_query;
        let got =
            redact_url_query("https://api.example.com/v1/items?page=2&api_key=ak_live_1&sort=name");
        assert!(!got.contains("ak_live_1"), "api key leaked: {got}");
        assert!(
            got.contains("page=2") && got.contains("sort=name"),
            "lost context: {got}"
        );
        assert!(
            got.contains("api_key=[REDACTED]"),
            "expected the marker: {got}"
        );
        // No query string, or a malformed one: returned unchanged, never dropped.
        assert_eq!(redact_url_query("https://x/y"), "https://x/y");
        assert_eq!(redact_url_query("https://x/y?flag"), "https://x/y?flag");
        let hyphen = redact_url_query("https://x/y?api-key=ak_live_2&page=1");
        assert!(
            !hyphen.contains("ak_live_2"),
            "hyphenated key leaked: {hyphen}"
        );
    }

    #[test]
    fn secret_headers_are_caught_by_list_and_by_substring() {
        use super::header_is_secret;
        for name in [
            "Authorization",
            "Cookie",
            "X-Api-Key",
            "Api-Key",
            "X-Stripe-Api-Key",
            "X-Auth-Token",
        ] {
            assert!(header_is_secret(name), "{name} must be redacted");
        }
        for name in [
            "Accept",
            "Content-Type",
            "Host",
            "User-Agent",
            "X-Request-Id",
        ] {
            assert!(!header_is_secret(name), "{name} must pass");
        }
    }

    /// Documented over-redaction: substring matching catches these, and that
    /// is the deliberate direction. Pinned so the trade stays a decision
    /// rather than a surprise.
    #[test]
    fn over_redacts_a_few_innocent_names_on_purpose() {
        for key in ["author", "secretary", "authenticated_at"] {
            assert!(
                looks_sensitive(key),
                "{key} is expected to be caught by substring matching"
            );
        }
    }
}

/// Depth at which the debug redactor stops descending.
///
/// The environment dump runs on the error path, and a runtime value graph can
/// be cyclic (`parent` ↔ `children` is ordinary application code) or simply
/// very deep. Recursing without a bound would overflow the native stack, which
/// aborts the entire process — every worker, every tenant — instead of
/// producing the 500 the handler was already on its way to returning.
const MAX_REDACT_DEPTH: usize = 32;

/// Copy a runtime value with every secret-bearing hash key replaced by
/// [`REDACTED`], at any depth.
///
/// Redacting only top-level *variable names* left the biggest leak wide open:
/// `req`, `params` and `cookies` are globals in scope for every handler, none
/// of those three names looks sensitive, and their contents are exactly the
/// session cookie, the `Authorization` header and the submitted password.
pub(crate) fn redact_value_for_debug(
    value: &crate::interpreter::value::Value,
) -> crate::interpreter::value::Value {
    redact_value(value, 0)
}

fn redact_value(
    value: &crate::interpreter::value::Value,
    depth: usize,
) -> crate::interpreter::value::Value {
    use crate::interpreter::value::{HashKey, HashPairs, Value};
    use std::cell::RefCell;
    use std::rc::Rc;

    if depth >= MAX_REDACT_DEPTH {
        return Value::String("<max depth>".into());
    }

    match value {
        Value::Hash(pairs) => {
            // `try_borrow`: the dump can run while a hash is mutably borrowed
            // further up the stack (that is often *why* the handler raised),
            // and panicking inside the error reporter would lose the error.
            let Ok(borrowed) = pairs.try_borrow() else {
                return Value::String("<borrowed>".into());
            };
            // The `req` global carries the raw request body next to its parsed
            // forms. The parsed fields are redacted key by key below, but the
            // raw string (`password=hunter2&…`) has no key to match, so it went
            // out in full. Recognise the request by its shape and drop the raw
            // body wholesale, as the request snapshot already does.
            let is_request = ["method", "path", "headers"]
                .iter()
                .all(|k| borrowed.contains_key(&HashKey::String((*k).into())));
            let mut out = HashPairs::default();
            for (key, val) in borrowed.iter() {
                let key_text = match key {
                    HashKey::String(s) => s.to_string(),
                    other => other.to_value().to_string(),
                };
                let raw_body = is_request && matches!(key_text.as_str(), "body" | "raw_body");
                let redacted = if raw_body || looks_sensitive(&key_text) {
                    Value::String(REDACTED.into())
                } else if is_request && key_text == "headers" {
                    // A request's headers get the header rule as well: the
                    // exact list, not only the key-name substrings.
                    redact_headers(val, depth + 1)
                } else {
                    redact_value(val, depth + 1)
                };
                out.insert(key.clone(), redacted);
            }
            Value::Hash(Rc::new(RefCell::new(out)))
        }
        Value::Array(items) => {
            let Ok(borrowed) = items.try_borrow() else {
                return Value::String("<borrowed>".into());
            };
            let out: Vec<Value> = borrowed
                .iter()
                .map(|item| redact_value(item, depth + 1))
                .collect();
            Value::Array(Rc::new(RefCell::new(out)))
        }
        // Instances serialise through `is_sensitive_field_name` already, and
        // everything else is a scalar with no nested keys to walk.
        other => other.clone(),
    }
}

/// A request hash's `headers`: every secret header's value replaced, the rest
/// walked as usual.
fn redact_headers(
    value: &crate::interpreter::value::Value,
    depth: usize,
) -> crate::interpreter::value::Value {
    use crate::interpreter::value::{HashKey, HashPairs, Value};
    use std::cell::RefCell;
    use std::rc::Rc;

    let Value::Hash(pairs) = value else {
        return redact_value(value, depth);
    };
    let Ok(borrowed) = pairs.try_borrow() else {
        return Value::String("<borrowed>".into());
    };
    let mut out = HashPairs::default();
    for (key, val) in borrowed.iter() {
        let secret = match key {
            HashKey::String(s) => header_is_secret(s),
            other => header_is_secret(&other.to_value().to_string()),
        };
        let redacted = if secret {
            Value::String(REDACTED.into())
        } else {
            redact_value(val, depth + 1)
        };
        out.insert(key.clone(), redacted);
    }
    Value::Hash(Rc::new(RefCell::new(out)))
}

#[cfg(test)]
mod redact_value_tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs, Value};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut out = HashPairs::default();
        for (k, v) in pairs {
            out.insert(HashKey::String((*k).into()), v);
        }
        Value::Hash(Rc::new(RefCell::new(out)))
    }

    fn dump(value: &Value) -> String {
        crate::interpreter::value::stringify_to_string(&redact_value_for_debug(value)).unwrap()
    }

    #[test]
    fn nested_secrets_are_redacted_not_just_top_level_names() {
        // The shape of the `req` global that used to be dumped verbatim.
        let request = hash(vec![
            (
                "headers",
                hash(vec![
                    ("cookie", Value::String("session_id=abc123".into())),
                    ("authorization", Value::String("Bearer live-token".into())),
                    ("accept", Value::String("text/html".into())),
                ]),
            ),
            (
                "params",
                hash(vec![
                    ("password", Value::String("hunter2".into())),
                    ("email", Value::String("user@example.com".into())),
                ]),
            ),
        ]);

        let json = dump(&request);
        assert!(!json.contains("abc123"), "session cookie leaked: {json}");
        assert!(!json.contains("live-token"), "bearer token leaked: {json}");
        assert!(!json.contains("hunter2"), "password leaked: {json}");
        // Non-secret context must survive, or the dump stops being useful.
        assert!(json.contains("text/html"), "{json}");
        assert!(json.contains("user@example.com"), "{json}");
        assert!(json.contains(REDACTED), "{json}");
    }

    /// The parsed form is redacted field by field; the raw body beside it has
    /// no field names, so a request-shaped hash drops it whole.
    #[test]
    fn the_raw_request_body_is_redacted() {
        let request = hash(vec![
            ("method", Value::String("POST".into())),
            ("path", Value::String("/checkout".into())),
            ("headers", hash(vec![])),
            ("body", Value::String("card=4242&password=hunter2".into())),
        ]);
        let json = dump(&request);
        assert!(!json.contains("hunter2"), "raw body leaked: {json}");
        assert!(json.contains("/checkout"), "{json}");

        // A `body` that is not a request's (a blog post's) is left alone.
        let post = hash(vec![("body", Value::String("Hello world".into()))]);
        assert!(dump(&post).contains("Hello world"));
    }

    /// `X-Api-Key` on the `req` global went into the tracker's stored locals
    /// and its replay `curl` line: the substring list only knew `api_key`.
    #[test]
    fn a_requests_api_key_header_and_hyphenated_params_are_redacted() {
        let request = hash(vec![
            ("method", Value::String("GET".into())),
            ("path", Value::String("/v1/items".into())),
            (
                "headers",
                hash(vec![
                    ("x-api-key", Value::String("ak_live_header".into())),
                    ("x-stripe-api-key", Value::String("sk_live_custom".into())),
                    ("accept", Value::String("application/json".into())),
                ]),
            ),
            (
                "query",
                hash(vec![("api-key", Value::String("ak_live_query".into()))]),
            ),
        ]);
        let json = dump(&request);
        for leak in ["ak_live_header", "sk_live_custom", "ak_live_query"] {
            assert!(!json.contains(leak), "{leak} leaked: {json}");
        }
        assert!(json.contains("application/json"), "{json}");
    }

    #[test]
    fn secrets_inside_arrays_are_redacted() {
        let value = hash(vec![(
            "sessions",
            Value::Array(Rc::new(RefCell::new(vec![hash(vec![(
                "csrf_token",
                Value::String("tok-1".into()),
            )])]))),
        )]);
        assert!(!dump(&value).contains("tok-1"));
    }

    #[test]
    fn a_cyclic_value_terminates_instead_of_overflowing_the_stack() {
        // A stack overflow here is not a caught panic: it aborts the process.
        let node = hash(vec![("name", Value::String("root".into()))]);
        if let Value::Hash(pairs) = &node {
            pairs
                .borrow_mut()
                .insert(HashKey::String("self".into()), node.clone());
        }
        let json = dump(&node);
        assert!(json.contains("max depth"), "{json}");
    }
}
