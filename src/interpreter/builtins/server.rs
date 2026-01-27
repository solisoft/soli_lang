//! HTTP server built-in functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

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
                .or_insert_with(Vec::new)
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
pub fn match_path(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    // Fast path: exact match (no params)
    if pattern == path {
        return Some(HashMap::new());
    }

    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();

    if pattern_parts.len() != path_parts.len() {
        return None;
    }

    let mut params = HashMap::new();

    for (pat, actual) in pattern_parts.iter().zip(path_parts.iter()) {
        if let Some(param_name) = pat.strip_prefix(':') {
            params.insert(param_name.to_string(), actual.to_string());
        } else if pat != actual {
            // Literal mismatch
            return None;
        }
    }

    Some(params)
}

/// Parse query string into a hash map.
pub fn parse_query_string(query: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    if query.is_empty() {
        return result;
    }

    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            // URL decode (basic)
            let decoded_value = value
                .replace("%20", " ")
                .replace("+", " ")
                .replace("%2F", "/")
                .replace("%3A", ":")
                .replace("%3F", "?")
                .replace("%3D", "=")
                .replace("%26", "&");
            result.insert(key.to_string(), decoded_value);
        } else {
            result.insert(pair.to_string(), String::new());
        }
    }

    result
}

// Pre-allocated static keys for request hash to avoid repeated String allocations
thread_local! {
    static KEY_METHOD: Value = Value::String("method".to_string());
    static KEY_PATH: Value = Value::String("path".to_string());
    static KEY_PARAMS: Value = Value::String("params".to_string());
    static KEY_QUERY: Value = Value::String("query".to_string());
    static KEY_HEADERS: Value = Value::String("headers".to_string());
    static KEY_BODY: Value = Value::String("body".to_string());
    static KEY_JSON: Value = Value::String("json".to_string());
    static KEY_FORM: Value = Value::String("form".to_string());
    static KEY_FILES: Value = Value::String("files".to_string());
    static KEY_ALL: Value = Value::String("all".to_string());
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
    let pairs: Vec<(Value, Value)> = form_data
        .into_iter()
        .map(|(k, v)| (Value::String(k), Value::String(v)))
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
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
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
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
    parsed: ParsedBody,
) -> Value {
    // Pre-allocate with known capacity
    let params_pairs: Vec<(Value, Value)> = if params.is_empty() {
        Vec::new()
    } else {
        params
            .into_iter()
            .map(|(k, v)| (Value::String(k), Value::String(v)))
            .collect()
    };

    let query_pairs: Vec<(Value, Value)> = if query.is_empty() {
        Vec::new()
    } else {
        query
            .into_iter()
            .map(|(k, v)| (Value::String(k), Value::String(v)))
            .collect()
    };

    let header_pairs: Vec<(Value, Value)> = if headers.is_empty() {
        Vec::new()
    } else {
        headers
            .into_iter()
            .map(|(k, v)| (Value::String(k), Value::String(v)))
            .collect()
    };

    // Build unified "all" params - merges route params, query params, and body params
    let all_pairs = build_unified_params(&params_pairs, &query_pairs, &parsed);

    // Build request hash using cached keys
    let request_pairs: Vec<(Value, Value)> = KEY_METHOD.with(|key_method| {
        KEY_PATH.with(|key_path| {
            KEY_PARAMS.with(|key_params| {
                KEY_QUERY.with(|key_query| {
                    KEY_HEADERS.with(|key_headers| {
                        KEY_BODY.with(|key_body| {
                            KEY_JSON.with(|key_json| {
                                KEY_FORM.with(|key_form| {
                                    KEY_FILES.with(|key_files| {
                                        KEY_ALL.with(|key_all| {
                                            vec![
                                                (
                                                    key_method.clone(),
                                                    Value::String(method.to_string()),
                                                ),
                                                (key_path.clone(), Value::String(path.to_string())),
                                                (
                                                    key_params.clone(),
                                                    Value::Hash(Rc::new(RefCell::new(
                                                        params_pairs,
                                                    ))),
                                                ),
                                                (
                                                    key_query.clone(),
                                                    Value::Hash(Rc::new(RefCell::new(query_pairs))),
                                                ),
                                                (
                                                    key_headers.clone(),
                                                    Value::Hash(Rc::new(RefCell::new(
                                                        header_pairs,
                                                    ))),
                                                ),
                                                (key_body.clone(), Value::String(body)),
                                                (
                                                    key_json.clone(),
                                                    parsed.json.unwrap_or(Value::Null),
                                                ),
                                                (
                                                    key_form.clone(),
                                                    parsed.form.unwrap_or(Value::Null),
                                                ),
                                                (
                                                    key_files.clone(),
                                                    parsed.files.unwrap_or(Value::Array(Rc::new(
                                                        RefCell::new(Vec::new()),
                                                    ))),
                                                ),
                                                (
                                                    key_all.clone(),
                                                    Value::Hash(Rc::new(RefCell::new(all_pairs))),
                                                ),
                                            ]
                                        })
                                    })
                                })
                            })
                        })
                    })
                })
            })
        })
    });

    Value::Hash(Rc::new(RefCell::new(request_pairs)))
}

