/// Server configuration constants
use std::path::Path;
use std::time::SystemTime;

/// Default number of worker threads if CPU parallelism cannot be detected
pub const DEFAULT_WORKER_COUNT: usize = 4;

/// Capacity per worker for request queue (bounded channels for backpressure)
pub const CAPACITY_PER_WORKER: usize = 64;

/// Batch size for processing operations
pub const BATCH_SIZE: usize = 64;

/// Request timeout in seconds
pub const REQUEST_TIMEOUT_SECS: u64 = 5;

/// Maximum time the HTTP handler waits for a worker thread's response before
/// giving up and returning 504. Bounds the otherwise-unbounded wait on the
/// worker reply channel: if a worker parks in a blocking DB/HTTP call or a
/// lock, the request would hang forever ("pending" in the browser) with the
/// system idle. MUST exceed the 30s outbound HTTP/DB client timeouts so an
/// inner timeout fires first with a precise error; this is the backstop for a
/// genuinely wedged worker.
pub const RESPONSE_WAIT_TIMEOUT_SECS: u64 = 40;

/// Heartbeat acknowledgment timeout in seconds
pub const HEARTBEAT_TIMEOUT_SECS: u64 = 5;

/// Hot reload file check interval in seconds
#[allow(dead_code)]
pub const HOT_RELOAD_CHECK_INTERVAL_SECS: u64 = 1;

/// Static file cache control max-age for production (1 year in seconds)
pub const STATIC_CACHE_MAX_AGE: &str = "public, max-age=31536000, immutable";

/// MIME types for static file serving
pub const MIME_TYPES: &[(&str, &str)] = &[
    ("css", "text/css"),
    ("js", "application/javascript"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("ico", "image/x-icon"),
    ("svg", "image/svg+xml"),
    ("html", "text/html"),
    ("json", "application/json"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("ttf", "font/ttf"),
    ("gif", "image/gif"),
    ("mp4", "video/mp4"),
    ("webm", "video/webm"),
    ("ogg", "video/ogg"),
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wav"),
];

/// Extensions that are considered static files for hot reload
pub const STATIC_FILE_EXTENSIONS: &[&str] = &[
    "css", "js", "svg", "ico", "png", "jpg", "jpeg", "gif", "woff", "woff2", "ttf",
];

/// Valid static file extensions for serving
pub const VALID_STATIC_EXTENSIONS: &[&str] = &[
    "css", "js", "svg", "ico", "png", "jpg", "jpeg", "gif", "woff", "woff2", "ttf", "html", "json",
    "mp4", "webm", "ogg", "mp3", "wav",
];

/// HTTP success status code range start (inclusive)
#[allow(dead_code)]
pub const HTTP_SUCCESS_RANGE_START: u16 = 200;

/// HTTP success status code range end (inclusive)
#[allow(dead_code)]
pub const HTTP_SUCCESS_RANGE_END: u16 = 299;

/// WebSocket event channel capacity
#[allow(dead_code)]
pub const WS_EVENT_CHANNEL_CAPACITY: usize = 16;

/// LiveView event channel capacity
#[allow(dead_code)]
pub const LV_EVENT_CHANNEL_CAPACITY: usize = 32;

/// LiveView message channel capacity
#[allow(dead_code)]
pub const LV_MESSAGE_CHANNEL_CAPACITY: usize = 32;

/// Broadcast channel capacity for live reload
#[allow(dead_code)]
pub const LIVE_RELOAD_BROADCAST_CAPACITY: usize = 16;

/// Get the MIME type for a file based on its extension.
pub fn get_mime_type(file_path: &Path) -> &'static str {
    file_path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|ext| MIME_TYPES.iter().find(|(k, _)| *k == ext).map(|(_, v)| *v))
        .unwrap_or("application/octet-stream")
}

/// Generate an ETag from a file's modification time.
pub fn generate_etag(modified: SystemTime) -> String {
    let secs = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("\"{:x}\"", secs)
}

/// Check if an extension is a valid static file extension.
#[allow(dead_code)]
pub fn is_static_extension(ext: &str) -> bool {
    VALID_STATIC_EXTENSIONS.contains(&ext)
}

/// Check if a file extension is tracked for hot reload.
pub fn is_tracked_static_extension(ext: &str) -> bool {
    STATIC_FILE_EXTENSIONS.contains(&ext)
}

