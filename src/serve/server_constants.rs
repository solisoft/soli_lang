/// Server configuration constants
use std::path::Path;
use std::time::SystemTime;

/// Default number of worker threads if CPU parallelism cannot be detected
/// (non-production / when the core count cannot be read).
pub const DEFAULT_WORKER_COUNT: usize = 4;

/// HTTP workers when `APP_ENV=production` and neither `SOLI_WORKERS` nor
/// `--workers` is set.
///
/// Each worker is a full interpreter (parsed app + builtins). Spawning one per
/// CPU core maximises throughput but multiplies baseline RSS. Production
/// defaults to a small fixed pool so a 16-core box does not open 16 copies of
/// the app unless the operator opts in.
pub const PRODUCTION_DEFAULT_WORKERS: usize = 2;

/// Default number of background-job worker threads (overridable via
/// `SOLI_JOB_WORKERS`). The job pool is opt-in background work and each worker
/// is a full interpreter copy, so the default is deliberately conservative —
/// bump `SOLI_JOB_WORKERS` for higher background throughput.
pub const DEFAULT_JOB_WORKERS: usize = 1;

/// True when `APP_ENV` names a production-style environment (case-insensitive).
pub fn is_production_env() -> bool {
    std::env::var("APP_ENV")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "production" || v == "prod"
        })
        .unwrap_or(false)
}

/// Minimum `SOLI_SESSION_SECRET` length in production (matches the cookie
/// jar / cookie-session driver). Shorter secrets are guessable HMAC keys.
pub const PRODUCTION_SESSION_SECRET_MIN_LEN: usize = 32;

/// Fail closed in production unless the operator declared public hosts and
/// a long session secret. `--dev` and non-production `APP_ENV` skip this so
/// local `soli serve --dev` keeps working without those variables.
///
/// Call after `.env` is loaded. CSRF origin checks use `SOLI_APP_HOSTS`
/// rather than a forgeable `Host` / `X-Forwarded-Host`; sealed cookies
/// derive from `SOLI_SESSION_SECRET`.
pub fn check_production_boot(dev_mode: bool) -> Result<(), String> {
    // `--dev` in a production environment is refused, not waved through.
    //
    // Passing `--dev` used to skip every check below *silently*, so
    // `APP_ENV=production soli serve --dev` — a plausible thing to do to get the
    // dev bar on a staging box — started with no `SOLI_APP_HOSTS`, no session
    // secret requirement, security headers off, and the `/__solidev/*`
    // diagnostic endpoints exposed, with nothing in the output saying so.
    if dev_mode && is_production_env() {
        return Err(
            "refusing to start with --dev while APP_ENV names a production environment. \
             Development mode disables the production boot checks (SOLI_APP_HOSTS, \
             SOLI_SESSION_SECRET), turns off the security headers, and exposes the \
             /__solidev diagnostic endpoints. Drop --dev, or set APP_ENV to something \
             other than production."
                .to_string(),
        );
    }

    if dev_mode || !is_production_env() {
        return Ok(());
    }

    let hosts_ok = std::env::var("SOLI_APP_HOSTS")
        .ok()
        .map(|raw| raw.split(',').map(str::trim).any(|host| !host.is_empty()))
        .unwrap_or(false);
    if !hosts_ok {
        return Err("production boot refuses to start without SOLI_APP_HOSTS \
             (comma-separated public hostnames, e.g. app.example.com). \
             CSRF origin checks use this list, not a forgeable Host header."
            .to_string());
    }

    let secret = std::env::var("SOLI_SESSION_SECRET").unwrap_or_default();
    if secret.len() < PRODUCTION_SESSION_SECRET_MIN_LEN {
        return Err(format!(
            "production boot refuses to start without SOLI_SESSION_SECRET \
             of at least {} characters (got {}). Sealed cookies and the \
             cookie session driver derive keys from it.",
            PRODUCTION_SESSION_SECRET_MIN_LEN,
            secret.len()
        ));
    }

    Ok(())
}

