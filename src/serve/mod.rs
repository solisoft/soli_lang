//! MVC framework with convention-based routing and hot reload.
//!
//! This module implements a Rails-like MVC framework for Soli applications:
//! - Convention-based routing from controller filenames and function names
//! - Hot reload of changed files without server restart
//! - Automatic route derivation
//! - Middleware support for request interception

mod hot_reload;
pub mod live_reload;
mod live_reload_ws;  // WebSocket-based live reload
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

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use crossbeam::channel;
use futures_util::SinkExt;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{header, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};
use tokio_tungstenite::WebSocketStream;
use uuid::Uuid;

use crate::error::RuntimeError;
use crate::interpreter::builtins::server::{
    build_request_hash_with_parsed, extract_response, find_route, get_routes,
    parse_form_urlencoded_body, parse_json_body, parse_query_string,
    register_route_with_handler, routes_to_worker_routes, set_worker_routes, ParsedBody, WorkerRoute,
};

// Thread-local storage for tokio runtime handle (used by HTTP builtins for async operations)
thread_local! {
    /// Tokio runtime handle for the current worker thread.
    /// Set during worker initialization, used by HTTP builtins to execute async requests.
    pub static TOKIO_HANDLE: RefCell<Option<tokio::runtime::Handle>> = const { RefCell::new(None) };
}

/// Get the tokio runtime handle for the current thread.
/// Returns None if called outside of a server worker context.
pub fn get_tokio_handle() -> Option<tokio::runtime::Handle> {
    TOKIO_HANDLE.with(|h| h.borrow().clone())
}

/// Set the tokio runtime handle for the current worker thread.
fn set_tokio_handle(handle: tokio::runtime::Handle) {
    TOKIO_HANDLE.with(|h| *h.borrow_mut() = Some(handle));
}
use crate::interpreter::builtins::session::{
    create_session_cookie, ensure_session, extract_session_id_from_cookie, set_current_session_id,
};
use crate::live::socket::{extract_session_id as extract_live_session_id, handle_live_connection};
use crate::interpreter::builtins::template::{clear_template_cache, init_templates};
use crate::interpreter::builtins::controller::controller::ControllerInfo;
use crate::interpreter::builtins::controller::CONTROLLER_REGISTRY;
use crate::interpreter::{Interpreter, Value};
use crate::span::Span;
use crate::ExecutionMode;

/// Uploaded file information
#[derive(Clone)]
pub struct UploadedFile {
    pub name: String,
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

/// Request data sent to interpreter thread
struct RequestData {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: String,
    /// Raw body bytes (for multipart parsing)
    #[allow(dead_code)]
    body_bytes: Option<Vec<u8>>,
    /// Pre-parsed form fields from multipart
    multipart_form: Option<HashMap<String, String>>,
    /// Pre-parsed files from multipart
    multipart_files: Option<Vec<UploadedFile>>,
    response_tx: oneshot::Sender<ResponseData>,
}

/// Response data from interpreter thread
#[derive(Clone)]
struct ResponseData {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

/// Serve an MVC application from a folder in production mode by default.
pub fn serve_folder(folder: &Path, port: u16) -> Result<(), RuntimeError> {
    serve_folder_with_options(folder, port, false)
}

/// Serve an MVC application from a folder with configurable options.
pub fn serve_folder_with_options(
    folder: &Path,
    port: u16,
    dev_mode: bool,
) -> Result<(), RuntimeError> {
    // Default to number of CPU cores for optimal parallelism
    let num_workers = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4); // Fallback to 4 if unable to detect
    serve_folder_with_options_and_mode(folder, port, dev_mode, ExecutionMode::Bytecode, num_workers)
}

/// Serve an MVC application from a folder with configurable options and execution mode.
pub fn serve_folder_with_options_and_mode(
    folder: &Path,
    port: u16,
    dev_mode: bool,
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

    // Set the app root for LiveView template resolution
    crate::live::component::set_app_root(folder.to_path_buf());

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

    // Load view helpers from app/helpers directory (only accessible in templates)
    let helpers_dir = app_dir.join("helpers");
    if helpers_dir.exists() {
        match crate::interpreter::builtins::template::load_view_helpers(&helpers_dir) {
            Ok(count) => {
                if count > 0 {
                    println!("Loaded {} view helper(s) from {}", count, helpers_dir.display());
                }
            }
            Err(e) => {
                eprintln!("Error loading view helpers: {}", e);
            }
        }
        // Track helper files for hot reload
        for entry in std::fs::read_dir(&helpers_dir).unwrap() {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "sl") {
                    file_tracker.track(&path);
                }
            }
        }
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
                if path.extension().map_or(false, |ext| ext == "sl") {
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

    // Set live reload flag for template injection (only in dev mode)
    live_reload::set_live_reload_enabled(dev_mode);

    // Load routes from config/routes.sl if it exists
    let routes_file = folder.join("config").join("routes.sl");
    if routes_file.exists() {

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

    // Start Tailwind CSS watch process (dev mode only)
    let tailwind_process = if dev_mode {
        spawn_tailwind_watch(folder)
    } else {
        None
    };

    // Always use hyper-based MVC server
    run_hyper_server_worker_pool(
        folder,
        port,
        controllers_dir,
        models_dir,
        middleware_dir,
        helpers_dir,
        public_dir,
        file_tracker,
        dev_mode,
        mode,
        workers,
        views_dir,
        routes_file,
        tailwind_process,
    )
}

/// Spawn Tailwind in watch mode as a background process.
/// Returns the child process handle if successful.
fn spawn_tailwind_watch(folder: &Path) -> Option<std::process::Child> {
    let tailwind_config = folder.join("tailwind.config.js");
    if !tailwind_config.exists() {
        return None;
    }

    let package_json = folder.join("package.json");
    if !package_json.exists() {
        return None;
    }

    println!("Starting Tailwind CSS in watch mode...");

    // Use npx tailwindcss directly for watch mode
    match std::process::Command::new("npx")
        .args([
            "tailwindcss",
            "-i",
            "./app/assets/css/application.css",
            "-o",
            "./public/css/application.css",
            "--watch",
        ])
        .current_dir(folder)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => {
            println!("   ✓ Tailwind CSS watch process started (PID: {})", child.id());
            Some(child)
        }
        Err(e) => {
            eprintln!("   ✗ Failed to start Tailwind watch: {}", e);
            // Fall back to single compilation
            compile_tailwind_css_once(folder);
            None
        }
    }
}

/// Run a single Tailwind CSS compilation (fallback/initial).
/// Returns true if compilation was successful.
fn compile_tailwind_css_once(folder: &Path) -> bool {
    let tailwind_config = folder.join("tailwind.config.js");
    if !tailwind_config.exists() {
        return false;
    }

    // Check for package.json with build:css script
    let package_json = folder.join("package.json");
    if !package_json.exists() {
        return false;
    }

    println!("Compiling Tailwind CSS...");

    let output = std::process::Command::new("npm")
        .arg("run")
        .arg("build:css")
        .current_dir(folder)
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                println!("   ✓ Tailwind CSS compiled successfully");
                true
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                eprintln!("   ✗ Tailwind CSS compilation failed: {}", stderr);
                false
            }
        }
        Err(e) => {
            eprintln!("   ✗ Failed to run npm: {}", e);
            false
        }
    }
}

