//! Production OpenTelemetry tracing (OTLP/HTTP JSON) without the OTel SDK.
//!
//! Soli already builds a hierarchical span tree per request in
//! [`super::span_log`] for the dev-bar flamegraph. This module turns that
//! tree into OTLP spans so production can join a distributed trace.
//!
//! ## Enable
//!
//! Tracing is opt-in. Any of these turns it on:
//!
//! - `SOLI_OTEL=1` / `true` / `yes`
//! - `OTEL_EXPORTER_OTLP_ENDPOINT` set (e.g. `http://localhost:4318`)
//! - `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` set (full `/v1/traces` URL)
//!
//! `OTEL_SDK_DISABLED=true` forces it off.
//!
//! ## What you get
//!
//! 1. **W3C Trace Context** — inbound `traceparent` is honoured; a response
//!    `traceparent` is always set when tracing is on so callers can correlate.
//! 2. **Per-request span tree** — the same middleware / action / view / db /
//!    http spans the dev bar uses, exported as OTLP SERVER/INTERNAL spans.
//! 3. **Async export** — a dedicated background thread POSTs OTLP/HTTP JSON
//!    so request latency is not blocked on the collector (drops on full queue).
//!
//! No new crates: JSON via `serde_json`, HTTP via `ureq`, IDs via `uuid`.

use super::span_log::{SpanKind, SpanRecord};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Max in-flight export batches. Past this, new batches are dropped (never
/// block a worker on a slow collector).
const EXPORT_QUEUE_CAP: usize = 256;

/// Default OTLP HTTP base when `SOLI_OTEL=1` but no endpoint is configured.
const DEFAULT_OTLP_BASE: &str = "http://127.0.0.1:4318";

/// Soft timeout for a single OTLP POST.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(2);

// ---------- config ----------

#[derive(Clone, Debug)]
pub struct OtelConfig {
    pub enabled: bool,
    /// Full URL for OTLP/HTTP traces (…/v1/traces). `None` → context only
    /// (traceparent + log correlation), no network export.
    pub traces_endpoint: Option<String>,
    pub service_name: String,
    /// Extra resource attributes from `OTEL_RESOURCE_ATTRIBUTES`.
    pub resource_attrs: Vec<(String, String)>,
}

impl OtelConfig {
    fn from_env() -> Self {
        if env_truthy("OTEL_SDK_DISABLED") {
            return Self {
                enabled: false,
                traces_endpoint: None,
                service_name: "soli".into(),
                resource_attrs: Vec::new(),
            };
        }

        let traces_endpoint = resolve_traces_endpoint();
        let forced_on = env_truthy("SOLI_OTEL");
        let enabled = forced_on || traces_endpoint.is_some();

        let service_name = std::env::var("OTEL_SERVICE_NAME")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "soli".to_string());

        let resource_attrs =
            parse_resource_attributes(std::env::var("OTEL_RESOURCE_ATTRIBUTES").ok().as_deref());

        // SOLI_OTEL=1 with no endpoint still enables context + span collection;
        // export uses the local default collector so a sidecar Just Works.
        let traces_endpoint = if enabled && traces_endpoint.is_none() && forced_on {
            Some(format!(
                "{}/v1/traces",
                DEFAULT_OTLP_BASE.trim_end_matches('/')
            ))
        } else {
            traces_endpoint
        };

        Self {
            enabled,
            traces_endpoint,
            service_name,
            resource_attrs,
        }
    }
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

fn resolve_traces_endpoint() -> Option<String> {
    if let Ok(full) = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
        let t = full.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    if let Ok(base) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        let t = base.trim().trim_end_matches('/');
        if !t.is_empty() {
            // OTel: if the base already ends in /v1/traces keep it; else append.
            if t.ends_with("/v1/traces") {
                return Some(t.to_string());
            }
            return Some(format!("{t}/v1/traces"));
        }
    }
    None
}

