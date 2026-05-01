//! Per-template timing for the dev bar.
//!
//! `phase_log` already tracks the *total* time spent in views so the
//! render breakdown can compute "controller = total - middleware - view -
//! db - http". When a request renders multiple templates (the main view,
//! its layout, any partials), we want the dev bar to break that aggregate
//! down per template (name + duration), so this module captures one entry
//! per render call instead of a single rolling sum.
//!
//! Mirrors `middleware_log` in shape: thread-local Vec, gated on an
//! `ENABLED` atomic so production has zero cost.
//!
//! Note: nested calls (partials inside layouts inside the main view) each
//! record their own entry. Their durations therefore overlap and will
//! sum to *more* than the aggregate "view" phase — that's expected; the
//! sub-rows are a flat list of *what was rendered*, not a partition.

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
