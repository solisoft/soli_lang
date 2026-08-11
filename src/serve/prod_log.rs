//! Production log channels.
//!
//! The rich per-request diagnostics — the AQL query log, the outgoing
//! HTTP log, and the middleware/view/phase timing breakdown — are
//! otherwise hard-gated to `--dev`, where they feed the injected dev
//! bar. That left operators with no way to see them in production
//! without redeploying in dev mode (which also disables the VM, enables
//! hot-reload, and injects the bar — none of which you want in prod).
//!
//! This module reads a single `SOLI_LOG` env var once at startup and
//! decides which channels are active. When a channel is on, the worker
//! flips the matching `*_log::set_enabled` gate even in production, and
//! [`emit`] prints the buffered snapshot as an indented block under the
//! per-request access line.
//!
//! `SOLI_LOG` is a comma-separated list of channel names:
//!
//! ```text
//! SOLI_LOG=access            # per-request METHOD PATH - status (ms)
//! SOLI_LOG=query             # AQL queries with binds + duration
//! SOLI_LOG=http              # outgoing HTTP.* calls
//! SOLI_LOG=kv                # SoliKV / Cache (KV.* / Cache.*) commands
//! SOLI_LOG=timing            # middleware / view / phase timings
//! SOLI_LOG=query,http,timing # any combination
//! SOLI_LOG=all               # everything
//! ```
//!
//! Turning on any of `query`/`http`/`kv`/`timing` implies `access` so the
//! detail block has a request line to anchor to. The legacy
//! `SOLI_REQUEST_LOG=1` still works as an alias for the `access` channel.
//!
//! ## Log format (`SOLI_LOG_FORMAT`)
//!
//! Default is human-readable multi-line text. Set `SOLI_LOG_FORMAT=json`
//! for one NDJSON object per request (machine-parseable: ship to Loki,
//! CloudWatch, Datadog, …). Detail channels become nested arrays on the
//! same object. Errors go through the same switch in
//! [`super::error_logging`].
//!
//! ## Slow-request mode (`SOLI_SLOW_REQUEST_MS`)
//!
//! `SOLI_LOG=all` prints a block for *every* request — too noisy to leave
//! on in production. `SOLI_SLOW_REQUEST_MS=<ms>` instead emits the full
//! detail block (all channels, plus the queue-wait split) only for
//! requests whose total time (queue wait + handler) crosses the
//! threshold. It composes with `SOLI_LOG`: explicitly requested channels
//! still print for every request; the threshold only adds the `[SLOW]`
//! full-detail block on top. With `SOLI_SLOW_REQUEST_MS` alone, fast
//! requests log nothing at all.

use std::sync::OnceLock;

/// Output shape for production request (and error) logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// Multi-line human text (historical default).
    #[default]
    Text,
    /// One JSON object per line (NDJSON).
    Json,
}

/// Process-wide log format, parsed once from `SOLI_LOG_FORMAT`.
pub fn format() -> LogFormat {
    static FMT: OnceLock<LogFormat> = OnceLock::new();
    *FMT.get_or_init(|| {
        match std::env::var("SOLI_LOG_FORMAT")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("json") | Some("ndjson") => LogFormat::Json,
            Some("text") | Some("plain") | Some("") | None => LogFormat::Text,
            Some(other) => {
                eprintln!(
                    "[WARN] SOLI_LOG_FORMAT: unknown value '{}' (use text|json); defaulting to text",
                    other
                );
                LogFormat::Text
            }
        }
    })
}

/// Optional correlation fields for a single emit.
#[derive(Clone, Copy, Default)]
pub struct EmitMeta<'a> {
    pub request_id: Option<&'a str>,
    pub trace_id: Option<&'a str>,
    pub span_id: Option<&'a str>,
}

/// Fields shared by the text and JSON emit paths.
struct EmitRequest<'a> {
    method: &'a str,
    path: &'a str,
    status: u16,
    elapsed_ms: f64,
    queue_ms: Option<f64>,
    total_ms: f64,
    slow_hit: bool,
    ch: LogChannels,
    meta: EmitMeta<'a>,
}

