//! MVC framework with convention-based routing and hot reload.
//!
//! This module implements a Rails-like MVC framework for Soli applications:
//! - Convention-based routing from controller filenames and function names
//! - Hot reload of changed files without server restart
//! - Automatic route derivation
//! - Middleware support for request interception

mod accept;
mod asset_cache;
mod builtin_endpoints;
pub mod camera;
pub mod cors;
mod coverage;
mod csrf;
pub mod dev_bar;
mod dev_catalog;
mod dev_inbox;
mod dev_jobs;
mod dev_routes;
pub mod dev_store;
mod error_response;
mod file_watcher;
pub mod files;
mod finalize;
mod framework_assets;
mod hot_reload;
pub mod live_reload;
mod live_reload_ws; // WebSocket-based live reload
pub(crate) mod middleware;
pub mod middleware_log;
pub mod native;
pub mod nav;
pub mod openapi;
mod origin;
pub mod otel;
pub mod phase_log;
mod pipeline;
pub mod prefetch;
mod probes;
pub mod prod_log;
mod request_input;
mod request_scope;
pub mod route_listing;
pub mod route_log;
mod route_match;
mod router;
pub mod sensors;
pub mod server_constants;
pub mod shutdown;
pub mod span_log;
mod static_files;
pub mod template_warnings;
pub mod tenant;
mod upgrade;
pub(crate) mod uploads_prelude;
pub mod vhost;
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
pub mod job_worker;
mod json;
mod repl_session;
pub(crate) mod tailwind;
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
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::virtual_fs::VirtualFileSystem;

#[cfg(feature = "eui")]
pub mod eui;

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
/// Set by `soli serve --strict-port`: bind the requested port or fail, instead
/// of scanning upward for a free one.
///
/// Scanning is right for a human running `soli serve` twice; it is wrong under
/// a supervisor. soli-proxy allocates a specific port, passes it as `$PORT`,
/// and health-checks that port — so an app that quietly moved to `port + 1`
/// looks unhealthy, and after three such "crashes" the proxy quarantines it and
/// stops retrying. A port race then presents as a broken deployment.
static STRICT_PORT: OnceLock<bool> = OnceLock::new();

/// Require the exact requested port. See [`STRICT_PORT`].
pub fn request_strict_port() {
    let _ = STRICT_PORT.set(true);
}

fn strict_port_requested() -> bool {
    STRICT_PORT.get().copied().unwrap_or(false)
}

static BOOT_START: OnceLock<Instant> = OnceLock::new();
static BOOT_LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

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
use futures_util::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use http_body_util::Full;
use http_body_util::StreamBody;
use hyper::body::Incoming;
use hyper::{header, Request, Response, StatusCode};
use tokio::sync::{broadcast, oneshot};
use uuid::Uuid;

use self::origin::{first_forwarded_token, normalize_request_authority, origin_authority};

#[cfg(test)]
use self::csrf::clear_csrf_skip_patterns;
pub use self::csrf::register_csrf_skip_pattern;
use self::csrf::{apply_form_method_override, check_csrf_origin, verify_csrf_token};
#[cfg(test)]
use self::dev_catalog::mailer_view_names;

use crate::error::RuntimeError;
use crate::interpreter::builtins::server::{
    build_request_hash_with_parsed, extract_response, find_route, get_routes,
    parse_form_urlencoded_body, parse_json_body, parse_query_pairs, routes_to_worker_routes,
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
pub fn set_tokio_handle(handle: tokio::runtime::Handle) {
    TOKIO_HANDLE.with(|h| *h.borrow_mut() = Some(handle));
}

/// Process-wide LiveView event sender. Set once during server startup and
/// read by `handle_liveview_event` when it needs to spawn a per-instance
/// tick task that posts back into the worker queue.
/// One private LiveView event queue per realtime worker, for the sessions
/// that must land on the same thread every time.
///
/// `LV_EVENT_TX` is one queue every realtime worker drains, so a session's
/// frames run on whichever worker gets there first. Each worker is its own
/// interpreter: the objects an application keeps between renders — and the
/// EUI memo that keeps a rendered card by the identity of the object it came
/// from — live on one thread, so a session that moves between workers finds
/// them cold. An EUI session is pinned to one worker by hashing its id;
/// these are the queues, in realtime-worker order, and `lv_sender_for`
/// picks one.
/// Per application: these are *that* application's realtime worker queues, and
/// its workers hold its interpreters. A `OnceLock` here meant the second
/// application to boot silently kept the first one's queues, so its EUI
/// sessions would have been rendered by workers that had never loaded its code.
static PINNED_LV_TX: TenantCell<Arc<Vec<channel::Sender<LiveViewEventData>>>> = TenantCell::new();

/// The queue for one LiveView instance's events: its pinned worker's for an
/// EUI component, the shared queue for everything else.
pub(crate) fn lv_sender_for(
    liveview_id: &str,
    component: &str,
) -> Option<channel::Sender<LiveViewEventData>> {
    #[cfg(feature = "eui")]
    if eui::is_eui_component(component) {
        if let Some(queues) = PINNED_LV_TX.get().filter(|q| !q.is_empty()) {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            liveview_id.hash(&mut hasher);
            let index = (hasher.finish() % queues.len() as u64) as usize;
            return Some(queues[index].clone());
        }
    }
    let _ = (liveview_id, component);
    LV_EVENT_TX.get()
}

/// The shared LiveView queue, per application for the same reason as
/// [`PINNED_LV_TX`].
static LV_EVENT_TX: TenantCell<channel::Sender<LiveViewEventData>> = TenantCell::new();
use crate::interpreter::builtins::controller::controller::ControllerInfo;
use crate::interpreter::builtins::controller::CONTROLLER_REGISTRY;
use crate::interpreter::builtins::session::set_current_session_id;
use crate::interpreter::builtins::template::{clear_template_cache, init_templates};
use crate::interpreter::value::{HashKey, HashPairs, StrKey};
use crate::interpreter::{Interpreter, Value};
use crate::span::Span;

/// Uploaded file information
#[derive(Clone)]
pub struct UploadedFile {
    pub name: String,
    pub filename: String,
    pub content_type: String,
    /// `Bytes`, not `Vec<u8>`: the part is a refcounted slice of the request
    /// body, so cloning an `UploadedFile` (the LiveView upload path does)
    /// bumps a count instead of duplicating the upload.
    pub data: bytes::Bytes,
}

// Import REPL session store from the dedicated module

// Import worker pool structures
use tenant::TenantCell;
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
    /// Slice of the aggregate in-flight body budget, held for as long as this
    /// request owns its buffered body. Dropping `RequestData` returns it, on
    /// every path including a worker panic.
    ///
    /// Never read: it exists for its `Drop`. Unlike the `body_bytes` field this
    /// replaced — which was also never read and genuinely did nothing — removing
    /// this one would leak the budget on every request.
    #[allow(dead_code)]
    pub(crate) body_reservation: Option<crate::interpreter::builtins::body_limit::BodyReservation>,
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
    /// Set only in file mode (`soli serve` on a plain directory), to the path
    /// of a `.slv`/`.erb` template relative to the served root with its
    /// extension stripped. The worker renders it through the template engine
    /// instead of matching a route — there is no route table to match against.
    pub(crate) file_template: Option<String>,
    pub(crate) response_tx: oneshot::Sender<WorkerResponse>,
}

/// A response that must not have the dev live-reload script spliced into it.
///
/// Dev mode injects that script into every `text/html` body, which is right
/// for a page rendered from templates somebody is editing and wrong for one
/// compiled into the binary: there is nothing to reload, so the splice buys a
/// socket and fourteen kilobytes of script on a twenty-kilobyte page. The
/// header is stripped on the way out and never reaches the client.
pub(crate) const NO_INJECT_HEADER: &str = "x-soli-no-inject";

/// Borrow a header value from the wire `HeaderMap` by its lowercase name.
/// Non-UTF-8 header values read as absent (same as the previous
/// `HashMap<String, String>` extraction, which skipped them).
#[inline]
pub(crate) fn header_str<'a>(headers: &'a hyper::header::HeaderMap, name: &str) -> Option<&'a str> {
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
pub(crate) fn full(body: Bytes) -> ResponseBody {
    Full::<Bytes>::new(body)
        .map_err(|never| match never {})
        .boxed()
}

/// Box a `Response<Full<Bytes>>` (handlers that still build `Full`, e.g. the
/// live-reload handlers) into the unified boxed body type.
/// A live SSE response subscribed to `topic`, for the native bridge stream.
///
/// Deliberately not routed through a worker: the connection lives as an async
/// `StreamBody` fed by a channel, so thousands of idle subscribers cost async
/// tasks rather than worker threads — the same contract `sse_subscribe` gets.
fn native_stream_response(topic: &str) -> Response<ResponseBody> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    // A first comment frame flushes headers, so the client's `onopen` fires
    // immediately instead of when the first notification happens to arrive.
    let _ = tx.try_send(b": connected\n\n".to_vec());
    crate::interpreter::builtins::streaming::register_subscriber(topic, tx);

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|chunk| Ok::<_, std::io::Error>(hyper::body::Frame::data(Bytes::from(chunk))));

    Response::builder()
        .status(200)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        // Nginx buffers proxied responses by default, which holds events until
        // the buffer fills — indistinguishable from a broken bridge.
        .header("X-Accel-Buffering", "no")
        .header("Server", "soliMVC")
        .body(BodyExt::boxed(StreamBody::new(stream)))
        .unwrap_or_else(|_| {
            box_full(
                Response::builder()
                    .status(500)
                    .body(Full::new(Bytes::from_static(b"stream init error")))
                    .unwrap(),
            )
        })
}

