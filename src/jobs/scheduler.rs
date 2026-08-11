//! Cron evaluation and firing, on Soli's side.
//!
//! Cron definitions live in `_cron_jobs` (one row per name). Every poller tick
//! asks for rows whose `next_run_at` has passed and tries to claim each slot
//! with a compare-and-swap on that very field. Exactly one process wins a given
//! slot, so N app processes sharing a database still fire a schedule once.
//!
//! Expressions are the same six-field form SolidB used (`sec min hour dom mon
//! dow`), evaluated here with the `cron` crate.

use std::str::FromStr;

use super::{store, JobDoc};
use crate::db;
use crate::interpreter::builtins::model::crud;

/// Validate a cron expression and return the next firing time after `after`
/// (Unix seconds), as an ISO-8601 UTC string.
pub fn next_run_after(expression: &str, after_unix: i64) -> Result<String, String> {
    let schedule = cron::Schedule::from_str(expression).map_err(|e| {
        format!(
            "invalid cron expression {expression:?}: {e}. Soli uses six fields \
             (sec min hour day-of-month month day-of-week) — e.g. \"0 0 3 * * *\" \
             for 03:00 daily. The Cron.every/daily_at/hourly/weekly_at builders \
             emit the correct form."
        )
    })?;
    let after = chrono::DateTime::<chrono::Utc>::from_timestamp(after_unix, 0)
        .ok_or_else(|| "cron: timestamp out of range".to_string())?;
    let next = schedule
        .after(&after)
        .next()
        .ok_or_else(|| format!("cron expression {expression:?} has no future occurrence"))?;
    Ok(next.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// Validate an expression without computing a schedule position.
pub fn validate(expression: &str) -> Result<(), String> {
    next_run_after(expression, super::unix_now()).map(|_| ())
}

/// Fire every cron row that is due. Returns the number of jobs enqueued by
/// *this* process (rows lost to another process are not counted).
pub fn tick() -> Result<usize, String> {
    let now = super::now_iso();
    let rows = store::list_crons()?;
    let mut fired = 0usize;

    for row in rows {
        if row.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let Some(name) = row
            .get("name")
            .or_else(|| row.get("_key"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let Some(expression) = row.get("cron_expression").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(handler) = row.get("handler").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(next_run_at) = row.get("next_run_at").and_then(|v| v.as_str()) else {
            // No schedule position yet (freshly inserted by another process
            // mid-write): compute one next tick.
            continue;
        };
        if next_run_at > now.as_str() {
            continue;
        }

        // Advance from the slot we are firing, not from "now" — otherwise a
        // slow tick would skip intervening occurrences of a fast schedule.
        let anchor = parse_iso(next_run_at).unwrap_or_else(super::unix_now);
        let following = match next_run_after(expression, anchor) {
            Ok(next) => next,
            Err(e) => {
                eprintln!("[cron] {name}: {e}");
                continue;
            }
        };

        // Claim the slot: only the process whose CAS lands enqueues the job.
        let patch = serde_json::json!({
            "last_run_at": now,
            "next_run_at": following,
        });
        let won = claim_slot(name, next_run_at, patch)?;
        if !won {
            continue;
        }

        let args = row.get("args").cloned().unwrap_or(serde_json::json!({}));
        let mut job = JobDoc::new(handler, args, &super::config().default_queue, now.clone());
        job.cron_name = Some(name.to_string());
        store::enqueue(&job)?;
        fired += 1;
    }
    Ok(fired)
}

/// Compare-and-swap one cron row's schedule position.
fn claim_slot(
    name: &str,
    expected_next_run_at: &str,
    patch: serde_json::Value,
) -> Result<bool, String> {
    if db::is_sql() {
        return db::sql::claim_cron_slot(name, expected_next_run_at, patch);
    }
    // SoliDB: If-Match CAS on the row's _rev. A mismatch means another process
    // updated the row first, i.e. it won this slot.
    let Some(current) = store::get_cron(name)? else {
        return Ok(false);
    };
    // Re-check under the read we are about to CAS on: if the position already
    // moved, someone else fired this slot.
    if current.get("next_run_at").and_then(|v| v.as_str()) != Some(expected_next_run_at) {
        return Ok(false);
    }
    let Some(rev) = current.get("_rev").and_then(|v| v.as_str()) else {
        // No _rev to guard with — fall back to a plain update. Single-process
        // dev setups behave correctly; multi-process needs a versioned store.
        store::update_cron(name, patch)?;
        return Ok(true);
    };
    match crud::exec_update_if_match(super::CRON_COLLECTION, name, patch, rev) {
        Ok(_) => Ok(true),
        Err(e) if crud::is_rev_mismatch_error(&e) => Ok(false),
        Err(e) => Err(format!("cron slot CAS failed for {name}: {e}")),
    }
}

/// Register (or refresh) a cron definition, computing its first firing time.
pub fn upsert(
    name: &str,
    expression: &str,
    handler: &str,
    args: serde_json::Value,
) -> Result<String, String> {
    validate(expression)?;
    let next = next_run_after(expression, super::unix_now())?;
    store::upsert_cron(name, expression, handler, args, &next)
}

/// Parse one of our own fixed-width ISO stamps back to Unix seconds.
fn parse_iso(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_field_expressions_are_accepted() {
        // The forms the Cron.* builders emit.
        for expr in [
            "0 */5 * * * *",
            "0 0 3 * * *",
            "0 0 * * * *",
            "0 0 9 * * Mon",
            "0 0 0 */2 * *",
        ] {
            assert!(validate(expr).is_ok(), "{expr} should be valid");
        }
    }

    #[test]
    fn five_field_unix_expressions_are_rejected_with_guidance() {
        // Users copying crontab lines get told what shape Soli wants.
        let err = validate("*/5 * * * *").expect_err("5-field must be rejected");
        assert!(err.contains("six fields"), "{err}");
        assert!(err.contains("Cron.every"), "{err}");
    }

    #[test]
    fn next_run_advances_and_is_fixed_width() {
        // 03:00 daily, evaluated from a known instant (2021-01-01T00:00:00Z).
        let base = 1_609_459_200;
        let next = next_run_after("0 0 3 * * *", base).unwrap();
        assert_eq!(next, "2021-01-01T03:00:00Z");
        // Firing from that slot yields the following day, never the same slot.
        let following = next_run_after("0 0 3 * * *", parse_iso(&next).unwrap()).unwrap();
        assert_eq!(following, "2021-01-02T03:00:00Z");
    }

    #[test]
    fn every_minute_schedule_steps_one_minute() {
        let base = 1_609_459_200; // midnight
        let next = next_run_after("0 * * * * *", base).unwrap();
        assert_eq!(next, "2021-01-01T00:01:00Z");
    }

    #[test]
    fn iso_round_trips_through_parse() {
        let stamp = super::super::iso_from_unix(1_609_459_200);
        assert_eq!(parse_iso(&stamp), Some(1_609_459_200));
        assert_eq!(parse_iso("not-a-date"), None);
    }
}
