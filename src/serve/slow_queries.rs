//! Built-in slow-query tracking: every database query slower than
//! `SOLI_SLOW_QUERY_MS` is grouped by shape and stored in the app's own
//! database, for `/__soli/slow_queries`.
//!
//! Every query the ORM runs — SoliDB over HTTP or the native driver, and the
//! Postgres, MySQL and SQLite adapters — already passes through one of a few
//! timing points that feed the dev query log and the Prometheus DB counter.
//! Each calls [`observe`] with the elapsed time. A fast query costs one
//! comparison there: the query text and binds are only read, redacted and
//! copied once a query is over the threshold.
//!
//! A slow query is handed to a per-application writer thread over a bounded
//! channel ([`tenant_writer`]), which batches for [`FLUSH_EVERY`] and writes one
//! update per group into `_soli_slow_queries`:
//!
//! ```text
//! { _key: <fingerprint>, query: <the shape>, count, total_ms, max_ms,
//!   first_seen, last_seen, last_context: "GET /orders → orders#index",
//!   hourly: [ ["2026-09-25T18", 12], … ],
//!   samples: [ the slowest MAX_SAMPLES, slowest first ] }
//! ```
//!
//! The shape is the query with its literals taken out — string and number
//! literals become `?` and a list of them one `?` — so the same query with a
//! different id is one group, and a query that inlines its values does not
//! grow the table by one row per value. At most [`MAX_GROUPS`] groups are kept;
//! a new shape past that is counted, not stored.
//!
//! Samples keep the query as it ran and its binds, bind values under a
//! secret-looking name redacted and long values cut; `SOLI_SLOW_QUERY_BINDS=off`
//! keeps no binds at all. Queries against the framework's own `_soli_*`
//! collections, and the writer's own writes, are never recorded.
//!
//! On by default at 200 ms; `SOLI_SLOW_QUERIES=off` disables it, and it stays
//! off under `APP_ENV=test` unless `SOLI_SLOW_QUERIES=on`.
//!
//! [`tenant_writer`]: super::tenant_writer

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::internal_store;
use super::tenant::TenantId;
pub(crate) use super::tenant_writer::StatsSnapshot;
use super::tenant_writer::{self, Stats, Writers};

pub(crate) const SLOW_COLLECTION: &str = "_soli_slow_queries";

/// Samples kept per group: the slowest, slowest first.
pub(crate) const MAX_SAMPLES: usize = 5;

/// Distinct query shapes kept per application.
pub(crate) const MAX_GROUPS: u64 = 1000;

/// Hours of per-hour counts kept on a group, for the list's trend line.
pub(crate) const HOURS_KEPT: usize = 24;

const DEFAULT_THRESHOLD_MS: u64 = 200;
const QUEUE_CAP: usize = 1024;
const FLUSH_EVERY: Duration = Duration::from_secs(1);
const MAX_QUERY_CHARS: usize = 4000;
const MAX_BIND_CHARS: usize = 200;
const MAX_CONTEXT_CHARS: usize = 300;

/// The sort orders the dashboard offers, as (name, stored field).
pub(crate) const ORDERS: [(&str, &str); 4] = [
    ("impact", "total_ms"),
    ("slowest", "max_ms"),
    ("frequent", "count"),
    ("recent", "last_seen"),
];

/// Threshold in whole milliseconds; `u64::MAX` until first read.
static THRESHOLD_MS: AtomicU64 = AtomicU64::new(u64::MAX);

static WRITERS: Writers<SlowQuery> = Writers::new("slow-queries", QUEUE_CAP);

thread_local! {
    /// What this worker is running: `GET /orders → orders#index`, or
    /// `job ReportJob`. Set per request and per job, into a reused buffer so
    /// setting it allocates nothing once warm.
    static CONTEXT: RefCell<String> = const { RefCell::new(String::new()) };
    /// Set on the writer threads: their own reads and writes are not
    /// application queries.
    static SUPPRESSED: Cell<bool> = const { Cell::new(false) };
}

