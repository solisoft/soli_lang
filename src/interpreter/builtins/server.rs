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
    pub handler_name: String,  // Store function name instead of Value
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
    pub static ROUTES: RefCell<Vec<Route>> = RefCell::new(Vec::new());
}

// Track if routes are "direct" (http_server_get/post) vs MVC (get/post DSL)
static DIRECT_ROUTES_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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

/// Build a request hash from HTTP request data.
pub fn build_request_hash(
    method: &str,
    path: &str,
    params: HashMap<String, String>,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
) -> Value {
    let params_pairs: Vec<(Value, Value)> = params
        .into_iter()
        .map(|(k, v)| (Value::String(k), Value::String(v)))
        .collect();

    let query_pairs: Vec<(Value, Value)> = query
        .into_iter()
        .map(|(k, v)| (Value::String(k), Value::String(v)))
        .collect();

    let header_pairs: Vec<(Value, Value)> = headers
        .into_iter()
        .map(|(k, v)| (Value::String(k), Value::String(v)))
        .collect();

    let request_pairs: Vec<(Value, Value)> = vec![
        (
            Value::String("method".to_string()),
            Value::String(method.to_string()),
        ),
        (
            Value::String("path".to_string()),
            Value::String(path.to_string()),
        ),
        (
            Value::String("params".to_string()),
            Value::Hash(Rc::new(RefCell::new(params_pairs))),
        ),
        (
            Value::String("query".to_string()),
            Value::Hash(Rc::new(RefCell::new(query_pairs))),
        ),
        (
            Value::String("headers".to_string()),
            Value::Hash(Rc::new(RefCell::new(header_pairs))),
        ),
        (Value::String("body".to_string()), Value::String(body)),
    ];

    Value::Hash(Rc::new(RefCell::new(request_pairs)))
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
    // Note: The websocket() DSL function is defined in routes.soli via router_websocket()
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
