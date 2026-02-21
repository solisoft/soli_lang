//! HTTP server built-in functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};

/// A registered route with its handler.
/// For worker threads, we use a separate struct without middleware Values.
#[derive(Clone)]
pub struct Route {
    pub method: String,
    pub path_pattern: String,
    pub handler_name: String, // Store function name instead of Value
    pub middleware: Vec<Value>,
}

/// A worker-safe route struct without middleware Values.
/// Used to pass routes to worker threads.
#[derive(Clone)]
pub struct WorkerRoute {
    pub method: String,
    pub path_pattern: String,
    pub handler_name: String,
}

// Route registry stored in thread-local storage.
// Routes contain handler names that are looked up in each worker's interpreter.
thread_local! {
    pub static ROUTES: RefCell<Vec<Route>> = const { RefCell::new(Vec::new()) };
    // Method-indexed route cache for O(1) method lookup + O(m) route search
    // where m = routes for that method, instead of O(n) for all routes
    static ROUTE_INDEX: RefCell<RouteIndex> = RefCell::new(RouteIndex::new());
}

/// Method-indexed route cache for faster lookups.
/// Groups routes by HTTP method and provides fast exact-match lookup.
#[derive(Clone, Default)]
pub struct RouteIndex {
    /// Routes grouped by HTTP method
    by_method: HashMap<String, Vec<usize>>, // method -> indices into ROUTES
    /// Exact path matches for fast lookup (method:path -> route index)
    exact_matches: HashMap<String, usize>,
    /// Version counter to detect stale index
    version: u64,
}

impl RouteIndex {
    fn new() -> Self {
        Self::default()
    }

    /// Rebuild the index from the current routes.
    fn rebuild(&mut self, routes: &[Route]) {
        self.by_method.clear();
        self.exact_matches.clear();

        for (idx, route) in routes.iter().enumerate() {
            // Add to method index
            self.by_method
                .entry(route.method.clone())
                .or_default()
                .push(idx);

            // Add exact matches (routes without dynamic segments)
            if !route.path_pattern.contains(':') {
                let key = format!("{}:{}", route.method, route.path_pattern);
                self.exact_matches.insert(key, idx);
            }
        }

        self.version += 1;
    }
}

