//! Camera preview and barcode scanning — server half.
//!
//! Ships `camera.js` at `/__soli/camera.js`, injected only into pages that
//! actually use a camera. The gate is the markup itself: a `data-soli-camera`
//! attribute, which `camera_preview(...)` emits.
//!
//! Showing a camera needs no framework at all — `getUserMedia` into a `<video>`
//! is six lines, and works as soon as the shell stops denying it. What the
//! script adds is the part hand-written code reliably forgets: **stopping the
//! tracks**. Instant navigation swaps the body without a page unload, so a
//! stream started by a page that has since been replaced keeps running, and the
//! camera indicator stays lit. That, plus a scan loop that is throttled rather
//! than running at frame rate, is the whole justification for its existence.

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;

use crate::serve::live_reload::rfind_ascii_case_insensitive;
use crate::serve::prefetch;

/// Client JS — compiled into the binary, like `nav.js`.
pub const CAMERA_SCRIPT: &str = include_str!("camera.js");

/// Makes injection idempotent when a body is rewrapped.
const INJECTED_MARKER: &str = "__soli_camera_injected";

/// The attribute that turns the feature on for a page.
const CAMERA_ATTR: &str = "data-soli-camera";

fn camera_hash() -> u64 {
    use std::sync::OnceLock;
    static HASH: OnceLock<u64> = OnceLock::new();
    *HASH.get_or_init(|| prefetch::fnv1a_64(CAMERA_SCRIPT.as_bytes()))
}

/// Does this page use a camera? A page that does not gets no script.
pub fn page_uses_camera(html: &str) -> bool {
    html.contains(CAMERA_ATTR)
}

fn camera_tag() -> String {
    format!(
        "<!-- {} --><script src=\"/__soli/camera.js?v={:016x}\" defer></script>",
        INJECTED_MARKER,
        camera_hash()
    )
}

/// Insert the camera `<script>` before `</body>` — or `</html>`, or at the end.
pub fn inject_camera_tag(html: &str) -> String {
    if html.contains(INJECTED_MARKER) || !page_uses_camera(html) {
        return html.to_string();
    }
    let tag = camera_tag();
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

/// `GET /__soli/camera.js`.
pub fn handle_camera_js() -> Response<Full<Bytes>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/javascript; charset=utf-8")
        .header("Cache-Control", "public, max-age=86400, immutable")
        .body(Full::new(Bytes::from_static(CAMERA_SCRIPT.as_bytes())))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITH_CAMERA: &str = "<html><body><video data-soli-camera autoplay></video></body></html>";

    #[test]
    fn injects_for_a_page_with_a_camera() {
        let out = inject_camera_tag(WITH_CAMERA);
        assert!(out.contains("/__soli/camera.js?v="));
        assert!(out.find("camera.js").unwrap() < out.find("</body>").unwrap());
    }

    /// A page with no camera must not download a camera script.
    #[test]
    fn does_not_inject_otherwise() {
        let plain = "<html><body><p>hi</p></body></html>";
        assert_eq!(inject_camera_tag(plain), plain);
    }

    #[test]
    fn injection_is_idempotent() {
        let once = inject_camera_tag(WITH_CAMERA);
        assert_eq!(inject_camera_tag(&once), once);
    }

    /// Scanning implies a camera, so the scan attribute alone must not slip a
    /// page past the gate without the preview attribute that starts the stream.
    #[test]
    fn the_scan_attribute_travels_with_the_camera_attribute() {
        let scan_only = "<html><body><video data-soli-scan=\"qr_code\"></video></body></html>";
        assert_eq!(inject_camera_tag(scan_only), scan_only);
    }
}
