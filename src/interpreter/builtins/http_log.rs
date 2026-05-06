//! Per-request log of outgoing HTTP calls made via the `HTTP.*` builtin.
//!
//! Mirrors `model::query_log` but for the user-facing HTTP client. The server
//! clears the log at the start of each incoming request so the dev bar shows
//! only the outbound calls fired during that single request.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct LoggedHttpRequest {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub duration_ms: f64,
    pub error: Option<String>,
}

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOG: RefCell<Vec<LoggedHttpRequest>> = const { RefCell::new(Vec::new()) };
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn scrub_url_for_log(url: &str) -> String {
    const SENSITIVE_QUERY_PARAMS: &[&str] = &[
        "api_key",
        "token",
        "access_token",
        "secret",
        "password",
        "private_key",
    ];

    if let Some(at_pos) = url.find('@') {
        if let Some(scheme_end) = url.find("://") {
            let after_scheme = &url[scheme_end + 3..];
            if after_scheme
                .find('@')
                .is_some_and(|i| i < at_pos.saturating_sub(scheme_end + 3))
            {
                let scheme = &url[..scheme_end + 3];
                let after_userinfo = &url[at_pos + 1..];
                if let Some(path_start) = after_userinfo.find('/') {
                    return format!(
                        "{}{}{}",
                        scheme,
                        &after_userinfo[..path_start],
                        &after_userinfo[path_start..]
                    );
                }
                return format!("{}{}", scheme, after_userinfo);
            }
        }
    }

    if let Some(query_pos) = url.find('?') {
        let base = &url[..query_pos];
        let query = &url[query_pos + 1..];
        let params: Vec<(String, String)> = query
            .split('&')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                let key = parts.next()?.to_string();
                if SENSITIVE_QUERY_PARAMS.iter().any(|s| key == *s) {
                    None
                } else {
                    Some((key, parts.next().unwrap_or("").to_string()))
                }
            })
            .collect();

        if params.is_empty() {
            base.to_string()
        } else {
            let scrubbed_query = params
                .into_iter()
                .map(|(k, v)| {
                    if v.is_empty() {
                        k
                    } else {
                        format!("{}={}", k, v)
                    }
                })
                .collect::<Vec<_>>()
                .join("&");
            format!("{}?{}", base, scrubbed_query)
        }
    } else {
        url.to_string()
    }
}

pub fn clear() {
    LOG.with(|l| l.borrow_mut().clear());
}

pub fn record(method: String, url: String, status: u16, duration_ms: f64, error: Option<String>) {
    let scrubbed_url = scrub_url_for_log(&url);
    // Anchor the span "end" to now and back-date the start by the
    // measured duration — close enough for visualisation, since the
    // call site doesn't expose the original start instant here.
    let dur_us = (duration_ms * 1000.0).max(0.0) as u64;
    let start = std::time::Instant::now() - std::time::Duration::from_micros(dur_us);
    record_with_start(method, scrubbed_url, status, duration_ms, error, start);
}

/// Like `record`, but uses the caller's real `Instant` for the flamegraph
/// span. Used by parallel-fetch helpers (`HTTP.get_all` etc.) that capture
/// the start timestamp on a worker thread and then record on the main
/// thread after `join()` — back-dating from `Instant::now()` would collapse
/// every concurrent call to the same end time and lose the parallelism.
pub fn record_with_start(
    method: String,
    url: String,
    status: u16,
    duration_ms: f64,
    error: Option<String>,
    real_start: std::time::Instant,
) {
    // Mirror this call as a span so it shows up in the dev-bar flamegraph
    // nested under whatever action / view fired it. Span_log is its own
    // gate, so this is a no-op when --dev is off.
    let scrubbed_url = scrub_url_for_log(&url);
    if crate::serve::span_log::is_enabled() {
        let dur_us = (duration_ms * 1000.0).max(0.0) as u64;
        let name = format!("{} {}", method, scrubbed_url);
        crate::serve::span_log::record(
            &name,
            crate::serve::span_log::SpanKind::Http,
            real_start,
            dur_us,
            error.clone(),
        );
    }

    LOG.with(|l| {
        l.borrow_mut().push(LoggedHttpRequest {
            method,
            url: scrubbed_url,
            status,
            duration_ms,
            error,
        })
    });
}

pub fn snapshot() -> Vec<LoggedHttpRequest> {
    LOG.with(|l| l.borrow().clone())
}
