//! The request the handler sees: body, scheme, host, headers, params, cookies.
//!
//! Lifted out of `handle_request`. It is a function rather than a stage because
//! it is a conversion — hyper's `HeaderMap`, the raw body and the matched params
//! in, the Soli hash a controller reads out — and because it is the last thing
//! that needs the request mutably: it takes `headers` and `query` out of
//! [`RequestData`] rather than cloning them, and everything downstream borrows.
//!
//! Three security rules live here and travel with their comments: SEC-044's
//! leftmost-token rule for an appending proxy's `X-Forwarded-Host`, the
//! `SOLI_APP_HOSTS` check that stops a client-supplied `Host` from ending up in
//! a password-reset link, and SEC-028's `Secure` cookie flag.
//!
//! - [`scheme`] — TLS and the `Secure` cookie flag, answered before routing
//!   because the 404 needs them too.
//! - [`build`] — the conversion, and what it hands back.
//! - [`Input`] — the two values the later stages read from it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::{HashKey, HashPairs};
use crate::interpreter::{Interpreter, Value};

use super::{
    build_request_hash_with_parsed, dev_bar, first_forwarded_token, header_str, parse_request_body,
    ParsedBody, RequestData,
};

/// What [`build`] produced, for the stages that run after it.
pub(super) struct Input {
    /// `req` — the whole request as a Soli hash.
    pub(super) request_hash: Value,
    /// Whether this is an HTMx partial swap. Captured here because the header
    /// it is read from is taken out of `data` a few lines later, and the dev
    /// bar needs the answer at the very end of the request.
    pub(super) is_htmx: bool,
}

/// How this request arrived, as far as the server can tell.
///
/// Computed before routing rather than with the rest of the request, because a
/// 404 still has to decide whether the session cookie it emits is `Secure` —
/// which is how the two copies of this block came to exist in the first place.
pub(super) struct Scheme {
    /// Whether `X-Forwarded-*` may be believed at all.
    trust_proxy: bool,
    /// Whether the client's hop to the edge was TLS.
    is_https: bool,
    /// Whether a `Set-Cookie` on this response gets the `Secure` flag.
    pub(super) cookie_secure: bool,
}

/// Parse the body, resolve scheme and host, and publish `params` / `cookies`
/// into both engines.
pub(super) fn build(
    interpreter: &mut Interpreter,
    vm: &mut Option<crate::vm::Vm>,
    data: &mut RequestData,
    params: HashMap<String, String>,
    cookie_pairs: HashPairs,
    scheme: &Scheme,
) -> Input {
    // Skip body parsing for GET/HEAD requests (no body to parse)
    let parsed_body = if data.method == "GET" || data.method == "HEAD" {
        ParsedBody::default()
    } else {
        let content_type = header_str(&data.headers, "content-type");
        parse_request_body(
            &data.body,
            content_type,
            data.multipart_form.as_deref(),
            data.multipart_files.as_ref(),
        )
    };

    publish_request_host(data, scheme);

    // Capture whether this is an HTMx partial-swap request before `headers`
    // is moved into `RequestData`. HTMx returns the response fragment into
    // the live DOM, where the page-level dev bar already exists — injecting
    // a second one into the fragment produces stacked bars.
    let is_htmx = dev_bar::is_htmx_request(header_str(&data.headers, "hx-request"));

    // Take ownership of headers and query to avoid cloning individual keys/values.
    // This is the ONE place the wire headers become owned Strings: straight
    // from hyper's HeaderMap into the Soli HashPairs handlers see as
    // req["headers"] (non-UTF-8 values are skipped, as before).
    let wire_headers = std::mem::take(&mut data.headers);
    let mut headers =
        HashPairs::with_capacity_and_hasher(wire_headers.keys_len(), ahash::RandomState::default());
    for (name, value) in &wire_headers {
        if let Ok(v) = value.to_str() {
            headers.insert(
                HashKey::String(name.as_str().into()),
                Value::String(v.into()),
            );
        }
    }
    let query = std::mem::take(&mut data.query);

    // The single cookie parse from above becomes `req["cookies"]`; keep an
    // Rc handle so the `cookies` global below reuses it without re-probing
    // the request hash.
    let cookies_value = Value::Hash(Rc::new(RefCell::new(cookie_pairs)));

    // Build request hash with parsed body (owned headers/query avoid String
    // clones). Also hands back the "all" params value so the `params` global
    // below doesn't re-probe the hash by string key.
    let (request_hash, all_params) = build_request_hash_with_parsed(
        &data.method,
        &data.path,
        params,
        query,
        headers,
        cookies_value.clone(),
        &data.body,
        parsed_body,
        &data.peer_ip,
    );

    // Expose params and cookies as globals so middleware, handlers, and views
    // can reference them directly. Both values are already in hand (returned
    // by build_request_hash_with_parsed / created above) — no string-key
    // re-probe of the request hash. They are also re-set inside
    // dispatch_request after middleware may have modified the request hash.
    let middleware_params =
        all_params.unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))));
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("params", middleware_params.clone());
    crate::interpreter::taint::mark_request_value(&middleware_params);
    if let Some(vm_ref) = vm.as_mut() {
        vm_ref
            .globals
            .insert("params".to_string(), middleware_params);
    }
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("cookies", cookies_value.clone());
    crate::interpreter::taint::mark_request_value(&cookies_value);
    if let Some(vm_ref) = vm.as_mut() {
        vm_ref.globals.insert("cookies".to_string(), cookies_value);
    }

    Input {
        request_hash,
        is_htmx,
    }
}

