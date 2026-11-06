//! App usage stats for the `/__soli/` overview: requests per route pattern and
//! per period, their status classes and a latency histogram.
//!
//! Recorded on the worker, around [`super::dispatch_http_request`]: every
//! request the application answered, whatever its status, minus the framework's
//! own endpoints (`/__soli/*`, `/__solidev/*`, the probes, `/up`) and a miss
//! for what looks like a static file. Static files and the operator pages are
//! answered before a worker is asked, so they never reach it in the first place.
//!
//! # The hot path
//!
//! Each worker thread owns a [`Shard`]: one counter cell per route slot for the
//! current five-minute period, behind a mutex that only this thread and a
//! reader ever take. Recording a request is that uncontended lock, two counter
//! increments and the unlock — no allocation, no shared cache line between
//! workers. When a thread meets a new period it folds its cells into the
//! application's [`Ring`] (one shared lock, once per five minutes per thread)
//! and starts over. A reader folds every shard first, so what it shows is live.
//!
//! A route is a *pattern*: `GET /posts/:id`, never `/posts/42`. The matched
//! route's position in the table is turned into a slot once per thread
//! ([`note_route`]); at most [`MAX_ROUTES`] slots exist per application, slot 0
//! collects every route past the cap ("other routes") and slot 1 every request
//! no route matched.
//!
//! # What is kept
//!
//! A [`Cell`] is four status classes (2xx and below, 3xx, 4xx, 5xx and above)
//! and [`LAT_BUCKETS`] power-of-two latency buckets from 250 µs, enough for a
//! p50/p95 within a bucket (interpolated). Per application:
//!
//! - five-minute cells per route for the last two hours (the 1 h view and the
//!   hour before it);
//! - hourly cells per route for the last seven days;
//! - hourly totals for the last fourteen days (the 7 d view and the week
//!   before it).
//!
//! # Persistence
//!
//! The 1 h and 24 h views come from memory. Hourly cells are also written to
//! `_soli_usage` in the app's own database, one document per hour, by a
//! collector that wakes every minute and hands each application's new counts
//! to its writer ([`tenant_writer`]) — a write per hour per minute at most,
//! never one per request. The writer reads the stored hours back once when it
//! starts, so the 7 d view survives a restart. Counts are added to the stored
//! document, so several processes sharing a database add up (a concurrent
//! read-modify-write across hosts can undercount, as the error tracker's can).
//!
//! On by default; `SOLI_USAGE=off` turns it off, and it stays off under
//! `APP_ENV=test` unless `SOLI_USAGE=on`.
//!
//! [`tenant_writer`]: super::tenant_writer

use std::cell::{Cell as StdCell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, OnceLock};
use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::internal_store;
use super::tenant::TenantId;
use super::tenant_writer::{self, Stats, StatsSnapshot, Writers};

/// Route slots per application, the two reserved ones included.
pub(crate) const MAX_ROUTES: usize = 64;
/// Routes past [`MAX_ROUTES`].
pub(crate) const OTHER: u16 = 0;
/// Requests no route matched.
pub(crate) const UNMATCHED: u16 = 1;
pub(crate) const OTHER_LABEL: &str = "other routes";
pub(crate) const UNMATCHED_LABEL: &str = "no route matched";

/// Latency buckets: `[0, 250 µs)`, then `[250·2^(i-1), 250·2^i)` µs; the last
/// one is everything from about 65 s.
pub(crate) const LAT_BUCKETS: usize = 20;
const FIRST_BOUND_US: u64 = 250;
const CLASSES: usize = 4;
const CELL_LEN: usize = CLASSES + LAT_BUCKETS;

pub(crate) const FINE_SECS: u64 = 300;
pub(crate) const HOUR_SECS: u64 = 3600;
/// Five-minute periods kept: two hours and the one being filled.
const FINE_KEPT: u64 = 25;
/// Hours kept per route: seven days and the one being filled.
const ROUTE_HOURS_KEPT: u64 = 7 * 24 + 1;
/// Hours kept as totals: fourteen days and the one being filled.
const TOTAL_HOURS_KEPT: u64 = 14 * 24 + 1;

pub(crate) const USAGE_COLLECTION: &str = "_soli_usage";
const COLLECT_EVERY: Duration = Duration::from_secs(60);
const QUEUE_CAP: usize = 8;
/// Stored hours older than this are deleted.
const STORED_DAYS: u64 = 15;

// --- one cell ----------------------------------------------------------------

/// Requests by status class, and their latencies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cell([u32; CELL_LEN]);

impl Default for Cell {
    fn default() -> Self {
        Cell([0; CELL_LEN])
    }
}

impl Cell {
    fn add(&mut self, class: usize, bucket: usize) {
        self.0[class] = self.0[class].saturating_add(1);
        let b = CLASSES + bucket;
        self.0[b] = self.0[b].saturating_add(1);
    }

    fn merge(&mut self, other: &Cell) {
        for (mine, theirs) in self.0.iter_mut().zip(other.0.iter()) {
            *mine = mine.saturating_add(*theirs);
        }
    }

    fn is_empty(&self) -> bool {
        self.0[..CLASSES].iter().all(|n| *n == 0)
    }

    fn to_vec(self) -> Vec<u64> {
        self.0.iter().map(|n| *n as u64).collect()
    }

    fn from_slice(values: &[u64]) -> Option<Cell> {
        if values.len() != CELL_LEN {
            return None;
        }
        let mut cell = Cell::default();
        for (slot, value) in cell.0.iter_mut().zip(values) {
            *slot = (*value).min(u32::MAX as u64) as u32;
        }
        Some(cell)
    }
}

/// 0 for 1xx and 2xx, 1 for 3xx, 2 for 4xx, 3 for 5xx and anything stranger.
pub(crate) fn status_class(status: u16) -> usize {
    match status {
        0..=299 => 0,
        300..=399 => 1,
        400..=499 => 2,
        _ => 3,
    }
}

pub(crate) fn latency_bucket(micros: u64) -> usize {
    if micros < FIRST_BOUND_US {
        return 0;
    }
    let q = micros / FIRST_BOUND_US;
    (1 + (63 - q.leading_zeros()) as usize).min(LAT_BUCKETS - 1)
}

