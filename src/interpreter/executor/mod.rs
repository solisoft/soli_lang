//! Tree-walking interpreter for Solilang.

mod expressions;
mod operators;
mod statements;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::error::RuntimeError;
use crate::interpreter::builtins::register_builtins;
use crate::interpreter::builtins::server::{
    build_request_hash, extract_response, get_routes, match_path, parse_query_string,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Function, Value};
use crate::span::Span;

pub(crate) type RuntimeResult<T> = Result<T, RuntimeError>;

/// Internal result type that can carry return values and exceptions.
pub(crate) enum ControlFlow {
    Normal,
    Return(Value),
    Throw(Value),
}

/// The Solilang interpreter.
pub struct Interpreter {
    pub(crate) environment: Rc<RefCell<Environment>>,
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = Rc::new(RefCell::new(Environment::new()));
        register_builtins(&mut globals.borrow_mut());

        Self {
            environment: globals,
        }
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
        let call_env = Environment::with_enclosing(func.closure.clone());
        let mut call_env = call_env;

        for (param, value) in func.params.iter().zip(arguments) {
            call_env.define(param.name.clone(), value);
        }

        match self.execute_block(&func.body, call_env)? {
            ControlFlow::Normal => Ok(Value::Null),
            ControlFlow::Return(return_value) => Ok(return_value),
            ControlFlow::Throw(e) => Err(RuntimeError::General {
                message: format!("Unhandled exception: {}", e),
                span: Span::default(),
            }),
        }
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
