//! MVC framework with convention-based routing and hot reload.
//!
//! This module implements a Rails-like MVC framework for Soli applications:
//! - Convention-based routing from controller filenames and function names
//! - Hot reload of changed files without server restart
//! - Automatic route derivation
//! - Middleware support for request interception

mod asset_cache;
pub mod cors;
pub mod dev_bar;
pub mod dev_store;
mod hot_reload;
pub mod live_reload;
mod live_reload_ws; // WebSocket-based live reload
pub(crate) mod middleware;
pub mod middleware_log;
pub mod nav;
pub mod openapi;
pub mod phase_log;
pub mod prefetch;
pub mod prod_log;
pub mod route_listing;
pub mod route_log;
mod router;
mod server_constants;
pub mod span_log;
pub mod template_warnings;
mod uploads_prelude;
pub mod view_log;
pub mod websocket;

// Modularized subcomponents
pub(crate) mod app_loader;
pub mod background_jobs;
pub mod engine_loader;
pub mod env_loader;
mod error_logging;
mod error_pages;
mod file_tracker;
pub(crate) mod file_upload;
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
    clear_websocket_routes, get_runtime_handle, get_websocket_routes, match_websocket_route,
    register_websocket_route, restore_websocket_routes, set_runtime_handle, take_websocket_routes,
    WebSocketConnection, WebSocketEvent, WebSocketHandlerAction, WebSocketRegistry,
};

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::virtual_fs::VirtualFileSystem;

// Global Virtual File System — set during server boot. When set, all
// file reads go through this VFS. When not set, the helper functions
// fall back to std::fs on disk.
static GLOBAL_VFS: OnceLock<Box<dyn VirtualFileSystem>> = OnceLock::new();

/// Initialize the global VFS (called before server boot).
pub fn init_global_vfs(vfs: impl VirtualFileSystem + 'static) {
    let _ = GLOBAL_VFS.set(Box::new(vfs));
}

/// Read a file to string, falling back to std::fs if no VFS is set.
pub fn vfs_read_to_string(path: &str) -> Result<String, String> {
    if let Some(vfs) = GLOBAL_VFS.get() {
        vfs.read_to_string(path)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read '{}': {}", path, e))
    }
}

/// Read a file's raw bytes, falling back to std::fs if no VFS is set.
pub fn vfs_read(path: &str) -> Result<Vec<u8>, String> {
    if let Some(vfs) = GLOBAL_VFS.get() {
        vfs.read(path)
    } else {
        std::fs::read(path).map_err(|e| format!("Failed to read '{}': {}", path, e))
    }
}

/// Check if a path exists, falling back to std::fs if no VFS is set.
pub fn vfs_exists(path: &str) -> bool {
    if let Some(vfs) = GLOBAL_VFS.get() {
        vfs.exists(path)
    } else {
        std::path::Path::new(path).exists()
    }
}

/// Walk a directory, falling back to walkdir if no VFS is set.
pub fn vfs_walk_dir(dir: &str) -> Result<Vec<String>, String> {
    if let Some(vfs) = GLOBAL_VFS.get() {
        vfs.walk_dir(dir)
    } else {
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        {
            let entry = entry.map_err(|e| format!("Walk error: {}", e))?;
            if entry.file_type().is_file() {
                let full_path = entry.path().to_string_lossy().to_string();
                files.push(full_path);
            }
        }
        files.sort();
        Ok(files)
    }
}

/// Check if a path is a directory, falling back to std::fs if no VFS is set.
pub fn vfs_is_dir(path: &str) -> bool {
    if let Some(vfs) = GLOBAL_VFS.get() {
        vfs.is_dir(path)
    } else {
        std::path::Path::new(path).is_dir()
    }
}

/// Env-gated boot tracing. Set `SOLI_TRACE_BOOT=1` to print
/// `[boot+Xms] <phase>` to stderr at each major startup step. The first
/// `boot_trace` call captures the baseline; subsequent calls show ms
/// since that baseline, plus the delta from the previous call.
static BOOT_START: OnceLock<Instant> = OnceLock::new();
static BOOT_LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
static DEV_REPL_AUTH_TOKEN: OnceLock<String> = OnceLock::new();

fn dev_repl_auth_token() -> &'static str {
    DEV_REPL_AUTH_TOKEN
        .get_or_init(|| {
            // SEC-051: in remote-allowed mode the operator must supply
            // an explicit shared secret via SOLI_DEV_REPL_SECRET so the
            // auto-generated UUID never lands in an HTML error page
            // someone on the LAN can scrape. The startup check in
            // serve_folder_with_options_and_workers refuses to launch
            // with ALLOW_REMOTE+no SECRET, so by the time anything
            // calls this we either have a SECRET (remote mode) or a
            // generated UUID (loopback-only mode).
            std::env::var("SOLI_DEV_REPL_SECRET")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| Uuid::new_v4().to_string())
        })
        .as_str()
}

fn dev_repl_allows_remote() -> bool {
    std::env::var("SOLI_DEV_REPL_ALLOW_REMOTE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn dev_repl_secret_set() -> bool {
    std::env::var("SOLI_DEV_REPL_SECRET")
        .ok()
        .is_some_and(|s| !s.is_empty())
}

fn boot_trace(phase: &str) {
    if std::env::var("SOLI_TRACE_BOOT").is_err() {
        return;
    }
    let start = *BOOT_START.get_or_init(Instant::now);
    let now = Instant::now();
    let total_ms = now.duration_since(start).as_millis();
    let mut last = BOOT_LAST.lock().unwrap();
    let delta_ms = match *last {
        Some(prev) => now.duration_since(prev).as_millis(),
        None => 0,
    };
    *last = Some(now);
    eprintln!(
        "{} [boot+{total_ms:>5}ms Δ{delta_ms:>4}ms] {phase}",
        log_timestamp()
    );
}

/// Wall-clock timestamp (local time, millisecond precision) for log lines,
/// e.g. `2026-06-01 14:23:45.123`. Used to correlate boot/request prints when
/// debugging latency.
pub fn log_timestamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string()
}

use bytes::Bytes;
use crossbeam::channel;
use futures_util::SinkExt;
use futures_util::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use http_body_util::Full;
use http_body_util::Limited;
use http_body_util::StreamBody;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{header, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};
use uuid::Uuid;

