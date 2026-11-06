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
//!    so request latency is not blocked on the collector (drops on full queue),
//!    with `OTEL_EXPORTER_OTLP_{,TRACES_,LOGS_}HEADERS` on every POST.
//! 4. **Exceptions** — an uncaught request error ([`record_exception`]) marks
//!    the root span ERROR with an OTel `exception` event.
//! 5. **Logs** (opt-in, `SOLI_OTEL_LOGS=1` / `OTEL_LOGS_EXPORTER=otlp`) — the
//!    JSON log events as OTLP LogRecords ([`export_log`]), batched on their
//!    own thread.
//! 6. **Datadog** — `DD_SERVICE` / `DD_ENV` / `DD_VERSION` as fallbacks for the
//!    service identity ([`identity`]) and [`datadog_id`] for `dd.trace_id`.
//!
//! No new crates: JSON via `serde_json`, HTTP via `ureq`, IDs via `uuid`.

use super::span_log::{SpanKind, SpanRecord};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::OnceLock;
use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Max in-flight export batches. Past this, new batches are dropped (never
/// block a worker on a slow collector).
const EXPORT_QUEUE_CAP: usize = 256;

/// Default OTLP HTTP base when `SOLI_OTEL=1` but no endpoint is configured.
const DEFAULT_OTLP_BASE: &str = "http://127.0.0.1:4318";

/// Soft timeout for a single OTLP POST.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(2);

// ---------- config ----------

/// Unified service tagging: what Datadog calls `service` / `env` /
/// `version`, OpenTelemetry `service.name` / `deployment.environment` /
/// `service.version`.
///
/// Resolved independently of whether tracing is on, so JSON logs carry the
/// tags even when nothing is exported. Each field is `None` unless the
/// operator configured it — a log line tagged `service: "soli"` would
/// override the service the Datadog agent derives from container labels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceIdentity {
    pub service: Option<String>,
    pub env: Option<String>,
    pub version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct OtelConfig {
    pub enabled: bool,
    /// Full URL for OTLP/HTTP traces (…/v1/traces). `None` → context only
    /// (traceparent + log correlation), no network export.
    pub traces_endpoint: Option<String>,
    pub service_name: String,
    /// Extra resource attributes from `OTEL_RESOURCE_ATTRIBUTES` (minus
    /// `service.name`, which lives in `service_name`), plus the
    /// environment / version filled in from `DD_ENV` / `DD_VERSION`.
    pub resource_attrs: Vec<(String, String)>,
    /// `service` / `env` / `version` for log correlation.
    pub identity: ServiceIdentity,
    /// Headers sent on every traces POST (`OTEL_EXPORTER_OTLP_HEADERS`
    /// overridden by `OTEL_EXPORTER_OTLP_TRACES_HEADERS`). Values are
    /// credentials (`dd-api-key`): never print them.
    pub traces_headers: Vec<(String, String)>,
    /// Full URL for OTLP/HTTP logs (…/v1/logs). `Some` only when log export
    /// was asked for (`SOLI_OTEL_LOGS=1` / `OTEL_LOGS_EXPORTER=otlp`).
    pub logs_endpoint: Option<String>,
    /// Headers sent on every logs POST (generic + `…_LOGS_HEADERS`).
    pub logs_headers: Vec<(String, String)>,
    /// Per-POST timeout (`OTEL_EXPORTER_OTLP_TIMEOUT`, milliseconds).
    pub export_timeout: Duration,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            traces_endpoint: None,
            service_name: "soli".into(),
            resource_attrs: Vec::new(),
            identity: ServiceIdentity::default(),
            traces_headers: Vec::new(),
            logs_endpoint: None,
            logs_headers: Vec::new(),
            export_timeout: EXPORT_TIMEOUT,
        }
    }
}

impl OtelConfig {
    fn from_env() -> Self {
        let (cfg, warnings) = Self::from_lookup(&|name| crate::platform::env::var(name).ok());
        for w in warnings {
            eprintln!("[WARN] {w}");
        }
        cfg
    }

