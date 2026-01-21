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
    get_middleware_by_name, has_middleware, register_middleware, register_middleware_with_options,
    scan_middleware_files, with_middleware, Middleware, MiddlewareResult,
};
pub use router::{derive_routes_from_controller, ControllerRoute};
pub use crate::interpreter::builtins::router::{get_controllers, set_controllers};
pub use websocket::{
    clear_websocket_routes, get_websocket_routes, match_websocket_route, register_websocket_route,
    WebSocketConnection, WebSocketEvent, WebSocketHandlerAction, WebSocketRegistry,
};

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use crossbeam::channel;
use futures_util::SinkExt;
use futures_util::StreamExt;
use http_body_util::Full;
use hyper::body::Incoming;
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
    build_request_hash, extract_response, find_route, get_routes, match_path, parse_query_string,
    rebuild_route_index,
    register_route_with_handler, routes_to_worker_routes, set_worker_routes, WorkerRoute,
};
use crate::interpreter::builtins::template::{clear_template_cache, init_templates};
use crate::interpreter::builtins::controller::controller::ControllerInfo;
use crate::interpreter::builtins::controller::CONTROLLER_REGISTRY;
use crate::interpreter::{Interpreter, Value};
use crate::span::Span;
use crate::ExecutionMode;

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
    // Default to number of CPU cores for optimal parallelism
    let num_workers = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4); // Fallback to 4 if unable to detect
    serve_folder_with_options_and_mode(folder, port, live_reload, ExecutionMode::Bytecode, num_workers)
}