use crate::error::RuntimeError;
use crate::interpreter::builtins::server::{
    build_request_hash_with_parsed, extract_response, find_route, get_routes,
    parse_form_urlencoded_body, parse_json_body, parse_query_pairs, parse_query_string,
    routes_to_worker_routes, set_worker_routes, ParsedBody, WorkerRoute,
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
pub fn set_tokio_handle(handle: tokio::runtime::Handle) {
    TOKIO_HANDLE.with(|h| *h.borrow_mut() = Some(handle));
}

/// Process-wide LiveView event sender. Set once during server startup and
/// read by `handle_liveview_event` when it needs to spawn a per-instance
/// tick task that posts back into the worker queue.
static LV_EVENT_TX: std::sync::OnceLock<channel::Sender<LiveViewEventData>> =
    std::sync::OnceLock::new();
use crate::interpreter::builtins::controller::controller::ControllerInfo;
use crate::interpreter::builtins::controller::CONTROLLER_REGISTRY;
use crate::interpreter::builtins::session::{
    clear_response_cookies, ensure_session, finalize_session_cookie, get_current_session_id,
    parse_cookie_pairs, session_id_from_cookie_pairs, set_current_session_id,
    take_response_cookies,
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
    pub(crate) query: Vec<(String, String)>,
    /// The wire headers, moved straight out of hyper (names are lowercase).
    /// `HeaderMap` is `Send`, so no per-header String copies happen on the
    /// async side; the worker converts to Soli `HashPairs` exactly once when
    /// it builds `req["headers"]`.
    pub(crate) headers: hyper::header::HeaderMap,
    pub(crate) body: String,
    /// Raw body bytes (for multipart parsing)
    #[allow(dead_code)]
    pub(crate) body_bytes: Option<Vec<u8>>,
    /// Pre-parsed form fields from multipart
    pub(crate) multipart_form: Option<Vec<(String, String)>>,
    /// Pre-parsed files from multipart
    pub(crate) multipart_files: Option<Vec<UploadedFile>>,
    /// SEC-030: actual TCP peer IP (no port). Threaded to handlers as
    /// `req["remote_addr"]` so the rate limiter can fall back to it
    /// when `enable_trust_proxy()` is off — without this, an attacker
    /// rotating `X-Forwarded-For` per request would mint a fresh
    /// rate-limit bucket each time and bypass the limiter entirely.
    pub(crate) peer_ip: String,
    /// When request logging is active (`SOLI_LOG` / `SOLI_SLOW_REQUEST_MS`),
    /// the instant the hyper handler enqueued this request — the worker
    /// diffs it at handling time to expose queue wait. `None` when logging
    /// is off so the no-logging hot path keeps zero clock reads.
    pub(crate) enqueued_at: Option<std::time::Instant>,
    /// True when this request is a dev-bar replay of a previously captured
    /// request (`POST /__solidev/replay/:id`). The worker skips the per-form
    /// CSRF token check for replays — a rotated session token would otherwise
    /// 403 a faithful re-dispatch. Always false for real client traffic.
    pub(crate) replay: bool,
    pub(crate) response_tx: oneshot::Sender<WorkerResponse>,
}

/// Borrow a header value from the wire `HeaderMap` by its lowercase name.
/// Non-UTF-8 header values read as absent (same as the previous
/// `HashMap<String, String>` extraction, which skipped them).
#[inline]
fn header_str<'a>(headers: &'a hyper::header::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Response data from interpreter thread
#[derive(Clone)]
pub(crate) struct ResponseData {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

/// Worker → service reply. Buffered responses carry a complete `ResponseData`;
/// streaming responses carry the status/headers plus a channel the worker feeds
/// body chunks into (handler-driven SSE / chunked bodies).
pub(crate) enum WorkerResponse {
    Buffered(ResponseData),
    Stream {
        status: u16,
        headers: Vec<(String, String)>,
        rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    },
}

/// Boxed response body so buffered (`Full`) and streaming (`StreamBody`)
/// responses share one type at the hyper service boundary.
pub(crate) type ResponseBody = BoxBody<Bytes, std::io::Error>;

/// Wrap fully-buffered bytes as a boxed response body.
fn full(body: Bytes) -> ResponseBody {
    Full::<Bytes>::new(body)
        .map_err(|never| match never {})
        .boxed()
}

/// Box a `Response<Full<Bytes>>` (handlers that still build `Full`, e.g. the
/// live-reload handlers) into the unified boxed body type.
fn box_full(resp: Response<Full<Bytes>>) -> Response<ResponseBody> {
    resp.map(|b| b.map_err(|never| match never {}).boxed())
}

/// Add a response header only if its name and value are valid per HTTP rules.
///
/// Controller- and cookie-supplied header strings can carry CR/LF/NUL (header-
/// injection attempts). Passing those to `Builder::header` silently poisons the
/// builder so a later `.body(...)` returns `Err` — and with `panic = "abort"`
/// that turns a single crafted request into a whole-process crash. Validating
/// up-front lets us drop the malformed header and keep serving instead of
/// aborting, and hyper's own byte rejection means CRLF can never reach the wire
/// (so this is a DoS fix, not response-splitting mitigation).
fn add_header_checked(
    builder: hyper::http::response::Builder,
    key: &str,
    value: &str,
) -> hyper::http::response::Builder {
    match (
        hyper::header::HeaderName::try_from(key),
        hyper::header::HeaderValue::try_from(value),
    ) {
        (Ok(name), Ok(val)) => builder.header(name, val),
        _ => builder,
    }
}

/// Finish a response, falling back to a static 500 if the builder is somehow in
/// an error state. Prevents the `panic = "abort"` whole-process crash that a
/// bare `.body(..).unwrap()` would cause on a poisoned builder.
fn finish_response(builder: hyper::http::response::Builder, body: Bytes) -> Response<ResponseBody> {
    builder.body(full(body)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(full(Bytes::from_static(b"Internal Server Error")))
            .expect("static 500 response is always valid")
    })
}

// File tracking functions (used by app_loader for initial file tracking in workers)
// The watcher thread now uses notify crate for event-driven file watching.

// File upload functions are now in file_upload module
use file_upload::uploaded_files_to_value;

/// Serve an MVC application from a folder in production mode by default.
///
/// Respects `SOLI_WORKERS` env var (or falls back to CPU core count).
/// Operators running on boxes with many cores can pin this low (e.g. 2-4)
/// to keep baseline RSS from duplicated interpreter state + tokio runtimes
/// under control — directly relevant to "keep RAM low as much as possible".
pub fn serve_folder(folder: &Path, port: u16) -> Result<(), RuntimeError> {
    // Allow operators to explicitly cap workers (common on multi-core boxes
    // when the goal is minimizing memory rather than maximizing throughput).
    let num_workers = std::env::var("SOLI_WORKERS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(server_constants::DEFAULT_WORKER_COUNT)
        });

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

use env_loader::load_env_files;

/// Serve an MVC application from a folder with configurable options and worker count.
pub fn serve_folder_with_options_and_workers(
    folder: &Path,
    port: u16,
    dev_mode: bool,
    workers: usize,
) -> Result<(), RuntimeError> {
    boot_trace("serve_folder enter");

    // SEC-051: refuse to start with --dev + ALLOW_REMOTE unless an
    // explicit shared secret is also pinned. The /__dev/repl endpoint
    // is full server-side code execution; in loopback-only mode the
    // auto-generated UUID is fine because attackers can't reach the
    // port, but ALLOW_REMOTE pairs that auto-token with HTML error
    // pages anyone on the LAN can render — one error response would
    // leak the token. Forcing SOLI_DEV_REPL_SECRET makes the operator
    // pick (and not embed) the credential.
    if dev_mode && dev_repl_allows_remote() && !dev_repl_secret_set() {
        return Err(RuntimeError::General {
            message: "[SEC-051] --dev with SOLI_DEV_REPL_ALLOW_REMOTE=1 \
                refuses to start without SOLI_DEV_REPL_SECRET. The \
                remote-allowed dev REPL is full code execution; pin it \
                to an explicit non-empty secret so the token isn't \
                leaked via every dev-mode error page."
                .to_string(),
            span: Span::default(),
        });
    }

    // SEC-056: security headers default to ON. In `--dev` mode, flip
    // them back off so the dev bar's inline scripts and the dev REPL
    // aren't blocked by a CSP the operator didn't choose. Production
    // (`--no-dev`) keeps the baseline (X-Frame-Options: SAMEORIGIN +
    // X-Content-Type-Options: nosniff) without any explicit opt-in.
    if dev_mode {
        crate::interpreter::builtins::security_headers::set_security_headers_enabled(false);
    }

    // Resolve to an absolute path — notify emits absolute event paths, so
    // storing watch dirs as relative would break the `starts_with` checks
    // that classify hot-reload events by category.
    let folder_owned = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let folder = folder_owned.as_path();

    // Load `.env` (and `.env.{APP_ENV}` if set) before any builtin reads
    // SOLIDB_* / SOLI_* env vars. Was dropped as collateral damage in
    // a17f300; without it `init_db_config` + `init_jwt_token` below run
    // with no credentials, every DB request goes out unauthenticated,
    // and SolidB 401s.
    load_env_files(folder);
    boot_trace("env loaded");

    // Cache SoliDB host/database/api-key/basic-auth derived from the env
    // we just loaded. Must run before `init_jwt_token` so the JWT login
    // and the cursor URL see the same `SOLIDB_HOST` parse.
    crate::interpreter::builtins::model::init_db_config();
    boot_trace("db config init");

    // Validate folder structure
    let app_dir = folder.join("app");
    let controllers_dir = app_dir.join("controllers");
    let controllers_ok = if GLOBAL_VFS.get().is_some() {
        vfs_is_dir("app/controllers") || vfs_is_dir("app")
    } else {
        controllers_dir.exists()
    };

    if !controllers_ok {
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

    // SEC-006: enable the filesystem jail for the `File` builtins so a
    // controller calling `File.read(req["params"]["path"])` cannot reach
    // outside the project directory. Code that needs full access (log
    // shippers, backup scripts) goes through the parallel `Trusted`
    // class. CLI invocations (`soli run`, the REPL, the test runner)
    // never reach this branch and keep their unrestricted access.
    crate::interpreter::builtins::file::set_file_jail(folder.to_path_buf());

    // SEC-063: set the image jail so Image.new / Image.to_file cannot
    // read or write outside the app directory.
    crate::interpreter::builtins::image::set_image_jail(folder.to_path_buf());

    // If the parent process enabled coverage collection (via the test
    // runner), install a global coverage tracker so every interpreter in
    // every worker thread records line hits into it. The hits are returned
    // to the parent via the `/__coverage__` JSON endpoint at shutdown.
    if std::env::var("SOLI_COVERAGE_ENABLED").is_ok() {
        use crate::coverage::tracker::set_global_coverage_tracker;
        use crate::coverage::{CoverageConfig, CoverageTracker, OutputFormat};
        let config = CoverageConfig {
            enabled: true,
            output_dir: std::path::PathBuf::from("coverage"),
            formats: vec![OutputFormat::Console],
            threshold: None,
            exclude_patterns: Vec::new(),
            exclude_lines: Vec::new(),
            show_uncovered: false,
            per_test: false,
            root_dir: Some(folder.to_path_buf()),
        };
        let mut tracker = CoverageTracker::new(config);
        register_app_source_lines_for_server(&mut tracker, folder);
        set_global_coverage_tracker(std::sync::Arc::new(std::sync::Mutex::new(tracker)));
    }

    // Create interpreter
    let mut interpreter = Interpreter::new();
    // Define the Mailer/Message base classes before any app code that may
    // subclass Mailer (app/mailers/*.sl) loads.
    crate::interpreter::builtins::mailer::ensure_prelude(&mut interpreter);
    boot_trace("interpreter created");

    // Load models first (shared code)
    let models_dir = app_dir.join("models");
    if models_dir.exists() {
        load_models(&mut interpreter, &models_dir)?;
    }
    boot_trace("models loaded");

    // Dev convenience: ensure class-body index declarations (`index`,
    // `vector_index`, `fulltext_index`, `geo_index`) exist in the DB.
    // Production deploys run `soli db:indexes` (or migrations) instead.
    if dev_mode {
        static INDEX_SYNC_ONCE: std::sync::Once = std::sync::Once::new();
        INDEX_SYNC_ONCE.call_once(|| {
            for line in crate::interpreter::builtins::model::index_sync::sync_declared_indexes() {
                println!("  [indexes] {}", line);
            }
        });
        boot_trace("declared indexes synced");
    }

    // Load services (integration helpers — Stripe, etc.) right after models
    // so controllers can reference them. Same loader as models since the
    // shape (just `.sl` files defining classes / bare fns) is identical.
    let services_dir = app_dir.join("services");
    if services_dir.exists() {
        load_models(&mut interpreter, &services_dir)?;
    }
    boot_trace("services loaded");

    // Load authorization policies (app/policies/*.sl) before controllers so
    // controller actions can call `authorize(...)` and `const_get("XPolicy")`
    // can resolve the policy classes. Same loader shape as models: each file
    // just defines classes / bare functions into the shared global env.
    let policies_dir = app_dir.join("policies");
    if policies_dir.exists() {
        load_models(&mut interpreter, &policies_dir)?;
    }
    boot_trace("policies loaded");

    // Load mailers (app/mailers/*.sl) — `class UserMailer < Mailer`. The
    // Mailer base class was defined above (ensure_prelude), so subclasses
    // resolve their superclass at load time.
    let mailers_dir = app_dir.join("mailers");
    if mailers_dir.exists() {
        load_models(&mut interpreter, &mailers_dir)?;
    }
    boot_trace("mailers loaded");

    // Initialize file tracker for hot reload
    let mut file_tracker = FileTracker::new();

    // Load background-job classes (app/jobs/*_job.sl) before controllers so
    // controllers can reference them. Worker 0 also syncs `static cron`
    // declarations to SolidB.
    let jobs_dir = app_dir.join("jobs");
    if jobs_dir.exists() {
        app_loader::load_jobs_in_worker(0, &mut interpreter, &jobs_dir, &mut file_tracker, true);
    }
    boot_trace("jobs loaded");

    // Load middleware
    let middleware_dir = app_dir.join("middleware");
    if middleware_dir.exists() {
        load_middleware(&mut interpreter, &middleware_dir, &mut file_tracker)?;
    }
    boot_trace("middleware loaded");

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
    boot_trace("helpers loaded");

    // Ship the framework upload helpers + `AttachmentsController` class
    // BEFORE user controllers so user-defined definitions cleanly override
    // by being later in the same env.
    if let Err(e) = uploads_prelude::define_uploads_prelude(&mut interpreter) {
        eprintln!("Warning: failed to load uploads prelude: {}", e);
    }
    boot_trace("uploads prelude");

    // Scan and load controllers
    let controller_files = scan_controllers(&controllers_dir)?;
    for controller_path in &controller_files {
        load_controller(
            &mut interpreter,
            &controllers_dir,
            controller_path,
            &mut file_tracker,
        )?;
    }
    boot_trace("controllers loaded");

    // Populate the controller metadata registry (before/after hooks, layout,
    // inheritance) by textually scanning `app/controllers/*.sl`. Without this,
    // `execute_before_actions`/`execute_after_actions` see no registered hooks
    // and silently skip them.
    if let Err(e) =
        crate::interpreter::builtins::controller::registry::scan_controllers(&controllers_dir)
    {
        eprintln!("Warning: Failed to scan controller metadata: {}", e);
    }
    boot_trace("controller metadata scanned");

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
    boot_trace("engines loaded");

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
    boot_trace("templates initialized");

    // Set live reload flag for template injection (only in dev mode)
    live_reload::set_live_reload_enabled(dev_mode);

    // Load app-level startup config from config/application.sl if it exists.
    // Runs before routes so calls like `enable_trust_proxy()` or
    // `set_max_body_size(...)` are in effect by the time the first request
    // is handled.
    let application_file = folder.join("config").join("application.sl");
    if application_file.exists() {
        execute_file(&mut interpreter, &application_file)?;
    }
    boot_trace("application config loaded");

    // Load translations from config/locales/*.yml so I18n.translate(...) can
    // resolve keys against the project's locale files without callers having
    // to pass a translations hash on every call.
    crate::interpreter::builtins::i18n::helpers::load_locales_from_config_dir(
        &folder.join("config"),
    );
    boot_trace("locales loaded");

    // Load routes from config/routes.sl if it exists
    let routes_file = folder.join("config").join("routes.sl");
    if routes_file.exists() {
        // Define DSL helpers (resources/get/post/uploads/etc.). Single
        // source of truth lives in `app_loader::ROUTES_DSL_SOURCE` so
        // initial load and worker hot-reload can never drift.
        define_routes_dsl(&mut interpreter)?;

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
    boot_trace("routes loaded");

    // Public directory for static files
    let public_dir = folder.join("public");

    // Compile Tailwind CSS once at startup (not watch mode to avoid reload
    // loops). Skip in test mode — the test runner spawns one server per
    // worker, and `tailwindcss` is a heavy subprocess (~hundreds of ms);
    // 8 in parallel was the dominant cost of `soli test --jobs 8` boot.
    // Tests hit controllers directly, so the CSS bundle is irrelevant.
    let app_env = std::env::var("APP_ENV").unwrap_or_default();
    if dev_mode && app_env != "test" {
        tailwind::compile_tailwind_css_once(folder);
        boot_trace("tailwind compiled");
    }

    boot_trace("ready — entering hyper server");

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
        jobs_dir,
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
    jobs_dir: PathBuf,
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
    // Make the sender available to `handle_liveview_event` so it can spawn
    // per-instance tick tasks that re-enter the worker queue.
    let _ = LV_EVENT_TX.set(lv_event_tx.clone());
    // crossbeam Sender is cheap to clone - no need for Arc<Mutex<Option<>>>
    // Use AtomicBool for shutdown signaling (lock-free check)
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_for_tokio = shutdown_flag.clone();

    // Single shared queue drained by all workers: any free worker pulls the
    // next request, so a request is never stranded behind a busy worker.
    let worker_queues = Arc::new(WorkerQueues::new(num_workers, capacity_per_worker));
    let worker_queues_for_tokio = worker_queues.clone();

    // Channel to pass actual bound port from tokio thread to main thread
    let (bound_port_tx, bound_port_rx) = std::sync::mpsc::channel::<u16>();

    // Wrap public_dir in Arc for cheap cloning across connections
    let public_dir_arc = Arc::new(public_dir.clone());
    // Build prod-mode in-memory snapshot of CSS/JS assets so a mid-deploy file
    // swap on disk doesn't desync against still-cached HTML in browsers.
    let asset_cache = asset_cache::build(&public_dir, dev_mode);
    let asset_cache_for_tokio = asset_cache.clone();
    let ws_registry_for_tokio = ws_registry.clone();
    let dev_mode_for_tokio = dev_mode;

    // Channel to pass runtime handle from tokio thread to main thread
    let (runtime_handle_tx, runtime_handle_rx) =
        std::sync::mpsc::channel::<tokio::runtime::Handle>();

    // Spawn tokio runtime for HTTP server.
    //
    // The runtime drives ALL async I/O: inbound HTTP serving *and* every
    // outbound SoliDB/HTTP connection, which the Soli interpreter workers reach
    // via `block_on` on this runtime's handle. Sizing it to `num_workers`
    // starved it: with `--workers 1` the single I/O thread had to serve the
    // inbound request *and* drive the outbound DB connection at the same time,
    // so a worker blocked in `block_on` left no thread to advance its own DB
    // call — it stalled to the 30s client timeout on every request. With more
    // workers there was incidental slack, which is why the symptom looked
    // intermittent / first-call-only. The runtime's thread pool must therefore
    // be sized for I/O concurrency, not for the interpreter-worker count: give
    // it room for every worker's outbound call plus inbound serving headroom,
    // with a sane floor.
    let tokio_worker_threads = (num_workers + 2).max(4);

    // Bind address. `SOLI_HOST` restricts the listening interface (e.g.
    // `127.0.0.1` to keep a dev server off the LAN); default is all
    // interfaces, preserving prior behavior. An invalid value is a hard
    // error rather than a silent fallback — quietly binding 0.0.0.0 when
    // the operator asked for loopback would expose an interface they
    // explicitly tried to close.
    let bind_host: std::net::IpAddr = match std::env::var("SOLI_HOST") {
        Ok(v) if !v.trim().is_empty() => v.trim().parse().unwrap_or_else(|_| {
            eprintln!(
                "Invalid SOLI_HOST '{}': expected an IP address like 127.0.0.1 or ::1",
                v
            );
            std::process::exit(1);
        }),
        _ => std::net::IpAddr::from([0, 0, 0, 0]),
    };

    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(tokio_worker_threads)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        runtime.block_on(async move {
            // Send runtime handle to main thread for workers to use
            let handle = tokio::runtime::Handle::current();
            crate::serve::websocket::set_runtime_handle(handle.clone());
            let _ = runtime_handle_tx.send(handle);

            // Try the requested port, then scan for a free one
            let mut try_port = port;
            let listener = loop {
                let addr = SocketAddr::from((bind_host, try_port));
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
                let (stream, peer_addr) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => continue,
                };
                // Disable Nagle's algorithm: without this, small responses
                // written across multiple TCP segments stall ~40ms waiting on
                // the peer's delayed-ACK, even when the CPU is idle. Matches the
                // proxy/db convention which already set this on their sockets.
                let _ = stream.set_nodelay(true);
                let io = TokioIo::new(stream);
                let request_tx = worker_queues_for_tokio.get_sender();
                let reload_tx = reload_tx_for_tokio.clone();
                let public_dir = public_dir_arc.clone(); // Arc clone is cheap
                let asset_cache = asset_cache_for_tokio.clone(); // Arc clone is cheap
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
                        let asset_cache = asset_cache.clone(); // Arc clone is cheap
                        let ws_event_tx = ws_event_tx.clone();
                        let lv_event_tx = lv_event_tx.clone();
                        let shutdown_flag = shutdown_flag.clone();

                        async move {
                            // Lock-free shutdown check (AtomicBool)
                            if shutdown_flag.load(Ordering::Relaxed) {
                                return Ok(Response::builder()
                                    .status(StatusCode::SERVICE_UNAVAILABLE)
                                    .body(full(Bytes::from("Server shutting down")))
                                    .unwrap());
                            }
                            // Built-in CORS (`cors("/api/*", {...})` in
                            // config/routes.sl). Wraps the whole handler so
                            // every response of a CORS-managed path —
                            // buffered, streamed, static, or error — carries
                            // the allow headers, and preflights are answered
                            // before routing.
                            let cors_decision = cors::evaluate(
                                req.method().as_str(),
                                req.uri().path(),
                                req.headers(),
                            );
                            if let Some(preflight) =
                                cors_decision.as_ref().and_then(|d| d.preflight.as_ref())
                            {
                                let mut builder = Response::builder()
                                    .status(StatusCode::NO_CONTENT)
                                    .header("Server", "soliMVC");
                                for (key, value) in preflight {
                                    builder = add_header_checked(builder, key, value);
                                }
                                return Ok(builder
                                    .body(full(Bytes::new()))
                                    .unwrap_or_else(|_| Response::new(full(Bytes::new()))));
                            }
                            let result = handle_hyper_request(
                                req,
                                request_tx,
                                reload_tx,
                                public_dir,
                                asset_cache,
                                ws_event_tx,
                                lv_event_tx,
                                dev_mode,
                                peer_addr,
                            )
                            .await;
                            match (result, cors_decision) {
                                (Ok(mut response), Some(decision)) => {
                                    for (key, value) in decision.response_headers {
                                        if let (Ok(name), Ok(val)) = (
                                            hyper::header::HeaderName::try_from(key.as_str()),
                                            hyper::header::HeaderValue::try_from(value.as_str()),
                                        ) {
                                            response.headers_mut().append(name, val);
                                        }
                                    }
                                    Ok(response)
                                }
                                (result, _) => result,
                            }
                        }
                    });

                    // `hyper_util::server::conn::auto::Builder` auto-detects
                    // HTTP/1.1 vs HTTP/2 (h2c prior knowledge) from the first
                    // bytes the client sends. h1 connections go through the
                    // same `http1::Builder` as before; h2c connections get a
                    // multiplexed stream handler with one TCP connection
                    // carrying N concurrent requests. SEC-045's 10 s header
                    // read timeout still applies on the h1 path.
                    //
                    // `Builder::new` takes an executor (used by h2 to spawn
                    // stream tasks), not the IO — the IO goes into
                    // `serve_connection_with_upgrades` below.
                    let mut builder = auto::Builder::new(hyper_util::rt::TokioExecutor::new());
                    // Configure h1: bound the header read timeout so slowloris
                    // attackers don't pin accept slots. h2c has its own
                    // header-equivalent timeouts inside hyper.
                    builder
                        .http1()
                        .timer(TokioTimer::new())
                        .header_read_timeout(Duration::from_secs(10));
                    // MUST be the `_with_upgrades` variant: plain
                    // `serve_connection` never performs the HTTP/1.1 protocol
                    // upgrade after a 101, so every WebSocket (live reload,
                    // /ws/* routes, LiveView, presence) dies with
                    // "Handshake not finished". h2 streams are unaffected by
                    // the wrapper — it only arms the h1 upgrade path.
                    if let Err(_e) = builder.serve_connection_with_upgrades(io, service).await {
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
        let watch_services_dir = folder.join("app/services");
        let watch_mailers_dir = folder.join("app/mailers");
        let watch_jobs_dir = jobs_dir.clone();
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
                    .watch(&watch_controllers_dir, RecursiveMode::Recursive)
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
            // Recursive: models/services load recursively, so nested files
            // (e.g. app/models/billing/invoice.sl) must hot-reload too. The
            // event handler matches by `path.starts_with(dir)`, which already
            // covers nested paths.
            if watch_models_dir.exists()
                && watcher
                    .watch(&watch_models_dir, RecursiveMode::Recursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if watch_services_dir.exists()
                && watcher
                    .watch(&watch_services_dir, RecursiveMode::Recursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if watch_mailers_dir.exists()
                && watcher
                    .watch(&watch_mailers_dir, RecursiveMode::Recursive)
                    .is_ok()
            {
                watch_count += 1;
            }
            if watch_jobs_dir.exists()
                && watcher
                    .watch(&watch_jobs_dir, RecursiveMode::Recursive)
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
                            matches!(ext, "sl" | "erb" | "slv" | "md")
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
                let mut jobs_changed = false;
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
                        } else if name.ends_with(".sl") && path.starts_with(&watch_services_dir) {
                            // Reuse the models signal — workers already reload
                            // app/services/ in the same step (see app_loader).
                            models_changed = true;
                        } else if name.ends_with(".sl") && path.starts_with(&watch_mailers_dir) {
                            // Workers reload app/mailers/ in the same step as
                            // models (see worker reload path).
                            models_changed = true;
                        } else if name.ends_with("_job.sl") && path.starts_with(&watch_jobs_dir) {
                            jobs_changed = true;
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
                if jobs_changed {
                    hot_reload_versions_for_watcher
                        .jobs
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled jobs reload to all workers");
                }
                if views_changed {
                    hot_reload_versions_for_watcher
                        .views
                        .fetch_add(1, Ordering::Release);
                    println!("   ✓ Signaled template cache clear to all workers");
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

                // Bump the generation after the per-kind counters (Release) so
                // a worker that observes it (Acquire) also observes every
                // per-kind bump above. Workers only scan the individual
                // counters when this one moved. A bump where no individual
                // counter moved (e.g. the static_files special case) is
                // harmless — the scan just finds nothing to do.
                if controllers_changed
                    || middleware_changed
                    || helpers_changed
                    || models_changed
                    || jobs_changed
                    || views_changed
                    || static_files_changed
                    || routes_changed
                {
                    hot_reload_versions_for_watcher
                        .generation
                        .fetch_add(1, Ordering::Release);
                }

                // Recompile Tailwind CSS when source files change (views may
                // introduce new classes, asset CSS may have new directives).
                // This blocks the watcher thread — possibly for seconds on a
                // cold run (binary download, first compile) — so it must run
                // AFTER the version bumps above or workers keep serving stale
                // cached bodies until Tailwind finishes. It still runs before
                // the browser-reload send (the browser must refetch the new
                // CSS) and before the debounce drain (which swallows the
                // watcher events Tailwind's own writes generate).
                if views_changed || asset_css_changed || controllers_changed || helpers_changed {
                    tailwind::compile_tailwind_css_once(&watch_folder);
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

    println!("\nServer listening on http://{}:{}", bind_host, actual_port);
    // The LAN URL only exists when listening on all interfaces; when
    // SOLI_HOST pins the server to loopback (or one address), advertising a
    // LAN address that won't answer would be misleading.
    if bind_host.is_unspecified() {
        if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if socket.connect("8.8.8.8:80").is_ok() {
                if let Ok(addr) = socket.local_addr() {
                    println!("  Local network:    http://{}:{}", addr.ip(), actual_port);
                }
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

    // Make the long-lived runtime handle reachable from this boot thread so
    // the session warmup below (and any other block_on helper) drives work on
    // it rather than a transient fallback runtime.
    set_tokio_handle(runtime_handle.clone());

    // Pre-warm the session store's backend connection (no-op for in-memory /
    // disk) AND drive the `/up` readiness signal. Without warming, the SoliDB
    // session driver opens the process's first connection inside
    // `ensure_session` on the first request, which stalls to the HTTP client
    // timeout (~10s) before recovering. The readiness probe anchors a live
    // pooled connection up front (retrying until the store is reachable) and
    // only reports the slot "ready" at `/up` once a session round-trip
    // succeeds — so soli-proxy's blue/green deploy keeps serving the old slot
    // until the new one can actually serve, instead of promoting it on a bare
    // liveness 200 and switching traffic into the cold-connection window.
    crate::interpreter::builtins::session::spawn_session_readiness_probe(runtime_handle.clone());

    // Keep that session connection warm. The readiness probe above performs
    // the one-shot boot warm; on a quiet server a network-backed session
    // store's pooled connection still idles out between requests (the model DB
    // keep-warm below only pings the model host), so the next request pays a
    // cold reconnect — surfacing as intermittent latency spikes on trivial
    // routes like a `/session/ping` heartbeat. No-op for in-memory/disk.
    crate::interpreter::builtins::session::spawn_session_keep_warm(runtime_handle.clone());

    // Login to SoliDB once to get a JWT token (uses ureq, no tokio needed).
    // Must be after .env loading and DB config init.
    crate::interpreter::builtins::model::core::init_jwt_token();

    // Keep the model-DB connection pool warm. Pooled connections idle out
    // after `SOLI_DB_POOL_IDLE_SECS` (default 90s); on a quiet server the
    // next request then paid a cold DNS + TCP (+ TLS) connect mid-request —
    // intermittent 400ms+ spikes. A periodic read-only ping keeps a live
    // connection pooled (first tick also pre-warms the model DB at boot,
    // which only the session store did before). Only spawned when a DB is
    // explicitly configured: with none of these env vars set the app either
    // has no DB or talks to a loopback default, where a cold connect is
    // sub-millisecond anyway.
    let db_configured = std::env::var("SOLIDB_HOST").is_ok()
        || std::env::var("SOLIDB_USERNAME").is_ok()
        || std::env::var("SOLIDB_API_KEY").is_ok();
    if db_configured {
        crate::interpreter::builtins::model::db_config::spawn_db_keep_warm(&runtime_handle);
    }

    // Partition the pool into HTTP and realtime (WS/LiveView) workers so a
    // burst of realtime events can't starve HTTP request handling and a slow
    // HTTP handler can't delay presence/move broadcasts. `SOLI_WS_WORKERS`
    // (default 1) reserves that many threads for realtime, clamped so at least
    // one HTTP worker always remains. With a single total worker the split
    // collapses (num_rt_workers == 0) and that one worker drains every channel,
    // preserving the prior single-worker behavior.
    let requested_rt_workers = std::env::var("SOLI_WS_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1);
    let num_rt_workers = requested_rt_workers.min(num_workers.saturating_sub(1));
    let num_http_workers = num_workers - num_rt_workers;
    let split_realtime = num_rt_workers > 0;
    if split_realtime {
        println!(
            "Worker pool: {} HTTP + {} realtime (WS/LiveView)",
            num_http_workers, num_rt_workers
        );
    }

    // Background job pool: jobs that opt in with `static background: Bool = true` run
    // here so a long handler acks SolidB immediately (no callback timeout, no
    // duplicate retry) and never occupies a web worker. Only started when jobs
    // are actually active — a callback secret is set and `app/jobs` exists.
    // `SOLI_JOB_WORKERS` sizes it (default 2); 0 disables backgrounding, so all
    // jobs run inline exactly as before.
    {
        let jobs_secret_set = std::env::var("SOLI_WEBHOOK_SECRET")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::env::var("SOLI_JOBS_SECRET")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .is_some();
        let num_job_workers = std::env::var("SOLI_JOB_WORKERS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2);
        if jobs_dir.exists() && jobs_secret_set && num_job_workers > 0 {
            background_jobs::start_pool(background_jobs::PoolConfig {
                models_dir: models_dir.clone(),
                helpers_dir: helpers_dir.clone(),
                views_dir: views_dir.clone(),
                jobs_dir: jobs_dir.clone(),
                routes: worker_routes.clone(),
                runtime_handle: runtime_handle.clone(),
                dev_mode,
                num_workers: num_job_workers,
            });
        }
    }

    for i in 0..num_workers {
        // Role for this worker. When the pool isn't split, every worker drains
        // all channels (prior behavior). Otherwise the first `num_http_workers`
        // serve HTTP and the rest serve realtime events exclusively.
        let (http_enabled, realtime_enabled, role_label) = if !split_realtime {
            (true, true, "worker")
        } else if i < num_http_workers {
            (true, false, "worker")
        } else {
            (false, true, "rt-worker")
        };
        // Every worker shares the one queue (clones of the same receiver),
        // competing to pull whichever request is next.
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
        let jobs_dir = jobs_dir.clone();

        let builder = thread::Builder::new().name(format!("{}-{}", role_label, i));
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
                let jobs_dir = jobs_dir.clone();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut interpreter = Interpreter::new_for_serve();
                    // Mailer/Message base classes available before app load.
                    crate::interpreter::builtins::mailer::ensure_prelude(&mut interpreter);

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
                        jobs_dir,
                        http_enabled,
                        realtime_enabled,
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

/// Pre-compile every top-level handler function in the VM's globals to bytecode,
/// populating each `Function.jit_cache`. This is pure compilation (no handler
/// bodies run, no side effects), so it is safe to call before the worker serves
/// traffic. A function that fails to pre-compile is left cold — the normal
/// JIT-on-first-call path still handles it — so warmup never fails the worker.
fn warm_vm_handlers(worker_id: usize, vm: &crate::vm::Vm) {
    let mut warmed = 0usize;
    // Seed the compiler with the worker's full set of global names so bare
    // assignments inside handlers resolve local-vs-global exactly as the
    // tree-walking interpreter would.
    let global_names: Vec<String> = vm.globals.keys().cloned().collect();
    for value in vm.globals.values() {
        if let crate::interpreter::value::Value::Function(f) = value {
            if crate::vm::vm_calls::jit_compile_function(f, global_names.iter().cloned()).is_ok() {
                warmed += 1;
            }
        }
    }

    // Pre-compile OOP class methods too. Without this, every `call_method_bound`
    // for a class-based controller action would JIT-compile its AST to bytecode
    // on the first request, and `call_method_bound` does not consult the
    // worker-global `known_globals` hint, so the work would re-run on every
    // request. Mirrors the function-handler pass above.
    let mut warmed_methods = 0usize;
    for value in vm.globals.values() {
        if let crate::interpreter::value::Value::Class(class) = value {
            for method in class.methods.borrow().values() {
                if crate::vm::vm_calls::jit_compile_method(method, global_names.iter().cloned())
                    .is_ok()
                {
                    warmed_methods += 1;
                }
            }
        }
    }
    println!(
        "Worker {}: warmed {} handlers and {} class methods",
        worker_id, warmed, warmed_methods
    );
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
    jobs_dir: PathBuf,
    // Pool role. HTTP workers drain `work_rx`; realtime workers drain the
    // WS/LiveView event channels. When the pool isn't split (single-worker
    // deployments) both are true and one worker drains everything.
    http_enabled: bool,
    realtime_enabled: bool,
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

    // Capture every AQL query the request makes so the dev tool can show them.
    // Off in production unless an operator opts a channel in via SOLI_LOG;
    // otherwise the gate is a single relaxed atomic load.
    let log_channels = prod_log::channels();
    crate::interpreter::builtins::model::query_log::set_enabled(
        dev_mode || log_channels.collect_query(),
    );

    // Same for outgoing HTTP.* calls — feeds the dev bar's "http" panel.
    crate::interpreter::builtins::http_log::set_enabled(dev_mode || log_channels.collect_http());

    // Same for SoliKV / Cache (KV.* / Cache.*) commands — feeds the "kv" panel.
    crate::interpreter::builtins::kv_log::set_enabled(dev_mode || log_channels.collect_kv());

    // Phase timers (middleware/view) for the render-breakdown panel.
    phase_log::set_enabled(dev_mode || log_channels.collect_timing());
    middleware_log::set_enabled(dev_mode || log_channels.collect_timing());
    view_log::set_enabled(dev_mode || log_channels.collect_timing());
    // The span flamegraph stays dev-only — it's a visualization the prod
    // log block doesn't consume, and it's the heaviest of the gates.
    span_log::set_enabled(dev_mode);
    // Component prop warnings are a dev-only surface (dev bar + console).
    template_warnings::set_enabled(dev_mode);

    // Matched route per request — feeds the dev bar's "requests" panel and the
    // `X-Soli-Route` response header the client patch reads.
    route_log::set_enabled(dev_mode || log_channels.collect_timing());

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

    // Load services (sibling of models) so integration classes — Stripe,
    // etc. — are visible to controllers loaded later in this worker.
    if let Some(parent) = _models_dir.parent() {
        let services_dir = parent.join("services");
        if services_dir.exists() {
            if let Err(e) = load_models(interpreter, &services_dir) {
                eprintln!("Worker {}: Error loading services: {}", worker_id, e);
            }
        }
        // Load authorization policies (sibling of models) so `authorize(...)`
        // and the `<Model>Policy` classes are visible to controllers.
        let policies_dir = parent.join("policies");
        if policies_dir.exists() {
            if let Err(e) = load_models(interpreter, &policies_dir) {
                eprintln!("Worker {}: Error loading policies: {}", worker_id, e);
            }
        }
        // Load mailers (sibling of models). The Mailer base class is defined by
        // ensure_prelude when the worker interpreter is built.
        let mailers_dir = parent.join("mailers");
        if mailers_dir.exists() {
            if let Err(e) = load_models(interpreter, &mailers_dir) {
                eprintln!("Worker {}: Error loading mailers: {}", worker_id, e);
            }
        }
    }

    // Define DSL helpers for routes (needed for hot reload)
    if let Err(e) = define_routes_dsl(interpreter) {
        eprintln!("Worker {}: Error defining routes DSL: {}", worker_id, e);
    }

    // Ship the framework upload helpers + `AttachmentsController` class
    // BEFORE user controllers load, so a user-defined `AttachmentsController`
    // (or a user `attach_upload`/`detach_upload`/etc.) cleanly overrides the
    // default by being defined later in the same env.
    if let Err(e) = uploads_prelude::define_uploads_prelude(interpreter) {
        eprintln!("Worker {}: Error loading uploads prelude: {}", worker_id, e);
    }

    // Load controllers in this worker so functions are defined in environment.
    // Anything user-defined here shadows the framework prelude above.
    load_controllers_in_worker(worker_id, interpreter, &controllers_dir);

    // Load app/jobs/*_job.sl in this worker so XJob classes are available
    // to the callback dispatcher and to controller code that calls
    // `XJob.perform_later(...)`. Worker 0 also syncs `static cron`
    // declarations to SolidB.
    if jobs_dir.exists() {
        let mut tracker = FileTracker::new();
        app_loader::load_jobs_in_worker(worker_id, interpreter, &jobs_dir, &mut tracker, true);
    }

    let _worker_routes = get_routes();

    // Define `<name>_path` / `<name>_url` helpers in this worker's env from
    // the route table we just received. Must run BEFORE the VM globals copy
    // below so prod mode picks them up. Re-runs on hot reload via the same
    // call inside `reload_routes_in_worker`.
    {
        let mut env = interpreter.environment.borrow_mut();
        crate::interpreter::builtins::named_routes::register_named_route_helpers(&mut env);
    }

    // Create VM for production mode (bytecode execution for handler calls)
    let mut vm: Option<crate::vm::Vm> = if !dev_mode {
        let mut vm = crate::vm::Vm::new();
        // Copy all globals from interpreter environment into VM
        // This includes all native builtins, classes, and user-defined functions
        let all_globals = interpreter.environment.borrow().get_all_bindings();
        for (name, value) in all_globals {
            vm.globals.insert(name, value);
        }
        // Warm the worker: pre-compile every handler function to bytecode now,
        // before this worker accepts traffic. Otherwise each handler is
        // JIT-compiled lazily on its FIRST request, so the first hit to every
        // route pays a one-time compile cost — felt as a "cold start", and
        // paid again per worker (round-robin) and after any worker restart.
        warm_vm_handlers(worker_id, &vm);
        Some(vm)
    } else {
        None
    };

    // Realtime workers drain these; HTTP-only workers leave them `None` so the
    // `if let Some(..)` guards below skip WS/LiveView entirely, and the `select`
    // never offers them an event channel.
    let mut ws_event_rx_inner = realtime_enabled.then_some(ws_event_rx);
    let ws_registry_inner = realtime_enabled.then_some(ws_registry);
    let mut lv_event_rx_inner = realtime_enabled.then_some(lv_event_rx);

    // Track last seen hot reload versions
    let mut last_generation = hot_reload_versions.generation.load(Ordering::Acquire);
    let mut last_controllers_version = hot_reload_versions.controllers.load(Ordering::Acquire);
    let mut last_middleware_version = hot_reload_versions.middleware.load(Ordering::Acquire);
    let mut last_helpers_version = hot_reload_versions.helpers.load(Ordering::Acquire);
    let mut last_models_version = hot_reload_versions.models.load(Ordering::Acquire);
    let mut last_views_version = hot_reload_versions.views.load(Ordering::Acquire);
    let mut last_static_files_version = hot_reload_versions.static_files.load(Ordering::Acquire);
    let mut last_routes_version = hot_reload_versions.routes.load(Ordering::Acquire);
    let mut last_jobs_version = hot_reload_versions.jobs.load(Ordering::Acquire);

    loop {
        // Check for hot reload via the single generation counter: one
        // Acquire load per tick in the steady state. Only when it moved
        // (the watcher bumps it last, with Release, after the per-kind
        // counters) do we scan the individual versions below.
        let current_generation = hot_reload_versions.generation.load(Ordering::Acquire);
        let scan_versions = current_generation != last_generation;
        last_generation = current_generation;

        let (
            current_controllers,
            current_middleware,
            current_helpers,
            current_models,
            current_views,
            current_static_files,
            current_routes,
            current_jobs,
        ) = if scan_versions {
            (
                hot_reload_versions.controllers.load(Ordering::Acquire),
                hot_reload_versions.middleware.load(Ordering::Acquire),
                hot_reload_versions.helpers.load(Ordering::Acquire),
                hot_reload_versions.models.load(Ordering::Acquire),
                hot_reload_versions.views.load(Ordering::Acquire),
                hot_reload_versions.static_files.load(Ordering::Acquire),
                hot_reload_versions.routes.load(Ordering::Acquire),
                hot_reload_versions.jobs.load(Ordering::Acquire),
            )
        } else {
            // Unchanged generation → report the last-seen values so every
            // per-kind `!=` check below is false without touching the
            // shared cache lines.
            (
                last_controllers_version,
                last_middleware_version,
                last_helpers_version,
                last_models_version,
                last_views_version,
                last_static_files_version,
                last_routes_version,
                last_jobs_version,
            )
        };

        // Any hot-reload signal invalidates cached rendered bodies: view edits
        // change the AST, helper/route edits change output without changing the
        // cache key, and static-asset changes alter public_path() version hashes
        // embedded in cached HTML. The watcher only exists in dev mode, so this
        // never fires in production; the LRU is 64 entries, trivial to rebuild.
        if scan_versions {
            crate::template::response_cache::clear_cache();
        }

        if current_controllers != last_controllers_version {
            last_controllers_version = current_controllers;
            // Re-load all controllers
            load_controllers_in_worker(worker_id, interpreter, &controllers_dir);
            // Re-scan controller metadata registry (before/after hooks,
            // layout, inheritance). Without this rescan, modifying an
            // existing hook body updates via the class binding but ADDING
            // a new hook entry would not take effect — the registry used
            // by `execute_before_actions` / `execute_after_actions` keeps
            // its stale startup snapshot and only a full restart picks up
            // the new entry.
            if let Err(e) = crate::interpreter::builtins::controller::registry::scan_controllers(
                &controllers_dir,
            ) {
                eprintln!(
                    "Worker {}: Error rescanning controller metadata: {}",
                    worker_id, e
                );
            }
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
                // Re-warm so the reloaded handlers come back hot instead of
                // cold on their next request.
                warm_vm_handlers(worker_id, vm);
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
            // Drop any cached translation tables — a locale_*.sl edit must be
            // reflected on the next render rather than serving the stale table.
            crate::interpreter::builtins::i18n::clear_table_cache();
            if let Err(e) = crate::interpreter::builtins::template::load_view_helpers(&helpers_dir)
            {
                eprintln!("Worker {}: Error reloading view helpers: {}", worker_id, e);
            }
            // Drop the template builtins env so the next render rebuilds it
            // with the updated helpers seeded into the enclosing scope.
            crate::template::core_eval::reset_builtins_rc();
        }

        if current_models != last_models_version {
            last_models_version = current_models;
            // Re-load all models
            if let Err(e) = load_models(interpreter, &_models_dir) {
                eprintln!("Worker {}: Error reloading models: {}", worker_id, e);
            }
            // Re-load services in lockstep with models — they share a
            // signal (see watcher) and live in app/services/ alongside
            // models in the load order.
            if let Some(parent) = _models_dir.parent() {
                let services_dir = parent.join("services");
                if services_dir.exists() {
                    if let Err(e) = load_models(interpreter, &services_dir) {
                        eprintln!("Worker {}: Error reloading services: {}", worker_id, e);
                    }
                }
                let policies_dir = parent.join("policies");
                if policies_dir.exists() {
                    if let Err(e) = load_models(interpreter, &policies_dir) {
                        eprintln!("Worker {}: Error reloading policies: {}", worker_id, e);
                    }
                }
            }
        }

        if current_jobs != last_jobs_version {
            last_jobs_version = current_jobs;
            if jobs_dir.exists() {
                let mut tracker = FileTracker::new();
                app_loader::load_jobs_in_worker(
                    worker_id,
                    interpreter,
                    &jobs_dir,
                    &mut tracker,
                    true,
                );
            }
            // Update VM globals so production-mode bytecode sees reloaded job classes
            if let Some(ref mut vm) = vm {
                let all_globals = interpreter.environment.borrow().get_all_bindings();
                for (name, value) in all_globals {
                    vm.globals.insert(name, value);
                }
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
            // Drop the template builtins env so the next render rebuilds it
            // and re-seeds `<name>_path` / `<name>_url` helpers from the
            // refreshed route table — otherwise views keep calling the old
            // helpers (or fail to resolve names added in this edit).
            crate::template::core_eval::reset_builtins_rc();
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
        // (HTTP workers only; realtime workers never touch the request queue).
        if http_enabled {
            for _ in 0..server_constants::BATCH_SIZE {
                match work_rx.try_recv() {
                    Ok(mut data) => {
                        crate::interpreter::builtins::streaming::clear_pending_stream();
                        let resp_data = handle_request(interpreter, &mut vm, &mut data, dev_mode);
                        match crate::interpreter::builtins::streaming::take_pending_stream() {
                            Some(spec) => {
                                let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
                                let resp = WorkerResponse::Stream {
                                    status: spec.status,
                                    headers: spec.headers.clone(),
                                    rx,
                                };
                                if let Some(topic) = spec.subscribe_topic.clone() {
                                    // Async pub/sub: register the sender under the
                                    // topic and return — the connection lives as
                                    // the async StreamBody, NOT on this worker, so
                                    // many idle subscribers cost no threads. Events
                                    // arrive via sse_broadcast.
                                    let _ = tx.try_send(b": connected\n\n".to_vec());
                                    crate::interpreter::builtins::streaming::register_subscriber(
                                        &topic, tx,
                                    );
                                    let _ = data.response_tx.send(resp);
                                } else {
                                    // Handler-driven streaming: hold the worker
                                    // while the block runs and feeds chunks.
                                    let id =
                                        crate::interpreter::builtins::streaming::register_sender(
                                            tx,
                                        );
                                    let _ = data.response_tx.send(resp);
                                    crate::interpreter::builtins::streaming::run_stream_block(
                                        interpreter,
                                        &spec,
                                        id,
                                    );
                                    crate::interpreter::builtins::streaming::unregister_sender(id);
                                }
                            }
                            None => {
                                let _ = data.response_tx.send(WorkerResponse::Buffered(resp_data));
                            }
                        }
                    }
                    Err(channel::TryRecvError::Empty) => {
                        break;
                    }
                    Err(channel::TryRecvError::Disconnected) => {
                        return;
                    }
                }
            }
        }

        // Block waiting for events on any channel using crossbeam select.
        // This avoids busy-waiting: the thread sleeps until an event arrives
        // on any channel (or timeout fires for dev-mode hot reload checks).
        {
            let mut sel = channel::Select::new();
            let work_idx = http_enabled.then(|| sel.recv(&work_rx));
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
                if Some(idx) == work_idx {
                    if let Ok(mut data) = oper.recv(&work_rx) {
                        // Check hot reload before handling: a parked worker
                        // serves this request before the loop-top version scan
                        // runs, so clear both the template AST cache and this
                        // thread's rendered-body cache here — otherwise the
                        // first request after a view edit gets the stale body.
                        if dev_mode {
                            let current_views = hot_reload_versions.views.load(Ordering::Acquire);
                            if current_views != last_views_version {
                                last_views_version = current_views;
                                clear_template_cache();
                                crate::template::response_cache::clear_cache();
                            }
                        }
                        crate::interpreter::builtins::streaming::clear_pending_stream();
                        let resp_data = handle_request(interpreter, &mut vm, &mut data, dev_mode);
                        match crate::interpreter::builtins::streaming::take_pending_stream() {
                            Some(spec) => {
                                let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
                                let resp = WorkerResponse::Stream {
                                    status: spec.status,
                                    headers: spec.headers.clone(),
                                    rx,
                                };
                                if let Some(topic) = spec.subscribe_topic.clone() {
                                    // Async pub/sub: register the sender under the
                                    // topic and return — the connection lives as
                                    // the async StreamBody, NOT on this worker, so
                                    // many idle subscribers cost no threads. Events
                                    // arrive via sse_broadcast.
                                    let _ = tx.try_send(b": connected\n\n".to_vec());
                                    crate::interpreter::builtins::streaming::register_subscriber(
                                        &topic, tx,
                                    );
                                    let _ = data.response_tx.send(resp);
                                } else {
                                    // Handler-driven streaming: hold the worker
                                    // while the block runs and feeds chunks.
                                    let id =
                                        crate::interpreter::builtins::streaming::register_sender(
                                            tx,
                                        );
                                    let _ = data.response_tx.send(resp);
                                    crate::interpreter::builtins::streaming::run_stream_block(
                                        interpreter,
                                        &spec,
                                        id,
                                    );
                                    crate::interpreter::builtins::streaming::unregister_sender(id);
                                }
                            }
                            None => {
                                let _ = data.response_tx.send(WorkerResponse::Buffered(resp_data));
                            }
                        }
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
#[allow(clippy::too_many_arguments)]
async fn handle_hyper_request(
    mut req: Request<Incoming>,
    request_tx: WorkerSender,
    reload_tx: Option<broadcast::Sender<()>>,
    public_dir: Arc<PathBuf>,
    asset_cache: asset_cache::AssetCache,
    ws_event_tx: channel::Sender<WebSocketEventData>,
    lv_event_tx: channel::Sender<LiveViewEventData>,
    dev_mode: bool,
    peer_addr: SocketAddr,
) -> Result<Response<ResponseBody>, hyper::Error> {
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
    let request_start = std::time::Instant::now();

    // Increment total request counter before any routing decisions
    crate::metrics::Metrics::global()
        .http_requests_total
        .fetch_add(1, Ordering::Relaxed);

    // Prometheus metrics endpoint — no CSRF check, intended for scraping
    if path == "/_metrics" && method == "GET" {
        let body = crate::metrics::Metrics::global().render_prometheus();
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(full(Bytes::from(body)))
            .unwrap());
    }

    // Built-in LiveView client, embedded in the binary so the client is
    // always in sync with the server's patch protocol (no vendored copy to
    // go stale). `no-cache` + version ETag: browsers revalidate and get a
    // 304 until the binary changes.
    if path == crate::live::LIVE_CLIENT_PATH && (method == "GET" || method == "HEAD") {
        const ETAG: &str = concat!("\"soli-live-", env!("CARGO_PKG_VERSION"), "\"");
        let not_modified = req
            .headers()
            .get("if-none-match")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains(ETAG));
        if not_modified {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header("ETag", ETAG)
                .header("Cache-Control", "no-cache")
                .body(full(Bytes::new()))
                .unwrap());
        }
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/javascript; charset=utf-8")
            .header("Cache-Control", "no-cache")
            .header("ETag", ETAG)
            .body(full(Bytes::from_static(
                crate::live::LIVE_CLIENT_JS.as_bytes(),
            )))
            .unwrap());
    }

    // SEC-014: same-origin gate for state-changing requests. Runs before
    // routing so a cross-origin POST is rejected before any controller
    // sees it. WebSocket upgrades have their own check (`websocket_origin_allowed`)
    // a few lines below, so the early return doesn't fire on those.
    if !hyper_tungstenite::is_upgrade_request(&req) {
        if let Err(reason) = check_csrf_origin(req.headers(), &method, &path) {
            return Ok(forbidden_csrf_response(&reason));
        }
    }

    // Check for WebSocket upgrade request
    if hyper_tungstenite::is_upgrade_request(&req) {
        // Handle live reload WebSocket endpoint
        if path == "/__livereload_ws" {
            if !websocket_origin_allowed(req.headers()) {
                return Ok(forbidden_websocket_origin_response());
            }
            if let Some(ref tx) = reload_tx {
                return live_reload_ws::handle_live_reload_websocket(req, tx.subscribe())
                    .await
                    .map(box_full);
            } else {
                // Live reload disabled
                return Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(full(Bytes::from("Live reload is disabled")))
                    .unwrap());
            }
        }

        // Handle LiveView WebSocket endpoint
        if path == "/live/socket" || path.starts_with("/live/socket/") {
            if !websocket_origin_allowed(req.headers()) {
                return Ok(forbidden_websocket_origin_response());
            }

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
            let ws_config = default_websocket_config();
            let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, Some(ws_config))
            {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("[LiveView] Upgrade error: {}", e);
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(full(Bytes::from(format!("WebSocket upgrade error: {}", e))))
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
                let liveview_id = format!("{}:{}", session_id, component);
                handle_live_connection(component.clone(), session_id, tx_arc.clone());

                // Fire a synthetic `connect` event so user handlers can seed
                // initial state and request a tick interval. We fire-and-forget
                // here — the worker still posts a response, but no one needs to
                // await it (the receiver gets dropped).
                if crate::live::socket::get_liveview_handler(&component).is_some() {
                    let (response_tx, _response_rx) = oneshot::channel();
                    let _ = lv_event_tx.try_send(LiveViewEventData {
                        liveview_id: liveview_id.clone(),
                        component: component.clone(),
                        event: "connect".to_string(),
                        params: serde_json::json!({}),
                        response_tx,
                    });
                }

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
                                        } else if event_type == "resync" {
                                            // The client lost its shadow copy (or a splice
                                            // failed to apply): replay the last full render
                                            // so it can rebuild without resetting server state.
                                            if let Some(id) = liveview_id {
                                                use crate::live::view::{
                                                    ServerMessage, LIVE_REGISTRY,
                                                };
                                                if let Some(instance) = LIVE_REGISTRY.get(&id) {
                                                    let _ = LIVE_REGISTRY.send(
                                                        &id,
                                                        ServerMessage::Render {
                                                            html: instance.last_html,
                                                            liveview_id: id.clone(),
                                                        },
                                                    );
                                                }
                                            }
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
                crate::live::socket::cancel_tick_task(&liveview_id);
            });

            return Ok(box_full(response));
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
                .body(full(Bytes::from("WebSocket endpoint not found")))
                .unwrap());
        }
    }

    // Check for static file in public directory
    if method == "GET" && public_dir.exists() {
        match resolve_static_file(&path, &public_dir) {
            Err(()) => {
                return Ok(Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(full(Bytes::from("Forbidden")))
                    .unwrap());
            }
            Ok(Some(file_path)) => {
                let mime_type = server_constants::get_mime_type(&file_path);

                // Production fast path: serve cached CSS/JS bytes loaded at startup.
                // The cache is populated only in prod (`!dev_mode`); a miss here
                // (e.g. images, fonts, files added post-startup) falls through to
                // the disk-read path below.
                if !dev_mode {
                    let canonical =
                        std::fs::canonicalize(&file_path).unwrap_or_else(|_| file_path.clone());
                    if let Some(asset) = asset_cache.get(&canonical) {
                        // Conditional GET: 304 short-circuit on matching ETag.
                        if let Some(if_none_match) = req.headers().get("if-none-match") {
                            if let Ok(client_etag) = if_none_match.to_str() {
                                if client_etag == asset.etag
                                    || client_etag == format!("W/{}", asset.etag)
                                {
                                    return Ok(Response::builder()
                                        .status(StatusCode::NOT_MODIFIED)
                                        .header("ETag", &asset.etag)
                                        .header(
                                            "Cache-Control",
                                            server_constants::STATIC_CACHE_MAX_AGE,
                                        )
                                        .body(full(Bytes::new()))
                                        .unwrap());
                                }
                            }
                        }

                        let total_size = asset.bytes.len() as u64;

                        // Range support: slice cheaply from refcounted Bytes.
                        if let Some(range_header) = req.headers().get("range") {
                            if let Ok(range_str) = range_header.to_str() {
                                if let Some((start, end)) =
                                    server_constants::parse_range_header(range_str, total_size)
                                {
                                    let length = end - start + 1;
                                    let slice = asset.bytes.slice(start as usize..=(end as usize));
                                    return Ok(Response::builder()
                                        .status(StatusCode::PARTIAL_CONTENT)
                                        .header("Content-Type", asset.content_type)
                                        .header(
                                            "Content-Range",
                                            format!("bytes {}-{}/{}", start, end, total_size),
                                        )
                                        .header("Content-Length", length.to_string())
                                        .header("Accept-Ranges", "bytes")
                                        .header("ETag", &asset.etag)
                                        .header(
                                            "Cache-Control",
                                            server_constants::STATIC_CACHE_MAX_AGE,
                                        )
                                        .body(full(slice))
                                        .unwrap());
                                } else {
                                    return Ok(Response::builder()
                                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                                        .header("Content-Range", format!("bytes */{}", total_size))
                                        .body(full(Bytes::new()))
                                        .unwrap());
                                }
                            }
                        }

                        return Ok(Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", asset.content_type)
                            .header("Content-Length", asset.bytes.len().to_string())
                            .header("Accept-Ranges", "bytes")
                            .header("ETag", &asset.etag)
                            .header("Cache-Control", server_constants::STATIC_CACHE_MAX_AGE)
                            .body(full(asset.bytes.clone()))
                            .unwrap());
                    }
                }

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
                                            .body(full(Bytes::new()))
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
                                        // SEC-048: open + seek + bounded read so a tiny
                                        // Range against a large asset doesn't slurp the
                                        // whole file into memory each request.
                                        let slice = match server_constants::read_file_range(
                                            &file_path, start, length,
                                        ) {
                                            Ok(buf) => buf,
                                            Err(_) => {
                                                return Ok(Response::builder()
                                                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                                                    .body(full(Bytes::from("Error reading file")))
                                                    .unwrap())
                                            }
                                        };
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
                                            .body(full(Bytes::from(slice)))
                                            .unwrap());
                                    } else {
                                        // Range not satisfiable
                                        return Ok(Response::builder()
                                            .status(StatusCode::RANGE_NOT_SATISFIABLE)
                                            .header(
                                                "Content-Range",
                                                format!("bytes */{}", file_size),
                                            )
                                            .body(full(Bytes::new()))
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
                                        .body(full(Bytes::from("Error reading file")))
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
                                .body(full(Bytes::from(content)))
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
                            .body(full(Bytes::from("Error reading file")))
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
                                .body(full(Bytes::copy_from_slice(slice)))
                                .unwrap());
                        } else {
                            return Ok(Response::builder()
                                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                                .header("Content-Range", format!("bytes */{}", file_size))
                                .body(full(Bytes::new()))
                                .unwrap());
                        }
                    }
                }

                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", mime_type)
                    .header("Content-Length", content.len().to_string())
                    .header("Accept-Ranges", "bytes")
                    .body(full(Bytes::from(content)))
                    .unwrap());
            }
            Ok(None) => {} // Not a static file, fall through to route matching
        }
    }

    // Framework-bundled hover-prefetch script. Served at a reserved path so
    // strict-CSP apps can use `<script src>` instead of inline JS.
    if path == "/__soli/prefetch.js" && method == "GET" {
        return Ok(box_full(prefetch::handle_prefetch_js()));
    }

    // Framework-bundled instant-navigation script (body swap + pushState).
    if path == "/__soli/nav.js" && method == "GET" {
        return Ok(box_full(nav::handle_nav_js()));
    }

    // Handle live reload SSE endpoint
    if path == "/__livereload" {
        // SEC-043: gate the dev-only SSE endpoint by Origin, mirroring
        // the WebSocket variant a few lines above. Without this any
        // browser tab on any origin can open the long-poll, hold a
        // worker for 55 s per connection, and fan out hundreds in
        // parallel to exhaust the broadcast channel + worker pool.
        // `websocket_origin_allowed` requires Origin whenever a Cookie
        // is present (SEC-046) and otherwise requires it to match
        // `Host`; cookie-less curl from the dev box still works.
        if !websocket_origin_allowed(req.headers()) {
            return Ok(Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(full(Bytes::from("Forbidden live-reload origin")))
                .unwrap());
        }
        if let Some(ref tx) = reload_tx {
            return Ok(box_full(
                live_reload::handle_live_reload_sse(tx.subscribe()).await,
            ));
        } else {
            // Live reload disabled
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(full(Bytes::from("Live reload is disabled")))
                .unwrap());
        }
    }

    // Development mode endpoints
    if dev_mode {
        // REPL endpoint
        if path == "/__dev/repl" && method == "POST" {
            return handle_dev_repl(req, peer_addr).await;
        }
        // Source code endpoint
        if path == "/__dev/source" && method == "GET" {
            return handle_dev_source(req, peer_addr).await;
        }
        // Per-request dev snapshot: the dev bar's requests panel fetches this to
        // re-render a listed request's panels (db / http / kv / flame). Reads
        // the process-wide store (populated on the worker thread at finalize),
        // so it's safe to serve straight from the async handler. Dev-only; the
        // store is empty in production so this never leaks anything.
        if method == "GET" {
            if let Some(id) = path.strip_prefix("/__solidev/request/") {
                return Ok(match dev_store::get(id) {
                    Some(ctx) => Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "text/html; charset=utf-8")
                        .body(full(Bytes::from(dev_bar::render_for_inspect(&ctx))))
                        .unwrap(),
                    None => Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(full(Bytes::from("unknown or expired request id")))
                        .unwrap(),
                });
            }
        }
        // Replay a captured request server-side to reproduce a bug. Re-dispatches
        // the stored raw request through the real worker path (fresh request id,
        // handler re-runs). The `/_`-prefixed path is exempt from the origin gate
        // above, and the replay flag bypasses the worker's per-form CSRF token.
        if method == "POST" {
            if let Some(id) = path.strip_prefix("/__solidev/replay/") {
                return Ok(handle_replay(id, &request_tx).await);
            }
        }
        // Component preview catalog (Lookbook-style), dev-only.
        if method == "GET" && path == "/__soli/components" {
            return Ok(handle_component_catalog());
        }
        if method == "GET" {
            if let Some(name) = path.strip_prefix("/__soli/components/") {
                return Ok(handle_component_preview(name));
            }
        }
        // Mailer preview gallery, dev-only.
        if method == "GET" && path == "/__soli/mailers" {
            return Ok(handle_mailer_catalog());
        }
        if method == "GET" {
            if let Some(rel) = path.strip_prefix("/__soli/mailers/") {
                return Ok(handle_mailer_preview(rel));
            }
        }
        // Database browser, dev-only: list collections, page rows, view a
        // document, run a read-only query. Sync DB calls are wrapped in
        // block_in_place inside the handlers (this is an async hyper task).
        if method == "GET" && path == "/__soli/db" {
            return Ok(handle_db_index(req.uri().query()));
        }
        if method == "GET" {
            if let Some(rest) = path.strip_prefix("/__soli/db/") {
                return Ok(match rest.split_once('/') {
                    Some((coll, key)) => handle_db_document(coll, key),
                    None => handle_db_collection(rest, req.uri().query()),
                });
            }
        }
    }

    // Coverage dump endpoint: only active when the parent process asked us
    // to collect coverage (via SOLI_COVERAGE_ENABLED). Returns a JSON blob
    // the test runner merges into its own aggregated report.
    //
    // SEC-080: gate the dump on a per-process `SOLI_COVERAGE_TOKEN`. The
    // test runner mints a fresh random token, hands it to each child via
    // env, and presents it as `X-Coverage-Token` when scraping. Coverage
    // accidentally enabled in production would otherwise let any remote
    // client read source paths and line-hit counts; with the token gate
    // an unauthenticated GET returns 403, even if `SOLI_COVERAGE_ENABLED`
    // is set. The token is required — running without it (legacy callers,
    // misconfiguration) is rejected too, so the endpoint is never open.
    if path == "/__coverage__" && method == "GET" && std::env::var("SOLI_COVERAGE_ENABLED").is_ok()
    {
        let expected = std::env::var("SOLI_COVERAGE_TOKEN")
            .ok()
            .filter(|t| !t.is_empty());
        let provided = req
            .headers()
            .get("x-coverage-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !coverage_request_authorized(expected.as_deref(), provided) {
            return Ok(Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(full(Bytes::from(
                    "coverage endpoint requires X-Coverage-Token matching SOLI_COVERAGE_TOKEN",
                )))
                .unwrap());
        }
        let body = coverage_dump_json();
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(body)))
            .unwrap());
    }

    let query_str = uri.query().unwrap_or("");

    // Parse query string into ordered pairs (order matters for bracket
    // arrays like tags[]=a&tags[]=b — the worker nests them Rack-style).
    let query = parse_query_pairs(query_str);

    // Split the request: take ownership of the wire headers (moved into
    // RequestData as-is — `HeaderMap` is Send) and the body stream. No
    // per-header String copies happen here; the worker converts the map to
    // Soli HashPairs exactly once when it builds `req["headers"]`.
    let (parts, req_body) = req.into_parts();
    let headers = parts.headers;

    // Keep the conditional-GET validator around so the response-assembly
    // block below can short-circuit to 304 when the controller's rendered
    // ETag matches the browser's cached copy.
    let if_none_match = header_str(&headers, "if-none-match").map(|v| v.to_owned());

    // Is this a browser speculative prefetch (hover-preload)? If so the
    // response-assembly block below relaxes the HTML `Cache-Control` so the
    // eventual click reuses the prefetched bytes without a revalidation
    // round-trip — see `prefetch::prefetch_cache_control`.
    let is_prefetch =
        crate::serve::prefetch::is_prefetch_request(|name| header_str(&headers, name));

    // Content headers used by the body-reading block below.
    let declared_content_length =
        header_str(&headers, "content-length").and_then(|v| v.parse::<usize>().ok());
    let req_content_type = header_str(&headers, "content-type").map(|v| v.to_owned());

    // Read body - skip for GET/HEAD requests (usually empty). Cap the
    // read so a hostile client can't exhaust worker memory by streaming
    // an unbounded body. Content-Length lets us short-circuit before any
    // bytes are buffered; chunked uploads (no Content-Length) are caught
    // mid-stream by `Limited`.
    let max_body = crate::interpreter::builtins::body_limit::get_max_body_size();
    if method != "GET" && method != "HEAD" {
        if let Some(declared) = declared_content_length {
            if declared > max_body {
                return Ok(Response::builder()
                    .status(StatusCode::PAYLOAD_TOO_LARGE)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(full(Bytes::from("Request body too large")))
                    .unwrap());
            }
        }
    }
    let (body, body_bytes_opt, multipart_form, multipart_files) =
        if method == "GET" || method == "HEAD" {
            (String::new(), None, None, None)
        } else {
            let collected = BodyExt::collect(Limited::new(req_body, max_body)).await;
            let body_bytes = match collected {
                Ok(b) => b.to_bytes().to_vec(),
                Err(_) => {
                    // `Limited` returns an error once the running total
                    // crosses `max_body`. Treat any failure here as oversize:
                    // we can't reliably distinguish a transport error from a
                    // length-limit hit, but in either case we don't want to
                    // proceed with a partial body.
                    return Ok(Response::builder()
                        .status(StatusCode::PAYLOAD_TOO_LARGE)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .body(full(Bytes::from("Request body too large")))
                        .unwrap());
                }
            };

            // Check if this is a multipart form
            let content_type = req_content_type.as_deref();
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

    // HTML forms can only express GET and POST. Rails-style method override:
    // a POST whose form body carries `_method=PUT|PATCH|DELETE` (the hidden
    // input `form_with` / `button_to` and the scaffold emit) is routed and
    // dispatched to the app as that verb. Applied after the CSRF origin gate
    // above — the overridden verbs are state-changing either way.
    let method = apply_form_method_override(
        method,
        &body,
        req_content_type.as_deref(),
        multipart_form.as_deref(),
    );

    // Create oneshot channel for response
    let (response_tx, response_rx) = oneshot::channel();

    // Keep copies for the response-timeout log below (both fields are moved
    // into RequestData). `method` is a Cow and `path` a String — cheap clones.
    let log_method = method.clone();
    let log_path = path.clone();

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
        peer_ip: peer_addr.ip().to_string(),
        enqueued_at: prod_log::channels().any().then(std::time::Instant::now),
        replay: false,
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
            .body(full(Bytes::from("Server busy")))
            .unwrap());
    }

    // Wait for response, bounded by RESPONSE_WAIT_TIMEOUT_SECS. The worker
    // reply is otherwise awaited with no timeout: a worker parked in a
    // blocking DB/HTTP call or a lock would hang this request forever
    // ("pending" in the browser, system idle). On timeout we free the
    // connection with a 504 and log which route stalled. Dropping
    // `response_rx` here is safe — the worker's reply send is a discarded
    // `let _ = ...send(...)`, so it won't panic on a closed receiver.
    match tokio::time::timeout(
        Duration::from_secs(server_constants::RESPONSE_WAIT_TIMEOUT_SECS),
        response_rx,
    )
    .await
    {
        Err(_) => {
            eprintln!(
                "[WARN] layer=lang_serve method={} path={} timeout_secs={} elapsed_ms={} \
                 worker response timed out; returning 504",
                log_method,
                log_path,
                server_constants::RESPONSE_WAIT_TIMEOUT_SECS,
                request_start.elapsed().as_millis(),
            );
            Ok(Response::builder()
                .status(StatusCode::GATEWAY_TIMEOUT)
                .header("Server", "soliMVC")
                .body(full(Bytes::from("Gateway Timeout")))
                .unwrap())
        }
        Ok(Ok(worker_response)) => {
            // Streaming responses (SSE / chunked) bypass the buffered path
            // entirely: build a chunked body fed by the worker's channel.
            let resp_data = match worker_response {
                WorkerResponse::Stream {
                    status,
                    headers,
                    rx,
                } => {
                    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|chunk| {
                        Ok::<_, std::io::Error>(hyper::body::Frame::data(Bytes::from(chunk)))
                    });
                    let body = BodyExt::boxed(StreamBody::new(stream));
                    let mut builder = Response::builder()
                        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
                        .header("Server", "soliMVC");
                    for (key, value) in &headers {
                        builder = builder.header(key, value);
                    }
                    return Ok(builder.body(body).unwrap_or_else(|_| {
                        Response::new(full(Bytes::from("stream init error")))
                    }));
                }
                WorkerResponse::Buffered(rd) => rd,
            };
            // Conditional-GET short-circuit: if the controller produced an
            // ETag matching the browser's If-None-Match, return 304 with
            // just the validator headers. Enables the hover-prefetch feature
            // to deliver "instant navigation" — the body is already in the
            // prefetched-resources cache; revalidation costs one tiny round
            // trip instead of re-sending tens of KB of HTML.
            //
            // Skipped in --dev: the dev bar is injected after the ETag is
            // computed, so a 304 would replay an HTML snapshot with stale
            // bar contents (old timings, old query log, old req counter).
            if !dev_mode {
                if let Some(ref client_etag) = if_none_match {
                    if let Some(server_etag) = resp_data.headers.iter().find_map(|(k, v)| {
                        if k.eq_ignore_ascii_case("etag") {
                            Some(v.as_str())
                        } else {
                            None
                        }
                    }) {
                        fn strip_weak(s: &str) -> &str {
                            s.trim_start_matches("W/").trim()
                        }
                        if strip_weak(client_etag) == strip_weak(server_etag) {
                            let mut b304 = Response::builder()
                                .status(StatusCode::NOT_MODIFIED)
                                .header("Server", "soliMVC");
                            // RFC 7232 §4.1: 304 MUST include the ETag it validated
                            // against and SHOULD include Cache-Control so the
                            // browser knows the freshness semantics for the next
                            // reuse.
                            for (key, value) in &resp_data.headers {
                                if key.eq_ignore_ascii_case("etag")
                                    || key.eq_ignore_ascii_case("cache-control")
                                    || key.eq_ignore_ascii_case("vary")
                                {
                                    b304 = add_header_checked(b304, key.as_str(), value.as_str());
                                }
                            }
                            return Ok(finish_response(b304, Bytes::new()));
                        }
                    }
                }
            }

            let mut builder = Response::builder()
                .status(StatusCode::from_u16(resp_data.status).unwrap_or(StatusCode::OK))
                .header("Server", "soliMVC");

            // For a speculative prefetch of an HTML page, swap the page's
            // `private, no-cache` for a short `private, max-age=N` so the click
            // serves the prefetched bytes straight from the browser cache — no
            // conditional GET, so a CDN that won't relay a 304 (Cloudflare et
            // al.) can't break instant navigation. The ETag still rides along
            // for revalidation once the window lapses.
            let prefetch_cache_control = if is_prefetch
                && resp_data
                    .headers
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("text/html"))
            {
                Some(crate::serve::prefetch::prefetch_cache_control())
            } else {
                None
            };

            for (key, value) in &resp_data.headers {
                if let Some(ref cache_control) = prefetch_cache_control {
                    if key.eq_ignore_ascii_case("cache-control") {
                        builder = add_header_checked(builder, key.as_str(), cache_control.as_str());
                        continue;
                    }
                }
                builder = add_header_checked(builder, key.as_str(), value.as_str());
            }

            // Inject live reload script for HTML responses (only in dev mode).
            // HTML is UTF-8, so we can safely view the body as &str for injection.
            // Binary responses (images/files) skip this path via the content-type guard.
            let body: Vec<u8> = if reload_tx.is_some() {
                let is_html = resp_data.headers.iter().any(|(k, v)| {
                    k.eq_ignore_ascii_case("content-type") && v.contains("text/html")
                });
                if is_html {
                    match std::str::from_utf8(&resp_data.body) {
                        Ok(html) => live_reload::inject_live_reload_script(html).into_bytes(),
                        Err(_) => resp_data.body,
                    }
                } else {
                    resp_data.body
                }
            } else {
                resp_data.body
            };

            Ok(finish_response(builder, Bytes::from(body)))
        }
        Ok(Err(_)) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(full(Bytes::from("Internal Server Error")))
            .unwrap()),
    }
}