#[derive(Clone, Copy, Default)]
pub struct LogChannels {
    /// Per-request access line: `[LOG] METHOD PATH - status (ms)`.
    pub access: bool,
    /// AQL query log (binds + duration).
    pub query: bool,
    /// Outgoing HTTP.* call log.
    pub http: bool,
    /// SoliKV / Cache (`KV.*` / `Cache.*`) command log.
    pub kv: bool,
    /// Middleware / view / phase timing breakdown.
    pub timing: bool,
    /// `SOLI_SLOW_REQUEST_MS`: when set, a request whose total time
    /// (queue wait + handler) reaches this many ms emits the full detail
    /// block regardless of which channels were requested. The four bools
    /// above stay "what SOLI_LOG asked for" — collection gating uses the
    /// `collect_*` helpers so slow mode records detail without printing
    /// it for fast requests.
    pub slow_ms: Option<f64>,
}

impl LogChannels {
    /// True if any channel is on. Used to decide whether the per-request
    /// timer and the thread-local buffer clearing are worth their cost.
    #[inline]
    pub fn any(&self) -> bool {
        self.access || self.query || self.http || self.kv || self.timing || self.slow_ms.is_some()
    }

    /// True if any *detail* channel (beyond the bare access line) is on.
    #[inline]
    pub fn has_detail(&self) -> bool {
        self.query || self.http || self.kv || self.timing || self.slow_ms.is_some()
    }

    /// Should the per-query AQL log collect? (Printed per request only if
    /// `query` was requested; slow mode prints it just for slow requests.)
    #[inline]
    pub fn collect_query(&self) -> bool {
        self.query || self.slow_ms.is_some()
    }

    /// Should the outgoing-HTTP log collect?
    #[inline]
    pub fn collect_http(&self) -> bool {
        self.http || self.slow_ms.is_some()
    }

    /// Should the SoliKV / Cache command log collect?
    #[inline]
    pub fn collect_kv(&self) -> bool {
        self.kv || self.slow_ms.is_some()
    }

    /// Should the middleware/view/phase timers collect?
    #[inline]
    pub fn collect_timing(&self) -> bool {
        self.timing || self.slow_ms.is_some()
    }
}

fn parse(soli_log: Option<&str>, request_log: bool, slow_ms: Option<f64>) -> LogChannels {
    let mut ch = LogChannels {
        slow_ms: slow_ms.filter(|&v| v > 0.0),
        ..LogChannels::default()
    };

    if let Some(raw) = soli_log {
        for token in raw.split(',') {
            match token.trim().to_ascii_lowercase().as_str() {
                "" => {}
                "all" | "1" | "true" => {
                    ch.access = true;
                    ch.query = true;
                    ch.http = true;
                    ch.kv = true;
                    ch.timing = true;
                }
                "access" | "request" | "requests" => ch.access = true,
                "query" | "queries" | "db" | "sql" | "aql" => ch.query = true,
                "http" => ch.http = true,
                "kv" | "cache" | "solikv" => ch.kv = true,
                "timing" | "timings" | "phase" | "phases" => ch.timing = true,
                other => {
                    eprintln!("[WARN] SOLI_LOG: unknown channel '{}' (ignored)", other);
                }
            }
        }
    }

    // Legacy alias: SOLI_REQUEST_LOG=1 enables the access channel.
    if request_log {
        ch.access = true;
    }

    // A detail channel without the access line would print orphaned
    // blocks with no request to anchor them — fold access in. Only the
    // explicit SOLI_LOG channels count here: slow mode alone must NOT
    // turn on the per-request access line (it prints nothing until a
    // request crosses the threshold).
    if ch.query || ch.http || ch.kv || ch.timing {
        ch.access = true;
    }

    ch
}

/// Process-wide channel set, parsed once from the environment.
pub fn channels() -> LogChannels {
    static CHANNELS: OnceLock<LogChannels> = OnceLock::new();
    *CHANNELS.get_or_init(|| {
        let soli_log = std::env::var("SOLI_LOG").ok();
        let request_log = std::env::var("SOLI_REQUEST_LOG")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);
        let slow_ms = std::env::var("SOLI_SLOW_REQUEST_MS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok());
        parse(soli_log.as_deref(), request_log, slow_ms)
    })
}

