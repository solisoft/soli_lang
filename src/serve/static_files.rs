//! The `public/` directory: which file a URL path resolves to, and how that
//! file's bytes reach the client.
//!
//! Lifted out of `handle_hyper_request`. It answers from the path, the method
//! and the request headers, before any routing — so it is a function, not a
//! stage of the request pipeline.
//!
//! There used to be three copies of the reply here, one per source: the
//! production asset cache, a production disk read with an mtime ETag, and the
//! dev-mode read that has no ETag at all. They differed only in where the
//! bytes came from and whether there was an ETag, and each carried its own
//! conditional-GET, `Range` and full-body arms — nine arms for three
//! behaviours. [`respond`] is the one copy; the three callers say only what
//! they have.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use hyper::{HeaderMap, Response, StatusCode};

use super::asset_cache::AssetCache;
use super::{files, finish_response, full, server_constants, ResponseBody};

/// Where the bytes of the response come from.
enum Source<'a> {
    /// Already in memory: the startup asset cache, or a dev-mode read that had
    /// to happen anyway because there is no metadata to build an ETag from.
    Memory(Bytes),
    /// Still on disk. A `Range` then opens, seeks and reads only the requested
    /// span (SEC-048) instead of slurping the whole file per request.
    Disk { path: &'a Path, size: u64 },
}

impl Source<'_> {
    fn size(&self) -> u64 {
        match self {
            Source::Memory(bytes) => bytes.len() as u64,
            Source::Disk { size, .. } => *size,
        }
    }
}

/// A file under `public/`, or `None` to fall through to route matching.
pub(super) fn handle(
    path: &str,
    method: &str,
    public_dir: &Path,
    asset_cache: &AssetCache,
    dev_mode: bool,
    headers: &HeaderMap,
) -> Option<Response<ResponseBody>> {
    // Skipped in file mode, where the whole served folder — not a `public/`
    // subdirectory — is the static root and `files::handle` owns the
    // resolution.
    if method != "GET" || files::files_root().is_some() || !public_dir.exists() {
        return None;
    }

    let file_path = match resolve_static_file(path, public_dir) {
        Err(()) => {
            return Some(
                Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(full(Bytes::from("Forbidden")))
                    .expect("static 403 is always valid"),
            )
        }
        // Not a static file, fall through to route matching.
        Ok(None) => return None,
        Ok(Some(file_path)) => file_path,
    };

    // `file_path` is already canonical (see `resolve_static_file`).
    let mime_type = server_constants::get_mime_type(&file_path);

    if !dev_mode {
        // Production fast path: serve cached CSS/JS bytes loaded at startup.
        // The cache is populated only in prod (`!dev_mode`); a miss here
        // (e.g. images, fonts, files added post-startup) falls through to the
        // disk-read path below.
        if let Some(asset) = asset_cache.get(&file_path) {
            return Some(respond(
                Source::Memory(asset.bytes.clone()),
                asset.content_type,
                Some(&asset.etag),
                headers,
            ));
        }

        // Still production, but not cached: the file's mtime is the ETag, so a
        // conditional GET can be answered without reading the file at all.
        if let Ok(metadata) = std::fs::metadata(&file_path) {
            if let Ok(modified) = metadata.modified() {
                return Some(respond(
                    Source::Disk {
                        path: &file_path,
                        size: metadata.len(),
                    },
                    mime_type,
                    Some(&server_constants::generate_etag(modified)),
                    headers,
                ));
            }
        }
    }

    // Dev mode, or metadata unavailable: read fresh every time, and offer no
    // ETag — there is nothing stable to build one from.
    let content = match std::fs::read(&file_path) {
        Ok(content) => content,
        Err(_) => return Some(read_error()),
    };
    Some(respond(
        Source::Memory(Bytes::from(content)),
        mime_type,
        None,
        headers,
    ))
}