/// Check if the request is a WebSocket upgrade request.
fn forbidden_websocket_origin_response() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(full(Bytes::from("Forbidden WebSocket origin")))
        .unwrap()
}

/// SEC-047: tungstenite's default caps are 64 MiB per message and 16 MiB
/// per frame. Combined with the per-connection mpsc channel of 32
/// pending messages, a few hundred connections drip-feeding maximum-size
/// payloads can pin tens of GiB of worker memory. 1 MiB is plenty for
/// live-reload signals, LiveView event payloads, and form-shaped data;
/// large blobs (file uploads, images) belong on HTTP, not WS.
pub(crate) fn default_websocket_config() -> hyper_tungstenite::tungstenite::protocol::WebSocketConfig
{
    hyper_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(1 << 20),
        max_frame_size: Some(1 << 20),
        ..Default::default()
    }
}

/// SEC-014: app-registered CSRF exemption patterns. Populated from Soli
/// code via `skip_csrf("/path[/*]")` (typically called from
/// `config/routes.sl` or a controller's `static` block before route
/// matching runs). Each entry is a path pattern; `*` suffix means "any
/// path that starts with this prefix".
///
/// `RwLock` so writes from the boot phase don't block the request hot
/// path's reads.
static CSRF_SKIP_PATTERNS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

pub fn register_csrf_skip_pattern(pattern: String) {
    if let Ok(mut guard) = CSRF_SKIP_PATTERNS.write() {
        if !guard.iter().any(|p| p == &pattern) {
            guard.push(pattern);
        }
    }
}