/// Touch the input CSS file to trigger Tailwind's watch mode.
fn trigger_tailwind_rebuild(folder: &Path) {
    let input_css = folder.join("app/assets/css/application.css");
    if input_css.exists() {
        // Touch file by reading and rewriting (works cross-platform)
        if let Ok(content) = std::fs::read(&input_css) {
            let _ = std::fs::write(&input_css, content);
        }
    }
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
        if path.extension().map_or(false, |ext| ext == "sl") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with("_controller.sl") {
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
            if path.extension().map_or(false, |ext| ext == "sl") {
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

    // Check if this is an OOP controller (class-based)
    let controller_key = controller_name.trim_end_matches("_controller");
    let class_name = to_pascal_case_controller(controller_key);
    let is_oop_controller = interpreter
        .environment
        .borrow()
        .get(&class_name)
        .map(|v| matches!(v, Value::Class(_)))
        .unwrap_or(false);

    // Register routes using the interpreter's environment
    for route in routes {
        // Create full handler name: controller#action
        let full_handler_name = format!("{}#{}", controller_key, route.function_name);

        if is_oop_controller {
            // OOP controller: methods are inside the class, resolved at runtime
        } else {
            // Function-based controller: look up the function in the environment
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
            crate::interpreter::builtins::router::register_controller_action(
                controller_key,
                &route.function_name,
                func_value.clone(),
            );
        }

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
    interpreter.set_source_path(path.to_path_buf());
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
    /// Incremented when static files (CSS, JS) change
    static_files: AtomicU64,
    /// Incremented when routes.sl changes
    routes: AtomicU64,
    /// Incremented when view helpers change
    helpers: AtomicU64,
}

impl HotReloadVersions {
    fn new() -> Self {
        Self {
            controllers: AtomicU64::new(0),
            middleware: AtomicU64::new(0),
            views: AtomicU64::new(0),
            static_files: AtomicU64::new(0),
            routes: AtomicU64::new(0),
            helpers: AtomicU64::new(0),
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
    folder: &Path,
    port: u16,
    controllers_dir: PathBuf,
    models_dir: PathBuf,
    middleware_dir: PathBuf,
    helpers_dir: PathBuf,
    public_dir: PathBuf,
    _file_tracker: FileTracker,
    dev_mode: bool,
    _mode: ExecutionMode,
    num_workers: usize,
    views_dir: PathBuf,
    routes_file: PathBuf,
    tailwind_process: Option<std::process::Child>,
) -> Result<(), RuntimeError> {
    let reload_tx = if dev_mode {
        let (tx, _) = broadcast::channel::<()>(16);
        Some(tx)
    } else {
        None
    };
    let reload_tx_for_tokio = reload_tx.clone();

    let ws_registry = crate::serve::websocket::get_ws_registry();

    // Bounded channels for backpressure
    let capacity_per_worker = 64;
    let (ws_event_tx, ws_event_rx) = channel::bounded(num_workers * capacity_per_worker);
    // LiveView event channel
    let (lv_event_tx, lv_event_rx): (channel::Sender<LiveViewEventData>, channel::Receiver<LiveViewEventData>) = channel::bounded(num_workers * capacity_per_worker);
    // crossbeam Sender is cheap to clone - no need for Arc<Mutex<Option<>>>
    // Use AtomicBool for shutdown signaling (lock-free check)
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_for_tokio = shutdown_flag.clone();

    // Per-worker queues eliminate receiver contention
    let worker_queues = Arc::new(WorkerQueues::new(num_workers, capacity_per_worker));
    let worker_queues_for_tokio = worker_queues.clone();

    println!("\nServer listening on http://0.0.0.0:{}", port);
    if dev_mode {
        println!("Development mode - hot reload enabled, no caching");
        println!("  Edit controllers/middleware/views to see changes");
        println!("  Browsers will auto-refresh on changes");
    } else {
        println!("Production mode - caching enabled, no hot reload");
    }
    if public_dir.exists() {
        println!("Static files served from {}", public_dir.display());
    }
    println!("Using hyper async HTTP server with {} worker threads\n", num_workers);

    // Wrap public_dir in Arc for cheap cloning across connections
    let public_dir_arc = Arc::new(public_dir.clone());
    let ws_registry_for_tokio = ws_registry.clone();
    let dev_mode_for_tokio = dev_mode;

    // Channel to pass runtime handle from tokio thread to main thread
    let (runtime_handle_tx, runtime_handle_rx) = std::sync::mpsc::channel::<tokio::runtime::Handle>();

    // Spawn tokio runtime for HTTP server
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        runtime.block_on(async move {
            // Send runtime handle to main thread for workers to use
            let _ = runtime_handle_tx.send(tokio::runtime::Handle::current());
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
                let lv_event_tx = lv_event_tx.clone(); // LiveView event sender
                let shutdown_flag = shutdown_flag_for_tokio.clone();
                let dev_mode = dev_mode_for_tokio;

                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let request_tx = request_tx.clone();
                        let reload_tx = reload_tx.clone();
                        let public_dir = public_dir.clone(); // Arc clone is cheap
                        let ws_event_tx = ws_event_tx.clone();
                        let lv_event_tx = lv_event_tx.clone();
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
                                lv_event_tx,
                                dev_mode,
                            )
                            .await
                        }
                    });

                    // Use with_upgrades() to support WebSocket upgrades
                    if let Err(_e) = http1::Builder::new()
                        .serve_connection(io, service)
                        .with_upgrades()
                        .await
                    {
                        // Silently ignore connection errors
                    }
                });
            }
        });
    });

    // Hot reload version counters (shared between file watcher and workers)
    let hot_reload_versions = Arc::new(HotReloadVersions::new());
    let hot_reload_versions_for_watcher = hot_reload_versions.clone();

    // Spawn file watcher thread for hot reload (only in dev mode)
    if dev_mode {
    let watch_folder = folder.to_path_buf();
    let watch_controllers_dir = controllers_dir.clone();
    let watch_views_dir = views_dir.clone();
    let watch_middleware_dir = middleware_dir.clone();
    let watch_helpers_dir = helpers_dir.clone();
    let watch_public_dir = public_dir.clone();
    let watch_routes_file = routes_file.clone();
    let browser_reload_tx = reload_tx.clone();
    let dev_mode_for_watcher = dev_mode;
    thread::spawn(move || {
        // Hold onto the Tailwind process - it will be killed when dropped
        let _tailwind_process = tailwind_process;

        let mut file_tracker = FileTracker::new();

        // Track all controller files
        if let Ok(entries) = std::fs::read_dir(&watch_controllers_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "sl") {
                    file_tracker.track(&path);
                }
            }
        }

        // Track middleware files
        if let Ok(entries) = std::fs::read_dir(&watch_middleware_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "sl") {
                    file_tracker.track(&path);
                }
            }
        }

        // Track routes.sl file
        if watch_routes_file.exists() {
            file_tracker.track(&watch_routes_file);
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

        // Track static files (CSS, JS) in public directory
        fn track_static_recursive(dir: &Path, tracker: &mut FileTracker) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        track_static_recursive(&path, tracker);
                    } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ["css", "js", "svg", "ico", "png", "jpg", "jpeg", "gif", "woff", "woff2", "ttf"].contains(&ext) {
                            tracker.track(&path);
                        }
                    }
                }
            }
        }
        if watch_public_dir.exists() {
            track_static_recursive(&watch_public_dir, &mut file_tracker);
        }

        // Note: We don't track source CSS files (app/assets/css) here
        // because Tailwind's --watch mode already handles CSS changes.
        // Tracking them would cause an infinite loop since trigger_tailwind_rebuild
        // touches the CSS file to force a rebuild.

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
            let mut helpers_changed = false;
            let mut static_files_changed = false;
            let mut routes_changed = false;

            for path in &changed {
                println!("   {}", path.display());

                // Check if it's a static file (CSS, JS, images)
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ["css", "js", "svg", "ico", "png", "jpg", "jpeg", "gif", "woff", "woff2", "ttf"].contains(&ext) {
                        static_files_changed = true;
                    }
                }

                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "routes.sl" {
                        routes_changed = true;
                    } else if name.ends_with("_controller.sl") {
                        controllers_changed = true;
                    } else if name.ends_with(".sl") && path.starts_with(&watch_middleware_dir) {
                        middleware_changed = true;
                    } else if name.ends_with(".sl") && path.starts_with(&watch_helpers_dir) {
                        helpers_changed = true;
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
            if helpers_changed {
                hot_reload_versions_for_watcher.helpers.fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled view helpers reload to all workers");
            }
            if views_changed {
                hot_reload_versions_for_watcher.views.fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled template cache clear to all workers");

                // Touch input CSS to trigger Tailwind watch mode rebuild
                // (new classes may have been added to templates)
                if dev_mode_for_watcher {
                    trigger_tailwind_rebuild(&watch_folder);
                    // Note: No need to set static_files_changed here - the file watcher
                    // will detect when Tailwind updates public/css/application.css
                }
            }
            if static_files_changed {
                hot_reload_versions_for_watcher.static_files.fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled static file reload to all workers");
            }
            if routes_changed {
                hot_reload_versions_for_watcher.routes.fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled routes reload to all workers");
            }

            // Notify browser for live reload
            if let Some(ref tx) = browser_reload_tx {
                let _ = tx.send(());
            }

            println!();
        }
    });
    } // end if dev_mode for hot reload thread

    // Spawn worker threads
    let mut workers = Vec::new();
    // Get routes in main thread and convert to worker-safe formats
    let routes = get_routes();
    let worker_routes = routes_to_worker_routes(&routes);

    // Receive tokio runtime handle from the tokio thread (blocks until available)
    let runtime_handle = runtime_handle_rx
        .recv()
        .expect("Failed to receive runtime handle from tokio thread");

    for i in 0..num_workers {
        // Each worker gets its own dedicated receiver (no contention!)
        let work_rx = worker_queues.get_receiver(i);
        let models_dir = models_dir.clone();
        let middleware_dir = middleware_dir.clone();
        let helpers_dir = helpers_dir.clone();
        let ws_event_rx = ws_event_rx.clone();
        let lv_event_rx = lv_event_rx.clone();
        let ws_registry = ws_registry.clone();
        let reload_tx = reload_tx.clone();
        let worker_routes = worker_routes.clone();
        let controllers_dir = controllers_dir.clone();
        let views_dir = views_dir.clone();
        let hot_reload_versions = hot_reload_versions.clone();
        let runtime_handle = runtime_handle.clone();
        let routes_file = routes_file.clone();

        let builder = thread::Builder::new().name(format!("worker-{}", i));
        let handler = builder.spawn(move || {
            // Set tokio runtime handle for this worker thread (used by HTTP builtins)
            set_tokio_handle(runtime_handle.clone());

            // Panic catch wrapper
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut interpreter = Interpreter::new();

                worker_loop(i, work_rx, models_dir, middleware_dir, helpers_dir, ws_event_rx, lv_event_rx, ws_registry, reload_tx, &mut interpreter, worker_routes, controllers_dir, views_dir, hot_reload_versions, runtime_handle, routes_file, dev_mode);
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
    helpers_dir: PathBuf,
    ws_event_rx: channel::Receiver<WebSocketEventData>,
    lv_event_rx: channel::Receiver<LiveViewEventData>,
    ws_registry: Arc<WebSocketRegistry>,
    _reload_tx: Option<broadcast::Sender<()>>,
    interpreter: &mut Interpreter,
    routes: Vec<WorkerRoute>,
    controllers_dir: PathBuf,
    views_dir: PathBuf,
    hot_reload_versions: Arc<HotReloadVersions>,
    runtime_handle: tokio::runtime::Handle,
    routes_file: PathBuf,
    dev_mode: bool,
) {
    // Initialize routes in this worker thread
    set_worker_routes(routes);

    // Initialize template engine in this worker
    if views_dir.exists() {
        crate::interpreter::builtins::template::init_templates(views_dir.clone());
    }

    // Load view helpers in this worker (thread-local)
    if helpers_dir.exists() {
        if let Err(e) = crate::interpreter::builtins::template::load_view_helpers(&helpers_dir) {
            eprintln!("Worker {}: Error loading view helpers: {}", worker_id, e);
        }
    }

    // Set dev mode for file hash caching (production = permanent cache, dev = check mtime)
    crate::interpreter::builtins::template::set_dev_mode(dev_mode);

    // Load controllers in this worker so functions are defined in environment
    load_controllers_in_worker(worker_id, interpreter, &controllers_dir);

    // Define DSL helpers for routes (needed for hot reload)
    if let Err(e) = define_routes_dsl(interpreter) {
        eprintln!("Worker {}: Error defining routes DSL: {}", worker_id, e);
    }

    let _worker_routes = get_routes();

    let check_interval = Duration::from_millis(100);
    let mut ws_event_rx_inner = Some(ws_event_rx);
    let ws_registry_inner = Some(ws_registry);
    let mut lv_event_rx_inner = Some(lv_event_rx);

    // Track last seen hot reload versions
    let mut last_controllers_version = hot_reload_versions.controllers.load(Ordering::Acquire);
    let mut last_middleware_version = hot_reload_versions.middleware.load(Ordering::Acquire);
    let mut last_helpers_version = hot_reload_versions.helpers.load(Ordering::Acquire);
    let mut last_views_version = hot_reload_versions.views.load(Ordering::Acquire);
    let mut last_static_files_version = hot_reload_versions.static_files.load(Ordering::Acquire);
    let mut last_routes_version = hot_reload_versions.routes.load(Ordering::Acquire);

    const BATCH_SIZE: usize = 64;

    loop {
        // Check for hot reload (lock-free version check)
        let current_controllers = hot_reload_versions.controllers.load(Ordering::Acquire);
        let current_middleware = hot_reload_versions.middleware.load(Ordering::Acquire);
        let current_helpers = hot_reload_versions.helpers.load(Ordering::Acquire);
        let current_views = hot_reload_versions.views.load(Ordering::Acquire);
        let current_static_files = hot_reload_versions.static_files.load(Ordering::Acquire);
        let current_routes = hot_reload_versions.routes.load(Ordering::Acquire);

        if current_controllers != last_controllers_version {
            last_controllers_version = current_controllers;
            // Re-load all controllers
            load_controllers_in_worker(worker_id, interpreter, &controllers_dir);
        }

        if current_middleware != last_middleware_version {
            last_middleware_version = current_middleware;
            // Clear and reload middleware
            let mut file_tracker = FileTracker::new();
            if let Err(e) = load_middleware(interpreter, &middleware_dir, &mut file_tracker) {
                eprintln!("Worker {}: Error reloading middleware: {}", worker_id, e);
            }
        }

        if current_helpers != last_helpers_version {
            last_helpers_version = current_helpers;
            // Clear and reload view helpers
            crate::interpreter::builtins::template::clear_view_helpers();
            if let Err(e) = crate::interpreter::builtins::template::load_view_helpers(&helpers_dir) {
                eprintln!("Worker {}: Error reloading view helpers: {}", worker_id, e);
            }
        }

        if current_views != last_views_version {
            last_views_version = current_views;
            clear_template_cache();
        }

        // Static files changed - trigger browser refresh via SSE
        if current_static_files != last_static_files_version {
            last_static_files_version = current_static_files;
            // Clear file mtime cache so public_path() refreshes versions
            crate::interpreter::builtins::template::clear_file_mtime_cache();
            // Notify browser for live reload (browsers will re-fetch CSS/JS)
            if let Some(ref tx) = _reload_tx {
                let _ = tx.send(());
            }
        }

        // Routes changed - reload routes.sl
        if current_routes != last_routes_version {
            last_routes_version = current_routes;
            reload_routes_in_worker(worker_id, interpreter, &routes_file);
        }

        // Process WebSocket events first (quick non-blocking check)
        if let (Some(ref mut rx), Some(ref _registry)) =
            (ws_event_rx_inner.as_mut(), ws_registry_inner.as_ref())
        {
            // Use try_recv for non-blocking check instead of recv_timeout(ZERO)
            match rx.try_recv() {
                Ok(data) => {
                    handle_websocket_event(interpreter, &data, &runtime_handle);
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

        // Process LiveView events (quick non-blocking check)
        if let Some(ref mut rx) = lv_event_rx_inner {
            match rx.try_recv() {
                Ok(data) => {
                    let result = handle_liveview_event(interpreter, &data);
                    let _ = data.response_tx.send(result);
                }
                Err(channel::TryRecvError::Empty) => {}
                Err(channel::TryRecvError::Disconnected) => {
                    lv_event_rx_inner = None;
                }
            }
        }

        // Batch process HTTP requests using try_recv for non-blocking drain
        for _ in 0..BATCH_SIZE {
            match work_rx.try_recv() {
                Ok(data) => {
                    let resp_data = handle_request(interpreter, &data, dev_mode);
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
            // Check hot reload again before handling this request
            // (version may have changed while we were blocked)
            let current_views = hot_reload_versions.views.load(Ordering::Acquire);
            if current_views != last_views_version {
                last_views_version = current_views;
                clear_template_cache();
            }

            let resp_data = handle_request(interpreter, &data, dev_mode);
            let _ = data.response_tx.send(resp_data);
        }
    }
}

/// Load all controllers in a worker thread
fn load_controllers_in_worker(worker_id: usize, interpreter: &mut Interpreter, controllers_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(controllers_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "sl") {
                if let Err(e) = execute_file(interpreter, &path) {
                    eprintln!("Worker {}: Error loading {}: {}", worker_id, path.display(), e);
                }

                // Also register controller actions in this worker (only for function-based controllers)
                if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                    if name.ends_with("_controller") {
                        let controller_key = name.trim_end_matches("_controller");
                        let class_name = to_pascal_case_controller(controller_key);

                        // Check if this is an OOP controller (class-based)
                        let is_oop_controller = interpreter
                            .environment
                            .borrow()
                            .get(&class_name)
                            .map(|v| matches!(v, Value::Class(_)))
                            .unwrap_or(false);

                        // Only register actions for function-based controllers
                        // OOP controllers have their methods resolved at runtime
                        if !is_oop_controller {
                            let source = std::fs::read_to_string(&path).unwrap_or_default();
                            let routes = derive_routes_from_controller(name, &source).unwrap_or_default();
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
}

/// Define DSL helpers for routes in the interpreter.
/// This must be called before routes.sl can be executed.
fn define_routes_dsl(interpreter: &mut Interpreter) -> Result<(), RuntimeError> {
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

        fn websocket(path: Any, action: Any) { router_websocket(path, action); }
    "#;

    let tokens = crate::lexer::Scanner::new(dsl_source)
        .scan_tokens()
        .map_err(|e| RuntimeError::General {
            message: format!("DSL Lexer error: {}", e),
            span: Span::default(),
        })?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| RuntimeError::General {
            message: format!("DSL Parser error: {}", e),
            span: Span::default(),
        })?;
    interpreter.interpret(&program)
}

/// Reload routes in a worker thread.
/// Clears existing routes, resets router context, and re-executes routes.sl.
fn reload_routes_in_worker(
    worker_id: usize,
    interpreter: &mut Interpreter,
    routes_file: &Path,
) {
    // 1. Clear existing routes
    crate::interpreter::builtins::server::clear_routes();

    // 2. Clear WebSocket routes
    crate::serve::websocket::clear_websocket_routes();

    // 3. Reset router context
    crate::interpreter::builtins::router::reset_router_context();

    // 4. Re-execute routes.sl (DSL helpers are already defined in interpreter)
    if routes_file.exists() {
        if let Err(e) = execute_file(interpreter, routes_file) {
            eprintln!("Worker {}: Error reloading routes: {}", worker_id, e);
            return;
        }
    }

    // 5. Rebuild route index
    crate::interpreter::builtins::server::rebuild_route_index();

    println!("Worker {}: Routes reloaded successfully", worker_id);
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

/// Data for LiveView events sent to the interpreter thread.
pub struct LiveViewEventData {
    /// LiveView instance ID (session_id:component)
    pub liveview_id: String,
    /// Component name (e.g., "counter")
    pub component: String,
    /// Event name (e.g., "increment", "decrement")
    pub event: String,
    /// Event parameters
    pub params: serde_json::Value,
    /// Response channel - sends back result
    pub response_tx: oneshot::Sender<Result<(), String>>,
}

/// Parse multipart form data into form fields and files.
async fn parse_multipart_body(
    body_bytes: &[u8],
    content_type: &str,
) -> (HashMap<String, String>, Vec<UploadedFile>) {
    let mut form_fields = HashMap::new();
    let mut files = Vec::new();

    // Extract boundary from content-type header
    let boundary = content_type
        .split(';')
        .find_map(|part| {
            let part = part.trim();
            if part.starts_with("boundary=") {
                Some(part.trim_start_matches("boundary=").trim_matches('"').to_string())
            } else {
                None
            }
        });

    let boundary = match boundary {
        Some(b) => b,
        None => return (form_fields, files),
    };

    // Use multer to parse the multipart data
    let stream = futures_util::stream::once(async move {
        Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(body_bytes))
    });

    let mut multipart = multer::Multipart::new(stream, boundary);

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        let filename = field.file_name().map(|s| s.to_string());
        let content_type = field.content_type().map(|m| m.to_string()).unwrap_or_default();

        if let Ok(data) = field.bytes().await {
            if let Some(fname) = filename {
                // This is a file upload
                files.push(UploadedFile {
                    name: name.clone(),
                    filename: fname,
                    content_type,
                    data: data.to_vec(),
                });
            } else {
                // This is a regular form field
                let value = String::from_utf8_lossy(&data).to_string();
                form_fields.insert(name, value);
            }
        }
    }

    (form_fields, files)
}

/// Handle a hyper request
async fn handle_hyper_request(
    mut req: Request<Incoming>,
    request_tx: WorkerSender,
    reload_tx: Option<broadcast::Sender<()>>,
    public_dir: Arc<PathBuf>,
    ws_event_tx: channel::Sender<WebSocketEventData>,
    lv_event_tx: channel::Sender<LiveViewEventData>,
    dev_mode: bool,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().to_string().to_uppercase();
    let uri = req.uri();
    let path = uri.path().to_string();

    // Check for WebSocket upgrade request
    if hyper_tungstenite::is_upgrade_request(&req) {
        // Handle live reload WebSocket endpoint
        if path == "/__livereload_ws" {
            if let Some(ref tx) = reload_tx {
                return live_reload_ws::handle_live_reload_websocket(req, tx.subscribe()).await;
            } else {
                // Live reload disabled
                return Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Full::new(Bytes::from("Live reload is disabled")))
                    .unwrap());
            }
        }

        // Handle LiveView WebSocket endpoint
        if path == "/live/socket" || path.starts_with("/live/socket/") {
            // Extract component name from path
            let component = if path == "/live/socket" {
                "counter".to_string()
            } else {
                path.trim_start_matches("/live/socket/")
                    .trim_end_matches("/socket")
                    .to_string()
            };

            // Extract session ID from cookies
            let cookies = req.headers().get("cookie").map(|v| v.to_str().unwrap_or(""));
            let session_id = extract_live_session_id(cookies);

            // Perform the WebSocket upgrade
            let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, None) {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("[LiveView] Upgrade error: {}", e);
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Full::new(Bytes::from(format!("WebSocket upgrade error: {}", e))))
                        .unwrap());
                }
            };

            // Spawn a task to handle the LiveView WebSocket connection
            let component = component.clone();
            let session_id = session_id.clone();
            let lv_event_tx = lv_event_tx.clone();

            tokio::spawn(async move {
                let stream = match websocket.await {
                    Ok(ws) => ws,
                    Err(e) => {
                        eprintln!("[LiveView] WebSocket handshake error: {}", e);
                        return;
                    }
                };

                // Create async channel for LiveView messages
                let (tx, rx) = async_channel::bounded::<Result<tungstenite::Message, tungstenite::Error>>(32);
                let tx_arc = Arc::new(tx);

                // Initialize the LiveView connection
                handle_live_connection(component.clone(), session_id, tx_arc.clone());

                // Split the WebSocket stream
                let (mut ws_write, mut ws_read) = stream.split();

                // Spawn task to forward messages from channel to WebSocket
                let write_task = tokio::spawn(async move {
                    while let Ok(msg_result) = rx.recv().await {
                        match msg_result {
                            Ok(msg) => {
                                if ws_write.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });

                // Handle incoming messages (events from client)
                while let Some(msg_result) = ws_read.next().await {
                    match msg_result {
                        Ok(msg) => {
                            if msg.is_close() {
                                break;
                            }
                            if msg.is_text() {
                                if let Ok(text) = msg.to_text() {
                                    // Parse the event message
                                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
                                        let event_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                        let liveview_id = parsed.get("liveview_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                                        let event_name = parsed.get("event").and_then(|v| v.as_str()).map(|s| s.to_string());
                                        let params = parsed.get("params").cloned().unwrap_or(serde_json::json!({}));

                                        if event_type == "event" {
                                            if let Some(id) = liveview_id {
                                                if let Some(event) = event_name {
                                                    // Send event to worker thread for controller dispatch
                                                    let (response_tx, response_rx) = oneshot::channel();
                                                    let event_data = LiveViewEventData {
                                                        liveview_id: id.clone(),
                                                        component: component.clone(),
                                                        event: event.clone(),
                                                        params,
                                                        response_tx,
                                                    };

                                                    if lv_event_tx.send(event_data).is_ok() {
                                                        // Wait for response (with timeout)
                                                        match tokio::time::timeout(
                                                            std::time::Duration::from_secs(5),
                                                            response_rx
                                                        ).await {
                                                            Ok(Ok(Ok(()))) => {
                                                                // Event handled successfully
                                                            }
                                                            Ok(Ok(Err(e))) => {
                                                                eprintln!("[LiveView] Event error: {}", e);
                                                            }
                                                            Ok(Err(_)) => {
                                                                eprintln!("[LiveView] Response channel closed");
                                                            }
                                                            Err(_) => {
                                                                eprintln!("[LiveView] Event handling timed out");
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        } else if event_type == "heartbeat" {
                                            // Send heartbeat acknowledgment
                                            let ack = serde_json::json!({
                                                "type": "heartbeat_ack"
                                            });
                                            let _ = tx_arc.send(Ok(tungstenite::Message::Text(ack.to_string())));
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }

                write_task.abort();
            });

            return Ok(response);
        }

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
                let mime_type = match file_path.extension().and_then(|e| e.to_str()) {
                    Some("css") => "text/css",
                    Some("js") => "application/javascript",
                    Some("png") => "image/png",
                    Some("jpg") | Some("jpeg") => "image/jpeg",
                    Some("ico") => "image/x-icon",
                    Some("svg") => "image/svg+xml",
                    Some("html") => "text/html",
                    Some("json") => "application/json",
                    Some("woff") => "font/woff",
                    Some("woff2") => "font/woff2",
                    Some("ttf") => "font/ttf",
                    Some("gif") => "image/gif",
                    _ => "application/octet-stream",
                };

                // In production mode, check for conditional request (If-None-Match)
                if !dev_mode {
                    if let Ok(metadata) = std::fs::metadata(&file_path) {
                        if let Ok(modified) = metadata.modified() {
                            let etag = format!("\"{:x}\"", modified
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs());

                            // Check If-None-Match header
                            if let Some(if_none_match) = req.headers().get("if-none-match") {
                                if let Ok(client_etag) = if_none_match.to_str() {
                                    // ETags match - return 304 Not Modified (skip file read!)
                                    if client_etag == etag || client_etag == format!("W/{}", etag) {
                                        return Ok(Response::builder()
                                            .status(StatusCode::NOT_MODIFIED)
                                            .header("ETag", &etag)
                                            .header("Cache-Control", "public, max-age=31536000, immutable")
                                            .body(Full::new(Bytes::new()))
                                            .unwrap());
                                    }
                                }
                            }

                            // ETags don't match or no If-None-Match - read and serve file
                            let content = match std::fs::read(&file_path) {
                                Ok(c) => c,
                                Err(_) => {
                                    return Ok(Response::builder()
                                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                                        .body(Full::new(Bytes::from("Error reading file")))
                                        .unwrap())
                                }
                            };

                            return Ok(Response::builder()
                                .status(StatusCode::OK)
                                .header("Content-Type", mime_type)
                                .header("ETag", etag)
                                .header("Cache-Control", "public, max-age=31536000, immutable")
                                .body(Full::new(Bytes::from(content)))
                                .unwrap());
                        }
                    }
                }

                // Dev mode or metadata unavailable - serve without caching
                let content = match std::fs::read(&file_path) {
                    Ok(c) => c,
                    Err(_) => {
                        return Ok(Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Full::new(Bytes::from("Error reading file")))
                            .unwrap())
                    }
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

    // Development mode endpoints
    if dev_mode {
        // REPL endpoint
        if path == "/__dev/repl" && method == "POST" {
            return handle_dev_repl(req).await;
        }
        // Source code endpoint
        if path == "/__dev/source" && method == "GET" {
            return handle_dev_source(req).await;
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
    let (body, body_bytes_opt, multipart_form, multipart_files) = if method == "GET" || method == "HEAD" {
        (String::new(), None, None, None)
    } else {
        let body_bytes = http_body_util::BodyExt::collect(req.into_body())
            .await
            .map(|b| b.to_bytes().to_vec())
            .unwrap_or_default();

        // Check if this is a multipart form
        let content_type = headers.get("content-type").map(|s| s.as_str());
        if let Some(ct) = content_type {
            if ct.starts_with("multipart/form-data") {
                // Parse multipart form data
                let (form_fields, files) = parse_multipart_body(&body_bytes, ct).await;
                let body_str = String::from_utf8_lossy(&body_bytes).to_string();
                (body_str, Some(body_bytes), Some(form_fields), Some(files))
            } else {
                let body_str = String::from_utf8_lossy(&body_bytes).to_string();
                (body_str, None, None, None)
            }
        } else {
            let body_str = String::from_utf8_lossy(&body_bytes).to_string();
            (body_str, None, None, None)
        }
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
        body_bytes: body_bytes_opt,
        multipart_form,
        multipart_files,
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

            // Check if response is HTML for live reload injection
            let is_html = resp_data.headers.iter().any(|(k, v)| {
                k.to_lowercase() == "content-type" && v.contains("text/html")
            });

            for (key, value) in &resp_data.headers {
                builder = builder.header(key.as_str(), value.as_str());
            }

            // Inject live reload script for HTML responses when enabled
            let body = if is_html && reload_tx.is_some() {
                live_reload::inject_live_reload_script(&resp_data.body)
            } else {
                resp_data.body
            };

            Ok(builder
                .body(Full::new(Bytes::from(body)))
                .unwrap())
        }
        Err(_) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from("Internal Server Error")))
            .unwrap()),
    }
}

/// Check if the request is a WebSocket upgrade request.
#[allow(dead_code)]
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
    mut req: Request<Incoming>,
    ws_registry: Arc<WebSocketRegistry>,
    path: String,
    ws_event_tx: channel::Sender<WebSocketEventData>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Check if this is a valid WebSocket upgrade request
    if !hyper_tungstenite::is_upgrade_request(&req) {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Full::new(Bytes::from("Not a WebSocket upgrade request")))
            .unwrap());
    }

    // Perform the WebSocket upgrade
    let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, None) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("[WS] Upgrade error: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from(format!("WebSocket upgrade error: {}", e))))
                .unwrap());
        }
    };

    // Spawn a task to handle the WebSocket connection
    let ws_registry = ws_registry.clone();
    let ws_event_tx = ws_event_tx.clone();
    let path = path.clone();

    tokio::spawn(async move {
        // Wait for the WebSocket handshake to complete
        let stream = match websocket.await {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("[WS] WebSocket handshake error: {}", e);
                return;
            }
        };

        // Split the WebSocket stream into read and write halves
        let (mut ws_write, mut ws_read) = stream.split();

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

        // Spawn task to forward messages from channel to WebSocket
        let write_task = tokio::spawn(async move {
            while let Some(msg_result) = ws_rx.recv().await {
                match msg_result {
                    Ok(msg) => {
                        if ws_write.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Handle incoming messages
        while let Some(msg_result) = ws_read.next().await {
            match msg_result {
                Ok(msg) => {
                    if msg.is_close() {
                        break;
                    }
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
        write_task.abort();
    });

    // Return the upgrade response directly
    Ok(response)
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
fn handle_websocket_event(interpreter: &mut Interpreter, data: &WebSocketEventData, runtime_handle: &tokio::runtime::Handle) {
    use crate::interpreter::value::Value;

    // Clone connection_id for use in async spawns
    let connection_id = data.connection_id.to_string();

    // Find the WebSocket route for this path
    let routes = crate::serve::websocket::get_websocket_routes();
    let route = match routes.iter().find(|r| r.path_pattern == data.path) {
        Some(r) => r,
        None => return,
    };

    // Look up the handler from CONTROLLERS registry using the handler_name
    // Fall back to looking up the function directly in the environment
    let handler =
        match crate::interpreter::builtins::router::resolve_handler(&route.handler_name, None) {
            Ok(h) => h,
            Err(_) => {
                // Try to look up the function directly in the environment
                // handler_name format: "controller#action" - extract the action part
                let action_name = route.handler_name.split('#').last().unwrap_or(&route.handler_name);
                match interpreter.environment.borrow().get(action_name) {
                    Some(h) => h,
                    None => {
                        eprintln!(
                            "[WS] Failed to resolve handler '{}' - function '{}' not found",
                            route.handler_name, action_name
                        );
                        return;
                    }
                }
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
                                runtime_handle.spawn(async move {
                                    registry_clone.broadcast_all(&msg).await;
                                });
                            }
                            "send" => {
                                // Send to this specific client
                                let registry = crate::serve::websocket::get_ws_registry();
                                let registry_clone = registry.clone();
                                let msg = value.clone();
                                if let Ok(uuid) = connection_id.parse::<uuid::Uuid>() {
                                    runtime_handle.spawn(async move {
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

/// Handle a LiveView event by calling the controller handler.
fn handle_liveview_event(interpreter: &mut Interpreter, data: &LiveViewEventData) -> Result<(), String> {
    use crate::interpreter::value::Value;
    use crate::live::view::LIVE_REGISTRY;
    use crate::live::view::ServerMessage;

    // Get the LiveView instance
    let mut instance = LIVE_REGISTRY
        .get(&data.liveview_id)
        .ok_or_else(|| format!("LiveView not found: {}", data.liveview_id))?;

    let component = instance.component.clone();

    // Try to find a registered handler for this component
    let handler_name = crate::live::socket::get_liveview_handler(&component);

    // Build event hash for the controller: {event, params, state}
    let state_value = json_to_value(&instance.state);
    let params_value = json_to_value(&data.params);

    let event_pairs: Vec<(Value, Value)> = vec![
        (
            Value::String("event".to_string()),
            Value::String(data.event.clone()),
        ),
        (Value::String("params".to_string()), params_value),
        (Value::String("state".to_string()), state_value),
    ];
    let event_value = Value::Hash(Rc::new(RefCell::new(event_pairs)));

    // If we have a registered handler, call it
    if let Some(handler_name) = handler_name {
        // Try to resolve the handler from the controller registry
        let handler = match crate::interpreter::builtins::router::resolve_handler(&handler_name, None) {
            Ok(h) => h,
            Err(_) => {
                // Try to look up the function directly in the environment
                let action_name = handler_name.split('#').last().unwrap_or(&handler_name);
                match interpreter.environment.borrow().get(action_name) {
                    Some(h) => h,
                    None => {
                        // Fall back to hardcoded handler
                        return handle_liveview_event_fallback(data, &mut instance);
                    }
                }
            }
        };

        // Call the handler function
        match interpreter.call_value(handler, vec![event_value], Span::default()) {
            Ok(result) => {
                // Handler should return new state as a hash
                // If it returns null, fall back to built-in handler
                match &result {
                    Value::Null => {
                        return handle_liveview_event_fallback(data, &mut instance);
                    }
                    Value::Hash(_) => {
                        // Convert Value hash to JSON state
                        let new_state = value_to_json(&result);

                        // Preserve the id
                        let mut state = new_state.clone();
                        if let (serde_json::Value::Object(old), serde_json::Value::Object(ref mut new_obj)) =
                            (&instance.state, &mut state)
                        {
                            if let Some(id) = old.get("id") {
                                new_obj.insert("id".to_string(), id.clone());
                            }
                        }

                        instance.state = state;
                    }
                    _ => {
                        // Unexpected return type, fall back
                        return handle_liveview_event_fallback(data, &mut instance);
                    }
                }
            }
            Err(e) => {
                eprintln!("[LiveView] Handler error: {}", e);
                // Fall back to hardcoded handler
                return handle_liveview_event_fallback(data, &mut instance);
            }
        }
    } else {
        // No registered handler, use fallback
        return handle_liveview_event_fallback(data, &mut instance);
    }

    // Render new HTML and send patch
    render_and_send_patch(&component, &mut instance)
}

/// Fallback handler for LiveView events (for backwards compatibility)
fn handle_liveview_event_fallback(
    data: &LiveViewEventData,
    instance: &mut crate::live::view::LiveViewInstance,
) -> Result<(), String> {
    use serde_json::json;

    let component = instance.component.clone();

    // Update state based on event (hardcoded logic for backwards compatibility)
    // Note: Most handlers should be in .sl controller files via router_live()
    match (component.as_str(), data.event.as_str()) {
        ("counter", "increment") => {
            if let Some(count) = instance.state["count"].as_i64() {
                instance.state["count"] = json!(count + 1);
            }
        }
        ("counter", "decrement") => {
            if let Some(count) = instance.state["count"].as_i64() {
                instance.state["count"] = json!(count - 1);
            }
        }
        _ => return Err(format!("Unknown event: {} for component {}", data.event, component)),
    }

    // Render new HTML and send patch
    render_and_send_patch(&component, instance)
}

/// Render new HTML for a LiveView component and send the patch to the client.
fn render_and_send_patch(
    component: &str,
    instance: &mut crate::live::view::LiveViewInstance,
) -> Result<(), String> {
    use crate::live::component::render_component;
    use crate::live::view::{ServerMessage, LIVE_REGISTRY};

    // Render new HTML
    let new_html = render_component(component, &instance.state)?;
    let old_html = instance.last_html.clone();

    // Compute patch
    let patch = crate::live::diff::compute_patch(&old_html, &new_html);

    // Update last_html and save instance back to registry
    let liveview_id = instance.id.clone();
    instance.last_html = new_html;
    instance.touch();
    LIVE_REGISTRY.update(instance.clone());

    // Send patch to client
    let _ = LIVE_REGISTRY.send(
        &liveview_id,
        ServerMessage::Patch {
            liveview_id: liveview_id.to_string(),
            diff: patch,
        },
    );

    Ok(())
}

/// Convert serde_json::Value to interpreter Value
fn json_to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            let values: Vec<Value> = arr.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        serde_json::Value::Object(obj) => {
            let pairs: Vec<(Value, Value)> = obj
                .iter()
                .map(|(k, v)| (Value::String(k.clone()), json_to_value(v)))
                .collect();
            Value::Hash(Rc::new(RefCell::new(pairs)))
        }
    }
}

/// Convert interpreter Value to serde_json::Value
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::json!(*i),
        Value::Float(f) => serde_json::json!(*f),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => {
            let values: Vec<serde_json::Value> = arr.borrow().iter().map(value_to_json).collect();
            serde_json::Value::Array(values)
        }
        Value::Hash(hash) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in hash.borrow().iter() {
                if let Value::String(key) = k {
                    obj.insert(key.clone(), value_to_json(v));
                }
            }
            serde_json::Value::Object(obj)
        }
        _ => serde_json::Value::Null,
    }
}

/// Call the route handler with the request hash.
fn call_handler(interpreter: &mut Interpreter, handler_name: &str, request_hash: Value, dev_mode: bool, request_data: &RequestData) -> ResponseData {
    // Check if this is an OOP controller action (contains #)
    if handler_name.contains('#') {
        if let Some(response) = call_oop_controller_action(interpreter, handler_name, &request_hash, dev_mode, request_data) {
            return response;
        }
        // If not an OOP controller or error, fall through to function-based handling
    }

    // Use CONTROLLERS registry to look up handler by full name (controller#action)
    let handler_result = crate::interpreter::builtins::router::resolve_handler(handler_name, None);

    // Push stack frame for the handler (source path will be set from the function when called)
    interpreter.push_frame(handler_name, crate::span::Span::new(0, 0, 1, 1), None);

    match handler_result {
        Ok(handler_value) => {
            match interpreter.call_value(handler_value, vec![request_hash], Span::default()) {
                Ok(result) => {
                    interpreter.pop_frame();
                    let (status, headers, body) = extract_response(&result);
                    let headers: Vec<_> = headers.into_iter().collect();
                    ResponseData {
                        status,
                        headers,
                        body,
                    }
                }
                Err(e) => {
                    interpreter.pop_frame();
                    if dev_mode {
                        let stack_trace = interpreter.get_stack_trace();
                        let breakpoint_env = e.breakpoint_env_json();
                        let error_html = render_error_page(&e.to_string(), interpreter, request_data, &stack_trace, breakpoint_env);
                        ResponseData {
                            status: if e.is_breakpoint() { 200 } else { 500 },
                            headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                            body: error_html,
                        }
                    } else {
                        let error_html = render_production_error_page(500, "An error occurred while processing your request.");
                        ResponseData {
                            status: 500,
                            headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                            body: error_html,
                        }
                    }
                }
            }
        }
        Err(e) => {
            interpreter.pop_frame();
            if dev_mode {
                let stack_trace = interpreter.get_stack_trace();
                let error_html = render_error_page(&e.to_string(), interpreter, request_data, &stack_trace, None);
                ResponseData {
                    status: 500,
                    headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                    body: error_html,
                }
            } else {
                let error_html = render_production_error_page(500, "An error occurred while processing your request.");
                ResponseData {
                    status: 500,
                    headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                    body: error_html,
                }
            }
        }
    }
}

/// Call an OOP controller action (controller#action).
/// Returns Some(ResponseData) if handled, None if not an OOP controller.
fn call_oop_controller_action(interpreter: &mut Interpreter, handler_name: &str, request_hash: &Value, dev_mode: bool, request_data: &RequestData) -> Option<ResponseData> {
    let (controller_key, action_name) = handler_name.split_once('#')?;

    // Check if this is an OOP controller (has a class definition)
    // Convert controller_key (e.g., "posts") to PascalCase class name (e.g., "PostsController")
    let class_name = to_pascal_case_controller(controller_key);

    // Look up the class in the environment
    let class_value = interpreter.environment.borrow().get(&class_name)?;

    // Check if it's actually a class
    let class_rc = match class_value {
        Value::Class(class_rc) => class_rc,
        _ => return None,
    };

    // Get controller info from registry (optional - may not exist for simple OOP controllers)
    let controller_info = {
        let registry = CONTROLLER_REGISTRY.read().unwrap();
        registry.get(controller_key).cloned()
    };

    // Extract request components
    let req = request_hash.clone();
    let params = get_hash_field(request_hash, "params").unwrap_or(Value::Null);
    let session = get_hash_field(request_hash, "session").unwrap_or(Value::Null);
    let headers = get_hash_field(request_hash, "headers").unwrap_or(Value::Null);

    // Execute before_action hooks (if controller info exists)
    if let Some(ref info) = controller_info {
        if let Some(before_response) = execute_before_actions(interpreter, info, &action_name, req.clone(), &params, &session, &headers) {
            return Some(before_response);
        }
    }

    // Instantiate the controller
    let controller_instance = match create_controller_instance(&class_name, interpreter) {
        Ok(inst) => inst,
        Err(e) => {
            return Some(if dev_mode {
                let stack_trace = interpreter.get_stack_trace();
                let error_html = render_error_page(&e, interpreter, request_data, &stack_trace, None);
                ResponseData {
                    status: 500,
                    headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                    body: error_html,
                }
            } else {
                let error_html = render_production_error_page(500, "An error occurred while processing your request.");
                ResponseData {
                    status: 500,
                    headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                    body: error_html,
                }
            });
        }
    };

    // Set up controller context (req, params, session, headers)
    setup_controller_context(&controller_instance, &req, &params, &session, &headers);

    // Call the action method on the class
    // For OOP controllers, the method is inside the class, not in the global environment
    let action_result = call_class_method(interpreter, &class_rc, &controller_instance, action_name, &req);

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
        Err(e) => {
            if dev_mode {
                // Use breakpoint's captured stack trace if available, otherwise get current
                let stack_trace: Vec<String> = e.breakpoint_stack_trace()
                    .map(|st| st.to_vec())
                    .unwrap_or_else(|| interpreter.get_stack_trace());
                let breakpoint_env = e.breakpoint_env_json();
                let error_html = render_error_page(&e.to_string(), interpreter, request_data, &stack_trace, breakpoint_env);
                ResponseData {
                    status: if e.is_breakpoint() { 200 } else { 500 },
                    headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                    body: error_html,
                }
            } else {
                let error_html = render_production_error_page(500, "An error occurred while processing your request.");
                ResponseData {
                    status: 500,
                    headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                    body: error_html,
                }
            }
        }
    };

    // Execute after_action hooks (if controller info exists)
    if let Some(ref info) = controller_info {
        return Some(execute_after_actions(interpreter, info, &action_name, req, &response));
    }

    Some(response)
}

/// Call a method on a class instance.
fn call_class_method(
    interpreter: &mut Interpreter,
    class: &Rc<crate::interpreter::value::Class>,
    _instance: &Value,
    method_name: &str,
    request_hash: &Value,
) -> Result<Value, RuntimeError> {
    // Look up the method in the class
    if let Some(method) = class.methods.get(method_name) {
        // Push stack frame for the method call with the method's source path
        let method_span = method.span.unwrap_or_else(|| crate::span::Span::new(0, 0, 1, 1));
        interpreter.push_frame(&format!("{}#{}", class.name, method_name), method_span, method.source_path.clone());

        let result = interpreter.call_value(Value::Function(method.clone()), vec![request_hash.clone()], method_span);

        interpreter.pop_frame();

        result
    } else {
        Err(RuntimeError::General {
            message: format!("Method '{}' not found in class '{}'", method_name, class.name),
            span: Span::default(),
        })
    }
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
#[allow(dead_code)]
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

/// Convert uploaded files to Soli Value array.
fn uploaded_files_to_value(files: &[UploadedFile]) -> Value {
    let file_values: Vec<Value> = files
        .iter()
        .map(|f| {
            let pairs: Vec<(Value, Value)> = vec![
                (Value::String("name".to_string()), Value::String(f.name.clone())),
                (Value::String("filename".to_string()), Value::String(f.filename.clone())),
                (Value::String("content_type".to_string()), Value::String(f.content_type.clone())),
                (Value::String("size".to_string()), Value::Int(f.data.len() as i64)),
                (
                    Value::String("data".to_string()),
                    Value::Array(Rc::new(RefCell::new(
                        f.data.iter().map(|&b| Value::Int(b as i64)).collect(),
                    ))),
                ),
            ];
            Value::Hash(Rc::new(RefCell::new(pairs)))
        })
        .collect();
    Value::Array(Rc::new(RefCell::new(file_values)))
}

/// Parse request body based on Content-Type header.
fn parse_request_body(
    body: &str,
    content_type: Option<&str>,
    multipart_form: Option<&HashMap<String, String>>,
    multipart_files: Option<&Vec<UploadedFile>>,
) -> ParsedBody {
    let mut parsed = ParsedBody::default();

    // Handle multipart data if available (parsed in async context)
    if let Some(form_fields) = multipart_form {
        if !form_fields.is_empty() {
            let pairs: Vec<(Value, Value)> = form_fields
                .iter()
                .map(|(k, v)| (Value::String(k.clone()), Value::String(v.clone())))
                .collect();
            parsed.form = Some(Value::Hash(Rc::new(RefCell::new(pairs))));
        }
    }

    if let Some(files) = multipart_files {
        if !files.is_empty() {
            parsed.files = Some(uploaded_files_to_value(files));
        }
    }

    // If we already have multipart data, skip other parsing
    if parsed.form.is_some() || parsed.files.is_some() {
        return parsed;
    }

    if body.is_empty() {
        return parsed;
    }

    let content_type = match content_type {
        Some(ct) => ct.to_lowercase(),
        None => return parsed,
    };

    if content_type.starts_with("application/json") {
        parsed.json = parse_json_body(body);
    } else if content_type.starts_with("application/x-www-form-urlencoded") {
        parsed.form = parse_form_urlencoded_body(body);
    }

    parsed
}

/// Handle a single request (called on interpreter thread)
fn handle_request(interpreter: &mut Interpreter, data: &RequestData, dev_mode: bool) -> ResponseData {
    let start_time = Instant::now();
    let method = &data.method;
    let path = &data.path;

    // Check if request logging is enabled (default: true, disable with SOLI_REQUEST_LOG=false or SOLI_REQUEST_LOG=0)
    let log_requests = std::env::var("SOLI_REQUEST_LOG")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);

    // Set up session for this request
    let cookie_header = data.headers.get("cookie").map(|s| s.as_str());
    let existing_session_id = extract_session_id_from_cookie(cookie_header);
    let session_id = ensure_session(existing_session_id.as_deref());
    let is_new_session = existing_session_id.as_deref() != Some(&session_id);
    set_current_session_id(Some(session_id.clone()));

    // Find matching route using indexed lookup (O(1) for exact matches, O(m) for patterns)
    let (route, matched_params) = match find_route(method, path) {
        Some((r, params)) => (r, params),
        None => {
            // Clear session context before returning
            set_current_session_id(None);
            // Log timing for 404 responses (skip health checks)
            if log_requests && path != "/health" {
                let elapsed = start_time.elapsed();
                println!("[LOG] {} {} - 404 ({:.3}ms)", method, path, elapsed.as_secs_f64() * 1000.0);
            }
            let error_html = render_production_error_page(404, "The page you're looking for doesn't exist.");
            return ResponseData {
                status: 404,
                headers: if is_new_session {
                    vec![("Set-Cookie".to_string(), create_session_cookie(&session_id)), ("Content-Type".to_string(), "text/html; charset=utf-8".to_string())]
                } else {
                    vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())]
                },
                body: error_html,
            };
        }
    };

    let handler_name = route.handler_name.clone();
    let scoped_middleware = route.middleware.clone();

    // Extract Content-Type for body parsing
    let content_type = data.headers.get("content-type").map(|s| s.as_str());

    // Parse body based on Content-Type (including multipart data parsed in async context)
    let parsed_body = parse_request_body(
        &data.body,
        content_type,
        data.multipart_form.as_ref(),
        data.multipart_files.as_ref(),
    );

    // Build request hash with parsed body - optimize for empty query/headers/body
    let mut request_hash = if data.query.is_empty() && data.headers.is_empty() && data.body.is_empty() {
        build_request_hash_with_parsed(
            &data.method,
            &data.path,
            matched_params,
            HashMap::new(),
            HashMap::new(),
            String::new(),
            parsed_body,
        )
    } else {
        build_request_hash_with_parsed(
            &data.method,
            &data.path,
            matched_params,
            data.query.clone(),
            data.headers.clone(),
            data.body.clone(),
            parsed_body,
        )
    };

    // Helper to finalize response with session cookie and timing
    let finalize_response = |mut resp: ResponseData| -> ResponseData {
        // Add session cookie if it's a new session
        if is_new_session {
            resp.headers.push(("Set-Cookie".to_string(), create_session_cookie(&session_id)));
        }
        // Log timing (skip health checks to avoid benchmark noise)
        if log_requests && path != "/health" {
            let elapsed = start_time.elapsed();
            println!("[LOG] {} {} - {} ({:.3}ms)", method, path, resp.status, elapsed.as_secs_f64() * 1000.0);
        }
        // Clear session context
        set_current_session_id(None);
        resp
    };

    // Fast path: no middleware at all (avoid cloning middleware list if empty)
    if scoped_middleware.is_empty() && !has_middleware() {
        return finalize_response(call_handler(interpreter, &handler_name, request_hash, dev_mode, data));
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
                    return finalize_response(ResponseData {
                        status,
                        headers,
                        body,
                    });
                }
                MiddlewareResult::Error(err) => {
                    if dev_mode {
                        let stack_trace = interpreter.get_stack_trace();
                        let error_html = render_error_page(&err.to_string(), interpreter, data, &stack_trace, None);
                        return finalize_response(ResponseData {
                            status: 500,
                            headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                            body: error_html,
                        });
                    }
                    let error_html = render_production_error_page(500, "An error occurred while processing your request.");
                    return finalize_response(ResponseData {
                        status: 500,
                        headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                        body: error_html,
                    });
                }
            },
            Err(e) => {
                if dev_mode {
                    let stack_trace = interpreter.get_stack_trace();
                    let breakpoint_env = e.breakpoint_env_json();
                    let error_html = render_error_page(&e.to_string(), interpreter, data, &stack_trace, breakpoint_env);
                    return finalize_response(ResponseData {
                        status: if e.is_breakpoint() { 200 } else { 500 },
                        headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                        body: error_html,
                    });
                }
                let error_html = render_production_error_page(500, "An error occurred while processing your request.");
                return finalize_response(ResponseData {
                    status: 500,
                    headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                    body: error_html,
                });
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
                    return finalize_response(ResponseData {
                        status,
                        headers,
                        body,
                    });
                }
                MiddlewareResult::Error(err) => {
                    if dev_mode {
                        let stack_trace = interpreter.get_stack_trace();
                        let error_html = render_error_page(&err.to_string(), interpreter, data, &stack_trace, None);
                        return finalize_response(ResponseData {
                            status: 500,
                            headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                            body: error_html,
                        });
                    }
                    let error_html = render_production_error_page(500, "An error occurred while processing your request.");
                    return finalize_response(ResponseData {
                        status: 500,
                        headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                        body: error_html,
                    });
                }
            },
            Err(e) => {
                if dev_mode {
                    let stack_trace = interpreter.get_stack_trace();
                    let breakpoint_env = e.breakpoint_env_json();
                    let error_html = render_error_page(&e.to_string(), interpreter, data, &stack_trace, breakpoint_env);
                    return finalize_response(ResponseData {
                        status: if e.is_breakpoint() { 200 } else { 500 },
                        headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                        body: error_html,
                    });
                }
                let error_html = render_production_error_page(500, "An error occurred while processing your request.");
                return finalize_response(ResponseData {
                    status: 500,
                    headers: vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string())],
                    body: error_html,
                });
            }
        }
    }

    // Call the route handler
    finalize_response(call_handler(interpreter, &handler_name, request_hash, dev_mode, data))
}

/// Handle REPL execution for dev mode.
async fn handle_dev_repl(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let body = req.into_body().collect().await?.to_bytes();
    let body_str = String::from_utf8_lossy(&body);

    // Parse JSON body
    let json: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(json) => json,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(r#"{"error": "Invalid JSON body"}"#)))
                .unwrap());
        }
    };

    let code = json.get("code").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let request_data = json.get("request_data").cloned();
    let breakpoint_env = json.get("breakpoint_env").cloned();

    // Execute the code using the interpreter
    let result = execute_repl_code(&code, request_data, breakpoint_env);

    let response_json = serde_json::json!({
        "result": result.result,
        "error": result.error
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(response_json.to_string())))
        .unwrap())
}

/// Handle source code fetching for dev mode.
async fn handle_dev_source(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let uri = req.uri();
    let query = uri.query().unwrap_or("");

    // Parse query parameters
    let file = query.split('&')
        .filter_map(|p| {
            let mut parts = p.split('=');
            match (parts.next(), parts.next()) {
                (Some("file"), Some(f)) => Some(("file", f)),
                _ => None,
            }
        })
        .find(|(k, _)| *k == "file")
        .map(|(_, f)| urlencoding::decode(f).unwrap_or_else(|_| Cow::Borrowed(f)).into_owned())
        .unwrap_or_else(String::new);

    if file.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(r#"{"error": "Missing file parameter"}"#)))
            .unwrap());
    }

    // Try to read the file
    let path = std::path::Path::new(&file);
    if !path.exists() {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(r#"{"error": "File not found"}"#)))
            .unwrap());
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(r#"{"error": "Could not read file"}"#)))
                .unwrap());
        }
    };

    // Parse line from query
    let line: usize = query.split('&')
        .filter_map(|p| {
            let mut parts = p.split('=');
            match (parts.next(), parts.next()) {
                (Some("line"), Some(l)) => l.parse().ok(),
                _ => None,
            }
        })
        .next()
        .unwrap_or(1);

    // Build lines map
    let lines: std::collections::HashMap<usize, String> = content
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.to_string()))
        .collect();

    let response = serde_json::json!({
        "file": file,
        "line": line,
        "lines": lines
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(response.to_string())))
        .unwrap())
}

struct ReplResult {
    result: String,
    error: Option<String>,
}

fn execute_repl_code(code: &str, request_data: Option<serde_json::Value>, breakpoint_env: Option<serde_json::Value>) -> ReplResult {
    if code.trim().is_empty() {
        return ReplResult {
            result: "null".to_string(),
            error: None,
        };
    }

    let mut interpreter = crate::interpreter::Interpreter::new();

    // Inject view helpers into REPL environment (same helpers available in templates)
    for (name, value) in crate::interpreter::builtins::template::get_view_helpers() {
        interpreter.environment.borrow_mut().define(name, value);
    }

    // Set up breakpoint environment variables first (these are the captured variables)
    if let Some(env_obj) = breakpoint_env {
        if let serde_json::Value::Object(map) = env_obj {
            for (name, value) in map {
                // Skip internal variables
                if !name.starts_with("__") {
                    interpreter.environment.borrow_mut().define(name, convert_json_to_value(value));
                }
            }
        }
    }

    // Set up environment variables from request data
    if let Some(data) = request_data {
        let req_val = convert_json_to_value(data.clone());
        interpreter.environment.borrow_mut().define("req".to_string(), req_val);

        if let Some(v) = data.get("params").cloned() {
            interpreter.environment.borrow_mut().define("params".to_string(), convert_json_to_value(v));
        }
        if let Some(v) = data.get("query").cloned() {
            interpreter.environment.borrow_mut().define("query".to_string(), convert_json_to_value(v));
        }
        if let Some(v) = data.get("body").cloned() {
            interpreter.environment.borrow_mut().define("body".to_string(), convert_json_to_value(v));
        }
        if let Some(v) = data.get("headers").cloned() {
            interpreter.environment.borrow_mut().define("headers".to_string(), convert_json_to_value(v));
        }
        if let Some(v) = data.get("session").cloned() {
            interpreter.environment.borrow_mut().define("session".to_string(), convert_json_to_value(v));
        }
    }

    // Strip trailing semicolon for expression evaluation
    let code_trimmed = code.trim().trim_end_matches(';').trim();

    // First, try to evaluate as an expression (to capture and return the value)
    let wrapped_code = format!("let __repl_result__ = ({});", code_trimmed);
    let tokens = crate::lexer::Scanner::new(&wrapped_code).scan_tokens();
    let parse_result = tokens.map_err(|e| format!("{:?}", e))
        .and_then(|tokens| crate::parser::Parser::new(tokens).parse().map_err(|e| format!("{:?}", e)));

    if let Ok(program) = parse_result {
        match interpreter.interpret(&program) {
            Ok(_) => {
                // Get the result from environment
                let result_val = interpreter.environment.borrow().get("__repl_result__");
                let result_str = match result_val {
                    Some(v) => format!("{}", v),
                    None => "null".to_string(),
                };
                return ReplResult {
                    result: result_str,
                    error: None,
                };
            }
            Err(e) => {
                return ReplResult {
                    result: "null".to_string(),
                    error: Some(format!("Execution error: {}", e)),
                };
            }
        }
    }

    // If expression evaluation failed, try parsing as a complete program (statements)
    let tokens = crate::lexer::Scanner::new(code).scan_tokens();
    let parse_result = tokens.map_err(|e| format!("{:?}", e))
        .and_then(|tokens| crate::parser::Parser::new(tokens).parse().map_err(|e| format!("{:?}", e)));

    match parse_result {
        Ok(program) => {
            match interpreter.interpret(&program) {
                Ok(_) => {
                    ReplResult {
                        result: "ok".to_string(),
                        error: None,
                    }
                }
                Err(e) => ReplResult {
                    result: "null".to_string(),
                    error: Some(format!("Execution error: {}", e)),
                },
            }
        }
        Err(parse_errors) => {
            ReplResult {
                result: "null".to_string(),
                error: Some(format!("Parse error: {}", parse_errors)),
            }
        }
    }
}

/// Helper to convert JSON to Value, returning Null on error.
fn convert_json_to_value(json: serde_json::Value) -> crate::interpreter::value::Value {
    crate::interpreter::value::json_to_value(&json).unwrap_or(crate::interpreter::value::Value::Null)
}

/// Helper function to render error page with full details.
/// If `breakpoint_env_json` is provided, it will be used to populate REPL variables.
/// Otherwise, variables are captured from the current interpreter environment.
fn render_error_page(error_msg: &str, interpreter: &Interpreter, request_data: &RequestData, stack_trace: &[String], breakpoint_env_json: Option<&str>) -> String {
    let error_type = if breakpoint_env_json.is_some() { "Breakpoint" } else { "RuntimeError" };

    // Capture environment: use breakpoint env if provided, otherwise capture from interpreter
    let captured_env = if let Some(env) = breakpoint_env_json {
        env.to_string()
    } else {
        interpreter.serialize_environment_for_debug()
    };
    let env_json_for_render: Option<&str> = Some(&captured_env);
    let location;
    let mut full_stack_trace: Vec<String> = Vec::new();

    // Extract error message and stack trace from the combined error
    let (actual_error, embedded_stack): (String, Vec<String>) = if let Some(stack_start) = error_msg.find("Stack trace:\n") {
        let error_part = error_msg[..stack_start].trim().to_string();
        let stack_part = error_msg[stack_start + "Stack trace:\n".len()..]
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        (error_part, stack_part)
    } else {
        (error_msg.to_string(), Vec::new())
    };

    // Try to parse span info from error message
    let span_info = extract_span_from_error(&actual_error);
    let error_line = span_info.line;

    // Try to get file from error message, or fall back to first stack frame
    let error_file = span_info.file.clone().or_else(|| {
        // Try embedded stack trace first
        for frame in &embedded_stack {
            if let Some(file) = extract_file_from_frame(frame) {
                return Some(file);
            }
        }
        // Then try passed stack trace
        for frame in stack_trace {
            if let Some(file) = extract_file_from_frame(frame) {
                return Some(file);
            }
        }
        None
    }).unwrap_or_else(|| "unknown".to_string());

    location = format!("{}:{}", error_file, error_line);

    // Build stack trace - first the error, then controller stack trace, then view
    full_stack_trace.push(format!("Error: {}", actual_error));

    // Add embedded stack trace (from interpreter) and passed stack trace (controllers)
    full_stack_trace.extend(embedded_stack);
    full_stack_trace.extend(stack_trace.iter().cloned());

    // If the error is from a view file, add a stack frame for it AFTER controllers
    // The error format can be either:
    // 1. "error at line:col in path/file.html.erb" (from interpreter)
    // 2. "error in path/file.html.erb at line:col" (alternative format)
    // 3. "error at path/file.html.erb:line" (new template renderer format)
    // 4. "error in path/file.html.erb" (no line info, use line 1)

    // Try format 1: "at line:col in file.html.erb"
    let view_pattern1 = regex::Regex::new(r"at (\d+):(\d+) in ([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb))").ok();
    // Try format 2: "in file.html.erb at line:col"
    let view_pattern2 = regex::Regex::new(r"in ([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb)) at (\d+):(\d+)").ok();
    // Try format 3: "at file.html.erb:line" (new template renderer format)
    let view_pattern3 = regex::Regex::new(r"at ([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb)):(\d+)").ok();
    // Try format 4: "in file.html.erb" (no line number)
    let view_pattern4 = regex::Regex::new(r"in ([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb))(?:\s|$)").ok();

    let mut view_added = false;

    // Try pattern 1 first
    if let Some(re) = view_pattern1 {
        if let Some(caps) = re.captures(&actual_error) {
            let view_line = caps.get(1).map(|m| m.as_str()).unwrap_or("1");
            let view_file = caps.get(3).map(|m| m.as_str()).unwrap_or("");
            if !view_file.is_empty() {
                full_stack_trace.push(format!("[view] at {}:{}", view_file, view_line));
                view_added = true;
            }
        }
    }

    // Try pattern 2 if pattern 1 didn't match
    if !view_added {
        if let Some(re) = view_pattern2 {
            if let Some(caps) = re.captures(&actual_error) {
                let view_file = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let view_line = caps.get(2).map(|m| m.as_str()).unwrap_or("1");
                if !view_file.is_empty() {
                    full_stack_trace.push(format!("[view] at {}:{}", view_file, view_line));
                    view_added = true;
                }
            }
        }
    }

    // Try pattern 3 if pattern 2 didn't match (new template renderer format)
    if !view_added {
        if let Some(re) = view_pattern3 {
            if let Some(caps) = re.captures(&actual_error) {
                let view_file = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let view_line = caps.get(2).map(|m| m.as_str()).unwrap_or("1");
                if !view_file.is_empty() {
                    full_stack_trace.push(format!("[view] at {}:{}", view_file, view_line));
                    view_added = true;
                }
            }
        }
    }

    // Try pattern 4 if no line info found
    if !view_added {
        if let Some(re) = view_pattern4 {
            if let Some(caps) = re.captures(&actual_error) {
                let view_file = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                if !view_file.is_empty() {
                    full_stack_trace.push(format!("[view] at {}:1", view_file));
                }
            }
        }
    }

    if full_stack_trace.len() == 1 {
        full_stack_trace.push(format!("{}:{} (error location)", error_file, error_line));
    }

    // Get source code around the error line
    let _source_preview = get_source_preview(&error_file, error_line);

    let mut request_hash_map = HashMap::new();
    request_hash_map.insert("method".to_string(), Value::String(request_data.method.clone()));
    request_hash_map.insert("path".to_string(), Value::String(request_data.path.clone()));
    request_hash_map.insert("params".to_string(), Value::String(format!("{:?}", request_data.query)));
    request_hash_map.insert("headers".to_string(), Value::String(format!("{:?}", request_data.headers)));
    request_hash_map.insert("body".to_string(), Value::String(request_data.body.clone()));
    request_hash_map.insert("session".to_string(), Value::String("N/A".to_string()));

    // Serialize request data for REPL (manual serialization)
    // Include both "params" and "query" as aliases for the query string parameters
    let query_json = format!("{:?}", request_data.query);
    let headers_json = format!("{:?}", request_data.headers);
    let request_data_json = format!(r#"{{"method":"{}","path":"{}","params":{},"query":{},"headers":{},"body":"{}","session":"N/A"}}"#,
        request_data.method,
        request_data.path,
        query_json,
        query_json,
        headers_json,
        request_data.body
    );

    render_dev_error_page(
        &actual_error,
        error_type,
        &location,
        &full_stack_trace,
        &request_data_json,
        env_json_for_render,
    )
}

fn extract_json_field(json: &str, field: &str) -> Option<String> {
    let pattern = format!(r#""{}":"#, field);
    if let Some(start) = json.find(&pattern) {
        let after_start = start + pattern.len();
        // Find the end of the value (comma or closing brace)
        let mut end = after_start;
        let mut depth = 0;
        let chars: Vec<char> = json[after_start..].chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if *c == '{' || *c == '[' {
                depth += 1;
            } else if *c == '}' || *c == ']' {
                depth -= 1;
                if depth == 0 {
                    end = after_start + i + 1;
                    break;
                }
            } else if *c == ',' && depth == 0 {
                end = after_start + i;
                break;
            }
        }
        return Some(json[after_start..end].to_string());
    }
    None
}

#[allow(dead_code)]
struct SpanInfo {
    file: Option<String>,
    line: usize,
    column: usize,
}

fn extract_span_from_error(error_msg: &str) -> SpanInfo {
    // Try to find file path pattern - supports .sl, .html.erb, and .erb files
    // Include @ for paths like /home/user@domain.com/...
    let file_re = regex::Regex::new(r"([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb|\.sl))").unwrap();
    let file = file_re.captures(error_msg)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string());

    // Try format "at file.html.erb:line" first (new template renderer format)
    // This prioritizes view file errors over controller errors
    let at_file_line_re = regex::Regex::new(r"at ([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb)):(\d+)").unwrap();
    if let Some(caps) = at_file_line_re.captures(error_msg) {
        let file = caps.get(1).map(|m| m.as_str().to_string());
        let line = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        return SpanInfo { file, line, column: 1 };
    }

    // Try to find span format "at line:column" (e.g., "at 11:23")
    // This is the standard Span display format from error messages
    let span_re = regex::Regex::new(r" at (\d+):(\d+)").unwrap();
    if let Some(caps) = span_re.captures(error_msg) {
        let line = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        let column = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        return SpanInfo { file, line, column };
    }

    // Try to find patterns like "file.sl:line" or "file.html.erb:line"
    let file_line_re = regex::Regex::new(r"([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb|\.sl)):(\d+)").unwrap();
    if let Some(caps) = file_line_re.captures(error_msg) {
        let file = caps.get(1).map(|m| m.as_str().to_string());
        let line = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        return SpanInfo { file, line, column: 1 };
    }

    // Try to find "line X" or "line: X" patterns
    let line_re = regex::Regex::new(r"(?:at\s+)?line\s*[=:]\s*(\d+)").unwrap();
    if let Some(caps) = line_re.captures(error_msg) {
        let line = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        return SpanInfo { file, line, column: 1 };
    }

    SpanInfo { file, line: 1, column: 1 }
}

