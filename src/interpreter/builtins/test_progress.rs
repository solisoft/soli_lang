//! Live counters for the test runner's progress display.
//!
//! The authoritative per-file assertion count lives in a thread-local
//! (`assertions::ASSERTION_COUNT`), read once when a file finishes. That is the
//! right home for it — it is what the summary and the per-file report need —
//! but it makes the numbers on screen stand still: the thread painting the
//! progress bar cannot see another thread's thread-local, so a spec file that
//! runs for twenty seconds contributes nothing to the display until it ends.
//!
//! These are process-global instead, so the painter can read them mid-file.
//! They accumulate across the whole suite and are never reset per file; the
//! runner resets them once before it starts.
//!
//! `Relaxed` throughout: these are counters nothing else is ordered against,
//! and a display that is a few assertions behind for a few microseconds is
//! indistinguishable from one that is not.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Mutex;

static ASSERTIONS: AtomicI64 = AtomicI64::new(0);
static TESTS_PASSED: AtomicUsize = AtomicUsize::new(0);
static TESTS_FAILED: AtomicUsize = AtomicUsize::new(0);
static FAIL_FAST: AtomicBool = AtomicBool::new(false);
static FAIL_FAST_TRIPPED: AtomicBool = AtomicBool::new(false);
static NAME_FILTER: Mutex<Option<String>> = Mutex::new(None);

/// `soli test --filter <text>`: only tests whose full description (enclosing
/// `describe` names then the test name, space-joined) contains `text`,
/// case-insensitively, run.
pub fn set_name_filter(filter: Option<String>) {
    *NAME_FILTER.lock().unwrap() = filter.map(|f| f.to_lowercase());
}

/// True when no filter is set or `description` matches it.
pub fn matches_name_filter(description: &str) -> bool {
    match NAME_FILTER.lock().unwrap().as_deref() {
        Some(filter) => description.to_lowercase().contains(filter),
        None => true,
    }
}

/// `soli test --fail-fast`: stop scheduling tests after the first failure.
pub fn set_fail_fast(enabled: bool) {
    FAIL_FAST.store(enabled, Ordering::Relaxed);
    FAIL_FAST_TRIPPED.store(false, Ordering::Relaxed);
}

/// True once `--fail-fast` is on and a test has failed.
pub fn fail_fast_tripped() -> bool {
    FAIL_FAST_TRIPPED.load(Ordering::Relaxed)
}

/// What the progress bar draws, read in one go so the three numbers on screen
/// belong to roughly the same instant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveCounts {
    pub assertions: i64,
    pub tests_passed: usize,
    pub tests_failed: usize,
}

impl LiveCounts {
    /// Tests finished so far, passed and failed together.
    pub fn tests_run(&self) -> usize {
        self.tests_passed + self.tests_failed
    }
}

/// Count one assertion. Called alongside the thread-local bump, never instead
/// of it — the per-file count is still what the report prints.
pub fn record_assertion() {
    ASSERTIONS.fetch_add(1, Ordering::Relaxed);
}

/// Count one finished `test(...)` block. Nothing else counts them: before this
/// existed the runner knew only how many *files* had run.
pub fn record_test(passed: bool) {
    if passed {
        TESTS_PASSED.fetch_add(1, Ordering::Relaxed);
    } else {
        TESTS_FAILED.fetch_add(1, Ordering::Relaxed);
        if FAIL_FAST.load(Ordering::Relaxed) {
            FAIL_FAST_TRIPPED.store(true, Ordering::Relaxed);
        }
    }
}

pub fn snapshot() -> LiveCounts {
    LiveCounts {
        assertions: ASSERTIONS.load(Ordering::Relaxed),
        tests_passed: TESTS_PASSED.load(Ordering::Relaxed),
        tests_failed: TESTS_FAILED.load(Ordering::Relaxed),
    }
}

/// Zero the counters. The runner calls this once before the first worker
/// starts; a suite is one accumulation.
pub fn reset() {
    ASSERTIONS.store(0, Ordering::Relaxed);
    TESTS_PASSED.store(0, Ordering::Relaxed);
    TESTS_FAILED.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counters are global, so this is the one test in the module: two of
    /// them running at once would read each other's writes.
    #[test]
    fn counts_accumulate_and_reset() {
        reset();
        assert_eq!(snapshot(), LiveCounts::default());

        record_assertion();
        record_assertion();
        record_test(true);
        record_test(true);
        record_test(false);

        let counts = snapshot();
        assert_eq!(counts.assertions, 2);
        assert_eq!(counts.tests_passed, 2);
        assert_eq!(counts.tests_failed, 1);
        assert_eq!(counts.tests_run(), 3);

        reset();
        assert_eq!(snapshot(), LiveCounts::default());
    }
}