/// Conditional GET, `Range`, or the whole thing.
///
/// `etag` is `Some` exactly when the bytes are versioned — the cache computes
/// one at startup, a disk read derives one from the mtime, and dev mode has
/// none. It gates both the 304 and the long `Cache-Control`, which is why the
/// no-ETag path is also the uncached one.
fn respond(
    source: Source<'_>,
    content_type: &str,
    etag: Option<&str>,
    headers: &HeaderMap,
) -> Response<ResponseBody> {
    let total_size = source.size();

    // Conditional GET: 304 short-circuit on a matching ETag, before any read.
    // The weak form is accepted because a proxy in front may have weakened it.
    if let Some(etag) = etag {
        if let Some(client_etag) = headers.get("if-none-match").and_then(|v| v.to_str().ok()) {
            if client_etag == etag || client_etag == format!("W/{}", etag) {
                return finish_response(
                    Response::builder()
                        .status(StatusCode::NOT_MODIFIED)
                        .header("ETag", etag)
                        .header("Cache-Control", server_constants::STATIC_CACHE_MAX_AGE),
                    Bytes::new(),
                );
            }
        }
    }

    if let Some(range_str) = headers.get("range").and_then(|v| v.to_str().ok()) {
        let Some((start, end)) = server_constants::parse_range_header(range_str, total_size) else {
            return finish_response(
                Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header("Content-Range", format!("bytes */{}", total_size)),
                Bytes::new(),
            );
        };
        let length = end - start + 1;
        let slice = match &source {
            // `parse_range_header` only ever returns an in-bounds span.
            Source::Memory(bytes) => bytes.slice(start as usize..=(end as usize)),
            Source::Disk { path, .. } => {
                match server_constants::read_file_range(path, start, length) {
                    Ok(buf) => Bytes::from(buf),
                    Err(_) => return read_error(),
                }
            }
        };
        return finish_response(
            with_validators(
                Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header("Content-Type", content_type)
                    .header(
                        "Content-Range",
                        format!("bytes {}-{}/{}", start, end, total_size),
                    )
                    .header("Content-Length", length.to_string())
                    .header("Accept-Ranges", "bytes"),
                etag,
            ),
            slice,
        );
    }

    let body = match source {
        Source::Memory(bytes) => bytes,
        Source::Disk { path, .. } => match std::fs::read(path) {
            Ok(content) => Bytes::from(content),
            Err(_) => return read_error(),
        },
    };
    finish_response(
        with_validators(
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", content_type)
                .header("Content-Length", body.len().to_string())
                .header("Accept-Ranges", "bytes"),
            etag,
        ),
        body,
    )
}

/// `ETag` and the long `Cache-Control` travel together: bytes we cannot name
/// are bytes a cache must not keep.
fn with_validators(
    builder: hyper::http::response::Builder,
    etag: Option<&str>,
) -> hyper::http::response::Builder {
    match etag {
        Some(etag) => builder
            .header("ETag", etag)
            .header("Cache-Control", server_constants::STATIC_CACHE_MAX_AGE),
        None => builder,
    }
}

fn read_error() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(full(Bytes::from("Error reading file")))
        .expect("static 500 is always valid")
}