/// Extract file path from a stack frame string like "func_name at ./path/file.sl:10"
fn extract_file_from_frame(frame: &str) -> Option<String> {
    // Support .sl, .html.erb, and .erb files (include @ for paths like /home/user@domain.com/...)
    let file_re = regex::Regex::new(r"([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb|\.sl))").ok()?;
    file_re.captures(frame)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

fn get_source_preview(file_path: &str, error_line: usize) -> Vec<String> {
    if file_path.is_empty() || file_path == "unknown" {
        return Vec::new();
    }

    let path = std::path::Path::new(file_path);
    if !path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let start = if error_line > 3 { error_line - 3 } else { 1 };
    let end = error_line + 3;

    content
        .lines()
        .enumerate()
        .filter(|(i, _)| {
            let line_num = i + 1;
            line_num >= start && line_num <= end
        })
        .map(|(i, line)| {
            let line_num = i + 1;
            let marker = if line_num == error_line { ">>>" } else { "   " };
            format!("{} {:4} | {}", marker, line_num, line)
        })
        .collect()
}

/// Render the development error page with request details and REPL.
pub fn render_dev_error_page(
    error: &str,
    error_type: &str,
    location: &str,
    stack_trace: &[String],
    request_data_json: &str,
    breakpoint_env_json: Option<&str>,
) -> String {
    let error_message = escape_html(error);
    let error_type = escape_html(error_type);
    let error_location = escape_html(location);

    // Format stack trace
    // Parse stack frames which can be in various formats:
    // 1. "{function_name} at {file}:{line}" - from get_stack_trace()
    // 2. "{file}:{line} at {span}" - when span info is appended
    // 3. "{file}:{line} (error location)" - fallback format
    let mut stack_frames = Vec::new();
    let mut frame_index = 0;

    // Regex to find file paths with line numbers - supports .sl, .html.erb, .erb (include @ for paths)
    let file_regex = regex::Regex::new(r"([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb|\.sl)):(\d+)").unwrap();
    // Regex to find span info after "at" (line:column)
    let span_regex = regex::Regex::new(r" at (\d+):(\d+)").unwrap();
    // Regex to find view files in error messages (e.g., "error in /path/to/file.html.erb")
    let view_file_regex = regex::Regex::new(r"in ([./a-zA-Z0-9_@-]+(?:\.html\.erb|\.erb))").unwrap();

    for frame in stack_trace.iter() {
        // Skip "Error: ..." entries - they're not actual stack frames
        if frame.starts_with("Error: ") {
            continue;
        }

        // Check if this is a view frame (marked with [view])
        let is_view_frame = frame.starts_with("[view]");

        let mut file = "unknown".to_string();
        let mut line: usize = 0;

        // First, try to extract file path (supports .sl, .html.erb, .erb)
        if let Some(caps) = file_regex.captures(frame) {
            file = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            // Get line from file:line pattern as fallback
            line = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        } else if let Some(caps) = view_file_regex.captures(frame) {
            // Try to find view file path in error message like "error in /path/file.html.erb"
            file = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
        }

        // If there's a span after "at" (like "at 11:23"), prefer that line number
        // because it's the actual error location, not the function definition line
        if let Some(caps) = span_regex.captures(frame) {
            if let Some(span_line) = caps.get(1).and_then(|m| m.as_str().parse().ok()) {
                line = span_line;
            }
        }

        // Helper to check if string contains a source file extension
        let contains_source_ext = |s: &str| s.contains(".sl") || s.contains(".html.erb") || s.contains(".erb");

        // Determine display name based on frame type
        let (display_name, icon_html) = if is_view_frame {
            // For view frames, extract just the view name from the file path
            let view_name = file.rsplit('/').next().unwrap_or(&file);
            // Add a document icon for views
            (view_name.to_string(), r#"<svg class="inline-block w-4 h-4 mr-1.5 -mt-0.5 text-teal-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path></svg>"#)
        } else {
            // Try to extract function name - look for pattern before " at "
            let func = if let Some(at_pos) = frame.find(" at ") {
                let before_at = &frame[..at_pos];
                // If what's before "at" doesn't contain a source file, it's the function name
                if !contains_source_ext(before_at) {
                    before_at.to_string()
                } else {
                    // Extract from file name
                    extract_controller_name(&file)
                }
            } else if file != "unknown" {
                // No " at " - extract function name from file
                extract_controller_name(&file)
            } else {
                // Fallback: treat entire frame as function name
                frame.clone()
            };

            // Clean up function name - if it looks like a file path, extract the name
            let display = if func.contains('#') || func.contains("::") {
                func.clone()
            } else if func.contains('/') || contains_source_ext(&func) {
                extract_controller_name(&func)
            } else if func == "unknown" && file != "unknown" {
                extract_controller_name(&file)
            } else {
                func.clone()
            };
            (display, "")
        };

        let location_display = format!("{}:{}", file, line);

        // Use different colors for views vs controllers
        let (name_color, location_color, border_color) = if is_view_frame {
            ("text-teal-300", "text-teal-400/70", "border-l-2 border-teal-400")
        } else {
            ("text-white", "text-gray-400", "")
        };

        stack_frames.push(format!(
            r#"<div class="stack-frame px-4 py-3 border-b border-white/5 hover:bg-white/5 transition-colors {}" onclick="showSource('{}', {}, this)">
                <div class="flex items-start gap-3">
                    <span class="text-gray-500 text-xs mt-0.5">{}</span>
                    <div class="flex-1 min-w-0">
                        <div class="font-medium {} truncate">{}{}</div>
                        <div class="{} text-sm truncate">{}</div>
                    </div>
                </div>
            </div>"#,
            border_color, escape_html(&file), line, frame_index, name_color, icon_html, escape_html(&display_name), location_color, escape_html(&location_display)
        ));
        frame_index += 1;
    }

    // Parse request data from JSON
    let request_method = extract_json_field(request_data_json, "method").unwrap_or("UNKNOWN".to_string());
    let request_path = extract_json_field(request_data_json, "path").unwrap_or("/".to_string());
    let request_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Error - {error_type}</title>
    <script src="https://cdn.tailwindcss.com"></script>
    <style>
        .code-editor {{
            font-family: 'JetBrains Mono', 'Fira Code', monospace;
            font-size: 14px;
            line-height: 1.6;
        }}
        .repl-output {{ min-height: 100px; max-height: 400px; overflow-y: auto; }}
        .stack-frame {{ cursor: pointer; }}
        .stack-frame:hover {{ background-color: rgba(99, 102, 241, 0.1); }}
        .stack-frame.active {{ background-color: rgba(99, 102, 241, 0.2); border-left: 3px solid #6366f1; }}
        .section-content {{ display: none; }}
        .section-content.active {{ display: block; }}
        .request-tab.active {{ background-color: rgba(99, 102, 241, 0.2); border-bottom: 2px solid #6366f1; }}
        .loading-spinner {{
            border: 2px solid rgba(255,255,255,0.3);
            border-top: 2px solid #6366f1;
            border-radius: 50%;
            width: 16px;
            height: 16px;
            animation: spin 1s linear infinite;
        }}
        @keyframes spin {{ 0% {{ transform: rotate(0deg); }} 100% {{ transform: rotate(360deg); }} }}
    </style>
</head>
<body class="bg-gray-950 text-gray-100 min-h-screen">
    <div class="max-w-7xl mx-auto p-6">
        <div class="mb-8 border-b border-white/10 pb-6">
            <div class="flex items-center gap-3 mb-2">
                <div class="px-3 py-1 rounded-full bg-red-500/20 text-red-400 text-sm font-medium">{error_type}</div>
                <span class="text-gray-500">Development Mode</span>
            </div>
            <h1 class="text-3xl font-bold text-white mb-2">{error_message}</h1>
            <p class="text-gray-400">{error_location}</p>
        </div>

        <div class="mb-8 rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
            <div class="flex items-center justify-between px-4 py-3 bg-gray-800 border-b border-white/10">
                <div class="flex items-center gap-2">
                    <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                    </svg>
                    <span class="font-semibold text-white">Interactive REPL</span>
                </div>
                <button onclick="clearRepl()" class="text-gray-400 hover:text-white text-sm">Clear</button>
            </div>
            <div class="p-4">
                <div class="flex gap-2 mb-3">
                    <input type="text" id="repl-input" class="flex-1 bg-gray-800 border border-white/20 rounded-lg px-4 py-2 text-white placeholder-gray-500 focus:outline-none focus:border-indigo-500 code-editor" placeholder="Type Soli code to inspect request state..." onkeydown="if(event.key==='Enter'&&!event.shiftKey){{event.preventDefault();executeRepl();}}">
                    <button onclick="executeRepl()" class="px-6 py-2 bg-indigo-600 hover:bg-indigo-700 text-white rounded-lg font-medium transition-colors flex items-center gap-2">
                        <span>Run</span>
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z" />
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                        </svg>
                    </button>
                </div>
                <div id="repl-output" class="repl-output bg-gray-800 rounded-lg p-4 text-sm code-editor min-h-[120px]">
                    <div class="text-gray-500 italic">// Try: req["params"]["id"] or session["user_id"] or headers["Content-Type"]</div>
                </div>
            </div>
        </div>

        <div class="grid grid-cols-1 lg:grid-cols-3 gap-8">
            <div class="lg:col-span-2">
                <div class="mb-6">
                    <h2 class="text-xl font-bold text-white mb-4 flex items-center gap-2">
                        <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
                        </svg>
                        Stack Trace
                    </h2>
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        {stack_frames}
                    </div>
                </div>

                <div class="mb-6">
                    <h2 class="text-xl font-bold text-white mb-4 flex items-center gap-2">
                        <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                        </svg>
                        Source Code
                    </h2>
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <div class="px-4 py-2 bg-gray-800 border-b border-white/10 flex items-center justify-between">
                            <span id="source-file" class="text-sm text-gray-400 font-mono">Select a stack frame to view source</span>
                            <span id="source-line" class="text-sm text-gray-500"></span>
                        </div>
                        <pre id="source-code" class="p-4 overflow-x-auto code-editor text-sm"><code class="language-soli text-gray-400">// Click on a stack frame above to see the source code</code></pre>
                    </div>
                </div>

                <div class="mb-6">
                    <h2 class="text-xl font-bold text-white mb-4 flex items-center gap-2">
                        <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                        </svg>
                        Quick Inspect
                    </h2>
                    <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
                        <button onclick="quickInspect('req')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors">
                            <div class="text-xs text-gray-500 mb-1">Request</div>
                            <div class="text-sm text-white font-mono truncate">req</div>
                        </button>
                        <button onclick="quickInspect('req[\"params\"]')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors">
                            <div class="text-xs text-gray-500 mb-1">Params</div>
                            <div class="text-sm text-white font-mono truncate">params</div>
                        </button>
                        <button onclick="quickInspect('req[\"query\"]')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors">
                            <div class="text-xs text-gray-500 mb-1">Query</div>
                            <div class="text-sm text-white font-mono truncate">query</div>
                        </button>
                        <button onclick="quickInspect('req[\"body\"]')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors">
                            <div class="text-xs text-gray-500 mb-1">Body</div>
                            <div class="text-sm text-white font-mono truncate">body</div>
                        </button>
                        <button onclick="quickInspect('session')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors">
                            <div class="text-xs text-gray-500 mb-1">Session</div>
                            <div class="text-sm text-white font-mono truncate">session</div>
                        </button>
                        <button onclick="quickInspect('headers')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors">
                            <div class="text-xs text-gray-500 mb-1">Headers</div>
                            <div class="text-sm text-white font-mono truncate">headers</div>
                        </button>
                    </div>
                </div>
            </div>

            <div>
                <h2 class="text-xl font-bold text-white mb-4 flex items-center gap-2">
                    <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
                    </svg>
                    Request Details
                </h2>
                <div class="flex border-b border-white/10 mb-4">
                    <button class="request-tab active px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('params')">Params</button>
                    <button class="request-tab px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('query')">Query</button>
                    <button class="request-tab px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('body')">Body</button>
                    <button class="request-tab px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('headers')">Headers</button>
                    <button class="request-tab px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('session')">Session</button>
                </div>
                <div id="tab-params" class="section-content active">
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <pre id="json-params" class="p-4 overflow-x-auto text-sm code-editor"></pre>
                    </div>
                </div>
                <div id="tab-query" class="section-content">
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <pre id="json-query" class="p-4 overflow-x-auto text-sm code-editor"></pre>
                    </div>
                </div>
                <div id="tab-body" class="section-content">
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <pre id="json-body" class="p-4 overflow-x-auto text-sm code-editor"></pre>
                    </div>
                </div>
                <div id="tab-headers" class="section-content">
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <pre id="json-headers" class="p-4 overflow-x-auto text-sm code-editor"></pre>
                    </div>
                </div>
                <div id="tab-session" class="section-content">
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <pre id="json-session" class="p-4 overflow-x-auto text-sm code-editor"></pre>
                    </div>
                </div>
                <div class="mt-6">
                    <h3 class="text-lg font-semibold text-white mb-3">Environment</h3>
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <div class="divide-y divide-white/5">
                            <div class="px-4 py-2 flex justify-between">
                                <span class="text-gray-500">Time</span>
                                <span class="text-gray-300 font-mono text-sm">{request_time}</span>
                            </div>
                            <div class="px-4 py-2 flex justify-between">
                                <span class="text-gray-500">Method</span>
                                <span class="text-gray-300 font-mono text-sm">{request_method}</span>
                            </div>
                            <div class="px-4 py-2 flex justify-between">
                                <span class="text-gray-500">Path</span>
                                <span class="text-gray-300 font-mono text-sm truncate ml-2">{request_path}</span>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    </div>

    <script>
        const sourceCache = {{}};
        const currentRequestData = {request_data_json};
        const breakpointEnv = {breakpoint_env_js};

        function showRequestTab(tabName) {{
            document.querySelectorAll('.section-content').forEach(el => el.classList.remove('active'));
            document.querySelectorAll('.request-tab').forEach(el => el.classList.remove('active'));
            document.getElementById('tab-' + tabName).classList.add('active');
            event.target.classList.add('active');
        }}

        async function showSource(file, line, element) {{
            document.querySelectorAll('.stack-frame').forEach(el => el.classList.remove('active'));
            element.classList.add('active');
            document.getElementById('source-file').textContent = file + ':' + line;
            document.getElementById('source-line').textContent = 'Line ' + line;

            const cacheKey = file + ':' + line;
            if (sourceCache[cacheKey]) {{
                displaySource(sourceCache[cacheKey], line);
                return;
            }}

            try {{
                const response = await fetch('/__dev/source?file=' + encodeURIComponent(file) + '&line=' + line);
                if (response.ok) {{
                    const data = await response.json();
                    sourceCache[cacheKey] = data;
                    displaySource(data, line);
                }} else {{
                    document.getElementById('source-code').innerHTML = '<code class="text-gray-500">// Source not available</code>';
                }}
            }} catch (e) {{
                document.getElementById('source-code').innerHTML = '<code class="text-gray-500">// Error loading source</code>';
            }}
        }}

        function escapeHtml(text) {{
            if (!text) return '';
            return text
                .replace(/&/g, '&amp;')
                .replace(/</g, '&lt;')
                .replace(/>/g, '&gt;')
                .replace(/"/g, '&quot;')
                .replace(/'/g, '&#039;');
        }}

        function displaySource(data, highlightLine) {{
            let html = '';
            const start = Math.max(1, data.line - 5);
            const end = data.line + 5;
            for (let i = start; i <= end; i++) {{
                const isErrorLine = i === data.line;
                const lineNum = String(i).padStart(4, ' ');
                const lineClass = isErrorLine ? 'bg-red-500/20 text-red-300' : 'text-gray-400';
                const bgClass = isErrorLine ? 'bg-red-500/10' : '';
                const escapedLine = escapeHtml(data.lines[i] || '');
                html += '<tr class="' + bgClass + '"><td class="text-gray-600 select-none pr-2">' + lineNum + '</td><td class="' + lineClass + '"><pre style="margin:0;white-space:pre-wrap;">' + escapedLine + '</pre></td></tr>';
            }}
            document.getElementById('source-code').innerHTML = '<table class="w-full">' + html + '</table>';
        }}

        async function executeRepl() {{
            const input = document.getElementById('repl-input');
            let code = input.value.trim();
            if (!code) return;

            // Expand @ to lastResult (e.g., @ + 10 becomes (4) + 10)
            if (lastResult !== null) {{
                code = code.replace(/@/g, '(' + lastResult + ')');
            }}

            // Add to history
            if (code && history[history.length - 1] !== code) {{
                history.push(code);
            }}
            historyIndex = history.length;

            const output = document.getElementById('repl-output');
            output.innerHTML += '<div class="flex items-center gap-2 text-gray-400 mt-2"><div class="loading-spinner"></div><span>Executing...</span></div>';
            output.scrollTop = output.scrollHeight;

            try {{
                const response = await fetch('/__dev/repl', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{ code: code, request_data: currentRequestData, breakpoint_env: breakpointEnv }})
                }});
                const result = await response.json();
                if (result.error) {{
                    output.innerHTML += '<div class="text-red-400 mt-2">❌ ' + escapeHtml(result.error) + '</div>';
                    lastResult = null;
                }} else {{
                    output.innerHTML += '<div class="text-gray-300 mt-2"><span class="text-indigo-400">❯</span> <span class="text-gray-500">// ' + escapeHtml(input.value.trim()) + '</span></div>';
                    if (result.result && result.result !== "ok") {{
                        output.innerHTML += '<div class="text-green-400 mt-1">' + escapeHtml(result.result) + '</div>';
                        lastResult = result.result;
                    }} else {{
                        lastResult = null;
                    }}
                }}
            }} catch (e) {{
                output.innerHTML += '<div class="text-red-400 mt-2">❌ Error: ' + escapeHtml(e.message) + '</div>';
                lastResult = null;
            }}
            output.scrollTop = output.scrollHeight;
            input.value = '';
        }}

        // History and result variables
        let history = [];
        let historyIndex = -1;
        let lastResult = null;

        function navigateHistory(direction) {{
            const input = document.getElementById('repl-input');
            if (history.length === 0) return;

            if (direction === 'up') {{
                if (historyIndex > 0) {{
                    historyIndex--;
                    input.value = history[historyIndex];
                }}
            }} else if (direction === 'down') {{
                if (historyIndex < history.length - 1) {{
                    historyIndex++;
                    input.value = history[historyIndex];
                }} else {{
                    historyIndex = history.length;
                    input.value = '';
                }}
            }}
            // Move cursor to end
            const length = input.value.length;
            setTimeout(() => {{
                input.setSelectionRange(length, length);
            }}, 0);
        }}

        function quickInspect(expr) {{
            // Replace @ with lastResult
            let expanded = expr;
            if (lastResult !== null) {{
                expanded = expr.replace(/@/g, '(' + lastResult + ')');
            }}
            document.getElementById('repl-input').value = expanded;
            executeRepl();
        }}

        function clearRepl() {{
            document.getElementById('repl-output').innerHTML = '<div class="text-gray-500 italic">// REPL cleared.</div>';
            history = [];
            historyIndex = -1;
            lastResult = null;
        }}

        function escapeHtml(text) {{
            const div = document.createElement('div');
            div.textContent = text;
            return div.innerHTML;
        }}

        function formatJson(obj, indent = 0) {{
            const spaces = '  '.repeat(indent);
            const nextSpaces = '  '.repeat(indent + 1);

            if (obj === null) {{
                return '<span class="text-orange-400">null</span>';
            }}
            if (obj === undefined) {{
                return '<span class="text-gray-500">undefined</span>';
            }}
            if (typeof obj === 'boolean') {{
                return '<span class="text-orange-400">' + obj + '</span>';
            }}
            if (typeof obj === 'number') {{
                return '<span class="text-purple-400">' + obj + '</span>';
            }}
            if (typeof obj === 'string') {{
                return '<span class="text-green-400">"' + escapeHtml(obj) + '"</span>';
            }}
            if (Array.isArray(obj)) {{
                if (obj.length === 0) {{
                    return '<span class="text-gray-400">[]</span>';
                }}
                let result = '<span class="text-gray-400">[</span>\n';
                obj.forEach((item, i) => {{
                    result += nextSpaces + formatJson(item, indent + 1);
                    if (i < obj.length - 1) result += '<span class="text-gray-400">,</span>';
                    result += '\n';
                }});
                result += spaces + '<span class="text-gray-400">]</span>';
                return result;
            }}
            if (typeof obj === 'object') {{
                const keys = Object.keys(obj);
                if (keys.length === 0) {{
                    return '<span class="text-gray-400">{{}}</span>';
                }}
                let result = '<span class="text-gray-400">{{</span>\n';
                keys.forEach((key, i) => {{
                    result += nextSpaces + '<span class="text-indigo-300">"' + escapeHtml(key) + '"</span><span class="text-gray-400">:</span> ';
                    result += formatJson(obj[key], indent + 1);
                    if (i < keys.length - 1) result += '<span class="text-gray-400">,</span>';
                    result += '\n';
                }});
                result += spaces + '<span class="text-gray-400">}}</span>';
                return result;
            }}
            return '<span class="text-gray-400">' + escapeHtml(String(obj)) + '</span>';
        }}

        function initJsonDisplays() {{
            const displays = {{
                'json-params': currentRequestData.params,
                'json-query': currentRequestData.query,
                'json-body': currentRequestData.body,
                'json-headers': currentRequestData.headers,
                'json-session': currentRequestData.session
            }};

            for (const [id, data] of Object.entries(displays)) {{
                const el = document.getElementById(id);
                if (el) {{
                    if (data === null || data === undefined) {{
                        el.innerHTML = '<span class="text-gray-500 italic">No data</span>';
                    }} else if (typeof data === 'object' && Object.keys(data).length === 0) {{
                        el.innerHTML = '<span class="text-gray-500 italic">Empty</span>';
                    }} else {{
                        el.innerHTML = formatJson(data);
                    }}
                }}
            }}
        }}

        // Initialize JSON displays on page load
        initJsonDisplays();

        // Keyboard shortcuts
        document.addEventListener('keydown', function(e) {{
            // Ctrl+` to focus REPL
            if (e.ctrlKey && e.key === '`') {{
                e.preventDefault();
                document.getElementById('repl-input').focus();
            }}
            // Escape to clear selection
            if (e.key === 'Escape') {{
                document.querySelectorAll('.stack-frame').forEach(el => el.classList.remove('active'));
            }}
            // Arrow keys for history navigation
            if (e.target.id === 'repl-input') {{
                if (e.key === 'ArrowUp') {{
                    e.preventDefault();
                    navigateHistory('up');
                }} else if (e.key === 'ArrowDown') {{
                    e.preventDefault();
                    navigateHistory('down');
                }}
            }}
        }});

        // Auto-select the latest (last) stack frame on page load
        document.addEventListener('DOMContentLoaded', function() {{
            const frames = document.querySelectorAll('.stack-frame');
            if (frames.length > 0) {{
                frames[frames.length - 1].click();
            }}
            // Focus REPL input
            document.getElementById('repl-input').focus();
        }});
    </script>
</body>
</html>"#,
        error_type = error_type,
        error_message = error_message,
        error_location = error_location,
        stack_frames = stack_frames.join("\n"),
        request_data_json = request_data_json,
        breakpoint_env_js = breakpoint_env_json.unwrap_or("null"),
        request_method = escape_html(&request_method),
        request_path = escape_html(&request_path),
        request_time = request_time,
    )
}

/// Extract controller name from a file path
/// "./app/controllers/home_controller.sl" -> "home_controller"
fn extract_controller_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn get_source_file(file_path: &str, _line: usize) -> Option<HashMap<String, HashMap<usize, String>>> {
    let path = std::path::Path::new(file_path);
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let lines: HashMap<usize, String> = content
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.to_string()))
        .collect();

    Some([(file_path.to_string(), lines)].iter().cloned().collect())
}

fn render_production_error_page(status_code: u16, message: &str) -> String {
    let request_id = format!("{:08x}", rand::random::<u32>());

    // Try to render a custom error template first
    // Custom templates should be placed in app/views/errors/{status_code}.html.erb
    // Available context variables: status, message, request_id
    if let Some(custom_html) = crate::interpreter::builtins::template::render_error_template(
        status_code,
        message,
        &request_id,
    ) {
        return custom_html;
    }

    // Fall back to default error page
    let (title, heading, description, code_class) = match status_code {
        400 => (
            "400 Bad Request".to_string(),
            "Bad Request".to_string(),
            "The request could not be understood by the server due to malformed syntax.".to_string(),
            "warning".to_string(),
        ),
        403 => (
            "403 Forbidden".to_string(),
            "Forbidden".to_string(),
            "You don't have permission to access this resource.".to_string(),
            "warning".to_string(),
        ),
        404 => (
            "404 Not Found".to_string(),
            "Page Not Found".to_string(),
            "The page you're looking for doesn't exist or has been moved.".to_string(),
            "warning".to_string(),
        ),
        405 => (
            "405 Method Not Allowed".to_string(),
            "Method Not Allowed".to_string(),
            "The HTTP method used is not allowed for this resource.".to_string(),
            "warning".to_string(),
        ),
        500 => (
            "500 Internal Server Error".to_string(),
            "Internal Server Error".to_string(),
            "Something went wrong on our end. Please try again later.".to_string(),
            "error".to_string(),
        ),
        502 => (
            "502 Bad Gateway".to_string(),
            "Bad Gateway".to_string(),
            "The server received an invalid response from the upstream server.".to_string(),
            "error".to_string(),
        ),
        503 => (
            "503 Service Unavailable".to_string(),
            "Service Unavailable".to_string(),
            "The service is temporarily unavailable. Please try again later.".to_string(),
            "error".to_string(),
        ),
        _ => {
            let status_text = get_status_text(status_code).to_string();
            (
                format!("{} {}", status_code, status_text),
                status_text.clone(),
                "An error occurred while processing your request.".to_string(),
                "info".to_string(),
            )
        }
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{}</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background-color: #f8f9fa;
            color: #212529;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .container {{
            text-align: center;
            max-width: 500px;
        }}
        .error-code {{
            font-size: 120px;
            font-weight: 700;
            color: #e9ecef;
            line-height: 1;
            margin-bottom: 20px;
        }}
        .error-code.error {{ color: #f8d7da; }}
        .error-code.warning {{ color: #fff3cd; }}
        .error-code.info {{ color: #d1ecf1; }}
        h1 {{
            font-size: 28px;
            font-weight: 600;
            color: #343a40;
            margin-bottom: 12px;
        }}
        p {{
            font-size: 16px;
            color: #6c757d;
            line-height: 1.6;
            margin-bottom: 24px;
        }}
        .actions {{
            display: flex;
            gap: 12px;
            justify-content: center;
            flex-wrap: wrap;
        }}
        .btn {{
            display: inline-flex;
            align-items: center;
            padding: 12px 24px;
            font-size: 14px;
            font-weight: 500;
            text-decoration: none;
            border-radius: 6px;
            cursor: pointer;
            transition: all 0.2s ease;
        }}
        .btn-primary {{
            background-color: #007bff;
            color: white;
            border: none;
        }}
        .btn-primary:hover {{
            background-color: #0056b3;
        }}
        .btn-secondary {{
            background-color: transparent;
            color: #6c757d;
            border: 1px solid #dee2e6;
        }}
        .btn-secondary:hover {{
            background-color: #f8f9fa;
            border-color: #adb5bd;
        }}
        .error-details {{
            margin-top: 32px;
            padding-top: 24px;
            border-top: 1px solid #dee2e6;
            font-size: 12px;
            color: #adb5bd;
        }}
        .error-id {{
            font-family: monospace;
            background: #e9ecef;
            padding: 2px 6px;
            border-radius: 4px;
            font-size: 11px;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-code {code_class}">{status_code}</div>
        <h1>{heading}</h1>
        <p>{description}</p>
        <div class="actions">
            <a href="/" class="btn btn-primary">Go to Homepage</a>
            <button onclick="history.back()" class="btn btn-secondary">Go Back</button>
        </div>
        <div class="error-details">
            <p>If this problem persists, please contact support with the error ID below.</p>
            <p style="margin-top: 8px;">Error ID: <span class="error-id">{request_id}</span></p>
        </div>
    </div>
</body>
</html>"#,
        title = escape_html(&title),
        status_code = status_code,
        heading = escape_html(&heading),
        description = escape_html(&description),
        code_class = code_class,
        request_id = request_id,
    )
}

fn get_status_text(status_code: u16) -> &'static str {
    match status_code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}
