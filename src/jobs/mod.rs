//! Soli-side background job engine.
//!
//! Jobs are ordinary documents in the `_jobs` collection/table on the default
//! database connection, so the engine works identically on SoliDB, Postgres,
//! MySQL, and SQLite. Soli owns the whole lifecycle: enqueue, claim, execute,
//! retry, and cron scheduling. Nothing calls back into the app over HTTP.
//!
//! - [`store`] — backend-agnostic row operations.
//! - [`claim`] — atomic claim (Postgres `SKIP LOCKED`, MySQL token-claim,
//!   SQLite write-lock, SoliDB `If-Match` CAS) so several processes can poll one
//!   queue safely.
//! - [`scheduler`] — cron evaluation and single-winner firing.
//! - [`engine`] — the poller thread, retry/backoff, and the native webhook job.

pub mod claim;
pub mod engine;
pub mod scheduler;
pub mod store;

use serde::{Deserialize, Serialize};

/// Collection/table holding queued jobs.
pub const JOBS_COLLECTION: &str = "_jobs";
/// Collection/table holding cron definitions (`_key` = cron name).
pub const CRON_COLLECTION: &str = "_cron_jobs";
/// Handler name of the built-in outbound-webhook job.
pub const WEBHOOK_HANDLER: &str = "__WebhookDelivery";

/// Lifecycle state of a job row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    /// Enqueued for a future `run_at`.
    Scheduled,
    /// Due (or immediately runnable).
    Pending,
    /// Claimed by a worker; `locked_until` is its lease.
    Running,
    /// Failed and awaiting the next retry at `run_at`.
    Failed,
    /// Exhausted its retries (or cancelled) — terminal.
    Dead,
    /// Completed successfully — terminal, pruned after the retention window.
    Done,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Scheduled => "scheduled",
            JobState::Pending => "pending",
            JobState::Running => "running",
            JobState::Failed => "failed",
            JobState::Dead => "dead",
            JobState::Done => "done",
        }
    }

    /// Whether a row in this state can still be cancelled.
    pub fn is_cancellable(self) -> bool {
        matches!(
            self,
            JobState::Scheduled | JobState::Pending | JobState::Failed
        )
    }

    /// Whether a failed or dead row can be put back on the queue.
    pub fn is_retryable(self) -> bool {
        matches!(self, JobState::Failed | JobState::Dead)
    }
}

/// Outbound-HTTP payload of a `__WebhookDelivery` job.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WebhookSpec {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<serde_json::Value>,
}

/// One job row. Serialized as the document body; `_key` is the job id.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobDoc {
    #[serde(rename = "_key")]
    pub key: String,
    pub handler: String,
    #[serde(default)]
    pub args: serde_json::Value,
    pub queue: String,
    #[serde(default)]
    pub priority: i64,
    pub state: JobState,
    #[serde(default)]
    pub attempts: i64,
    pub max_retries: i64,
    pub run_at: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook: Option<WebhookSpec>,
}

impl JobDoc {
    /// A fresh row for `handler`, due at `run_at` (ISO-8601 UTC).
    pub fn new(handler: &str, args: serde_json::Value, queue: &str, run_at: String) -> Self {
        let now = now_iso();
        // A future run_at is `scheduled` purely for readability — the claim
        // predicate keys off run_at, not the state name.
        let state = if run_at > now {
            JobState::Scheduled
        } else {
            JobState::Pending
        };
        Self {
            key: uuid::Uuid::new_v4().to_string(),
            handler: handler.to_string(),
            args,
            queue: queue.to_string(),
            priority: 0,
            state,
            attempts: 0,
            max_retries: config().max_retries,
            run_at,
            created_at: now,
            locked_by: None,
            locked_until: None,
            last_error: None,
            finished_at: None,
            cron_name: None,
            webhook: None,
        }
    }

    /// Apply the pass-through option keys shared by every enqueue entry point
    /// (`priority`, `max_retries`, `run_at`). Unknown keys are ignored so the
    /// public option hash stays forward-compatible.
    pub fn apply_opts(&mut self, opts: &serde_json::Value) {
        if let Some(map) = opts.as_object() {
            if let Some(p) = map.get("priority").and_then(|v| v.as_i64()) {
                self.priority = p;
            }
            if let Some(r) = map.get("max_retries").and_then(|v| v.as_i64()) {
                self.max_retries = r.max(0);
            }
            if let Some(s) = map.get("run_at").and_then(|v| v.as_str()) {
                self.run_at = s.to_string();
                self.state = if self.run_at > now_iso() {
                    JobState::Scheduled
                } else {
                    JobState::Pending
                };
            }
        }
    }