/// Serve an MVC application from a folder with configurable options and execution mode.
pub fn serve_folder_with_options_and_mode(
    folder: &Path,
    port: u16,
    live_reload: bool,
    mode: ExecutionMode,
    workers: usize,
) -> Result<(), RuntimeError> {
    // Set up panic hook to catch worker panics
    std::panic::set_hook(Box::new(|panic_info| {
        let msg = panic_info.to_string();
        eprintln!("PANIC: {}", msg);
    }));

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
    println!("Execution mode: {:?}", mode);

    // Create interpreter or bytecode compiler based on mode
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

    // Always use hyper-based MVC server
    run_hyper_server_worker_pool(
        port,
        controllers_dir,
        models_dir,
        middleware_dir,
        public_dir,
        file_tracker,
        live_reload,
        mode,
        workers,
        views_dir,
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

        // Create full handler name: controller#action
        let full_handler_name = format!("{}#{}", controller_key, route.function_name);

        println!(
            "  {} {} -> {}()",
            route.method, route.path, route.function_name
        );

        register_route_with_handler(&route.method, &route.path, full_handler_name);
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

/// Hot reload version counters - shared between file watcher and workers.
/// Workers periodically check if versions changed and reload accordingly.
struct HotReloadVersions {
    /// Incremented when controllers change
    controllers: AtomicU64,
    /// Incremented when middleware changes
    middleware: AtomicU64,
    /// Incremented when views change
    views: AtomicU64,
}

impl HotReloadVersions {
    fn new() -> Self {
        Self {
            controllers: AtomicU64::new(0),
            middleware: AtomicU64::new(0),
            views: AtomicU64::new(0),
        }
    }
}

use std::sync::atomic::AtomicU64;

/// Per-worker queue for distributing requests without contention.
/// Each worker has its own dedicated channel, eliminating receiver contention.
struct WorkerQueues {
    senders: Vec<channel::Sender<RequestData>>,
    receivers: Vec<channel::Receiver<RequestData>>,
}

impl WorkerQueues {
    fn new(num_workers: usize, capacity_per_worker: usize) -> Self {
        let mut senders = Vec::with_capacity(num_workers);
        let mut receivers = Vec::with_capacity(num_workers);

        for _ in 0..num_workers {
            let (tx, rx) = channel::bounded(capacity_per_worker);
            senders.push(tx);
            receivers.push(rx);
        }

        Self { senders, receivers }
    }

    /// Get a sender that round-robins across workers (lock-free)
    fn get_sender(&self) -> WorkerSender {
        WorkerSender {
            senders: self.senders.clone(),
            next_worker: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Get the receiver for a specific worker
    fn get_receiver(&self, worker_id: usize) -> channel::Receiver<RequestData> {
        self.receivers[worker_id].clone()
    }
}

/// A sender that distributes requests across workers using round-robin
#[derive(Clone)]
struct WorkerSender {
    senders: Vec<channel::Sender<RequestData>>,
    next_worker: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl WorkerSender {
    fn send(&self, data: RequestData) -> Result<(), channel::SendError<RequestData>> {
        // Round-robin distribution (lock-free)
        let worker = self.next_worker.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.senders.len();
        self.senders[worker].send(data)
    }
}

/// Run the MVC HTTP server with a worker pool for parallel request processing.
fn run_hyper_server_worker_pool(
    port: u16,
    controllers_dir: PathBuf,
    models_dir: PathBuf,
    middleware_dir: PathBuf,
    public_dir: PathBuf,
    _file_tracker: FileTracker,
    live_reload: bool,
    _mode: ExecutionMode,
    num_workers: usize,
    views_dir: PathBuf,
) -> Result<(), RuntimeError> {
    let reload_tx = if live_reload {
        let (tx, _) = broadcast::channel::<()>(16);
        Some(tx)
    } else {
        None
    };
    let reload_tx_for_tokio = reload_tx.clone();

    let ws_registry = Arc::new(WebSocketRegistry::new());

    // Bounded channels for backpressure
    let capacity_per_worker = 64;
    let (ws_event_tx, ws_event_rx) = channel::bounded(num_workers * capacity_per_worker);
    // crossbeam Sender is cheap to clone - no need for Arc<Mutex<Option<>>>
    // Use AtomicBool for shutdown signaling (lock-free check)
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_for_tokio = shutdown_flag.clone();

    // Per-worker queues eliminate receiver contention
    let worker_queues = Arc::new(WorkerQueues::new(num_workers, capacity_per_worker));
    let worker_queues_for_tokio = worker_queues.clone();

    println!("\nServer listening on http://0.0.0.0:{}", port);
    println!("Hot reload enabled - edit controllers/middleware/views to see changes");
    if live_reload {
        println!("Live reload enabled - browsers will auto-refresh on changes");
    }
    if public_dir.exists() {
        println!("Static files served from {}", public_dir.display());
    }
    println!("Using hyper async HTTP server with {} worker threads\n", num_workers);

    // Wrap public_dir in Arc for cheap cloning across connections
    let public_dir_arc = Arc::new(public_dir.clone());
    let ws_registry_for_tokio = ws_registry.clone();

    // Spawn tokio runtime for HTTP server
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
                let request_tx = worker_queues_for_tokio.get_sender();
                let reload_tx = reload_tx_for_tokio.clone();
                let public_dir = public_dir_arc.clone(); // Arc clone is cheap
                let _ws_registry = ws_registry_for_tokio.clone();
                let ws_event_tx = ws_event_tx.clone(); // crossbeam Sender is cheap to clone
                let shutdown_flag = shutdown_flag_for_tokio.clone();

                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let request_tx = request_tx.clone();
                        let reload_tx = reload_tx.clone();
                        let public_dir = public_dir.clone(); // Arc clone is cheap
                        let ws_event_tx = ws_event_tx.clone();
                        let shutdown_flag = shutdown_flag.clone();

                        async move {
                            // Lock-free shutdown check (AtomicBool)
                            if shutdown_flag.load(Ordering::Relaxed) {
                                return Ok(Response::builder()
                                    .status(StatusCode::SERVICE_UNAVAILABLE)
                                    .body(Full::new(Bytes::from("Server shutting down")))
                                    .unwrap());
                            }
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

                    if let Err(_e) = http1::Builder::new().serve_connection(io, service).await {
                        // Silently ignore connection errors
                    }
                });
            }
        });
    });

    // Hot reload version counters (shared between file watcher and workers)
    let hot_reload_versions = Arc::new(HotReloadVersions::new());
    let hot_reload_versions_for_watcher = hot_reload_versions.clone();

    // Spawn file watcher thread for hot reload
    let watch_controllers_dir = controllers_dir.clone();
    let watch_views_dir = views_dir.clone();
    let watch_middleware_dir = middleware_dir.clone();
    let browser_reload_tx = reload_tx.clone();
    thread::spawn(move || {
        let mut file_tracker = FileTracker::new();

        // Track all controller files
        if let Ok(entries) = std::fs::read_dir(&watch_controllers_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "soli") {
                    file_tracker.track(&path);
                }
            }
        }

        // Track middleware files
        if let Ok(entries) = std::fs::read_dir(&watch_middleware_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "soli") {
                    file_tracker.track(&path);
                }
            }
        }

        // Track view files recursively
        fn track_views_recursive(dir: &Path, tracker: &mut FileTracker) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        track_views_recursive(&path, tracker);
                    } else if path.extension().map_or(false, |ext| ext == "erb") {
                        tracker.track(&path);
                    }
                }
            }
        }
        track_views_recursive(&watch_views_dir, &mut file_tracker);

        println!("Hot reload: Watching {} files", file_tracker.tracked_count());

        loop {
            thread::sleep(Duration::from_secs(1));

            let changed = file_tracker.get_changed_files();
            if changed.is_empty() {
                continue;
            }

            println!("\n🔄 Hot reload triggered for:");
            let mut views_changed = false;
            let mut controllers_changed = false;
            let mut middleware_changed = false;

            for path in &changed {
                println!("   {}", path.display());

                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with("_controller.soli") {
                        controllers_changed = true;
                    } else if name.ends_with(".soli") && path.starts_with(&watch_middleware_dir) {
                        middleware_changed = true;
                    } else if name.ends_with(".erb") {
                        views_changed = true;
                    }
                }

                // Track new modification time
                file_tracker.track(path);
            }

            // Increment version counters - workers will pick this up
            if controllers_changed {
                hot_reload_versions_for_watcher.controllers.fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled controller reload to all workers");
            }
            if middleware_changed {
                hot_reload_versions_for_watcher.middleware.fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled middleware reload to all workers");
            }
            if views_changed {
                hot_reload_versions_for_watcher.views.fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled template cache clear to all workers");
            }

            // Notify browser for live reload
            if let Some(ref tx) = browser_reload_tx {
                let _ = tx.send(());
            }

            println!();
        }
    });

    // Spawn worker threads
    let mut workers = Vec::new();
    // Get routes in main thread and convert to worker-safe formats
    let routes = get_routes();
    let worker_routes = routes_to_worker_routes(&routes);

    for i in 0..num_workers {
        // Each worker gets its own dedicated receiver (no contention!)
        let work_rx = worker_queues.get_receiver(i);
        let models_dir = models_dir.clone();
        let middleware_dir = middleware_dir.clone();
        let ws_event_rx = ws_event_rx.clone();
        let ws_registry = ws_registry.clone();
        let reload_tx = reload_tx.clone();
        let worker_routes = worker_routes.clone();
        let controllers_dir = controllers_dir.clone();
        let views_dir = views_dir.clone();
        let hot_reload_versions = hot_reload_versions.clone();

        let builder = thread::Builder::new().name(format!("worker-{}", i));
        let handler = builder.spawn(move || {
            // Panic catch wrapper
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut interpreter = Interpreter::new();

                worker_loop(i, work_rx, models_dir, middleware_dir, ws_event_rx, ws_registry, reload_tx, &mut interpreter, worker_routes, controllers_dir, views_dir, hot_reload_versions);
            }));

            if result.is_err() {
                eprintln!("Worker {} panicked", i);
            }
        });

        match handler {
            Ok(h) => workers.push(h),
            Err(e) => eprintln!("Failed to spawn worker {}: {}", i, e),
        }
    }
    println!("Started {} worker threads", workers.len());

    // Wait for workers (they run forever until killed)
    for (i, worker) in workers.into_iter().enumerate() {
        match worker.join() {
            Ok(_) => eprintln!("Worker {} exited normally", i),
            Err(e) => eprintln!("Worker {} panicked: {:?}", i, e),
        }
    }

    Ok(())
}