/// `[lo, hi)` of bucket `i`, in µs; the last bucket's `hi` is its `lo`.
fn bucket_bounds(i: usize) -> (u64, u64) {
    if i == 0 {
        return (0, FIRST_BOUND_US);
    }
    let lo = FIRST_BOUND_US << (i - 1);
    if i == LAT_BUCKETS - 1 {
        (lo, lo)
    } else {
        (lo, lo * 2)
    }
}

/// The `q` quantile (0..=1) of a histogram, in µs: the bucket it falls in,
/// interpolated geometrically inside it (linearly in the first). `None` with
/// nothing counted.
pub(crate) fn quantile(hist: &[u64; LAT_BUCKETS], q: f64) -> Option<u64> {
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return None;
    }
    let rank = (q.clamp(0.0, 1.0) * total as f64).max(1.0);
    let mut before = 0u64;
    for (i, n) in hist.iter().enumerate() {
        if *n == 0 {
            continue;
        }
        if (before + n) as f64 >= rank {
            let frac = ((rank - before as f64) / *n as f64).clamp(0.0, 1.0);
            let (lo, hi) = bucket_bounds(i);
            let value = if i == 0 {
                hi as f64 * frac
            } else if lo == hi {
                lo as f64
            } else {
                lo as f64 * (hi as f64 / lo as f64).powf(frac)
            };
            return Some(value.round() as u64);
        }
        before += n;
    }
    None
}

// --- what a reader gets ------------------------------------------------------

/// Requests over a window: the sum of its cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Window {
    pub status: [u64; CLASSES],
    pub latency: [u64; LAT_BUCKETS],
}

impl Window {
    fn add(&mut self, cell: &Cell) {
        for i in 0..CLASSES {
            self.status[i] += cell.0[i] as u64;
        }
        for i in 0..LAT_BUCKETS {
            self.latency[i] += cell.0[CLASSES + i] as u64;
        }
    }

    pub(crate) fn requests(&self) -> u64 {
        self.status.iter().sum()
    }

    pub(crate) fn server_errors(&self) -> u64 {
        self.status[3]
    }

    /// 5xx share, 0..=1. 0 with no requests.
    pub(crate) fn error_rate(&self) -> f64 {
        let n = self.requests();
        if n == 0 {
            0.0
        } else {
            self.status[3] as f64 / n as f64
        }
    }

    pub(crate) fn p50_us(&self) -> Option<u64> {
        quantile(&self.latency, 0.50)
    }

    pub(crate) fn p95_us(&self) -> Option<u64> {
        quantile(&self.latency, 0.95)
    }

    /// Test and sample helper: `n` requests of `status`, each taking `micros`.
    #[cfg(test)]
    pub(crate) fn with(mut self, status: u16, micros: u64, n: u64) -> Self {
        self.status[status_class(status)] += n;
        self.latency[latency_bucket(micros)] += n;
        self
    }
}

/// The three windows the page offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Range {
    Hour,
    Day,
    Week,
}

impl Range {
    pub(crate) const ALL: [Range; 3] = [Range::Hour, Range::Day, Range::Week];

    pub(crate) fn parse(value: &str) -> Option<Range> {
        match value {
            "1h" => Some(Range::Hour),
            "24h" => Some(Range::Day),
            "7d" => Some(Range::Week),
            _ => None,
        }
    }

    /// The `?range=` value.
    pub(crate) fn param(self) -> &'static str {
        match self {
            Range::Hour => "1h",
            Range::Day => "24h",
            Range::Week => "7d",
        }
    }

    /// `1 h`, as the selector shows it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Range::Hour => "1 h",
            Range::Day => "24 h",
            Range::Week => "7 d",
        }
    }

    /// `the last 24 hours`.
    pub(crate) fn phrase(self) -> &'static str {
        match self {
            Range::Hour => "the last hour",
            Range::Day => "the last 24 hours",
            Range::Week => "the last 7 days",
        }
    }

    /// `the hour before`, for a comparison.
    pub(crate) fn before(self) -> &'static str {
        match self {
            Range::Hour => "the hour before",
            Range::Day => "the 24 h before",
            Range::Week => "the week before",
        }
    }

    /// Seconds per bar of the chart.
    pub(crate) fn bar_secs(self) -> u64 {
        match self {
            Range::Hour => FINE_SECS,
            Range::Day => HOUR_SECS,
            Range::Week => 6 * HOUR_SECS,
        }
    }
}

/// One bar of the chart: the requests that started in `[start, start + secs)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bar {
    pub start: u64,
    pub requests: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteRow {
    pub label: String,
    pub window: Window,
}

/// Everything the usage section shows for one range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsageView {
    pub range: Range,
    pub now: u64,
    pub bars: Vec<Bar>,
    pub current: Window,
    /// The window before, or `None` when nothing was recorded in it.
    pub previous: Option<Window>,
    /// Busiest first.
    pub routes: Vec<RouteRow>,
}

// --- the shared ring ---------------------------------------------------------

/// One application's counts, folded from the shards.
struct Ring {
    names: Vec<String>,
    by_name: HashMap<String, u16>,
    fine: BTreeMap<u64, Vec<Cell>>,
    hours: BTreeMap<u64, Vec<Cell>>,
    totals: BTreeMap<u64, Cell>,
    /// Hourly counts not written to the store yet.
    pending: BTreeMap<u64, Vec<Cell>>,
}

fn add_at(cells: &mut Vec<Cell>, slot: usize, cell: &Cell) {
    if cells.len() <= slot {
        cells.resize(slot + 1, Cell::default());
    }
    cells[slot].merge(cell);
}