#[cfg(test)]
fn clear_csrf_skip_patterns() {
    if let Ok(mut guard) = CSRF_SKIP_PATTERNS.write() {
        guard.clear();
    }
}

fn csrf_skipped_by_app(path: &str) -> bool {
    let Ok(guard) = CSRF_SKIP_PATTERNS.read() else {
        return false;
    };
    guard.iter().any(|pattern| {
        if let Some(prefix) = pattern.strip_suffix("/*") {
            path == prefix || path.starts_with(&format!("{}/", prefix))
        } else if let Some(prefix) = pattern.strip_suffix('*') {
            path.starts_with(prefix)
        } else {
            path == pattern
        }
    })
}

/// `SOLI_DISABLE_CSRF` operator kill switch — turns off both the
/// Origin/Referer gate and per-form token verification.
fn csrf_disabled_by_env() -> bool {
    std::env::var("SOLI_DISABLE_CSRF")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// `SOLI_CSRF_TOKENS=require` strict mode: browser form posts
/// (urlencoded/multipart bodies) MUST carry a valid per-form token.
fn csrf_tokens_required() -> bool {
    std::env::var("SOLI_CSRF_TOKENS")
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("require"))
        .unwrap_or(false)
}

/// Does this content type mark a browser form submission?
fn is_form_content_type(content_type: &str) -> bool {
    content_type.starts_with("application/x-www-form-urlencoded")
        || content_type.starts_with("multipart/form-data")
}

/// Extract a single field from an `application/x-www-form-urlencoded` body
/// (percent-decoded via the shared query-string parser; form bodies are
/// small, so the full parse is cheap).
fn form_body_param(body: &str, name: &str) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    parse_query_string(body).remove(name)
}

/// Rails-style method override: HTML forms can only express GET/POST, so a
/// POST whose form body carries `_method` is treated as that verb. Only the
/// three verbs a form can't express are honored — anything else (including
/// an attempt to downgrade to GET and dodge CSRF checks) is ignored.
fn apply_form_method_override(
    method: Cow<'static, str>,
    body: &str,
    content_type: Option<&str>,
    multipart_form: Option<&[(String, String)]>,
) -> Cow<'static, str> {
    if method != "POST" {
        return method;
    }
    let requested = match content_type {
        Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => {
            form_body_param(body, "_method")
        }
        Some(ct) if ct.starts_with("multipart/form-data") => multipart_form
            .and_then(|form| form.iter().find(|(k, _)| k == "_method"))
            .map(|(_, v)| v.clone()),
        _ => None,
    };
    match requested.as_deref().map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("PUT") => Cow::Borrowed("PUT"),
        Some(v) if v.eq_ignore_ascii_case("PATCH") => Cow::Borrowed("PATCH"),
        Some(v) if v.eq_ignore_ascii_case("DELETE") => Cow::Borrowed("DELETE"),
        _ => method,
    }
}

/// Per-form CSRF token verification, run on the worker (where the session
/// lives) after the session ID is resolved. Complements the Origin/Referer
/// gate that already ran in the hyper layer:
///
/// - A request that **carries** a token (`_csrf_token` form field from
///   `csrf_field()` / `X-CSRF-Token` header from `csrf_meta_tag()`) must
///   present the session's token — a mismatch or a token-less session is a
///   403 even when Origin passed.
/// - A request with **no** token stays on the Origin/Referer posture,
///   unless `SOLI_CSRF_TOKENS=require` makes tokens mandatory for browser
///   form posts (JSON/API traffic is never token-gated; use `skip_csrf`
///   or the header for API clients that opt in).
fn verify_csrf_token(data: &RequestData, method: &str, path: &str) -> Result<(), String> {
    if matches!(method, "GET" | "HEAD" | "OPTIONS") {
        return Ok(());
    }
    if path.starts_with("/_") || csrf_skipped_by_app(path) || csrf_disabled_by_env() {
        return Ok(());
    }

    let content_type = header_str(&data.headers, "content-type").unwrap_or("");
    let supplied = header_str(&data.headers, "x-csrf-token")
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| {
            if content_type.starts_with("application/x-www-form-urlencoded") {
                form_body_param(&data.body, "_csrf_token")
            } else if content_type.starts_with("multipart/form-data") {
                data.multipart_form
                    .as_ref()
                    .and_then(|form| form.iter().find(|(k, _)| k == "_csrf_token"))
                    .map(|(_, v)| v.clone())
            } else {
                None
            }
        });

    match supplied {
        Some(token) => match crate::interpreter::builtins::session::current_csrf_token() {
            Some(expected)
                if crate::interpreter::builtins::crypto::do_secure_compare(&expected, &token) =>
            {
                Ok(())
            }
            Some(_) => Err("CSRF token does not match the session's".to_string()),
            None => Err(
                "CSRF token supplied but the session holds none (expired or new session)"
                    .to_string(),
            ),
        },
        None if csrf_tokens_required() && is_form_content_type(content_type) => {
            Err("missing CSRF token (SOLI_CSRF_TOKENS=require)".to_string())
        }
        None => Ok(()),
    }
}

/// SEC-014: reject state-changing browser requests that can't prove they
/// originate from the same site. Returns `Ok(())` to continue, `Err(reason)`
/// to reject with 403.
///
/// Rules:
/// - Safe methods (GET/HEAD/OPTIONS) are always allowed.
/// - Paths under `/_` are exempt (machine-to-machine endpoints like
///   `/_jobs/run/:name` carry their own HMAC auth).
/// - Paths matching a `skip_csrf("/pattern[/*]")` declaration in user
///   Soli code are exempt. This is the per-route opt-out — call it
///   from `config/routes.sl` or a controller's `static` block for
///   webhook endpoints, public APIs, etc.
/// - `SOLI_DISABLE_CSRF=true` operator-level kill switch — for API-only
///   deployments where no cookie session is in play.
/// - When `Origin` is present, it must equal the request authority
///   (`Host`/`X-Forwarded-Host`). `null` Origin (sandboxed iframe etc.)
///   is rejected.
/// - When `Origin` is absent but `Referer` is present, the Referer's
///   authority must match.
/// - When **neither** is present, the decision branches on the
///   `Cookie` header (SEC-078). Cookie-bearing requests get rejected
///   because they have no proof of same-site provenance — the threat
///   surface is exactly a stripped UA / proxy / Origin-less form POST
///   replaying the session cookie. Cookie-less requests stay on the
///   non-browser API path and are allowed; route-level opt-outs via
///   `skip_csrf("/path[/*]")` and the `SOLI_DISABLE_CSRF` operator
///   kill switch remain available for non-browser endpoints that
///   legitimately ride a cookie.
///
/// The intent matches `websocket_origin_allowed`'s authority semantics so
/// the two surfaces (HTTP + WebSocket) reject under the same rules.
fn check_csrf_origin(headers: &hyper::HeaderMap, method: &str, path: &str) -> Result<(), String> {
    if matches!(method, "GET" | "HEAD" | "OPTIONS") {
        return Ok(());
    }
    if path.starts_with("/_") {
        return Ok(());
    }
    if csrf_skipped_by_app(path) {
        return Ok(());
    }
    if csrf_disabled_by_env() {
        return Ok(());
    }
    // A `cors("/path", {...})` declaration allowing this Origin is an
    // explicit cross-origin opt-in for the path — more precise than
    // `skip_csrf`, since the origin is checked against the declared list.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if cors::allows_cross_origin(path, origin.trim()) {
            return Ok(());
        }
    }

    let request_authority = match websocket_request_authority(headers) {
        Some(a) => a,
        None => return Err("missing Host header".to_string()),
    };

    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let origin = origin.trim();
        // `null` Origin is what sandboxed iframes / data URLs send.
        // Treat it the same as a foreign origin.
        if origin.eq_ignore_ascii_case("null") {
            return Err("Origin is 'null'".to_string());
        }
        let Some(origin_auth) = origin_authority(origin) else {
            return Err(format!("malformed Origin header: {}", origin));
        };
        if origin_auth == request_authority {
            return Ok(());
        }
        return Err(format!(
            "Origin {} does not match request authority {}",
            origin_auth, request_authority
        ));
    }

    if let Some(referer) = headers.get(header::REFERER).and_then(|v| v.to_str().ok()) {
        let Some(referer_auth) = origin_authority(referer) else {
            return Err(format!("malformed Referer header: {}", referer));
        };
        if referer_auth == request_authority {
            return Ok(());
        }
        return Err(format!(
            "Referer {} does not match request authority {}",
            referer_auth, request_authority
        ));
    }

    // Neither Origin nor Referer. SEC-078: a cookie-bearing request in
    // this state has no proof of same-site provenance — modern browsers
    // do set Origin on cross-site state-changing requests, but stripped
    // user agents, transparent proxies, and Origin-less form posts still
    // happen, and the threat is precisely a session-cookie replay riding
    // such a request. Reject. Cookie-less requests stay on the non-
    // browser API path (curl, mobile clients) where there is no session
    // to ride. Mirrors the same Cookie-presence rule that
    // `websocket_origin_allowed` already enforces for WS upgrades.
    if headers.contains_key(header::COOKIE) {
        return Err("missing both Origin and Referer on cookie-bearing request".to_string());
    }
    Ok(())
}

/// SEC-080: decide whether a `/__coverage__` GET is authorised. The
/// endpoint is only reachable when `SOLI_COVERAGE_ENABLED` is set; the
/// test runner additionally mints a random `SOLI_COVERAGE_TOKEN` per
/// run and presents it as `X-Coverage-Token`. `expected` is the env
/// value (None = not configured = reject), `provided` is the request
/// header value (empty = no header = reject). Constant-time compare so
/// the negative result doesn't leak token shape.
fn coverage_request_authorized(expected: Option<&str>, provided: &str) -> bool {
    let Some(tok) = expected else { return false };
    if tok.is_empty() || provided.is_empty() {
        return false;
    }
    crate::interpreter::builtins::crypto::do_secure_compare(tok, provided)
}

fn forbidden_csrf_response(reason: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(full(Bytes::from(format!("CSRF check failed: {}", reason))))
        .unwrap()
}

fn websocket_origin_allowed(headers: &hyper::HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        // SEC-046: an Origin-less upgrade was previously allowed because
        // non-browser clients (curl, native apps) often omit it. But the
        // CSWSH threat is precisely a non-browser pivot — e.g. an SSRF
        // target inside the network forging a cookie-bearing handshake
        // to a privileged endpoint. Require Origin whenever the request
        // carries a Cookie; allow the missing header only on
        // unauthenticated upgrades, where there are no credentials to
        // ride.
        return !headers.contains_key(header::COOKIE);
    };

    let Some(origin_authority) = origin_authority(origin) else {
        return false;
    };
    let Some(request_authority) = websocket_request_authority(headers) else {
        return false;
    };

    origin_authority == request_authority
}

fn websocket_request_authority(headers: &hyper::HeaderMap) -> Option<String> {
    // SEC-032: only consult `X-Forwarded-Host` when the operator has
    // explicitly opted into the trust-proxy gate. On a directly-exposed
    // app, an attacker controls every inbound header, so trusting XFH
    // unconditionally would let a cross-origin WebSocket handshake
    // present `Origin: http://evil` and `X-Forwarded-Host: evil` and
    // pass the same-origin check (CSWSH against cookie-authenticated
    // WS endpoints, including LiveView's `/live/socket`).
    let value = if crate::interpreter::builtins::trust_proxy::is_trust_proxy_enabled() {
        headers
            .get("x-forwarded-host")
            .or_else(|| headers.get(header::HOST))
    } else {
        headers.get(header::HOST)
    };
    value
        .and_then(|v| v.to_str().ok())
        .map(first_forwarded_token)
        .map(normalize_request_authority)
        .filter(|host| !host.is_empty())
}

fn origin_authority(origin: &str) -> Option<String> {
    let origin = origin.trim();
    let (scheme, rest) = origin
        .strip_prefix("http://")
        .map(|rest| ("http", rest))
        .or_else(|| origin.strip_prefix("https://").map(|rest| ("https", rest)))?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }

    Some(normalize_origin_authority(authority, scheme))
}

fn normalize_origin_authority(authority: &str, scheme: &str) -> String {
    let authority = normalize_authority(authority);
    match (scheme, authority.as_str()) {
        ("http", value) if value.ends_with(":80") => value.trim_end_matches(":80").to_string(),
        ("https", value) if value.ends_with(":443") => value.trim_end_matches(":443").to_string(),
        _ => authority,
    }
}

fn normalize_request_authority(authority: &str) -> String {
    let authority = normalize_authority(authority);
    if authority.ends_with(":80") {
        return authority.trim_end_matches(":80").to_string();
    }
    if authority.ends_with(":443") {
        return authority.trim_end_matches(":443").to_string();
    }
    authority
}