// Track if routes are "direct" (http_server_get/post) vs MVC (get/post DSL)
static DIRECT_ROUTES_MODE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Mark that we're using direct route registration (http_server_get/post)
pub fn set_direct_routes_mode() {
    DIRECT_ROUTES_MODE.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Check if we're using direct route registration
pub fn is_direct_routes_mode() -> bool {
    DIRECT_ROUTES_MODE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Clear all registered routes (useful for testing or restarting).
pub fn clear_routes() {
    ROUTES.with(|routes| routes.borrow_mut().clear());
    ROUTE_INDEX.with(|index| *index.borrow_mut() = RouteIndex::new());
}

/// Take all routes (consumes and returns them), leaving empty routes.
/// Used for hot reload to save routes before reloading.
pub fn take_routes() -> Vec<Route> {
    ROUTES.with(|routes| {
        let mut r = routes.borrow_mut();
        std::mem::take(&mut *r)
    })
}

/// Restore routes from a previous state (after failed reload).
pub fn restore_routes(routes: Vec<Route>) {
    ROUTES.with(|r| *r.borrow_mut() = routes);
    ROUTE_INDEX.with(|index| *index.borrow_mut() = RouteIndex::new());
}

/// Clear routes that match a specific path prefix.
/// Used for hot reload to clear routes from a specific controller.
pub fn clear_routes_for_prefix(prefix: &str) {
    ROUTES.with(|routes| {
        routes.borrow_mut().retain(|r| {
            if prefix == "/" {
                let path = &r.path_pattern;
                if path == "/" {
                    return false;
                }
                return true;
            }
            !r.path_pattern.starts_with(prefix)
        });
    });
}

/// Register a route with a handler name.
/// Used by the MVC framework to register derived routes.
pub fn register_route_with_handler(method: &str, path: &str, handler_name: String) {
    register_route(method, path, handler_name, Vec::new());
}

/// Register a route with a handler and scoped middleware.
pub fn register_route_with_middleware(
    method: &str,
    path: &str,
    handler_name: String,
    middleware: Vec<Value>,
) {
    register_route(method, path, handler_name, middleware);
}

/// Get all registered routes.
pub fn get_routes() -> Vec<Route> {
    ROUTES.with(|routes| routes.borrow().clone())
}

/// Rebuild the route index from current routes.
/// Call this after modifying routes to enable fast lookups.
pub fn rebuild_route_index() {
    ROUTES.with(|routes| {
        ROUTE_INDEX.with(|index| {
            index.borrow_mut().rebuild(&routes.borrow());
        });
    });
}

/// Find a matching route by method and path using the index.
/// Returns the matched route (cloned) and extracted parameters.
/// This is more efficient than iterating through all routes.
pub fn find_route(method: &str, path: &str) -> Option<(Route, HashMap<String, String>)> {
    ROUTES.with(|routes| {
        ROUTE_INDEX.with(|index| {
            let routes = routes.borrow();
            let index = index.borrow();

            // Fast path: try exact match first
            let exact_key = format!("{}:{}", method, path);
            if let Some(&idx) = index.exact_matches.get(&exact_key) {
                if let Some(route) = routes.get(idx) {
                    return Some((route.clone(), HashMap::new()));
                }
            }

            // Fall back to method-indexed search with pattern matching
            if let Some(indices) = index.by_method.get(method) {
                for &idx in indices {
                    if let Some(route) = routes.get(idx) {
                        if let Some(params) = match_path(&route.path_pattern, path) {
                            return Some((route.clone(), params));
                        }
                    }
                }
            }

            None
        })
    })
}

/// Expand a wildcard action pattern using matched path parameters.
///
/// Handles patterns like:
/// - "docs#*" with {path: "/routing"} → "docs#routing"
/// - "api#*" with {version: "/v1", action: "/users"} → "api#users"
/// - "controller#action" (no wildcard) → unchanged
///
/// Returns None if wildcard can't be expanded.
pub fn expand_wildcard_action(
    handler_name: &str,
    params: &HashMap<String, String>,
) -> Option<String> {
    // Check if handler_name contains # (controller#action format)
    if let Some((controller, action)) = handler_name.split_once('#') {
        if action == "*" {
            // Try to expand from splat params
            // Look for common splat param names
            if let Some(splat) = params.get("splat") {
                let action_name = splat.trim_start_matches('/');
                if !action_name.is_empty() {
                    return Some(format!("{}#{}", controller, action_name));
                }
            }
            // Look for first splat param (any param starting with * in pattern becomes captured without *)
            for (key, value) in params.iter() {
                if key == "path"
                    || key == "filepath"
                    || key == "filename"
                    || key == "action"
                    || key == "resource"
                {
                    let action_name = value.trim_start_matches('/');
                    if !action_name.is_empty() {
                        return Some(format!("{}#{}", controller, action_name));
                    }
                }
            }
            // Look for any param that could be the action
            for (_, value) in params.iter() {
                let action_name = value.trim_start_matches('/');
                if !action_name.is_empty() && !action_name.contains('/') {
                    // Prefer non-nested paths as action names
                    return Some(format!("{}#{}", controller, action_name));
                }
            }
            None
        } else {
            // No wildcard, return as-is
            Some(handler_name.to_string())
        }
    } else {
        // No controller#action format, check if it's just "*"
        if handler_name == "*" {
            for (_, value) in params.iter() {
                let action_name = value.trim_start_matches('/');
                if !action_name.is_empty() {
                    return Some(action_name.to_string());
                }
            }
            None
        } else {
            Some(handler_name.to_string())
        }
    }
}

/// Convert routes to worker-safe routes (without middleware Values).
pub fn routes_to_worker_routes(routes: &[Route]) -> Vec<WorkerRoute> {
    routes
        .iter()
        .map(|r| WorkerRoute {
            method: r.method.clone(),
            path_pattern: r.path_pattern.clone(),
            handler_name: r.handler_name.clone(),
        })
        .collect()
}

/// Set routes in the current thread's storage.
/// Used by worker threads to initialize their route tables from the main thread.
pub fn set_routes(routes: Vec<Route>) {
    ROUTES.with(|r| *r.borrow_mut() = routes);
    rebuild_route_index();
}

/// Set worker routes in the current thread's storage (for worker threads).
pub fn set_worker_routes(routes: Vec<WorkerRoute>) {
    // Convert WorkerRoute back to Route (with empty middleware for workers)
    let routes: Vec<Route> = routes
        .into_iter()
        .map(|r| Route {
            method: r.method,
            path_pattern: r.path_pattern,
            handler_name: r.handler_name,
            middleware: Vec::new(),
        })
        .collect();
    ROUTES.with(|r| *r.borrow_mut() = routes);
    rebuild_route_index();
}

/// Register a route.
fn register_route(method: &str, path: &str, handler_name: String, middleware: Vec<Value>) {
    ROUTES.with(|routes| {
        routes.borrow_mut().push(Route {
            method: method.to_string(),
            path_pattern: path.to_string(),
            handler_name,
            middleware,
        });
    });
}

/// Match a path against a pattern and extract parameters.
/// Pattern format: "/users/:id" matches "/users/123" with params {"id": "123"}
/// Splat format: "/files/*path" matches "/files/a/b" with params {"path": "/a/b"}
pub fn match_path(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    // Fast path: exact match (no params)
    if pattern == path {
        return Some(HashMap::new());
    }

    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();

    // Count splat segments in pattern
    let splat_count = pattern_parts.iter().filter(|p| p.starts_with('*')).count();

    // If splats exist, pattern can have fewer parts than path
    // Otherwise, parts must match exactly
    if splat_count == 0 && pattern_parts.len() != path_parts.len() {
        return None;
    }

    // With splats, pattern parts must be <= path parts
    if splat_count > 0 && pattern_parts.len() > path_parts.len() {
        return None;
    }

    let mut params = HashMap::new();

    // Find indices of splat segments in pattern
    let splat_indices: Vec<usize> = pattern_parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.starts_with('*'))
        .map(|(i, _)| i)
        .collect();

    if splat_indices.is_empty() {
        // No splats - exact part matching
        for (pat, actual) in pattern_parts.iter().zip(path_parts.iter()) {
            if let Some(param_name) = pat.strip_prefix(':') {
                params.insert(param_name.to_string(), actual.to_string());
            } else if pat != actual {
                return None;
            }
        }
    } else {
        // Has splats - handle multiple splats
        // All splats except the last consume exactly one segment
        // The last splat consumes all remaining segments (if there are no literals after it)
        let mut path_idx = 0;
        let last_splat_idx = *splat_indices.last().unwrap_or(&0);

        for (pat_idx, pat) in pattern_parts.iter().enumerate() {
            if pat.starts_with('*') {
                let param_name = pat.strip_prefix('*').unwrap();
                // Check if there are literal segments after this splat
                let has_literals_after = pattern_parts[pat_idx + 1..]
                    .iter()
                    .any(|p| !p.starts_with(':') && !p.starts_with('*'));

                if pat_idx == last_splat_idx && !has_literals_after {
                    // Last splat with no literals after - consume all remaining path parts
                    let remaining_parts = &path_parts[path_idx..];
                    let captured = remaining_parts.join("/");
                    let with_leading_slash = format!("/{}", captured);
                    params.insert(param_name.to_string(), with_leading_slash);
                    path_idx = path_parts.len();
                } else {
                    // Non-last splat or splat followed by literals - consume exactly one segment
                    if path_idx >= path_parts.len() {
                        return None;
                    }
                    let segment = path_parts[path_idx];
                    params.insert(param_name.to_string(), format!("/{}", segment));
                    path_idx += 1;
                }
            } else {
                // Non-splat segment
                if path_idx >= path_parts.len() {
                    return None;
                }
                if let Some(param_name) = pat.strip_prefix(':') {
                    params.insert(param_name.to_string(), path_parts[path_idx].to_string());
                } else if *pat != path_parts[path_idx] {
                    return None;
                }
                path_idx += 1;
            }
        }

        // Verify we consumed all path parts
        if path_idx != path_parts.len() {
            return None;
        }
    }

    Some(params)
}

#[cfg(test)]
mod match_path_tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert_eq!(match_path("/users", "/users"), Some(HashMap::new()));
        assert_eq!(match_path("/api/v1", "/api/v1"), Some(HashMap::new()));
    }

    #[test]
    fn test_param_match() {
        let result = match_path("/users/:id", "/users/123");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_multiple_params() {
        let result = match_path("/users/:user_id/posts/:post_id", "/users/456/posts/789");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("user_id"), Some(&"456".to_string()));
        assert_eq!(params.get("post_id"), Some(&"789".to_string()));
    }

    #[test]
    fn test_splat_basic() {
        let result = match_path("/files/*path", "/files/a/b/c");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("path"), Some(&"/a/b/c".to_string()));
    }

    #[test]
    fn test_splat_single_segment() {
        let result = match_path("/files/*path", "/files/document.pdf");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("path"), Some(&"/document.pdf".to_string()));
    }

    #[test]
    fn test_splat_with_prefix() {
        let result = match_path("/api/*version/users", "/api/v1/users");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("version"), Some(&"/v1".to_string()));
    }

    #[test]
    fn test_splat_combined_with_params() {
        let result = match_path("/users/:id/*action", "/users/123/edit/delete");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"123".to_string()));
        assert_eq!(params.get("action"), Some(&"/edit/delete".to_string()));
    }

    #[test]
    fn test_multiple_splats() {
        let result = match_path("/api/*version/*resource", "/api/v1/users/list");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("version"), Some(&"/v1".to_string()));
        assert_eq!(params.get("resource"), Some(&"/users/list".to_string()));
    }

    #[test]
    fn test_splat_no_match_different_prefix() {
        assert!(match_path("/files/*path", "/documents/a/b").is_none());
    }

    #[test]
    fn test_splat_at_root() {
        let result = match_path("/*splat", "/anything/here/goes");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(
            params.get("splat"),
            Some(&"/anything/here/goes".to_string())
        );
    }

    #[test]
    fn test_param_no_match() {
        assert!(match_path("/users/:id", "/posts/123").is_none());
    }

    #[test]
    fn test_literal_no_match() {
        assert!(match_path("/users", "/admins").is_none());
    }

    #[test]
    fn test_empty_path() {
        assert_eq!(match_path("/", "/"), Some(HashMap::new()));
    }

    #[test]
    fn test_three_splats() {
        let result = match_path("/*a/*b/*c", "/one/two/three/four");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("a"), Some(&"/one".to_string()));
        assert_eq!(params.get("b"), Some(&"/two".to_string()));
        assert_eq!(params.get("c"), Some(&"/three/four".to_string()));
    }
}

