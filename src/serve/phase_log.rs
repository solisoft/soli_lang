//! Per-request phase timing for the dev bar.
//!
//! Mirrors `query_log` / `http_log`, but instead of one entry per call we
//! accumulate wall-clock microseconds per named phase. The framework
//! increments these at known wrap points:
//!
//! - `middleware` — sum of every `invoke_middleware_with_frame` call
//! - `view`       — sum of every `cache.render` / `cache.render_partial`
//!
//! "controller" time is derived in the dev bar as `total - middleware - view
//! - db - http`, so we don't need to instrument it directly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOG: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
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

/// Add `dur_us` microseconds to `phase`. No-op when not enabled.
pub fn record(phase: &'static str, dur_us: u64) {
    if !is_enabled() {
        return;
    }
    LOG.with(|l| {
        let mut log = l.borrow_mut();
        *log.entry(phase).or_insert(0) += dur_us;
    });
}

pub fn snapshot() -> Vec<(String, u64)> {
    LOG.with(|l| {
        let mut entries: Vec<(String, u64)> = l
            .borrow()
            .iter()
            .map(|(k, v)| ((*k).to_string(), *v))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    })
}

/// RAII timer: starts a clock on construction and adds the elapsed time to
/// `phase` on drop. Cheap when phase logging is disabled (skips the clock
/// read entirely).
pub struct PhaseTimer {
    phase: &'static str,
    start: Option<Instant>,
}

impl PhaseTimer {
    pub fn start(phase: &'static str) -> Self {
        let start = is_enabled().then(Instant::now);
        Self { phase, start }
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        if let Some(s) = self.start {
            let us = s.elapsed().as_micros() as u64;
            record(self.phase, us);
        }
    }
}