impl Ring {
    fn new() -> Ring {
        let names = vec![OTHER_LABEL.to_string(), UNMATCHED_LABEL.to_string()];
        let by_name = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i as u16))
            .collect();
        Ring {
            names,
            by_name,
            fine: BTreeMap::new(),
            hours: BTreeMap::new(),
            totals: BTreeMap::new(),
            pending: BTreeMap::new(),
        }
    }

    /// The slot for `label`, given out first come first served; past
    /// [`MAX_ROUTES`], [`OTHER`].
    fn slot_for(&mut self, label: &str) -> u16 {
        if let Some(slot) = self.by_name.get(label) {
            return *slot;
        }
        if self.names.len() >= MAX_ROUTES {
            return OTHER;
        }
        let slot = self.names.len() as u16;
        self.names.push(label.to_string());
        self.by_name.insert(label.to_string(), slot);
        slot
    }

    /// Fold one shard's cells for five-minute `period`.
    fn add_fine(&mut self, period: u64, cells: &[Cell]) {
        let hour = period * FINE_SECS / HOUR_SECS;
        for (slot, cell) in cells.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            add_at(self.fine.entry(period).or_default(), slot, cell);
            add_at(self.hours.entry(hour).or_default(), slot, cell);
            add_at(self.pending.entry(hour).or_default(), slot, cell);
            self.totals.entry(hour).or_default().merge(cell);
        }
    }

    /// Stored counts for `hour`, read back when the writer starts.
    fn add_stored(&mut self, hour: u64, routes: &[(String, Cell)]) {
        for (label, cell) in routes {
            let slot = self.slot_for(label) as usize;
            add_at(self.hours.entry(hour).or_default(), slot, cell);
            self.totals.entry(hour).or_default().merge(cell);
        }
    }

    fn prune(&mut self, now: u64) {
        let fine_floor = (now / FINE_SECS).saturating_sub(FINE_KEPT - 1);
        let hour = now / HOUR_SECS;
        let route_floor = hour.saturating_sub(ROUTE_HOURS_KEPT - 1);
        let total_floor = hour.saturating_sub(TOTAL_HOURS_KEPT - 1);
        self.fine = self.fine.split_off(&fine_floor);
        self.hours = self.hours.split_off(&route_floor);
        self.totals = self.totals.split_off(&total_floor);
        self.pending = self.pending.split_off(&route_floor);
    }

    fn window_of<'a>(cells: impl Iterator<Item = &'a Cell>) -> Window {
        let mut window = Window::default();
        for cell in cells {
            window.add(cell);
        }
        window
    }

    fn view(&self, range: Range, now: u64) -> UsageView {
        let (map, unit, span): (&BTreeMap<u64, Vec<Cell>>, u64, u64) = match range {
            Range::Hour => (&self.fine, FINE_SECS, 12),
            Range::Day => (&self.hours, HOUR_SECS, 24),
            Range::Week => (&self.hours, HOUR_SECS, 168),
        };
        let last = now / unit;
        let first = (last + 1).saturating_sub(span);
        let prev_first = first.saturating_sub(span);

        // Per route over the window.
        let mut per_route: Vec<Window> = vec![Window::default(); self.names.len()];
        for (_, cells) in map.range(first..=last) {
            for (slot, cell) in cells.iter().enumerate() {
                if let Some(w) = per_route.get_mut(slot) {
                    w.add(cell);
                }
            }
        }
        let mut routes: Vec<RouteRow> = per_route
            .into_iter()
            .enumerate()
            .filter(|(_, w)| w.requests() > 0)
            .map(|(slot, window)| RouteRow {
                label: self.names[slot].clone(),
                window,
            })
            .collect();
        routes.sort_by(|a, b| {
            b.window
                .requests()
                .cmp(&a.window.requests())
                .then_with(|| a.label.cmp(&b.label))
        });

        // Totals: the week reads the longer-kept hourly totals.
        let sum = |from: u64, to: u64| -> Window {
            match range {
                Range::Week => Self::window_of(self.totals.range(from..=to).map(|(_, c)| c)),
                _ => Self::window_of(map.range(from..=to).flat_map(|(_, cells)| cells.iter())),
            }
        };
        let current = sum(first, last);
        let previous = if first == 0 {
            Window::default()
        } else {
            sum(prev_first, first - 1)
        };
        let previous = (previous.requests() > 0).then_some(previous);

        let per_bar = range.bar_secs() / unit;
        let mut bars = Vec::new();
        let mut start = first;
        while start <= last {
            let end = (start + per_bar - 1).min(last);
            let requests = match range {
                Range::Week => self
                    .totals
                    .range(start..=end)
                    .map(|(_, c)| Window::window_requests(c))
                    .sum(),
                _ => map
                    .range(start..=end)
                    .flat_map(|(_, cells)| cells.iter())
                    .map(Window::window_requests)
                    .sum(),
            };
            bars.push(Bar {
                start: start * unit,
                requests,
            });
            start += per_bar;
        }

        UsageView {
            range,
            now,
            bars,
            current,
            previous,
            routes,
        }
    }

    /// Hourly counts not stored yet, by route label; forgotten here.
    fn take_pending(&mut self) -> Vec<HourDelta> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .map(|(hour, cells)| HourDelta {
                hour,
                routes: cells
                    .into_iter()
                    .enumerate()
                    .filter(|(_, c)| !c.is_empty())
                    .filter_map(|(slot, c)| Some((self.names.get(slot)?.clone(), c)))
                    .collect(),
            })
            .filter(|d| !d.routes.is_empty())
            .collect()
    }
}

impl Window {
    fn window_requests(cell: &Cell) -> u64 {
        cell.0[..CLASSES].iter().map(|n| *n as u64).sum()
    }
}

// --- the shards --------------------------------------------------------------

/// One thread's counts for the five-minute period it is in.
struct Shard {
    tenant: TenantId,
    period: u64,
    cells: Vec<Cell>,
    dirty: bool,
}

/// The shards and the rings of every application in the process.
pub(crate) struct Recorder {
    /// Told apart from other recorders by this, not by address: a dropped
    /// recorder's address can be reused, and this thread's shards of it
    /// must not be.
    id: AtomicU64,
    shards: Mutex<Vec<Arc<Mutex<Shard>>>>,
    rings: Mutex<BTreeMap<TenantId, Ring>>,
}

static NEXT_RECORDER: AtomicU64 = AtomicU64::new(1);

/// (recorder id, application, this thread's shard).
type LocalShard = (u64, TenantId, Arc<Mutex<Shard>>);

