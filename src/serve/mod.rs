//! MVC framework with convention-based routing and hot reload.
//!
//! This module implements a Rails-like MVC framework for Soli applications:
//! - Convention-based routing from controller filenames and function names
//! - Hot reload of changed files without server restart
//! - Automatic route derivation
//! - Middleware support for request interception

mod hot_reload;
pub mod live_reload;
mod middleware;
mod router;
pub mod websocket;

pub use hot_reload::FileTracker;
pub use middleware::{
    clear_middleware, extract_middleware_functions, extract_middleware_result, get_middleware,
    get_middleware_by_name, register_middleware, register_middleware_with_options,
    scan_middleware_files, Middleware, MiddlewareResult,
};
pub use router::{derive_routes_from_controller, ControllerRoute};
pub use websocket::{
    clear_websocket_routes, get_websocket_routes, match_websocket_route, register_websocket_route,
    WebSocketConnection, WebSocketEvent, WebSocketHandlerAction, WebSocketRegistry,
};

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use bytes::Bytes;
use futures_util::future::Either;
use futures_util::SinkExt;
use futures_util::StreamExt;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{header, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};
use tokio_tungstenite::WebSocketStream;
use tungstenite::protocol::WebSocketConfig;
use uuid::Uuid;

use crate::error::RuntimeError;
use crate::interpreter::builtins::server::{
    build_request_hash, extract_response, get_routes, match_path, parse_query_string,
    register_route_with_handler,
};
use crate::interpreter::builtins::template::{clear_template_cache, init_templates};
use crate::interpreter::Interpreter;
use crate::span::Span;

/// Request data sent to interpreter thread
struct RequestData {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
    response_tx: oneshot::Sender<ResponseData>,
}

/// Response data from interpreter thread
#[derive(Clone)]
struct ResponseData {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

/// Serve an MVC application from a folder with live reload enabled by default.
pub fn serve_folder(folder: &Path, port: u16) -> Result<(), RuntimeError> {
    serve_folder_with_options(folder, port, true)
}

/// Serve an MVC application from a folder with configurable options.
pub fn serve_folder_with_options(
    folder: &Path,
    port: u16,
    live_reload: bool,
) -> Result<(), RuntimeError> {
    // Validate folder structure
    let app_dir = folder.join("app");
    let controllers_dir = app_dir.join("controllers");

    if !controllers_dir.exists() {
        return Err(RuntimeError::General {
            message: format!(
                "Invalid MVC structure: {} does not exist. Expected app/controllers/ directory.",
                controllers_dir.display()
            ),
            span: Span::default(),
        });
    }

    println!("Starting MVC server from {}", folder.display());

    // Create interpreter
    let mut interpreter = Interpreter::new();

    // Load models first (shared code)
    let models_dir = app_dir.join("models");
    if models_dir.exists() {
        load_models(&mut interpreter, &models_dir)?;
    }

    // Initialize file tracker for hot reload
    let mut file_tracker = FileTracker::new();

    // Load middleware
    let middleware_dir = app_dir.join("middleware");
    if middleware_dir.exists() {
        load_middleware(&mut interpreter, &middleware_dir, &mut file_tracker)?;
    }

    // Scan and load controllers
    let controller_files = scan_controllers(&controllers_dir)?;
    for controller_path in &controller_files {
        load_controller(&mut interpreter, controller_path, &mut file_tracker)?;
    }

    // Track model files too
    if models_dir.exists() {
        for entry in std::fs::read_dir(&models_dir).map_err(|e| RuntimeError::General {
            message: format!("Failed to read models directory: {}", e),
            span: Span::default(),
        })? {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "soli") {
                    file_tracker.track(&path);
                }
            }
        }
    }

    // Initialize template engine with views directory
    let views_dir = app_dir.join("views");
    init_templates(views_dir.clone());
    if views_dir.exists() {
        println!("Template engine initialized from {}", views_dir.display());
        // Track view files for hot reload
        track_view_files(&views_dir, &mut file_tracker)?;
    }

    // Set live reload flag for template injection
    live_reload::set_live_reload_enabled(live_reload);

    // Load routes from config/routes.soli if it exists
    let routes_file = folder.join("config").join("routes.soli");
    if routes_file.exists() {
        println!("Loading routes from config/routes.soli");

        // Define DSL helpers in Soli
        // Note: Using named functions for blocks since lambda expressions are not supported
        // IMPORTANT: Function parameters require type annotations in Soli
        let dsl_source = r#"
            fn resources(name: Any, block: Any) {
                router_resource_enter(name, null);
                if (block != null) { block(); }
                router_resource_exit();
            }

            fn namespace(name: Any, block: Any) {
                router_namespace_enter(name);
                if (block != null) { block(); }
                router_namespace_exit();
            }

            fn member(block: Any) {
                router_member_enter();
                if (block != null) { block(); }
                router_member_exit();
            }

            fn collection(block: Any) {
                router_collection_enter();
                if (block != null) { block(); }
                router_collection_exit();
            }

            // Scope middleware to a block of routes
            // middleware("auth", -> { get("/admin", "admin#index"); })
            fn middleware(mw_names: Any, block: Any) {
                router_middleware_scope(mw_names);
                if (block != null) { block(); }
                router_middleware_scope_exit();
            }

            fn get(path: Any, action: Any) { router_match("GET", path, action); }
            fn post(path: Any, action: Any) { router_match("POST", path, action); }
            fn put(path: Any, action: Any) { router_match("PUT", path, action); }
            fn delete(path: Any, action: Any) { router_match("DELETE", path, action); }
            fn patch(path: Any, action: Any) { router_match("PATCH", path, action); }

            // WebSocket route registration
            // websocket("/path", "controller#handler")
            fn websocket(path: Any, action: Any) { router_websocket(path, action); }
        "#;

        // Execute DSL definitions
        // Lex and Parse DSL
        let dsl_tokens = crate::lexer::Scanner::new(dsl_source)
            .scan_tokens()
            .map_err(|e| RuntimeError::General {
                message: format!("DSL Lexer error: {}", e),
                span: Span::default(),
            })?;
        let dsl_program = crate::parser::Parser::new(dsl_tokens)
            .parse()
            .map_err(|e| RuntimeError::General {
                message: format!("DSL Parser error: {}", e),
                span: Span::default(),
            })?;
        interpreter.interpret(&dsl_program)?;

        // Clear auto-derived routes to prefer explicit ones
        crate::interpreter::builtins::server::clear_routes();

        // Execute routes file
        execute_file(&mut interpreter, &routes_file)?;
    }

    // Public directory for static files
    let public_dir = folder.join("public");

    // Start HTTP server with hyper
    run_hyper_server(
        interpreter,
        port,
        controllers_dir,
        models_dir,
        middleware_dir,
        public_dir,
        file_tracker,
        live_reload,
    )
}