/// Parse an HTTP Range header value like "bytes=0-1023" or "bytes=1024-" or "bytes=-500".
/// Returns (start, end_inclusive) for the byte range, clamped to file_size.
/// Returns None if the header is malformed or unsatisfiable.
pub fn parse_range_header(range_header: &str, file_size: u64) -> Option<(u64, u64)> {
    let range_str = range_header.strip_prefix("bytes=")?;
    // Only support a single range (no multi-range)
    if range_str.contains(',') {
        return None;
    }
    let (start_str, end_str) = range_str.split_once('-')?;
    if start_str.is_empty() {
        // Suffix range: "bytes=-500" means last 500 bytes
        let suffix_len: u64 = end_str.parse().ok()?;
        if suffix_len == 0 || suffix_len > file_size {
            return None;
        }
        Some((file_size - suffix_len, file_size - 1))
    } else {
        let start: u64 = start_str.parse().ok()?;
        if start >= file_size {
            return None;
        }
        let end = if end_str.is_empty() {
            file_size - 1
        } else {
            let e: u64 = end_str.parse().ok()?;
            e.min(file_size - 1)
        };
        if start > end {
            return None;
        }
        Some((start, end))
    }
}

/// SEC-048: read a byte range from a file without slurping the entire
/// file into memory.
///
/// The production cache-miss path used to do `std::fs::read(path)` and
/// then slice — so a 1-byte Range request against a 1 GiB asset
/// allocated 1 GiB per request. Repeated tiny-range requests amplified
/// into a memory-pressure DoS. Open + seek + `read_exact` bounds the
/// allocation to the requested span; the page cache still amortizes the
/// disk I/O.
///
/// Caller is responsible for ensuring `start` and `length` are within
/// the file (typically by going through `parse_range_header`).
pub fn read_file_range(
    path: &std::path::Path,
    start: u64,
    length: u64,
) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let len = usize::try_from(length)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "range too large"))?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    // ---------- response wait timeout ----------

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn response_wait_timeout_exceeds_outbound_client_timeout() {
        // The handler's response-wait timeout MUST be longer than the 30s
        // outbound DB/HTTP client timeouts so an inner timeout fires first
        // with a precise error, leaving this as the wedged-worker backstop.
        // (Const assertion is intentional — it guards the invariant at the
        // place a future edit to the constant would break it.)
        const OUTBOUND_CLIENT_TIMEOUT_SECS: u64 = 30;
        assert!(
            RESPONSE_WAIT_TIMEOUT_SECS > OUTBOUND_CLIENT_TIMEOUT_SECS,
            "RESPONSE_WAIT_TIMEOUT_SECS ({RESPONSE_WAIT_TIMEOUT_SECS}) must exceed the 30s client timeout"
        );
    }

    // ---------- get_mime_type ----------

    #[test]
    fn mime_known_extensions() {
        let cases = [
            ("style.css", "text/css"),
            ("app.js", "application/javascript"),
            ("logo.png", "image/png"),
            ("photo.jpg", "image/jpeg"),
            ("photo.jpeg", "image/jpeg"),
            ("favicon.ico", "image/x-icon"),
            ("icon.svg", "image/svg+xml"),
            ("page.html", "text/html"),
            ("data.json", "application/json"),
            ("font.woff2", "font/woff2"),
            ("song.mp3", "audio/mpeg"),
        ];
        for (path, expected) in cases {
            assert_eq!(get_mime_type(&PathBuf::from(path)), expected, "for {path}");
        }
    }

    #[test]
    fn mime_unknown_extension_falls_back_to_octet_stream() {
        assert_eq!(
            get_mime_type(&PathBuf::from("file.xyz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn mime_no_extension_falls_back_to_octet_stream() {
        assert_eq!(
            get_mime_type(&PathBuf::from("README")),
            "application/octet-stream"
        );
    }

    #[test]
    fn mime_extension_match_is_case_sensitive() {
        // Pin the current behavior: "PNG" (uppercase) is NOT recognised.
        // Path::extension preserves case; the table is lowercase-only.
        assert_eq!(
            get_mime_type(&PathBuf::from("logo.PNG")),
            "application/octet-stream"
        );
    }

    // ---------- generate_etag ----------

    #[test]
    fn etag_is_quoted_hex_seconds_since_epoch() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(0xDEAD);
        assert_eq!(generate_etag(t), "\"dead\"");
    }

    #[test]
    fn etag_for_unix_epoch_is_zero() {
        assert_eq!(generate_etag(SystemTime::UNIX_EPOCH), "\"0\"");
    }

    #[test]
    fn etag_pre_epoch_falls_back_to_zero() {
        // Times before UNIX_EPOCH yield Err from duration_since; the
        // function uses unwrap_or_default → 0 secs → "0".
        let pre = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(generate_etag(pre), "\"0\"");
    }

    // ---------- extension predicates ----------

    #[test]
    fn is_static_extension_recognises_common_assets() {
        for ext in ["css", "js", "html", "json", "png", "mp3", "wav", "mp4"] {
            assert!(is_static_extension(ext), "expected {ext} to be static");
        }
    }

    #[test]
    fn is_static_extension_rejects_unknown() {
        assert!(!is_static_extension("xyz"));
        assert!(!is_static_extension(""));
        // Case-sensitive: uppercase variants are not recognised.
        assert!(!is_static_extension("CSS"));
    }

    #[test]
    fn is_tracked_extension_subset_excludes_html_json_video_audio() {
        // The "tracked" list is for hot-reload watching — code
        // assets only, not media or HTML/JSON.
        assert!(is_tracked_static_extension("css"));
        assert!(is_tracked_static_extension("js"));
        assert!(is_tracked_static_extension("png"));

        // These ARE valid static extensions but NOT tracked for hot reload.
        assert!(is_static_extension("html"));
        assert!(!is_tracked_static_extension("html"));
        assert!(is_static_extension("json"));
        assert!(!is_tracked_static_extension("json"));
        assert!(is_static_extension("mp4"));
        assert!(!is_tracked_static_extension("mp4"));
    }

    // ---------- parse_range_header ----------

    #[test]
    fn range_full_form() {
        assert_eq!(parse_range_header("bytes=0-1023", 2048), Some((0, 1023)));
        assert_eq!(parse_range_header("bytes=10-99", 1000), Some((10, 99)));
    }

    #[test]
    fn range_clamps_end_to_file_size_minus_one() {
        // End larger than file gets clamped.
        assert_eq!(parse_range_header("bytes=0-9999", 100), Some((0, 99)));
    }

    #[test]
    fn range_open_ended_uses_file_size_minus_one() {
        // "bytes=1024-" means from 1024 to end of file.
        assert_eq!(parse_range_header("bytes=1024-", 2048), Some((1024, 2047)));
    }

    #[test]
    fn range_suffix_form() {
        // "bytes=-500" means last 500 bytes.
        assert_eq!(parse_range_header("bytes=-500", 2000), Some((1500, 1999)));
    }

    #[test]
    fn range_suffix_zero_is_unsatisfiable() {
        assert!(parse_range_header("bytes=-0", 1000).is_none());
    }

    #[test]
    fn range_suffix_larger_than_file_is_unsatisfiable() {
        // Prevents underflow on file_size - suffix_len.
        assert!(parse_range_header("bytes=-2000", 1000).is_none());
    }

    #[test]
    fn range_start_at_or_past_file_size_is_unsatisfiable() {
        assert!(parse_range_header("bytes=1000-", 1000).is_none());
        assert!(parse_range_header("bytes=2000-", 1000).is_none());
    }

    #[test]
    fn range_start_greater_than_end_is_unsatisfiable() {
        assert!(parse_range_header("bytes=500-100", 1000).is_none());
    }

    #[test]
    fn range_multi_range_is_rejected() {
        // The implementation explicitly does not support multi-range.
        assert!(parse_range_header("bytes=0-100,200-300", 1000).is_none());
    }

    #[test]
    fn range_missing_bytes_prefix_is_rejected() {
        assert!(parse_range_header("0-1023", 2048).is_none());
        assert!(parse_range_header("octets=0-1023", 2048).is_none());
    }

    #[test]
    fn range_missing_dash_is_rejected() {
        // No `-` separator at all.
        assert!(parse_range_header("bytes=100", 1000).is_none());
    }

    #[test]
    fn range_non_numeric_components_are_rejected() {
        assert!(parse_range_header("bytes=abc-def", 1000).is_none());
        assert!(parse_range_header("bytes=10-xyz", 1000).is_none());
        assert!(parse_range_header("bytes=-xyz", 1000).is_none());
    }

    #[test]
    fn range_zero_to_zero_returns_first_byte() {
        // Single-byte range at the start: 0-0 is a one-byte response.
        assert_eq!(parse_range_header("bytes=0-0", 100), Some((0, 0)));
    }

    // ---------- read_file_range ----------

    #[test]
    fn read_file_range_returns_only_requested_span() {
        // SEC-048: the helper must not slurp the whole file. Verify it
        // returns exactly `length` bytes from `start`.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.bin");
        std::fs::write(&path, b"abcdefghij").unwrap();

        let buf = read_file_range(&path, 3, 4).unwrap();
        assert_eq!(buf, b"defg");
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn read_file_range_first_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.bin");
        std::fs::write(&path, b"abcdefghij").unwrap();

        let buf = read_file_range(&path, 0, 1).unwrap();
        assert_eq!(buf, b"a");
    }

    #[test]
    fn read_file_range_past_end_errors() {
        // Bounds enforcement is `parse_range_header`'s job, but if a
        // caller passes a span that overruns the file, `read_exact`
        // surfaces it rather than silently truncating.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.bin");
        std::fs::write(&path, b"short").unwrap();

        assert!(read_file_range(&path, 0, 10).is_err());
    }
}
