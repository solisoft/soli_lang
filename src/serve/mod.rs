//! MVC framework with convention-based routing and hot reload.
//!
//! This module implements a Rails-like MVC framework for Soli applications:
//! - Convention-based routing from controller filenames and function names
//! - Hot reload of changed files without server restart
//! - Automatic route derivation
//! - Middleware support for request interception

mod hot_reload;
pub mod live_reload;
mod live_reload_ws; // WebSocket-based live reload
mod middleware;
mod router;
mod server_constants;
pub mod websocket;

// Modularized subcomponents
pub(crate) mod app_loader;
pub mod engine_loader;
pub mod env_loader;
mod error_pages;
mod file_tracker;
mod file_upload;
mod json;
mod repl_session;
mod tailwind;
mod worker_pool;

pub use crate::interpreter::builtins::router::{get_controllers, set_controllers};
pub use hot_reload::FileTracker;
pub use middleware::{
    clear_middleware, extract_middleware_functions, extract_middleware_result, get_middleware,
    get_middleware_by_name, has_middleware, register_middleware, register_middleware_with_options,
    scan_middleware_files, with_middleware, Middleware, MiddlewareResult,
};
pub use router::{derive_routes_from_controller, ControllerRoute};
pub use websocket::{
    clear_websocket_routes, get_websocket_routes, match_websocket_route, register_websocket_route,
    restore_websocket_routes, take_websocket_routes, WebSocketConnection, WebSocketEvent,
    WebSocketHandlerAction, WebSocketRegistry,
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
    parse_form_urlencoded_body, parse_json_body, parse_query_string, routes_to_worker_routes,
    set_worker_routes, ParsedBody, WorkerRoute,
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
use crate::interpreter::builtins::controller::controller::ControllerInfo;
use crate::interpreter::builtins::controller::CONTROLLER_REGISTRY;
use crate::interpreter::builtins::session::{
    create_session_cookie, ensure_session, extract_session_id_from_cookie, get_current_session_id,
    session_cookie_if_changed, set_current_session_id,
};
use crate::interpreter::builtins::template::{clear_template_cache, init_templates};
use crate::interpreter::value::{HashKey, HashPairs};
use crate::interpreter::{Interpreter, Value};
use crate::live::socket::{extract_session_id as extract_live_session_id, handle_live_connection};
use crate::span::Span;

/// Uploaded file information
#[derive(Clone)]
pub struct UploadedFile {
    pub name: String,
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

// Import REPL session store from the dedicated module
use repl_session::REPL_STORE;

// Import worker pool structures
use worker_pool::{HotReloadVersions, WorkerQueues, WorkerSender};

/// Request data sent to interpreter thread
pub(crate) struct RequestData {
    pub(crate) method: Cow<'static, str>,
    pub(crate) path: String,
    pub(crate) query: HashMap<String, String>,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: String,
    /// Raw body bytes (for multipart parsing)
    #[allow(dead_code)]
    pub(crate) body_bytes: Option<Vec<u8>>,
    /// Pre-parsed form fields from multipart
    pub(crate) multipart_form: Option<HashMap<String, String>>,
    /// Pre-parsed files from multipart
    pub(crate) multipart_files: Option<Vec<UploadedFile>>,
    pub(crate) response_tx: oneshot::Sender<ResponseData>,
}

/// Response data from interpreter thread
#[derive(Clone)]
pub(crate) struct ResponseData {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

// File tracking functions (used by app_loader for initial file tracking in workers)
// The watcher thread now uses notify crate for event-driven file watching.

// File upload functions are now in file_upload module
use file_upload::uploaded_files_to_value;

/// Serve an MVC application from a folder in production mode by default.
pub fn serve_folder(folder: &Path, port: u16) -> Result<(), RuntimeError> {
    // Default to number of CPU cores for optimal parallelism
    let num_workers = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(server_constants::DEFAULT_WORKER_COUNT);
    serve_folder_with_options(folder, port, false, num_workers)
}

/// Serve an MVC application from a folder with configurable options.
pub fn serve_folder_with_options(
    folder: &Path,
    port: u16,
    dev_mode: bool,
    workers: usize,
) -> Result<(), RuntimeError> {
    serve_folder_with_options_and_workers(folder, port, dev_mode, workers)
}

// Environment loading functions are now in env_loader module
use env_loader::load_env_files;

/// Serve an MVC application from a folder with configurable options and worker count.
pub fn serve_folder_with_options_and_workers(
    folder: &Path,
    port: u16,
    dev_mode: bool,
    workers: usize,
) -> Result<(), RuntimeError> {
    // Load .env file before anything else
    load_env_files(folder);

    // Initialize DB config cache (must be after .env is loaded)
    crate::interpreter::builtins::model::init_db_config();

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

    // Set the app root for LiveView template resolution
    crate::live::component::set_app_root(folder.to_path_buf());

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

    // Load view helpers from app/helpers directory (only accessible in templates)
    let helpers_dir = app_dir.join("helpers");
    if helpers_dir.exists() {
        match crate::interpreter::builtins::template::load_view_helpers(&helpers_dir) {
            Ok(count) => {
                if count > 0 {
                    println!(
                        "Loaded {} view helper(s) from {}",
                        count,
                        helpers_dir.display()
                    );
                }
            }
            Err(e) => {
                eprintln!("Error loading view helpers: {}", e);
            }
        }
        // Track helper files for hot reload
        for entry in std::fs::read_dir(&helpers_dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "sl") {
                file_tracker.track(&path);
            }
        }
    }

    // Scan and load controllers
    let controller_files = scan_controllers(&controllers_dir)?;
    for controller_path in &controller_files {
        load_controller(&mut interpreter, controller_path, &mut file_tracker)?;
    }

    // Load engines (if config/engines.sl exists)
    match engine_loader::load_engines_config(folder) {
        Ok(config) => {
            if !config.engines.is_empty() {
                println!("Loading engines...");
                if let Err(e) = engine_loader::mount_engines(folder, &config) {
                    eprintln!("Warning: Failed to mount engines: {}", e);
                }
                // Load engine controllers and models
                if let Err(e) =
                    engine_loader::load_engine_controllers(&mut interpreter, &mut file_tracker)
                {
                    eprintln!("Warning: Failed to load engine controllers: {}", e);
                }
                if let Err(e) = engine_loader::load_engine_models(&mut interpreter) {
                    eprintln!("Warning: Failed to load engine models: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("Warning: Failed to load engine config: {}", e);
        }
    }

    // Track model files too
    if models_dir.exists() {
        for entry in std::fs::read_dir(&models_dir)
            .map_err(|e| RuntimeError::General {
                message: format!("Failed to read models directory: {}", e),
                span: Span::default(),
            })?
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "sl") {
                file_tracker.track(&path);
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
            fn resources(name: Any, block: Any = null) {
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

        // Load engine routes
        if let Err(e) = engine_loader::load_engine_routes(&mut interpreter) {
            eprintln!("Warning: Failed to load engine routes: {}", e);
        }

        // Rebuild route index to include engine routes
        crate::interpreter::builtins::server::rebuild_route_index();
    }

    // Public directory for static files
    let public_dir = folder.join("public");

    // Compile Tailwind CSS once at startup (not watch mode to avoid reload loops)
    if dev_mode {
        tailwind::compile_tailwind_css_once(folder);
    }

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
        workers,
        views_dir,
        routes_file,
    )
}

// Import app_loader functions
use app_loader::{
    define_routes_dsl, execute_file, load_controller, load_controllers_in_worker, load_middleware,
    load_models, reload_routes_in_worker, scan_controllers, track_view_files,
};

// Import tailwind functions

/// Run the MVC HTTP server with a worker pool for parallel request processing.
#[allow(clippy::too_many_arguments)]
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
    num_workers: usize,
    views_dir: PathBuf,
    routes_file: PathBuf,
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
    let capacity_per_worker = server_constants::CAPACITY_PER_WORKER;
    let (ws_event_tx, ws_event_rx) = channel::bounded(num_workers * capacity_per_worker);
    // LiveView event channel
    let (lv_event_tx, lv_event_rx): (
        channel::Sender<LiveViewEventData>,
        channel::Receiver<LiveViewEventData>,
    ) = channel::bounded(num_workers * capacity_per_worker);
    // crossbeam Sender is cheap to clone - no need for Arc<Mutex<Option<>>>
    // Use AtomicBool for shutdown signaling (lock-free check)
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_for_tokio = shutdown_flag.clone();

    // Per-worker queues eliminate receiver contention
    let worker_queues = Arc::new(WorkerQueues::new(num_workers, capacity_per_worker));
    let worker_queues_for_tokio = worker_queues.clone();

    // Channel to pass actual bound port from tokio thread to main thread
    let (bound_port_tx, bound_port_rx) = std::sync::mpsc::channel::<u16>();

    // Wrap public_dir in Arc for cheap cloning across connections
    let public_dir_arc = Arc::new(public_dir.clone());
    let ws_registry_for_tokio = ws_registry.clone();
    let dev_mode_for_tokio = dev_mode;

    // Channel to pass runtime handle from tokio thread to main thread
    let (runtime_handle_tx, runtime_handle_rx) =
        std::sync::mpsc::channel::<tokio::runtime::Handle>();

    // Spawn tokio runtime for HTTP server
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(num_workers)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        runtime.block_on(async move {
            // Send runtime handle to main thread for workers to use
            let _ = runtime_handle_tx.send(tokio::runtime::Handle::current());

            // Try the requested port, then scan for a free one
            let mut try_port = port;
            let listener = loop {
                let addr = SocketAddr::from(([0, 0, 0, 0], try_port));
                match TcpListener::bind(addr).await {
                    Ok(l) => break l,
                    Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                        if try_port == port {
                            eprintln!(
                                "Port {} is already in use, looking for a free port...",
                                port
                            );
                        }
                        try_port = try_port.checked_add(1).unwrap_or_else(|| {
                            eprintln!("No free port found");
                            std::process::exit(1);
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to bind: {}", e);
                        std::process::exit(1);
                    }
                }
            };
            let _ = bound_port_tx.send(try_port);

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
        let watch_controllers_dir = controllers_dir.clone();
        let watch_views_dir = views_dir.clone();
        let watch_middleware_dir = middleware_dir.clone();
        let watch_helpers_dir = helpers_dir.clone();
        let watch_models_dir = models_dir.clone();
        let watch_public_dir = public_dir.clone();
        let watch_routes_file = routes_file.clone();
        let watch_assets_css_dir = folder.join("app/assets/css");
        let watch_folder = folder.to_path_buf();
        let browser_reload_tx = reload_tx.clone();
        thread::spawn(move || {
            use notify::{RecursiveMode, Watcher};

            let (tx, rx) = std::sync::mpsc::channel();
            let mut watcher = match notify::recommended_watcher(tx) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("Hot reload: Failed to create file watcher: {}", e);
                    return;
                }
            };

            // Watch directories — handles new files automatically
            let mut watch_count = 0u32;
            if watch_controllers_dir.exists()
                && watcher
                    .watch(&watch_controllers_dir, RecursiveMode::NonRecursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if watch_middleware_dir.exists()
                && watcher
                    .watch(&watch_middleware_dir, RecursiveMode::NonRecursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if watch_helpers_dir.exists()
                && watcher
                    .watch(&watch_helpers_dir, RecursiveMode::NonRecursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if watch_models_dir.exists()
                && watcher
                    .watch(&watch_models_dir, RecursiveMode::NonRecursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if watch_views_dir.exists()
                && watcher
                    .watch(&watch_views_dir, RecursiveMode::Recursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if watch_public_dir.exists()
                && watcher
                    .watch(&watch_public_dir, RecursiveMode::Recursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if let Some(routes_parent) = watch_routes_file.parent() {
                if routes_parent.exists()
                    && watcher
                        .watch(routes_parent, RecursiveMode::NonRecursive)
                        .is_ok()
                {
                    watch_count += 1;
                }
            }
            if watch_assets_css_dir.exists()
                && watcher
                    .watch(&watch_assets_css_dir, RecursiveMode::NonRecursive)
                    .is_ok()
            {
                watch_count += 1;
            }

            println!(
                "Hot reload: Watching {} directories (event-driven)",
                watch_count
            );

            // Debounce: collect events over a short window before processing
            const DEBOUNCE_MS: u64 = 300;
            // Cooldown to prevent reload loops (e.g., when Tailwind rebuilds CSS after view changes)
            const RELOAD_COOLDOWN_MS: u64 = 2000;
            let mut last_reload_time: Option<Instant> = None;

            while let Ok(first) = rx.recv() {
                // Collect additional events that arrive within the debounce window
                let mut raw_events = vec![first];
                let deadline = Instant::now() + Duration::from_millis(DEBOUNCE_MS);
                loop {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    match rx.recv_timeout(remaining) {
                        Ok(ev) => raw_events.push(ev),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }

                // Extract unique paths from content-change events only.
                // Exclude metadata-only events (e.g. atime updates from reads)
                // which fire IN_ATTRIB on Linux when the server reads templates.
                let mut changed_paths = std::collections::HashSet::new();
                for event in raw_events.into_iter().flatten() {
                    use notify::EventKind;
                    match event.kind {
                        EventKind::Create(_)
                        | EventKind::Remove(_)
                        | EventKind::Modify(notify::event::ModifyKind::Data(_))
                        | EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                            for path in event.paths {
                                changed_paths.insert(path);
                            }
                        }
                        _ => {} // Ignore Access, Metadata, and Other events
                    }
                }

                // Filter to relevant extensions only
                let changed: Vec<PathBuf> = changed_paths
                    .into_iter()
                    .filter(|path| {
                        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                            matches!(ext, "sl" | "erb" | "slv")
                                || server_constants::is_tracked_static_extension(ext)
                        } else {
                            false
                        }
                    })
                    .collect();

                if changed.is_empty() {
                    continue;
                }

                println!("\n🔄 Hot reload triggered for:");
                let mut views_changed = false;
                let mut controllers_changed = false;
                let mut middleware_changed = false;
                let mut helpers_changed = false;
                let mut models_changed = false;
                let mut static_files_changed = false;
                let mut routes_changed = false;
                let mut asset_css_changed = false;

                // Track the public/css output directory to distinguish
                // Tailwind output changes from source changes
                let public_css_dir = watch_public_dir.join("css");

                for path in &changed {
                    println!("   {}", path.display());

                    // Check if it's a source CSS file in app/assets/css/
                    if path.starts_with(&watch_assets_css_dir) {
                        if path.extension().and_then(|e| e.to_str()) == Some("css") {
                            asset_css_changed = true;
                        }
                        continue; // Don't also count as static file
                    }

                    // Check if it's a static file (CSS, JS, images)
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if server_constants::is_tracked_static_extension(ext) {
                            // Ignore public/css/ changes caused by Tailwind output
                            // to avoid recompilation loops
                            if !(ext == "css" && path.starts_with(&public_css_dir)) {
                                static_files_changed = true;
                            }
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
                        } else if name.ends_with(".sl") && path.starts_with(&watch_models_dir) {
                            models_changed = true;
                        } else if name.ends_with(".erb")
                            || name.ends_with(".slv")
                            || name.ends_with(".md")
                        {
                            views_changed = true;
                        }
                    }
                }

                // Increment version counters - workers will pick this up
                if controllers_changed {
                    hot_reload_versions_for_watcher
                        .controllers
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled controller reload to all workers");
                }
                if middleware_changed {
                    hot_reload_versions_for_watcher
                        .middleware
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled middleware reload to all workers");
                }
                if helpers_changed {
                    hot_reload_versions_for_watcher
                        .helpers
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled view helpers reload to all workers");
                }
                if models_changed {
                    hot_reload_versions_for_watcher
                        .models
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled models reload to all workers");
                }
                if views_changed {
                    hot_reload_versions_for_watcher
                        .views
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled template cache clear to all workers");
                }

                // Recompile Tailwind CSS when source files change
                // (views may introduce new classes, asset CSS may have new directives)
                if views_changed || asset_css_changed || controllers_changed || helpers_changed {
                    tailwind::compile_tailwind_css_once(&watch_folder);
                }

                if static_files_changed && !views_changed && !asset_css_changed {
                    // Only signal static file reload when no views/asset CSS changed.
                    // When views or asset CSS change, Tailwind rebuilds CSS into public/
                    // which would trigger a redundant reload — the view reload already
                    // causes the browser to fetch updated CSS.
                    hot_reload_versions_for_watcher
                        .static_files
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled static file reload to all workers");
                }
                if routes_changed {
                    hot_reload_versions_for_watcher
                        .routes
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled routes reload to all workers");
                }

                // Notify browser for live reload (with cooldown to prevent loops)
                let should_reload = match last_reload_time {
                    Some(last_time) => {
                        let elapsed = Instant::now().duration_since(last_time);
                        elapsed.as_millis() as u64 >= RELOAD_COOLDOWN_MS
                    }
                    None => true,
                };

                if should_reload {
                    if let Some(ref tx) = browser_reload_tx {
                        let _ = tx.send(());
                    }
                    last_reload_time = Some(Instant::now());
                    println!(
                        "   -> Browser reload sent (cooldown: {}ms)",
                        RELOAD_COOLDOWN_MS
                    );
                } else {
                    let elapsed = Instant::now().duration_since(last_reload_time.unwrap());
                    println!(
                        "   -> Skipped reload (cooldown active: {}ms remaining)",
                        RELOAD_COOLDOWN_MS.saturating_sub(elapsed.as_millis() as u64)
                    );
                }

                println!();

                // Drain any events that arrived during processing to prevent
                // cascading reload loops (e.g. workers reading files can generate
                // inotify events on some Linux configurations).
                std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
                while rx.try_recv().is_ok() {}
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

    // Receive the actual bound port (may differ from requested if it was in use)
    let actual_port = bound_port_rx
        .recv()
        .expect("Failed to receive bound port from tokio thread");

    println!("\nServer listening on http://0.0.0.0:{}", actual_port);
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                println!("  Local network:    http://{}:{}", addr.ip(), actual_port);
            }
        }
    }
    if dev_mode {
        println!("Development mode - hot reload enabled, no caching");
        println!("  Edit models/controllers/middleware/views to see changes");
        println!("  Browsers will auto-refresh on changes");
    } else {
        println!("Production mode - caching enabled, no hot reload");
    }
    if public_dir.exists() {
        println!("Static files served from {}", public_dir.display());
    }
    println!(
        "Using hyper async HTTP server with {} worker threads\n",
        num_workers
    );

    // Eagerly initialize the shared HTTP client within the tokio runtime context.
    // reqwest::Client requires a Tokio reactor during construction.
    runtime_handle.block_on(async {
        crate::interpreter::builtins::http_class::get_http_client();
    });

    // Login to SoliDB once to get a JWT token (uses ureq, no tokio needed).
    // Must be after .env loading and DB config init.
    crate::interpreter::builtins::model::core::init_jwt_token();

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

            // Auto-restart loop: if the worker panics, recreate interpreter and resume
            loop {
                // Clone values for this iteration (cheap Arc/crossbeam clones)
                let work_rx = work_rx.clone();
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

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut interpreter = Interpreter::new_for_serve();

                    worker_loop(
                        i,
                        work_rx,
                        models_dir,
                        middleware_dir,
                        helpers_dir,
                        ws_event_rx,
                        lv_event_rx,
                        ws_registry,
                        reload_tx,
                        &mut interpreter,
                        worker_routes,
                        controllers_dir,
                        views_dir,
                        hot_reload_versions,
                        runtime_handle,
                        routes_file,
                        dev_mode,
                    );
                }));

                match result {
                    Ok(_) => break, // Normal exit
                    Err(_) => {
                        eprintln!("Worker {} panicked, restarting...", i);
                    }
                }
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
#[allow(clippy::too_many_arguments)]
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

    // Load middleware in this worker (needed for scoped middleware resolution by name)
    {
        let mut file_tracker = FileTracker::new();
        if let Err(e) = load_middleware(interpreter, &middleware_dir, &mut file_tracker) {
            eprintln!("Worker {}: Error loading middleware: {}", worker_id, e);
        }
    }

    // Load models in this worker so classes are defined in environment
    if let Err(e) = load_models(interpreter, &_models_dir) {
        eprintln!("Worker {}: Error loading models: {}", worker_id, e);
    }

    // Load controllers in this worker so functions are defined in environment
    load_controllers_in_worker(worker_id, interpreter, &controllers_dir);

    // Define DSL helpers for routes (needed for hot reload)
    if let Err(e) = define_routes_dsl(interpreter) {
        eprintln!("Worker {}: Error defining routes DSL: {}", worker_id, e);
    }

    let _worker_routes = get_routes();

    // Create VM for production mode (bytecode execution for handler calls)
    let mut vm: Option<crate::vm::Vm> = if !dev_mode {
        let mut vm = crate::vm::Vm::new();
        // Copy all globals from interpreter environment into VM
        // This includes all native builtins, classes, and user-defined functions
        let all_globals = interpreter.environment.borrow().get_all_bindings();
        for (name, value) in all_globals {
            vm.globals.insert(name, value);
        }
        Some(vm)
    } else {
        None
    };

    let mut ws_event_rx_inner = Some(ws_event_rx);
    let ws_registry_inner = Some(ws_registry);
    let mut lv_event_rx_inner = Some(lv_event_rx);

    // Track last seen hot reload versions
    let mut last_controllers_version = hot_reload_versions.controllers.load(Ordering::Acquire);
    let mut last_middleware_version = hot_reload_versions.middleware.load(Ordering::Acquire);
    let mut last_helpers_version = hot_reload_versions.helpers.load(Ordering::Acquire);
    let mut last_models_version = hot_reload_versions.models.load(Ordering::Acquire);
    let mut last_views_version = hot_reload_versions.views.load(Ordering::Acquire);
    let mut last_static_files_version = hot_reload_versions.static_files.load(Ordering::Acquire);
    let mut last_routes_version = hot_reload_versions.routes.load(Ordering::Acquire);

    loop {
        // Check for hot reload (lock-free version check)
        let current_controllers = hot_reload_versions.controllers.load(Ordering::Acquire);
        let current_middleware = hot_reload_versions.middleware.load(Ordering::Acquire);
        let current_helpers = hot_reload_versions.helpers.load(Ordering::Acquire);
        let current_models = hot_reload_versions.models.load(Ordering::Acquire);
        let current_views = hot_reload_versions.views.load(Ordering::Acquire);
        let current_static_files = hot_reload_versions.static_files.load(Ordering::Acquire);
        let current_routes = hot_reload_versions.routes.load(Ordering::Acquire);

        if current_controllers != last_controllers_version {
            last_controllers_version = current_controllers;
            // Re-load all controllers
            load_controllers_in_worker(worker_id, interpreter, &controllers_dir);
            // Re-define DSL helpers (controllers may shadow get/post/put/delete/patch)
            if let Err(e) = define_routes_dsl(interpreter) {
                eprintln!("Worker {}: Error redefining routes DSL: {}", worker_id, e);
            }
            // Update VM globals after controller reload
            if let Some(ref mut vm) = vm {
                let all_globals = interpreter.environment.borrow().get_all_bindings();
                for (name, value) in all_globals {
                    vm.globals.insert(name, value);
                }
            }
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
            if let Err(e) = crate::interpreter::builtins::template::load_view_helpers(&helpers_dir)
            {
                eprintln!("Worker {}: Error reloading view helpers: {}", worker_id, e);
            }
        }

        if current_models != last_models_version {
            last_models_version = current_models;
            // Re-load all models
            if let Err(e) = load_models(interpreter, &_models_dir) {
                eprintln!("Worker {}: Error reloading models: {}", worker_id, e);
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
            let mut file_tracker = FileTracker::new();
            reload_routes_in_worker(
                worker_id,
                interpreter,
                &routes_file,
                &controllers_dir,
                &mut file_tracker,
            );
        }

        // Drain all pending events non-blockingly before sleeping

        // Process WebSocket events (quick non-blocking check)
        if let (Some(ref mut rx), Some(_registry)) =
            (ws_event_rx_inner.as_mut(), ws_registry_inner.as_ref())
        {
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
                        track: None,
                        untrack: None,
                        set_presence: None,
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
        for _ in 0..server_constants::BATCH_SIZE {
            match work_rx.try_recv() {
                Ok(mut data) => {
                    let resp_data = handle_request(interpreter, &mut vm, &mut data, dev_mode);
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

        // Block waiting for events on any channel using crossbeam select.
        // This avoids busy-waiting: the thread sleeps until an event arrives
        // on any channel (or timeout fires for dev-mode hot reload checks).
        {
            let mut sel = channel::Select::new();
            let work_idx = sel.recv(&work_rx);
            let ws_idx = ws_event_rx_inner
                .as_ref()
                .filter(|_| ws_registry_inner.is_some())
                .map(|rx| sel.recv(rx));
            let lv_idx = lv_event_rx_inner.as_ref().map(|rx| sel.recv(rx));

            let result = if dev_mode {
                // Dev mode: use timeout so we periodically check hot reload versions
                sel.select_timeout(Duration::from_millis(200))
            } else {
                // Production: block indefinitely - no hot reload to check
                Ok(sel.select())
            };

            if let Ok(oper) = result {
                let idx = oper.index();
                if idx == work_idx {
                    if let Ok(mut data) = oper.recv(&work_rx) {
                        // Check hot reload before handling
                        if dev_mode {
                            let current_views = hot_reload_versions.views.load(Ordering::Acquire);
                            if current_views != last_views_version {
                                last_views_version = current_views;
                                clear_template_cache();
                            }
                        }
                        let resp_data = handle_request(interpreter, &mut vm, &mut data, dev_mode);
                        let _ = data.response_tx.send(resp_data);
                    }
                } else if Some(idx) == ws_idx {
                    if let Some(ref rx) = ws_event_rx_inner {
                        if let Ok(data) = oper.recv(rx) {
                            handle_websocket_event(interpreter, &data, &runtime_handle);
                            let _ = data.response_tx.send(WebSocketActionData {
                                join: None,
                                leave: None,
                                send: None,
                                broadcast: None,
                                broadcast_room: None,
                                close: None,
                                track: None,
                                untrack: None,
                                set_presence: None,
                            });
                        }
                    }
                } else if Some(idx) == lv_idx {
                    if let Some(ref rx) = lv_event_rx_inner {
                        if let Ok(data) = oper.recv(rx) {
                            let result = handle_liveview_event(interpreter, &data);
                            let _ = data.response_tx.send(result);
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
    track: Option<std::collections::HashMap<String, String>>,
    untrack: Option<String>,
    set_presence: Option<std::collections::HashMap<String, String>>,
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

// File upload functions are now in file_upload module
use file_upload::parse_multipart_body;

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
    let method: Cow<'static, str> = match *req.method() {
        hyper::Method::GET => Cow::Borrowed("GET"),
        hyper::Method::POST => Cow::Borrowed("POST"),
        hyper::Method::PUT => Cow::Borrowed("PUT"),
        hyper::Method::DELETE => Cow::Borrowed("DELETE"),
        hyper::Method::PATCH => Cow::Borrowed("PATCH"),
        hyper::Method::HEAD => Cow::Borrowed("HEAD"),
        hyper::Method::OPTIONS => Cow::Borrowed("OPTIONS"),
        _ => Cow::Owned(req.method().to_string().to_uppercase()),
    };
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
            let cookies = req
                .headers()
                .get("cookie")
                .map(|v| v.to_str().unwrap_or(""));
            let session_id = extract_live_session_id(cookies);

            // Perform the WebSocket upgrade
            let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, None) {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("[LiveView] Upgrade error: {}", e);
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Full::new(Bytes::from(format!(
                            "WebSocket upgrade error: {}",
                            e
                        ))))
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
                let (tx, rx) =
                    async_channel::bounded::<Result<tungstenite::Message, tungstenite::Error>>(32);
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
                                    if let Ok(parsed) =
                                        serde_json::from_str::<serde_json::Value>(text)
                                    {
                                        let event_type = parsed
                                            .get("type")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let liveview_id = parsed
                                            .get("liveview_id")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());
                                        let event_name = parsed
                                            .get("event")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());
                                        let params = parsed
                                            .get("params")
                                            .cloned()
                                            .unwrap_or(serde_json::json!({}));

                                        if event_type == "event" {
                                            if let Some(id) = liveview_id {
                                                if let Some(event) = event_name {
                                                    // Send event to worker thread for controller dispatch
                                                    let (response_tx, response_rx) =
                                                        oneshot::channel();
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
                                                            std::time::Duration::from_secs(server_constants::HEARTBEAT_TIMEOUT_SECS),
                                                            response_rx,
                                                        )
                                                        .await
                                                        {
                                                            Ok(Ok(Ok(()))) => {
                                                                // Event handled successfully
                                                            }
                                                            Ok(Ok(Err(e))) => {
                                                                eprintln!(
                                                                    "[LiveView] Event error: {}",
                                                                    e
                                                                );
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
                                            // Send heartbeat acknowledgment (fire-and-forget)
                                            let ack = serde_json::json!({
                                                "type": "heartbeat_ack"
                                            });
                                            #[allow(clippy::let_underscore_future)]
                                            let _ = tx_arc.send(Ok(tungstenite::Message::Text(
                                                ack.to_string(),
                                            )));
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
        match resolve_static_file(&path, &public_dir) {
            Err(()) => {
                return Ok(Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Full::new(Bytes::from("Forbidden")))
                    .unwrap());
            }
            Ok(Some(file_path)) => {
                let mime_type = server_constants::get_mime_type(&file_path);

                // In production mode, check for conditional request (If-None-Match)
                if !dev_mode {
                    if let Ok(metadata) = std::fs::metadata(&file_path) {
                        if let Ok(modified) = metadata.modified() {
                            let etag = server_constants::generate_etag(modified);

                            // Check If-None-Match header
                            if let Some(if_none_match) = req.headers().get("if-none-match") {
                                if let Ok(client_etag) = if_none_match.to_str() {
                                    // ETags match - return 304 Not Modified (skip file read!)
                                    if client_etag == etag || client_etag == format!("W/{}", etag) {
                                        return Ok(Response::builder()
                                            .status(StatusCode::NOT_MODIFIED)
                                            .header("ETag", &etag)
                                            .header(
                                                "Cache-Control",
                                                server_constants::STATIC_CACHE_MAX_AGE,
                                            )
                                            .body(Full::new(Bytes::new()))
                                            .unwrap());
                                    }
                                }
                            }

                            let file_size = metadata.len();

                            // Check for Range request
                            if let Some(range_header) = req.headers().get("range") {
                                if let Ok(range_str) = range_header.to_str() {
                                    if let Some((start, end)) =
                                        server_constants::parse_range_header(range_str, file_size)
                                    {
                                        let length = end - start + 1;
                                        // Read only the requested range
                                        let content = match std::fs::read(&file_path) {
                                            Ok(c) => c,
                                            Err(_) => {
                                                return Ok(Response::builder()
                                                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                                                    .body(Full::new(Bytes::from(
                                                        "Error reading file",
                                                    )))
                                                    .unwrap())
                                            }
                                        };
                                        let slice = &content[start as usize
                                            ..=(end as usize).min(content.len() - 1)];
                                        return Ok(Response::builder()
                                            .status(StatusCode::PARTIAL_CONTENT)
                                            .header("Content-Type", mime_type)
                                            .header(
                                                "Content-Range",
                                                format!("bytes {}-{}/{}", start, end, file_size),
                                            )
                                            .header("Content-Length", length.to_string())
                                            .header("Accept-Ranges", "bytes")
                                            .header("ETag", &etag)
                                            .header(
                                                "Cache-Control",
                                                server_constants::STATIC_CACHE_MAX_AGE,
                                            )
                                            .body(Full::new(Bytes::copy_from_slice(slice)))
                                            .unwrap());
                                    } else {
                                        // Range not satisfiable
                                        return Ok(Response::builder()
                                            .status(StatusCode::RANGE_NOT_SATISFIABLE)
                                            .header(
                                                "Content-Range",
                                                format!("bytes */{}", file_size),
                                            )
                                            .body(Full::new(Bytes::new()))
                                            .unwrap());
                                    }
                                }
                            }

                            // No Range header - serve full file
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
                                .header("Content-Length", content.len().to_string())
                                .header("Accept-Ranges", "bytes")
                                .header("ETag", etag)
                                .header("Cache-Control", server_constants::STATIC_CACHE_MAX_AGE)
                                .body(Full::new(Bytes::from(content)))
                                .unwrap());
                        }
                    }
                }

                // Dev mode or metadata unavailable
                let content = match std::fs::read(&file_path) {
                    Ok(c) => c,
                    Err(_) => {
                        return Ok(Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Full::new(Bytes::from("Error reading file")))
                            .unwrap())
                    }
                };

                let file_size = content.len() as u64;

                // Check for Range request in dev mode too
                if let Some(range_header) = req.headers().get("range") {
                    if let Ok(range_str) = range_header.to_str() {
                        if let Some((start, end)) =
                            server_constants::parse_range_header(range_str, file_size)
                        {
                            let length = end - start + 1;
                            let slice =
                                &content[start as usize..=(end as usize).min(content.len() - 1)];
                            return Ok(Response::builder()
                                .status(StatusCode::PARTIAL_CONTENT)
                                .header("Content-Type", mime_type)
                                .header(
                                    "Content-Range",
                                    format!("bytes {}-{}/{}", start, end, file_size),
                                )
                                .header("Content-Length", length.to_string())
                                .header("Accept-Ranges", "bytes")
                                .body(Full::new(Bytes::copy_from_slice(slice)))
                                .unwrap());
                        } else {
                            return Ok(Response::builder()
                                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                                .header("Content-Range", format!("bytes */{}", file_size))
                                .body(Full::new(Bytes::new()))
                                .unwrap());
                        }
                    }
                }

                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", mime_type)
                    .header("Content-Length", content.len().to_string())
                    .header("Accept-Ranges", "bytes")
                    .body(Full::new(Bytes::from(content)))
                    .unwrap());
            }
            Ok(None) => {} // Not a static file, fall through to route matching
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

    // Extract headers (pre-allocate, use as_str to avoid Display formatting overhead)
    let mut headers = HashMap::with_capacity(req.headers().keys_len());
    for (name, value) in req.headers() {
        if let Ok(v) = value.to_str() {
            headers.insert(name.as_str().to_owned(), v.to_owned());
        }
    }

    // Read body - skip for GET/HEAD requests (usually empty)
    let (body, body_bytes_opt, multipart_form, multipart_files) =
        if method == "GET" || method == "HEAD" {
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
        method,
        path,
        query,
        headers,
        body,
        body_bytes: body_bytes_opt,
        multipart_form,
        multipart_files,
        response_tx,
    };

    // Non-blocking send: use try_send + async yield to avoid blocking tokio threads.
    // Blocking send() here would deadlock under high concurrency because:
    // - Full queues block tokio worker threads on send()
    // - Workers' Handle::block_on() futures need the tokio I/O driver to complete
    // - Blocked tokio threads can't drive the I/O driver → permanent deadlock
    let mut pending_data = Some(request_data);
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(server_constants::REQUEST_TIMEOUT_SECS);
    let send_ok = loop {
        if let Some(data) = pending_data.take() {
            match request_tx.try_send(data) {
                Ok(()) => break true,
                Err(crossbeam::channel::TrySendError::Full(returned)) => {
                    if tokio::time::Instant::now() >= deadline {
                        break false;
                    }
                    pending_data = Some(returned);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(crossbeam::channel::TrySendError::Disconnected(_)) => {
                    break false;
                }
            }
        }
    };

    if !send_ok {
        return Ok(Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Full::new(Bytes::from("Server busy")))
            .unwrap());
    }

    // Wait for response
    match response_rx.await {
        Ok(resp_data) => {
            let mut builder = Response::builder()
                .status(StatusCode::from_u16(resp_data.status).unwrap_or(StatusCode::OK))
                .header("Server", "soliMVC");

            for (key, value) in &resp_data.headers {
                builder = builder.header(key.as_str(), value.as_str());
            }

            // Inject live reload script for HTML responses (only in dev mode)
            let body = if reload_tx.is_some() {
                let is_html = resp_data.headers.iter().any(|(k, v)| {
                    k.eq_ignore_ascii_case("content-type") && v.contains("text/html")
                });
                if is_html {
                    live_reload::inject_live_reload_script(&resp_data.body)
                } else {
                    resp_data.body
                }
            } else {
                resp_data.body
            };

            Ok(builder.body(Full::new(Bytes::from(body))).unwrap())
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
                .body(Full::new(Bytes::from(format!(
                    "WebSocket upgrade error: {}",
                    e
                ))))
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
        let (ws_tx, mut ws_rx) =
            tokio::sync::mpsc::channel::<Result<tungstenite::Message, tungstenite::Error>>(32);
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

    // Silently ignore send errors
    let _ = ws_event_tx.send(connect_event);

    // Wait for handler response (don't block forever, max 5 seconds)
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(server_constants::REQUEST_TIMEOUT_SECS),
        response_rx,
    )
    .await;

    // Send ping to client
    let _ = stream.send(tungstenite::Message::Ping(vec![])).await;

    // Create a loop to handle both incoming messages and outgoing messages
    let (mut ws_sender, mut ws_receiver) = stream.split();

    // Forward messages from ws_rx to the WebSocket
    let forward_task = async {
        while let Some(msg) = ws_rx.recv().await {
            if let Err(e) = ws_sender
                .send(msg.unwrap_or(tungstenite::Message::Close(None)))
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
                            std::time::Duration::from_secs(server_constants::REQUEST_TIMEOUT_SECS),
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
fn handle_websocket_event(
    interpreter: &mut Interpreter,
    data: &WebSocketEventData,
    runtime_handle: &tokio::runtime::Handle,
) {
    use crate::interpreter::value::Value;
    use crate::serve::websocket::{
        PresenceDiff, UserPresencePayload, WebSocketHandlerAction, WebSocketRegistry,
    };

    // Clone connection_id for use in async spawns
    let connection_id = data.connection_id;
    let connection_id_str = connection_id.to_string();

    // Auto-untrack all presences on disconnect
    if data.event_type == "disconnect" {
        let registry = crate::serve::websocket::get_ws_registry();
        let registry_clone = registry.clone();
        runtime_handle.spawn(async move {
            // Untrack all presences for this connection
            let untracked = registry_clone.untrack_all(&connection_id).await;

            // Broadcast leave diffs for users whose last connection just left
            for (channel, user_id, was_last, meta) in untracked {
                if was_last && !user_id.is_empty() {
                    let mut leaves = std::collections::HashMap::new();
                    leaves.insert(user_id, UserPresencePayload { metas: vec![meta] });
                    let diff = PresenceDiff {
                        joins: std::collections::HashMap::new(),
                        leaves,
                    };
                    let diff_msg = WebSocketRegistry::build_presence_diff(&diff);
                    registry_clone
                        .broadcast_to_channel(&channel, &diff_msg)
                        .await;
                }
            }
        });
    }

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
                let action_name = route
                    .handler_name
                    .split('#')
                    .next_back()
                    .unwrap_or(&route.handler_name);
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
    let mut event_map: HashPairs = HashPairs::default();
    event_map.insert(
        HashKey::String("type".to_string()),
        Value::String(data.event_type.clone()),
    );
    event_map.insert(
        HashKey::String("connection_id".to_string()),
        Value::String(connection_id_str.clone()),
    );

    if let Some(ref msg) = data.message {
        event_map.insert(
            HashKey::String("message".to_string()),
            Value::String(msg.clone()),
        );
    }

    if let Some(ref channel) = data.channel {
        event_map.insert(
            HashKey::String("channel".to_string()),
            Value::String(channel.clone()),
        );
    }

    let event_value = Value::Hash(Rc::new(RefCell::new(event_map)));

    // Call the handler function
    match interpreter.call_value(handler, vec![event_value], Span::default()) {
        Ok(result) => {
            // Parse the handler result into actions
            let action = WebSocketHandlerAction::from_value(&result);
            let registry = crate::serve::websocket::get_ws_registry();

            // Process join action
            if let Some(ref channel) = action.join {
                let registry_clone = registry.clone();
                let channel_clone = channel.clone();
                runtime_handle.spawn(async move {
                    registry_clone
                        .join_channel(&connection_id, &channel_clone)
                        .await;
                });
            }

            // Process leave action
            if let Some(ref channel) = action.leave {
                let registry_clone = registry.clone();
                let channel_clone = channel.clone();
                runtime_handle.spawn(async move {
                    registry_clone
                        .leave_channel(&connection_id, &channel_clone)
                        .await;
                });
            }

            // Process broadcast action
            if let Some(ref msg) = action.broadcast {
                let registry_clone = registry.clone();
                let msg_clone = msg.clone();
                runtime_handle.spawn(async move {
                    registry_clone.broadcast_all(&msg_clone).await;
                });
            }

            // Process send action
            if let Some(ref msg) = action.send {
                let registry_clone = registry.clone();
                let msg_clone = msg.clone();
                runtime_handle.spawn(async move {
                    registry_clone
                        .send_to(&connection_id, &msg_clone)
                        .await
                        .ok();
                });
            }

            // Process broadcast_room action
            if let Some(ref msg) = action.broadcast_room {
                // broadcast_room expects format "channel:message" or just message to all joined channels
                // For simplicity, we'll broadcast to all channels this connection has joined
                if let Some(ref join_channel) = action.join {
                    let registry_clone = registry.clone();
                    let channel_clone = join_channel.clone();
                    let msg_clone = msg.clone();
                    runtime_handle.spawn(async move {
                        registry_clone
                            .broadcast_to_channel(&channel_clone, &msg_clone)
                            .await;
                    });
                }
            }

            // Process close action
            if let Some(ref reason) = action.close {
                let registry_clone = registry.clone();
                let reason_clone = reason.clone();
                runtime_handle.spawn(async move {
                    registry_clone.close(&connection_id, &reason_clone).await;
                });
            }

            // Process track action (presence tracking)
            if let Some(ref track_meta) = action.track {
                let channel = track_meta.get("channel").cloned();
                let user_id = track_meta.get("user_id").cloned();

                if let (Some(channel), Some(user_id)) = (channel, user_id) {
                    let registry_clone = registry.clone();
                    let meta = track_meta.clone();
                    runtime_handle.spawn(async move {
                        let (is_new_user, presence_meta) = registry_clone
                            .track(&connection_id, &channel, &user_id, meta)
                            .await;

                        // Get full presence state for the joining connection
                        let presences = registry_clone.list_presence(&channel).await;
                        let state_msg = WebSocketRegistry::build_presence_state(&presences);

                        // Send full presence_state to the joining connection
                        let _ = registry_clone.send_to(&connection_id, &state_msg).await;

                        // If this is a new user (not just another tab), broadcast presence_diff
                        if is_new_user {
                            let mut joins = std::collections::HashMap::new();
                            joins.insert(
                                user_id.clone(),
                                UserPresencePayload {
                                    metas: vec![presence_meta],
                                },
                            );
                            let diff = PresenceDiff {
                                joins,
                                leaves: std::collections::HashMap::new(),
                            };
                            let diff_msg = WebSocketRegistry::build_presence_diff(&diff);

                            // Broadcast to all in channel except the joining connection
                            registry_clone
                                .broadcast_to_channel_except(&channel, &diff_msg, &connection_id)
                                .await;
                        }
                    });
                }
            }

            // Process untrack action
            if let Some(ref channel) = action.untrack {
                let registry_clone = registry.clone();
                let channel_clone = channel.clone();
                runtime_handle.spawn(async move {
                    if let Some((was_last, meta)) =
                        registry_clone.untrack(&connection_id, &channel_clone).await
                    {
                        // If this was the last connection for this user, broadcast leave diff
                        if was_last {
                            // Find user_id from the meta's extra field
                            let user_id = meta.extra.get("user_id").cloned().unwrap_or_default();
                            if !user_id.is_empty() {
                                let mut leaves = std::collections::HashMap::new();
                                leaves.insert(user_id, UserPresencePayload { metas: vec![meta] });
                                let diff = PresenceDiff {
                                    joins: std::collections::HashMap::new(),
                                    leaves,
                                };
                                let diff_msg = WebSocketRegistry::build_presence_diff(&diff);
                                registry_clone
                                    .broadcast_to_channel(&channel_clone, &diff_msg)
                                    .await;
                            }
                        }
                    }
                });
            }

            // Process set_presence action
            if let Some(ref presence_data) = action.set_presence {
                let channel = presence_data.get("channel").cloned();
                let state = presence_data.get("state").cloned();

                if let (Some(channel), Some(state)) = (channel, state) {
                    let registry_clone = registry.clone();
                    runtime_handle.spawn(async move {
                        if let Some(updated_meta) = registry_clone
                            .set_presence(&connection_id, &channel, &state)
                            .await
                        {
                            // Find user_id for the diff
                            let user_id = updated_meta
                                .extra
                                .get("user_id")
                                .cloned()
                                .unwrap_or_default();
                            if !user_id.is_empty() {
                                // Broadcast presence_diff with the updated meta
                                let mut joins = std::collections::HashMap::new();
                                joins.insert(
                                    user_id,
                                    UserPresencePayload {
                                        metas: vec![updated_meta],
                                    },
                                );
                                let diff = PresenceDiff {
                                    joins,
                                    leaves: std::collections::HashMap::new(),
                                };
                                let diff_msg = WebSocketRegistry::build_presence_diff(&diff);
                                registry_clone
                                    .broadcast_to_channel(&channel, &diff_msg)
                                    .await;
                            }
                        }
                    });
                }
            }
        }
        Err(e) => {
            eprintln!("[WS] Handler error: {}", e);
        }
    }
}

/// Handle a LiveView event by calling the controller handler.
fn handle_liveview_event(
    interpreter: &mut Interpreter,
    data: &LiveViewEventData,
) -> Result<(), String> {
    use crate::interpreter::value::Value;
    use crate::live::view::LIVE_REGISTRY;

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

    let mut event_map: HashPairs = HashPairs::default();
    event_map.insert(
        HashKey::String("event".to_string()),
        Value::String(data.event.clone()),
    );
    event_map.insert(HashKey::String("params".to_string()), params_value);
    event_map.insert(HashKey::String("state".to_string()), state_value);
    let event_value = Value::Hash(Rc::new(RefCell::new(event_map)));

    // If we have a registered handler, call it
    if let Some(handler_name) = handler_name {
        // Try to resolve the handler from the controller registry
        let handler =
            match crate::interpreter::builtins::router::resolve_handler(&handler_name, None) {
                Ok(h) => h,
                Err(_) => {
                    // Try to look up the function directly in the environment
                    let action_name = handler_name.split('#').next_back().unwrap_or(&handler_name);
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
                        if let (
                            serde_json::Value::Object(old),
                            serde_json::Value::Object(new_obj),
                        ) = (&instance.state, &mut state)
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
        _ => {
            return Err(format!(
                "Unknown event: {} for component {}",
                data.event, component
            ))
        }
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

/// Convert serde_json::Value reference to interpreter Value
fn json_to_value(json: &serde_json::Value) -> Value {
    json::json_to_value(json)
}

/// Convert interpreter Value to serde_json::Value
fn value_to_json(value: &Value) -> serde_json::Value {
    json::value_to_json(value)
}

/// Call the route handler with the request hash.
fn call_handler(
    interpreter: &mut Interpreter,
    mut vm: Option<&mut crate::vm::Vm>,
    handler_name: &str,
    request_hash: Value,
    dev_mode: bool,
    request_data: &RequestData,
) -> ResponseData {
    // Expose req["all"] as global `params` so handlers/views can reference it directly.
    let params_value = get_hash_field(&request_hash, "all").unwrap_or(Value::Null);
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("params", params_value.clone());
    if let Some(vm_ref) = vm.as_deref_mut() {
        vm_ref.globals.insert("params".to_string(), params_value);
    }

    // Check if this is an OOP controller action (contains #)
    if handler_name.contains('#') {
        let oop_result = call_oop_controller_action(
            interpreter,
            vm.as_deref_mut(),
            handler_name,
            &request_hash,
            dev_mode,
            request_data,
        );
        if let Some(response) = oop_result {
            return response;
        }
        // If not an OOP controller or error, fall through to function-based handling
    }

    let handler_result = crate::interpreter::builtins::router::resolve_handler(handler_name, None);

    // Try VM execution in production mode for function-based handlers
    if let Some(ref mut vm) = vm {
        if !vm.failed_handlers.contains(handler_name) {
            if let Ok(ref handler_value) = handler_result {
                match vm.call_value_direct_one(
                    handler_value.clone(),
                    request_hash.clone(),
                    Span::default(),
                ) {
                    Ok(result) => {
                        vm.reset();
                        let (status, headers, body) = extract_response(result);
                        return ResponseData {
                            status,
                            headers,
                            body,
                        };
                    }
                    Err(_) => {
                        vm.failed_handlers.insert(handler_name.to_string());
                        vm.reset();
                    }
                }
            }
        }
    }

    // Push stack frame for the handler (source path will be set from the function when called)
    interpreter.push_frame(handler_name, crate::span::Span::new(0, 0, 1, 1), None);

    match handler_result {
        Ok(handler_value) => {
            match interpreter.call_value(handler_value, vec![request_hash], Span::default()) {
                Ok(result) => {
                    interpreter.pop_frame();
                    let (status, headers, body) = extract_response(result);
                    ResponseData {
                        status,
                        headers,
                        body,
                    }
                }
                Err(e) => {
                    // Capture environment BEFORE popping frame so we can see local variables
                    let captured_env = if dev_mode && e.breakpoint_env_json().is_none() {
                        Some(interpreter.serialize_environment_for_debug())
                    } else {
                        None
                    };
                    interpreter.pop_frame();
                    if dev_mode {
                        // Use captured stack trace from error if available, otherwise get from interpreter
                        let stack_trace: Vec<String> = e
                            .breakpoint_stack_trace()
                            .map(|st| st.to_vec())
                            .unwrap_or_else(|| interpreter.get_stack_trace());
                        let breakpoint_env = e
                            .breakpoint_env_json()
                            .map(|s| s.to_string())
                            .or(captured_env);
                        let error_html = error_pages::render_error_page(
                            &e.to_string(),
                            interpreter,
                            request_data,
                            &stack_trace,
                            breakpoint_env.as_deref(),
                        );
                        ResponseData {
                            status: if e.is_breakpoint() { 200 } else { 500 },
                            headers: vec![(
                                "Content-Type".to_string(),
                                "text/html; charset=utf-8".to_string(),
                            )],
                            body: error_html,
                        }
                    } else {
                        let request_id = Uuid::new_v4().to_string();
                        let error_msg = e.to_string();
                        eprintln!(
                            "[ERROR] request_id={} {} {} - {}",
                            request_id, request_data.method, request_data.path, error_msg
                        );
                        let error_html =
                            error_pages::render_production_error_page(500, &error_msg, &request_id);
                        ResponseData {
                            status: 500,
                            headers: vec![(
                                "Content-Type".to_string(),
                                "text/html; charset=utf-8".to_string(),
                            )],
                            body: error_html,
                        }
                    }
                }
            }
        }
        Err(e) => {
            let captured_env = if dev_mode {
                Some(interpreter.serialize_environment_for_debug())
            } else {
                None
            };
            interpreter.pop_frame();
            if dev_mode {
                // This error is a String from resolve_handler, no captured stack trace
                let stack_trace = interpreter.get_stack_trace();
                let error_html = error_pages::render_error_page(
                    &e.to_string(),
                    interpreter,
                    request_data,
                    &stack_trace,
                    captured_env.as_deref(),
                );
                ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html,
                }
            } else {
                let request_id = Uuid::new_v4().to_string();
                let error_msg = e.to_string();
                eprintln!(
                    "[ERROR] request_id={} {} {} - {}",
                    request_id, request_data.method, request_data.path, error_msg
                );
                let error_html =
                    error_pages::render_production_error_page(500, &error_msg, &request_id);
                ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html,
                }
            }
        }
    }
}

// Thread-local cache of controllers that have before/after action hooks.
// None means not yet initialized; Some(set) means we've checked the registry.
thread_local! {
    static CONTROLLERS_WITH_HOOKS: RefCell<Option<std::collections::HashSet<String>>> = const { RefCell::new(None) };
}

/// Check if a controller has hooks, using thread-local cache to avoid RwLock reads.
fn controller_has_hooks(controller_key: &str) -> bool {
    CONTROLLERS_WITH_HOOKS.with(|cache| {
        let cached = cache.borrow();
        if let Some(ref set) = *cached {
            return set.contains(controller_key);
        }
        drop(cached);

        // Build cache from registry (once per thread)
        let registry = CONTROLLER_REGISTRY.read().unwrap();
        let mut set = std::collections::HashSet::new();
        for (key, info) in registry.all().iter().map(|i| (&i.class_name, i)) {
            if !info.before_actions.is_empty() || !info.after_actions.is_empty() {
                set.insert(key.clone());
            }
        }
        let has_hooks = set.contains(controller_key);
        *cache.borrow_mut() = Some(set);
        has_hooks
    })
}

/// Call an OOP controller action (controller#action).
/// Returns Some(ResponseData) if handled, None if not an OOP controller.
fn call_oop_controller_action(
    interpreter: &mut Interpreter,
    vm: Option<&mut crate::vm::Vm>,
    handler_name: &str,
    request_hash: &Value,
    dev_mode: bool,
    request_data: &RequestData,
) -> Option<ResponseData> {
    let (controller_key, action_name) = handler_name.split_once('#')?;

    // Check if this is an OOP controller (has a class definition)
    // Convert controller_key (e.g., "posts") to PascalCase class name (e.g., "PostsController")
    let class_name = to_pascal_case_controller(controller_key);

    // Look up the class in the environment
    let class_value = match interpreter.environment.borrow().get(&class_name) {
        Some(v) => v,
        None => {
            return None;
        }
    };

    // Check if it's actually a class
    let class_rc = match class_value {
        Value::Class(class_rc) => class_rc,
        _ => return None,
    };

    // Only read controller info from registry if controller has hooks (avoids RwLock per request)
    let controller_info = if controller_has_hooks(controller_key) {
        let registry = CONTROLLER_REGISTRY.read().unwrap();
        registry.get(controller_key).cloned()
    } else {
        None
    };

    // Extract request components - pass by reference where possible
    let params = get_hash_field(request_hash, "params").unwrap_or(Value::Null);
    let session = get_hash_field(request_hash, "session").unwrap_or(Value::Null);
    let headers = get_hash_field(request_hash, "headers").unwrap_or(Value::Null);

    // Execute before_action hooks (if controller info exists)
    if let Some(ref info) = controller_info {
        if let Some(before_response) = execute_before_actions(
            interpreter,
            info,
            action_name,
            request_hash.clone(),
            &params,
            &session,
            &headers,
        ) {
            return Some(before_response);
        }
    }

    // Instantiate the controller
    let controller_instance = match create_controller_instance(&class_name, interpreter) {
        Ok(inst) => inst,
        Err(e) => {
            return Some(if dev_mode {
                let stack_trace = interpreter.get_stack_trace();
                let error_html = error_pages::render_error_page(
                    &e,
                    interpreter,
                    request_data,
                    &stack_trace,
                    None,
                );
                ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html,
                }
            } else {
                let request_id = Uuid::new_v4().to_string();
                let error_msg = e.to_string();
                eprintln!(
                    "[ERROR] request_id={} {} {} - {}",
                    request_id, request_data.method, request_data.path, error_msg
                );
                let error_html =
                    error_pages::render_production_error_page(500, &error_msg, &request_id);
                ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html,
                }
            });
        }
    };

    // Set up controller context (req, params, session, headers)
    setup_controller_context(
        &controller_instance,
        request_hash,
        &params,
        &session,
        &headers,
    );

    // Call the action method on the class
    // For OOP controllers, the method is inside the class, not in the global environment
    let action_result = call_class_method(
        interpreter,
        vm,
        &class_rc,
        &controller_instance,
        action_name,
        request_hash,
    );

    let response = match action_result {
        Ok(result) => {
            let (status, resp_headers, body) = extract_response(result);
            ResponseData {
                status,
                headers: resp_headers,
                body,
            }
        }
        Err(e) => {
            if dev_mode {
                // Use breakpoint's captured stack trace if available, otherwise get current
                let stack_trace: Vec<String> = e
                    .breakpoint_stack_trace()
                    .map(|st| st.to_vec())
                    .unwrap_or_else(|| interpreter.get_stack_trace());
                let breakpoint_env = e.breakpoint_env_json();
                let error_html = error_pages::render_error_page(
                    &e.to_string(),
                    interpreter,
                    request_data,
                    &stack_trace,
                    breakpoint_env,
                );
                ResponseData {
                    status: if e.is_breakpoint() { 200 } else { 500 },
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html,
                }
            } else {
                let request_id = Uuid::new_v4().to_string();
                let error_msg = e.to_string();
                eprintln!(
                    "[ERROR] request_id={} {} {} - {}",
                    request_id, request_data.method, request_data.path, error_msg
                );
                let error_html =
                    error_pages::render_production_error_page(500, &error_msg, &request_id);
                ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html,
                }
            }
        }
    };

    // Execute after_action hooks (if controller info exists)
    if let Some(ref info) = controller_info {
        return Some(execute_after_actions(
            interpreter,
            info,
            action_name,
            request_hash.clone(),
            &response,
        ));
    }

    Some(response)
}

/// Call a method on a class instance.
fn call_class_method(
    interpreter: &mut Interpreter,
    vm: Option<&mut crate::vm::Vm>,
    class: &Rc<crate::interpreter::value::Class>,
    instance: &Value,
    method_name: &str,
    request_hash: &Value,
) -> Result<Value, RuntimeError> {
    // Look up the method in the class (walks inheritance chain)
    if let Some(method) = class.find_method(method_name) {
        let method_span = method
            .span
            .unwrap_or_else(|| crate::span::Span::new(0, 0, 1, 1));

        // Try VM execution in production mode
        if let Some(vm) = vm {
            let handler_key = format!("{}#{}", class.name, method_name);
            if !vm.failed_handlers.contains(&handler_key) {
                match vm.call_method_bound(
                    &method,
                    instance.clone(),
                    request_hash.clone(),
                    Span::default(),
                ) {
                    Ok(result) => {
                        vm.reset();
                        return Ok(result);
                    }
                    Err(_) => {
                        vm.failed_handlers.insert(handler_key);
                        vm.reset();
                    }
                }
            }
        }

        // Interpreter fallback path
        interpreter.push_frame(
            &format!("{}#{}", class.name, method_name),
            method_span,
            method.source_path.clone(),
        );

        // Set current source path for proper error location tracking
        if let Some(ref source_path) = method.source_path {
            interpreter.set_source_path(std::path::PathBuf::from(source_path));
        }

        // Bind `this` to the instance by wrapping the method's closure.
        // Mirrors the dispatch pattern in interpreter/executor/access/member.rs.
        let bound_method = {
            let mut bound_env = crate::interpreter::environment::Environment::with_enclosing(
                method.closure.clone(),
            );
            bound_env.define("this".to_string(), instance.clone());
            Rc::new(crate::interpreter::value::Function {
                name: method.name.clone(),
                params: method.params.clone(),
                body: method.body.clone(),
                closure: Rc::new(RefCell::new(bound_env)),
                is_method: true,
                span: method.span,
                source_path: method.source_path.clone(),
                defining_superclass: method.defining_superclass.clone(),
                return_type: method.return_type.clone(),
                cached_env: RefCell::new(None),
            })
        };

        let result = interpreter.call_value(
            Value::Function(bound_method),
            vec![request_hash.clone()],
            method_span,
        );

        // Capture environment BEFORE popping frame so we preserve local variables for debugging
        let result = match result {
            Ok(v) => Ok(v),
            Err(e) => {
                // If error already has env (breakpoint or WithEnv), keep it; otherwise capture
                if e.breakpoint_env_json().is_some() {
                    Err(e)
                } else {
                    let env_json = interpreter.serialize_environment_for_debug();
                    let stack_trace = interpreter.get_stack_trace();
                    Err(RuntimeError::with_env(
                        e.to_string(),
                        e.span(),
                        env_json,
                        stack_trace,
                    ))
                }
            }
        };

        interpreter.pop_frame();

        result
    } else {
        Err(RuntimeError::General {
            message: format!(
                "Method '{}' not found in class '{}'",
                method_name, class.name
            ),
            span: Span::default(),
        })
    }
}

/// Get a field from a hash value.
fn get_hash_field(hash: &Value, field: &str) -> Option<Value> {
    match hash {
        Value::Hash(fields) => {
            let key = HashKey::String(field.to_string());
            fields.borrow().get(&key).cloned()
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
        if !before_action.actions.is_empty()
            && before_action.actions.iter().all(|a| a != action_name)
        {
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
    let headers_map: HashPairs = response
        .headers
        .iter()
        .map(|(k, v)| (HashKey::String(k.clone()), Value::String(v.clone())))
        .collect();
    let mut response_map: HashPairs = HashPairs::default();
    response_map.insert(
        HashKey::String("status".to_string()),
        Value::Int(response.status as i64),
    );
    response_map.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(headers_map))),
    );
    response_map.insert(
        HashKey::String("body".to_string()),
        Value::String(response.body.clone()),
    );
    let response_value = Value::Hash(Rc::new(RefCell::new(response_map)));

    for after_action in &controller_info.after_actions {
        // Check if this after_action applies to this action
        if !after_action.actions.is_empty() && after_action.actions.iter().all(|a| a != action_name)
        {
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
        let has_status = fields
            .iter()
            .any(|(k, _)| matches!(k, HashKey::String(s) if s == "status"));

        // If no status field, this is a modified request, not a response
        if !has_status {
            return None;
        }

        let mut status = 200i64;
        let mut body = String::new();
        let mut headers = Vec::new();

        for (key, val) in fields.iter() {
            if let HashKey::String(k) = key {
                match k.as_str() {
                    "status" => {
                        if let Value::Int(s) = val {
                            status = *s;
                        }
                    }
                    "body" => {
                        if let Value::String(b) = val {
                            body = b.clone();
                        }
                    }
                    "headers" => {
                        if let Value::Hash(h) = val {
                            for (hk, hv) in h.borrow().iter() {
                                if let (HashKey::String(key_str), Value::String(val_str)) = (hk, hv)
                                {
                                    headers.push((key_str.clone(), val_str.clone()));
                                }
                            }
                        }
                    }
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
fn create_controller_instance(
    class_name: &str,
    interpreter: &mut Interpreter,
) -> Result<Value, String> {
    crate::interpreter::builtins::controller::registry::create_controller_instance(
        class_name,
        interpreter,
    )
}

/// Set up the controller context (inject req, params, session, headers).
fn setup_controller_context(
    controller: &Value,
    req: &Value,
    params: &Value,
    session: &Value,
    headers: &Value,
) {
    crate::interpreter::builtins::controller::registry::setup_controller_context(
        controller, req, params, session, headers,
    );
}

/// Call a controller method with the request hash.
#[allow(dead_code)]
fn call_controller_method(
    request_hash: &Value,
    method_name: &str,
    interpreter: &mut Interpreter,
) -> Result<Value, String> {
    // Look up the function in the environment and call it with the request hash
    let method_value = match interpreter.environment.borrow().get(method_name) {
        Some(v) => v.clone(),
        None => return Err(format!("Method '{}' not found", method_name)),
    };

    interpreter
        .call_value(method_value, vec![request_hash.clone()], Span::default())
        .map_err(|e| format!("Error calling method: {}", e))
}

// Thread-local cache for PascalCase controller names to avoid per-request string allocation.
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static PASCAL_CASE_CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Convert a controller key (e.g., "posts", "user_profiles") to PascalCase class name (e.g., "PostsController", "UserProfilesController").
/// Uses thread-local cache to avoid per-request string allocation.
fn to_pascal_case_controller(controller_key: &str) -> String {
    PASCAL_CASE_CACHE.with(|cache| {
        let cache_ref = cache.borrow();
        if let Some(cached) = cache_ref.get(controller_key) {
            return cached.clone();
        }
        drop(cache_ref);

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
        cache
            .borrow_mut()
            .insert(controller_key.to_string(), result.clone());
        result
    })
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
            let form_map: HashPairs = form_fields
                .iter()
                .map(|(k, v)| (HashKey::String(k.clone()), Value::String(v.clone())))
                .collect();
            parsed.form = Some(Value::Hash(Rc::new(RefCell::new(form_map))));
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
fn handle_request(
    interpreter: &mut Interpreter,
    vm: &mut Option<crate::vm::Vm>,
    data: &mut RequestData,
    dev_mode: bool,
) -> ResponseData {
    let method = &data.method;
    let path = &data.path;

    // Check if request logging is enabled (cached per-thread to avoid env var lookup per request)
    thread_local! {
        static LOG_REQUESTS: bool = std::env::var("SOLI_REQUEST_LOG")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);
    }
    let log_requests = LOG_REQUESTS.with(|v| *v);

    // Only create timer when logging is enabled (avoids clock_gettime syscall per request)
    let start_time = if log_requests {
        Some(Instant::now())
    } else {
        None
    };

    // Resolve the session ID from the Cookie header (if any). When no cookie is
    // sent, we leave the thread-local unset — session_set / session_regenerate
    // will create one lazily on first use, and finalize_response emits
    // Set-Cookie whenever the post-handler session ID differs from the cookie's.
    let cookie_header = data.headers.get("cookie").map(|s| s.as_str());
    let cookie_session_id = extract_session_id_from_cookie(cookie_header);
    let session_id = if let Some(ref id) = cookie_session_id {
        let resolved = ensure_session(Some(id.as_str()));
        set_current_session_id(Some(resolved.clone()));
        Some(resolved)
    } else {
        set_current_session_id(None);
        None
    };
    let is_new_session = session_id.as_ref() != cookie_session_id.as_ref();

    // Find matching route using indexed lookup (O(1) for exact matches, O(m) for patterns)
    let (route_handler_name, scoped_middleware, matched_params) = match find_route(method, path) {
        Some(found) => found,
        None => {
            // Clear session context before returning
            set_current_session_id(None);
            // Log timing for 404 responses (skip health checks)
            if log_requests && path != "/health" {
                let elapsed = start_time.unwrap().elapsed();
                println!(
                    "[LOG] {} {} - 404 ({:.3}ms)",
                    method,
                    path,
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            let request_id = Uuid::new_v4().to_string();
            eprintln!("[WARN] request_id={} {} {} - 404", request_id, method, path);
            let error_html = error_pages::render_production_error_page(
                404,
                "The page you're looking for doesn't exist.",
                &request_id,
            );
            let is_https = data
                .headers
                .get("x-forwarded-proto")
                .map(|v| v == "https")
                .unwrap_or(false);
            return ResponseData {
                status: 404,
                headers: if is_new_session {
                    if let Some(ref sid) = session_id {
                        vec![
                            (
                                "Set-Cookie".to_string(),
                                create_session_cookie(sid, is_https),
                            ),
                            (
                                "Content-Type".to_string(),
                                "text/html; charset=utf-8".to_string(),
                            ),
                        ]
                    } else {
                        vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )]
                    }
                } else {
                    vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )]
                },
                body: error_html,
            };
        }
    };

    // Expand wildcard action pattern (e.g., "docs#*" → "docs#routing")
    // Skip expansion entirely when handler doesn't use wildcards (common case)
    let handler_name = if !route_handler_name.ends_with("#*") {
        route_handler_name
    } else {
        let expanded_handler = crate::interpreter::builtins::server::expand_wildcard_action(
            &route_handler_name,
            &matched_params,
        );
        if let Some(expanded) = expanded_handler {
            expanded
        } else {
            // Clear session context before returning 404
            set_current_session_id(None);
            let request_id = Uuid::new_v4().to_string();
            eprintln!("[WARN] request_id={} {} {} - 404", request_id, method, path);
            let error_html = error_pages::render_production_error_page(
                404,
                "Action not found for this route.",
                &request_id,
            );
            return ResponseData {
                status: 404,
                headers: vec![(
                    "Content-Type".to_string(),
                    "text/html; charset=utf-8".to_string(),
                )],
                body: error_html,
            };
        }
    };

    // Skip body parsing for GET/HEAD requests (no body to parse)
    let parsed_body = if data.method == "GET" || data.method == "HEAD" {
        ParsedBody::default()
    } else {
        let content_type = data.headers.get("content-type").map(|s| s.as_str());
        parse_request_body(
            &data.body,
            content_type,
            data.multipart_form.as_ref(),
            data.multipart_files.as_ref(),
        )
    };

    // Take ownership of headers and query to avoid cloning individual keys/values
    let headers = std::mem::take(&mut data.headers);
    let query = std::mem::take(&mut data.query);

    // Build request hash with parsed body (owned headers/query avoid String clones)
    let mut request_hash = build_request_hash_with_parsed(
        &data.method,
        &data.path,
        matched_params,
        query,
        headers,
        &data.body,
        parsed_body,
    );

    // Detect HTTPS from X-Forwarded-Proto header
    let is_https = data
        .headers
        .get("x-forwarded-proto")
        .map(|v| v == "https")
        .unwrap_or(false);

    // Helper to finalize response with session cookie and timing
    let finalize_response = |mut resp: ResponseData| -> ResponseData {
        if let Some(cookie_value) = session_cookie_if_changed(
            get_current_session_id().as_deref(),
            cookie_session_id.as_deref(),
            is_https,
        ) {
            resp.headers.push(("Set-Cookie".to_string(), cookie_value));
        }
        // Add security headers if enabled
        {
            use crate::interpreter::builtins::security_headers::get_security_headers;
            let security_headers = get_security_headers();
            for (name, value) in security_headers {
                resp.headers.push((name, value));
            }
        }
        // Log timing (skip health checks to avoid benchmark noise)
        if log_requests && path != "/health" {
            let elapsed = start_time.unwrap().elapsed();
            println!(
                "[LOG] {} {} - {} ({:.3}ms)",
                method,
                path,
                resp.status,
                elapsed.as_secs_f64() * 1000.0
            );
        }
        // Clear session context
        set_current_session_id(None);
        resp
    };

    // Fast path: no middleware at all (avoid cloning middleware list if empty)
    if scoped_middleware.is_empty() && !has_middleware() {
        return finalize_response(call_handler(
            interpreter,
            vm.as_mut(),
            &handler_name,
            request_hash,
            dev_mode,
            data,
        ));
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
                    let (status, headers, body) = extract_response(resp);
                    return finalize_response(ResponseData {
                        status,
                        headers,
                        body,
                    });
                }
                MiddlewareResult::Error(err) => {
                    if dev_mode {
                        let stack_trace = interpreter.get_stack_trace();
                        let error_html = error_pages::render_error_page(
                            &err.to_string(),
                            interpreter,
                            data,
                            &stack_trace,
                            None,
                        );
                        return finalize_response(ResponseData {
                            status: 500,
                            headers: vec![(
                                "Content-Type".to_string(),
                                "text/html; charset=utf-8".to_string(),
                            )],
                            body: error_html,
                        });
                    }
                    let request_id = Uuid::new_v4().to_string();
                    let error_msg = err.to_string();
                    eprintln!(
                        "[ERROR] request_id={} {} {} - {}",
                        request_id, method, path, error_msg
                    );
                    let error_html =
                        error_pages::render_production_error_page(500, &error_msg, &request_id);
                    return finalize_response(ResponseData {
                        status: 500,
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html,
                    });
                }
            },
            Err(e) => {
                if dev_mode {
                    // Use captured stack trace from error if available
                    let stack_trace: Vec<String> = e
                        .breakpoint_stack_trace()
                        .map(|st| st.to_vec())
                        .unwrap_or_else(|| interpreter.get_stack_trace());
                    let breakpoint_env = e.breakpoint_env_json();
                    let error_html = error_pages::render_error_page(
                        &e.to_string(),
                        interpreter,
                        data,
                        &stack_trace,
                        breakpoint_env,
                    );
                    return finalize_response(ResponseData {
                        status: if e.is_breakpoint() { 200 } else { 500 },
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html,
                    });
                }
                let request_id = Uuid::new_v4().to_string();
                let error_msg = e.to_string();
                eprintln!(
                    "[ERROR] request_id={} {} {} - {}",
                    request_id, method, path, error_msg
                );
                let error_html =
                    error_pages::render_production_error_page(500, &error_msg, &request_id);
                return finalize_response(ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
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
                    let (status, headers, body) = extract_response(resp);
                    return finalize_response(ResponseData {
                        status,
                        headers,
                        body,
                    });
                }
                MiddlewareResult::Error(err) => {
                    if dev_mode {
                        let stack_trace = interpreter.get_stack_trace();
                        let error_html = error_pages::render_error_page(
                            &err.to_string(),
                            interpreter,
                            data,
                            &stack_trace,
                            None,
                        );
                        return finalize_response(ResponseData {
                            status: 500,
                            headers: vec![(
                                "Content-Type".to_string(),
                                "text/html; charset=utf-8".to_string(),
                            )],
                            body: error_html,
                        });
                    }
                    let request_id = Uuid::new_v4().to_string();
                    let error_msg = err.to_string();
                    eprintln!(
                        "[ERROR] request_id={} {} {} - {}",
                        request_id, method, path, error_msg
                    );
                    let error_html =
                        error_pages::render_production_error_page(500, &error_msg, &request_id);
                    return finalize_response(ResponseData {
                        status: 500,
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html,
                    });
                }
            },
            Err(e) => {
                if dev_mode {
                    // Use captured stack trace from error if available
                    let stack_trace: Vec<String> = e
                        .breakpoint_stack_trace()
                        .map(|st| st.to_vec())
                        .unwrap_or_else(|| interpreter.get_stack_trace());
                    let breakpoint_env = e.breakpoint_env_json();
                    let error_html = error_pages::render_error_page(
                        &e.to_string(),
                        interpreter,
                        data,
                        &stack_trace,
                        breakpoint_env,
                    );
                    return finalize_response(ResponseData {
                        status: if e.is_breakpoint() { 200 } else { 500 },
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html,
                    });
                }
                let request_id = Uuid::new_v4().to_string();
                let error_msg = e.to_string();
                eprintln!(
                    "[ERROR] request_id={} {} {} - {}",
                    request_id, method, path, error_msg
                );
                let error_html =
                    error_pages::render_production_error_page(500, &error_msg, &request_id);
                return finalize_response(ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html,
                });
            }
        }
    }

    // Call the route handler
    finalize_response(call_handler(
        interpreter,
        vm.as_mut(),
        &handler_name,
        request_hash,
        dev_mode,
        data,
    ))
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

    let code = json
        .get("code")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let request_data = json.get("request_data").cloned();
    let breakpoint_env = json.get("breakpoint_env").cloned();
    let repl_session_id = json
        .get("repl_session_id")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    // Execute the code using the interpreter.
    // Use block_in_place so model DB queries can call block_on without panicking.
    let (result, new_session_id) = tokio::task::block_in_place(|| {
        execute_repl_code(&code, request_data, breakpoint_env, &repl_session_id)
    });

    let response_json = serde_json::json!({
        "result": result.result,
        "error": result.error,
        "repl_session_id": new_session_id
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
    let file = query
        .split('&')
        .filter_map(|p| {
            let mut parts = p.split('=');
            match (parts.next(), parts.next()) {
                (Some("file"), Some(f)) => Some(("file", f)),
                _ => None,
            }
        })
        .find(|(k, _)| *k == "file")
        .map(|(_, f)| {
            urlencoding::decode(f)
                .unwrap_or(Cow::Borrowed(f))
                .into_owned()
        })
        .unwrap_or_else(String::new);

    if file.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(
                r#"{"error": "Missing file parameter"}"#,
            )))
            .unwrap());
    }

    // Reject absolute paths - security measure
    if std::path::Path::new(&file).is_absolute() {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(
                r#"{"error": "Absolute paths not allowed"}"#,
            )))
            .unwrap());
    }

    // Try to read the file - resolve relative to app root
    let app_root = crate::live::component::get_app_root();
    let joined = app_root.join(&file);

    // Canonicalize and verify the path is within app_root
    let canonical_path = match std::fs::canonicalize(&joined) {
        Ok(p) => p,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(r#"{"error": "File not found"}"#)))
                .unwrap());
        }
    };

    let canonical_root = match std::fs::canonicalize(&app_root) {
        Ok(r) => r,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(
                    r#"{"error": "Could not determine app root"}"#,
                )))
                .unwrap());
        }
    };

    let canonical_path_str = canonical_path.to_string_lossy();
    let canonical_root_str = canonical_root.to_string_lossy();

    if !canonical_path_str.starts_with(&*canonical_root_str) {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(
                r#"{"error": "Path outside app directory"}"#,
            )))
            .unwrap());
    }

    if !canonical_path.is_file() {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(r#"{"error": "Not a file"}"#)))
            .unwrap());
    }

    let content = match std::fs::read_to_string(&canonical_path) {
        Ok(c) => c,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(
                    r#"{"error": "Could not read file"}"#,
                )))
                .unwrap());
        }
    };

    // Parse line from query
    let line: usize = query
        .split('&')
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

fn execute_repl_code(
    code: &str,
    request_data: Option<serde_json::Value>,
    breakpoint_env: Option<serde_json::Value>,
    repl_session_id: &str,
) -> (ReplResult, String) {
    let (session_id, session) = REPL_STORE.with(|store| store.get_or_create(repl_session_id));
    let mut interpreter = session.interpreter.borrow_mut();

    if code.trim().is_empty() {
        return (
            ReplResult {
                result: "null".to_string(),
                error: None,
            },
            session_id,
        );
    }

    // Load models into REPL session on first use
    if !*session.models_loaded.borrow() {
        let app_root = crate::live::component::get_app_root();
        let models_dir = app_root.join("app/models");
        if models_dir.exists() {
            if let Err(e) = load_models(&mut interpreter, &models_dir) {
                eprintln!("REPL: Error loading models: {}", e);
            }
        }
        *session.models_loaded.borrow_mut() = true;
    }

    // Inject view helpers into REPL environment (same helpers available in templates)
    for (name, value) in crate::interpreter::builtins::template::get_view_helpers() {
        interpreter.environment.borrow_mut().define(name, value);
    }

    // Set up breakpoint environment variables first (these are the captured variables)
    if let Some(serde_json::Value::Object(map)) = breakpoint_env {
        for (name, value) in map {
            // Skip internal variables
            if !name.starts_with("__") {
                interpreter
                    .environment
                    .borrow_mut()
                    .define(name, convert_json_to_value(value));
            }
        }
    }

    // Set up environment variables from request data
    if let Some(data) = request_data {
        let req_val = convert_json_to_value(data.clone());
        interpreter
            .environment
            .borrow_mut()
            .define("req".to_string(), req_val);

        if let Some(v) = data.get("params").cloned() {
            interpreter
                .environment
                .borrow_mut()
                .define("params".to_string(), convert_json_to_value(v));
        }
        if let Some(v) = data.get("query").cloned() {
            interpreter
                .environment
                .borrow_mut()
                .define("query".to_string(), convert_json_to_value(v));
        }
        if let Some(v) = data.get("body").cloned() {
            interpreter
                .environment
                .borrow_mut()
                .define("body".to_string(), convert_json_to_value(v));
        }
        if let Some(v) = data.get("headers").cloned() {
            interpreter
                .environment
                .borrow_mut()
                .define("headers".to_string(), convert_json_to_value(v));
        }
        if let Some(v) = data.get("session").cloned() {
            interpreter
                .environment
                .borrow_mut()
                .define("session".to_string(), convert_json_to_value(v));
        }
    }

    // Strip trailing semicolon for expression evaluation
    let code_trimmed = code.trim().trim_end_matches(';').trim();

    // First, try to evaluate as an expression (to capture and return the value)
    let wrapped_code = format!("let __repl_result__ = ({});", code_trimmed);
    let tokens = crate::lexer::Scanner::new(&wrapped_code).scan_tokens();
    let parse_result = tokens.map_err(|e| format!("{:?}", e)).and_then(|tokens| {
        crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| format!("{:?}", e))
    });

    if let Ok(program) = parse_result {
        match interpreter.interpret(&program) {
            Ok(_) => {
                // Get the result from environment
                let result_val = interpreter.environment.borrow().get("__repl_result__");
                let result_str = match result_val {
                    Some(v) => format!("{}", v),
                    None => "null".to_string(),
                };
                return (
                    ReplResult {
                        result: result_str,
                        error: None,
                    },
                    session_id,
                );
            }
            Err(e) => {
                return (
                    ReplResult {
                        result: "null".to_string(),
                        error: Some(format!("Execution error: {}", e)),
                    },
                    session_id,
                );
            }
        }
    }

    // If expression evaluation failed, try parsing as a complete program (statements)
    let tokens = crate::lexer::Scanner::new(code).scan_tokens();
    let parse_result = tokens.map_err(|e| format!("{:?}", e)).and_then(|tokens| {
        crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| format!("{:?}", e))
    });

    let result = match parse_result {
        Ok(program) => match interpreter.interpret(&program) {
            Ok(_) => ReplResult {
                result: "ok".to_string(),
                error: None,
            },
            Err(e) => ReplResult {
                result: "null".to_string(),
                error: Some(format!("Execution error: {}", e)),
            },
        },
        Err(parse_errors) => ReplResult {
            result: "null".to_string(),
            error: Some(format!("Parse error: {}", parse_errors)),
        },
    };

    (result, session_id)
}

/// Helper to convert JSON to Value, returning Null on error.
fn convert_json_to_value(json: serde_json::Value) -> crate::interpreter::value::Value {
    json::convert_json_to_value(json)
}

/// Resolve a request path to a static file in the public directory.
/// Returns:
///   Ok(Some(path)) - file found and safe to serve
///   Ok(None) - not a static file, fall through to route matching
///   Err(()) - path traversal attempt, should return 403
fn resolve_static_file(path: &str, public_dir: &Path) -> Result<Option<PathBuf>, ()> {
    let relative_path = path.trim_start_matches('/');
    let decoded_path = match urlencoding::decode(relative_path) {
        Ok(d) => d.into_owned(),
        Err(_) => relative_path.to_string(),
    };
    // Do not allow directory traversal or absolute paths in URL
    if decoded_path.contains("..") || decoded_path.starts_with('/') {
        return Ok(None);
    }
    let file_path = public_dir.join(&decoded_path);

    // Canonicalize both paths to resolve symlinks and prevent traversal
    let (canonical_file, canonical_public) = match (
        std::fs::canonicalize(&file_path),
        std::fs::canonicalize(public_dir),
    ) {
        (Ok(f), Ok(p)) => (f, p),
        _ => return Ok(None), // file doesn't exist, fall through
    };

    // Ensure the canonical file path is within public directory
    if !canonical_file
        .to_string_lossy()
        .starts_with(&*canonical_public.to_string_lossy())
    {
        return Err(()); // traversal attempt
    }

    if !canonical_file.is_file() {
        return Ok(None); // directory or special file, fall through
    }

    Ok(Some(file_path))
}

#[allow(dead_code)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_static_file_serves_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();
        fs::write(public.join("style.css"), "body{}").unwrap();

        let result = resolve_static_file("/style.css", &public);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_resolve_static_file_root_path_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();

        // "/" should NOT return 404 — it should fall through (None) so route matching handles it
        let result = resolve_static_file("/", &public);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_resolve_static_file_directory_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let subdir = public.join("css");
        fs::create_dir_all(&subdir).unwrap();

        // "/css" is a directory, should fall through
        let result = resolve_static_file("/css", &public);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_resolve_static_file_nonexistent_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();

        let result = resolve_static_file("/nope.js", &public);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_resolve_static_file_blocks_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();

        let result = resolve_static_file("/../etc/passwd", &public);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_resolve_static_file_blocks_encoded_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();

        let result = resolve_static_file("/%2e%2e/etc/passwd", &public);
        assert_eq!(result, Ok(None));
    }

    /// Regression test: controller actions dispatched via `call_class_method`
    /// must have `this` bound to the instance. Previously the dispatcher
    /// called the method as a plain function, so any `this.xxx()` inside an
    /// action threw "'this' outside of class" at runtime.
    #[test]
    fn test_call_class_method_binds_this_interpreter_path() {
        use crate::interpreter::value::Instance;
        use crate::lexer::Scanner;
        use crate::parser::Parser;

        let source = r#"
            class Foo {
                fn action(req) {
                    return this.helper()
                }
                fn helper() {
                    return 42
                }
            }
        "#;

        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).unwrap();

        let class_val = interpreter.environment.borrow().get("Foo").unwrap();
        let class_rc = match class_val {
            Value::Class(c) => c,
            _ => panic!("Foo did not resolve to a class"),
        };

        let instance = Value::Instance(Rc::new(RefCell::new(Instance::new(class_rc.clone()))));
        let request_hash = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));

        let result =
            call_class_method(&mut interpreter, None, &class_rc, &instance, "action", &request_hash)
                .expect("call_class_method should succeed when this is bound");

        assert_eq!(result, Value::Int(42));
    }

    /// Same regression coverage for the VM (production) dispatch path.
    #[test]
    fn test_call_class_method_binds_this_vm_path() {
        use crate::interpreter::value::Instance;
        use crate::lexer::Scanner;
        use crate::parser::Parser;

        let source = r#"
            class Bar {
                fn action(req) {
                    return this.helper()
                }
                fn helper() {
                    return 99
                }
            }
        "#;

        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).unwrap();

        let class_val = interpreter.environment.borrow().get("Bar").unwrap();
        let class_rc = match class_val {
            Value::Class(c) => c,
            _ => panic!("Bar did not resolve to a class"),
        };

        // Set up a VM seeded with the interpreter's globals (same as production).
        let mut vm = crate::vm::Vm::new();
        let all_globals = interpreter.environment.borrow().get_all_bindings();
        for (name, value) in all_globals {
            vm.globals.insert(name, value);
        }

        let instance = Value::Instance(Rc::new(RefCell::new(Instance::new(class_rc.clone()))));
        let request_hash = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));

        let result = call_class_method(
            &mut interpreter,
            Some(&mut vm),
            &class_rc,
            &instance,
            "action",
            &request_hash,
        )
        .expect("call_class_method should succeed when this is bound (VM)");

        assert_eq!(result, Value::Int(99));
    }
}
