//! The storage behind every metric: lock-free histograms and the labelled
//! families that app metrics (`Metrics.increment(...)`) and the per-route
//! request histogram live in.
//!
//! Workers are threads of one process, so a family is shared by all of them:
//! the maps are behind `RwLock`s that are only written when a *new* series
//! appears, and every sample after that is a read lock plus one atomic.
//!
//! Memory is bounded twice: at most [`MAX_FAMILIES`] app metric names, and at
//! most a cap of label sets per family. A sample that would open a series past
//! either cap is dropped, counted in `soli_metrics_dropped_total`, and warned
//! about once per family on stderr — a tag carrying a user id must not be able
//! to grow a worker without bound.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Default histogram buckets, in seconds: 1 ms to 10 s.
pub const DEFAULT_BUCKETS: &[f64] = &[
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// App metric names a process may hold.
pub const MAX_FAMILIES: usize = 1000;

/// Label sets one app metric may hold.
pub const MAX_SERIES_PER_FAMILY: usize = 1000;

/// Tags one sample may carry.
pub const MAX_TAGS: usize = 20;

const MAX_NAME_LEN: usize = 200;
const MAX_TAG_KEY_LEN: usize = 100;
const MAX_TAG_VALUE_LEN: usize = 200;

/// Add `v` to an `f64` kept as bits in an `AtomicU64`.
fn add_f64(cell: &AtomicU64, v: f64) {
    let mut current = cell.load(Ordering::Relaxed);
    loop {
        let next = (f64::from_bits(current) + v).to_bits();
        match cell.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(seen) => current = seen,
        }
    }
}

// ---------- histogram ----------

/// A fixed-bucket histogram updated with atomics only.
///
/// `counts[i]` holds the observations that fell in bucket `i` alone (not
/// cumulative), the last slot being `+Inf`; the cumulative form is built when
/// the histogram is read. The total count is the sum of the buckets, so the
/// `+Inf` bucket and `_count` always agree, however the reads interleave with
/// writes.
pub struct Histogram {
    bounds: Arc<[f64]>,
    counts: Box<[AtomicU64]>,
    sum_bits: AtomicU64,
}

/// A histogram as read at one instant.
#[derive(Clone, Debug, PartialEq)]
pub struct HistogramSnapshot {
    pub bounds: Arc<[f64]>,
    /// Per-bucket counts (not cumulative), `bounds.len() + 1` of them.
    pub counts: Vec<u64>,
    pub sum: f64,
}

impl HistogramSnapshot {
    pub fn count(&self) -> u64 {
        self.counts.iter().sum()
    }

    /// The cumulative counts Prometheus' `_bucket{le=…}` series carry,
    /// `+Inf` last.
    pub fn cumulative(&self) -> Vec<u64> {
        let mut total = 0;
        self.counts
            .iter()
            .map(|c| {
                total += c;
                total
            })
            .collect()
    }
}

impl Histogram {
    pub fn new(bounds: Arc<[f64]>) -> Self {
        let counts = (0..=bounds.len()).map(|_| AtomicU64::new(0)).collect();
        Self {
            bounds,
            counts,
            sum_bits: AtomicU64::new(0f64.to_bits()),
        }
    }

    /// Record one observation. Bounds are inclusive upper limits
    /// (Prometheus' `le`): `0.005` lands in the `le="0.005"` bucket.
    pub fn observe(&self, v: f64) {
        let idx = self.bounds.partition_point(|b| *b < v);
        self.counts[idx].fetch_add(1, Ordering::Relaxed);
        add_f64(&self.sum_bits, v);
    }

    pub fn count(&self) -> u64 {
        self.counts.iter().map(|c| c.load(Ordering::Relaxed)).sum()
    }

    pub fn snapshot(&self) -> HistogramSnapshot {
        HistogramSnapshot {
            bounds: self.bounds.clone(),
            counts: self
                .counts
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .collect(),
            sum: f64::from_bits(self.sum_bits.load(Ordering::Relaxed)),
        }
    }

    fn reset(&self) {
        for c in self.counts.iter() {
            c.store(0, Ordering::Relaxed);
        }
        self.sum_bits.store(0f64.to_bits(), Ordering::Relaxed);
    }
}

