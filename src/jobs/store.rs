//! Backend-agnostic persistence for job and cron rows.
//!
//! Every operation routes on `db::is_sql()`: SQL connections go through the
//! document facade (`db::sql`), SoliDB through the model CRUD helpers. Both
//! store the same JSON shape, so [`JobDoc`] is the single source of truth.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};

use super::{JobDoc, JobState, CRON_COLLECTION, JOBS_COLLECTION};
use crate::db;
use crate::interpreter::builtins::model::crud;

/// Build a `ListQuery` over `table` with equality filters only — the portable
/// subset every SQL backend supports.
///
/// `filter_sdbql` must be the hash-equality form the compiler validates
/// (`doc.field == @field`, one clause per bind) or `None`; anything else is
/// rejected as raw SDBQL. It is derived from `eq_filters` so the two can't
/// drift apart.
fn list_query(
    table: &str,
    eq_filters: BTreeMap<String, serde_json::Value>,
    order_field: Option<&str>,
    limit: Option<usize>,
) -> db::ListQuery {
    let filter_sdbql = if eq_filters.is_empty() {
        None
    } else {
        Some(
            eq_filters
                .keys()
                .map(|field| format!("doc.{field} == @{field}"))
                .collect::<Vec<_>>()
                .join(" AND "),
        )
    };
    db::ListQuery {
        table: table.to_string(),
        eq_filters,
        hash_filter: None,
        filter_sdbql,
        having: None,
        exists_filters: Vec::new(),
        soft_delete: db::SqlSoftDeleteMode::WithDeleted,
        is_soft_delete_model: false,
        order_field: order_field.map(str::to_string),
        order_desc: false,
        limit,
        offset: None,
    }
}

/// Collections whose SQL indexes this process has already ensured, keyed by
/// `connection/collection`.
fn indexed() -> &'static Mutex<HashSet<String>> {
    static INDEXED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    INDEXED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Index the queue tables on a SQL connection, once per process.
///
/// The claim query filters on `state`/`run_at` and orders by `priority`, and it
/// runs every poll tick — unindexed, that is a full scan of every job ever
/// enqueued (failed and dead rows are kept deliberately). Index creation is
/// idempotent, but the *check* costs a round trip, so it happens once.
fn ensure_sql_indexes(collection: &str, fields: &[&str]) {
    let key = format!("{}/{collection}", db::active_connection_name());
    {
        let mut seen = indexed().lock().unwrap_or_else(|e| e.into_inner());
        if !seen.insert(key) {
            return;
        }
    }
    for field in fields {
        let name = format!("idx_{collection}_{field}");
        // A failure here is a performance problem, not a correctness one: the
        // queue still works on a table scan, so warn and carry on.
        if let Err(e) = db::sql::ensure_doc_index(collection, &[(*field).to_string()], &name, false)
        {
            eprintln!("[jobs] could not index {collection}.{field}: {e}");
        }
    }
}

/// Insert a job row. Returns the job id.
pub fn enqueue(doc: &JobDoc) -> Result<String, String> {
    let json = doc.to_json()?;
    if db::is_sql() {
        db::sql::ensure_table(JOBS_COLLECTION)?;
        ensure_sql_indexes(JOBS_COLLECTION, &["state", "run_at", "priority"]);
        db::sql::insert(JOBS_COLLECTION, Some(&doc.key), json)?;
    } else {
        crud::exec_insert(JOBS_COLLECTION, Some(&doc.key), json)?;
    }
    Ok(doc.key.clone())
}

/// Fetch one job row, or `None` when the id is unknown.
pub fn get(id: &str) -> Result<Option<JobDoc>, String> {
    let raw = if db::is_sql() {
        db::sql::get(JOBS_COLLECTION, id)?
    } else {
        // A missing document is a normal answer here, not an error.
        crud::exec_get(JOBS_COLLECTION, id).ok()
    };
    match raw {
        Some(json) => Ok(Some(JobDoc::from_json(json)?)),
        None => Ok(None),
    }
}

