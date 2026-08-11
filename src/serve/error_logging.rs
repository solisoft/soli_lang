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
pub(super) fn log_production_error(
    request_id: &str,
    request_data: &RequestData,
    error_msg: &str,
    stack_trace: &[String],
    env_json: Option<&str>,
) {
    let snapshot = redacted_request_snapshot(request_data, /* redact_body = */ true);

    match log_format() {
        LogFormat::Json => {
            let env_value: serde_json::Value = env_json
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::Value::Null);
            let obj = serde_json::json!({
                "ts": super::prod_log::chrono_like_ts_public(),
                "level": "error",
                "msg": "request_error",
                "request_id": request_id,
                "method": request_data.method.as_ref(),
                "path": request_data.path,
                "error": error_msg,
                "stack": stack_trace,
                "request": snapshot,
                "env": env_value,
            });
            eprintln!("{}", obj);
        }
        LogFormat::Text => {
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
    }
}
