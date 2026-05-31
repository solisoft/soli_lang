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
//! SOLI_LOG=timing            # middleware / view / phase timings
//! SOLI_LOG=query,http,timing # any combination
//! SOLI_LOG=all               # everything
//! ```
//!
//! Turning on any of `query`/`http`/`timing` implies `access` so the
//! detail block has a request line to anchor to. The legacy
//! `SOLI_REQUEST_LOG=1` still works as an alias for the `access` channel.

use std::sync::OnceLock;

#[derive(Clone, Copy, Default)]
pub struct LogChannels {
    /// Per-request access line: `[LOG] METHOD PATH - status (ms)`.
    pub access: bool,
    /// AQL query log (binds + duration).
    pub query: bool,
    /// Outgoing HTTP.* call log.
    pub http: bool,
    /// Middleware / view / phase timing breakdown.
    pub timing: bool,
}

impl LogChannels {
    /// True if any channel is on. Used to decide whether the per-request
    /// timer and the thread-local buffer clearing are worth their cost.
    #[inline]
    pub fn any(&self) -> bool {
        self.access || self.query || self.http || self.timing
    }

    /// True if any *detail* channel (beyond the bare access line) is on.
    #[inline]
    pub fn has_detail(&self) -> bool {
        self.query || self.http || self.timing
    }
}

fn parse(soli_log: Option<&str>, request_log: bool) -> LogChannels {
    let mut ch = LogChannels::default();

    if let Some(raw) = soli_log {
        for token in raw.split(',') {
            match token.trim().to_ascii_lowercase().as_str() {
                "" => {}
                "all" | "1" | "true" => {
                    ch.access = true;
                    ch.query = true;
                    ch.http = true;
                    ch.timing = true;
                }
                "access" | "request" | "requests" => ch.access = true,
                "query" | "queries" | "db" | "sql" | "aql" => ch.query = true,
                "http" => ch.http = true,
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
    // blocks with no request to anchor them — fold access in.
    if ch.has_detail() {
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
        parse(soli_log.as_deref(), request_log)
    })
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
pub fn emit(method: &str, path: &str, status: u16, elapsed_ms: f64, ch: LogChannels) {
    use std::fmt::Write;

    let mut out = String::with_capacity(256);
    let _ = write!(
        out,
        "[LOG] {} {} - {} ({:.3}ms)",
        method, path, status, elapsed_ms
    );

    if ch.query {
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
                        let rendered =
                            serde_json::to_string(binds).unwrap_or_else(|_| "{}".to_string());
                        let _ = write!(out, " binds={}", rendered);
                    }
                }
            }
        }
    }

    if ch.http {
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
                    call.duration_ms, call.method, call.url, call.status
                );
                if let Some(err) = &call.error {
                    let _ = write!(out, " [error: {}]", err);
                }
            }
        }
    }

    if ch.timing {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_channels() {
        let ch = parse(Some("query,http"), false);
        assert!(ch.query && ch.http && ch.access);
        assert!(!ch.timing);
    }

    #[test]
    fn all_enables_everything() {
        let ch = parse(Some("all"), false);
        assert!(ch.access && ch.query && ch.http && ch.timing);
    }

    #[test]
    fn detail_channel_implies_access() {
        let ch = parse(Some("timing"), false);
        assert!(ch.access && ch.timing);
    }

    #[test]
    fn request_log_alias_enables_access_only() {
        let ch = parse(None, true);
        assert!(ch.access);
        assert!(!ch.has_detail());
    }

    #[test]
    fn empty_env_is_all_off() {
        let ch = parse(None, false);
        assert!(!ch.any());
    }

    #[test]
    fn aliases_resolve() {
        let ch = parse(Some("queries, db , phases"), false);
        assert!(ch.query && ch.timing);
    }
}
