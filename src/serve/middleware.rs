//! Middleware support for the MVC framework.
//!
//! Middleware functions intercept requests before they reach route handlers.
//! They can modify requests, short-circuit with responses, or pass through.
//!
//! ## Middleware Convention
//!
//! Middleware files are placed in `app/middleware/` and define functions that:
//! - Take a request hash as input
//! - Return a result hash with either:
//!   - `{"continue": true, "request": modified_request}` - pass to next middleware
//!   - `{"continue": false, "response": {...}}` - short-circuit with response
//!
//! ## Example Middleware
//!
//! ```soli
//! // app/middleware/auth.sl
//! fn authenticate(req: Any) -> Any {
//!     let token = req["headers"]["Authorization"];
//!     if (token == "") {
//!         return {
//!             "continue": false,
//!             "response": {"status": 401, "body": "Unauthorized"}
//!         };
//!     }
//!     return {"continue": true, "request": req};
//! }
//! ```
//!
//! ## Middleware Types
//!
//! ### 1. Global-Only Middleware
//!
//! Runs for ALL routes and cannot be scoped.
//!
//! ```soli
//! // app/middleware/cors.sl
//! // order: 5
//! // global_only: true
//!
//! fn add_cors_headers(req: Any) -> Any {
//!     // This runs for ALL requests
//!     // ...
//! }
//! ```
//!
//! ### 2. Scope-Only Middleware
//!
//! Does NOT run globally. Only runs when explicitly scoped.
//!
//! ```soli
//! // app/middleware/auth.sl
//! // order: 20
//! // scope_only: true
//!
//! fn authenticate(req: Any) -> Any {
//!     // This only runs when explicitly scoped
//!     // ...
//! }
//! ```
//!
//! ### 3. Regular Middleware
//!
//! Runs globally by default, but can also be scoped.
//!
//! ```soli
//! // app/middleware/validation.sl
//! // order: 15
//!
//! fn validate_request(req: Any) -> Any {
//!     // This runs globally by default
//!     // Can also be scoped to specific routes
//! }
//! ```
//!
//! In routes.sl, use the `middleware()` helper to apply scoped middleware:
//!
//! ```soli
//! // Only apply authentication to these routes
//! middleware("authenticate", -> {
//!     get("/admin", "admin#index");
//!     get("/admin/users", "admin#users");
//! });
//! ```
//!
//! ## Options
//!
//! - `// order: N` - Execution order (lower runs first, default: 100)
//! - `// global_only: true` - Only run globally, cannot be scoped
//! - `// scope_only: true` - Only run when explicitly scoped, never globally

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::error::RuntimeError;
use crate::interpreter::value::{HashKey, Value};
use crate::span::Span;

/// A registered middleware with its handler function.
#[derive(Clone)]
pub struct Middleware {
    pub name: String,
    pub handler: Value,
    pub order: i32,
    /// If true, this middleware only runs globally (not scoped to specific routes)
    pub global_only: bool,
    /// If true, this middleware only runs when explicitly scoped (not globally by default)
    pub scope_only: bool,
}

// Middleware registry stored in thread-local storage.
// Middleware contains Value (which uses Rc), so must be accessed from interpreter thread only.
thread_local! {
    pub static MIDDLEWARE: RefCell<Vec<Middleware>> = const { RefCell::new(Vec::new()) };
}

/// Clear all registered middleware.
pub fn clear_middleware() {
    MIDDLEWARE.with(|mw| mw.borrow_mut().clear());
}

/// Register a middleware function.
pub fn register_middleware(name: &str, handler: Value, order: i32) {
    register_middleware_with_options(name, handler, order, false, false);
}

/// Register a middleware function with options.
pub fn register_middleware_with_options(
    name: &str,
    handler: Value,
    order: i32,
    global_only: bool,
    scope_only: bool,
) {
    MIDDLEWARE.with(|mw| {
        let mut middleware = mw.borrow_mut();
        middleware.push(Middleware {
            name: name.to_string(),
            handler,
            order,
            global_only,
            scope_only,
        });
        // Sort by order (lower order runs first)
        middleware.sort_by_key(|m| m.order);
    });
}

/// Get all registered middleware in execution order (must be called from interpreter thread).
/// Note: This clones the middleware Vec. For performance-critical paths, use
/// `with_middleware()` to iterate without cloning.
pub fn get_middleware() -> Vec<Middleware> {
    MIDDLEWARE.with(|mw| {
        let mw = mw.borrow();
        if mw.is_empty() {
            Vec::new()
        } else {
            mw.clone()
        }
    })
}

