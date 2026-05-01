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
}

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOG: RefCell<Vec<LoggedQuery>> = const { RefCell::new(Vec::new()) };
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
    query: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
    duration_ms: f64,
) {
    // Note: the matching `span_log::record(SpanKind::Db, …)` is emitted
    // at the actual call site in `crud.rs::exec_async_query_*`, where
    // the original `Instant` is in scope. Recording it here too would
    // produce two db spans per query (one accurate, one back-dated).

    LOG.with(|l| {
        l.borrow_mut().push(LoggedQuery {
            query,
            bind_vars,
            duration_ms,
        })
    });
}

pub fn snapshot() -> Vec<LoggedQuery> {
    LOG.with(|l| l.borrow().clone())
}
