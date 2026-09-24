//! Built-in error tracking: every request that fails with a 500 is grouped by
//! fingerprint and stored in the app's own database, for `/__soli/errors`.
//!
//! [`error_logging::log_production_error`](super::error_logging) is the one
//! place every failing request passes through, and it calls [`record`]. Recording
//! must never cost the request anything, so [`record`] only builds the sample and
//! hands it to a per-application writer thread over a bounded channel; a full
//! channel drops the sample and counts it rather than block a worker.
//!
//! The writer batches for [`FLUSH_EVERY`], folds repeats of the same error into
//! one update per group, and writes the `_soli_errors` collection through the
//! same document facade the job queue uses, so it works on SoliDB and the SQL
//! adapters alike. One document per group:
//!
//! ```text
//! { _key: <fingerprint>, message, location, status: open|resolved|ignored,
//!   count, first_seen, last_seen, resolved_at?, regressed_at?,
//!   last_request: "GET /orders/7",
//!   hourly: [ ["2026-09-24T18", 12], … the last HOURS_KEPT hours seen ],
//!   samples: [ newest first, at most MAX_SAMPLES ] }
//! ```
//!
//! A new occurrence of a `resolved` group reopens it and stamps `regressed_at`;
//! an `ignored` group keeps counting and stays ignored.
//!
//! Samples carry the request snapshot the stderr block already prints — auth
//! headers, secret-looking params and the body redacted — so the table holds
//! nothing the log did not.
//!
//! Counts are exact within one process (one writer per application). Several
//! processes writing the same group read-modify-write it independently, so a
//! concurrent burst across hosts can undercount; the group itself is never lost.
//!
//! On by default; `SOLI_ERRORS=off` disables it, and it stays off under
//! `APP_ENV=test` unless `SOLI_ERRORS=on`, so spec runs do not fill the table.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::error_pages::redacted_request_snapshot;
use super::tenant::TenantId;
use super::RequestData;
use crate::db;
use crate::interpreter::builtins::model::crud;

pub(crate) const ERRORS_COLLECTION: &str = "_soli_errors";

/// Occurrences kept per group, newest first.
pub(crate) const MAX_SAMPLES: usize = 5;

/// Samples waiting for the writer. A burst beyond this is dropped and counted.
const QUEUE_CAP: usize = 1024;

/// Hours of per-hour counts kept on a group, for the list's trend line.
pub(crate) const HOURS_KEPT: usize = 24;

/// How long the writer gathers samples before it writes them.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

const MAX_STACK_FRAMES: usize = 50;
const MAX_MESSAGE_CHARS: usize = 2000;
const MAX_ENV_CHARS: usize = 16 * 1024;

/// The statuses a group can be in. Also the whitelist the dashboard filters on.
pub(crate) const STATUSES: [&str; 3] = ["open", "resolved", "ignored"];

static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Samples dropped because the writer's queue was full, since the process
/// started.
pub(crate) fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

/// Whether failures are recorded at all.
pub(crate) fn enabled() -> bool {
    let setting = std::env::var("SOLI_ERRORS")
        .unwrap_or_default()
        .to_ascii_lowercase();
    match setting.as_str() {
        "off" | "0" | "false" | "no" => false,
        "on" | "1" | "true" | "yes" => true,
        _ => std::env::var("APP_ENV")
            .map(|v| v != "test")
            .unwrap_or(true),
    }
}

struct Occurrence {
    fingerprint: String,
    message: String,
    location: String,
    /// `GET /orders/7`: what the list shows under the message.
    request_line: String,
    at: String,
    sample: serde_json::Value,
}