/// Queries at or above this many milliseconds are slow.
pub(crate) fn threshold_ms() -> u64 {
    let cached = THRESHOLD_MS.load(Ordering::Relaxed);
    if cached != u64::MAX {
        return cached;
    }
    let value = std::env::var("SOLI_SLOW_QUERY_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_THRESHOLD_MS);
    THRESHOLD_MS.store(value, Ordering::Relaxed);
    value
}

/// Whether slow queries are recorded at all.
pub(crate) fn enabled() -> bool {
    let setting = std::env::var("SOLI_SLOW_QUERIES")
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

fn keep_binds() -> bool {
    !matches!(
        std::env::var("SOLI_SLOW_QUERY_BINDS")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "off" | "0" | "false" | "no"
    )
}

/// This application's recording counters, since the process started.
pub(crate) fn stats() -> StatsSnapshot {
    WRITERS.stats(super::tenant::current_id())
}

/// Start a request's context: `GET /orders`.
pub(crate) fn begin_request(method: &str, path: &str) {
    CONTEXT.with(|c| {
        let mut c = c.borrow_mut();
        c.clear();
        c.push_str(method);
        c.push(' ');
        c.push_str(path);
    });
}

/// Add the matched handler: `GET /orders → orders#index`.
pub(crate) fn note_handler(handler: &str) {
    CONTEXT.with(|c| {
        let mut c = c.borrow_mut();
        c.push_str(" \u{2192} ");
        c.push_str(handler);
    });
}

/// Name what this worker thread is about to run outside a request: `job ReportJob`.
pub(crate) fn set_context(label: &str) {
    CONTEXT.with(|c| {
        let mut c = c.borrow_mut();
        c.clear();
        c.push_str(label);
    });
}

/// Whether a query that took `ms` is worth a second look. The one check every
/// query pays; call sites use it to keep their inputs only when it is true.
#[inline]
pub fn is_slow(ms: f64) -> bool {
    ms >= threshold_ms() as f64
}

/// Which language a query is written in: it decides what a quote means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// SoliDB's SDBQL: `"…"` and `'…'` are string literals.
    Sdbql,
    /// Postgres, MySQL, SQLite as the adapters write them: `"…"` is an
    /// identifier and a quoted string is a JSON key the adapter put there —
    /// values are always binds. Both are the query's shape and are kept.
    Sql,
}

/// Record `query` if it was slow. `binds` is only called then.
pub fn observe(
    query: &str,
    dialect: Dialect,
    ms: f64,
    binds: impl FnOnce() -> Option<HashMap<String, serde_json::Value>>,
) {
    if !is_slow(ms) || SUPPRESSED.with(|s| s.get()) || query.contains("_soli_") || !enabled() {
        return;
    }
    let shape = normalize_query(query, dialect);
    let binds = if keep_binds() {
        binds().map(redact_binds)
    } else {
        None
    };
    let context = CONTEXT.with(|c| c.borrow().clone());
    let at = crate::jobs::now_iso();
    let sample = serde_json::json!({
        "at": at,
        "ms": round_ms(ms),
        "context": context,
        "query": truncate_chars(query, MAX_QUERY_CHARS),
        "binds": binds,
    });
    let item = SlowQuery {
        fingerprint: fingerprint(&shape),
        shape: truncate_chars(&shape, MAX_QUERY_CHARS),
        ms,
        context: truncate_chars(&context, MAX_CONTEXT_CHARS),
        at,
        sample,
    };
    WRITERS.deliver(super::tenant::current_id(), item, &spawn_writer);
}

struct SlowQuery {
    fingerprint: String,
    shape: String,
    ms: f64,
    context: String,
    at: String,
    sample: serde_json::Value,
}

fn spawn_writer(tenant: TenantId, receiver: Receiver<SlowQuery>, stats: Arc<Stats>) -> bool {
    tenant_writer::spawn_thread("slow-queries", tenant, move || {
        SUPPRESSED.with(|s| s.set(true));
        run_writer(receiver, &DbStore, &stats)
    })
}

/// A shape's slow runs gathered during a flush window.
struct Pending {
    shape: String,
    count: u64,
    total_ms: f64,
    max_ms: f64,
    first_at: String,
    last_at: String,
    last_context: String,
    hourly: BTreeMap<String, u64>,
    /// Slowest first, at most MAX_SAMPLES.
    samples: Vec<serde_json::Value>,
}

fn run_writer(receiver: Receiver<SlowQuery>, store: &dyn Store, stats: &Stats) {
    let mut flusher = Flusher::new(store, stats);
    while let Ok(first) = receiver.recv() {
        let mut pending: HashMap<String, Pending> = HashMap::new();
        gather(&mut pending, first);
        let deadline = Instant::now() + FLUSH_EVERY;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left) {
                Ok(item) => gather(&mut pending, item),
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        flusher.flush(pending);
    }
}

fn gather(pending: &mut HashMap<String, Pending>, item: SlowQuery) {
    let group = pending.entry(item.fingerprint).or_insert_with(|| Pending {
        shape: item.shape.clone(),
        count: 0,
        total_ms: 0.0,
        max_ms: 0.0,
        first_at: item.at.clone(),
        last_at: item.at.clone(),
        last_context: String::new(),
        hourly: BTreeMap::new(),
        samples: Vec::new(),
    });
    group.count += 1;
    group.total_ms += item.ms;
    group.max_ms = group.max_ms.max(item.ms);
    *group.hourly.entry(hour_of(&item.at)).or_insert(0) += 1;
    group.last_at = item.at;
    group.last_context = item.context;
    group.samples.push(item.sample);
    keep_slowest(&mut group.samples);
}

/// Sort samples slowest first and keep MAX_SAMPLES.
fn keep_slowest(samples: &mut Vec<serde_json::Value>) {
    let ms = |v: &serde_json::Value| v.get("ms").and_then(|m| m.as_f64()).unwrap_or(0.0);
    samples.sort_by(|a, b| ms(b).total_cmp(&ms(a)));
    samples.truncate(MAX_SAMPLES);
}

/// Where the writer keeps groups: the app's database, or a map in the tests.
trait Store {
    fn ensure(&self) -> Result<(), String>;
    fn get(&self, key: &str) -> Result<Option<serde_json::Value>, String>;
    fn insert(&self, key: &str, doc: serde_json::Value) -> Result<(), String>;
    fn patch(&self, key: &str, fields: serde_json::Value) -> Result<(), String>;
    fn count(&self) -> Result<u64, String>;
}

struct DbStore;

impl Store for DbStore {
    fn ensure(&self) -> Result<(), String> {
        internal_store::ensure(SLOW_COLLECTION, "total_ms")
    }
    fn get(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        internal_store::get(SLOW_COLLECTION, key)
    }
    fn insert(&self, key: &str, doc: serde_json::Value) -> Result<(), String> {
        internal_store::insert(SLOW_COLLECTION, key, doc)
    }
    fn patch(&self, key: &str, fields: serde_json::Value) -> Result<(), String> {
        internal_store::patch(SLOW_COLLECTION, key, fields)
    }
    fn count(&self) -> Result<u64, String> {
        internal_store::count(SLOW_COLLECTION)
    }
}

struct Flusher<'a> {
    store: &'a dyn Store,
    stats: &'a Stats,
    ensured: bool,
    /// Groups known to exist; `None` until counted.
    groups: Option<u64>,
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

    fn flush(&mut self, pending: HashMap<String, Pending>) {
        self.recounted = false;
        if !self.ensured {
            let store = self.store;
            self.ensured = match tenant_writer::fenced(|| store.ensure()) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("[slow-queries] could not prepare {SLOW_COLLECTION}: {e}");
                    false
                }
            };
        }
        for (fingerprint, group) in pending {
            let count = group.count;
            match tenant_writer::fenced(|| self.write_group(&fingerprint, group)) {
                Ok(true) => {}
                Ok(false) => {
                    self.stats.overflowed.fetch_add(count, Ordering::Relaxed);
                }
                Err(e) => {
                    self.stats.failed.fetch_add(count, Ordering::Relaxed);
                    eprintln!("[slow-queries] could not record query group {fingerprint}: {e}");
                }
            }
        }
    }

    /// Store one group's window. `Ok(false)` when it is a new shape and there
    /// is no room for another group.
    fn write_group(&mut self, fingerprint: &str, group: Pending) -> Result<bool, String> {
        if let Some(existing) = self.store.get(fingerprint)? {
            self.store
                .patch(fingerprint, merged_patch(&existing, group))?;
            return Ok(true);
        }
        if !self.has_room()? {
            return Ok(false);
        }
        let event = super::notify::Event::slow_query(
            fingerprint,
            &group.shape,
            group.max_ms,
            &group.last_context,
        );
        let doc = serde_json::json!({
            "query": group.shape,
            "count": group.count,
            "total_ms": round_ms(group.total_ms),
            "max_ms": round_ms(group.max_ms),
            "first_seen": group.first_at,
            "last_seen": group.last_at,
            "last_context": group.last_context,
            "hourly": hourly_json(group.hourly),
            "samples": group.samples,
        });
        self.store.insert(fingerprint, doc)?;
        if let Some(n) = self.groups.as_mut() {
            *n += 1;
        }
        super::notify::emit(event);
        Ok(true)
    }

    fn has_room(&mut self) -> Result<bool, String> {
        match self.groups {
            Some(n) if n < MAX_GROUPS => return Ok(true),
            Some(_) if self.recounted => return Ok(false),
            _ => {}
        }
        let n = self.store.count()?;
        self.groups = Some(n);
        self.recounted = true;
        Ok(n < MAX_GROUPS)
    }
}