#[cfg(test)]
mod expand_wildcard_action_tests {
    use super::*;

    #[test]
    fn test_no_wildcard_returns_unchanged() {
        let params = HashMap::new();
        assert_eq!(
            expand_wildcard_action("docs#index", &params),
            Some("docs#index".to_string())
        );
        assert_eq!(
            expand_wildcard_action("users#show", &params),
            Some("users#show".to_string())
        );
    }

    #[test]
    fn test_wildcard_with_splat_param() {
        let mut params = HashMap::new();
        params.insert("splat".to_string(), "/routing".to_string());
        assert_eq!(
            expand_wildcard_action("docs#*", &params),
            Some("docs#routing".to_string())
        );
    }

    #[test]
    fn test_wildcard_with_path_param() {
        let mut params = HashMap::new();
        params.insert("path".to_string(), "/installation".to_string());
        assert_eq!(
            expand_wildcard_action("docs#*", &params),
            Some("docs#installation".to_string())
        );
    }

    #[test]
    fn test_wildcard_with_filepath_param() {
        let mut params = HashMap::new();
        params.insert("filepath".to_string(), "/guide/quickstart".to_string());
        assert_eq!(
            expand_wildcard_action("docs#*", &params),
            Some("docs#guide/quickstart".to_string())
        );
    }

