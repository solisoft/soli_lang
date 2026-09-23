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
    /// Already in memory: the startup asset cache.
    Memory(Bytes),
    /// Still on disk. A `Range` then opens, seeks and reads only the requested
    /// span (SEC-048) instead of slurping the whole file per request, and a
    /// body above `STREAM_THRESHOLD` is streamed rather than read whole.
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
    if method != "GET" || files::files_root().is_some() {
        return None;
    }
    // `/` and anything ending in `/` name a directory, which is never served
    // from here: no filesystem call for the most common dynamic GETs.
    if path.is_empty() || path.ends_with('/') {
        return None;
    }
    // Canonicalised once per thread, not twice per request (`exists()` plus
    // the `canonicalize` inside the jail check). No `public/` at all: nothing
    // to serve.
    let canonical_public = canonical_public_dir(public_dir, false)?;

    // Production fast path, before any filesystem call: the startup asset
    // cache is keyed by canonical path, and for a traversal-free URL path that
    // key is exactly `canonical_public.join(url_path)` (the cache skips
    // symlinks, so a cached key has no link in it to resolve). The bytes were
    // read from inside the jail at boot.
    if !dev_mode && !asset_cache.is_empty() {
        if let Some(relative) = sanitized_relative_path(path) {
            if let Some(asset) = asset_cache.get(&canonical_public.join(relative.as_ref())) {
                return Some(respond(
                    Source::Memory(asset.bytes.clone()),
                    asset.content_type,
                    Some(&asset.etag),
                    headers,
                ));
            }
        }
    }

    let file_path = match resolve_static_file_in(path, public_dir, &canonical_public) {
        // The public directory may itself be (or sit under) a symlink that a
        // deploy re-pointed since the root was cached. Re-resolve the root
        // once and judge again against the fresh one — the jail is always
        // checked against the directory as it is now.
        Err(()) => match canonical_public_dir(public_dir, true)
            .map(|fresh| resolve_static_file_in(path, public_dir, &fresh))
        {
            Some(Ok(resolved)) => resolved,
            // The directory is gone: nothing to serve, let routing answer.
            None => return None,
            Some(Err(())) => {
                return Some(
                    Response::builder()
                        .status(StatusCode::FORBIDDEN)
                        .body(full(Bytes::from("Forbidden")))
                        .expect("static 403 is always valid"),
                )
            }
        },
        Ok(resolved) => resolved,
    };
    // Not a static file, fall through to route matching.
    let file_path = file_path?;

    // `file_path` is already canonical (see `resolve_static_file`).
    let mime_type = server_constants::get_mime_type(&file_path);

    if !dev_mode {
        // A file the fast path above did not find under its URL spelling
        // (reached through a link inside `public/`, say) may still be cached.
        if let Some(asset) = asset_cache.get(&file_path) {
            return Some(respond(
                Source::Memory(asset.bytes.clone()),
                asset.content_type,
                Some(&asset.etag),
                headers,
            ));
        }
    }

    // Production: the file's mtime is the ETag, so a conditional GET can be
    // answered without reading the file at all. Dev mode offers no ETag —
    // an edit must always be seen — and so is never cached either.
    let metadata = match std::fs::metadata(&file_path) {
        Ok(metadata) => metadata,
        Err(_) => return Some(read_error()),
    };
    let etag = if dev_mode {
        None
    } else {
        metadata
            .modified()
            .ok()
            .map(server_constants::generate_etag)
    };
    Some(respond(
        Source::Disk {
            path: &file_path,
            size: metadata.len(),
        },
        mime_type,
        etag.as_deref(),
        headers,
    ))
}

/// Files above this size are streamed from disk in chunks rather than read
/// whole into memory on the async runtime.
const STREAM_THRESHOLD: u64 = 1024 * 1024;

/// Chunk size for a streamed file body.
const STREAM_CHUNK: usize = 64 * 1024;

