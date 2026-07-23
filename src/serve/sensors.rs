//! Motion sensors — server half.
//!
//! Ships `sensors.js` at `/__soli/sensors.js`, injected only into pages that
//! opt in. A page opts in by calling `motion_sensors()` (which emits a
//! `soli-sensors` meta tag) or simply by referencing `soli.sensors` in inline
//! script — either marker is enough for the gate below to fire.
//!
//! Reading a gyroscope/accelerometer/orientation is a `devicemotion` /
//! `deviceorientation` listener, which works in mobile browsers and both
//! WebView shells. What the script adds is the iOS 13+ permission gesture, a
//! single shared listener per event, normalized units, and — like the camera
//! script — stopping the listener when instant navigation replaces the page.

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;

use crate::serve::live_reload::rfind_ascii_case_insensitive;
use crate::serve::prefetch;

/// Client JS — compiled into the binary, like `camera.js`.
pub const SENSORS_SCRIPT: &str = include_str!("sensors.js");

/// Makes injection idempotent when a body is rewrapped.
const INJECTED_MARKER: &str = "__soli_sensors_injected";

/// The `motion_sensors()` meta marker that turns the feature on for a page.
const SENSORS_META: &str = "soli-sensors";
/// A bare inline use of the API also opts a page in, so a page that just calls
/// `soli.sensors.gyroscope(...)` needs no separate enabling helper.
const SENSORS_USAGE: &str = "soli.sensors";

fn sensors_hash() -> u64 {
    use std::sync::OnceLock;
    static HASH: OnceLock<u64> = OnceLock::new();
    *HASH.get_or_init(|| prefetch::fnv1a_64(SENSORS_SCRIPT.as_bytes()))
}

/// Does this page use motion sensors? A page that does not gets no script.
pub fn page_uses_sensors(html: &str) -> bool {
    html.contains(SENSORS_META) || html.contains(SENSORS_USAGE)
}

fn sensors_tag() -> String {
    format!(
        "<!-- {} --><script src=\"/__soli/sensors.js?v={:016x}\" defer></script>",
        INJECTED_MARKER,
        sensors_hash()
    )
}

/// Insert the sensors `<script>` before `</body>` — or `</html>`, or at the end.
pub fn inject_sensors_tag(html: &str) -> String {
    if html.contains(INJECTED_MARKER) || !page_uses_sensors(html) {
        return html.to_string();
    }
    let tag = sensors_tag();
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

/// `GET /__soli/sensors.js`.
pub fn handle_sensors_js() -> Response<Full<Bytes>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/javascript; charset=utf-8")
        .header("Cache-Control", "public, max-age=86400, immutable")
        .body(Full::new(Bytes::from_static(SENSORS_SCRIPT.as_bytes())))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITH_META: &str =
        "<html><head><meta name=\"soli-sensors\" content=\"1\"></head><body><p>hi</p></body></html>";
    const WITH_USAGE: &str =
        "<html><body><script>soli.sensors.gyroscope(function(){});</script></body></html>";

    #[test]
    fn injects_for_a_page_that_enabled_sensors() {
        let out = inject_sensors_tag(WITH_META);
        assert!(out.contains("/__soli/sensors.js?v="));
        assert!(out.find("sensors.js").unwrap() < out.find("</body>").unwrap());
    }

    /// Inline use of the API is enough — no separate enabling helper required.
    #[test]
    fn injects_for_a_page_that_only_uses_the_api() {
        let out = inject_sensors_tag(WITH_USAGE);
        assert!(out.contains("/__soli/sensors.js?v="));
    }

    /// A page that wants nothing must not download a sensors script.
    #[test]
    fn does_not_inject_otherwise() {
        let plain = "<html><body><p>hi</p></body></html>";
        assert_eq!(inject_sensors_tag(plain), plain);
    }

    #[test]
    fn injection_is_idempotent() {
        let once = inject_sensors_tag(WITH_META);
        assert_eq!(inject_sensors_tag(&once), once);
    }
}