    /// Resolve the config from `get` (the environment in production, a map
    /// in tests). Returns the warnings to print rather than printing them,
    /// so tests can assert on them — and so a warning can be checked for
    /// never containing a header value.
    fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let var = |name: &str| -> Option<String> {
            get(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let truthy = |name: &str| {
            var(name).is_some_and(|v| {
                v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
            })
        };

        // Service identity first: JSON logs want it even when OTEL is off.
        let mut resource_attrs =
            parse_resource_attributes(var("OTEL_RESOURCE_ATTRIBUTES").as_deref());
        let take_attr = |attrs: &[(String, String)], key: &str| {
            attrs
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .filter(|v| !v.is_empty())
        };
        // OTEL_SERVICE_NAME > OTEL_RESOURCE_ATTRIBUTES service.name > DD_SERVICE.
        let service = var("OTEL_SERVICE_NAME")
            .or_else(|| take_attr(&resource_attrs, "service.name"))
            .or_else(|| var("DD_SERVICE"));
        resource_attrs.retain(|(k, _)| k != "service.name");
        let attr_env = take_attr(&resource_attrs, "deployment.environment.name")
            .or_else(|| take_attr(&resource_attrs, "deployment.environment"));
        let env = attr_env.clone().or_else(|| var("DD_ENV"));
        if attr_env.is_none() {
            if let Some(e) = &env {
                // Both keys: Datadog reads the newer `.name`, older collectors
                // and backends the bare one.
                resource_attrs.push(("deployment.environment.name".into(), e.clone()));
                resource_attrs.push(("deployment.environment".into(), e.clone()));
            }
        }
        let attr_version = take_attr(&resource_attrs, "service.version");
        let version = attr_version.clone().or_else(|| var("DD_VERSION"));
        if attr_version.is_none() {
            if let Some(v) = &version {
                resource_attrs.push(("service.version".into(), v.clone()));
            }
        }
        let identity = ServiceIdentity {
            service: service.clone(),
            env,
            version,
        };
        let service_name = service.unwrap_or_else(|| "soli".to_string());

        if truthy("OTEL_SDK_DISABLED") {
            return (
                Self {
                    service_name,
                    resource_attrs,
                    identity,
                    ..Self::default()
                },
                warnings,
            );
        }

        let base = var("OTEL_EXPORTER_OTLP_ENDPOINT").map(|b| b.trim_end_matches('/').to_string());
        let traces_endpoint = var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
            .or_else(|| base.as_deref().map(|b| signal_url(b, "traces")));
        let forced_on = truthy("SOLI_OTEL");
        let enabled = forced_on || traces_endpoint.is_some();

        // SOLI_OTEL=1 with no endpoint still enables context + span collection;
        // export uses the local default collector so a sidecar Just Works.
        let traces_endpoint = if enabled && traces_endpoint.is_none() && forced_on {
            Some(signal_url(DEFAULT_OTLP_BASE, "traces"))
        } else {
            traces_endpoint
        };

        let logs_wanted = truthy("SOLI_OTEL_LOGS")
            || var("OTEL_LOGS_EXPORTER")
                .is_some_and(|v| v.split(',').any(|e| e.trim().eq_ignore_ascii_case("otlp")));
        let logs_endpoint = logs_wanted.then(|| {
            var("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT")
                .unwrap_or_else(|| signal_url(base.as_deref().unwrap_or(DEFAULT_OTLP_BASE), "logs"))
        });

        let generic = parse_headers(
            var("OTEL_EXPORTER_OTLP_HEADERS").as_deref(),
            "OTEL_EXPORTER_OTLP_HEADERS",
            &mut warnings,
        );
        let traces_headers = merge_headers(
            &generic,
            parse_headers(
                var("OTEL_EXPORTER_OTLP_TRACES_HEADERS").as_deref(),
                "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
                &mut warnings,
            ),
        );
        let logs_headers = merge_headers(
            &generic,
            parse_headers(
                var("OTEL_EXPORTER_OTLP_LOGS_HEADERS").as_deref(),
                "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
                &mut warnings,
            ),
        );

        let export_timeout = match var("OTEL_EXPORTER_OTLP_TIMEOUT") {
            None => EXPORT_TIMEOUT,
            Some(raw) => match raw.parse::<u64>() {
                Ok(ms) if ms > 0 => Duration::from_millis(ms),
                _ => {
                    warnings.push(format!(
                        "OTEL_EXPORTER_OTLP_TIMEOUT: expected a positive number of \
                         milliseconds, got '{raw}'; using {}ms",
                        EXPORT_TIMEOUT.as_millis()
                    ));
                    EXPORT_TIMEOUT
                }
            },
        };

        (
            Self {
                enabled,
                traces_endpoint,
                service_name,
                resource_attrs,
                identity,
                traces_headers,
                logs_endpoint,
                logs_headers,
                export_timeout,
            },
            warnings,
        )
    }
}

/// `{base}/v1/{signal}` — unless the base already ends in that path.
fn signal_url(base: &str, signal: &str) -> String {
    let base = base.trim_end_matches('/');
    let suffix = format!("/v1/{signal}");
    if base.ends_with(&suffix) {
        base.to_string()
    } else {
        format!("{base}{suffix}")
    }
}

/// Percent-decode a value of the W3C-baggage-style lists the OTel spec uses
/// for `OTEL_EXPORTER_OTLP_HEADERS` and `OTEL_RESOURCE_ATTRIBUTES`. An
/// invalid `%` sequence stays literal rather than failing the whole value.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// RFC 7230 `token` — what a header name may be made of.
fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Parse `key1=value1,key2=value2` (the OTel exporter-headers format).
///
/// Values are percent-decoded. A pair without `=`, with an invalid header
/// name, or whose decoded value holds CR / LF / NUL (header injection) is
/// skipped with a warning. Warnings name the variable and the key, never
/// the value: these headers carry API keys.
fn parse_headers(
    raw: Option<&str>,
    var_name: &str,
    warnings: &mut Vec<String>,
) -> Vec<(String, String)> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (idx, pair) in raw.split(',').enumerate() {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((k, v)) = pair.split_once('=') else {
            warnings.push(format!(
                "{var_name}: entry #{} has no '=' (ignored)",
                idx + 1
            ));
            continue;
        };
        let key = percent_decode(k.trim());
        if !is_header_name(&key) {
            warnings.push(format!(
                "{var_name}: entry #{} has an invalid header name (ignored)",
                idx + 1
            ));
            continue;
        }
        let value = percent_decode(v.trim());
        if value.contains(['\r', '\n', '\0']) {
            warnings.push(format!(
                "{var_name}: value of '{key}' contains a line break (ignored)"
            ));
            continue;
        }
        out.push((key, value));
    }
    out
}

