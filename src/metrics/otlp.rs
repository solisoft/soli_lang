//! OTLP/HTTP JSON metrics export: every metric, every interval.
//!
//! Opt-in (`SOLI_OTEL_METRICS=1` or `OTEL_METRICS_EXPORTER=otlp`). A
//! background thread wakes every `OTEL_METRIC_EXPORT_INTERVAL` ms, reads every
//! family — built-in and app — and POSTs one `ExportMetricsServiceRequest`.
//! Counters are cumulative monotonic sums, gauges gauges, histograms explicit-
//! bucket histograms, all with cumulative temporality since process start
//! (Datadog's OTLP intake turns cumulative into deltas). Workers are never
//! involved: the export reads the same atomics the scrape reads.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use web_time::{SystemTime, UNIX_EPOCH};

use super::config::OtlpConfig;
use super::registry::{Kind, SeriesValue};
use super::FamilySnapshot;

fn unix_nanos(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// When the process started counting: every cumulative point starts here.
pub fn start_time_nanos() -> u64 {
    static START: OnceLock<u64> = OnceLock::new();
    *START.get_or_init(|| unix_nanos(SystemTime::now()))
}

fn attributes(pairs: &[(String, String)]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|(k, v)| json!({ "key": k, "value": { "stringValue": v } }))
            .collect(),
    )
}

/// One `ExportMetricsServiceRequest`, as OTLP/JSON: 64-bit integers are
/// strings, as the protobuf JSON mapping requires.
pub fn build_payload(
    resource: &[(String, String)],
    families: &[FamilySnapshot],
    start_nanos: u64,
    now_nanos: u64,
) -> Value {
    let start = start_nanos.to_string();
    let now = now_nanos.to_string();
    let metrics: Vec<Value> = families
        .iter()
        .filter(|f| !f.series.is_empty())
        .map(|family| {
            let points: Vec<Value> = family
                .series
                .iter()
                .map(|(labels, value)| {
                    let mut point = json!({
                        "attributes": attributes(labels),
                        "startTimeUnixNano": start,
                        "timeUnixNano": now,
                    });
                    match value {
                        SeriesValue::Number(n) => {
                            point["asDouble"] = json!(n);
                        }
                        SeriesValue::Histogram(h) => {
                            point["count"] = json!(h.count().to_string());
                            point["sum"] = json!(h.sum);
                            point["bucketCounts"] = Value::Array(
                                h.counts.iter().map(|c| json!(c.to_string())).collect(),
                            );
                            point["explicitBounds"] = json!(h.bounds.to_vec());
                        }
                    }
                    point
                })
                .collect();
            let mut metric = json!({ "name": family.name });
            if let Some(help) = family.help {
                metric["description"] = json!(help);
            }
            if let Some(unit) = family.unit {
                metric["unit"] = json!(unit);
            }
            match family.kind {
                Kind::Counter => {
                    metric["sum"] = json!({
                        "dataPoints": points,
                        "aggregationTemporality": 2,
                        "isMonotonic": true,
                    });
                }
                Kind::Gauge => {
                    metric["gauge"] = json!({ "dataPoints": points });
                }
                Kind::Histogram => {
                    metric["histogram"] = json!({
                        "dataPoints": points,
                        "aggregationTemporality": 2,
                    });
                }
            }
            metric
        })
        .collect();
    json!({
        "resourceMetrics": [{
            "resource": { "attributes": attributes(resource) },
            "scopeMetrics": [{
                "scope": { "name": "soli", "version": env!("CARGO_PKG_VERSION") },
                "metrics": metrics,
            }],
        }],
    })
}