/// Scan for all controller files in the controllers directory.
fn scan_controllers(controllers_dir: &Path) -> Result<Vec<PathBuf>, RuntimeError> {
    let mut controllers = Vec::new();

    for entry in std::fs::read_dir(controllers_dir).map_err(|e| RuntimeError::General {
        message: format!("Failed to read controllers directory: {}", e),
        span: Span::default(),
    })? {
        let entry = entry.map_err(|e| RuntimeError::General {
            message: format!("Failed to read directory entry: {}", e),
            span: Span::default(),
        })?;

        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "soli") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with("_controller.soli") {
                    controllers.push(path);
                }
            }
        }
    }

    Ok(controllers)
}

/// Load all model files.
fn load_models(interpreter: &mut Interpreter, models_dir: &Path) -> Result<(), RuntimeError> {
    for entry in std::fs::read_dir(models_dir).map_err(|e| RuntimeError::General {
        message: format!("Failed to read models directory: {}", e),
        span: Span::default(),
    })? {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "soli") {
                println!("Loading model: {}", path.display());
                execute_file(interpreter, &path)?;
            }
        }
    }
    Ok(())
}

/// Load all middleware files and register middleware functions.
fn load_middleware(
    interpreter: &mut Interpreter,
    middleware_dir: &Path,
    file_tracker: &mut FileTracker,
) -> Result<(), RuntimeError> {
    // Clear existing middleware
    clear_middleware();

    let middleware_files = scan_middleware_files(middleware_dir)?;

    if middleware_files.is_empty() {
        return Ok(());
    }

    println!("Loading middleware:");

    for middleware_path in middleware_files {
        // Track file for hot reload
        file_tracker.track(&middleware_path);

        // Read source to extract function names and orders
        let source =
            std::fs::read_to_string(&middleware_path).map_err(|e| RuntimeError::General {
                message: format!("Failed to read middleware file: {}", e),
                span: Span::default(),
            })?;

        let functions = extract_middleware_functions(&source);

        // Execute the middleware file to define functions
        execute_file(interpreter, &middleware_path)?;

        // Register each middleware function
        for (func_name, order, global_only, scope_only) in functions {
            let func_value = interpreter
                .environment
                .borrow()
                .get(&func_name)
                .ok_or_else(|| RuntimeError::General {
                    message: format!(
                        "Middleware function '{}' not found in {}",
                        func_name,
                        middleware_path.display()
                    ),
                    span: Span::default(),
                })?;

            let flags = if global_only {
                " [global_only]".to_string()
            } else if scope_only {
                " [scope_only]".to_string()
            } else {
                "".to_string()
            };
            println!(
                "  [{}] {} (order: {}){}",
                middleware_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown"),
                func_name,
                order,
                flags
            );

            register_middleware_with_options(
                &func_name,
                func_value,
                order,
                global_only,
                scope_only,
            );
        }
    }

    Ok(())
}

/// Load a controller file and register its routes.
fn load_controller(
    interpreter: &mut Interpreter,
    controller_path: &Path,
    file_tracker: &mut FileTracker,
) -> Result<(), RuntimeError> {
    let controller_name = controller_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    println!("Loading controller: {}", controller_name);

    // Track file for hot reload
    file_tracker.track(controller_path);

    // Read and parse the controller to extract function names
    let source = std::fs::read_to_string(controller_path).map_err(|e| RuntimeError::General {
        message: format!("Failed to read controller file: {}", e),
        span: Span::default(),
    })?;

    // Derive routes from the controller
    let routes = derive_routes_from_controller(controller_name, &source)?;

    // Execute the controller file to define functions
    execute_file(interpreter, controller_path)?;

    // Register routes using the interpreter's environment
    for route in routes {
        // Look up the function in the environment
        let func_value = interpreter
            .environment
            .borrow()
            .get(&route.function_name)
            .ok_or_else(|| RuntimeError::General {
                message: format!(
                    "Function '{}' not found in controller {}",
                    route.function_name, controller_name
                ),
                span: Span::default(),
            })?;

        // Register action in global registry for DSL lookup
        let controller_key = controller_name.trim_end_matches("_controller");
        crate::interpreter::builtins::router::register_controller_action(
            controller_key,
            &route.function_name,
            func_value.clone(),
        );

        println!(
            "  {} {} -> {}()",
            route.method, route.path, route.function_name
        );

        register_route_with_handler(&route.method, &route.path, func_value);
    }

    Ok(())
}