/// The generic headers, with the signal-specific ones replacing any of the
/// same name (case-insensitive) — the OTel spec's precedence.
fn merge_headers(
    generic: &[(String, String)],
    specific: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = generic
        .iter()
        .filter(|(k, _)| !specific.iter().any(|(s, _)| s.eq_ignore_ascii_case(k)))
        .cloned()
        .collect();
    out.extend(specific);
    out
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
            Some((percent_decode(k), percent_decode(v)))
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

/// Per-trace salt for [`local_span_id_hex`], from the root span id.
fn span_salt(root_span_id: &str) -> u64 {
    u64::from_str_radix(&root_span_id[..16.min(root_span_id.len())], 16).unwrap_or(1)
}

/// The exported id of a non-root span_log span, known before the span is
/// recorded — what an outgoing `traceparent` names as its parent.
pub(super) fn child_span_hex(root_span_id: &str, local_id: u32) -> String {
    local_span_id_hex(local_id, u32::MAX, root_span_id, span_salt(root_span_id))
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
    let (trace_id, parent_span_id, inbound_flags) = inbound_traceparent
        .and_then(parse_traceparent)
        .map(|(tid, pid, f)| (tid, Some(pid), Some(f)))
        .unwrap_or_else(|| (random_trace_id(), None, None));
    // Head sampling (`OTEL_TRACES_SAMPLER`): an unsampled trace keeps its ids
    // and propagates with flags 00, but is not exported.
    let sampled = super::otel_sampler::sampler()
        .should_sample(&trace_id, inbound_flags.map(|f| f & 0x01 == 0x01));
    let flags = (inbound_flags.unwrap_or(0) & !0x01) | u8::from(sampled);

    let ctx = TraceContext {
        trace_id,
        span_id: random_span_id(),
        parent_span_id,
        flags,
        wall_start: SystemTime::now(),
        instant_start: Instant::now(),
        method: method.to_string(),
        path: path.to_string(),
    };
    CURRENT_IDS.with(|c| *c.borrow_mut() = Some((ctx.trace_id.clone(), ctx.span_id.clone())));
    PENDING_EXCEPTION.with(|p| *p.borrow_mut() = None);
    Some(ctx)
}

crate::request_local! {
    /// `(trace_id, span_id)` of the request being served, for log lines
    /// written away from the `Finalizer` (the production error block).
    static CURRENT_IDS: std::cell::RefCell<Option<(String, String)>> =
        const { std::cell::RefCell::new(None) };
    /// The uncaught error the request ended with, for the root span.
    static PENDING_EXCEPTION: std::cell::RefCell<Option<ExceptionInfo>> =
        const { std::cell::RefCell::new(None) };
}

/// `(trace_id, span_id)` (hex) of the request this worker is serving, when
/// tracing is on.
pub fn current_ids() -> Option<(String, String)> {
    if !enabled() {
        return None;
    }
    CURRENT_IDS.with(|c| c.borrow().clone())
}

// ---------- exceptions ----------

/// An uncaught error, in the shape of the OTel `exception` span event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionInfo {
    /// `exception.type` — the Soli error class (`RuntimeError`, `TypeError`,
    /// an app's own `PaymentError`, …).
    pub exception_type: String,
    /// `exception.message`.
    pub message: String,
    /// `exception.stacktrace` — the production error log's frames, one per
    /// line, paths relative to the app root.
    pub stacktrace: String,
}

/// Longest `exception.stacktrace` / `exception.message` exported.
const MAX_EXCEPTION_CHARS: usize = 8 * 1024;

fn clip_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &s[..cut]),
        None => s.to_string(),
    }
}

impl ExceptionInfo {
    /// Build from what the production error log has: the message and its
    /// stack frames. `app_root` is stripped from the frames, as
    /// `/__soli/errors` does, so a deploy path never reaches the backend.
    pub fn new(exception_type: &str, message: &str, stack: &[String], app_root: &str) -> Self {
        let frames: Vec<String> = stack
            .iter()
            .map(|f| {
                if app_root.is_empty() {
                    f.clone()
                } else {
                    f.replace(app_root, "")
                }
            })
            .collect();
        Self {
            exception_type: exception_type.to_string(),
            message: clip_chars(message, MAX_EXCEPTION_CHARS),
            stacktrace: clip_chars(&frames.join("\n"), MAX_EXCEPTION_CHARS),
        }
    }

    /// The OTel `exception` span event (semantic conventions), which
    /// Datadog maps to `error.type` / `error.message` / `error.stack`.
    pub fn event(&self, time_unix_nano: u64) -> Value {
        json!({
            "timeUnixNano": time_unix_nano.to_string(),
            "name": "exception",
            "attributes": [
                attr_str("exception.type", &self.exception_type),
                attr_str("exception.message", &self.message),
                attr_str("exception.stacktrace", &self.stacktrace),
            ],
        })
    }

    /// Mark an OTLP span (JSON) as failed by this exception: status ERROR
    /// with the message, an `error.type` attribute and, when `with_event`,
    /// the `exception` event. Shared with any other span builder (jobs).
    pub fn apply(&self, span: &mut Value, end_unix_nano: u64, with_event: bool) {
        let Some(obj) = span.as_object_mut() else {
            return;
        };
        obj.insert("status".into(), json!({"code": 2, "message": self.message}));
        if let Some(Value::Array(attrs)) = obj.get_mut("attributes") {
            attrs.push(attr_str("error.type", &self.exception_type));
        }
        if with_event {
            let events = obj
                .entry("events")
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(list) = events {
                list.push(self.event(end_unix_nano));
            }
        }
    }
}

/// Remember the uncaught error this request ended with; the root span is
/// marked with it when the request's trace is exported. Called by the
/// production error log, so deliberate outcomes (a `find` miss's 404,
/// `forbidden()`, `halt`, a `DomainError`) — which never reach that log —
/// are never exceptions here either.
pub fn record_exception(exception_type: &str, message: &str, stack: &[String], app_root: &str) {
    if !enabled() {
        return;
    }
    let info = ExceptionInfo::new(exception_type, message, stack, app_root);
    PENDING_EXCEPTION.with(|p| *p.borrow_mut() = Some(info));
}

fn take_exception() -> Option<ExceptionInfo> {
    PENDING_EXCEPTION.with(|p| p.borrow_mut().take())
}

// ---------- export ----------

/// Which OTLP signal a POST carries — picks its endpoint and headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Signal {
    Traces,
    Logs,
}

struct ExportBatch {
    signal: Signal,
    body: String,
}

static EXPORTER: OnceLock<Option<SyncSender<ExportBatch>>> = OnceLock::new();
static EXPORT_DROPPED: AtomicBool = AtomicBool::new(false);

/// POST one batch with its signal's endpoint and headers; warn once per
/// process on the first failure. The warning carries the transport error,
/// never the headers.
fn send_batch(batch: &ExportBatch) {
    let cfg = config();
    let (endpoint, headers) = match batch.signal {
        Signal::Traces => (cfg.traces_endpoint.as_deref(), &cfg.traces_headers),
        Signal::Logs => (cfg.logs_endpoint.as_deref(), &cfg.logs_headers),
    };
    let Some(endpoint) = endpoint else {
        return;
    };
    if let Err(e) = post_otlp(endpoint, &batch.body, headers, cfg.export_timeout) {
        // Rate-limit noise: one warn per process lifetime for the first
        // failure, then silence.
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            eprintln!("[WARN] OTEL export failed (further failures suppressed): {e}");
        }
    }
}

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
                        send_batch(&batch);
                    }
                })
                .ok();
            Some(tx)
        })
        .as_ref()
}