// ---------- kinds, names, tags ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Counter => "counter",
            Kind::Gauge => "gauge",
            Kind::Histogram => "histogram",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "counter" => Some(Kind::Counter),
            "gauge" => Some(Kind::Gauge),
            "histogram" => Some(Kind::Histogram),
            _ => None,
        }
    }
}

/// Sorted `(key, value)` pairs, keys already in their Prometheus form.
pub type Labels = Vec<(String, String)>;

/// `[a-zA-Z_][a-zA-Z0-9_.]*`, at most 200 characters, and not in the `soli`
/// namespace the built-in series use.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(format!(
            "metric name must be 1 to {MAX_NAME_LEN} characters, got {}",
            name.len()
        ));
    }
    if !is_identifier(name, true) {
        return Err(format!(
            "invalid metric name \"{name}\": use letters, digits, '_' and '.', not starting with a digit or '.'"
        ));
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("soli.") || lower.starts_with("soli_") {
        return Err(format!(
            "metric name \"{name}\" is reserved: the soli. / soli_ prefix belongs to the built-in series"
        ));
    }
    Ok(())
}

/// The name Prometheus is shown: dots become underscores.
pub fn prometheus_name(name: &str) -> String {
    name.replace('.', "_")
}

fn is_identifier(s: &str, allow_dot: bool) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || (allow_dot && c == '.'))
}

/// Validate a sample's tags and turn them into a series key: keys in their
/// Prometheus form (dots to underscores), sorted, unique.
pub fn normalize_tags(tags: &[(String, String)]) -> Result<Labels, String> {
    if tags.len() > MAX_TAGS {
        return Err(format!(
            "a metric sample takes at most {MAX_TAGS} tags, got {}",
            tags.len()
        ));
    }
    let mut labels: Labels = Vec::with_capacity(tags.len());
    for (key, value) in tags {
        if key.is_empty() || key.len() > MAX_TAG_KEY_LEN || !is_identifier(key, true) {
            return Err(format!(
                "invalid tag key \"{key}\": use letters, digits, '_' and '.' (at most {MAX_TAG_KEY_LEN} characters), not starting with a digit"
            ));
        }
        let prom = prometheus_name(key);
        if prom.starts_with("__") || prom == "le" {
            return Err(format!(
                "tag key \"{key}\" is reserved (\"le\" and names starting with \"__\")"
            ));
        }
        if value.chars().count() > MAX_TAG_VALUE_LEN {
            return Err(format!(
                "tag \"{key}\" value is longer than {MAX_TAG_VALUE_LEN} characters"
            ));
        }
        labels.push((prom, value.clone()));
    }
    labels.sort();
    if labels.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err("two tags map to the same key (a '.' and a '_' spelling?)".to_string());
    }
    Ok(labels)
}

// ---------- families ----------

enum Series {
    /// An `f64` as bits: counters can be incremented by fractions.
    Counter(AtomicU64),
    Gauge(AtomicU64),
    Histogram(Histogram),
}

impl Series {
    fn new(kind: Kind, bounds: &Arc<[f64]>) -> Self {
        match kind {
            Kind::Counter => Series::Counter(AtomicU64::new(0f64.to_bits())),
            Kind::Gauge => Series::Gauge(AtomicU64::new(0f64.to_bits())),
            Kind::Histogram => Series::Histogram(Histogram::new(bounds.clone())),
        }
    }

    fn apply(&self, value: f64) {
        match self {
            Series::Counter(cell) => add_f64(cell, value),
            Series::Gauge(cell) => cell.store(value.to_bits(), Ordering::Relaxed),
            Series::Histogram(h) => h.observe(value),
        }
    }

    fn read(&self) -> SeriesValue {
        match self {
            Series::Counter(cell) | Series::Gauge(cell) => {
                SeriesValue::Number(f64::from_bits(cell.load(Ordering::Relaxed)))
            }
            Series::Histogram(h) => SeriesValue::Histogram(h.snapshot()),
        }
    }
}

/// One series' value as read.
#[derive(Clone, Debug, PartialEq)]
pub enum SeriesValue {
    Number(f64),
    Histogram(HistogramSnapshot),
}

/// One metric name and its series.
pub struct Family {
    pub kind: Kind,
    /// The name as first recorded (dots kept): what OTLP is sent.
    pub name: String,
    /// The Prometheus spelling (dots to underscores).
    pub prom_name: String,
    max_series: usize,
    bounds: Arc<[f64]>,
    series: RwLock<HashMap<Labels, Arc<Series>>>,
    warned: AtomicBool,
}

