//! CSRF protection: the SEC-014 Origin/Referer same-origin gate
//! (`check_csrf_origin`), per-form token verification (`verify_csrf_token`),
//! the Rails-style `_method` form override (`apply_form_method_override`), and
//! the app-registered `skip_csrf` exemption patterns. Extracted from the serve
//! god-module; the same-origin authority helpers live in `super::origin` and
//! the 403 response builder (`forbidden_csrf_response`) stays in `super`.

use std::borrow::Cow;

use hyper::header;

use crate::interpreter::builtins::server::parse_query_string;

use super::origin::origin_authority;
use super::{cors, header_str, websocket_request_authority, RequestData};

/// SEC-014: app-registered CSRF exemption patterns. Populated from Soli
/// code via `skip_csrf("/path[/*]")` (typically called from
/// `config/routes.sl` or a controller's `static` block before route
/// matching runs). Each entry is a path pattern; `*` suffix means "any
/// path that starts with this prefix".
///
/// `RwLock` so writes from the boot phase don't block the request hot
/// path's reads.
static CSRF_SKIP_PATTERNS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

pub fn register_csrf_skip_pattern(pattern: String) {
    if let Ok(mut guard) = CSRF_SKIP_PATTERNS.write() {
        if !guard.iter().any(|p| p == &pattern) {
            guard.push(pattern);
        }
    }
}

#[cfg(test)]
pub(crate) fn clear_csrf_skip_patterns() {
    if let Ok(mut guard) = CSRF_SKIP_PATTERNS.write() {
        guard.clear();
    }
}

fn csrf_skipped_by_app(path: &str) -> bool {
    let Ok(guard) = CSRF_SKIP_PATTERNS.read() else {
        return false;
    };
    guard.iter().any(|pattern| {
        if let Some(prefix) = pattern.strip_suffix("/*") {
            path == prefix || path.starts_with(&format!("{}/", prefix))
        } else if let Some(prefix) = pattern.strip_suffix('*') {
            path.starts_with(prefix)
        } else {
            path == pattern
        }
    })
}

/// `SOLI_DISABLE_CSRF` operator kill switch — turns off both the
/// Origin/Referer gate and per-form token verification.
fn csrf_disabled_by_env() -> bool {
    std::env::var("SOLI_DISABLE_CSRF")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// `SOLI_CSRF_TOKENS=require` strict mode: browser form posts
/// (urlencoded/multipart bodies) MUST carry a valid per-form token.
fn csrf_tokens_required() -> bool {
    std::env::var("SOLI_CSRF_TOKENS")
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("require"))
        .unwrap_or(false)
}

/// Does this content type mark a browser form submission?
fn is_form_content_type(content_type: &str) -> bool {
    content_type.starts_with("application/x-www-form-urlencoded")
        || content_type.starts_with("multipart/form-data")
}

/// Extract a single field from an `application/x-www-form-urlencoded` body
/// (percent-decoded via the shared query-string parser; form bodies are
/// small, so the full parse is cheap).
fn form_body_param(body: &str, name: &str) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    parse_query_string(body).remove(name)
}

/// Rails-style method override: HTML forms can only express GET/POST, so a
/// POST whose form body carries `_method` is treated as that verb. Only the
/// three verbs a form can't express are honored — anything else (including
/// an attempt to downgrade to GET and dodge CSRF checks) is ignored.
pub(crate) fn apply_form_method_override(
    method: Cow<'static, str>,
    body: &str,
    content_type: Option<&str>,
    multipart_form: Option<&[(String, String)]>,
) -> Cow<'static, str> {
    if method != "POST" {
        return method;
    }
    let requested = match content_type {
        Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => {
            form_body_param(body, "_method")
        }
        Some(ct) if ct.starts_with("multipart/form-data") => multipart_form
            .and_then(|form| form.iter().find(|(k, _)| k == "_method"))
            .map(|(_, v)| v.clone()),
        _ => None,
    };
    match requested.as_deref().map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("PUT") => Cow::Borrowed("PUT"),
        Some(v) if v.eq_ignore_ascii_case("PATCH") => Cow::Borrowed("PATCH"),
        Some(v) if v.eq_ignore_ascii_case("DELETE") => Cow::Borrowed("DELETE"),
        _ => method,
    }
}

/// Per-form CSRF token verification, run on the worker (where the session
/// lives) after the session ID is resolved. Complements the Origin/Referer
/// gate that already ran in the hyper layer:
///
/// - A request that **carries** a token (`_csrf_token` form field from
///   `csrf_field()` / `X-CSRF-Token` header from `csrf_meta_tag()`) must
///   present the session's token — a mismatch or a token-less session is a
///   403 even when Origin passed.
/// - A request with **no** token stays on the Origin/Referer posture,
///   unless `SOLI_CSRF_TOKENS=require` makes tokens mandatory for browser
///   form posts (JSON/API traffic is never token-gated; use `skip_csrf`
///   or the header for API clients that opt in).
pub(crate) fn verify_csrf_token(
    data: &RequestData,
    method: &str,
    path: &str,
) -> Result<(), String> {
    if matches!(method, "GET" | "HEAD" | "OPTIONS") {
        return Ok(());
    }
    if path.starts_with("/_") || csrf_skipped_by_app(path) || csrf_disabled_by_env() {
        return Ok(());
    }

    let content_type = header_str(&data.headers, "content-type").unwrap_or("");
    let supplied = header_str(&data.headers, "x-csrf-token")
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| {
            if content_type.starts_with("application/x-www-form-urlencoded") {
                form_body_param(&data.body, "_csrf_token")
            } else if content_type.starts_with("multipart/form-data") {
                data.multipart_form
                    .as_ref()
                    .and_then(|form| form.iter().find(|(k, _)| k == "_csrf_token"))
                    .map(|(_, v)| v.clone())
            } else {
                None
            }
        });

    match supplied {
        Some(token) => match crate::interpreter::builtins::session::current_csrf_token() {
            Some(expected)
                if crate::interpreter::builtins::crypto::do_secure_compare(&expected, &token) =>
            {
                Ok(())
            }
            Some(_) => Err("CSRF token does not match the session's".to_string()),
            None => Err(
                "CSRF token supplied but the session holds none (expired or new session)"
                    .to_string(),
            ),
        },
        None if csrf_tokens_required() && is_form_content_type(content_type) => {
            Err("missing CSRF token (SOLI_CSRF_TOKENS=require)".to_string())
        }
        None => Ok(()),
    }
}