#[cfg(not(target_arch = "wasm32"))]
fn post_otlp(
    endpoint: &str,
    body: &str,
    headers: &[(String, String)],
    timeout: Duration,
) -> Result<(), String> {
    // ureq's default features include rustls with the webpki roots, so an
    // `https://` intake (Datadog agentless, a hosted collector) works as is.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_millis(500).min(timeout))
        .timeout(timeout)
        .build();
    let mut req = agent.post(endpoint).set("Content-Type", "application/json");
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let resp = match req.send_string(body) {
        Ok(resp) => resp,
        // Report the status, not ureq's Display: keep the message short and
        // free of anything the request carried.
        Err(ureq::Error::Status(code, _)) => return Err(format!("HTTP {code} from {endpoint}")),
        Err(e) => return Err(e.to_string()),
    };
    let status = resp.status();
    // 2xx and 429 (backpressure) are acceptable; 4xx/5xx otherwise warn once.
    if (200..300).contains(&status) || status == 429 {
        Ok(())
    } else {
        Err(format!("HTTP {status} from {endpoint}"))
    }
}
#[cfg(target_arch = "wasm32")]
fn post_otlp(
    _endpoint: &str,
    _body: &str,
    _headers: &[(String, String)],
    _timeout: Duration,
) -> Result<(), String> {
    Err(crate::platform::unsupported_on_edge("post_otlp"))
}

/// Convert span_log records into an OTLP payload and enqueue export.
///
/// `status_code` is the HTTP status of the request (sets span status ERROR
/// for 5xx). An uncaught error recorded by [`record_exception`] during the
/// request marks the root span (and the action that raised) with an
/// `exception` event. No-op when tracing is off or no endpoint is configured.
pub fn export_request(
    ctx: &TraceContext,
    spans: &[SpanRecord],
    http_status: u16,
    request_id: Option<&str>,
) {
    // Taken even when the trace is not sampled, so it never leaks into
    // the next request this worker serves.
    let exception = take_exception();
    // Not sampled (`OTEL_TRACES_SAMPLER`): ids and propagation only.
    if ctx.flags & 0x01 == 0 {
        return;
    }
    let cfg = config();
    if cfg.traces_endpoint.is_none() || exporter().is_none() {
        return;
    }
    enqueue_export(build_otlp_body_with_exception(
        cfg,
        ctx,
        spans,
        http_status,
        request_id,
        exception.as_ref(),
    ));
}

/// Queue an OTLP/JSON traces body for the export thread (sent with the
/// traces endpoint, headers and timeout). Drops (with one warning) when the
/// queue is full rather than block the caller.
pub(super) fn enqueue_export(body: String) {
    if config().traces_endpoint.is_none() {
        return;
    }
    let Some(tx) = exporter() else {
        return;
    };
    match tx.try_send(ExportBatch {
        signal: Signal::Traces,
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

// ---------- logs ----------

/// Max log records waiting for the logs exporter. Past this, records are
/// dropped (one warning) rather than blocking a worker.
const LOGS_QUEUE_CAP: usize = 4096;
/// Records per OTLP logs POST.
const LOGS_BATCH_MAX: usize = 512;
/// A partial batch is flushed after this long.
const LOGS_FLUSH_EVERY: Duration = Duration::from_secs(1);

static LOGS_EXPORTER: OnceLock<Option<SyncSender<Value>>> = OnceLock::new();
static LOGS_DROPPED: AtomicBool = AtomicBool::new(false);

/// Whether log records are exported over OTLP (`SOLI_OTEL_LOGS=1` or
/// `OTEL_LOGS_EXPORTER=otlp`).
#[inline]
pub fn logs_enabled() -> bool {
    config().logs_endpoint.is_some()
}

fn logs_exporter() -> Option<&'static SyncSender<Value>> {
    LOGS_EXPORTER
        .get_or_init(|| {
            config().logs_endpoint.as_ref()?;
            let (tx, rx) = sync_channel::<Value>(LOGS_QUEUE_CAP);
            std::thread::Builder::new()
                .name("soli-otel-logs".into())
                .spawn(move || {
                    use std::sync::mpsc::RecvTimeoutError;
                    let mut pending: Vec<Value> = Vec::new();
                    let mut oldest: Option<Instant> = None;
                    loop {
                        let wait = oldest
                            .map(|t| LOGS_FLUSH_EVERY.saturating_sub(t.elapsed()))
                            .unwrap_or(LOGS_FLUSH_EVERY);
                        let closed = match rx.recv_timeout(wait) {
                            Ok(record) => {
                                oldest.get_or_insert_with(Instant::now);
                                pending.push(record);
                                false
                            }
                            Err(RecvTimeoutError::Timeout) => false,
                            Err(RecvTimeoutError::Disconnected) => true,
                        };
                        let due = oldest.is_some_and(|t| t.elapsed() >= LOGS_FLUSH_EVERY);
                        if !pending.is_empty() && (closed || due || pending.len() >= LOGS_BATCH_MAX)
                        {
                            let records = std::mem::take(&mut pending);
                            oldest = None;
                            send_batch(&ExportBatch {
                                signal: Signal::Logs,
                                body: build_otlp_logs_body(config(), records),
                            });
                        }
                        if closed {
                            break;
                        }
                    }
                })
                .ok();
            Some(tx)
        })
        .as_ref()
}

/// Export one JSON log line (the object Soli prints under
/// `SOLI_LOG_FORMAT=json`) as an OTLP LogRecord. No-op unless log export is
/// on. Never blocks: a full queue drops the record with a one-time warning.
pub fn export_log(line: &serde_json::Map<String, Value>) {
    if !logs_enabled() {
        return;
    }
    let Some(tx) = logs_exporter() else {
        return;
    };
    let record = log_record(line, unix_nanos(SystemTime::now()));
    match tx.try_send(record) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            if !LOGS_DROPPED.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[WARN] OTEL logs queue full ({LOGS_QUEUE_CAP} records); dropping log \
                     records (further drops suppressed)"
                );
            }
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}

