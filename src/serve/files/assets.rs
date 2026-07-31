//! The CSS and JS behind the generated pages.
//!
//! Compiled into the binary (same `include_str!` pattern as
//! [`crate::serve::nav`]) and served from reserved paths, so a generated page
//! makes no network request at all — `soli serve` has to work offline.

use bytes::Bytes;
use hyper::{Response, StatusCode};

use super::{full, ResponseBody};
use crate::serve::prefetch::fnv1a_64;

pub(crate) const FILES_CSS: &str = include_str!("files.css");
pub(crate) const FILES_JS: &str = include_str!("files.js");

pub(crate) fn handle_files_css(if_none_match: Option<&str>) -> Response<ResponseBody> {
    asset(FILES_CSS, "text/css; charset=utf-8", if_none_match)
}

pub(crate) fn handle_files_js(if_none_match: Option<&str>) -> Response<ResponseBody> {
    asset(
        FILES_JS,
        "application/javascript; charset=utf-8",
        if_none_match,
    )
}

/// A content-hashed ETag lets a browser skip the bytes on every page after the
/// first, while a new binary (new content, new hash) invalidates immediately.
fn asset(
    body: &'static str,
    content_type: &'static str,
    if_none_match: Option<&str>,
) -> Response<ResponseBody> {
    let etag = format!("\"{:x}\"", fnv1a_64(body.as_bytes()));

    if if_none_match.is_some_and(|client| client == etag || client == format!("W/{}", etag)) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header("ETag", etag)
            .body(full(Bytes::new()))
            .unwrap();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", content_type)
        .header("Content-Length", body.len().to_string())
        .header("ETag", etag)
        .header("Cache-Control", "public, max-age=0, must-revalidate")
        .body(full(Bytes::from_static(body.as_bytes())))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_are_not_empty() {
        assert!(FILES_CSS.contains("--solar"));
        assert!(FILES_JS.contains("soli-files-theme"));
    }

    #[test]
    fn conditional_requests_match_the_served_etag() {
        let resp = handle_files_css(None);
        assert_eq!(resp.status(), StatusCode::OK);
        let etag = resp
            .headers()
            .get("ETag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        assert_eq!(
            handle_files_css(Some(&etag)).status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(
            handle_files_css(Some(&format!("W/{}", etag))).status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(
            handle_files_css(Some("\"deadbeef\"")).status(),
            StatusCode::OK
        );
    }
}