/// The update an existing group receives for a window's slow runs.
fn merged_patch(existing: &serde_json::Value, group: Pending) -> serde_json::Value {
    let number = |field: &str| existing.get(field).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let count = existing.get("count").and_then(|v| v.as_u64()).unwrap_or(0) + group.count;
    let mut samples = group.samples;
    if let Some(old) = existing.get("samples").and_then(|v| v.as_array()) {
        samples.extend(old.iter().cloned());
    }
    keep_slowest(&mut samples);
    let mut hourly = parse_hourly(existing.get("hourly"));
    for (hour, n) in group.hourly {
        *hourly.entry(hour).or_insert(0) += n;
    }
    serde_json::json!({
        "query": group.shape,
        "count": count,
        "total_ms": round_ms(number("total_ms") + group.total_ms),
        "max_ms": round_ms(number("max_ms").max(group.max_ms)),
        "last_seen": group.last_at,
        "last_context": group.last_context,
        "hourly": hourly_json(hourly),
        "samples": samples,
    })
}

/// `2026-09-25T18:34:17Z` → `2026-09-25T18`.
fn hour_of(iso: &str) -> String {
    iso.chars().take(13).collect()
}

/// The newest [`HOURS_KEPT`] hours as `[[hour, count], …]`, oldest first.
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
    super::error_tracker::parse_hourly(value)
}