/// Build unified params by merging route params, query params, and body params.
/// Body params take precedence, followed by query params, then route params.
fn build_unified_params(
    route_params: &[(Value, Value)],
    query_params: &[(Value, Value)],
    parsed: &ParsedBody,
) -> Vec<(Value, Value)> {
    let mut all: Vec<(Value, Value)> = Vec::new();

    // Start with route params
    for pair in route_params {
        all.push(pair.clone());
    }

    // Add query params (override route params)
    for pair in query_params {
        // Remove existing key if present
        all.retain(|(k, _)| *k != pair.0);
        all.push(pair.clone());
    }

    // Add body params from JSON (highest priority)
    if let Some(ref json) = parsed.json {
        if let Value::Hash(hash) = json {
            for (k, v) in hash.borrow().iter() {
                // Remove existing key if present
                all.retain(|(key, _)| *key != *k);
                all.push((k.clone(), v.clone()));
            }
        }
    }

    // Add body params from form (if no JSON)
    if parsed.json.is_none() {
        if let Some(ref form) = parsed.form {
            if let Value::Hash(hash) = form {
                for (k, v) in hash.borrow().iter() {
                    // Remove existing key if present
                    all.retain(|(key, _)| *key != *k);
                    all.push((k.clone(), v.clone()));
                }
            }
        }
    }

    all
}

/// Extract response data from a response hash returned by a handler.
pub fn extract_response(response: &Value) -> (u16, HashMap<String, String>, String) {
    let mut status = 200u16;
    let mut headers = HashMap::new();
    let mut body = String::new();

    if let Value::Hash(hash) = response {
        for (k, v) in hash.borrow().iter() {
            if let Value::String(key) = k {
                match key.as_str() {
                    "status" => {
                        if let Value::Int(s) = v {
                            status = *s as u16;
                        }
                    }
                    "headers" => {
                        if let Value::Hash(h) = v {
                            for (hk, hv) in h.borrow().iter() {
                                if let (Value::String(k), Value::String(v)) = (hk, hv) {
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
            let marker: Vec<(Value, Value)> = vec![
                (
                    Value::String("__server_listen__".to_string()),
                    Value::Bool(true),
                ),
                (Value::String("port".to_string()), Value::Int(port as i64)),
            ];

            Ok(Value::Hash(Rc::new(RefCell::new(marker))))
        })),
    );
}

/// Check if a value is a server listen marker.
pub fn is_server_listen_marker(value: &Value) -> Option<u16> {
    if let Value::Hash(hash) = value {
        let hash = hash.borrow();
        for (k, v) in hash.iter() {
            if let Value::String(key) = k {
                if key == "__server_listen__" {
                    if let Value::Bool(true) = v {
                        // Find the port
                        for (k2, v2) in hash.iter() {
                            if let Value::String(key2) = k2 {
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
            let pairs: Vec<(Value, Value)> = Vec::new();
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
            let pairs: Vec<(Value, Value)> = Vec::new();
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
}
