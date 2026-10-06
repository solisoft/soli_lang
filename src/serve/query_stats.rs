//! Every ORM query counted by shape — the fast ones too — for the "time spent"
//! view of `/__soli/slow_queries`.
//!
//! [`slow_queries`] keeps what crossed a threshold, so a 3 ms lookup that runs
//! forty times a request, on every request, never shows up there. This keeps
//! the totals of all of them: calls, time, the slowest call, and the most
//! times a shape ran within one request or job — an N+1 in production.
//!
//! The hot path is one hash of the query text and one update of a counter:
//! each worker thread owns a table keyed by that hash, behind a mutex nobody
//! else takes except the collector, once per [`FLUSH_EVERY`]. The text is
//! turned into its shape (the slow-query tracker's [`normalize_query`], so both
//! views share fingerprints) only the first time a thread meets it. The ORM
//! binds values, so its texts repeat; a text that inlines them is new on every
//! call, and past [`MAX_TEXTS_PER_THREAD`] live texts those are counted in one
//! overflow group instead of normalised.
//!
//! The collector drains every table each minute, merges by fingerprint and
//! hands each application's share to its writer ([`tenant_writer`]), which
//! writes one update per shape into `_soli_query_stats`:
//!
//! ```text
//! { _key: <fingerprint>, query: <the shape>, count, total_ms, max_ms,
//!   first_seen, last_seen, peak_per_request, peak_context,
//!   hourly: [ ["2026-09-25T18", count, ms, peak], … ] }
//! ```
//!
//! No bind value is ever kept. Queries on the framework's `_soli_*` tables are
//! not counted. On by default; `SOLI_QUERY_STATS=off` disables it, and it stays
//! off under `APP_ENV=test` unless `SOLI_QUERY_STATS=on`.
//!
//! [`slow_queries`]: super::slow_queries
//! [`normalize_query`]: super::slow_queries::normalize_query
//! [`tenant_writer`]: super::tenant_writer

use std::cell::{Cell, OnceCell};
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::internal_store;
use super::slow_queries::{self, round_ms, Dialect};
use super::tenant::TenantId;
pub(crate) use super::tenant_writer::StatsSnapshot;
use super::tenant_writer::{self, Stats, Writers};

pub(crate) const STATS_COLLECTION: &str = "_soli_query_stats";

/// Shapes stored per application. Past it, shapes unseen for
/// [`STALE_AFTER_DAYS`] are pruned to make room; failing that, a new shape is
/// counted as overflowed and not stored.
pub(crate) const MAX_GROUPS: u64 = 1000;

/// Hours of per-hour totals kept on a shape: the view is "the last 24 hours".
pub(crate) const HOURS_KEPT: usize = 24;

/// The group that counts queries whose text could not be broken down.
pub(crate) const OVERFLOW_FINGERPRINT: &str = "0000000000000000";
const OVERFLOW_SHAPE: &str = "(queries not broken down: too many distinct query texts)";

const FLUSH_EVERY: Duration = Duration::from_secs(60);
const MAX_TEXTS_PER_THREAD: usize = 2048;
const STALE_AFTER_DAYS: i64 = 7;
const MAX_QUERY_CHARS: usize = 4000;
const MAX_CONTEXT_CHARS: usize = 300;
const QUEUE_CAP: usize = 8;

const UNSTARTED: u8 = 0;
const RUNNING: u8 = 1;
const OFF: u8 = 2;

static STATE: AtomicU8 = AtomicU8::new(UNSTARTED);
static START: Mutex<()> = Mutex::new(());
static SLOTS: Mutex<Vec<Arc<Slot>>> = Mutex::new(Vec::new());
static WRITERS: Writers<Batch> = Writers::new("query-stats", QUEUE_CAP);

/// Fixed seeds: the key only has to be stable within the process.
static HASHER: ahash::RandomState = ahash::RandomState::with_seeds(
    0x243f_6a88_85a3_08d3,
    0x1319_8a2e_0370_7344,
    0xa409_3822_299f_31d0,
    0x082e_fa98_ec4e_6c89,
);