/// Read every family now and send it.
pub fn export_once(cfg: &OtlpConfig) -> Result<(), String> {
    let families = super::Metrics::global().families();
    let body = build_payload(
        &cfg.resource,
        &families,
        start_time_nanos(),
        unix_nanos(SystemTime::now()),
    );
    post(cfg, &body.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn post(cfg: &OtlpConfig, body: &str) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_millis(1000))
        .timeout(cfg.timeout)
        .build();
    let mut request = agent
        .post(&cfg.endpoint)
        .set("Content-Type", "application/json");
    for (k, v) in &cfg.headers {
        request = request.set(k, v);
    }
    match request.send_string(body) {
        Ok(_) => Ok(()),
        // Backpressure: the next interval carries the same cumulative values.
        Err(ureq::Error::Status(429, _)) => Ok(()),
        // The status alone: the error text would echo the URL, and the
        // headers stay out of every message.
        Err(ureq::Error::Status(status, _)) => Err(format!("HTTP {status}")),
        Err(ureq::Error::Transport(t)) => Err(t.kind().to_string()),
    }
}

#[cfg(target_arch = "wasm32")]
fn post(_cfg: &OtlpConfig, _body: &str) -> Result<(), String> {
    Err(crate::platform::unsupported_on_edge("OTLP metrics export"))
}

/// Start the export thread once, when OTLP metrics are configured.
pub fn ensure_started() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    let Some(cfg) = super::config::config().otlp.as_ref() else {
        return;
    };
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    start_time_nanos();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let cfg = cfg.clone();
        let spawned = std::thread::Builder::new()
            .name("soli-otlp-metrics".into())
            .spawn(move || loop {
                std::thread::sleep(cfg.interval);
                if let Err(e) = export_once(&cfg) {
                    static WARNED: AtomicBool = AtomicBool::new(false);
                    if !WARNED.swap(true, Ordering::Relaxed) {
                        eprintln!(
                            "[WARN] OTLP metrics export to {} failed (further failures suppressed): {e}",
                            redact(&cfg.endpoint)
                        );
                    }
                }
            });
        if let Err(e) = spawned {
            eprintln!("[WARN] OTLP metrics: cannot start the export thread: {e}");
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = cfg;
}