/// Render query bind variables for a log line, with secret-bearing ones
/// redacted.
///
/// `SOLI_LOG=query` is a **production** channel — the documentation offers it
/// as a way to get dev-grade diagnostics "without paying for full dev mode" —
/// and bind variables are where a query's *values* live. A login is
/// `FILTER u.email == @email AND u.password_digest == @password`, so printing
/// binds verbatim wrote the submitted password to the production log.
///
/// Uses the same rule as the error log's request snapshot and environment dump
/// (`crate::redaction`), so a value cannot be redacted in one log line and
/// printed in the next.
fn render_binds(binds: &std::collections::HashMap<String, serde_json::Value>) -> String {
    let safe: std::collections::BTreeMap<&str, serde_json::Value> = binds
        .iter()
        .map(|(k, v)| {
            let value = if crate::redaction::looks_sensitive(k) {
                serde_json::Value::String(crate::redaction::REDACTED.to_string())
            } else {
                v.clone()
            };
            (k.as_str(), value)
        })
        .collect();
    serde_json::to_string(&safe).unwrap_or_else(|_| "{}".to_string())
}

/// Collapse an AQL query to a single line so it stays one log entry.
fn one_line(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Print the per-request access line plus any enabled detail sections as
/// one `println!` (so worker threads can't interleave the block).
///
/// `dev_mode` callers already inject the dev bar from the same snapshots;
/// this is the production path, gated on the `SOLI_LOG` channels.
///
/// `queue_ms` is the time the request waited in the worker queue before a
/// worker picked it up (None when the enqueue timestamp wasn't captured).
///
/// The slow threshold compares against queue + handler so a request stuck
/// behind a busy worker is caught even when the handler itself was fast.
pub fn emit(
    method: &str,
    path: &str,
    status: u16,
    elapsed_ms: f64,
    queue_ms: Option<f64>,
    ch: LogChannels,
) {
    emit_with_meta(
        method,
        path,
        status,
        elapsed_ms,
        queue_ms,
        ch,
        EmitMeta::default(),
    );
}

/// Like [`emit`], with optional `request_id` / trace correlation fields
/// (populated when OpenTelemetry is on or a request id was minted).
pub fn emit_with_meta(
    method: &str,
    path: &str,
    status: u16,
    elapsed_ms: f64,
    queue_ms: Option<f64>,
    ch: LogChannels,
    meta: EmitMeta<'_>,
) {
    let total_ms = elapsed_ms + queue_ms.unwrap_or(0.0);
    let slow_hit = ch.slow_ms.is_some_and(|t| total_ms >= t);
    if !slow_hit && !ch.access {
        return;
    }

    // A slow hit prints every detail section regardless of which channels
    // SOLI_LOG asked for — the whole point is a full picture of where the
    // time went, without the operator having to turn on per-request noise.
    let ch = if slow_hit {
        LogChannels {
            access: true,
            query: true,
            http: true,
            kv: true,
            timing: true,
            slow_ms: ch.slow_ms,
        }
    } else {
        ch
    };

    let req = EmitRequest {
        method,
        path,
        status,
        elapsed_ms,
        queue_ms,
        total_ms,
        slow_hit,
        ch,
        meta,
    };
    match format() {
        LogFormat::Json => emit_json(req),
        LogFormat::Text => emit_text(req),
    }
}

fn emit_text(req: EmitRequest<'_>) {
    use std::fmt::Write;

    let mut out = String::with_capacity(256);
    let _ = write!(
        out,
        "[{}] {} {} - {} ({:.3}ms",
        if req.slow_hit { "SLOW" } else { "LOG" },
        req.method,
        req.path,
        req.status,
        req.elapsed_ms
    );
    match req.queue_ms {
        Some(q) => {
            let _ = write!(out, " + {:.3}ms queue)", q);
        }
        None => out.push(')'),
    }
    if let Some(rid) = req.meta.request_id {
        let _ = write!(out, " request_id={}", rid);
    }
    if let Some(tid) = req.meta.trace_id {
        let _ = write!(out, " trace_id={}", tid);
    }

    if req.ch.query {
        let queries = crate::interpreter::builtins::model::query_log::snapshot();
        if !queries.is_empty() {
            let total: f64 = queries.iter().map(|q| q.duration_ms).sum();
            let _ = write!(
                out,
                "\n  db: {} quer{} ({:.3}ms)",
                queries.len(),
                if queries.len() == 1 { "y" } else { "ies" },
                total
            );
            for q in &queries {
                let _ = write!(out, "\n    ({:.3}ms) {}", q.duration_ms, one_line(&q.query));
                if let Some(binds) = &q.bind_vars {
                    if !binds.is_empty() {
                        let _ = write!(out, " binds={}", render_binds(binds));
                    }
                }
            }
        }
    }

    if req.ch.http {
        let calls = crate::interpreter::builtins::http_log::snapshot();
        if !calls.is_empty() {
            let total: f64 = calls.iter().map(|c| c.duration_ms).sum();
            let _ = write!(
                out,
                "\n  http: {} call{} ({:.3}ms)",
                calls.len(),
                if calls.len() == 1 { "" } else { "s" },
                total
            );
            for call in &calls {
                let _ = write!(
                    out,
                    "\n    ({:.3}ms) {} {} -> {}",
                    call.duration_ms,
                    call.method,
                    crate::redaction::redact_url_query(&call.url),
                    call.status
                );
                if let Some(err) = &call.error {
                    let _ = write!(out, " [error: {}]", err);
                }
            }
        }
    }

    if req.ch.kv {
        let calls = crate::interpreter::builtins::kv_log::snapshot();
        if !calls.is_empty() {
            let total: f64 = calls.iter().map(|c| c.duration_ms).sum();
            let _ = write!(
                out,
                "\n  kv: {} call{} ({:.3}ms)",
                calls.len(),
                if calls.len() == 1 { "" } else { "s" },
                total
            );
            for call in &calls {
                let _ = write!(
                    out,
                    "\n    ({:.3}ms) {} {}",
                    call.duration_ms, call.command, call.key
                );
                if let Some(err) = &call.error {
                    let _ = write!(out, " [error: {}]", err);
                }
            }
        }
    }

    if req.ch.timing {
        let middlewares = crate::serve::middleware_log::snapshot();
        let views = crate::serve::view_log::snapshot();
        let phases = crate::serve::phase_log::snapshot();

        if !middlewares.is_empty() || !views.is_empty() || !phases.is_empty() {
            let _ = write!(out, "\n  timing:");
            for (name, dur_us) in &phases {
                let _ = write!(
                    out,
                    "\n    phase {} ({:.3}ms)",
                    name,
                    *dur_us as f64 / 1000.0
                );
            }
            for (name, dur_us) in &middlewares {
                let _ = write!(
                    out,
                    "\n    middleware {} ({:.3}ms)",
                    name,
                    *dur_us as f64 / 1000.0
                );
            }
            for (_id, parent, name, dur_us) in &views {
                // Indent nested partials one extra step so the render
                // tree is readable.
                let extra = if parent.is_some() { "  " } else { "" };
                let _ = write!(
                    out,
                    "\n    {}view {} ({:.3}ms)",
                    extra,
                    name,
                    *dur_us as f64 / 1000.0
                );
            }
        }
    }

    println!("{}", out);
}

fn emit_json(req: EmitRequest<'_>) {
    use serde_json::{json, Map, Value};

    let mut obj = Map::new();
    obj.insert("ts".into(), json!(chrono_like_ts()));
    obj.insert(
        "level".into(),
        json!(if req.slow_hit {
            "warn"
        } else if req.status >= 500 {
            "error"
        } else if req.status >= 400 {
            "warn"
        } else {
            "info"
        }),
    );
    obj.insert(
        "msg".into(),
        json!(if req.slow_hit {
            "slow_request"
        } else {
            "request"
        }),
    );
    obj.insert("method".into(), json!(req.method));
    obj.insert("path".into(), json!(req.path));
    obj.insert("status".into(), json!(req.status));
    obj.insert("duration_ms".into(), json!(round3(req.elapsed_ms)));
    if let Some(q) = req.queue_ms {
        obj.insert("queue_ms".into(), json!(round3(q)));
    }
    obj.insert("total_ms".into(), json!(round3(req.total_ms)));
    if req.slow_hit {
        obj.insert("slow".into(), json!(true));
    }
    if let Some(rid) = req.meta.request_id {
        obj.insert("request_id".into(), json!(rid));
    }
    if let Some(tid) = req.meta.trace_id {
        obj.insert("trace_id".into(), json!(tid));
    }
    if let Some(sid) = req.meta.span_id {
        obj.insert("span_id".into(), json!(sid));
    }

    if req.ch.query {
        let queries = crate::interpreter::builtins::model::query_log::snapshot();
        if !queries.is_empty() {
            let arr: Vec<Value> = queries
                .iter()
                .map(|q| {
                    let mut m = Map::new();
                    m.insert("duration_ms".into(), json!(round3(q.duration_ms)));
                    m.insert("query".into(), json!(one_line(&q.query)));
                    if let Some(binds) = &q.bind_vars {
                        if !binds.is_empty() {
                            m.insert("binds".into(), redacted_binds_value(binds));
                        }
                    }
                    Value::Object(m)
                })
                .collect();
            obj.insert("db".into(), Value::Array(arr));
        }
    }

    if req.ch.http {
        let calls = crate::interpreter::builtins::http_log::snapshot();
        if !calls.is_empty() {
            let arr: Vec<Value> = calls
                .iter()
                .map(|c| {
                    let mut m = Map::new();
                    m.insert("duration_ms".into(), json!(round3(c.duration_ms)));
                    m.insert("method".into(), json!(c.method));
                    m.insert(
                        "url".into(),
                        json!(crate::redaction::redact_url_query(&c.url)),
                    );
                    m.insert("status".into(), json!(c.status));
                    if let Some(err) = &c.error {
                        m.insert("error".into(), json!(err));
                    }
                    Value::Object(m)
                })
                .collect();
            obj.insert("http".into(), Value::Array(arr));
        }
    }

    if req.ch.kv {
        let calls = crate::interpreter::builtins::kv_log::snapshot();
        if !calls.is_empty() {
            let arr: Vec<Value> = calls
                .iter()
                .map(|c| {
                    let mut m = Map::new();
                    m.insert("duration_ms".into(), json!(round3(c.duration_ms)));
                    m.insert("command".into(), json!(c.command));
                    m.insert("key".into(), json!(c.key));
                    if let Some(err) = &c.error {
                        m.insert("error".into(), json!(err));
                    }
                    Value::Object(m)
                })
                .collect();
            obj.insert("kv".into(), Value::Array(arr));
        }
    }

    if req.ch.timing {
        let middlewares = crate::serve::middleware_log::snapshot();
        let views = crate::serve::view_log::snapshot();
        let phases = crate::serve::phase_log::snapshot();
        if !middlewares.is_empty() || !views.is_empty() || !phases.is_empty() {
            let mut timing = Map::new();
            if !phases.is_empty() {
                timing.insert(
                    "phases".into(),
                    Value::Array(
                        phases
                            .iter()
                            .map(|(name, us)| {
                                json!({"name": name, "duration_ms": round3(*us as f64 / 1000.0)})
                            })
                            .collect(),
                    ),
                );
            }
            if !middlewares.is_empty() {
                timing.insert(
                    "middleware".into(),
                    Value::Array(
                        middlewares
                            .iter()
                            .map(|(name, us)| {
                                json!({"name": name, "duration_ms": round3(*us as f64 / 1000.0)})
                            })
                            .collect(),
                    ),
                );
            }
            if !views.is_empty() {
                timing.insert(
                    "views".into(),
                    Value::Array(
                        views
                            .iter()
                            .map(|(_id, parent, name, us)| {
                                json!({
                                    "name": name,
                                    "duration_ms": round3(*us as f64 / 1000.0),
                                    "nested": parent.is_some(),
                                })
                            })
                            .collect(),
                    ),
                );
            }
            obj.insert("timing".into(), Value::Object(timing));
        }
    }

    println!("{}", Value::Object(obj));
}

fn redacted_binds_value(
    binds: &std::collections::HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    let safe: serde_json::Map<String, serde_json::Value> = binds
        .iter()
        .map(|(k, v)| {
            let value = if crate::redaction::looks_sensitive(k) {
                serde_json::Value::String(crate::redaction::REDACTED.to_string())
            } else {
                v.clone()
            };
            (k.clone(), value)
        })
        .collect();
    serde_json::Value::Object(safe)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// RFC3339-ish UTC timestamp without pulling chrono — good enough for log shippers.
/// Public so error logging can share the same clock format.
pub(crate) fn chrono_like_ts_public() -> String {
    chrono_like_ts()
}

fn chrono_like_ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let millis = dur.subsec_millis();
    // Manual UTC breakdown (no leap-second handling — fine for logs).
    let (y, mo, d, h, mi, s) = civil_from_days(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{millis:03}Z")
}

/// Convert unix seconds to (year, month, day, hour, min, sec) UTC.
fn civil_from_days(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let h = tod / 3600;
    let mi = (tod % 3600) / 60;
    let s = tod % 60;

    // Howard Hinnant civil_from_days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_channels() {
        let ch = parse(Some("query,http"), false, None);
        assert!(ch.query && ch.http && ch.access);
        assert!(!ch.timing);
    }

    #[test]
    fn all_enables_everything() {
        let ch = parse(Some("all"), false, None);
        assert!(ch.access && ch.query && ch.http && ch.timing);
    }

    #[test]
    fn detail_channel_implies_access() {
        let ch = parse(Some("timing"), false, None);
        assert!(ch.access && ch.timing);
    }

    #[test]
    fn request_log_alias_enables_access_only() {
        let ch = parse(None, true, None);
        assert!(ch.access);
        assert!(!ch.has_detail());
    }

    /// `SOLI_LOG=query` is a production channel, and bind variables are where a
    /// query's values live — a login binds the submitted password. Verbatim
    /// rendering wrote it to the log.
    #[test]
    fn bind_rendering_redacts_secret_names() {
        use std::collections::HashMap;
        let mut binds: HashMap<String, serde_json::Value> = HashMap::new();
        binds.insert("email".into(), serde_json::json!("ada@example.com"));
        binds.insert("password".into(), serde_json::json!("hunter2"));
        binds.insert("api_key".into(), serde_json::json!("ak_live_123"));
        binds.insert("limit".into(), serde_json::json!(10));

        let out = super::render_binds(&binds);

        assert!(!out.contains("hunter2"), "password leaked: {out}");
        assert!(!out.contains("ak_live_123"), "api key leaked: {out}");
        // …and the rest is still useful for debugging.
        assert!(
            out.contains("ada@example.com"),
            "email should survive: {out}"
        );
        assert!(out.contains("10"), "limit should survive: {out}");
        assert!(out.contains("[REDACTED]"), "expected the marker: {out}");
    }

    #[test]
    fn empty_env_is_all_off() {
        let ch = parse(None, false, None);
        assert!(!ch.any());
    }

    #[test]
    fn aliases_resolve() {
        let ch = parse(Some("queries, db , phases"), false, None);
        assert!(ch.query && ch.timing);
    }

    #[test]
    fn slow_mode_alone_collects_but_prints_nothing_for_fast_requests() {
        let ch = parse(None, false, Some(400.0));
        // Collection must be on (the slow block needs the buffers filled)…
        assert!(ch.any() && ch.has_detail());
        assert!(ch.collect_query() && ch.collect_http() && ch.collect_timing());
        // …but no per-request output channel was requested.
        assert!(!ch.access && !ch.query && !ch.http && !ch.timing);
        assert_eq!(ch.slow_ms, Some(400.0));
    }

    #[test]
    fn slow_mode_composes_with_explicit_channels() {
        let ch = parse(Some("access"), false, Some(100.0));
        assert!(ch.access);
        assert!(!ch.query && !ch.http && !ch.timing);
        assert!(ch.collect_query(), "slow mode still collects detail");
        assert_eq!(ch.slow_ms, Some(100.0));
    }

    #[test]
    fn zero_or_negative_slow_threshold_is_ignored() {
        assert_eq!(parse(None, false, Some(0.0)).slow_ms, None);
        assert_eq!(parse(None, false, Some(-5.0)).slow_ms, None);
    }

    #[test]
    fn civil_from_days_epoch() {
        // 1970-01-01T00:00:00Z
        assert_eq!(civil_from_days(0), (1970, 1, 1, 0, 0, 0));
        // 1970-01-01T00:00:01Z
        assert_eq!(civil_from_days(1), (1970, 1, 1, 0, 0, 1));
        // 2024-01-01T00:00:00Z = 1704067200
        assert_eq!(civil_from_days(1_704_067_200), (2024, 1, 1, 0, 0, 0));
    }

    #[test]
    fn chrono_like_ts_is_rfc3339ish() {
        let ts = chrono_like_ts();
        // 2026-08-09T12:00:00.123Z
        assert!(ts.ends_with('Z'), "{ts}");
        assert_eq!(ts.len(), 24, "{ts}");
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[10..11], "T");
    }
}