/// Execute a Soli file with the given interpreter.
fn execute_file(interpreter: &mut Interpreter, path: &Path) -> Result<(), RuntimeError> {
    let source = std::fs::read_to_string(path).map_err(|e| RuntimeError::General {
        message: format!("Failed to read file '{}': {}", path.display(), e),
        span: Span::default(),
    })?;

    // Lex
    let tokens = crate::lexer::Scanner::new(&source)
        .scan_tokens()
        .map_err(|e| RuntimeError::General {
            message: format!("Lexer error in {}: {}", path.display(), e),
            span: Span::default(),
        })?;

    // Parse
    let program =
        crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| RuntimeError::General {
                message: format!("Parser error in {}: {}", path.display(), e),
                span: Span::default(),
            })?;

    // Execute (skip type checking for flexibility)
    interpreter.interpret(&program)
}

/// Recursively track view files for hot reload.
fn track_view_files(views_dir: &Path, file_tracker: &mut FileTracker) -> Result<(), RuntimeError> {
    fn track_recursive(dir: &Path, file_tracker: &mut FileTracker) -> Result<(), RuntimeError> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir).map_err(|e| RuntimeError::General {
            message: format!("Failed to read views directory: {}", e),
            span: Span::default(),
        })? {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_dir() {
                    track_recursive(&path, file_tracker)?;
                } else if path.extension().map_or(false, |ext| ext == "erb") {
                    file_tracker.track(&path);
                }
            }
        }
        Ok(())
    }

    track_recursive(views_dir, file_tracker)
}

/// Run the MVC HTTP server using hyper (high-performance async).
fn run_hyper_server(
    mut interpreter: Interpreter,
    port: u16,
    _controllers_dir: PathBuf,
    models_dir: PathBuf,
    middleware_dir: PathBuf,
    public_dir: PathBuf,
    mut file_tracker: FileTracker,
    live_reload: bool,
) -> Result<(), RuntimeError> {
    // Channel for sending requests to the interpreter (main) thread
    // Use std::sync::mpsc for the interpreter side since it needs blocking_recv
    let (request_tx, request_rx) = std::sync::mpsc::channel::<RequestData>();

    // Create broadcast channel for live reload if enabled
    let reload_tx = if live_reload {
        let (tx, _) = broadcast::channel::<()>(16);
        Some(tx)
    } else {
        None
    };
    let reload_tx_for_tokio = reload_tx.clone();

    // Create WebSocket registry
    let ws_registry = Arc::new(WebSocketRegistry::new());

    // Create channel for sending WebSocket events to the interpreter thread
    // Use std::sync::mpsc for the interpreter side since it needs blocking_recv
    let (ws_event_tx, ws_event_rx) = std::sync::mpsc::channel::<WebSocketEventData>();

    // Clone for the interpreter loop
    let ws_registry_for_interpreter = ws_registry.clone();

    // Wrap the sender in an Arc with tokio's Mutex so it can be shared across tokio tasks
    // tokio::sync::Mutex is Send + Sync
    let ws_event_tx_arc = Arc::new(tokio::sync::Mutex::new(Some(ws_event_tx)));
    let ws_event_tx_for_tokio = ws_event_tx_arc.clone();

    println!("\nServer listening on http://0.0.0.0:{}", port);
    println!("Hot reload enabled - edit controllers/middleware/views to see changes");
    if live_reload {
        println!("Live reload enabled - browsers will auto-refresh on changes");
    }
    if public_dir.exists() {
        println!("Static files served from {}", public_dir.display());
    }
    println!("Using hyper async HTTP server\n");

    // Spawn tokio runtime on a separate thread
    let request_tx_clone = request_tx.clone();
    let public_dir_clone = public_dir.clone();
    let ws_registry_for_tokio = ws_registry.clone();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        runtime.block_on(async move {
            let addr = SocketAddr::from(([0, 0, 0, 0], port));
            let listener = TcpListener::bind(addr).await.expect("Failed to bind");

            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => continue,
                };
                let io = TokioIo::new(stream);
                let request_tx = request_tx_clone.clone();
                let reload_tx = reload_tx_for_tokio.clone();
                let public_dir = public_dir_clone.clone();
                let ws_registry = ws_registry_for_tokio.clone();
                // Clone the Arc for this task
                let ws_event_tx_arc = ws_event_tx_for_tokio.clone();

                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let request_tx = request_tx.clone();
                        let reload_tx = reload_tx.clone();
                        let public_dir = public_dir.clone();
                        let ws_event_tx_arc = ws_event_tx_arc.clone();

                        async move {
                            // Get the sender from the Arc using tokio lock
                            let guard = ws_event_tx_arc.lock().await;
                            let has_sender = guard.is_some();
                            drop(guard); // Release the lock

                            if !has_sender {
                                return Ok(Response::builder()
                                    .status(StatusCode::SERVICE_UNAVAILABLE)
                                    .body(Full::new(Bytes::from("Server shutting down")))
                                    .unwrap());
                            }
                            // Get the sender again and clone immediately
                            let guard = ws_event_tx_arc.lock().await;
                            let ws_event_tx = if let Some(ref tx) = *guard {
                                tx.clone()
                            } else {
                                return Ok(Response::builder()
                                    .status(StatusCode::SERVICE_UNAVAILABLE)
                                    .body(Full::new(Bytes::from("Server shutting down")))
                                    .unwrap());
                            };
                            drop(guard); // Release the lock before await
                            handle_hyper_request(
                                req,
                                request_tx,
                                reload_tx,
                                public_dir,
                                ws_event_tx,
                            )
                            .await
                        }
                    });

                    if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                        if !e.to_string().contains("connection closed") {
                            eprintln!("Connection error: {}", e);
                        }
                    }
                });
            }
        });
    });

    // Run interpreter loop on main thread (interpreter is not Send)
    // Also pass WebSocket registry and event channel
    interpreter_loop(
        &mut interpreter,
        &mut file_tracker,
        &models_dir,
        &middleware_dir,
        request_rx,
        reload_tx,
        Some(ws_registry_for_interpreter),
        Some(ws_event_rx),
    );

    Ok(())
}

