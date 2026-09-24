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
//! An application keeps at most [`MAX_GROUPS`] groups. Past that, occurrences
//! of a fingerprint not already stored are counted in one overflow group
//! ([`OVERFLOW_FINGERPRINT`]) instead of each starting its own, so a client
//! that can choose an error's words cannot grow the table without bound.
//!
//! Every write is fenced by `catch_unwind`, and a writer found dead is
//! replaced on the next error. The dashboard reports dropped (queue full),
//! failed (write error or caught panic), restarted and overflowed occurrences
//! separately.
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
use std::sync::{Arc, Mutex, OnceLock};
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

/// Groups one application may hold, not counting the overflow group. A message is partly the caller's words
/// (`normalize_message` takes out digits and quoted values, not every word),
/// so a client that can make a request fail with text of its choosing could
/// otherwise mint a new group per request and grow the table without bound.
/// Once this many groups exist, an occurrence of a *new* fingerprint is
/// counted in the one [`OVERFLOW_FINGERPRINT`] group instead, its own message
/// kept in the sample. Groups already stored keep counting as usual, and
/// deleting groups makes room again.
pub(crate) const MAX_GROUPS: u64 = 1000;

/// The group that collects new fingerprints once [`MAX_GROUPS`] is reached.
/// Sixteen hex digits, so the dashboard's routes accept it; no SHA-256 prefix
/// is expected to be all zeroes.
pub(crate) const OVERFLOW_FINGERPRINT: &str = "0000000000000000";

const OVERFLOW_MESSAGE: &str = "Too many error groups: new kinds of error are counted here";

/// What went wrong with recording itself, per application, since the process
/// started. Kept apart from the writer so a restarted writer keeps the counts.
#[derive(Default)]
pub(crate) struct Stats {
    /// Queue full: errors arrived faster than the writer could store them.
    dropped: AtomicU64,
    /// Occurrences whose write failed (a database error, or a panic in the
    /// writer caught around that write).
    failed: AtomicU64,
    /// Times the writer was found gone and started again.
    restarts: AtomicU64,
    /// Occurrences lost because no writer could take them.
    lost: AtomicU64,
    /// Occurrences of a new fingerprint counted in the overflow group.
    overflowed: AtomicU64,
}

/// A copy of [`Stats`] for the dashboard.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatsSnapshot {
    pub dropped: u64,
    pub failed: u64,
    pub restarts: u64,
    pub lost: u64,
    pub overflowed: u64,
}

impl Stats {
    fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            dropped: self.dropped.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            restarts: self.restarts.load(Ordering::Relaxed),
            lost: self.lost.load(Ordering::Relaxed),
            overflowed: self.overflowed.load(Ordering::Relaxed),
        }
    }
}

/// One application's writer: its channel, a generation that tells a stale
/// sender from its replacement, and the counters that outlive both.
struct Writer {
    sender: Option<SyncSender<Occurrence>>,
    generation: u64,
    stats: Arc<Stats>,
}

type Registry = Mutex<HashMap<TenantId, Writer>>;

fn registry() -> &'static Registry {
    static WRITERS: OnceLock<Registry> = OnceLock::new();
    WRITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// This application's recording counters, since the process started.
pub(crate) fn stats() -> StatsSnapshot {
    let tenant = super::tenant::current_id();
    let writers = registry().lock().unwrap_or_else(|e| e.into_inner());
    writers
        .get(&tenant)
        .map(|w| w.stats.snapshot())
        .unwrap_or_default()
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
    deliver(super::tenant::current_id(), occurrence, &spawn_writer);
}