/// What became of one sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Recorded,
    /// Refused by a cardinality cap; nothing was stored.
    Dropped,
}

impl Family {
    pub fn new(kind: Kind, name: &str, bounds: Arc<[f64]>, max_series: usize) -> Self {
        Self {
            kind,
            name: name.to_string(),
            prom_name: prometheus_name(name),
            max_series,
            bounds,
            series: RwLock::new(HashMap::new()),
            warned: AtomicBool::new(false),
        }
    }

    /// Apply `value` to the series `labels` names, opening it if the cap
    /// allows.
    pub fn record(&self, labels: &Labels, value: f64) -> Outcome {
        {
            let map = self.series.read().unwrap_or_else(|e| e.into_inner());
            if let Some(series) = map.get(labels) {
                series.apply(value);
                return Outcome::Recorded;
            }
        }
        let mut map = self.series.write().unwrap_or_else(|e| e.into_inner());
        if let Some(series) = map.get(labels) {
            series.apply(value);
            return Outcome::Recorded;
        }
        if map.len() >= self.max_series {
            drop(map);
            if !self.warned.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[WARN] metric \"{}\" reached {} tag combinations; new combinations are dropped (check for a high-cardinality tag such as an id)",
                    self.name, self.max_series
                );
            }
            return Outcome::Dropped;
        }
        let series = Arc::new(Series::new(self.kind, &self.bounds));
        series.apply(value);
        map.insert(labels.clone(), series);
        Outcome::Recorded
    }

    pub fn value(&self, labels: &Labels) -> Option<SeriesValue> {
        let map = self.series.read().unwrap_or_else(|e| e.into_inner());
        map.get(labels).map(|s| s.read())
    }

    pub fn series_count(&self) -> usize {
        self.series.read().map(|m| m.len()).unwrap_or(0)
    }

    /// Every series, sorted by labels so the exposition is stable.
    pub fn snapshot(&self) -> Vec<(Labels, SeriesValue)> {
        let map = self.series.read().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<(Labels, SeriesValue)> =
            map.iter().map(|(l, s)| (l.clone(), s.read())).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn clear(&self) {
        let map = self.series.read().unwrap_or_else(|e| e.into_inner());
        for series in map.values() {
            match series.as_ref() {
                Series::Counter(c) | Series::Gauge(c) => c.store(0f64.to_bits(), Ordering::Relaxed),
                Series::Histogram(h) => h.reset(),
            }
        }
    }
}

// ---------- the app registry ----------

/// Every app-defined metric, keyed by its Prometheus name (so `a.b` and `a_b`
/// are one metric, as they would be on the scrape).
pub struct Registry {
    families: RwLock<HashMap<String, Arc<Family>>>,
    bounds: Arc<[f64]>,
    max_families: usize,
    max_series: usize,
    pub dropped: AtomicU64,
    families_warned: AtomicBool,
}

impl Registry {
    pub fn new(bounds: Arc<[f64]>, max_families: usize, max_series: usize) -> Self {
        Self {
            families: RwLock::new(HashMap::new()),
            bounds,
            max_families,
            max_series,
            dropped: AtomicU64::new(0),
            families_warned: AtomicBool::new(false),
        }
    }

    /// Validate and apply one sample. A validation failure is an error the
    /// caller raises; a cap is not — the sample is dropped and counted.
    pub fn record(
        &self,
        kind: Kind,
        name: &str,
        value: f64,
        tags: &[(String, String)],
    ) -> Result<Outcome, String> {
        let labels = validate(kind, name, value, tags)?;
        let family = match self.family(kind, name)? {
            Some(family) => family,
            None => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(Outcome::Dropped);
            }
        };
        let outcome = family.record(&labels, value);
        if outcome == Outcome::Dropped {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        Ok(outcome)
    }