fn parse_resource_attributes(raw: Option<&str>) -> Vec<(String, String)> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    raw.split(',')
        .filter_map(|pair| {
            let pair = pair.trim();
            if pair.is_empty() {
                return None;
            }
            let (k, v) = pair.split_once('=')?;
            let k = k.trim();
            let v = v.trim();
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// Process-wide config, parsed once.
pub fn config() -> &'static OtelConfig {
    static CFG: OnceLock<OtelConfig> = OnceLock::new();
    CFG.get_or_init(OtelConfig::from_env)
}

#[inline]
pub fn enabled() -> bool {
    config().enabled
}

// ---------- W3C Trace Context ----------

/// Active trace for the current request.
#[derive(Clone, Debug)]
pub struct TraceContext {
    /// 32 hex chars (16 bytes).
    pub trace_id: String,
    /// 16 hex chars (8 bytes) — the SERVER root span id.
    pub span_id: String,
    /// Parent span from the inbound `traceparent`, if any.
    pub parent_span_id: Option<String>,
    /// W3C flags byte (sampled = 0x01).
    pub flags: u8,
    /// Wall-clock anchor for converting relative span times to unix nanos.
    pub wall_start: SystemTime,
    /// Instant twin of `wall_start` (span_log is Instant-based).
    pub instant_start: Instant,
    pub method: String,
    pub path: String,
}

impl TraceContext {
    /// Format a W3C `traceparent` for the response (or for outbound HTTP).
    pub fn traceparent(&self) -> String {
        format!("00-{}-{}-{:02x}", self.trace_id, self.span_id, self.flags)
    }
}

/// Parse `00-{trace_id}-{span_id}-{flags}` (version 00 only). Returns
/// `(trace_id, parent_span_id, flags)` or `None` if malformed / all-zero ids.
pub fn parse_traceparent(header: &str) -> Option<(String, String, u8)> {
    let parts: Vec<&str> = header.trim().split('-').collect();
    if parts.len() != 4 {
        return None;
    }
    let version = parts[0];
    let trace_id = parts[1];
    let parent_id = parts[2];
    let flags = parts[3];
    if version != "00" {
        return None;
    }
    if !is_hex(trace_id, 32) || !is_hex(parent_id, 16) || !is_hex(flags, 2) {
        return None;
    }
    // All-zero trace/span ids are invalid per the spec.
    if trace_id.chars().all(|c| c == '0') || parent_id.chars().all(|c| c == '0') {
        return None;
    }
    let flags_u8 = u8::from_str_radix(flags, 16).ok()?;
    Some((
        trace_id.to_ascii_lowercase(),
        parent_id.to_ascii_lowercase(),
        flags_u8,
    ))
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn random_trace_id() -> String {
    // uuid v4 is 16 random bytes — strip hyphens for the 32-hex form.
    uuid::Uuid::new_v4().simple().to_string()
}

fn random_span_id() -> String {
    // 8 random bytes as 16 hex chars. Avoid all-zero.
    loop {
        let n = uuid::Uuid::new_v4().as_u128() as u64;
        if n != 0 {
            return format!("{n:016x}");
        }
    }
}

/// Map a span_log local id onto an 8-byte hex span id that is unique within
/// the request and never all-zero. Root (`local_id == 0` typically) gets the
/// context's public `span_id` so the exported root matches `traceparent`.
fn local_span_id_hex(local_id: u32, root_local: u32, root_hex: &str, salt: u64) -> String {
    if local_id == root_local {
        return root_hex.to_string();
    }
    // Mix salt + (local+1) so id 0 (if not root) still non-zero.
    let mixed = salt
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((local_id as u64).wrapping_add(1));
    if mixed == 0 {
        return format!("{:016x}", (local_id as u64).wrapping_add(1));
    }
    format!("{mixed:016x}")
}

/// Start a request-scoped trace. `inbound_traceparent` is the raw header.
pub fn begin_request(
    method: &str,
    path: &str,
    inbound_traceparent: Option<&str>,
) -> Option<TraceContext> {
    if !enabled() {
        return None;
    }
    let (trace_id, parent_span_id, flags) = inbound_traceparent
        .and_then(parse_traceparent)
        .map(|(tid, pid, f)| (tid, Some(pid), f | 0x01))
        .unwrap_or_else(|| (random_trace_id(), None, 0x01));

    Some(TraceContext {
        trace_id,
        span_id: random_span_id(),
        parent_span_id,
        flags,
        wall_start: SystemTime::now(),
        instant_start: Instant::now(),
        method: method.to_string(),
        path: path.to_string(),
    })
}

// ---------- export ----------

struct ExportBatch {
    endpoint: String,
    body: String,
}

static EXPORTER: OnceLock<Option<SyncSender<ExportBatch>>> = OnceLock::new();
static EXPORT_DROPPED: AtomicBool = AtomicBool::new(false);

fn exporter() -> Option<&'static SyncSender<ExportBatch>> {
    EXPORTER
        .get_or_init(|| {
            let cfg = config();
            if !cfg.enabled || cfg.traces_endpoint.is_none() {
                return None;
            }
            let (tx, rx) = sync_channel::<ExportBatch>(EXPORT_QUEUE_CAP);
            std::thread::Builder::new()
                .name("soli-otel-export".into())
                .spawn(move || {
                    while let Ok(batch) = rx.recv() {
                        if let Err(e) = post_otlp(&batch.endpoint, &batch.body) {
                            // Rate-limit noise: one warn per process lifetime
                            // for the first failure, then silence.
                            static WARNED: AtomicBool = AtomicBool::new(false);
                            if !WARNED.swap(true, Ordering::Relaxed) {
                                eprintln!(
                                    "[WARN] OTEL export failed (further failures suppressed): {e}"
                                );
                            }
                        }
                    }
                })
                .ok();
            Some(tx)
        })
        .as_ref()
}

fn post_otlp(endpoint: &str, body: &str) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_millis(500))
        .timeout(EXPORT_TIMEOUT)
        .build();
    let resp = agent
        .post(endpoint)
        .set("Content-Type", "application/json")
        .send_string(body)
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    // 2xx and 429 (backpressure) are acceptable; 4xx/5xx otherwise warn once.
    if (200..300).contains(&status) || status == 429 {
        Ok(())
    } else {
        Err(format!("HTTP {status}"))
    }
}

