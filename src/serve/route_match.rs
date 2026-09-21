//! The route match, and the two ways it misses.
//!
//! Lifted out of `handle_request`. It is a function rather than a stage because
//! it is a lookup: a method and a path go in, and either a handler comes out or
//! the response that says nothing answers here.
//!
//! The two misses had drifted apart. A route miss wrote an access-log line and
//! called `finalize_session_cookie`; a wildcard miss — the same 404, one lookup
//! later — did neither. Under the ID session drivers that was invisible, because
//! `finalize_session_cookie` returns `None` when the id has not changed. Under
//! `SOLI_SESSION_DRIVER=cookie` it is not: the cookie driver re-emits whenever
//! the incoming blob was invalid or expired and got replaced, so a 404 from a
//! failed wildcard expansion dropped a replacement session cookie that a 404
//! from a route miss kept. There is one miss here now, and it does both.
//!
//! - [`resolve`] — the lookup and its two failures.
//! - [`Matched`] — what a hit hands to the rest of the pipeline.

use std::collections::HashMap;
use std::time::Instant;

use crate::interpreter::builtins::session::{finalize_session_cookie, set_current_session_id};
use crate::interpreter::Value;

use super::request_scope::Scope;
use super::{error_response, find_route, log_timestamp, RequestData, ResponseData};

/// A route that answered, after wildcard expansion.
pub(super) struct Matched {
    /// `controller#action`, with `#*` already resolved to the real action.
    pub(super) handler_name: String,
    /// Middleware declared on this route rather than globally.
    pub(super) scoped_middleware: Vec<Value>,
    /// The `:id`-style segments the pattern captured.
    pub(super) params: HashMap<String, String>,
}

/// Match `method` + `path` against the route table, or build the response that
/// says nothing here answers.
///
/// `log_start` is the request's start instant when access logging is on and
/// `None` when it is off — one option rather than two arguments, because the
/// caller only ever clocks the request in order to log it.
///
/// `cookie_secure` is computed once by the caller and passed in; this function
/// used to derive it a second time from the same headers.
// `data` is read only by the EUI browser fallback, which is compiled out with
// the feature.
#[cfg_attr(not(feature = "eui"), allow(unused_variables))]
pub(super) fn resolve(
    method: &str,
    path: &str,
    data: &RequestData,
    scope: &Scope,
    cookie_secure: bool,
    log_start: Option<Instant>,
) -> Result<Matched, ResponseData> {
    // Find matching route using indexed lookup (O(1) for exact matches, O(m) for patterns)
    let (route_handler_name, scoped_middleware, params) = match find_route(method, path) {
        Some(found) => found,
        None => {
            // Nothing in this application answers here. If it serves EUI and
            // the caller is a browser, that is not a missing page — it is
            // somebody who arrived over the wrong protocol, and a 404 teaches
            // them nothing. An application that *does* define a route for
            // this path never reaches this branch, so its own page always
            // wins; this is only what happens when nothing is defined.
            #[cfg(feature = "eui")]
            if let Some(resp) = eui_browser_fallback(method, data) {
                return Err(resp);
            }
            return Err(not_found(
                method,
                path,
                "The page you're looking for doesn't exist.",
                scope,
                cookie_secure,
                log_start,
            ));
        }
    };

    // Expand wildcard action pattern (e.g., "docs#*" → "docs#routing")
    // Skip expansion entirely when handler doesn't use wildcards (common case)
    let handler_name = if !route_handler_name.ends_with("#*") {
        route_handler_name
    } else {
        match crate::interpreter::builtins::server::expand_wildcard_action(
            &route_handler_name,
            &params,
        ) {
            Some(expanded) => expanded,
            None => {
                return Err(not_found(
                    method,
                    path,
                    "Action not found for this route.",
                    scope,
                    cookie_secure,
                    log_start,
                ))
            }
        }
    };

    Ok(Matched {
        handler_name,
        scoped_middleware,
        params,
    })
}

/// The 404 both misses return: the access line, the warn line, the production
/// error page, and the session cookie the response still owes the client.
fn not_found(
    method: &str,
    path: &str,
    message: &str,
    scope: &Scope,
    cookie_secure: bool,
    log_start: Option<Instant>,
) -> ResponseData {
    // Clear session context before returning
    set_current_session_id(None);
    // Log timing for 404 responses (skip health checks)
    if let Some(start) = log_start {
        if path != "/health" {
            println!(
                "{} [LOG] {} {} - 404 ({:.3}ms)",
                log_timestamp(),
                method,
                path,
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
    let mut resp = error_response::production(404, method, path, message, None);
    // Driver-aware: ID drivers re-emit only when the resolved ID
    // differs from the cookie's; the cookie driver re-emits when the
    // incoming blob was invalid/expired and got replaced. Uses the
    // explicit locals because the thread-local session ID was cleared
    // above.
    if let Some(cookie_value) = finalize_session_cookie(
        scope.session_id.as_deref(),
        scope.cookie_session_id.as_deref(),
        cookie_secure,
    ) {
        resp.headers.push(("Set-Cookie".to_string(), cookie_value));
    }
    resp
}

/// A browser that asked an EUI application for HTML gets the origin page, not a
/// 404 — the client completes it from the manifest.
///
/// The question is whether this is an EUI application at all, not which
/// component it would open.
#[cfg(feature = "eui")]
fn eui_browser_fallback(method: &str, data: &RequestData) -> Option<ResponseData> {
    use super::{eui, header_str, NO_INJECT_HEADER};

    if method != "GET" || !eui::snapshot::accepts_html(header_str(&data.headers, "accept")) {
        return None;
    }
    eui::default_component()?;
    set_current_session_id(None);
    let body = eui::snapshot::browser_body(
        header_str(&data.headers, "host"),
        header_str(&data.headers, "x-forwarded-host"),
        header_str(&data.headers, "x-forwarded-proto"),
    );
    Some(ResponseData {
        status: 200,
        headers: vec![
            (
                "Content-Type".to_string(),
                "text/html; charset=utf-8".to_string(),
            ),
            ("Cache-Control".to_string(), "no-store".to_string()),
            ("Vary".to_string(), "Accept".to_string()),
            // Compiled in, so there is nothing to hot-reload
            // and no reason to open a socket from the page
            // that exists to say a machine is doing too much.
            (NO_INJECT_HEADER.to_string(), "1".to_string()),
        ],
        body: body.into_bytes(),
    })
}
