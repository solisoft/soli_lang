//! Instant navigation ("Soli Nav") — Turbo-Drive-style body swapping.
//!
//! Ships a vanilla-JS script that intercepts same-origin link clicks, fetches
//! the target page, swaps `<body>` in place (merging title/stylesheets/meta
//! from the new `<head>`), and manages history with pushState — so navigation
//! keeps CSS/JS warm and feels instant while the app stays plain
//! server-rendered HTML. The JS is served at `/__soli/nav.js` as an external
//! file (not inline) so apps with strict CSP — no `unsafe-inline` — still work.
//!
//! When nav is enabled it **replaces** the hover-prefetch script: a `fetch()`
//! can't consume `<link rel="prefetch" as="document">` entries (cache
//! partitioning keys navigations and cors fetches separately), so nav.js does
//! its own hover prefetching into an in-memory cache. Its prefetch requests
//! carry `Purpose: prefetch`, which the existing server-side detection
//! ([`crate::serve::prefetch::is_prefetch_request`]) already turns into
//! `private, max-age=TTL` responses. `SOLI_PREFETCH=off` still disables hover
//! warming (via a `data-prefetch="off"` attribute on the script tag) without
//! disabling click swapping.
//!
//! Disable globally with `SOLI_NAV=off`; per link with `<a data-no-nav>` (or
//! any ancestor carrying that attribute); per page with
//! `<meta name="soli-nav" content="off">`.

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;

use crate::serve::live_reload::rfind_ascii_case_insensitive;
use crate::serve::prefetch;

/// Client JS — compiled into the binary so there's no filesystem dependency.
pub const NAV_SCRIPT: &str = include_str!("nav.js");

/// Marker used to make injection idempotent when the same body is rewrapped
/// (e.g. by a middleware that re-enters `html_response`).
const INJECTED_MARKER: &str = "__soli_nav_injected";

/// Content-derived version hash for the `?v=` cache-buster. Old binaries
/// served their old JS under `?v=OLD`; the new binary hands out `?v=NEW` URLs
/// so every browser loads the fresh script on the next page load.
fn nav_hash() -> u64 {
    use std::sync::OnceLock;
    static HASH: OnceLock<u64> = OnceLock::new();
    *HASH.get_or_init(|| prefetch::fnv1a_64(NAV_SCRIPT.as_bytes()))
}

/// Is instant navigation enabled? Reads `SOLI_NAV` at every call (cheap; once
/// per render). Default: on. Off when the value is one of `"off"`, `"false"`,
/// `"0"`, `"no"` (case-insensitive) — same convention as `SOLI_PREFETCH`.
pub fn is_enabled() -> bool {
    match std::env::var("SOLI_NAV") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "false" | "0" | "no"
        ),
        Err(_) => true,
    }
}

/// The `<script>` tag to inject. Built per call (not cached) because it embeds
/// the *current* `SOLI_PREFETCH` / `SOLI_PREFETCH_TTL` state as data
/// attributes the client script reads; only the content hash is cached.
fn nav_tag() -> String {
    let prefetch_attr = if prefetch::is_enabled() {
        ""
    } else {
        " data-prefetch=\"off\""
    };
    format!(
        "<!-- __soli_nav_injected --><script src=\"/__soli/nav.js?v={:016x}\" defer{} data-prefetch-ttl=\"{}\"></script>",
        nav_hash(),
        prefetch_attr,
        prefetch::prefetch_ttl()
    )
}

/// Insert the nav `<script>` tag into an HTML body. Idempotent — calling it
/// twice on the same string is a no-op.
///
/// Insertion order of preference:
///   1. Immediately before `</body>` (case-insensitive).
///   2. Fallback: before `</html>`.
///   3. Last resort: appended at the end.
pub fn inject_nav_tag(html: &str) -> String {
    if html.contains(INJECTED_MARKER) {
        return html.to_string();
    }
    let tag = nav_tag();
    if let Some(pos) = rfind_ascii_case_insensitive(html, b"</body>") {
        let mut out = String::with_capacity(html.len() + tag.len());
        out.push_str(&html[..pos]);
        out.push_str(&tag);
        out.push_str(&html[pos..]);
        out
    } else if let Some(pos) = rfind_ascii_case_insensitive(html, b"</html>") {
        let mut out = String::with_capacity(html.len() + tag.len());
        out.push_str(&html[..pos]);
        out.push_str(&tag);
        out.push_str(&html[pos..]);
        out
    } else {
        format!("{}{}", html, tag)
    }
}