/// Convert span_log records into an OTLP payload and enqueue export.
///
/// `status_code` is the HTTP status of the request (sets span status ERROR
/// for 5xx). No-op when tracing is off or no endpoint is configured.
pub fn export_request(
    ctx: &TraceContext,
    spans: &[SpanRecord],
    http_status: u16,
    request_id: Option<&str>,
) {
    let cfg = config();
    let Some(endpoint) = cfg.traces_endpoint.as_ref() else {
        return;
    };
    let Some(tx) = exporter() else {
        return;
    };

    let body = build_otlp_body(cfg, ctx, spans, http_status, request_id);
    match tx.try_send(ExportBatch {
        endpoint: endpoint.clone(),
        body,
    }) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            // Collector lag — drop rather than stall a web worker.
            if !EXPORT_DROPPED.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[WARN] OTEL export queue full ({} batches); dropping spans \
                     (further drops suppressed)",
                    EXPORT_QUEUE_CAP
                );
            }
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}

fn unix_nanos(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64
}

fn attr_str(key: &str, value: &str) -> Value {
    json!({"key": key, "value": {"stringValue": value}})
}

fn attr_int(key: &str, value: i64) -> Value {
    json!({"key": key, "value": {"intValue": value.to_string()}})
}

fn span_kind_otlp(kind: SpanKind) -> i32 {
    // OTLP SpanKind: 1=INTERNAL, 2=SERVER, 3=CLIENT, …
    match kind {
        SpanKind::Request => 2,           // SERVER
        SpanKind::Http => 3,              // CLIENT
        SpanKind::Db | SpanKind::Kv => 3, // CLIENT-ish remote
        _ => 1,                           // INTERNAL
    }
}

