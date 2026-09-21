//! The `--dev` diagnostics bank, and the peer gate in front of it.
//!
//! The REPL and the source reader, the request inspector and its replay, the
//! component and mailer catalogues, and the sent-mail inbox: eleven routes
//! that exist only under `--dev` and that all read or run something the
//! application was not meant to expose. Lifted out of `handle_hyper_request`,
//! where they were eleven sequential `if`s with seven repeats of
//! `if method == "GET"`.
//!
//! The gate in front of them is the point of the module. `--dev` binds
//! `0.0.0.0` like any other run, so without it the inspector, the inbox and
//! the catalogues are readable by anyone on the same network; it stays ahead
//! of every arm below. `/__dev/repl` and `/__dev/source` are not under it
//! because they carry a stricter one of their own — a shared token, checked
//! in constant time, on top of the same loopback rule.
//!
//! `dispatch` returns the request itself on a miss rather than an `Option`,
//! because two of its arms consume `req` and splitting those two routes out
//! would have put them in front of the gate instead of behind it.

use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use tokio::sync::oneshot;
use uuid::Uuid;

use super::dev_catalog::{
    handle_component_catalog, handle_component_preview, handle_mailer_catalog,
    handle_mailer_preview,
};
use super::pipeline;
use super::repl_session::REPL_STORE;
use super::{
    add_header_checked, dev_bar, dev_inbox, dev_store, full, json, load_models, prod_log,
    server_constants, RequestData, ResponseBody, WorkerResponse, WorkerSender,
};

/// The `--dev` diagnostics, or the request back untouched when the path is
/// not one of theirs (or the server is not in `--dev` at all).
///
/// The `Err` is a whole `Request`, which `clippy::result_large_err` dislikes
/// on the assumption that an error is propagated up a `?` chain. This one is
/// not: it is returned to the immediate caller, on the same stack frame, once,
/// and boxing it would put an allocation on the path of *every* request that
/// is not a dev route in order to add an indirection nobody follows.
#[allow(clippy::result_large_err)]
pub(super) async fn dispatch(
    req: Request<Incoming>,
    method: &str,
    path: &str,
    peer_addr: SocketAddr,
    request_tx: &WorkerSender,
    dev_mode: bool,
) -> Result<Response<ResponseBody>, Request<Incoming>> {
    if !dev_mode {
        return Err(req);
    }

    // These are diagnostics, not application routes: the request inspector
    // replays another visitor's request with their cookies, the mail inbox
    // lists every message the app sent (password-reset links with live
    // tokens included), and the component/mailer catalogues enumerate the
    // app's internals. `--dev` binds 0.0.0.0 like any other run, so all of
    // that was readable by anyone on the same network — only `/__dev/repl`
    // and `/__dev/source` were peer-gated. Hold them to the same rule.
    let dev_diagnostics_path = path.starts_with("/__solidev/")
        || path == "/__soli/components"
        || path.starts_with("/__soli/components/")
        || path == "/__soli/mailers"
        || path.starts_with("/__soli/mailers/")
        || path == "/__soli/inbox"
        || path.starts_with("/__soli/inbox/");
    if dev_diagnostics_path && !is_trusted_dev_peer(peer_addr.ip()) {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(full(Bytes::from("Not Found")))
            .unwrap());
    }

    match (method, path) {
        // REPL endpoint
        ("POST", "/__dev/repl") => Ok(handle_dev_repl(req, peer_addr).await),
        // Source code endpoint
        ("GET", "/__dev/source") => Ok(handle_dev_source(req, peer_addr).await),
        // Component preview catalog (Lookbook-style), dev-only.
        ("GET", "/__soli/components") => Ok(handle_component_catalog()),
        // Mailer preview gallery, dev-only.
        ("GET", "/__soli/mailers") => Ok(handle_mailer_catalog()),
        // Sent-mail inbox (MailCatcher-style), dev-only: what the app actually
        // delivered, including mail captured because no SMTP host is set.
        ("GET", "/__soli/inbox") => Ok(dev_inbox::handle_index(req.uri().query())),
        ("POST", "/__soli/inbox/clear") => Ok(dev_inbox::handle_clear()),

        // The rest are addressed by prefix. None of these arms consumes `req`,
        // so a miss still hands it back.
        ("GET", _) => {
            if let Some(id) = path.strip_prefix("/__solidev/request/") {
                Ok(request_snapshot(id))
            } else if let Some(name) = path.strip_prefix("/__soli/components/") {
                Ok(handle_component_preview(name, req.uri().query()))
            } else if let Some(rel) = path.strip_prefix("/__soli/mailers/") {
                Ok(handle_mailer_preview(rel, req.uri().query()))
            } else if let Some(rest) = path.strip_prefix("/__soli/inbox/") {
                Ok(dev_inbox::handle_message(rest))
            } else {
                Err(req)
            }
        }
        // Replay a captured request server-side to reproduce a bug. Re-dispatches
        // the stored raw request through the real worker path (fresh request id,
        // handler re-runs). The `/_`-prefixed path is exempt from the origin gate
        // in `handle_hyper_request`, and the replay flag bypasses the worker's
        // per-form CSRF token.
        ("POST", _) => match path.strip_prefix("/__solidev/replay/") {
            Some(id) => Ok(handle_replay(id, request_tx).await),
            None => Err(req),
        },
        _ => Err(req),
    }
}