    pub fn to_json(&self) -> Result<serde_json::Value, String> {
        serde_json::to_value(self).map_err(|e| format!("job serialize: {e}"))
    }

    pub fn from_json(json: serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(json).map_err(|e| format!("job parse: {e}"))
    }
}

/// Poll interval used when the server runs with `--dev` and the operator has
/// not set `SOLI_JOBS_POLL_MS`. A dev box often has several apps open at once,
/// each scaffolded with an `app/jobs/` directory it never actually uses, and
/// every tick costs a lease-renew + cron + claim round-trip against the shared
/// database. Five seconds keeps job work observable while cutting that idle
/// chatter by 5x.
pub const DEV_POLL_MS: u64 = 5_000;

/// Engine tunables, read from the environment once per process.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Poll interval in milliseconds.
    pub poll_ms: u64,
    /// Lease length: a claimed job is reclaimable this long after its claim.
    pub lease_secs: i64,
    /// Default retry budget for a new job.
    pub max_retries: i64,
    /// How long completed rows are kept before pruning (seconds).
    pub retention_secs: i64,
    /// Default queue name for enqueues that don't name one.
    pub default_queue: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            poll_ms: 1000,
            lease_secs: 60,
            max_retries: 3,
            retention_secs: 7 * 86_400,
            default_queue: "default".to_string(),
        }
    }
}

impl EngineConfig {
    fn from_env() -> Self {
        let d = Self::default();
        Self {
            poll_ms: env_num("SOLI_JOBS_POLL_MS", d.poll_ms as i64).max(50) as u64,
            lease_secs: env_num("SOLI_JOBS_LEASE_SECS", d.lease_secs).max(5),
            max_retries: env_num("SOLI_JOBS_MAX_RETRIES", d.max_retries).max(0),
            retention_secs: env_num("SOLI_JOBS_RETENTION_SECS", d.retention_secs).max(0),
            default_queue: std::env::var("SOLI_JOBS_DEFAULT_QUEUE")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or(d.default_queue),
        }
    }
}

/// Poll interval for a process running in dev mode: [`DEV_POLL_MS`] unless the
/// operator pinned `SOLI_JOBS_POLL_MS`, which always wins.
pub fn dev_poll_ms() -> u64 {
    match std::env::var("SOLI_JOBS_POLL_MS") {
        Ok(v) if v.parse::<i64>().is_ok() => config().poll_ms,
        _ => DEV_POLL_MS,
    }
}

fn env_num(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}

/// Process-wide engine config.
pub fn config() -> &'static EngineConfig {
    static CFG: std::sync::OnceLock<EngineConfig> = std::sync::OnceLock::new();
    CFG.get_or_init(EngineConfig::from_env)
}

/// Identity stamped into `locked_by`, so an operator can tell which process
/// holds a lease: `host:pid`.
pub fn worker_identity() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let host = std::env::var("HOSTNAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "local".to_string());
        format!("{host}:{}", std::process::id())
    })
}

/// Current time as a fixed-width ISO-8601 UTC string. Fixed width matters:
/// the claim predicates compare these lexicographically.
pub fn now_iso() -> String {
    iso_from_unix(unix_now())
}