    #[test]
    fn test_wildcard_with_action_param() {
        let mut params = HashMap::new();
        params.insert("action".to_string(), "/edit".to_string());
        assert_eq!(
            expand_wildcard_action("users#*", &params),
            Some("users#edit".to_string())
        );
    }

    #[test]
    fn test_wildcard_with_multiple_splats() {
        let mut params = HashMap::new();
        params.insert("version".to_string(), "/v1".to_string());
        params.insert("action".to_string(), "/users".to_string());
        assert_eq!(
            expand_wildcard_action("api#*", &params),
            Some("api#users".to_string())
        );
    }

    #[test]
    fn test_wildcard_with_nested_path() {
        let mut params = HashMap::new();
        params.insert(
            "resource".to_string(),
            "/users/profile/settings".to_string(),
        );
        // Should use the resource param (has / but we accept nested paths)
        assert_eq!(
            expand_wildcard_action("api#*", &params),
            Some("api#users/profile/settings".to_string())
        );
    }

    #[test]
    fn test_wildcard_returns_none_when_no_expansion() {
        let params = HashMap::new();
        assert_eq!(expand_wildcard_action("docs#*", &params), None);
    }

    #[test]
    fn test_standalone_wildcard() {
        let mut params = HashMap::new();
        params.insert("path".to_string(), "/some/action".to_string());
        assert_eq!(
            expand_wildcard_action("*", &params),
            Some("some/action".to_string())
        );
    }
}

/// Parse query string into a hash map.
pub fn parse_query_string(query: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    if query.is_empty() {
        return result;
    }

    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            let decoded_key = urlencoding::decode(&key.replace('+', " "))
                .unwrap_or_else(|_| key.into())
                .into_owned();
            let decoded_value = urlencoding::decode(&value.replace('+', " "))
                .unwrap_or_else(|_| value.into())
                .into_owned();
            result.insert(decoded_key, decoded_value);
        } else {
            let decoded = urlencoding::decode(&pair.replace('+', " "))
                .unwrap_or_else(|_| pair.into())
                .into_owned();
            result.insert(decoded, String::new());
        }
    }

    result
}

// Pre-allocated hash keys for request hash construction.
// Using a single struct with one thread_local access instead of 10 nested .with() calls.
struct RequestHashKeys {
    method: HashKey,
    path: HashKey,
    params: HashKey,
    query: HashKey,
    headers: HashKey,
    body: HashKey,
    json: HashKey,
    form: HashKey,
    files: HashKey,
    all: HashKey,
}

thread_local! {
    static REQUEST_KEYS: RequestHashKeys = RequestHashKeys {
        method: HashKey::String("method".to_string()),
        path: HashKey::String("path".to_string()),
        params: HashKey::String("params".to_string()),
        query: HashKey::String("query".to_string()),
        headers: HashKey::String("headers".to_string()),
        body: HashKey::String("body".to_string()),
        json: HashKey::String("json".to_string()),
        form: HashKey::String("form".to_string()),
        files: HashKey::String("files".to_string()),
        all: HashKey::String("all".to_string()),
    };
}

/// Pre-built response data for fast-path rendering (avoids Value::Hash round-trip).
/// Set by render_json/render_text, consumed by extract_response.
pub struct FastPathResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

thread_local! {
    static FAST_PATH_RESPONSE: RefCell<Option<FastPathResponse>> = const { RefCell::new(None) };
}

/// Set a fast-path response (called from render_json/render_text).
pub fn set_fast_path_response(resp: FastPathResponse) {
    FAST_PATH_RESPONSE.with(|cell| {
        *cell.borrow_mut() = Some(resp);
    });
}

/// Take the fast-path response if set (called from extract_response).
pub fn take_fast_path_response() -> Option<FastPathResponse> {
    FAST_PATH_RESPONSE.with(|cell| cell.borrow_mut().take())
}

/// Parsed request body data.
#[derive(Default)]
pub struct ParsedBody {
    /// Parsed JSON body (if Content-Type is application/json)
    pub json: Option<Value>,
    /// Parsed form body (if Content-Type is application/x-www-form-urlencoded)
    pub form: Option<Value>,
    /// Uploaded files (if Content-Type is multipart/form-data)
    pub files: Option<Value>,
}

