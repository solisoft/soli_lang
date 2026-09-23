//! The tail of a request: everything that happens after the handler returns.
//!
//! Lifted out of `handle_request`, where it was a 276-line closure reading
//! fourteen locals defined up to 550 lines above it — the clearest case in the
//! file of ambient captures wanting to be a struct. It is a function rather than
//! a stage because it runs on every exit: the fast path, the middleware halt and
//! the handler return all end here, and a closure that three `return` statements
//! share is a function with its arguments left implicit.
//!
//! The order inside [`finish`] is load-bearing and is the order it always had:
//! session and response cookies, security headers, trace headers, test-runner
//! headers, the dev snapshot, the OTLP export and the access line — each reads
//! what the last one wrote.
//!
//! - [`Finalizer`] — what the tail needs, gathered rather than captured.
//! - [`finish`] — the sequence.

use std::time::Instant;

use uuid::Uuid;

use crate::interpreter::builtins::session::{
    finalize_session_cookie, get_current_session_id, set_current_session_id, take_response_cookies,
};

use super::{
    dev_bar, dev_store, log_timestamp, middleware_log, otel, phase_log, prod_log, route_log,
    span_log, template_warnings, view_log, ResponseData,
};

/// Everything the tail of a request needs, gathered rather than captured.
pub(super) struct Finalizer {
    /// Whether the server runs with `--dev`. Decides the access-line format and
    /// whether the root span is closed here or by the dev snapshot.
    pub(super) dev_mode: bool,
    /// The dev bar's own timer. `Some` exactly when `--dev` is on; its elapsed
    /// value becomes `X-Soli-Render-Us`.
    pub(super) dev_started: Option<Instant>,
    /// Whether a span tree was opened for this request — by `--dev`, by an
    /// inbound `traceparent`, or by OTEL being on.
    pub(super) span_started: Option<Instant>,
    /// The W3C trace context, when OTEL is enabled. Drives the echoed
    /// `traceparent`, the minted `X-Request-Id` and the OTLP export.
    pub(super) trace_ctx: Option<otel::TraceContext>,
    /// The access-log timer. `Some` exactly when `log_requests` is true.
    pub(super) start_time: Option<Instant>,
    /// Time the request spent queued before a worker took it, in ms. `None`
    /// when logging is off, because then nothing timestamped the enqueue.
    pub(super) queue_ms: Option<f64>,
    /// Whether to write an access line at all: `--dev`, or any `SOLI_LOG`
    /// channel.
    pub(super) log_requests: bool,
    /// Which `SOLI_LOG` channels the operator turned on.
    pub(super) log_channels: prod_log::LogChannels,
    /// The session id the client arrived with. `Set-Cookie` is emitted only
    /// when the post-handler id differs from it.
    pub(super) cookie_session_id: Option<String>,
    /// Whether a `Set-Cookie` written here gets the `Secure` flag.
    pub(super) cookie_secure: bool,
    /// Whether this response is an HTMx partial swap. The dev bar is not
    /// injected into one — the page it swaps into already carries a bar.
    pub(super) is_htmx: bool,
    /// The request as it arrived, kept in `--dev` for the replay button.
    pub(super) captured_raw: Option<dev_store::RawRequest>,
}

/// Session and response cookies, security headers, the dev bar, the traces
/// and the access line — in that order, because each reads what the last wrote.
///
/// `method` and `path` are parameters rather than fields: they borrow from the
/// `RequestData` whose `headers` are taken while this struct is alive, and
/// disjoint field borrows are what makes that compile.
pub(super) fn finish(
    f: &Finalizer,
    method: &str,
    path: &str,
    mut resp: ResponseData,
) -> ResponseData {
    cookies(f, &mut resp);
    security_headers(&mut resp);
    trace_headers(f, &mut resp);
    test_runner_headers(&mut resp);
    dev_snapshot(f, method, path, &mut resp);
    export_traces(f, &resp);
    access_log(f, method, path, &resp);

    // Clear session context
    set_current_session_id(None);
    crate::interpreter::builtins::trust_proxy::set_current_peer_ip(None);
    resp
}

