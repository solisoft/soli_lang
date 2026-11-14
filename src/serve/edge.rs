//! The edge build's entry point: a Soli app inside a Cloudflare Worker.
//!
//! A Worker has no socket to accept on: the runtime hands it one request at a
//! time. So there is no hyper here and no worker pool. [`boot`] loads the app
//! once — the steps `serve_folder_with_options_and_hooks` runs before the pool,
//! then the per-worker setup of `worker_loop` — and [`handle`] answers each
//! request through the very `handle_request` the hyper workers call.
//!
//! It compiles natively as well, which is how it is tested (`tests/edge.rs`).

use super::*;
use std::cell::RefCell;

/// The Soli version this runtime was built from.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One HTTP request as the host saw it. `query` is the raw query string,
/// without the `?`; `body` the raw bytes, which a file upload makes binary.
#[derive(Debug, Default, Clone)]
pub struct EdgeRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct EdgeResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

struct App {
    interpreter: Interpreter,
    vm: Option<crate::vm::Vm>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn general(message: impl Into<String>) -> RuntimeError {
    RuntimeError::General {
        message: message.into(),
        span: Span::default(),
    }
}

/// Load the app rooted at `folder`, in production mode: handlers run on the VM.
///
/// File access goes through [`crate::platform::fs`], so on the edge build the
/// host mounts the app's files first; configuration comes from
/// [`crate::platform::env`], set by the host from the Worker's bindings.
pub fn boot(folder: &Path) -> Result<(), RuntimeError> {
    let app_dir = folder.join("app");
    let models_dir = app_dir.join("models");
    let middleware_dir = app_dir.join("middleware");
    let controllers_dir = app_dir.join("controllers");
    let helpers_dir = app_dir.join("helpers");
    let views_dir = app_dir.join("views");
    let jobs_dir = app_dir.join("jobs");
    let config_dir = folder.join("config");

    if !crate::platform::fs::is_dir(&controllers_dir) {
        return Err(general(format!(
            "{} is not a Soli app: app/controllers is missing",
            folder.display()
        )));
    }

    crate::serve::tenant::set_app_root(folder);
    // `static cron` runs on Cron Triggers here, not from the job table.
    crate::jobs::edge_cron::enable();
    crate::interpreter::builtins::model::init_db_config();
    // The app's named connections (`config/database.toml`), as `soli serve`
    // loads them: without this only the env-derived `primary` existed on the
    // edge, so a model with `connection "…"` failed to load.
    crate::db::init_from_app_path(folder).map_err(|e| general(e.message()))?;
    // A top-level name two files declare: refused, as under `soli serve`.
    crate::serve::global_names::check_app(folder).map_err(general)?;

    // Boot interpreter: fills the process-global registries (routes,
    // controller metadata, templates), exactly as the server's boot thread.
    let mut boot = Interpreter::new_for_serve();
    crate::interpreter::builtins::mailer::ensure_prelude(&mut boot);
    crate::interpreter::events::ensure_prelude(&mut boot);
    if crate::platform::fs::exists(&models_dir) {
        load_models(&mut boot, &models_dir)?;
    }
    for dir in ["services", "policies", "mailers"] {
        let dir = app_dir.join(dir);
        if crate::platform::fs::exists(&dir) {
            load_models(&mut boot, &dir)?;
        }
    }
    if crate::platform::fs::exists(&middleware_dir) {
        load_middleware(&mut boot, &middleware_dir)?;
    }
    if crate::interpreter::builtins::template::view_helpers_present(&helpers_dir) {
        crate::interpreter::builtins::template::load_view_helpers(&helpers_dir).map_err(general)?;
    }
    uploads_prelude::define_uploads_prelude(&mut boot)?;
    for controller in scan_controllers(&controllers_dir)? {
        load_controller(&mut boot, &controllers_dir, &controller)?;
    }
    crate::interpreter::builtins::controller::registry::scan_controllers(&controllers_dir)
        .map_err(general)?;
    init_templates(views_dir);
    crate::interpreter::builtins::template::init_public_dir(folder.join("public"));
    let application_file = config_dir.join("application.sl");
    if crate::platform::fs::exists(&application_file) {
        execute_file(&mut boot, &application_file)?;
    }
    crate::interpreter::builtins::i18n::helpers::load_locales_from_config_dir(&config_dir);
    if let Ok(default) = crate::platform::env::var("SOLI_DEFAULT_LOCALE") {
        if !default.trim().is_empty() {
            crate::interpreter::builtins::i18n::helpers::set_default_locale(default.trim());
        }
    }
    let routes_file = config_dir.join("routes.sl");
    if crate::platform::fs::exists(&routes_file) {
        define_routes_dsl(&mut boot)?;
        crate::interpreter::builtins::server::clear_routes();
        execute_file(&mut boot, &routes_file)?;
        crate::interpreter::builtins::server::rebuild_route_index();
    }
    drop(boot);

    // Worker side (`worker_loop`): its own interpreter with the app reloaded
    // into it, then the VM seeded from its globals.
    crate::interpreter::builtins::server::set_worker_routes(
        crate::interpreter::builtins::server::routes_to_worker_routes(
            &crate::interpreter::builtins::server::get_routes(),
        ),
    );
    crate::interpreter::builtins::template::set_dev_mode(false);
    let mut interpreter = Interpreter::new_for_serve();
    crate::interpreter::events::ensure_prelude(&mut interpreter);
    app_loader::load_app_in_worker(
        0,
        &mut interpreter,
        &models_dir,
        &middleware_dir,
        &controllers_dir,
        &jobs_dir,
    );
    // What a Cloudflare Queue message runs (`run_job`): the pool's runner.
    define_job_runner(&mut interpreter)?;
    let mut vm = crate::vm::Vm::new();
    for (name, value) in interpreter.environment.borrow().get_all_bindings() {
        vm.globals.insert(name, value);
    }
    crate::template::vm_template::set_worker_globals(&vm.globals);
    warm_vm_handlers(0, &vm);

    APP.with(|app| {
        *app.borrow_mut() = Some(App {
            interpreter,
            vm: Some(vm),
        })
    });
    Ok(())
}

/// Answer one request. Before [`boot`] has succeeded every request is a 503.
pub fn handle(request: EdgeRequest) -> EdgeResponse {
    if let Some(script) = framework_script(&request.method, &request.path) {
        return script;
    }
    if let Some(client) = live_client(&request) {
        return client;
    }
    // An isolate serves one request after another: each starts and ends with
    // no tenant, so one site's database never answers another site's request.
    crate::db::tenancy::clear();
    let _tenant_guard = crate::db::tenancy::ClearOnDrop;

    let mut headers = hyper::header::HeaderMap::new();
    for (name, value) in &request.headers {
        if let (Ok(name), Ok(value)) = (
            hyper::header::HeaderName::from_bytes(name.as_bytes()),
            hyper::header::HeaderValue::from_str(value),
        ) {
            headers.append(name, value);
        }
    }
    // The same-origin gate `soli serve` runs before routing (SEC-014): a
    // cross-origin POST never reaches a controller. Cloudflare passes the
    // visitor's Host through, so the request authority is the public one.
    if let Err(reason) = check_csrf_origin(&headers, &request.method, &request.path) {
        return text(403, &format!("CSRF check failed: {reason}"));
    }
    let query = url::form_urlencoded::parse(request.query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let peer_ip = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cf-connecting-ip"))
        .map(|(_, ip)| ip.clone())
        .unwrap_or_default();
    let content_type = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone());
    let (body, multipart_form, multipart_files) = match content_type.as_deref() {
        Some(ct) if ct.starts_with("multipart/form-data") => {
            let (fields, files) = parse_multipart(request.body, ct);
            (String::new(), Some(fields), Some(files))
        }
        _ => (
            String::from_utf8(request.body)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            None,
            None,
        ),
    };
    // `_method=PATCH` in a form, as under `soli serve`.
    let method = apply_form_method_override(
        Cow::Owned(request.method),
        &body,
        content_type.as_deref(),
        multipart_form.as_deref(),
    );
    // Nobody waits on the channel: the response is the return value.
    let (response_tx, _response_rx) = oneshot::channel();
    let mut data = RequestData {
        method,
        path: request.path,
        query,
        headers,
        body,
        body_reservation: None,
        multipart_form,
        multipart_files,
        peer_ip,
        enqueued_at: None,
        replay: false,
        file_template: None,
        response_tx,
    };

    APP.with(|app| {
        let mut app = app.borrow_mut();
        let Some(app) = app.as_mut() else {
            return text(503, "The Soli app has not booted");
        };
        crate::interpreter::builtins::streaming::clear_pending_stream();
        blob_stream::clear_pending();
        let _handler_budget = crate::interpreter::deadline::enter_default();
        let response = handle_request(&mut app.interpreter, &mut app.vm, &mut data, false);
        slow_queries::end_unit();

        // Streaming bodies (SSE, `stream`, `solidb_blob_response`) need the
        // hyper side to relay chunks; say so instead of sending a stub body.
        let blob = blob_stream::take_pending().is_some();
        let stream = crate::interpreter::builtins::streaming::take_pending_stream().is_some();
        if blob || stream {
            return text(
                501,
                "Streaming responses are not available on the edge (Cloudflare Workers) build",
            );
        }
        EdgeResponse {
            status: response.status,
            headers: response.headers,
            body: response.body.to_vec(),
        }
    })
}

/// Define `__soli_run_job_bg(name, args)`, the runner the native job pool
/// calls: `null` on success, the error as a string otherwise.
fn define_job_runner(interpreter: &mut Interpreter) -> Result<(), RuntimeError> {
    let tokens = crate::lexer::Scanner::new(background_jobs::JOBS_BACKGROUND_RUNNER)
        .scan_tokens()
        .map_err(|e| general(format!("job runner: {e}")))?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| general(format!("job runner: {e}")))?;
    interpreter.interpret(&program)
}