thread_local! {
    static LOCAL: OnceCell<Arc<Slot>> = const { OnceCell::new() };
    /// Inside a request or a job: only then do calls count towards the
    /// per-request peak, so a WebSocket or LiveView worker's stream of events
    /// is never read as one request running a query a thousand times.
    static IN_UNIT: Cell<bool> = const { Cell::new(false) };
}

/// Whether query stats are recorded at all.
pub(crate) fn enabled() -> bool {
    super::error_tracker::env_switch("SOLI_QUERY_STATS")
}

/// This application's writer counters, since the process started.
pub(crate) fn stats() -> StatsSnapshot {
    WRITERS.stats(super::tenant::current_id())
}

/// Whether this query should be counted; starts the collector on first use.
/// Outside `soli serve` (a script, a migration) nothing is counted.
#[inline]
fn running() -> bool {
    match STATE.load(Ordering::Relaxed) {
        RUNNING => true,
        OFF => false,
        _ => start(),
    }
}

#[cold]
fn start() -> bool {
    if !enabled() {
        STATE.store(OFF, Ordering::Relaxed);
        return false;
    }
    // The collector needs the server's runtime; a thread without one is not
    // serving, and asks again next time.
    if super::get_tokio_handle().is_none() {
        return false;
    }
    let _guard = START.lock().unwrap_or_else(|e| e.into_inner());
    if STATE.load(Ordering::Relaxed) == RUNNING {
        return true;
    }
    let tenant = super::tenant::current_id();
    if !tenant_writer::spawn_thread("query-stats", tenant, run_collector) {
        return false;
    }
    STATE.store(RUNNING, Ordering::Relaxed);
    true
}

/// The key a query text is counted under, or `None` when nothing is counted.
/// Taken before a call site moves the text into its request body.
#[inline]
pub fn key(query: &str) -> Option<u64> {
    running().then(|| HASHER.hash_one(query))
}

/// Count one run of `query` that took `ms`.
#[inline]
pub fn record(query: &str, dialect: Dialect, ms: f64) {
    record_key(key(query), dialect, ms, || Some(query.to_owned()));
}

/// Count one run under a [`key`]. `text` gives the query back, and is called
/// only the first time this thread meets that key.
#[inline]
pub fn record_key(
    key: Option<u64>,
    dialect: Dialect,
    ms: f64,
    text: impl FnOnce() -> Option<String>,
) {
    let Some(key) = key else {
        return;
    };
    let us = (ms * 1000.0).max(0.0) as u64;
    let tenant = super::tenant::current_id();
    let in_unit = IN_UNIT.with(Cell::get);
    LOCAL.with(|local| {
        let slot = local.get_or_init(register_slot);
        let mut table = slot.table.lock().unwrap_or_else(|e| e.into_inner());
        table.record(key, tenant, dialect, us, in_unit, text);
    });
}

fn register_slot() -> Arc<Slot> {
    let slot = Arc::new(Slot::default());
    SLOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(slot.clone());
    slot
}

/// Count nothing this thread runs: a framework thread whose queries are its
/// own bookkeeping (the job poller).
pub fn ignore_this_thread() {
    LOCAL.with(|local| {
        let slot = local.get_or_init(register_slot);
        slot.table.lock().unwrap_or_else(|e| e.into_inner()).ignored = true;
    });
}

/// A request or a job starts on this thread.
pub(crate) fn begin_unit() {
    IN_UNIT.with(|u| u.set(true));
    with_table(|table| table.end_unit(None));
}

/// The request or job that ran on this thread is over; `context` names it
/// (`GET /orders → orders#index`, `job ReportJob`).
pub(crate) fn end_unit(context: &str) {
    IN_UNIT.with(|u| u.set(false));
    with_table(|table| table.end_unit(Some(context)));
}

/// Run `f` on this thread's table, if it has one.
fn with_table(f: impl FnOnce(&mut Table)) {
    LOCAL.with(|local| {
        if let Some(slot) = local.get() {
            f(&mut slot.table.lock().unwrap_or_else(|e| e.into_inner()));
        }
    });
}

// --- the per-thread table ----------------------------------------------------

#[derive(Default)]
struct Slot {
    table: Mutex<Table>,
}

/// Keys are already hashes; hashing them again is wasted work.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(*b);
        }
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type KeyMap<V> = HashMap<u64, V, BuildHasherDefault<KeyHasher>>;

