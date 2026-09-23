//! Per-request AQL query log for the dev-mode debugging tool.
//!
//! When enabled (dev mode), every AQL query going through `crud::exec_async_*`
//! is appended to a thread-local `Vec<LoggedQuery>`. The server clears the log
//! at the start of each request so the snapshot returned by `dev_queries()`
//! corresponds to that single request.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct LoggedQuery {
    pub query: String,
    pub bind_vars: Option<HashMap<String, serde_json::Value>>,
    pub duration_ms: f64,
    /// Whether this query was issued inside a `grouped(fn() { ... })` block.
    ///
    /// Stamped even when the block is not coalescing (interactive `--dev`), so
    /// the dev bar can tell reads the author already grouped from ones that are
    /// still paying a round-trip each. See `batch::in_block`.
    pub grouped: bool,
}

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Most entries a per-request log (queries, HTTP calls, KV commands) keeps.
///
/// These logs are emptied when a request, socket event or job begins; code
/// that runs outside those (a long-lived loop, a job issuing a query per row)
/// would otherwise grow them without bound. Past the cap an entry is counted
/// in `dropped()` instead of stored.
pub const MAX_LOGGED_PER_REQUEST: usize = 10_000;

thread_local! {
    static LOG: RefCell<Vec<LoggedQuery>> = const { RefCell::new(Vec::new()) };
    static DROPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
    DROPPED.with(|d| d.set(0));
}

/// Queries not stored since the last `clear()` because the log was full.
pub fn dropped() -> usize {
    DROPPED.with(|d| d.get())
}

pub fn record(
    query: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
    duration_ms: f64,
) {
    // Note: the matching `span_log::record(SpanKind::Db, …)` is emitted
    // at the actual call site in `crud.rs::exec_async_query_*`, where
    // the original `Instant` is in scope. Recording it here too would
    // produce two db spans per query (one accurate, one back-dated).

    let grouped = super::batch::in_block();
    LOG.with(|l| {
        let mut log = l.borrow_mut();
        if log.len() >= MAX_LOGGED_PER_REQUEST {
            DROPPED.with(|d| d.set(d.get() + 1));
            return;
        }
        log.push(LoggedQuery {
            query,
            bind_vars,
            duration_ms,
            grouped,
        })
    });
}

pub fn snapshot() -> Vec<LoggedQuery> {
    LOG.with(|l| l.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Something that queries outside a request never has its log emptied;
    /// past the cap an entry is counted, not stored.
    #[test]
    fn the_log_stops_at_its_cap() {
        clear();
        for _ in 0..=MAX_LOGGED_PER_REQUEST {
            record("RETURN 1".to_string(), None, 0.0);
        }
        assert_eq!(snapshot().len(), MAX_LOGGED_PER_REQUEST);
        assert_eq!(dropped(), 1);
        clear();
        assert_eq!(dropped(), 0);
        assert!(snapshot().is_empty());
    }
}