/// Hand an occurrence to the application's writer, starting it if needed.
///
/// A full queue drops the occurrence and counts it as `dropped`. A writer that
/// is gone (its thread died) is a different fault: its sender is forgotten, a
/// new writer is started and the occurrence retried once — before, the dead
/// sender stayed registered and every later error was counted as a queue
/// overflow while nothing was recorded again until a restart.
fn deliver(
    tenant: TenantId,
    occurrence: Occurrence,
    spawn: &dyn Fn(TenantId, Receiver<Occurrence>, Arc<Stats>) -> bool,
) {
    let mut occurrence = occurrence;
    for attempt in 0..2 {
        let Some((sender, generation, stats)) = writer_for(tenant, spawn) else {
            return;
        };
        match sender.try_send(occurrence) {
            Ok(()) => return,
            Err(TrySendError::Full(_)) => {
                stats.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(TrySendError::Disconnected(back)) => {
                forget_writer(tenant, generation);
                stats.restarts.fetch_add(1, Ordering::Relaxed);
                eprintln!("[errors] the error tracker's writer had stopped; starting it again");
                if attempt == 1 {
                    stats.lost.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                occurrence = back;
            }
        }
    }
}

/// The live sender for `tenant`, starting a writer when there is none. The
/// writer's database calls need the application's tenant and the server's
/// runtime, and neither crosses `spawn` on its own.
fn writer_for(
    tenant: TenantId,
    spawn: &dyn Fn(TenantId, Receiver<Occurrence>, Arc<Stats>) -> bool,
) -> Option<(SyncSender<Occurrence>, u64, Arc<Stats>)> {
    let mut writers = registry().lock().unwrap_or_else(|e| e.into_inner());
    let writer = writers.entry(tenant).or_insert_with(|| Writer {
        sender: None,
        generation: 0,
        stats: Arc::new(Stats::default()),
    });
    if let Some(sender) = &writer.sender {
        return Some((sender.clone(), writer.generation, writer.stats.clone()));
    }
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAP);
    if !spawn(tenant, receiver, writer.stats.clone()) {
        writer.stats.lost.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    writer.generation += 1;
    writer.sender = Some(sender.clone());
    Some((sender, writer.generation, writer.stats.clone()))
}

/// Drop a dead writer's sender, unless another thread already replaced it.
fn forget_writer(tenant: TenantId, generation: u64) {
    let mut writers = registry().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(writer) = writers.get_mut(&tenant) {
        if writer.generation == generation {
            writer.sender = None;
        }
    }
}

fn spawn_writer(tenant: TenantId, receiver: Receiver<Occurrence>, stats: Arc<Stats>) -> bool {
    let Some(handle) = super::get_tokio_handle() else {
        return false;
    };
    let spawned = std::thread::Builder::new()
        .name("error-tracker".to_string())
        .spawn(move || {
            super::tenant::bind_current(tenant);
            super::set_tokio_handle(handle);
            run_writer(receiver, &DbStore, &stats);
        });
    match spawned {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[errors] could not start the error tracker: {e}");
            false
        }
    }
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

fn run_writer(receiver: Receiver<Occurrence>, store: &dyn Store, stats: &Stats) {
    let mut flusher = Flusher::new(store, stats);
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
        flusher.flush(pending);
    }
}

/// Where the writer keeps groups: the app's database, or a map in the tests.
trait Store {
    fn ensure(&self) -> Result<(), String>;
    fn get(&self, fingerprint: &str) -> Result<Option<serde_json::Value>, String>;
    fn insert(&self, fingerprint: &str, doc: serde_json::Value) -> Result<(), String>;
    fn patch(&self, fingerprint: &str, fields: serde_json::Value) -> Result<(), String>;
    fn count(&self) -> Result<u64, String>;
}

struct DbStore;

impl Store for DbStore {
    fn ensure(&self) -> Result<(), String> {
        ensure_collection()
    }
    fn get(&self, fingerprint: &str) -> Result<Option<serde_json::Value>, String> {
        get(fingerprint)
    }
    fn insert(&self, fingerprint: &str, doc: serde_json::Value) -> Result<(), String> {
        if db::is_sql() {
            db::sql::insert(ERRORS_COLLECTION, Some(fingerprint), doc)?;
        } else {
            crud::exec_insert(ERRORS_COLLECTION, Some(fingerprint), doc)?;
        }
        Ok(())
    }
    fn patch(&self, fingerprint: &str, fields: serde_json::Value) -> Result<(), String> {
        patch(fingerprint, fields)
    }
    fn count(&self) -> Result<u64, String> {
        count_groups()
    }
}

/// Writes gathered windows, one group at a time, and keeps track of how many
/// groups exist so [`MAX_GROUPS`] can be enforced without a count per error.
struct Flusher<'a> {
    store: &'a dyn Store,
    stats: &'a Stats,
    ensured: bool,
    /// Groups known to exist; `None` until counted.
    groups: Option<u64>,
    /// Whether the count was refreshed during this window already.
    recounted: bool,
}