/// Data for WebSocket events sent to the interpreter thread.
struct WebSocketEventData {
    path: String,
    connection_id: Uuid,
    event_type: String,
    message: Option<String>,
    channel: Option<String>,
    response_tx: oneshot::Sender<WebSocketActionData>,
}

/// Actions to take after processing a WebSocket event.
struct WebSocketActionData {
    join: Option<String>,
    leave: Option<String>,
    send: Option<String>,
    broadcast: Option<String>,
    broadcast_room: Option<String>,
    close: Option<String>,
}

/// Handle a hyper request
async fn handle_hyper_request(
    req: Request<Incoming>,
    request_tx: std::sync::mpsc::Sender<RequestData>,
    reload_tx: Option<broadcast::Sender<()>>,
    public_dir: PathBuf,
    ws_event_tx: std::sync::mpsc::Sender<WebSocketEventData>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().to_string().to_uppercase();
    let uri = req.uri();
    let path = uri.path().to_string();

    eprintln!("[HTTP] {} {}", method, path);

    // Check for WebSocket upgrade request
    if is_websocket_upgrade(&req) {
        eprintln!("[WS] WebSocket upgrade request for path: {}", path);

        // Check if there's a WebSocket route for this path
        let routes = crate::serve::websocket::get_websocket_routes();
        eprintln!(
            "[WS] Registered routes: {:?}",
            routes.iter().map(|r| &r.path_pattern).collect::<Vec<_>>()
        );

        let has_ws_route = routes.iter().any(|r| r.path_pattern == path);
        eprintln!("[WS] Has route for {}: {}", path, has_ws_route);

        if has_ws_route {
            // Get the global WebSocket registry
            let ws_registry = crate::serve::websocket::get_ws_registry();
            return handle_websocket_upgrade(req, ws_registry, path, ws_event_tx).await;
        } else {
            // No WebSocket route found
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from("WebSocket endpoint not found")))
                .unwrap());
        }
    }

    // Check for static file in public directory
    if method == "GET" && public_dir.exists() {
        let relative_path = path.trim_start_matches('/');
        // Do not allow directory traversal or absolute paths in URL
        if !relative_path.contains("..") && !relative_path.starts_with('/') {
            let file_path = public_dir.join(relative_path);

            if file_path.exists() && file_path.is_file() {
                let content = match std::fs::read(&file_path) {
                    Ok(c) => c,
                    Err(_) => {
                        return Ok(Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Full::new(Bytes::from("Error reading file")))
                            .unwrap())
                    }
                };

                let mime_type = match file_path.extension().and_then(|e| e.to_str()) {
                    Some("css") => "text/css",
                    Some("js") => "application/javascript",
                    Some("png") => "image/png",
                    Some("jpg") | Some("jpeg") => "image/jpeg",
                    Some("ico") => "image/x-icon",
                    Some("svg") => "image/svg+xml",
                    Some("html") => "text/html",
                    Some("json") => "application/json",
                    _ => "application/octet-stream",
                };

                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", mime_type)
                    .body(Full::new(Bytes::from(content)))
                    .unwrap());
            }
        }
    }

    // Handle live reload SSE endpoint
    if path == "/__livereload" {
        if let Some(ref tx) = reload_tx {
            return Ok(live_reload::handle_live_reload_sse(tx.subscribe()).await);
        } else {
            // Live reload disabled
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from("Live reload is disabled")))
                .unwrap());
        }
    }

    let query_str = uri.query().unwrap_or("");

    // Parse query string
    let query = parse_query_string(query_str);

    // Extract headers
    let mut headers = HashMap::new();
    for (name, value) in req.headers() {
        if let Ok(v) = value.to_str() {
            headers.insert(name.to_string(), v.to_string());
        }
    }

    // Read body
    let body_bytes = http_body_util::BodyExt::collect(req.into_body())
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let body = String::from_utf8_lossy(&body_bytes).to_string();

    // Create oneshot channel for response
    let (response_tx, response_rx) = oneshot::channel();

    // Send to interpreter thread
    let request_data = RequestData {
        method,
        path,
        query,
        headers,
        body,
        response_tx,
    };

    if request_tx.send(request_data).is_err() {
        return Ok(Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Full::new(Bytes::from("Server shutting down")))
            .unwrap());
    }

    // Wait for response
    match response_rx.await {
        Ok(resp_data) => {
            let mut builder = Response::builder()
                .status(StatusCode::from_u16(resp_data.status).unwrap_or(StatusCode::OK));

            for (key, value) in resp_data.headers {
                builder = builder.header(key, value);
            }

            Ok(builder
                .body(Full::new(Bytes::from(resp_data.body)))
                .unwrap())
        }
        Err(_) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from("Internal Server Error")))
            .unwrap()),
    }
}