pub(crate) fn box_full(resp: Response<Full<Bytes>>) -> Response<ResponseBody> {
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
/// an error state. Prevents the whole-process crash that a bare
/// `.body(..).unwrap()` would cause on a poisoned builder (invalid header
/// names/values). File-mode and other `src/serve/` helpers must use this
/// instead of unwrapping `.body()`.
pub(crate) fn finish_response(
    builder: hyper::http::response::Builder,
    body: Bytes,
) -> Response<ResponseBody> {
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
/// Respects `SOLI_WORKERS` / `APP_ENV=production` defaults (see
/// [`server_constants::resolve_http_workers_from_env`]).
/// Operators can pin workers low (e.g. 2) to keep baseline RSS from duplicated
/// interpreter state + tokio runtimes under control.
pub fn serve_folder(folder: &Path, port: u16) -> Result<(), RuntimeError> {
    let num_workers = server_constants::resolve_http_workers_from_env();
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

/// Called once, with the port the server actually bound, before it starts
/// accepting. See [`serve_folder_with_options_and_hooks`].
pub type BoundPortHook = Box<dyn FnOnce(u16) + Send>;

/// Serve an MVC application from a folder with configurable options and worker count.
pub fn serve_folder_with_options_and_workers(
    folder: &Path,
    port: u16,
    dev_mode: bool,
    workers: usize,
) -> Result<(), RuntimeError> {
    serve_folder_with_options_and_hooks(folder, port, dev_mode, workers, None)
}

/// As [`serve_folder_with_options_and_workers`], but invokes `on_bound_port`
/// with the port the server actually bound.
///
/// Serving blocks forever (it joins the worker threads), so an embedding caller
/// has no other moment to learn the port — and with `port = 0` it cannot know it
/// in advance, because the kernel assigns it. The desktop shell uses this to
/// open a browser at the right URL.
///
/// The hook runs on the serving thread just before the ready banner, so it must
/// not block: spawn if you need to do real work.
pub fn serve_folder_with_options_and_hooks(
    folder: &Path,
    port: u16,
    dev_mode: bool,
    workers: usize,
    on_bound_port: Option<BoundPortHook>,
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
    if dev_mode && dev_routes::dev_repl_allows_remote() && !dev_routes::dev_repl_secret_set() {
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

    // A folder with no `app/controllers/` and no `config/routes.sl` is not an
    // app. Rather than refusing to start, serve it as a plain directory:
    // files, rendered Markdown, generated indexes. A bundle is always an app.
    // `--app` pins the MVC path (and keeps its error); `--static` pins this
    // one.
    if GLOBAL_VFS.get().is_none() && files::resolve_mode(folder) == files::ServeMode::Files {
        return serve_folder_as_files(folder, port, dev_mode, workers, on_bound_port);
    }

    // Load `.env` (and `.env.{APP_ENV}` if set) before any builtin reads
    // SOLIDB_* / SOLI_* env vars. Was dropped as collateral damage in
    // a17f300; without it `init_db_config` + `init_jwt_token` below run
    // with no credentials, every DB request goes out unauthenticated,
    // and SolidB 401s.
    load_env_files(folder);
    boot_trace("env loaded");

    if let Err(message) = server_constants::check_production_boot(dev_mode) {
        return Err(RuntimeError::General {
            message,
            span: Span::default(),
        });
    }

    // Multi-DB: load config/database.toml when present (else env → primary).
    if let Err(e) = crate::db::init_from_app_path(folder) {
        return Err(RuntimeError::General {
            message: e.message(),
            span: Span::default(),
        });
    }
    boot_trace("db connections");

    // Validate SOLI_DB_ADAPTER / DATABASE_URL early (missing URL, unknown
    // adapter, SQL pool failure) so boot fails with a clear message instead
    // of a cryptic mid-request SoliDB or SQL error.
    if let Err(e) = crate::db::ensure_runtime_ready() {
        return Err(RuntimeError::General {
            message: e.message(),
            span: Span::default(),
        });
    }

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
    crate::serve::tenant::set_app_root(folder);

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
        coverage::register_app_source_lines(&mut tracker, folder);
        set_global_coverage_tracker(std::sync::Arc::new(std::sync::Mutex::new(tracker)));
    }

    // Create interpreter. Use the serve constructor (no test-only builtins):
    // this boot interpreter only populates the shared registries before the
    // worker pool starts and is reclaimed right after (see below), so the
    // ~9 test DSL modules would be pure waste here.
    let mut interpreter = Interpreter::new_for_serve();
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

    // Column-aware models (`table "…"`) map to schemas Soli does not own, so
    // introspect and validate them now: a missing table, a composite primary
    // key, or a conflicting class-body declaration must fail here rather than
    // on the first request that touches the model.
    {
        let problems = crate::interpreter::builtins::model::column_mode::validate_declared_models();
        if !problems.is_empty() {
            return Err(RuntimeError::General {
                message: format!("column-aware models:\n  - {}", problems.join("\n  - ")),
                span: Span::default(),
            });
        }
        boot_trace("column-aware models validated");
    }

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
    // The locale a request starts from, and the one a lookup falls back to.
    // Process-wide, so it is set here rather than on the boot thread's
    // thread-local — that thread is reclaimed and never serves a request.
    if let Ok(default) = std::env::var("SOLI_DEFAULT_LOCALE") {
        if !default.trim().is_empty() {
            crate::interpreter::builtins::i18n::helpers::set_default_locale(default.trim());
        }
    }
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

    // Reclaim the boot interpreter. Its only job was to populate the shared,
    // process-global registries (routes, controller/model metadata, the
    // template cache) that the router and worker pool read — it never serves a
    // request, yet this thread parks forever in the worker-pool join below, so
    // whatever it still holds (all builtins + the whole parsed app AST + the
    // 9 i18n locale tables) would be leaked for the process lifetime. Each
    // worker rebuilds its own interpreter, view helpers and model classes, and
    // the worker route snapshot is taken from *this* thread's `ROUTES`
    // thread-local (kept), so the boot thread's other holdings are dead weight.
    {
        // Free the largest boot-thread thread-local holdings first (the app's
        // view helpers, which include the big i18n locale tables, plus the
        // i18n table cache and the model classes).
        crate::interpreter::builtins::template::clear_view_helpers();
        crate::interpreter::builtins::i18n::clear_table_cache();
        crate::interpreter::builtins::model::clear_model_classes();
        // Break the global-env <-> top-level-`def` closure cycle (each function
        // captures the global env as its closure while living in it), otherwise
        // the drop below is a no-op: `reset_for_call` clears the root scope's
        // bindings, releasing every `Value::Function`/`Value::Class` back-edge.
        interpreter.environment.borrow_mut().reset_for_call();
        drop(interpreter);
        boot_trace("boot interpreter reclaimed");
    }

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
        on_bound_port,
    )
}

/// Serve a plain directory: static files, Markdown pages, `.slv`/`.erb`
/// templates and a generated index per folder.
///
/// Deliberately skips everything the MVC boot does before this point —
/// `.env` loading, SoliDB configuration, controllers, models, routes. Serving
/// a directory the operator happened to `cd` into must not read their secrets
/// or open a database connection. `worker_loop` makes the same cut on its side
/// by checking [`files::files_root`].
fn serve_folder_as_files(
    folder: &Path,
    port: u16,
    dev_mode: bool,
    workers: usize,
    on_bound_port: Option<BoundPortHook>,
) -> Result<(), RuntimeError> {
    println!("Serving files from {}", folder.display());
    for root in files::assets_roots() {
        println!("Extra assets root: {}", root.display());
    }

    files::set_files_root(folder.to_path_buf());

    // Templates in the folder are code. Jail the File/Image builtins to the
    // served directory so one cannot read or write outside it.
    crate::interpreter::builtins::file::set_file_jail(folder.to_path_buf());
    crate::interpreter::builtins::image::set_image_jail(folder.to_path_buf());

    // The folder itself is the views root, so `about.html.slv` is `/about`.
    let views_dir = folder.to_path_buf();

    // The app directories are passed only to satisfy the shared signature;
    // none of them exist in a plain directory and `worker_loop` skips the
    // loading pass entirely in file mode.
    let app_dir = folder.join("app");

    run_hyper_server_worker_pool(
        folder,
        port,
        app_dir.join("controllers"),
        app_dir.join("models"),
        app_dir.join("middleware"),
        app_dir.join("helpers"),
        folder.join("public"),
        FileTracker::new(),
        dev_mode,
        workers,
        views_dir,
        folder.join("config").join("routes.sl"),
        app_dir.join("jobs"),
        on_bound_port,
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
    on_bound_port: Option<BoundPortHook>,
) -> Result<(), RuntimeError> {
    let reload_tx = if dev_mode {
        let (tx, _) = broadcast::channel::<()>(16);
        Some(tx)
    } else {
        None
    };

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
    LV_EVENT_TX.set_once(lv_event_tx.clone());
    // crossbeam Sender is cheap to clone - no need for Arc<Mutex<Option<>>>
    // Shutdown state lives in `serve::shutdown` as process-global atomics rather
    // than an Arc threaded through here: the readiness probe is answered deep
    // inside `handle_hyper_request`, far from this scope.

    // Single shared queue drained by all workers: any free worker pulls the
    // next request, so a request is never stranded behind a busy worker.
    let worker_queues = Arc::new(WorkerQueues::new(num_workers, capacity_per_worker));

    // Channel to pass actual bound port from tokio thread to main thread
    let (bound_port_tx, bound_port_rx) = std::sync::mpsc::channel::<u16>();

    // Build prod-mode in-memory snapshot of CSS/JS assets so a mid-deploy file
    // swap on disk doesn't desync against still-cached HTML in browsers.
    // File mode never reads from `public/` — the whole served folder is the
    // static root and `files::handle` owns it — so skip the snapshot rather
    // than slurping a `public/` that happens to sit in the served directory
    // and announcing a cache nothing will ever hit.
    let asset_cache = if files::files_root().is_some() {
        asset_cache::empty()
    } else {
        asset_cache::build(&public_dir, dev_mode)
    };

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
        // File mode defaults to loopback: `soli serve` on a directory the
        // operator happened to `cd` into should not publish it to the LAN
        // without them saying so. `SOLI_HOST=0.0.0.0` still opts in.
        _ if files::files_root().is_some() => std::net::IpAddr::from([127, 0, 0, 1]),
        _ => std::net::IpAddr::from([0, 0, 0, 0]),
    };

    accept::spawn(accept::Server {
        bind_host,
        port,
        tokio_worker_threads,
        runtime: TenantRuntime {
            tenant: tenant::TenantId::PRIMARY,
            request_tx: worker_queues.get_sender(),
            reload_tx: reload_tx.clone(),
            // Arc so a clone per connection and per request is a refcount
            // bump rather than a path copy.
            public_dir: Arc::new(public_dir.clone()),
            asset_cache,
            ws_event_tx: ws_event_tx.clone(),
            lv_event_tx: lv_event_tx.clone(),
            dev_mode,
        },
        runtime_handle_tx,
        bound_port_tx,
    });

    // Hot reload version counters (shared between file watcher and workers)
    let hot_reload_versions = Arc::new(HotReloadVersions::new());
    let hot_reload_versions_for_watcher = hot_reload_versions.clone();

    // Code-graph auto-reindex (dev): keep the SolidB code graph fresh as source
    // files change. On by default when an embedding key is configured (semantic
    // search is the whole point of the graph); `SOLI_GRAPH_WATCH=1`/`0` forces
    // it on/off regardless. The watcher signals a background reindex thread.
    let graph_watch = dev_mode
        && match std::env::var("SOLI_GRAPH_WATCH") {
            Ok(v) => v == "1" || v.eq_ignore_ascii_case("true"),
            Err(_) => std::env::var("SOLI_EMBEDDING_API_KEY").is_ok(),
        };
    let (graph_reindex_tx, graph_reindex_rx): (
        Option<std::sync::mpsc::Sender<()>>,
        Option<std::sync::mpsc::Receiver<()>>,
    ) = if graph_watch {
        let (tx, rx) = std::sync::mpsc::channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    // Spawn file watcher thread for hot reload (only in dev mode)
    // Spawn the dev-mode file watcher: it notices a source change and tells the
    // workers (and the browser) to reload. See `file_watcher`.
    if dev_mode {
        file_watcher::spawn(
            file_watcher::WatchPaths::new(
                folder,
                &controllers_dir,
                &views_dir,
                &middleware_dir,
                &helpers_dir,
                &models_dir,
                &jobs_dir,
                &public_dir,
                &routes_file,
            ),
            reload_tx.clone(),
            hot_reload_versions_for_watcher,
            graph_reindex_tx.clone(),
        );
    }

    // Spawn worker threads
    let mut workers = Vec::new();
    // Get routes in main thread and convert to worker-safe formats
    let routes = get_routes();
    let worker_routes = routes_to_worker_routes(&routes);

    // Receive tokio runtime handle from the tokio thread (blocks until available)
    let runtime_handle = runtime_handle_rx
        .recv()
        .expect("Failed to receive runtime handle from tokio thread");

    // Spawn the code-graph reindex worker (dev, opt-in). It reuses the live
    // route table (never re-executing routes.sl, which would pollute the
    // process-global WebSocket registry) and does its SolidB + embedding work
    // off both the request path and the watcher thread.
    if let Some(reindex_rx) = graph_reindex_rx {
        let reindex_handle = runtime_handle.clone();
        let reindex_folder = folder.to_path_buf();
        // Convert to a `Send` string-only snapshot on this thread — `Route`
        // holds `Value`s (`!Send`) and can't cross into the worker.
        let route_snapshot = crate::graph::RouteSnapshot {
            routes: routes
                .iter()
                .map(|r| crate::graph::RouteRef {
                    method: r.method.clone(),
                    path: r.path_pattern.clone(),
                    handler: r.handler_name.clone(),
                })
                .collect(),
            websockets: crate::serve::websocket::get_websocket_routes()
                .iter()
                .map(|w| crate::graph::RouteRef {
                    method: "WS".to_string(),
                    path: w.path_pattern.clone(),
                    handler: w.handler_name.clone(),
                })
                .collect(),
        };
        thread::Builder::new()
            .name("graph-reindex".into())
            .spawn(move || {
                set_tokio_handle(reindex_handle);
                println!("🔁 code-graph auto-reindex on (embedding key detected; set SOLI_GRAPH_WATCH=0 to disable)");
                while reindex_rx.recv().is_ok() {
                    // Coalesce a burst of change signals into one rebuild.
                    while reindex_rx.try_recv().is_ok() {}
                    match crate::graph::reindex(&reindex_folder, None, &route_snapshot) {
                        Ok(r) => println!(
                            "🔁 code graph reindexed: {} nodes, {} edges ({} reused, {} re-embedded)",
                            r.nodes, r.edges, r.reused, r.reembedded
                        ),
                        Err(e) => eprintln!("⚠️  code-graph reindex failed: {}", e),
                    }
                }
            })
            .ok();
    }

    // Receive the actual bound port (may differ from requested if it was in use)
    let actual_port = bound_port_rx
        .recv()
        .expect("Failed to receive bound port from tokio thread");

    // Hand the bound port to an embedding caller before the banner. The
    // listener is already accepting at this point, so a hook that immediately
    // opens a browser won't race the first request.
    if let Some(hook) = on_bound_port {
        hook(actual_port);
    }

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
    if server_constants::using_production_worker_default(num_workers) {
        println!(
            "Using hyper async HTTP server with {} worker threads \
             (production default — set SOLI_WORKERS or --workers to raise)",
            num_workers
        );
    } else {
        println!(
            "Using hyper async HTTP server with {} worker threads",
            num_workers
        );
    }
    if otel::enabled() {
        let cfg = otel::config();
        match &cfg.traces_endpoint {
            Some(ep) => println!(
                "OpenTelemetry tracing enabled → {} (service.name={})",
                ep, cfg.service_name
            ),
            None => println!(
                "OpenTelemetry context enabled (service.name={}; no OTLP endpoint)",
                cfg.service_name
            ),
        }
    }
    if prod_log::format() == prod_log::LogFormat::Json {
        println!("Production logs: JSON (SOLI_LOG_FORMAT=json)");
    }
    println!();

    // Workers are spawned and the listener is accepting: `/_ready` flips to 200
    // here. Deliberately after the banner, so nothing routes to a process that
    // has not finished announcing itself.
    shutdown::mark_ready();

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
    // reserves that many threads for realtime, clamped so at least one HTTP
    // worker always remains. When the split collapses (num_rt_workers == 0)
    // every worker drains every channel, preserving the pre-split behavior —
    // realtime still works, it just shares the pool.
    //
    // The reservation costs one whole HTTP worker, so it is only applied *by
    // default* once the pool is big enough to absorb it
    // (`MIN_WORKERS_FOR_REALTIME_SPLIT`). Defaulting it on at every size made
    // `--workers 2` behave exactly like `--workers 1` — one HTTP worker either
    // way — which silently halved throughput for anyone running a small pool.
    // An explicit `SOLI_WS_WORKERS` is always honored, at any pool size.
    let explicit_rt_workers = std::env::var("SOLI_WS_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok());
    let (num_http_workers, num_rt_workers) =
        server_constants::realtime_worker_split(num_workers, explicit_rt_workers);
    let split_realtime = num_rt_workers > 0;
    if split_realtime {
        println!(
            "Worker pool: {} HTTP + {} realtime (WS/LiveView)",
            num_http_workers, num_rt_workers
        );
    } else {
        println!(
            "Worker pool: {} HTTP (realtime shares the pool)",
            num_http_workers
        );
    }

    // Background job engine: a poller claims due rows from `_jobs` and hands
    // them to this worker pool, so job code never runs on a web worker and a
    // slow handler can't delay requests. Started when the app actually has jobs
    // — `app/jobs` exists (handlers to run) or a mailer is configured
    // (`deliver_later` enqueues the built-in delivery job). `SOLI_JOB_WORKERS`
    // sizes the pool (default 1); 0 disables the engine entirely, leaving
    // enqueued rows for `soli jobs` (or another serve process) to pick up.
    // Under `--dev` the poller ticks every `jobs::DEV_POLL_MS` instead of every
    // second: `soli new` scaffolds `app/jobs/`, so every dev app starts an
    // engine whether or not it uses one, and several open at once would
    // otherwise hammer a shared dev database with idle claim round-trips.
    {
        let mailer_configured = std::env::var("SOLI_SMTP_HOST")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();
        let num_job_workers = std::env::var("SOLI_JOB_WORKERS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(server_constants::DEFAULT_JOB_WORKERS);
        if (jobs_dir.exists() || mailer_configured) && num_job_workers > 0 {
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
            crate::jobs::engine::start(num_job_workers, runtime_handle.clone(), dev_mode);
        }
    }

    // The pinned queues (see `PINNED_LV_TX`): one per realtime worker, in
    // realtime-worker order. With no split every worker is a realtime one.
    let num_pinned = if split_realtime {
        num_rt_workers
    } else {
        num_workers
    };
    let pinned_lv: Vec<(
        channel::Sender<LiveViewEventData>,
        channel::Receiver<LiveViewEventData>,
    )> = (0..num_pinned)
        .map(|_| channel::bounded(capacity_per_worker))
        .collect();
    PINNED_LV_TX.set_once(Arc::new(
        pinned_lv.iter().map(|(tx, _)| tx.clone()).collect(),
    ));

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
        let pinned_lv_rx = realtime_enabled.then(|| {
            let ordinal = if split_realtime {
                i - num_http_workers
            } else {
                i
            };
            pinned_lv[ordinal].1.clone()
        });
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

        // A big stack, because the interpreter recurses natively and a stack
        // overflow aborts the whole process rather than failing one request.
        let builder = thread::Builder::new()
            .name(format!("{}-{}", role_label, i))
            .stack_size(server_constants::worker_stack_bytes());
        // A worker serves the application that spawned it, for its whole life.
        // This is the binding every `TenantValue` on the worker path relies on,
        // and thread-locals do not cross `spawn`, so it is made explicit here
        // rather than assumed.
        let worker_tenant = tenant::current_id();
        let handler = builder.spawn(move || {
            tenant::bind_current(worker_tenant);
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
                let pinned_lv_rx = pinned_lv_rx.clone();
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
                        pinned_lv_rx,
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
    // This worker's own LiveView queue (see `PINNED_LV_TX`); `None` on an
    // HTTP-only worker.
    pinned_lv_rx: Option<channel::Receiver<LiveViewEventData>>,
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
    // Span tree: always on in --dev (flamegraph). Also on when OpenTelemetry
    // is enabled so production can export the same hierarchy over OTLP.
    span_log::set_enabled(dev_mode || otel::enabled());
    // Component prop warnings are a dev-only surface (dev bar + console).
    template_warnings::set_enabled(dev_mode);

    // Matched route per request — feeds the dev bar's "requests" panel and the
    // `X-Soli-Route` response header the client patch reads.
    route_log::set_enabled(dev_mode || log_channels.collect_timing());

    // File mode has no app to load — no models, middleware, controllers, jobs
    // or routes. The worker exists purely to render the `.slv`/`.erb`
    // templates found in the served folder, which the template engine
    // initialized above already points at.
    let app_mode = files::files_root().is_none();

    if app_mode {
        app_loader::load_app_in_worker(
            worker_id,
            interpreter,
            &_models_dir,
            &middleware_dir,
            &controllers_dir,
            &jobs_dir,
        );
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
    let mut pinned_lv_rx_inner = pinned_lv_rx.filter(|_| realtime_enabled);

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
            // Models and the three sibling directories that share their
            // reload signal — see `app_loader::load_models_and_siblings`,
            // which is also what the worker booted with.
            app_loader::load_models_and_siblings(worker_id, interpreter, &_models_dir, "reloading");
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
                    let result = handle_liveview_event_caught(interpreter, &data);
                    let _ = data.response_tx.send(result);
                }
                Err(channel::TryRecvError::Empty) => {}
                Err(channel::TryRecvError::Disconnected) => {
                    lv_event_rx_inner = None;
                }
            }
        }
        if let Some(ref mut rx) = pinned_lv_rx_inner {
            match rx.try_recv() {
                Ok(data) => {
                    let result = handle_liveview_event_caught(interpreter, &data);
                    let _ = data.response_tx.send(result);
                }
                Err(channel::TryRecvError::Empty) => {}
                Err(channel::TryRecvError::Disconnected) => {
                    pinned_lv_rx_inner = None;
                }
            }
        }

        // Batch process HTTP requests using try_recv for non-blocking drain
        // (HTTP workers only; realtime workers never touch the request queue).
        if http_enabled {
            for _ in 0..server_constants::BATCH_SIZE {
                match work_rx.try_recv() {
                    Ok(data) => {
                        // A slot just freed: wake a request waiting for one.
                        pipeline::queue_slot_freed();
                        dispatch_http_request(interpreter, &mut vm, data, dev_mode);
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
            let pinned_idx = pinned_lv_rx_inner.as_ref().map(|rx| sel.recv(rx));

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
                    if let Ok(data) = oper.recv(&work_rx) {
                        pipeline::queue_slot_freed();
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
                        dispatch_http_request(interpreter, &mut vm, data, dev_mode);
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
                            let result = handle_liveview_event_caught(interpreter, &data);
                            let _ = data.response_tx.send(result);
                        }
                    }
                } else if Some(idx) == pinned_idx {
                    if let Some(ref rx) = pinned_lv_rx_inner {
                        if let Ok(data) = oper.recv(rx) {
                            let result = handle_liveview_event_caught(interpreter, &data);
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
    /// The socket's identity, captured once at the upgrade.
    ///
    /// Realtime handlers used to run with no session at all: `set_current_session_id`
    /// was only ever called on the HTTP path, so `session_get` returned null and
    /// `get_current_user()` — which the WebSocket docs show in their presence
    /// example — could not work. The only identity left was whatever the client
    /// put in its first message, which is no identity at all: anyone could join
    /// `user:<id>` channels or track presence as another person. The docs also
    /// promised `headers`, `params` and `query` on the event and none were
    /// delivered.
    context: Arc<WebSocketContext>,
    response_tx: oneshot::Sender<WebSocketActionData>,
}

/// What the HTTP upgrade knew about a socket, kept for the life of the
/// connection so every event can be handled as that user.
#[derive(Debug, Default)]
struct WebSocketContext {
    /// Session id resolved from the upgrade request's cookies, when it had any.
    session_id: Option<String>,
    /// Request headers at upgrade time, lowercased names.
    headers: Vec<(String, String)>,
    /// Parsed query string of the upgrade URL.
    query: Vec<(String, String)>,
    /// Peer address of the socket.
    peer_ip: String,
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
    /// Session of the socket that sent this event, when a client sent it.
    ///
    /// Upload ids are owned by the session that uploaded them, and the taker has
    /// to be that same session. Using the *instance's* session instead meant a
    /// room — one instance deliberately shared by many visitors — only let the
    /// visitor who happened to mount it complete an upload. `None` for
    /// server-originated events (ticks, live-query wakeups), which carry no
    /// upload ids.
    pub sender_session: Option<String>,
    /// Response channel - sends back result
    pub response_tx: oneshot::Sender<Result<(), String>>,
}

/// Everything the request path needs that belongs to one served application.
///
/// Bundled rather than passed as seven arguments because the point of the
/// tenant work is that a process can serve more than one application: a host
/// picks *which* bundle from the `Host` header, and that has to be one lookup
/// against one value, not seven parallel maps. With a single application there
/// is exactly one of these and nothing about the request path changes.
///
/// Everything in it is cheap to clone — an `Arc`, a crossbeam `Sender`, a
/// `broadcast::Sender`. A request borrows it from the router rather than
/// cloning it, which would bump (and drop) each sender's shared refcount.
#[derive(Clone)]
struct TenantRuntime {
    /// Which application this serves. The request future is scoped to it
    /// (`tenant::task_scope`) before any tenant-keyed state is consulted; the
    /// worker side needs no such thing, its threads being pinned.
    tenant: tenant::TenantId,
    request_tx: WorkerSender,
    reload_tx: Option<broadcast::Sender<()>>,
    public_dir: Arc<PathBuf>,
    asset_cache: asset_cache::AssetCache,
    ws_event_tx: channel::Sender<WebSocketEventData>,
    lv_event_tx: channel::Sender<LiveViewEventData>,
    dev_mode: bool,
}

async fn handle_hyper_request(
    mut req: Request<Incoming>,
    runtime: &TenantRuntime,
    peer_addr: SocketAddr,
) -> Result<Response<ResponseBody>, hyper::Error> {
    // Destructured rather than accessed through `runtime.` throughout: the body
    // below is long, and every one of these names already meant exactly this.
    let TenantRuntime {
        // Not read here: the request future is already scoped to it, and the
        // spawns below inherit it through `tenant::spawn`.
        tenant: _tenant,
        request_tx,
        reload_tx,
        public_dir,
        asset_cache,
        ws_event_tx,
        lv_event_tx,
        dev_mode,
    } = runtime;
    let dev_mode = *dev_mode;
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
    let path = req.uri().path().to_string();
    // Owned so no borrow of `req` outlives this line: the blocks below hand
    // `req` itself to their handlers, and a live `req.uri()` borrow would stop
    // them. The native bridge's stream route also needs the raw query string
    // to verify its channel token.
    let raw_query = req.uri().query().map(|q| q.to_string());
    let request_start = std::time::Instant::now();

    // Increment total request counter before any routing decisions
    crate::metrics::Metrics::global()
        .http_requests_total
        .fetch_add(1, Ordering::Relaxed);

    // Desktop launch gate. Loopback keeps the network out but not the machine:
    // every process running as this user can reach the port. Runs before any
    // routing (including /_metrics) so an ungated caller cannot reach anything
    // at all. Completely inert unless a desktop boot armed it, so ordinary
    // `soli serve` pays one atomic load. See `desktop::token`.
    if let Some(response) = crate::desktop::token::gate_request(
        &path,
        req.uri().query(),
        req.headers()
            .get(hyper::header::COOKIE)
            .and_then(|v| v.to_str().ok()),
    ) {
        return Ok(response);
    }

    // Liveness, readiness and Prometheus metrics: answered from the path, the
    // headers and the peer, before any routing. See `probes`.
    if let Some(response) = probes::handle(&path, &method, req.headers(), peer_addr.ip()) {
        return Ok(response);
    }

    // The built-in LiveView client. Before the same-origin gate, where it has
    // always been. See `framework_assets`.
    if let Some(response) = framework_assets::live_client(
        &path,
        &method,
        req.headers()
            .get("if-none-match")
            .and_then(|v| v.to_str().ok()),
    ) {
        return Ok(response);
    }

    // SEC-014: same-origin gate for state-changing requests. Runs before
    // routing so a cross-origin POST is rejected before any controller
    // sees it. WebSocket upgrades have their own check (`websocket_origin_allowed`,
    // once per branch inside `upgrade`), so the early return doesn't fire on
    // those.
    if !hyper_tungstenite::is_upgrade_request(&req) {
        if let Err(reason) =
            crate::interpreter::builtins::trust_proxy::with_peer_ip(peer_addr.ip(), || {
                check_csrf_origin(req.headers(), &method, &path)
            })
        {
            return Ok(forbidden_csrf_response(&reason));
        }
    }

    // The three EUI things a plain GET can ask for: a content-addressed
    // asset, the manifest, and a one-shot render. After the desktop gate and
    // the origin check, where they stood — the `/_eui/view/` comment records
    // that being on this side of it is deliberate. See `eui::http_get`.
    #[cfg(feature = "eui")]
    if let Some(response) = eui::http_get(
        &path,
        &method,
        raw_query.as_deref(),
        req.headers(),
        lv_event_tx,
    )
    .await
    {
        return Ok(response);
    }

    // The four sockets this binary upgrades to: live reload, an EUI session,
    // a LiveView, and the application's own `websocket_routes`. This runs
    // *ungated* by the same-origin check above — that is what the
    // `!is_upgrade_request` on it buys — and each of the four compensates with
    // its own `websocket_origin_allowed`. See `upgrade`.
    if hyper_tungstenite::is_upgrade_request(&req) {
        return upgrade::handle(
            req,
            &path,
            raw_query.as_deref(),
            peer_addr,
            reload_tx.as_ref(),
            ws_event_tx,
            lv_event_tx,
        )
        .await;
    }

    // A file under `public/`: resolution, the traversal and symlink checks,
    // and the conditional-GET / `Range` / full-body reply. See `static_files`.
    if let Some(response) = static_files::handle(
        &path,
        &method,
        public_dir,
        asset_cache,
        dev_mode,
        req.headers(),
    ) {
        return Ok(response);
    }

    // Everything the binary serves from itself: the nav and prefetch scripts,
    // the native bridge and its helpers, the generated directory pages' own
    // assets. After the same-origin gate, where these blocks stood. See
    // `framework_assets`.
    if let Some(response) = framework_assets::bundled(
        &path,
        &method,
        req.headers()
            .get("if-none-match")
            .and_then(|v| v.to_str().ok()),
        raw_query.as_deref(),
    ) {
        return Ok(response);
    }

    // The live-reload long-poll, and the SEC-043 origin check in front of it.
    // See `live_reload`.
    if let Some(response) =
        live_reload::handle(&path, req.headers(), peer_addr.ip(), reload_tx.as_ref()).await
    {
        return Ok(response);
    }

    // Job dashboard: open in --dev to a local request (loopback peer, local
    // host name); otherwise, and in production, only when credentials
    // are configured (Basic and/or Bearer). Unconfigured production 404s
    // so the route does not advertise itself.
    if let Some(resp) = dev_jobs::dispatch(
        &method,
        &path,
        req.uri().query(),
        req.headers(),
        dev_mode,
        peer_addr.ip(),
    ) {
        return Ok(resp);
    }

    // The `--dev` diagnostics bank — REPL, source reader, request inspector,
    // replay, the component and mailer catalogues and the sent-mail inbox —
    // behind the one trusted-peer gate that keeps them off the LAN. See
    // `dev_routes`.
    req = match dev_routes::dispatch(req, &method, &path, peer_addr, request_tx, dev_mode).await {
        Ok(response) => return Ok(response),
        Err(req) => req,
    };

    // The coverage dump the test runner scrapes before it kills this
    // process, and the token that keeps it from being readable by anyone
    // else. See `coverage`.
    if let Some(response) = coverage::handle(&path, &method, req.headers()) {
        return Ok(response);
    }

    // Every framework handler for the reserved `/__…` namespace has had its
    // turn. Whatever is left must not reach the application: those paths are
    // exempt from both CSRF layers, and a `post("/:locale/account/delete")`
    // route would otherwise bind `:locale` to `__soli` and run with no CSRF
    // check at all (production answers none of the dev endpoints, so they
    // all fell through). See `csrf::is_reserved_framework_path`.
    if csrf::is_reserved_framework_path(&path) {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(full(Bytes::from("Not Found")))
            .unwrap());
    }

    // File mode: the served folder is a plain directory, not an MVC app.
    // Directory indexes, Markdown pages and static files are answered right
    // here; only a `.slv`/`.erb` template needs an interpreter, and that falls
    // through to the worker queue below with `file_template` set.
    //
    // Placed after the framework endpoints (`/__livereload`, `/__soli/*`) so
    // live reload and the generated pages' own assets keep working — the file
    // resolver would otherwise answer those paths with a 404 page.
    let mut file_template: Option<String> = None;
    if let Some(root) = files::files_root() {
        match files::handle(
            &path,
            &method,
            root,
            req.headers(),
            raw_query.as_deref(),
            dev_mode,
        ) {
            files::Outcome::Response(response) => return Ok(response),
            files::Outcome::Template(relative) => file_template = Some(relative),
        }
    }

    // From here the request is the application's to answer: read the body,
    // hand the work to a worker, and assemble the reply. See `pipeline`.
    let pipeline::Intake {
        method,
        query,
        headers,
        body,
        body_reservation,
        multipart_form,
        multipart_files,
        if_none_match,
        is_prefetch,
    } = match pipeline::intake(req, method, raw_query.as_deref(), peer_addr.ip()).await {
        Ok(intake) => intake,
        Err(response) => return Ok(*response),
    };

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
        body_reservation,
        multipart_form,
        multipart_files,
        peer_ip: peer_addr.ip().to_string(),
        enqueued_at: prod_log::channels().any().then(std::time::Instant::now),
        replay: false,
        file_template,
        response_tx,
    };

    if let Err(busy) = pipeline::enqueue(request_tx, request_data).await {
        return Ok(*busy);
    }

    let worker_response =
        match pipeline::await_worker(response_rx, &log_method, &log_path, request_start).await {
            Ok(worker_response) => worker_response,
            Err(response) => return Ok(*response),
        };

    Ok(pipeline::assemble(
        worker_response,
        if_none_match.as_deref(),
        is_prefetch,
        dev_mode,
        reload_tx.is_some(),
    ))
}

fn forbidden_csrf_response(reason: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(full(Bytes::from(format!("CSRF check failed: {}", reason))))
        .unwrap()
}

/// [`websocket_origin_allowed`] for a request from `peer`, so a
/// `SOLI_TRUSTED_PROXIES` list decides whether `X-Forwarded-Host` counts.
pub(crate) fn websocket_origin_allowed_from(
    headers: &hyper::HeaderMap,
    peer: std::net::IpAddr,
) -> bool {
    crate::interpreter::builtins::trust_proxy::with_peer_ip(peer, || {
        websocket_origin_allowed(headers)
    })
}

pub(crate) fn websocket_origin_allowed(headers: &hyper::HeaderMap) -> bool {
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

pub(crate) fn websocket_request_authority(headers: &hyper::HeaderMap) -> Option<String> {
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

/// Clears the thread-local session when a realtime handler returns, however it
/// returns. Worker threads are reused across sockets and requests, so a leaked
/// session id would be read by whatever runs next on that thread.
struct RealtimeSessionGuard;

impl Drop for RealtimeSessionGuard {
    fn drop(&mut self) {
        set_current_session_id(None);
    }
}

/// Restores the default locale when a request or a realtime frame ends.
///
/// The locale is a thread-local and workers are reused, so without this a
/// controller that never called `set_locale` rendered in whatever language
/// the previous visitor on that worker had asked for — and `Model#save`
/// wrote its translated fields into that visitor's locale slot.
struct LocaleGuard;

impl Drop for LocaleGuard {
    fn drop(&mut self) {
        crate::interpreter::builtins::i18n::helpers::set_locale("");
    }
}

/// The locale for a request: what the session or a `locale` cookie says,
/// else what `Accept-Language` asks for among the locales that are loaded,
/// else the application's default.
///
/// The application can still override it at any point with `set_locale`;
/// this only decides where the request *starts*.
fn resolve_request_locale(
    headers: &hyper::header::HeaderMap,
    cookie_pairs: &HashPairs,
) -> Option<String> {
    use crate::interpreter::builtins::i18n::helpers;

    // A stored choice wins: the person picked it. Read without creating a
    // session — `get_current_session_id` is already resolved by here, and is
    // `None` for a visitor who has never stored anything.
    if let Some(id) = crate::interpreter::builtins::session::get_current_session_id() {
        if let Some(stored) = crate::interpreter::builtins::session::get_current_store()
            .get(&id, "locale")
            .and_then(|json| json.as_str().map(str::to_owned))
        {
            if !stored.is_empty() {
                return Some(stored);
            }
        }
    }
    if let Some(Value::String(value)) = cookie_pairs.get(&HashKey::String("locale".into())) {
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    // Then what the browser asks for, but only among locales we actually
    // have — negotiating to a locale with no translations would just fall
    // back on every key.
    let available = helpers::available_locales();
    if !available.is_empty() {
        if let Some(header) = header_str(headers, "accept-language") {
            if let Some(hit) = helpers::negotiate(header, &available) {
                return Some(hit);
            }
        }
    }
    None
}

/// Build a Soli hash from `(name, value)` pairs.
fn string_pairs_to_hash(pairs: &[(String, String)]) -> Value {
    let mut map: HashPairs = HashPairs::default();
    for (name, value) in pairs {
        map.insert(
            HashKey::String(name.clone().into()),
            Value::String(value.clone().into()),
        );
    }
    Value::Hash(Rc::new(RefCell::new(map)))
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

    // A socket event is a request as far as the per-request logs go: it never
    // passes through `request_scope::install`, so without this a WebSocket
    // worker's query/HTTP/span logs only ever filled (see the LiveView/EUI
    // event path below for the measured cost).
    request_scope::forget_request_logs(crate::interpreter::builtins::template::is_dev_mode());

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

    // Install the socket's session for the duration of the handler, so
    // `session_get` / `get_current_user()` answer for the connected user
    // instead of returning null. Cleared again below on every exit path — a
    // worker thread is reused, and a leaked session would be read by the next
    // request it serves.
    set_current_session_id(data.context.session_id.clone());
    let _session_guard = RealtimeSessionGuard;

    // Build event hash: {type, connection_id, message, channel?, headers,
    // query, peer_ip}
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

    // The upgrade context the docs already promised. `headers` and `query` let
    // a handler authenticate a socket without trusting whatever the client puts
    // in its first message.
    event_map.insert(
        HashKey::String("headers".into()),
        string_pairs_to_hash(&data.context.headers),
    );
    event_map.insert(
        HashKey::String("query".into()),
        string_pairs_to_hash(&data.context.query),
    );
    event_map.insert(
        HashKey::String("params".into()),
        string_pairs_to_hash(&data.context.query),
    );
    event_map.insert(
        HashKey::String("peer_ip".into()),
        Value::String(data.context.peer_ip.clone().into()),
    );

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

/// `POST /live/upload` — stash multipart files and return their ids.
fn handle_live_upload(data: &RequestData) -> ResponseData {
    // Bind each file to the uploading session so only that session's LiveView
    // can hydrate it, and so the per-session slot cap applies.
    let owner = crate::interpreter::builtins::session::extract_session_id_from_cookie(
        data.headers.get("cookie").and_then(|v| v.to_str().ok()),
    );
    let files = data.multipart_files.as_deref().unwrap_or(&[]);
    if files.is_empty() {
        return ResponseData {
            status: 400,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: br#"{"error":"no file"}"#.to_vec(),
        };
    }
    let chunk_id = header_str(&data.headers, "x-soli-upload-id").map(|s| s.to_string());
    let chunk_index =
        header_str(&data.headers, "x-soli-chunk-index").and_then(|s| s.parse::<usize>().ok());
    let chunk_total =
        header_str(&data.headers, "x-soli-chunk-count").and_then(|s| s.parse::<usize>().ok());
    let mut out = Vec::new();
    for file in files {
        let result = if let (Some(id), Some(index), Some(total)) =
            (chunk_id.as_deref(), chunk_index, chunk_total)
        {
            crate::live::upload::put_chunk(
                owner.as_deref(),
                id,
                index,
                total,
                &file.name,
                &file.filename,
                &file.content_type,
                file.data.to_vec(),
            )
        } else {
            crate::live::upload::put(
                owner.as_deref(),
                &file.name,
                &file.filename,
                &file.content_type,
                file.data.to_vec(),
            )
        };
        match result {
            Ok(meta) => out.push(meta),
            Err(e) => {
                return ResponseData {
                    status: 413,
                    headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                    body: serde_json::json!({ "error": e }).to_string().into_bytes(),
                };
            }
        }
    }
    let payload = if out.len() == 1 {
        out.remove(0)
    } else {
        serde_json::Value::Array(out)
    };
    ResponseData {
        status: 200,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: payload.to_string().into_bytes(),
    }
}

/// Report a LiveView handler failure to the client and the log.
///
/// The browser gets the message only in dev; production gets a generic string so
/// a handler's error text (which can carry paths or query fragments) stays
/// server-side.
fn report_liveview_handler_error(
    liveview_id: &str,
    handler_name: &str,
    error: &str,
) -> Result<(), String> {
    eprintln!("[LiveView] {handler_name} failed: {error}");
    let message = if live_reload::is_live_reload_enabled() {
        format!("{handler_name}: {error}")
    } else {
        "LiveView handler error".to_string()
    };
    let _ = crate::live::view::live_registry().send(
        liveview_id,
        crate::live::view::ServerMessage::Error { message },
    );
    Err(error.to_string())
}

/// Handle a LiveView event by calling the controller handler.
fn adopt_live_state(
    instance: &mut crate::live::view::LiveViewInstance,
    mut state: serde_json::Value,
) {
    if let (serde_json::Value::Object(old), serde_json::Value::Object(new_obj)) =
        (&instance.state, &mut state)
    {
        if let Some(id) = old.get("id") {
            new_obj.insert("id".to_string(), id.clone());
        }
        let key = crate::live::nested::COMPONENTS_KEY;
        if let Some(comps) = old.get(key) {
            if !new_obj.contains_key(key) {
                new_obj.insert(key.to_string(), comps.clone());
            }
        }
    }
    instance.state = state;
}

fn lookup_live_handler(interpreter: &Interpreter, component: &str) -> Option<Value> {
    let handler_name = crate::live::socket::get_liveview_handler(component)?;
    match crate::interpreter::builtins::router::resolve_handler(&handler_name, None) {
        Ok(h) => Some(h),
        Err(_) => {
            let action = handler_name.split('#').next_back().unwrap_or(&handler_name);
            interpreter.environment.borrow().get(action)
        }
    }
}

fn invoke_component_handler(
    interpreter: &mut Interpreter,
    handler: Value,
    event: &str,
    state: &serde_json::Value,
    params: &serde_json::Value,
    assigns: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let mut event_map: HashPairs = HashPairs::default();
    event_map.insert(
        HashKey::String("event".into()),
        Value::String(event.to_string().into()),
    );
    event_map.insert(HashKey::String("params".into()), json_to_value(params));
    event_map.insert(HashKey::String("state".into()), json_to_value(state));
    event_map.insert(HashKey::String("assigns".into()), json_to_value(assigns));
    let event_value = Value::Hash(Rc::new(RefCell::new(event_map)));
    match interpreter.call_value(handler, vec![event_value], Span::default()) {
        Ok(Value::Null) => Ok(None),
        Ok(ref result @ Value::Hash(_)) => {
            let unwrapped = unwrap_handler_return(value_to_json(result));
            Ok(unwrapped.state)
        }
        Ok(_) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn apply_live_updates(
    interpreter: &mut Interpreter,
    instance: &mut crate::live::view::LiveViewInstance,
    extra: Option<serde_json::Value>,
) {
    let named = crate::live::update::apply_to(&mut instance.state, extra);
    for (name, cid, assigns) in named {
        let Some(handler) = lookup_live_handler(interpreter, &name) else {
            continue;
        };
        let child = crate::live::nested::get_component_state(&instance.state, &cid)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        match invoke_component_handler(interpreter, handler, "update", &child, &assigns, &assigns) {
            Ok(Some(new_state)) => {
                crate::live::nested::put_component_state(&mut instance.state, &cid, new_state);
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("[LiveView] child update {name} failed: {e}");
            }
        }
    }
}

/// `handle_liveview_event` behind the guard HTTP handlers already have.
///
/// A panic in a handler or in the EUI encoder used to unwind out of the
/// worker loop, where the catch-all rebuilds the interpreter and reloads the
/// whole application — every event in flight on that worker lost with it,
/// and a client that can provoke the panic (a keyed list with two rows of one
/// key was enough) could keep the realtime side restarting. Now it fails this
/// one event and answers the socket; the frame lock and the encoder lock are
/// poisoned by the unwind and both are taken with `into_inner`, and an encoder
/// left mid-render simply re-mounts on its next frame.
fn handle_liveview_event_caught(
    interpreter: &mut Interpreter,
    data: &LiveViewEventData,
) -> Result<(), String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handle_liveview_event(interpreter, data)
    })) {
        Ok(result) => result,
        Err(_) => {
            crate::metrics::Metrics::global()
                .handler_panics_total
                .fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "[LiveView] handler panicked on {} {:?} — event dropped (worker survived)",
                data.component, data.event
            );
            Err(format!("handler panicked on event '{}'", data.event))
        }
    }
}

fn handle_liveview_event(
    interpreter: &mut Interpreter,
    data: &LiveViewEventData,
) -> Result<(), String> {
    use crate::interpreter::value::Value;
    use crate::live::view::live_registry;

    // Run the handler as the socket's user. `set_current_session_id` was only
    // called on the HTTP path, so `session_get` and `get_current_user()`
    // returned null inside every LiveView handler — leaving a client-supplied id
    // as the only identity available, which is no identity at all. A synthetic
    // `sess-<uuid>` handle (a cookie-less socket) is not a session and is not
    // installed; those handlers see no user, which is the truth.
    // A session that has gone lets go of what this worker kept for it. The
    // memo is thread-local, so this has to run here, on the worker the
    // session was pinned to — not on the socket task that noticed the close.
    #[cfg(feature = "eui")]
    if data.event == eui::FORGET_EVENT {
        eui::forget_on_worker(&data.liveview_id);
        return Ok(());
    }

    let socket_session = data
        .sender_session
        .as_deref()
        .filter(|id| !id.starts_with("sess-"))
        .map(|id| id.to_string());
    set_current_session_id(socket_session);
    let _session_guard = RealtimeSessionGuard;
    // The socket's own locale, from its session — a realtime worker serves
    // many sessions and would otherwise render this frame in whatever
    // language the last frame on this thread used. An EUI session is pinned
    // to one worker, so without this it would inherit a neighbour's locale
    // and keep it for the life of the socket.
    let socket_locale = crate::interpreter::builtins::session::get_current_session_id()
        .and_then(|id| {
            crate::interpreter::builtins::session::get_current_store()
                .get(&id, "locale")
                .and_then(|json| json.as_str().map(str::to_owned))
        })
        .unwrap_or_default();
    crate::interpreter::builtins::i18n::helpers::set_locale(&socket_locale);
    let _locale_guard = LocaleGuard;

    // One frame at a time for this LiveView. A tick and a client event land on
    // different workers; without this they both read the same state, both render
    // from it, and the slower one overwrites the other's state and regresses
    // `last_html` — the client then gets a diff against markup it never saw.
    let frame_lock = live_registry().frame_lock(&data.liveview_id);
    let _frame = frame_lock.lock().unwrap_or_else(|e| e.into_inner());

    // An event is a request, as far as the per-request logs are concerned.
    //
    // They are thread-local `Vec`s that an HTTP request empties on the way
    // in (`request_scope::forget_request_logs`) and fills as it goes; a socket event
    // never went through that door, so on a realtime worker they only ever
    // filled. In `--dev` the flamegraph records a span per function call:
    // an EUI view that redraws ten times a second grew that log by close
    // to a megabyte a second until the window went slow, then froze — and
    // it read like a leak in the view, because nothing in the view was
    // keeping anything. Measured on herdr-eui: 0.85 MB/s with `--dev`,
    // 0.16 MB/s without, on the same page.
    request_scope::forget_request_logs(crate::interpreter::builtins::template::is_dev_mode());

    // Get the LiveView instance
    let mut instance = live_registry()
        .get(&data.liveview_id)
        .ok_or_else(|| format!("LiveView not found: {}", data.liveview_id))?;

    let component = instance.component.clone();

    // An EUI component renders a node tree, not HTML: same handler, other
    // wire. Decided here so the worker loop stays untouched.
    #[cfg(feature = "eui")]
    if eui::is_eui_component(&component) {
        return eui::handle_eui_event(interpreter, data, &mut instance);
    }

    // Try to find a registered handler for this component
    let handler_name = crate::live::socket::get_liveview_handler(&component);

    // Build event hash for the controller: {event, params, state}.
    // Hydrate uploads and merge child `_assigns` *before* snapshotting
    // state — a typical handler returns that hash, so a pre-merge
    // snapshot would drop the child's patch.
    let mut event_params = data.params.clone();

    // `_assigns` and `_component` are chosen by the socket. Constrain both to
    // what this instance's own markup declares: the client may echo back the
    // `soli-assign-*` attributes and drive the components the server rendered
    // for it, and nothing else. Without this, one message could overwrite any
    // root-state key (identity, tenant, price) or invoke a component handler
    // that was never mounted, with a fabricated state bag.
    let allowed_assigns = crate::live::nested::allowed_assign_keys(&instance.last_html);
    let refused = crate::live::nested::restrict_event_assigns(&mut event_params, &allowed_assigns);
    if !refused.is_empty() {
        eprintln!(
            "[LiveView] {} ignored undeclared assign(s) on event {:?}: {}",
            component,
            data.event,
            refused.join(", ")
        );
    }
    // The session that actually sent the event owns any upload ids in it. Falling
    // back to the instance's own session keeps server-originated events working;
    // using it *first* was the bug — a room instance carries its creator's
    // session, so no other visitor in the room could complete an upload.
    let taker_session = data
        .sender_session
        .clone()
        .unwrap_or_else(|| instance.session_id.clone());
    let state_value = json_to_value(&crate::live::nested::prepare_handler_state(
        &mut instance.state,
        &mut event_params,
        Some(taker_session.as_str()),
    ));
    let params_value = json_to_value(&event_params);

    let mut event_map: HashPairs = HashPairs::default();
    event_map.insert(
        HashKey::String("event".into()),
        Value::String(data.event.clone().into()),
    );
    event_map.insert(HashKey::String("params".into()), params_value);
    event_map.insert(HashKey::String("state".into()), state_value);
    let event_value = Value::Hash(Rc::new(RefCell::new(event_map)));

    // Child event with its own router_live handler: run update-style
    // dispatch on the child bag and skip the parent handler.
    if let Some(child_name) = event_params
        .get("_component")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    {
        if child_name != component {
            let cid = event_params
                .get("_component_id")
                .and_then(|v| v.as_str())
                .unwrap_or(child_name.as_str())
                .to_string();
            // Only components this instance actually rendered, or that already
            // hold state here, may be addressed. `lookup_live_handler` searches
            // every `router_live` registration in the process, so without this
            // any socket could name any component in the app.
            let already_mounted =
                crate::live::nested::get_component_state(&instance.state, &cid).is_some();
            let rendered_here = crate::live::nested::rendered_component_names(&instance.last_html)
                .contains(&child_name);
            if !already_mounted && !rendered_here {
                eprintln!(
                    "[LiveView] refused event for component {child_name:?}: not rendered in this view"
                );
                return Err(format!(
                    "component '{child_name}' is not part of this LiveView"
                ));
            }
            if let Some(handler) = lookup_live_handler(interpreter, &child_name) {
                let mut child = crate::live::nested::get_component_state(&instance.state, &cid)
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                if let Some(assigns) = event_params.get("_assigns") {
                    crate::live::nested::merge_child_assigns(&mut child, assigns);
                }
                match invoke_component_handler(
                    interpreter,
                    handler,
                    &data.event,
                    &child,
                    &event_params,
                    &child,
                ) {
                    Ok(Some(new_state)) => {
                        crate::live::nested::put_component_state(
                            &mut instance.state,
                            &cid,
                            new_state,
                        );
                    }
                    Ok(None) => {}
                    Err(e) => {
                        return report_liveview_handler_error(
                            &instance.id,
                            &format!("live#{child_name}"),
                            &e,
                        );
                    }
                }
                return render_and_send_patch(&component, &mut instance);
            }
        }
    }

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
                //   2. `{ "state": {...}, "tick_interval": N, "stream": {...},
                //          "redirect": "/x", "js": [...] }`
                //      — wrapped form; `state` (optional) is the new state,
                //      `tick_interval` (ms) controls the tick timer, `stream`
                //      carries targeted container ops, `redirect` navigates
                //      the client away, `patch` updates the URL without leaving
                //      the socket, and `js` is an eval-free command list.
                match &result {
                    // No return value means "no state change" — re-render and
                    // let the (usually empty) diff speak. It must not run the
                    // built-in demo state machine.
                    Value::Null => {
                        apply_live_updates(interpreter, &mut instance, None);
                    }
                    Value::Hash(_) => {
                        let json = value_to_json(&result);
                        let unwrapped = unwrap_handler_return(json);

                        if let Some(url) = unwrapped.redirect.as_deref() {
                            use crate::live::view::live_registry;
                            let _ = live_registry().send(
                                &instance.id,
                                crate::live::view::ServerMessage::Redirect {
                                    url: url.to_string(),
                                },
                            );
                            // Persist any state change, then leave — the
                            // client is navigating away.
                            if let Some(state) = unwrapped.state {
                                adopt_live_state(&mut instance, state);
                            }
                            apply_live_updates(interpreter, &mut instance, unwrapped.update);
                            apply_tick_interval(&mut instance, unwrapped.tick);
                            instance.touch();
                            live_registry().commit(&instance);
                            return Ok(());
                        }

                        if let Some(cmds) = unwrapped.js {
                            use crate::live::view::live_registry;
                            let _ = live_registry()
                                .send(&instance.id, crate::live::view::ServerMessage::Js { cmds });
                        }

                        if let Some((url, replace)) = unwrapped.patch {
                            use crate::live::view::live_registry;
                            let _ = live_registry().send(
                                &instance.id,
                                crate::live::view::ServerMessage::Url { url, replace },
                            );
                        }

                        if let Some(url) = unwrapped.live {
                            use crate::live::view::live_registry;
                            let _ = live_registry()
                                .send(&instance.id, crate::live::view::ServerMessage::Live { url });
                            if let Some(state) = unwrapped.state {
                                adopt_live_state(&mut instance, state);
                            }
                            apply_live_updates(interpreter, &mut instance, unwrapped.update);
                            apply_tick_interval(&mut instance, unwrapped.tick);
                            instance.touch();
                            live_registry().commit(&instance);
                            return Ok(());
                        }

                        // Replace state only when the handler supplied one (a
                        // stream-only emission leaves the current state intact).
                        if let Some(state) = unwrapped.state {
                            adopt_live_state(&mut instance, state);
                        }
                        apply_live_updates(interpreter, &mut instance, unwrapped.update);

                        // Apply tick scheduling change (if any)
                        apply_tick_interval(&mut instance, unwrapped.tick);

                        // Push targeted collection ops straight to the client
                        // (append/prepend/remove/…). Streamed rows live outside
                        // the diff shadow, so this never fights render patches.
                        if let Some(stream_val) = unwrapped.stream {
                            let ops = build_stream_ops(&stream_val);
                            if !ops.is_empty() {
                                use crate::live::view::live_registry;
                                let _ = live_registry().send(
                                    &instance.id,
                                    crate::live::view::ServerMessage::Stream {
                                        liveview_id: instance.id.clone(),
                                        ops,
                                    },
                                );
                            }
                        }
                    }
                    other => {
                        // A handler that returns something else is a bug in the
                        // app; say so instead of silently running unrelated
                        // built-in logic.
                        return report_liveview_handler_error(
                            &instance.id,
                            &handler_name,
                            &format!(
                                "handler must return a hash or nothing, got {}",
                                other.type_name()
                            ),
                        );
                    }
                }
            }
            Err(e) => {
                return report_liveview_handler_error(&instance.id, &handler_name, &e.to_string());
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

/// Unpacked LiveView handler return.
struct LiveHandlerReturn {
    state: Option<serde_json::Value>,
    tick: Option<u64>,
    stream: Option<serde_json::Value>,
    redirect: Option<String>,
    js: Option<serde_json::Value>,
    /// In-socket URL update: `(url, replace)`.
    patch: Option<(String, bool)>,
    /// Swap the page-root LiveView to another `/live/socket/<component>`.
    live: Option<String>,
    /// Phoenix-style `update:` hash merged onto state after the handler.
    update: Option<serde_json::Value>,
    /// `{"close": reason}`: end the session. An EUI `connect` that finds no
    /// user, or no right to this component, says so here — the socket is
    /// the only place a component can be refused, since no middleware runs
    /// for an upgrade.
    close: Option<String>,
}

/// True when a handler's hash is the new state itself — none of the
/// wrapper keys (`state` as an object, `stream`, `js`, `redirect`, `patch`,
/// `live`, `update` as an object, a truthy `close`) is present.
pub(crate) fn handler_return_is_bare(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    !(map.get("state").is_some_and(|v| v.is_object())
        || map.contains_key("stream")
        || map.contains_key("js")
        || map.contains_key("redirect")
        || map.contains_key("patch")
        || map.contains_key("live")
        || map.get("update").is_some_and(|v| v.is_object())
        || map.get("close").is_some_and(|v| {
            !matches!(v, serde_json::Value::Null | serde_json::Value::Bool(false))
        }))
}

/// Unwrap the handler return value. The wrapped form
/// `{ "state": {...}, "tick_interval": N, "stream": {...}, "redirect": "/x",
///    "patch": "/y", "js": [...] }` yields the inner state (or `None` to keep
/// the current state), the tick interval, stream ops, a client redirect, an
/// in-socket URL update, and JS commands.
/// The bare form (no wrapper keys) is the new state itself.
///
/// `tick_interval` interpretation:
///   * key absent → `None`     (don't touch the running timer)
///   * value 0    → `Some(0)`  (stop the timer)
///   * value > 0  → `Some(ms)` (start or replace the timer)
fn unwrap_handler_return(json: serde_json::Value) -> LiveHandlerReturn {
    // Caller only invokes this with a hash result, but be defensive.
    let mut map = match json {
        serde_json::Value::Object(m) => m,
        other => {
            return LiveHandlerReturn {
                state: Some(other),
                tick: None,
                stream: None,
                redirect: None,
                js: None,
                patch: None,
                live: None,
                update: None,
                close: None,
            }
        }
    };

    let has_state_obj = map.get("state").is_some_and(|v| v.is_object());
    let has_update = map.get("update").is_some_and(|v| v.is_object());
    let has_close = map
        .get("close")
        .is_some_and(|v| !matches!(v, serde_json::Value::Null | serde_json::Value::Bool(false)));

    // Bare shape: the whole hash is the new state.
    if handler_return_is_bare(&map) {
        return LiveHandlerReturn {
            state: Some(serde_json::Value::Object(map)),
            tick: None,
            stream: None,
            redirect: None,
            js: None,
            patch: None,
            live: None,
            update: None,
            close: None,
        };
    }

    let state = if has_state_obj {
        map.remove("state")
    } else {
        None
    };
    let tick = map.get("tick_interval").and_then(|v| v.as_u64());
    let stream = map.remove("stream");
    let redirect = map
        .remove("redirect")
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let js = map.remove("js");
    let patch = parse_patch_url(map.remove("patch"));
    let live = map
        .remove("live")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    let update = if has_update {
        map.remove("update")
    } else {
        None
    };
    // `close: true` closes with no particular reason; a string is the reason
    // the client is shown; `false`/`null` is not a close at all.
    let close = if has_close {
        map.remove("close").map(|v| match v {
            serde_json::Value::String(reason) => reason,
            serde_json::Value::Bool(true) => "closed".to_string(),
            other => other.to_string(),
        })
    } else {
        None
    };
    LiveHandlerReturn {
        state,
        tick,
        stream,
        redirect,
        js,
        patch,
        live,
        update,
        close,
    }
}

/// `patch: "/path"` or `patch: { "url": "/path", "replace": true }`.
fn parse_patch_url(value: Option<serde_json::Value>) -> Option<(String, bool)> {
    match value? {
        serde_json::Value::String(url) if !url.is_empty() => Some((url, false)),
        serde_json::Value::Object(map) => {
            let url = map.get("url").and_then(|v| v.as_str())?.to_string();
            if url.is_empty() {
                return None;
            }
            let replace = map
                .get("replace")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some((url, replace))
        }
        _ => None,
    }
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

    // No-op only if the interval is unchanged *and* a task is actually running.
    // After a reconnect the instance still remembers its interval while the task
    // was aborted at disconnect — treating that as unchanged silently stops a
    // ticking view for good.
    let tick_running = crate::live::socket::has_tick_task(&instance.id);
    if instance.tick_interval_ms == Some(requested) && tick_running {
        return;
    }

    let Some(tx) = lv_sender_for(&instance.id, &instance.component) else {
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
                // Server-originated: no client socket, so no upload ids to claim.
                sender_session: None,
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
    if LV_EVENT_TX.get().is_none() {
        return;
    }
    for (liveview_id, component) in subscribers {
        let Some(tx) = lv_sender_for(&liveview_id, &component) else {
            continue;
        };
        let (response_tx, _response_rx) = oneshot::channel();
        // try_send: a backed-up worker drops this wake rather than block the
        // write path; the next write catches up.
        let _ = tx.try_send(LiveViewEventData {
            liveview_id,
            component,
            event: "live_query_changed".to_string(),
            params: serde_json::json!({}),
            // Server-originated: no client socket, so no upload ids to claim.
            sender_session: None,
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
    use crate::live::view::{live_registry, ServerMessage};

    // Render new HTML
    let new_html = render_component(component, &instance.state)?;
    let old_html = instance.last_html.clone();

    // Compute patch
    let patch = crate::live::diff::compute_patch(&old_html, &new_html);

    // Save the frame back onto the registered instance. `commit` reports a
    // closed socket instead of re-inserting a dead view.
    let liveview_id = instance.id.clone();
    instance.last_html = new_html;
    instance.touch();
    if !live_registry().commit(instance) {
        return Ok(());
    }

    // Send patch to client
    let _ = live_registry().send(
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
///
/// There is deliberately no `--dev` branch: the worker only builds a VM when
/// `!dev_mode` (see the `Option<Vm>` in `worker_loop`), so nothing here is
/// reachable under `--dev` — including `soli test`, which serves with it.
///
/// `SOLI_FAIL_ON_VM_DEMOTION=1` turns a *refusal* into a hard stop so CI cannot
/// ship one silently. Two details matter for that to work:
///
/// * It fires only on [`RuntimeError::EngineFallback`] — the VM saying "I can't
///   compile/run this". Every other error is the handler's own (`throw`, a
///   `RecordNotFound` on a 404 route, a bad arity); those demote too, but they
///   are the app behaving as written, not a coverage regression.
/// * It exits the process rather than panicking. A panic here is swallowed by
///   the per-request `catch_unwind` in `run_caught` into a 500, so the run
///   would look like a flaky request instead of a failure — and it would unwind
///   past the `failed_handlers` insert, making *every* later request to the
///   handler 500 as well.
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
    if !matches!(err, RuntimeError::EngineFallback(..)) {
        return;
    }
    static FAIL: OnceLock<bool> = OnceLock::new();
    let fail = *FAIL.get_or_init(|| {
        std::env::var("SOLI_FAIL_ON_VM_DEMOTION")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    });
    if fail {
        eprintln!(
            "[soli engine] SOLI_FAIL_ON_VM_DEMOTION: handler '{handler}' was refused by the VM: {err}"
        );
        let _ = std::io::Write::flush(&mut std::io::stderr());
        std::process::exit(70);
    }
}

/// Decide whether a failed VM handler may be retried on the tree-walker.
///
/// Normally it may: the VM either refused the code or blew up before doing
/// anything observable, and re-running on the interpreter is what makes the
/// fallback invisible. The exception is a handler that already **committed a
/// transaction** — `Model.transaction { ... }` and the `grouped` block form now
/// run on the VM, so a commit can land and a later `EngineFallback` in the same
/// handler can still demote it. Re-running then repeats the committed writes
/// (the order is created twice). Returns `Some(response)` when the retry must
/// be suppressed; the handler is blacklisted by the caller, so the *next*
/// request runs on the interpreter from a clean slate.
fn no_retry_after_commit(
    handler: &str,
    err: &RuntimeError,
    request_data: &RequestData,
) -> Option<ResponseData> {
    if !crate::interpreter::builtins::model::crud::had_durable_commit() {
        return None;
    }
    // A raised 404/403 is a deliberate outcome, not a failure to re-drive.
    if let Some(resp) = record_not_found_response(err) {
        return Some(resp);
    }
    if let Some(resp) = forbidden_response(err) {
        return Some(resp);
    }
    let request_id = Uuid::new_v4().to_string();
    let error_msg = err.to_string();
    eprintln!(
        "[soli engine] handler '{handler}' failed after committing a transaction on the VM \
         — not retrying on the interpreter (that would repeat the committed writes)"
    );
    error_logging::log_production_error(&request_id, request_data, &error_msg, &[], None);
    Some(panic_response())
}

/// Listen for `SIGTERM`/`SIGINT` and drive the drain sequence.
///
/// On signal: flip to draining (so `/_ready` starts failing and the load
/// balancer takes this instance out of rotation), wait for in-flight connections
/// to finish, then exit `0`. A second signal skips the wait.
///
/// Exits via `std::process::exit`, never `abort`, so `atexit` handlers still run
/// — including the `cargo llvm-cov` profile flush that `main.rs` documents.
#[cfg(unix)]
fn spawn_drain_on_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    // `main.rs` installs a `sigaction` handler that calls `process::exit(0)`
    // immediately. Tokio's signal driver *chains* to whatever handler it finds,
    // so leaving it installed would exit before a single connection drained.
    // Reset to the default disposition first — tokio does not chain to
    // `SIG_DFL`, and the coverage flush is preserved because the drain path
    // below also ends in `process::exit(0)`.
    unsafe {
        use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
        let dfl = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        let _ = sigaction(Signal::SIGTERM, &dfl);
        let _ = sigaction(Signal::SIGINT, &dfl);
    }

    tokio::spawn(async move {
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut int = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(_) => return,
        };

        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }

        if !shutdown::begin_drain() {
            return;
        }

        let grace = shutdown::grace_period();
        let outstanding = shutdown::in_flight();
        if outstanding > 0 {
            println!(
                "Shutting down: draining {} connection(s), up to {}s (SOLI_SHUTDOWN_GRACE_SECS)",
                outstanding,
                grace.as_secs()
            );
        }

        let drained = tokio::select! {
            _ = async {
                while shutdown::in_flight() > 0 {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            } => true,
            _ = tokio::time::sleep(grace) => false,
            _ = async {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int.recv() => {}
                }
            } => false,
        };

        if !drained {
            let stranded = shutdown::in_flight();
            if stranded > 0 {
                eprintln!(
                    "Shutdown: {} connection(s) still open, exiting anyway",
                    stranded
                );
            }
        }

        std::process::exit(0);
    });
}

/// Windows has no `SIGTERM`, and the console-close path gives no window to drain
/// in. Keeping the call site unconditional avoids a `#[cfg]` in the accept loop.
#[cfg(not(unix))]
fn spawn_drain_on_signal() {}

/// The response a worker returns when a handler panics. Deliberately terse and
/// identical in dev and production: the panic payload and backtrace already went
/// to stderr, and echoing them to the client would leak internals.
fn panic_response() -> ResponseData {
    ResponseData {
        status: 500,
        headers: vec![(
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )],
        body: b"Internal Server Error".to_vec(),
    }
}

/// Run a request handler, converting a panic into a `500` instead of losing the
/// worker.
///
/// Kept separate from `dispatch_http_request` (which needs a live `Interpreter`
/// and a wired-up channel) purely so it can be tested directly.
///
/// Note this is a no-op guard unless panics unwind — see the `compile_error!` in
/// `serve::shutdown`, which fails the build if `panic = "abort"` comes back.
fn run_caught<F>(handler: F, method: &str, path: &str) -> ResponseData
where
    F: FnOnce() -> ResponseData,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(handler)) {
        Ok(resp) => resp,
        Err(_) => {
            crate::metrics::Metrics::global()
                .handler_panics_total
                .fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "[soli] handler panicked serving {} {} — returning 500 (worker survived)",
                method, path
            );
            crate::interpreter::builtins::streaming::clear_pending_stream();
            panic_response()
        }
    }
}

/// Run one HTTP request on this worker and reply on its response channel.
///
/// Shared by `worker_loop`'s two dispatch arms — the non-blocking batch drain
/// and the `select`-blocked path — which were otherwise byte-identical.
///
/// Without the guard a panicking handler never sends on `response_tx`, so the
/// hyper side waits the full `RESPONSE_WAIT_TIMEOUT_SECS` before giving up with
/// a `504` — and the worker is gone for the rest of the process's life.
fn dispatch_http_request(
    interpreter: &mut Interpreter,
    vm: &mut Option<crate::vm::Vm>,
    mut data: RequestData,
    dev_mode: bool,
) {
    crate::interpreter::builtins::streaming::clear_pending_stream();

    // Bound how long this request may spend *executing*. The hyper side already
    // gives up at RESPONSE_WAIT_TIMEOUT_SECS with a 504, but the worker stayed
    // in the handler forever, so one request that reached a runaway loop
    // removed a worker permanently. Dropped when the request ends, so the next
    // one starts with a fresh budget.
    let _handler_budget = crate::interpreter::deadline::enter_default();

    let method = data.method.to_string();
    let path = data.path.clone();
    let resp_data = run_caught(
        || handle_request(interpreter, vm, &mut data, dev_mode),
        &method,
        &path,
    );

    match crate::interpreter::builtins::streaming::take_pending_stream() {
        Some(spec) => {
            let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
            let resp = WorkerResponse::Stream {
                status: spec.status,
                headers: spec.headers.clone(),
                rx,
            };
            if let Some(topic) = spec.subscribe_topic.clone() {
                let _ = tx.try_send(b": connected\n\n".to_vec());
                crate::interpreter::builtins::streaming::register_subscriber(&topic, tx);
                let _ = data.response_tx.send(resp);
            } else {
                let id = crate::interpreter::builtins::streaming::register_sender(tx);
                let _ = data.response_tx.send(resp);
                let streamed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::interpreter::builtins::streaming::run_stream_block(
                        interpreter,
                        &spec,
                        id,
                    );
                }));
                if streamed.is_err() {
                    crate::metrics::Metrics::global()
                        .handler_panics_total
                        .fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "[soli] stream block panicked serving {} {} — closing the stream early",
                        method, path
                    );
                }
                crate::interpreter::builtins::streaming::unregister_sender(id);
            }
        }
        None => {
            let _ = data.response_tx.send(WorkerResponse::Buffered(resp_data));
        }
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
    // When true, `params`/`cookies` were just published and middleware did
    // not run, so we skip re-probing the request hash and rewriting those
    // globals. `req` is still published here (it is not set earlier).
    request_globals_fresh: bool,
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

    let helpers_need_context = crate::interpreter::builtins::template::helper_env_loaded();

    // After middleware the request hash may have changed, so republish
    // `params`/`cookies`. On the no-middleware fast path they were just set.
    let (params_value, cookies_value) = if request_globals_fresh && !helpers_need_context {
        (None, None)
    } else {
        let params_value = get_hash_field(&request_hash, "all")
            .unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))));
        let cookies_value = get_hash_field(&request_hash, "cookies")
            .unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))));
        if !request_globals_fresh {
            interpreter
                .global_env()
                .borrow_mut()
                .define_or_update("params", params_value.clone());
            crate::interpreter::taint::mark_request_value(&params_value);
            if let Some(vm_ref) = vm.as_deref_mut() {
                vm_ref
                    .globals
                    .insert("params".to_string(), params_value.clone());
            }
            interpreter
                .global_env()
                .borrow_mut()
                .define_or_update("cookies", cookies_value.clone());
            crate::interpreter::taint::mark_request_value(&cookies_value);
            if let Some(vm_ref) = vm.as_deref_mut() {
                vm_ref
                    .globals
                    .insert("cookies".to_string(), cookies_value.clone());
            }
        }
        (Some(params_value), Some(cookies_value))
    };

    // Expose the full request hash as a global `req` so actions can omit the
    // `(req)` parameter when they don't need to destructure the request.
    interpreter
        .global_env()
        .borrow_mut()
        .define_or_update("req", request_hash.clone());
    // Remember that everything reachable from the request came from a client.
    // `.where` consults this before letting a value act as an operator map:
    // `where({"api_token": params["token"]})` must compare, not accept
    // `{"ne": null}` and drop the predicate.
    crate::interpreter::taint::mark_request_value(&request_hash);
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
    if helpers_need_context {
        let params_value = params_value.unwrap_or_else(|| {
            get_hash_field(&request_hash, "all")
                .unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))))
        });
        let cookies_value = cookies_value.unwrap_or_else(|| {
            get_hash_field(&request_hash, "cookies")
                .unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default()))))
        });
        let session_value = get_hash_field(&request_hash, "session").unwrap_or(Value::Null);
        let headers_value = get_hash_field(&request_hash, "headers").unwrap_or(Value::Null);
        crate::interpreter::builtins::template::set_helper_request_context(
            &request_hash,
            &params_value,
            &session_value,
            &cookies_value,
            &headers_value,
        );
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

    // Handlers may be declared as either `fn name(req)` or, more idiomatically
    // for read-only actions, `fn name` with no parameters (they read `req`
    // via the request-time global). Match the call arity to the declaration
    // so the latter form doesn't trip the runtime arity check.
    let handler_wants_request = match &handler_result {
        Ok(Value::Function(f)) => f.full_arity() > 0,
        _ => true,
    };

    // Reserved for handlers that must never run on the VM. Job code no longer
    // arrives over HTTP (the engine runs it on the pool's interpreters), so no
    // handler currently needs the override.
    let force_interpreter = false;

    // Try VM execution in production mode for function-based handlers
    if let Some(ref mut vm) = vm {
        if !force_interpreter && !vm.failed_handlers.contains(handler_name) {
            if let Ok(ref handler_value) = handler_result {
                // Arm the durable-commit tripwire for this attempt: if the VM
                // run commits a transaction and *then* fails, the fallback
                // below must not re-run the handler.
                crate::interpreter::builtins::model::crud::clear_durable_commit();
                let call_result = if handler_wants_request {
                    vm.call_value_direct_one(
                        handler_value.clone(),
                        request_hash.clone(),
                        Span::default(),
                    )
                } else {
                    vm.call_value_direct(handler_value.clone(), &[], Span::default())
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
                        if let Some(resp) = no_retry_after_commit(handler_name, &err, request_data)
                        {
                            return resp;
                        }
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
                    error_response::handler_failure(
                        dev_mode,
                        interpreter,
                        request_data,
                        &e.to_string(),
                        &stack_trace,
                        env_json.as_deref(),
                        &request_id,
                        e.is_breakpoint(),
                    )
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
            error_response::handler_failure(
                dev_mode,
                interpreter,
                request_data,
                &e.to_string(),
                &stack_trace,
                captured_env.as_deref(),
                &request_id,
                // `e` is the `String` `resolve_handler` returns; a breakpoint
                // cannot arrive through it.
                false,
            )
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
        let set = CONTROLLER_REGISTRY.read(|registry| {
            let mut set = std::collections::HashSet::new();
            for (key, info) in registry.all().iter().map(|i| (&i.class_name, i)) {
                if !info.before_actions.is_empty() || !info.after_actions.is_empty() {
                    set.insert(key.clone());
                }
            }
            set
        });
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
        CONTROLLER_REGISTRY.read(|registry| registry.get(controller_key).cloned())
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
            return Some(error_response::handler_failure(
                dev_mode,
                interpreter,
                request_data,
                &e.to_string(),
                &stack_trace,
                Some(&env_json),
                &request_id,
                // `e` is the `String` `create_controller_instance` returns; a
                // breakpoint cannot arrive through it.
                false,
            ));
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
                error_response::handler_failure(
                    dev_mode,
                    interpreter,
                    request_data,
                    &e.to_string(),
                    &stack_trace,
                    env_json.as_deref(),
                    &request_id,
                    e.is_breakpoint(),
                )
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

thread_local! {
    /// Scratch buffer for the "Class#method" key probed against
    /// `Vm::failed_handlers` in [`call_class_method`].
    static HANDLER_KEY_BUF: RefCell<String> = RefCell::new(String::with_capacity(64));
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
            // Probe the demotion set without allocating the "Class#method"
            // key on every request: the set is empty on a healthy worker, and
            // otherwise the key is built into a reused per-thread buffer.
            let demoted = !vm.failed_handlers.is_empty()
                && HANDLER_KEY_BUF.with(|buf| {
                    let mut buf = buf.borrow_mut();
                    buf.clear();
                    buf.push_str(&class.name);
                    buf.push('#');
                    buf.push_str(method_name);
                    vm.failed_handlers.contains(buf.as_str())
                });
            // Only a method that takes `(req)` runs on the VM. A zero-parameter
            // action (`def index`) reads the request through the `req` global
            // and stays on the tree-walker: 2.4.0 sent it to the VM, and
            // actions that had always worked there failed in production.
            if !demoted && !method.params.is_empty() {
                crate::interpreter::builtins::model::crud::clear_durable_commit();
                let action_arg = Some(request_hash.clone());
                match vm.call_method_bound(&method, instance.clone(), action_arg, Span::default()) {
                    Ok(result) => {
                        vm.reset();
                        return Ok(result);
                    }
                    Err(err) => {
                        let handler_key = format!("{}#{}", class.name, method_name);
                        record_vm_demotion(&handler_key, &err);
                        vm.failed_handlers.insert(handler_key);
                        vm.reset();
                        // Committed already — re-running the action on the
                        // tree-walker would repeat the committed writes. See
                        // `no_retry_after_commit`.
                        if crate::interpreter::builtins::model::crud::had_durable_commit() {
                            return Err(err);
                        }
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
                // If error already has env (breakpoint or WithEnv), keep it; otherwise capture.
                //
                // A `Thrown` is deliberately NOT exempt here, unlike at the two
                // interpreter sites that preserve a throw in flight. This is the
                // controller-action boundary: an error arriving here escaped every
                // `catch` in user code, so nothing downstream will unwrap the value,
                // and flattening it to its message is what the dev error page wants.
                // Exempting it would cost those locals and buy nothing.
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

/// Get a field from a hash value. Uses a borrowed `StrKey` so a serve-path
/// probe (`req["all"]`, `req["cookies"]`, …) does not allocate a `String`.
fn get_hash_field(hash: &Value, field: &str) -> Option<Value> {
    match hash {
        Value::Hash(fields) => fields.borrow().get(&StrKey(field)).cloned(),
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
    Some(error_response::page(404, &err.record_not_found_message()?))
}

/// If the error came from an authorization failure (the `forbidden()` builtin
/// or the Policy layer's `authorize()` helper), build a 403 response using the
/// standard production error-page pipeline (so apps can ship a custom
/// `app/views/errors/403.html.slv`). Returns None otherwise.
fn forbidden_response(err: &RuntimeError) -> Option<ResponseData> {
    Some(error_response::page(403, &err.forbidden_message()?))
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
                data_pairs.insert(HashKey::String(k.clone()), v.clone());
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

/// Handle a single request (called on interpreter thread).
///
/// A cascade of named stages: forget the previous visitor
/// ([`request_scope::reset_worker_thread_locals`]), answer what the framework
/// owns ([`builtin_endpoints::handle`]), install this request's session and
/// locale ([`request_scope::install`]), match a route ([`route_match::resolve`]),
/// build the request the handler sees ([`request_input::build`]), run
/// middleware, dispatch, and finish ([`finalize::finish`]).
///
/// What stays here is the order they run in and the short-circuits between
/// them — several of which are load-bearing, and say so where they sit.
fn handle_request(
    interpreter: &mut Interpreter,
    vm: &mut Option<crate::vm::Vm>,
    data: &mut RequestData,
    dev_mode: bool,
) -> ResponseData {
    request_scope::reset_worker_thread_locals(dev_mode);

    // File mode: this request resolved to a `.slv`/`.erb` file in the served
    // folder. There is no route table, no session and no CSRF gate to run —
    // render the template and return.
    if let Some(relative) = data.file_template.clone() {
        return files::render_template(data, &relative);
    }

    // In --dev, snapshot the raw request now — before headers/query/body are
    // moved out downstream — so the dev bar's replay button can re-dispatch it
    // faithfully. Stashed by `finalize::finish` under the same request id as
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

    // The endpoints the framework answers itself. `/up` in particular has to
    // answer here, before any session or cookie work, so the readiness probe
    // never creates a session or touches the store — see the module.
    if let Some(resp) = builtin_endpoints::handle(&data.method, &data.path) {
        return resp;
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

    // OpenTelemetry: parse inbound W3C `traceparent` (if any) and mint a
    // request-scoped TraceContext. Cheap early-out when SOLI_OTEL / OTLP
    // endpoint is unset.
    let trace_ctx = otel::begin_request(
        data.method.as_ref(),
        data.path.as_str(),
        header_str(&data.headers, "traceparent"),
    );

    // Anchor the span log to this request's start so every span's
    // `start_us` / `end_us` is encoded as microseconds-since-request-start.
    // Also open the synthetic root request span so the flamegraph has a
    // single top-level rectangle (e.g. `GET /docs/getting_started`) and
    // every other span — middleware, action, view, db — nests beneath it
    // instead of appearing as detached sibling roots. The root is closed
    // explicitly inside `finalize::finish` right before the snapshot, so
    // it actually ends up in the recorded log.
    //
    // Same tree is used for OTLP export when tracing is on (dev or prod).
    let span_started = dev_started.or_else(|| {
        trace_ctx
            .as_ref()
            .map(|c| c.instant_start)
            .or_else(|| otel::enabled().then(Instant::now))
    });
    if let Some(t) = span_started {
        span_log::begin_request(t);
        span_log::open_request_root(format!("{} {}", data.method, data.path));
    }

    // Network session stores (solidb, solikv) load the session document once
    // per request instead of once per read: resolving the cookie, the stored
    // locale, the CSRF token and every `session_get` share that load. Bound
    // before `install`, which does the first read, and dropped last.
    let _session_memo =
        crate::interpreter::builtins::session_request_cache::RequestSessionCache::begin();
    let scope = request_scope::install(data);
    // Bound here and not inside `install`: a guard built and dropped in there
    // would restore the default locale before the handler ever ran.
    let _locale_guard = LocaleGuard;

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
        if let Err(reason) = verify_csrf_token(data, &data.method, &data.path) {
            set_current_session_id(None);
            return error_response::production(
                403,
                &data.method,
                &data.path,
                "CSRF verification failed. Reload the page and resubmit the form.",
                Some(&format!("CSRF: {}", reason)),
            );
        }
    }

    // LiveView file bytes travel over HTTP (WS is capped at 1 MiB). The
    // client then sends only the returned id over the socket.
    if data.method == "POST" && data.path == "/live/upload" {
        return handle_live_upload(data);
    }

    // Resolved before the route lookup because a 404 emits a session cookie
    // too, and has to know whether it is `Secure`.
    let scheme = request_input::scheme(data);

    let matched = match route_match::resolve(
        &data.method,
        &data.path,
        data,
        &scope,
        scheme.cookie_secure,
        start_time,
    ) {
        Ok(matched) => matched,
        Err(resp) => return resp,
    };
    let route_match::Matched {
        handler_name,
        scoped_middleware,
        params: matched_params,
    } = matched;

    // Record the matched route (post wildcard-expansion) for the dev bar's
    // "requests" panel + the `X-Soli-Route` header. Early-outs on the gate in
    // production. 404s never reach here, so a miss leaves the route unset.
    route_log::record(&handler_name);

    let request_scope::Scope {
        cookie_pairs,
        cookie_session_id,
        ..
    } = scope;
    let input = request_input::build(interpreter, vm, data, matched_params, cookie_pairs, &scheme);
    let mut request_hash = input.request_hash;

    // `headers` and `query` are gone from `data` now; `method` and `path` are
    // untouched and outlive the rest of the request.
    let method = &data.method;
    let path = &data.path;

    let finalizer = finalize::Finalizer {
        dev_mode,
        dev_started,
        span_started,
        trace_ctx,
        start_time,
        queue_ms,
        log_requests,
        log_channels,
        cookie_session_id,
        cookie_secure: scheme.cookie_secure,
        is_htmx: input.is_htmx,
        captured_raw,
    };

    // Fast path: no middleware at all (avoid cloning middleware list if empty)
    if scoped_middleware.is_empty() && !has_middleware() {
        return finalize::finish(
            &finalizer,
            method,
            path,
            call_handler(
                interpreter,
                vm.as_mut(),
                &handler_name,
                request_hash,
                dev_mode,
                data,
                true,
            ),
        );
    }

    // Only clone middleware list if we need it
    let global_middleware = get_middleware();

    // Execute scoped (route-specific) middleware
    for mw in &scoped_middleware {
        match middleware::run(interpreter, data, mw.clone(), None, request_hash, dev_mode) {
            middleware::Step::Continue(modified_request) => request_hash = modified_request,
            middleware::Step::Halt(response) => {
                return finalize::finish(&finalizer, method, path, response)
            }
        }
    }

    // Execute global middleware
    let has_scoped_middleware = !scoped_middleware.is_empty();
    for mw in global_middleware.iter() {
        if has_scoped_middleware && mw.global_only {
            continue;
        }
        if mw.scope_only {
            continue;
        }

        match middleware::run(
            interpreter,
            data,
            mw.handler.clone(),
            Some(mw.name.as_str()),
            request_hash,
            dev_mode,
        ) {
            middleware::Step::Continue(modified_request) => request_hash = modified_request,
            middleware::Step::Halt(response) => {
                return finalize::finish(&finalizer, method, path, response)
            }
        }
    }

    // Call the route handler
    finalize::finish(
        &finalizer,
        method,
        path,
        call_handler(
            interpreter,
            vm.as_mut(),
            &handler_name,
            request_hash,
            dev_mode,
            data,
            false,
        ),
    )
}

/// May this request read `/_metrics`?
///
/// With `SOLI_METRICS_TOKEN` set, only a matching bearer token (compared in
/// constant time). Without it, only a peer on the loopback or a private range —
/// where a scraper actually runs — so the endpoint stops being readable from
/// the public internet by default.
///
/// The peer rule cannot tell a scraper from a reverse proxy on the same host:
/// behind nginx on loopback, *every* public client arrives from `127.0.0.1`.
/// So without a token the request is also refused when it carries a
/// forwarding header, or when the app trusts a proxy at all — in both cases
/// the TCP peer is a hop, not the caller. Such deployments set
/// `SOLI_METRICS_TOKEN`.
fn metrics_request_allowed(headers: &hyper::HeaderMap, peer: std::net::IpAddr) -> bool {
    if let Ok(expected) = std::env::var("SOLI_METRICS_TOKEN") {
        let expected = expected.trim();
        if !expected.is_empty() {
            let presented = headers
                .get(hyper::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .unwrap_or("")
                .trim();
            return crate::interpreter::builtins::crypto::do_secure_compare(presented, expected);
        }
    }
    if metrics_request_is_proxied(headers) {
        return false;
    }
    if crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED.read(|on| *on) {
        return false;
    }
    is_private_metrics_peer(peer)
}

/// Does this request name a forwarding hop? Any of these means the TCP peer is
/// a proxy relaying someone else, so its private address proves nothing.
fn metrics_request_is_proxied(headers: &hyper::HeaderMap) -> bool {
    ["x-forwarded-for", "x-real-ip", "forwarded"]
        .iter()
        .any(|name| headers.contains_key(*name))
}

/// Loopback, RFC1918, CGNAT, link-local or IPv6 unique-local: the addresses a
/// monitoring scraper realistically comes from.
fn is_private_metrics_peer(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || {
                // 100.64.0.0/10, the carrier-grade NAT range many container
                // networks use.
                let o = v4.octets();
                o[0] == 100 && (64..128).contains(&o[1])
            }
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_metrics_peer(std::net::IpAddr::V4(v4));
            }
            v6.is_loopback()
                // fc00::/7 unique-local, fe80::/10 link-local.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

#[cfg(test)]
mod metrics_gate_tests {
    use super::{metrics_request_allowed, metrics_request_is_proxied};

    /// Behind a same-host reverse proxy every public client arrives from
    /// loopback; a forwarding header is the tell that the peer is a hop.
    #[test]
    fn a_proxied_request_from_loopback_is_refused_without_a_token() {
        if std::env::var("SOLI_METRICS_TOKEN").is_ok_and(|t| !t.trim().is_empty()) {
            return;
        }
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        for name in ["x-forwarded-for", "x-real-ip", "forwarded"] {
            let mut headers = hyper::HeaderMap::new();
            headers.insert(name, "203.0.113.9".parse().unwrap());
            assert!(metrics_request_is_proxied(&headers), "{name}");
            assert!(!metrics_request_allowed(&headers, loopback), "{name}");
        }
        assert!(!metrics_request_is_proxied(&hyper::HeaderMap::new()));
    }
}

pub(crate) fn html_ok(html: String) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(full(Bytes::from(html)))
        .unwrap()
}

#[allow(dead_code)]
#[cfg(test)]
mod tests {
    use super::*;

    /// A panicking handler must become a `500` rather than take the worker with
    /// it, and the panic must be counted so `/_metrics` can surface it.
    #[test]
    fn a_panicking_handler_becomes_500_and_is_counted() {
        let before = crate::metrics::Metrics::global()
            .handler_panics_total
            .load(Ordering::Relaxed);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let resp = run_caught(|| panic!("boom from a handler"), "GET", "/explode");
        std::panic::set_hook(previous);
        assert_eq!(resp.status, 500, "a panicking handler must yield a 500");
        assert_eq!(resp.body, b"Internal Server Error".to_vec());
        assert!(
            crate::metrics::Metrics::global()
                .handler_panics_total
                .load(Ordering::Relaxed)
                > before,
            "the panic must be counted for /_metrics"
        );
    }

    /// One panicking request must not stop the ones after it.
    #[test]
    fn a_panic_does_not_stop_subsequent_requests() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut statuses = Vec::new();
        for i in 0..5 {
            let resp = run_caught(
                || {
                    if i == 2 {
                        panic!("boom on the third request");
                    }
                    ResponseData {
                        status: 200,
                        headers: vec![],
                        body: format!("ok {}", i).into_bytes(),
                    }
                },
                "GET",
                "/mixed",
            );
            statuses.push(resp.status);
        }
        std::panic::set_hook(previous);
        assert_eq!(
            statuses,
            vec![200, 200, 500, 200, 200],
            "only the panicking request may fail; the loop must keep serving"
        );
    }

    /// `catch_unwind` is only a guard if panics actually unwind.
    #[test]
    fn panics_unwind_in_this_build_profile() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let caught = std::panic::catch_unwind(|| panic!("must be catchable"));
        std::panic::set_hook(previous);
        assert!(caught.is_err(), "panics must unwind, not abort");
    }

    use std::sync::Mutex;

    /// Serialises every test that reads or writes a `SOLI_*` env var.
    /// `dev_routes::tests` shares it: both suites drive `SOLI_DEV_REPL_*`,
    /// and two locks would not have serialised against each other.
    pub(super) static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn preview_back_link_only_when_standalone() {
        // Framed by a gallery card — no chrome, or every card would carry it.
        assert!(dev_catalog::preview_back_link(Some("framed=1")).is_empty());
        // Opened directly (the catalog's title link, or a pasted URL).
        assert!(dev_catalog::preview_back_link(None).contains("href=\"/\""));
        assert!(dev_catalog::preview_back_link(Some("")).contains("href=\"/\""));
        // A `framed` that isn't the flag doesn't suppress it.
        assert!(dev_catalog::preview_back_link(Some("framed=0")).contains("href=\"/\""));
        assert!(dev_catalog::preview_back_link(Some("x=framed=1")).contains("href=\"/\""));
        // The flag is still found beside other params.
        assert!(dev_catalog::preview_back_link(Some("a=1&framed=1")).is_empty());
    }

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
    fn unwrap_bare_state_shape_returns_whole_hash() {
        let json = serde_json::json!({ "count": 7, "name": "alice" });
        let got = unwrap_handler_return(json.clone());
        assert_eq!(got.state, Some(json));
        assert_eq!(got.tick, None);
        assert!(got.stream.is_none());
        assert!(got.redirect.is_none());
        assert!(got.js.is_none());
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
        let got = unwrap_handler_return(json);
        assert_eq!(got.state, Some(serde_json::json!({ "count": 3 })));
        assert_eq!(got.tick, Some(50));
    }

    #[test]
    fn unwrap_wrapped_shape_without_tick_interval_returns_none() {
        let json = serde_json::json!({ "state": { "x": 1 } });
        let got = unwrap_handler_return(json);
        assert_eq!(got.state, Some(serde_json::json!({ "x": 1 })));
        assert_eq!(got.tick, None);
    }

    #[test]
    fn unwrap_treats_non_object_state_key_as_bare_shape() {
        // A user setting `"state": 42` doesn't look like the wrapped form — we
        // treat the whole hash as the new state to avoid silently dropping it.
        let json = serde_json::json!({ "state": 42, "tick_interval": 50 });
        let got = unwrap_handler_return(json.clone());
        assert_eq!(got.state, Some(json));
        assert_eq!(got.tick, None);
    }

    #[test]
    fn unwrap_close_is_a_close_and_keeps_the_state() {
        let got = unwrap_handler_return(serde_json::json!({ "close": "not you" }));
        assert_eq!(got.close.as_deref(), Some("not you"));
        assert_eq!(got.state, None, "a close is not a state");
        let got = unwrap_handler_return(serde_json::json!({ "close": true }));
        assert_eq!(got.close.as_deref(), Some("closed"));
        // `false` and `null` are not closes: the hash is a bare state.
        let got = unwrap_handler_return(serde_json::json!({ "close": false, "n": 1 }));
        assert_eq!(got.close, None);
        assert_eq!(
            got.state,
            Some(serde_json::json!({ "close": false, "n": 1 }))
        );
    }

    #[test]
    fn unwrap_wrapped_shape_with_zero_tick_interval_returns_zero() {
        let json = serde_json::json!({
            "state": { "x": 1 },
            "tick_interval": 0,
        });
        let got = unwrap_handler_return(json);
        assert_eq!(got.tick, Some(0));
    }

    #[test]
    fn unwrap_stream_only_keeps_state_and_extracts_ops() {
        // A stream-only emission (no `state` key) must not wipe the state.
        let json = serde_json::json!({
            "stream": { "container": "posts", "ops": [
                { "op": "append", "id": "post-7", "html": "<li>hi</li>" }
            ] }
        });
        let got = unwrap_handler_return(json);
        assert_eq!(got.state, None); // keep current state
        assert_eq!(got.tick, None);
        assert!(got.stream.is_some());
    }

    #[test]
    fn unwrap_extracts_redirect_and_js_commands() {
        let json = serde_json::json!({
            "state": { "ok": true },
            "redirect": "/done",
            "js": [{ "op": "add_class", "to": "#flash", "class": "show" }]
        });
        let got = unwrap_handler_return(json);
        assert_eq!(got.state, Some(serde_json::json!({ "ok": true })));
        assert_eq!(got.redirect.as_deref(), Some("/done"));
        let cmds = got.js.expect("js");
        assert_eq!(cmds[0]["op"], "add_class");
        assert_eq!(cmds[0]["to"], "#flash");
    }

    #[test]
    fn unwrap_extracts_patch_url() {
        let got = unwrap_handler_return(serde_json::json!({
            "state": { "tab": "comments" },
            "patch": "/posts/1?tab=comments"
        }));
        assert_eq!(got.state, Some(serde_json::json!({ "tab": "comments" })));
        assert_eq!(
            got.patch,
            Some(("/posts/1?tab=comments".to_string(), false))
        );

        let replace = unwrap_handler_return(serde_json::json!({
            "patch": { "url": "/x", "replace": true }
        }));
        assert_eq!(replace.patch, Some(("/x".to_string(), true)));
        assert!(replace.state.is_none());
    }

    #[test]
    fn unwrap_extracts_live_socket_url() {
        let got = unwrap_handler_return(serde_json::json!({
            "state": { "ok": true },
            "live": "/live/socket/about"
        }));
        assert_eq!(got.live.as_deref(), Some("/live/socket/about"));
        assert_eq!(got.state, Some(serde_json::json!({ "ok": true })));
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
        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED.write(|on| *on = true);

        let mut headers = hyper::HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:5011".parse().unwrap());
        headers.insert("x-forwarded-host", "app.test".parse().unwrap());
        headers.insert(header::ORIGIN, "https://app.test".parse().unwrap());

        assert!(websocket_origin_allowed(&headers));

        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED.write(|on| *on = prev_trust);
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
    fn csrf_allows_framework_endpoints() {
        // Framework endpoints are machine-to-machine, not browser form
        // targets, so we don't expect Origin/Referer on them.
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com")]);
        assert!(check_csrf_origin(&h, "POST", "/__coverage__").is_ok());
        assert!(check_csrf_origin(&h, "POST", "/__solidev/replay/abc").is_ok());
        assert!(check_csrf_origin(&h, "POST", "/_metrics").is_ok());

        // The probes answer GET/HEAD only: a cross-origin POST to one of
        // their paths can only be an application route, so it is not exempt.
        let cross = make_headers(&[("host", "example.com"), ("origin", "https://evil.test")]);
        assert!(check_csrf_origin(&cross, "POST", "/_metrics").is_err());
        assert!(check_csrf_origin(&cross, "POST", "/__solidev/replay/abc").is_ok());
    }

    #[test]
    fn csrf_still_guards_application_routes_under_underscore() {
        // The exemption used to be a bare `starts_with("/_")`, which handed
        // every app route in that namespace a free pass. `/_internal/probe`
        // is an ordinary route: it keeps the Origin gate.
        let _g = csrf_lock();
        let h = make_headers(&[("host", "example.com"), ("origin", "https://evil.test")]);
        assert!(check_csrf_origin(&h, "POST", "/_internal/probe").is_err());
        assert!(check_csrf_origin(&h, "POST", "/_admin/users").is_err());
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
        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED.write(|on| *on = false);
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
        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED.write(|on| *on = true);
        let h = make_headers(&[
            ("host", "example.com"),
            ("x-forwarded-host", "app.example.com"),
        ]);
        assert_eq!(
            websocket_request_authority(&h),
            Some("app.example.com".to_string())
        );

        // Restore.
        crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED.write(|on| *on = prev_trust);
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
            body_reservation: None,
            path: "/posts".to_string(),
            query: Vec::new(),
            headers: make_headers(headers),
            body: body.to_string(),
            multipart_form,
            multipart_files: None,
            peer_ip: "127.0.0.1".to_string(),
            enqueued_at: None,
            replay: false,
            file_template: None,
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

        // Framework endpoints are exempt.
        let data = make_request_data(&[("x-csrf-token", "wrong")], "", None);
        assert!(verify_csrf_token(&data, "POST", "/__solidev/replay/abc").is_ok());
        assert!(verify_csrf_token(&data, "POST", "/__soli/inbox/clear").is_ok());
        // The probes answer GET/HEAD only, so a POST to one of their paths is
        // an application route (`post("/:slug")`) and keeps token checking.
        assert!(verify_csrf_token(&data, "POST", "/_metrics").is_err());

        // An application route in the same namespace is not: a bad token is a
        // 403 there like anywhere else. The old blanket `/_` prefix let
        // `/_internal/probe` through with a token it had made up.
        let data = make_request_data(&[("x-csrf-token", "wrong")], "", None);
        assert!(verify_csrf_token(&data, "POST", "/_internal/probe").is_err());

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
