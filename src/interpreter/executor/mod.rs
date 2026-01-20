//! Tree-walking interpreter for Solilang.

mod expressions;
mod operators;
mod statements;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
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

/// Internal result type that can carry return values.
pub(crate) enum ControlFlow {
    Normal,
    Return(Value),
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
            ControlFlow::Return(v) => Ok(v),
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

        // Create the server
        let addr = format!("0.0.0.0:{}", port);
        let server = match tiny_http::Server::http(&addr) {
            Ok(s) => s,
            Err(e) => {
                return Err(RuntimeError::General {
                    message: format!("Failed to start HTTP server on port {}: {}", port, e),
                    span: Span::default(),
                });
            }
        };

        println!("Server listening on http://0.0.0.0:{}", port);

        // Main server loop
        loop {
            // Wait for a request
            let mut request = match server.recv() {
                Ok(req) => req,
                Err(e) => {
                    eprintln!("Error receiving request: {}", e);
                    continue;
                }
            };

            // Extract request info
            let method = request.method().to_string().to_uppercase();
            let url = request.url().to_string();

            // Split path and query string
            let (path, query_str) = if let Some(pos) = url.find('?') {
                (&url[..pos], &url[pos + 1..])
            } else {
                (url.as_str(), "")
            };

            // Parse query string
            let query = parse_query_string(query_str);

            // Extract headers
            let mut headers = HashMap::new();
            for header in request.headers() {
                headers.insert(header.field.to_string(), header.value.to_string());
            }

            // Read body
            let mut body = String::new();
            if let Some(len) = request.body_length() {
                if len > 0 {
                    let mut reader = request.as_reader();
                    let mut buf = Vec::with_capacity(len);
                    if reader.read_to_end(&mut buf).is_ok() {
                        body = String::from_utf8_lossy(&buf).to_string();
                    }
                }
            }

            // Find matching route
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

            // Handle the request
            let response = if let Some(route) = matched_route {
                // Build request hash
                let request_hash =
                    build_request_hash(&method, path, matched_params, query, headers, body);

                // Call the handler
                match self.call_value(route.handler.clone(), vec![request_hash], Span::default()) {
                    Ok(result) => {
                        let (status, resp_headers, resp_body) = extract_response(&result);

                        let mut response =
                            tiny_http::Response::from_string(resp_body).with_status_code(status);

                        // Add headers
                        for (key, value) in resp_headers {
                            if let Ok(header) =
                                tiny_http::Header::from_bytes(key.as_bytes(), value.as_bytes())
                            {
                                response = response.with_header(header);
                            }
                        }

                        response.boxed()
                    }
                    Err(e) => {
                        eprintln!("Handler error: {}", e);
                        tiny_http::Response::from_string(format!("Internal Server Error: {}", e))
                            .with_status_code(500)
                            .boxed()
                    }
                }
            } else {
                // 404 Not Found
                tiny_http::Response::from_string("Not Found")
                    .with_status_code(404)
                    .boxed()
            };

            // Send response
            if let Err(e) = request.respond(response) {
                eprintln!("Error sending response: {}", e);
            }
        }
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}
