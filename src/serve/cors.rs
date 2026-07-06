//! Built-in CORS.
//!
//! Apps declare cross-origin access per path in `config/routes.sl`:
//!
//! ```soli
//! cors("/api/*", {
//!   "origins": ["https://app.example.com"],   # or "*" (the default)
//!   "methods": ["GET", "POST", "PATCH", "DELETE"],
//!   "headers": ["Content-Type", "Authorization"],
//!   "expose": ["X-Request-Id"],
//!   "credentials": true,
//!   "max_age": 86400
//! })
//! ```
//!
//! The server then answers preflights (`OPTIONS` + `Origin` +
//! `Access-Control-Request-Method`) before routing, stamps the allow
//! headers onto every response of a CORS-managed path (buffered, streamed,
//! static, and error responses alike — the layer wraps the whole request
//! handler), and lets an allowed `Origin` pass the same-origin CSRF gate
//! for that path — a `cors()` declaration is a more precise cross-origin
//! opt-in than `skip_csrf`, since the origin is checked against the
//! declared list.

use std::sync::RwLock;

#[derive(Clone, Debug)]
pub struct CorsRule {
    /// Path pattern, `skip_csrf`-style: exact, `/prefix/*`, or `prefix*`.
    pub pattern: String,
    /// Allowed origins: `["*"]` or explicit `scheme://host[:port]` values.
    pub origins: Vec<String>,
    /// Methods advertised on preflight (uppercase).
    pub methods: Vec<String>,
    /// Request headers advertised on preflight; empty echoes whatever the
    /// preflight asked for (`Access-Control-Request-Headers`).
    pub allow_headers: Vec<String>,
    /// Response headers exposed to cross-origin JS.
    pub expose_headers: Vec<String>,
    /// Allow cookies/credentials. The allow-origin header then echoes the
    /// requesting origin (never `*`, per spec).
    pub credentials: bool,
    /// Preflight cache lifetime in seconds.
    pub max_age: u64,
}

/// Everything the service layer needs for one request: headers to stamp on
/// the eventual response, and — for preflights — the full header set to
/// answer with immediately (204, no routing).
pub struct CorsDecision {
    pub response_headers: Vec<(String, String)>,
    pub preflight: Option<Vec<(String, String)>>,
}

/// Registered rules. RwLock: writes happen at boot / routes hot-reload,
/// reads on the request hot path.
static CORS_RULES: RwLock<Vec<CorsRule>> = RwLock::new(Vec::new());

/// Register (or update) the rule for a path pattern. Keyed by pattern so a
/// routes hot-reload re-running `cors(...)` picks up config edits instead
/// of stacking duplicates.
pub fn register_cors_rule(rule: CorsRule) {
    if let Ok(mut guard) = CORS_RULES.write() {
        if let Some(existing) = guard.iter_mut().find(|r| r.pattern == rule.pattern) {
            *existing = rule;
        } else {
            guard.push(rule);
        }
    }
}

#[cfg(test)]
pub fn clear_cors_rules() {
    if let Ok(mut guard) = CORS_RULES.write() {
        guard.clear();
    }
}

/// Same pattern semantics as `skip_csrf`: exact match, `/prefix/*` (which
/// also covers `/prefix` itself), or a bare `prefix*`.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        path == prefix || path.starts_with(&format!("{}/", prefix))
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        path.starts_with(prefix)
    } else {
        path == pattern
    }
}

fn rule_for_path(path: &str) -> Option<CorsRule> {
    let guard = CORS_RULES.read().ok()?;
    guard
        .iter()
        .find(|r| pattern_matches(&r.pattern, path))
        .cloned()
}

fn origin_allowed(rule: &CorsRule, origin: &str) -> bool {
    rule.origins
        .iter()
        .any(|o| o == "*" || o.eq_ignore_ascii_case(origin))
}

