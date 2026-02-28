//! Tree-walking interpreter for Solilang.

mod expressions;
mod literals;
mod operators;
mod pattern_matching;
mod statements;
mod variables;

pub mod access;
pub mod calls;
pub mod control;
pub mod objects;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::*;
#[cfg(feature = "coverage")]
use crate::coverage::CoverageTracker;
use crate::error::RuntimeError;
use crate::interpreter::builtins::register_builtins;
use crate::interpreter::builtins::server::{
    build_request_hash, extract_response, get_routes, match_path, parse_query_string,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{value_matches_type, Function, HashKey, Value};
use crate::span::Span;

pub(crate) type RuntimeResult<T> = Result<T, RuntimeError>;

/// Represents a single frame in the call stack.
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub function_name: String,
    pub file_path: Option<String>,
    pub line: usize,
    pub column: usize,
}

/// Internal result type that can carry return values and exceptions.
pub(crate) enum ControlFlow {
    Normal(Value),
    Return(Value),
    Throw(Value),
}

/// The Solilang interpreter.
pub struct Interpreter {
    pub(crate) environment: Rc<RefCell<Environment>>,
    #[cfg(feature = "coverage")]
    pub(crate) coverage_tracker: Option<Rc<RefCell<CoverageTracker>>>,
    pub(crate) current_source_path: Option<PathBuf>,
    pub(crate) call_stack: Vec<StackFrame>,
    pub assertion_count: i64,
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = Rc::new(RefCell::new(Environment::new()));
        register_builtins(&mut globals.borrow_mut());