/// Resolve the scheme this request arrived on and what the `Secure` cookie flag
/// follows from it.
///
/// Reads the headers, so it must run before [`build`] takes them.
pub(super) fn scheme(data: &RequestData) -> Scheme {
    // Read scheme + host out of the headers BEFORE `std::mem::take` strips
    // them. `is_https` is also used further down (session cookie Secure
    // flag) — keep it computed here rather than re-reading the now-empty
    // headers map. `X-Forwarded-*` are honored only when
    // `enable_trust_proxy()` has been opted into; otherwise an attacker on
    // a directly-exposed deploy could spoof the scheme/host used to set the
    // session-cookie `Secure` flag and to build absolute URL helpers.
    let trust_proxy = crate::interpreter::builtins::trust_proxy::is_trust_proxy_enabled();
    let is_https = if trust_proxy {
        header_str(&data.headers, "x-forwarded-proto")
            .map(|v| first_forwarded_token(v) == "https")
            .unwrap_or(false)
    } else {
        false
    };
    // SEC-028: cookie Secure flag uses `is_https || force_secure_cookies()`
    // so a TLS deployment without `enable_trust_proxy()` (or without an
    // X-Forwarded-Proto: https header) still emits Secure cookies once the
    // operator opts in.
    let cookie_secure =
        is_https || crate::interpreter::builtins::secure_cookies::is_force_secure_cookies_enabled();
    Scheme {
        trust_proxy,
        is_https,
        cookie_secure,
    }
}