    /// The family for `name`, opened if the cap allows (`Ok(None)` when it
    /// does not); an error when `name` is already another kind of metric.
    fn family(&self, kind: Kind, name: &str) -> Result<Option<Arc<Family>>, String> {
        let key = prometheus_name(name);
        {
            let map = self.families.read().unwrap_or_else(|e| e.into_inner());
            if let Some(family) = map.get(&key) {
                return check_kind(family, kind, name).map(|_| Some(family.clone()));
            }
        }
        let mut map = self.families.write().unwrap_or_else(|e| e.into_inner());
        if let Some(family) = map.get(&key) {
            return check_kind(family, kind, name).map(|_| Some(family.clone()));
        }
        if map.len() >= self.max_families {
            if !self.families_warned.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[WARN] {} app metrics are defined; samples for new metric names are dropped",
                    self.max_families
                );
            }
            return Ok(None);
        }
        let family = Arc::new(Family::new(
            kind,
            name,
            self.bounds.clone(),
            self.max_series,
        ));
        map.insert(key, family.clone());
        Ok(Some(family))
    }

    /// The current value of one series, `None` when it was never recorded.
    pub fn value(
        &self,
        name: &str,
        tags: &[(String, String)],
    ) -> Result<Option<SeriesValue>, String> {
        validate_name(name)?;
        let labels = normalize_tags(tags)?;
        let map = self.families.read().unwrap_or_else(|e| e.into_inner());
        Ok(map
            .get(&prometheus_name(name))
            .and_then(|family| family.value(&labels)))
    }

    /// Every family, sorted by name.
    pub fn families(&self) -> Vec<Arc<Family>> {
        let map = self.families.read().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<Arc<Family>> = map.values().cloned().collect();
        out.sort_by(|a, b| a.prom_name.cmp(&b.prom_name));
        out
    }

    /// Forget every app metric (tests only: `Metrics.reset`).
    pub fn reset(&self) {
        let mut map = self.families.write().unwrap_or_else(|e| e.into_inner());
        for family in map.values() {
            family.clear();
        }
        map.clear();
        self.dropped.store(0, Ordering::Relaxed);
    }
}

fn check_kind(family: &Family, kind: Kind, name: &str) -> Result<(), String> {
    if family.kind == kind {
        Ok(())
    } else {
        Err(format!(
            "metric \"{name}\" is already a {}, it cannot be used as a {}",
            family.kind.name(),
            kind.name()
        ))
    }
}

/// Everything [`Registry::record`] checks, without storing: the series key.
pub fn validate(
    kind: Kind,
    name: &str,
    value: f64,
    tags: &[(String, String)],
) -> Result<Labels, String> {
    validate_name(name)?;
    validate_value(kind, value)?;
    normalize_tags(tags)
}

fn validate_value(kind: Kind, value: f64) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("metric value must be a finite number, got {value}"));
    }
    if kind == Kind::Counter && value < 0.0 {
        return Err(format!(
            "a counter only goes up: increment by {value} is refused (use Metrics.gauge for a value that goes down)"
        ));
    }
    Ok(())
}