/// `length` bytes of `path` from `start`, as a body read on the blocking pool
/// and sent a chunk at a time.
///
/// Reading a large file with `std::fs::read` blocked a runtime thread for the
/// whole read and held the whole file in memory per request, so a few
/// concurrent downloads of a big asset stalled every other connection on those
/// threads. The file is opened here, synchronously, so a failure is still a
/// clean 500 rather than a truncated 200.
fn stream_file(path: &Path, start: u64, length: u64) -> std::io::Result<ResponseBody> {
    use std::io::{Read, Seek, SeekFrom};
    use tokio_stream::StreamExt;

    // `spawn_blocking` needs a runtime; outside one (a unit test) the caller
    // falls back to an in-memory read.
    let runtime =
        tokio::runtime::Handle::try_current().map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut file = std::fs::File::open(path)?;
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<Bytes>>(4);
    runtime.spawn_blocking(move || {
        // Never more than was announced in `Content-Length`, even if the file
        // grew in the meantime.
        let mut reader = file.take(length);
        loop {
            let mut chunk = vec![0u8; STREAM_CHUNK];
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    chunk.truncate(n);
                    if tx.blocking_send(Ok(Bytes::from(chunk))).is_err() {
                        // The client went away.
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    break;
                }
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|chunk| chunk.map(hyper::body::Frame::data));
    Ok(http_body_util::BodyExt::boxed(
        http_body_util::StreamBody::new(stream),
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
        let builder = with_validators(
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
        );
        let slice = match &source {
            // `parse_range_header` only ever returns an in-bounds span.
            Source::Memory(bytes) => bytes.slice(start as usize..=(end as usize)),
            Source::Disk { path, .. } => {
                if length > STREAM_THRESHOLD {
                    if let Ok(body) = stream_file(path, start, length) {
                        return finish_streamed(builder, body);
                    }
                }
                match server_constants::read_file_range(path, start, length) {
                    Ok(buf) => Bytes::from(buf),
                    Err(_) => return read_error(),
                }
            }
        };
        return finish_response(builder, slice);
    }

    let builder = |length: u64| {
        with_validators(
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", content_type)
                .header("Content-Length", length.to_string())
                .header("Accept-Ranges", "bytes"),
            etag,
        )
    };
    let body = match source {
        Source::Memory(bytes) => bytes,
        Source::Disk { path, size } => {
            if size > STREAM_THRESHOLD {
                if let Ok(body) = stream_file(path, 0, size) {
                    return finish_streamed(builder(size), body);
                }
            }
            match std::fs::read(path) {
                Ok(content) => Bytes::from(content),
                Err(_) => return read_error(),
            }
        }
    };
    finish_response(builder(body.len() as u64), body)
}

/// [`finish_response`] for a body that is already a stream.
fn finish_streamed(
    builder: hyper::http::response::Builder,
    body: ResponseBody,
) -> Response<ResponseBody> {
    builder.body(body).unwrap_or_else(|_| read_error())
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

/// Resolve a request path to a static file in the public directory (test
/// entry point; the server passes its cached root to `resolve_static_file_in`).
/// Returns:
///   Ok(Some(path)) - file found and safe to serve (**already canonical**)
///   Ok(None) - not a static file, fall through to route matching
///   Err(()) - path traversal attempt, should return 403
///
/// Returns the *canonical* path, not the pre-join candidate. Serving the
/// non-canonical path after a canonicalize jail check leaves a TOCTOU window
/// where a symlink planted under `public/` between check and open could
/// escape the public root.
#[cfg(test)]
fn resolve_static_file(path: &str, public_dir: &Path) -> Result<Option<PathBuf>, ()> {
    match std::fs::canonicalize(public_dir) {
        Ok(canonical_public) => resolve_static_file_in(path, public_dir, &canonical_public),
        Err(_) => Ok(None),
    }
}

/// The URL path as a path relative to `public/`: percent-decoded, and `None`
/// for anything that tries to leave it (`..`, or absolute once decoded).
fn sanitized_relative_path(path: &str) -> Option<std::borrow::Cow<'_, str>> {
    let relative_path = path.trim_start_matches('/');
    let decoded_path = match urlencoding::decode(relative_path) {
        Ok(decoded) => decoded,
        Err(_) => std::borrow::Cow::Borrowed(relative_path),
    };
    // Do not allow directory traversal or absolute paths in URL
    if decoded_path.contains("..") || decoded_path.starts_with('/') {
        return None;
    }
    Some(decoded_path)
}

/// `public_dir`, canonicalised — cached per thread and per directory, since it
/// is the same answer for every request. `refresh` re-resolves it (a deploy
/// may have re-pointed a symlink on the way). `None` when the directory does
/// not exist; that answer is not cached, so a `public/` created later is
/// picked up.
fn canonical_public_dir(public_dir: &Path, refresh: bool) -> Option<PathBuf> {
    thread_local! {
        static CANONICAL: std::cell::RefCell<Vec<(PathBuf, PathBuf)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    CANONICAL.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !refresh {
            if let Some((_, canonical)) = cache.iter().find(|(dir, _)| dir == public_dir) {
                return Some(canonical.clone());
            }
        }
        cache.retain(|(dir, _)| dir != public_dir);
        let canonical = std::fs::canonicalize(public_dir).ok()?;
        cache.push((public_dir.to_path_buf(), canonical.clone()));
        Some(canonical)
    })
}

/// Resolve a request path against an already-canonical public root: same
/// contract as `resolve_static_file` — `Ok(Some(canonical))` to serve,
/// `Ok(None)` to fall through, `Err(())` for an escape from the jail.
fn resolve_static_file_in(
    path: &str,
    public_dir: &Path,
    canonical_public: &Path,
) -> Result<Option<PathBuf>, ()> {
    let Some(decoded_path) = sanitized_relative_path(path) else {
        return Ok(None);
    };
    let file_path = public_dir.join(decoded_path.as_ref());

    // Canonicalize to resolve symlinks and prevent traversal
    let canonical_file = match std::fs::canonicalize(&file_path) {
        Ok(f) => f,
        Err(_) => return Ok(None), // file doesn't exist, fall through
    };

    // Ensure the canonical file path is within public directory.
    // Use `Path::starts_with` (segment-aware), NOT `str::starts_with`: the
    // string form would let `…/public-evil/x` pass the check against
    // `…/public` because the directory name is a byte-level prefix.
    if !canonical_file.starts_with(canonical_public) {
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

    /// A file above the streaming threshold is sent in chunks from the
    /// blocking pool — the bytes and the length must still be the file's.
    #[tokio::test]
    async fn a_large_file_is_streamed_whole_and_in_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let size = STREAM_THRESHOLD as usize * 2 + 123;
        let content: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        fs::write(&path, &content).unwrap();

        let response = respond(
            Source::Disk {
                path: &path,
                size: size as u64,
            },
            "application/octet-stream",
            Some("\"v1\""),
            &HeaderMap::new(),
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header_of(&response, "content-length"),
            Some(size.to_string())
        );
        assert_eq!(body_of(response).await, content);

        let start = 1000usize;
        let end = start + STREAM_THRESHOLD as usize + 10;
        let range = format!("bytes={start}-{end}");
        let response = respond(
            Source::Disk {
                path: &path,
                size: size as u64,
            },
            "application/octet-stream",
            Some("\"v1\""),
            &headers(&[("range", range.as_str())]),
        );
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(body_of(response).await, &content[start..=end]);
    }

    /// The production asset cache answers by URL path before any filesystem
    /// call — and a traversal spelling never reaches it.
    #[test]
    fn the_asset_cache_answers_by_url_path() {
        if files::files_root().is_some() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        fs::create_dir_all(public.join("css")).unwrap();
        fs::write(public.join("css/app.css"), "on disk").unwrap();
        let canonical = fs::canonicalize(&public).unwrap();
        let mut map = std::collections::HashMap::new();
        map.insert(
            canonical.join("css/app.css"),
            super::super::asset_cache::CachedAsset {
                bytes: Bytes::from_static(b"cached at boot"),
                etag: "\"e1\"".to_string(),
                content_type: "text/css",
            },
        );
        let cache: AssetCache = std::sync::Arc::new(map);

        let response = handle(
            "/css/app.css",
            "GET",
            &public,
            &cache,
            false,
            &HeaderMap::new(),
        )
        .expect("served");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header_of(&response, "etag").as_deref(), Some("\"e1\""));
        assert_eq!(
            header_of(&response, "content-length").as_deref(),
            Some("14")
        );

        assert!(handle(
            "/css/../css/app.css",
            "GET",
            &public,
            &cache,
            false,
            &HeaderMap::new()
        )
        .is_none());
        // Directories and `/` never touch the disk and fall through.
        assert!(handle("/", "GET", &public, &cache, false, &HeaderMap::new()).is_none());
        assert!(handle("/css/", "GET", &public, &cache, false, &HeaderMap::new()).is_none());
    }

    #[test]
    fn the_canonical_public_root_is_cached_and_refreshable() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        assert!(
            canonical_public_dir(&public, false).is_none(),
            "missing dir"
        );
        fs::create_dir(&public).unwrap();
        // A missing directory was not cached as missing.
        let first = canonical_public_dir(&public, false).expect("now exists");
        assert_eq!(first, fs::canonicalize(&public).unwrap());
        assert_eq!(canonical_public_dir(&public, false), Some(first.clone()));
        assert_eq!(canonical_public_dir(&public, true), Some(first));
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
