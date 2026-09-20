//! The scripts and stylesheets the binary serves from itself.
//!
//! `/__soli/nav.js`, `/__soli/prefetch.js`, the native bridge shim, the camera
//! and sensor helpers, the pages `soli serve` generates for a plain directory,
//! and the LiveView client. None of them touches the application: each is a
//! function of the path, the method and a conditional-GET validator.
//!
//! Lifted out of `handle_hyper_request`, which is 1,457 lines. **Two entry
//! points, not one, and that is deliberate:** the LiveView client is served
//! *before* the same-origin gate and everything else *after* it. Merging them
//! would move eight endpoints to the wrong side of a security check, which the
//! comments in that function are explicit about. Each is called where its block
//! already stood.

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

use super::{
    box_full, camera, files, full, native, native_stream_response, nav, prefetch, sensors,
    ResponseBody,
};

/// The LiveView client, embedded so it cannot go stale against the patch
/// protocol it speaks. `no-cache` plus a version ETag: a browser revalidates
/// and gets a 304 until the binary changes.
///
/// Served before the same-origin gate, where its block has always been.
pub(super) fn live_client(
    path: &str,
    method: &str,
    if_none_match: Option<&str>,
) -> Option<Response<ResponseBody>> {
    if path != crate::live::LIVE_CLIENT_PATH || !(method == "GET" || method == "HEAD") {
        return None;
    }
    const ETAG: &str = concat!("\"soli-live-", env!("CARGO_PKG_VERSION"), "\"");

    if if_none_match.is_some_and(|v| v.contains(ETAG)) {
        return Some(
            Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header("ETag", ETAG)
                .header("Cache-Control", "no-cache")
                .body(full(Bytes::new()))
                .expect("static 304 is always valid"),
        );
    }
    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/javascript; charset=utf-8")
            .header("Cache-Control", "no-cache")
            .header("ETag", ETAG)
            .body(full(Bytes::from_static(
                crate::live::LIVE_CLIENT_JS.as_bytes(),
            )))
            .expect("static script response is always valid"),
    )
}

/// Everything else the binary serves from itself, after the same-origin gate.
///
/// `raw_query` is only read by the native bridge's SSE stream, which is the one
/// place a channel is trusted: the signed token is verified before subscribing,
/// and a rejection is a flat 403 rather than an explanation an attacker could
/// probe with.
pub(super) fn bundled(
    path: &str,
    method: &str,
    if_none_match: Option<&str>,
    raw_query: Option<&str>,
) -> Option<Response<ResponseBody>> {
    if method != "GET" {
        return None;
    }

    let response = match path {
        // Hover-prefetch and instant navigation, at reserved paths so a
        // strict-CSP application can use `<script src>` instead of inline JS.
        "/__soli/prefetch.js" => box_full(prefetch::handle_prefetch_js()),
        "/__soli/nav.js" => box_full(nav::handle_nav_js()),

        // Behind the pages `soli serve` generates for a plain directory, so
        // those pages make no network request at all.
        "/__soli/files.css" => files::handle_files_css(if_none_match),
        "/__soli/files.js" => files::handle_files_js(if_none_match),

        // The native bridge: the client shim, the camera and barcode helpers,
        // the motion sensors.
        "/__soli/native.js" => box_full(native::handle_native_js()),
        "/__soli/camera.js" => box_full(camera::handle_camera_js()),
        "/__soli/barcode-decoder.js" => box_full(camera::handle_barcode_decoder_js()),
        "/__soli/sensors.js" => box_full(sensors::handle_sensors_js()),

        "/__soli/native/stream" => match native::topic_for_query(raw_query) {
            Some(topic) => native_stream_response(&topic),
            None => box_full(
                Response::builder()
                    .status(403)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(Full::new(Bytes::from_static(
                        b"native channel token missing, invalid or expired",
                    )))
                    .expect("static 403 is always valid"),
            ),
        },

        _ => return None,
    };
    Some(response)
}