thread_local! {
    /// This thread's shard of each recorder it wrote to (the process has one;
    /// the tests make their own).
    static LOCAL: RefCell<Vec<LocalShard>> =
        const { RefCell::new(Vec::new()) };
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Recorder {
    pub(crate) const fn new() -> Recorder {
        Recorder {
            id: AtomicU64::new(0),
            shards: Mutex::new(Vec::new()),
            rings: Mutex::new(BTreeMap::new()),
        }
    }

    fn id(&self) -> u64 {
        let id = self.id.load(Ordering::Relaxed);
        if id != 0 {
            return id;
        }
        let fresh = NEXT_RECORDER.fetch_add(1, Ordering::Relaxed);
        match self
            .id
            .compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => fresh,
            Err(won) => won,
        }
    }

    /// The slot for `label` in `tenant`'s table.
    pub(crate) fn slot_for(&self, tenant: TenantId, label: &str) -> u16 {
        lock(&self.rings)
            .entry(tenant)
            .or_insert_with(Ring::new)
            .slot_for(label)
    }

    /// The label of `slot`, if given out.
    pub(crate) fn label_of(&self, tenant: TenantId, slot: u16) -> Option<String> {
        lock(&self.rings)
            .get(&tenant)
            .and_then(|ring| ring.names.get(slot as usize).cloned())
    }

    /// Count one request, at `now` (unix seconds), on this thread's shard.
    pub(crate) fn record_at(
        &self,
        tenant: TenantId,
        slot: u16,
        status: u16,
        micros: u64,
        now: u64,
    ) {
        let key = self.id();
        let period = now / FINE_SECS;
        let slot = (slot as usize).min(MAX_ROUTES - 1);
        LOCAL.with(|local| {
            let mut local = local.borrow_mut();
            let index = match local.iter().position(|(k, t, _)| *k == key && *t == tenant) {
                Some(index) => index,
                None => {
                    let shard = Arc::new(Mutex::new(Shard {
                        tenant,
                        period,
                        cells: vec![Cell::default(); MAX_ROUTES],
                        dirty: false,
                    }));
                    lock(&self.shards).push(shard.clone());
                    local.push((key, tenant, shard));
                    local.len() - 1
                }
            };
            let mut shard = lock(&local[index].2);
            if shard.period != period {
                if shard.dirty {
                    self.fold(&mut shard, now);
                }
                shard.period = period;
            }
            shard.cells[slot].add(status_class(status), latency_bucket(micros));
            shard.dirty = true;
        });
    }

    /// Move a shard's counts into its ring. Lock order: shard, then rings.
    fn fold(&self, shard: &mut Shard, now: u64) {
        resync_clock();
        let mut rings = lock(&self.rings);
        let ring = rings.entry(shard.tenant).or_insert_with(Ring::new);
        ring.add_fine(shard.period, &shard.cells);
        ring.prune(now);
        drop(rings);
        shard.cells.fill(Cell::default());
        shard.dirty = false;
    }

    /// Fold every shard of `tenant` (all of them when `None`).
    fn drain(&self, tenant: Option<TenantId>, now: u64) {
        let shards: Vec<Arc<Mutex<Shard>>> = lock(&self.shards).clone();
        for shard in shards {
            let mut shard = lock(&shard);
            if shard.dirty && tenant.is_none_or(|t| t == shard.tenant) {
                self.fold(&mut shard, now);
            }
        }
    }

    /// The usage section for `tenant` over `range`, as of `now`.
    pub(crate) fn snapshot(&self, tenant: TenantId, range: Range, now: u64) -> UsageView {
        self.drain(Some(tenant), now);
        let mut rings = lock(&self.rings);
        let ring = rings.entry(tenant).or_insert_with(Ring::new);
        ring.prune(now);
        ring.view(range, now)
    }

    /// Everything not stored yet, per application.
    fn take_pending(&self, now: u64) -> Vec<(TenantId, Vec<HourDelta>)> {
        self.drain(None, now);
        lock(&self.rings)
            .iter_mut()
            .map(|(tenant, ring)| (*tenant, ring.take_pending()))
            .filter(|(_, deltas)| !deltas.is_empty())
            .collect()
    }

    fn add_stored(&self, tenant: TenantId, hours: &[HourDelta], now: u64) {
        let mut rings = lock(&self.rings);
        let ring = rings.entry(tenant).or_insert_with(Ring::new);
        for delta in hours {
            ring.add_stored(delta.hour, &delta.routes);
        }
        ring.prune(now);
    }
}

// --- the process's recorder and the request path -----------------------------

static RECORDER: Recorder = Recorder::new();

const UNKNOWN: u8 = 0;
const ON: u8 = 1;
const OFF: u8 = 2;
static SWITCH: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Whether usage is recorded at all (`SOLI_USAGE`).
pub(crate) fn enabled() -> bool {
    match SWITCH.load(Ordering::Relaxed) {
        ON => true,
        OFF => false,
        _ => {
            let on = super::error_tracker::env_switch("SOLI_USAGE");
            SWITCH.store(if on { ON } else { OFF }, Ordering::Relaxed);
            on
        }
    }
}

/// Seconds since the epoch.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The monotonic instant the request clock counts from.
static CLOCK_EPOCH: OnceLock<Instant> = OnceLock::new();
/// Unix milliseconds at [`CLOCK_EPOCH`]; 0 until first synced.
static CLOCK_OFFSET_MS: AtomicU64 = AtomicU64::new(0);

/// Re-read the wall clock against the monotonic one. Done when a shard folds
/// (every five minutes per worker) and by the collector, so a step of the
/// system clock is followed within minutes while a request reads no wall
/// clock at all.
fn resync_clock() -> u64 {
    let epoch = *CLOCK_EPOCH.get_or_init(Instant::now);
    let wall_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mono_ms = Instant::now().saturating_duration_since(epoch).as_millis() as u64;
    let offset = wall_ms.saturating_sub(mono_ms).max(1);
    CLOCK_OFFSET_MS.store(offset, Ordering::Relaxed);
    offset
}