/// SEC-014: reject state-changing browser requests that can't prove they
/// originate from the same site. Returns `Ok(())` to continue, `Err(reason)`
/// to reject with 403.
///
/// Rules:
/// - Safe methods (GET/HEAD/OPTIONS) are always allowed.
/// - Paths under `/_` are exempt (machine-to-machine endpoints like
///   `/_jobs/run/:name` carry their own HMAC auth).
/// - Paths matching a `skip_csrf("/pattern[/*]")` declaration in user
///   Soli code are exempt. This is the per-route opt-out — call it
///   from `config/routes.sl` or a controller's `static` block for
///   webhook endpoints, public APIs, etc.
/// - `SOLI_DISABLE_CSRF=true` operator-level kill switch — for API-only
///   deployments where no cookie session is in play.
/// - When `Origin` is present, it must equal the request authority
///   (`Host`/`X-Forwarded-Host`). `null` Origin (sandboxed iframe etc.)
///   is rejected.
/// - When `Origin` is absent but `Referer` is present, the Referer's
///   authority must match.
/// - When **neither** is present, the decision branches on the
///   `Cookie` header (SEC-078). Cookie-bearing requests get rejected
///   because they have no proof of same-site provenance — the threat
///   surface is exactly a stripped UA / proxy / Origin-less form POST
///   replaying the session cookie. Cookie-less requests stay on the
///   non-browser API path and are allowed; route-level opt-outs via
///   `skip_csrf("/path[/*]")` and the `SOLI_DISABLE_CSRF` operator
///   kill switch remain available for non-browser endpoints that
///   legitimately ride a cookie.
///
/// The intent matches `websocket_origin_allowed`'s authority semantics so
/// the two surfaces (HTTP + WebSocket) reject under the same rules.
pub(crate) fn check_csrf_origin(
    headers: &hyper::HeaderMap,
    method: &str,
    path: &str,
) -> Result<(), String> {
    if matches!(method, "GET" | "HEAD" | "OPTIONS") {
        return Ok(());
    }
    if path.starts_with("/_") {
        return Ok(());
    }
    if csrf_skipped_by_app(path) {
        return Ok(());
    }
    if csrf_disabled_by_env() {
        return Ok(());
    }
    // A `cors("/path", {...})` declaration allowing this Origin is an
    // explicit cross-origin opt-in for the path — more precise than
    // `skip_csrf`, since the origin is checked against the declared list.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if cors::allows_cross_origin(path, origin.trim()) {
            return Ok(());
        }
    }

    let request_authority = match websocket_request_authority(headers) {
        Some(a) => a,
        None => return Err("missing Host header".to_string()),
    };

    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let origin = origin.trim();
        // `null` Origin is what sandboxed iframes / data URLs send.
        // Treat it the same as a foreign origin.
        if origin.eq_ignore_ascii_case("null") {
            return Err("Origin is 'null'".to_string());
        }
        let Some(origin_auth) = origin_authority(origin) else {
            return Err(format!("malformed Origin header: {}", origin));
        };
        if origin_auth == request_authority {
            return Ok(());
        }
        return Err(format!(
            "Origin {} does not match request authority {}",
            origin_auth, request_authority
        ));
    }

    if let Some(referer) = headers.get(header::REFERER).and_then(|v| v.to_str().ok()) {
        let Some(referer_auth) = origin_authority(referer) else {
            return Err(format!("malformed Referer header: {}", referer));
        };
        if referer_auth == request_authority {
            return Ok(());
        }
        return Err(format!(
            "Referer {} does not match request authority {}",
            referer_auth, request_authority
        ));
    }

    // Neither Origin nor Referer. SEC-078: a cookie-bearing request in
    // this state has no proof of same-site provenance — modern browsers
    // do set Origin on cross-site state-changing requests, but stripped
    // user agents, transparent proxies, and Origin-less form posts still
    // happen, and the threat is precisely a session-cookie replay riding
    // such a request. Reject. Cookie-less requests stay on the non-
    // browser API path (curl, mobile clients) where there is no session
    // to ride. Mirrors the same Cookie-presence rule that
    // `websocket_origin_allowed` already enforces for WS upgrades.
    if headers.contains_key(header::COOKIE) {
        return Err("missing both Origin and Referer on cookie-bearing request".to_string());
    }
    Ok(())
}
