//! One home for the error responses the request path assembles by hand.
//!
//! Lifted out of `handle_request`, `call_handler` and `call_oop_controller_action`,
//! where the same four lines — mint a request id, render the production error
//! page, wrap it in an HTML [`ResponseData`] — were written out nine times, and
//! where the `dev_mode` fork between the debug error page and the production one
//! was written out four times. It is a function rather than a stage because it
//! takes a status and a message and returns a response: nothing it does depends
//! on where in the pipeline the failure happened.
//!
//! The duplication had already drifted. Two of the four 500 forks map a
//! breakpoint to `200` in `--dev` and two return a flat `500`; [`handler_failure`]
//! carries the distinction as a parameter so each caller states which it is
//! instead of re-deriving it. See the note on that function for why the two flat
//! sites keep `false`.
//!
//! - [`html`] / [`text`] — the response envelope, nothing else.
//! - [`page`] — request id + production error page, no log line.
//! - [`production`] — [`page`] plus the `[WARN]` access line.
//! - [`handler_failure`] — the dev/production 500 fork.

use uuid::Uuid;

use crate::interpreter::Interpreter;

use super::{error_logging, error_pages, RequestData, ResponseData};

/// An HTML response: `Content-Type` and nothing else.
pub(super) fn html(status: u16, body: String) -> ResponseData {
    ResponseData {
        status,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: body.into_bytes(),
    }
}

/// A plain-text response, for the built-in endpoints that answer without a
/// template.
pub(super) fn text(status: u16, body: &'static str) -> ResponseData {
    ResponseData {
        status,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: body.as_bytes().to_vec(),
    }
}

/// Mint a request id and render the production error page for `status`.
///
/// Apps can ship `app/views/errors/<status>.html.slv` and have it rendered
/// here, which is why even the 403/404 that never reach a controller go through
/// this pipeline rather than emitting a string.
///
/// No log line: the `RecordNotFound` 404 and the `forbidden()` 403 are ordinary
/// application outcomes and have never written one.
pub(super) fn page(status: u16, message: &str) -> ResponseData {
    let request_id = Uuid::new_v4().to_string();
    html(
        status,
        error_pages::render_production_error_page(status, message, &request_id),
    )
}

/// [`page`], preceded by the `[WARN] request_id=… METHOD PATH - STATUS` line
/// the request path writes when it refuses a request outright.
///
/// `note` is appended as `: {note}` — only the CSRF 403 carries one (the
/// verification failure's reason). Everything else passes `None` and the line is
/// what it has always been.
pub(super) fn production(
    status: u16,
    method: &str,
    path: &str,
    message: &str,
    note: Option<&str>,
) -> ResponseData {
    let request_id = Uuid::new_v4().to_string();
    match note {
        Some(note) => eprintln!(
            "[WARN] request_id={} {} {} - {} {}",
            request_id, method, path, status, note
        ),
        None => eprintln!(
            "[WARN] request_id={} {} {} - {}",
            request_id, method, path, status
        ),
    }
    html(
        status,
        error_pages::render_production_error_page(status, message, &request_id),
    )
}

/// The 500 a failing handler turns into: the debug page in `--dev`, the
/// production page otherwise, with the stderr context block in both.
///
/// `breakpoint` says whether `error_msg` came from an intentional debug pause.
/// A breakpoint is not a failure, so it skips the stderr block and answers `200`
/// in `--dev` — the debugger UI is the page. Two of this function's four callers
/// pass a hard `false`: their errors are `String`s from `resolve_handler` and
/// `create_controller_instance`, which have no breakpoint channel to carry one.
#[allow(clippy::too_many_arguments)]
pub(super) fn handler_failure(
    dev_mode: bool,
    interpreter: &Interpreter,
    request_data: &RequestData,
    error_msg: &str,
    stack_trace: &[String],
    env_json: Option<&str>,
    request_id: &str,
    breakpoint: bool,
) -> ResponseData {
    if !breakpoint {
        error_logging::log_production_error(
            request_id,
            request_data,
            error_msg,
            stack_trace,
            env_json,
        );
    }
    if dev_mode {
        html(
            if breakpoint { 200 } else { 500 },
            error_pages::render_error_page(
                error_msg,
                interpreter,
                request_data,
                stack_trace,
                env_json,
            ),
        )
    } else {
        html(
            500,
            error_pages::render_production_error_page(500, error_msg, request_id),
        )
    }
}