fn normalize_authority(authority: &str) -> String {
    authority.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// SEC-044: pick the first comma-separated token from a forwarded-header
/// value (`X-Forwarded-Proto`, `X-Forwarded-Host`). Some proxies append
/// instead of overwriting, so a request with `X-Forwarded-Host: real,
/// attacker` would otherwise reach our scheme/host code as the whole
/// concatenated string. The leftmost entry — written by the *outermost*
/// trusted proxy in a chain — is the canonical value once `trust_proxy`
/// is enabled. Empty input or empty first token returns `""`, which lets
/// callers fall back to defaults without an extra branch.
fn first_forwarded_token(value: &str) -> &str {
    value.split(',').next().unwrap_or("").trim()
}

/// Handle WebSocket upgrade request.
async fn handle_websocket_upgrade(
    mut req: Request<Incoming>,
    ws_registry: Arc<WebSocketRegistry>,
    path: String,
    ws_event_tx: channel::Sender<WebSocketEventData>,
) -> Result<Response<ResponseBody>, hyper::Error> {
    // Check if this is a valid WebSocket upgrade request
    if !hyper_tungstenite::is_upgrade_request(&req) {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(full(Bytes::from("Not a WebSocket upgrade request")))
            .unwrap());
    }

    if !websocket_origin_allowed(req.headers()) {
        return Ok(forbidden_websocket_origin_response());
    }

    // Perform the WebSocket upgrade
    let ws_config = default_websocket_config();
    let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, Some(ws_config)) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("[WS] Upgrade error: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full(Bytes::from(format!("WebSocket upgrade error: {}", e))))
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
    Ok(box_full(response))
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
        HashKey::String("type".into()),
        Value::String(data.event_type.clone().into()),
    );
    event_map.insert(
        HashKey::String("connection_id".into()),
        Value::String(connection_id_str.clone().into()),
    );

    if let Some(ref msg) = data.message {
        event_map.insert(
            HashKey::String("message".into()),
            Value::String(msg.clone().into()),
        );
    }

    if let Some(ref channel) = data.channel {
        event_map.insert(
            HashKey::String("channel".into()),
            Value::String(channel.clone().into()),
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

            // Process broadcast_room action: fan the payload out to everyone in
            // the connection's most-recently-joined room (sender included — the
            // client filters its own echo). When the SAME return also carries a
            // `join`, that channel is the most recent one and we use it directly
            // (the async join above may not have landed in the registry yet);
            // otherwise we look up the connection's current rooms. Previously
            // this only fired when `join` was present in the same return, so a
            // bare `{ "broadcast_room": ... }` (e.g. a move frame after the
            // initial join) was silently dropped.
            if let Some(ref msg) = action.broadcast_room {
                let registry_clone = registry.clone();
                let msg_clone = msg.clone();
                let join_channel = action.join.clone();
                runtime_handle.spawn(async move {
                    let target = match join_channel {
                        Some(channel) => Some(channel),
                        None => registry_clone.most_recent_channel(&connection_id).await,
                    };
                    if let Some(channel) = target {
                        registry_clone
                            .broadcast_to_channel(&channel, &msg_clone)
                            .await;
                    }
                });
            }

            // Process broadcast_channel action: deliver to an explicitly named
            // channel. The registry's channels are server-wide, so a handler on
            // one socket path can address a room joined on another (e.g. a
            // per-user channel on the shared app socket).
            if let Some((ref channel, ref msg)) = action.broadcast_channel {
                let registry_clone = registry.clone();
                let channel_clone = channel.clone();
                let msg_clone = msg.clone();
                runtime_handle.spawn(async move {
                    registry_clone
                        .broadcast_to_channel(&channel_clone, &msg_clone)
                        .await;
                });
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
        HashKey::String("event".into()),
        Value::String(data.event.clone().into()),
    );
    event_map.insert(HashKey::String("params".into()), params_value);
    event_map.insert(HashKey::String("state".into()), state_value);
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

        // Mark this LiveView as the one rendering, so any `Model.live_where`
        // the handler runs subscribes it to the queried collection. The guard
        // clears the thread-local when the handler call returns.
        let _lv_query_guard =
            crate::live::live_query::set_current(instance.id.clone(), component.clone());

        // Call the handler function
        match interpreter.call_value(handler, vec![event_value], Span::default()) {
            Ok(result) => {
                // The handler may return either of two shapes:
                //   1. `{ ...state }`        — used directly as the new state
                //   2. `{ "state": {...}, "tick_interval": N, "stream": {...} }`
                //      — wrapped form; `state` (optional) is the new state,
                //      `tick_interval` (ms) controls the tick timer, and `stream`
                //      carries targeted container ops pushed as a Stream message.
                match &result {
                    Value::Null => {
                        return handle_liveview_event_fallback(data, &mut instance);
                    }
                    Value::Hash(_) => {
                        let json = value_to_json(&result);
                        let (new_state_json, tick_interval, stream) = unwrap_handler_return(json);

                        // Replace state only when the handler supplied one (a
                        // stream-only emission leaves the current state intact).
                        if let Some(mut state) = new_state_json {
                            // Preserve the id across state replacement
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

                        // Apply tick scheduling change (if any)
                        apply_tick_interval(&mut instance, tick_interval);

                        // Push targeted collection ops straight to the client
                        // (append/prepend/remove/…). Streamed rows live outside
                        // the diff shadow, so this never fights render patches.
                        if let Some(stream_val) = stream {
                            let ops = build_stream_ops(&stream_val);
                            if !ops.is_empty() {
                                use crate::live::view::LIVE_REGISTRY;
                                let _ = LIVE_REGISTRY.send(
                                    &instance.id,
                                    crate::live::view::ServerMessage::Stream {
                                        liveview_id: instance.id.clone(),
                                        ops,
                                    },
                                );
                            }
                        }
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

    // For `connect`, the initial render has already been sent by
    // `handle_live_connection`. Subsequent events render a diff patch.
    if data.event == "connect" {
        // Persist the (possibly tick-scheduled) instance and re-render so the
        // initial DOM reflects the connect-time state the handler returned.
        return render_and_send_patch(&component, &mut instance);
    }

    render_and_send_patch(&component, &mut instance)
}

/// Unwrap the handler return value. The wrapped form
/// `{ "state": {...}, "tick_interval": N, "stream": {...} }` yields the inner
/// state (or `None` to keep the current state, for a stream-only emission), the
/// tick interval, and the raw stream sub-hash. The bare form (no `state` object
/// and no `stream` key) is the new state itself.
///
/// `tick_interval` interpretation:
///   * key absent → `None`     (don't touch the running timer)
///   * value 0    → `Some(0)`  (stop the timer)
///   * value > 0  → `Some(ms)` (start or replace the timer)
#[allow(clippy::type_complexity)]
fn unwrap_handler_return(
    json: serde_json::Value,
) -> (
    Option<serde_json::Value>,
    Option<u64>,
    Option<serde_json::Value>,
) {
    // Caller only invokes this with a hash result, but be defensive.
    let mut map = match json {
        serde_json::Value::Object(m) => m,
        other => return (Some(other), None, None),
    };

    let has_state_obj = map.get("state").is_some_and(|v| v.is_object());
    let has_stream = map.contains_key("stream");

    // Bare shape (no `state` object and no `stream`): the whole hash is the new
    // state, no tick change, no stream.
    if !has_state_obj && !has_stream {
        return (Some(serde_json::Value::Object(map)), None, None);
    }

    // Wrapped shape. A missing `state` (stream-only emission) leaves the current
    // state untouched (`None`), rather than wiping it to `{}`.
    let state = if has_state_obj {
        map.remove("state")
    } else {
        None
    };
    let tick_interval = map.get("tick_interval").and_then(|v| v.as_u64());
    let stream = map.remove("stream");
    (state, tick_interval, stream)
}

/// Build the typed `StreamOp`s from a handler's `stream` sub-hash, of shape
/// `{ "container": "<id>", "ops": [ { "op": "append"|"prepend"|"insert"|
/// "remove"|"reset", "id": "<dom-id>", "html": "<markup>", "before"? }, … ] }`.
/// The container hoisted at the top applies to every op; malformed ops are
/// skipped.
fn build_stream_ops(stream: &serde_json::Value) -> Vec<crate::live::view::StreamOp> {
    use crate::live::view::StreamOp;
    let container = stream
        .get("container")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let Some(ops) = stream.get("ops").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        let kind = op.get("op").and_then(|v| v.as_str()).unwrap_or_default();
        let id = op
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let html = op
            .get("html")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        // Per-op container override, else the hoisted one.
        let container = op
            .get("container")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| container.clone());
        let built = match kind {
            "append" => StreamOp::Append {
                container,
                id,
                html,
            },
            "prepend" => StreamOp::Prepend {
                container,
                id,
                html,
            },
            "insert" => StreamOp::Insert {
                container,
                id,
                html,
                before: op
                    .get("before")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            },
            "remove" => StreamOp::Remove { id },
            "reset" => StreamOp::Reset { container },
            _ => continue, // unknown op — skip
        };
        out.push(built);
    }
    out
}

/// Reconcile the requested tick interval with the instance's currently
/// running tick task. Spawns a new tokio task when the interval changes,
/// cancels it when set to 0, leaves it alone when unspecified.
fn apply_tick_interval(instance: &mut crate::live::view::LiveViewInstance, requested: Option<u64>) {
    let Some(requested) = requested else { return };

    // Stop any existing timer.
    if requested == 0 {
        if instance.tick_interval_ms.is_some() {
            crate::live::socket::cancel_tick_task(&instance.id);
            instance.tick_interval_ms = None;
        }
        return;
    }

    // No-op if the interval hasn't changed.
    if instance.tick_interval_ms == Some(requested) {
        return;
    }

    let Some(tx) = LV_EVENT_TX.get().cloned() else {
        eprintln!("[LiveView] tick scheduling unavailable: lv_event_tx not initialized");
        return;
    };
    let Some(handle) = get_tokio_handle() else {
        eprintln!("[LiveView] tick scheduling unavailable: no tokio runtime handle");
        return;
    };

    let liveview_id = instance.id.clone();
    let component = instance.component.clone();
    let interval_ms = requested;

    let join = handle.spawn(async move {
        // The first `tick()` fires immediately; skip it so the user's tick
        // cadence starts after `interval_ms`, not at t=0.
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        interval.tick().await;
        loop {
            interval.tick().await;
            let (response_tx, _response_rx) = oneshot::channel();
            // try_send: if the worker is backed up, drop this tick rather
            // than queue indefinitely. The next tick will catch up.
            let send_result = tx.try_send(LiveViewEventData {
                liveview_id: liveview_id.clone(),
                component: component.clone(),
                event: "tick".to_string(),
                params: serde_json::json!({}),
                response_tx,
            });
            // If the channel is permanently disconnected, stop the task.
            if let Err(channel::TrySendError::Disconnected(_)) = send_result {
                break;
            }
        }
    });

    crate::live::socket::set_tick_task(&instance.id, join.abort_handle());
    instance.tick_interval_ms = Some(requested);
}

/// Wake a set of LiveViews after a DB write matched their live query. Enqueues
/// one synthetic `live_query_changed` event per subscriber onto the LiveView
/// event bus (mirroring `apply_tick_interval`'s throwaway `oneshot`); the worker
/// re-runs each handler and `render_and_send_patch` drops the frame if the diff
/// is empty. Called from `crate::live::live_query::notify_change`. No-op before
/// the bus is initialized (e.g. non-server processes) or when realtime is off.
pub(crate) fn enqueue_live_query_changed(subscribers: Vec<(String, String)>) {
    let Some(tx) = LV_EVENT_TX.get() else {
        return;
    };
    for (liveview_id, component) in subscribers {
        let (response_tx, _response_rx) = oneshot::channel();
        // try_send: a backed-up worker drops this wake rather than block the
        // write path; the next write catches up.
        let _ = tx.try_send(LiveViewEventData {
            liveview_id,
            component,
            event: "live_query_changed".to_string(),
            params: serde_json::json!({}),
            response_tx,
        });
    }
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

/// Record a VM→interpreter handler demotion: bump the metrics counter
/// (`soli_vm_handler_demotions_total` on `/_metrics`) and, when
/// `SOLI_ENGINE_LOG=1`, log which handler was demoted and why. Demotions are
/// cached in `vm.failed_handlers`, so this fires once per handler per worker.
fn record_vm_demotion(handler: &str, err: &RuntimeError) {
    crate::metrics::Metrics::global()
        .vm_handler_demotions_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    static ENGINE_LOG: OnceLock<bool> = OnceLock::new();
    let log = *ENGINE_LOG.get_or_init(|| {
        std::env::var("SOLI_ENGINE_LOG")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    });
    if log {
        eprintln!("[soli engine] handler '{handler}' demoted to the interpreter: {err}");
    }
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
    // One span per dispatched handler — covers before_action + method body
    // + after_action so they nest as children. Cheap when --dev is off.
    let _action_span = span_log::SpanGuard::start(handler_name, span_log::SpanKind::Action);

    // Record the action name ("controller#action" -> "action") so the auth
    // Policy layer's `current_action()` builtin can infer the policy method in
    // `authorize(record)`. For function handlers without a "#", use the whole name.
    let action_name = handler_name.rsplit('#').next().unwrap_or(handler_name);
    crate::interpreter::builtins::set_current_action(action_name);

    // Publish the current request to the thread-local so request-aware builtins
    // (`render_jsonp`'s `?callback` lookup, `current_path()`/`current_method()`)
    // work during the action body regardless of controller style. OOP controllers
    // also set this in `setup_controller_context`; setting it here first covers
    // function-based handlers, which have no controller instance.
    crate::interpreter::builtins::template::set_current_request(request_hash.clone());

    // Reset any view debug context left over from a prior request on this
    // reused worker thread. `render()` keeps the context set on error (so the
    // failing locals reach the dev error page), so a controller-only error in
    // this request must not inherit a previous request's stale `_view_data`.
    crate::interpreter::builtins::template::clear_view_debug_context();

    // Expose req["all"] as global `params` so handlers/views can reference it directly.
    // Default to an empty hash (not Null) so callers can safely index into it.
    let params_value = get_hash_field(&request_hash, "all")
        .unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))));
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("params", params_value.clone());
    if let Some(vm_ref) = vm.as_deref_mut() {
        vm_ref
            .globals
            .insert("params".to_string(), params_value.clone());
    }

    // Expose parsed cookies as global `cookies` so handlers/view can reference
    // cookies directly. Default to an empty hash (not Null).
    let cookies_value = get_hash_field(&request_hash, "cookies")
        .unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))));
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("cookies", cookies_value.clone());
    if let Some(vm_ref) = vm.as_deref_mut() {
        vm_ref
            .globals
            .insert("cookies".to_string(), cookies_value.clone());
    }

    // Expose the full request hash as a global `req` so actions can omit the
    // `(req)` parameter when they don't need to destructure the request.
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("req", request_hash.clone());
    if let Some(vm_ref) = vm.as_deref_mut() {
        vm_ref
            .globals
            .insert("req".to_string(), request_hash.clone());
    }

    // Rebind request-scoped names on the view helpers' closure env so user
    // helpers in `app/helpers/*.sl` see the *post-middleware* request — e.g.
    // a `current_user()` helper that reads `req["current_user"]` set by
    // `app/middleware/auth.sl`. Without this, helpers see only the env they
    // closed over at load time (builtins + sibling helpers) and `req` is
    // undefined.
    let session_value = get_hash_field(&request_hash, "session").unwrap_or(Value::Null);
    let headers_value = get_hash_field(&request_hash, "headers").unwrap_or(Value::Null);
    crate::interpreter::builtins::template::set_helper_request_context(
        &request_hash,
        &params_value,
        &session_value,
        &cookies_value,
        &headers_value,
    );

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

    // Handlers may be declared as either `fn name(req)` or, more idiomatically
    // for read-only actions, `fn name` with no parameters (they read `req`
    // via the request-time global). Match the call arity to the declaration
    // so the latter form doesn't trip the runtime arity check.
    let handler_wants_request = match &handler_result {
        Ok(Value::Function(f)) => f.full_arity() > 0,
        _ => true,
    };

    // The background-job callback (`_jobs#run`) must run on the tree-walking
    // interpreter, never the VM. Its prelude wraps `cls.perform(...)` in a
    // try/catch that turns ANY error into a 500 return value — so a VM-only
    // failure (optional-`let` bare assignment compiling to a doomed SetGlobal,
    // a missing model property, etc.) is swallowed as a "successful" 500
    // instead of bubbling up to trigger the normal VM->interpreter handler
    // fallback below. That made every VM gap in a job's call graph a hard 500
    // with no fallback. Jobs are infrequent and not latency-critical, so the
    // interpreter — which honors optional-`let` and returns nil for absent
    // properties — is the correct, safe runtime for the whole job call graph.
    let force_interpreter = handler_name == "_jobs#run";

    // Try VM execution in production mode for function-based handlers
    if let Some(ref mut vm) = vm {
        if !force_interpreter && !vm.failed_handlers.contains(handler_name) {
            if let Ok(ref handler_value) = handler_result {
                let call_result = if handler_wants_request {
                    vm.call_value_direct_one(
                        handler_value.clone(),
                        request_hash.clone(),
                        Span::default(),
                    )
                } else {
                    vm.call_value_direct(handler_value.clone(), Vec::new(), Span::default())
                };
                match call_result {
                    Ok(result) => {
                        vm.reset();
                        let (status, headers, body) = extract_response(result);
                        return ResponseData {
                            status,
                            headers,
                            body,
                        };
                    }
                    Err(err) => {
                        record_vm_demotion(handler_name, &err);
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
            let args = if handler_wants_request {
                vec![request_hash]
            } else {
                Vec::new()
            };
            match interpreter.call_value(handler_value, args, Span::default()) {
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
                    if let Some(resp) = record_not_found_response(&e) {
                        interpreter.pop_frame();
                        return resp;
                    }
                    if let Some(resp) = forbidden_response(&e) {
                        interpreter.pop_frame();
                        return resp;
                    }
                    // Capture environment BEFORE popping the frame so local
                    // variables of the failing call are still visible.
                    let captured_env = if e.breakpoint_env_json().is_none() {
                        Some(interpreter.serialize_environment_for_debug())
                    } else {
                        None
                    };
                    interpreter.pop_frame();
                    let stack_trace: Vec<String> = e
                        .breakpoint_stack_trace()
                        .map(|st| st.to_vec())
                        .unwrap_or_else(|| interpreter.get_stack_trace());
                    let env_json: Option<String> = e
                        .breakpoint_env_json()
                        .map(|s| s.to_string())
                        .or(captured_env);
                    let request_id = Uuid::new_v4().to_string();
                    let error_msg = e.to_string();
                    // Breakpoints are intentional debug pauses, not failures,
                    // so don't emit the stderr error block for them.
                    if !e.is_breakpoint() {
                        error_logging::log_production_error(
                            &request_id,
                            request_data,
                            &error_msg,
                            &stack_trace,
                            env_json.as_deref(),
                        );
                    }
                    if dev_mode {
                        let error_html = error_pages::render_error_page(
                            &error_msg,
                            interpreter,
                            request_data,
                            &stack_trace,
                            env_json.as_deref(),
                        );
                        ResponseData {
                            status: if e.is_breakpoint() { 200 } else { 500 },
                            headers: vec![(
                                "Content-Type".to_string(),
                                "text/html; charset=utf-8".to_string(),
                            )],
                            body: error_html.into_bytes(),
                        }
                    } else {
                        let error_html =
                            error_pages::render_production_error_page(500, &error_msg, &request_id);
                        ResponseData {
                            status: 500,
                            headers: vec![(
                                "Content-Type".to_string(),
                                "text/html; charset=utf-8".to_string(),
                            )],
                            body: error_html.into_bytes(),
                        }
                    }
                }
            }
        }
        Err(e) => {
            let captured_env = Some(interpreter.serialize_environment_for_debug());
            interpreter.pop_frame();
            // This error is a String from resolve_handler, no captured
            // stack trace — use whatever the interpreter still holds.
            let stack_trace = interpreter.get_stack_trace();
            let request_id = Uuid::new_v4().to_string();
            let error_msg = e.to_string();
            error_logging::log_production_error(
                &request_id,
                request_data,
                &error_msg,
                &stack_trace,
                captured_env.as_deref(),
            );
            if dev_mode {
                let error_html = error_pages::render_error_page(
                    &error_msg,
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
                    body: error_html.into_bytes(),
                }
            } else {
                let error_html =
                    error_pages::render_production_error_page(500, &error_msg, &request_id);
                ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html.into_bytes(),
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
    // For nested paths like "dashboard/cluster", also try the simple class name "ClusterController"
    let class_name = to_pascal_case_controller(controller_key);

    // Look up the class in the environment - try both full-path and simple names
    let class_value = interpreter
        .environment
        .borrow()
        .get(&class_name)
        .or_else(|| {
            // For nested controllers, try the simple class name (last segment)
            if controller_key.contains('/') {
                controller_key.rsplit('/').next().and_then(|simple| {
                    let simple_class = to_pascal_case_controller(simple);
                    interpreter.environment.borrow().get(&simple_class)
                })
            } else {
                None
            }
        });

    // Look up the class in the environment
    let class_value = match class_value {
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
    let cookies = get_hash_field(request_hash, "cookies")
        .unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))));

    // Instantiate the controller
    let controller_instance = match create_controller_instance(&class_name, interpreter) {
        Ok(inst) => inst,
        Err(e) => {
            let stack_trace = interpreter.get_stack_trace();
            let env_json = interpreter.serialize_environment_for_debug();
            let request_id = Uuid::new_v4().to_string();
            let error_msg = e.to_string();
            error_logging::log_production_error(
                &request_id,
                request_data,
                &error_msg,
                &stack_trace,
                Some(&env_json),
            );
            return Some(if dev_mode {
                let error_html = error_pages::render_error_page(
                    &error_msg,
                    interpreter,
                    request_data,
                    &stack_trace,
                    Some(&env_json),
                );
                ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html.into_bytes(),
                }
            } else {
                let error_html =
                    error_pages::render_production_error_page(500, &error_msg, &request_id);
                ResponseData {
                    status: 500,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/html; charset=utf-8".to_string(),
                    )],
                    body: error_html.into_bytes(),
                }
            });
        }
    };

    // Set up controller context (req, params, session, headers, cookies)
    setup_controller_context(
        &controller_instance,
        request_hash,
        &params,
        &session,
        &headers,
        &cookies,
    );

    // Publish the instance as the thread-local "current controller" so `render(...)`
    // can auto-expose its fields as view locals. The guard clears it on every exit
    // path (success, error, panic unwind) to avoid leaking state across requests.
    crate::interpreter::builtins::controller::registry::set_current_controller(
        controller_instance.clone(),
    );
    struct CurrentControllerGuard;
    impl Drop for CurrentControllerGuard {
        fn drop(&mut self) {
            crate::interpreter::builtins::controller::registry::clear_current_controller();
        }
    }
    let _current_controller_guard = CurrentControllerGuard;

    // Execute before_action hooks AFTER the instance exists and is published as
    // CURRENT_CONTROLLER, so `@foo = ...` inside a hook writes to the instance
    // and is picked up by the render-time auto-injection.
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

    // If the action succeeded but its return value isn't a response hash, try the
    // auto-render path. A template-not-found falls through to raw value serialization;
    // a real render error (e.g. `<%= 3 / 0 %>` in the view or layout) gets promoted
    // into the same RuntimeError pipeline as an action that itself raised — so the
    // user sees a 500/dev error page instead of a blank 200.
    let action_result = match action_result {
        Ok(result) if !is_response_hash(&result) => {
            let (controller_key, _) = handler_name.split_once('#').unwrap_or((handler_name, ""));
            let default_template = format!("{}/{}", controller_key, action_name);
            match try_render_template(interpreter, &controller_instance, &default_template) {
                Ok(Some(auto_result)) => Ok(auto_result),
                Ok(None) => Ok(result),
                Err(msg) => Err(RuntimeError::new(msg, Span::new(0, 0, 1, 1))),
            }
        }
        other => other,
    };

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
            if let Some(resp) = record_not_found_response(&e) {
                resp
            } else if let Some(resp) = forbidden_response(&e) {
                resp
            } else {
                let stack_trace: Vec<String> = e
                    .breakpoint_stack_trace()
                    .map(|st| st.to_vec())
                    .unwrap_or_else(|| interpreter.get_stack_trace());
                let env_json: Option<String> = e
                    .breakpoint_env_json()
                    .map(|s| s.to_string())
                    .or_else(|| Some(interpreter.serialize_environment_for_debug()));
                let request_id = Uuid::new_v4().to_string();
                let error_msg = e.to_string();
                // Breakpoints are intentional debug pauses, not failures,
                // so don't emit the stderr error block for them.
                if !e.is_breakpoint() {
                    error_logging::log_production_error(
                        &request_id,
                        request_data,
                        &error_msg,
                        &stack_trace,
                        env_json.as_deref(),
                    );
                }
                if dev_mode {
                    let error_html = error_pages::render_error_page(
                        &error_msg,
                        interpreter,
                        request_data,
                        &stack_trace,
                        env_json.as_deref(),
                    );
                    ResponseData {
                        status: if e.is_breakpoint() { 200 } else { 500 },
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html.into_bytes(),
                    }
                } else {
                    let error_html =
                        error_pages::render_production_error_page(500, &error_msg, &request_id);
                    ResponseData {
                        status: 500,
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html.into_bytes(),
                    }
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
    let error_html = error_pages::render_production_error_page(500, err, &request_id);
    ResponseData {
        status: 500,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: error_html.into_bytes(),
    }
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
    let error_html = error_pages::render_production_error_page(500, &error_msg, &request_id);
    ResponseData {
        status: 500,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: error_html.into_bytes(),
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

    // Always measure for the coarse production Prometheus counter (Phase A).
    // The rich per-middleware log stays gated to --dev.
    let mw_start = std::time::Instant::now();
    let per_mw_start = middleware_log::is_enabled().then(std::time::Instant::now);

    interpreter.push_frame(name, span, source_path.map(|s| s.to_string()));
    if let Some(path) = source_path {
        interpreter.set_source_path(PathBuf::from(path));
    }
    let result = interpreter.call_value(handler, vec![request_hash], span);
    interpreter.pop_frame();

    let elapsed = mw_start.elapsed();
    crate::metrics::Metrics::global().record_middleware(elapsed);

    if let Some(start) = per_mw_start {
        middleware_log::record(name, start.elapsed().as_micros() as u64);
    }
    result
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
            // Only use VM for methods that take a (req) parameter. Zero-arg
            // methods get req via the global and fall back to the interpreter.
            if !vm.failed_handlers.contains(&handler_key) && !method.params.is_empty() {
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
                    Err(err) => {
                        record_vm_demotion(&handler_key, &err);
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
                jit_cache: RefCell::new(None),
            })
        };

        // Only pass the request hash if the action expects a parameter (e.g. `def index(req)`).
        // Zero-arg actions get `req` implicitly via the global, so don't pass it as an argument.
        let action_args: Vec<Value> = if method.params.is_empty() {
            vec![]
        } else {
            vec![request_hash.clone()]
        };
        let result =
            interpreter.call_value(Value::Function(bound_method), action_args, method_span);

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
            let key = HashKey::String(field.to_string().into());
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

        let _ba_span = span_log::SpanGuard::start_with_meta(
            "before_action",
            span_log::SpanKind::BeforeAction,
            Some(action_name.to_string()),
        );
        // Execute the before_action handler
        match crate::interpreter::builtins::controller::registry::execute_handler_source(
            &before_action.handler_source,
            before_action.source_line,
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
                    body: format!("Before action error: {}", e).into_bytes(),
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
        .map(|(k, v)| {
            (
                HashKey::String(k.clone().into()),
                Value::String(v.clone().into()),
            )
        })
        .collect();
    let mut response_map: HashPairs = HashPairs::default();
    response_map.insert(
        HashKey::String("status".into()),
        Value::Int(response.status as i64),
    );
    response_map.insert(
        HashKey::String("headers".into()),
        Value::Hash(Rc::new(RefCell::new(headers_map))),
    );
    response_map.insert(
        HashKey::String("body".into()),
        match std::str::from_utf8(&response.body) {
            Ok(s) => Value::String(s.to_string().into()),
            Err(_) => Value::Array(Rc::new(RefCell::new(
                response
                    .body
                    .iter()
                    .map(|&b| Value::Int(b as i64))
                    .collect(),
            ))),
        },
    );
    let response_value = Value::Hash(Rc::new(RefCell::new(response_map)));

    for after_action in &controller_info.after_actions {
        // Check if this after_action applies to this action
        if !after_action.actions.is_empty() && after_action.actions.iter().all(|a| a != action_name)
        {
            continue;
        }

        let _aa_span = span_log::SpanGuard::start_with_meta(
            "after_action",
            span_log::SpanKind::AfterAction,
            Some(action_name.to_string()),
        );
        // Execute the after_action handler
        match crate::interpreter::builtins::controller::registry::execute_after_handler_source(
            &after_action.handler_source,
            after_action.source_line,
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
/// If the error came from a record-not-found path (e.g. `Model.find("x")`
/// where "x" doesn't exist), build a 404 response. Returns None otherwise,
/// letting the default 500 handling run.
///
/// Uses the standard production error-page pipeline so apps can ship a
/// custom `app/views/errors/404.html.slv` template and have it rendered
/// automatically (same mechanism the route-not-found 404 uses).
fn record_not_found_response(err: &RuntimeError) -> Option<ResponseData> {
    let message = err.record_not_found_message()?;
    let request_id = Uuid::new_v4().to_string();
    let body = error_pages::render_production_error_page(404, &message, &request_id);
    Some(ResponseData {
        status: 404,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: body.into_bytes(),
    })
}

/// If the error came from an authorization failure (the `forbidden()` builtin
/// or the Policy layer's `authorize()` helper), build a 403 response using the
/// standard production error-page pipeline (so apps can ship a custom
/// `app/views/errors/403.html.slv`). Returns None otherwise.
fn forbidden_response(err: &RuntimeError) -> Option<ResponseData> {
    let message = err.forbidden_message()?;
    let request_id = Uuid::new_v4().to_string();
    let body = error_pages::render_production_error_page(403, &message, &request_id);
    Some(ResponseData {
        status: 403,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: body.into_bytes(),
    })
}

fn check_for_response(value: &Value) -> Option<ResponseData> {
    // A response is a Hash with a "status" field (and optionally headers, body)
    // A modified request hash has "method", "path", etc. but no "status"
    if let Value::Hash(hash) = value {
        let fields = hash.borrow();

        // Check if this is a response hash by looking for "status" field
        let has_status = fields
            .iter()
            .any(|(k, _)| matches!(k, HashKey::String(s) if **s == *"status"));

        // If no status field, this is a modified request, not a response
        if !has_status {
            return None;
        }

        let mut status = 200i64;
        let mut body: Vec<u8> = Vec::new();
        let mut headers = Vec::new();

        for (key, val) in fields.iter() {
            if let HashKey::String(k) = key {
                match k.as_ref() {
                    "status" => {
                        if let Value::Int(s) = val {
                            status = *s;
                        }
                    }
                    "body" => match val {
                        Value::String(b) => body = b.as_bytes().to_vec(),
                        Value::Array(arr) => {
                            let borrowed = arr.borrow();
                            let mut bytes = Vec::with_capacity(borrowed.len());
                            let mut ok = true;
                            for item in borrowed.iter() {
                                if let Value::Int(n) = item {
                                    bytes.push(*n as u8);
                                } else {
                                    ok = false;
                                    break;
                                }
                            }
                            if ok {
                                body = bytes;
                            }
                        }
                        _ => {}
                    },
                    "headers" => {
                        if let Value::Hash(h) = val {
                            for (hk, hv) in h.borrow().iter() {
                                if let (HashKey::String(key_str), Value::String(val_str)) = (hk, hv)
                                {
                                    headers.push((key_str.to_string(), val_str.to_string()));
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

/// Check if a value is a response hash (has a "status" field).
fn is_response_hash(value: &Value) -> bool {
    if let Value::Hash(hash) = value {
        hash.borrow()
            .iter()
            .any(|(k, _)| matches!(k, HashKey::String(s) if **s == *"status"))
    } else {
        false
    }
}

/// Auto-render the default template for an action when no explicit render/redirect was called.
/// Returns:
///   * `Ok(Some(value))` — template rendered successfully.
///   * `Ok(None)` — template file doesn't exist; caller should fall through to raw value serialization.
///   * `Err(msg)` — template exists but rendering raised an error (e.g. `<%= 3 / 0 %>` in the
///     view or its layout). The caller must surface this as a 500 / dev error page rather than
///     silently returning a blank 200.
fn try_render_template(
    interpreter: &mut Interpreter,
    controller_instance: &Value,
    template_name: &str,
) -> Result<Option<Value>, String> {
    use crate::interpreter::builtins::template::get_template_cache;

    // Build data hash from controller instance fields and params
    let mut data_pairs: crate::interpreter::value::HashPairs = HashPairs::default();

    // Add params (req["all"]) as available data
    if let Some(params_val) = interpreter.global_env().borrow().get("params") {
        data_pairs.insert(HashKey::String("params".into()), params_val.clone());
    }

    // Add all controller instance fields (@ variables) to data
    if let Value::Instance(inst) = controller_instance {
        for (k, v) in inst.borrow().fields.iter() {
            if !k.starts_with('_') {
                data_pairs.insert(HashKey::String(k.clone().into()), v.clone());
            }
        }
    }

    let data = Value::Hash(Rc::new(RefCell::new(data_pairs)));

    // Get template cache and render
    let cache = match get_template_cache() {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };

    // E2E test client: capture the pristine locals (the controller's @vars)
    // for assigns() *before* the req/helper/instance-var injection below, which
    // would otherwise pollute assigns() with framework internals. Committed only
    // if the render below succeeds. Test-runner only (single atomic load).
    let captured_assigns: Option<(String, bool)> =
        if crate::interpreter::builtins::test_server::is_test_runner_process() {
            Some(crate::interpreter::builtins::template::capture_assigns_json(&data))
        } else {
            None
        };

    // Inject req context, controller vars, helpers (same as render() builtin)
    crate::interpreter::builtins::template::inject_request_context(&data);
    crate::interpreter::builtins::template::inject_controller_instance_vars(&data);
    crate::interpreter::builtins::template::inject_template_helpers(&data);

    // Resolve the controller's registered layout for the in-flight action
    // (`static { this.layout = ... }`, including per-action rules and those
    // inherited from a base controller). The explicit `render(...)` builtin
    // already does this; without it here the auto-render path passed `None`
    // and silently fell back to the "application" layout, so OOP controllers
    // that omit `render` (set `@vars`, let the matching view render) never
    // got their declared layout. `None` preserves the prior "application"
    // default for controllers that declared no layout.
    let registered_layout =
        crate::interpreter::builtins::template::registered_layout_for_instance(controller_instance);
    let layout_arg: Option<Option<&str>> = registered_layout.as_deref().map(Some);

    // Render template — returns full HTML string
    let body = match cache.render(template_name, &data, layout_arg) {
        Ok(html) => html,
        Err(e) => {
            // The only legitimate fall-through case is "the top-level view file
            // for this action doesn't exist" — the action returned a raw value
            // and there's no matching view. A nested partial or layout that
            // can't be resolved is a real bug and must surface, not blank-page.
            let top_level_missing = format!("Template '{}' not found", template_name);
            if e.starts_with(&top_level_missing) {
                return Ok(None);
            }
            return Err(e);
        }
    };

    // E2E test client: the auto-render succeeded, so ship the captured view
    // path + locals back (test-runner only). Mirrors the render() builtin.
    if let Some((assigns_json, partial)) = captured_assigns {
        crate::interpreter::builtins::test_server::set_captured_render(
            crate::interpreter::builtins::template::captured_view_path(template_name),
            assigns_json,
            partial,
        );
    }

    // Route the auto-rendered body through `html_response` so it gets the same
    // treatment as an explicit `render(...)` call: Content-Type, content-derived
    // ETag, `Cache-Control`, and the hover-prefetch / live-reload script
    // injection. Building the response hash inline here (Content-Type only) used
    // to silently strip all of that — so OOP controllers that rely on auto-render
    // (set `@vars`, let the matching view render) never got prefetch or
    // conditional-GET caching, while explicit `render()` calls did.
    Ok(Some(crate::template::html_response(body, 200)))
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
    cookies: &Value,
) {
    crate::interpreter::builtins::controller::registry::setup_controller_context(
        controller, req, params, session, headers, cookies,
    );
}

// Thread-local cache for PascalCase controller names to avoid per-request string allocation.
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static PASCAL_CASE_CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Convert a controller key (e.g., "posts", "user_profiles", "admin/merchants")
/// to PascalCase class name (e.g., "PostsController", "UserProfilesController",
/// "AdminMerchantsController"). Both `_` and `/` act as word separators.
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
            if c == '_' || c == '/' {
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
    multipart_form: Option<&[(String, String)]>,
    multipart_files: Option<&Vec<UploadedFile>>,
) -> ParsedBody {
    let mut parsed = ParsedBody::default();

    // Handle multipart data if available (parsed in async context). Bracket
    // keys nest exactly like urlencoded bodies.
    if let Some(form_fields) = multipart_form {
        if !form_fields.is_empty() {
            let form_map =
                crate::interpreter::builtins::server::nest_query_pairs(form_fields.to_vec());
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
    // Reset the per-request AQL log so `dev_queries()` only returns this
    // request's queries. Cheap when dev mode is off (early-out on the flag).
    // Also clear when production logging is on, otherwise the thread-local
    // buffers would accumulate across requests on the same worker thread.
    if dev_mode || prod_log::channels().has_detail() {
        crate::interpreter::builtins::model::query_log::clear();
        crate::interpreter::builtins::http_log::clear();
        crate::interpreter::builtins::kv_log::clear();
        phase_log::clear();
        middleware_log::clear();
        view_log::clear();
        span_log::clear();
        route_log::clear();
        template_warnings::clear();
    }

    // E2E test client: clear any render captured by a prior request on this
    // pooled worker thread, so assigns()/view_path()/render_template() reflect
    // only the current request. A single atomic load in non-test processes.
    if crate::interpreter::builtins::test_server::is_test_runner_process() {
        crate::interpreter::builtins::test_server::clear_captured_render();
    }

    let method = &data.method;
    let path = &data.path;

    // In --dev, snapshot the raw request now — before headers/query/body are
    // moved out downstream — so the dev bar's replay button can re-dispatch it
    // faithfully. Stored in finalize_response keyed by the same request id as
    // the profiling snapshot. Dev-only, so this clone never costs production.
    let captured_raw = if dev_mode {
        Some(dev_store::RawRequest {
            method: data.method.as_ref().to_string(),
            path: data.path.clone(),
            query: data.query.clone(),
            headers: data.headers.clone(),
            body: data.body.clone(),
            peer_ip: data.peer_ip.clone(),
        })
    } else {
        None
    };

    // Built-in readiness probe for blue/green deploys (soli-proxy's health
    // gate). Returns 503 until the session store's backing connection has been
    // warmed, and 200 afterwards. A liveness-only health check (a bare 200
    // from a freshly-booted slot) promotes the slot before its first session
    // round-trip can complete, so traffic switches into the cold-connection
    // window and requests stall to the HTTP client timeout. Gating promotion
    // on this endpoint keeps the old slot serving until the new one is truly
    // ready. Answered here, before any session/cookie work, so the probe never
    // creates a session or touches the store. Apps should not define their own
    // `/up` route — this built-in shadows it.
    if path == "/up" {
        let ready = crate::interpreter::builtins::session::session_store_ready();
        return ResponseData {
            status: if ready { 200 } else { 503 },
            headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
            body: if ready {
                b"ready".to_vec()
            } else {
                b"warming".to_vec()
            },
        };
    }

    // Opt-in OpenAPI (SOLI_OPENAPI): the spec + a Scalar UI over it, built from
    // the app's registered routes (a per-worker thread-local, hence answered
    // here on the worker rather than the async layer). Always-on when enabled,
    // production included. 404 when disabled so it's invisible by default.
    if method == "GET" && (path == "/openapi.json" || path == "/openapi") {
        if !openapi::openapi_enabled() {
            return ResponseData {
                status: 404,
                headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                body: b"Not Found".to_vec(),
            };
        }
        return if path == "/openapi.json" {
            ResponseData {
                status: 200,
                headers: vec![(
                    "Content-Type".to_string(),
                    "application/json; charset=utf-8".to_string(),
                )],
                body: openapi::generate_spec_json().into_bytes(),
            }
        } else {
            ResponseData {
                status: 200,
                headers: vec![(
                    "Content-Type".to_string(),
                    "text/html; charset=utf-8".to_string(),
                )],
                body: openapi::ui_page().into_bytes(),
            }
        };
    }

    // Check if request logging is enabled. `--dev` implies access logging —
    // operators expect every route to show up in the terminal without setting
    // anything first. In production the `SOLI_LOG` channels (parsed once,
    // process-wide) decide; any detail channel folds in `access` so the block
    // has a request line to hang off.
    let log_channels = prod_log::channels();
    let log_requests = dev_mode || log_channels.any();

    // Only create timer when logging is enabled (avoids clock_gettime syscall per request)
    let start_time = if log_requests {
        Some(Instant::now())
    } else {
        None
    };

    // Queue wait: time between the hyper handler enqueueing the request and
    // this worker picking it up. Only captured when logging is active (the
    // enqueue timestamp is None otherwise).
    let queue_ms = match (data.enqueued_at, start_time) {
        (Some(enqueued), Some(start)) => {
            Some(start.saturating_duration_since(enqueued).as_secs_f64() * 1000.0)
        }
        _ => None,
    };

    // Independent timer for the dev bar. Always on when dev_mode is on so the
    // injected bar can show server-side render time. Cheap when off.
    let dev_started = if dev_mode { Some(Instant::now()) } else { None };

    // Anchor the span log to this request's start so every span's
    // `start_us` / `end_us` is encoded as microseconds-since-request-start.
    // Also open the synthetic root request span so the flamegraph has a
    // single top-level rectangle (e.g. `GET /docs/getting_started`) and
    // every other span — middleware, action, view, db — nests beneath it
    // instead of appearing as detached sibling roots. The root is closed
    // explicitly inside `finalize_response` right before the snapshot, so
    // it actually ends up in the recorded log.
    if let Some(t) = dev_started {
        span_log::begin_request(t);
        span_log::open_request_root(format!("{} {}", method, path));
    }

    // Parse the Cookie header ONCE: the same parse feeds both the session-ID
    // resolution here and `req["cookies"]` in the request hash below (the
    // header used to be scanned twice per request).
    let cookie_pairs = parse_cookie_pairs(header_str(&data.headers, "cookie"));

    // Hand the raw header to the cookie jar so `read_cookie` can verify/open
    // sealed values on demand. Installing `None` doubles as the per-request
    // clear, alongside the session-state clears below.
    crate::interpreter::builtins::cookie_jar::install_request_cookie_header(header_str(
        &data.headers,
        "cookie",
    ));

    // Drop any cookie-driver session state a previous request left on this
    // worker thread. Must happen before `ensure_session` installs this
    // request's state — a no-cookie request would otherwise silently inherit
    // (and re-emit) the previous visitor's session.
    crate::interpreter::builtins::session_cookie::clear_request_state();

    // Resolve the session ID from the parsed cookies (if any). When no cookie
    // is sent, we leave the thread-local unset — session_set / session_regenerate
    // will create one lazily on first use, and finalize_response emits
    // Set-Cookie whenever the post-handler session ID differs from the cookie's.
    // SEC-077 precedence (`__Host-session_id` over `session_id`) is preserved
    // inside session_id_from_cookie_pairs.
    let cookie_session_id = session_id_from_cookie_pairs(&cookie_pairs);
    let session_id = if let Some(ref id) = cookie_session_id {
        let resolved = ensure_session(Some(id.as_str()));
        set_current_session_id(Some(resolved.clone()));
        Some(resolved)
    } else {
        set_current_session_id(None);
        None
    };
    // Clear response cookies from any previous request on this thread.
    clear_response_cookies();
    // Reset the static-page response cacheability flags so this request
    // starts clean. set_cookie / session_set trip `mark_response_dirty`
    // and clock / random trip `mark_data_dirty` while the controller
    // runs; the cache lookup in TemplateCache::render consults both
    // and short-circuits to a cache hit only when neither is set.
    crate::template::response_cache::reset_for_new_request();

    // Per-form CSRF token verification. The hyper layer's Origin/Referer
    // gate ran before the body was read; this second gate runs where the
    // session lives, so a request that carries a token (scaffolded forms and
    // `csrf_field()` embed one) must present this session's token.
    //
    // Dev-bar replays skip this gate: a replay re-dispatches a captured
    // request verbatim, but the session's CSRF token may have rotated since
    // capture, which would 403 an otherwise-faithful replay. Replays only
    // originate from the dev-only `/__solidev/replay/:id` endpoint (empty in
    // production), so nothing untrusted can set this flag.
    if !data.replay {
        if let Err(reason) = verify_csrf_token(data, method, path) {
            set_current_session_id(None);
            let request_id = Uuid::new_v4().to_string();
            eprintln!(
                "[WARN] request_id={} {} {} - 403 CSRF: {}",
                request_id, method, path, reason
            );
            let error_html = error_pages::render_production_error_page(
                403,
                "CSRF verification failed. Reload the page and resubmit the form.",
                &request_id,
            );
            return ResponseData {
                status: 403,
                headers: vec![(
                    "Content-Type".to_string(),
                    "text/html; charset=utf-8".to_string(),
                )],
                body: error_html.into_bytes(),
            };
        }
    }

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
                    "{} [LOG] {} {} - 404 ({:.3}ms)",
                    log_timestamp(),
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
            let is_https = if crate::interpreter::builtins::trust_proxy::is_trust_proxy_enabled() {
                header_str(&data.headers, "x-forwarded-proto")
                    .map(|v| first_forwarded_token(v) == "https")
                    .unwrap_or(false)
            } else {
                false
            };
            // SEC-028: cookie Secure flag also fires when the operator has
            // explicitly opted into "always Secure" via
            // `SOLI_FORCE_SECURE_COOKIES=1` or `enable_force_secure_cookies()`,
            // covering the TLS-without-trust_proxy / TLS-without-XFP-header case.
            let cookie_secure = is_https
                || crate::interpreter::builtins::secure_cookies::is_force_secure_cookies_enabled();
            // Driver-aware: ID drivers re-emit only when the resolved ID
            // differs from the cookie's; the cookie driver re-emits when the
            // incoming blob was invalid/expired and got replaced. Uses the
            // explicit locals because the thread-local session ID was cleared
            // above.
            let mut headers = vec![(
                "Content-Type".to_string(),
                "text/html; charset=utf-8".to_string(),
            )];
            if let Some(cookie_value) = finalize_session_cookie(
                session_id.as_deref(),
                cookie_session_id.as_deref(),
                cookie_secure,
            ) {
                headers.push(("Set-Cookie".to_string(), cookie_value));
            }
            return ResponseData {
                status: 404,
                headers,
                body: error_html.into_bytes(),
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
                body: error_html.into_bytes(),
            };
        }
    };

    // Record the matched route (post wildcard-expansion) for the dev bar's
    // "requests" panel + the `X-Soli-Route` header. Early-outs on the gate in
    // production. 404s never reach here, so a miss leaves the route unset.
    route_log::record(&handler_name);

    // Skip body parsing for GET/HEAD requests (no body to parse)
    let parsed_body = if data.method == "GET" || data.method == "HEAD" {
        ParsedBody::default()
    } else {
        let content_type = header_str(&data.headers, "content-type");
        parse_request_body(
            &data.body,
            content_type,
            data.multipart_form.as_deref(),
            data.multipart_files.as_ref(),
        )
    };

    // Read scheme + host out of the headers BEFORE `std::mem::take` strips
    // them. `is_https` is also used further down (session cookie Secure
    // flag) — keep it computed here rather than re-reading the now-empty
    // headers map. The host falls back to the `Host` header per RFC 7230
    // when no proxy header is present; empty string is fine — `*_url` will
    // reject it with a clear error. `X-Forwarded-*` are honored only when
    // `enable_trust_proxy()` has been opted into; otherwise an attacker on
    // a directly-exposed deploy could spoof the scheme/host used to set the
    // session-cookie `Secure` flag and to build absolute URL helpers.
    let trust_proxy = crate::interpreter::builtins::trust_proxy::is_trust_proxy_enabled();
    let is_https = if trust_proxy {
        header_str(&data.headers, "x-forwarded-proto")
            .map(|v| first_forwarded_token(v) == "https")
            .unwrap_or(false)
    } else {
        false
    };
    // SEC-028: cookie Secure flag uses `is_https || force_secure_cookies()`
    // so a TLS deployment without `enable_trust_proxy()` (or without an
    // X-Forwarded-Proto: https header) still emits Secure cookies once the
    // operator opts in.
    let cookie_secure =
        is_https || crate::interpreter::builtins::secure_cookies::is_force_secure_cookies_enabled();
    let req_host = if trust_proxy {
        // SEC-044: take only the first comma-separated entry from
        // X-Forwarded-Host. A nginx-style appending proxy sends
        // "real, attacker" when a client supplied an XFH already; the
        // leftmost token is the value the trusted proxy wrote, so use
        // that for cookie / *_url decisions instead of the verbatim
        // string.
        header_str(&data.headers, "x-forwarded-host")
            .map(|v| first_forwarded_token(v).to_string())
            .or_else(|| header_str(&data.headers, "host").map(|v| v.to_string()))
            .unwrap_or_default()
    } else {
        header_str(&data.headers, "host")
            .map(|v| v.to_string())
            .unwrap_or_default()
    };

    // Capture whether this is an HTMx partial-swap request before `headers`
    // is moved into `RequestData`. HTMx returns the response fragment into
    // the live DOM, where the page-level dev bar already exists — injecting
    // a second one into the fragment produces stacked bars.
    let is_htmx_request = dev_bar::is_htmx_request(header_str(&data.headers, "hx-request"));

    // Take ownership of headers and query to avoid cloning individual keys/values.
    // This is the ONE place the wire headers become owned Strings: straight
    // from hyper's HeaderMap into the Soli HashPairs handlers see as
    // req["headers"] (non-UTF-8 values are skipped, as before).
    let wire_headers = std::mem::take(&mut data.headers);
    let mut headers =
        HashPairs::with_capacity_and_hasher(wire_headers.keys_len(), ahash::RandomState::default());
    for (name, value) in &wire_headers {
        if let Ok(v) = value.to_str() {
            headers.insert(
                HashKey::String(name.as_str().into()),
                Value::String(v.into()),
            );
        }
    }
    let query = std::mem::take(&mut data.query);

    // The single cookie parse from above becomes `req["cookies"]`; keep an
    // Rc handle so the `cookies` global below reuses it without re-probing
    // the request hash.
    let cookies_value = Value::Hash(Rc::new(RefCell::new(cookie_pairs)));

    // Build request hash with parsed body (owned headers/query avoid String
    // clones). Also hands back the "all" params value so the `params` global
    // below doesn't re-probe the hash by string key.
    let (mut request_hash, all_params) = build_request_hash_with_parsed(
        &data.method,
        &data.path,
        matched_params,
        query,
        headers,
        cookies_value.clone(),
        &data.body,
        parsed_body,
        &data.peer_ip,
    );

    // Publish scheme + host to the per-request thread-local so `<name>_url`
    // helpers can build absolute URLs without threading the request through
    // every callsite. Cleared in `finalize_response` below.
    let req_scheme = if is_https { "https" } else { "http" }.to_string();
    crate::interpreter::builtins::named_routes::set_current_request_host(req_scheme, req_host);

    // Expose params and cookies as globals so middleware, handlers, and views
    // can reference them directly. Both values are already in hand (returned
    // by build_request_hash_with_parsed / created above) — no string-key
    // re-probe of the request hash. They are also re-set inside
    // dispatch_request after middleware may have modified the request hash.
    let middleware_params =
        all_params.unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))));
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("params", middleware_params.clone());
    if let Some(vm_ref) = vm.as_mut() {
        vm_ref
            .globals
            .insert("params".to_string(), middleware_params);
    }
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("cookies", cookies_value.clone());
    if let Some(vm_ref) = vm.as_mut() {
        vm_ref.globals.insert("cookies".to_string(), cookies_value);
    }

    // Helper to finalize response with session cookie and timing
    let finalize_response = |mut resp: ResponseData| -> ResponseData {
        // Drop the per-request scheme/host so a `<name>_url` call between
        // requests (e.g. from a background timer) errors clearly instead of
        // building a URL with a stale host.
        crate::interpreter::builtins::named_routes::clear_current_request_host();
        if let Some(cookie_value) = finalize_session_cookie(
            get_current_session_id().as_deref(),
            cookie_session_id.as_deref(),
            cookie_secure,
        ) {
            resp.headers.push(("Set-Cookie".to_string(), cookie_value));
        }
        // Emit any response cookies accumulated via set_cookie()
        for (name, value, attrs) in take_response_cookies() {
            resp.headers.push((
                "Set-Cookie".to_string(),
                format!("{}={}{}", name, value, attrs),
            ));
        }
        // Add security headers if enabled
        {
            use crate::interpreter::builtins::security_headers::get_security_headers;
            let security_headers = get_security_headers();
            for (name, value) in security_headers {
                resp.headers.push((name, value));
            }
        }
        // E2E test client: ship the render captured by render() back as
        // response headers, so assigns()/view_path()/render_template() work
        // across the test-runner -> server process boundary. The locals JSON
        // is base64-encoded so arbitrary UTF-8 / control chars stay
        // header-safe. Test-runner only; absent (no render) -> no headers ->
        // render_template() reports false on redirects/JSON responses.
        if crate::interpreter::builtins::test_server::is_test_runner_process() {
            if let Some(captured) =
                crate::interpreter::builtins::test_server::take_captured_render()
            {
                let assigns_b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    captured.assigns_json.as_bytes(),
                );
                resp.headers
                    .push(("x-soli-test-view-path".to_string(), captured.view_path));
                resp.headers
                    .push(("x-soli-test-assigns".to_string(), assigns_b64));
                if captured.partial {
                    resp.headers
                        .push(("x-soli-test-assigns-partial".to_string(), "1".to_string()));
                }
            }
            // Ship the AQL query count + any N+1 groups back to the test-runner
            // process so `assert_query_count` / `assert_no_n_plus_one` can inspect
            // them across the process boundary. The query log is a thread-local on
            // this worker; snapshot it here (before we cross back to hyper) and
            // reuse the dev bar's own detector so a spec sees exactly what the
            // dev-bar badge would flag. The runner always runs the server with
            // `--dev`, so the log is populated.
            {
                use crate::interpreter::builtins::model::query_log;
                let queries = query_log::snapshot();
                resp.headers.push((
                    "x-soli-test-query-count".to_string(),
                    queries.len().to_string(),
                ));
                let n1_groups = dev_bar::detect_n_plus_one(&queries, 2);
                if !n1_groups.is_empty() {
                    let arr: Vec<serde_json::Value> = n1_groups
                        .iter()
                        .map(|(template, count, _total_us)| {
                            serde_json::json!({ "query": template, "count": count })
                        })
                        .collect();
                    let b64 = base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        serde_json::Value::Array(arr).to_string().as_bytes(),
                    );
                    resp.headers.push(("x-soli-test-n1".to_string(), b64));
                }
            }
        }
        // Inject the dev bar into HTML responses when running --dev. The bar
        // is rendered here on the worker thread because the AQL query log is
        // a thread-local, so the snapshot must happen before we cross the
        // channel back to the hyper handler.
        if let Some(start) = dev_started {
            // Tag EVERY dev-mode response (HTML, JSON, HTMx fragment, …) with
            // its matched route so the client-side fetch/XHR patch can attribute
            // each request to a controller#action in the dev bar's requests
            // panel. Set before the HTML-only guard below so XHR/HTMx responses
            // — which never get a dev bar injected — still carry it.
            let route = route_log::snapshot();
            if let Some(handler) = &route {
                resp.headers
                    .push(("X-Soli-Route".to_string(), handler.clone()));
            }
            // Stable per-request id: the requests-panel drill-down fetches
            // `/__solidev/request/:id` to inspect any listed request's panels.
            let request_id = Uuid::new_v4().to_string();
            resp.headers
                .push(("X-Soli-Request-Id".to_string(), request_id.clone()));

            // Server-side handler time (µs). The requests panel shows this as
            // each row's duration — the real time spent in the app — rather
            // than the client-observed round-trip (which folds in queue +
            // network + transfer and is what `performance.now()` would measure).
            let elapsed_us = start.elapsed().as_micros() as u64;
            resp.headers
                .push(("X-Soli-Render-Us".to_string(), elapsed_us.to_string()));

            let is_html = resp
                .headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("text/html"));

            // Build the per-request snapshot for EVERY dev response (not just
            // HTML), so an XHR/HTMx call's panels can be inspected later via the
            // requests panel. Close the root span first so the flamegraph has
            // its top-level rectangle (span_log only records a span on close).
            span_log::close_request_root();
            let ctx = dev_bar::DevBarContext {
                method: method.as_ref().to_string(),
                path: path.clone(),
                status: resp.status,
                elapsed_us,
                request_id: request_id.clone(),
                route: route.clone(),
                queries: crate::interpreter::builtins::model::query_log::snapshot(),
                http_requests: crate::interpreter::builtins::http_log::snapshot(),
                kv_calls: crate::interpreter::builtins::kv_log::snapshot(),
                phases: phase_log::snapshot(),
                middlewares: middleware_log::snapshot(),
                views: view_log::snapshot(),
                spans: span_log::snapshot(),
                warnings: template_warnings::snapshot(),
            };

            // Feed coarse totals into the always-on Prometheus metrics (Phase A).
            // Kept HTML-scoped, matching the prior behavior.
            if is_html {
                let mw_total_us: u64 = ctx.middlewares.iter().map(|(_, us)| *us).sum();
                if mw_total_us > 0 {
                    crate::metrics::Metrics::global()
                        .record_middleware(std::time::Duration::from_micros(mw_total_us));
                }
                let db_total_ns: u64 = ctx
                    .queries
                    .iter()
                    .map(|q| (q.duration_ms * 1_000_000.0) as u64)
                    .sum();
                if db_total_ns > 0 {
                    crate::metrics::Metrics::global()
                        .record_db_queries(std::time::Duration::from_nanos(db_total_ns));
                }
            }

            // Stash for the `/__solidev/request/:id` drill-down endpoint, and
            // the raw request for the `/__solidev/replay/:id` replay button.
            if let Some(raw) = &captured_raw {
                dev_store::put_raw(request_id.clone(), raw.clone());
            }
            dev_store::put(request_id, ctx.clone());

            // Inject the bar only into full HTML pages. HTMx partial responses
            // share the page that already carries the dev bar; injecting again
            // would append a second one into the live DOM on each swap.
            if is_html && !is_htmx_request {
                if let Ok(body_str) = std::str::from_utf8(&resp.body) {
                    resp.body = dev_bar::inject_dev_bar(body_str, &ctx).into_bytes();
                }
            }
        }
        // Log timing (skip health checks to avoid benchmark noise)
        if log_requests && path != "/health" {
            let elapsed_ms = start_time.unwrap().elapsed().as_secs_f64() * 1000.0;
            if dev_mode {
                // The injected dev bar already surfaces the per-request
                // queries/http/timing detail; the terminal just gets the
                // one-line access entry.
                println!(
                    "{} [LOG] {} {} - {} ({:.3}ms)",
                    log_timestamp(),
                    method,
                    path,
                    resp.status,
                    elapsed_ms
                );
            } else {
                // Production: emit the access line plus whatever detail
                // channels the operator enabled via SOLI_LOG. `emit` also
                // gates the SOLI_SLOW_REQUEST_MS full-detail block on
                // queue + handler time and prints nothing for fast
                // requests when only the slow threshold is configured.
                prod_log::emit(
                    method.as_ref(),
                    path.as_str(),
                    resp.status,
                    elapsed_ms,
                    queue_ms,
                    log_channels,
                );
            }
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
        let (mw_name, mw_source, mw_span) = middleware_source_info(mw, None);
        let call_result = invoke_middleware_with_frame(
            interpreter,
            &mw_name,
            mw_source.as_deref(),
            mw_span,
            mw.clone(),
            request_hash.clone(),
        );
        match call_result {
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
                        let error_html = error_pages::render_error_page(
                            &err,
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
                            body: error_html.into_bytes(),
                        });
                    }
                    return finalize_response(middleware_prod_error_string(
                        interpreter,
                        data,
                        &mw_name,
                        mw_source.as_deref(),
                        &err,
                    ));
                }
            },
            Err(e) => {
                if dev_mode {
                    // Prefer the captured stack trace (populated by the inner
                    // call_function) over interpreter.get_stack_trace(), then
                    // fall back to a synthetic middleware frame so the error
                    // page can still show the source file.
                    let captured = e.breakpoint_stack_trace().map(|st| st.to_vec());
                    let stack_trace = captured.unwrap_or_else(|| {
                        middleware_fallback_stack(&mw_name, mw_source.as_deref())
                    });
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
                    return finalize_response(ResponseData {
                        status: if e.is_breakpoint() { 200 } else { 500 },
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html.into_bytes(),
                    });
                }
                return finalize_response(middleware_prod_error_runtime(
                    interpreter,
                    data,
                    &mw_name,
                    mw_source.as_deref(),
                    &e,
                ));
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

        let (mw_name, mw_source, mw_span) =
            middleware_source_info(&mw.handler, Some(mw.name.as_str()));
        let call_result = invoke_middleware_with_frame(
            interpreter,
            &mw_name,
            mw_source.as_deref(),
            mw_span,
            mw.handler.clone(),
            request_hash.clone(),
        );
        match call_result {
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
                        let error_html = error_pages::render_error_page(
                            &err,
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
                            body: error_html.into_bytes(),
                        });
                    }
                    return finalize_response(middleware_prod_error_string(
                        interpreter,
                        data,
                        &mw_name,
                        mw_source.as_deref(),
                        &err,
                    ));
                }
            },
            Err(e) => {
                if dev_mode {
                    let captured = e.breakpoint_stack_trace().map(|st| st.to_vec());
                    let stack_trace = captured.unwrap_or_else(|| {
                        middleware_fallback_stack(&mw_name, mw_source.as_deref())
                    });
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
                    return finalize_response(ResponseData {
                        status: if e.is_breakpoint() { 200 } else { 500 },
                        headers: vec![(
                            "Content-Type".to_string(),
                            "text/html; charset=utf-8".to_string(),
                        )],
                        body: error_html.into_bytes(),
                    });
                }
                return finalize_response(middleware_prod_error_runtime(
                    interpreter,
                    data,
                    &mw_name,
                    mw_source.as_deref(),
                    &e,
                ));
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
async fn handle_dev_repl(
    req: Request<Incoming>,
    peer_addr: SocketAddr,
) -> Result<Response<ResponseBody>, hyper::Error> {
    if !is_authorized_dev_repl_request(req.headers(), peer_addr) {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(
                r#"{"error": "Forbidden dev REPL request"}"#,
            )))
            .unwrap());
    }

    let max_body = crate::interpreter::builtins::body_limit::get_max_body_size();
    let body = match BodyExt::collect(Limited::new(req.into_body(), max_body)).await {
        Ok(b) => b.to_bytes(),
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::PAYLOAD_TOO_LARGE)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(r#"{"error": "Request body too large"}"#)))
                .unwrap());
        }
    };
    let body_str = String::from_utf8_lossy(&body);

    // Parse JSON body
    let json: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(json) => json,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(r#"{"error": "Invalid JSON body"}"#)))
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
        .body(full(Bytes::from(response_json.to_string())))
        .unwrap())
}

/// A peer is "trusted" for the dev REPL only when it's on the host itself
/// (loopback). The dev REPL is arbitrary server-side code execution, so
/// trusting the whole private LAN meant any co-resident host (office/café
/// Wi-Fi, shared container network) could scrape the auto-generated token
/// from a dev error page and POST code — a LAN-wide RCE whenever `--dev`
/// is bound to a non-loopback address (e.g. `0.0.0.0`, common for "test
/// from my phone"). Accessing the REPL from another device is now an
/// explicit opt-in: set `SOLI_DEV_REPL_ALLOW_REMOTE=1` *and* a stable
/// `SOLI_DEV_REPL_SECRET` (the startup check enforces the pairing).
pub(super) fn is_trusted_dev_peer(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback(),
        std::net::IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return true;
            }
            // IPv4-mapped IPv6 (`::ffff:127.0.0.1`) is the same peer as the
            // wrapped v4 — apply the v4 loopback rule so a dual-stack
            // listener still recognizes the local host.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback();
            }
            false
        }
    }
}