/// HTTP handler for `GET /__soli/nav.js`. Returns the bundled script with
/// long cache (safe: content is compiled into the binary and the `?v=` hash
/// changes whenever the script does).
pub fn handle_nav_js() -> Response<Full<Bytes>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/javascript; charset=utf-8")
        .header("Cache-Control", "public, max-age=86400, immutable")
        .body(Full::new(Bytes::from_static(NAV_SCRIPT.as_bytes())))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    // `SOLI_NAV` / `SOLI_PREFETCH` / `SOLI_PREFETCH_TTL` are process-global,
    // so env-touching tests save/restore them and run serially under a mutex
    // to avoid other parallel tests racing. (`nav_tag()` reads the prefetch
    // vars, so injection tests guard those too.)
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        name: &'static str,
        prior: Option<String>,
    }
    impl EnvGuard {
        fn set(name: &'static str, value: Option<&str>) -> Self {
            let prior = std::env::var(name).ok();
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
            Self { name, prior }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(v) => std::env::set_var(self.name, v),
                None => std::env::remove_var(self.name),
            }
        }
    }

    #[test]
    fn is_enabled_defaults_on() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::set("SOLI_NAV", None);
        assert!(is_enabled());
    }

    #[test]
    fn is_enabled_respects_off_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        for v in ["off", "OFF", "false", "FALSE", "0", "no", "  off "] {
            let _g = EnvGuard::set("SOLI_NAV", Some(v));
            assert!(!is_enabled(), "SOLI_NAV={:?} should disable", v);
        }
        for v in ["on", "1", "true", "yes"] {
            let _g = EnvGuard::set("SOLI_NAV", Some(v));
            assert!(is_enabled(), "SOLI_NAV={:?} should enable", v);
        }
    }

    #[test]
    fn nav_tag_default_carries_ttl_and_no_prefetch_off() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g1 = EnvGuard::set("SOLI_PREFETCH", None);
        let _g2 = EnvGuard::set("SOLI_PREFETCH_TTL", None);
        let tag = nav_tag();
        assert!(tag.contains("/__soli/nav.js?v="));
        assert!(tag.contains("data-prefetch-ttl=\"30\""), "tag: {tag}");
        assert!(!tag.contains("data-prefetch=\"off\""), "tag: {tag}");
        assert!(tag.contains(INJECTED_MARKER));
    }

    #[test]
    fn nav_tag_carries_data_prefetch_off_when_prefetch_disabled() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g1 = EnvGuard::set("SOLI_PREFETCH", Some("off"));
        let _g2 = EnvGuard::set("SOLI_PREFETCH_TTL", Some("5"));
        let tag = nav_tag();
        assert!(tag.contains("data-prefetch=\"off\""), "tag: {tag}");
        assert!(tag.contains("data-prefetch-ttl=\"5\""), "tag: {tag}");
    }

    #[test]
    fn inject_before_body() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g1 = EnvGuard::set("SOLI_PREFETCH", None);
        let _g2 = EnvGuard::set("SOLI_PREFETCH_TTL", None);
        let html = "<html><body><h1>Hi</h1></body></html>";
        let result = inject_nav_tag(html);
        let script_pos = result.find(&nav_tag()).expect("tag inserted");
        let body_pos = result.find("</body>").expect("body still there");
        assert!(script_pos < body_pos, "tag must land before </body>");
        assert!(result.contains("<h1>Hi</h1>"), "existing content preserved");
    }

    #[test]
    fn inject_case_insensitive() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g1 = EnvGuard::set("SOLI_PREFETCH", None);
        let _g2 = EnvGuard::set("SOLI_PREFETCH_TTL", None);
        let html = "<HTML><BODY><h1>Hi</h1></BODY></HTML>";
        let result = inject_nav_tag(html);
        assert!(result.contains(&nav_tag()), "tag missing in uppercase HTML");
    }

    #[test]
    fn inject_idempotent() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g1 = EnvGuard::set("SOLI_PREFETCH", None);
        let _g2 = EnvGuard::set("SOLI_PREFETCH_TTL", None);
        let html = "<html><body>x</body></html>";
        let once = inject_nav_tag(html);
        let twice = inject_nav_tag(&once);
        assert_eq!(once, twice, "second inject must be a no-op");
        assert_eq!(once.matches(&nav_tag()).count(), 1);
    }

    #[test]
    fn inject_no_body_tag_falls_back_to_html_close() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g1 = EnvGuard::set("SOLI_PREFETCH", None);
        let _g2 = EnvGuard::set("SOLI_PREFETCH_TTL", None);
        let html = "<html><h1>hi</h1></html>";
        let result = inject_nav_tag(html);
        let script_pos = result.find(&nav_tag()).expect("tag inserted");
        let html_pos = result.find("</html>").unwrap();
        assert!(script_pos < html_pos);
    }

    #[test]
    fn inject_no_tags_appends() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g1 = EnvGuard::set("SOLI_PREFETCH", None);
        let _g2 = EnvGuard::set("SOLI_PREFETCH_TTL", None);
        let html = "<h1>Bare fragment</h1>";
        let result = inject_nav_tag(html);
        assert!(result.starts_with(html));
        assert!(result.ends_with(&nav_tag()));
    }

    #[test]
    fn inject_does_not_add_prefetch_marker() {
        // Nav and prefetch injection are mutually exclusive; the nav tag must
        // not smuggle the prefetch marker in and suppress nothing/confuse
        // middleware that re-enters html_response.
        let _lock = ENV_LOCK.lock().unwrap();
        let _g1 = EnvGuard::set("SOLI_PREFETCH", None);
        let _g2 = EnvGuard::set("SOLI_PREFETCH_TTL", None);
        let result = inject_nav_tag("<html><body>x</body></html>");
        assert!(!result.contains("__soli_prefetch_injected"));
    }

    #[test]
    fn handle_nav_js_returns_200_and_js_content_type() {
        let resp = handle_nav_js();
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
    fn script_is_guarded_iife_with_core_behaviors() {
        // Smoke-check the embedded JS carries the load-bearing pieces: the
        // install-once guard, the full click skip-list, history management,
        // and the swap machinery.
        assert!(NAV_SCRIPT.contains("__soliNavInstalled"));
        assert!(NAV_SCRIPT.contains("defaultPrevented"));
        assert!(NAV_SCRIPT.contains("data-no-nav"));
        assert!(NAV_SCRIPT.contains("hx-"));
        assert!(NAV_SCRIPT.contains("pushState"));
        assert!(NAV_SCRIPT.contains("popstate"));
        assert!(NAV_SCRIPT.contains("DOMParser"));
        assert!(NAV_SCRIPT.contains("executedSrcs"));
    }

    #[test]
    fn script_reinitializes_client_stack_after_swap() {
        // Alpine/htmx must be re-wired into the fresh body, and view
        // transitions stay opt-in behind the meta tag.
        assert!(NAV_SCRIPT.contains("Alpine.initTree"));
        assert!(NAV_SCRIPT.contains("htmx.process"));
        assert!(NAV_SCRIPT.contains("startViewTransition"));
        assert!(NAV_SCRIPT.contains("meta[name=\"view-transition\"][content=\"same-origin\"]"));
        assert!(
            NAV_SCRIPT.contains("template[x-teleport]"),
            "teleported Alpine pages must bail to a full navigation"
        );
    }

    #[test]
    fn script_executes_swapped_scripts_sequentially() {
        // Scripts must re-execute in document order with externals awaited
        // (parser semantics): activating them all at once runs inline scripts
        // before the externals they depend on (`tailwind is not defined`),
        // and Alpine/htmx re-init must wait for the chain or x-data scopes
        // registered by page-specific bundles don't exist yet.
        assert!(NAV_SCRIPT.contains("executeScripts"));
        assert!(NAV_SCRIPT.contains("queue.reduce"));
        assert!(NAV_SCRIPT.contains("fresh.onload = fresh.onerror"));
        assert!(
            NAV_SCRIPT.contains("setTimeout(initNewBody, 0)"),
            "Alpine/htmx re-init must run after the script queue AND one \
             macrotask later, so replayed alpine:init registrations land first"
        );
        assert!(
            NAV_SCRIPT.contains("\"alpine:init\""),
            "alpine:init must be replayed for page bundles first executed by a swap"
        );
        assert!(
            NAV_SCRIPT.contains("setAttribute(\"x-ignore\""),
            "the incoming body must carry x-ignore or Alpine's MutationObserver \
             initializes it before the page scripts have run"
        );
    }

    #[test]
    fn script_replays_domcontentloaded_after_swap() {
        // Inline scripts re-executed after a swap routinely register
        // DOMContentLoaded/load listeners — events that never fire again on
        // this document. The script must replay such late registrations
        // (jQuery-ready semantics) or page init silently breaks after the
        // first navigation.
        assert!(NAV_SCRIPT.contains("dclFired"));
        assert!(NAV_SCRIPT.contains("patchAddEventListener"));
        assert!(NAV_SCRIPT.contains("\"DOMContentLoaded\""));
        assert!(
            !NAV_SCRIPT.contains("dispatchEvent(new Event(\"DOMContentLoaded\""),
            "must never re-dispatch DOMContentLoaded globally — Alpine's CDN \
             bootstrap listens for it and would double-start"
        );
    }

    #[test]
    fn script_prefetches_with_purpose_header() {
        // The hover prefetch must announce itself so the server's existing
        // is_prefetch_request() detection answers with private, max-age=TTL.
        assert!(NAV_SCRIPT.contains("\"Purpose\"] = \"prefetch\""));
        assert!(NAV_SCRIPT.contains("data-prefetch-ttl"));
        assert!(NAV_SCRIPT.contains("data-no-prefetch"));
    }
}