/// Check if the request is a WebSocket upgrade request.
fn is_websocket_upgrade(req: &Request<Incoming>) -> bool {
    if req.method() != hyper::Method::GET {
        return false;
    }

    if let Some(upgrade_header) = req.headers().get(header::UPGRADE) {
        eprintln!("[WS] Upgrade header: {:?}", upgrade_header);
        return upgrade_header == "websocket";
    }

    false
}

/// Handle WebSocket upgrade request.
async fn handle_websocket_upgrade(
    req: Request<Incoming>,
    ws_registry: Arc<WebSocketRegistry>,
    path: String,
    ws_event_tx: std::sync::mpsc::Sender<WebSocketEventData>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    eprintln!("[WS] handle_websocket_upgrade called for path: {}", path);

    // Create WebSocket config
    let config = WebSocketConfig::default();

    // Check if there's an upgrade header
    if !is_websocket_upgrade(&req) {
        eprintln!("[WS] No upgrade header found");
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Full::new(Bytes::from("Not a WebSocket upgrade request")))
            .unwrap());
    }

    eprintln!("[WS] Upgrade header present, creating upgrade response...");

    // For WebSocket, we need to handle the upgrade differently
    // Spawn a task to handle the WebSocket connection
    let ws_registry = ws_registry.clone();
    let ws_event_tx = ws_event_tx.clone();
    let path = path.clone();
    let config = config.clone();

    tokio::spawn(async move {
        eprintln!("[WS] Handling WebSocket upgrade...");

        // Use hyper's upgrade
        match hyper::upgrade::on(req).await {
            Ok(upgraded) => {
                eprintln!("[WS] Upgrade successful, creating WebSocket stream...");

                // Wrap with TokioIo
                let mut io = TokioIo::new(upgraded);

                // Complete the WebSocket handshake using tungstenite
                eprintln!("[WS] Completing WebSocket handshake...");

                // Read the HTTP request to get the Sec-WebSocket-Key
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut http_request = String::new();
                match io.read_to_string(&mut http_request).await {
                    Ok(_) => eprintln!("[WS] Read HTTP request ({} bytes)", http_request.len()),
                    Err(e) => {
                        eprintln!("[WS] Failed to read HTTP request: {}", e);
                        return;
                    }
                }

                // Parse the Sec-WebSocket-Key from the request
                let sec_websocket_key = http_request
                    .lines()
                    .find(|line| line.to_lowercase().starts_with("sec-websocket-key:"))
                    .and_then(|line| line.split(":").nth(1))
                    .map(|s| s.trim().as_bytes());

                if sec_websocket_key.is_none() {
                    eprintln!("[WS] No Sec-WebSocket-Key found in request");
                    return;
                }

                // Generate the Sec-WebSocket-Accept header
                let key = sec_websocket_key.unwrap();
                let accept = tungstenite::handshake::derive_accept_key(key);

                // Write the HTTP response
                let response = format!(
                    "HTTP/1.1 101 Switching Protocols\r\n\
                     Upgrade: websocket\r\n\
                     Connection: upgrade\r\n\
                     Sec-WebSocket-Accept: {}\r\n\
                     \r\n",
                    accept
                );
                if let Err(e) = io.write_all(response.as_bytes()).await {
                    eprintln!("[WS] Failed to write response: {}", e);
                    return;
                }
                eprintln!("[WS] Sent 101 response");

                // Now create the WebSocket stream
                let mut stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
                    io,
                    tungstenite::protocol::Role::Server,
                    Some(config),
                )
                .await;
                eprintln!("[WS] WebSocket stream created");

                // Create connection in registry
                let (ws_tx, mut ws_rx) = tokio::sync::mpsc::channel::<
                    Result<tungstenite::Message, tungstenite::Error>,
                >(32);
                let ws_tx_arc = Arc::new(ws_tx);
                let connection = WebSocketConnection::new(ws_tx_arc.clone());
                let connection_id = connection.id;

                ws_registry.register(connection).await;

                // Send connect event
                let (response_tx, _) = oneshot::channel();
                let connect_event = WebSocketEventData {
                    path: path.clone(),
                    connection_id,
                    event_type: "connect".to_string(),
                    message: None,
                    channel: None,
                    response_tx,
                };
                let _ = ws_event_tx.send(connect_event);

                // Handle messages
                while let Some(msg_result) = stream.next().await {
                    match msg_result {
                        Ok(msg) => {
                            if msg.is_text() || msg.is_binary() {
                                if let Ok(text) = msg.to_text() {
                                    let (response_tx, _) = oneshot::channel();
                                    let msg_event = WebSocketEventData {
                                        path: path.clone(),
                                        connection_id,
                                        event_type: "message".to_string(),
                                        message: Some(text.to_string()),
                                        channel: None,
                                        response_tx,
                                    };
                                    let _ = ws_event_tx.send(msg_event);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[WS] Message error: {}", e);
                            break;
                        }
                    }
                }

                // Send disconnect event
                let (response_tx, _) = oneshot::channel();
                let disconnect_event = WebSocketEventData {
                    path: path.clone(),
                    connection_id,
                    event_type: "disconnect".to_string(),
                    message: None,
                    channel: None,
                    response_tx,
                };
                let _ = ws_event_tx.send(disconnect_event);

                ws_registry.unregister(&connection_id).await;
                eprintln!("[WS] Connection closed for: {}", connection_id);
            }
            Err(e) => {
                eprintln!("[WS] Upgrade error: {}", e);
            }
        }
    });

    // Create the 101 response with proper WebSocket headers
    let response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::UPGRADE, "websocket")
        .header(header::CONNECTION, "upgrade")
        .header("Sec-WebSocket-Accept", "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
        .body(Full::new(Bytes::new()))
        .unwrap();

    eprintln!("[WS] Returning 101 Switching Protocols");
    return Ok(response);
}

