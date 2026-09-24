//! CSRF protection: the SEC-014 Origin/Referer same-origin gate
//! (`check_csrf_origin`), per-form token verification (`verify_csrf_token`),
//! the Rails-style `_method` form override (`apply_form_method_override`), and
//! the app-registered `skip_csrf` exemption patterns. Extracted from the serve
//! god-module; the same-origin authority helpers live in `super::origin` and
//! the 403 response builder (`forbidden_csrf_response`) stays in `super`.

use std::borrow::Cow;
use std::sync::OnceLock;

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
/// Per application: `skip_csrf("/webhooks/*")` in one app must not disable the
/// CSRF barrier on another's `/webhooks/*`.
static CSRF_SKIP_PATTERNS: crate::serve::tenant::TenantValue<Vec<String>> =
    crate::serve::tenant::TenantValue::new(Vec::new);

pub fn register_csrf_skip_pattern(pattern: String) {
    CSRF_SKIP_PATTERNS.write(|patterns| {
        if !patterns.iter().any(|p| p == &pattern) {
            patterns.push(pattern);
        }
    });
}

#[cfg(test)]
pub(crate) fn clear_csrf_skip_patterns() {
    CSRF_SKIP_PATTERNS.write(|patterns| patterns.clear());
}

/// Match a request path against one `skip_csrf` pattern.
///
/// `prefix/*` matches the prefix itself and any child (`prefix/...`).
/// A trailing bare `*` is a pure prefix match. Otherwise the path must
/// equal the pattern exactly. Allocation-free on the request hot path.
fn path_matches_csrf_skip(path: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        path.starts_with(prefix)
    } else {
        path == pattern
    }
}

fn csrf_skipped_by_app(path: &str) -> bool {
    CSRF_SKIP_PATTERNS.read(|patterns| {
        patterns
            .iter()
            .any(|pattern| path_matches_csrf_skip(path, pattern))
    })
}

/// Framework-served endpoints that are exempt from both CSRF barriers.
///
/// This used to be a bare `path.starts_with("/_")`, which is far wider than
/// the set of paths the framework actually answers: a perfectly ordinary
/// application route such as `POST /_internal/wipe` inherited the exemption
/// and lost both the Origin gate and token verification. Single-underscore
/// paths are a plausible app namespace and none of them is exempt; the `/__…`
/// namespace is genuinely reserved (see [`is_reserved_framework_path`]) and
/// keeps prefix matching so new internal endpoints don't have to be
/// enumerated here one by one.
///
/// An app that *wants* an exemption for its own route says so explicitly
/// with `skip_csrf("/path[/*]")`.
fn is_framework_path(path: &str) -> bool {
    // `/__soli/jobs` and `/__soli/errors` are deliberately NOT exempt. They are
    // the endpoints in the reserved namespace that are both state-changing and
    // reachable in production: `POST /__soli/jobs/<id>/retry` (and `/cancel`),
    // `POST /__soli/errors/<id>/resolve` (and the rest) behind Basic auth,
    // which a browser attaches automatically — so a blanket exemption made them
    // cross-site-forgeable against a logged-in operator. Their own forms are
    // same-origin, so the Origin/Referer gate passes for them.
    if is_operator_dashboard_path(path) {
        return false;
    }
    // `/_health`, `/_ready` and `/_metrics` used to be listed here too. The
    // probes answer GET/HEAD only, which neither barrier checks, so the entry
    // only ever exempted a *POST* to those paths — which can only reach an
    // application route such as `post("/:slug")`.
    is_reserved_framework_path(path)
}

/// The reserved `/__…` namespace the framework owns.
///
/// Exempt from both CSRF barriers (via [`is_framework_path`]), which is only
/// sound if application code never sees these paths: `handle_hyper_request`
/// answers 404 for anything in here that no framework handler claimed. Without
/// that, production — where none of the dev endpoints exist — fell through to
/// app routing and `POST /__soli/account/delete` matched
/// `post("/:locale/account/delete")` with no CSRF check at all.
///
/// `/__livereload` is matched exactly (plus `/__livereload/…` and the
/// `/__livereload_ws` socket), not as a bare prefix that also swallowed
/// `/__livereloadanything`.
pub(crate) fn is_reserved_framework_path(path: &str) -> bool {
    matches!(path, "/__coverage__" | "/__livereload" | "/__livereload_ws")
        || path.starts_with("/__soli/")
        || path.starts_with("/__solidev/")
        || path.starts_with("/__dev/")
        || path.starts_with("/__livereload/")
}

