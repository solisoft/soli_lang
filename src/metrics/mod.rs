//! Metrics: the built-in series, app metrics (`Metrics.increment(...)` and
//! friends in Soli), and the three places they go.
//!
//! - **Prometheus** text at `GET /_metrics` (`SOLI_METRICS=1`) — always served,
//!   filled only when something reads the metrics.
//! - **DogStatsD** (`DD_AGENT_HOST`, `DD_DOGSTATSD_URL` or `SOLI_STATSD`):
//!   app samples and the request duration as UDP / Unix datagram lines
//!   ([`statsd`]).
//! - **OTLP/HTTP JSON** (`SOLI_OTEL_METRICS=1` or `OTEL_METRICS_EXPORTER=otlp`):
//!   every family, every interval ([`otlp`]).
//!
//! Any one of them turns collection on ([`metrics_enabled`]); with none, the
//! built-in timers skip their `Instant::now()` and an app sample is validated
//! and dropped. Durations are real histograms (`_bucket{le=…}`, `_sum`,
//! `_count`), buckets in seconds ([`registry::DEFAULT_BUCKETS`], or
//! `SOLI_METRICS_BUCKETS`).

pub mod config;
pub mod otlp;
pub mod registry;
pub mod statsd;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, OnceLock};
use web_time::Duration;

use registry::{Family, Histogram, Kind, Labels, Registry, SeriesValue};
use statsd::StatsdType;

/// Label sets the per-route request histogram may hold (methods × routes ×
/// status classes).
const MAX_REQUEST_SERIES: usize = 5000;

/// Whether metrics collection is enabled for this process: `SOLI_METRICS=1`,
/// a DogStatsD target, or OTLP metrics export. Read once and cached.
///
/// Collection is opt-in: it is only worth the per-operation `Instant::now()`
/// and atomic bookkeeping when something reads the metrics.
#[inline]
pub fn metrics_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| config::config().any_sink())
}

/// Whether app samples are stored: collection is on, or this is a test run
/// (`APP_ENV=test`, which `soli test` sets), so a spec can read them back.
pub fn app_metrics_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        metrics_enabled() || crate::platform::env::var("APP_ENV").as_deref() == Ok("test")
    })
}

/// Start whatever background exporter is configured (the OTLP interval
/// thread; the DogStatsD sender starts with its first line). Idempotent and
/// cheap after the first call.
pub fn start_exporters() {
    otlp::ensure_started();
    let _ = statsd::sink();
}

/// One metric family as read at an instant: what the Prometheus text and the
/// OTLP payload are both built from.
#[derive(Clone, Debug)]
pub struct FamilySnapshot {
    /// The OTLP / DogStatsD name (dotted).
    pub name: String,
    pub prom_name: String,
    pub help: Option<&'static str>,
    pub unit: Option<&'static str>,
    pub kind: Kind,
    pub series: Vec<(Labels, SeriesValue)>,
}

pub struct Metrics {
    pub http_requests_total: AtomicU64,
    pub lexing: Histogram,
    pub parsing: Histogram,
    pub vm_execution: Histogram,
    /// Wall time spent rendering templates (views + layouts + partials).
    pub template_render: Histogram,
    /// Time spent inside all middleware of one request (from middleware_log).
    pub middleware: Histogram,
    /// Time spent in DB / SolidB queries of one request (from query_log).
    pub db_query: Histogram,
    /// Number of handlers demoted from the VM to the tree-walking interpreter
    /// (per worker, once — the demotion is then cached): the VM refused them,
    /// failed where the tree-walker's re-run succeeded, or failed after a
    /// durable write. An error both engines raise demotes nothing.
    /// A non-zero value means some production handlers run on the slower
    /// engine; set `SOLI_ENGINE_LOG=1` to log which handler and why.
    pub vm_handler_demotions_total: AtomicU64,
    /// Number of handlers that failed on the VM and then succeeded when re-run
    /// on the tree-walker: the two engines disagree on the same code, which
    /// is a VM bug whatever the error said. Should stay at zero.
    pub vm_engine_divergences_total: AtomicU64,
    /// Number of requests whose handler panicked and was contained by the
    /// per-request `catch_unwind` in the worker loop (the client got a `500`
    /// and the worker survived). Any non-zero value is a bug worth chasing —
    /// the panic payload and backtrace are on stderr.
    ///
    /// Counted unconditionally, like `vm_handler_demotions_total`: a panic is
    /// rare enough that the atomic is free, and the number is most wanted
    /// exactly when nobody thought to set `SOLI_METRICS` in advance.
    pub handler_panics_total: AtomicU64,
    /// `soli_http_request_duration_seconds{method,route,status_class}`.
    pub http_request_duration: Family,
    /// Every app-defined metric.
    pub app: Registry,
}