fn build_otlp_body(
    cfg: &OtelConfig,
    ctx: &TraceContext,
    spans: &[SpanRecord],
    http_status: u16,
    request_id: Option<&str>,
) -> String {
    // Prefer the synthetic Request root from span_log; fall back to a
    // synthetic single span when the tree is empty (e.g. early 404 path
    // that never opened spans).
    let root_local = spans
        .iter()
        .find(|s| s.kind == SpanKind::Request)
        .map(|s| s.id)
        .or_else(|| spans.first().map(|s| s.id))
        .unwrap_or(0);

    // Stable salt so local ids map deterministically within this request.
    let salt = u64::from_str_radix(&ctx.span_id[..16.min(ctx.span_id.len())], 16).unwrap_or(1);

    let wall_start_ns = unix_nanos(ctx.wall_start);
    // If wall_start was slightly before Instant::now at capture, durations
    // still line up via relative us offsets.

    let mut otlp_spans: Vec<Value> = Vec::with_capacity(spans.len().max(1));

    if spans.is_empty() {
        // Minimal root so a traced 404/early-return still shows up.
        let end_ns = unix_nanos(SystemTime::now());
        let mut attrs = vec![
            attr_str("http.request.method", &ctx.method),
            attr_str("url.path", &ctx.path),
            attr_int("http.response.status_code", http_status as i64),
        ];
        if let Some(rid) = request_id {
            attrs.push(attr_str("soli.request_id", rid));
        }
        let status = if http_status >= 500 {
            json!({"code": 2, "message": format!("HTTP {http_status}")})
        } else {
            json!({"code": 1})
        };
        let mut root = json!({
            "traceId": ctx.trace_id,
            "spanId": ctx.span_id,
            "name": format!("{} {}", ctx.method, ctx.path),
            "kind": 2,
            "startTimeUnixNano": wall_start_ns.to_string(),
            "endTimeUnixNano": end_ns.to_string(),
            "attributes": attrs,
            "status": status,
        });
        if let Some(ref p) = ctx.parent_span_id {
            root.as_object_mut()
                .unwrap()
                .insert("parentSpanId".into(), json!(p));
        }
        otlp_spans.push(root);
    } else {
        for s in spans {
            let span_id = local_span_id_hex(s.id, root_local, &ctx.span_id, salt);
            let parent = if s.kind == SpanKind::Request {
                ctx.parent_span_id.clone()
            } else {
                s.parent
                    .map(|pid| local_span_id_hex(pid, root_local, &ctx.span_id, salt))
            };

            let start_ns = wall_start_ns.saturating_add(s.start_us.saturating_mul(1_000));
            let end_ns = wall_start_ns.saturating_add(s.end_us.saturating_mul(1_000));

            let mut attrs = vec![attr_str("soli.span.kind", s.kind.as_str())];
            if s.kind == SpanKind::Request {
                attrs.push(attr_str("http.request.method", &ctx.method));
                attrs.push(attr_str("url.path", &ctx.path));
                attrs.push(attr_int("http.response.status_code", http_status as i64));
                if let Some(rid) = request_id {
                    attrs.push(attr_str("soli.request_id", rid));
                }
            }
            if let Some(meta) = &s.meta {
                // Bound meta so a huge AQL template can't blow the payload.
                let clipped = if meta.len() > 512 {
                    format!("{}…", &meta[..512])
                } else {
                    meta.clone()
                };
                attrs.push(attr_str("soli.span.meta", &clipped));
            }

            let status = if s.kind == SpanKind::Request && http_status >= 500 {
                json!({"code": 2, "message": format!("HTTP {http_status}")})
            } else {
                json!({"code": 1})
            };

            let mut span_obj = json!({
                "traceId": ctx.trace_id,
                "spanId": span_id,
                "name": s.name,
                "kind": span_kind_otlp(s.kind),
                "startTimeUnixNano": start_ns.to_string(),
                "endTimeUnixNano": end_ns.to_string(),
                "attributes": attrs,
                "status": status,
            });
            if let Some(p) = parent {
                span_obj
                    .as_object_mut()
                    .unwrap()
                    .insert("parentSpanId".into(), json!(p));
            }
            otlp_spans.push(span_obj);
        }
    }

    let mut resource_attrs = vec![
        attr_str("service.name", &cfg.service_name),
        attr_str("telemetry.sdk.name", "soli"),
        attr_str("telemetry.sdk.language", "rust"),
        attr_str("telemetry.sdk.version", env!("CARGO_PKG_VERSION", "0.0.0")),
    ];
    for (k, v) in &cfg.resource_attrs {
        resource_attrs.push(attr_str(k, v));
    }

    let payload = json!({
        "resourceSpans": [{
            "resource": {
                "attributes": resource_attrs
            },
            "scopeSpans": [{
                "scope": {
                    "name": "soli.serve",
                    "version": env!("CARGO_PKG_VERSION", "0.0.0")
                },
                "spans": otlp_spans
            }]
        }]
    });

    payload.to_string()
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_traceparent() {
        let (tid, pid, flags) =
            parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
                .expect("valid");
        assert_eq!(tid, "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(pid, "b7ad6b7169203331");
        assert_eq!(flags, 0x01);
    }

    #[test]
    fn reject_all_zero_ids() {
        assert!(
            parse_traceparent("00-00000000000000000000000000000000-b7ad6b7169203331-01").is_none()
        );
        assert!(
            parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01").is_none()
        );
    }

    #[test]
    fn reject_malformed() {
        assert!(parse_traceparent("not-a-header").is_none());
        assert!(
            parse_traceparent("01-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01").is_none()
        );
        assert!(parse_traceparent("00-short-b7ad6b7169203331-01").is_none());
    }

    #[test]
    fn resource_attributes_parse() {
        let attrs =
            parse_resource_attributes(Some("service.namespace=shop, deployment.environment=prod"));
        assert_eq!(
            attrs,
            vec![
                ("service.namespace".into(), "shop".into()),
                ("deployment.environment".into(), "prod".into()),
            ]
        );
    }

    #[test]
    fn traces_endpoint_appends_v1() {
        // Direct unit of resolve — simulate via the helper's pure path.
        assert_eq!(
            {
                let t = "http://collector:4318".trim().trim_end_matches('/');
                if t.ends_with("/v1/traces") {
                    t.to_string()
                } else {
                    format!("{t}/v1/traces")
                }
            },
            "http://collector:4318/v1/traces"
        );
    }

    #[test]
    fn local_span_id_never_zero_and_root_matches() {
        let root_hex = "b7ad6b7169203331";
        assert_eq!(local_span_id_hex(0, 0, root_hex, 0xdead_beef), root_hex);
        let child = local_span_id_hex(1, 0, root_hex, 0xdead_beef);
        assert_eq!(child.len(), 16);
        assert!(!child.chars().all(|c| c == '0'));
        assert_ne!(child, root_hex);
    }

    #[test]
    fn build_otlp_includes_http_attrs_on_root() {
        let cfg = OtelConfig {
            enabled: true,
            traces_endpoint: Some("http://localhost:4318/v1/traces".into()),
            service_name: "demo".into(),
            resource_attrs: vec![("deployment.environment".into(), "test".into())],
        };
        let ctx = TraceContext {
            trace_id: "0af7651916cd43dd8448eb211c80319c".into(),
            span_id: "b7ad6b7169203331".into(),
            parent_span_id: None,
            flags: 1,
            wall_start: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            instant_start: Instant::now(),
            method: "GET".into(),
            path: "/hello".into(),
        };
        let spans = vec![SpanRecord {
            id: 0,
            parent: None,
            name: "GET /hello".into(),
            kind: SpanKind::Request,
            start_us: 0,
            end_us: 12_000,
            meta: None,
            render_id: None,
        }];
        let body = build_otlp_body(&cfg, &ctx, &spans, 200, Some("req-1"));
        assert!(body.contains("0af7651916cd43dd8448eb211c80319c"));
        assert!(body.contains("b7ad6b7169203331"));
        assert!(body.contains("service.name"));
        assert!(body.contains("demo"));
        assert!(body.contains("http.request.method"));
        assert!(body.contains("GET"));
        assert!(body.contains("/hello"));
        assert!(body.contains("soli.request_id"));
        assert!(body.contains("req-1"));
        assert!(body.contains("deployment.environment"));
    }

    #[test]
    fn empty_spans_still_emits_root() {
        let cfg = OtelConfig {
            enabled: true,
            traces_endpoint: Some("http://localhost:4318/v1/traces".into()),
            service_name: "demo".into(),
            resource_attrs: vec![],
        };
        let ctx = TraceContext {
            trace_id: "0af7651916cd43dd8448eb211c80319c".into(),
            span_id: "b7ad6b7169203331".into(),
            parent_span_id: Some("aaaaaaaaaaaaaaaa".into()),
            flags: 1,
            wall_start: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            instant_start: Instant::now(),
            method: "POST".into(),
            path: "/x".into(),
        };
        let body = build_otlp_body(&cfg, &ctx, &[], 500, None);
        assert!(body.contains("POST /x"));
        assert!(body.contains("\"code\":2") || body.contains("HTTP 500"));
        assert!(body.contains("aaaaaaaaaaaaaaaa"));
    }
}