/// The session cookie, then whatever `set_cookie()` accumulated.
fn cookies(f: &Finalizer, resp: &mut ResponseData) {
    // Drop the per-request scheme/host so a `<name>_url` call between
    // requests (e.g. from a background timer) errors clearly instead of
    // building a URL with a stale host.
    crate::interpreter::builtins::named_routes::clear_current_request_host();
    if let Some(cookie_value) = finalize_session_cookie(
        get_current_session_id().as_deref(),
        f.cookie_session_id.as_deref(),
        f.cookie_secure,
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
}

/// Add security headers if enabled
fn security_headers(resp: &mut ResponseData) {
    crate::interpreter::builtins::security_headers::append_security_headers(&mut resp.headers);
}

/// W3C Trace Context: always echo a traceparent when OTEL is on so
/// upstream proxies / clients can correlate with our OTLP export and
/// JSON access logs. Also stamp a stable request id (dev already does
/// via X-Soli-Request-Id) so logs, traces, and clients share a key.
fn trace_headers(f: &Finalizer, resp: &mut ResponseData) {
    let Some(ctx) = f.trace_ctx.as_ref() else {
        return;
    };
    resp.headers
        .push(("traceparent".to_string(), ctx.traceparent()));
    if request_id_header(resp).is_none() {
        resp.headers
            .push(("X-Request-Id".to_string(), Uuid::new_v4().to_string()));
    }
}

/// The request id already stamped on the response, under either name.
fn request_id_header(resp: &ResponseData) -> Option<&str> {
    resp.headers
        .iter()
        .find(|(k, _)| {
            k.eq_ignore_ascii_case("x-soli-request-id") || k.eq_ignore_ascii_case("x-request-id")
        })
        .map(|(_, v)| v.as_str())
}

/// E2E test client: ship the render captured by render() back as
/// response headers, so assigns()/view_path()/render_template() work
/// across the test-runner -> server process boundary. The locals JSON
/// is base64-encoded so arbitrary UTF-8 / control chars stay
/// header-safe. Test-runner only; absent (no render) -> no headers ->
/// render_template() reports false on redirects/JSON responses.
fn test_runner_headers(resp: &mut ResponseData) {
    if !crate::interpreter::builtins::test_server::is_test_runner_process() {
        return;
    }
    if let Some(captured) = crate::interpreter::builtins::test_server::take_captured_render() {
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
    let queries = crate::interpreter::builtins::model::query_log::snapshot();
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
        resp.headers
            .push(("x-soli-test-n1".to_string(), base64_json(&arr)));
    }
    // Coalescing opportunities: reads that each paid their own
    // round-trip outside any `grouped` block. The runner's server
    // coalesces (see `batch::should_coalesce`), so what a spec sees
    // here is what production would do — a grouped action reports
    // one query, not one per read.
    let ungrouped = dev_bar::detect_ungrouped_reads(&queries, dev_bar::UNGROUPED_READ_THRESHOLD);
    if !ungrouped.is_empty() {
        let arr: Vec<serde_json::Value> = ungrouped
            .iter()
            .map(|template| serde_json::json!({ "query": template }))
            .collect();
        resp.headers
            .push(("x-soli-test-ungrouped".to_string(), base64_json(&arr)));
    }
}

/// A JSON array, base64-encoded so it survives as a header value.
fn base64_json(arr: &[serde_json::Value]) -> String {
    base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::Value::Array(arr.to_vec())
            .to_string()
            .as_bytes(),
    )
}

/// Inject the dev bar into HTML responses when running --dev. The bar
/// is rendered here on the worker thread because the AQL query log is
/// a thread-local, so the snapshot must happen before we cross the
/// channel back to the hyper handler.
fn dev_snapshot(f: &Finalizer, method: &str, path: &str, resp: &mut ResponseData) {
    let Some(start) = f.dev_started else {
        return;
    };
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
        method: method.to_string(),
        path: path.to_string(),
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
    if let Some(raw) = &f.captured_raw {
        dev_store::put_raw(request_id.clone(), raw.clone());
    }
    dev_store::put(request_id, ctx.clone());

    // Inject the bar only into full HTML pages. HTMx partial responses
    // share the page that already carries the dev bar; injecting again
    // would append a second one into the live DOM on each swap.
    if is_html && !f.is_htmx {
        if let Ok(body_str) = std::str::from_utf8(&resp.body) {
            resp.body = dev_bar::inject_dev_bar(body_str, &ctx).into_bytes();
        }
    }
}

/// Close the request-root span and export the tree over OTLP when
/// tracing is on. In --dev the same close happens above for the
/// flamegraph; close_request_root is idempotent.
fn export_traces(f: &Finalizer, resp: &ResponseData) {
    if f.span_started.is_some() && !f.dev_mode {
        span_log::close_request_root();
    }
    if let Some(ctx) = f.trace_ctx.as_ref() {
        // Prefer the request id already stamped on the response (dev
        // path); otherwise mint one so logs and OTLP share a key.
        let req_id = request_id_header(resp);
        let spans = span_log::snapshot();
        otel::export_request(ctx, &spans, resp.status, req_id);
    }
}

/// Log timing (skip health checks to avoid benchmark noise)
fn access_log(f: &Finalizer, method: &str, path: &str, resp: &ResponseData) {
    if !f.log_requests || path == "/health" {
        return;
    }
    let elapsed_ms = f.start_time.unwrap().elapsed().as_secs_f64() * 1000.0;
    if f.dev_mode {
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
        return;
    }
    // Production: emit the access line plus whatever detail
    // channels the operator enabled via SOLI_LOG. `emit` also
    // gates the SOLI_SLOW_REQUEST_MS full-detail block on
    // queue + handler time and prints nothing for fast
    // requests when only the slow threshold is configured.
    let emit_meta = prod_log::EmitMeta {
        request_id: request_id_header(resp),
        trace_id: f.trace_ctx.as_ref().map(|c| c.trace_id.as_str()),
        span_id: f.trace_ctx.as_ref().map(|c| c.span_id.as_str()),
    };
    prod_log::emit_with_meta(
        method,
        path,
        resp.status,
        elapsed_ms,
        f.queue_ms,
        f.log_channels,
        emit_meta,
    );
}