impl<'a> Flusher<'a> {
    fn new(store: &'a dyn Store, stats: &'a Stats) -> Self {
        Flusher {
            store,
            stats,
            ensured: false,
            groups: None,
            recounted: false,
        }
    }

    /// Write one window. Every write is fenced by `catch_unwind`: a panic in
    /// one group's write (a malformed stored document, a driver bug) costs that
    /// group's occurrences — counted as failed — and never the writer thread.
    fn flush(&mut self, pending: HashMap<String, Pending>) {
        self.recounted = false;
        if !self.ensured {
            let store = self.store;
            self.ensured =
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.ensure())) {
                    Ok(Ok(())) => true,
                    Ok(Err(e)) => {
                        eprintln!("[errors] could not prepare {ERRORS_COLLECTION}: {e}");
                        false
                    }
                    Err(_) => false,
                };
        }
        let mut overflow: Option<Pending> = None;
        for (fingerprint, group) in pending {
            let count = group.count;
            match self.write_fenced(&fingerprint, group) {
                Ok(None) => {}
                Ok(Some(refused)) => {
                    self.stats.overflowed.fetch_add(count, Ordering::Relaxed);
                    fold_into_overflow(&mut overflow, refused);
                }
                Err(e) => {
                    self.stats.failed.fetch_add(count, Ordering::Relaxed);
                    eprintln!("[errors] could not record error group {fingerprint}: {e}");
                }
            }
        }
        if let Some(group) = overflow {
            let count = group.count;
            if let Err(e) = self.write_fenced(OVERFLOW_FINGERPRINT, group) {
                self.stats.failed.fetch_add(count, Ordering::Relaxed);
                eprintln!("[errors] could not record the overflow group: {e}");
            }
        }
    }

    fn write_fenced(
        &mut self,
        fingerprint: &str,
        group: Pending,
    ) -> Result<Option<Pending>, String> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.write_group(fingerprint, group)
        })) {
            Ok(result) => result,
            Err(panic) => {
                let what = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "panic".to_string());
                Err(format!("the writer panicked: {what}"))
            }
        }
    }

    /// Store one group's window. `Ok(Some(group))` hands the window back when
    /// it is a new fingerprint and there is no room for another group.
    fn write_group(
        &mut self,
        fingerprint: &str,
        group: Pending,
    ) -> Result<Option<Pending>, String> {
        if let Some(existing) = self.store.get(fingerprint)? {
            self.store
                .patch(fingerprint, merged_patch(&existing, group))?;
            return Ok(None);
        }
        if fingerprint != OVERFLOW_FINGERPRINT && !self.has_room()? {
            return Ok(Some(group));
        }
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
        self.store.insert(fingerprint, doc)?;
        if let Some(n) = self
            .groups
            .as_mut()
            .filter(|_| fingerprint != OVERFLOW_FINGERPRINT)
        {
            *n += 1;
        }
        Ok(None)
    }

    /// Whether another group fits. The cached count only grows here, and
    /// groups deleted from the dashboard shrink the table behind its back, so
    /// a full cache is re-counted — once per window, whatever the burst.
    fn has_room(&mut self) -> Result<bool, String> {
        match self.groups {
            Some(n) if n < MAX_GROUPS => return Ok(true),
            Some(_) if self.recounted => return Ok(false),
            _ => {}
        }
        // The overflow group is not one of the MAX_GROUPS.
        let overflow = self.store.get(OVERFLOW_FINGERPRINT)?.is_some();
        let n = self.store.count()?.saturating_sub(overflow as u64);
        self.groups = Some(n);
        self.recounted = true;
        Ok(n < MAX_GROUPS)
    }
}