/// Worker loop - processes requests from dedicated per-worker queue
fn worker_loop(
    worker_id: usize,
    work_rx: channel::Receiver<RequestData>,
    _models_dir: PathBuf,
    middleware_dir: PathBuf,
    ws_event_rx: channel::Receiver<WebSocketEventData>,
    ws_registry: Arc<WebSocketRegistry>,
    _reload_tx: Option<broadcast::Sender<()>>,
    interpreter: &mut Interpreter,
    routes: Vec<WorkerRoute>,
    controllers_dir: PathBuf,
    views_dir: PathBuf,
    hot_reload_versions: Arc<HotReloadVersions>,
) {
    // Initialize routes in this worker thread
    set_worker_routes(routes);

    // Initialize template engine in this worker
    if views_dir.exists() {
        crate::interpreter::builtins::template::init_templates(views_dir.clone());
    }

    // Load controllers in this worker so functions are defined in environment
    load_controllers_in_worker(worker_id, interpreter, &controllers_dir);

    let _worker_routes = get_routes();

    let check_interval = Duration::from_millis(100);
    let mut ws_event_rx_inner = Some(ws_event_rx);
    let ws_registry_inner = Some(ws_registry);

    // Track last seen hot reload versions
    let mut last_controllers_version = hot_reload_versions.controllers.load(Ordering::Acquire);
    let mut last_middleware_version = hot_reload_versions.middleware.load(Ordering::Acquire);
    let mut last_views_version = hot_reload_versions.views.load(Ordering::Acquire);

    const BATCH_SIZE: usize = 64;

    loop {
        // Check for hot reload (lock-free version check)
        let current_controllers = hot_reload_versions.controllers.load(Ordering::Acquire);
        let current_middleware = hot_reload_versions.middleware.load(Ordering::Acquire);
        let current_views = hot_reload_versions.views.load(Ordering::Acquire);

        if current_controllers != last_controllers_version {
            last_controllers_version = current_controllers;
            println!("Worker {}: Reloading controllers", worker_id);
            // Re-load all controllers
            load_controllers_in_worker(worker_id, interpreter, &controllers_dir);
        }

        if current_middleware != last_middleware_version {
            last_middleware_version = current_middleware;
            println!("Worker {}: Reloading middleware", worker_id);
            // Clear and reload middleware
            let mut file_tracker = FileTracker::new();
            if let Err(e) = load_middleware(interpreter, &middleware_dir, &mut file_tracker) {
                eprintln!("Worker {}: Error reloading middleware: {}", worker_id, e);
            }
        }

        if current_views != last_views_version {
            last_views_version = current_views;
            println!("Worker {}: Clearing template cache", worker_id);
            clear_template_cache();
        }

        // Process WebSocket events first (quick non-blocking check)
        if let (Some(ref mut rx), Some(ref _registry)) =
            (ws_event_rx_inner.as_mut(), ws_registry_inner.as_ref())
        {
            // Use try_recv for non-blocking check instead of recv_timeout(ZERO)
            match rx.try_recv() {
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
                Err(channel::TryRecvError::Empty) => {}
                Err(channel::TryRecvError::Disconnected) => {
                    ws_event_rx_inner = None;
                }
            }
        }

        // Batch process HTTP requests using try_recv for non-blocking drain
        for _ in 0..BATCH_SIZE {
            match work_rx.try_recv() {
                Ok(data) => {
                    let resp_data = handle_request(interpreter, &data);
                    let _ = data.response_tx.send(resp_data);
                }
                Err(channel::TryRecvError::Empty) => {
                    break;
                }
                Err(channel::TryRecvError::Disconnected) => {
                    return;
                }
            }
        }

        // Block waiting for more requests (proper blocking, not busy-wait)
        if let Ok(data) = work_rx.recv_timeout(check_interval) {
            let resp_data = handle_request(interpreter, &data);
            let _ = data.response_tx.send(resp_data);
        }
    }
}