/// Execute a closure with a reference to the middleware list (avoids cloning).
/// Returns None if there's no middleware, otherwise returns the closure result.
#[inline]
pub fn with_middleware<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&[Middleware]) -> R,
{
    MIDDLEWARE.with(|mw| {
        let mw = mw.borrow();
        if mw.is_empty() {
            None
        } else {
            Some(f(&mw))
        }
    })
}

/// Check if there's any middleware registered (fast path).
#[inline]
pub fn has_middleware() -> bool {
    MIDDLEWARE.with(|mw| !mw.borrow().is_empty())
}

/// Get a middleware by name (must be called from interpreter thread).
pub fn get_middleware_by_name(name: &str) -> Option<Middleware> {
    MIDDLEWARE.with(|mw| {
        let middleware = mw.borrow();
        middleware.iter().find(|m| m.name == name).cloned()
    })
}

/// Result of middleware execution.
pub enum MiddlewareResult {
    /// Continue to next middleware/handler with (possibly modified) request
    Continue(Value),
    /// Short-circuit with a response
    Response(Value),
    /// Error during middleware execution
    Error(String),
}

/// Extract the middleware result from a handler's return value.
///
/// Expected format:
/// - `{"continue": true, "request": {...}}` - continue processing
/// - `{"continue": false, "response": {...}}` - short-circuit
pub fn extract_middleware_result(result: &Value) -> MiddlewareResult {
    if let Value::Hash(hash) = result {
        let hash = hash.borrow();

        // Look for "continue" key
        let mut should_continue = true;
        let mut request = None;
        let mut response = None;

        for (k, v) in hash.iter() {
            if let HashKey::String(key) = k {
                match key.as_str() {
                    "continue" => {
                        if let Value::Bool(b) = v {
                            should_continue = *b;
                        }
                    }
                    "request" => {
                        request = Some(v.clone());
                    }
                    "response" => {
                        response = Some(v.clone());
                    }
                    _ => {}
                }
            }
        }

        if should_continue {
            // Continue with the request (modified or original)
            match request {
                Some(req) => MiddlewareResult::Continue(req),
                None => MiddlewareResult::Error(
                    "Middleware returned continue=true but no request".to_string(),
                ),
            }
        } else {
            // Short-circuit with response
            match response {
                Some(resp) => MiddlewareResult::Response(resp),
                None => MiddlewareResult::Error(
                    "Middleware returned continue=false but no response".to_string(),
                ),
            }
        }
    } else {
        // If not a hash, treat as an error
        MiddlewareResult::Error(format!(
            "Middleware must return a hash, got {}",
            result.type_name()
        ))
    }
}

/// Scan for middleware files in the middleware directory.
pub fn scan_middleware_files(middleware_dir: &Path) -> Result<Vec<PathBuf>, RuntimeError> {
    let mut files = Vec::new();

    if !middleware_dir.exists() {
        return Ok(files);
    }

    for entry in std::fs::read_dir(middleware_dir).map_err(|e| RuntimeError::General {
        message: format!("Failed to read middleware directory: {}", e),
        span: Span::default(),
    })? {
        let entry = entry.map_err(|e| RuntimeError::General {
            message: format!("Failed to read directory entry: {}", e),
            span: Span::default(),
        })?;

        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sl") {
            files.push(path);
        }
    }

    // Sort by filename for predictable ordering
    files.sort();

    Ok(files)
}