/// Unix seconds at `at`, from the monotonic clock and the last sync.
pub(crate) fn unix_at(at: Instant) -> u64 {
    let epoch = *CLOCK_EPOCH.get_or_init(Instant::now);
    let offset = match CLOCK_OFFSET_MS.load(Ordering::Relaxed) {
        0 => resync_clock(),
        offset => offset,
    };
    (offset + at.saturating_duration_since(epoch).as_millis() as u64) / 1000
}

/// The route cache of one worker thread: route position → slot.
struct RouteCache {
    generation: u64,
    tenant: TenantId,
    slots: Vec<u16>,
}

const NO_SLOT: u16 = u16::MAX;

crate::request_local! {
    /// The slot of the route the current request matched.
    static CURRENT: StdCell<u16> = const { StdCell::new(UNMATCHED) };
}

thread_local! {
    /// Per worker, like the route table it caches positions of.
    static ROUTE_CACHE: RefCell<RouteCache> = const {
        RefCell::new(RouteCache { generation: u64::MAX, tenant: TenantId::PRIMARY, slots: Vec::new() })
    };
}

/// A request starts on this worker: the clock it is timed by, or `None` when
/// usage is off.
#[inline]
pub(crate) fn begin_request() -> Option<Instant> {
    if !enabled() {
        return None;
    }
    CURRENT.with(|c| c.set(UNMATCHED));
    Some(Instant::now())
}

/// The route at `index` (as `find_route` returned it) answered this request.
pub(crate) fn note_route(index: usize) {
    if !enabled() {
        return;
    }
    let generation = crate::interpreter::builtins::server::routes_generation();
    let tenant = super::tenant::current_id();
    let slot = ROUTE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.generation != generation || cache.tenant != tenant {
            cache.generation = generation;
            cache.tenant = tenant;
            cache.slots.clear();
        }
        if let Some(slot) = cache.slots.get(index).copied().filter(|s| *s != NO_SLOT) {
            return slot;
        }
        let slot = match crate::interpreter::builtins::server::route_label(index) {
            Some(label) => RECORDER.slot_for(tenant, &label),
            None => OTHER,
        };
        if cache.slots.len() <= index {
            cache.slots.resize(index + 1, NO_SLOT);
        }
        cache.slots[index] = slot;
        slot
    });
    CURRENT.with(|c| c.set(slot));
}

/// The request begun by [`begin_request`] answered `status`.
pub(crate) fn finish_request(started: Option<Instant>, path: &str, status: u16) {
    let Some(started) = started else {
        return;
    };
    let slot = CURRENT.with(|c| c.replace(UNMATCHED));
    if excluded(path, slot) {
        return;
    }
    let ended = Instant::now();
    let micros = ended
        .saturating_duration_since(started)
        .as_micros()
        .min(u64::MAX as u128) as u64;
    RECORDER.record_at(
        super::tenant::current_id(),
        slot,
        status,
        micros,
        unix_at(ended),
    );
    ensure_collector();
}

/// `GET /posts/:id` for the route this worker is serving, if one matched.
pub(crate) fn current_route_label() -> Option<String> {
    let slot = CURRENT.with(|c| c.get());
    if slot == UNMATCHED {
        return None;
    }
    RECORDER.label_of(super::tenant::current_id(), slot)
}

/// Requests left out of the stats: the framework's own endpoints, and a miss
/// for what looks like a static file (`/favicon.ico`, a stale asset URL).
pub(crate) fn excluded(path: &str, slot: u16) -> bool {
    if path.starts_with("/__") || matches!(path, "/_health" | "/_ready" | "/_metrics" | "/up") {
        return true;
    }
    slot == UNMATCHED && looks_static(path)
}

fn looks_static(path: &str) -> bool {
    let last = path.rsplit('/').next().unwrap_or("");
    let Some((_, ext)) = last.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "js" | "mjs"
            | "css"
            | "map"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "svg"
            | "ico"
            | "webp"
            | "avif"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "txt"
            | "xml"
            | "webmanifest"
    )
}

/// The usage section for the current application.
pub(crate) fn snapshot(range: Range) -> UsageView {
    RECORDER.snapshot(super::tenant::current_id(), range, unix_now())
}

/// The usage writer's counters for the current application.
pub(crate) fn stats() -> StatsSnapshot {
    WRITERS.stats(super::tenant::current_id())
}

// --- persistence ---------------------------------------------------------------

/// One hour's new counts, by route label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HourDelta {
    pub hour: u64,
    pub routes: Vec<(String, Cell)>,
}

struct Batch {
    /// Read the stored hours back first (the writer's first batch).
    load: bool,
    hours: Vec<HourDelta>,
}

static COLLECTOR: AtomicU8 = AtomicU8::new(UNKNOWN);
static COLLECTOR_START: Mutex<()> = Mutex::new(());
static WRITERS: Writers<Batch> = Writers::new("usage", QUEUE_CAP);

#[inline]
fn ensure_collector() {
    if COLLECTOR.load(Ordering::Relaxed) == UNKNOWN {
        start_collector();
    }
}

#[cold]
fn start_collector() {
    // The collector needs the server's runtime; a thread without one is not
    // serving, and asks again next time.
    if super::get_tokio_handle().is_none() {
        return;
    }
    let _guard = lock(&COLLECTOR_START);
    if COLLECTOR.load(Ordering::Relaxed) != UNKNOWN {
        return;
    }
    let tenant = super::tenant::current_id();
    if tenant_writer::spawn_thread("usage-collector", tenant, run_collector) {
        COLLECTOR.store(ON, Ordering::Relaxed);
    }
}

fn run_collector() {
    let mut loaded: HashSet<TenantId> = HashSet::new();
    loop {
        std::thread::sleep(COLLECT_EVERY);
        resync_clock();
        for (tenant, hours) in RECORDER.take_pending(unix_now()) {
            let load = loaded.insert(tenant);
            WRITERS.deliver(tenant, Batch { load, hours }, &spawn_writer);
        }
    }
}

fn spawn_writer(tenant: TenantId, receiver: Receiver<Batch>, stats: Arc<Stats>) -> bool {
    tenant_writer::spawn_thread("usage", tenant, move || {
        run_writer(tenant, receiver, &stats)
    })
}