/// Merge `patch` into a job row.
fn patch_job(id: &str, patch: serde_json::Value) -> Result<(), String> {
    if db::is_sql() {
        db::sql::update(JOBS_COLLECTION, id, patch, true)?;
    } else {
        crud::exec_update(JOBS_COLLECTION, id, patch, true)?;
    }
    Ok(())
}

/// Rows in `queue`, newest scheduling first. Terminal rows are included so an
/// admin view can show recent history.
pub fn list(queue: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
    let mut eq = BTreeMap::new();
    if let Some(q) = queue {
        eq.insert(
            "queue".to_string(),
            serde_json::Value::String(q.to_string()),
        );
    }
    if db::is_sql() {
        let q = list_query(JOBS_COLLECTION, eq, Some("run_at"), Some(500));
        return db::sql::select(&q);
    }
    let filter = match queue {
        Some(q) => format!(" FILTER doc.queue == \"{}\"", escape_sdbql(q)),
        None => String::new(),
    };
    let sdbql = format!(
        "FOR doc IN {}{} SORT doc.run_at ASC LIMIT 500 RETURN doc",
        JOBS_COLLECTION, filter
    );
    crud::exec_query(JOBS_COLLECTION, sdbql)
}

/// Distinct queue names across non-terminal rows.
pub fn queues() -> Result<Vec<serde_json::Value>, String> {
    let rows = list(None)?;
    let mut seen: Vec<String> = Vec::new();
    for row in rows {
        let terminal = matches!(
            row.get("state").and_then(|v| v.as_str()),
            Some("done") | Some("dead")
        );
        if terminal {
            continue;
        }
        if let Some(q) = row.get("queue").and_then(|v| v.as_str()) {
            if !seen.iter().any(|s| s == q) {
                seen.push(q.to_string());
            }
        }
    }
    Ok(seen.into_iter().map(serde_json::Value::String).collect())
}

/// Cancel a not-yet-running job. Errors when it is already running or terminal
/// so a caller never believes it stopped work that is actually in flight.
pub fn cancel(id: &str) -> Result<bool, String> {
    let Some(doc) = get(id)? else {
        return Ok(false);
    };
    if !doc.state.is_cancellable() {
        return Err(format!(
            "job {id} is {} and cannot be cancelled",
            doc.state.as_str()
        ));
    }
    if db::is_sql() {
        db::sql::delete(JOBS_COLLECTION, id)?;
    } else {
        crud::exec_delete(JOBS_COLLECTION, id)?;
    }
    Ok(true)
}

/// Mark a job finished successfully.
pub fn complete(id: &str) -> Result<(), String> {
    patch_job(
        id,
        serde_json::json!({
            "state": JobState::Done.as_str(),
            "finished_at": super::now_iso(),
            "locked_by": serde_json::Value::Null,
            "locked_until": serde_json::Value::Null,
            "last_error": serde_json::Value::Null,
        }),
    )
}

/// Record a failure: schedule the next retry, or bury the job when its retry
/// budget is spent. Returns true when a retry was scheduled.
pub fn fail(doc: &JobDoc, error: &str) -> Result<bool, String> {
    let retryable = doc.attempts <= doc.max_retries;
    // Errors can be enormous (stack traces, HTML bodies); keep rows bounded.
    let trimmed = truncate_chars(error, 2000);
    if retryable {
        let delay = super::backoff_secs(doc.attempts, &doc.key);
        patch_job(
            &doc.key,
            serde_json::json!({
                "state": JobState::Failed.as_str(),
                "run_at": super::iso_from_unix(super::unix_now() + delay),
                "last_error": trimmed,
                "locked_by": serde_json::Value::Null,
                "locked_until": serde_json::Value::Null,
            }),
        )?;
    } else {
        patch_job(
            &doc.key,
            serde_json::json!({
                "state": JobState::Dead.as_str(),
                "finished_at": super::now_iso(),
                "last_error": trimmed,
                "locked_by": serde_json::Value::Null,
                "locked_until": serde_json::Value::Null,
            }),
        )?;
    }
    Ok(retryable)
}