/// Run the job one Cloudflare Queue message carries — a [`crate::jobs::JobDoc`]
/// sent by an enqueue on the edge — and say what the Worker's `queue()` handler
/// does with the message: `{status: "done"}`, `{status: "retry", delay}` or
/// `{status: "dead"}`, by the native engine's rule
/// ([`crate::jobs::queues::outcome`]). `attempts` is Cloudflare's count,
/// 1 on the first delivery.
pub fn run_job(message: &str, attempts: u32) -> serde_json::Value {
    let doc: crate::jobs::JobDoc = match serde_json::from_str(message) {
        Ok(doc) => doc,
        Err(e) => {
            return serde_json::json!({
                "status": "dead",
                "error": format!("not a Soli job message: {e}"),
            })
        }
    };
    // Under the tenant the job was enqueued under, and none after it.
    let error = crate::db::tenancy::scoped(doc.tenant.clone(), || {
        if doc.handler == crate::jobs::WEBHOOK_HANDLER {
            let _handler_budget = crate::interpreter::deadline::enter_default();
            return crate::jobs::engine::deliver_webhook(&doc).err();
        }
        perform(&doc.handler, &doc.args)
    });
    let mut report = match crate::jobs::queues::outcome(&doc, attempts, error.clone()) {
        crate::jobs::queues::Outcome::Done => serde_json::json!({ "status": "done" }),
        crate::jobs::queues::Outcome::Retry(delay) => {
            serde_json::json!({ "status": "retry", "delay": delay })
        }
        crate::jobs::queues::Outcome::Dead(_) => serde_json::json!({ "status": "dead" }),
    };
    report["job"] = serde_json::json!({ "id": doc.key, "handler": doc.handler });
    if let Some(error) = error {
        report["error"] = error.into();
    }
    report
}