/// Queue one failed request for the writer. Never blocks, never fails the
/// request: a missing runtime handle or a full queue drops the sample.
pub(super) fn record(
    request_id: &str,
    request_data: &RequestData,
    error_msg: &str,
    stack_trace: &[String],
    env_json: Option<&str>,
) {
    if !enabled() {
        return;
    }
    let Some(sender) = sender_for_current_tenant() else {
        return;
    };
    let message = truncate_chars(error_msg, MAX_MESSAGE_CHARS);
    // Paths relative to the app, so the same bug deployed to another
    // directory stays in its group.
    let root = format!("{}/", super::tenant::app_root().display());
    let stack: Vec<String> = stack_trace
        .iter()
        // The innermost frames are last, and they are the ones worth keeping.
        .skip(stack_trace.len().saturating_sub(MAX_STACK_FRAMES))
        .map(|frame| frame.replace(&root, ""))
        .collect();
    let location = innermost_frame(&stack).unwrap_or_default();
    let at = crate::jobs::now_iso();
    let env = env_json
        .filter(|s| !s.is_empty())
        .map(|s| {
            serde_json::from_str::<serde_json::Value>(s)
                .unwrap_or_else(|_| serde_json::Value::String(truncate_chars(s, MAX_ENV_CHARS)))
        })
        .filter(|v| v.to_string().len() <= MAX_ENV_CHARS)
        .unwrap_or(serde_json::Value::Null);
    let sample = serde_json::json!({
        "at": at,
        "request_id": request_id,
        "method": request_data.method.as_ref(),
        "path": request_data.path,
        "error": message,
        "stack": stack,
        "request": redacted_request_snapshot(request_data, /* redact_body = */ true),
        "env": env,
    });
    let occurrence = Occurrence {
        fingerprint: fingerprint(&message, &location),
        message,
        location,
        request_line: format!("{} {}", request_data.method.as_ref(), request_data.path),
        at,
        sample,
    };
    match sender.try_send(occurrence) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// One writer per application, started by its first failure. The writer's
/// database calls need the application's tenant and the server's runtime, and
/// neither crosses `spawn` on its own.
fn sender_for_current_tenant() -> Option<SyncSender<Occurrence>> {
    static SENDERS: OnceLock<Mutex<HashMap<TenantId, SyncSender<Occurrence>>>> = OnceLock::new();
    let tenant = super::tenant::current_id();
    let mut senders = SENDERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(sender) = senders.get(&tenant) {
        return Some(sender.clone());
    }
    let handle = super::get_tokio_handle()?;
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAP);
    let spawned = std::thread::Builder::new()
        .name("error-tracker".to_string())
        .spawn(move || {
            super::tenant::bind_current(tenant);
            super::set_tokio_handle(handle);
            run_writer(receiver);
        });
    if let Err(e) = spawned {
        eprintln!("[errors] could not start the error tracker: {e}");
        return None;
    }
    senders.insert(tenant, sender.clone());
    Some(sender)
}

/// Occurrences of one group gathered during a flush window.
struct Pending {
    message: String,
    location: String,
    count: u64,
    first_at: String,
    last_at: String,
    last_request: String,
    /// Occurrences per hour (`2026-09-24T18`), oldest first.
    hourly: BTreeMap<String, u64>,
    /// Newest first.
    samples: Vec<serde_json::Value>,
}

fn run_writer(receiver: Receiver<Occurrence>) {
    let mut ensured = false;
    // Blocks for the first occurrence, then gathers for FLUSH_EVERY.
    while let Ok(first) = receiver.recv() {
        let mut pending: HashMap<String, Pending> = HashMap::new();
        gather(&mut pending, first);
        let deadline = Instant::now() + FLUSH_EVERY;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left) {
                Ok(occurrence) => gather(&mut pending, occurrence),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if !ensured {
            ensured = ensure_collection();
        }
        for (fingerprint, group) in pending {
            if let Err(e) = write_group(&fingerprint, group) {
                eprintln!("[errors] could not record error group {fingerprint}: {e}");
            }
        }
    }
}

fn gather(pending: &mut HashMap<String, Pending>, occurrence: Occurrence) {
    let group = pending
        .entry(occurrence.fingerprint)
        .or_insert_with(|| Pending {
            message: occurrence.message.clone(),
            location: occurrence.location.clone(),
            count: 0,
            first_at: occurrence.at.clone(),
            last_at: occurrence.at.clone(),
            last_request: String::new(),
            hourly: BTreeMap::new(),
            samples: Vec::new(),
        });
    group.count += 1;
    *group.hourly.entry(hour_of(&occurrence.at)).or_insert(0) += 1;
    group.last_request = occurrence.request_line;
    group.message = occurrence.message;
    group.location = occurrence.location;
    group.last_at = occurrence.at;
    group.samples.insert(0, occurrence.sample);
    group.samples.truncate(MAX_SAMPLES);
}

fn ensure_collection() -> bool {
    let result = if db::is_sql() {
        db::sql::ensure_table(ERRORS_COLLECTION).and_then(|_| {
            db::sql::ensure_doc_index(
                ERRORS_COLLECTION,
                &["status".to_string()],
                "idx__soli_errors_status",
                false,
            )
            .map(|_| ())
        })
    } else {
        crud::ensure_collection(ERRORS_COLLECTION)
            .and_then(|_| crud::ensure_index(ERRORS_COLLECTION, "status"))
    };
    match result {
        Ok(()) => true,
        Err(e) => {
            eprintln!("[errors] could not prepare {ERRORS_COLLECTION}: {e}");
            false
        }
    }
}