/// String-keyed variant so callers that only hold `RequestData::peer_ip`
/// (which is a pre-stringified IP, no port) don't need to re-parse a
/// SocketAddr. A malformed string is treated as untrusted.
pub(super) fn is_trusted_dev_peer_str(peer_ip: &str) -> bool {
    peer_ip
        .parse::<std::net::IpAddr>()
        .map(is_trusted_dev_peer)
        .unwrap_or(false)
}

fn is_authorized_dev_repl_request(headers: &hyper::HeaderMap, peer_addr: SocketAddr) -> bool {
    if !is_trusted_dev_peer(peer_addr.ip()) && !dev_repl_allows_remote() {
        return false;
    }

    let Some(header_token) = headers
        .get("x-soli-dev-token")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };

    constant_time_eq(header_token, dev_repl_auth_token())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

fn html_ok(html: String) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(full(Bytes::from(html)))
        .unwrap()
}

/// Read a view template's raw `.html.slv` source by views-relative path (VFS-aware).
fn view_raw_source(views_dir: &std::path::Path, rel: &str) -> Option<String> {
    let path = views_dir.join(format!("{}.html.slv", rel));
    vfs_read_to_string(&path.to_string_lossy()).ok()
}

/// Read a component's raw `.html.slv` source.
fn component_raw_source(views_dir: &std::path::Path, name: &str) -> Option<String> {
    view_raw_source(views_dir, &format!("components/{}", name))
}

/// Extract example preview data from a leading `<%# preview: {json} %>` header;
/// an empty hash when absent or malformed.
fn component_preview_data(raw: &str) -> crate::interpreter::value::Value {
    use crate::interpreter::value::{HashPairs, Value};
    let empty = || {
        Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
            HashPairs::default(),
        )))
    };
    let Some(start) = raw.find("<%#") else {
        return empty();
    };
    let after = &raw[start + 3..];
    let Some(end) = after.find("%>") else {
        return empty();
    };
    let Some(json_str) = after[..end].trim().strip_prefix("preview:") else {
        return empty();
    };
    let Ok(j) = serde_json::from_str::<serde_json::Value>(json_str.trim()) else {
        return empty();
    };
    match crate::interpreter::value_json::json_to_value_ref(&j) {
        Ok(v) => {
            crate::interpreter::builtins::template::inject_template_helpers(&v);
            v
        }
        Err(_) => empty(),
    }
}

/// Names declared via `props("a", "b")` in a component's source (display only).
fn component_declared_props(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = raw;
    while let Some(pos) = rest.find("props(") {
        let after = &rest[pos + 6..];
        let end = after.find(')').unwrap_or(after.len());
        let args = &after[..end];
        let mut in_str = false;
        let mut cur = String::new();
        for c in args.chars() {
            if c == '"' {
                if in_str {
                    if !cur.is_empty() && !out.contains(&cur) {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                    in_str = false;
                } else {
                    in_str = true;
                }
            } else if in_str {
                cur.push(c);
            }
        }
        rest = &after[end..];
    }
    out
}

fn catalog_shell(heading: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Soli \u{b7} {heading}</title>\
<style>body{{margin:0;font-family:'JetBrains Mono',ui-monospace,monospace;background:#08090b;color:#c9d1d9;padding:1.5rem;}}\
h1{{font-size:14px;letter-spacing:0.08em;color:#8b949e;font-weight:600;margin:0 0 0.25rem;}}\
a:hover{{text-decoration:underline;}}</style></head>\
<body><h1>SOLI \u{b7} {heading}</h1>\
<p style=\"font-size:11px;color:#8b949e;margin:0 0 1.25rem;\">Dev-only. Previews render with built-in helpers plus any \
<code>&lt;%# preview: {{...}} %&gt;</code> data; app-defined view helpers and request context aren't available here.</p>\
{body}</body></html>",
        heading = heading,
        body = body,
    )
}

/// Dev-only component catalog index (`GET /__soli/components`).
fn handle_component_catalog() -> Response<ResponseBody> {
    let cache = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(c) => c,
        Err(e) => {
            return html_ok(catalog_shell(
                "COMPONENT CATALOG",
                &format!(
                    "<p style=\"color:#ff6b6b\">Template cache unavailable: {}</p>",
                    dev_bar::html_escape(&e)
                ),
            ))
        }
    };
    let views_dir = cache.views_dir().to_path_buf();
    let comp_dir = views_dir.join("components");
    let dir_str = comp_dir.to_string_lossy().to_string();
    let prefix = format!("{}/", dir_str.trim_end_matches('/'));
    let mut names: Vec<String> = vfs_walk_dir(&dir_str)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.ends_with(".html.slv"))
        .map(|f| {
            f.strip_prefix(&prefix)
                .unwrap_or(&f)
                .trim_end_matches(".html.slv")
                .to_string()
        })
        .filter(|n| crate::template::is_safe_template_name(n))
        .collect();
    names.sort();
    names.dedup();

    if names.is_empty() {
        return html_ok(catalog_shell(
            "COMPONENT CATALOG",
            "<p style=\"color:#8b949e\">No components found in <code>app/views/components/</code>.</p>",
        ));
    }

    let mut cards = String::new();
    for name in &names {
        let esc = dev_bar::html_escape(name);
        let raw = component_raw_source(&views_dir, name).unwrap_or_default();
        let declared = component_declared_props(&raw);
        let declared_html = if declared.is_empty() {
            String::new()
        } else {
            format!(
                "<div style=\"font-size:11px;color:#8b949e;margin-top:0.2rem;\">props: {}</div>",
                dev_bar::html_escape(&declared.join(", "))
            )
        };
        cards.push_str(&format!(
            "<div style=\"border:1px solid #30363d;border-radius:6px;overflow:hidden;\">\
<div style=\"padding:0.5rem 0.75rem;border-bottom:1px solid #30363d;background:#0b0d0f;\">\
<a href=\"/__soli/components/{esc}\" style=\"color:#8be9fd;text-decoration:none;font-weight:600;\">{esc}</a>{declared_html}\
</div>\
<iframe src=\"/__soli/components/{esc}\" style=\"width:100%;height:190px;border:0;background:#fff;\" title=\"{esc}\"></iframe>\
</div>",
        ));
    }
    html_ok(catalog_shell(
        "COMPONENT CATALOG",
        &format!(
            "<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:1rem;\">{}</div>",
            cards
        ),
    ))
}

/// Dev-only single-component preview (`GET /__soli/components/<name>`), used by
/// the catalog iframes and directly linkable.
fn handle_component_preview(name: &str) -> Response<ResponseBody> {
    if !crate::template::is_safe_template_name(name) {
        return html_ok("<!doctype html><p>invalid component name</p>".to_string());
    }
    let inner = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(cache) => {
            let raw = component_raw_source(cache.views_dir(), name).unwrap_or_default();
            let data = component_preview_data(&raw);
            match cache.render_component(name, &data) {
                Ok(html) => html,
                Err(e) => format!(
                    "<pre style=\"color:#b00\">render error: {}</pre>",
                    dev_bar::html_escape(&e)
                ),
            }
        }
        Err(e) => format!("template cache unavailable: {}", dev_bar::html_escape(&e)),
    };
    // Bare doc + the app stylesheet (best-effort) so previews approximate reality.
    html_ok(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<link rel=\"stylesheet\" href=\"/css/application.css\">\
<style>body{{margin:0;padding:1rem;font-family:system-ui,sans-serif;}}</style>\
</head><body>{}</body></html>",
        inner
    ))
}