/// The built-in operator pages (jobs, errors), which the Origin/Referer gate
/// covers (see [`is_framework_path`]) but the per-form token layer cannot.
///
/// Their action forms are rendered by the framework itself, behind Basic
/// auth rather than a cookie session — so there is no session CSRF token to
/// embed in them, and `SOLI_CSRF_TOKENS=require` would 403 every one of the
/// framework's own buttons. Same-origin enforcement stays; only the mandatory
/// *token* is lifted.
fn is_operator_dashboard_path(path: &str) -> bool {
    ["/__soli/jobs", "/__soli/errors"]
        .iter()
        .any(|base| path == *base || path.strip_prefix(base).is_some_and(|r| r.starts_with('/')))
}

/// Public hostnames this app is served under, from `SOLI_APP_HOSTS`.
///
/// The safe way to run behind a reverse proxy, and the reason
/// `enable_trust_proxy()` should not be reached for to solve a CSRF rejection.
///
/// The proxy forwards to `localhost:9002`, so the browser's `Origin`
/// (`app.example.com`) can never equal the request authority the app sees. The
/// tempting fix is to trust `X-Forwarded-Host` — but apps bind `0.0.0.0`, so
/// anyone who can reach the port directly then sends
/// `X-Forwarded-Host: evil.test` alongside `Origin: https://evil.test` and the
/// same-origin check passes for every state-changing request. That is a
/// complete CSRF bypass, gated on nothing the attacker does not control.
///
/// An allowlist has no such property: the value comes from the deployment, not
/// from the request. Same idea as Django's `ALLOWED_HOSTS` and Rails'
/// `config.hosts`.
///
/// Comma-separated, host or host:port, compared case-insensitively:
///
/// ```text
/// SOLI_APP_HOSTS=app.example.com,www.app.example.com
/// ```
pub(super) fn origin_matches_declared_host(origin_auth: &str) -> bool {
    let Some(raw) = app_hosts_env() else {
        return false;
    };
    host_matches_declared(&raw, origin_auth)
}

// The CSRF settings are read on every state-changing request (and
// `SOLI_APP_HOSTS` on every URL build), and none of them changes after boot:
// `.env` is loaded once, before the listener opens. Each `std::env::var` takes
// the process environment lock and allocates, so they are read once and kept.
// Unit tests flip these variables at run time, so under `cfg(test)` every read
// goes to the environment as before.
static APP_HOSTS_ENV: OnceLock<Option<String>> = OnceLock::new();
static DISABLE_CSRF_ENV: OnceLock<Option<String>> = OnceLock::new();
static CSRF_TOKENS_ENV: OnceLock<Option<String>> = OnceLock::new();

#[cfg(not(test))]
fn cached_env_var(
    slot: &'static OnceLock<Option<String>>,
    name: &str,
) -> Option<Cow<'static, str>> {
    slot.get_or_init(|| std::env::var(name).ok())
        .as_deref()
        .map(Cow::Borrowed)
}

#[cfg(test)]
fn cached_env_var(
    _slot: &'static OnceLock<Option<String>>,
    name: &str,
) -> Option<Cow<'static, str>> {
    std::env::var(name).ok().map(Cow::Owned)
}

fn app_hosts_env() -> Option<Cow<'static, str>> {
    cached_env_var(&APP_HOSTS_ENV, "SOLI_APP_HOSTS")
}

/// The first host in `SOLI_APP_HOSTS`, when it is set and non-empty.
///
/// Production boot refuses to start without that variable, but only the CSRF
/// gate ever read it: absolute URLs were built from the request's own `Host`
/// header, which any client controls. A mailer that builds a reset link with a
/// `*_url` helper therefore emitted whatever host the request carried, so a
/// single `POST /password/reset` with `Host: evil.example` mailed the victim a
/// link to the attacker's site, token attached. This is the value to fall back
/// on when the request's host is not one we declared.
pub fn primary_declared_host() -> Option<String> {
    let raw = app_hosts_env()?;
    raw.split(',')
        .map(|host| host.trim())
        .find(|host| !host.is_empty())
        .map(|host| host.to_string())
}