/// Load all controllers in a worker thread
fn load_controllers_in_worker(worker_id: usize, interpreter: &mut Interpreter, controllers_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(controllers_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "soli") {
                if let Err(e) = execute_file(interpreter, &path) {
                    eprintln!("Worker {}: Error loading {}: {}", worker_id, path.display(), e);
                }

                // Also register controller actions in this worker
                if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                    if name.ends_with("_controller") {
                        let source = std::fs::read_to_string(&path).unwrap_or_default();
                        let routes = derive_routes_from_controller(name, &source).unwrap_or_default();
                        let controller_key = name.trim_end_matches("_controller");
                        for route in routes {
                            if let Some(func_value) = interpreter.environment.borrow().get(&route.function_name) {
                                crate::interpreter::builtins::router::register_controller_action(
                                    controller_key,
                                    &route.function_name,
                                    func_value.clone(),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
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
#[allow(dead_code)]
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
    request_tx: WorkerSender,
    reload_tx: Option<broadcast::Sender<()>>,
    public_dir: Arc<PathBuf>,
    ws_event_tx: channel::Sender<WebSocketEventData>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().to_string().to_uppercase();
    let uri = req.uri();
    let path = uri.path().to_string();

    // Check for WebSocket upgrade request
    if is_websocket_upgrade(&req) {
        // Check if there's a WebSocket route for this path
        let routes = crate::serve::websocket::get_websocket_routes();
        let has_ws_route = routes.iter().any(|r| r.path_pattern == path);

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

    // Read body - skip for GET/HEAD requests (usually empty)
    let body = if method == "GET" || method == "HEAD" {
        String::new()
    } else {
        let body_bytes = http_body_util::BodyExt::collect(req.into_body())
            .await
            .map(|b| b.to_bytes())
            .unwrap_or_default();
        String::from_utf8_lossy(&body_bytes).to_string()
    };

    // Create oneshot channel for response
    let (response_tx, response_rx) = oneshot::channel();

    // Send to interpreter thread
    let request_data = RequestData {
        method: method.clone(),
        path: path.clone(),
        query,
        headers,
        body,
        response_tx,
    };

    let send_result = request_tx.send(request_data);

    if send_result.is_err() {
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
        return upgrade_header == "websocket";
    }

    false
}

/// Handle WebSocket upgrade request.
async fn handle_websocket_upgrade(
    req: Request<Incoming>,
    ws_registry: Arc<WebSocketRegistry>,
    path: String,
    ws_event_tx: channel::Sender<WebSocketEventData>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Create WebSocket config
    let config = WebSocketConfig::default();

    // Check if there's an upgrade header
    if !is_websocket_upgrade(&req) {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Full::new(Bytes::from("Not a WebSocket upgrade request")))
            .unwrap());
    }

    // For WebSocket, we need to handle the upgrade differently
    // Spawn a task to handle the WebSocket connection
    let ws_registry = ws_registry.clone();
    let ws_event_tx = ws_event_tx.clone();
    let path = path.clone();
    let config = config.clone();

    tokio::spawn(async move {
        // Use hyper's upgrade
        match hyper::upgrade::on(req).await {
            Ok(upgraded) => {
                // Wrap with TokioIo
                let mut io = TokioIo::new(upgraded);

                // Complete the WebSocket handshake using tungstenite
                // Read the HTTP request to get the Sec-WebSocket-Key
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut http_request = String::new();
                if let Err(_) = io.read_to_string(&mut http_request).await {
                    return;
                }

                // Parse the Sec-WebSocket-Key from the request
                let sec_websocket_key = http_request
                    .lines()
                    .find(|line| line.to_lowercase().starts_with("sec-websocket-key:"))
                    .and_then(|line| line.split(":").nth(1))
                    .map(|s| s.trim().as_bytes());

                if sec_websocket_key.is_none() {
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
                if let Err(_) = io.write_all(response.as_bytes()).await {
                    return;
                }

                // Now create the WebSocket stream
                let mut stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
                    io,
                    tungstenite::protocol::Role::Server,
                    Some(config),
                )
                .await;

                // Create connection in registry
                let (ws_tx, _ws_rx) = tokio::sync::mpsc::channel::<
                    Result<tungstenite::Message, tungstenite::Error>,
                >(32);
                let ws_tx_arc = Arc::new(ws_tx);
                let connection = WebSocketConnection::new(ws_tx_arc.clone());
                let connection_id = connection.id;

                ws_registry.register(connection).await;

                // Send connect event (non-blocking channel send)
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
                        Err(_) => {
                            break;
                        }
                    }
                }

                // Send disconnect event (non-blocking channel send)
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
            }
            Err(_) => {
                // Upgrade error, silently ignore
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

    return Ok(response);
}

/// Handle WebSocket stream for a single connection.
#[allow(dead_code)]
async fn handle_websocket_stream<S>(
    mut stream: WebSocketStream<S>,
    ws_rx: &mut tokio::sync::mpsc::Receiver<Result<tungstenite::Message, tungstenite::Error>>,
    connection_id: Uuid,
    ws_registry: Arc<WebSocketRegistry>,
    path: String,
    ws_event_tx: channel::Sender<WebSocketEventData>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
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

    if let Err(_) = ws_event_tx.send(connect_event) {
        // Silently ignore send errors
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

/// Call the route handler with the request hash.
fn call_handler(interpreter: &mut Interpreter, handler_name: &str, request_hash: Value) -> ResponseData {
    // Check if this is an OOP controller action (contains #)
    if handler_name.contains('#') {
        if let Some(response) = call_oop_controller_action(interpreter, handler_name, &request_hash) {
            return response;
        }
        // If not an OOP controller or error, fall through to function-based handling
    }

    // Use CONTROLLERS registry to look up handler by full name (controller#action)
    let handler_result = crate::interpreter::builtins::router::resolve_handler(handler_name, None);

    match handler_result {
        Ok(handler_value) => {
            match interpreter.call_value(handler_value, vec![request_hash], Span::default()) {
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
        Err(e) => ResponseData {
            status: 500,
            headers: vec![],
            body: format!("Handler not found: {}", e),
        },
    }
}

/// Call an OOP controller action (controller#action).
/// Returns Some(ResponseData) if handled, None if not an OOP controller.
fn call_oop_controller_action(interpreter: &mut Interpreter, handler_name: &str, request_hash: &Value) -> Option<ResponseData> {
    let (controller_key, action_name) = handler_name.split_once('#')?;

    // Check if this is an OOP controller (has a class definition)
    // Convert controller_key (e.g., "posts") to PascalCase class name (e.g., "PostsController")
    let class_name = to_pascal_case_controller(controller_key);

    // Look up the class in the environment
    let class_value = interpreter.environment.borrow().get(&class_name)?;

    // Check if it's actually a class
    let _class = match class_value {
        Value::Class(class_rc) => class_rc,
        _ => return None,
    };

    // Get controller info from registry (read lock for concurrent access)
    let controller_info = {
        let registry = CONTROLLER_REGISTRY.read().unwrap();
        match registry.get(controller_key).cloned() {
            Some(info) => info,
            None => return None,
        }
    };

    // Extract request components
    let req = request_hash.clone();
    let params = get_hash_field(request_hash, "params").unwrap_or(Value::Null);
    let session = get_hash_field(request_hash, "session").unwrap_or(Value::Null);
    let headers = get_hash_field(request_hash, "headers").unwrap_or(Value::Null);

    // Execute before_action hooks
    if let Some(before_response) = execute_before_actions(interpreter, &controller_info, &action_name, req.clone(), &params, &session, &headers) {
        return Some(before_response);
    }

    // Instantiate the controller
    let controller_instance = match create_controller_instance(&class_name, interpreter) {
        Ok(inst) => inst,
        Err(e) => {
            return Some(ResponseData {
                status: 500,
                headers: vec![],
                body: format!("Controller instantiation error: {}", e),
            });
        }
    };

    // Set up controller context (req, params, session, headers)
    setup_controller_context(&controller_instance, &req, &params, &session, &headers);

    // Call the action method - pass the request hash
    // Action functions are registered with their bare names (e.g., "index", not "posts_index")
    let action_result = call_controller_method(&req, action_name, interpreter);

    let response = match action_result {
        Ok(result) => {
            let (status, resp_headers, body) = extract_response(&result);
            let resp_headers: Vec<_> = resp_headers.into_iter().collect();
            ResponseData {
                status,
                headers: resp_headers,
                body,
            }
        }
        Err(e) => ResponseData {
            status: 500,
            headers: vec![],
            body: format!("Action error: {}", e),
        },
    };

    // Execute after_action hooks
    let final_response = execute_after_actions(interpreter, &controller_info, &action_name, req, &response);

    Some(final_response)
}

/// Get a field from a hash value.
fn get_hash_field(hash: &Value, field: &str) -> Option<Value> {
    match hash {
        Value::Hash(fields) => {
            let key = Value::String(field.to_string());
            fields.borrow().iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
        }
        _ => None,
    }
}

/// Execute before_action hooks for a controller action.
fn execute_before_actions(
    interpreter: &mut Interpreter,
    controller_info: &ControllerInfo,
    action_name: &str,
    req: Value,
    _params: &Value,
    _session: &Value,
    _headers: &Value,
) -> Option<ResponseData> {
    for before_action in &controller_info.before_actions {
        // Check if this before_action applies to this action
        if !before_action.actions.is_empty() && before_action.actions.iter().all(|a| a != action_name) {
            continue;
        }

        // Execute the before_action handler
        match crate::interpreter::builtins::controller::registry::execute_handler_source(
            &before_action.handler_source,
            interpreter,
            req.clone(),
        ) {
            Ok(result) => {
                // Check if the handler returned a response (short-circuit)
                if let Some(response) = check_for_response(&result) {
                    return Some(response);
                }
            }
            Err(e) => {
                return Some(ResponseData {
                    status: 500,
                    headers: vec![],
                    body: format!("Before action error: {}", e),
                });
            }
        }
    }
    None
}

/// Execute after_action hooks for a controller action.
fn execute_after_actions(
    interpreter: &mut Interpreter,
    controller_info: &ControllerInfo,
    action_name: &str,
    req: Value,
    response: &ResponseData,
) -> ResponseData {
    let response_value = Value::Hash(Rc::new(RefCell::new(vec![
        (Value::String("status".to_string()), Value::Int(response.status as i64)),
        (Value::String("headers".to_string()), Value::Hash(Rc::new(RefCell::new(
            response.headers.iter().map(|(k, v)| (Value::String(k.clone()), Value::String(v.clone()))).collect()
        )))),
        (Value::String("body".to_string()), Value::String(response.body.clone())),
    ])));

    for after_action in &controller_info.after_actions {
        // Check if this after_action applies to this action
        if !after_action.actions.is_empty() && after_action.actions.iter().all(|a| a != action_name) {
            continue;
        }

        // Execute the after_action handler
        match crate::interpreter::builtins::controller::registry::execute_after_handler_source(
            &after_action.handler_source,
            interpreter,
            req.clone(),
            response_value.clone(),
        ) {
            Ok(result) => {
                // Update response if handler returned a modified response
                if let Some(updated) = extract_response_from_value(&result) {
                    return updated;
                }
            }
            Err(e) => {
                eprintln!("After action error: {}", e);
            }
        }
    }
    response.clone()
}

/// Check if a before_action result is a response (short-circuit).
/// Returns Some(ResponseData) only if the value is a response hash (has "status" field).
/// Returns None if it's a modified request hash (should continue processing).
fn check_for_response(value: &Value) -> Option<ResponseData> {
    // A response is a Hash with a "status" field (and optionally headers, body)
    // A modified request hash has "method", "path", etc. but no "status"
    if let Value::Hash(hash) = value {
        let fields = hash.borrow();

        // Check if this is a response hash by looking for "status" field
        let has_status = fields.iter().any(|(k, _)| {
            matches!(k, Value::String(s) if s == "status")
        });

        // If no status field, this is a modified request, not a response
        if !has_status {
            return None;
        }

        let mut status = 200i64;
        let mut body = String::new();
        let mut headers = Vec::new();

        for (key, val) in fields.iter() {
            if let Value::String(k) = key {
                match k.as_str() {
                    "status" => if let Value::Int(s) = val { status = *s; },
                    "body" => if let Value::String(b) = val { body = b.clone(); },
                    "headers" => if let Value::Hash(h) = val {
                        for (hk, hv) in h.borrow().iter() {
                            if let (Value::String(key_str), Value::String(val_str)) = (hk, hv) {
                                headers.push((key_str.clone(), val_str.clone()));
                            }
                        }
                    },
                    _ => {}
                }
            }
        }

        return Some(ResponseData {
            status: status as u16,
            headers,
            body,
        });
    }
    None
}

/// Extract response from a value returned by after action.
fn extract_response_from_value(value: &Value) -> Option<ResponseData> {
    check_for_response(value)
}

/// Create a new controller instance.
fn create_controller_instance(class_name: &str, interpreter: &mut Interpreter) -> Result<Value, String> {
    crate::interpreter::builtins::controller::registry::create_controller_instance(class_name, interpreter)
}

/// Set up the controller context (inject req, params, session, headers).
fn setup_controller_context(controller: &Value, req: &Value, params: &Value, session: &Value, headers: &Value) {
    crate::interpreter::builtins::controller::registry::setup_controller_context(controller, req, params, session, headers);
}

/// Call a controller method with the request hash.
fn call_controller_method(request_hash: &Value, method_name: &str, interpreter: &mut Interpreter) -> Result<Value, String> {
    // Look up the function in the environment and call it with the request hash
    let method_value = match interpreter.environment.borrow().get(method_name) {
        Some(v) => v.clone(),
        None => return Err(format!("Method '{}' not found", method_name)),
    };

    interpreter.call_value(method_value, vec![request_hash.clone()], Span::default())
        .map_err(|e| format!("Error calling method: {}", e))
}

/// Convert a controller key (e.g., "posts", "user_profiles") to PascalCase class name (e.g., "PostsController", "UserProfilesController").
fn to_pascal_case_controller(controller_key: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in controller_key.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result.push_str("Controller");
    result
}

/// Handle a single request (called on interpreter thread)
fn handle_request(interpreter: &mut Interpreter, data: &RequestData) -> ResponseData {
    let method = &data.method;
    let path = &data.path;

    // Find matching route using indexed lookup (O(1) for exact matches, O(m) for patterns)
    let (route, matched_params) = match find_route(method, path) {
        Some((r, params)) => (r, params),
        None => {
            return ResponseData {
                status: 404,
                headers: vec![],
                body: "Not Found".to_string(),
            };
        }
    };

    let handler_name = route.handler_name.clone();
    let scoped_middleware = route.middleware.clone();

    // Build request hash - optimize for empty query/headers/body
    let mut request_hash = if data.query.is_empty() && data.headers.is_empty() && data.body.is_empty() {
        build_request_hash(
            &data.method,
            &data.path,
            matched_params,
            HashMap::new(),
            HashMap::new(),
            String::new(),
        )
    } else {
        build_request_hash(
            &data.method,
            &data.path,
            matched_params,
            data.query.clone(),
            data.headers.clone(),
            data.body.clone(),
        )
    };

    // Fast path: no middleware at all (avoid cloning middleware list if empty)
    if scoped_middleware.is_empty() && !has_middleware() {
        return call_handler(interpreter, &handler_name, request_hash);
    }

    // Only clone middleware list if we need it
    let global_middleware = get_middleware();

    // Execute scoped (route-specific) middleware
    for mw in &scoped_middleware {
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
    let has_scoped_middleware = !scoped_middleware.is_empty();
    for mw in &global_middleware {
        if has_scoped_middleware && mw.global_only {
            continue;
        }
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
    call_handler(interpreter, &handler_name, request_hash)
}