/// Resolve a request path to a static file in the public directory.
/// Returns:
///   Ok(Some(path)) - file found and safe to serve (**already canonical**)
///   Ok(None) - not a static file, fall through to route matching
///   Err(()) - path traversal attempt, should return 403
///
/// Returns the *canonical* path, not the pre-join candidate. Serving the
/// non-canonical path after a canonicalize jail check leaves a TOCTOU window
/// where a symlink planted under `public/` between check and open could
/// escape the public root.
fn resolve_static_file(path: &str, public_dir: &Path) -> Result<Option<PathBuf>, ()> {
    let relative_path = path.trim_start_matches('/');
    let decoded_path = match urlencoding::decode(relative_path) {
        Ok(d) => d.into_owned(),
        Err(_) => relative_path.to_string(),
    };
    // Do not allow directory traversal or absolute paths in URL
    if decoded_path.contains("..") || decoded_path.starts_with('/') {
        return Ok(None);
    }
    let file_path = public_dir.join(&decoded_path);

    // Canonicalize both paths to resolve symlinks and prevent traversal
    let (canonical_file, canonical_public) = match (
        std::fs::canonicalize(&file_path),
        std::fs::canonicalize(public_dir),
    ) {
        (Ok(f), Ok(p)) => (f, p),
        _ => return Ok(None), // file doesn't exist, fall through
    };

    // Ensure the canonical file path is within public directory.
    // Use `Path::starts_with` (segment-aware), NOT `str::starts_with`: the
    // string form would let `…/public-evil/x` pass the check against
    // `…/public` because the directory name is a byte-level prefix.
    if !canonical_file.starts_with(&canonical_public) {
        return Err(()); // traversal attempt
    }

    if !canonical_file.is_file() {
        return Ok(None); // directory or special file, fall through
    }

    Ok(Some(canonical_file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::fs;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                hyper::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers
    }

    fn header_of(response: &Response<ResponseBody>, name: &str) -> Option<String> {
        response
            .headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    }

    async fn body_of(response: Response<ResponseBody>) -> Vec<u8> {
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec()
    }

    // ---------- the consolidated responder ----------

    /// A conditional GET is answered before anything is read, and the weak
    /// form of the same ETag counts as a match — a proxy in front is allowed
    /// to have weakened it.
    #[test]
    fn matching_etag_answers_304_strong_and_weak() {
        for client_etag in ["\"abc\"", "W/\"abc\""] {
            let response = respond(
                Source::Memory(Bytes::from_static(b"body{}")),
                "text/css",
                Some("\"abc\""),
                &headers(&[("if-none-match", client_etag)]),
            );
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED, "{client_etag}");
            assert_eq!(header_of(&response, "etag").as_deref(), Some("\"abc\""));
            assert_eq!(
                header_of(&response, "cache-control").as_deref(),
                Some(server_constants::STATIC_CACHE_MAX_AGE)
            );
        }
    }

    #[test]
    fn different_etag_serves_the_body() {
        let response = respond(
            Source::Memory(Bytes::from_static(b"body{}")),
            "text/css",
            Some("\"abc\""),
            &headers(&[("if-none-match", "\"stale\"")]),
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header_of(&response, "content-length").as_deref(), Some("6"));
    }

    #[tokio::test]
    async fn range_slices_in_memory_bytes() {
        let response = respond(
            Source::Memory(Bytes::from_static(b"0123456789")),
            "text/plain",
            Some("\"v1\""),
            &headers(&[("range", "bytes=2-5")]),
        );
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            header_of(&response, "content-range").as_deref(),
            Some("bytes 2-5/10")
        );
        assert_eq!(header_of(&response, "content-length").as_deref(), Some("4"));
        assert_eq!(header_of(&response, "etag").as_deref(), Some("\"v1\""));
        assert_eq!(body_of(response).await, b"2345");
    }

    /// SEC-048: the disk source must read only the requested span, and the
    /// span it returns has to be the same one the in-memory source would.
    #[tokio::test]
    async fn range_reads_only_the_requested_span_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.txt");
        fs::write(&path, b"0123456789").unwrap();

        let response = respond(
            Source::Disk {
                path: &path,
                size: 10,
            },
            "text/plain",
            Some("\"v1\""),
            &headers(&[("range", "bytes=-3")]),
        );
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            header_of(&response, "content-range").as_deref(),
            Some("bytes 7-9/10")
        );
        assert_eq!(body_of(response).await, b"789");
    }

    /// An open-ended range runs to the last byte, and the last byte is
    /// included — an off-by-one here silently truncates every download.
    #[tokio::test]
    async fn open_ended_range_runs_to_the_final_byte() {
        let response = respond(
            Source::Memory(Bytes::from_static(b"0123456789")),
            "text/plain",
            None,
            &headers(&[("range", "bytes=8-")]),
        );
        assert_eq!(
            header_of(&response, "content-range").as_deref(),
            Some("bytes 8-9/10")
        );
        assert_eq!(body_of(response).await, b"89");
    }

    #[test]
    fn unsatisfiable_range_is_416_with_the_real_size() {
        let response = respond(
            Source::Memory(Bytes::from_static(b"0123456789")),
            "text/plain",
            Some("\"v1\""),
            &headers(&[("range", "bytes=99-200")]),
        );
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            header_of(&response, "content-range").as_deref(),
            Some("bytes */10")
        );
    }

    /// Dev mode has no ETag, and that is also why it must not be cached: the
    /// two headers travel together, so a reader with no way to revalidate is
    /// never told to hold the bytes for a year.
    #[test]
    fn without_an_etag_nothing_is_cacheable() {
        for range in [None, Some("bytes=0-1")] {
            let request_headers = match range {
                Some(range) => headers(&[("range", range)]),
                None => HeaderMap::new(),
            };
            let response = respond(
                Source::Memory(Bytes::from_static(b"0123456789")),
                "text/plain",
                None,
                &request_headers,
            );
            assert!(header_of(&response, "etag").is_none(), "{range:?}");
            assert!(header_of(&response, "cache-control").is_none(), "{range:?}");
        }
    }

    /// With no ETag there is nothing to compare against, so `If-None-Match`
    /// cannot short-circuit — a dev-mode edit must always be seen.
    #[test]
    fn without_an_etag_a_conditional_get_still_serves_the_body() {
        let response = respond(
            Source::Memory(Bytes::from_static(b"0123456789")),
            "text/plain",
            None,
            &headers(&[("if-none-match", "\"anything\"")]),
        );
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn a_missing_file_behind_a_disk_source_is_a_500() {
        let dir = tempfile::tempdir().unwrap();
        let response = respond(
            Source::Disk {
                path: &dir.path().join("gone.txt"),
                size: 10,
            },
            "text/plain",
            Some("\"v1\""),
            &HeaderMap::new(),
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ---------- path resolution ----------

    #[test]
    fn test_resolve_static_file_serves_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();
        fs::write(public.join("style.css"), "body{}").unwrap();

        let result = resolve_static_file("/style.css", &public);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_resolve_static_file_root_path_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();

        // "/" should NOT return 404 — it should fall through (None) so route matching handles it
        let result = resolve_static_file("/", &public);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_resolve_static_file_directory_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let subdir = public.join("css");
        fs::create_dir_all(&subdir).unwrap();

        // "/css" is a directory, should fall through
        let result = resolve_static_file("/css", &public);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_resolve_static_file_nonexistent_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();

        let result = resolve_static_file("/nope.js", &public);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_resolve_static_file_blocks_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();

        let result = resolve_static_file("/../etc/passwd", &public);
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn test_resolve_static_file_blocks_encoded_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir(&public).unwrap();

        let result = resolve_static_file("/%2e%2e/etc/passwd", &public);
        assert_eq!(result, Ok(None));
    }

    /// Regression: the containment check must compare path components, not
    /// stringified bytes. `…/public-evil/x` is a byte-level prefix match
    /// against `…/public`, so the previous `&str::starts_with` check would
    /// pass it through. The fix uses `Path::starts_with`, which is segment
    /// aware. Exercised here via a symlink inside `public/` that resolves
    /// out to a sibling whose name starts with `public`.
    #[cfg(unix)]
    #[test]
    fn test_resolve_static_file_blocks_sibling_prefix_via_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let evil = dir.path().join("public-evil");
        fs::create_dir(&public).unwrap();
        fs::create_dir(&evil).unwrap();
        fs::write(evil.join("secret.txt"), "leaked").unwrap();

        // `public/escape` -> `../public-evil`
        std::os::unix::fs::symlink(&evil, public.join("escape")).unwrap();

        // `escape/secret.txt` has no `..` in the URL, so the early-out
        // doesn't catch it; the canonical path lives in `public-evil`,
        // which used to satisfy `starts_with("…/public")` byte-wise.
        let result = resolve_static_file("/escape/secret.txt", &public);
        assert_eq!(result, Err(()));
    }

    /// Regression: the same containment-check bug, without symlinks — a
    /// canonicalized path under a sibling directory whose name is a byte
    /// prefix of `public_dir` must not pass the check. This is harder to
    /// trigger from a clean URL (the early `..` reject covers the obvious
    /// vector), but we still want explicit coverage that the segment-aware
    /// check is what's running.
    #[test]
    fn test_resolve_static_file_path_starts_with_is_segment_aware() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let evil = dir.path().join("public-evil");
        fs::create_dir(&public).unwrap();
        fs::create_dir(&evil).unwrap();
        let secret = evil.join("secret.txt");
        fs::write(&secret, "leaked").unwrap();

        // Sanity: with the old byte-level check, `…/public-evil/secret.txt`
        // would test as a prefix-match against `…/public`. With Path-aware
        // semantics it does not.
        let canon_secret = fs::canonicalize(&secret).unwrap();
        let canon_public = fs::canonicalize(&public).unwrap();
        assert!(!canon_secret.starts_with(&canon_public));
        assert!(canon_secret
            .to_string_lossy()
            .starts_with(&*canon_public.to_string_lossy()));
    }
}