fn write_group(fingerprint: &str, group: Pending) -> Result<(), String> {
    match get(fingerprint)? {
        None => {
            let doc = serde_json::json!({
                "message": group.message,
                "location": group.location,
                "status": "open",
                "count": group.count,
                "first_seen": group.first_at,
                "last_seen": group.last_at,
                "last_request": group.last_request,
                "hourly": hourly_json(group.hourly),
                "samples": group.samples,
            });
            if db::is_sql() {
                db::sql::insert(ERRORS_COLLECTION, Some(fingerprint), doc)?;
            } else {
                crud::exec_insert(ERRORS_COLLECTION, Some(fingerprint), doc)?;
            }
            Ok(())
        }
        Some(existing) => patch(fingerprint, merged_patch(&existing, group)),
    }
}

/// The update an existing group receives for a window's occurrences.
fn merged_patch(existing: &serde_json::Value, group: Pending) -> serde_json::Value {
    let count = existing.get("count").and_then(|v| v.as_u64()).unwrap_or(0) + group.count;
    let mut samples = group.samples;
    if let Some(old) = existing.get("samples").and_then(|v| v.as_array()) {
        samples.extend(old.iter().cloned());
    }
    samples.truncate(MAX_SAMPLES);
    let mut hourly = parse_hourly(existing.get("hourly"));
    for (hour, n) in group.hourly {
        *hourly.entry(hour).or_insert(0) += n;
    }
    let mut patch = serde_json::json!({
        "message": group.message,
        "location": group.location,
        "count": count,
        "last_seen": group.last_at,
        "last_request": group.last_request,
        "hourly": hourly_json(hourly),
        "samples": samples,
    });
    if existing.get("status").and_then(|v| v.as_str()) == Some("resolved") {
        patch["status"] = "open".into();
        patch["regressed_at"] = group.first_at.into();
    }
    patch
}

/// `2026-09-24T18:34:17Z` → `2026-09-24T18`.
fn hour_of(iso: &str) -> String {
    iso.chars().take(13).collect()
}

/// The newest [`HOURS_KEPT`] hours as `[[hour, count], …]`, oldest first. An
/// array rather than an object: a merge-patch deep-merges objects, so hours
/// dropped here would survive in the stored document.
fn hourly_json(hourly: BTreeMap<String, u64>) -> serde_json::Value {
    let skip = hourly.len().saturating_sub(HOURS_KEPT);
    hourly
        .into_iter()
        .skip(skip)
        .map(|(hour, n)| serde_json::json!([hour, n]))
        .collect()
}

/// Read back what [`hourly_json`] wrote; anything malformed is skipped.
pub(crate) fn parse_hourly(value: Option<&serde_json::Value>) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for pair in value.and_then(|v| v.as_array()).into_iter().flatten() {
        if let (Some(hour), Some(n)) = (
            pair.get(0).and_then(|h| h.as_str()),
            pair.get(1).and_then(|n| n.as_u64()),
        ) {
            out.insert(hour.to_string(), n);
        }
    }
    out
}

// --- the store the dashboard reads -------------------------------------------

/// One group, or `None` when the fingerprint is unknown.
pub(crate) fn get(fingerprint: &str) -> Result<Option<serde_json::Value>, String> {
    if db::is_sql() {
        return db::sql::get(ERRORS_COLLECTION, fingerprint);
    }
    // A missing document is a normal answer here, not an error.
    Ok(crud::exec_get(ERRORS_COLLECTION, fingerprint).ok())
}

fn patch(fingerprint: &str, fields: serde_json::Value) -> Result<(), String> {
    if db::is_sql() {
        db::sql::update(ERRORS_COLLECTION, fingerprint, fields, true)?;
    } else {
        crud::exec_update(ERRORS_COLLECTION, fingerprint, fields, true)?;
    }
    Ok(())
}

