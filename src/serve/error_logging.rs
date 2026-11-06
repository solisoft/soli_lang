//! Production-mode error logging.
//!
//! In production mode the server used to log a single line per failed
//! request (`[ERROR] request_id=… METHOD PATH - msg`). That gave
//! operators no way to reproduce the failure path without redeploying
//! with `--dev`. This module writes a multi-line block that includes
//! the call stack, the redacted request snapshot, and the local
//! environment at the moment the request raised — the same context the
//! dev error page already surfaces.
//!
//! Each error is emitted via a single `eprintln!` so worker threads
//! don't interleave their output mid-block. When `SOLI_LOG_FORMAT=json`,
//! the same payload is one NDJSON object on stderr.
//!
//! Secret-bearing values (auth headers, password/token-like params,
//! request body) are redacted via [`super::error_pages::
//! redacted_request_snapshot`] before formatting.

use super::error_pages::redacted_request_snapshot;
use super::prod_log::{format as log_format, LogFormat};
use super::RequestData;

/// Write a full-context production error block to stderr.
///
/// The block preserves the original `[ERROR] request_id=… METHOD PATH -
/// msg` first line so log parsers keyed on that prefix keep working,
/// then appends indented `stack:`, `request:`, and `env:` sections.
/// JSON mode emits a single object with the same fields.
///
/// `error_type` is the Soli error class when the caller had the
/// `RuntimeError` in hand ([`crate::error::RuntimeError::exception_type`]);
/// `None` derives it from the message. It becomes `error_type` on the JSON
/// line and `exception.type` on the request's root span.
pub(super) fn log_production_error(
    request_id: &str,
    request_data: &RequestData,
    error_msg: &str,
    stack_trace: &[String],
    env_json: Option<&str>,
    error_type: Option<&str>,
) {
    // Grouped for `/__soli/errors`; queued, so it costs the request nothing.
    super::error_tracker::record(request_id, request_data, error_msg, stack_trace, env_json);

    let classified;
    let error_type = match error_type {
        Some(t) => t,
        None => {
            classified = crate::error::classify_error_message(error_msg);
            classified
        }
    };
    // The same errors `/__soli/errors` groups become the exception on the
    // request's trace (a deliberate 404/403/halt never gets here).
    if super::otel::enabled() {
        let root = format!("{}/", super::tenant::app_root().display());
        super::otel::record_exception(error_type, error_msg, stack_trace, &root);
    }

    let snapshot = redacted_request_snapshot(request_data, /* redact_body = */ true);
    let format = log_format();
    if format == LogFormat::Json || super::otel::logs_enabled() {
        let env_value: serde_json::Value = env_json
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null);
        let serde_json::Value::Object(mut obj) = serde_json::json!({
            "ts": super::prod_log::chrono_like_ts_public(),
            "level": "error",
            "msg": "request_error",
            "request_id": request_id,
            "method": request_data.method.as_ref(),
            "path": request_data.path,
            "error": error_msg,
            "error_type": error_type,
            "stack": stack_trace,
            "request": snapshot.clone(),
            "env": env_value,
        }) else {
            return;
        };
        let ids = super::otel::current_ids();
        if let Some((trace_id, span_id)) = &ids {
            obj.insert("trace_id".into(), serde_json::json!(trace_id));
            obj.insert("span_id".into(), serde_json::json!(span_id));
        }
        super::prod_log::add_correlation(
            &mut obj,
            ids.as_ref().map(|(t, _)| t.as_str()),
            ids.as_ref().map(|(_, s)| s.as_str()),
        );
        if format == LogFormat::Json {
            eprintln!("{}", serde_json::Value::Object(obj.clone()));
        }
        super::otel::export_log(&obj);
        if format == LogFormat::Json {
            return;
        }
    }

    let mut block = String::with_capacity(512);

    block.push_str(&format!(
        "[ERROR] request_id={} {} {} - {}\n",
        request_id,
        request_data.method.as_ref(),
        request_data.path,
        error_msg,
    ));

    block.push_str("  stack:\n");
    if stack_trace.is_empty() {
        block.push_str("    <no stack frames captured>\n");
    } else {
        for frame in stack_trace {
            block.push_str("    ");
            block.push_str(frame);
            block.push('\n');
        }
    }

    block.push_str("  request:\n");
    let snapshot_text =
        serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| "{}".to_string());
    for line in snapshot_text.lines() {
        block.push_str("    ");
        block.push_str(line);
        block.push('\n');
    }

    block.push_str("  env: ");
    match env_json {
        Some(env) if !env.is_empty() => block.push_str(env),
        _ => block.push_str("<no environment captured>"),
    }

    // Single eprintln! call so worker threads can't interleave the
    // multi-line block. Trailing newline is appended by eprintln!.
    eprintln!("{}", block);
}