/// Re-queue a `failed` or `dead` job so a worker will pick it up again.
///
/// Resets the lease and `finished_at` but keeps `attempts` and `last_error`
/// so the dashboard can still show why it died. Returns `false` when the id
/// is unknown.
pub fn retry(id: &str) -> Result<bool, String> {
    let Some(doc) = get(id)? else {
        return Ok(false);
    };
    if !doc.state.is_retryable() {
        return Err(format!(
            "job {id} is {} and cannot be retried",
            doc.state.as_str()
        ));
    }
    patch_job(
        id,
        serde_json::json!({
            "state": JobState::Pending.as_str(),
            "run_at": super::now_iso(),
            "locked_by": serde_json::Value::Null,
            "locked_until": serde_json::Value::Null,
            "finished_at": serde_json::Value::Null,
        }),
    )?;
    Ok(true)
}

/// Push the lease of an in-flight job out by `lease_secs`, so a long job is not
/// reclaimed by another poller while it is still running.
pub fn renew_lease(id: &str, lease_secs: i64) -> Result<(), String> {
    patch_job(
        id,
        serde_json::json!({
            "locked_until": super::iso_from_unix(super::unix_now() + lease_secs),
        }),
    )
}

/// Delete `done` rows finished before `older_than_iso`. Returns rows removed.
pub fn prune_done(older_than_iso: &str) -> Result<usize, String> {
    let rows = list(None)?;
    let mut removed = 0usize;
    for row in rows {
        let is_done = row.get("state").and_then(|v| v.as_str()) == Some("done");
        let old = row
            .get("finished_at")
            .and_then(|v| v.as_str())
            .is_some_and(|f| f < older_than_iso);
        if is_done && old {
            if let Some(key) = row.get("_key").and_then(|v| v.as_str()) {
                let deleted = if db::is_sql() {
                    db::sql::delete(JOBS_COLLECTION, key).is_ok()
                } else {
                    crud::exec_delete(JOBS_COLLECTION, key).is_ok()
                };
                if deleted {
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

// ---------- cron rows ----------

/// Upsert a cron definition keyed by `name`. The deterministic `_key` makes
/// concurrent boot-time upserts across workers converge instead of duplicating.
pub fn upsert_cron(
    name: &str,
    expression: &str,
    handler: &str,
    args: serde_json::Value,
    next_run_at: &str,
) -> Result<String, String> {
    let existing = get_cron(name)?;
    let doc = serde_json::json!({
        "_key": name,
        "name": name,
        "cron_expression": expression,
        "handler": handler,
        "args": args,
        "enabled": true,
        // Preserve schedule position on re-declare so a redeploy doesn't
        // re-fire a schedule that already ran in this window.
        "last_run_at": existing
            .as_ref()
            .and_then(|d| d.get("last_run_at").cloned())
            .unwrap_or(serde_json::Value::Null),
        "next_run_at": next_run_at,
    });
    if existing.is_some() {
        let patch = serde_json::json!({
            "cron_expression": expression,
            "handler": handler,
            "args": doc["args"].clone(),
            "enabled": true,
        });
        if db::is_sql() {
            db::sql::update(CRON_COLLECTION, name, patch, true)?;
        } else {
            crud::exec_update(CRON_COLLECTION, name, patch, true)?;
        }
    } else if db::is_sql() {
        db::sql::ensure_table(CRON_COLLECTION)?;
        ensure_sql_indexes(CRON_COLLECTION, &["next_run_at", "enabled"]);
        db::sql::insert(CRON_COLLECTION, Some(name), doc)?;
    } else {
        crud::exec_insert(CRON_COLLECTION, Some(name), doc)?;
    }
    Ok(name.to_string())
}

pub fn get_cron(name: &str) -> Result<Option<serde_json::Value>, String> {
    if db::is_sql() {
        return db::sql::get(CRON_COLLECTION, name);
    }
    Ok(crud::exec_get(CRON_COLLECTION, name).ok())
}

pub fn list_crons() -> Result<Vec<serde_json::Value>, String> {
    if db::is_sql() {
        let q = list_query(CRON_COLLECTION, BTreeMap::new(), Some("name"), Some(500));
        return db::sql::select(&q);
    }
    let sdbql = format!(
        "FOR doc IN {} SORT doc.name ASC LIMIT 500 RETURN doc",
        CRON_COLLECTION
    );
    crud::exec_query(CRON_COLLECTION, sdbql)
}

pub fn update_cron(name: &str, fields: serde_json::Value) -> Result<(), String> {
    if db::is_sql() {
        db::sql::update(CRON_COLLECTION, name, fields, true)?;
    } else {
        crud::exec_update(CRON_COLLECTION, name, fields, true)?;
    }
    Ok(())
}

pub fn delete_cron(name: &str) -> Result<(), String> {
    if db::is_sql() {
        db::sql::delete(CRON_COLLECTION, name)?;
    } else {
        crud::exec_delete(CRON_COLLECTION, name)?;
    }
    Ok(())
}

/// Escape a value interpolated into an SDBQL string literal.
fn escape_sdbql(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Truncate to `max` characters on a char boundary (never mid-UTF-8).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdbql_escaping_neutralizes_quotes_and_backslashes() {
        assert_eq!(escape_sdbql("plain"), "plain");
        assert_eq!(escape_sdbql("a\"b"), "a\\\"b");
        assert_eq!(escape_sdbql("a\\b"), "a\\\\b");
    }

    #[test]
    fn error_truncation_is_char_safe() {
        let long = "é".repeat(3000);
        let out = truncate_chars(&long, 2000);
        assert_eq!(out.chars().count(), 2001); // 2000 + ellipsis
        assert!(out.ends_with('…'));
        // Short strings pass through untouched.
        assert_eq!(truncate_chars("boom", 2000), "boom");
    }

    #[test]
    fn list_query_carries_eq_filters_and_order() {
        let mut eq = BTreeMap::new();
        eq.insert("queue".to_string(), serde_json::json!("mail"));
        let q = list_query(JOBS_COLLECTION, eq, Some("run_at"), Some(10));
        assert_eq!(q.table, JOBS_COLLECTION);
        assert_eq!(q.eq_filters.get("queue"), Some(&serde_json::json!("mail")));
        assert_eq!(q.order_field.as_deref(), Some("run_at"));
        assert_eq!(q.limit, Some(10));
        // Jobs must never be hidden by the soft-delete scope.
        assert!(matches!(q.soft_delete, db::SqlSoftDeleteMode::WithDeleted));
    }

    /// The SQL compiler rejects any `filter_sdbql` that isn't the hash-equality
    /// shape matching its binds. An unfiltered list must therefore carry no
    /// filter at all — a placeholder string made every unfiltered read
    /// (`Job.list()`, `Job.queues()`, the cron tick, pruning) fail on SQL.
    #[test]
    fn list_query_filters_are_compiler_portable() {
        use crate::db::sql_compile::assert_portable_filter;

        let unfiltered = list_query(JOBS_COLLECTION, BTreeMap::new(), Some("run_at"), Some(500));
        assert!(
            unfiltered.filter_sdbql.is_none(),
            "no filters → no filter string"
        );
        assert_portable_filter(unfiltered.filter_sdbql.as_deref(), &unfiltered.eq_filters)
            .expect("unfiltered list must compile on SQL");

        let mut eq = BTreeMap::new();
        eq.insert("queue".to_string(), serde_json::json!("mail"));
        let filtered = list_query(JOBS_COLLECTION, eq, Some("run_at"), None);
        assert_eq!(
            filtered.filter_sdbql.as_deref(),
            Some("doc.queue == @queue"),
            "filter text must be derived from the binds"
        );
        assert_portable_filter(filtered.filter_sdbql.as_deref(), &filtered.eq_filters)
            .expect("filtered list must compile on SQL");
    }
}