/// Run one job class's `perform` with `args` through the pool's runner;
/// `Some(error)` when it failed.
fn perform(handler: &str, args: &serde_json::Value) -> Option<String> {
    APP.with(|app| {
        let mut app = app.borrow_mut();
        let Some(app) = app.as_mut() else {
            return Some("the Soli app has not booted".to_string());
        };
        let _handler_budget = crate::interpreter::deadline::enter_default();
        let Some(runner) = app
            .interpreter
            .environment
            .borrow()
            .get("__soli_run_job_bg")
        else {
            return Some("the job runner is not defined".to_string());
        };
        let args = match crate::interpreter::value::parse_json(&args.to_string()) {
            Ok(args) => args,
            Err(e) => return Some(format!("invalid args JSON: {e}")),
        };
        let name = Value::String(handler.to_string().into());
        match app
            .interpreter
            .call_value(runner, vec![name, args], Span::default())
        {
            Ok(Value::String(error)) => Some(error.to_string()),
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        }
    })
}

/// Run the jobs a Cloudflare Cron Trigger stands for: every `static cron`
/// whose expression, without its seconds, is `cron` (`jobs::edge_cron`). Each
/// runs in turn, with no tenant, as `perform_now` would; one failing does not
/// stop the others. The report lists them:
/// `{"cron", "ran": [{"job", "status": "done" | "failed", "error"?}]}`.
pub fn run_cron(cron: &str) -> serde_json::Value {
    let mut ran = Vec::new();
    for entry in crate::jobs::edge_cron::matching(cron) {
        let error =
            crate::db::tenancy::scoped(None, || perform(&entry.class, &serde_json::json!({})));
        let mut report = serde_json::json!({
            "job": entry.class,
            "status": if error.is_some() { "failed" } else { "done" },
        });
        if let Some(error) = error {
            report["error"] = error.into();
        }
        ran.push(report);
    }
    serde_json::json!({ "cron": cron, "ran": ran })
}