fn round_ms(ms: f64) -> f64 {
    (ms * 10.0).round() / 10.0
}

// --- the store the dashboard reads -------------------------------------------

/// One group, or `None` when the fingerprint is unknown.
pub(crate) fn get(fingerprint: &str) -> Result<Option<serde_json::Value>, String> {
    internal_store::get(SLOW_COLLECTION, fingerprint)
}

/// Groups in `order` (one of [`ORDERS`]), without their samples.
pub(crate) fn list(order: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
    let Some((_, field)) = ORDERS.iter().find(|(name, _)| *name == order) else {
        return Err(format!("unknown order {order:?}"));
    };
    internal_store::list(
        SLOW_COLLECTION,
        None,
        field,
        true,
        limit,
        &[
            "query",
            "count",
            "total_ms",
            "max_ms",
            "first_seen",
            "last_seen",
            "last_context",
            "hourly",
        ],
    )
}

/// Forget a group. `false` when it did not exist.
pub(crate) fn delete(fingerprint: &str) -> Result<bool, String> {
    internal_store::delete(SLOW_COLLECTION, fingerprint)
}

pub(crate) fn valid_fingerprint(value: &str) -> bool {
    super::error_tracker::valid_fingerprint(value)
}

// --- grouping ----------------------------------------------------------------

