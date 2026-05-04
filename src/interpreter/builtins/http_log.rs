//! Per-request log of outgoing HTTP calls made via the `HTTP.*` builtin.
//!
//! Mirrors `model::query_log` but for the user-facing HTTP client. The server
//! clears the log at the start of each incoming request so the dev bar shows
//! only the outbound calls fired during that single request.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct LoggedHttpRequest {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub duration_ms: f64,
    pub error: Option<String>,
}

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOG: RefCell<Vec<LoggedHttpRequest>> = const { RefCell::new(Vec::new()) };
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn clear() {
    LOG.with(|l| l.borrow_mut().clear());
}

pub fn record(method: String, url: String, status: u16, duration_ms: f64, error: Option<String>) {
    // Anchor the span "end" to now and back-date the start by the
    // measured duration — close enough for visualisation, since the
    // call site doesn't expose the original start instant here.
    let dur_us = (duration_ms * 1000.0).max(0.0) as u64;
    let start = std::time::Instant::now() - std::time::Duration::from_micros(dur_us);
    record_with_start(method, url, status, duration_ms, error, start);
}

/// Like `record`, but uses the caller's real `Instant` for the flamegraph
/// span. Used by parallel-fetch helpers (`HTTP.get_all` etc.) that capture
/// the start timestamp on a worker thread and then record on the main
/// thread after `join()` — back-dating from `Instant::now()` would collapse
/// every concurrent call to the same end time and lose the parallelism.
pub fn record_with_start(
    method: String,
    url: String,
    status: u16,
    duration_ms: f64,
    error: Option<String>,
    real_start: std::time::Instant,
) {
    // Mirror this call as a span so it shows up in the dev-bar flamegraph
    // nested under whatever action / view fired it. Span_log is its own
    // gate, so this is a no-op when --dev is off.
    if crate::serve::span_log::is_enabled() {
        let dur_us = (duration_ms * 1000.0).max(0.0) as u64;
        let name = format!("{} {}", method, url);
        crate::serve::span_log::record(
            &name,
            crate::serve::span_log::SpanKind::Http,
            real_start,
            dur_us,
            error.clone(),
        );
    }

    LOG.with(|l| {
        l.borrow_mut().push(LoggedHttpRequest {
            method,
            url,
            status,
            duration_ms,
            error,
        })
    });
}

pub fn snapshot() -> Vec<LoggedHttpRequest> {
    LOG.with(|l| l.borrow().clone())
}
