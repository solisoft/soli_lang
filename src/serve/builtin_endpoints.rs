//! The endpoints the framework answers itself, before the application is asked.
//!
//! Lifted out of `handle_request`. They are `probes::handle` on the worker side
//! of the channel: a path and a method go in, a response comes out, and nothing
//! about them depends on the session, the route table or the request body — so
//! they are a function rather than a stage of the pipeline. The reason they run
//! on the worker at all is that `/openapi` reads the route registry, which is a
//! per-worker thread-local.
//!
//! Answering here, ahead of everything, is also what keeps `/up` honest: see the
//! comment on [`up`] for what a probe that created a session would cost.
//!
//! - [`handle`] — the dispatch, `None` when the path belongs to the app.
//! - [`up`] — the readiness probe.
//! - [`openapi`] — the spec and the UI over it.

use super::{error_response, openapi as openapi_spec, ResponseData};

/// `/up`, `/openapi.json` and `/openapi`, or `None` for anything else.
pub(super) fn handle(method: &str, path: &str) -> Option<ResponseData> {
    match path {
        "/up" => Some(up()),
        "/openapi.json" | "/openapi" if method == "GET" => Some(openapi(path)),
        _ => None,
    }
}

/// Built-in readiness probe for blue/green deploys (soli-proxy's health
/// gate). Returns 503 until the session store's backing connection has been
/// warmed, and 200 afterwards. A liveness-only health check (a bare 200
/// from a freshly-booted slot) promotes the slot before its first session
/// round-trip can complete, so traffic switches into the cold-connection
/// window and requests stall to the HTTP client timeout. Gating promotion
/// on this endpoint keeps the old slot serving until the new one is truly
/// ready. Answered here, before any session/cookie work, so the probe never
/// creates a session or touches the store. Apps should not define their own
/// `/up` route — this built-in shadows it.
fn up() -> ResponseData {
    if crate::interpreter::builtins::session::session_store_ready() {
        error_response::text(200, "ready")
    } else {
        error_response::text(503, "warming")
    }
}

/// Opt-in OpenAPI (SOLI_OPENAPI): the spec + a Scalar UI over it, built from
/// the app's registered routes (a per-worker thread-local, hence answered
/// here on the worker rather than the async layer). Always-on when enabled,
/// production included. 404 when disabled so it's invisible by default.
fn openapi(path: &str) -> ResponseData {
    if !openapi_spec::openapi_enabled() {
        return error_response::text(404, "Not Found");
    }
    if path == "/openapi.json" {
        ResponseData {
            status: 200,
            headers: vec![(
                "Content-Type".to_string(),
                "application/json; charset=utf-8".to_string(),
            )],
            body: openapi_spec::generate_spec_json().into_bytes(),
        }
    } else {
        error_response::html(200, openapi_spec::ui_page())
    }
}