fn fingerprint(shape: &str) -> String {
    let digest = Sha256::digest(shape.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// The query with its literals taken out: numbers — and in SDBQL quoted
/// strings — become `?`, a run of them in a list becomes one `?`, and
/// whitespace collapses. Placeholders (`@name`, `$1`, `?`) and identifiers
/// holding digits (`t1`) are kept.
pub(crate) fn normalize_query(query: &str, dialect: Dialect) -> String {
    let chars: Vec<char> = query.chars().collect();
    let word_char = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(query.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' || c == '`' {
            // A backtick quotes an identifier (MySQL): keep it. So does any
            // quote in the adapters' SQL (see `Dialect::Sql`).
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '\\' {
                    j += 2;
                    continue;
                }
                if chars[j] == c {
                    // SQL doubles a quote to escape it.
                    if chars.get(j + 1) == Some(&c) {
                        j += 2;
                        continue;
                    }
                    break;
                }
                j += 1;
            }
            if c == '`' || dialect == Dialect::Sql {
                out.extend(&chars[i..(j + 1).min(chars.len())]);
            } else {
                push_literal(&mut out);
            }
            i = j + 1;
            continue;
        }
        if c.is_ascii_digit()
            && (i == 0 || !(word_char(chars[i - 1]) || chars[i - 1] == '$' || chars[i - 1] == '@'))
        {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.') {
                j += 1;
            }
            push_literal(&mut out);
            i = j;
            continue;
        }
        if c.is_whitespace() {
            if !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    let trimmed = out.trim_end().to_string();
    collapse_literal_lists(&trimmed)
}

fn push_literal(out: &mut String) {
    out.push('?');
}

/// `IN (?, ?, ?)` and `[?, ?]` → `IN (?)` and `[?]`, so a list of ids of any
/// length is one shape.
fn collapse_literal_lists(shape: &str) -> String {
    let mut out = shape.to_string();
    for (from, to) in [("?, ?", "?"), ("?,?", "?")] {
        while out.contains(from) {
            out = out.replace(from, to);
        }
    }
    out
}

/// Bind values as they will be stored: a value under a secret-looking name
/// redacted, a long value cut to its first characters and its real length.
fn redact_binds(binds: HashMap<String, serde_json::Value>) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = binds
        .into_iter()
        .map(|(name, value)| {
            let value = if crate::redaction::looks_sensitive(&name) {
                serde_json::Value::String("[REDACTED]".into())
            } else {
                cut_value(value)
            };
            (name, value)
        })
        .collect();
    serde_json::Value::Object(map)
}