/// OTLP `SeverityNumber` for a JSON line's `level`.
fn severity_number(level: &str) -> i32 {
    match level {
        "trace" => 1,
        "debug" => 5,
        "info" => 9,
        "warn" | "warning" => 13,
        "error" => 17,
        "fatal" => 21,
        _ => 0,
    }
}

/// A JSON value as an OTLP `AnyValue`.
fn any_value(v: &Value) -> Value {
    match v {
        Value::Null => json!({}),
        Value::Bool(b) => json!({"boolValue": b}),
        Value::Number(n) => match n.as_i64() {
            Some(i) => json!({"intValue": i.to_string()}),
            None => json!({"doubleValue": n.as_f64().unwrap_or(0.0)}),
        },
        Value::String(s) => json!({"stringValue": s}),
        Value::Array(items) => {
            json!({"arrayValue": {"values": items.iter().map(any_value).collect::<Vec<_>>()}})
        }
        Value::Object(map) => json!({"kvlistValue": {"values": map
            .iter()
            .map(|(k, v)| json!({"key": k, "value": any_value(v)}))
            .collect::<Vec<_>>()}}),
    }
}

/// Keys of a JSON log line that become LogRecord fields (or the resource),
/// not attributes.
const LOG_LINE_RESERVED: &[&str] = &[
    "ts", "level", "msg", "trace_id", "span_id", "dd", "service", "env", "version",
];

/// One OTLP LogRecord from a JSON log line: `msg` is the body, `level` the
/// severity, `trace_id` / `span_id` the correlation, the rest attributes.
/// `service` / `env` / `version` are left to the resource.
fn log_record(line: &serde_json::Map<String, Value>, now_ns: u64) -> Value {
    let level = line.get("level").and_then(Value::as_str).unwrap_or("info");
    let mut record = serde_json::Map::new();
    record.insert("timeUnixNano".into(), json!(now_ns.to_string()));
    record.insert("observedTimeUnixNano".into(), json!(now_ns.to_string()));
    record.insert("severityNumber".into(), json!(severity_number(level)));
    record.insert("severityText".into(), json!(level.to_ascii_uppercase()));
    record.insert(
        "body".into(),
        json!({"stringValue": line.get("msg").and_then(Value::as_str).unwrap_or("")}),
    );
    let attrs: Vec<Value> = line
        .iter()
        .filter(|(k, _)| !LOG_LINE_RESERVED.contains(&k.as_str()))
        .map(|(k, v)| json!({"key": k, "value": any_value(v)}))
        .collect();
    record.insert("attributes".into(), Value::Array(attrs));
    if let Some(tid) = line.get("trace_id").and_then(Value::as_str) {
        record.insert("traceId".into(), json!(tid));
        record.insert("flags".into(), json!(1));
    }
    if let Some(sid) = line.get("span_id").and_then(Value::as_str) {
        record.insert("spanId".into(), json!(sid));
    }
    Value::Object(record)
}

fn build_otlp_logs_body(cfg: &OtelConfig, records: Vec<Value>) -> String {
    json!({
        "resourceLogs": [{
            "resource": {"attributes": resource_attributes(cfg)},
            "scopeLogs": [{
                "scope": {
                    "name": "soli.serve",
                    "version": env!("CARGO_PKG_VERSION", "0.0.0")
                },
                "logRecords": records
            }]
        }]
    })
    .to_string()
}

/// The resource every signal shares: `service.name`, the SDK identity, and
/// the configured / Datadog-derived attributes.
fn resource_attributes(cfg: &OtelConfig) -> Vec<Value> {
    let mut attrs = vec![
        attr_str("service.name", &cfg.service_name),
        attr_str("telemetry.sdk.name", "soli"),
        attr_str("telemetry.sdk.language", "rust"),
        attr_str("telemetry.sdk.version", env!("CARGO_PKG_VERSION", "0.0.0")),
    ];
    for (k, v) in &cfg.resource_attrs {
        attrs.push(attr_str(k, v));
    }
    attrs
}

// ---------- Datadog correlation ----------

/// Datadog's id for an OTel hex id: the lower 64 bits, as an unsigned
/// decimal string (what `dd.trace_id` / `dd.span_id` hold in logs). `None`
/// when `hex` is not hex.
pub fn datadog_id(hex: &str) -> Option<String> {
    if !hex.is_ascii() {
        return None;
    }
    let low = &hex[hex.len().saturating_sub(16)..];
    if low.is_empty() || !low.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(low, 16).ok().map(|n| n.to_string())
}

/// The service identity for log lines — resolved even when tracing is off.
pub fn identity() -> &'static ServiceIdentity {
    &config().identity
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

/// The OTLP/JSON body for a request (or job) trace with no exception.
pub(super) fn build_otlp_body(
    cfg: &OtelConfig,
    ctx: &TraceContext,
    spans: &[SpanRecord],
    http_status: u16,
    request_id: Option<&str>,
) -> String {
    build_otlp_body_with_exception(cfg, ctx, spans, http_status, request_id, None)
}

/// The span of the request the error unwound out of: among the handler
/// spans (middleware, before/after action, action), the one that ended
/// last. `None` when the request never reached one.
fn raising_span(spans: &[SpanRecord]) -> Option<u32> {
    spans
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                SpanKind::Middleware
                    | SpanKind::BeforeAction
                    | SpanKind::Action
                    | SpanKind::AfterAction
            )
        })
        .max_by_key(|s| s.end_us)
        .map(|s| s.id)
}

