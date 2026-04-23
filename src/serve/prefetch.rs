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

/// Client JS — compiled into the binary so there's no filesystem dependency
/// and no cache-busting headache (it's pinned to the Soli build version).
pub const PREFETCH_SCRIPT: &str = include_str!("prefetch.js");

/// Marker used to make injection idempotent when the same body is rewrapped
/// (e.g. by a middleware that re-enters `html_response`).
const INJECTED_MARKER: &str = "__soli_prefetch_injected";

/// Tag injected into responses. The comment marker immediately before the
/// `<script>` lets us detect prior injection without parsing the HTML.
const PREFETCH_TAG: &str =
    "<!-- __soli_prefetch_injected --><script src=\"/__soli/prefetch.js\" defer></script>";

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

    if let Some(pos) = rfind_ascii_case_insensitive(html, b"</body>") {
        let mut out = String::with_capacity(html.len() + PREFETCH_TAG.len());
        out.push_str(&html[..pos]);
        out.push_str(PREFETCH_TAG);
        out.push_str(&html[pos..]);
        out
    } else if let Some(pos) = rfind_ascii_case_insensitive(html, b"</html>") {
        let mut out = String::with_capacity(html.len() + PREFETCH_TAG.len());
        out.push_str(&html[..pos]);
        out.push_str(PREFETCH_TAG);
        out.push_str(&html[pos..]);
        out
    } else {
        format!("{}{}", html, PREFETCH_TAG)
    }
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
        let script_pos = result.find(PREFETCH_TAG).expect("tag inserted");
        let body_pos = result.find("</body>").expect("body still there");
        assert!(script_pos < body_pos, "tag must land before </body>");
        assert!(result.contains("<h1>Hi</h1>"), "existing content preserved");
    }

    #[test]
    fn inject_case_insensitive() {
        let html = "<HTML><BODY><h1>Hi</h1></BODY></HTML>";
        let result = inject_prefetch_tag(html);
        assert!(
            result.contains(PREFETCH_TAG),
            "tag missing in uppercase HTML"
        );
    }

    #[test]
    fn inject_idempotent() {
        let html = "<html><body>x</body></html>";
        let once = inject_prefetch_tag(html);
        let twice = inject_prefetch_tag(&once);
        assert_eq!(once, twice, "second inject must be a no-op");
        assert_eq!(once.matches(PREFETCH_TAG).count(), 1);
    }

    #[test]
    fn inject_no_body_tag_falls_back_to_html_close() {
        let html = "<html><h1>hi</h1></html>";
        let result = inject_prefetch_tag(html);
        let script_pos = result.find(PREFETCH_TAG).expect("tag inserted");
        let html_pos = result.find("</html>").unwrap();
        assert!(script_pos < html_pos);
    }

    #[test]
    fn inject_no_tags_appends() {
        let html = "<h1>Bare fragment</h1>";
        let result = inject_prefetch_tag(html);
        assert!(result.starts_with(html));
        assert!(result.ends_with(PREFETCH_TAG));
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
        // Smoke-check the embedded JS is wrapped and uses `<link rel="prefetch">`
        // (no `as`) as the mechanism — navigations consume from the prefetched-
        // resources cache, which is the semantic we need.
        assert!(PREFETCH_SCRIPT.contains("__soliPrefetchInstalled"));
        assert!(PREFETCH_SCRIPT.contains("link.rel = \"prefetch\""));
        // Guard against regressing to `as="document"` — that routes into a
        // stricter document-prefetch cache with extra reuse conditions.
        assert!(!PREFETCH_SCRIPT.contains("link.as ="));
    }

    #[test]
    fn script_skips_self_links() {
        // Regression: `<a href="/foo">` on `/foo` used to prefetch itself.
        // The fix compares pathname + search (ignoring hash) against location.
        assert!(PREFETCH_SCRIPT.contains("a.pathname === location.pathname"));
        assert!(PREFETCH_SCRIPT.contains("a.search === location.search"));
    }
}