/// A built-in histogram's names and help.
struct Builtin {
    name: &'static str,
    prom_name: &'static str,
    help: &'static str,
}

const LEXING: Builtin = Builtin {
    name: "soli.lexing.duration",
    prom_name: "soli_lexing_duration_seconds",
    help: "Time spent in the lexer, in seconds.",
};
const PARSING: Builtin = Builtin {
    name: "soli.parsing.duration",
    prom_name: "soli_parsing_duration_seconds",
    help: "Time spent in the parser, in seconds.",
};
const VM_EXECUTION: Builtin = Builtin {
    name: "soli.vm.execution.duration",
    prom_name: "soli_vm_execution_seconds",
    help: "Time spent executing bytecode in the VM (outermost runs), in seconds.",
};
const TEMPLATE_RENDER: Builtin = Builtin {
    name: "soli.template.render.duration",
    prom_name: "soli_template_render_duration_seconds",
    help: "Time spent rendering templates (views, layouts, partials), in seconds.",
};
const MIDDLEWARE: Builtin = Builtin {
    name: "soli.middleware.duration",
    prom_name: "soli_middleware_duration_seconds",
    help: "Time spent in middleware per request (all middleware combined), in seconds.",
};
const DB_QUERY: Builtin = Builtin {
    name: "soli.db.query.duration",
    prom_name: "soli_db_query_duration_seconds",
    help: "Time spent in database / SolidB queries per request, in seconds.",
};
const HTTP_REQUEST_HELP: &str =
    "Time from routing to response per request, by method, route (controller#action) and status class, in seconds.";

impl Metrics {
    pub fn new() -> Self {
        let buckets = config::config().buckets.clone();
        let h = || Histogram::new(buckets.clone());
        Self {
            http_requests_total: AtomicU64::new(0),
            lexing: h(),
            parsing: h(),
            vm_execution: h(),
            template_render: h(),
            middleware: h(),
            db_query: h(),
            vm_handler_demotions_total: AtomicU64::new(0),
            vm_engine_divergences_total: AtomicU64::new(0),
            handler_panics_total: AtomicU64::new(0),
            http_request_duration: Family::new(
                Kind::Histogram,
                "soli.http.request.duration",
                buckets.clone(),
                MAX_REQUEST_SERIES,
            ),
            app: Registry::new(
                buckets,
                registry::MAX_FAMILIES,
                registry::MAX_SERIES_PER_FAMILY,
            ),
        }
    }