/// Does a registered rule allow this (path, origin)? Consulted by the CSRF
/// origin gate: a matching `cors()` declaration is an explicit cross-origin
/// opt-in for the path.
pub fn allows_cross_origin(path: &str, origin: &str) -> bool {
    rule_for_path(path)
        .map(|rule| origin_allowed(&rule, origin))
        .unwrap_or(false)
}

/// Evaluate a request against the registered rules. `None` when the request
/// carries no `Origin` or no rule matches the path — the common same-origin
/// case pays one header probe and (only when Origin is present) one registry
/// read.
pub fn evaluate(method: &str, path: &str, headers: &hyper::HeaderMap) -> Option<CorsDecision> {
    let origin = headers
        .get(hyper::header::ORIGIN)?
        .to_str()
        .ok()?
        .trim()
        .to_string();
    let rule = rule_for_path(path)?;

    // The response now depends on Origin — tell caches, even for origins
    // the rule rejects (their cached copy must not leak allow headers).
    let mut response_headers = vec![("Vary".to_string(), "Origin".to_string())];

    if !origin_allowed(&rule, &origin) {
        // CORS-managed path, disallowed origin: no allow headers — the
        // browser blocks the read. Not an error response; the request
        // itself proceeds under the normal same-origin rules.
        return Some(CorsDecision {
            response_headers,
            preflight: None,
        });
    }

    // With credentials the allow-origin must echo the caller (never `*`).
    let allow_origin = if !rule.credentials && rule.origins.iter().any(|o| o == "*") {
        "*".to_string()
    } else {
        origin
    };
    response_headers.push(("Access-Control-Allow-Origin".to_string(), allow_origin));
    if rule.credentials {
        response_headers.push((
            "Access-Control-Allow-Credentials".to_string(),
            "true".to_string(),
        ));
    }

    let is_preflight = method == "OPTIONS" && headers.contains_key("access-control-request-method");
    if is_preflight {
        let mut preflight = response_headers.clone();
        preflight.push((
            "Access-Control-Allow-Methods".to_string(),
            rule.methods.join(", "),
        ));
        let allow_headers = if rule.allow_headers.is_empty() {
            // Echo whatever the browser asked to send.
            headers
                .get("access-control-request-headers")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string()
        } else {
            rule.allow_headers.join(", ")
        };
        if !allow_headers.is_empty() {
            preflight.push(("Access-Control-Allow-Headers".to_string(), allow_headers));
        }
        preflight.push((
            "Access-Control-Max-Age".to_string(),
            rule.max_age.to_string(),
        ));
        return Some(CorsDecision {
            response_headers,
            preflight: Some(preflight),
        });
    }

    if !rule.expose_headers.is_empty() {
        response_headers.push((
            "Access-Control-Expose-Headers".to_string(),
            rule.expose_headers.join(", "),
        ));
    }
    Some(CorsDecision {
        response_headers,
        preflight: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The rule registry is process-global; serialize tests that mutate it.
    static RULES_LOCK: Mutex<()> = Mutex::new(());

    fn rule(pattern: &str) -> CorsRule {
        CorsRule {
            pattern: pattern.to_string(),
            origins: vec!["*".to_string()],
            methods: vec!["GET".to_string(), "POST".to_string()],
            allow_headers: vec![],
            expose_headers: vec![],
            credentials: false,
            max_age: 86400,
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> hyper::HeaderMap {
        let mut map = hyper::HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                hyper::header::HeaderName::try_from(*k).unwrap(),
                hyper::header::HeaderValue::try_from(*v).unwrap(),
            );
        }
        map
    }

    fn header_value<'a>(list: &'a [(String, String)], name: &str) -> Option<&'a str> {
        list.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn no_origin_or_no_rule_is_none() {
        let _lock = RULES_LOCK.lock().unwrap();
        clear_cors_rules();
        register_cors_rule(rule("/api/*"));

        // No Origin header → not a CORS request.
        assert!(evaluate("GET", "/api/users", &headers(&[])).is_none());
        // Origin, but the path has no rule.
        assert!(evaluate(
            "GET",
            "/admin",
            &headers(&[("origin", "https://app.example.com")])
        )
        .is_none());
    }

    #[test]
    fn wildcard_origin_gets_star_and_pattern_covers_prefix() {
        let _lock = RULES_LOCK.lock().unwrap();
        clear_cors_rules();
        register_cors_rule(rule("/api/*"));

        let decision = evaluate(
            "GET",
            "/api/users/7",
            &headers(&[("origin", "https://any.example")]),
        )
        .unwrap();
        assert_eq!(
            header_value(&decision.response_headers, "Access-Control-Allow-Origin"),
            Some("*")
        );
        assert_eq!(
            header_value(&decision.response_headers, "Vary"),
            Some("Origin")
        );
        assert!(decision.preflight.is_none());
    }

    #[test]
    fn credentials_echo_origin_never_star() {
        let _lock = RULES_LOCK.lock().unwrap();
        clear_cors_rules();
        let mut r = rule("/api/*");
        r.credentials = true;
        register_cors_rule(r);

        let decision = evaluate(
            "POST",
            "/api/users",
            &headers(&[("origin", "https://app.example.com")]),
        )
        .unwrap();
        assert_eq!(
            header_value(&decision.response_headers, "Access-Control-Allow-Origin"),
            Some("https://app.example.com")
        );
        assert_eq!(
            header_value(
                &decision.response_headers,
                "Access-Control-Allow-Credentials"
            ),
            Some("true")
        );
    }

    #[test]
    fn disallowed_origin_gets_vary_only_and_no_csrf_bypass() {
        let _lock = RULES_LOCK.lock().unwrap();
        clear_cors_rules();
        let mut r = rule("/api/*");
        r.origins = vec!["https://app.example.com".to_string()];
        register_cors_rule(r);

        let decision = evaluate(
            "GET",
            "/api/users",
            &headers(&[("origin", "https://evil.example")]),
        )
        .unwrap();
        assert_eq!(decision.response_headers.len(), 1);
        assert_eq!(
            header_value(&decision.response_headers, "Vary"),
            Some("Origin")
        );

        assert!(allows_cross_origin("/api/users", "https://app.example.com"));
        assert!(!allows_cross_origin("/api/users", "https://evil.example"));
        assert!(!allows_cross_origin("/other", "https://app.example.com"));
    }

    #[test]
    fn preflight_carries_methods_headers_and_max_age() {
        let _lock = RULES_LOCK.lock().unwrap();
        clear_cors_rules();
        register_cors_rule(rule("/api/*"));

        let decision = evaluate(
            "OPTIONS",
            "/api/users",
            &headers(&[
                ("origin", "https://app.example.com"),
                ("access-control-request-method", "POST"),
                ("access-control-request-headers", "content-type, x-token"),
            ]),
        )
        .unwrap();
        let preflight = decision.preflight.expect("preflight");
        assert_eq!(
            header_value(&preflight, "Access-Control-Allow-Methods"),
            Some("GET, POST")
        );
        // Empty configured list echoes the requested headers.
        assert_eq!(
            header_value(&preflight, "Access-Control-Allow-Headers"),
            Some("content-type, x-token")
        );
        assert_eq!(
            header_value(&preflight, "Access-Control-Max-Age"),
            Some("86400")
        );

        // A plain OPTIONS without Access-Control-Request-Method is NOT a
        // preflight.
        let decision = evaluate(
            "OPTIONS",
            "/api/users",
            &headers(&[("origin", "https://app.example.com")]),
        )
        .unwrap();
        assert!(decision.preflight.is_none());
    }

    #[test]
    fn re_registering_a_pattern_replaces_the_rule() {
        let _lock = RULES_LOCK.lock().unwrap();
        clear_cors_rules();
        register_cors_rule(rule("/api/*"));
        let mut updated = rule("/api/*");
        updated.origins = vec!["https://only.example".to_string()];
        register_cors_rule(updated);

        assert!(allows_cross_origin("/api/x", "https://only.example"));
        assert!(!allows_cross_origin("/api/x", "https://other.example"));
    }
}