fn build_otlp_body_with_exception(
    cfg: &OtelConfig,
    ctx: &TraceContext,
    spans: &[SpanRecord],
    http_status: u16,
    request_id: Option<&str>,
    exception: Option<&ExceptionInfo>,
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
    let salt = span_salt(&ctx.span_id);

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
        if let Some(exc) = exception {
            exc.apply(&mut root, end_ns, true);
        }
        otlp_spans.push(root);
    } else {
        let raising = exception.and_then(|_| raising_span(spans));
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
                    // Back off to a char boundary — a byte slice mid-UTF-8 panics.
                    let mut cut = 512;
                    while !meta.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    format!("{}…", &meta[..cut])
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
            if let Some(exc) = exception {
                // The event on the SERVER root (what Datadog Error Tracking
                // reads); ERROR status on the action that raised as well.
                if s.kind == SpanKind::Request {
                    exc.apply(&mut span_obj, end_ns, true);
                } else if raising == Some(s.id) {
                    exc.apply(&mut span_obj, end_ns, false);
                }
            }
            otlp_spans.push(span_obj);
        }
    }

    let resource_attrs = resource_attributes(cfg);

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
    fn a_child_span_id_is_known_before_export() {
        let root = "b7ad6b7169203331";
        let salt = span_salt(root);
        for local in [1u32, 2, 7, 400] {
            assert_eq!(
                child_span_hex(root, local),
                local_span_id_hex(local, 0, root, salt)
            );
        }
    }

    #[test]
    fn build_otlp_includes_http_attrs_on_root() {
        let cfg = OtelConfig {
            enabled: true,
            traces_endpoint: Some("http://localhost:4318/v1/traces".into()),
            service_name: "demo".into(),
            resource_attrs: vec![("deployment.environment".into(), "test".into())],
            ..OtelConfig::default()
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
            ..OtelConfig::default()
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

    // ---------- Datadog / headers / exceptions / logs ----------

    use std::collections::HashMap;

    fn cfg_from(pairs: &[(&str, &str)]) -> (OtelConfig, Vec<String>) {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        OtelConfig::from_lookup(&|name| map.get(name).cloned())
    }

    fn attr<'a>(cfg: &'a OtelConfig, key: &str) -> Option<&'a str> {
        cfg.resource_attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn datadog_id_is_lower_64_bits_in_decimal() {
        assert_eq!(
            datadog_id("00000000000000000000000000000001").as_deref(),
            Some("1")
        );
        // High 64 bits are ignored.
        assert_eq!(
            datadog_id("ffffffffffffffff0000000000000002").as_deref(),
            Some("2")
        );
        assert_eq!(
            datadog_id("0000000000000000ffffffffffffffff").as_deref(),
            Some("18446744073709551615")
        );
        // A 16-hex span id converts whole.
        assert_eq!(
            datadog_id("b7ad6b7169203331").as_deref(),
            Some("13235353014750950193")
        );
        assert_eq!(datadog_id("not-hex-at-all!!"), None);
        assert_eq!(datadog_id(""), None);
        assert_eq!(datadog_id("é0000000000000001"), None);
    }

    #[test]
    fn identity_is_unset_without_configuration() {
        let (cfg, warnings) = cfg_from(&[]);
        assert!(warnings.is_empty());
        assert_eq!(cfg.identity, ServiceIdentity::default());
        assert_eq!(cfg.service_name, "soli");
        assert!(!cfg.enabled);
        assert!(cfg.logs_endpoint.is_none());
    }

    #[test]
    fn dd_vars_fill_service_env_version() {
        let (cfg, _) = cfg_from(&[
            ("DD_SERVICE", "shop"),
            ("DD_ENV", "prod"),
            ("DD_VERSION", "1.4.2"),
        ]);
        assert_eq!(cfg.service_name, "shop");
        assert_eq!(cfg.identity.service.as_deref(), Some("shop"));
        assert_eq!(cfg.identity.env.as_deref(), Some("prod"));
        assert_eq!(cfg.identity.version.as_deref(), Some("1.4.2"));
        assert_eq!(attr(&cfg, "deployment.environment.name"), Some("prod"));
        assert_eq!(attr(&cfg, "deployment.environment"), Some("prod"));
        assert_eq!(attr(&cfg, "service.version"), Some("1.4.2"));
        // service.name is never duplicated into the extra attributes.
        assert_eq!(attr(&cfg, "service.name"), None);
    }

    #[test]
    fn otel_vars_win_over_dd_vars() {
        let (cfg, _) = cfg_from(&[
            ("OTEL_SERVICE_NAME", "checkout"),
            ("DD_SERVICE", "shop"),
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "service.name=ignored,deployment.environment=staging,service.version=2.0",
            ),
            ("DD_ENV", "prod"),
            ("DD_VERSION", "1.4.2"),
        ]);
        assert_eq!(cfg.service_name, "checkout");
        assert_eq!(cfg.identity.env.as_deref(), Some("staging"));
        assert_eq!(cfg.identity.version.as_deref(), Some("2.0"));
        // The operator's own attributes are kept as written, not doubled.
        assert_eq!(attr(&cfg, "deployment.environment"), Some("staging"));
        assert_eq!(attr(&cfg, "deployment.environment.name"), None);
        assert_eq!(
            cfg.resource_attrs
                .iter()
                .filter(|(k, _)| k == "service.version")
                .count(),
            1
        );
    }

    #[test]
    fn resource_service_name_wins_over_dd_service() {
        let (cfg, _) = cfg_from(&[
            ("OTEL_RESOURCE_ATTRIBUTES", "service.name=billing"),
            ("DD_SERVICE", "shop"),
        ]);
        assert_eq!(cfg.service_name, "billing");
    }

    #[test]
    fn sdk_disabled_keeps_identity_for_logs() {
        let (cfg, _) = cfg_from(&[
            ("OTEL_SDK_DISABLED", "true"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://c:4318"),
            ("SOLI_OTEL_LOGS", "1"),
            ("DD_ENV", "prod"),
        ]);
        assert!(!cfg.enabled);
        assert!(cfg.traces_endpoint.is_none());
        assert!(cfg.logs_endpoint.is_none());
        assert_eq!(cfg.identity.env.as_deref(), Some("prod"));
    }

    #[test]
    fn headers_parse_with_percent_decoding() {
        let mut w = Vec::new();
        let h = parse_headers(
            Some("dd-api-key=abc%20123 , x-scope=a%3Db%2Cc,empty="),
            "OTEL_EXPORTER_OTLP_HEADERS",
            &mut w,
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(
            h,
            vec![
                ("dd-api-key".into(), "abc 123".into()),
                ("x-scope".into(), "a=b,c".into()),
                ("empty".into(), "".into()),
            ]
        );
    }

    #[test]
    fn invalid_percent_sequences_stay_literal() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
        assert_eq!(percent_decode("%41%"), "A%");
    }

    #[test]
    fn malformed_header_pairs_are_skipped_with_a_warning() {
        let mut w = Vec::new();
        let h = parse_headers(
            Some("novalue,=nokey,bad name=x,ok=1"),
            "OTEL_EXPORTER_OTLP_HEADERS",
            &mut w,
        );
        assert_eq!(h, vec![("ok".into(), "1".into())]);
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w
            .iter()
            .all(|m| m.starts_with("OTEL_EXPORTER_OTLP_HEADERS")));
    }

    #[test]
    fn header_values_with_line_breaks_are_rejected_without_echoing_them() {
        let mut w = Vec::new();
        let h = parse_headers(
            Some("dd-api-key=secret%0D%0AX-Evil: 1,raw=a\nb,ok=1"),
            "OTEL_EXPORTER_OTLP_HEADERS",
            &mut w,
        );
        assert_eq!(h, vec![("ok".into(), "1".into())]);
        assert_eq!(w.len(), 2, "{w:?}");
        for msg in &w {
            assert!(!msg.contains("secret"), "value leaked: {msg}");
            assert!(!msg.contains("Evil"), "value leaked: {msg}");
        }
    }

    #[test]
    fn signal_headers_override_generic_ones() {
        let (cfg, _) = cfg_from(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://otlp.example.com"),
            ("OTEL_EXPORTER_OTLP_HEADERS", "dd-api-key=generic,x-a=1"),
            ("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "DD-API-KEY=traces"),
            ("OTEL_EXPORTER_OTLP_LOGS_HEADERS", "x-b=2"),
            ("OTEL_LOGS_EXPORTER", "otlp"),
        ]);
        assert_eq!(
            cfg.traces_headers,
            vec![
                ("x-a".into(), "1".into()),
                ("DD-API-KEY".into(), "traces".into())
            ]
        );
        assert_eq!(
            cfg.logs_headers,
            vec![
                ("dd-api-key".into(), "generic".into()),
                ("x-a".into(), "1".into()),
                ("x-b".into(), "2".into())
            ]
        );
        assert_eq!(
            cfg.traces_endpoint.as_deref(),
            Some("https://otlp.example.com/v1/traces")
        );
        assert_eq!(
            cfg.logs_endpoint.as_deref(),
            Some("https://otlp.example.com/v1/logs")
        );
    }

    #[test]
    fn logs_endpoint_resolution() {
        // Not asked for: no logs export even with an endpoint.
        let (cfg, _) = cfg_from(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://c:4318/")]);
        assert!(cfg.logs_endpoint.is_none());
        assert_eq!(
            cfg.traces_endpoint.as_deref(),
            Some("http://c:4318/v1/traces")
        );
        // Explicit logs endpoint wins.
        let (cfg, _) = cfg_from(&[
            ("SOLI_OTEL_LOGS", "1"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://c:4318"),
            ("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", "http://l:9999/ingest"),
        ]);
        assert_eq!(cfg.logs_endpoint.as_deref(), Some("http://l:9999/ingest"));
        // No endpoint at all: the local default collector.
        let (cfg, _) = cfg_from(&[("OTEL_LOGS_EXPORTER", "console,otlp")]);
        assert_eq!(
            cfg.logs_endpoint.as_deref(),
            Some("http://127.0.0.1:4318/v1/logs")
        );
        let (cfg, _) = cfg_from(&[("OTEL_LOGS_EXPORTER", "none")]);
        assert!(cfg.logs_endpoint.is_none());
    }

    #[test]
    fn export_timeout_from_env() {
        let (cfg, w) = cfg_from(&[("OTEL_EXPORTER_OTLP_TIMEOUT", "5000")]);
        assert!(w.is_empty());
        assert_eq!(cfg.export_timeout, Duration::from_millis(5000));
        let (cfg, w) = cfg_from(&[("OTEL_EXPORTER_OTLP_TIMEOUT", "soon")]);
        assert_eq!(w.len(), 1);
        assert_eq!(cfg.export_timeout, EXPORT_TIMEOUT);
    }

    #[test]
    fn resource_attributes_are_percent_decoded() {
        let attrs = parse_resource_attributes(Some("team=a%2Cb,owner=x%20y"));
        assert_eq!(
            attrs,
            vec![
                ("team".into(), "a,b".into()),
                ("owner".into(), "x y".into())
            ]
        );
    }

    fn test_ctx() -> TraceContext {
        TraceContext {
            trace_id: "0af7651916cd43dd8448eb211c80319c".into(),
            span_id: "b7ad6b7169203331".into(),
            parent_span_id: None,
            flags: 1,
            wall_start: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            instant_start: Instant::now(),
            method: "GET".into(),
            path: "/boom".into(),
        }
    }

    fn span(id: u32, parent: Option<u32>, kind: SpanKind, name: &str, end_us: u64) -> SpanRecord {
        SpanRecord {
            id,
            parent,
            name: name.into(),
            kind,
            start_us: 0,
            end_us,
            meta: None,
            render_id: None,
        }
    }

    #[test]
    fn exception_marks_root_with_event_and_raising_action_with_status() {
        let exc = ExceptionInfo::new(
            "PaymentError",
            "card declined at 12:5",
            &[
                "PaymentsController#create at /srv/app/app/controllers/payments_controller.sl:12"
                    .to_string(),
            ],
            "/srv/app/",
        );
        assert_eq!(
            exc.stacktrace,
            "PaymentsController#create at app/controllers/payments_controller.sl:12"
        );
        let spans = vec![
            span(1, Some(0), SpanKind::Middleware, "auth", 1_000),
            span(2, Some(0), SpanKind::Action, "payments#create", 9_000),
            span(3, Some(2), SpanKind::Db, "query", 5_000),
            span(0, None, SpanKind::Request, "GET /boom", 10_000),
        ];
        let body = build_otlp_body_with_exception(
            &OtelConfig::default(),
            &test_ctx(),
            &spans,
            500,
            None,
            Some(&exc),
        );
        let v: Value = serde_json::from_str(&body).unwrap();
        let out = v["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .unwrap();
        let by_name = |n: &str| out.iter().find(|s| s["name"] == n).unwrap();

        let root = by_name("GET /boom");
        assert_eq!(root["kind"], 2);
        assert_eq!(root["status"]["code"], 2);
        assert_eq!(root["status"]["message"], "card declined at 12:5");
        let events = root["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["name"], "exception");
        let ev_attr = |k: &str| {
            events[0]["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["key"] == k)
                .map(|a| a["value"]["stringValue"].as_str().unwrap().to_string())
        };
        assert_eq!(ev_attr("exception.type").as_deref(), Some("PaymentError"));
        assert_eq!(
            ev_attr("exception.message").as_deref(),
            Some("card declined at 12:5")
        );
        assert!(ev_attr("exception.stacktrace")
            .unwrap()
            .contains("payments_controller.sl:12"));
        // Event time = root end.
        assert_eq!(events[0]["timeUnixNano"], root["endTimeUnixNano"]);

        let action = by_name("payments#create");
        assert_eq!(action["status"]["code"], 2);
        assert!(action.get("events").is_none());
        // Neither the middleware that ran earlier nor the db span is marked.
        assert_eq!(by_name("auth")["status"]["code"], 1);
        assert_eq!(by_name("query")["status"]["code"], 1);
    }

    #[test]
    fn no_exception_leaves_spans_unmarked() {
        let spans = vec![span(0, None, SpanKind::Request, "GET /ok", 10)];
        let body = build_otlp_body_with_exception(
            &OtelConfig::default(),
            &test_ctx(),
            &spans,
            404,
            None,
            None,
        );
        assert!(!body.contains("\"events\""));
        assert!(!body.contains("exception"));
    }

    #[test]
    fn exception_on_empty_span_tree_still_reaches_the_root() {
        let exc = ExceptionInfo::new("RuntimeError", "boom", &[], "");
        let body = build_otlp_body_with_exception(
            &OtelConfig::default(),
            &test_ctx(),
            &[],
            500,
            None,
            Some(&exc),
        );
        assert!(body.contains("\"exception.type\""));
        assert!(body.contains("\"message\":\"boom\""));
    }

    #[test]
    fn exception_text_is_clipped() {
        let long = "x".repeat(MAX_EXCEPTION_CHARS + 10);
        let exc = ExceptionInfo::new("RuntimeError", &long, &[], "");
        assert_eq!(exc.message.chars().count(), MAX_EXCEPTION_CHARS + 1);
        assert!(exc.message.ends_with('…'));
    }

    #[test]
    fn log_record_shape() {
        let line: serde_json::Map<String, Value> = serde_json::from_value(json!({
            "ts": "2026-11-06T10:00:00.000Z",
            "level": "error",
            "msg": "request_error",
            "method": "GET",
            "status": 500,
            "duration_ms": 1.5,
            "trace_id": "0af7651916cd43dd8448eb211c80319c",
            "span_id": "b7ad6b7169203331",
            "service": "shop",
            "dd": {"trace_id": "1", "span_id": "2"},
            "stack": ["a", "b"],
            "request": {"path": "/x"},
        }))
        .unwrap();
        let rec = log_record(&line, 42);
        assert_eq!(rec["timeUnixNano"], "42");
        assert_eq!(rec["severityNumber"], 17);
        assert_eq!(rec["severityText"], "ERROR");
        assert_eq!(rec["body"]["stringValue"], "request_error");
        assert_eq!(rec["traceId"], "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(rec["spanId"], "b7ad6b7169203331");
        let attrs = rec["attributes"].as_array().unwrap();
        let get = |k: &str| {
            attrs
                .iter()
                .find(|a| a["key"] == k)
                .map(|a| a["value"].clone())
        };
        assert_eq!(get("method").unwrap()["stringValue"], "GET");
        assert_eq!(get("status").unwrap()["intValue"], "500");
        assert_eq!(get("duration_ms").unwrap()["doubleValue"], 1.5);
        assert_eq!(
            get("stack").unwrap()["arrayValue"]["values"][1]["stringValue"],
            "b"
        );
        assert_eq!(
            get("request").unwrap()["kvlistValue"]["values"][0]["key"],
            "path"
        );
        // Correlation and identity fields are not repeated as attributes.
        for k in ["ts", "level", "msg", "trace_id", "span_id", "dd", "service"] {
            assert!(get(k).is_none(), "{k} should not be an attribute");
        }
    }

    #[test]
    fn severity_mapping() {
        assert_eq!(severity_number("info"), 9);
        assert_eq!(severity_number("warn"), 13);
        assert_eq!(severity_number("error"), 17);
        assert_eq!(severity_number("whatever"), 0);
    }

    #[test]
    fn logs_body_shares_the_trace_resource() {
        let (cfg, _) = cfg_from(&[("DD_SERVICE", "shop"), ("DD_ENV", "prod")]);
        let body = build_otlp_logs_body(&cfg, vec![json!({"body": {"stringValue": "x"}})]);
        let v: Value = serde_json::from_str(&body).unwrap();
        let res = v["resourceLogs"][0]["resource"]["attributes"]
            .as_array()
            .unwrap();
        let get = |k: &str| {
            res.iter()
                .find(|a| a["key"] == k)
                .map(|a| a["value"]["stringValue"].as_str().unwrap().to_string())
        };
        assert_eq!(get("service.name").as_deref(), Some("shop"));
        assert_eq!(get("deployment.environment.name").as_deref(), Some("prod"));
        assert_eq!(
            v["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["body"]["stringValue"],
            "x"
        );
    }
}