fn run_writer(tenant: TenantId, receiver: Receiver<Batch>, stats: &Stats) {
    let mut ensured = false;
    let mut since_prune = u32::MAX;
    while let Ok(batch) = receiver.recv() {
        if !ensured {
            ensured = match tenant_writer::fenced(|| internal_store::ensure(USAGE_COLLECTION, "at"))
            {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("[usage] could not prepare {USAGE_COLLECTION}: {e}");
                    stats
                        .failed
                        .fetch_add(batch.hours.len() as u64, Ordering::Relaxed);
                    continue;
                }
            };
        }
        if batch.load {
            match tenant_writer::fenced(load_stored) {
                Ok(stored) => RECORDER.add_stored(tenant, &stored, unix_now()),
                Err(e) => eprintln!("[usage] could not read {USAGE_COLLECTION} back: {e}"),
            }
        }
        for delta in &batch.hours {
            if let Err(e) = tenant_writer::fenced(|| write_delta(delta)) {
                eprintln!("[usage] could not store an hour: {e}");
                stats.failed.fetch_add(1, Ordering::Relaxed);
            }
        }
        since_prune = since_prune.saturating_add(1);
        if since_prune >= 60 {
            since_prune = 0;
            if let Err(e) = tenant_writer::fenced(prune_stored) {
                eprintln!("[usage] could not prune {USAGE_COLLECTION}: {e}");
            }
        }
    }
}

/// `2026110614`: the document key of an hour.
pub(crate) fn hour_key(hour: u64) -> String {
    chrono::DateTime::from_timestamp((hour * HOUR_SECS) as i64, 0)
        .map(|t| t.format("%Y%m%d%H").to_string())
        .unwrap_or_else(|| hour.to_string())
}

/// The stored document for `delta` added to `existing`.
pub(crate) fn merged_doc(
    existing: Option<&serde_json::Value>,
    delta: &HourDelta,
) -> serde_json::Value {
    let mut routes: Vec<(String, Cell)> = existing.map(parse_routes).unwrap_or_default();
    for (label, cell) in &delta.routes {
        match routes.iter().position(|(l, _)| l == label) {
            Some(i) => routes[i].1.merge(cell),
            None if routes.len() < MAX_ROUTES * 2 => routes.push((label.clone(), *cell)),
            None => {}
        }
    }
    serde_json::json!({
        "at": delta.hour * HOUR_SECS,
        "routes": routes
            .into_iter()
            .map(|(label, cell)| serde_json::json!([label, cell.to_vec()]))
            .collect::<Vec<_>>(),
    })
}

fn parse_routes(doc: &serde_json::Value) -> Vec<(String, Cell)> {
    doc.get("routes")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .filter_map(|pair| {
            let label = pair.get(0)?.as_str()?.to_string();
            let values: Vec<u64> = pair
                .get(1)?
                .as_array()?
                .iter()
                .map(|v| v.as_u64().unwrap_or(0))
                .collect();
            Some((label, Cell::from_slice(&values)?))
        })
        .collect()
}

/// A stored document back as an hour of counts.
pub(crate) fn parse_doc(doc: &serde_json::Value) -> Option<HourDelta> {
    let at = doc.get("at")?.as_u64()?;
    Some(HourDelta {
        hour: at / HOUR_SECS,
        routes: parse_routes(doc),
    })
}

fn write_delta(delta: &HourDelta) -> Result<(), String> {
    let key = hour_key(delta.hour);
    match internal_store::get(USAGE_COLLECTION, &key)? {
        Some(existing) => {
            internal_store::patch(USAGE_COLLECTION, &key, merged_doc(Some(&existing), delta))
        }
        None => internal_store::insert(USAGE_COLLECTION, &key, merged_doc(None, delta)),
    }
}

fn load_stored() -> Result<Vec<HourDelta>, String> {
    let floor = (unix_now() / HOUR_SECS).saturating_sub(TOTAL_HOURS_KEPT - 1);
    let rows = match internal_store::list(
        USAGE_COLLECTION,
        None,
        "at",
        true,
        TOTAL_HOURS_KEPT as usize,
        &["at", "routes"],
    ) {
        Ok(rows) => rows,
        Err(e) if internal_store::is_missing_collection(&e) => Vec::new(),
        Err(e) => return Err(e),
    };
    Ok(rows
        .iter()
        .filter_map(parse_doc)
        .filter(|d| d.hour >= floor)
        .collect())
}