/// Split a multipart body into form fields and files with the parser `soli
/// serve` uses. The body is already whole, so the parser's future never waits
/// on anything: it is polled to the end here rather than handed to a runtime
/// the edge build does not have.
fn parse_multipart(
    body: Vec<u8>,
    content_type: &str,
) -> (Vec<(String, String)>, Vec<UploadedFile>) {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};
    let mut parse = std::pin::pin!(super::file_upload::parse_multipart_body(
        bytes::Bytes::from(body),
        content_type
    ));
    let mut cx = Context::from_waker(Waker::noop());
    // Never pending in practice; bounded so a parser that ever did wait could
    // not spin the isolate forever.
    for _ in 0..1024 {
        if let Poll::Ready(parsed) = parse.as_mut().poll(&mut cx) {
            return parsed;
        }
    }
    (Vec::new(), Vec::new())
}

/// The scripts every page loads from the binary itself (`framework_assets` on
/// the hyper side): instant navigation and hover prefetch. The native-bridge,
/// camera and LiveView clients need what the edge build does not serve.
/// `/live/client.js`, as `soli serve` answers it (`framework_assets::live_client`):
/// `no-cache` and an ETag of the client's text, so a browser revalidates and a
/// new client reaches it at once. Not `immutable` like the `?v=`-stamped
/// scripts below: pages load the client at its bare path.
fn live_client(request: &EdgeRequest) -> Option<EdgeResponse> {
    if request.path != crate::live::LIVE_CLIENT_PATH
        || !(request.method == "GET" || request.method == "HEAD")
    {
        return None;
    }
    let etag = crate::live::live_client_etag();
    let fresh = request
        .headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("if-none-match") && value.contains(etag));
    let mut headers = vec![
        ("Cache-Control".to_string(), "no-cache".to_string()),
        ("ETag".to_string(), etag.to_string()),
    ];
    if fresh {
        return Some(EdgeResponse {
            status: 304,
            headers,
            body: Vec::new(),
        });
    }
    headers.push((
        "Content-Type".to_string(),
        "application/javascript; charset=utf-8".to_string(),
    ));
    Some(EdgeResponse {
        status: 200,
        headers,
        body: if request.method == "HEAD" {
            Vec::new()
        } else {
            crate::live::LIVE_CLIENT_JS.as_bytes().to_vec()
        },
    })
}

fn framework_script(method: &str, path: &str) -> Option<EdgeResponse> {
    if method != "GET" && method != "HEAD" {
        return None;
    }
    let script = match path {
        "/__soli/nav.js" => nav::NAV_SCRIPT,
        "/__soli/prefetch.js" => prefetch::PREFETCH_SCRIPT,
        _ => return None,
    };
    Some(EdgeResponse {
        status: 200,
        headers: vec![
            (
                "Content-Type".to_string(),
                "application/javascript; charset=utf-8".to_string(),
            ),
            // The pages version the URL (`?v=`), as under `soli serve`.
            (
                "Cache-Control".to_string(),
                "public, max-age=86400, immutable".to_string(),
            ),
        ],
        body: if method == "HEAD" {
            Vec::new()
        } else {
            script.as_bytes().to_vec()
        },
    })
}

fn text(status: u16, body: &str) -> EdgeResponse {
    EdgeResponse {
        status,
        headers: vec![(
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )],
        body: body.as_bytes().to_vec(),
    }
}