/// Parse a JSON string into a Soli Value.
pub fn parse_json_body(body: &str) -> Option<Value> {
    if body.is_empty() {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(json) => json_to_value(&json).ok(),
        Err(_) => None,
    }
}

/// Parse a URL-encoded form body into a Soli Value (Hash).
pub fn parse_form_urlencoded_body(body: &str) -> Option<Value> {
    if body.is_empty() {
        return None;
    }
    let form_data = parse_query_string(body);
    if form_data.is_empty() {
        return None;
    }
    let pairs: IndexMap<HashKey, Value> = form_data
        .into_iter()
        .map(|(k, v)| (HashKey::String(k), Value::String(v)))
        .collect();
    Some(Value::Hash(Rc::new(RefCell::new(pairs))))
}

// Use centralized json_to_value from value module
use crate::interpreter::value::json_to_value;

/// Build a request hash from HTTP request data.
/// Uses thread-local cached keys to avoid repeated String allocations.
pub fn build_request_hash(
    method: &str,
    path: &str,
    params: HashMap<String, String>,
    query: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    body: &str,
) -> Value {
    build_request_hash_with_parsed(
        method,
        path,
        params,
        query,
        headers,
        body,
        ParsedBody::default(),
    )
}

/// Build a request hash with parsed body data.
pub fn build_request_hash_with_parsed(
    method: &str,
    path: &str,
    params: HashMap<String, String>,
    query: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    body: &str,
    parsed: ParsedBody,
) -> Value {
    // Build IndexMap directly from references with pre-allocated capacity
    let params_pairs: IndexMap<HashKey, Value> = if params.is_empty() {
        IndexMap::new()
    } else {
        let mut map = IndexMap::with_capacity(params.len());
        for (k, v) in params {
            map.insert(HashKey::String(k), Value::String(v));
        }
        map
    };

    let query_pairs: IndexMap<HashKey, Value> = if query.is_empty() {
        IndexMap::new()
    } else {
        let mut map = IndexMap::with_capacity(query.len());
        for (k, v) in query {
            map.insert(HashKey::String(k.clone()), Value::String(v.clone()));
        }
        map
    };

    let header_pairs: IndexMap<HashKey, Value> = if headers.is_empty() {
        IndexMap::new()
    } else {
        let mut map = IndexMap::with_capacity(headers.len());
        for (k, v) in headers {
            map.insert(HashKey::String(k.clone()), Value::String(v.clone()));
        }
        map
    };

    // Build unified "all" params - merges route params, query params, and body params
    let all_pairs = build_unified_params(&params_pairs, &query_pairs, &parsed);

    // Build request hash using cached keys (single thread_local access)
    let request_pairs: IndexMap<HashKey, Value> = REQUEST_KEYS.with(|keys| {
        let mut map = IndexMap::with_capacity(10);
        map.insert(keys.method.clone(), Value::String(method.to_string()));
        map.insert(keys.path.clone(), Value::String(path.to_string()));
        map.insert(
            keys.params.clone(),
            Value::Hash(Rc::new(RefCell::new(params_pairs))),
        );
        map.insert(
            keys.query.clone(),
            Value::Hash(Rc::new(RefCell::new(query_pairs))),
        );
        map.insert(
            keys.headers.clone(),
            Value::Hash(Rc::new(RefCell::new(header_pairs))),
        );
        map.insert(keys.body.clone(), Value::String(body.to_string()));
        map.insert(keys.json.clone(), parsed.json.unwrap_or(Value::Null));
        map.insert(keys.form.clone(), parsed.form.unwrap_or(Value::Null));
        map.insert(
            keys.files.clone(),
            parsed
                .files
                .unwrap_or(Value::Array(Rc::new(RefCell::new(Vec::new())))),
        );
        map.insert(
            keys.all.clone(),
            Value::Hash(Rc::new(RefCell::new(all_pairs))),
        );
        map
    });

    Value::Hash(Rc::new(RefCell::new(request_pairs)))
}

/// Build unified params by merging route params, query params, and body params.
/// Body params take precedence, followed by query params, then route params.
fn build_unified_params(
    route_params: &IndexMap<HashKey, Value>,
    query_params: &IndexMap<HashKey, Value>,
    parsed: &ParsedBody,
) -> IndexMap<HashKey, Value> {
    let mut all: IndexMap<HashKey, Value> =
        IndexMap::with_capacity(route_params.len() + query_params.len());

    // Start with route params
    for (k, v) in route_params {
        all.insert(k.clone(), v.clone());
    }

    // Add query params (override route params)
    for (k, v) in query_params {
        all.insert(k.clone(), v.clone());
    }

    // Add body params from JSON (highest priority)
    if let Some(Value::Hash(hash)) = parsed.json.as_ref() {
        for (k, v) in hash.borrow().iter() {
            all.insert(k.clone(), v.clone());
        }
    }

    // Add body params from form (if no JSON)
    if parsed.json.is_none() {
        if let Some(Value::Hash(hash)) = parsed.form.as_ref() {
            for (k, v) in hash.borrow().iter() {
                all.insert(k.clone(), v.clone());
            }
        }
    }

    all
}