    pub fn global() -> &'static Self {
        static METRICS: LazyLock<Metrics> = LazyLock::new(|| {
            otlp::start_time_nanos();
            Metrics::new()
        });
        &METRICS
    }

    /// Every family — built-in first, then the app's by name.
    pub fn families(&self) -> Vec<FamilySnapshot> {
        let counter =
            |name: &str, prom_name: &str, help: &'static str, v: &AtomicU64| FamilySnapshot {
                name: name.to_string(),
                prom_name: prom_name.to_string(),
                help: Some(help),
                unit: None,
                kind: Kind::Counter,
                series: vec![(
                    Vec::new(),
                    SeriesValue::Number(v.load(Ordering::Relaxed) as f64),
                )],
            };
        let histogram = |b: &Builtin, h: &Histogram| FamilySnapshot {
            name: b.name.to_string(),
            prom_name: b.prom_name.to_string(),
            help: Some(b.help),
            unit: Some("s"),
            kind: Kind::Histogram,
            series: vec![(Vec::new(), SeriesValue::Histogram(h.snapshot()))],
        };
        let mut out = vec![
            counter(
                "soli.http.requests",
                "soli_http_requests_total",
                "Total number of HTTP requests handled.",
                &self.http_requests_total,
            ),
            histogram(&LEXING, &self.lexing),
            histogram(&PARSING, &self.parsing),
            histogram(&VM_EXECUTION, &self.vm_execution),
            histogram(&TEMPLATE_RENDER, &self.template_render),
            histogram(&MIDDLEWARE, &self.middleware),
            histogram(&DB_QUERY, &self.db_query),
            FamilySnapshot {
                name: self.http_request_duration.name.clone(),
                prom_name: "soli_http_request_duration_seconds".to_string(),
                help: Some(HTTP_REQUEST_HELP),
                unit: Some("s"),
                kind: Kind::Histogram,
                series: self.http_request_duration.snapshot(),
            },
            counter(
                "soli.vm.handler.demotions",
                "soli_vm_handler_demotions_total",
                "Handlers demoted from the bytecode VM to the tree-walking interpreter (cached per worker; non-zero means some handlers run on the slower engine).",
                &self.vm_handler_demotions_total,
            ),
            counter(
                "soli.vm.engine.divergences",
                "soli_vm_engine_divergences_total",
                "Handlers that failed on the bytecode VM and succeeded when re-run on the tree-walking interpreter (a VM bug; should stay at zero).",
                &self.vm_engine_divergences_total,
            ),
            counter(
                "soli.handler.panics",
                "soli_handler_panics_total",
                "Requests whose handler panicked and was contained by the worker (client received a 500; the worker stayed up).",
                &self.handler_panics_total,
            ),
        ];
        let statsd_dropped = statsd::sink()
            .map(|s| s.dropped.load(Ordering::Relaxed))
            .unwrap_or(0);
        out.push(FamilySnapshot {
            name: "soli.metrics.dropped".to_string(),
            prom_name: "soli_metrics_dropped_total".to_string(),
            help: Some(
                "App metric samples dropped: past a cardinality cap, or a full DogStatsD queue.",
            ),
            unit: None,
            kind: Kind::Counter,
            series: vec![(
                Vec::new(),
                SeriesValue::Number(
                    (self.app.dropped.load(Ordering::Relaxed) + statsd_dropped) as f64,
                ),
            )],
        });
        for family in self.app.families() {
            out.push(FamilySnapshot {
                name: family.name.clone(),
                prom_name: family.prom_name.clone(),
                help: None,
                unit: None,
                kind: family.kind,
                series: family.snapshot(),
            });
        }
        out
    }

    pub fn render_prometheus(&self) -> String {
        render_prometheus(&self.families())
    }

    pub fn record_lexing(&self, elapsed: Duration) {
        if metrics_enabled() {
            self.lexing.observe(elapsed.as_secs_f64());
        }
    }

    pub fn record_parsing(&self, elapsed: Duration) {
        if metrics_enabled() {
            self.parsing.observe(elapsed.as_secs_f64());
        }
    }

    /// Record one outermost VM activation. The caller has checked
    /// [`metrics_enabled`]: a VM reads it once, when it is made, and times
    /// only its outermost `run` (see `Vm::run`).
    pub fn record_vm_execution(&self, elapsed: Duration) {
        self.vm_execution.observe(elapsed.as_secs_f64());
    }

    /// Record one template render (view + layout + any partials executed during it).
    pub fn record_template_render(&self, elapsed: Duration) {
        if metrics_enabled() {
            self.template_render.observe(elapsed.as_secs_f64());
        }
    }

    /// Record total time spent in middleware for one request.
    pub fn record_middleware(&self, elapsed: Duration) {
        if metrics_enabled() {
            self.middleware.observe(elapsed.as_secs_f64());
        }
    }

    /// Record total time spent in DB/SolidB queries for one request.
    pub fn record_db_queries(&self, elapsed: Duration) {
        if metrics_enabled() {
            self.db_query.observe(elapsed.as_secs_f64());
        }
    }

    /// Record one served request: the per-route histogram, and the same
    /// sample to DogStatsD as `soli.http.request.duration` (a distribution).
    /// `route` is the matched `controller#action`, `None` when nothing matched.
    pub fn record_request(
        &self,
        method: &str,
        status: u16,
        route: Option<&str>,
        elapsed: Duration,
    ) {
        if !metrics_enabled() {
            return;
        }
        let labels = request_labels(method, status, route);
        let secs = elapsed.as_secs_f64();
        if self.http_request_duration.record(&labels, secs) == registry::Outcome::Recorded {
            if let Some(sink) = statsd::sink() {
                sink.send(statsd::format_line(
                    "soli.http.request.duration",
                    secs,
                    StatsdType::Distribution,
                    &labels,
                    &sink.global_tags,
                ));
            }
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// `method`, `route` and `status_class` labels, sorted. Methods outside the
/// standard set become `OTHER` so a client cannot mint label values.
pub fn request_labels(method: &str, status: u16, route: Option<&str>) -> Labels {
    const METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
    let method = METHODS
        .iter()
        .find(|m| m.eq_ignore_ascii_case(method))
        .copied()
        .unwrap_or("OTHER");
    let status_class = match status {
        100..=599 => format!("{}xx", status / 100),
        _ => "other".to_string(),
    };
    vec![
        ("method".to_string(), method.to_string()),
        (
            "route".to_string(),
            route.unwrap_or("unmatched").to_string(),
        ),
        ("status_class".to_string(), status_class),
    ]
}

// ---------- app samples ----------

/// One app sample from Soli (`Metrics.increment` & co.). Always validated;
/// stored and forwarded only when [`app_metrics_enabled`].
pub fn record_app(
    kind: Kind,
    statsd_type: StatsdType,
    name: &str,
    value: f64,
    tags: &[(String, String)],
) -> Result<(), String> {
    if !app_metrics_enabled() {
        return registry::validate(kind, name, value, tags).map(|_| ());
    }
    let outcome = Metrics::global().app.record(kind, name, value, tags)?;
    if outcome == registry::Outcome::Recorded {
        if let Some(sink) = statsd::sink() {
            sink.send(statsd::format_line(
                name,
                value,
                statsd_type,
                tags,
                &sink.global_tags,
            ));
        }
    }
    otlp::ensure_started();
    Ok(())
}

// ---------- Prometheus text ----------

/// A sample value as the text format writes it.
pub fn format_prom_value(v: f64) -> String {
    if v.is_nan() {
        "NaN".to_string()
    } else if v.is_infinite() {
        if v > 0.0 { "+Inf" } else { "-Inf" }.to_string()
    } else {
        format!("{v}")
    }
}

fn escape_label_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

fn escape_help(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('\n', "\\n")
}

/// `{k="v",…}` with `extra` (the `le` of a bucket) last; empty for no labels.
fn label_block(labels: &Labels, extra: Option<(&str, &str)>) -> String {
    if labels.is_empty() && extra.is_none() {
        return String::new();
    }
    let mut parts: Vec<String> = labels
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", escape_label_value(v)))
        .collect();
    if let Some((k, v)) = extra {
        parts.push(format!("{k}=\"{v}\""));
    }
    format!("{{{}}}", parts.join(","))
}

/// The Prometheus text exposition (version 0.0.4) of `families`.
pub fn render_prometheus(families: &[FamilySnapshot]) -> String {
    let mut out = String::with_capacity(4096);
    for family in families {
        let name = &family.prom_name;
        if let Some(help) = family.help {
            out.push_str(&format!("# HELP {name} {}\n", escape_help(help)));
        }
        out.push_str(&format!("# TYPE {name} {}\n", family.kind.name()));
        for (labels, value) in &family.series {
            match value {
                SeriesValue::Number(n) => {
                    out.push_str(&format!(
                        "{name}{} {}\n",
                        label_block(labels, None),
                        format_prom_value(*n)
                    ));
                }
                SeriesValue::Histogram(h) => {
                    let cumulative = h.cumulative();
                    for (i, count) in cumulative.iter().enumerate() {
                        let le = match h.bounds.get(i) {
                            Some(bound) => format_prom_value(*bound),
                            None => "+Inf".to_string(),
                        };
                        out.push_str(&format!(
                            "{name}_bucket{} {count}\n",
                            label_block(labels, Some(("le", &le)))
                        ));
                    }
                    let block = label_block(labels, None);
                    out.push_str(&format!("{name}_sum{block} {}\n", format_prom_value(h.sum)));
                    out.push_str(&format!("{name}_count{block} {}\n", h.count()));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn histogram_exposition_is_cumulative_with_sum_and_count() {
        let h = Histogram::new(Arc::from(vec![0.1, 1.0]));
        h.observe(0.05);
        h.observe(0.5);
        h.observe(4.0);
        let text = render_prometheus(&[FamilySnapshot {
            name: "checkout.time".into(),
            prom_name: "checkout_time".into(),
            help: Some("Checkout time.\nIn seconds \\ really."),
            unit: Some("s"),
            kind: Kind::Histogram,
            series: vec![(
                vec![("plan".into(), "pro \"x\"\nline\\".into())],
                SeriesValue::Histogram(h.snapshot()),
            )],
        }]);
        let expected = "# HELP checkout_time Checkout time.\\nIn seconds \\\\ really.\n\
# TYPE checkout_time histogram\n\
checkout_time_bucket{plan=\"pro \\\"x\\\"\\nline\\\\\",le=\"0.1\"} 1\n\
checkout_time_bucket{plan=\"pro \\\"x\\\"\\nline\\\\\",le=\"1\"} 2\n\
checkout_time_bucket{plan=\"pro \\\"x\\\"\\nline\\\\\",le=\"+Inf\"} 3\n\
checkout_time_sum{plan=\"pro \\\"x\\\"\\nline\\\\\"} 4.55\n\
checkout_time_count{plan=\"pro \\\"x\\\"\\nline\\\\\"} 3\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn counters_and_gauges_render_plain_samples() {
        let text = render_prometheus(&[
            FamilySnapshot {
                name: "a".into(),
                prom_name: "a".into(),
                help: None,
                unit: None,
                kind: Kind::Counter,
                series: vec![(Vec::new(), SeriesValue::Number(5.0))],
            },
            FamilySnapshot {
                name: "g".into(),
                prom_name: "g".into(),
                help: None,
                unit: None,
                kind: Kind::Gauge,
                series: vec![(vec![("k".into(), "v".into())], SeriesValue::Number(-1.5))],
            },
        ]);
        assert_eq!(
            text,
            "# TYPE a counter\na 5\n# TYPE g gauge\ng{k=\"v\"} -1.5\n"
        );
        assert_eq!(format_prom_value(f64::INFINITY), "+Inf");
        assert_eq!(format_prom_value(f64::NAN), "NaN");
    }

    #[test]
    fn builtin_exposition_keeps_counter_names_and_adds_histograms() {
        let m = Metrics::new();
        m.http_requests_total.fetch_add(2, Ordering::Relaxed);
        m.db_query.observe(0.003);
        m.http_request_duration
            .record(&request_labels("get", 404, Some("posts#show")), 0.02);
        m.http_request_duration
            .record(&request_labels("BREW", 200, None), 0.02);
        let text = m.render_prometheus();
        assert!(
            text.contains("# TYPE soli_http_requests_total counter\nsoli_http_requests_total 2\n"),
            "{text}"
        );
        assert!(
            text.contains("# TYPE soli_db_query_duration_seconds histogram\n"),
            "{text}"
        );
        assert!(
            text.contains("soli_db_query_duration_seconds_bucket{le=\"0.005\"} 1\n"),
            "{text}"
        );
        assert!(
            text.contains("soli_db_query_duration_seconds_bucket{le=\"0.0025\"} 0\n"),
            "{text}"
        );
        assert!(
            text.contains("soli_db_query_duration_seconds_count 1\n"),
            "{text}"
        );
        assert!(
            text.contains("soli_db_query_duration_seconds_sum 0.003\n"),
            "{text}"
        );
        assert!(
            text.contains("soli_http_request_duration_seconds_count{method=\"GET\",route=\"posts#show\",status_class=\"4xx\"} 1\n"),
            "{text}"
        );
        assert!(
            text.contains("soli_http_request_duration_seconds_count{method=\"OTHER\",route=\"unmatched\",status_class=\"2xx\"} 1\n"),
            "{text}"
        );
        assert!(text.contains("soli_vm_handler_demotions_total 0\n"));
        assert!(text.contains("soli_metrics_dropped_total 0\n"));
        // The old bare-name sum is gone: it is `_sum` now.
        assert!(
            !text.contains("\nsoli_db_query_duration_seconds "),
            "{text}"
        );
    }

    #[test]
    fn app_families_follow_the_builtins() {
        let m = Metrics::new();
        m.app
            .record(
                Kind::Counter,
                "orders.created",
                1.0,
                &[("plan".into(), "pro".into())],
            )
            .unwrap();
        let text = m.render_prometheus();
        assert!(
            text.ends_with("# TYPE orders_created counter\norders_created{plan=\"pro\"} 1\n"),
            "{text}"
        );
    }
}