        Self {
            environment: globals,
            #[cfg(feature = "coverage")]
            coverage_tracker: None,
            current_source_path: None,
            call_stack: Vec::new(),
            assertion_count: 0,
        }
    }

    /// Create an interpreter with a pre-built environment (skips register_builtins).
    /// Used by the template engine with a cached builtins environment.
    pub fn with_environment(environment: Rc<RefCell<Environment>>) -> Self {
        Self {
            environment,
            #[cfg(feature = "coverage")]
            coverage_tracker: None,
            current_source_path: None,
            call_stack: Vec::new(),
            assertion_count: 0,
        }
    }

    #[cfg(feature = "coverage")]
    pub fn with_coverage_tracker(tracker: Rc<RefCell<CoverageTracker>>) -> Self {
        let globals = Rc::new(RefCell::new(Environment::new()));
        register_builtins(&mut globals.borrow_mut());

        Self {
            environment: globals,
            coverage_tracker: Some(tracker),
            current_source_path: None,
            call_stack: Vec::new(),
            assertion_count: 0,
        }
    }

    #[cfg(feature = "coverage")]
    pub fn set_coverage_tracker(&mut self, tracker: Rc<RefCell<CoverageTracker>>) {
        self.coverage_tracker = Some(tracker);
    }

    pub fn set_source_path(&mut self, path: PathBuf) {
        self.current_source_path = Some(path);
    }

    #[cfg(feature = "coverage")]
    #[inline(always)]
    pub fn record_coverage(&self, line: usize) {
        if let Some(ref tracker) = self.coverage_tracker {
            if let Some(ref path) = self.current_source_path {
                tracker.borrow_mut().record_line_hit(path, line);
            }
        }
    }

    #[cfg(not(feature = "coverage"))]
    #[inline(always)]
    pub fn record_coverage(&self, _line: usize) {
        // No-op when coverage feature is disabled - optimized away completely
    }

    pub fn get_assertion_count(&self) -> i64 {
        self.assertion_count
    }

    pub fn global_env(&self) -> &Rc<RefCell<Environment>> {
        &self.environment
    }

    pub fn increment_assertion_count(&mut self) {
        self.assertion_count += 1;
    }

    /// Serialize the current environment for debugging.
    /// Returns a JSON string with all variables (excluding functions/classes for simplicity).
    /// Also includes view context data if a template error occurred.
    /// Futures are resolved before serialization to capture their actual values.
    pub fn serialize_environment_for_debug(&self) -> String {
        let vars = self.environment.borrow().get_all_variables();
        let mut json_parts = Vec::new();

        for (name, value) in vars {
            // Skip functions and classes - they're not useful in the debug view
            match &value {
                Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => continue,
                _ => {}
            }

            // Resolve futures before serialization to get their actual values
            let resolved_value = if value.is_future() {
                match value.resolve() {
                    Ok(v) => v,
                    Err(e) => Value::String(format!("<future error: {}>", e)),
                }
            } else {
                value
            };

            let json_value = self.value_to_json(&resolved_value);
            json_parts.push(format!(r#""{}": {}"#, name, json_value));
        }

        // Check for view context (data passed to render())
        // Only add view context variables if they don't already exist in the environment
        if let Some(view_data) = crate::interpreter::builtins::template::get_view_debug_context() {
            // Collect existing variable names to avoid duplicates
            let existing_names: std::collections::HashSet<String> = json_parts
                .iter()
                .filter_map(|part| {
                    // Extract key name from "\"key\": value" format
                    if part.starts_with('"') {
                        part.split(':').next().and_then(|k| {
                            let k = k.trim().trim_matches('"');
                            if !k.is_empty() {
                                Some(k.to_string())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    }
                })
                .collect();

            // Add view data as a special "_view_data" variable (always add this)
            if !existing_names.contains("_view_data") {
                let view_json = self.value_to_json(&view_data);
                json_parts.push(format!(r#""_view_data": {}"#, view_json));
            }

            // Also extract individual keys from the view data hash for easy access
            // But ONLY if they don't already exist in the environment
            if let Value::Hash(hash) = &view_data {
                for (key, value) in hash.borrow().iter() {
                    if let HashKey::String(key_str) = key {
                        // Skip if this key already exists in the environment
                        if existing_names.contains(key_str) {
                            continue;
                        }
                        // Skip functions and classes
                        match value {
                            Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => {
                                continue
                            }
                            _ => {}
                        }
                        let value_json = self.value_to_json(value);
                        json_parts.push(format!(r#""{}": {}"#, key_str, value_json));
                    }
                }
            }
        }

        format!("{{{}}}", json_parts.join(", "))
    }

    /// Serialize a HashMap of variables to JSON string.
    pub fn serialize_environment(&self, vars: &std::collections::HashMap<String, Value>) -> String {
        let mut json_parts = Vec::new();

        for (name, value) in vars {
            // Skip functions and classes
            match value {
                Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => continue,
                _ => {}
            }

            // Resolve futures before serialization
            let resolved_value = if value.is_future() {
                match value.clone().resolve() {
                    Ok(v) => v,
                    Err(e) => Value::String(format!("<future error: {}>", e)),
                }
            } else {
                value.clone()
            };

            let json_value = self.value_to_json(&resolved_value);
            json_parts.push(format!(r#""{}": {}"#, name, json_value));
        }

        format!("{{{}}}", json_parts.join(", "))
    }

    /// Convert a Value to a JSON string representation.
    #[allow(clippy::only_used_in_recursion)]
    fn value_to_json(&self, value: &Value) -> String {
        match value {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(n) => n.to_string(),
            Value::Decimal(d) => d.to_string(),
            Value::String(s) => {
                // Escape string for JSON
                let escaped = s
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t");
                format!("\"{}\"", escaped)
            }
            Value::Array(arr) => {
                let items: Vec<String> =
                    arr.borrow().iter().map(|v| self.value_to_json(v)).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Hash(hash) => {
                let pairs: Vec<String> = hash
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        let key = match k {
                            HashKey::String(s) => s.clone(),
                            other => format!("{}", other),
                        };
                        let escaped_key = key.replace('\\', "\\\\").replace('"', "\\\"");
                        format!(r#""{}": {}"#, escaped_key, self.value_to_json(v))
                    })
                    .collect();
                format!("{{{}}}", pairs.join(", "))
            }
            Value::Instance(inst) => {
                let inst = inst.borrow();
                let fields: Vec<String> = inst
                    .fields
                    .iter()
                    .map(|(k, v)| format!(r#""{}": {}"#, k, self.value_to_json(v)))
                    .collect();
                if fields.is_empty() {
                    format!(r#"{{"__class__": "{}"}}"#, inst.class.name)
                } else {
                    format!(
                        r#"{{"__class__": "{}", {}}}"#,
                        inst.class.name,
                        fields.join(", ")
                    )
                }
            }
            Value::Function(_) => "\"<function>\"".to_string(),
            Value::NativeFunction(_) => "\"<native function>\"".to_string(),
            Value::Class(c) => format!("\"<class {}>\"", c.name),
            Value::Future(_) => "\"<future>\"".to_string(),
            Value::Method(_) => "\"<method>\"".to_string(),
            Value::Breakpoint => "\"<breakpoint>\"".to_string(),
            Value::QueryBuilder(_) => "\"<query builder>\"".to_string(),
            Value::Super(c) => format!("\"<super of {}>\"", c.name),
            Value::VmClosure(c) => format!("\"<fn {}>\"", c.proto.name),
        }
    }

    /// Push a frame onto the call stack.
    /// If `source_path` is provided, it takes precedence over `current_source_path`.
    pub(crate) fn push_frame(
        &mut self,
        function_name: &str,
        span: Span,
        source_path: Option<String>,
    ) {
        let file_path = source_path.or_else(|| {
            self.current_source_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
        });
        self.call_stack.push(StackFrame {
            function_name: function_name.to_string(),
            file_path,
            line: span.line,
            column: span.column,
        });
    }

    /// Pop a frame from the call stack.
    pub(crate) fn pop_frame(&mut self) {
        self.call_stack.pop();
    }

    /// Get the current call stack as formatted strings.
    /// Returns frames from outermost (entry point) to innermost (most recent call).
    pub fn get_stack_trace(&self) -> Vec<String> {
        self.call_stack
            .iter()
            .map(|frame| {
                let file = frame
                    .file_path
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                format!("{} at {}:{}", frame.function_name, file, frame.line)
            })
            .collect()
    }

    /// Interpret a complete program.
    pub fn interpret(&mut self, program: &Program) -> RuntimeResult<()> {
        for stmt in &program.statements {
            self.execute(stmt)?;
        }
        Ok(())
    }

    pub(crate) fn execute_block(
        &mut self,
        statements: &[Stmt],
        env: Environment,
    ) -> RuntimeResult<ControlFlow> {
        let previous = std::mem::replace(&mut self.environment, Rc::new(RefCell::new(env)));

        let mut result = Ok(ControlFlow::Normal(Value::Null));
        for stmt in statements {
            result = self.execute(stmt);
            match &result {
                Err(_) => break,
                Ok(ControlFlow::Return(_)) => break,
                Ok(ControlFlow::Throw(_)) => break,
                Ok(ControlFlow::Normal(_)) => {}
            }
        }

        // Capture environment and stack trace BEFORE restoring if there's an error
        // This preserves local variables for debugging
        let result = match result {
            Err(e) if !e.is_breakpoint() && e.breakpoint_env_json().is_none() => {
                let captured_env = self.environment.borrow().get_all_variables();
                let env_json = self.serialize_environment(&captured_env);

                // Get current file path for error location
                let file_path = self
                    .current_source_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                // Capture stack trace and update the last frame with actual error line
                let mut stack_trace = self.get_stack_trace();

                // Get the current function name from the last stack frame
                let func_name = self
                    .call_stack
                    .last()
                    .map(|f| f.function_name.clone())
                    .unwrap_or_else(|| "unknown".to_string());

                // Replace the last frame with one that has the actual error line number
                if !stack_trace.is_empty() {
                    stack_trace.pop();
                }
                stack_trace.push(format!("{} at {}:{}", func_name, file_path, e.span().line));

                Err(RuntimeError::with_env(
                    e.to_string(),
                    e.span(),
                    env_json,
                    stack_trace,
                ))
            }
            other => other,
        };

        self.environment = previous;
        result
    }

    /// Call a function with the given arguments and return the result.
    pub(crate) fn call_function(
        &mut self,
        func: &Function,
        arguments: Vec<Value>,
    ) -> RuntimeResult<Value> {
        // Push stack frame with the function's source path (where it was defined)
        let span = func.span.unwrap_or_else(|| Span::new(0, 0, 1, 1));
        self.push_frame(&func.name, span, func.source_path.clone());

        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        let mut call_env_inner = call_env_rc.borrow_mut();

        for (param, value) in func.params.iter().zip(arguments) {
            call_env_inner.define(param.name.clone(), value);
        }

        // Store defining_superclass for super calls
        if let Some(ref sc) = func.defining_superclass {
            call_env_inner.define(
                "__defining_superclass__".to_string(),
                Value::Class(sc.clone()),
            );
        }

        // Store reference to capture environment on error
        let env_for_capture = call_env_rc.clone();

        // Drop mutable borrow before executing block
        drop(call_env_inner);

        // Execute the function body
        let result = match self.execute_block(&func.body, call_env_rc.borrow().clone()) {
            Ok(ControlFlow::Normal(v)) => Ok(v),
            Ok(ControlFlow::Return(return_value)) => Ok(return_value),
            Ok(ControlFlow::Throw(e)) => Err(RuntimeError::General {
                message: format!("Unhandled exception: {}", e),
                span: Span::default(),
            }),
            Err(e) => {
                // Preserve errors that already have captured environment (breakpoint or WithEnv)
                if e.is_breakpoint() || e.breakpoint_env_json().is_some() {
                    Err(e)
                } else {
                    // Capture the local environment before it's lost
                    let captured_env = env_for_capture.borrow().get_all_variables();
                    let env_json = self.serialize_environment(&captured_env);

                    // Capture stack trace before popping frame
                    let stack_trace = self.get_stack_trace();

                    Err(RuntimeError::with_env(
                        e.to_string(),
                        e.span(),
                        env_json,
                        stack_trace,
                    ))
                }
            }
        };

        // Validate return type if annotated
        let result = match result {
            Ok(ref value) => {
                if let Some(ref expected_type) = func.return_type {
                    if !value_matches_type(value, expected_type) {
                        Err(RuntimeError::General {
                            message: format!(
                                "function '{}' expected to return {}, got {}",
                                func.name,
                                expected_type,
                                value.type_name()
                            ),
                            span,
                        })
                    } else {
                        result
                    }
                } else {
                    result
                }
            }
            _ => result,
        };

        // Pop stack frame
        self.pop_frame();

        result
    }

    /// Run the HTTP server on the given port.
    /// This is called when http_server_listen returns its marker value.
    pub fn run_http_server(&mut self, port: u16) -> RuntimeResult<Value> {
        let routes = get_routes();

        if routes.is_empty() {
            return Err(RuntimeError::General {
                message: "No routes registered. Use http_server_get/post/put/delete to register routes before calling http_server_listen.".to_string(),
                span: Span::default(),
            });
        }

        let addr = format!("0.0.0.0:{}", port);

        let listener = std::net::TcpListener::bind(&addr).map_err(|e| RuntimeError::General {
            message: format!("Failed to start HTTP server on port {}: {}", port, e),
            span: Span::default(),
        })?;

        println!("Server listening on http://0.0.0.0:{}", port);

        for stream in listener.incoming() {
            let stream = stream.map_err(|e| RuntimeError::General {
                message: format!("Failed to accept connection: {}", e),
                span: Span::default(),
            })?;

            let _ = self.handle_http_connection(stream);
        }

        Ok(Value::Null)
    }

    fn handle_http_connection(&mut self, stream: std::net::TcpStream) -> RuntimeResult<()> {
        use std::io::{BufRead, Read};

        let mut stream = stream;
        let mut reader = std::io::BufReader::new(&mut stream);

        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .map_err(|e| RuntimeError::General {
                message: format!("Failed to read request: {}", e),
                span: Span::default(),
            })?;

        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            self.send_error_response(&mut stream, "400 Bad Request")?;
            return Ok(());
        }

        let method = parts[0].to_uppercase();
        let url = parts[1];

        let (path, query_str) = if let Some(pos) = url.find('?') {
            (&url[..pos], &url[pos + 1..])
        } else {
            (url, "")
        };

        let query = parse_query_string(query_str);

        let mut headers = HashMap::new();
        for line in reader.by_ref().lines() {
            let line = line.map_err(|e| RuntimeError::General {
                message: format!("Failed to read header: {}", e),
                span: Span::default(),
            })?;
            if line.trim().is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        let body = if let Some(content_length) = headers.get("Content-Length") {
            if let Ok(len) = content_length.parse::<usize>() {
                let mut buf = vec![0u8; len];
                reader
                    .read_exact(&mut buf)
                    .map_err(|e| RuntimeError::General {
                        message: format!("Failed to read body: {}", e),
                        span: Span::default(),
                    })?;
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let routes = get_routes();
        let mut matched_route = None;
        let mut matched_params = HashMap::new();

        for route in &routes {
            if route.method == method {
                if let Some(params) = match_path(&route.path_pattern, path) {
                    matched_route = Some(route);
                    matched_params = params;
                    break;
                }
            }
        }

        if let Some(route) = matched_route {
            let request_hash =
                build_request_hash(&method, path, matched_params, query, headers, &body);

            let handler = self.environment.borrow().get(&route.handler_name);

            match handler {
                Some(handler_value) => {
                    match self.call_value(handler_value, vec![request_hash], Span::default()) {
                        Ok(result) => {
                            let (status, resp_headers, resp_body) = extract_response(result);
                            self.build_http_response(&mut stream, status, resp_headers, resp_body)?
                        }
                        Err(e) => self.build_http_response(
                            &mut stream,
                            500,
                            Vec::new(),
                            format!("Error: {}", e),
                        )?,
                    }
                }
                None => self.build_http_response(
                    &mut stream,
                    500,
                    Vec::new(),
                    format!("Handler not found: {}", route.handler_name),
                )?,
            }
        } else {
            self.build_http_response(&mut stream, 404, Vec::new(), "Not Found".to_string())?;
        }

        Ok(())
    }

    fn build_http_response(
        &self,
        stream: &mut std::net::TcpStream,
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    ) -> RuntimeResult<()> {
        let status_text = match status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "Unknown",
        };

        let mut response = format!("HTTP/1.1 {} {}\r\n", status, status_text);
        response.push_str("Content-Type: text/plain\r\n");
        response.push_str(&format!("Content-Length: {}\r\n", body.len()));
        response.push_str("Connection: close\r\n");

        for (key, value) in headers {
            response.push_str(&format!("{}: {}\r\n", key, value));
        }

        response.push_str("\r\n");
        response.push_str(&body);

        std::io::Write::write_all(stream, response.as_bytes()).map_err(|e| {
            RuntimeError::General {
                message: format!("Failed to send response: {}", e),
                span: Span::default(),
            }
        })?;

        Ok(())
    }

    fn send_error_response(
        &self,
        stream: &mut std::net::TcpStream,
        message: &str,
    ) -> RuntimeResult<()> {
        self.build_http_response(stream, 400, Vec::new(), message.to_string())
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod return_type_enforcement_tests {
    use super::*;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    fn run(source: &str) -> Result<(), String> {
        let tokens = Scanner::new(source)
            .scan_tokens()
            .map_err(|e| e.to_string())?;
        let program = Parser::new(tokens).parse().map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).map_err(|e| e.to_string())
    }

    #[test]
    fn test_correct_return_type_string() {
        let src = r#"
fn greet() -> String
  return "hello"
end
greet()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_correct_return_type_int() {
        let src = r#"
fn add() -> Int
  return 42
end
add()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_wrong_return_type_int_instead_of_string() {
        let src = r#"
fn greet() -> String
  return 42
end
greet()
"#;
        let result = run(src);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("expected to return String"),
            "Error was: {}",
            err
        );
        assert!(err.contains("got int"), "Error was: {}", err);
    }

    #[test]
    fn test_nullable_return_type_allows_null() {
        let src = r#"
fn maybe_name() -> String?
  return null
end
maybe_name()
"#;
        let result = run(src);
        assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result.err());
    }

    #[test]
    fn test_nullable_return_type_allows_value() {
        let src = r#"
fn maybe_name() -> String?
  return "Alice"
end
maybe_name()
"#;
        let result = run(src);
        assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result.err());
    }

    #[test]
    fn test_nullable_return_type_rejects_wrong_type() {
        let src = r#"
fn maybe_name() -> String?
  return 42
end
maybe_name()
"#;
        let result = run(src);
        assert!(result.is_err());
    }

    #[test]
    fn test_unannotated_function_allows_anything() {
        let src = r#"
fn flexible()
  return 42
end
flexible()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_lambda_with_return_type() {
        let src = r#"
let f = fn(x) -> Int
  return x + 1
end
f(5)
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_lambda_with_wrong_return_type() {
        let src = r#"
let f = fn(x) -> Int
  return "not an int"
end
f(5)
"#;
        let result = run(src);
        assert!(result.is_err());
    }

    #[test]
    fn test_bool_return_type() {
        let src = r#"
fn is_even(n) -> Bool
  return n % 2 == 0
end
is_even(4)
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_array_return_type() {
        let src = r#"
fn get_list() -> Array
  return [1, 2, 3]
end
get_list()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_hash_return_type() {
        let src = r#"
fn get_map() -> Hash
  return { "a" => 1 }
end
get_map()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_implicit_null_return_fails_for_typed_function() {
        let src = r#"
fn greet() -> String
  let x = 1
end
greet()
"#;
        let result = run(src);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("expected to return String"),
            "Error was: {}",
            err
        );
    }

    #[test]
    fn test_void_return_type_allows_null() {
        let src = r#"
fn do_stuff() -> Void
  let x = 1
end
do_stuff()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_void_return_type_rejects_value() {
        let src = r#"
fn do_stuff() -> Void
  return 42
end
do_stuff()
"#;
        let result = run(src);
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod safe_navigation_tests {
    use crate::interpreter::value::Value;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    use super::Interpreter;

    fn eval(source: &str) -> Value {
        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).unwrap();
        // Read the result variable from the environment
        let val = interpreter
            .environment
            .borrow()
            .get("result")
            .unwrap_or(Value::Null);
        val
    }

    #[test]
    fn test_safe_nav_null_returns_null() {
        let val = eval("let x = null; let result = x&.name;");
        assert_eq!(val, Value::Null);
    }

    #[test]
    fn test_safe_nav_non_null_returns_field() {
        // Use hash for field access since it's simpler and well-tested
        let val = eval(
            r#"
let u = { "name" => "Alice" }
let result = u&.name
"#,
        );
        assert_eq!(val, Value::String("Alice".to_string()));
    }

    #[test]
    fn test_safe_nav_method_null() {
        let val = eval(
            r#"
let x = null
let result = x&.greet()
"#,
        );
        assert_eq!(val, Value::Null);
    }

    #[test]
    fn test_safe_nav_method_non_null() {
        // Use array with a method to test non-null safe nav method call
        let val = eval(
            r#"
let arr = [3, 1, 2]
let result = arr&.length()
"#,
        );
        assert_eq!(val, Value::Int(3));
    }

    #[test]
    fn test_safe_nav_chained_null_at_first() {
        let val = eval("let x = null; let result = x&.inner&.field;");
        assert_eq!(val, Value::Null);
    }

    #[test]
    fn test_safe_nav_chained_non_null() {
        let val = eval(
            r#"
let u = { "address" => { "city" => "Paris" } }
let result = u&.address&.city
"#,
        );
        assert_eq!(val, Value::String("Paris".to_string()));
    }

    #[test]
    fn test_safe_nav_with_nullish_coalescing() {
        let val = eval(r#"let x = null; let result = x&.name ?? "default";"#);
        assert_eq!(val, Value::String("default".to_string()));
    }

    #[test]
    fn test_safe_nav_on_non_null_with_nullish() {
        let val = eval(
            r#"
let u = { "name" => "Eve" }
let result = u&.name ?? "default"
"#,
        );
        assert_eq!(val, Value::String("Eve".to_string()));
    }
}