/// Extract response data from a response hash returned by a handler.
/// Checks fast-path thread-local first (set by render_json/render_text).
pub fn extract_response(response: &Value) -> (u16, HashMap<String, String>, String) {
    // Fast path: if render_json/render_text set a pre-built response, use it directly
    if let Some(fast) = take_fast_path_response() {
        return (fast.status, fast.headers.into_iter().collect(), fast.body);
    }

    let mut status = 200u16;
    let mut headers = HashMap::new();
    let mut body = String::new();

    if let Value::Hash(hash) = response {
        for (k, v) in hash.borrow().iter() {
            if let HashKey::String(key) = k {
                match key.as_str() {
                    "status" => {
                        if let Value::Int(s) = v {
                            status = *s as u16;
                        }
                    }
                    "headers" => {
                        if let Value::Hash(h) = v {
                            for (hk, hv) in h.borrow().iter() {
                                if let (HashKey::String(k), Value::String(v)) = (hk, hv) {
                                    headers.insert(k.clone(), v.clone());
                                }
                            }
                        }
                    }
                    "body" => {
                        body = match v {
                            Value::String(s) => s.clone(),
                            _ => format!("{}", v),
                        };
                    }
                    _ => {}
                }
            }
        }
    } else {
        // If not a hash, use the value as the body
        body = format!("{}", response);
    }

    (status, headers, body)
}

/// Register HTTP server functions in the given environment.
pub fn register_server_builtins(env: &mut Environment) {
    // http_server_get(path, handler_name) - Register GET route
    env.define(
        "http_server_get".to_string(),
        Value::NativeFunction(NativeFunction::new("http_server_get", Some(2), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_get() expects string path, got {}",
                        other.type_name()
                    ))
                }
            };

            let handler_name = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_get() expects string handler name, got {}",
                        other.type_name()
                    ))
                }
            };

            set_direct_routes_mode();
            register_route("GET", &path, handler_name, Vec::new());
            Ok(Value::Null)
        })),
    );

    // http_server_post(path, handler_name) - Register POST route
    env.define(
        "http_server_post".to_string(),
        Value::NativeFunction(NativeFunction::new("http_server_post", Some(2), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_post() expects string path, got {}",
                        other.type_name()
                    ))
                }
            };

            let handler_name = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_post() expects string handler name, got {}",
                        other.type_name()
                    ))
                }
            };

            register_route("POST", &path, handler_name, Vec::new());
            Ok(Value::Null)
        })),
    );

    // http_server_put(path, handler_name) - Register PUT route
    env.define(
        "http_server_put".to_string(),
        Value::NativeFunction(NativeFunction::new("http_server_put", Some(2), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_put() expects string path, got {}",
                        other.type_name()
                    ))
                }
            };

            let handler_name = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_put() expects string handler name, got {}",
                        other.type_name()
                    ))
                }
            };

            register_route("PUT", &path, handler_name, Vec::new());
            Ok(Value::Null)
        })),
    );

    // http_server_delete(path, handler_name) - Register DELETE route
    env.define(
        "http_server_delete".to_string(),
        Value::NativeFunction(NativeFunction::new("http_server_delete", Some(2), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_delete() expects string path, got {}",
                        other.type_name()
                    ))
                }
            };

            let handler_name = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_delete() expects string handler name, got {}",
                        other.type_name()
                    ))
                }
            };

            register_route("DELETE", &path, handler_name, Vec::new());
            Ok(Value::Null)
        })),
    );

    // http_server_route(method, path, handler_name) - Register generic route
    env.define(
        "http_server_route".to_string(),
        Value::NativeFunction(NativeFunction::new("http_server_route", Some(3), |args| {
            let method = match &args[0] {
                Value::String(s) => s.to_uppercase(),
                other => {
                    return Err(format!(
                        "http_server_route() expects string method, got {}",
                        other.type_name()
                    ))
                }
            };

            let path = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_route() expects string path, got {}",
                        other.type_name()
                    ))
                }
            };

            let handler_name = match &args[2] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "http_server_route() expects string handler name, got {}",
                        other.type_name()
                    ))
                }
            };

            register_route(&method, &path, handler_name, Vec::new());
            Ok(Value::Null)
        })),
    );

    // http_server_listen(port) - This is a marker function.
    // The actual server loop is implemented in the executor because it needs
    // access to the interpreter to call handler functions.
    // This function just stores the port and returns a special marker.
    env.define(
        "http_server_listen".to_string(),
        Value::NativeFunction(NativeFunction::new("http_server_listen", Some(1), |args| {
            let port = match &args[0] {
                Value::Int(p) => *p as u16,
                other => {
                    return Err(format!(
                        "http_server_listen() expects integer port, got {}",
                        other.type_name()
                    ))
                }
            };

            // Return a special marker hash that the executor will recognize
            let mut marker: IndexMap<HashKey, Value> = IndexMap::new();
            marker.insert(
                HashKey::String("__server_listen__".to_string()),
                Value::Bool(true),
            );
            marker.insert(HashKey::String("port".to_string()), Value::Int(port as i64));

            Ok(Value::Hash(Rc::new(RefCell::new(marker))))
        })),
    );
}