/// Publish scheme + host for the named-route URL helpers.
///
/// Runs on a matched request only: a 404 never built a URL, and making it warn
/// about `SOLI_APP_HOSTS` would be new noise on a path that has never had it.
/// Must run before `headers` is taken — it is the last reader of the wire map.
fn publish_request_host(data: &RequestData, scheme: &Scheme) {
    // The host falls back to the `Host` header per RFC 7230 when no proxy
    // header is present; empty string is fine — `*_url` will reject it with a
    // clear error.
    //
    // Skip the whole block when no named routes exist so a JSON API does not
    // allocate host strings per request.
    if !crate::interpreter::builtins::named_routes::any_named_routes() {
        return;
    }
    // SEC-044: take only the first comma-separated entry from
    // X-Forwarded-Host. A nginx-style appending proxy sends
    // "real, attacker" when a client supplied an XFH already; the
    // leftmost token is the value the trusted proxy wrote, so use
    // that for cookie / *_url decisions instead of the verbatim
    // string.
    let req_host = if scheme.trust_proxy {
        header_str(&data.headers, "x-forwarded-host")
            .map(|v| first_forwarded_token(v).to_string())
            .or_else(|| header_str(&data.headers, "host").map(|v| v.to_string()))
            .unwrap_or_default()
    } else {
        header_str(&data.headers, "host")
            .map(|v| v.to_string())
            .unwrap_or_default()
    };
    // Validate the host before any absolute URL is built from it.
    //
    // `Host` is client-controlled on every HTTP/1.1 request, and `*_url`
    // helpers interpolated it verbatim — so a password-reset mailer sent the
    // victim a link pointing at whatever host the attacker's request
    // carried, with a live token in the query string. Production boot
    // already refuses to start without `SOLI_APP_HOSTS`; this makes the
    // rest of the app honour it. Nothing declared (a dev server) keeps the
    // old behaviour.
    let req_host = if crate::serve::csrf::is_declared_host(&req_host) {
        req_host
    } else {
        let fallback = crate::serve::csrf::primary_declared_host().unwrap_or_default();
        match undeclared_host_warning(&req_host) {
            HostWarning::First => eprintln!(
                "[WARN] request Host {req_host:?} is not in SOLI_APP_HOSTS; \
                 building URLs with {fallback:?} instead"
            ),
            HostWarning::CapReached => eprintln!(
                "[WARN] request Host {req_host:?} is not in SOLI_APP_HOSTS; \
                 building URLs with {fallback:?} instead (further undeclared hosts \
                 are no longer logged)"
            ),
            HostWarning::Quiet => {}
        }
        fallback
    };
    let req_scheme = if scheme.is_https { "https" } else { "http" }.to_string();
    crate::interpreter::builtins::named_routes::set_current_request_host(req_scheme, req_host);
}

/// How many distinct undeclared hosts get their own warning line.
const UNDECLARED_HOST_LOG_CAP: usize = 32;

#[derive(Debug, PartialEq, Eq)]
enum HostWarning {
    /// First sighting of this host: log it.
    First,
    /// The host that fills the cap: log it and say the rest are suppressed.
    CapReached,
    /// Already logged, or past the cap.
    Quiet,
}

/// Should a request carrying this undeclared `Host` write a warning?
///
/// It used to be written on *every* such request — and `Host` is
/// client-chosen, so a scanner (or one misconfigured health check) turned
/// stderr into a firehose. Each distinct host is reported once, up to
/// [`UNDECLARED_HOST_LOG_CAP`] of them; after that nothing, since an attacker
/// can mint hosts without limit.
fn undeclared_host_warning(host: &str) -> HostWarning {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
        std::sync::Mutex::new(None);
    let mut guard = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = guard.get_or_insert_with(std::collections::HashSet::new);
    classify_undeclared_host(seen, host)
}

fn classify_undeclared_host(
    seen: &mut std::collections::HashSet<String>,
    host: &str,
) -> HostWarning {
    // Bound what an attacker-chosen value can make us store.
    let key: String = host.chars().take(255).collect();
    if seen.len() >= UNDECLARED_HOST_LOG_CAP || seen.contains(&key) {
        return HostWarning::Quiet;
    }
    seen.insert(key);
    if seen.len() == UNDECLARED_HOST_LOG_CAP {
        HostWarning::CapReached
    } else {
        HostWarning::First
    }
}

#[cfg(test)]
mod undeclared_host_warning_tests {
    use super::*;

    #[test]
    fn each_host_is_reported_once_and_the_total_is_capped() {
        let mut seen = std::collections::HashSet::new();
        assert_eq!(
            classify_undeclared_host(&mut seen, "evil.test"),
            HostWarning::First
        );
        assert_eq!(
            classify_undeclared_host(&mut seen, "evil.test"),
            HostWarning::Quiet
        );
        for i in 1..UNDECLARED_HOST_LOG_CAP - 1 {
            assert_eq!(
                classify_undeclared_host(&mut seen, &format!("h{i}.test")),
                HostWarning::First
            );
        }
        assert_eq!(
            classify_undeclared_host(&mut seen, "last.test"),
            HostWarning::CapReached
        );
        assert_eq!(
            classify_undeclared_host(&mut seen, "more.test"),
            HostWarning::Quiet
        );
        assert_eq!(seen.len(), UNDECLARED_HOST_LOG_CAP);
    }
}