fn cut_value(value: serde_json::Value) -> serde_json::Value {
    let text = match &value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => value.to_string(),
        _ => return value,
    };
    let chars = text.chars().count();
    if chars <= MAX_BIND_CHARS {
        return value;
    }
    let head: String = text.chars().take(MAX_BIND_CHARS).collect();
    serde_json::Value::String(format!("{head}\u{2026} ({chars} chars)"))
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
    use std::sync::Mutex;

    use Dialect::{Sdbql, Sql};

    #[test]
    fn literals_become_placeholders() {
        assert_eq!(
            normalize_query(
                "FOR u IN users FILTER u.id == 42 AND u.name == \"bob\" RETURN u",
                Sdbql
            ),
            "FOR u IN users FILTER u.id == ? AND u.name == ? RETURN u"
        );
        assert_eq!(
            normalize_query("FOR u IN users FILTER u.note == 'it\\'s' RETURN u", Sdbql),
            "FOR u IN users FILTER u.note == ? RETURN u"
        );
    }

    #[test]
    fn sql_keeps_its_identifiers_and_json_keys() {
        assert_eq!(
            normalize_query(
                "SELECT doc FROM \"items\" WHERE (doc ->> 'tag') = $1 LIMIT 20 OFFSET 40",
                Sql
            ),
            "SELECT doc FROM \"items\" WHERE (doc ->> 'tag') = $1 LIMIT ? OFFSET ?"
        );
        assert_ne!(
            normalize_query("SELECT doc FROM \"items\" WHERE (doc ->> 'tag') = ?", Sql),
            normalize_query("SELECT doc FROM \"items\" WHERE (doc ->> 'name') = ?", Sql)
        );
    }

    #[test]
    fn placeholders_and_identifiers_are_kept() {
        assert_eq!(
            normalize_query(
                "SELECT * FROM `order_2` WHERE id = $1 AND k = @key2 AND x = ?",
                Sql
            ),
            "SELECT * FROM `order_2` WHERE id = $1 AND k = @key2 AND x = ?"
        );
    }

    #[test]
    fn a_list_of_any_length_is_one_shape() {
        assert_eq!(
            normalize_query("SELECT * FROM t WHERE id IN (1, 2, 3)", Sql),
            normalize_query("SELECT * FROM t WHERE id IN (7)", Sql)
        );
        assert_eq!(
            normalize_query("FILTER d._key IN [\"a\",\"b\"]", Sdbql),
            "FILTER d._key IN [?]"
        );
    }

    #[test]
    fn whitespace_does_not_split_a_group() {
        assert_eq!(
            fingerprint(&normalize_query("SELECT *\n  FROM t\tWHERE a = 1", Sql)),
            fingerprint(&normalize_query("SELECT * FROM t WHERE a = 2", Sql))
        );
    }

    #[test]
    fn secret_binds_are_redacted_and_long_ones_cut() {
        let binds = HashMap::from([
            ("password".to_string(), serde_json::json!("hunter2")),
            ("email".to_string(), serde_json::json!("a@b.c")),
            ("blob".to_string(), serde_json::json!("x".repeat(500))),
        ]);
        let out = redact_binds(binds);
        assert_eq!(out["password"], "[REDACTED]");
        assert_eq!(out["email"], "a@b.c");
        let blob = out["blob"].as_str().unwrap();
        assert!(blob.ends_with("(500 chars)"), "{blob}");
    }

    fn pending(ms: &[f64]) -> Pending {
        let mut p = Pending {
            shape: "q".into(),
            count: 0,
            total_ms: 0.0,
            max_ms: 0.0,
            first_at: "2026-09-25T10:00:00Z".into(),
            last_at: "2026-09-25T10:00:01Z".into(),
            last_context: "GET /x".into(),
            hourly: BTreeMap::from([("2026-09-25T10".to_string(), ms.len() as u64)]),
            samples: Vec::new(),
        };
        for m in ms {
            p.count += 1;
            p.total_ms += m;
            p.max_ms = p.max_ms.max(*m);
            p.samples.push(serde_json::json!({ "ms": m }));
        }
        keep_slowest(&mut p.samples);
        p
    }

    #[test]
    fn a_merge_adds_up_and_keeps_the_slowest_samples() {
        let existing = serde_json::json!({
            "count": 3, "total_ms": 900.0, "max_ms": 400.0,
            "samples": [{"ms": 400.0}, {"ms": 300.0}, {"ms": 200.0}],
            "hourly": [["2026-09-25T10", 3]],
        });
        let patch = merged_patch(&existing, pending(&[250.0, 800.0, 210.0]));
        assert_eq!(patch["count"], 6);
        assert_eq!(patch["total_ms"], 2160.0);
        assert_eq!(patch["max_ms"], 800.0);
        let kept: Vec<f64> = patch["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["ms"].as_f64().unwrap())
            .collect();
        assert_eq!(kept, vec![800.0, 400.0, 300.0, 250.0, 210.0]);
        assert_eq!(patch["hourly"], serde_json::json!([["2026-09-25T10", 6]]));
    }

    #[derive(Default)]
    struct MemStore {
        docs: Mutex<HashMap<String, serde_json::Value>>,
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
            let mut docs = self.docs.lock().unwrap();
            let doc = docs.get_mut(key).ok_or("missing")?;
            for (k, v) in fields.as_object().unwrap() {
                doc[k] = v.clone();
            }
            Ok(())
        }
        fn count(&self) -> Result<u64, String> {
            Ok(self.docs.lock().unwrap().len() as u64)
        }
    }

    #[test]
    fn new_shapes_past_the_cap_are_counted_not_stored() {
        let store = MemStore::default();
        for i in 0..MAX_GROUPS {
            store
                .insert(&format!("{i:016x}"), serde_json::json!({}))
                .unwrap();
        }
        let stats = Stats::default();
        let mut flusher = Flusher::new(&store, &stats);
        flusher.flush(HashMap::from([(
            "ffffffffffffffff".to_string(),
            pending(&[300.0, 400.0]),
        )]));
        assert!(store.get("ffffffffffffffff").unwrap().is_none());
        assert_eq!(stats.snapshot().overflowed, 2);
        // An existing shape still counts.
        flusher.flush(HashMap::from([(format!("{:016x}", 1), pending(&[300.0]))]));
        assert_eq!(
            store.get(&format!("{:016x}", 1)).unwrap().unwrap()["count"],
            1
        );
    }
}
