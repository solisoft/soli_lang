//! Hover-preload for links.
//!
//! Ships a small vanilla-JS script that listens for `mouseover` on `<a>` tags
//! and injects `<link rel="prefetch">` to warm the browser cache before the
//! user actually clicks. The JS is served at `/__soli/prefetch.js` as an
//! external file (not inline) so apps with strict CSP — no `unsafe-inline` —
//! still work.
//!
//! The framework auto-injects a `<script src="/__soli/prefetch.js" defer>` tag
//! into every HTML response via `html_response`, next to the existing live
//! reload injection. Disable globally with `SOLI_PREFETCH=off`; disable for
//! individual links with `<a data-no-prefetch>` or any ancestor carrying that
//! attribute.

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;

use crate::serve::live_reload::rfind_ascii_case_insensitive;

/// Client JS — compiled into the binary so there's no filesystem dependency.
pub const PREFETCH_SCRIPT: &str = include_str!("prefetch.js");

/// Marker used to make injection idempotent when the same body is rewrapped
/// (e.g. by a middleware that re-enters `html_response`).
const INJECTED_MARKER: &str = "__soli_prefetch_injected";

/// Lazily-computed `<script>` tag with a content-derived query-string version.
/// Gives us "immutable with cache-bust" semantics: old binaries served their
/// old JS under `?v=OLD`; the new binary hands out `?v=NEW` URLs so every
/// browser loads the fresh script on the next page load. No hard-reload dance.
fn prefetch_tag() -> &'static str {
    use std::sync::OnceLock;
    static TAG: OnceLock<String> = OnceLock::new();
    TAG.get_or_init(|| {
        let hash = fnv1a_64(PREFETCH_SCRIPT.as_bytes());
        format!(
            "<!-- __soli_prefetch_injected --><script src=\"/__soli/prefetch.js?v={:016x}\" defer></script>",
            hash
        )
    })
}

/// FNV-1a 64-bit over the embedded script bytes. Cheap, deterministic per
/// binary, collision-free for our single-file cache-buster use case.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Is the hover-prefetch feature enabled? Reads `SOLI_PREFETCH` env var at
/// every call (cheap; std::env::var is ~a few syscalls and we only hit this
/// once per render). Default: on. Off when the value is one of
/// `"off"`, `"false"`, `"0"`, `"no"` (case-insensitive).
pub fn is_enabled() -> bool {
    match std::env::var("SOLI_PREFETCH") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "false" | "0" | "no"
        ),
        Err(_) => true,
    }
}

/// Insert the prefetch `<script>` tag into an HTML body. Idempotent — calling
/// it twice on the same string is a no-op.
///
/// Insertion order of preference:
///   1. Immediately before `</body>` (case-insensitive).
///   2. Fallback: before `</html>`.
///   3. Last resort: appended at the end.
pub fn inject_prefetch_tag(html: &str) -> String {
    if html.contains(INJECTED_MARKER) {
        return html.to_string();
    }
    let tag = prefetch_tag();
    if let Some(pos) = rfind_ascii_case_insensitive(html, b"</body>") {
        let mut out = String::with_capacity(html.len() + tag.len());
        out.push_str(&html[..pos]);
        out.push_str(tag);
        out.push_str(&html[pos..]);
        out
    } else if let Some(pos) = rfind_ascii_case_insensitive(html, b"</html>") {
        let mut out = String::with_capacity(html.len() + tag.len());
        out.push_str(&html[..pos]);
        out.push_str(tag);
        out.push_str(&html[pos..]);
        out
    } else {
        format!("{}{}", html, tag)
    }
}

/// True when the request headers indicate a browser-issued *speculative*
/// prefetch rather than a real navigation. Chrome/Edge send
/// `Sec-Purpose: prefetch` (sometimes `prefetch;prerender`), older Chrome
/// `Purpose: prefetch`, Firefox `X-Moz: prefetch`. `get_header` is invoked
/// with lowercased header names and returns the raw value if present.
pub fn is_prefetch_request<'a, F>(get_header: F) -> bool
where
    F: Fn(&str) -> Option<&'a str>,
{
    let signals_prefetch = |name: &str| {
        get_header(name)
            .map(|v| v.to_ascii_lowercase().contains("prefetch"))
            .unwrap_or(false)
    };
    signals_prefetch("sec-purpose") || signals_prefetch("purpose") || signals_prefetch("x-moz")
}