/// Resolve HTTP worker count for `soli serve`.
///
/// Priority:
/// 1. `SOLI_WORKERS` if set to a positive integer
/// 2. Explicit override from the CLI (`--workers`), when `cli_workers` is `Some`
/// 3. `PRODUCTION_DEFAULT_WORKERS` when `APP_ENV` is production
/// 4. CPU core count (or [`DEFAULT_WORKER_COUNT`])
///
/// Call sites that already applied `--workers` into a concrete number should
/// pass `None` for `cli_workers` and put the resolved value in
/// `explicit_workers` instead — see [`resolve_http_workers_for_serve`].
pub fn resolve_http_workers_from_env() -> usize {
    if let Some(n) = std::env::var("SOLI_WORKERS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
    {
        return n;
    }
    if is_production_env() {
        return PRODUCTION_DEFAULT_WORKERS;
    }
    std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(DEFAULT_WORKER_COUNT)
}

/// Whether the resolved worker count is the production memory default (so the
/// boot banner can say how to raise it).
pub fn using_production_worker_default(workers: usize) -> bool {
    is_production_env()
        && std::env::var("SOLI_WORKERS").is_err()
        && workers == PRODUCTION_DEFAULT_WORKERS
}

/// Smallest worker pool that gets a *default* realtime (WS/LiveView) worker.
///
/// Reserving a thread for realtime events stops a burst of them starving HTTP
/// (and vice versa), but the reservation costs one whole HTTP worker. On a tiny
/// pool that is the wrong default: at `--workers 2` it halved HTTP throughput,
/// making `--workers 2` measure identically to `--workers 1` — a DB-read route
/// stayed at ~11k req/s instead of ~20k. At 4 workers the same reservation
/// costs 25%, which is a fair price for the isolation.
///
/// Below this threshold every worker drains both channels (the behavior from
/// before the split), so realtime still works — it just shares the pool.
/// An explicit `SOLI_WS_WORKERS` overrides this and is always honored.
pub const MIN_WORKERS_FOR_REALTIME_SPLIT: usize = 4;

/// Split a worker pool into (http_workers, realtime_workers).
///
/// `explicit_rt` is the parsed `SOLI_WS_WORKERS` value when the operator set
/// it. An explicit value is always honored (clamped so at least one HTTP worker
/// survives); otherwise a realtime worker is reserved only once the pool is at
/// least `MIN_WORKERS_FOR_REALTIME_SPLIT`. A result of 0 realtime workers means
/// the split collapses and every worker drains both channels.
pub fn realtime_worker_split(num_workers: usize, explicit_rt: Option<usize>) -> (usize, usize) {
    let requested =
        explicit_rt.unwrap_or(usize::from(num_workers >= MIN_WORKERS_FOR_REALTIME_SPLIT));
    // Never starve HTTP: at least one worker must keep serving requests.
    let rt = requested.min(num_workers.saturating_sub(1));
    (num_workers - rt, rt)
}

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

/// Maximum simultaneous TCP connections the server keeps open.
///
/// Each connection holds a task, a socket and (once a body starts arriving) a
/// buffer, and nothing bounded how many could be open at once — so a client
/// opening connections and trickling bodies exhausted file descriptors and
/// memory without ever completing a request. Accepting and immediately closing
/// past the cap keeps the listener responsive instead of letting the backlog
/// silently absorb the flood. `SOLI_MAX_CONNECTIONS` overrides it; `0` disables
/// the cap.
pub fn max_connections() -> usize {
    env_usize("SOLI_MAX_CONNECTIONS", 20_000)
}

/// How long a request body may take to arrive in full.
///
/// `header_read_timeout` bounded the head; the body had no deadline at all, so
/// a client could announce a large `Content-Length` and trickle a byte every
/// thirty seconds, holding a connection, a task and up to the body cap of
/// buffer for as long as it liked — thousands of those exhaust file descriptors
/// and memory without ever sending a complete request. `SOLI_BODY_READ_TIMEOUT_SECS`
/// overrides it; a slow uploader on a bad link still has a generous window.
pub fn body_read_timeout_secs() -> u64 {
    env_u64("SOLI_BODY_READ_TIMEOUT_SECS", 60)
}

/// Stack size for threads that run Soli code (HTTP workers, job workers).
///
/// The tree-walking interpreter recurses on the native stack, and its 256-frame
/// budget does not fit the 2 MiB a thread gets by default: measured on a
/// release build, a plain `1 + f(n-1)` aborted around 250 frames, and one with
/// a few nested expressions per frame aborted at 30. That is not a caught
/// panic — a native stack overflow is a `SIGABRT` that takes down every worker
/// and every tenant, where the request should have produced a 500.
///
/// The interpreter is on the production path for zero-argument controller
/// actions, VM-demoted handlers, all template rendering, jobs, and everything
/// under `--dev`, so this is reachable from ordinary code rather than only from
/// deliberate recursion.
///
/// 64 MiB is virtual address space, not resident memory: pages are committed
/// only as they are touched, so a worker that never recurses deeply pays
/// nothing. `SOLI_WORKER_STACK_MB` overrides it.
pub fn worker_stack_bytes() -> usize {
    let mb = env_usize("SOLI_WORKER_STACK_MB", 64).clamp(2, 1024);
    mb * 1024 * 1024
}

/// How long a WebSocket frame waits for a free slot in the realtime worker
/// queue before the socket is closed with 1013 (Try Again Later). The wait is
/// async (`try_send` + sleep), never a blocking `send`: a blocking enqueue from
/// a tokio task parks a tokio worker thread, and the interpreter workers need
/// that pool's I/O driver to finish their `block_on` DB/HTTP calls — enough
/// parked threads and the whole server wedges. Overridable with
/// `SOLI_WS_ENQUEUE_TIMEOUT_SECS`.
pub fn ws_enqueue_timeout_secs() -> u64 {
    env_u64("SOLI_WS_ENQUEUE_TIMEOUT_SECS", 5)
}

/// Maximum simultaneous WebSocket connections, all routes and IPs combined.
/// Each connection holds a tokio task plus a 32-slot channel, so an unbounded
/// registry is a memory/FD exhaustion primitive. `SOLI_WS_MAX_CONNECTIONS`.
pub fn ws_max_connections() -> usize {
    env_usize("SOLI_WS_MAX_CONNECTIONS", 10_000)
}

/// Maximum simultaneous WebSocket connections from one peer IP.
/// `SOLI_WS_MAX_CONNECTIONS_PER_IP`; `0` disables the per-IP cap.
pub fn ws_max_connections_per_ip() -> usize {
    env_usize("SOLI_WS_MAX_CONNECTIONS_PER_IP", 64)
}

/// Sustained inbound frames per second allowed on one WebSocket connection
/// before it is closed. `SOLI_WS_MAX_MESSAGES_PER_SEC`; `0` disables.
pub fn ws_max_messages_per_sec() -> u32 {
    env_u32("SOLI_WS_MAX_MESSAGES_PER_SEC", 100)
}

/// Burst allowance on top of [`ws_max_messages_per_sec`].
/// `SOLI_WS_MESSAGE_BURST`.
pub fn ws_message_burst() -> u32 {
    env_u32("SOLI_WS_MESSAGE_BURST", 200)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

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
    // A PWA manifest served as octet-stream is ignored by every browser, which
    // is a silent "your app is not installable" rather than an error.
    ("webmanifest", "application/manifest+json"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("bmp", "image/bmp"),
    ("otf", "font/otf"),
    ("mjs", "application/javascript"),
    ("map", "application/json"),
    ("htm", "text/html"),
    ("txt", "text/plain; charset=utf-8"),
    ("xml", "application/xml"),
    ("m4a", "audio/mp4"),
    ("oga", "audio/ogg"),
    ("vtt", "text/vtt"),
    // File mode renders `.md` as HTML, but a Markdown file reached any other
    // way (a direct link into `public/`, a `?raw` fetch) should read as text
    // in the browser rather than download as an unnamed binary.
    ("md", "text/markdown; charset=utf-8"),
    ("markdown", "text/markdown; charset=utf-8"),
    // Needed by the file-mode viewer, which classifies by MIME type:
    // without these a PDF or a CSV reads as an unnamed binary blob.
    ("pdf", "application/pdf"),
    ("csv", "text/csv; charset=utf-8"),
    ("yaml", "text/yaml; charset=utf-8"),
    ("yml", "text/yaml; charset=utf-8"),
    ("toml", "text/plain; charset=utf-8"),
];

/// Extensions that are considered static files for hot reload
pub const STATIC_FILE_EXTENSIONS: &[&str] = &[
    "css", "js", "svg", "ico", "png", "jpg", "jpeg", "gif", "woff", "woff2", "ttf",
];

/// Valid static file extensions for serving. Keep in step with the bundler's
/// `BUNDLE_EXTENSIONS`: an asset that ships inside a bundle but is not listed
/// here is 404 in a standalone app while working fine from disk in dev.
pub const VALID_STATIC_EXTENSIONS: &[&str] = &[
    "css",
    "js",
    "svg",
    "ico",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "woff",
    "woff2",
    "ttf",
    "html",
    "json",
    "mp4",
    "webm",
    "ogg",
    "mp3",
    "wav",
    "webmanifest",
    "webp",
    "avif",
    "bmp",
    "otf",
    "mjs",
    "map",
    "htm",
    "txt",
    "xml",
    "m4a",
    "oga",
    "vtt",
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
///
/// Matched case-insensitively: `LOGO.PNG` is a PNG. Serving it as
/// `application/octet-stream` made the browser download the file instead of
/// showing it, which reads as a broken image rather than as a naming rule.
pub fn get_mime_type(file_path: &Path) -> &'static str {
    file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.to_ascii_lowercase())
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
    use std::sync::{Mutex, OnceLock};
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

    // ---------- realtime_worker_split ----------

    #[test]
    fn small_pools_keep_every_worker_on_http_by_default() {
        // The regression this guards: defaulting the realtime reservation on at
        // every pool size made `--workers 2` allocate 1 HTTP worker, so it
        // measured identically to `--workers 1` (a DB-read route stuck at ~11k
        // req/s instead of ~20k).
        assert_eq!(realtime_worker_split(1, None), (1, 0));
        assert_eq!(realtime_worker_split(2, None), (2, 0));
        assert_eq!(realtime_worker_split(3, None), (3, 0));
    }

    #[test]
    fn pools_at_the_threshold_reserve_one_realtime_worker() {
        assert_eq!(realtime_worker_split(4, None), (3, 1));
        assert_eq!(realtime_worker_split(8, None), (7, 1));
        assert_eq!(realtime_worker_split(16, None), (15, 1));
    }

    #[test]
    fn threshold_is_the_boundary_between_the_two_regimes() {
        // Pin the boundary itself, so moving the constant has to move this test.
        const N: usize = MIN_WORKERS_FOR_REALTIME_SPLIT;
        assert_eq!(realtime_worker_split(N - 1, None).1, 0);
        assert_eq!(realtime_worker_split(N, None).1, 1);
    }

    #[test]
    fn explicit_setting_is_honored_below_the_threshold() {
        // An operator running LiveView on a small pool can still buy the
        // isolation the default declines to take.
        assert_eq!(realtime_worker_split(2, Some(1)), (1, 1));
        assert_eq!(realtime_worker_split(3, Some(2)), (1, 2));
    }

    #[test]
    fn explicit_zero_disables_the_split_on_a_large_pool() {
        assert_eq!(realtime_worker_split(16, Some(0)), (16, 0));
    }

    #[test]
    fn at_least_one_http_worker_always_survives() {
        // Even an absurd request cannot leave HTTP with nothing to serve on.
        for workers in 1..=16 {
            for req in [1, 2, 5, 100, usize::MAX] {
                let (http, rt) = realtime_worker_split(workers, Some(req));
                assert!(http >= 1, "workers={workers} req={req} left 0 HTTP workers");
                assert_eq!(
                    http + rt,
                    workers,
                    "workers={workers} req={req} lost a worker"
                );
            }
        }
    }

    #[test]
    fn split_never_loses_or_invents_a_worker() {
        for workers in 0..=32 {
            for explicit in [None, Some(0), Some(1), Some(3)] {
                let (http, rt) = realtime_worker_split(workers, explicit);
                assert_eq!(
                    http + rt,
                    workers,
                    "workers={workers} explicit={explicit:?}"
                );
            }
        }
    }

    // ---------- resolve_http_workers_from_env ----------

    /// One process-wide lock for every env-mutating test helper in this
    /// module. Two separate locks would not serialize against each other —
    /// both helpers mutate `APP_ENV`, so concurrent libtest threads raced
    /// and a worker-resolution test could observe another test's env
    /// mid-restore (flaky only under some thread schedules, e.g. Windows).
    static ENV_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn set_env(name: &str, value: Option<&str>) {
        match value {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    fn with_worker_env(app_env: Option<&str>, soli_workers: Option<&str>, f: impl FnOnce()) {
        let _g = lock_env();
        let prev_app = std::env::var("APP_ENV").ok();
        let prev_workers = std::env::var("SOLI_WORKERS").ok();
        set_env("APP_ENV", app_env);
        set_env("SOLI_WORKERS", soli_workers);
        f();
        set_env("APP_ENV", prev_app.as_deref());
        set_env("SOLI_WORKERS", prev_workers.as_deref());
    }

    #[test]
    fn production_env_defaults_to_two_workers() {
        with_worker_env(Some("production"), None, || {
            assert_eq!(resolve_http_workers_from_env(), PRODUCTION_DEFAULT_WORKERS);
            assert!(using_production_worker_default(PRODUCTION_DEFAULT_WORKERS));
        });
        with_worker_env(Some("prod"), None, || {
            assert_eq!(resolve_http_workers_from_env(), PRODUCTION_DEFAULT_WORKERS);
        });
        with_worker_env(Some("PRODUCTION"), None, || {
            assert_eq!(resolve_http_workers_from_env(), PRODUCTION_DEFAULT_WORKERS);
        });
    }

    #[test]
    fn soli_workers_overrides_production_default() {
        with_worker_env(Some("production"), Some("8"), || {
            assert_eq!(resolve_http_workers_from_env(), 8);
            assert!(!using_production_worker_default(8));
        });
    }

    #[test]
    fn non_production_is_not_forced_to_two() {
        with_worker_env(Some("development"), None, || {
            let n = resolve_http_workers_from_env();
            assert!(n >= 1);
            assert!(!using_production_worker_default(n));
        });
    }

    // ---------- check_production_boot ----------

    fn with_production_boot_env(
        app_env: Option<&str>,
        hosts: Option<&str>,
        secret: Option<&str>,
        f: impl FnOnce(),
    ) {
        let _g = lock_env();
        let prev_app = std::env::var("APP_ENV").ok();
        let prev_hosts = std::env::var("SOLI_APP_HOSTS").ok();
        let prev_secret = std::env::var("SOLI_SESSION_SECRET").ok();
        set_env("APP_ENV", app_env);
        set_env("SOLI_APP_HOSTS", hosts);
        set_env("SOLI_SESSION_SECRET", secret);
        f();
        set_env("APP_ENV", prev_app.as_deref());
        set_env("SOLI_APP_HOSTS", prev_hosts.as_deref());
        set_env("SOLI_SESSION_SECRET", prev_secret.as_deref());
    }

    #[test]
    fn production_missing_app_hosts_is_err() {
        with_production_boot_env(
            Some("production"),
            None,
            Some("abcdefghijklmnopqrstuvwxyz012345"),
            || {
                let err = check_production_boot(false).expect_err("hosts required");
                assert!(
                    err.contains("SOLI_APP_HOSTS"),
                    "error must name SOLI_APP_HOSTS, got: {err}"
                );
            },
        );
    }

    #[test]
    fn production_empty_app_hosts_is_err() {
        with_production_boot_env(
            Some("production"),
            Some("  ,  "),
            Some("abcdefghijklmnopqrstuvwxyz012345"),
            || {
                let err = check_production_boot(false).expect_err("empty hosts");
                assert!(err.contains("SOLI_APP_HOSTS"), "got: {err}");
            },
        );
    }

    #[test]
    fn production_missing_session_secret_is_err() {
        with_production_boot_env(Some("production"), Some("app.example.com"), None, || {
            let err = check_production_boot(false).expect_err("secret required");
            assert!(
                err.contains("SOLI_SESSION_SECRET"),
                "error must name SOLI_SESSION_SECRET, got: {err}"
            );
            assert!(
                err.contains("32"),
                "error must name the 32-char rule, got: {err}"
            );
        });
    }

    #[test]
    fn production_short_session_secret_is_err() {
        with_production_boot_env(
            Some("production"),
            Some("app.example.com"),
            Some("too-short"),
            || {
                let err = check_production_boot(false).expect_err("short secret");
                assert!(err.contains("SOLI_SESSION_SECRET"), "got: {err}");
                assert!(err.contains("32"), "got: {err}");
            },
        );
    }

    #[test]
    fn production_with_hosts_and_long_secret_is_ok() {
        with_production_boot_env(
            Some("production"),
            Some("app.example.com,www.app.example.com"),
            Some("abcdefghijklmnopqrstuvwxyz012345"),
            || {
                check_production_boot(false).expect("valid production boot");
            },
        );
    }

    #[test]
    fn non_production_without_hosts_or_secret_is_ok() {
        with_production_boot_env(None, None, None, || {
            check_production_boot(false).expect("non-production boot");
        });
        with_production_boot_env(Some("development"), None, None, || {
            check_production_boot(false).expect("development boot");
        });
    }

    /// `--dev` used to skip the production gate silently. It now refuses to
    /// boot in a production environment: development mode turns off the
    /// security headers, waives the `SOLI_APP_HOSTS` / `SOLI_SESSION_SECRET`
    /// requirements, and exposes the `/__solidev` diagnostics — an operator
    /// reaching for the dev bar on a production box got all of that with
    /// nothing in the output to say so.
    #[test]
    fn dev_mode_in_a_production_environment_refuses_to_boot() {
        with_production_boot_env(Some("production"), None, None, || {
            let err = check_production_boot(true).expect_err("--dev must not boot in production");
            assert!(err.contains("--dev"), "{err}");
        });
    }

    /// Development itself is unaffected.
    #[test]
    fn dev_mode_boots_outside_production() {
        with_production_boot_env(Some("development"), None, None, || {
            check_production_boot(true).expect("--dev in development");
        });
        with_production_boot_env(None, None, None, || {
            check_production_boot(true).expect("--dev with no APP_ENV");
        });
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
            ("site.webmanifest", "application/manifest+json"),
            ("hero.webp", "image/webp"),
            ("robots.txt", "text/plain; charset=utf-8"),
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
    fn mime_extension_match_is_case_insensitive() {
        // `Path::extension` preserves case and the table is lowercase-only,
        // so the lookup lowercases first: a file named `logo.PNG` is a PNG.
        assert_eq!(get_mime_type(&PathBuf::from("logo.PNG")), "image/png");
        assert_eq!(get_mime_type(&PathBuf::from("Style.CSS")), "text/css");
    }

    #[test]
    fn mime_knows_markdown() {
        assert_eq!(
            get_mime_type(&PathBuf::from("notes.md")),
            "text/markdown; charset=utf-8"
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