/// Handle WebSocket stream for a single connection.
async fn handle_websocket_stream<S>(
    mut stream: WebSocketStream<S>,
    ws_rx: &mut tokio::sync::mpsc::Receiver<Result<tungstenite::Message, tungstenite::Error>>,
    connection_id: Uuid,
    ws_registry: Arc<WebSocketRegistry>,
    path: String,
    ws_event_tx: std::sync::mpsc::Sender<WebSocketEventData>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    eprintln!(
        "[WS] handle_websocket_stream started for connection: {}",
        connection_id
    );

    // Create a oneshot channel for sending events to interpreter and getting actions back
    let (response_tx, response_rx) = oneshot::channel();

    // Send connect event to interpreter thread
    let connect_event = WebSocketEventData {
        path: path.clone(),
        connection_id,
        event_type: "connect".to_string(),
        message: None,
        channel: None,
        response_tx,
    };

    if let Err(e) = ws_event_tx.send(connect_event) {
        eprintln!("Failed to send WebSocket connect event: {}", e);
    }

    // Wait for handler response (don't block forever, max 5 seconds)
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), response_rx).await;

    // Send ping to client
    let _ = stream.send(tungstenite::Message::Ping(vec![])).await;

    // Create a loop to handle both incoming messages and outgoing messages
    let (mut ws_sender, mut ws_receiver) = stream.split();

    // Forward messages from ws_rx to the WebSocket
    let forward_task = async {
        while let Some(msg) = ws_rx.recv().await {
            if let Err(e) = ws_sender
                .send(msg.unwrap_or_else(|_| tungstenite::Message::Close(None)))
                .await
            {
                eprintln!("WebSocket send error: {}", e);
                break;
            }
        }
    };

    // Handle incoming messages
    let receive_task = async {
        while let Some(result) = ws_receiver.next().await {
            match result {
                Ok(msg) => {
                    if msg.is_close() {
                        break;
                    }

                    if msg.is_pong() {
                        continue;
                    }

                    if let Ok(text) = msg.to_text() {
                        // Create oneshot channel for this message event
                        let (msg_response_tx, msg_response_rx) = oneshot::channel();

                        // Send message event to interpreter thread
                        let event = WebSocketEventData {
                            path: path.clone(),
                            connection_id,
                            event_type: "message".to_string(),
                            message: Some(text.to_string()),
                            channel: None,
                            response_tx: msg_response_tx,
                        };

                        if let Err(e) = ws_event_tx.send(event) {
                            eprintln!("Failed to send WebSocket message event: {}", e);
                        }

                        // Wait for handler response (don't block forever, max 5 seconds)
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            msg_response_rx,
                        )
                        .await;
                    }
                }
                Err(e) => {
                    eprintln!("WebSocket receive error: {}", e);
                    break;
                }
            }
        }
    };

    // Wait for either task to complete
    tokio::select! {
        _ = forward_task => {},
        _ = receive_task => {},
    }

    // Clean up: unregister and send disconnect event
    ws_registry.unregister(&connection_id).await;

    // Send disconnect event to interpreter thread
    let (disconnect_response_tx, _) = oneshot::channel();
    let disconnect_event = WebSocketEventData {
        path: path.clone(),
        connection_id,
        event_type: "disconnect".to_string(),
        message: None,
        channel: None,
        response_tx: disconnect_response_tx,
    };

    let _ = ws_event_tx.send(disconnect_event);
}