/// One LiveView frame for a host that keeps the instance itself — on the edge,
/// a Durable Object that hibernates between events and holds the instance in
/// its storage.
///
/// `input` is `{"op": "id", "component", "session", "room"?}` (the instance
/// id alone), `{"op": "mount", "component", "session", "room"?}` or
/// `{"op": "event", "instance", "event", "params"?, "session"?}`, `instance`
/// being what an earlier frame returned. The answer is
/// `{"instance": {id, component, session_id, state, last_html},
/// "messages": [<the JSON frames to send to the browser>], "error"?}`.
///
/// Nothing outlives the call: the instance goes through the same registry and
/// the same `handle_liveview_event` as under `soli serve`, then leaves it, so
/// the isolate can be evicted at any time. What needs a long-lived process —
/// `tick` intervals, live queries, `live_update` from elsewhere — is not
/// carried over (see the edge docs).
pub fn live_frame(input: &serde_json::Value) -> serde_json::Value {
    use crate::live::view::{live_registry, LiveViewInstance};

    let text = |key: &str| input[key].as_str().unwrap_or_default().to_string();
    // `?room=` counts only for a component `live_rooms` declared, as in
    // `serve::upgrade`: anything else gets its session's own instance.
    let room = input["room"]
        .as_str()
        .filter(|room| !room.is_empty())
        .filter(|_| crate::live::socket::is_room_component(&text("component")))
        .map(str::to_string);
    let (tx, rx) = async_channel::unbounded();
    let sender = Arc::new(tx);
    let mut error = None;

    // The event to run once the instance is in the registry: the client's, or
    // on a mount the synthetic `connect` `soli serve` fires so a handler can
    // seed the initial state.
    let (liveview_id, component, event) = match text("op").as_str() {
        // The instance a socket belongs to, by the rules `soli serve` applies:
        // `?room=` counts only for a component `live_rooms` declared. The
        // host names the Durable Object after it, so an undeclared room
        // cannot make two visitors share one object.
        "id" => {
            return serde_json::json!({
                "id": crate::live::socket::liveview_instance_id(
                    &text("session"),
                    &text("component"),
                    room.as_deref(),
                ),
            });
        }
        "mount" => {
            let component = text("component");
            let session = text("session");
            crate::live::socket::handle_live_connection(
                component.clone(),
                session.clone(),
                sender.clone(),
                room.clone(),
            );
            let id =
                crate::live::socket::liveview_instance_id(&session, &component, room.as_deref());
            let connect = crate::live::socket::get_liveview_handler(&component)
                .map(|_| "connect".to_string());
            (id, component, connect)
        }
        "event" => {
            let stored = &input["instance"];
            let field = |key: &str| stored[key].as_str().unwrap_or_default().to_string();
            let (id, component) = (field("id"), field("component"));
            let mut instance = LiveViewInstance::new(
                component.clone(),
                PathBuf::from(format!("app/views/live/{component}.sliv")),
                stored["state"].clone(),
                field("session_id"),
                sender.clone(),
            );
            instance.id = id.clone();
            instance.last_html = field("last_html");
            live_registry().register(instance);
            (id, component, Some(text("event")))
        }
        other => {
            return serde_json::json!({
                "error": format!("unknown LiveView op {other:?}: use \"mount\" or \"event\""),
            })
        }
    };

    if let Some(event) = event {
        let (response_tx, _response_rx) = oneshot::channel();
        let data = LiveViewEventData {
            liveview_id: liveview_id.clone(),
            component,
            event,
            params: input
                .get("params")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
            sender_session: input["session"].as_str().map(str::to_string),
            response_tx,
        };
        let outcome = APP.with(|app| match app.borrow_mut().as_mut() {
            Some(app) => handle_liveview_event_caught(&mut app.interpreter, &data),
            None => Err("the app has not booted".to_string()),
        });
        if let Err(e) = outcome {
            error = Some(e);
        }
    }

    let instance = live_registry().get(&liveview_id);
    live_registry().unregister(&liveview_id);
    crate::live::socket::cancel_tick_task(&liveview_id);
    let mut messages = Vec::new();
    while let Ok(Ok(message)) = rx.try_recv() {
        if let Ok(frame) = message.to_text() {
            messages.push(serde_json::Value::String(frame.to_string()));
        }
    }
    let mut answer = serde_json::json!({ "messages": messages });
    if let Some(instance) = instance {
        answer["instance"] = serde_json::json!({
            "id": instance.id,
            "component": instance.component,
            "session_id": instance.session_id,
            "state": instance.state,
            "last_html": instance.last_html,
        });
    }
    if let Some(error) = error {
        answer["error"] = serde_json::Value::String(error);
    }
    answer
}