/// Add a refused window to the overflow group's window.
fn fold_into_overflow(overflow: &mut Option<Pending>, group: Pending) {
    let target = overflow.get_or_insert_with(|| Pending {
        message: OVERFLOW_MESSAGE.to_string(),
        location: String::new(),
        count: 0,
        first_at: group.first_at.clone(),
        last_at: group.last_at.clone(),
        last_request: String::new(),
        hourly: BTreeMap::new(),
        samples: Vec::new(),
    });
    target.count += group.count;
    if group.first_at < target.first_at {
        target.first_at = group.first_at;
    }
    if group.last_at >= target.last_at {
        target.last_at = group.last_at;
        target.last_request = group.last_request;
    }
    for (hour, n) in group.hourly {
        *target.hourly.entry(hour).or_insert(0) += n;
    }
    target.samples.extend(group.samples);
    // Newest first, whichever group each sample came from.
    target.samples.sort_by(|a, b| {
        let at = |v: &serde_json::Value| {
            v.get("at")
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_string()
        };
        at(b).cmp(&at(a))
    });
    target.samples.truncate(MAX_SAMPLES);
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

fn ensure_collection() -> Result<(), String> {
    if db::is_sql() {
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
    }
}

/// Groups stored for this application.
fn count_groups() -> Result<u64, String> {
    if db::is_sql() {
        let query = db::ListQuery {
            table: ERRORS_COLLECTION.to_string(),
            eq_filters: std::collections::BTreeMap::new(),
            hash_filter: None,
            filter_sdbql: None,
            having: None,
            exists_filters: Vec::new(),
            soft_delete: db::SqlSoftDeleteMode::WithDeleted,
            is_soft_delete_model: false,
            order_field: None,
            order_desc: false,
            limit: None,
            offset: None,
        };
        return db::sql::count(&query).map(|n| n.max(0) as u64);
    }
    let rows = crud::exec_query(
        ERRORS_COLLECTION,
        format!("RETURN COLLECTION_COUNT(\"{ERRORS_COLLECTION}\")"),
    )?;
    match crate::interpreter::builtins::model::core::parse_count_result(&rows) {
        crate::interpreter::value::Value::Int(n) => Ok(n.max(0) as u64),
        other => Err(format!("unexpected count result: {other}")),
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
    // A missing document is a normal answer here; anything else (a timeout,
    // a refused connection) is an error. Reading every failure as "missing"
    // made the writer insert over a group it merely could not read.
    match crud::exec_get(ERRORS_COLLECTION, fingerprint) {
        Ok(doc) => Ok(Some(doc)),
        Err(e) if is_missing_document(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Does this SoliDB error say the document (or its collection) is not there?
fn is_missing_document(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("404")
        || lower.contains("not found")
        || lower.contains("notfound")
        || lower.contains("does not exist")
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
///
/// `"` always quotes. `'` quotes only where a quotation can start — at the
/// start of the message or after a space or punctuation — and closes only
/// where one can end, so the apostrophe in `can't` or `User's` is text: read
/// as a quote it swallowed the words after it, merging `can't find X` with
/// `can't divide` and letting the value in `User's email 'bob'` into the
/// fingerprint.
fn normalize_message(message: &str) -> String {
    let chars: Vec<char> = message.chars().collect();
    let word_char = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let mut out = String::with_capacity(message.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let opens = c == '"' || (c == '\'' && (i == 0 || !chars[i - 1].is_alphanumeric()));
        if opens {
            let mut j = i + 1;
            let mut closed = false;
            while j < chars.len() {
                let closes = chars[j] == c
                    && (c == '"' || chars.get(j + 1).is_none_or(|n| !n.is_alphanumeric()));
                if closes {
                    closed = true;
                    break;
                }
                j += 1;
            }
            out.push('?');
            if !closed {
                break;
            }
            i = j + 1;
            continue;
        }
        if word_char(c) {
            let start = i;
            while i < chars.len() && word_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if word.chars().any(|ch| ch.is_ascii_digit()) {
                out.push('#');
            } else {
                out.push_str(&word);
            }
            continue;
        }
        out.push(c);
        i += 1;
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
    }

    #[test]
    fn an_apostrophe_is_text_and_a_single_quote_still_quotes() {
        assert_eq!(normalize_message("it's broken"), "it's broken");
        assert_ne!(
            fingerprint("can't find X", "show at a.sl:1"),
            fingerprint("can't divide", "show at a.sl:1")
        );
        assert_eq!(
            normalize_message("User's email 'bob' is taken"),
            "User's email ? is taken"
        );
        assert_eq!(
            fingerprint("User's email 'bob' is taken", "l"),
            fingerprint("User's email 'alice' is taken", "l")
        );
        assert_eq!(normalize_message("no key 'o'brien' (x)"), "no key ? (x)");
        assert_eq!(normalize_message("bad value: 'x"), "bad value: ?");
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

    /// An in-memory [`Store`], with a fingerprint whose write panics.
    #[derive(Default)]
    struct MemStore {
        docs: std::cell::RefCell<BTreeMap<String, serde_json::Value>>,
        panic_on: Option<String>,
    }

    impl Store for MemStore {
        fn ensure(&self) -> Result<(), String> {
            Ok(())
        }
        fn get(&self, fingerprint: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(self.docs.borrow().get(fingerprint).cloned())
        }
        fn insert(&self, fingerprint: &str, doc: serde_json::Value) -> Result<(), String> {
            if self.panic_on.as_deref() == Some(fingerprint) {
                panic!("boom in insert");
            }
            self.docs.borrow_mut().insert(fingerprint.to_string(), doc);
            Ok(())
        }
        fn patch(&self, fingerprint: &str, fields: serde_json::Value) -> Result<(), String> {
            let mut docs = self.docs.borrow_mut();
            let doc = docs.get_mut(fingerprint).ok_or("missing")?;
            for (k, v) in fields.as_object().unwrap() {
                doc[k] = v.clone();
            }
            Ok(())
        }
        fn count(&self) -> Result<u64, String> {
            Ok(self.docs.borrow().len() as u64)
        }
    }

    fn window(fps: impl IntoIterator<Item = String>) -> HashMap<String, Pending> {
        let mut pending = HashMap::new();
        for fp in fps {
            gather(
                &mut pending,
                Occurrence {
                    fingerprint: fp.clone(),
                    message: format!("error {fp}"),
                    location: "l".into(),
                    request_line: "GET /".into(),
                    at: "2026-09-24T10:00:00Z".into(),
                    sample: serde_json::json!({"at": "2026-09-24T10:00:00Z", "error": format!("error {fp}")}),
                },
            );
        }
        pending
    }

    #[test]
    fn new_fingerprints_past_the_cap_fold_into_one_overflow_group() {
        let store = MemStore::default();
        let stats = Stats::default();
        let mut flusher = Flusher::new(&store, &stats);
        flusher.flush(window((1..=MAX_GROUPS).map(|i| format!("{i:016x}"))));
        assert_eq!(store.count().unwrap(), MAX_GROUPS);

        // Attacker-chosen words: every one a new fingerprint.
        flusher.flush(window((0..50).map(|i| format!("new-{i}"))));
        assert_eq!(
            store.count().unwrap(),
            MAX_GROUPS + 1,
            "one overflow group, not fifty"
        );
        let overflow = store
            .get(OVERFLOW_FINGERPRINT)
            .unwrap()
            .expect("overflow group");
        assert_eq!(overflow["count"], 50);
        assert_eq!(overflow["message"], OVERFLOW_MESSAGE);
        assert_eq!(overflow["samples"].as_array().unwrap().len(), MAX_SAMPLES);
        assert_eq!(stats.snapshot().overflowed, 50);

        // A group already stored keeps counting past the cap.
        flusher.flush(window([format!("{:016x}", 3)]));
        assert_eq!(
            store.get(&format!("{:016x}", 3)).unwrap().unwrap()["count"],
            2
        );

        // Deleting a group makes room again (the full count is re-read).
        store.docs.borrow_mut().remove(&format!("{:016x}", 7));
        flusher.flush(window(["fresh".to_string()]));
        assert!(
            store.get("fresh").unwrap().is_some(),
            "room made by a delete is used"
        );
    }

    #[test]
    fn a_panicking_write_is_counted_and_the_writer_carries_on() {
        let store = MemStore {
            panic_on: Some("bad".into()),
            ..Default::default()
        };
        let stats = Stats::default();
        let mut flusher = Flusher::new(&store, &stats);
        flusher.flush(window([
            "bad".to_string(),
            "good".to_string(),
            "bad".to_string(),
        ]));
        assert!(
            store.get("good").unwrap().is_some(),
            "the other group was still written"
        );
        assert_eq!(
            stats.snapshot().failed,
            2,
            "both occurrences of the panicking group"
        );
        assert_eq!(
            stats.snapshot().dropped,
            0,
            "a failure is not reported as overload"
        );
        flusher.flush(window(["later".to_string()]));
        assert!(store.get("later").unwrap().is_some(), "the writer survived");
    }

    #[test]
    fn a_dead_writer_is_replaced_not_reported_as_overflow() {
        use std::sync::atomic::AtomicUsize;
        let tenant = TenantId(0xE77_0001);
        let spawned = AtomicUsize::new(0);
        let kept: Mutex<Vec<Receiver<Occurrence>>> = Mutex::new(Vec::new());
        let spawn = |_: TenantId, rx: Receiver<Occurrence>, _: Arc<Stats>| {
            // The first writer dies at once; the second stays up.
            if spawned.fetch_add(1, Ordering::SeqCst) > 0 {
                kept.lock().unwrap().push(rx);
            }
            true
        };
        let occurrence = |fp: &str| Occurrence {
            fingerprint: fp.into(),
            message: "m".into(),
            location: "l".into(),
            request_line: "GET /".into(),
            at: "t".into(),
            sample: serde_json::Value::Null,
        };
        deliver(tenant, occurrence("a"), &spawn);
        deliver(tenant, occurrence("b"), &spawn);
        assert_eq!(
            spawned.load(Ordering::SeqCst),
            2,
            "respawned once, then reused"
        );
        let received: Vec<String> = kept.lock().unwrap()[0]
            .try_iter()
            .map(|o| o.fingerprint)
            .collect();
        assert_eq!(received, vec!["a", "b"], "the retried occurrence arrived");
        let stats = registry().lock().unwrap()[&tenant].stats.snapshot();
        assert_eq!(stats.restarts, 1);
        assert_eq!(stats.dropped, 0);
        assert_eq!(stats.lost, 0);
    }

    #[test]
    fn only_a_missing_document_reads_as_absent() {
        assert!(is_missing_document("HTTP 404 Not Found http://db/x: {}"));
        assert!(is_missing_document("driver get failed: document not found"));
        assert!(!is_missing_document("HTTP error: connection refused"));
        assert!(!is_missing_document(
            "HTTP 503 Service Unavailable http://db/x: busy"
        ));
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate_chars("héllo", 2), "hé\u{2026}");
        assert_eq!(truncate_chars("hi", 5), "hi");
    }
}