/// Interpreter loop - executes handlers sequentially (runs on main thread)
fn interpreter_loop(
    interpreter: &mut Interpreter,
    file_tracker: &mut FileTracker,
    models_dir: &Path,
    middleware_dir: &Path,
    request_rx: std::sync::mpsc::Receiver<RequestData>,
    reload_tx: Option<broadcast::Sender<()>>,
    ws_registry: Option<Arc<WebSocketRegistry>>,
    ws_event_rx: Option<std::sync::mpsc::Receiver<WebSocketEventData>>,
) {
    use std::sync::mpsc::RecvTimeoutError;

    let check_interval = std::time::Duration::from_millis(500);

    // Clone the options for use in the loop
    let mut ws_event_rx_inner = ws_event_rx;
    let ws_registry_inner = ws_registry;

    loop {
        // Check for file changes
        let changed_files = file_tracker.get_changed_files();
        if !changed_files.is_empty() {
            handle_hot_reload(
                interpreter,
                file_tracker,
                &changed_files,
                models_dir,
                middleware_dir,
            );

            // Send live reload signal to connected browsers
            if let Some(ref tx) = reload_tx {
                let _ = tx.send(());
                println!("   Sent reload signal to browsers\n");
            }
        }

        // Non-blocking receive with timeout so we can check for file changes periodically
        // First check WebSocket events
        if let (Some(ref mut rx), Some(ref _registry)) =
            (ws_event_rx_inner.as_mut(), ws_registry_inner.as_ref())
        {
            match rx.recv_timeout(check_interval) {
                Ok(data) => {
                    handle_websocket_event(interpreter, &data);
                    let _ = data.response_tx.send(WebSocketActionData {
                        join: None,
                        leave: None,
                        send: None,
                        broadcast: None,
                        broadcast_room: None,
                        close: None,
                    });
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    // Channel closed, disable WebSocket event handling
                    ws_event_rx_inner = None;
                }
            }
        }

        // Then check HTTP requests
        match request_rx.recv_timeout(check_interval) {
            Ok(data) => {
                let resp_data = handle_request(interpreter, &data);
                let _ = data.response_tx.send(resp_data);
            }
            Err(RecvTimeoutError::Timeout) => {
                // Timeout - loop back to check for file changes
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => {
                // Channel closed, exit
                return;
            }
        }
    }
}

/// Handle a WebSocket event by calling the handler function.
fn handle_websocket_event(interpreter: &mut Interpreter, data: &WebSocketEventData) {
    use crate::interpreter::value::Value;

    // Clone connection_id for use in async spawns
    let connection_id = data.connection_id.to_string();

    // Find the WebSocket route for this path
    let routes = crate::serve::websocket::get_websocket_routes();
    let route = match routes.iter().find(|r| r.path_pattern == data.path) {
        Some(r) => r,
        None => {
            println!("[WS] No handler found for path {}", data.path);
            return;
        }
    };

    // Look up the handler from CONTROLLERS registry using the handler_name
    let handler =
        match crate::interpreter::builtins::router::resolve_handler(&route.handler_name, None) {
            Ok(h) => h,
            Err(e) => {
                eprintln!(
                    "[WS] Failed to resolve handler '{}': {}",
                    route.handler_name, e
                );
                return;
            }
        };

    // Build event hash: {type, connection_id, message, channel?}
    let mut event_pairs: Vec<(Value, Value)> = vec![
        (
            Value::String("type".to_string()),
            Value::String(data.event_type.clone()),
        ),
        (
            Value::String("connection_id".to_string()),
            Value::String(connection_id.clone()),
        ),
    ];

    if let Some(ref msg) = data.message {
        event_pairs.push((
            Value::String("message".to_string()),
            Value::String(msg.clone()),
        ));
    }

    if let Some(ref channel) = data.channel {
        event_pairs.push((
            Value::String("channel".to_string()),
            Value::String(channel.clone()),
        ));
    }

    let event_value = Value::Hash(Rc::new(RefCell::new(event_pairs)));

    // Call the handler function
    match interpreter.call_value(handler, vec![event_value], Span::default()) {
        Ok(result) => {
            println!("[WS] Handler returned: {:?}", result);

            // Handle broadcast response from handler
            if let Value::Hash(hash) = &result {
                for (k, v) in hash.borrow().iter() {
                    if let (Value::String(key), Value::String(value)) = (k, v) {
                        match key.as_str() {
                            "broadcast" => {
                                // Broadcast to all clients
                                let registry = crate::serve::websocket::get_ws_registry();
                                let registry_clone = registry.clone();
                                let msg = value.clone();
                                tokio::spawn(async move {
                                    registry_clone.broadcast_all(&msg).await;
                                });
                            }
                            "send" => {
                                // Send to this specific client
                                let registry = crate::serve::websocket::get_ws_registry();
                                let registry_clone = registry.clone();
                                let msg = value.clone();
                                if let Ok(uuid) = connection_id.parse::<uuid::Uuid>() {
                                    tokio::spawn(async move {
                                        registry_clone.send_to(&uuid, &msg).await.ok();
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[WS] Handler error: {}", e);
        }
    }
}

/// Handle a single request (called on interpreter thread)
fn handle_request(interpreter: &mut Interpreter, data: &RequestData) -> ResponseData {
    // Get current routes
    let routes = get_routes();

    // Find matching route and extract params
    let mut matched_route = None;
    let mut matched_params = HashMap::new();
    for route in &routes {
        if route.method == data.method {
            if let Some(params) = match_path(&route.path_pattern, &data.path) {
                matched_route = Some(route);
                matched_params = params;
                break;
            }
        }
    }

    let route = match matched_route {
        Some(r) => r,
        None => {
            return ResponseData {
                status: 404,
                headers: vec![],
                body: "Not Found".to_string(),
            };
        }
    };

    // Build request hash with extracted params
    let mut request_hash = build_request_hash(
        &data.method,
        &data.path,
        matched_params,
        data.query.clone(),
        data.headers.clone(),
        data.body.clone(),
    );

    // Execute middleware chain: scoped middleware first, then global middleware
    // Scoped middleware runs first so namespace/controller-specific middleware runs before global
    let scoped_middleware: Vec<_> = route.middleware.iter().collect();
    let global_middleware = get_middleware();

    // Execute scoped (route-specific) middleware
    for mw in scoped_middleware {
        match interpreter.call_value(mw.clone(), vec![request_hash.clone()], Span::default()) {
            Ok(result) => match extract_middleware_result(&result) {
                MiddlewareResult::Continue(modified_request) => {
                    request_hash = modified_request;
                }
                MiddlewareResult::Response(resp) => {
                    let (status, headers, body) = extract_response(&resp);
                    let headers: Vec<_> = headers.into_iter().collect();
                    return ResponseData {
                        status,
                        headers,
                        body,
                    };
                }
                MiddlewareResult::Error(err) => {
                    return ResponseData {
                        status: 500,
                        headers: vec![],
                        body: format!("Middleware error: {}", err),
                    };
                }
            },
            Err(e) => {
                return ResponseData {
                    status: 500,
                    headers: vec![],
                    body: format!("Middleware error: {}", e),
                };
            }
        }
    }

    // Execute global middleware
    // Skip global_only middleware if route has scoped middleware
    // Skip scope_only middleware (only runs when explicitly scoped)
    let has_scoped_middleware = !route.middleware.is_empty();
    for mw in &global_middleware {
        // Skip global-only middleware when route has scoped middleware
        if has_scoped_middleware && mw.global_only {
            continue;
        }
        // Skip scope_only middleware (it only runs when explicitly scoped)
        if mw.scope_only {
            continue;
        }

        match interpreter.call_value(
            mw.handler.clone(),
            vec![request_hash.clone()],
            Span::default(),
        ) {
            Ok(result) => match extract_middleware_result(&result) {
                MiddlewareResult::Continue(modified_request) => {
                    request_hash = modified_request;
                }
                MiddlewareResult::Response(resp) => {
                    let (status, headers, body) = extract_response(&resp);
                    let headers: Vec<_> = headers.into_iter().collect();
                    return ResponseData {
                        status,
                        headers,
                        body,
                    };
                }
                MiddlewareResult::Error(err) => {
                    return ResponseData {
                        status: 500,
                        headers: vec![],
                        body: format!("Middleware error: {}", err),
                    };
                }
            },
            Err(e) => {
                return ResponseData {
                    status: 500,
                    headers: vec![],
                    body: format!("Middleware error: {}", e),
                };
            }
        }
    }

    // Call the route handler
    match interpreter.call_value(route.handler.clone(), vec![request_hash], Span::default()) {
        Ok(result) => {
            let (status, headers, body) = extract_response(&result);
            let headers: Vec<_> = headers.into_iter().collect();
            ResponseData {
                status,
                headers,
                body,
            }
        }
        Err(e) => ResponseData {
            status: 500,
            headers: vec![],
            body: format!("Internal Server Error: {}", e),
        },
    }
}

/// Handle hot reload of changed files
fn handle_hot_reload(
    interpreter: &mut Interpreter,
    file_tracker: &mut FileTracker,
    changed_files: &[PathBuf],
    models_dir: &Path,
    middleware_dir: &Path,
) {
    println!("\n🔄 Hot reload triggered for:");
    for path in changed_files {
        println!("   {}", path.display());
    }

    // Check if any middleware files changed - reload all middleware
    let middleware_changed = changed_files.iter().any(|p| p.starts_with(middleware_dir));
    if middleware_changed && middleware_dir.exists() {
        if let Err(e) = load_middleware(interpreter, middleware_dir, file_tracker) {
            eprintln!("Error reloading middleware: {}", e);
        } else {
            println!("   ✓ Reloaded middleware");
        }
    }

    // Reload changed controllers, models, and views
    let mut views_changed = false;
    for path in changed_files {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with("_controller.soli") {
                // Clear existing routes for this controller
                let controller_name = name.trim_end_matches(".soli");
                let base_path = router::controller_base_path(controller_name);
                clear_routes_for_controller(&base_path);

                // Reload controller
                if let Err(e) = load_controller(interpreter, path, file_tracker) {
                    eprintln!("Error reloading {}: {}", name, e);
                } else {
                    println!("   ✓ Reloaded {}", name);
                }
            } else if name.ends_with(".soli") && path.starts_with(models_dir) {
                // Reload model
                if let Err(e) = execute_file(interpreter, path) {
                    eprintln!("Error reloading model {}: {}", name, e);
                } else {
                    println!("   ✓ Reloaded model {}", name);
                }
            } else if name.ends_with(".erb") {
                // View file changed
                views_changed = true;
            }
        }
        // Update tracking
        file_tracker.track(path);
    }

    // Clear template cache if any view files changed
    if views_changed {
        clear_template_cache();
        println!("   ✓ Cleared template cache");
    }
    println!();
}

/// Clear routes that match a specific controller's base path.
fn clear_routes_for_controller(base_path: &str) {
    crate::interpreter::builtins::server::clear_routes_for_prefix(base_path);
}