/// Extract middleware function names from source code.
/// Returns (function_name, order, global_only, scope_only) tuples.
/// Order is determined by a comment like `// order: 10` or `# order: 10` before the function,
/// or defaults to 100.
/// Global-only is determined by a comment like `// global_only: true` or `# global_only: true`.
/// Scope-only is determined by a comment like `// scope_only: true` or `# scope_only: true`.
/// Function declarations can use either `fn` or `def` keyword.
pub fn extract_middleware_functions(source: &str) -> Vec<(String, i32, bool, bool)> {
    let mut functions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    let mut pending_order: Option<i32> = None;
    let mut pending_global_only: Option<bool> = None;
    let mut pending_scope_only: Option<bool> = None;

    for line in lines.iter() {
        let trimmed = line.trim();

        // Strip comment prefix (// or #) and get the directive content
        let comment_body = trimmed
            .strip_prefix("//")
            .or_else(|| trimmed.strip_prefix('#'))
            .map(|rest| rest.trim_start());

        if let Some(body) = comment_body {
            // Check for order directive
            if body.starts_with("order:") {
                if let Some(order_str) = body.split(':').nth(1) {
                    if let Ok(order) = order_str.trim().parse::<i32>() {
                        pending_order = Some(order);
                    }
                }
            }

            // Check for global_only directive
            if body.starts_with("global_only:") {
                if let Some(value_str) = body.split(':').nth(1) {
                    let value = value_str.trim().to_lowercase();
                    pending_global_only = Some(
                        value.starts_with("true")
                            || value.starts_with("1")
                            || value.starts_with("yes"),
                    );
                }
            }

            // Check for scope_only directive
            if body.starts_with("scope_only:") {
                if let Some(value_str) = body.split(':').nth(1) {
                    let value = value_str.trim().to_lowercase();
                    pending_scope_only = Some(
                        value.starts_with("true")
                            || value.starts_with("1")
                            || value.starts_with("yes"),
                    );
                }
            }
        }

        // Check for function declaration (fn or def)
        let func_rest = if trimmed.starts_with("fn ") {
            trimmed.strip_prefix("fn ")
        } else if trimmed.starts_with("def ") {
            trimmed.strip_prefix("def ")
        } else {
            None
        };

        if let Some(rest) = func_rest {
            if let Some(paren_pos) = rest.find('(') {
                let func_name = rest[..paren_pos].trim().to_string();

                // Skip private functions
                if !func_name.starts_with('_') {
                    let order = pending_order.unwrap_or(100);
                    let global_only = pending_global_only.unwrap_or(false);
                    let scope_only = pending_scope_only.unwrap_or(false);
                    functions.push((func_name, order, global_only, scope_only));
                }
            }
            pending_order = None;
            pending_global_only = None;
            pending_scope_only = None;
        }
    }

    functions
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use std::rc::Rc;

    #[test]
    fn test_extract_middleware_functions() {
        let source = r#"
// order: 10
fn authenticate(req: Any) -> Any {
    return {"continue": true, "request": req};
}

// order: 20
fn log_request(req: Any) -> Any {
    print("Request: " + req["path"]);
    return {"continue": true, "request": req};
}

fn default_order(req: Any) -> Any {
    return {"continue": true, "request": req};
}

fn _private_helper() {
    // should be skipped
}
"#;

        let functions = extract_middleware_functions(source);

        assert_eq!(functions.len(), 3);
        assert_eq!(functions[0], ("authenticate".to_string(), 10, false, false));
        assert_eq!(functions[1], ("log_request".to_string(), 20, false, false));
        assert_eq!(
            functions[2],
            ("default_order".to_string(), 100, false, false)
        );
    }

    #[test]
    fn test_extract_middleware_functions_def_and_hash_comments() {
        let source = r#"
# order: 10
# scope_only: true

def require_auth(req: Any) -> Any
    return {"continue": true, "request": req}
end

# order: 5
# global_only: true

def add_cors(req: Any) -> Any
    return {"continue": true, "request": req}
end

def _private_helper()
    # should be skipped
end
"#;

        let functions = extract_middleware_functions(source);

        assert_eq!(functions.len(), 2);
        assert_eq!(functions[0], ("require_auth".to_string(), 10, false, true));
        assert_eq!(functions[1], ("add_cors".to_string(), 5, true, false));
    }

    #[test]
    fn test_middleware_result_continue() {
        let mut request_map: IndexMap<HashKey, Value> = IndexMap::new();
        request_map.insert(
            HashKey::String("path".to_string()),
            Value::String("/test".to_string()),
        );
        let request = Value::Hash(Rc::new(RefCell::new(request_map)));

        let mut result_map: IndexMap<HashKey, Value> = IndexMap::new();
        result_map.insert(HashKey::String("continue".to_string()), Value::Bool(true));
        result_map.insert(HashKey::String("request".to_string()), request.clone());
        let result = Value::Hash(Rc::new(RefCell::new(result_map)));

        match extract_middleware_result(&result) {
            MiddlewareResult::Continue(_) => {}
            _ => panic!("Expected Continue result"),
        }
    }

    #[test]
    fn test_middleware_result_response() {
        let mut response_map: IndexMap<HashKey, Value> = IndexMap::new();
        response_map.insert(HashKey::String("status".to_string()), Value::Int(401));
        response_map.insert(
            HashKey::String("body".to_string()),
            Value::String("Unauthorized".to_string()),
        );
        let response = Value::Hash(Rc::new(RefCell::new(response_map)));

        let mut result_map: IndexMap<HashKey, Value> = IndexMap::new();
        result_map.insert(HashKey::String("continue".to_string()), Value::Bool(false));
        result_map.insert(HashKey::String("response".to_string()), response);
        let result = Value::Hash(Rc::new(RefCell::new(result_map)));

        match extract_middleware_result(&result) {
            MiddlewareResult::Response(_) => {}
            _ => panic!("Expected Response result"),
        }
    }
}