/// `Cache-Control` to emit for an HTML response to a *speculative prefetch*
/// request. A short `private, max-age=N` lets the browser reuse the prefetched
/// body on the eventual click directly from its own cache — no conditional GET,
/// so a CDN in front of the app (Cloudflare et al.) that won't relay a `304`
/// can't break "instant navigation". `private` keeps the prefetched HTML out of
/// shared/edge caches. Normal navigations still get `private, no-cache` (set in
/// `html_response`) because they don't carry the prefetch headers.
///
/// Default window: 30s — long enough to bridge hover→click, short enough to
/// bound staleness. Override with `SOLI_PREFETCH_TTL` (seconds, clamped 1..=300).
pub fn prefetch_cache_control() -> String {
    let ttl = std::env::var("SOLI_PREFETCH_TTL")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|secs| secs.clamp(1, 300))
        .unwrap_or(30);
    format!("private, max-age={}", ttl)
}

/// HTTP handler for `GET /__soli/prefetch.js`. Returns the bundled script with
/// long cache (safe: content is compiled into the binary, so it only changes
/// when the soli version changes and the old cached file keeps working).
pub fn handle_prefetch_js() -> Response<Full<Bytes>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/javascript; charset=utf-8")
        .header("Cache-Control", "public, max-age=86400, immutable")
        .body(Full::new(Bytes::from_static(PREFETCH_SCRIPT.as_bytes())))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    // `SOLI_PREFETCH` is process-global, so the env-var tests save/restore it
    // and run serially under a mutex to avoid other parallel tests racing.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        prior: Option<String>,
    }
    impl EnvGuard {
        fn set(value: Option<&str>) -> Self {
            let prior = std::env::var("SOLI_PREFETCH").ok();
            match value {
                Some(v) => std::env::set_var("SOLI_PREFETCH", v),
                None => std::env::remove_var("SOLI_PREFETCH"),
            }
            Self { prior }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(v) => std::env::set_var("SOLI_PREFETCH", v),
                None => std::env::remove_var("SOLI_PREFETCH"),
            }
        }
    }

    struct TtlEnvGuard {
        prior: Option<String>,
    }
    impl TtlEnvGuard {
        fn set(value: Option<&str>) -> Self {
            let prior = std::env::var("SOLI_PREFETCH_TTL").ok();
            match value {
                Some(v) => std::env::set_var("SOLI_PREFETCH_TTL", v),
                None => std::env::remove_var("SOLI_PREFETCH_TTL"),
            }
            Self { prior }
        }
    }
    impl Drop for TtlEnvGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(v) => std::env::set_var("SOLI_PREFETCH_TTL", v),
                None => std::env::remove_var("SOLI_PREFETCH_TTL"),
            }
        }
    }

    #[test]
    fn is_enabled_defaults_on() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::set(None);
        assert!(is_enabled());
    }

    #[test]
    fn is_enabled_respects_off_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        for v in ["off", "OFF", "false", "FALSE", "0", "no", "  off "] {
            let _g = EnvGuard::set(Some(v));
            assert!(!is_enabled(), "SOLI_PREFETCH={:?} should disable", v);
        }
        for v in ["on", "1", "true", "yes", ""] {
            let _g = EnvGuard::set(Some(v));
            // Empty-string and unrecognized values enable by design — we err on
            // the side of "prefetch is on unless you clearly turned it off".
            assert!(
                is_enabled() || v.is_empty(),
                "SOLI_PREFETCH={:?} should enable",
                v
            );
        }
    }

    #[test]
    fn inject_before_body() {
        let html = "<html><body><h1>Hi</h1></body></html>";
        let result = inject_prefetch_tag(html);
        let script_pos = result.find(prefetch_tag()).expect("tag inserted");
        let body_pos = result.find("</body>").expect("body still there");
        assert!(script_pos < body_pos, "tag must land before </body>");
        assert!(result.contains("<h1>Hi</h1>"), "existing content preserved");
    }

    #[test]
    fn inject_case_insensitive() {
        let html = "<HTML><BODY><h1>Hi</h1></BODY></HTML>";
        let result = inject_prefetch_tag(html);
        assert!(
            result.contains(prefetch_tag()),
            "tag missing in uppercase HTML"
        );
    }

    #[test]
    fn inject_idempotent() {
        let html = "<html><body>x</body></html>";
        let once = inject_prefetch_tag(html);
        let twice = inject_prefetch_tag(&once);
        assert_eq!(once, twice, "second inject must be a no-op");
        assert_eq!(once.matches(prefetch_tag()).count(), 1);
    }

    #[test]
    fn inject_no_body_tag_falls_back_to_html_close() {
        let html = "<html><h1>hi</h1></html>";
        let result = inject_prefetch_tag(html);
        let script_pos = result.find(prefetch_tag()).expect("tag inserted");
        let html_pos = result.find("</html>").unwrap();
        assert!(script_pos < html_pos);
    }

    #[test]
    fn inject_no_tags_appends() {
        let html = "<h1>Bare fragment</h1>";
        let result = inject_prefetch_tag(html);
        assert!(result.starts_with(html));
        assert!(result.ends_with(prefetch_tag()));
    }

    #[test]
    fn handle_prefetch_js_returns_200_and_js_content_type() {
        let resp = handle_prefetch_js();
        assert_eq!(resp.status().as_u16(), 200);
        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.starts_with("application/javascript"),
            "Content-Type should be application/javascript, got {:?}",
            ct
        );
        assert!(
            resp.headers()
                .get("Cache-Control")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .contains("immutable"),
            "should be cacheable-forever"
        );
    }

    #[test]
    fn script_is_non_empty_iife() {
        // Smoke-check the embedded JS is wrapped and uses
        // `<link rel="prefetch" as="document">` — the form Chromium promotes
        // to a top-level navigation response. Plain `rel="prefetch"` (no
        // `as`) lands in a subresource cache that navigations skip, so the
        // prefetch never accelerates anything.
        assert!(PREFETCH_SCRIPT.contains("__soliPrefetchInstalled"));
        assert!(PREFETCH_SCRIPT.contains("link.rel = \"prefetch\""));
        assert!(
            PREFETCH_SCRIPT.contains("link.as = \"document\""),
            "must set `as=\"document\"` so navigations consume the prefetched body"
        );
    }

    #[test]
    fn detects_prefetch_request_headers() {
        // Chrome/Edge speculative prefetch.
        let h = |n: &str| match n {
            "sec-purpose" => Some("prefetch"),
            _ => None,
        };
        assert!(is_prefetch_request(h));

        // `prefetch;prerender` compound value still matches.
        let h = |n: &str| match n {
            "sec-purpose" => Some("prefetch;prerender"),
            _ => None,
        };
        assert!(is_prefetch_request(h));

        // Legacy Chrome + Firefox spellings.
        assert!(is_prefetch_request(
            |n| (n == "purpose").then_some("prefetch")
        ));
        assert!(is_prefetch_request(|n| (n == "x-moz").then_some("prefetch")));

        // A real navigation carries none of these.
        let h = |n: &str| match n {
            "sec-purpose" => Some("prerender"),
            "sec-fetch-mode" => Some("navigate"),
            _ => None,
        };
        assert!(!is_prefetch_request(h));
        assert!(!is_prefetch_request(|_| None));
    }

    #[test]
    fn prefetch_cache_control_is_private_with_default_window() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = TtlEnvGuard::set(None);
        let cc = prefetch_cache_control();
        assert!(cc.starts_with("private,"), "must stay private: {cc}");
        assert_eq!(cc, "private, max-age=30");
    }

    #[test]
    fn prefetch_cache_control_honors_and_clamps_ttl() {
        let _lock = ENV_LOCK.lock().unwrap();
        {
            let _g = TtlEnvGuard::set(Some("5"));
            assert_eq!(prefetch_cache_control(), "private, max-age=5");
        }
        {
            // Above the ceiling clamps to 300, below the floor clamps to 1.
            let _g = TtlEnvGuard::set(Some("100000"));
            assert_eq!(prefetch_cache_control(), "private, max-age=300");
        }
        {
            let _g = TtlEnvGuard::set(Some("0"));
            assert_eq!(prefetch_cache_control(), "private, max-age=1");
        }
        {
            // Garbage falls back to the default.
            let _g = TtlEnvGuard::set(Some("soon"));
            assert_eq!(prefetch_cache_control(), "private, max-age=30");
        }
    }

    #[test]
    fn script_skips_self_links() {
        // Regression: `<a href="/foo">` on `/foo` used to prefetch itself.
        // The fix compares pathname + search (ignoring hash) against location.
        assert!(PREFETCH_SCRIPT.contains("a.pathname === location.pathname"));
        assert!(PREFETCH_SCRIPT.contains("a.search === location.search"));
    }
}
