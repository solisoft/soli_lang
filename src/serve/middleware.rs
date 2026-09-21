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

use uuid::Uuid;

use crate::error::RuntimeError;
use crate::interpreter::builtins::server::extract_response;
use crate::interpreter::value::{HashKey, Value};
use crate::interpreter::Interpreter;
use crate::span::Span;

use super::{
    error_logging, error_pages, error_response, middleware_log, phase_log, span_log, RequestData,
    ResponseData,
};

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
                match key.as_ref() {
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

// ---------------------------------------------------------------------------
// Running one
// ---------------------------------------------------------------------------

/// What running one middleware decided.
pub(crate) enum Step {
    /// Carry on to the next one with this request hash, which the middleware
    /// may have modified.
    Continue(Value),
    /// Stop here: this is the response. Either the middleware chose it
    /// (`{"continue": false, "response": …}`) or it failed.
    Halt(ResponseData),
}

/// Run one middleware and decide what happens next.
///
/// `handler` is the function to call and `preferred_name` the name to show if
/// it has none of its own — the two things that differed between the route's
/// own middleware and the global list, which otherwise ran through a hundred
/// identical lines each. They are one copy now.
///
/// Everything else here is error presentation, and it is most of the volume:
/// a middleware can fail in two ways (return `{"error": …}`, or raise), and
/// each is reported one way under `--dev` (the full error page, with the
/// interpreter environment) and another in production (a request id, the
/// context to stderr, and nothing else to the client).
pub(crate) fn run(
    interpreter: &mut Interpreter,
    data: &RequestData,
    handler: Value,
    preferred_name: Option<&str>,
    request_hash: Value,
    dev_mode: bool,
) -> Step {
    let (mw_name, mw_source, mw_span) = middleware_source_info(&handler, preferred_name);
    let call_result = invoke_middleware_with_frame(
        interpreter,
        &mw_name,
        mw_source.as_deref(),
        mw_span,
        handler,
        request_hash,
    );
    match call_result {
        Ok(result) => match extract_middleware_result(&result) {
            MiddlewareResult::Continue(modified_request) => Step::Continue(modified_request),
            MiddlewareResult::Response(resp) => {
                let (status, headers, body) = extract_response(resp);
                Step::Halt(ResponseData {
                    status,
                    headers,
                    body,
                })
            }
            MiddlewareResult::Error(err) => {
                if dev_mode {
                    let stack_trace = middleware_fallback_stack(&mw_name, mw_source.as_deref());
                    let request_id = Uuid::new_v4().to_string();
                    let env_json = interpreter.serialize_environment_for_debug();
                    error_logging::log_production_error(
                        &request_id,
                        data,
                        &err,
                        &stack_trace,
                        Some(&env_json),
                    );
                    let error_html =
                        error_pages::render_error_page(&err, interpreter, data, &stack_trace, None);
                    return Step::Halt(ResponseData {
                        status: 500,
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html.into_bytes(),
                    });
                }
                Step::Halt(middleware_prod_error_string(
                    interpreter,
                    data,
                    &mw_name,
                    mw_source.as_deref(),
                    &err,
                ))
            }
        },
        Err(e) => {
            if dev_mode {
                // Prefer the captured stack trace (populated by the inner
                // call_function) over interpreter.get_stack_trace(), then
                // fall back to a synthetic middleware frame so the error
                // page can still show the source file.
                let captured = e.breakpoint_stack_trace().map(|st| st.to_vec());
                let stack_trace = captured
                    .unwrap_or_else(|| middleware_fallback_stack(&mw_name, mw_source.as_deref()));
                let breakpoint_env = e.breakpoint_env_json();
                let fallback_env: Option<String> = if breakpoint_env.is_none() {
                    Some(interpreter.serialize_environment_for_debug())
                } else {
                    None
                };
                let env_for_log = breakpoint_env.or(fallback_env.as_deref());
                // Breakpoints are intentional debug pauses, not failures,
                // so don't emit the stderr error block for them.
                if !e.is_breakpoint() {
                    let request_id = Uuid::new_v4().to_string();
                    error_logging::log_production_error(
                        &request_id,
                        data,
                        &e.to_string(),
                        &stack_trace,
                        env_for_log,
                    );
                }
                let error_html = error_pages::render_error_page(
                    &e.to_string(),
                    interpreter,
                    data,
                    &stack_trace,
                    breakpoint_env,
                );
                return Step::Halt(ResponseData {
                    status: if e.is_breakpoint() { 200 } else { 500 },
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html.into_bytes(),
                });
            }
            Step::Halt(middleware_prod_error_runtime(
                interpreter,
                data,
                &mw_name,
                mw_source.as_deref(),
                &e,
            ))
        }
    }
}

/// Extract `(name, source_path, span)` metadata for a middleware handler.
///
/// If the handler is a `Value::Function`, its declared name/source/span are
/// used; otherwise we fall back to `preferred_name` (the registered name for
/// global middleware) or `"middleware"`.
fn middleware_source_info(
    handler: &Value,
    preferred_name: Option<&str>,
) -> (String, Option<String>, Span) {
    if let Value::Function(ref func) = handler {
        let name = if !func.name.is_empty() {
            func.name.clone()
        } else {
            preferred_name.unwrap_or("middleware").to_string()
        };
        let source = func.source_path.clone();
        let span = func.span.unwrap_or_else(|| Span::new(0, 0, 1, 1));
        (name, source, span)
    } else {
        (
            preferred_name.unwrap_or("middleware").to_string(),
            None,
            Span::new(0, 0, 1, 1),
        )
    }
}

/// Synthesize a single-frame stack trace for a middleware that failed before
/// (or while returning) a RuntimeError could be captured. Lets the dev error
/// page pick up the source file via its regex-based frame parser.
fn middleware_fallback_stack(name: &str, source_path: Option<&str>) -> Vec<String> {
    match source_path {
        Some(path) => vec![format!("{} at {}:1", name, path)],
        None => vec![format!("{} at unknown:1", name)],
    }
}

/// Invoke a middleware handler with an interpreter frame set up so errors
/// carry the middleware's source path in their captured stack trace.
fn invoke_middleware_with_frame(
    interpreter: &mut Interpreter,
    name: &str,
    source_path: Option<&str>,
    span: Span,
    handler: Value,
    request_hash: Value,
) -> Result<Value, RuntimeError> {
    let _phase = phase_log::PhaseTimer::start("middleware");
    let _span = span_log::SpanGuard::start(name, span_log::SpanKind::Middleware);

    // Clock only when something will read it: Prometheus (`SOLI_METRICS=1`)
    // or the --dev middleware log. Otherwise Instant::now is a syscall per
    // middleware on every request.
    let record_metrics = crate::metrics::metrics_enabled();
    let record_dev = middleware_log::is_enabled();
    let mw_start = (record_metrics || record_dev).then(std::time::Instant::now);

    interpreter.push_frame(name, span, source_path.map(|s| s.to_string()));
    if let Some(path) = source_path {
        interpreter.set_source_path(PathBuf::from(path));
    }
    let result = interpreter.call_value(handler, vec![request_hash], span);
    interpreter.pop_frame();

    if let Some(start) = mw_start {
        let elapsed = start.elapsed();
        if record_metrics {
            crate::metrics::Metrics::global().record_middleware(elapsed);
        }
        if record_dev {
            middleware_log::record(name, elapsed.as_micros() as u64);
        }
    }
    result
}

/// Build a production 500 response for a middleware that returned an
/// `Error(String)` result. Captures the synthetic middleware stack
/// frame and the interpreter's current environment, writes the full
/// context block to stderr, and embeds the same context in the
/// rendered HTML.
fn middleware_prod_error_string(
    interpreter: &Interpreter,
    data: &RequestData,
    mw_name: &str,
    mw_source: Option<&str>,
    err: &str,
) -> ResponseData {
    let request_id = Uuid::new_v4().to_string();
    let stack_trace = middleware_fallback_stack(mw_name, mw_source);
    let env_json = interpreter.serialize_environment_for_debug();
    error_logging::log_production_error(&request_id, data, err, &stack_trace, Some(&env_json));
    error_response::html(
        500,
        error_pages::render_production_error_page(500, err, &request_id),
    )
}

/// Build a production 500 response for a middleware that raised a
/// `RuntimeError`. Prefers the error's captured stack/env when present
/// (set by the inner interpreter frame) and falls back to a synthetic
/// middleware frame plus the current environment otherwise.
fn middleware_prod_error_runtime(
    interpreter: &Interpreter,
    data: &RequestData,
    mw_name: &str,
    mw_source: Option<&str>,
    e: &RuntimeError,
) -> ResponseData {
    let request_id = Uuid::new_v4().to_string();
    let error_msg = e.to_string();
    let stack_trace: Vec<String> = e
        .breakpoint_stack_trace()
        .map(|st| st.to_vec())
        .unwrap_or_else(|| middleware_fallback_stack(mw_name, mw_source));
    let env_json: String = e
        .breakpoint_env_json()
        .map(|s| s.to_string())
        .unwrap_or_else(|| interpreter.serialize_environment_for_debug());
    error_logging::log_production_error(
        &request_id,
        data,
        &error_msg,
        &stack_trace,
        Some(&env_json),
    );
    error_response::html(
        500,
        error_pages::render_production_error_page(500, &error_msg, &request_id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashPairs;
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
        let mut request_map: HashPairs = HashPairs::default();
        request_map.insert(
            HashKey::String("path".into()),
            Value::String("/test".into()),
        );
        let request = Value::Hash(Rc::new(RefCell::new(request_map)));

        let mut result_map: HashPairs = HashPairs::default();
        result_map.insert(HashKey::String("continue".into()), Value::Bool(true));
        result_map.insert(HashKey::String("request".into()), request.clone());
        let result = Value::Hash(Rc::new(RefCell::new(result_map)));

        match extract_middleware_result(&result) {
            MiddlewareResult::Continue(_) => {}
            _ => panic!("Expected Continue result"),
        }
    }

    #[test]
    fn test_middleware_result_response() {
        let mut response_map: HashPairs = HashPairs::default();
        response_map.insert(HashKey::String("status".into()), Value::Int(401));
        response_map.insert(
            HashKey::String("body".into()),
            Value::String("Unauthorized".into()),
        );
        let response = Value::Hash(Rc::new(RefCell::new(response_map)));

        let mut result_map: HashPairs = HashPairs::default();
        result_map.insert(HashKey::String("continue".into()), Value::Bool(false));
        result_map.insert(HashKey::String("response".into()), response);
        let result = Value::Hash(Rc::new(RefCell::new(result_map)));

        match extract_middleware_result(&result) {
            MiddlewareResult::Response(_) => {}
            _ => panic!("Expected Response result"),
        }
    }

    /// Regression test: a middleware that throws must produce an error whose
    /// captured stack trace includes the middleware's source path, so the dev
    /// error page can show the right file.
    #[test]
    fn test_middleware_error_carries_source_path() {
        use crate::interpreter::value::Function as ValueFunction;
        use crate::lexer::Scanner;
        use crate::parser::Parser;

        let source = r#"
            fn failing_middleware(req) {
                let x = undefined_variable
                return req
            }
        "#;

        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).unwrap();

        // Mark the parsed function as coming from a concrete middleware file
        // (the real server sets source_path when it loads middleware files).
        let handler_val = interpreter
            .environment
            .borrow()
            .get("failing_middleware")
            .unwrap();
        let handler_with_source = match handler_val {
            Value::Function(ref f) => {
                let mut cloned: ValueFunction = (**f).clone();
                cloned.source_path = Some("app/middleware/failing.sl".to_string());
                Value::Function(Rc::new(cloned))
            }
            _ => panic!("failing_middleware did not resolve to a function"),
        };

        let (name, source_path, span) = middleware_source_info(&handler_with_source, None);
        assert_eq!(name, "failing_middleware");
        assert_eq!(source_path.as_deref(), Some("app/middleware/failing.sl"));

        let request_hash = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));
        let err = invoke_middleware_with_frame(
            &mut interpreter,
            &name,
            source_path.as_deref(),
            span,
            handler_with_source,
            request_hash,
        )
        .expect_err("middleware should raise a runtime error");

        // The dispatch-time frame PLUS the inner call_function frame both
        // reference the middleware source path, so the captured trace is
        // guaranteed to expose it to render_error_page.
        let captured = err
            .breakpoint_stack_trace()
            .expect("thrown error should carry a captured stack trace");
        assert!(
            captured
                .iter()
                .any(|frame| frame.contains("app/middleware/failing.sl")),
            "captured stack trace should include the middleware source path; got {:?}",
            captured
        );
    }

    // ---------- running one ----------

    /// A `RequestData` with nothing interesting in it: the error paths below
    /// only read it to log, and the `response_tx` is never used.
    fn request_data() -> RequestData {
        let (response_tx, _rx) = tokio::sync::oneshot::channel();
        RequestData {
            method: std::borrow::Cow::Borrowed("GET"),
            path: "/probe".to_string(),
            query: Vec::new(),
            headers: hyper::HeaderMap::new(),
            body: String::new(),
            body_reservation: None,
            multipart_form: None,
            multipart_files: None,
            peer_ip: "127.0.0.1".to_string(),
            enqueued_at: None,
            replay: false,
            file_template: None,
            response_tx,
        }
    }

    /// An interpreter with `mw` defined, and the empty hash to hand it.
    fn middleware_from(source: &str) -> (Interpreter, Value, Value) {
        use crate::lexer::Scanner;
        use crate::parser::Parser;
        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).unwrap();
        let handler = interpreter.environment.borrow().get("mw").unwrap();
        let request = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));
        (interpreter, handler, request)
    }

    fn body_of(step: &Step) -> String {
        match step {
            Step::Halt(response) => String::from_utf8_lossy(&response.body).to_string(),
            Step::Continue(_) => panic!("expected Halt"),
        }
    }

    #[test]
    fn a_passing_middleware_hands_its_request_to_the_next_one() {
        let (mut interpreter, handler, request) = middleware_from(
            r#"
            fn mw(req) {
                req["stamp"] = "seen"
                return {"continue": true, "request": req}
            }
        "#,
        );
        let data = request_data();
        match run(&mut interpreter, &data, handler, None, request, false) {
            Step::Continue(modified) => {
                let Value::Hash(hash) = modified else {
                    panic!("expected a hash back")
                };
                let stamped = hash
                    .borrow()
                    .get(&HashKey::String("stamp".into()))
                    .cloned()
                    .expect("the middleware's edit survived");
                assert_eq!(stamped, Value::String("seen".into()));
            }
            Step::Halt(_) => panic!("a passing middleware must not halt"),
        }
    }

    #[test]
    fn a_short_circuiting_middleware_is_the_response() {
        let (mut interpreter, handler, request) = middleware_from(
            r#"
            fn mw(req) {
                return {"continue": false, "response": {"status": 401, "body": "sign in"}}
            }
        "#,
        );
        let data = request_data();
        match run(&mut interpreter, &data, handler, None, request, false) {
            Step::Halt(response) => {
                assert_eq!(response.status, 401);
                assert_eq!(String::from_utf8_lossy(&response.body), "sign in");
            }
            Step::Continue(_) => panic!("a short-circuiting middleware must halt"),
        }
    }

    /// A middleware whose *return value* is the wrong shape. The framework
    /// says what is wrong; production must not repeat it to the client.
    #[test]
    fn a_malformed_return_is_a_500_that_says_nothing_in_production() {
        let source = r#"
            fn mw(req) {
                return {"continue": true}
            }
        "#;
        let complaint = "no request";

        let (mut interpreter, handler, request) = middleware_from(source);
        let data = request_data();
        let dev = run(&mut interpreter, &data, handler, None, request, true);
        assert!(matches!(&dev, Step::Halt(r) if r.status == 500));
        assert!(
            body_of(&dev).contains(complaint),
            "dev page names the fault"
        );

        let (mut interpreter, handler, request) = middleware_from(source);
        let prod = run(&mut interpreter, &data, handler, None, request, false);
        assert!(matches!(&prod, Step::Halt(r) if r.status == 500));
        assert!(
            !body_of(&prod).contains(complaint),
            "production page must not leak the fault"
        );
    }

    /// A middleware that *raises*. Same rule, different arm — these were two
    /// separate hundred-line blocks, so it is worth pinning both.
    #[test]
    fn a_raised_error_is_a_500_that_says_nothing_in_production() {
        let source = r#"
            fn mw(req) {
                let x = undefined_variable
                return {"continue": true, "request": req}
            }
        "#;

        let (mut interpreter, handler, request) = middleware_from(source);
        let data = request_data();
        let dev = run(&mut interpreter, &data, handler, None, request, true);
        assert!(matches!(&dev, Step::Halt(r) if r.status == 500));
        assert!(
            body_of(&dev).contains("undefined_variable"),
            "dev page names the undefined variable"
        );

        let (mut interpreter, handler, request) = middleware_from(source);
        let prod = run(&mut interpreter, &data, handler, None, request, false);
        assert!(matches!(&prod, Step::Halt(r) if r.status == 500));
        assert!(
            !body_of(&prod).contains("undefined_variable"),
            "production page must not leak the interpreter's message"
        );
    }
}