#[derive(Default)]
struct Table {
    entries: KeyMap<Entry>,
    /// Keys run in the current request or job.
    touched: Vec<u64>,
    /// Per application: runs of texts this table had no room to break down.
    overflow: HashMap<TenantId, Totals>,
    /// See [`ignore_this_thread`].
    ignored: bool,
}

struct Entry {
    tenant: TenantId,
    /// `None` for a query on a framework table: known, never counted.
    shape: Option<Arc<Shape>>,
    /// Run since the collector last came by; an entry that was not is dropped.
    seen: bool,
    totals: Totals,
    /// Runs in the current request or job.
    in_unit: u32,
}

struct Shape {
    fingerprint: String,
    text: String,
}

#[derive(Default, Clone, Debug, PartialEq)]
struct Totals {
    count: u64,
    total_us: u64,
    max_us: u64,
    /// Most runs within one request or job, and which one it was.
    peak: u32,
    peak_context: String,
}

impl Totals {
    fn add(&mut self, us: u64) {
        self.count += 1;
        self.total_us += us;
        self.max_us = self.max_us.max(us);
    }

    fn merge(&mut self, other: Totals) {
        self.count += other.count;
        self.total_us += other.total_us;
        self.max_us = self.max_us.max(other.max_us);
        if other.peak > self.peak {
            self.peak = other.peak;
            self.peak_context = other.peak_context;
        }
    }
}

impl Table {
    fn record(
        &mut self,
        key: u64,
        tenant: TenantId,
        dialect: Dialect,
        us: u64,
        in_unit: bool,
        text: impl FnOnce() -> Option<String>,
    ) {
        if self.ignored {
            return;
        }
        // Two applications can run the same text; their counts stay apart.
        let key = key ^ u64::from(tenant.0).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.seen = true;
            if entry.shape.is_some() {
                entry.totals.add(us);
                if in_unit {
                    if entry.in_unit == 0 {
                        self.touched.push(key);
                    }
                    entry.in_unit += 1;
                }
            }
            return;
        }
        if self.entries.len() >= MAX_TEXTS_PER_THREAD {
            self.overflow.entry(tenant).or_default().add(us);
            return;
        }
        let Some(text) = text() else {
            return;
        };
        let shape = (!text.contains("_soli_")).then(|| {
            let shape = slow_queries::normalize_query(&text, dialect);
            Arc::new(Shape {
                fingerprint: slow_queries::fingerprint(&shape),
                text: super::error_tracker::truncate_chars(&shape, MAX_QUERY_CHARS),
            })
        });
        let mut entry = Entry {
            tenant,
            seen: true,
            totals: Totals::default(),
            in_unit: 0,
            shape,
        };
        if entry.shape.is_some() {
            entry.totals.add(us);
            if in_unit {
                entry.in_unit = 1;
                self.touched.push(key);
            }
        }
        self.entries.insert(key, entry);
    }

    /// Fold the current unit's per-shape runs into each shape's peak. `None`
    /// forgets them instead (a unit that never reached its end).
    fn end_unit(&mut self, context: Option<&str>) {
        let mut touched = std::mem::take(&mut self.touched);
        for key in touched.drain(..) {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if let Some(context) = context {
                if entry.in_unit > entry.totals.peak {
                    entry.totals.peak = entry.in_unit;
                    entry.totals.peak_context =
                        super::error_tracker::truncate_chars(context, MAX_CONTEXT_CHARS);
                }
            }
            entry.in_unit = 0;
        }
        self.touched = touched;
    }

    /// Take everything counted since the last drain into `out`, by application
    /// and fingerprint. Texts not run since the last drain are forgotten, so a
    /// table holds what is live, not everything ever run.
    fn drain_into(&mut self, out: &mut HashMap<TenantId, HashMap<String, Group>>) {
        self.entries.retain(|_, entry| {
            let keep = entry.seen || entry.in_unit > 0;
            entry.seen = false;
            keep
        });
        for entry in self.entries.values_mut() {
            let Some(shape) = &entry.shape else {
                continue;
            };
            if entry.totals.count == 0 {
                continue;
            }
            let totals = std::mem::take(&mut entry.totals);
            out.entry(entry.tenant)
                .or_default()
                .entry(shape.fingerprint.clone())
                .or_insert_with(|| Group::new(&shape.text))
                .totals
                .merge(totals);
        }
        for (tenant, totals) in self.overflow.drain() {
            out.entry(tenant)
                .or_default()
                .entry(OVERFLOW_FINGERPRINT.to_string())
                .or_insert_with(|| Group::new(OVERFLOW_SHAPE))
                .totals
                .merge(totals);
        }
    }
}

