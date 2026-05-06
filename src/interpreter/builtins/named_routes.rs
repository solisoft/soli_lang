//! Rails-style named route helpers (`*_path` and `*_url`).
//!
//! Routes registered via `resources()` (and `get/post/...` with an `as:` arg)
//! get tagged with a Rails-style name. At worker boot we walk those routes
//! and define one `<name>_path` and one `<name>_url` `NativeFunction` in the
//! worker's interpreter env.
//!
//! `*_url` resolves scheme + host from the per-request thread-local set in
//! `serve/mod.rs`, falling back to `SOLI_DEFAULT_URL_HOST` (and
//! `SOLI_DEFAULT_URL_SCHEME`, default `http`) for use outside a request.

use crate::interpreter::builtins::server::{get_routes, Route};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Clone)]
pub struct NamedRouteEntry {
    pub method: String,
    pub path_pattern: String,
}

thread_local! {
    static NAMED_ROUTES: RefCell<HashMap<String, NamedRouteEntry>> = RefCell::new(HashMap::new());
    static CURRENT_REQUEST_HOST: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

/// Set the (scheme, host) pair for the current request. Called by the server
/// just before dispatching to the action so `*_url` helpers can read it.
pub fn set_current_request_host(scheme: String, host: String) {
    CURRENT_REQUEST_HOST.with(|c| {
        *c.borrow_mut() = Some((scheme, host));
    });
}

/// Clear the per-request host (called after the response is built).
pub fn clear_current_request_host() {
    CURRENT_REQUEST_HOST.with(|c| {
        *c.borrow_mut() = None;
    });
}

fn get_current_request_host() -> Option<(String, String)> {
    CURRENT_REQUEST_HOST.with(|c| c.borrow().clone())
}

/// Public accessor for the per-request (scheme, host). Used by `redirect(:back)`
/// to validate that the Referer points at our own host before honoring it.
pub fn current_request_host() -> Option<(String, String)> {
    get_current_request_host()
}

/// Rebuild the named-route lookup table from the current ROUTES list.
/// Called from `RouteIndex::rebuild` so the name index stays in lock-step
/// with the route table on initial load and on hot reload.
pub fn rebuild_named_routes(routes: &[Route]) {
    NAMED_ROUTES.with(|n| {
        let mut map = n.borrow_mut();
        map.clear();
        for r in routes {
            if let Some(name) = &r.name {
                map.insert(
                    name.clone(),
                    NamedRouteEntry {
                        method: r.method.clone(),
                        path_pattern: r.path_pattern.clone(),
                    },
                );
            }
        }
    });
}

fn lookup(name: &str) -> Option<NamedRouteEntry> {
    NAMED_ROUTES.with(|n| n.borrow().get(name).cloned())
}

/// Read field `:id` (or another param name) off a Soli class instance.
fn read_instance_field(value: &Value, field: &str) -> Option<String> {
    if let Value::Instance(inst) = value {
        if let Some(v) = inst.borrow().get(field) {
            return Some(value_to_path_segment(&v));
        }
    }
    None
}

/// Read a key off a Soli hash. Tries the bare param name; falls back to
/// the `:`-prefixed symbol form some users write.
fn read_hash_key(value: &Value, key: &str) -> Option<String> {
    if let Value::Hash(h) = value {
        let pairs = h.borrow();
        if let Some(v) = pairs.get(&HashKey::String(key.to_string())) {
            return Some(value_to_path_segment(v));
        }
        if let Some(v) = pairs.get(&HashKey::Symbol(key.to_string())) {
            return Some(value_to_path_segment(v));
        }
    }
    None
}

/// Convert a value to its URL path segment representation.
/// Strings, ints, decimals, etc. all stringify naturally.
fn value_to_path_segment(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Decimal(d) => d.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// Extract `:param`-style placeholders from a path pattern in declaration order.
fn extract_param_names(pattern: &str) -> Vec<String> {
    pattern
        .split('/')
        .filter_map(|seg| seg.strip_prefix(':').map(|s| s.to_string()))
        .collect()
}

/// Build the relative path for a named route by substituting params.
fn build_path_for_name(name: &str, args: &[Value]) -> Result<String, String> {
    let entry =
        lookup(name).ok_or_else(|| format!("{}: no route registered with this name", name))?;
    let param_names = extract_param_names(&entry.path_pattern);

    if param_names.is_empty() {
        // Static route — ignore any args silently to match Rails (`root_path()`).
        return Ok(entry.path_pattern);
    }

    // Resolve each :param in declaration order.
    let mut values: HashMap<String, String> = HashMap::new();

    // Strategy 1: single arg that is an Instance or Hash — pull every param off it.
    if args.len() == 1 {
        let arg = &args[0];
        let mut all_resolved = true;
        for p in &param_names {
            if let Some(v) = read_instance_field(arg, p).or_else(|| read_hash_key(arg, p)) {
                values.insert(p.clone(), v);
            } else {
                all_resolved = false;
                break;
            }
        }
        // Strategy 1b: single arg, single param, primitive — assign directly.
        if !all_resolved && param_names.len() == 1 && is_primitive(arg) {
            values.insert(param_names[0].clone(), value_to_path_segment(arg));
        } else if !all_resolved {
            // Fall through to per-param error below.
            values.clear();
        }
    } else {
        // Strategy 2: positional primitives in declaration order.
        if args.len() == param_names.len() {
            for (p, a) in param_names.iter().zip(args.iter()) {
                values.insert(p.clone(), value_to_path_segment(a));
            }
        }
    }

    // Substitute into the pattern.
    let mut out = String::with_capacity(entry.path_pattern.len() + 8);
    let mut first = true;
    for seg in entry.path_pattern.split('/') {
        if !first {
            out.push('/');
        }
        first = false;
        if let Some(p) = seg.strip_prefix(':') {
            match values.get(p) {
                Some(v) => out.push_str(v),
                None => {
                    return Err(format!(
                        "{}: missing param :{} (pattern: {})",
                        name, p, entry.path_pattern
                    ));
                }
            }
        } else {
            out.push_str(seg);
        }
    }
    Ok(out)
}

fn is_primitive(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Int(_) | Value::Float(_) | Value::Decimal(_) | Value::Bool(_)
    )
}

/// Build the absolute URL for a named route. Pulls scheme+host from the
/// current request, or from `SOLI_DEFAULT_URL_HOST` env if set.
fn build_url_for_name(name: &str, args: &[Value]) -> Result<String, String> {
    let path = build_path_for_name(name, args)?;
    let (scheme, host) = match get_current_request_host() {
        Some(pair) => pair,
        None => {
            let host = std::env::var("SOLI_DEFAULT_URL_HOST").map_err(|_| {
                format!(
                    "{}: cannot resolve host (no active request and SOLI_DEFAULT_URL_HOST not set)",
                    name
                )
            })?;
            let scheme =
                std::env::var("SOLI_DEFAULT_URL_SCHEME").unwrap_or_else(|_| "http".to_string());
            (scheme, host)
        }
    };
    Ok(format!("{}://{}{}", scheme, host, path))
}

/// Iterate the current thread's routes and define a `<name>_path` and
/// `<name>_url` `NativeFunction` in `env` for each named route.
///
/// Idempotent: re-registering the same name overwrites the previous binding,
/// which is exactly what hot-reload wants.
pub fn register_named_route_helpers(env: &mut Environment) {
    let routes = get_routes();
    let mut seen = std::collections::HashSet::new();
    for r in &routes {
        let Some(base) = &r.name else { continue };
        if !seen.insert(base.clone()) {
            continue;
        }
        define_helpers_for(env, base);
    }
}

fn define_helpers_for(env: &mut Environment, base: &str) {
    let path_name = format!("{}_path", base);
    let url_name = format!("{}_url", base);
    let base_owned = base.to_string();
    let path_base = base_owned.clone();
    env.define(
        path_name.clone(),
        Value::NativeFunction(NativeFunction::new(path_name, None, move |args| {
            build_path_for_name(&path_base, &args).map(Value::String)
        })),
    );
    let url_base = base_owned;
    env.define(
        url_name.clone(),
        Value::NativeFunction(NativeFunction::new(url_name, None, move |args| {
            build_url_for_name(&url_base, &args).map(Value::String)
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashPairs;
    use std::rc::Rc;
    use std::sync::Mutex;

    // Tests that read or write `SOLI_DEFAULT_URL_*` share the process's env
    // and must serialize against each other; otherwise one test's
    // remove_var races another's set_var.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that snapshots an env var on entry and restores it on
    /// drop, even if the test panics. Use together with `ENV_LOCK` to
    /// serialize tests that touch the same variable.
    struct EnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvVarGuard {
        fn capture(key: &'static str) -> Self {
            Self {
                key,
                prev: std::env::var(key).ok(),
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn entry(method: &str, pattern: &str) -> NamedRouteEntry {
        NamedRouteEntry {
            method: method.to_string(),
            path_pattern: pattern.to_string(),
        }
    }

    fn install(routes: Vec<(&str, NamedRouteEntry)>) {
        NAMED_ROUTES.with(|n| {
            let mut map = n.borrow_mut();
            map.clear();
            for (k, v) in routes {
                map.insert(k.to_string(), v);
            }
        });
    }

    fn hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut p = HashPairs::default();
        for (k, v) in pairs {
            p.insert(HashKey::String(k.to_string()), v);
        }
        Value::Hash(Rc::new(std::cell::RefCell::new(p)))
    }

    #[test]
    fn collection_path_takes_no_params() {
        install(vec![("posts", entry("GET", "/posts"))]);
        assert_eq!(build_path_for_name("posts", &[]).unwrap(), "/posts");
        // Static routes ignore extra args (Rails behavior).
        assert_eq!(
            build_path_for_name("posts", &[Value::Int(99)]).unwrap(),
            "/posts"
        );
    }

    #[test]
    fn member_path_with_primitive_int() {
        install(vec![("post", entry("GET", "/posts/:id"))]);
        assert_eq!(
            build_path_for_name("post", &[Value::Int(42)]).unwrap(),
            "/posts/42"
        );
    }

    #[test]
    fn member_path_with_string_id() {
        install(vec![("post", entry("GET", "/posts/:id"))]);
        assert_eq!(
            build_path_for_name("post", &[Value::String("abc-123".to_string())]).unwrap(),
            "/posts/abc-123"
        );
    }

    #[test]
    fn member_path_with_hash() {
        install(vec![("post", entry("GET", "/posts/:id"))]);
        let h = hash(vec![("id", Value::Int(7))]);
        assert_eq!(build_path_for_name("post", &[h]).unwrap(), "/posts/7");
    }

    #[test]
    fn edit_member_path() {
        install(vec![("edit_post", entry("GET", "/posts/:id/edit"))]);
        assert_eq!(
            build_path_for_name("edit_post", &[Value::Int(3)]).unwrap(),
            "/posts/3/edit"
        );
    }

    #[test]
    fn missing_param_is_a_clear_error() {
        install(vec![("post", entry("GET", "/posts/:id"))]);
        let err = build_path_for_name("post", &[]).unwrap_err();
        assert!(err.contains("missing param :id"), "got: {}", err);
    }

    #[test]
    fn unknown_helper_name_is_a_clear_error() {
        install(vec![]);
        let err = build_path_for_name("ghost", &[]).unwrap_err();
        assert!(err.contains("no route registered"), "got: {}", err);
    }

    #[test]
    fn nested_route_with_hash_resolves_both_params() {
        install(vec![(
            "post_comment",
            entry("GET", "/posts/:post_id/comments/:id"),
        )]);
        let h = hash(vec![("post_id", Value::Int(5)), ("id", Value::Int(9))]);
        assert_eq!(
            build_path_for_name("post_comment", &[h]).unwrap(),
            "/posts/5/comments/9"
        );
    }

    #[test]
    fn nested_route_with_positional_primitives() {
        install(vec![(
            "post_comment",
            entry("GET", "/posts/:post_id/comments/:id"),
        )]);
        assert_eq!(
            build_path_for_name("post_comment", &[Value::Int(5), Value::Int(9)]).unwrap(),
            "/posts/5/comments/9"
        );
    }

    #[test]
    fn url_uses_request_host_when_set() {
        install(vec![("post", entry("GET", "/posts/:id"))]);
        set_current_request_host("https".to_string(), "example.com".to_string());
        assert_eq!(
            build_url_for_name("post", &[Value::Int(42)]).unwrap(),
            "https://example.com/posts/42"
        );
        clear_current_request_host();
    }

    #[test]
    fn url_falls_back_to_env_default_host() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g_host = EnvVarGuard::capture("SOLI_DEFAULT_URL_HOST");
        let _g_scheme = EnvVarGuard::capture("SOLI_DEFAULT_URL_SCHEME");

        install(vec![("posts", entry("GET", "/posts"))]);
        clear_current_request_host();
        std::env::set_var("SOLI_DEFAULT_URL_HOST", "default.test");
        std::env::remove_var("SOLI_DEFAULT_URL_SCHEME");

        assert_eq!(
            build_url_for_name("posts", &[]).unwrap(),
            "http://default.test/posts"
        );
    }

    #[test]
    fn url_uses_env_default_scheme_when_set() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g_host = EnvVarGuard::capture("SOLI_DEFAULT_URL_HOST");
        let _g_scheme = EnvVarGuard::capture("SOLI_DEFAULT_URL_SCHEME");

        install(vec![("posts", entry("GET", "/posts"))]);
        clear_current_request_host();
        std::env::set_var("SOLI_DEFAULT_URL_HOST", "default.test");
        std::env::set_var("SOLI_DEFAULT_URL_SCHEME", "https");

        assert_eq!(
            build_url_for_name("posts", &[]).unwrap(),
            "https://default.test/posts"
        );
    }

    #[test]
    fn url_errors_when_no_host_available() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g_host = EnvVarGuard::capture("SOLI_DEFAULT_URL_HOST");

        install(vec![("posts", entry("GET", "/posts"))]);
        clear_current_request_host();
        std::env::remove_var("SOLI_DEFAULT_URL_HOST");

        let err = build_url_for_name("posts", &[]).unwrap_err();
        assert!(err.contains("cannot resolve host"), "got: {}", err);
    }
}
