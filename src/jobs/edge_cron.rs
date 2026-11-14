//! `static cron` on the edge build: Cloudflare Cron Triggers.
//!
//! A Worker has no poller to read `_cron_jobs` with (see [`super::scheduler`]),
//! but Cloudflare calls its `scheduled()` handler at the times
//! `[triggers] crons = [...]` in `wrangler.toml` lists. So on the edge the job
//! classes' `static cron` declarations are kept here at boot, and a trigger runs
//! the jobs whose expression it is (`serve::edge::run_cron`).
//!
//! Soli's expressions have six fields, seconds first (`0 15 3 * * *`);
//! Cloudflare's have five, in UTC, to the minute (`15 3 * * *`). [`trigger`]
//! converts one into the other, and refuses a schedule that needs seconds:
//! `soli edge build` writes the triggers with it, and the Worker matches the
//! trigger it is called with against the same conversion, so the two cannot
//! drift.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// A job class's `static cron`, as the edge boot found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeCron {
    /// The class's cron name (`nightly_report_job`).
    pub name: String,
    /// The class that runs (`NightlyReportJob`).
    pub class: String,
    /// The six-field expression the class declares.
    pub expression: String,
}

static CRONS: Mutex<Vec<EdgeCron>> = Mutex::new(Vec::new());

/// Set by `serve::edge::boot`: the app's `static cron` declarations are kept
/// here for Cron Triggers instead of being written to the job table.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// This process boots the edge build: keep the crons here.
pub fn enable() {
    ENABLED.store(true, Ordering::Release);
}

/// Whether [`enable`] was called.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// The Cloudflare Cron Trigger for a six-field Soli expression: the same
/// schedule without its seconds field. Refused when the seconds field is not
/// `0` — a trigger fires on the minute — or the expression does not have six
/// fields.
pub fn trigger(expression: &str) -> Result<String, String> {
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if fields.len() != 6 {
        return Err(format!(
            "{expression:?} is not a six-field cron expression (sec min hour day month weekday)"
        ));
    }
    if !matches!(fields[0], "0" | "00") {
        return Err(format!(
            "{expression:?} fires on seconds, and Cloudflare Cron Triggers fire on the minute: \
             start it with 0"
        ));
    }
    Ok(fields[1..].join(" "))
}

/// The `static cron` declarations of the app the edge booted.
pub fn set(crons: Vec<EdgeCron>) {
    *CRONS.lock().unwrap_or_else(|e| e.into_inner()) = crons;
}

/// Every declaration, in load order.
pub fn all() -> Vec<EdgeCron> {
    CRONS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The declarations a Cron Trigger stands for: those whose expression converts
/// to it. Several jobs may share one trigger; all of them run.
pub fn matching(cron: &str) -> Vec<EdgeCron> {
    let wanted: Vec<&str> = cron.split_whitespace().collect();
    all()
        .into_iter()
        .filter(|entry| {
            trigger(&entry.expression)
                .is_ok_and(|t| t.split_whitespace().eq(wanted.iter().copied()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_soli_expression_becomes_a_trigger_without_its_seconds() {
        assert_eq!(trigger("0 15 3 * * *").unwrap(), "15 3 * * *");
        assert_eq!(trigger("0 0 9 * * Mon").unwrap(), "0 9 * * Mon");
        assert_eq!(trigger("0  */5 *  * * *").unwrap(), "*/5 * * * *");
        assert!(trigger("30 * * * * *")
            .unwrap_err()
            .contains("on the minute"));
        assert!(trigger("15 3 * * *").unwrap_err().contains("six-field"));
    }

    #[test]
    fn a_trigger_runs_every_job_that_declares_it() {
        let entry = |class: &str, expression: &str| EdgeCron {
            name: class.to_lowercase(),
            class: class.to_string(),
            expression: expression.to_string(),
        };
        set(vec![
            entry("Nightly", "0 0 3 * * *"),
            entry("Also", "0 0 3 * * *"),
            entry("Hourly", "0 0 * * * *"),
            entry("Seconds", "30 0 3 * * *"),
        ]);
        let classes = |cron: &str| {
            matching(cron)
                .into_iter()
                .map(|e| e.class)
                .collect::<Vec<_>>()
        };
        assert_eq!(classes("0 3 * * *"), ["Nightly", "Also"]);
        assert_eq!(classes("0  * * * *"), ["Hourly"]);
        assert!(classes("5 4 * * *").is_empty());
        set(Vec::new());
    }
}