pub fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn iso_from_unix(unix_seconds: i64) -> String {
    use chrono::{DateTime, Utc};
    let dt = DateTime::<Utc>::from_timestamp(unix_seconds, 0).unwrap_or_else(Utc::now);
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Retry delay after `attempts` failures: exponential with a deterministic
/// spread so a burst of sibling failures doesn't retry in lockstep.
pub fn backoff_secs(attempts: i64, key: &str) -> i64 {
    const BASE: i64 = 5;
    const CAP: i64 = 3600;
    let exp = attempts.clamp(1, 16) - 1;
    let raw = BASE.saturating_mul(1_i64 << exp).min(CAP);
    // Jitter up to 20% of the delay, derived from the job key so it needs no RNG.
    let spread = (raw / 5).max(1);
    let hash = key
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    (raw + (hash % spread as u64) as i64).min(CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_strings_round_trip_through_serde() {
        for state in [
            JobState::Scheduled,
            JobState::Pending,
            JobState::Running,
            JobState::Failed,
            JobState::Dead,
            JobState::Done,
        ] {
            let json = serde_json::to_value(state).unwrap();
            assert_eq!(json.as_str(), Some(state.as_str()));
            let back: JobState = serde_json::from_value(json).unwrap();
            assert_eq!(back, state);
        }
    }

    #[test]
    fn only_pre_terminal_states_are_cancellable() {
        assert!(JobState::Pending.is_cancellable());
        assert!(JobState::Scheduled.is_cancellable());
        assert!(JobState::Failed.is_cancellable());
        // A running job holds a lease and a live worker; dead/done are terminal.
        assert!(!JobState::Running.is_cancellable());
        assert!(!JobState::Dead.is_cancellable());
        assert!(!JobState::Done.is_cancellable());
    }

    #[test]
    fn only_failed_and_dead_are_retryable() {
        assert!(JobState::Failed.is_retryable());
        assert!(JobState::Dead.is_retryable());
        assert!(!JobState::Pending.is_retryable());
        assert!(!JobState::Scheduled.is_retryable());
        assert!(!JobState::Running.is_retryable());
        assert!(!JobState::Done.is_retryable());
    }

    #[test]
    fn job_doc_round_trips_and_defaults_to_pending() {
        let doc = JobDoc::new(
            "EmailJob",
            serde_json::json!({"to": "a@b.c"}),
            "mail",
            now_iso(),
        );
        assert_eq!(doc.state, JobState::Pending);
        let json = doc.to_json().unwrap();
        assert_eq!(json["handler"], "EmailJob");
        assert_eq!(json["queue"], "mail");
        // Absent options serialize away rather than as nulls.
        assert!(json.get("locked_by").is_none());
        let back = JobDoc::from_json(json).unwrap();
        assert_eq!(back.key, doc.key);
        assert_eq!(back.args["to"], "a@b.c");
    }

    #[test]
    fn future_run_at_is_scheduled() {
        let later = iso_from_unix(unix_now() + 3600);
        let doc = JobDoc::new("EmailJob", serde_json::json!({}), "default", later);
        assert_eq!(doc.state, JobState::Scheduled);
    }

    #[test]
    fn apply_opts_reads_priority_retries_and_run_at() {
        let mut doc = JobDoc::new("J", serde_json::json!({}), "default", now_iso());
        let later = iso_from_unix(unix_now() + 600);
        doc.apply_opts(&serde_json::json!({
            "priority": 5, "max_retries": 9, "run_at": later, "unknown": "ignored"
        }));
        assert_eq!(doc.priority, 5);
        assert_eq!(doc.max_retries, 9);
        assert_eq!(doc.state, JobState::Scheduled);
    }

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        let first = backoff_secs(1, "k");
        let second = backoff_secs(2, "k");
        let third = backoff_secs(3, "k");
        assert!((5..10).contains(&first), "first={first}");
        assert!(second > first, "{second} > {first}");
        assert!(third > second, "{third} > {second}");
        // Never exceeds the hour cap, however many attempts.
        assert!(backoff_secs(50, "k") <= 3600);
    }

    #[test]
    fn backoff_jitter_varies_by_key_not_by_call() {
        // Deterministic per key (no RNG — resume/replay safe) but spread across keys.
        assert_eq!(backoff_secs(4, "job-a"), backoff_secs(4, "job-a"));
        let keys = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let distinct: std::collections::HashSet<i64> =
            keys.iter().map(|k| backoff_secs(6, k)).collect();
        assert!(distinct.len() > 1, "jitter should differ across job keys");
    }

    #[test]
    fn iso_timestamps_are_fixed_width_for_lexicographic_compare() {
        // The SQL claim predicates compare run_at as text, so width must not vary.
        let a = iso_from_unix(1);
        let b = iso_from_unix(1_900_000_000);
        assert_eq!(a.len(), b.len());
        assert!(a < b);
    }
}