/// Groups in `status`, most recently seen first, without their samples.
pub(crate) fn list(status: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
    if !STATUSES.contains(&status) {
        return Err(format!("unknown status {status:?}"));
    }
    let mut rows = if db::is_sql() {
        let mut eq = std::collections::BTreeMap::new();
        eq.insert("status".to_string(), serde_json::Value::from(status));
        let query = db::ListQuery {
            table: ERRORS_COLLECTION.to_string(),
            eq_filters: eq,
            hash_filter: None,
            filter_sdbql: Some("doc.status == @status".to_string()),
            having: None,
            exists_filters: Vec::new(),
            soft_delete: db::SqlSoftDeleteMode::WithDeleted,
            is_soft_delete_model: false,
            order_field: Some("last_seen".to_string()),
            order_desc: true,
            limit: Some(limit),
            offset: None,
        };
        db::sql::select(&query)?
    } else {
        // `status` is one of STATUSES, checked above, so it is safe to inline.
        let sdbql = format!(
            "FOR doc IN {ERRORS_COLLECTION} FILTER doc.status == \"{status}\" \
             SORT doc.last_seen DESC LIMIT {limit} \
             RETURN {{_key: doc._key, message: doc.message, location: doc.location, \
             status: doc.status, count: doc.count, first_seen: doc.first_seen, \
             last_seen: doc.last_seen, regressed_at: doc.regressed_at, \
             last_request: doc.last_request, hourly: doc.hourly}}"
        );
        crud::exec_query(ERRORS_COLLECTION, sdbql)?
    };
    for row in &mut rows {
        if let Some(map) = row.as_object_mut() {
            map.remove("samples");
        }
    }
    Ok(rows)
}

/// Move a group to `status`. `false` when the group does not exist.
pub(crate) fn set_status(fingerprint: &str, status: &str) -> Result<bool, String> {
    if !STATUSES.contains(&status) {
        return Err(format!("unknown status {status:?}"));
    }
    if get(fingerprint)?.is_none() {
        return Ok(false);
    }
    let mut fields = serde_json::json!({ "status": status });
    if status == "resolved" {
        fields["resolved_at"] = crate::jobs::now_iso().into();
    }
    patch(fingerprint, fields)?;
    Ok(true)
}

/// Forget a group. `false` when it did not exist.
pub(crate) fn delete(fingerprint: &str) -> Result<bool, String> {
    if get(fingerprint)?.is_none() {
        return Ok(false);
    }
    if db::is_sql() {
        db::sql::delete(ERRORS_COLLECTION, fingerprint)?;
    } else {
        crud::exec_delete(ERRORS_COLLECTION, fingerprint)?;
    }
    Ok(true)
}

/// Whether `value` has the shape [`fingerprint`] produces.
pub(crate) fn valid_fingerprint(value: &str) -> bool {
    value.len() == 16 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

// --- grouping ----------------------------------------------------------------

/// The group an error belongs to: its message with the variable parts taken
/// out, and where it was raised without the line number — so an id in the
/// message does not split one bug into thousands of groups, and an edit above
/// the failing line does not start a new one.
pub(crate) fn fingerprint(message: &str, location: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_message(message).as_bytes());
    hasher.update([0]);
    hasher.update(strip_line(location).as_bytes());
    let digest = hasher.finalize();
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// The frame the error was raised in. The executor appends it last.
fn innermost_frame(stack_trace: &[String]) -> Option<String> {
    stack_trace
        .iter()
        .rev()
        .find(|frame| !frame.starts_with("Error:"))
        .cloned()
}

fn strip_line(location: &str) -> &str {
    let trimmed = location.trim_end_matches(" (error location)");
    match trimmed.rsplit_once(':') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => trimmed,
    }
}

