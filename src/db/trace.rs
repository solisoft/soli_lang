//! Query instrumentation for the SQL adapters.
//!
//! The dev bar, `dev_queries()`, the N+1 detector, `assert_no_n_plus_one`, and
//! `soli test --fail-on-n1` all read one source: the per-request query log. Only
//! the SoliDB path wrote to it, so every one of those tools was blind on
//! Postgres, MySQL, and SQLite — including the framework's own N+1 guard.
//!
//! [`start`] returns a guard that records on drop, so a call site is one line and
//! the timing covers the whole database round trip:
//!
//! ```ignore
//! let compiled = compile_select_d(Dialect::Postgres, q)?;
//! let _trace = trace::start(&compiled.sql, &compiled.params);
//! query_docs(&compiled.sql, &compiled.params)
//! ```
//!
//! The Prometheus DB-time counter is fed either way; the rich per-query log stays
//! gated to `--dev`, matching the SoliDB path.

use std::collections::HashMap;
use std::time::Instant;

use super::sql_compile::SqlBind;
use crate::interpreter::builtins::model::query_log;

/// Bind values longer than this are truncated in the log — a base64 upload or a
/// large document should not fill the dev bar.
const MAX_BIND_CHARS: usize = 200;

/// An in-flight statement. Records when dropped.
pub struct Trace {
    started: Instant,
    /// Only populated when the query log is on, so production pays nothing but
    /// the `Instant`.
    logged: Option<Logged>,
}

struct Logged {
    sql: String,
    binds: Option<HashMap<String, serde_json::Value>>,
}

/// Begin tracing a statement. Always returns a guard: the duration feeds the
/// production metric even when the per-query log is off.
pub fn start(sql: &str, params: &[SqlBind]) -> Trace {
    let logged = if query_log::is_enabled() {
        Some(Logged {
            sql: sql.to_string(),
            binds: render_binds(params),
        })
    } else {
        None
    };
    Trace {
        started: Instant::now(),
        logged,
    }
}

/// Trace a statement that takes no bind parameters (DDL, `table_exists`).
pub fn start_plain(sql: &str) -> Trace {
    start(sql, &[])
}

/// Binds as `{"1": …, "2": …}`, matching the `$1` / `?` numbering a developer
/// sees in the logged SQL.
fn render_binds(params: &[SqlBind]) -> Option<HashMap<String, serde_json::Value>> {
    if params.is_empty() {
        return None;
    }
    let mut out = HashMap::with_capacity(params.len());
    for (index, param) in params.iter().enumerate() {
        out.insert((index + 1).to_string(), render_bind(param));
    }
    Some(out)
}

fn render_bind(param: &SqlBind) -> serde_json::Value {
    match param {
        SqlBind::Text(s) => serde_json::json!(truncate(s)),
        SqlBind::I64(n) => serde_json::json!(n),
        SqlBind::F64(f) => serde_json::json!(f),
        SqlBind::Bool(b) => serde_json::json!(b),
        SqlBind::Json(value) => {
            let text = value.to_string();
            if text.len() > MAX_BIND_CHARS {
                serde_json::json!(truncate(&text))
            } else {
                value.clone()
            }
        }
    }
}

fn truncate(value: &str) -> String {
    if value.chars().count() <= MAX_BIND_CHARS {
        return value.to_string();
    }
    let head: String = value.chars().take(MAX_BIND_CHARS).collect();
    format!("{head}… ({} chars)", value.chars().count())
}

impl Drop for Trace {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        // Coarse production counter, as on the SoliDB path.
        crate::metrics::Metrics::global().record_db_queries(elapsed);

        let Some(logged) = self.logged.take() else {
            return;
        };
        let ms = elapsed.as_secs_f64() * 1000.0;
        // The flamegraph groups by a short name; the panel shows the full SQL.
        let span_name: String = logged.sql.chars().take(80).collect();
        crate::serve::span_log::record(
            &span_name,
            crate::serve::span_log::SpanKind::Db,
            self.started,
            (ms * 1000.0).max(0.0) as u64,
            None,
        );
        query_log::record(logged.sql, logged.binds, ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_are_numbered_like_the_placeholders() {
        let params = vec![
            SqlBind::Text("open".into()),
            SqlBind::I64(7),
            SqlBind::Bool(true),
        ];
        let rendered = render_binds(&params).expect("binds");
        assert_eq!(rendered["1"], serde_json::json!("open"));
        assert_eq!(rendered["2"], serde_json::json!(7));
        assert_eq!(rendered["3"], serde_json::json!(true));
        // No parameters logs no bind map at all, rather than an empty one.
        assert!(render_binds(&[]).is_none());
    }

    #[test]
    fn a_long_bind_is_truncated_with_its_real_length() {
        let long = "x".repeat(MAX_BIND_CHARS + 50);
        let rendered = render_bind(&SqlBind::Text(long.clone()));
        let text = rendered.as_str().expect("string");
        assert!(text.starts_with(&"x".repeat(10)));
        assert!(
            text.contains(&format!("({} chars)", long.len())),
            "the real length must survive truncation: {text}"
        );
        assert!(text.chars().count() < long.chars().count());

        // A small JSON bind is kept as JSON so the panel can render it.
        let small = render_bind(&SqlBind::Json(serde_json::json!({ "a": 1 })));
        assert_eq!(small, serde_json::json!({ "a": 1 }));
    }

    #[test]
    fn tracing_is_inert_when_the_log_is_off() {
        // Production path: no log, no panic, and the guard still records the
        // duration for the metrics counter.
        let before = query_log::snapshot().len();
        {
            let _trace = start("SELECT 1", &[]);
        }
        assert_eq!(query_log::snapshot().len(), before);
    }
}
