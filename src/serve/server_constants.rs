/// Server configuration constants
use std::path::Path;
use std::time::SystemTime;

/// Default number of worker threads if CPU parallelism cannot be detected
pub const DEFAULT_WORKER_COUNT: usize = 4;

/// Capacity per worker for request queue (bounded channels for backpressure)
pub const CAPACITY_PER_WORKER: usize = 64;

/// Batch size for processing operations
pub const BATCH_SIZE: usize = 64;

/// Worker poll interval in milliseconds for recv_timeout between batch drains.
/// Hot reload checks use lock-free AtomicU64 loads (nanoseconds), so this can be very short.
pub const WORKER_POLL_INTERVAL_MS: u64 = 10;

/// Request timeout in seconds
pub const REQUEST_TIMEOUT_SECS: u64 = 5;

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
];

/// Extensions that are considered static files for hot reload
pub const STATIC_FILE_EXTENSIONS: &[&str] = &[
    "css", "js", "svg", "ico", "png", "jpg", "jpeg", "gif", "woff", "woff2", "ttf",
];

/// Valid static file extensions for serving
pub const VALID_STATIC_EXTENSIONS: &[&str] = &[
    "css", "js", "svg", "ico", "png", "jpg", "jpeg", "gif", "woff", "woff2", "ttf", "html", "json",
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