/// Quoted text becomes `?`, and any word holding a digit (ids, counts, UUIDs,
/// timestamps, hex) becomes `#`.
fn normalize_message(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == c {
                    closed = true;
                    break;
                }
            }
            out.push('?');
            if !closed {
                break;
            }
            continue;
        }
        if c.is_alphanumeric() || c == '_' || c == '-' {
            let mut word = String::from(c);
            while let Some(&next) = chars.peek() {
                if next.is_alphanumeric() || next == '_' || next == '-' {
                    word.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if word.chars().any(|ch| ch.is_ascii_digit()) {
                out.push('#');
            } else {
                out.push_str(&word);
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((idx, _)) => format!("{}\u{2026}", &s[..idx]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_quoted_values_do_not_split_a_group() {
        let a = fingerprint(
            "User 42 not found: \"alice@x.io\"",
            "show at app/users.sl:10",
        );
        let b = fingerprint("User 97 not found: \"bob@y.io\"", "show at app/users.sl:10");
        assert_eq!(a, b);
    }

    #[test]
    fn a_moved_line_stays_in_its_group() {
        let a = fingerprint("boom", "show at app/users.sl:10");
        let b = fingerprint("boom", "show at app/users.sl:14");
        assert_eq!(a, b);
    }

    #[test]
    fn different_places_or_messages_are_different_groups() {
        let base = fingerprint("boom", "show at app/users.sl:10");
        assert_ne!(base, fingerprint("boom", "index at app/users.sl:10"));
        assert_ne!(base, fingerprint("bang", "show at app/users.sl:10"));
    }

    #[test]
    fn uuids_and_hex_collapse() {
        assert_eq!(
            normalize_message("no job 550e8400-e29b-41d4-a716-446655440000 (0xff)"),
            "no job # (#)"
        );
        assert_eq!(normalize_message("it's broken"), "it?");
    }

    #[test]
    fn fingerprints_are_sixteen_hex_chars() {
        let fp = fingerprint("x", "");
        assert!(valid_fingerprint(&fp), "{fp}");
        assert!(!valid_fingerprint("../../etc/passwd"));
        assert!(!valid_fingerprint("ABCDEFGHIJKLMNOP"));
    }

    #[test]
    fn the_innermost_frame_is_the_location() {
        let stack = vec![
            "index at app/controllers/a.sl:3".to_string(),
            "load at app/models/b.sl:9".to_string(),
        ];
        assert_eq!(
            innermost_frame(&stack).as_deref(),
            Some("load at app/models/b.sl:9")
        );
        assert_eq!(innermost_frame(&[]), None);
    }

    fn pending(count: u64, sample: &str) -> Pending {
        Pending {
            message: "m".into(),
            location: "l".into(),
            count,
            first_at: "2026-09-24T10:00:00Z".into(),
            last_at: "2026-09-24T10:00:01Z".into(),
            last_request: "GET /x".into(),
            hourly: BTreeMap::from([("2026-09-24T10".to_string(), count)]),
            samples: vec![serde_json::json!(sample)],
        }
    }

    #[test]
    fn a_resolved_group_reopens_as_a_regression() {
        let existing = serde_json::json!({"status": "resolved", "count": 3, "samples": ["old"]});
        let patch = merged_patch(&existing, pending(2, "new"));
        assert_eq!(patch["count"], 5);
        assert_eq!(patch["status"], "open");
        assert_eq!(patch["regressed_at"], "2026-09-24T10:00:00Z");
        assert_eq!(patch["samples"], serde_json::json!(["new", "old"]));
    }

    #[test]
    fn an_ignored_group_keeps_counting_and_stays_ignored() {
        let existing = serde_json::json!({"status": "ignored", "count": 1});
        let patch = merged_patch(&existing, pending(1, "s"));
        assert_eq!(patch["count"], 2);
        assert!(patch.get("status").is_none());
    }

    #[test]
    fn samples_are_capped_newest_first() {
        let old: Vec<_> = (0..MAX_SAMPLES).map(|i| format!("old{i}")).collect();
        let existing = serde_json::json!({"status": "open", "count": 9, "samples": old});
        let patch = merged_patch(&existing, pending(1, "new"));
        let samples = patch["samples"].as_array().unwrap();
        assert_eq!(samples.len(), MAX_SAMPLES);
        assert_eq!(samples[0], "new");
    }

    #[test]
    fn gathering_folds_repeats_into_one_group() {
        let mut pending = HashMap::new();
        for i in 0..(MAX_SAMPLES + 3) {
            gather(
                &mut pending,
                Occurrence {
                    fingerprint: "f".into(),
                    message: "m".into(),
                    location: "l".into(),
                    request_line: format!("GET /{i}"),
                    at: format!("t{i}"),
                    sample: serde_json::json!(i),
                },
            );
        }
        let group = &pending["f"];
        assert_eq!(group.count as usize, MAX_SAMPLES + 3);
        assert_eq!(group.first_at, "t0");
        assert_eq!(group.samples.len(), MAX_SAMPLES);
        assert_eq!(group.samples[0], serde_json::json!(MAX_SAMPLES + 2));
        assert_eq!(group.last_request, format!("GET /{}", MAX_SAMPLES + 2));
    }

    #[test]
    fn hourly_counts_add_up_and_keep_the_newest_hours() {
        let existing = serde_json::json!({
            "status": "open",
            "count": 30,
            "hourly": (0..HOURS_KEPT)
                .map(|h| serde_json::json!([format!("2026-09-23T{h:02}"), 1]))
                .chain([serde_json::json!(["2026-09-24T10", 4])])
                .collect::<Vec<_>>(),
        });
        let patch = merged_patch(&existing, pending(2, "s"));
        let hourly = parse_hourly(patch.get("hourly"));
        assert_eq!(hourly.len(), HOURS_KEPT);
        assert_eq!(hourly["2026-09-24T10"], 6);
        assert!(!hourly.contains_key("2026-09-23T00"), "oldest hour dropped");
        assert!(
            patch["hourly"].is_array(),
            "stored as an array, not a mergeable object"
        );
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate_chars("héllo", 2), "hé\u{2026}");
        assert_eq!(truncate_chars("hi", 5), "hi");
    }
}
