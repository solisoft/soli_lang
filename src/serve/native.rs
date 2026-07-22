//! Native bridge — server half.
//!
//! Ships `native.js` at `/__soli/native.js` and serves the SSE stream it
//! subscribes to at `/__soli/native/stream`. The Soli-side API and the channel
//! token format live in [`crate::interpreter::builtins::native`].
//!
//! Injection is **gated on the page**, unlike `nav.js`: the script tag is added
//! only when the HTML already carries a `soli-native` meta tag, which is what
//! `native_channel(...)` emits in a view. A page that wants nothing from the
//! shell therefore downloads nothing and opens no stream.
//!
//! The stream endpoint is the only place a channel is trusted, so it verifies
//! the token before subscribing. Every rejection is a flat `403` — the reason
//! is deliberately not echoed back, since "expired" versus "bad signature" is
//! information an attacker probing tokens would enjoy having.

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;

use crate::interpreter::builtins::native::{topic_for, verify_channel};
use crate::serve::live_reload::rfind_ascii_case_insensitive;
use crate::serve::prefetch;

/// Client JS — compiled into the binary so there is no filesystem dependency.
pub const NATIVE_SCRIPT: &str = include_str!("native.js");

/// Marker making injection idempotent when a body is rewrapped.
const INJECTED_MARKER: &str = "__soli_native_injected";

/// The meta tag `native_channel(...)` emits. Its presence is what turns the
/// bridge on for a page.
const CHANNEL_META: &str = "name=\"soli-native\"";

/// Content-derived `?v=` cache-buster, so a new binary hands out a new URL.
fn native_hash() -> u64 {
    use std::sync::OnceLock;
    static HASH: OnceLock<u64> = OnceLock::new();
    *HASH.get_or_init(|| prefetch::fnv1a_64(NATIVE_SCRIPT.as_bytes()))
}

/// Does this page ask for the bridge? Cheap substring check on the rendered
/// HTML, once per response.
pub fn page_wants_bridge(html: &str) -> bool {
    html.contains(CHANNEL_META)
}

fn native_tag() -> String {
    format!(
        "<!-- {} --><script src=\"/__soli/native.js?v={:016x}\" defer></script>",
        INJECTED_MARKER,
        native_hash()
    )
}

/// Insert the bridge `<script>` before `</body>` — or `</html>`, or at the end.
/// Idempotent, and a no-op for pages with no channel meta tag.
pub fn inject_native_tag(html: &str) -> String {
    if html.contains(INJECTED_MARKER) || !page_wants_bridge(html) {
        return html.to_string();
    }
    let tag = native_tag();
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

/// `GET /__soli/native.js`.
pub fn handle_native_js() -> Response<Full<Bytes>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/javascript; charset=utf-8")
        .header("Cache-Control", "public, max-age=86400, immutable")
        .body(Full::new(Bytes::from_static(NATIVE_SCRIPT.as_bytes())))
        .unwrap()
}

/// The channel a `/__soli/native/stream` request is entitled to, or `None`.
///
/// Split from the response so the query parsing and the verification are
/// testable without a live server.
pub fn channel_for_query(query: Option<&str>) -> Option<String> {
    let token = query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token").then_some(value)
    })?;
    let decoded = percent_decode(token);
    verify_channel(&decoded).ok()
}

/// Minimal percent-decoding for a token, whose alphabet is base64url + `.`.
/// Anything that fails to decode stays as-is and simply fails verification.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The SSE topic a verified request subscribes to.
pub fn topic_for_query(query: Option<&str>) -> Option<String> {
    channel_for_query(query).map(|channel| topic_for(&channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "<html><head><meta name=\"soli-native\" content=\"tok\"></head>\
                        <body><p>hi</p></body></html>";

    #[test]
    fn injects_before_body_close() {
        let out = inject_native_tag(BODY);
        assert!(out.contains("/__soli/native.js?v="));
        let script = out.find("__soli/native.js").unwrap();
        let body_close = out.find("</body>").unwrap();
        assert!(script < body_close, "script must precede </body>");
    }

    /// A page that never called `native_channel` must not pay for the bridge:
    /// no script, no stream, no cost.
    #[test]
    fn does_not_inject_without_a_channel_meta_tag() {
        let plain = "<html><body><p>hi</p></body></html>";
        assert_eq!(inject_native_tag(plain), plain);
    }

    #[test]
    fn injection_is_idempotent() {
        let once = inject_native_tag(BODY);
        assert_eq!(inject_native_tag(&once), once);
    }

    #[test]
    fn falls_back_to_html_close_then_append() {
        let no_body = "<html><meta name=\"soli-native\" content=\"t\"></html>";
        assert!(inject_native_tag(no_body).contains("native.js"));
        let bare = "<meta name=\"soli-native\" content=\"t\">";
        assert!(inject_native_tag(bare).ends_with("</script>"));
    }

    #[test]
    fn a_request_without_a_token_gets_no_channel() {
        assert!(channel_for_query(None).is_none());
        assert!(channel_for_query(Some("")).is_none());
        assert!(channel_for_query(Some("channel=user:42")).is_none());
    }

    /// An unsigned or forged token must not yield a channel — this is the only
    /// gate between a client and someone else's notifications.
    #[test]
    fn a_forged_token_gets_no_channel() {
        assert!(channel_for_query(Some("token=not-a-real-token")).is_none());
        assert!(channel_for_query(Some("token=a.b.c")).is_none());
    }

    #[test]
    fn percent_decoding_handles_encoded_tokens() {
        assert_eq!(percent_decode("a%2Eb"), "a.b");
        assert_eq!(percent_decode("plain"), "plain");
        // A stray '%' is left alone rather than swallowing the next character.
        assert_eq!(percent_decode("100%"), "100%");
    }
}
