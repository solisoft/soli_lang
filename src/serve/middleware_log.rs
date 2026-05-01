//! Per-middleware timing for the dev bar.
//!
//! `phase_log` already tracks the *total* time spent in middleware so the
//! render breakdown can compute "controller = total - middleware - view -
//! db - http". When more than one middleware fires on a request, we want
//! the dev bar to break that aggregate down per middleware (name +
//! duration), so this module captures one entry per call instead of a
//! single rolling sum.
//!
//! Cheap when dev mode is off — `record` early-outs on the flag.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOG: RefCell<Vec<(String, u64)>> = const { RefCell::new(Vec::new()) };
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

pub fn record(name: &str, dur_us: u64) {
    if !is_enabled() {
        return;
    }
    LOG.with(|l| l.borrow_mut().push((name.to_string(), dur_us)));
}

pub fn snapshot() -> Vec<(String, u64)> {
    LOG.with(|l| l.borrow().clone())
}