/// From a flat list of view file paths (as returned by `vfs_walk_dir`), pick the
/// sorted, deduped `<mailer>/<action>` names: `.html.slv` files directly under a
/// `*_mailer/` directory. The `.text.slv` companions don't end in `.html.slv`,
/// so they're excluded; unsafe/traversal names are dropped.
fn mailer_view_names(files: &[String], prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = files
        .iter()
        .filter(|f| f.ends_with(".html.slv"))
        .filter_map(|f| {
            let rel = f
                .strip_prefix(prefix)
                .unwrap_or(f)
                .trim_end_matches(".html.slv")
                .to_string();
            let (dir, _action) = rel.split_once('/')?;
            if dir.ends_with("_mailer") {
                Some(rel)
            } else {
                None
            }
        })
        .filter(|n| crate::template::is_safe_template_name(n))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Dev-only mailer preview gallery index (`GET /__soli/mailers`). Lists every
/// `app/views/<name>_mailer/<action>.html.slv` view and previews each in an
/// iframe — the email equivalent of the component catalog.
fn handle_mailer_catalog() -> Response<ResponseBody> {
    let cache = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(c) => c,
        Err(e) => {
            return html_ok(catalog_shell(
                "MAILER PREVIEWS",
                &format!(
                    "<p style=\"color:#ff6b6b\">Template cache unavailable: {}</p>",
                    dev_bar::html_escape(&e)
                ),
            ))
        }
    };
    let views_dir = cache.views_dir().to_path_buf();
    let dir_str = views_dir.to_string_lossy().to_string();
    let prefix = format!("{}/", dir_str.trim_end_matches('/'));
    let names = mailer_view_names(&vfs_walk_dir(&dir_str).unwrap_or_default(), &prefix);

    if names.is_empty() {
        return html_ok(catalog_shell(
            "MAILER PREVIEWS",
            "<p style=\"color:#8b949e\">No mailer views found. Generate one with \
<code>soli generate mailer user welcome</code>.</p>",
        ));
    }

    let mut cards = String::new();
    for rel in &names {
        let esc = dev_bar::html_escape(rel);
        cards.push_str(&format!(
            "<div style=\"border:1px solid #30363d;border-radius:6px;overflow:hidden;\">\
<div style=\"padding:0.5rem 0.75rem;border-bottom:1px solid #30363d;background:#0b0d0f;\">\
<a href=\"/__soli/mailers/{esc}\" style=\"color:#8be9fd;text-decoration:none;font-weight:600;\">{esc}</a>\
</div>\
<iframe src=\"/__soli/mailers/{esc}\" style=\"width:100%;height:320px;border:0;background:#fff;\" title=\"{esc}\"></iframe>\
</div>",
        ));
    }
    html_ok(catalog_shell(
        "MAILER PREVIEWS",
        &format!(
            "<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(360px,1fr));gap:1rem;\">{}</div>",
            cards
        ),
    ))
}

/// Dev-only single mailer preview (`GET /__soli/mailers/<mailer>/<action>`),
/// used by the gallery iframes and directly linkable. Renders the HTML body
/// only (no layout), with example data from a `<%# preview: {json} %>` header.
fn handle_mailer_preview(rel: &str) -> Response<ResponseBody> {
    if !crate::template::is_safe_template_name(rel) {
        return html_ok("<!doctype html><p>invalid mailer template</p>".to_string());
    }
    let inner = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(cache) => {
            let raw = view_raw_source(cache.views_dir(), rel).unwrap_or_default();
            let data = component_preview_data(&raw);
            match cache.render(rel, &data, Some(None)) {
                Ok(html) => html,
                Err(e) => format!(
                    "<pre style=\"color:#b00\">render error: {}</pre>",
                    dev_bar::html_escape(&e)
                ),
            }
        }
        Err(e) => format!("template cache unavailable: {}", dev_bar::html_escape(&e)),
    };
    // Mailer bodies bring their own markup; render them on a plain white page.
    html_ok(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<style>body{{margin:0;padding:1rem;font-family:system-ui,sans-serif;background:#fff;color:#111;}}</style>\
</head><body>{}</body></html>",
        inner
    ))
}

/// Dev-only request replay (`POST /__solidev/replay/:id`). Re-dispatches a
/// previously captured request through the real worker path so a bug can be
/// reproduced server-side (fresh request id, handler re-runs). Returns the
/// replayed response tagged with `X-Soli-Replay: 1`; its new
/// `X-Soli-Request-Id` lets the dev bar retarget its panels to the replay.
async fn handle_replay(id: &str, request_tx: &WorkerSender) -> Response<ResponseBody> {
    let Some(raw) = dev_store::get_raw(id) else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(full(Bytes::from("unknown or expired request id")))
            .unwrap();
    };

    let (response_tx, response_rx) = oneshot::channel();
    let request_data = RequestData {
        method: Cow::Owned(raw.method.clone()),
        path: raw.path.clone(),
        query: raw.query.clone(),
        headers: raw.headers.clone(),
        body: raw.body.clone(),
        // Multipart re-parsing is punted for v1: a replayed multipart POST
        // carries the raw body but no pre-parsed fields/files.
        body_bytes: None,
        multipart_form: None,
        multipart_files: None,
        peer_ip: raw.peer_ip.clone(),
        enqueued_at: prod_log::channels().any().then(std::time::Instant::now),
        replay: true,
        response_tx,
    };

    // Mirror the main dispatch's non-blocking send loop.
    let mut pending = Some(request_data);
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(server_constants::REQUEST_TIMEOUT_SECS);
    let send_ok = loop {
        if let Some(data) = pending.take() {
            match request_tx.try_send(data) {
                Ok(()) => break true,
                Err(crossbeam::channel::TrySendError::Full(returned)) => {
                    if tokio::time::Instant::now() >= deadline {
                        break false;
                    }
                    pending = Some(returned);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(crossbeam::channel::TrySendError::Disconnected(_)) => break false,
            }
        }
    };
    if !send_ok {
        return Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(full(Bytes::from("Server busy")))
            .unwrap();
    }

    let worker_response = match tokio::time::timeout(
        Duration::from_secs(server_constants::RESPONSE_WAIT_TIMEOUT_SECS),
        response_rx,
    )
    .await
    {
        Ok(Ok(resp)) => resp,
        Ok(Err(_)) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(full(Bytes::from("worker dropped replay")))
                .unwrap();
        }
        Err(_) => {
            return Response::builder()
                .status(StatusCode::GATEWAY_TIMEOUT)
                .header("Server", "soliMVC")
                .body(full(Bytes::from("Gateway Timeout")))
                .unwrap();
        }
    };

    // Materialize the response (streaming replays are collected — dev-only and
    // small) and tag it so the dev bar can show a replay badge.
    let (status, headers, body) = match worker_response {
        WorkerResponse::Buffered(rd) => (rd.status, rd.headers, Bytes::from(rd.body)),
        WorkerResponse::Stream {
            status,
            headers,
            mut rx,
        } => {
            let mut buf = Vec::new();
            while let Some(chunk) = rx.recv().await {
                buf.extend_from_slice(&chunk);
            }
            (status, headers, Bytes::from(buf))
        }
    };

    let mut builder = Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
        .header("Server", "soliMVC")
        .header("X-Soli-Replay", "1");
    for (key, value) in &headers {
        builder = add_header_checked(builder, key.as_str(), value.as_str());
    }
    builder
        .body(full(body))
        .unwrap_or_else(|_| Response::new(full(Bytes::from("replay response error"))))
}

// ---------------------------------------------------------------------------
// Dev-only database browser (`/__soli/db`)
// ---------------------------------------------------------------------------

