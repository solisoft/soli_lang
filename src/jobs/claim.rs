//! Atomic job claiming.
//!
//! Several processes may poll one queue, so claiming must be atomic or a job
//! runs twice. Each backend has a different primitive:
//!
//! - Postgres: `UPDATE … WHERE _key IN (SELECT … FOR UPDATE SKIP LOCKED)`.
//! - MySQL: a unique claim token stamped by one `UPDATE … LIMIT n`, then read
//!   back by token (MySQL cannot self-reference a table in an UPDATE subquery).
//! - SoliDB: fetch due candidates, then `If-Match` CAS per row; a rev mismatch
//!   means another process won that row, so it is skipped.
//!
//! All three also reclaim `running` rows whose `locked_until` has passed — that
//! is how a job survives the crash of the worker that held it.

use super::{JobDoc, JobState, JOBS_COLLECTION};
use crate::db;
use crate::interpreter::builtins::model::crud;

/// Claim up to `batch` due jobs, marking each `running` with a fresh lease and
/// incrementing its attempt counter. Returns the claimed rows.
pub fn claim(batch: usize) -> Result<Vec<JobDoc>, String> {
    if batch == 0 {
        return Ok(Vec::new());
    }
    let cfg = super::config();
    let now = super::now_iso();
    let locked_until = super::iso_from_unix(super::unix_now() + cfg.lease_secs);
    let worker = super::worker_identity();

    let rows = if db::is_sql() {
        db::sql::claim_jobs(&now, worker, &locked_until, batch)?
    } else {
        claim_solidb(&now, worker, &locked_until, batch)?
    };

    // A row that fails to parse would otherwise wedge the poller, re-claimed
    // every tick forever. Skip it loudly instead.
    let mut jobs = Vec::with_capacity(rows.len());
    for row in rows {
        match JobDoc::from_json(row) {
            Ok(doc) => jobs.push(doc),
            Err(e) => eprintln!("[jobs] skipping unparsable job row: {e}"),
        }
    }
    Ok(jobs)
}

