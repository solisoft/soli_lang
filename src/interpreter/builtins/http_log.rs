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

pub fn record(
    method: String,
    url: String,
    status: u16,
    duration_ms: f64,
    error: Option<String>,
) {
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
