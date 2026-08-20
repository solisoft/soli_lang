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

/// Remove a `user:password@` userinfo component from a URL's authority.
///
/// Returns the input unchanged when there is no scheme, no `@`, or the `@`
/// belongs to the path or query rather than the authority (`/a@b`, `?to=a@b`).
fn strip_userinfo(url: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let Some(scheme_end) = url.find("://") else {
        return Cow::Borrowed(url);
    };
    let authority_start = scheme_end + 3;
    let after_scheme = &url[authority_start..];
    // The authority ends at the first `/`, `?` or `#`.
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    // `rfind`: a password may itself contain an `@`.
    let Some(at_in_authority) = authority.rfind('@') else {
        return Cow::Borrowed(url);
    };
    Cow::Owned(format!(
        "{}{}",
        &url[..authority_start],
        &after_scheme[at_in_authority + 1..]
    ))
}

/// Strip userinfo and sensitive query values from a URL before it is logged.
///
/// `pub(crate)` because the flamegraph span name needs the same treatment: it
/// used to be built from the raw URL, so `?api_key=sk_live_…` was exported to
/// the OTel collector even though the query panel next to it was scrubbed.
pub(crate) fn scrub_url_for_log(url: &str) -> String {
    const SENSITIVE_QUERY_PARAMS: &[&str] = &[
        "api_key",
        "token",
        "access_token",
        "secret",
        "password",
        "private_key",
    ];

    // Strip `user:password@` from the authority, then fall through to query
    // scrubbing. Two bugs lived here: the guard compared the `@` offset within
    // the post-scheme slice against the same offset computed a second way and
    // required `<` where the two are always equal, so it never fired at all;
    // and it `return`ed on success, so a URL carrying both userinfo and an
    // `?api_key=` would have kept the key.
    let without_userinfo = strip_userinfo(url);
    let url: &str = &without_userinfo;

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

#[cfg(test)]
mod tests {
    use super::scrub_url_for_log;

    /// Both the query panel and the flamegraph span name go through this, and
    /// span names are exported off-box over OTLP — so a credential in the URL
    /// must not survive it.
    #[test]
    fn credentials_are_stripped_from_a_logged_url() {
        for (raw, must_not_contain) in [
            ("https://api.test/v1?api_key=sk_live_abc", "sk_live_abc"),
            ("https://api.test/v1?token=tok_abc", "tok_abc"),
            ("https://api.test/v1?access_token=at_abc", "at_abc"),
            ("https://api.test/v1?secret=s3cr3t", "s3cr3t"),
            ("https://api.test/v1?password=hunter2", "hunter2"),
            ("https://api.test/v1?private_key=pk_abc", "pk_abc"),
            ("https://user:pw@api.test/v1", "pw"),
        ] {
            let scrubbed = scrub_url_for_log(raw);
            assert!(
                !scrubbed.contains(must_not_contain),
                "{must_not_contain:?} survived scrubbing of {raw:?}: {scrubbed}"
            );
        }
    }

    /// Userinfo stripping used to `return` early, so a URL with both kinds of
    /// secret kept the query one.
    #[test]
    fn userinfo_and_query_secrets_are_both_removed() {
        let scrubbed = scrub_url_for_log("https://user:pw@api.test/v1?api_key=sk_live_abc&page=2");
        assert!(!scrubbed.contains("pw@"), "{scrubbed}");
        assert!(!scrubbed.contains("sk_live_abc"), "{scrubbed}");
        assert!(
            scrubbed.contains("page=2"),
            "harmless params survive: {scrubbed}"
        );
        assert!(scrubbed.starts_with("https://api.test/v1"), "{scrubbed}");
    }

    /// An `@` outside the authority is not userinfo and must be left alone.
    #[test]
    fn an_at_sign_in_the_path_or_query_is_not_userinfo() {
        for url in [
            "https://api.test/users/a@b.test",
            "https://api.test/v1?to=a@b.test",
            "https://api.test/v1#a@b",
        ] {
            assert_eq!(scrub_url_for_log(url), url, "{url}");
        }
    }

    #[test]
    fn a_url_with_nothing_sensitive_is_left_alone() {
        let plain = "https://api.test/v1/orders?page=2&per=50";
        assert_eq!(scrub_url_for_log(plain), plain);
    }
}