/// SoliDB claim: select due candidates, then win each one with an `If-Match`
/// compare-and-swap on its `_rev`.
fn claim_solidb(
    now: &str,
    worker: &str,
    locked_until: &str,
    batch: usize,
) -> Result<Vec<serde_json::Value>, String> {
    // Over-fetch: contended rows are lost to other processes, so asking for
    // exactly `batch` would usually come back short under concurrency.
    let candidate_limit = (batch * 4).min(200);
    let sdbql = format!(
        "FOR doc IN {coll} \
         FILTER (doc.state IN [\"pending\", \"scheduled\", \"failed\"] AND doc.run_at <= \"{now}\") \
             OR (doc.state == \"running\" AND doc.locked_until < \"{now}\") \
         SORT doc.priority DESC, doc.run_at ASC \
         LIMIT {candidate_limit} RETURN doc",
        coll = JOBS_COLLECTION,
    );
    let candidates = crud::exec_query(JOBS_COLLECTION, sdbql)?;

    let mut claimed = Vec::new();
    for candidate in candidates {
        if claimed.len() >= batch {
            break;
        }
        let Some(key) = candidate.get("_key").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(rev) = candidate.get("_rev").and_then(|v| v.as_str()) else {
            continue;
        };
        let attempts = candidate
            .get("attempts")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            + 1;
        let patch = serde_json::json!({
            "state": JobState::Running.as_str(),
            "locked_by": worker,
            "locked_until": locked_until,
            "attempts": attempts,
        });
        match crud::exec_update_if_match(JOBS_COLLECTION, key, patch, rev) {
            Ok(_) => {
                // Return the row as it now stands, so the caller sees the
                // incremented attempt count and its own lease.
                let mut won = candidate.clone();
                if let Some(map) = won.as_object_mut() {
                    map.insert(
                        "state".to_string(),
                        serde_json::json!(JobState::Running.as_str()),
                    );
                    map.insert("locked_by".to_string(), serde_json::json!(worker));
                    map.insert("locked_until".to_string(), serde_json::json!(locked_until));
                    map.insert("attempts".to_string(), serde_json::json!(attempts));
                }
                claimed.push(won);
            }
            // Lost the race for this row — another poller claimed it first.
            Err(e) if crud::is_rev_mismatch_error(&e) => continue,
            Err(e) => return Err(format!("solidb claim failed: {e}")),
        }
    }
    Ok(claimed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_batch_never_touches_the_database() {
        // Guards the poller's "no idle workers" tick: it must be a pure no-op,
        // not a query (and not a panic when no DB is configured in tests).
        assert!(claim(0).unwrap().is_empty());
    }
}

/// Integration tests for the Postgres claim primitive. These need a reachable
/// Postgres; they skip (with a note) otherwise, mirroring `db::postgres`'s
/// integration suite.
#[cfg(all(test, feature = "postgres"))]
mod integration_tests {
    use super::*;
    use crate::db::registry::{
        clear_registry_override, registry_test_lock, set_registry_for_tests, ConnectionRegistry,
        ConnectionSpec,
    };
    use crate::db::Adapter;
    use crate::jobs::{iso_from_unix, now_iso, store, unix_now};
    use std::collections::HashMap;

    fn with_pg(f: impl FnOnce()) {
        let _g = registry_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let url = std::env::var("PG_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()
            .filter(|u| u.starts_with("postgres"))
            .unwrap_or_else(|| "postgres://soli@localhost:5432/soli_test".into());
        let mut connections = HashMap::new();
        connections.insert(
            "primary".into(),
            ConnectionSpec {
                name: "primary".into(),
                adapter: Adapter::Postgres,
                url: Some(url),
                solidb_host: None,
                solidb_database: None,
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: Some(5),
            },
        );
        set_registry_for_tests(ConnectionRegistry {
            default: "primary".into(),
            connections,
            from_file: false,
        });
        struct ClearOnDrop;
        impl Drop for ClearOnDrop {
            fn drop(&mut self) {
                clear_registry_override();
            }
        }
        let _clear = ClearOnDrop;
        f();
    }

    /// True when a live Postgres backed this run; tests skip otherwise.
    fn pg_ready() -> bool {
        if db::sql::ensure_connected().is_err() {
            eprintln!("skip: postgres not reachable");
            return false;
        }
        let _ = db::sql::drop_table(JOBS_COLLECTION);
        db::sql::ensure_table(JOBS_COLLECTION).expect("ensure _jobs");
        true
    }

    #[test]
    fn claim_is_exclusive_priority_ordered_and_reclaims_expired_leases() {
        with_pg(|| {
            if !pg_ready() {
                return;
            }

            // Two due jobs, one higher priority.
            let mut low = JobDoc::new("LowJob", serde_json::json!({}), "default", now_iso());
            low.priority = 0;
            let mut high = JobDoc::new("HighJob", serde_json::json!({}), "default", now_iso());
            high.priority = 10;
            store::enqueue(&low).expect("enqueue low");
            store::enqueue(&high).expect("enqueue high");

            // Priority wins the single slot.
            let first = claim(1).expect("claim 1");
            assert_eq!(first.len(), 1);
            assert_eq!(first[0].handler, "HighJob");
            assert_eq!(first[0].attempts, 1, "attempts increment at claim time");
            assert_eq!(first[0].state, JobState::Running);

            // The claimed row is now invisible to other pollers; only the
            // remaining job is claimable.
            let second = claim(5).expect("claim rest");
            assert_eq!(second.len(), 1, "a running lease must not be re-claimed");
            assert_eq!(second[0].handler, "LowJob");

            // Nothing left to claim.
            assert!(claim(5).expect("claim empty").is_empty());

            // Expire the high-priority job's lease: it becomes claimable again
            // (crash recovery) and its attempt count advances.
            let past = iso_from_unix(unix_now() - 3600);
            db::sql::update(
                JOBS_COLLECTION,
                &high.key,
                serde_json::json!({ "locked_until": past }),
                true,
            )
            .expect("expire lease");
            let reclaimed = claim(5).expect("reclaim");
            assert_eq!(reclaimed.len(), 1);
            assert_eq!(reclaimed[0].key, high.key);
            assert_eq!(reclaimed[0].attempts, 2, "reclaim counts the lost attempt");

            let _ = db::sql::drop_table(JOBS_COLLECTION);
        });
    }

    #[test]
    fn future_jobs_are_not_claimed_until_due() {
        with_pg(|| {
            if !pg_ready() {
                return;
            }

            let later = iso_from_unix(unix_now() + 3600);
            let scheduled = JobDoc::new("LaterJob", serde_json::json!({}), "default", later);
            assert_eq!(scheduled.state, JobState::Scheduled);
            store::enqueue(&scheduled).expect("enqueue scheduled");

            assert!(
                claim(5).expect("claim").is_empty(),
                "a job scheduled in the future must not be claimed"
            );

            // Make it due; now it claims.
            db::sql::update(
                JOBS_COLLECTION,
                &scheduled.key,
                serde_json::json!({ "run_at": now_iso() }),
                true,
            )
            .expect("make due");
            assert_eq!(claim(5).expect("claim due").len(), 1);

            let _ = db::sql::drop_table(JOBS_COLLECTION);
        });
    }

    #[test]
    fn complete_and_fail_move_rows_through_the_lifecycle() {
        with_pg(|| {
            if !pg_ready() {
                return;
            }

            // Success path: done, lease cleared.
            let ok_job = JobDoc::new("OkJob", serde_json::json!({}), "default", now_iso());
            store::enqueue(&ok_job).expect("enqueue ok");
            let claimed = claim(1).expect("claim ok");
            store::complete(&claimed[0].key).expect("complete");
            let done = store::get(&ok_job.key).expect("get").expect("row");
            assert_eq!(done.state, JobState::Done);
            assert!(done.locked_until.is_none() && done.finished_at.is_some());
            // A completed job is never claimed again.
            assert!(claim(5).expect("claim after done").is_empty());

            // Failure path: retry scheduled while budget remains, then dead.
            let mut retry_job =
                JobDoc::new("FlakyJob", serde_json::json!({}), "default", now_iso());
            retry_job.max_retries = 1;
            store::enqueue(&retry_job).expect("enqueue flaky");

            let first = claim(1).expect("claim flaky");
            assert!(
                store::fail(&first[0], "boom").expect("fail 1"),
                "attempt 1 of max_retries 1 must be retryable"
            );
            let after = store::get(&retry_job.key).expect("get").expect("row");
            assert_eq!(after.state, JobState::Failed);
            assert!(after.run_at > now_iso(), "retry is delayed");
            assert_eq!(after.last_error.as_deref(), Some("boom"));

            // Force it due again, claim (attempts -> 2), and exhaust the budget.
            db::sql::update(
                JOBS_COLLECTION,
                &retry_job.key,
                serde_json::json!({ "run_at": now_iso() }),
                true,
            )
            .expect("make due");
            let second = claim(1).expect("claim retry");
            assert_eq!(second[0].attempts, 2);
            assert!(
                !store::fail(&second[0], "boom again").expect("fail 2"),
                "exceeding max_retries must bury the job"
            );
            let buried = store::get(&retry_job.key).expect("get").expect("row");
            assert_eq!(buried.state, JobState::Dead);
            assert!(claim(5).expect("claim after dead").is_empty());

            let _ = db::sql::drop_table(JOBS_COLLECTION);
        });
    }

    #[test]
    fn cancel_refuses_running_jobs_and_removes_pending_ones() {
        with_pg(|| {
            if !pg_ready() {
                return;
            }

            let pending = JobDoc::new("P", serde_json::json!({}), "default", now_iso());
            store::enqueue(&pending).expect("enqueue");
            assert!(store::cancel(&pending.key).expect("cancel pending"));
            assert!(store::get(&pending.key).expect("get").is_none());
            // Cancelling an unknown id is a benign false, not an error.
            assert!(!store::cancel("no-such-job").expect("cancel missing"));

            let running = JobDoc::new("R", serde_json::json!({}), "default", now_iso());
            store::enqueue(&running).expect("enqueue running");
            claim(1).expect("claim");
            let err = store::cancel(&running.key).expect_err("running must not cancel");
            assert!(err.contains("running"), "{err}");

            let _ = db::sql::drop_table(JOBS_COLLECTION);
        });
    }
}