/// Is `candidate` one of the declared application hosts?
///
/// `None` from [`primary_declared_host`] means nothing was declared, in which
/// case every host is accepted — the pre-existing behaviour, kept so a
/// development server with no configuration still works.
pub fn is_declared_host(candidate: &str) -> bool {
    match app_hosts_env() {
        Some(raw) if raw.split(',').any(|h| !h.trim().is_empty()) => {
            host_matches_declared(&raw, candidate)
        }
        _ => true,
    }
}

fn host_matches_declared(raw: &str, origin_auth: &str) -> bool {
    raw.split(',')
        .map(|host| host.trim())
        .filter(|host| !host.is_empty())
        .any(|host| {
            // The declared value may carry a port or not; an Origin always
            // omits the default port for its scheme. Accept both so
            // `app.example.com` matches an Origin of `app.example.com:443`
            // without the operator having to think about it.
            host.eq_ignore_ascii_case(origin_auth)
                || origin_auth
                    .split_once(':')
                    .is_some_and(|(bare, _)| host.eq_ignore_ascii_case(bare))
        })
}

/// `SOLI_DISABLE_CSRF` operator kill switch — turns off both the
/// Origin/Referer gate and per-form token verification.
fn csrf_disabled_by_env() -> bool {
    cached_env_var(&DISABLE_CSRF_ENV, "SOLI_DISABLE_CSRF")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// `SOLI_CSRF_TOKENS=require` strict mode: browser form posts
/// (urlencoded/multipart bodies) MUST carry a valid per-form token.
fn csrf_tokens_required() -> bool {
    cached_env_var(&CSRF_TOKENS_ENV, "SOLI_CSRF_TOKENS")
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
    if is_framework_path(path) || csrf_skipped_by_app(path) || csrf_disabled_by_env() {
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
        None if csrf_tokens_required()
            && is_form_content_type(content_type)
            && !is_operator_dashboard_path(path) =>
        {
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
/// - Framework-served endpoints are exempt (see [`is_framework_path`]: the
///   reserved `/__soli/`, `/__solidev/`, `/__dev/`, `/__livereload`
///   namespaces, which application code never sees — the server answers 404
///   for any of them no framework handler claims). Application routes are never
///   exempt implicitly — not even under `/_` — they opt out via
///   `skip_csrf`.
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
    if is_framework_path(path) {
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
        if origin_auth == request_authority || origin_matches_declared_host(&origin_auth) {
            return Ok(());
        }
        return Err(format!(
            "Origin {} does not match request authority {} \
             (behind a proxy? set SOLI_APP_HOSTS to the public hostname)",
            origin_auth, request_authority
        ));
    }

    if let Some(referer) = headers.get(header::REFERER).and_then(|v| v.to_str().ok()) {
        let Some(referer_auth) = origin_authority(referer) else {
            return Err(format!("malformed Referer header: {}", referer));
        };
        if referer_auth == request_authority || origin_matches_declared_host(&referer_auth) {
            return Ok(());
        }
        return Err(format!(
            "Referer {} does not match request authority {} \
             (behind a proxy? set SOLI_APP_HOSTS to the public hostname)",
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

#[cfg(test)]
mod framework_path_tests {
    use super::is_framework_path;

    #[test]
    fn the_framework_endpoints_are_exempt() {
        for path in [
            "/__coverage__",
            "/__soli/prefetch.js",
            "/__soli/inbox/clear",
            "/__solidev/replay/abc",
            "/__dev/repl",
            "/__livereload",
            "/__livereload_ws",
        ] {
            assert!(is_framework_path(path), "expected exempt: {path}");
        }
    }

    #[test]
    fn an_application_route_under_underscore_is_not_exempt() {
        // The whole point of narrowing the old `starts_with("/_")`: these are
        // ordinary app routes and must keep both CSRF barriers.
        for path in [
            "/_internal/probe",
            "/_admin/users",
            "/_health/subresource",
            "/_",
            "/__",
            "/__soli",
            "/__solidev",
            "/__soliboom/x",
            "/__livereloadx",
            "/__livereload_wsx",
            "/posts",
            // The probes answer GET/HEAD only; a POST there is an app route.
            "/_health",
            "/_ready",
            "/_metrics",
        ] {
            assert!(!is_framework_path(path), "expected NOT exempt: {path}");
        }
    }

    /// Every exempt path must be one `handle_hyper_request` refuses to hand to
    /// the application when no framework handler claimed it — otherwise
    /// `POST /__soli/account/delete` reaches `post("/:locale/account/delete")`
    /// with no CSRF check.
    #[test]
    fn the_reserved_namespace_is_exactly_the_exempt_one() {
        use super::is_reserved_framework_path;
        for path in [
            "/__coverage__",
            "/__soli/account/delete",
            "/__soli/jobs/abc/retry",
            "/__solidev/replay/abc",
            "/__dev/repl",
            "/__livereload",
            "/__livereload/anything",
            "/__livereload_ws",
        ] {
            assert!(
                is_reserved_framework_path(path),
                "expected reserved: {path}"
            );
        }
        for path in [
            "/__livereloadx",
            "/__soli",
            "/__soliboom/x",
            "/_health",
            "/_internal/probe",
            "/en/account/delete",
            "/",
        ] {
            assert!(
                !is_reserved_framework_path(path),
                "expected NOT reserved: {path}"
            );
            assert!(!is_framework_path(path), "exempt but not reserved: {path}");
        }
    }

    /// The jobs and errors pages are the production-reachable, state-changing
    /// endpoints in the reserved namespace. They sit behind Basic auth, which a
    /// browser attaches automatically, so exempting them made
    /// `POST /__soli/jobs/<id>/retry` forgeable from any other site.
    #[test]
    fn the_operator_dashboards_keep_both_csrf_barriers() {
        for path in [
            "/__soli/jobs",
            "/__soli/jobs/abc/retry",
            "/__soli/jobs/abc/cancel",
            "/__soli/errors",
            "/__soli/errors/0123456789abcdef/resolve",
            "/__soli/errors/0123456789abcdef/delete",
        ] {
            assert!(!is_framework_path(path), "expected NOT exempt: {path}");
        }
        // Their siblings in the namespace stay exempt.
        assert!(is_framework_path("/__soli/jobsomething"));
        assert!(is_framework_path("/__soli/errorsomething"));
    }
}

#[cfg(test)]
mod skip_pattern_tests {
    use super::path_matches_csrf_skip;

    #[test]
    fn prefix_slash_star_matches_path_and_children() {
        assert!(path_matches_csrf_skip("/api", "/api/*"));
        assert!(path_matches_csrf_skip("/api/", "/api/*"));
        assert!(path_matches_csrf_skip("/api/v1", "/api/*"));
        assert!(path_matches_csrf_skip("/api/v1/hooks", "/api/*"));
        // Must not match a longer sibling prefix.
        assert!(!path_matches_csrf_skip("/apis", "/api/*"));
        assert!(!path_matches_csrf_skip("/other", "/api/*"));
    }

    #[test]
    fn exact_path_is_not_prefix() {
        assert!(path_matches_csrf_skip("/webhook", "/webhook"));
        assert!(!path_matches_csrf_skip("/webhook/extra", "/webhook"));
    }

    #[test]
    fn bare_star_is_prefix_match() {
        assert!(path_matches_csrf_skip(
            "/api/legacy/upload",
            "/api/legacy/*"
        ));
        assert!(path_matches_csrf_skip("/apix", "/api*"));
        assert!(!path_matches_csrf_skip("/xapi", "/api*"));
    }
}

#[cfg(test)]
mod declared_host_tests {
    use super::*;

    /// `SOLI_APP_HOSTS` is process-global and Rust runs tests in parallel, so
    /// without a lock one test's teardown clears another's setup mid-assertion
    /// — which is exactly how this helper failed the first time it was written.
    pub(super) static HOSTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_hosts<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = HOSTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("SOLI_APP_HOSTS").ok();
        match value {
            Some(v) => unsafe { std::env::set_var("SOLI_APP_HOSTS", v) },
            None => unsafe { std::env::remove_var("SOLI_APP_HOSTS") },
        }
        let out = body();
        match previous {
            Some(v) => unsafe { std::env::set_var("SOLI_APP_HOSTS", v) },
            None => unsafe { std::env::remove_var("SOLI_APP_HOSTS") },
        }
        out
    }

    #[test]
    fn a_declared_host_is_accepted() {
        with_hosts(Some("app.example.com"), || {
            assert!(origin_matches_declared_host("app.example.com"));
        });
    }

    #[test]
    fn an_undeclared_host_is_not() {
        // The whole point: the allowlist comes from the deployment, so an
        // attacker cannot widen it with a header.
        with_hosts(Some("app.example.com"), || {
            assert!(!origin_matches_declared_host("evil.example"));
            assert!(!origin_matches_declared_host("app.example.com.evil.test"));
            assert!(!origin_matches_declared_host("notapp.example.com"));
        });
    }

    #[test]
    fn an_unset_allowlist_accepts_nothing() {
        // Absent must never mean permissive — that would silently disable the
        // same-origin gate for every app that has not configured it.
        with_hosts(None, || {
            assert!(!origin_matches_declared_host("app.example.com"));
            assert!(!origin_matches_declared_host(""));
        });
    }

    #[test]
    fn several_hosts_and_stray_whitespace_are_handled() {
        with_hosts(Some(" app.example.com , www.app.example.com ,, "), || {
            assert!(origin_matches_declared_host("app.example.com"));
            assert!(origin_matches_declared_host("www.app.example.com"));
            // An empty entry from a trailing comma must not match everything.
            assert!(!origin_matches_declared_host(""));
            assert!(!origin_matches_declared_host("other.example.com"));
        });
    }

    #[test]
    fn the_comparison_is_case_insensitive() {
        // Hostnames are case-insensitive, and a browser may send either.
        with_hosts(Some("App.Example.COM"), || {
            assert!(origin_matches_declared_host("app.example.com"));
        });
    }

    #[test]
    fn an_explicit_port_on_the_origin_still_matches_a_bare_host() {
        // An Origin omits the default port for its scheme but not a custom
        // one. Making the operator declare both forms would be a footgun with
        // a rejection as its only symptom.
        with_hosts(Some("app.example.com"), || {
            assert!(origin_matches_declared_host("app.example.com:8443"));
        });
        with_hosts(Some("app.example.com:8443"), || {
            assert!(origin_matches_declared_host("app.example.com:8443"));
        });
    }
}

#[cfg(test)]
mod app_host_validation_tests {
    use super::*;

    /// `SOLI_APP_HOSTS` is process-global and tests run in parallel; share the
    /// sibling module's lock so one test's teardown cannot clear another's
    /// setup mid-assertion.
    use super::declared_host_tests::HOSTS_LOCK;

    /// `SOLI_APP_HOSTS` is required in production but was only ever read by the
    /// CSRF gate, so `*_url` helpers happily built links from a spoofed `Host`.
    #[test]
    fn a_host_outside_the_declared_list_is_rejected() {
        with_hosts(Some("app.example.com,www.app.example.com"), || {
            assert!(is_declared_host("app.example.com"));
            assert!(is_declared_host("www.app.example.com"));
            assert!(is_declared_host("app.example.com:443"));
            assert!(!is_declared_host("evil.example"));
            assert!(!is_declared_host("app.example.com.evil.example"));
        });
    }

    #[test]
    fn the_first_declared_host_is_the_fallback() {
        with_hosts(Some("app.example.com,www.app.example.com"), || {
            assert_eq!(primary_declared_host().as_deref(), Some("app.example.com"));
        });
    }

    /// Nothing declared (a dev server) accepts any host, as before.
    #[test]
    fn an_unconfigured_server_accepts_any_host() {
        with_hosts(None, || {
            assert!(is_declared_host("localhost:5011"));
            assert!(is_declared_host("anything"));
            assert!(primary_declared_host().is_none());
        });
    }

    /// An empty or whitespace-only value must not be read as "one host named
    /// nothing", which would reject every real request.
    #[test]
    fn an_empty_declaration_behaves_as_unconfigured() {
        with_hosts(Some("  , "), || {
            assert!(is_declared_host("localhost:5011"));
        });
    }

    fn with_hosts<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = HOSTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("SOLI_APP_HOSTS").ok();
        match value {
            Some(v) => unsafe { std::env::set_var("SOLI_APP_HOSTS", v) },
            None => unsafe { std::env::remove_var("SOLI_APP_HOSTS") },
        }
        let out = body();
        match previous {
            Some(v) => unsafe { std::env::set_var("SOLI_APP_HOSTS", v) },
            None => unsafe { std::env::remove_var("SOLI_APP_HOSTS") },
        }
        out
    }
}
