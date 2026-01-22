//! Tree-walking interpreter for Solilang.

mod expressions;
mod operators;
mod statements;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::*;
use crate::coverage::CoverageTracker;
use crate::error::RuntimeError;
use crate::interpreter::builtins::register_builtins;
use crate::interpreter::builtins::server::{
    build_request_hash, extract_response, get_routes, match_path, parse_query_string,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Function, Value};
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
    Normal,
    Return(Value),
    Throw(Value),
}

/// The Solilang interpreter.
pub struct Interpreter {
    pub(crate) environment: Rc<RefCell<Environment>>,
    pub(crate) coverage_tracker: Option<Rc<RefCell<CoverageTracker>>>,
    pub(crate) current_source_path: Option<PathBuf>,
    pub(crate) call_stack: Vec<StackFrame>,
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = Rc::new(RefCell::new(Environment::new()));
        register_builtins(&mut globals.borrow_mut());

        Self {
            environment: globals,
            coverage_tracker: None,
            current_source_path: None,
            call_stack: Vec::new(),
        }
    }

    pub fn with_coverage_tracker(tracker: Rc<RefCell<CoverageTracker>>) -> Self {
        let globals = Rc::new(RefCell::new(Environment::new()));
        register_builtins(&mut globals.borrow_mut());

        Self {
            environment: globals,
            coverage_tracker: Some(tracker),
            current_source_path: None,
            call_stack: Vec::new(),
        }
    }

    pub fn set_coverage_tracker(&mut self, tracker: Rc<RefCell<CoverageTracker>>) {
        self.coverage_tracker = Some(tracker);
    }

    pub fn set_source_path(&mut self, path: PathBuf) {
        self.current_source_path = Some(path);
    }

    pub fn record_coverage(&self, line: usize) {
        if let Some(ref tracker) = self.coverage_tracker {
            if let Some(ref path) = self.current_source_path {
                tracker.borrow_mut().record_line_hit(path, line);
            }
        }
    }

    /// Serialize the current environment for debugging.
    /// Returns a JSON string with all variables (excluding functions/classes for simplicity).
    pub fn serialize_environment_for_debug(&self) -> String {
        let vars = self.environment.borrow().get_all_variables();
        let mut json_parts = Vec::new();

        for (name, value) in vars {
            // Skip functions and classes - they're not useful in the debug view
            match &value {
                Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => continue,
                _ => {}
            }

            let json_value = self.value_to_json(&value);
            json_parts.push(format!(r#""{}": {}"#, name, json_value));
        }

        format!("{{{}}}", json_parts.join(", "))
    }

    /// Convert a Value to a JSON string representation.
    fn value_to_json(&self, value: &Value) -> String {
        match value {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(n) => n.to_string(),
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
                let items: Vec<String> = arr
                    .borrow()
                    .iter()
                    .map(|v| self.value_to_json(v))
                    .collect();
                format!("[{}]", items.join(", "))
            }
            Value::Hash(hash) => {
                let pairs: Vec<String> = hash
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        let key = match k {
                            Value::String(s) => s.clone(),
                            other => format!("{}", other),
                        };
                        let escaped_key = key
                            .replace('\\', "\\\\")
                            .replace('"', "\\\"");
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
                format!(r#"{{"__class__": "{}", {}}}"#, inst.class.name, fields.join(", "))
            }
            Value::Function(_) => "\"<function>\"".to_string(),
            Value::NativeFunction(_) => "\"<native function>\"".to_string(),
            Value::Class(c) => format!("\"<class {}>\"", c.name),
            Value::Future(_) => "\"<future>\"".to_string(),
            Value::Method(_) => "\"<method>\"".to_string(),
            Value::Breakpoint => "\"<breakpoint>\"".to_string(),
        }
    }

    /// Push a frame onto the call stack.
    /// If `source_path` is provided, it takes precedence over `current_source_path`.
    pub(crate) fn push_frame(&mut self, function_name: &str, span: Span, source_path: Option<String>) {
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
    pub fn get_stack_trace(&self) -> Vec<String> {
        self.call_stack
            .iter()
            .rev()
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

        let mut result = Ok(ControlFlow::Normal);
        for stmt in statements {
            result = self.execute(stmt);
            match &result {
                Err(_) => break,
                Ok(ControlFlow::Return(_)) => break,
                Ok(ControlFlow::Throw(_)) => break,
                Ok(ControlFlow::Normal) => {}
            }
        }

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
        let mut call_env = call_env;

        for (param, value) in func.params.iter().zip(arguments) {
            call_env.define(param.name.clone(), value);
        }

        let result = match self.execute_block(&func.body, call_env) {
            Ok(ControlFlow::Normal) => Ok(Value::Null),
            Ok(ControlFlow::Return(return_value)) => Ok(return_value),
            Ok(ControlFlow::Throw(e)) => Err(RuntimeError::General {
                message: format!("Unhandled exception: {}", e),
                span: Span::default(),
            }),
            Err(e) => {
                // Capture stack trace before popping frame
                let stack_trace = self
                    .call_stack
                    .iter()
                    .rev()
                    .map(|frame| {
                        let file = frame
                            .file_path
                            .as_ref()
                            .cloned()
                            .unwrap_or_else(|| "unknown".to_string());
                        format!("{} at {}:{}", frame.function_name, file, frame.line)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                // Create a new error with stack trace
                let error_with_stack = format!("{}\nStack trace:\n{}", e, stack_trace);
                Err(RuntimeError::General {
                    message: error_with_stack,
                    span: e.span(),
                })
            }
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
        use std::io::{BufRead, Read, Write};

        let mut stream = stream;
        let mut reader = std::io::BufReader::new(&mut stream);

        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .map_err(|e| RuntimeError::General {
                message: format!("Failed to read request: {}", e),
                span: Span::default(),
            })?;

        let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
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
                build_request_hash(&method, path, matched_params, query, headers, body);

            let handler = self.environment.borrow().get(&route.handler_name);

            match handler {
                Some(handler_value) => {
                    match self.call_value(handler_value, vec![request_hash], Span::default()) {
                        Ok(result) => {
                            let (status, resp_headers, resp_body) = extract_response(&result);
                            self.build_http_response(&mut stream, status, resp_headers, resp_body)?
                        }
                        Err(e) => self.build_http_response(
                            &mut stream,
                            500,
                            HashMap::new(),
                            format!("Error: {}", e),
                        )?,
                    }
                }
                None => self.build_http_response(
                    &mut stream,
                    500,
                    HashMap::new(),
                    format!("Handler not found: {}", route.handler_name),
                )?,
            }
        } else {
            self.build_http_response(&mut stream, 404, HashMap::new(), "Not Found".to_string())?;
        }

        Ok(())
    }

    fn build_http_response(
        &self,
        stream: &mut std::net::TcpStream,
        status: u16,
        headers: HashMap<String, String>,
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
        self.build_http_response(stream, 400, HashMap::new(), message.to_string())
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}
