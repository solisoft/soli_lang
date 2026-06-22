//! Per-request log of SoliKV / Cache commands made via the `KV.*` and
//! `Cache.*` builtins.
//!
//! Mirrors `http_log` but for the RESP key-value store. Every command goes
//! through `solikv::resp_cmd`, which records the verb (`GET`, `SET`, …), the
//! key it touched, the round-trip duration, and any error. The server clears
//! the log at the start of each incoming request so the dev bar shows only
//! the KV calls fired during that single request.
//!
//! Values are deliberately NOT logged — only the command verb and the key —
//! so cached payloads (which may hold secrets or large blobs) never leak into
//! the dev bar or the production log.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct LoggedKvCall {
    /// The RESP verb, upper-cased: `GET`, `SET`, `DEL`, `INCR`, …
    pub command: String,
    /// The key the command operated on (the first argument after the verb),
    /// or an empty string for keyless commands such as `PING`.
    pub key: String,
    pub duration_ms: f64,
    pub error: Option<String>,
}

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOG: RefCell<Vec<LoggedKvCall>> = const { RefCell::new(Vec::new()) };
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

pub fn record(command: String, key: String, duration_ms: f64, error: Option<String>) {
    // Mirror this call as a span so it shows up in the dev-bar flamegraph
    // nested under whatever action / view fired it. `span_log` has its own
    // gate, so this is a no-op when --dev is off. Back-date the start by the
    // measured duration — the call site doesn't expose the original instant.
    if crate::serve::span_log::is_enabled() {
        let dur_us = (duration_ms * 1000.0).max(0.0) as u64;
        let start = std::time::Instant::now() - std::time::Duration::from_micros(dur_us);
        let name = if key.is_empty() {
            command.clone()
        } else {
            format!("{} {}", command, key)
        };
        crate::serve::span_log::record(
            &name,
            crate::serve::span_log::SpanKind::Kv,
            start,
            dur_us,
            error.clone(),
        );
    }

    LOG.with(|l| {
        l.borrow_mut().push(LoggedKvCall {
            command,
            key,
            duration_ms,
            error,
        })
    });
}

pub fn snapshot() -> Vec<LoggedKvCall> {
    LOG.with(|l| l.borrow().clone())
}
