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
    crate::interpreter::builtins::model::init_db_config();

    // Boot interpreter: fills the process-global registries (routes,
    // controller metadata, templates), exactly as the server's boot thread.
    let mut boot = Interpreter::new_for_serve();
    crate::interpreter::builtins::mailer::ensure_prelude(&mut boot);
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
    app_loader::load_app_in_worker(
        0,
        &mut interpreter,
        &models_dir,
        &middleware_dir,
        &controllers_dir,
        &jobs_dir,
    );
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

    let mut headers = hyper::header::HeaderMap::new();
    for (name, value) in &request.headers {
        if let (Ok(name), Ok(value)) = (
            hyper::header::HeaderName::from_bytes(name.as_bytes()),
            hyper::header::HeaderValue::from_str(value),
        ) {
            headers.append(name, value);
        }
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