/// Parse `SOLI_METRICS_BUCKETS` (`"0.005,0.01,0.1,1"`): finite, strictly
/// increasing, at least one. `None` when the text is not that.
pub fn parse_buckets(raw: &str) -> Option<Arc<[f64]>> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let v: f64 = part.parse().ok()?;
        if !v.is_finite() || out.last().is_some_and(|last| *last >= v) {
            return None;
        }
        out.push(v);
    }
    if out.is_empty() || out.len() > 64 {
        return None;
    }
    Some(out.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Arc<[f64]> {
        DEFAULT_BUCKETS.into()
    }

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn histogram_buckets_are_inclusive_upper_bounds() {
        let h = Histogram::new(Arc::from(vec![0.1, 1.0]));
        for v in [0.05, 0.1, 0.5, 1.0, 3.0] {
            h.observe(v);
        }
        let snap = h.snapshot();
        assert_eq!(snap.counts, vec![2, 2, 1]);
        assert_eq!(snap.cumulative(), vec![2, 4, 5]);
        assert_eq!(snap.count(), 5);
        assert!((snap.sum - 4.65).abs() < 1e-9);
    }

    #[test]
    fn histogram_survives_concurrent_writers() {
        let h = Arc::new(Histogram::new(bounds()));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let h = h.clone();
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        h.observe(0.003);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("writer");
        }
        let snap = h.snapshot();
        assert_eq!(snap.count(), 8000);
        assert!((snap.sum - 24.0).abs() < 1e-6, "{}", snap.sum);
    }

    #[test]
    fn names_are_validated() {
        assert!(validate_name("orders.created").is_ok());
        assert!(validate_name("_private_x9").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("9lives").is_err());
        assert!(validate_name(".dot").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("dash-ed").is_err());
        assert!(validate_name("soli.http.x").is_err(), "reserved prefix");
        assert!(validate_name("SOLI_x").is_err(), "reserved prefix");
        assert!(validate_name(&"a".repeat(201)).is_err());
        assert_eq!(prometheus_name("orders.created"), "orders_created");
    }

    #[test]
    fn tags_are_validated_and_sorted() {
        let labels = normalize_tags(&tags(&[("region", "eu"), ("http.method", "GET")])).unwrap();
        assert_eq!(
            labels,
            vec![
                ("http_method".to_string(), "GET".to_string()),
                ("region".to_string(), "eu".to_string())
            ]
        );
        assert!(normalize_tags(&tags(&[("le", "1")])).is_err());
        assert!(normalize_tags(&tags(&[("__name__", "x")])).is_err());
        assert!(normalize_tags(&tags(&[("bad key", "x")])).is_err());
        assert!(normalize_tags(&tags(&[("a.b", "1"), ("a_b", "2")])).is_err());
        assert!(normalize_tags(&tags(&[("k", &"v".repeat(201))])).is_err());
        let many: Vec<(String, String)> = (0..21).map(|i| (format!("k{i}"), "v".into())).collect();
        assert!(normalize_tags(&many).is_err());
    }

    #[test]
    fn counters_add_gauges_set_and_kinds_do_not_mix() {
        let reg = Registry::new(bounds(), 10, 10);
        reg.record(Kind::Counter, "jobs.done", 1.0, &[]).unwrap();
        reg.record(Kind::Counter, "jobs.done", 2.5, &[]).unwrap();
        assert_eq!(
            reg.value("jobs.done", &[]).unwrap(),
            Some(SeriesValue::Number(3.5))
        );
        reg.record(Kind::Gauge, "queue.depth", 7.0, &[]).unwrap();
        reg.record(Kind::Gauge, "queue.depth", 3.0, &[]).unwrap();
        assert_eq!(
            reg.value("queue_depth", &[]).unwrap(),
            Some(SeriesValue::Number(3.0)),
            "the Prometheus spelling reaches the same metric"
        );
        let err = reg.record(Kind::Gauge, "jobs.done", 1.0, &[]).unwrap_err();
        assert!(err.contains("already a counter"), "{err}");
        assert!(reg.record(Kind::Counter, "jobs.done", -1.0, &[]).is_err());
        assert!(reg.record(Kind::Gauge, "g", f64::NAN, &[]).is_err());
        assert_eq!(reg.value("never", &[]).unwrap(), None);
    }

    #[test]
    fn series_cap_drops_new_combinations_but_keeps_old_ones() {
        let reg = Registry::new(bounds(), 10, 3);
        for i in 0..5 {
            let outcome = reg
                .record(
                    Kind::Counter,
                    "hits",
                    1.0,
                    &tags(&[("user", &i.to_string())]),
                )
                .unwrap();
            assert_eq!(
                outcome,
                if i < 3 {
                    Outcome::Recorded
                } else {
                    Outcome::Dropped
                }
            );
        }
        // An existing combination still records past the cap.
        assert_eq!(
            reg.record(Kind::Counter, "hits", 1.0, &tags(&[("user", "0")]))
                .unwrap(),
            Outcome::Recorded
        );
        assert_eq!(reg.dropped.load(Ordering::Relaxed), 2);
        assert_eq!(reg.families()[0].series_count(), 3);
    }

    #[test]
    fn family_cap_drops_new_names() {
        let reg = Registry::new(bounds(), 2, 10);
        reg.record(Kind::Counter, "a", 1.0, &[]).unwrap();
        reg.record(Kind::Counter, "b", 1.0, &[]).unwrap();
        assert_eq!(
            reg.record(Kind::Counter, "c", 1.0, &[]).unwrap(),
            Outcome::Dropped
        );
        assert_eq!(reg.families().len(), 2);
        reg.reset();
        assert!(reg.families().is_empty());
        assert_eq!(reg.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn bucket_override_parses_or_refuses() {
        assert_eq!(
            parse_buckets("0.01, 0.1,1").as_deref(),
            Some(&[0.01, 0.1, 1.0][..])
        );
        assert!(parse_buckets("1,0.5").is_none(), "not increasing");
        assert!(parse_buckets("a,b").is_none());
        assert!(parse_buckets("").is_none());
        assert!(parse_buckets("inf").is_none());
    }
}