/// An endpoint without its userinfo or query, for a log line.
fn redact(endpoint: &str) -> String {
    let no_query = endpoint.split(['?', '#']).next().unwrap_or("");
    let Some((scheme, rest)) = no_query.split_once("://") else {
        return no_query.to_string();
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or(authority);
    format!("{scheme}://{host}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::registry::Histogram;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::Arc;
    use std::time::Duration;

    fn families() -> Vec<FamilySnapshot> {
        let h = Histogram::new(Arc::from(vec![0.1, 1.0]));
        h.observe(0.05);
        h.observe(2.0);
        vec![
            FamilySnapshot {
                name: "orders.created".into(),
                prom_name: "orders_created".into(),
                help: None,
                unit: None,
                kind: Kind::Counter,
                series: vec![(
                    vec![("plan".into(), "pro".into())],
                    SeriesValue::Number(3.0),
                )],
            },
            FamilySnapshot {
                name: "queue.depth".into(),
                prom_name: "queue_depth".into(),
                help: None,
                unit: None,
                kind: Kind::Gauge,
                series: vec![(Vec::new(), SeriesValue::Number(7.5))],
            },
            FamilySnapshot {
                name: "soli.http.request.duration".into(),
                prom_name: "soli_http_request_duration_seconds".into(),
                help: Some("Request duration."),
                unit: Some("s"),
                kind: Kind::Histogram,
                series: vec![(Vec::new(), SeriesValue::Histogram(h.snapshot()))],
            },
            FamilySnapshot {
                name: "empty".into(),
                prom_name: "empty".into(),
                help: None,
                unit: None,
                kind: Kind::Gauge,
                series: Vec::new(),
            },
        ]
    }

    #[test]
    fn payload_has_the_otlp_shape() {
        let resource = vec![("service.name".to_string(), "shop".to_string())];
        let body = build_payload(&resource, &families(), 100, 200);
        let rm = &body["resourceMetrics"][0];
        assert_eq!(rm["resource"]["attributes"][0]["key"], "service.name");
        assert_eq!(
            rm["resource"]["attributes"][0]["value"]["stringValue"],
            "shop"
        );
        let metrics = rm["scopeMetrics"][0]["metrics"]
            .as_array()
            .expect("metrics");
        assert_eq!(metrics.len(), 3, "a family with no series is left out");

        let counter = &metrics[0];
        assert_eq!(counter["name"], "orders.created");
        assert_eq!(counter["sum"]["isMonotonic"], true);
        assert_eq!(counter["sum"]["aggregationTemporality"], 2);
        let point = &counter["sum"]["dataPoints"][0];
        assert_eq!(point["asDouble"], 3.0);
        assert_eq!(point["startTimeUnixNano"], "100");
        assert_eq!(point["timeUnixNano"], "200");
        assert_eq!(point["attributes"][0]["key"], "plan");

        assert_eq!(metrics[1]["gauge"]["dataPoints"][0]["asDouble"], 7.5);

        let hist = &metrics[2];
        assert_eq!(hist["unit"], "s");
        assert_eq!(hist["description"], "Request duration.");
        let point = &hist["histogram"]["dataPoints"][0];
        assert_eq!(point["count"], "2");
        assert_eq!(point["bucketCounts"], json!(["1", "0", "1"]));
        assert_eq!(point["explicitBounds"], json!([0.1, 1.0]));
        assert_eq!(point["sum"], 2.05);
    }

    /// A loopback collector: answers one POST with `status`, hands back the
    /// request head and body.
    fn collector(status: u16) -> (String, std::thread::JoinHandle<(String, String)>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/v1/metrics", listener.local_addr().expect("addr"));
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut head = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("line");
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().expect("length");
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).expect("body");
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .expect("respond");
            (head, String::from_utf8(body).expect("utf8"))
        });
        (url, handle)
    }

    fn cfg(endpoint: String) -> OtlpConfig {
        OtlpConfig {
            endpoint,
            headers: vec![("DD-API-KEY".to_string(), "secret".to_string())],
            interval: Duration::from_secs(60),
            timeout: Duration::from_secs(5),
            resource: vec![("service.name".to_string(), "loop".to_string())],
        }
    }

    #[test]
    fn a_loopback_collector_receives_the_export_with_headers() {
        let (url, handle) = collector(200);
        crate::metrics::Metrics::global()
            .app
            .record(Kind::Counter, "otlp_loopback.hits", 2.0, &[])
            .expect("record");
        export_once(&cfg(url)).expect("export");
        let (head, body) = handle.join().expect("collector");
        let head = head.to_ascii_lowercase();
        assert!(head.starts_with("post /v1/metrics"), "{head}");
        assert!(head.contains("content-type: application/json"), "{head}");
        assert!(head.contains("dd-api-key: secret"), "{head}");
        let payload: Value = serde_json::from_str(&body).expect("json body");
        let metrics = payload["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
            .as_array()
            .expect("metrics");
        let hit = metrics
            .iter()
            .find(|m| m["name"] == "otlp_loopback.hits")
            .expect("the app counter is exported");
        assert_eq!(hit["sum"]["dataPoints"][0]["asDouble"], 2.0);
        assert!(
            metrics.iter().any(|m| m["name"] == "soli.http.requests"),
            "built-in series ride along"
        );
    }

    #[test]
    fn a_refusing_collector_is_an_error_without_secrets() {
        let (url, handle) = collector(403);
        let err = export_once(&cfg(url)).expect_err("403");
        handle.join().expect("collector");
        assert_eq!(err, "HTTP 403");
        let (url, handle) = collector(429);
        export_once(&cfg(url)).expect("429 is backpressure, not failure");
        handle.join().expect("collector");
    }

    #[test]
    fn endpoints_are_redacted_for_logs() {
        assert_eq!(
            redact("https://user:pw@host/v1/metrics?k=1"),
            "https://host/v1/metrics"
        );
        assert_eq!(
            redact("http://host:4318/v1/metrics"),
            "http://host:4318/v1/metrics"
        );
    }
}