fn prune_stored() -> Result<(), String> {
    let cutoff = unix_now().saturating_sub(STORED_DAYS * 24 * HOUR_SECS);
    let oldest = internal_store::list(USAGE_COLLECTION, None, "at", false, 50, &["at"])?;
    for row in oldest {
        let at = row.get("at").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
        if at >= cutoff {
            break;
        }
        if let Some(key) = row.get("_key").and_then(|k| k.as_str()) {
            internal_store::delete(USAGE_COLLECTION, key)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: TenantId = TenantId(0x05A6_0001);
    /// 2025-11-06 14:00:00 UTC, on an hour boundary.
    const NOON: u64 = 1_762_437_600;

    fn hist(pairs: &[(u64, u64)]) -> [u64; LAT_BUCKETS] {
        let mut h = [0u64; LAT_BUCKETS];
        for (micros, n) in pairs {
            h[latency_bucket(*micros)] += n;
        }
        h
    }

    #[test]
    fn the_request_clock_follows_the_wall_clock() {
        let now = Instant::now();
        let wall = unix_now();
        let derived = unix_at(now);
        assert!(derived.abs_diff(wall) <= 1, "{derived} vs {wall}");
        assert_eq!(unix_at(now + Duration::from_secs(3_600)), derived + 3_600);
    }

    #[test]
    fn status_classes() {
        assert_eq!(status_class(101), 0);
        assert_eq!(status_class(200), 0);
        assert_eq!(status_class(204), 0);
        assert_eq!(status_class(301), 1);
        assert_eq!(status_class(404), 2);
        assert_eq!(status_class(500), 3);
        assert_eq!(status_class(503), 3);
        assert_eq!(status_class(999), 3);
    }

    #[test]
    fn latency_buckets_double() {
        assert_eq!(latency_bucket(0), 0);
        assert_eq!(latency_bucket(249), 0);
        assert_eq!(latency_bucket(250), 1);
        assert_eq!(latency_bucket(499), 1);
        assert_eq!(latency_bucket(500), 2);
        assert_eq!(latency_bucket(1_000), 3);
        assert_eq!(latency_bucket(u64::MAX), LAT_BUCKETS - 1);
        for i in 1..LAT_BUCKETS - 1 {
            let (lo, hi) = bucket_bounds(i);
            assert_eq!(latency_bucket(lo), i);
            assert_eq!(latency_bucket(hi - 1), i);
        }
    }

    #[test]
    fn percentiles_fall_in_the_right_bucket() {
        assert_eq!(quantile(&[0; LAT_BUCKETS], 0.95), None);
        // 95 fast requests and 5 slow ones: p50 is fast, p95 at the edge.
        let h = hist(&[(3_000, 95), (800_000, 5)]);
        let p50 = quantile(&h, 0.5).unwrap_or(0);
        assert!((2_000..4_000).contains(&p50), "p50 {p50}");
        let p99 = quantile(&h, 0.99).unwrap_or(0);
        assert!((512_000..1_024_000).contains(&p99), "p99 {p99}");
        // All in one bucket: every quantile stays inside it, rising.
        let one = hist(&[(10_000, 100)]);
        let (lo, hi) = bucket_bounds(latency_bucket(10_000));
        let a = quantile(&one, 0.1).unwrap_or(0);
        let b = quantile(&one, 0.9).unwrap_or(0);
        assert!(lo <= a && a < b && b <= hi, "{lo} {a} {b} {hi}");
        // The open-ended last bucket answers its floor.
        let huge = hist(&[(100_000_000, 3)]);
        assert_eq!(
            quantile(&huge, 0.95),
            Some(bucket_bounds(LAT_BUCKETS - 1).0)
        );
    }

    #[test]
    fn requests_are_bucketed_by_route_and_status() {
        let r = Recorder::new();
        let a = r.slot_for(T, "GET /posts/:id");
        let b = r.slot_for(T, "POST /posts");
        assert_eq!(r.slot_for(T, "GET /posts/:id"), a, "a label keeps its slot");
        r.record_at(T, a, 200, 1_000, NOON + 10);
        r.record_at(T, a, 200, 1_000, NOON + 20);
        r.record_at(T, a, 503, 2_000, NOON + 30);
        r.record_at(T, b, 302, 500, NOON + 40);
        r.record_at(T, UNMATCHED, 404, 300, NOON + 50);
        let view = r.snapshot(T, Range::Hour, NOON + 60);
        assert_eq!(view.current.requests(), 5);
        assert_eq!(view.current.status, [2, 1, 1, 1]);
        assert_eq!(view.routes[0].label, "GET /posts/:id");
        assert_eq!(view.routes[0].window.requests(), 3);
        assert_eq!(view.routes[0].window.server_errors(), 1);
        let labels: Vec<&str> = view.routes.iter().map(|r| r.label.as_str()).collect();
        assert!(labels.contains(&UNMATCHED_LABEL));
        assert!(labels.contains(&"POST /posts"));
        assert_eq!(view.previous, None, "nothing before");
    }

    #[test]
    fn routes_past_the_cap_fold_into_other() {
        let r = Recorder::new();
        for i in 0..MAX_ROUTES + 10 {
            let slot = r.slot_for(T, &format!("GET /r{i}"));
            r.record_at(T, slot, 200, 100, NOON);
        }
        let view = r.snapshot(T, Range::Hour, NOON);
        assert_eq!(view.current.requests(), (MAX_ROUTES + 10) as u64);
        assert!(view.routes.len() <= MAX_ROUTES);
        let other = view
            .routes
            .iter()
            .find(|row| row.label == OTHER_LABEL)
            .map(|row| row.window.requests());
        // Two slots are reserved, so 62 named routes fit and 12 overflow.
        assert_eq!(other, Some((MAX_ROUTES + 10 - (MAX_ROUTES - 2)) as u64));
    }

    #[test]
    fn threads_add_up() {
        let r = Arc::new(Recorder::new());
        let slot = r.slot_for(T, "GET /");
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let r = r.clone();
                std::thread::spawn(move || {
                    for i in 0..1_000u64 {
                        r.record_at(
                            T,
                            slot,
                            if i % 10 == 0 { 500 } else { 200 },
                            2_000,
                            NOON + 5,
                        );
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread");
        }
        let view = r.snapshot(T, Range::Day, NOON + 6);
        assert_eq!(view.current.requests(), 8_000);
        assert_eq!(view.current.server_errors(), 800);
        // The shards were folded, so a second read sees the same.
        assert_eq!(
            r.snapshot(T, Range::Day, NOON + 7).current.requests(),
            8_000
        );
    }

    #[test]
    fn hours_roll_over_and_compare_with_the_window_before() {
        let r = Recorder::new();
        let slot = r.slot_for(T, "GET /");
        // An hour ago: 10 requests; now: 30 in the current hour.
        for _ in 0..10 {
            r.record_at(T, slot, 200, 1_000, NOON - 1_800);
        }
        for _ in 0..30 {
            r.record_at(T, slot, 200, 1_000, NOON + 60);
        }
        let day = r.snapshot(T, Range::Day, NOON + 120);
        assert_eq!(day.bars.len(), 24);
        assert_eq!(
            day.bars[23],
            Bar {
                start: NOON,
                requests: 30
            }
        );
        assert_eq!(
            day.bars[22],
            Bar {
                start: NOON - 3_600,
                requests: 10
            }
        );
        assert_eq!(day.current.requests(), 40);
        // The 1 h view is the last twelve five-minute periods.
        let hour = r.snapshot(T, Range::Hour, NOON + 120);
        assert_eq!(hour.bars.len(), 12);
        assert_eq!(hour.current.requests(), 40);
        // An hour later all 40 are the hour before.
        let later = r.snapshot(T, Range::Hour, NOON + 3_600 + 120);
        assert_eq!(later.current.requests(), 0);
        assert_eq!(later.previous.map(|w| w.requests()), Some(40));
        // A day later, the 24 h view compares with the day before.
        let tomorrow = r.snapshot(T, Range::Day, NOON + 24 * 3_600);
        assert_eq!(tomorrow.current.requests(), 0);
        assert_eq!(tomorrow.previous.map(|w| w.requests()), Some(40));
        let week = r.snapshot(T, Range::Week, NOON + 3 * 24 * 3_600);
        assert_eq!(week.bars.len(), 28);
        assert_eq!(week.current.requests(), 40);
    }

    #[test]
    fn applications_are_counted_apart() {
        let r = Recorder::new();
        let other = TenantId(0x05A6_0002);
        let a = r.slot_for(T, "GET /");
        let b = r.slot_for(other, "GET /");
        r.record_at(T, a, 200, 100, NOON);
        r.record_at(other, b, 200, 100, NOON);
        r.record_at(other, b, 200, 100, NOON);
        assert_eq!(r.snapshot(T, Range::Hour, NOON).current.requests(), 1);
        assert_eq!(r.snapshot(other, Range::Hour, NOON).current.requests(), 2);
    }

    #[test]
    fn framework_endpoints_and_asset_misses_are_left_out() {
        for path in [
            "/__soli/",
            "/__soli/errors",
            "/__solidev/request/1",
            "/_health",
            "/_ready",
            "/_metrics",
            "/up",
        ] {
            assert!(excluded(path, 5), "{path}");
        }
        assert!(excluded("/favicon.ico", UNMATCHED));
        assert!(excluded("/assets/app-3f2a.js", UNMATCHED));
        // A route that answers with an extension is the app's.
        assert!(!excluded("/sitemap.xml", 7));
        assert!(!excluded("/posts/42", UNMATCHED));
        assert!(!excluded("/", 2));
        assert!(!excluded("/_internal", 2));
    }

    #[test]
    fn stored_hours_merge_and_read_back() {
        let mut cell = Cell::default();
        cell.add(0, 3);
        cell.add(3, 5);
        let delta = HourDelta {
            hour: NOON / HOUR_SECS,
            routes: vec![("GET /".to_string(), cell)],
        };
        let first = merged_doc(None, &delta);
        let second = merged_doc(Some(&first), &delta);
        let back = parse_doc(&second).expect("parses");
        assert_eq!(back.hour, NOON / HOUR_SECS);
        assert_eq!(back.routes.len(), 1);
        assert_eq!(back.routes[0].1 .0[0], 2, "counts add up");
        assert_eq!(back.routes[0].1 .0[3], 2);
        assert_eq!(hour_key(NOON / HOUR_SECS), "2025110614");
        // A malformed cell is skipped, not trusted.
        let bad = serde_json::json!({"at": NOON, "routes": [["GET /", [1, 2]], "junk"]});
        assert_eq!(parse_doc(&bad).map(|d| d.routes.len()), Some(0));

        // Stored hours show in the week, and are not stored again.
        let r = Recorder::new();
        r.add_stored(T, &[back], NOON + 60);
        let week = r.snapshot(T, Range::Week, NOON + 60);
        assert_eq!(week.current.requests(), 4);
        assert!(r.take_pending(NOON + 60).is_empty());
    }

    #[test]
    fn new_counts_are_pending_until_taken() {
        let r = Recorder::new();
        let slot = r.slot_for(T, "GET /");
        r.record_at(T, slot, 200, 100, NOON);
        let pending = r.take_pending(NOON + 1);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1[0].routes[0].0, "GET /");
        assert!(r.take_pending(NOON + 2).is_empty(), "taken once");
        // Still in memory for the page.
        assert_eq!(r.snapshot(T, Range::Hour, NOON + 3).current.requests(), 1);
    }

    /// The hot path's cost, printed for the report: one thread per core
    /// recording at once. Not an assertion on speed (CI machines vary), only
    /// that it stays far from anything a request would notice.
    #[test]
    fn recording_costs_tens_of_nanoseconds() {
        let r = Arc::new(Recorder::new());
        let slot = r.slot_for(T, "GET /");
        const N: u64 = 200_000;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(8);
        let started = Instant::now();
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let r = r.clone();
                std::thread::spawn(move || {
                    let t0 = Instant::now();
                    for i in 0..N {
                        r.record_at(T, slot, 200, i % 50_000, NOON + 1);
                    }
                    t0.elapsed().as_nanos() as f64 / N as f64
                })
            })
            .collect();
        let per_op: Vec<f64> = handles
            .into_iter()
            .map(|h| h.join().unwrap_or(0.0))
            .collect();
        let worst = per_op.iter().cloned().fold(0.0, f64::max);
        eprintln!(
            "[usage bench] {threads} threads x {N} records in {:?}: worst {:.1} ns/record per thread",
            started.elapsed(),
            worst
        );
        assert_eq!(
            r.snapshot(T, Range::Hour, NOON + 2).current.requests(),
            threads as u64 * N
        );
        assert!(worst < 5_000.0, "{worst} ns per record");

        // What a request pays in all: the start clock, the elapsed read, the
        // period's clock, and the record — what `finish_request` does.
        let full = Arc::new(Recorder::new());
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let r = full.clone();
                std::thread::spawn(move || {
                    let t0 = Instant::now();
                    for _ in 0..N {
                        let started = Instant::now();
                        let ended = Instant::now();
                        let micros = ended.saturating_duration_since(started).as_micros() as u64;
                        r.record_at(T, slot, 200, micros, unix_at(ended));
                    }
                    t0.elapsed().as_nanos() as f64 / N as f64
                })
            })
            .collect();
        let per_request: Vec<f64> = handles
            .into_iter()
            .map(|h| h.join().unwrap_or(0.0))
            .collect();
        let worst_full = per_request.iter().cloned().fold(0.0, f64::max);
        eprintln!(
            "[usage bench] {threads} threads, clocks included: worst {worst_full:.1} ns/request per thread"
        );
        assert!(worst_full < 10_000.0, "{worst_full} ns per request");
    }
}