// --- collector and writer ----------------------------------------------------

/// One shape's totals for one flush window, all threads merged.
struct Group {
    shape: String,
    totals: Totals,
}

impl Group {
    fn new(shape: &str) -> Self {
        Group {
            shape: shape.to_string(),
            totals: Totals::default(),
        }
    }
}

/// One application's window.
struct Batch {
    at: String,
    groups: HashMap<String, Group>,
}

fn run_collector() {
    loop {
        std::thread::sleep(FLUSH_EVERY);
        let at = crate::jobs::now_iso();
        for (tenant, groups) in drain_all() {
            if groups.is_empty() {
                continue;
            }
            let batch = Batch {
                at: at.clone(),
                groups,
            };
            WRITERS.deliver(tenant, batch, &spawn_writer);
        }
    }
}

fn drain_all() -> HashMap<TenantId, HashMap<String, Group>> {
    let slots: Vec<Arc<Slot>> = SLOTS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut out = HashMap::new();
    for slot in &slots {
        slot.table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain_into(&mut out);
    }
    drop(slots);
    // A thread that has ended holds its slot no more: it was just drained.
    SLOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|slot| Arc::strong_count(slot) > 1);
    out
}

fn spawn_writer(tenant: TenantId, receiver: Receiver<Batch>, stats: Arc<Stats>) -> bool {
    tenant_writer::spawn_thread("query-stats-writer", tenant, move || {
        let mut flusher = Flusher::new(&DbStore, &stats);
        while let Ok(batch) = receiver.recv() {
            flusher.flush(batch);
        }
    })
}

/// Where the writer keeps shapes: the app's database, or a map in the tests.
trait Store {
    fn ensure(&self) -> Result<(), String>;
    fn get(&self, key: &str) -> Result<Option<serde_json::Value>, String>;
    fn insert(&self, key: &str, doc: serde_json::Value) -> Result<(), String>;
    fn patch(&self, key: &str, fields: serde_json::Value) -> Result<(), String>;
    fn count(&self) -> Result<u64, String>;
    /// Up to `limit` keys, least recently seen first, with their `last_seen`.
    fn oldest(&self, limit: usize) -> Result<Vec<(String, String)>, String>;
    fn delete(&self, key: &str) -> Result<bool, String>;
}

struct DbStore;

impl Store for DbStore {
    fn ensure(&self) -> Result<(), String> {
        internal_store::ensure(STATS_COLLECTION, "last_seen")
    }
    fn get(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        internal_store::get(STATS_COLLECTION, key)
    }
    fn insert(&self, key: &str, doc: serde_json::Value) -> Result<(), String> {
        internal_store::insert(STATS_COLLECTION, key, doc)
    }
    fn patch(&self, key: &str, fields: serde_json::Value) -> Result<(), String> {
        internal_store::patch(STATS_COLLECTION, key, fields)
    }
    fn count(&self) -> Result<u64, String> {
        internal_store::count(STATS_COLLECTION)
    }
    fn oldest(&self, limit: usize) -> Result<Vec<(String, String)>, String> {
        let rows = internal_store::list(
            STATS_COLLECTION,
            None,
            "last_seen",
            false,
            limit,
            &["last_seen"],
        )?;
        Ok(rows
            .iter()
            .map(|row| (str_of(row, "_key"), str_of(row, "last_seen")))
            .collect())
    }
    fn delete(&self, key: &str) -> Result<bool, String> {
        internal_store::delete(STATS_COLLECTION, key)
    }
}