/// Per-request dev snapshot: the dev bar's requests panel fetches this to
/// re-render a listed request's panels (db / http / kv / flame). Reads the
/// process-wide store (populated on the worker thread at finalize), so it's
/// safe to serve straight from the async handler. Dev-only; the store is empty
/// in production so this never leaks anything.
fn request_snapshot(id: &str) -> Response<ResponseBody> {
    match dev_store::get(id) {
        Some(ctx) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(full(Bytes::from(dev_bar::render_for_inspect(&ctx))))
            .unwrap(),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(full(Bytes::from("unknown or expired request id")))
            .unwrap(),
    }
}

/// Handle REPL execution for dev mode.
async fn handle_dev_repl(req: Request<Incoming>, peer_addr: SocketAddr) -> Response<ResponseBody> {
    if !is_authorized_dev_repl_request(req.headers(), peer_addr) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(
                r#"{"error": "Forbidden dev REPL request"}"#,
            )))
            .unwrap();
    }

    let max_body = crate::interpreter::builtins::body_limit::get_max_body_size();
    let body = match BodyExt::collect(Limited::new(req.into_body(), max_body)).await {
        Ok(b) => b.to_bytes(),
        Err(_) => {
            return Response::builder()
                .status(StatusCode::PAYLOAD_TOO_LARGE)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(r#"{"error": "Request body too large"}"#)))
                .unwrap();
        }
    };
    let body_str = String::from_utf8_lossy(&body);

    // Parse JSON body
    let json: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(json) => json,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(r#"{"error": "Invalid JSON body"}"#)))
                .unwrap();
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

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(full(Bytes::from(response_json.to_string())))
        .unwrap()
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
) -> Response<ResponseBody> {
    if !is_authorized_dev_repl_request(req.headers(), peer_addr) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(
                r#"{"error": "Forbidden dev source request"}"#,
            )))
            .unwrap();
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
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(r#"{"error": "Missing file parameter"}"#)))
            .unwrap();
    }

    // Reject absolute paths - security measure
    if std::path::Path::new(&file).is_absolute() {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(
                r#"{"error": "Absolute paths not allowed"}"#,
            )))
            .unwrap();
    }

    // Try to read the file - resolve relative to app root
    let app_root = crate::serve::tenant::app_root();
    let joined = app_root.join(&file);

    // Canonicalize and verify the path is within app_root
    let canonical_path = match std::fs::canonicalize(&joined) {
        Ok(p) => p,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(r#"{"error": "File not found"}"#)))
                .unwrap();
        }
    };

    let canonical_root = match std::fs::canonicalize(&app_root) {
        Ok(r) => r,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(
                    r#"{"error": "Could not determine app root"}"#,
                )))
                .unwrap();
        }
    };

    // SEC-010: use `Path::starts_with` (segment-aware), not the string
    // form. Plain string `starts_with` would treat
    // `/home/me/app-secrets/x` as inside `/home/me/app` because the
    // prefix matches character-by-character — exactly the leak this
    // task was filed for. `static_files` already uses this idiom.
    if !canonical_path.starts_with(&canonical_root) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(
                r#"{"error": "Path outside app directory"}"#,
            )))
            .unwrap();
    }

    if !canonical_path.is_file() {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(r#"{"error": "Not a file"}"#)))
            .unwrap();
    }

    let content = match std::fs::read_to_string(&canonical_path) {
        Ok(c) => c,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(full(Bytes::from(r#"{"error": "Could not read file"}"#)))
                .unwrap();
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

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(full(Bytes::from(response.to_string())))
        .unwrap()
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
        // Dev-tool replay of a stashed request: the body is already resident
        // in the dev store, so it claims no new budget.
        body_reservation: None,
        // Multipart re-parsing is punted for v1: a replayed multipart POST
        // carries the raw body but no pre-parsed fields/files.
        multipart_form: None,
        multipart_files: None,
        peer_ip: raw.peer_ip.clone(),
        enqueued_at: prod_log::channels().any().then(std::time::Instant::now),
        replay: true,
        file_template: None,
        response_tx,
    };

    // The same non-blocking send the main dispatch uses — this used to be a
    // hand-copied duplicate of it, under a comment saying so.
    if let Err(busy) = pipeline::enqueue(request_tx, request_data).await {
        return *busy;
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
        let app_root = crate::serve::tenant::app_root();
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

static DEV_REPL_AUTH_TOKEN: OnceLock<String> = OnceLock::new();

pub(super) fn dev_repl_auth_token() -> &'static str {
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

pub(super) fn dev_repl_allows_remote() -> bool {
    std::env::var("SOLI_DEV_REPL_ALLOW_REMOTE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

pub(super) fn dev_repl_secret_set() -> bool {
    std::env::var("SOLI_DEV_REPL_SECRET")
        .ok()
        .is_some_and(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Shared with `serve::tests`: both suites drive `SOLI_DEV_REPL_*`, so one
    // lock has to cover them both.
    use crate::serve::tests::ENV_TEST_LOCK;

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
}