/// Check if a value is a server listen marker.
pub fn is_server_listen_marker(value: &Value) -> Option<u16> {
    if let Value::Hash(hash) = value {
        let hash = hash.borrow();
        for (k, v) in hash.iter() {
            if let HashKey::String(key) = k {
                if key == "__server_listen__" {
                    if let Value::Bool(true) = v {
                        // Find the port
                        for (k2, v2) in hash.iter() {
                            if let HashKey::String(key2) = k2 {
                                if key2 == "port" {
                                    if let Value::Int(port) = v2 {
                                        return Some(*port as u16);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

// Get a reference to the global WebSocket registry from the websocket module.
pub fn get_ws_registry() -> std::sync::Arc<crate::serve::websocket::WebSocketRegistry> {
    crate::serve::websocket::get_ws_registry()
}

/// Register WebSocket server functions in the given environment.
pub fn register_websocket_builtins(env: &mut Environment) {
    // Note: The websocket() DSL function is defined in routes.sl via router_websocket()
    // WebSocket routes are registered using the DSL: websocket("/path", "controller#handler")

    // ws_send(connection_id, message) - Send message to a specific client
    env.define(
        "ws_send".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_send", Some(2), |args| {
            let connection_id = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_send() expects string connection_id, got {}",
                        other.type_name()
                    ))
                }
            };

            let message = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_send() expects string message, got {}",
                        other.type_name()
                    ))
                }
            };

            let registry = get_ws_registry();
            let uuid: uuid::Uuid = connection_id.parse().map_err(|_| "Invalid UUID format")?;

            // Spawn async task to send message
            let registry_clone = registry.clone();
            tokio::spawn(async move {
                let _ = registry_clone.send_to(&uuid, &message).await;
            });

            Ok(Value::Null)
        })),
    );

    // ws_broadcast(message) - Broadcast message to all clients
    env.define(
        "ws_broadcast".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_broadcast", Some(1), |args| {
            let message = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_broadcast() expects string message, got {}",
                        other.type_name()
                    ))
                }
            };

            let registry = get_ws_registry();
            let registry_clone = registry.clone();
            tokio::spawn(async move {
                registry_clone.broadcast_all(&message).await;
            });

            Ok(Value::Null)
        })),
    );

    // ws_broadcast_room(channel, message) - Broadcast message to a channel
    env.define(
        "ws_broadcast_room".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_broadcast_room", Some(2), |args| {
            let channel = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_broadcast_room() expects string channel, got {}",
                        other.type_name()
                    ))
                }
            };

            let message = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_broadcast_room() expects string message, got {}",
                        other.type_name()
                    ))
                }
            };

            let registry = get_ws_registry();
            let registry_clone = registry.clone();
            tokio::spawn(async move {
                registry_clone
                    .broadcast_to_channel(&channel, &message)
                    .await;
            });

            Ok(Value::Null)
        })),
    );

    // ws_join(channel) - Join the current connection to a channel
    // Note: This needs to be called from within a WebSocket handler context
    env.define(
        "ws_join".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_join", Some(1), |args| {
            let _channel = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_join() expects string channel, got {}",
                        other.type_name()
                    ))
                }
            };

            // This would need the current connection ID
            // For now, return null - actual implementation needs context
            Ok(Value::Null)
        })),
    );

    // ws_leave(channel) - Leave a channel
    env.define(
        "ws_leave".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_leave", Some(1), |args| {
            let _channel = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_leave() expects string channel, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(Value::Null)
        })),
    );

    // ws_clients() - Get all connected client IDs
    env.define(
        "ws_clients".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_clients", Some(0), |_args| {
            // Return an empty hash for now
            let pairs: IndexMap<HashKey, Value> = IndexMap::new();
            Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
        })),
    );

    // ws_clients_in(channel) - Get clients in a specific channel
    env.define(
        "ws_clients_in".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_clients_in", Some(1), |args| {
            let _channel = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_clients_in() expects string channel, got {}",
                        other.type_name()
                    ))
                }
            };

            // Return empty for now since this requires async
            let pairs: IndexMap<HashKey, Value> = IndexMap::new();
            Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
        })),
    );

    // ws_close(connection_id, reason) - Close a specific connection
    env.define(
        "ws_close".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_close", Some(2), |args| {
            let connection_id = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_close() expects string connection_id, got {}",
                        other.type_name()
                    ))
                }
            };

            let reason = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_close() expects string reason, got {}",
                        other.type_name()
                    ))
                }
            };

            let registry = get_ws_registry();
            let uuid: uuid::Uuid = connection_id.parse().map_err(|_| "Invalid UUID format")?;
            let registry_clone = registry.clone();
            tokio::spawn(async move {
                registry_clone.close(&uuid, &reason).await;
            });

            Ok(Value::Null)
        })),
    );

    // ws_count() - Get the number of active connections
    env.define(
        "ws_count".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_count", Some(0), |_args| {
            // Return 0 for now since this requires async
            Ok(Value::Int(0))
        })),
    );

    // ws_list_presence(channel) - Get all users in a room
    // Returns: [{ user_id, metas: [{ connection_id, state, phx_ref, online_at, ...extra }] }, ...]
    env.define(
        "ws_list_presence".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_list_presence", Some(1), |args| {
            let channel = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_list_presence() expects string channel, got {}",
                        other.type_name()
                    ))
                }
            };

            let registry = get_ws_registry();

            // We need to block on the async operation to get the result
            // Use a channel to get the result back synchronously
            let (tx, rx) = std::sync::mpsc::channel();
            let registry_clone = registry.clone();
            tokio::spawn(async move {
                let presences = registry_clone.list_presence(&channel).await;
                let _ = tx.send(presences);
            });

            // Wait for result with timeout
            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(presences) => {
                    // Convert to Soli array of hashes
                    let result: Vec<Value> = presences
                        .iter()
                        .map(|p| {
                            let metas: Vec<Value> = p
                                .metas
                                .iter()
                                .map(|m| {
                                    let mut meta_map: IndexMap<HashKey, Value> = IndexMap::new();
                                    meta_map.insert(
                                        HashKey::String("connection_id".to_string()),
                                        Value::String(m.connection_id.to_string()),
                                    );
                                    meta_map.insert(
                                        HashKey::String("phx_ref".to_string()),
                                        Value::String(m.phx_ref.clone()),
                                    );
                                    meta_map.insert(
                                        HashKey::String("state".to_string()),
                                        Value::String(m.state.clone()),
                                    );
                                    meta_map.insert(
                                        HashKey::String("online_at".to_string()),
                                        Value::Int(m.online_at as i64),
                                    );
                                    // Add extra fields
                                    for (k, v) in &m.extra {
                                        meta_map.insert(
                                            HashKey::String(k.clone()),
                                            Value::String(v.clone()),
                                        );
                                    }
                                    Value::Hash(Rc::new(RefCell::new(meta_map)))
                                })
                                .collect();

                            let mut user_map: IndexMap<HashKey, Value> = IndexMap::new();
                            user_map.insert(
                                HashKey::String("user_id".to_string()),
                                Value::String(p.user_id.clone()),
                            );
                            user_map.insert(
                                HashKey::String("metas".to_string()),
                                Value::Array(Rc::new(RefCell::new(metas))),
                            );
                            Value::Hash(Rc::new(RefCell::new(user_map)))
                        })
                        .collect();

                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                Err(_) => Ok(Value::Array(Rc::new(RefCell::new(Vec::new())))),
            }
        })),
    );

    // ws_presence_count(channel) - Get user count in a room (unique users, not connections)
    // Returns: number
    env.define(
        "ws_presence_count".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_presence_count", Some(1), |args| {
            let channel = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_presence_count() expects string channel, got {}",
                        other.type_name()
                    ))
                }
            };

            let registry = get_ws_registry();

            let (tx, rx) = std::sync::mpsc::channel();
            let registry_clone = registry.clone();
            tokio::spawn(async move {
                let count = registry_clone.presence_count(&channel).await;
                let _ = tx.send(count);
            });

            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(count) => Ok(Value::Int(count as i64)),
                Err(_) => Ok(Value::Int(0)),
            }
        })),
    );

    // ws_get_presence(channel, user_id) - Get specific user's presence in a room
    // Returns: { user_id, metas: [...] } or null
    env.define(
        "ws_get_presence".to_string(),
        Value::NativeFunction(NativeFunction::new("ws_get_presence", Some(2), |args| {
            let channel = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_get_presence() expects string channel, got {}",
                        other.type_name()
                    ))
                }
            };

            let user_id = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ws_get_presence() expects string user_id, got {}",
                        other.type_name()
                    ))
                }
            };

            let registry = get_ws_registry();

            let (tx, rx) = std::sync::mpsc::channel();
            let registry_clone = registry.clone();
            tokio::spawn(async move {
                let presence = registry_clone.get_user_presence(&channel, &user_id).await;
                let _ = tx.send(presence);
            });

            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(Some(presence)) => {
                    let metas: Vec<Value> = presence
                        .metas
                        .iter()
                        .map(|m| {
                            let mut meta_map: IndexMap<HashKey, Value> = IndexMap::new();
                            meta_map.insert(
                                HashKey::String("connection_id".to_string()),
                                Value::String(m.connection_id.to_string()),
                            );
                            meta_map.insert(
                                HashKey::String("phx_ref".to_string()),
                                Value::String(m.phx_ref.clone()),
                            );
                            meta_map.insert(
                                HashKey::String("state".to_string()),
                                Value::String(m.state.clone()),
                            );
                            meta_map.insert(
                                HashKey::String("online_at".to_string()),
                                Value::Int(m.online_at as i64),
                            );
                            for (k, v) in &m.extra {
                                meta_map
                                    .insert(HashKey::String(k.clone()), Value::String(v.clone()));
                            }
                            Value::Hash(Rc::new(RefCell::new(meta_map)))
                        })
                        .collect();

                    let mut user_map: IndexMap<HashKey, Value> = IndexMap::new();
                    user_map.insert(
                        HashKey::String("user_id".to_string()),
                        Value::String(presence.user_id),
                    );
                    user_map.insert(
                        HashKey::String("metas".to_string()),
                        Value::Array(Rc::new(RefCell::new(metas))),
                    );

                    Ok(Value::Hash(Rc::new(RefCell::new(user_map))))
                }
                _ => Ok(Value::Null),
            }
        })),
    );
}