/// Dark, dev-bar-styled page chrome for the DB browser (adds table CSS).
fn db_page(body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Soli \u{b7} Database</title>\
<style>body{{margin:0;font-family:'JetBrains Mono',ui-monospace,monospace;background:#08090b;color:#c9d1d9;padding:1.5rem;}}\
h1{{font-size:14px;letter-spacing:0.08em;color:#8b949e;font-weight:600;margin:0 0 0.75rem;}}\
a{{color:#8be9fd;text-decoration:none;}}a:hover{{text-decoration:underline;}}\
table{{border-collapse:collapse;width:100%;font-size:11px;}}\
th,td{{border:1px solid #30363d;padding:0.3rem 0.5rem;text-align:left;vertical-align:top;max-width:420px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}}\
th{{background:#0b0d0f;color:#8b949e;position:sticky;top:0;}}\
pre{{background:#0b0d0f;border:1px solid #30363d;border-radius:6px;padding:0.75rem;overflow:auto;font-size:12px;}}\
textarea{{width:100%;box-sizing:border-box;background:#0b0d0f;color:#c9d1d9;border:1px solid #30363d;border-radius:6px;padding:0.5rem;font:inherit;}}\
button{{background:#1f6feb;color:#fff;border:0;border-radius:6px;padding:0.4rem 0.9rem;font:inherit;cursor:pointer;}}\
.muted{{color:#8b949e;font-size:11px;}}.err{{color:#ff6b6b;}}</style></head>\
<body><h1><a href=\"/__soli/db\">SOLI \u{b7} DATABASE</a></h1>{body}</body></html>",
    )
}

/// Dev-only DB browser error page (message is escaped).
fn db_error_page(msg: &str) -> Response<ResponseBody> {
    html_ok(db_page(&format!(
        "<p class=\"err\">{}</p>",
        dev_bar::html_escape(msg)
    )))
}

/// Build a SoliDB client from the model DB config (auth + database), for the
/// dev browser. Mirrors `jobs::make_client`.
fn db_browser_client() -> Result<crate::solidb_http::SoliDBClient, String> {
    use crate::interpreter::builtins::model::core::{
        get_api_key, get_basic_auth, get_database_name, get_jwt_token, DB_CONFIG,
    };
    use crate::solidb_http::SoliDBClient;
    let host = &DB_CONFIG.host;
    let mut client =
        SoliDBClient::connect(host).map_err(|e| format!("SoliDB connect failed: {}", e))?;
    if let Some(jwt) = get_jwt_token() {
        client = client.with_jwt_token(&jwt);
    } else if let Some(key) = get_api_key() {
        client = client.with_api_key(key);
    } else if let Some(basic) = get_basic_auth() {
        if let Some(rest) = basic.strip_prefix("Basic ") {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            if let Ok(bytes) = STANDARD.decode(rest) {
                if let Ok(s) = String::from_utf8(bytes) {
                    if let Some((u, p)) = s.split_once(':') {
                        client = client.with_basic_auth(u, p);
                    }
                }
            }
        }
    }
    client.set_database(&get_database_name());
    Ok(client)
}

/// Non-system collection names, sorted. The caller wraps this in block_in_place.
fn db_list_collection_names() -> Result<Vec<String>, String> {
    let client = db_browser_client()?;
    let cols = client.list_collections().map_err(|e| e.to_string())?;
    let mut names: Vec<String> = cols
        .iter()
        .filter_map(|c| c.get("name").and_then(|v| v.as_str()).map(str::to_string))
        .filter(|n| !n.starts_with('_'))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Reject non-read queries (a lexical guard for the dev query box, paired with
/// the collection allow-list; the endpoint is dev-gated regardless).
fn is_write_query(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    ["INSERT", "UPDATE", "REPLACE", "REMOVE", "UPSERT"]
        .iter()
        .any(|kw| {
            upper
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|tok| tok == *kw)
        })
}

/// A collection name safe to interpolate into a query (it can't be a bind var).
fn valid_collection_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Escaped display text for a JSON cell (scalars bare, containers as JSON).
fn db_json_cell(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    dev_bar::html_escape(&s)
}

/// Render an array of row objects as an HTML table (union of keys as columns,
/// `_key`/`_rev` first). Shared by the collection view and query results.
fn db_rows_table(rows: &[serde_json::Value]) -> String {
    if rows.is_empty() {
        return "<p class=\"muted\">No rows.</p>".to_string();
    }
    let mut cols: Vec<String> = Vec::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            for k in obj.keys() {
                if !cols.contains(k) {
                    cols.push(k.clone());
                }
            }
        }
    }
    cols.sort_by_key(|k| match k.as_str() {
        "_key" => (0, k.clone()),
        "_rev" => (1, k.clone()),
        _ => (2, k.clone()),
    });
    let mut html = String::from("<div style=\"overflow-x:auto;\"><table><thead><tr>");
    for c in &cols {
        html.push_str(&format!("<th>{}</th>", dev_bar::html_escape(c)));
    }
    html.push_str("</tr></thead><tbody>");
    for row in rows {
        html.push_str("<tr>");
        let obj = row.as_object();
        for c in &cols {
            let content = obj
                .and_then(|o| o.get(c))
                .map(db_json_cell)
                .unwrap_or_default();
            html.push_str(&format!("<td title=\"{content}\">{content}</td>"));
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table></div>");
    html
}

/// `GET /__soli/db` — collections index + a read-only query box (runs `?q=`).
fn handle_db_index(query: Option<&str>) -> Response<ResponseBody> {
    use crate::interpreter::builtins::model::core::{get_database_name, DB_CONFIG};
    let params = query.map(parse_query_string).unwrap_or_default();

    let names = match tokio::task::block_in_place(db_list_collection_names) {
        Ok(n) => n,
        Err(e) => return db_error_page(&format!("Database unavailable: {}", e)),
    };

    let mut body = format!(
        "<p class=\"muted\">Database <b>{}</b> @ <code>{}</code> \u{b7} {} collections \u{b7} dev-only.</p>",
        dev_bar::html_escape(&get_database_name()),
        dev_bar::html_escape(&DB_CONFIG.host),
        names.len()
    );

    body.push_str("<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:0.4rem;margin:0.75rem 0 1.25rem;\">");
    for n in &names {
        let esc = dev_bar::html_escape(n);
        body.push_str(&format!(
            "<a href=\"/__soli/db/{esc}\" style=\"border:1px solid #30363d;border-radius:6px;padding:0.4rem 0.6rem;\">{esc}</a>"
        ));
    }
    body.push_str("</div>");

    let q = params.get("q").cloned().unwrap_or_default();
    let placeholder = names.first().map(|s| s.as_str()).unwrap_or("collection");
    body.push_str(&format!(
        "<form method=\"get\" action=\"/__soli/db\">\
<div class=\"muted\" style=\"margin-bottom:0.25rem;\">Read-only query (SDBQL) \u{2014} writes are rejected:</div>\
<textarea name=\"q\" rows=\"3\" placeholder=\"FOR d IN {} LIMIT 20 RETURN d\">{}</textarea>\
<div style=\"margin-top:0.4rem;\"><button type=\"submit\">Run</button></div></form>",
        dev_bar::html_escape(placeholder),
        dev_bar::html_escape(&q)
    ));

    if !q.trim().is_empty() {
        if is_write_query(&q) {
            body.push_str("<p class=\"err\" style=\"margin-top:1rem;\">Only read queries are allowed here (INSERT/UPDATE/REPLACE/REMOVE/UPSERT rejected).</p>");
        } else {
            let sql = q.clone();
            match tokio::task::block_in_place(move || {
                crate::interpreter::builtins::model::crud::exec_async_query_with_binds(sql, None)
            }) {
                Ok(rows) => {
                    body.push_str(&format!(
                        "<div class=\"muted\" style=\"margin-top:1rem;\">{} row(s)</div>",
                        rows.len()
                    ));
                    body.push_str(&db_rows_table(&rows));
                }
                Err(e) => body.push_str(&format!(
                    "<p class=\"err\" style=\"margin-top:1rem;\">{}</p>",
                    dev_bar::html_escape(&e)
                )),
            }
        }
    }

    html_ok(db_page(&body))
}

/// `GET /__soli/db/<collection>?page=N&size=M` — paginated rows.
fn handle_db_collection(coll: &str, query: Option<&str>) -> Response<ResponseBody> {
    if !valid_collection_name(coll) {
        return db_error_page("invalid collection name");
    }
    // Allow-list against the real collections: the name is interpolated into
    // the query (it can't be a bind var), so never trust it blindly.
    let known = match tokio::task::block_in_place(db_list_collection_names) {
        Ok(n) => n,
        Err(e) => return db_error_page(&format!("Database unavailable: {}", e)),
    };
    if !known.iter().any(|n| n == coll) {
        return db_error_page(&format!("unknown collection: {}", coll));
    }

    let params = query.map(parse_query_string).unwrap_or_default();
    let page: usize = params.get("page").and_then(|s| s.parse().ok()).unwrap_or(0);
    let size: usize = params
        .get("size")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let offset = page.saturating_mul(size);

    let mut binds = std::collections::HashMap::new();
    binds.insert("off".to_string(), serde_json::json!(offset));
    binds.insert("cnt".to_string(), serde_json::json!(size));
    let sql = format!("FOR d IN {} LIMIT @off, @cnt RETURN d", coll);
    let rows = match tokio::task::block_in_place(move || {
        crate::interpreter::builtins::model::crud::exec_async_query_with_binds(sql, Some(binds))
    }) {
        Ok(r) => r,
        Err(e) => return db_error_page(&e),
    };

    let esc = dev_bar::html_escape(coll);
    let mut body = format!(
        "<p class=\"muted\"><a href=\"/__soli/db\">collections</a> / <b>{}</b> \u{b7} page {} \u{b7} {} row(s) \u{b7} open one at <code>/__soli/db/{}/&lt;_key&gt;</code></p>",
        esc,
        page,
        rows.len(),
        esc
    );
    let mut nav = String::new();
    if page > 0 {
        nav.push_str(&format!(
            "<a href=\"/__soli/db/{esc}?page={}&size={size}\">&larr; prev</a> ",
            page - 1
        ));
    }
    if rows.len() == size {
        nav.push_str(&format!(
            "<a href=\"/__soli/db/{esc}?page={}&size={size}\">next &rarr;</a>",
            page + 1
        ));
    }
    if !nav.is_empty() {
        body.push_str(&format!("<p>{}</p>", nav));
    }
    body.push_str(&db_rows_table(&rows));
    html_ok(db_page(&body))
}

/// `GET /__soli/db/<collection>/<key>` — one document as pretty JSON.
fn handle_db_document(coll: &str, key: &str) -> Response<ResponseBody> {
    if !valid_collection_name(coll) {
        return db_error_page("invalid collection name");
    }
    // Allow-list so a bogus URL can't trip exec_get's auto-create side effect.
    let known = match tokio::task::block_in_place(db_list_collection_names) {
        Ok(n) => n,
        Err(e) => return db_error_page(&format!("Database unavailable: {}", e)),
    };
    if !known.iter().any(|n| n == coll) {
        return db_error_page(&format!("unknown collection: {}", coll));
    }

    let coll_owned = coll.to_string();
    let key_owned = key.to_string();
    let doc = match tokio::task::block_in_place(move || {
        crate::interpreter::builtins::model::crud::exec_get(&coll_owned, &key_owned)
    }) {
        Ok(d) => d,
        Err(e) => return db_error_page(&e),
    };
    let pretty = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string());
    let body = format!(
        "<p class=\"muted\"><a href=\"/__soli/db\">collections</a> / <a href=\"/__soli/db/{c}\">{c}</a> / <b>{k}</b></p><pre>{}</pre>",
        dev_bar::html_escape(&pretty),
        c = dev_bar::html_escape(coll),
        k = dev_bar::html_escape(key),
    );
    html_ok(db_page(&body))
}

/// Handle source code fetching for dev mode.
///
/// SEC-009: this endpoint reads arbitrary files inside `app_root`
/// (`.env`, `app/models/*.sl`, controllers, config). It must share the
/// same `is_authorized_dev_repl_request` gate as `/__dev/repl` so a
/// dev server reachable on a shared box / container / port-forward
/// doesn't leak secrets to anyone who can hit the port.
async fn handle_dev_source(
    req: Request<Incoming>,
    peer_addr: SocketAddr,
) -> Result<Response<ResponseBody>, hyper::Error> {
    if !is_authorized_dev_repl_request(req.headers(), peer_addr) {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(
                r#"{"error": "Forbidden dev source request"}"#,
            )))
            .unwrap());
    }

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
            .body(full(Bytes::from(r#"{"error": "Missing file parameter"}"#)))
            .unwrap());
    }

    // Reject absolute paths - security measure
    if std::path::Path::new(&file).is_absolute() {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(
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
                .body(full(Bytes::from(r#"{"error": "File not found"}"#)))
                .unwrap());
        }
    };

    let canonical_root = match std::fs::canonicalize(&app_root) {
        Ok(r) => r,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(
                    r#"{"error": "Could not determine app root"}"#,
                )))
                .unwrap());
        }
    };

    // SEC-010: use `Path::starts_with` (segment-aware), not the string
    // form. Plain string `starts_with` would treat
    // `/home/me/app-secrets/x` as inside `/home/me/app` because the
    // prefix matches character-by-character — exactly the leak this
    // task was filed for. `resolve_static_file` already uses this idiom.
    if !canonical_path.starts_with(&canonical_root) {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(
                r#"{"error": "Path outside app directory"}"#,
            )))
            .unwrap());
    }

    if !canonical_path.is_file() {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(r#"{"error": "Not a file"}"#)))
            .unwrap());
    }

    let content = match std::fs::read_to_string(&canonical_path) {
        Ok(c) => c,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(r#"{"error": "Could not read file"}"#)))
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
        .body(full(Bytes::from(response.to_string())))
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
        let policies_dir = app_root.join("app/policies");
        if policies_dir.exists() {
            if let Err(e) = load_models(&mut interpreter, &policies_dir) {
                eprintln!("REPL: Error loading policies: {}", e);
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

    // Ensure the canonical file path is within public directory.
    // Use `Path::starts_with` (segment-aware), NOT `str::starts_with`: the
    // string form would let `…/public-evil/x` pass the check against
    // `…/public` because the directory name is a byte-level prefix.
    if !canonical_file.starts_with(&canonical_public) {
        return Err(()); // traversal attempt
    }

    if !canonical_file.is_file() {
        return Ok(None); // directory or special file, fall through
    }

    Ok(Some(file_path))
}

/// Walk the MVC app directories that the test runner also walks for coverage
/// (`app/`, `config/`, `lib/`) and pre-register every `.sl` file's executable
/// lines on the server-side coverage tracker. Without this, lines that are
/// never hit would be absent from the report (the aggregator only knows about
/// lines it has seen hit).
fn register_app_source_lines_for_server(
    tracker: &mut crate::coverage::CoverageTracker,
    app_dir: &Path,
) {
    let source_dirs = [
        app_dir.join("app"),
        app_dir.join("config"),
        app_dir.join("lib"),
    ];
    for source_dir in &source_dirs {
        if source_dir.is_dir() {
            collect_and_register_server_sources(tracker, source_dir);
        }
    }
}

fn collect_and_register_server_sources(tracker: &mut crate::coverage::CoverageTracker, dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_and_register_server_sources(tracker, &path);
            } else if path.extension().is_some_and(|e| e == "sl") {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    tracker.register_executable_lines_from_source(&path, &source);
                }
            }
        }
    }
}

/// Build a JSON response that enumerates every recorded line hit on the
/// server-side global coverage tracker. Consumed by the test runner right
/// before it kills the subprocess so the parent process can merge the data
/// into its own aggregated report.
fn coverage_dump_json() -> String {
    let Some(tracker) = crate::coverage::tracker::get_global_coverage_tracker() else {
        return "{}".to_string();
    };
    let Ok(tracker) = tracker.lock() else {
        return "{}".to_string();
    };
    let coverage = tracker.get_aggregated_coverage();
    let mut out = String::from("{\"files\":[");
    let mut first = true;
    for (path, file_cov) in &coverage.file_coverages {
        if !first {
            out.push(',');
        }
        first = false;
        let path_str = path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        out.push_str(&format!("{{\"path\":\"{}\",\"hits\":[", path_str));
        let mut line_first = true;
        for (line_num, line_cov) in &file_cov.lines {
            if line_cov.hits == 0 {
                continue;
            }
            if !line_first {
                out.push(',');
            }
            line_first = false;
            out.push_str(&format!("[{},{}]", line_num, line_cov.hits));
        }
        out.push_str("]}");
    }
    out.push_str("]}");
    out
}

#[allow(dead_code)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn mailer_view_names_selects_mailer_html_views() {
        let prefix = "/app/views/";
        let files = vec![
            "/app/views/user_mailer/welcome.html.slv".to_string(),
            "/app/views/user_mailer/welcome.text.slv".to_string(), // text companion — excluded
            "/app/views/order_mailer/shipped.html.slv".to_string(),
            "/app/views/components/card.html.slv".to_string(), // not a mailer dir
            "/app/views/home/index.html.slv".to_string(),      // not a mailer dir
            "/app/views/user_mailer/welcome.html.slv".to_string(), // dup
        ];
        let names = mailer_view_names(&files, prefix);
        assert_eq!(
            names,
            vec![
                "order_mailer/shipped".to_string(),
                "user_mailer/welcome".to_string(),
            ]
        );
    }

    #[test]
    fn mailer_view_names_empty_when_no_mailers() {
        let prefix = "/app/views/";
        let files = vec![
            "/app/views/components/card.html.slv".to_string(),
            "/app/views/home/index.html.slv".to_string(),
        ];
        assert!(mailer_view_names(&files, prefix).is_empty());
    }

    #[test]
    fn db_write_query_guard_rejects_mutations_only() {
        assert!(is_write_query("FOR d IN posts REMOVE d IN posts"));
        assert!(is_write_query("INSERT {a:1} INTO posts"));
        assert!(is_write_query("update posts set x=1")); // case-insensitive
        assert!(is_write_query("UPSERT {a:1} INSERT {} UPDATE {} IN c"));
        // Reads pass, including field/collection names that merely contain a
        // keyword as a substring (word-boundary check).
        assert!(!is_write_query("FOR d IN posts LIMIT 20 RETURN d"));
        assert!(!is_write_query("FOR d IN updates RETURN d")); // 'updates' != 'update'
        assert!(!is_write_query("FOR d IN posts RETURN d.inserted_at"));
    }

    #[test]
    fn db_collection_name_validation() {
        assert!(valid_collection_name("posts"));
        assert!(valid_collection_name("user_sessions-2"));
        assert!(!valid_collection_name(""));
        assert!(!valid_collection_name("posts; DROP")); // space/semicolon
        assert!(!valid_collection_name("a/b"));
        assert!(!valid_collection_name(&"x".repeat(200)));
    }

    #[test]
    fn db_rows_table_orders_key_columns_first() {
        let rows = vec![
            serde_json::json!({ "title": "a", "_key": "1", "_rev": "r1" }),
            serde_json::json!({ "title": "b", "_key": "2", "_rev": "r2", "extra": 9 }),
        ];
        let html = db_rows_table(&rows);
        let key_pos = html.find("<th>_key</th>").unwrap();
        let rev_pos = html.find("<th>_rev</th>").unwrap();
        let title_pos = html.find("<th>title</th>").unwrap();
        assert!(key_pos < rev_pos && rev_pos < title_pos);
        assert!(html.contains("<th>extra</th>")); // union of keys
        assert_eq!(db_rows_table(&[]), "<p class=\"muted\">No rows.</p>");
    }

    #[test]
    fn unwrap_bare_state_shape_returns_whole_hash() {
        let json = serde_json::json!({ "count": 7, "name": "alice" });
        let (state, tick, stream) = unwrap_handler_return(json.clone());
        assert_eq!(state, Some(json));
        assert_eq!(tick, None);
        assert!(stream.is_none());
    }

    /// SEC-044: an appending proxy chain reaches the app as
    /// "real, attacker" — the trusted-proxy contract is that the
    /// outermost (leftmost) proxy's value is the canonical one.
    #[test]
    fn first_forwarded_token_returns_leftmost_trimmed() {
        assert_eq!(
            first_forwarded_token("real.example.com"),
            "real.example.com"
        );
        assert_eq!(
            first_forwarded_token("real.example.com, attacker.test"),
            "real.example.com"
        );
        assert_eq!(
            first_forwarded_token("  real.example.com  ,evil"),
            "real.example.com"
        );
        assert_eq!(first_forwarded_token("https"), "https");
        assert_eq!(first_forwarded_token("https, http"), "https");
        assert_eq!(first_forwarded_token(""), "");
        assert_eq!(first_forwarded_token(",real"), "");
    }

    #[test]
    fn unwrap_wrapped_shape_extracts_state_and_tick() {
        let json = serde_json::json!({
            "state": { "count": 3 },
            "tick_interval": 50,
        });
        let (state, tick, _) = unwrap_handler_return(json);
        assert_eq!(state, Some(serde_json::json!({ "count": 3 })));
        assert_eq!(tick, Some(50));
    }

    #[test]
    fn unwrap_wrapped_shape_without_tick_interval_returns_none() {
        let json = serde_json::json!({ "state": { "x": 1 } });
        let (state, tick, _) = unwrap_handler_return(json);
        assert_eq!(state, Some(serde_json::json!({ "x": 1 })));
        assert_eq!(tick, None);
    }

    #[test]
    fn unwrap_treats_non_object_state_key_as_bare_shape() {
        // A user setting `"state": 42` doesn't look like the wrapped form — we
        // treat the whole hash as the new state to avoid silently dropping it.
        let json = serde_json::json!({ "state": 42, "tick_interval": 50 });
        let (state, tick, _) = unwrap_handler_return(json.clone());
        assert_eq!(state, Some(json));
        assert_eq!(tick, None);
    }

    #[test]
    fn unwrap_wrapped_shape_with_zero_tick_interval_returns_zero() {
        let json = serde_json::json!({
            "state": { "x": 1 },
            "tick_interval": 0,
        });
        let (_, tick, _) = unwrap_handler_return(json);
        assert_eq!(tick, Some(0));
    }

    #[test]
    fn unwrap_stream_only_keeps_state_and_extracts_ops() {
        // A stream-only emission (no `state` key) must not wipe the state.
        let json = serde_json::json!({
            "stream": { "container": "posts", "ops": [
                { "op": "append", "id": "post-7", "html": "<li>hi</li>" }
            ] }
        });
        let (state, tick, stream) = unwrap_handler_return(json);
        assert_eq!(state, None); // keep current state
        assert_eq!(tick, None);
        assert!(stream.is_some());
    }

    #[test]
    fn build_stream_ops_parses_all_op_kinds() {
        use crate::live::view::StreamOp;
        let stream = serde_json::json!({
            "container": "posts",
            "ops": [
                { "op": "append",  "id": "post-7", "html": "<li>a</li>" },
                { "op": "prepend", "id": "post-8", "html": "<li>b</li>" },
                { "op": "insert",  "id": "post-9", "html": "<li>c</li>", "before": "post-7" },
                { "op": "remove",  "id": "post-1" },
                { "op": "reset" },
                { "op": "bogus",   "id": "x" }
            ]
        });
        let ops = build_stream_ops(&stream);
        assert_eq!(ops.len(), 5); // bogus op skipped
        assert!(
            matches!(&ops[0], StreamOp::Append { container, id, .. } if container == "posts" && id == "post-7")
        );
        assert!(matches!(&ops[1], StreamOp::Prepend { .. }));
        assert!(matches!(&ops[2], StreamOp::Insert { before: Some(b), .. } if b == "post-7"));
        assert!(matches!(&ops[3], StreamOp::Remove { id } if id == "post-1"));
        assert!(matches!(&ops[4], StreamOp::Reset { container } if container == "posts"));
    }

    #[test]
    fn dev_repl_auth_accepts_loopback_with_token() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert("x-soli-dev-token", dev_repl_auth_token().parse().unwrap());
        let peer_addr: SocketAddr = "127.0.0.1:5011".parse().unwrap();

        assert!(is_authorized_dev_repl_request(&headers, peer_addr));
    }

    #[test]
    fn dev_repl_auth_rejects_missing_token() {
        let headers = hyper::HeaderMap::new();
        let peer_addr: SocketAddr = "127.0.0.1:5011".parse().unwrap();

        assert!(!is_authorized_dev_repl_request(&headers, peer_addr));
    }

    #[test]
    fn dev_repl_auth_rejects_non_loopback_peer() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("SOLI_DEV_REPL_ALLOW_REMOTE");
        let mut headers = hyper::HeaderMap::new();
        headers.insert("x-soli-dev-token", dev_repl_auth_token().parse().unwrap());
        let peer_addr: SocketAddr = "192.0.2.10:5011".parse().unwrap();

        assert!(!is_authorized_dev_repl_request(&headers, peer_addr));
    }

    #[test]
    fn dev_repl_auth_rejects_rfc1918_peer_without_allow_remote() {
        // SEC-051: a LAN peer (e.g. a phone on the same Wi-Fi, but also any
        // co-resident host on a shared office/café network or container LAN)
        // must NOT be able to drive the REPL just because it can scrape the
        // token — that is a LAN-wide RCE whenever the dev server binds a
        // non-loopback address. Only loopback is trusted by default; LAN access
        // is an explicit SOLI_DEV_REPL_ALLOW_REMOTE (+ SECRET) opt-in.
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("SOLI_DEV_REPL_ALLOW_REMOTE");
        let mut headers = hyper::HeaderMap::new();
        headers.insert("x-soli-dev-token", dev_repl_auth_token().parse().unwrap());

        for ip in ["192.168.1.30", "10.0.0.5", "172.16.0.1"] {
            let peer_addr: SocketAddr = format!("{}:5011", ip).parse().unwrap();
            assert!(
                !is_authorized_dev_repl_request(&headers, peer_addr),
                "expected {} (RFC 1918) to be REJECTED without ALLOW_REMOTE",
                ip
            );
        }
    }

    #[test]
    fn is_trusted_dev_peer_classifies_known_ranges() {
        use std::net::IpAddr;
        // SEC-051: only loopback is trusted. Private/LAN ranges are NOT
        // trusted — a dev server on a shared network must not hand the REPL
        // execution credential to every peer (LAN-RCE). LAN access requires
        // the explicit SOLI_DEV_REPL_ALLOW_REMOTE opt-in.
        let trusted: &[&str] = &["127.0.0.1", "127.0.0.2", "::1", "::ffff:127.0.0.1"];
        let untrusted: &[&str] = &[
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "192.168.255.255",
            "::ffff:192.168.1.1",
            "8.8.8.8",
            "192.0.2.10",
            "172.32.0.1",
            "2001:4860:4860::8888",
        ];
        for ip in trusted {
            let parsed: IpAddr = ip.parse().unwrap();
            assert!(is_trusted_dev_peer(parsed), "{} should be trusted", ip);
            assert!(
                is_trusted_dev_peer_str(ip),
                "{} (str) should be trusted",
                ip
            );
        }
        for ip in untrusted {
            let parsed: IpAddr = ip.parse().unwrap();
            assert!(!is_trusted_dev_peer(parsed), "{} should NOT be trusted", ip);
            assert!(
                !is_trusted_dev_peer_str(ip),
                "{} (str) should NOT be trusted",
                ip
            );
        }
        assert!(
            !is_trusted_dev_peer_str("not-an-ip"),
            "malformed peer_ip is untrusted"
        );
    }

    #[test]
    fn dev_repl_auth_accepts_remote_peer_when_explicitly_enabled() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("SOLI_DEV_REPL_ALLOW_REMOTE", "1");
        let mut headers = hyper::HeaderMap::new();
        headers.insert("x-soli-dev-token", dev_repl_auth_token().parse().unwrap());
        let peer_addr: SocketAddr = "192.0.2.10:5011".parse().unwrap();

        assert!(is_authorized_dev_repl_request(&headers, peer_addr));
        std::env::remove_var("SOLI_DEV_REPL_ALLOW_REMOTE");
    }

    #[test]
    fn dev_repl_secret_set_reflects_env_state() {
        // SEC-051: dev_repl_secret_set is the gate the startup check
        // consults — exercise the env reads.
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("SOLI_DEV_REPL_SECRET");
        assert!(!dev_repl_secret_set(), "unset → false");

        std::env::set_var("SOLI_DEV_REPL_SECRET", "");
        assert!(
            !dev_repl_secret_set(),
            "empty → false (won't satisfy startup gate)"
        );

        std::env::set_var("SOLI_DEV_REPL_SECRET", "s3cret");
        assert!(dev_repl_secret_set(), "non-empty → true");

        std::env::remove_var("SOLI_DEV_REPL_SECRET");
    }

    #[test]
    fn serve_folder_refuses_dev_remote_without_secret() {
        // SEC-051: --dev + ALLOW_REMOTE without SOLI_DEV_REPL_SECRET must
        // fail at startup so the auto-generated token never lands in HTML.
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("SOLI_DEV_REPL_ALLOW_REMOTE", "1");
        std::env::remove_var("SOLI_DEV_REPL_SECRET");

        // Use a tempdir without an `app/controllers/` so the folder
        // validation would also fail — but the SEC-051 check runs first.
        let dir = tempfile::tempdir().unwrap();
        let err = serve_folder_with_options_and_workers(dir.path(), 0, true, 1).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("SEC-051"),
            "expected SEC-051 startup refusal, got: {msg}"
        );

        std::env::remove_var("SOLI_DEV_REPL_ALLOW_REMOTE");
    }

    #[test]
    fn default_websocket_config_caps_message_and_frame_size() {
        // SEC-047: tungstenite's defaults (64 MiB / 16 MiB) are too
        // generous; lock the caps in so a future refactor can't quietly
        // restore them.
        let cfg = default_websocket_config();
        assert_eq!(cfg.max_message_size, Some(1 << 20));
        assert_eq!(cfg.max_frame_size, Some(1 << 20));
    }

    #[test]
    fn websocket_origin_allows_missing_origin_without_cookie() {
        // SEC-046: cookie-less upgrade has no credentials to ride, so a
        // missing Origin is still acceptable (curl from the dev box, native
        // clients hitting an unauthenticated endpoint).
        let mut headers = hyper::HeaderMap::new();
        headers.insert(header::HOST, "app.test:5011".parse().unwrap());

        assert!(websocket_origin_allowed(&headers));
    }

    #[test]
    fn websocket_origin_rejects_missing_origin_with_cookie() {
        // SEC-046: the CSWSH threat is a non-browser pivot forging a
        // cookie-bearing handshake. When a Cookie is present, Origin must
        // be too.
        let mut headers = hyper::HeaderMap::new();
        headers.insert(header::HOST, "app.test:5011".parse().unwrap());
        headers.insert(header::COOKIE, "session=abc".parse().unwrap());

        assert!(!websocket_origin_allowed(&headers));
    }

    #[test]
    fn websocket_origin_allows_same_origin_host() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(header::HOST, "app.test:5011".parse().unwrap());
        headers.insert(header::ORIGIN, "http://app.test:5011".parse().unwrap());

        assert!(websocket_origin_allowed(&headers));
    }

    #[test]
    fn websocket_origin_rejects_cross_origin_host() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(header::HOST, "app.test:5011".parse().unwrap());
        headers.insert(header::ORIGIN, "http://evil.test:5011".parse().unwrap());

        assert!(!websocket_origin_allowed(&headers));
    }

    #[test]
    fn websocket_origin_uses_forwarded_host_for_proxied_apps() {
        // SEC-032: X-Forwarded-Host is only honored when the operator
        // has opted into trust-proxy. Flip the flag for the duration of
        // this test and restore it on the way out.
        let _g = csrf_lock();
        let prev_trust = crate::interpreter::builtins::trust_proxy::is_trust_proxy_enabled();
        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let mut headers = hyper::HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:5011".parse().unwrap());
        headers.insert("x-forwarded-host", "app.test".parse().unwrap());
        headers.insert(header::ORIGIN, "https://app.test".parse().unwrap());

        assert!(websocket_origin_allowed(&headers));

        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED
            .store(prev_trust, std::sync::atomic::Ordering::Relaxed);
    }

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

    /// Regression: the containment check must compare path components, not
    /// stringified bytes. `…/public-evil/x` is a byte-level prefix match
    /// against `…/public`, so the previous `&str::starts_with` check would
    /// pass it through. The fix uses `Path::starts_with`, which is segment
    /// aware. Exercised here via a symlink inside `public/` that resolves
    /// out to a sibling whose name starts with `public`.
    #[cfg(unix)]
    #[test]
    fn test_resolve_static_file_blocks_sibling_prefix_via_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let evil = dir.path().join("public-evil");
        fs::create_dir(&public).unwrap();
        fs::create_dir(&evil).unwrap();
        fs::write(evil.join("secret.txt"), "leaked").unwrap();

        // `public/escape` -> `../public-evil`
        std::os::unix::fs::symlink(&evil, public.join("escape")).unwrap();

        // `escape/secret.txt` has no `..` in the URL, so the early-out
        // doesn't catch it; the canonical path lives in `public-evil`,
        // which used to satisfy `starts_with("…/public")` byte-wise.
        let result = resolve_static_file("/escape/secret.txt", &public);
        assert_eq!(result, Err(()));
    }

    /// Regression: the same containment-check bug, without symlinks — a
    /// canonicalized path under a sibling directory whose name is a byte
    /// prefix of `public_dir` must not pass the check. This is harder to
    /// trigger from a clean URL (the early `..` reject covers the obvious
    /// vector), but we still want explicit coverage that the segment-aware
    /// check is what's running.
    #[test]
    fn test_resolve_static_file_path_starts_with_is_segment_aware() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let evil = dir.path().join("public-evil");
        fs::create_dir(&public).unwrap();
        fs::create_dir(&evil).unwrap();
        let secret = evil.join("secret.txt");
        fs::write(&secret, "leaked").unwrap();

        // Sanity: with the old byte-level check, `…/public-evil/secret.txt`
        // would test as a prefix-match against `…/public`. With Path-aware
        // semantics it does not.
        let canon_secret = fs::canonicalize(&secret).unwrap();
        let canon_public = fs::canonicalize(&public).unwrap();
        assert!(!canon_secret.starts_with(&canon_public));
        assert!(canon_secret
            .to_string_lossy()
            .starts_with(&*canon_public.to_string_lossy()));
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

        let result = call_class_method(
            &mut interpreter,
            None,
            &class_rc,
            &instance,
            "action",
            &request_hash,
        )
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

    // SEC-014 — `check_csrf_origin` regression coverage.

    fn make_headers(pairs: &[(&str, &str)]) -> hyper::HeaderMap {
        let mut h = hyper::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                hyper::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                hyper::header::HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    // SOLI_DISABLE_CSRF is process-global; serialise every test in this
    // module that depends on its value (i.e. all of them) behind one mutex
    // so a parallel kill-switch test can't leak its `set_var` into a peer.
    fn csrf_lock() -> std::sync::MutexGuard<'static, ()> {
        let g = ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("SOLI_DISABLE_CSRF");
        g
    }

    #[test]
    fn csrf_allows_safe_methods() {
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com"), ("origin", "https://evil.test")]);
        assert!(check_csrf_origin(&h, "GET", "/posts").is_ok());
        assert!(check_csrf_origin(&h, "HEAD", "/posts").is_ok());
        assert!(check_csrf_origin(&h, "OPTIONS", "/posts").is_ok());
    }

    #[test]
    fn csrf_allows_under_underscore_paths() {
        // /_jobs/run/:name etc carry their own HMAC auth; they're
        // machine-to-machine so we don't expect Origin/Referer.
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com")]);
        assert!(check_csrf_origin(&h, "POST", "/_jobs/run/EmailJob").is_ok());
        assert!(check_csrf_origin(&h, "POST", "/__coverage__").is_ok());
    }

    #[test]
    fn csrf_allows_when_origin_matches_host() {
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com"), ("origin", "https://example.com")]);
        assert!(check_csrf_origin(&h, "POST", "/users").is_ok());
    }

    #[test]
    fn csrf_rejects_cross_origin_post() {
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com"), ("origin", "https://evil.test")]);
        let err = check_csrf_origin(&h, "POST", "/users").unwrap_err();
        assert!(err.contains("does not match"), "{}", err);
    }

    #[test]
    fn csrf_rejects_null_origin() {
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com"), ("origin", "null")]);
        let err = check_csrf_origin(&h, "POST", "/users").unwrap_err();
        assert!(err.contains("'null'"), "{}", err);
    }

    #[test]
    fn csrf_falls_back_to_referer_when_origin_missing() {
        let _g = csrf_lock();
        let h = make_headers(&[
            ("host", "example.com"),
            ("referer", "https://example.com/page"),
        ]);
        assert!(check_csrf_origin(&h, "POST", "/users").is_ok());

        let h = make_headers(&[
            ("host", "example.com"),
            ("referer", "https://evil.test/page"),
        ]);
        let err = check_csrf_origin(&h, "POST", "/users").unwrap_err();
        assert!(err.contains("Referer"), "{}", err);
    }

    #[test]
    fn csrf_allows_when_neither_origin_nor_referer_present_without_cookie() {
        // SEC-078: Non-browser clients (curl, mobile API client) typically
        // don't send Origin/Referer and don't ride a session cookie either,
        // so they're not the CSRF threat. Allow.
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com")]);
        assert!(check_csrf_origin(&h, "POST", "/users").is_ok());
    }

    #[test]
    fn csrf_rejects_when_neither_origin_nor_referer_present_with_cookie() {
        // SEC-078: a cookie-bearing POST without either header has no
        // proof of same-site provenance — this is the stripped-UA / proxy
        // / Origin-less form-post bypass surface the previous "allow" path
        // left open. Reject by default.
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com"), ("cookie", "session_id=x")]);
        let err = check_csrf_origin(&h, "POST", "/users").unwrap_err();
        assert!(err.contains("Origin"), "{}", err);
        assert!(err.contains("Referer"), "{}", err);
    }

    #[test]
    fn csrf_skip_pattern_still_allows_cookie_post_without_headers() {
        // SEC-078: route-level skip_csrf is the documented escape hatch for
        // cookie-bearing endpoints that legitimately can't rely on Origin
        // (rare, but supportable).
        let _g = csrf_lock();
        clear_csrf_skip_patterns();
        register_csrf_skip_pattern("/api/legacy/*".to_string());
        let h = make_headers(&[("host", "example.com"), ("cookie", "session_id=x")]);
        assert!(check_csrf_origin(&h, "POST", "/api/legacy/upload").is_ok());
        clear_csrf_skip_patterns();
    }

    #[test]
    fn csrf_kill_switch_disables_check() {
        let _g = csrf_lock();
        std::env::set_var("SOLI_DISABLE_CSRF", "true");
        let h = make_headers(&[("host", "example.com"), ("origin", "https://evil.test")]);
        let result = check_csrf_origin(&h, "POST", "/users");
        std::env::remove_var("SOLI_DISABLE_CSRF");
        assert!(result.is_ok(), "kill switch should bypass: {:?}", result);
    }

    #[test]
    fn csrf_skip_pattern_exact_path() {
        let _g = csrf_lock();
        clear_csrf_skip_patterns();
        register_csrf_skip_pattern("/webhooks/stripe".to_string());
        let h = make_headers(&[("host", "example.com"), ("origin", "https://evil.test")]);
        // Exact-pattern path: skipped.
        assert!(check_csrf_origin(&h, "POST", "/webhooks/stripe").is_ok());
        // Different path under /webhooks/: not skipped.
        assert!(check_csrf_origin(&h, "POST", "/webhooks/paypal").is_err());
        clear_csrf_skip_patterns();
    }

    // SEC-080 — `coverage_request_authorized` regression coverage.

    #[test]
    fn coverage_rejected_without_env_token() {
        // Test runner forgot to set SOLI_COVERAGE_TOKEN — endpoint must
        // refuse rather than fall back to "any caller wins".
        assert!(!coverage_request_authorized(None, "anything"));
        assert!(!coverage_request_authorized(Some(""), "anything"));
    }

    #[test]
    fn coverage_rejected_without_request_header() {
        assert!(!coverage_request_authorized(Some("secret-token"), ""));
    }

    #[test]
    fn coverage_rejected_on_token_mismatch() {
        assert!(!coverage_request_authorized(
            Some("secret-token"),
            "wrong-token"
        ));
    }

    #[test]
    fn coverage_accepted_on_token_match() {
        assert!(coverage_request_authorized(
            Some("secret-token"),
            "secret-token"
        ));
    }

    #[test]
    fn csrf_skip_pattern_wildcard_suffix() {
        let _g = csrf_lock();
        clear_csrf_skip_patterns();
        register_csrf_skip_pattern("/api/*".to_string());
        let h = make_headers(&[("host", "example.com"), ("origin", "https://evil.test")]);
        // Anything under /api/ is skipped.
        assert!(check_csrf_origin(&h, "POST", "/api/users").is_ok());
        assert!(check_csrf_origin(&h, "POST", "/api/v2/orders").is_ok());
        // A path that merely shares the prefix without the slash boundary
        // doesn't match — `/api/*` means "/api or /api/...".
        assert!(check_csrf_origin(&h, "POST", "/apifoo").is_err());
        clear_csrf_skip_patterns();
    }

    /// SEC-032: `websocket_request_authority` must only consult
    /// `X-Forwarded-Host` when the operator has opted into trust-proxy.
    /// Otherwise an attacker-controlled XFH header could spoof the
    /// authority that gets compared to `Origin`, bypassing CSWSH defense
    /// (and the same-helper-driven CSRF check).
    #[test]
    fn websocket_request_authority_gates_xfh_on_trust_proxy() {
        let _g = csrf_lock();
        let prev_trust = crate::interpreter::builtins::trust_proxy::is_trust_proxy_enabled();

        // Trust-proxy OFF: the real Host wins, the attacker's XFH is ignored.
        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let h = make_headers(&[("host", "example.com"), ("x-forwarded-host", "evil.test")]);
        assert_eq!(
            websocket_request_authority(&h),
            Some("example.com".to_string()),
            "trust_proxy OFF must ignore X-Forwarded-Host"
        );

        // The CSWSH attack scenario from the SEC-032 task md:
        //   Origin: http://evil.test  AND  X-Forwarded-Host: evil.test
        // With trust_proxy OFF the request authority resolves to the real
        // Host (`example.com`), so the Origin check fails and CSWSH is
        // blocked.
        let h = make_headers(&[
            ("host", "example.com"),
            ("x-forwarded-host", "evil.test"),
            ("origin", "http://evil.test"),
        ]);
        assert!(
            !websocket_origin_allowed(&h),
            "trust_proxy OFF must block spoofed X-Forwarded-Host CSWSH"
        );

        // Trust-proxy ON: XFH is honored, since the operator has stated
        // the deployment terminates that header at a trusted proxy hop.
        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let h = make_headers(&[
            ("host", "example.com"),
            ("x-forwarded-host", "app.example.com"),
        ]);
        assert_eq!(
            websocket_request_authority(&h),
            Some("app.example.com".to_string())
        );

        // Restore.
        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED
            .store(prev_trust, std::sync::atomic::Ordering::Relaxed);
    }

    // --- form method override + CSRF token verification -------------------

    fn make_request_data(
        headers: &[(&str, &str)],
        body: &str,
        multipart_form: Option<Vec<(String, String)>>,
    ) -> RequestData {
        // _rx is dropped: verify_csrf_token never sends a response.
        let (tx, _rx) = oneshot::channel();
        RequestData {
            method: Cow::Borrowed("POST"),
            path: "/posts".to_string(),
            query: Vec::new(),
            headers: make_headers(headers),
            body: body.to_string(),
            body_bytes: None,
            multipart_form,
            multipart_files: None,
            peer_ip: "127.0.0.1".to_string(),
            enqueued_at: None,
            replay: false,
            response_tx: tx,
        }
    }

    #[test]
    fn method_override_honors_form_verbs_only() {
        let ct = Some("application/x-www-form-urlencoded");
        assert_eq!(
            apply_form_method_override(Cow::Borrowed("POST"), "_method=DELETE&id=7", ct, None),
            "DELETE"
        );
        assert_eq!(
            apply_form_method_override(Cow::Borrowed("POST"), "_method=patch", ct, None),
            "PATCH"
        );
        // No downgrade to safe verbs, no arbitrary verbs.
        assert_eq!(
            apply_form_method_override(Cow::Borrowed("POST"), "_method=GET", ct, None),
            "POST"
        );
        assert_eq!(
            apply_form_method_override(Cow::Borrowed("POST"), "_method=TRACE", ct, None),
            "POST"
        );
        // Only POST is overridable, and only for form content types.
        assert_eq!(
            apply_form_method_override(Cow::Borrowed("GET"), "_method=DELETE", ct, None),
            "GET"
        );
        assert_eq!(
            apply_form_method_override(
                Cow::Borrowed("POST"),
                "{\"_method\":\"DELETE\"}",
                Some("application/json"),
                None
            ),
            "POST"
        );
        // Multipart reads the pre-parsed form map.
        let multipart = vec![("_method".to_string(), "put".to_string())];
        assert_eq!(
            apply_form_method_override(
                Cow::Borrowed("POST"),
                "",
                Some("multipart/form-data; boundary=x"),
                Some(&multipart)
            ),
            "PUT"
        );
    }

    #[test]
    fn csrf_token_verification_paths() {
        use crate::interpreter::builtins::session::{ensure_csrf_token, set_current_session_id};
        let _lock = ENV_TEST_LOCK.lock().unwrap();
        set_current_session_id(None);

        // Safe methods and token-less requests pass (Origin gate covers them).
        let data = make_request_data(&[], "", None);
        assert!(verify_csrf_token(&data, "GET", "/posts").is_ok());
        assert!(verify_csrf_token(&data, "POST", "/posts").is_ok());

        // A supplied token with no session token behind it is rejected.
        let data = make_request_data(&[("x-csrf-token", "forged")], "", None);
        assert!(verify_csrf_token(&data, "POST", "/posts").is_err());

        // Matching session token passes — header and form-body variants.
        let token = ensure_csrf_token();
        let data = make_request_data(&[("x-csrf-token", token.as_str())], "", None);
        assert!(verify_csrf_token(&data, "POST", "/posts").is_ok());
        let body = format!("_csrf_token={}&title=hi", token);
        let data = make_request_data(
            &[("content-type", "application/x-www-form-urlencoded")],
            &body,
            None,
        );
        assert!(verify_csrf_token(&data, "DELETE", "/posts/7").is_ok());

        // Wrong token is rejected even though a session token exists.
        let data = make_request_data(&[("x-csrf-token", "wrong")], "", None);
        assert!(verify_csrf_token(&data, "POST", "/posts").is_err());

        // `/_` paths are exempt.
        let data = make_request_data(&[("x-csrf-token", "wrong")], "", None);
        assert!(verify_csrf_token(&data, "POST", "/_jobs/run/x").is_ok());

        set_current_session_id(None);
    }

    #[test]
    fn csrf_strict_mode_requires_token_for_form_posts() {
        use crate::interpreter::builtins::session::set_current_session_id;
        let _lock = ENV_TEST_LOCK.lock().unwrap();
        set_current_session_id(None);
        std::env::set_var("SOLI_CSRF_TOKENS", "require");

        // Form post without a token is rejected...
        let data = make_request_data(
            &[("content-type", "application/x-www-form-urlencoded")],
            "title=hi",
            None,
        );
        assert!(verify_csrf_token(&data, "POST", "/posts").is_err());
        // ...but JSON/API traffic is never token-gated.
        let data = make_request_data(&[("content-type", "application/json")], "{}", None);
        assert!(verify_csrf_token(&data, "POST", "/posts").is_ok());

        std::env::remove_var("SOLI_CSRF_TOKENS");
    }
}
