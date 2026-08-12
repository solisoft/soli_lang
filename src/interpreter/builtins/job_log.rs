//! Per-request log of jobs enqueued during the current request.
//!
//! Mirrors `kv_log` / `http_log`: the server clears the log when a request
//! starts and snapshots it when the response is finalized, so the dev bar shows
//! exactly the jobs that request queued. Under the old SolidB-webhook engine an
//! enqueue was invisible to the dev bar (it bypassed the HTTP log); this makes
//! it a first-class panel entry.
//!
//! Job *arguments* are deliberately not logged — only handler, queue, and
//! scheduling metadata — so payloads (which routinely carry emails, tokens, and
//! other user data) never leak into the dev bar.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct LoggedJobCall {
    /// Job class name, or `__WebhookDelivery` for outbound webhooks.
    pub handler: String,
    pub queue: String,
    /// Job id, so a dev-bar row can be matched against the queue table.
    pub job_id: String,
    /// When the job becomes eligible to run (ISO-8601 UTC).
    pub run_at: String,
    pub priority: i64,
    /// True when `run_at` is in the future (a delayed / scheduled enqueue).
    pub delayed: bool,
}

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOG: RefCell<Vec<LoggedJobCall>> = const { RefCell::new(Vec::new()) };
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

/// Record an enqueue. Cheap no-op when the dev bar is off.
pub fn record(doc: &crate::jobs::JobDoc) {
    if !is_enabled() {
        return;
    }
    let delayed = doc.state == crate::jobs::JobState::Scheduled;
    LOG.with(|l| {
        l.borrow_mut().push(LoggedJobCall {
            handler: doc.handler.clone(),
            queue: doc.queue.clone(),
            job_id: doc.key.clone(),
            run_at: doc.run_at.clone(),
            priority: doc.priority,
            delayed,
        })
    });
}

pub fn snapshot() -> Vec<LoggedJobCall> {
    LOG.with(|l| l.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{iso_from_unix, now_iso, unix_now, JobDoc};
    use std::sync::Mutex;

    /// `ENABLED` is a process-global flag, so these tests must not run
    /// concurrently — one flipping it off mid-run would empty another's log.
    fn log_lock() -> &'static Mutex<()> {
        static LOCK: Mutex<()> = Mutex::new(());
        &LOCK
    }

    #[test]
    fn recording_is_a_noop_while_disabled() {
        let _g = log_lock().lock().unwrap_or_else(|e| e.into_inner());
        set_enabled(false);
        clear();
        record(&JobDoc::new(
            "J",
            serde_json::json!({}),
            "default",
            now_iso(),
        ));
        assert!(snapshot().is_empty(), "no rows should be kept in prod");
    }

    #[test]
    fn enabled_log_captures_metadata_but_never_args() {
        let _g = log_lock().lock().unwrap_or_else(|e| e.into_inner());
        set_enabled(true);
        clear();
        let mut doc = JobDoc::new(
            "EmailJob",
            serde_json::json!({ "token": "secret-value" }),
            "mail",
            now_iso(),
        );
        doc.priority = 7;
        record(&doc);

        let rows = snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].handler, "EmailJob");
        assert_eq!(rows[0].queue, "mail");
        assert_eq!(rows[0].priority, 7);
        assert!(!rows[0].delayed);
        assert_eq!(rows[0].job_id, doc.key);
        // The payload must not be reachable from the log entry at all.
        let rendered = format!("{:?}", rows[0]);
        assert!(!rendered.contains("secret-value"), "{rendered}");

        set_enabled(false);
        clear();
    }

    #[test]
    fn future_enqueues_are_flagged_delayed() {
        let _g = log_lock().lock().unwrap_or_else(|e| e.into_inner());
        set_enabled(true);
        clear();
        let later = iso_from_unix(unix_now() + 300);
        record(&JobDoc::new(
            "LaterJob",
            serde_json::json!({}),
            "default",
            later,
        ));
        assert!(snapshot()[0].delayed);
        set_enabled(false);
        clear();
    }
}