fn str_of(row: &serde_json::Value, field: &str) -> String {
    row.get(field)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

struct Flusher<'a> {
    store: &'a dyn Store,
    stats: &'a Stats,
    ensured: bool,
    /// Shapes known to exist; `None` until counted.
    groups: Option<u64>,
    /// The table was already counted (and pruned) in this window.
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

    fn flush(&mut self, batch: Batch) {
        self.recounted = false;
        if !self.ensured {
            let store = self.store;
            self.ensured = match tenant_writer::fenced(|| store.ensure()) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("[query-stats] could not prepare {STATS_COLLECTION}: {e}");
                    false
                }
            };
        }
        for (fingerprint, group) in batch.groups {
            let count = group.totals.count;
            match tenant_writer::fenced(|| self.write_group(&fingerprint, group, &batch.at)) {
                Ok(true) => {}
                Ok(false) => {
                    self.stats.overflowed.fetch_add(count, Ordering::Relaxed);
                }
                Err(e) => {
                    self.stats.failed.fetch_add(count, Ordering::Relaxed);
                    eprintln!("[query-stats] could not record query group {fingerprint}: {e}");
                }
            }
        }
    }

    /// Store one shape's window. `Ok(false)` when it is a new shape and there
    /// is no room for it.
    fn write_group(&mut self, fingerprint: &str, group: Group, at: &str) -> Result<bool, String> {
        if let Some(existing) = self.store.get(fingerprint)? {
            self.store
                .patch(fingerprint, merged(Some(&existing), group, at))?;
            return Ok(true);
        }
        if !self.has_room()? {
            return Ok(false);
        }
        self.store.insert(fingerprint, merged(None, group, at))?;
        if let Some(n) = self.groups.as_mut() {
            *n += 1;
        }
        Ok(true)
    }

    fn has_room(&mut self) -> Result<bool, String> {
        match self.groups {
            Some(n) if n < MAX_GROUPS => return Ok(true),
            Some(_) if self.recounted => return Ok(false),
            _ => {}
        }
        self.recounted = true;
        let mut n = self.store.count()?;
        if n >= MAX_GROUPS {
            n = n.saturating_sub(self.prune_stale()?);
        }
        self.groups = Some(n);
        Ok(n < MAX_GROUPS)
    }

    /// Forget shapes nothing has run for [`STALE_AFTER_DAYS`]: code that was
    /// changed or removed. Returns how many went.
    fn prune_stale(&self) -> Result<u64, String> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(STALE_AFTER_DAYS))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut removed = 0;
        for (key, last_seen) in self.store.oldest(100)? {
            // ISO timestamps compare as strings.
            if last_seen.as_str() >= cutoff.as_str() {
                break;
            }
            if self.store.delete(&key)? {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// One hour's totals on a stored shape.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bucket {
    pub count: u64,
    pub ms: f64,
    pub peak: u64,
}

/// The document a shape is stored as, `existing` (if any) plus this window.
fn merged(existing: Option<&serde_json::Value>, group: Group, at: &str) -> serde_json::Value {
    let number = |field: &str| {
        existing
            .and_then(|doc| doc.get(field))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    let text = |field: &str| {
        existing
            .and_then(|doc| doc.get(field))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let totals = group.totals;
    let ms = totals.total_us as f64 / 1000.0;
    let mut hourly = parse_hourly(existing.and_then(|doc| doc.get("hourly")));
    let bucket = hourly.entry(at.chars().take(13).collect()).or_default();
    bucket.count += totals.count;
    bucket.ms += ms;
    bucket.peak = bucket.peak.max(u64::from(totals.peak));
    let skip = hourly.len().saturating_sub(HOURS_KEPT);
    let hourly: BTreeMap<String, Bucket> = hourly.into_iter().skip(skip).collect();

    // The peak is the day's: a fixed N+1 stops showing once its hours age out.
    let peak = hourly.values().map(|b| b.peak).max().unwrap_or(0);
    let peak_context = if totals.peak > 1 && u64::from(totals.peak) >= peak {
        totals.peak_context
    } else if peak > 1 {
        text("peak_context")
    } else {
        String::new()
    };
    let first_seen = match text("first_seen") {
        seen if seen.is_empty() => at.to_string(),
        seen => seen,
    };
    serde_json::json!({
        "query": group.shape,
        "count": number("count") as u64 + totals.count,
        "total_ms": round_ms(number("total_ms") + ms),
        "max_ms": round_ms(number("max_ms").max(totals.max_us as f64 / 1000.0)),
        "first_seen": first_seen,
        "last_seen": at,
        "peak_per_request": peak,
        "peak_context": peak_context,
        "hourly": hourly
            .into_iter()
            .map(|(hour, b)| serde_json::json!([hour, b.count, round_ms(b.ms), b.peak]))
            .collect::<Vec<_>>(),
    })
}

/// Read back the `[[hour, count, ms, peak], …]` a shape stores; anything
/// malformed is skipped.
pub(crate) fn parse_hourly(value: Option<&serde_json::Value>) -> BTreeMap<String, Bucket> {
    let mut out = BTreeMap::new();
    for item in value.and_then(|v| v.as_array()).into_iter().flatten() {
        let Some(parts) = item.as_array() else {
            continue;
        };
        let Some(hour) = parts.first().and_then(|h| h.as_str()) else {
            continue;
        };
        let at = |i: usize| parts.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
        out.insert(
            hour.to_string(),
            Bucket {
                count: at(1) as u64,
                ms: at(2),
                peak: at(3) as u64,
            },
        );
    }
    out
}

// --- the store the dashboard reads -------------------------------------------

/// Every stored shape, without order: the page ranks them by their last 24
/// hours, which no stored field holds.
pub(crate) fn list_all() -> Result<Vec<serde_json::Value>, String> {
    internal_store::list(
        STATS_COLLECTION,
        None,
        "last_seen",
        true,
        MAX_GROUPS as usize + 1,
        &[
            "query",
            "count",
            "total_ms",
            "max_ms",
            "first_seen",
            "last_seen",
            "peak_per_request",
            "peak_context",
            "hourly",
        ],
    )
}

pub(crate) fn valid_fingerprint(value: &str) -> bool {
    super::error_tracker::valid_fingerprint(value)
}

/// One shape, or `None` when the fingerprint is unknown.
pub(crate) fn get(fingerprint: &str) -> Result<Option<serde_json::Value>, String> {
    internal_store::get(STATS_COLLECTION, fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    const APP: TenantId = TenantId(0);

    fn run(table: &mut Table, query: &str, us: u64, in_unit: bool) {
        let key = HASHER.hash_one(query);
        table.record(key, APP, Dialect::Sdbql, us, in_unit, || {
            Some(query.to_string())
        });
    }

    fn drained(table: &mut Table) -> HashMap<String, Group> {
        let mut out = HashMap::new();
        table.drain_into(&mut out);
        out.remove(&APP).unwrap_or_default()
    }

    #[test]
    fn texts_that_differ_by_literals_are_one_shape() {
        let mut table = Table::default();
        run(
            &mut table,
            "FOR u IN users FILTER u.id == 1 RETURN u",
            1000,
            false,
        );
        run(
            &mut table,
            "FOR u IN users FILTER u.id == 2 RETURN u",
            3000,
            false,
        );
        let groups = drained(&mut table);
        assert_eq!(groups.len(), 1);
        let group = groups.values().next().unwrap();
        assert_eq!(group.shape, "FOR u IN users FILTER u.id == ? RETURN u");
        assert_eq!(
            (
                group.totals.count,
                group.totals.total_us,
                group.totals.max_us
            ),
            (2, 4000, 3000)
        );
        // Shares the slow-query tracker's fingerprint, so the page can link them.
        assert_eq!(
            groups.keys().next().unwrap(),
            &slow_queries::fingerprint("FOR u IN users FILTER u.id == ? RETURN u")
        );
    }

    #[test]
    fn the_text_is_asked_for_once() {
        let mut table = Table::default();
        let asked = Cell::new(0);
        for _ in 0..3 {
            table.record(7, APP, Dialect::Sdbql, 10, false, || {
                asked.set(asked.get() + 1);
                Some("RETURN 1".to_string())
            });
        }
        assert_eq!(asked.get(), 1);
        assert_eq!(drained(&mut table).values().next().unwrap().totals.count, 3);
    }

    #[test]
    fn framework_tables_are_not_counted() {
        let mut table = Table::default();
        run(&mut table, "FOR doc IN _soli_errors RETURN doc", 10, true);
        run(&mut table, "FOR doc IN _soli_errors RETURN doc", 10, true);
        assert!(drained(&mut table).is_empty());
        assert!(table.touched.is_empty());
    }

    #[test]
    fn the_peak_is_the_most_runs_in_one_unit() {
        let mut table = Table::default();
        for _ in 0..3 {
            run(
                &mut table,
                "FOR c IN comments FILTER c.post == @p RETURN c",
                5,
                true,
            );
        }
        table.end_unit(Some("GET /posts → posts#index"));
        for _ in 0..40 {
            run(
                &mut table,
                "FOR c IN comments FILTER c.post == @p RETURN c",
                5,
                true,
            );
        }
        table.end_unit(Some("GET /feed → feed#index"));
        run(
            &mut table,
            "FOR c IN comments FILTER c.post == @p RETURN c",
            5,
            true,
        );
        table.end_unit(Some("GET /one → posts#show"));
        // Outside a unit (a LiveView event): counted, never a peak.
        for _ in 0..500 {
            run(
                &mut table,
                "FOR c IN comments FILTER c.post == @p RETURN c",
                5,
                false,
            );
        }
        let group = drained(&mut table).into_values().next().unwrap();
        assert_eq!(group.totals.count, 544);
        assert_eq!(group.totals.peak, 40);
        assert_eq!(group.totals.peak_context, "GET /feed → feed#index");
    }

    #[test]
    fn an_unfinished_unit_is_forgotten_not_counted_as_a_peak() {
        let mut table = Table::default();
        for _ in 0..9 {
            run(&mut table, "RETURN 1", 5, true);
        }
        table.end_unit(None);
        assert_eq!(drained(&mut table).values().next().unwrap().totals.peak, 0);
    }

    #[test]
    fn an_ignored_thread_counts_nothing() {
        let mut table = Table {
            ignored: true,
            ..Table::default()
        };
        run(&mut table, "FOR doc IN _jobs RETURN doc", 5, true);
        assert!(table.entries.is_empty() && table.touched.is_empty());
    }

    #[test]
    fn a_text_not_run_since_the_last_drain_is_dropped() {
        let mut table = Table::default();
        run(&mut table, "RETURN 1", 5, false);
        drained(&mut table);
        assert_eq!(table.entries.len(), 1, "run in the window: kept");
        assert!(drained(&mut table).is_empty());
        assert_eq!(table.entries.len(), 0, "idle for a window: dropped");
    }

    #[test]
    fn past_the_cap_new_texts_go_to_the_overflow_group() {
        let mut table = Table::default();
        for i in 0..MAX_TEXTS_PER_THREAD {
            table.record(i as u64, APP, Dialect::Sdbql, 1, false, || {
                Some(format!("RETURN \"{i}\""))
            });
        }
        table.record(u64::MAX, APP, Dialect::Sdbql, 9, false, || {
            panic!("the text of an overflowed query is never read")
        });
        let groups = drained(&mut table);
        let overflow = &groups[OVERFLOW_FINGERPRINT];
        assert_eq!((overflow.totals.count, overflow.totals.total_us), (1, 9));
        assert_eq!(overflow.shape, OVERFLOW_SHAPE);
    }

    #[test]
    fn applications_are_counted_apart() {
        let mut table = Table::default();
        let key = HASHER.hash_one("RETURN 1");
        table.record(key, TenantId(0), Dialect::Sdbql, 1, false, || {
            Some("RETURN 1".into())
        });
        table.record(key, TenantId(1), Dialect::Sdbql, 1, false, || {
            Some("RETURN 1".into())
        });
        let mut out = HashMap::new();
        table.drain_into(&mut out);
        assert_eq!(out.len(), 2);
    }

    fn group(count: u64, total_us: u64, peak: u32, context: &str) -> Group {
        Group {
            shape: "RETURN ?".to_string(),
            totals: Totals {
                count,
                total_us,
                max_us: total_us / count.max(1),
                peak,
                peak_context: context.to_string(),
            },
        }
    }

    #[test]
    fn windows_add_up_by_hour_and_keep_the_days_peak() {
        let first = merged(
            None,
            group(10, 20_000, 12, "GET /a"),
            "2026-09-25T18:01:00Z",
        );
        assert_eq!(first["count"], 10);
        assert_eq!(first["total_ms"], 20.0);
        assert_eq!(first["peak_per_request"], 12);
        assert_eq!(first["peak_context"], "GET /a");
        assert_eq!(first["first_seen"], "2026-09-25T18:01:00Z");

        let second = merged(
            Some(&first),
            group(5, 5_000, 3, "GET /b"),
            "2026-09-25T18:02:00Z",
        );
        assert_eq!(second["count"], 15);
        assert_eq!(second["total_ms"], 25.0);
        assert_eq!(
            second["peak_per_request"], 12,
            "a lower peak does not replace it"
        );
        assert_eq!(second["peak_context"], "GET /a");
        assert_eq!(second["first_seen"], "2026-09-25T18:01:00Z");
        let hourly = parse_hourly(second.get("hourly"));
        assert_eq!(
            hourly["2026-09-25T18"],
            Bucket {
                count: 15,
                ms: 25.0,
                peak: 12
            }
        );
    }

    #[test]
    fn a_peak_ages_out_with_its_hours() {
        let start = chrono::DateTime::parse_from_rfc3339("2026-09-24T00:00:00Z").unwrap();
        let mut doc = merged(
            None,
            group(1, 1_000, 30, "GET /n+1"),
            "2026-09-24T00:00:00Z",
        );
        for hour in 1..=HOURS_KEPT as i64 {
            let at = (start + chrono::Duration::hours(hour))
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
            doc = merged(Some(&doc), group(1, 1_000, 1, "GET /fixed"), &at);
        }
        assert_eq!(parse_hourly(doc.get("hourly")).len(), HOURS_KEPT);
        assert_eq!(doc["peak_per_request"], 1);
        assert_eq!(doc["peak_context"], "");
    }

    #[derive(Default)]
    struct MemStore {
        docs: StdMutex<BTreeMap<String, serde_json::Value>>,
    }

    impl Store for MemStore {
        fn ensure(&self) -> Result<(), String> {
            Ok(())
        }
        fn get(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(self.docs.lock().unwrap().get(key).cloned())
        }
        fn insert(&self, key: &str, doc: serde_json::Value) -> Result<(), String> {
            self.docs.lock().unwrap().insert(key.to_string(), doc);
            Ok(())
        }
        fn patch(&self, key: &str, fields: serde_json::Value) -> Result<(), String> {
            self.insert(key, fields)
        }
        fn count(&self) -> Result<u64, String> {
            Ok(self.docs.lock().unwrap().len() as u64)
        }
        fn oldest(&self, limit: usize) -> Result<Vec<(String, String)>, String> {
            let mut rows: Vec<(String, String)> = self
                .docs
                .lock()
                .unwrap()
                .iter()
                .map(|(k, d)| (k.clone(), str_of(d, "last_seen")))
                .collect();
            rows.sort_by(|a, b| a.1.cmp(&b.1));
            rows.truncate(limit);
            Ok(rows)
        }
        fn delete(&self, key: &str) -> Result<bool, String> {
            Ok(self.docs.lock().unwrap().remove(key).is_some())
        }
    }

    #[test]
    fn a_full_table_makes_room_by_pruning_stale_shapes_only() {
        let store = MemStore::default();
        let stats = Stats::default();
        let stale = (chrono::Utc::now() - chrono::Duration::days(STALE_AFTER_DAYS + 1))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let fresh = crate::jobs::now_iso();
        for i in 0..MAX_GROUPS {
            let at = if i < 3 { &stale } else { &fresh };
            store
                .insert(&format!("{i:016x}"), serde_json::json!({ "last_seen": at }))
                .unwrap();
        }
        let mut flusher = Flusher::new(&store, &stats);
        let groups: HashMap<String, Group> = (0..5)
            .map(|i| (format!("f{i:015x}"), group(1, 1_000, 1, "")))
            .collect();
        flusher.flush(Batch {
            at: fresh.clone(),
            groups,
        });
        assert_eq!(
            store.count().unwrap(),
            MAX_GROUPS,
            "three stale out, three new in"
        );
        assert_eq!(stats.overflowed.load(Ordering::Relaxed), 2);
        assert!(store.get("0000000000000000").unwrap().is_none());
        assert!(store.get("0000000000000003").unwrap().is_some());
    }
}
