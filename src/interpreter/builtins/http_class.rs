//! HTTP built-in class for SoliLang.
//!
//! Provides the HTTP class with static methods for making HTTP requests:
//! - HTTP.get(url, options?) -> Future<String>
//! - HTTP.post(url, body, options?) -> Future<String>
//! - HTTP.put(url, body, options?) -> Future<String>
//! - HTTP.delete(url, options?) -> Future<String>
//! - HTTP.patch(url, body, options?) -> Future<String>
//! - HTTP.head(url, options?) -> Future<String>
//! - HTTP.get_json(url) -> Future<Value>
//! - HTTP.post_json(url, data) -> Future<Value>
//! - HTTP.put_json(url, data) -> Future<Value>
//! - HTTP.patch_json(url, data) -> Future<Value>
//! - HTTP.request(method, url, options?, body?) -> Future<HTTPResponse>
//! - HTTP.get_all(urls) -> Array<Future<String>>
//! - HTTP.get_all_json(urls) -> Array<Future<Value>>
//! - HTTP.parallel(requests) -> Array<Future<HTTPResponse>>
//!
//! Also provides helpers for working with HTTP responses.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;

use reqwest::Client;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{
    hash_from_pairs, Class, FutureState, HashKey, HashPairs, HttpFutureKind, NativeFunction, Value,
};
use crate::serve::get_tokio_handle;

const BLOCKED_SCHEMES: &[&str] = &["javascript", "file", "ftp", "ssh", "telnet", "gopher"];

/// SEC-017: process-wide flag that allows the SSRF blocklist to permit
/// loopback / private IPs. Previously the bypass keyed off
/// `APP_ENV=test`, but `APP_ENV` is a normal-looking env name that
/// staging/CI/etc routinely set — operators occasionally turn it on in
/// production by mistake and lost the SSRF guardrail without warning.
///
/// The flag now starts `false` in every process and is flipped to `true`
/// only by the `soli test` parent (in-process call to
/// [`enable_ssrf_test_mode`]) and by `soli serve` children spawned by
/// that parent (via the `SOLI_INTERNAL_TEST_RUNNER` env var, read once
/// at startup in `main.rs`). SEC-084: the env value must be a fresh
/// UUID v4 minted by the test runner — legacy `=1` payloads are
/// rejected so an accidental env-var leak in production cannot disable
/// the SSRF guardrail. Application code, controllers, and production
/// deployments cannot set it.
static SSRF_TEST_MODE: AtomicBool = AtomicBool::new(false);

/// Mark this process as the test runner / a test-runner-spawned child.
/// SSRF blocklist will allow loopback and private addresses while
/// running. **Never call this from production code paths.**
pub fn enable_ssrf_test_mode() {
    SSRF_TEST_MODE.store(true, Ordering::SeqCst);
}

pub fn ssrf_test_mode() -> bool {
    SSRF_TEST_MODE.load(Ordering::Relaxed)
}

/// SEC-020: cap the number of items the user can submit to a parallel
/// fan-out builtin (`HTTP.get_all`, `HTTP.get_all_json`, `HTTP.parallel`,
/// `Image.process_all`). Without this, a controller that does
/// `HTTP.get_all(req["urls"])` with attacker-supplied input lets a
/// single request spawn thousands of OS threads and exhaust the worker.
pub(crate) fn parallel_max_items() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SOLI_PARALLEL_MAX_ITEMS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(256)
    })
}

/// SEC-020: cap the number of OS threads a single parallel fan-out call
/// may have alive at once. The runner consumes the input list in chunks
/// of this size — one chunk fully completes before the next starts.
pub(crate) fn parallel_max_concurrency() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SOLI_PARALLEL_MAX_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(16)
    })
}

/// Run an async future safely, avoiding blocking the I/O driver if already in async context.
/// If called from within an async runtime, spawns a dedicated single-thread runtime to avoid
/// blocking the worker thread. Otherwise uses the runtime handle directly.
fn http_block_on<F>(rt: &tokio::runtime::Handle, future: F) -> F::Output
where
    F: std::future::Future + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        // Already inside async runtime — create a dedicated single-thread runtime
        // so we don't block the caller's I/O driver thread
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(future)
    } else {
        rt.block_on(future)
    }
}

pub fn validate_url_for_ssrf(url: &str) -> Result<(), String> {
    validate_url_for_ssrf_impl(url, ssrf_test_mode())
}

/// SSRF check with an explicit `test_mode` flag. Splitting this out lets tests
/// force the blocklist on/off without flipping the process-global
/// `SSRF_TEST_MODE` (which races other tests that enable it).
fn validate_url_for_ssrf_impl(url: &str, test_mode: bool) -> Result<(), String> {
    let url = url.trim();

    if url.is_empty() {
        return Err("URL cannot be empty".to_string());
    }

    // SEC-087: parse with `reqwest::Url` (a re-export of `url::Url`) so
    // bracketed IPv6 literals like `http://[::1]:8080/` get normalised
    // before reaching `is_blocked_host`. The previous hand-rolled
    // authority parse split on `:` and produced `"["` as the host,
    // letting any IPv6 literal slip past the SSRF blocklist (loopback,
    // ULA, IPv4-mapped, link-local, ...). Routing through a real URL
    // parser also handles userinfo, percent-encoding, and oddly-shaped
    // authorities consistently with whatever the underlying HTTP client
    // would observe.
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| format!("URL must have a scheme (e.g., http:// or https://): {}", e))?;

    let scheme = parsed.scheme().to_ascii_lowercase();
    if scheme.is_empty() {
        return Err("URL scheme cannot be empty".to_string());
    }
    if BLOCKED_SCHEMES.contains(&scheme.as_str()) {
        return Err(format!(
            "URL scheme '{}:' is not allowed for security reasons",
            scheme
        ));
    }
    if scheme != "http" && scheme != "https" {
        return Err("Only HTTP and HTTPS URLs are allowed".to_string());
    }

    // `host()` is `None` for non-special schemes; for http/https it's
    // always populated unless the URL has no authority (e.g. `http:`
    // alone, which `parse` would reject anyway). Defensive: treat None
    // as an empty host.
    let host = parsed
        .host()
        .ok_or_else(|| "URL host cannot be empty".to_string())?;

    let blocked = match host {
        // Skip DNS — IP literals never resolve, the connect-time
        // resolver wouldn't see them, so the up-front `is_blocked_ip`
        // call is the only line of defence for these.
        url::Host::Ipv4(v4) => is_blocked_ip(IpAddr::V4(v4)),
        url::Host::Ipv6(v6) => is_blocked_ip(IpAddr::V6(v6)),
        url::Host::Domain(domain) => {
            if domain.is_empty() {
                return Err("URL host cannot be empty".to_string());
            }
            is_blocked_host(domain)
        }
    };

    if blocked {
        // SEC-017 / SEC-084: bypass loopback/private only when the
        // process is running under the test runner (in-process
        // `AtomicBool` set by `soli test`, or by `main.rs` after
        // validating a UUID-v4 `SOLI_INTERNAL_TEST_RUNNER` token).
        // Production/dev/staging — no matter what `APP_ENV` says —
        // stays blocked.
        // SEC-085: allow `SOLI_DEV_ALLOW_SSRF=1` for local development.
        let dev_allowed = std::env::var("SOLI_DEV_ALLOW_SSRF")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .map(|n| n != 0)
            .unwrap_or(false);
        if !test_mode && !dev_allowed {
            return Err("Access to private/localhost addresses is not allowed".to_string());
        }
    }

    Ok(())
}

fn is_blocked_host(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_blocked_ip(ip);
    }

    let lower_host = host.to_lowercase();
    if lower_host == "localhost"
        || lower_host == "localhost."
        || lower_host.starts_with("localhost.")
    {
        return true;
    }

    if let Ok(addrs) = (host, 0u16).to_socket_addrs() {
        for addr in addrs {
            if is_blocked_ip(addr.ip()) {
                return true;
            }
        }
    }

    false
}

/// SEC-016: SSRF blocklist for any IP we'd connect to. Centralised here so
/// the `validate_url_for_ssrf` up-front check, the SEC-015 connect-time DNS
/// resolver, and any future call site share one definition. Earlier the
/// IPv6 branch only blocked `fe80::` (link-local) and `ff01::` (one
/// multicast slice), missing IPv4-mapped loopback / ULA / discard / docs
/// prefixes — anything in those ranges resolved through DNS or smuggled
/// in via `[::ffff:127.0.0.1]` would have slipped past.
///
/// We deliberately do not call `Ipv6Addr::is_private` (still unstable in
/// stable Rust) — instead we check the bit patterns explicitly and let
/// the standard `is_loopback` / `is_unspecified` / `is_multicast`
/// helpers cover what they cover.
pub(crate) fn is_blocked_ip(ip: IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            // RFC1918 private + RFC3927 link-local are stable on Ipv4Addr.
            if v4.is_private() || v4.is_link_local() {
                return true;
            }
            // RFC6598 carrier-grade NAT (100.64.0.0/10) — not covered by
            // `is_private`.
            let octets = v4.octets();
            if octets[0] == 100 && (octets[1] & 0xc0) == 64 {
                return true;
            }
            false
        }
        IpAddr::V6(v6) => {
            // IPv4-mapped (::ffff:a.b.c.d): defer to the IPv4 rule so a
            // request to e.g. `[::ffff:127.0.0.1]` is rejected exactly
            // like `127.0.0.1`.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(v4));
            }
            // ULA — RFC4193 fc00::/7 (covers fc00::/8 and fd00::/8).
            let octets = v6.octets();
            if (octets[0] & 0xfe) == 0xfc {
                return true;
            }
            // Link-local — RFC4291 fe80::/10. `(fe80..fec0)` so check
            // the high 10 bits: byte0 == 0xfe AND top two bits of
            // byte1 are `10`.
            if octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80 {
                return true;
            }
            // Discard prefix — RFC6666 100::/64.
            let segments = v6.segments();
            if segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
                return true;
            }
            // Documentation prefix — RFC3849 2001:db8::/32 is reserved
            // for examples and must never be reached on a real network.
            if segments[0] == 0x2001 && segments[1] == 0x0db8 {
                return true;
            }
            // Site-local — RFC3879 deprecated fec0::/10 — reject for
            // historical safety.
            if octets[0] == 0xfe && (octets[1] & 0xc0) == 0xc0 {
                return true;
            }
            false
        }
    }
}

static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();
static USER_HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

/// SEC-015: DNS resolver that filters out blocked IPs at the connect-time
/// resolution. `validate_url_for_ssrf` runs once on the URL string up-front
/// — without this, the actual reqwest call would resolve DNS *again* later,
/// opening a textbook DNS-rebinding window where the first answer passes
/// the check (public IP) and the second targets `127.0.0.1`.
///
/// Plugged into `get_user_http_client` (HTTP class / SOAP — URLs come from
/// user code) only. The shared `get_http_client` used by Model queries and
/// the SoliDB HTTP path keeps the default resolver: SOLIDB_HOST typically
/// points at `localhost:6745` in dev/staging, and a blanket loopback block
/// would refuse every Model.find() call in production.
#[derive(Debug)]
struct SsrfBlockingResolver;

impl reqwest::dns::Resolve for SsrfBlockingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            // `to_socket_addrs` is blocking; spawn_blocking keeps the
            // tokio I/O driver thread responsive on slow DNS.
            let safe = tokio::task::spawn_blocking(
                move || -> Result<Vec<std::net::SocketAddr>, std::io::Error> {
                    let raw: Vec<std::net::SocketAddr> =
                        (host.as_str(), 0u16).to_socket_addrs()?.collect();
                    let allow_loopback = ssrf_test_mode();
                    let safe: Vec<std::net::SocketAddr> = raw
                        .into_iter()
                        .filter(|sa| allow_loopback || !is_blocked_ip(sa.ip()))
                        .collect();
                    if safe.is_empty() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!(
                                "SSRF: hostname {:?} resolved only to blocked addresses",
                                host
                            ),
                        ));
                    }
                    Ok(safe)
                },
            )
            .await
            .map_err(|join_err| {
                Box::new(std::io::Error::other(format!(
                    "DNS task panicked: {}",
                    join_err
                ))) as Box<dyn std::error::Error + Send + Sync>
            })?
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
            Ok(Box::new(safe.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Build the SEC-007 redirect policy that re-runs `validate_url_for_ssrf`
/// on each hop and caps the chain at 10 redirects. Same policy for both
/// clients — internal Model traffic doesn't expect redirects but the cap
/// is harmless, and a misconfigured SoliDB shouldn't be allowed to follow
/// a redirect into the cloud metadata IP either.
fn build_ssrf_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error(std::io::Error::other("too many redirects (max 10)"));
        }
        if let Err(e) = validate_url_for_ssrf(attempt.url().as_str()) {
            return attempt.error(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("SSRF: redirect target rejected: {}", e),
            ));
        }
        attempt.follow()
    })
}

/// Idle lifetime for pooled connections in the internal SoliDB client
/// (`SOLI_DB_POOL_IDLE_SECS`, default 90). A retired idle connection means
/// the next DB query pays a fresh DNS + TCP (+ TLS for a remote host)
/// connect — observed as request-latency spikes (~40ms → 400ms+) on
/// low-traffic servers whenever the gap between requests exceeded the old
/// 5s idle window. The keep-warm ping (`model::db_config::spawn_db_keep_warm`)
/// is sized from this value so a pooled connection stays alive between
/// sparse requests.
pub fn db_pool_idle_secs() -> u64 {
    static SECS: OnceLock<u64> = OnceLock::new();
    *SECS.get_or_init(|| {
        std::env::var("SOLI_DB_POOL_IDLE_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(90)
            .max(1)
    })
}

/// Internal HTTP client — used by Model queries and SoliDB's HTTP path.
/// URL is operator-configured (`SOLIDB_HOST`, typically loopback in dev
/// and a private hostname behind a VPC in production), so the SSRF
/// blocklist would refuse every connection. Default DNS resolver +
/// SEC-007 redirect policy.
pub fn get_http_client() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            // Bound any residual stall (e.g. a stale-connection half-open read)
            // to 10s instead of 30s. The real deadlock that produced 30s hangs
            // was tokio-runtime starvation (the runtime was sized to --workers),
            // fixed separately in `serve::mod`; this is just a backstop.
            .timeout(std::time::Duration::from_secs(10))
            // Fail fast on an unreachable/slow SoliDB host instead of letting a
            // dead connect ride the full timeout.
            .connect_timeout(std::time::Duration::from_secs(5))
            // Keep-alive pooling IS worth it: a typical page fires several
            // queries, and reusing one connection across them avoids a TCP
            // setup (syscalls + CPU) per query. The idle timeout was once 5s
            // so a connection the peer/intermediary silently dropped would be
            // retired quickly — but that meant ANY >5s gap between requests
            // forced a cold DNS+TCP+TLS connect mid-request (400ms+ spikes on
            // quiet servers). It is now 90s (SOLI_DB_POOL_IDLE_SECS) and the
            // half-open-socket concern is handled by the periodic keep-warm
            // ping (`spawn_db_keep_warm`), which exercises the pooled
            // connection well inside both this window and typical NAT/LB
            // idle limits, plus the tcp_keepalive probe below.
            .pool_idle_timeout(std::time::Duration::from_secs(db_pool_idle_secs()))
            .pool_max_idle_per_host(8)
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .redirect(build_ssrf_redirect_policy())
            // SEC-042: pin a hard floor of TLS 1.2. The `rustls-tls`
            // backend already refuses TLS 1.0/1.1 today, but the
            // explicit setting is defense in depth — if reqwest's
            // default ever broadens, or someone swaps the backend,
            // we don't silently regain a downgrade-prone handshake.
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()
            .expect("Failed to create internal HTTP client")
    })
}

/// SEC-018: maximum bytes Soli will buffer from a single outbound HTTP
/// response body. The defaults of `reqwest::Response::text()` and
/// `ureq::Response::into_string()` are unbounded — a malicious or
/// compromised upstream returning a multi-GB body would OOM the worker.
/// Configurable via `SOLI_HTTP_MAX_RESPONSE_BYTES`; default 50 MiB,
/// which is generous for legitimate JSON / HTML / file payloads.
fn http_max_response_bytes() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SOLI_HTTP_MAX_RESPONSE_BYTES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(50 * 1024 * 1024)
    })
}

/// Read a reqwest response body into a `String`, aborting once the
/// accumulated bytes exceed [`http_max_response_bytes`]. Used in place
/// of the unbounded `Response::text().await`.
pub async fn read_capped_text_async(resp: reqwest::Response) -> Result<String, String> {
    use futures_util::StreamExt;
    let cap = http_max_response_bytes();
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if buf.len().saturating_add(chunk.len()) > cap {
            return Err(format!(
                "HTTP response exceeded {} bytes (SOLI_HTTP_MAX_RESPONSE_BYTES)",
                cap
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    String::from_utf8(buf).map_err(|e| format!("invalid UTF-8 in response body: {}", e))
}

/// Read a ureq response body into a `String`, aborting once the
/// accumulated bytes exceed [`http_max_response_bytes`]. Used in place
/// of the unbounded `Response::into_string()`.
pub fn read_capped_text_sync(resp: ureq::Response) -> Result<String, String> {
    use std::io::Read;
    let cap = http_max_response_bytes();
    let mut buf: Vec<u8> = Vec::new();
    resp.into_reader()
        .take((cap as u64).saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() > cap {
        return Err(format!(
            "HTTP response exceeded {} bytes (SOLI_HTTP_MAX_RESPONSE_BYTES)",
            cap
        ));
    }
    String::from_utf8(buf).map_err(|e| format!("invalid UTF-8 in response body: {}", e))
}

/// User-facing HTTP client — used by `HTTP.*` builtins and SOAP. URL
/// comes from user Soli code, so DNS rebinding is a real threat:
/// `SsrfBlockingResolver` filters resolved IPs through `is_blocked_ip`
/// at connect time, closing the TOCTOU between `validate_url_for_ssrf`
/// and the actual TCP connect. Same SEC-007 redirect policy as the
/// internal client.
pub fn get_user_http_client() -> &'static Client {
    USER_HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            // Match get_http_client's 15s floor: a longer client idle than the
            // peer's keep-alive window lets the pool hand back a connection the
            // server already dropped, stalling the reuse for seconds (see
            // get_http_client). External APIs commonly close idle keep-alives
            // near 60s, so keep this well under it.
            .pool_idle_timeout(std::time::Duration::from_secs(15))
            .pool_max_idle_per_host(8)
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .redirect(build_ssrf_redirect_policy())
            .dns_resolver(std::sync::Arc::new(SsrfBlockingResolver))
            // SEC-042: same TLS-1.2 floor as the internal client (see
            // `get_http_client`).
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()
            .expect("Failed to create user HTTP client")
    })
}

/// Shared `ureq::Agent` for the HTTP class. SEC-007: redirects are
/// disabled (`redirects(0)`) — ureq has no per-redirect callback hook,
/// so the only safe option is to refuse the auto-follow and surface the
/// 3xx response to the caller. Apps that need transparent redirect
/// support should use the reqwest-backed paths (Model queries / async
/// futures), which install a custom policy that re-runs
/// `validate_url_for_ssrf` on each hop.
///
/// SEC-042: TLS minimum is governed by ureq's default `tls` feature,
/// which uses rustls — and rustls only implements TLS 1.2 and 1.3, so a
/// downgrade to 1.0/1.1 isn't reachable from this agent today.
/// `AgentBuilder` has no `.min_tls_version` knob; if the feature flag
/// were ever swapped to `native-tls`, the floor would need to be
/// re-asserted via a custom `tls_connector`.
static UREQ_AGENT: OnceLock<ureq::Agent> = OnceLock::new();

pub fn ureq_agent() -> &'static ureq::Agent {
    UREQ_AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .redirects(0)
            .build()
    })
}

#[allow(clippy::arc_with_non_send_sync)]
fn spawn_http_future<F>(f: F, kind: HttpFutureKind) -> Value
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });
    Value::Future(Arc::new(Mutex::new(FutureState::Pending {
        receiver: rx,
        kind,
    })))
}

/// SEC-015a: route a fallback (no-shared-tokio-handle) HTTP call through
/// the user-facing reqwest client instead of `ureq_agent()`. The reqwest
/// client carries `SsrfBlockingResolver` from SEC-015 — every connect-time
/// DNS lookup goes through the same blocked-IP filter as
/// `validate_url_for_ssrf`, closing the DNS-rebinding TOCTOU. `ureq` has
/// no DNS-resolver hook, so any code path that calls `ureq_agent()` for a
/// user-supplied URL is rebinding-vulnerable.
///
/// Builds a private current-thread runtime per call. CLI HTTP usage is
/// infrequent and the build cost is sub-millisecond, so a thread-local
/// cache buys nothing — we'd be on a fresh thread anyway.
fn run_user_http_request<F, Fut, T>(f: F) -> Result<T, String>
where
    F: FnOnce(reqwest::Client) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let client = get_user_http_client().clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed to build tokio runtime: {}", e))?;
    rt.block_on(f(client))
}

fn value_to_json(value: &Value) -> Result<String, String> {
    crate::interpreter::value::stringify_to_string(value)
}

/// Per-call request timeout. Reads a `timeout` key (in **seconds**, given as
/// an `Int` or `Float`) from an options hash and converts it to a `Duration`,
/// overriding the client-wide 30s default for this one request. Returns
/// `Ok(None)` when no options hash is supplied, it isn't a hash, or it has no
/// `timeout` key (or the key is null) — in which case the client default
/// applies. A non-numeric or non-positive value is a hard error so a typo
/// fails loudly instead of being silently ignored.
fn extract_timeout(options: Option<&Value>) -> Result<Option<std::time::Duration>, String> {
    let Some(Value::Hash(hash)) = options else {
        return Ok(None);
    };
    let hash = hash.borrow();
    let raw = match hash.get(&HashKey::String("timeout".into())) {
        Some(v) => v,
        None => return Ok(None),
    };
    let secs = match raw {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        Value::Null => return Ok(None),
        other => {
            return Err(format!(
                "HTTP timeout must be a number of seconds, got {}",
                other.type_name()
            ))
        }
    };
    if !secs.is_finite() || secs <= 0.0 {
        return Err(format!(
            "HTTP timeout must be a positive number of seconds, got {}",
            secs
        ));
    }
    Ok(Some(std::time::Duration::from_secs_f64(secs)))
}

/// Apply an optional per-call timeout to a request builder, leaving it
/// untouched (client default applies) when `None`.
fn apply_timeout(
    builder: reqwest::RequestBuilder,
    timeout: Option<std::time::Duration>,
) -> reqwest::RequestBuilder {
    match timeout {
        Some(d) => builder.timeout(d),
        None => builder,
    }
}

/// Send a reqwest request, recording method/url/status/duration in the
/// per-request HTTP log when dev mode is on. Returns the response on success
/// or the original error string on failure.
async fn send_logged(
    method: &str,
    url: &str,
    builder: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    // Either log enables timing capture — http_log for the queries panel,
    // span_log for the flamegraph. They share the same `start` instant.
    let logging = crate::interpreter::builtins::http_log::is_enabled()
        || crate::serve::span_log::is_enabled();
    let start = logging.then(std::time::Instant::now);
    match builder.send().await {
        Ok(resp) => {
            if let Some(s) = start {
                let dur = s.elapsed().as_secs_f64() * 1000.0;
                let status = resp.status().as_u16();
                let span_name = format!("{} {}", method, url);
                crate::serve::span_log::record(
                    &span_name,
                    crate::serve::span_log::SpanKind::Http,
                    s,
                    s.elapsed().as_micros() as u64,
                    None,
                );
                crate::interpreter::builtins::http_log::record(
                    method.to_string(),
                    url.to_string(),
                    status,
                    dur,
                    None,
                );
            }
            Ok(resp)
        }
        Err(e) => {
            let msg = e.to_string();
            if let Some(s) = start {
                let dur = s.elapsed().as_secs_f64() * 1000.0;
                let span_name = format!("{} {}", method, url);
                crate::serve::span_log::record(
                    &span_name,
                    crate::serve::span_log::SpanKind::Http,
                    s,
                    s.elapsed().as_micros() as u64,
                    Some(msg.clone()),
                );
                crate::interpreter::builtins::http_log::record(
                    method.to_string(),
                    url.to_string(),
                    0,
                    dur,
                    Some(msg.clone()),
                );
            }
            Err(msg)
        }
    }
}

fn json_to_value(json: serde_json::Value) -> Result<Value, String> {
    crate::interpreter::value::json_to_value(json)
}

pub fn register_http_class(env: &mut Environment) {
    let mut http_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    http_static_methods.insert(
        "get".to_string(),
        Rc::new(NativeFunction::new("HTTP.get", None, |args| {
            if args.is_empty() {
                return Err("HTTP.get() requires a URL".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.get() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let timeout = extract_timeout(args.get(1))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let resp =
                            send_logged("GET", &url, apply_timeout(client.get(&*url), timeout))
                                .await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        read_capped_text_async(resp).await
                    }) {
                        Ok(text) => Ok(Value::String(text.into())),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(client.get(&*url), timeout)
                                .send()
                                .await
                                .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "post".to_string(),
        Rc::new(NativeFunction::new("HTTP.post", None, |args| {
            if args.len() < 2 {
                return Err("HTTP.post() requires a URL and body".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.post() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let body = match &args[1] {
                Value::String(s) => s.clone(),
                Value::Hash(_) => value_to_json(&args[1])?.into(),
                other => {
                    return Err(format!(
                        "HTTP.post() expects string or hash body, got {}",
                        other.type_name()
                    ))
                }
            };

            let content_type = if args[1].type_name() == "Hash" {
                "application/json".to_string()
            } else {
                "text/plain".to_string()
            };

            let timeout = extract_timeout(args.get(2))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = apply_timeout(
                            client
                                .post(&*url)
                                .header("Content-Type", content_type)
                                .body(body.to_string()),
                            timeout,
                        );
                        let resp = send_logged("POST", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        read_capped_text_async(resp).await
                    }) {
                        Ok(text) => Ok(Value::String(text.into())),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(
                                client
                                    .post(&*url)
                                    .header("Content-Type", content_type)
                                    .body(body.to_string()),
                                timeout,
                            )
                            .send()
                            .await
                            .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "put".to_string(),
        Rc::new(NativeFunction::new("HTTP.put", None, |args| {
            if args.len() < 2 {
                return Err("HTTP.put() requires a URL and body".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.put() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let body = match &args[1] {
                Value::String(s) => s.clone(),
                Value::Hash(_) => value_to_json(&args[1])?.into(),
                other => {
                    return Err(format!(
                        "HTTP.put() expects string or hash body, got {}",
                        other.type_name()
                    ))
                }
            };

            let content_type = if args[1].type_name() == "Hash" {
                "application/json".to_string()
            } else {
                "text/plain".to_string()
            };

            let timeout = extract_timeout(args.get(2))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = apply_timeout(
                            client
                                .put(&*url)
                                .header("Content-Type", content_type)
                                .body(body.to_string()),
                            timeout,
                        );
                        let resp = send_logged("PUT", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        read_capped_text_async(resp).await
                    }) {
                        Ok(text) => Ok(Value::String(text.into())),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(
                                client
                                    .put(&*url)
                                    .header("Content-Type", content_type)
                                    .body(body.to_string()),
                                timeout,
                            )
                            .send()
                            .await
                            .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "patch".to_string(),
        Rc::new(NativeFunction::new("HTTP.patch", None, |args| {
            if args.len() < 2 {
                return Err("HTTP.patch() requires a URL and body".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.patch() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let body = match &args[1] {
                Value::String(s) => s.clone(),
                Value::Hash(_) => value_to_json(&args[1])?.into(),
                other => {
                    return Err(format!(
                        "HTTP.patch() expects string or hash body, got {}",
                        other.type_name()
                    ))
                }
            };

            let content_type = if args[1].type_name() == "Hash" {
                "application/json".to_string()
            } else {
                "text/plain".to_string()
            };

            let timeout = extract_timeout(args.get(2))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = apply_timeout(
                            client
                                .patch(&*url)
                                .header("Content-Type", content_type)
                                .body(body.to_string()),
                            timeout,
                        );
                        let resp = send_logged("PATCH", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        read_capped_text_async(resp).await
                    }) {
                        Ok(text) => Ok(Value::String(text.into())),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(
                                client
                                    .patch(&*url)
                                    .header("Content-Type", content_type)
                                    .body(body.to_string()),
                                timeout,
                            )
                            .send()
                            .await
                            .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("HTTP.delete", None, |args| {
            if args.is_empty() {
                return Err("HTTP.delete() requires a URL".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.delete() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let timeout = extract_timeout(args.get(1))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let resp = send_logged(
                            "DELETE",
                            &url,
                            apply_timeout(client.delete(&*url), timeout),
                        )
                        .await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        read_capped_text_async(resp).await
                    }) {
                        Ok(text) => Ok(Value::String(text.into())),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(client.delete(&*url), timeout)
                                .send()
                                .await
                                .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "head".to_string(),
        Rc::new(NativeFunction::new("HTTP.head", None, |args| {
            if args.is_empty() {
                return Err("HTTP.head() requires a URL".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.head() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let timeout = extract_timeout(args.get(1))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let resp =
                            send_logged("HEAD", &url, apply_timeout(client.head(&*url), timeout))
                                .await?;
                        let status = resp.status().as_u16();
                        Ok(format!(
                            "{} {}",
                            status,
                            resp.status().canonical_reason().unwrap_or("")
                        ))
                    }) {
                        Ok(text) => Ok(Value::String(text.into())),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(client.head(&*url), timeout)
                                .send()
                                .await
                                .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            Ok(format!(
                                "{} {}",
                                status.as_u16(),
                                status.canonical_reason().unwrap_or("")
                            ))
                        })
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "get_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_json", None, |args| {
            if args.is_empty() {
                return Err("HTTP.get_json() requires a URL".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.get_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let timeout = extract_timeout(args.get(1))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = apply_timeout(
                            client.get(&*url).header("Accept", "application/json"),
                            timeout,
                        );
                        let resp = send_logged("GET", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = read_capped_text_async(resp).await?;
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json_to_value(json),
                            Err(e) => Err(format!("Failed to parse JSON: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(
                                client.get(&*url).header("Accept", "application/json"),
                                timeout,
                            )
                            .send()
                            .await
                            .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    // HTTP.get_jsonp(url) - Fetch a JSONP endpoint and unwrap the `callback(...)`
    // padding, returning the parsed value. Mirrors get_json but strips the
    // JavaScript wrapper before parsing.
    http_static_methods.insert(
        "get_jsonp".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_jsonp", None, |args| {
            if args.is_empty() {
                return Err("HTTP.get_jsonp() requires a URL".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.get_jsonp() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let timeout = extract_timeout(args.get(1))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = apply_timeout(client.get(&*url), timeout);
                        let resp = send_logged("GET", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = read_capped_text_async(resp).await?;
                        let inner = crate::interpreter::jsonp::strip_jsonp_padding(&text)?;
                        match serde_json::from_str::<serde_json::Value>(inner) {
                            Ok(json) => json_to_value(json),
                            Err(e) => Err(format!("Failed to parse JSONP: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(client.get(&*url), timeout)
                                .send()
                                .await
                                .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::Jsonp,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "post_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.post_json", None, |args| {
            if args.len() < 2 {
                return Err("HTTP.post_json() requires a URL and data".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.post_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let json_body = value_to_json(&args[1])?;

            let timeout = extract_timeout(args.get(2))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = apply_timeout(
                            client
                                .post(&*url)
                                .header("Content-Type", "application/json")
                                .body(json_body),
                            timeout,
                        );
                        let resp = send_logged("POST", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = read_capped_text_async(resp).await?;
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json_to_value(json),
                            Err(e) => Err(format!("Failed to parse JSON: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(
                                client
                                    .post(&*url)
                                    .header("Content-Type", "application/json")
                                    .body(json_body),
                                timeout,
                            )
                            .send()
                            .await
                            .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "put_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.put_json", None, |args| {
            if args.len() < 2 {
                return Err("HTTP.put_json() requires a URL and data".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.put_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let json_body = value_to_json(&args[1])?;

            let timeout = extract_timeout(args.get(2))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = apply_timeout(
                            client
                                .put(&*url)
                                .header("Content-Type", "application/json")
                                .body(json_body),
                            timeout,
                        );
                        let resp = send_logged("PUT", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = read_capped_text_async(resp).await?;
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json_to_value(json),
                            Err(e) => Err(format!("Failed to parse JSON: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(
                                client
                                    .put(&*url)
                                    .header("Content-Type", "application/json")
                                    .body(json_body),
                                timeout,
                            )
                            .send()
                            .await
                            .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "patch_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.patch_json", None, |args| {
            if args.len() < 2 {
                return Err("HTTP.patch_json() requires a URL and data".to_string());
            }
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.patch_json() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let json_body = value_to_json(&args[1])?;

            let timeout = extract_timeout(args.get(2))?;

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = apply_timeout(
                            client
                                .patch(&*url)
                                .header("Content-Type", "application/json")
                                .body(json_body),
                            timeout,
                        );
                        let resp = send_logged("PATCH", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = read_capped_text_async(resp).await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = read_capped_text_async(resp).await?;
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json_to_value(json),
                            Err(e) => Err(format!("Failed to parse JSON: {}", e)),
                        }
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || {
                        run_user_http_request(move |client| async move {
                            let resp = apply_timeout(
                                client
                                    .patch(&*url)
                                    .header("Content-Type", "application/json")
                                    .body(json_body),
                                timeout,
                            )
                            .send()
                            .await
                            .map_err(|e| format!("HTTP request failed: {}", e))?;
                            let status = resp.status();
                            if !status.is_success() {
                                let body = read_capped_text_async(resp).await.unwrap_or_default();
                                return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                            }
                            read_capped_text_async(resp).await
                        })
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "request".to_string(),
        Rc::new(NativeFunction::new("HTTP.request", None, |args| {
            if args.len() < 2 {
                return Err("HTTP.request() requires at least method and URL".to_string());
            }

            let method = match &args[0] {
                Value::String(s) => s.to_uppercase(),
                other => {
                    return Err(format!(
                        "HTTP.request() method must be string, got {}",
                        other.type_name()
                    ))
                }
            };

            let url = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.request() URL must be string, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            // The 3rd arg is a flat headers hash. A `timeout` key (seconds) is
            // pulled out as the per-call timeout rather than sent as a header.
            let timeout = extract_timeout(args.get(2))?;

            let mut headers_vec: Vec<(String, String)> = Vec::new();
            if args.len() > 2 {
                if let Value::Hash(headers) = &args[2] {
                    for (key, value) in headers.borrow().iter() {
                        let key_str = match key {
                            HashKey::String(s) => s.clone(),
                            _ => continue,
                        };
                        if key_str.as_ref() == "timeout" {
                            continue;
                        }
                        let value_str = match value {
                            Value::String(s) => s.clone(),
                            _ => format!("{}", value).into(),
                        };
                        headers_vec.push((key_str.to_string(), value_str.to_string()));
                    }
                }
            }

            let body_opt: Option<String> = if args.len() > 3 {
                Some(match &args[3] {
                    Value::String(s) => s.clone().to_string(),
                    Value::Hash(_) => value_to_json(&args[3])?,
                    Value::Null => String::new(),
                    other => format!("{}", other),
                })
            } else {
                None
            };

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    let method_clone = method.clone();
                    let body_opt_clone = body_opt.clone();
                    let headers_vec_clone = headers_vec.clone();
                    match http_block_on(&rt, async move {
                        let mut request = match method_clone.as_str() {
                            "GET" => client.get(&*url),
                            "POST" => client.post(&*url),
                            "PUT" => client.put(&*url),
                            "DELETE" => client.delete(&*url),
                            "PATCH" => client.patch(&*url),
                            "HEAD" => client.head(&*url),
                            _ => return Err(format!("Unsupported HTTP method: {}", method_clone)),
                        };

                        for (key, value) in &headers_vec_clone {
                            request = request.header(key.as_str(), value.as_str());
                        }

                        if let Some(body) = body_opt_clone {
                            request = request.body(body);
                        }

                        let request = apply_timeout(request, timeout);
                        let resp = send_logged(&method_clone, &url, request).await?;

                        let status = resp.status().as_u16();
                        let status_text =
                            resp.status().canonical_reason().unwrap_or("").to_string();

                        let mut headers_map = serde_json::Map::new();
                        for (name, value) in resp.headers().iter() {
                            if let Ok(v) = value.to_str() {
                                headers_map.insert(
                                    name.to_string(),
                                    serde_json::Value::String(v.to_string()),
                                );
                            }
                        }

                        let body = read_capped_text_async(resp).await?;

                        create_http_response(status, status_text, headers_map, body)
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => {
                    let method_clone = method.clone();
                    let body_opt_clone = body_opt.clone();
                    let headers_vec_clone = headers_vec.clone();
                    Ok(spawn_http_future(
                        move || {
                            run_user_http_request(move |client| async move {
                                let mut request = match method_clone.as_str() {
                                    "GET" => client.get(&*url),
                                    "POST" => client.post(&*url),
                                    "PUT" => client.put(&*url),
                                    "DELETE" => client.delete(&*url),
                                    "PATCH" => client.patch(&*url),
                                    "HEAD" => client.head(&*url),
                                    _ => {
                                        return Err(format!(
                                            "Unsupported HTTP method: {}",
                                            method_clone
                                        ))
                                    }
                                };

                                for (key, value) in &headers_vec_clone {
                                    request = request.header(key.as_str(), value.as_str());
                                }

                                if let Some(body) = body_opt_clone {
                                    request = request.body(body);
                                }

                                let request = apply_timeout(request, timeout);
                                let resp = request
                                    .send()
                                    .await
                                    .map_err(|e| format!("HTTP request failed: {}", e))?;

                                let status = resp.status().as_u16();
                                let status_text =
                                    resp.status().canonical_reason().unwrap_or("").to_string();

                                let mut headers_map = serde_json::Map::new();
                                for (name, value) in resp.headers().iter() {
                                    if let Ok(v) = value.to_str() {
                                        headers_map.insert(
                                            name.to_string(),
                                            serde_json::Value::String(v.to_string()),
                                        );
                                    }
                                }

                                let body = read_capped_text_async(resp).await?;

                                let result = serde_json::json!({
                                    "status": status,
                                    "status_text": status_text,
                                    "headers": headers_map,
                                    "body": body
                                });

                                Ok(result.to_string())
                            })
                        },
                        HttpFutureKind::FullResponse,
                    ))
                }
            }
        })),
    );

    http_static_methods.insert(
        "json_parse".to_string(),
        Rc::new(NativeFunction::new("HTTP.json_parse", Some(1), |args| {
            let json_str = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "HTTP.json_parse() expects string, got {}",
                        other.type_name()
                    ))
                }
            };

            match serde_json::from_str::<serde_json::Value>(&json_str) {
                Ok(json) => json_to_value(json),
                Err(e) => Err(format!("Failed to parse JSON: {}", e)),
            }
        })),
    );

    http_static_methods.insert(
        "json_stringify".to_string(),
        Rc::new(NativeFunction::new(
            "HTTP.json_stringify",
            Some(1),
            |args| value_to_json(&args[0]).map(|s| Value::String(s.into())),
        )),
    );

    http_static_methods.insert(
        "get_all".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_all", None, |args| {
            if args.is_empty() {
                return Err("HTTP.get_all() requires an array of URLs".to_string());
            }
            let urls = match &args[0] {
                Value::Array(arr) => {
                    let mut url_strings = Vec::new();
                    for item in arr.borrow().iter() {
                        match item {
                            Value::String(s) => url_strings.push(s.to_string()),
                            other => {
                                return Err(format!(
                                    "HTTP.get_all() expects array of strings, got {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    url_strings
                }
                other => {
                    return Err(format!(
                        "HTTP.get_all() expects array of URLs, got {}",
                        other.type_name()
                    ))
                }
            };

            // SEC-020: refuse oversized input lists before they fan out.
            let cap = parallel_max_items();
            if urls.len() > cap {
                return Err(format!(
                    "HTTP.get_all() input size {} exceeds limit {} (set SOLI_PARALLEL_MAX_ITEMS to raise)",
                    urls.len(),
                    cap
                ));
            }

            for u in &urls {
                validate_url_for_ssrf(u)?;
            }

            // Optional trailing options hash applies one timeout to every
            // request in the batch.
            let timeout = extract_timeout(args.get(1))?;

            let results = run_parallel_gets(urls, timeout);

            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(body) => Value::String(body.into()),
                    Err(e) => hash_from_pairs([("error".to_string(), Value::String(e.into()))]),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    http_static_methods.insert(
        "get_all_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_all_json", None, |args| {
            if args.is_empty() {
                return Err("HTTP.get_all_json() requires an array of URLs".to_string());
            }
            let urls = match &args[0] {
                Value::Array(arr) => {
                    let mut url_strings = Vec::new();
                    for item in arr.borrow().iter() {
                        match item {
                            Value::String(s) => url_strings.push(s.to_string()),
                            other => {
                                return Err(format!(
                                    "HTTP.get_all_json() expects array of strings, got {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    url_strings
                }
                other => {
                    return Err(format!(
                        "HTTP.get_all_json() expects array of URLs, got {}",
                        other.type_name()
                    ))
                }
            };

            // SEC-020: refuse oversized input lists before they fan out.
            let cap = parallel_max_items();
            if urls.len() > cap {
                return Err(format!(
                    "HTTP.get_all_json() input size {} exceeds limit {} (set SOLI_PARALLEL_MAX_ITEMS to raise)",
                    urls.len(),
                    cap
                ));
            }

            for u in &urls {
                validate_url_for_ssrf(u)?;
            }

            // Optional trailing options hash applies one timeout to every
            // request in the batch.
            let timeout = extract_timeout(args.get(1))?;

            let results = run_parallel_gets_json(urls, timeout);

            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(value) => value,
                    Err(e) => hash_from_pairs([("error".to_string(), Value::String(e.into()))]),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    http_static_methods.insert(
        "parallel".to_string(),
        Rc::new(NativeFunction::new("HTTP.parallel", Some(1), |args| {
            let requests = match &args[0] {
                Value::Array(arr) => {
                    let mut req_configs = Vec::new();
                    for item in arr.borrow().iter() {
                        let config = parse_request_config(item)?;
                        req_configs.push(config);
                    }
                    req_configs
                }
                other => {
                    return Err(format!(
                        "HTTP.parallel() expects array of request configs, got {}",
                        other.type_name()
                    ))
                }
            };

            // SEC-020: refuse oversized input lists before they fan out.
            let cap = parallel_max_items();
            if requests.len() > cap {
                return Err(format!(
                    "HTTP.parallel() input size {} exceeds limit {} (set SOLI_PARALLEL_MAX_ITEMS to raise)",
                    requests.len(),
                    cap
                ));
            }

            for c in &requests {
                validate_url_for_ssrf(&c.url)?;
            }

            let results = run_parallel_requests(requests);

            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(response) => response_to_value(response),
                    Err(e) => hash_from_pairs([("error".to_string(), Value::String(e.into()))]),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    let http_class = Class {
        name: "HTTP".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: http_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("HTTP".to_string(), Value::Class(Rc::new(http_class)));
}

fn create_http_response(
    status: u16,
    status_text: String,
    headers_map: serde_json::Map<String, serde_json::Value>,
    body: String,
) -> Result<Value, String> {
    let response_headers: HashPairs = headers_map
        .into_iter()
        .map(|(k, v)| {
            (
                HashKey::String(k.into()),
                Value::String(v.as_str().unwrap_or("").to_string().into()),
            )
        })
        .collect();

    let mut result: HashPairs = HashPairs::default();
    result.insert(HashKey::String("status".into()), Value::Int(status as i64));
    result.insert(
        HashKey::String("status_text".into()),
        Value::String(status_text.into()),
    );
    result.insert(
        HashKey::String("headers".into()),
        Value::Hash(Rc::new(RefCell::new(response_headers))),
    );
    result.insert(HashKey::String("body".into()), Value::String(body.into()));

    Ok(Value::Hash(Rc::new(RefCell::new(result))))
}

#[derive(Clone)]
struct RequestConfig {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout: Option<std::time::Duration>,
}

struct HttpResponse {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: String,
}

fn parse_request_config(value: &Value) -> Result<RequestConfig, String> {
    match value {
        Value::String(url) => Ok(RequestConfig {
            method: "GET".to_string(),
            url: url.clone().to_string(),
            headers: vec![],
            body: None,
            timeout: None,
        }),
        Value::Hash(hash) => {
            let hash = hash.borrow();
            let mut url = None;
            let mut method = "GET".to_string();
            let mut headers = vec![];
            let mut body = None;
            let timeout = extract_timeout(Some(value))?;

            for (k, v) in hash.iter() {
                if let HashKey::String(key) = k {
                    match key.as_ref() {
                        "url" => {
                            if let Value::String(s) = v {
                                url = Some(s.clone());
                            }
                        }
                        "method" => {
                            if let Value::String(s) = v {
                                method = s.to_uppercase().to_string();
                            }
                        }
                        "headers" => {
                            if let Value::Hash(h) = v {
                                for (hk, hv) in h.borrow().iter() {
                                    if let (HashKey::String(k), Value::String(v)) = (hk, hv) {
                                        headers.push((k.to_string(), v.to_string()));
                                    }
                                }
                            }
                        }
                        "body" => match v {
                            Value::String(s) => body = Some(s.clone()),
                            Value::Hash(_) => body = Some(value_to_json(v)?.into()),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }

            let url = url.ok_or("Request config must have 'url' field")?;
            Ok(RequestConfig {
                method,
                url: url.to_string(),
                headers,
                body: body.map(|s| s.to_string()),
                timeout,
            })
        }
        other => Err(format!(
            "Request config must be string URL or hash, got {}",
            other.type_name()
        )),
    }
}

/// Per-call timing+status snapshot captured on the worker thread so the
/// main thread can replay it into the dev-bar logs after `join()`. Both
/// `http_log` and `span_log` are thread-local, so logging from the worker
/// directly would write into a never-read store.
struct ParallelCallStats {
    method: String,
    url: String,
    start: std::time::Instant,
    duration_ms: f64,
    status: u16,
    error: Option<String>,
}

fn record_parallel_stats(stats: Vec<ParallelCallStats>) {
    if !crate::interpreter::builtins::http_log::is_enabled()
        && !crate::serve::span_log::is_enabled()
    {
        return;
    }
    for s in stats {
        crate::interpreter::builtins::http_log::record_with_start(
            s.method,
            s.url,
            s.status,
            s.duration_ms,
            s.error,
            s.start,
        );
    }
}

fn run_parallel_gets(
    urls: Vec<String>,
    timeout: Option<std::time::Duration>,
) -> Vec<Result<String, String>> {
    // SEC-020: process at most `parallel_max_concurrency()` URLs at a time.
    // Each chunk fully completes before the next starts so we never hold
    // more than that many OS threads alive.
    let max_concurrency = parallel_max_concurrency();
    let mut joined: Vec<(Result<String, String>, ParallelCallStats)> =
        Vec::with_capacity(urls.len());
    let mut iter = urls.into_iter();
    loop {
        let chunk: Vec<String> = iter.by_ref().take(max_concurrency).collect();
        if chunk.is_empty() {
            break;
        }
        let handles: Vec<_> =
            chunk
                .into_iter()
                .map(|url| {
                    thread::spawn(move || {
                        let start = std::time::Instant::now();
                        let url_for_call = url.clone();
                        // SEC-015a: route through the SSRF-aware reqwest client so
                        // the connect-time DNS lookup goes through
                        // `SsrfBlockingResolver` (no equivalent on `ureq`).
                        let (status, body): (u16, Result<String, String>) =
                            match run_user_http_request::<_, _, (u16, Result<String, String>)>(
                                move |client| async move {
                                    let resp = apply_timeout(client.get(&url_for_call), timeout)
                                        .send()
                                        .await
                                        .map_err(|e| format!("Request failed: {}", e))?;
                                    let code = resp.status().as_u16();
                                    if !resp.status().is_success() {
                                        let body =
                                            read_capped_text_async(resp).await.unwrap_or_default();
                                        return Ok((
                                            code,
                                            Err(format!("HTTP {} error: {}", code, body)),
                                        ));
                                    }
                                    let body = read_capped_text_async(resp)
                                        .await
                                        .map_err(|e| format!("Failed to read response: {}", e));
                                    Ok((code, body))
                                },
                            ) {
                                Ok(pair) => pair,
                                Err(e) => (0, Err(e)),
                            };
                        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
                        let stats = ParallelCallStats {
                            method: "GET".to_string(),
                            url,
                            start,
                            duration_ms,
                            status,
                            error: body.as_ref().err().cloned(),
                        };
                        (body, stats)
                    })
                })
                .collect();
        for h in handles {
            joined.push(h.join().unwrap_or_else(|_| {
                (
                    Err("Thread panicked".to_string()),
                    ParallelCallStats {
                        method: "GET".to_string(),
                        url: String::new(),
                        start: std::time::Instant::now(),
                        duration_ms: 0.0,
                        status: 0,
                        error: Some("Thread panicked".to_string()),
                    },
                )
            }));
        }
    }

    let (results, stats): (Vec<_>, Vec<_>) = joined.into_iter().unzip();
    record_parallel_stats(stats);
    results
}

fn run_parallel_gets_json(
    urls: Vec<String>,
    timeout: Option<std::time::Duration>,
) -> Vec<Result<Value, String>> {
    // Fetch bodies on worker threads, then parse JSON on the main thread —
    // `Value` is `!Send` (contains Rc), so JSON parsing can't happen inside
    // the spawned threads.
    //
    // SEC-020: bound concurrent threads at `parallel_max_concurrency()`.
    let max_concurrency = parallel_max_concurrency();
    let mut joined: Vec<(Result<String, String>, ParallelCallStats)> =
        Vec::with_capacity(urls.len());
    let mut iter = urls.into_iter();
    loop {
        let chunk: Vec<String> = iter.by_ref().take(max_concurrency).collect();
        if chunk.is_empty() {
            break;
        }
        let handles: Vec<_> =
            chunk
                .into_iter()
                .map(|url| {
                    thread::spawn(move || {
                        let start = std::time::Instant::now();
                        let url_for_call = url.clone();
                        // SEC-015a: SSRF-aware reqwest client.
                        let (status, body): (u16, Result<String, String>) =
                            match run_user_http_request::<_, _, (u16, Result<String, String>)>(
                                move |client| async move {
                                    let resp = apply_timeout(
                                        client
                                            .get(&url_for_call)
                                            .header("Accept", "application/json"),
                                        timeout,
                                    )
                                    .send()
                                    .await
                                    .map_err(|e| format!("Request failed: {}", e))?;
                                    let code = resp.status().as_u16();
                                    if !resp.status().is_success() {
                                        let body =
                                            read_capped_text_async(resp).await.unwrap_or_default();
                                        return Ok((
                                            code,
                                            Err(format!("HTTP {} error: {}", code, body)),
                                        ));
                                    }
                                    let body = read_capped_text_async(resp)
                                        .await
                                        .map_err(|e| format!("Failed to read response: {}", e));
                                    Ok((code, body))
                                },
                            ) {
                                Ok(pair) => pair,
                                Err(e) => (0, Err(e)),
                            };
                        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
                        let stats = ParallelCallStats {
                            method: "GET".to_string(),
                            url,
                            start,
                            duration_ms,
                            status,
                            error: body.as_ref().err().cloned(),
                        };
                        (body, stats)
                    })
                })
                .collect();
        for h in handles {
            joined.push(h.join().unwrap_or_else(|_| {
                (
                    Err("Thread panicked".to_string()),
                    ParallelCallStats {
                        method: "GET".to_string(),
                        url: String::new(),
                        start: std::time::Instant::now(),
                        duration_ms: 0.0,
                        status: 0,
                        error: Some("Thread panicked".to_string()),
                    },
                )
            }));
        }
    }

    let (bodies, stats): (Vec<_>, Vec<_>) = joined.into_iter().unzip();
    record_parallel_stats(stats);

    bodies
        .into_iter()
        .map(|body_result| {
            let body = body_result?;
            let json: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| format!("Failed to parse JSON: {}", e))?;
            json_to_value(json)
        })
        .collect()
}

fn run_parallel_requests(requests: Vec<RequestConfig>) -> Vec<Result<HttpResponse, String>> {
    // SEC-020: bound concurrent threads at `parallel_max_concurrency()`.
    let max_concurrency = parallel_max_concurrency();
    let mut joined: Vec<(Result<HttpResponse, String>, ParallelCallStats)> =
        Vec::with_capacity(requests.len());
    let mut iter = requests.into_iter();
    loop {
        let chunk: Vec<RequestConfig> = iter.by_ref().take(max_concurrency).collect();
        if chunk.is_empty() {
            break;
        }
        let handles: Vec<_> = chunk
            .into_iter()
            .map(|config| {
                thread::spawn(move || {
                    let method = config.method.clone();
                    let url = config.url.clone();
                    let start = std::time::Instant::now();
                    let result = execute_request(config);
                    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let (status, error) = match &result {
                        Ok(resp) => (resp.status, None),
                        Err(e) => (0, Some(e.clone())),
                    };
                    let stats = ParallelCallStats {
                        method,
                        url,
                        start,
                        duration_ms,
                        status,
                        error,
                    };
                    (result, stats)
                })
            })
            .collect();
        for h in handles {
            joined.push(h.join().unwrap_or_else(|_| {
                (
                    Err("Thread panicked".to_string()),
                    ParallelCallStats {
                        method: String::new(),
                        url: String::new(),
                        start: std::time::Instant::now(),
                        duration_ms: 0.0,
                        status: 0,
                        error: Some("Thread panicked".to_string()),
                    },
                )
            }));
        }
    }

    let (results, stats): (Vec<_>, Vec<_>) = joined.into_iter().unzip();
    record_parallel_stats(stats);
    results
}

fn execute_request(config: RequestConfig) -> Result<HttpResponse, String> {
    // SEC-015a: SSRF-aware reqwest client with `SsrfBlockingResolver`.
    // Was on `ureq_agent()` which had no DNS-resolver hook, so the
    // SEC-015 DNS-rebinding TOCTOU defense couldn't apply here.
    run_user_http_request::<_, _, HttpResponse>(move |client| async move {
        let mut request = match config.method.as_str() {
            "GET" => client.get(&config.url),
            "POST" => client.post(&config.url),
            "PUT" => client.put(&config.url),
            "DELETE" => client.delete(&config.url),
            "PATCH" => client.patch(&config.url),
            "HEAD" => client.head(&config.url),
            _ => return Err(format!("Unsupported HTTP method: {}", config.method)),
        };

        for (key, value) in &config.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        if let Some(body) = config.body {
            request = request.body(body);
        }

        let request = apply_timeout(request, config.timeout);
        let resp = request
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        let status = resp.status().as_u16();
        let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
        let mut headers = vec![];
        for (name, value) in resp.headers().iter() {
            if let Ok(v) = value.to_str() {
                headers.push((name.to_string(), v.to_string()));
            }
        }
        let body = read_capped_text_async(resp)
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        Ok(HttpResponse {
            status,
            status_text,
            headers,
            body,
        })
    })
}

fn response_to_value(response: HttpResponse) -> Value {
    let headers: HashPairs = response
        .headers
        .into_iter()
        .map(|(k, v)| (HashKey::String(k.into()), Value::String(v.into())))
        .collect();

    let mut result: HashPairs = HashPairs::default();
    result.insert(
        HashKey::String("status".into()),
        Value::Int(response.status as i64),
    );
    result.insert(
        HashKey::String("status_text".into()),
        Value::String(response.status_text.into()),
    );
    result.insert(
        HashKey::String("headers".into()),
        Value::Hash(Rc::new(RefCell::new(headers))),
    );
    result.insert(
        HashKey::String("body".into()),
        Value::String(response.body.into()),
    );

    Value::Hash(Rc::new(RefCell::new(result)))
}

#[cfg(test)]
mod parallel_ssrf_tests {
    use super::*;

    fn http_static(name: &str) -> Rc<NativeFunction> {
        let mut env = Environment::new();
        register_http_class(&mut env);
        match env.get("HTTP").expect("HTTP class registered") {
            Value::Class(c) => c
                .native_static_methods
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("HTTP.{} not registered", name)),
            other => panic!("HTTP is not a Class: {:?}", other),
        }
    }

    fn arr(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    /// `file://` is rejected by the scheme guard regardless of APP_ENV, so this
    /// test exercises the parallel-helper SSRF check without depending on the
    /// localhost bypass that `soli test` enables.
    #[test]
    fn get_all_rejects_blocked_scheme() {
        let f = http_static("get_all");
        let err = (f.func)(vec![arr(vec![Value::String("file:///etc/passwd".into())])])
            .expect_err("get_all should reject file:// URLs");
        assert!(err.contains("not allowed"), "got: {}", err);
    }

    #[test]
    fn get_all_json_rejects_blocked_scheme() {
        let f = http_static("get_all_json");
        let err = (f.func)(vec![arr(vec![
            Value::String("https://example.com/ok".into()),
            Value::String("gopher://internal/".into()),
        ])])
        .expect_err("get_all_json should reject gopher:// URLs");
        assert!(err.contains("not allowed"), "got: {}", err);
    }

    #[test]
    fn parallel_rejects_blocked_scheme_in_config() {
        let f = http_static("parallel");
        let cfg = {
            let mut h = HashPairs::default();
            h.insert(
                HashKey::String("url".into()),
                Value::String("ftp://internal/etc/passwd".into()),
            );
            Value::Hash(Rc::new(RefCell::new(h)))
        };
        let err = (f.func)(vec![arr(vec![cfg])]).expect_err("parallel should reject ftp:// URLs");
        assert!(err.contains("not allowed"), "got: {}", err);
    }

    /// SEC-020: oversized inputs must be rejected before any thread spawns.
    /// Generates `parallel_max_items() + 1` `https://` URLs (no actual I/O —
    /// the cap fires before validate_url_for_ssrf does any DNS work).
    #[test]
    fn get_all_rejects_oversized_input() {
        let f = http_static("get_all");
        let cap = parallel_max_items();
        let urls: Vec<Value> = (0..=cap)
            .map(|i| Value::String(format!("https://example.com/{}", i).into()))
            .collect();
        let err = (f.func)(vec![arr(urls)]).expect_err("get_all should reject oversized input");
        assert!(
            err.contains("exceeds limit") && err.contains("SOLI_PARALLEL_MAX_ITEMS"),
            "got: {}",
            err
        );
    }
}

#[cfg(test)]
mod parallel_logging_tests {
    use super::*;
    use crate::interpreter::builtins::http_log;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn spawn_mock_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                thread::spawn(move || {
                    let mut s = stream;
                    let mut buf = [0u8; 4096];
                    let mut total = Vec::new();
                    loop {
                        let n = s.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        total.extend_from_slice(&buf[..n]);
                        if total.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                        if total.len() > 64 * 1024 {
                            break;
                        }
                    }
                    let body = b"{\"ok\":true}";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(resp.as_bytes());
                    let _ = s.write_all(body);
                });
            }
        });
        port
    }

    /// Bundles all three parallel-helper checks into one test so we don't
    /// race on the process-global `http_log::ENABLED` flag with concurrent
    /// tests.
    #[test]
    fn parallel_helpers_record_to_http_log() {
        let port = spawn_mock_server();
        let url = |p: &str| format!("http://127.0.0.1:{}{}", port, p);

        http_log::set_enabled(true);

        // get_all
        http_log::clear();
        let urls = vec![url("/a"), url("/b"), url("/c")];
        let _ = run_parallel_gets(urls.clone(), None);
        let snap = http_log::snapshot();
        assert_eq!(snap.len(), 3, "get_all should record 3 entries");
        for (i, entry) in snap.iter().enumerate() {
            assert_eq!(entry.method, "GET");
            assert_eq!(entry.status, 200);
            assert!(entry.error.is_none());
            assert_eq!(entry.url, urls[i]);
        }

        // get_all_json
        http_log::clear();
        let urls = vec![url("/x"), url("/y")];
        let _ = run_parallel_gets_json(urls.clone(), None);
        let snap = http_log::snapshot();
        assert_eq!(snap.len(), 2, "get_all_json should record 2 entries");
        for (i, entry) in snap.iter().enumerate() {
            assert_eq!(entry.method, "GET");
            assert_eq!(entry.status, 200);
            assert_eq!(entry.url, urls[i]);
        }

        // parallel (mixed methods)
        http_log::clear();
        let configs = vec![
            RequestConfig {
                method: "GET".to_string(),
                url: url("/g"),
                headers: vec![],
                body: None,
                timeout: None,
            },
            RequestConfig {
                method: "POST".to_string(),
                url: url("/p"),
                headers: vec![],
                body: Some("{}".to_string()),
                timeout: None,
            },
        ];
        let _ = run_parallel_requests(configs);
        let snap = http_log::snapshot();
        assert_eq!(snap.len(), 2, "parallel should record 2 entries");
        assert_eq!(snap[0].method, "GET");
        assert_eq!(snap[1].method, "POST");
        assert_eq!(snap[0].status, 200);
        assert_eq!(snap[1].status, 200);

        // Errors: no listener at the chosen port → status 0, error populated.
        http_log::clear();
        let dead_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = dead_listener.local_addr().unwrap().port();
        drop(dead_listener);
        let _ = run_parallel_gets(vec![format!("http://127.0.0.1:{}/", dead_port)], None);
        let snap = http_log::snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].status, 0);
        assert!(snap[0].error.is_some());

        // Disabled mode: same thread, same TLS LOG — recording must
        // short-circuit when neither dev log is enabled.
        http_log::set_enabled(false);
        http_log::clear();
        let _ = run_parallel_gets(vec![url("/d1"), url("/d2")], None);
        let snap = http_log::snapshot();
        assert_eq!(
            snap.len(),
            0,
            "calls must not be logged when http_log is disabled"
        );

        http_log::clear();
    }

    // SEC-016 — `is_blocked_ip` regression coverage. Each test names the
    // RFC the rule comes from so the intent stays obvious if the ranges
    // are ever touched again.
    mod ssrf_blocklist {
        use super::super::is_blocked_ip;
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

        #[test]
        fn blocks_ipv4_loopback_and_unspecified() {
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 1, 2, 3))));
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
        }

        #[test]
        fn blocks_ipv4_rfc1918_private_ranges() {
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 5, 5))));
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(172, 31, 5, 5))));
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        }

        #[test]
        fn blocks_ipv4_link_local_and_cgnat() {
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)))); // metadata
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)))); // CGNAT
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 127, 0, 1))));
        }

        #[test]
        fn blocks_ipv4_multicast() {
            assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1))));
        }

        #[test]
        fn allows_public_ipv4() {
            assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
            assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
            // 172.32 is right outside 172.16/12 — stays unblocked.
            assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(172, 32, 0, 1))));
            // 100.128 is right outside 100.64/10.
            assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))));
        }

        #[test]
        fn blocks_ipv6_loopback_and_unspecified() {
            assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
            assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        }

        #[test]
        fn blocks_ipv4_mapped_ipv6() {
            // `::ffff:127.0.0.1` — the bypass the previous IPv6 branch missed.
            let mapped = Ipv4Addr::new(127, 0, 0, 1).to_ipv6_mapped();
            assert!(is_blocked_ip(IpAddr::V6(mapped)));
            // Same for an RFC1918 mapped address.
            let mapped_priv = Ipv4Addr::new(10, 0, 0, 1).to_ipv6_mapped();
            assert!(is_blocked_ip(IpAddr::V6(mapped_priv)));
            // Mapped public IP stays unblocked.
            let mapped_public = Ipv4Addr::new(8, 8, 8, 8).to_ipv6_mapped();
            assert!(!is_blocked_ip(IpAddr::V6(mapped_public)));
        }

        #[test]
        fn blocks_ipv6_unique_local() {
            // fc00::/7 — both fc00::/8 and fd00::/8.
            assert!(is_blocked_ip(IpAddr::V6("fc00::1".parse().unwrap())));
            assert!(is_blocked_ip(IpAddr::V6(
                "fd12:3456:789a::1".parse().unwrap()
            )));
        }

        #[test]
        fn blocks_ipv6_link_local() {
            assert!(is_blocked_ip(IpAddr::V6("fe80::1".parse().unwrap())));
            assert!(is_blocked_ip(IpAddr::V6("febf::ffff".parse().unwrap())));
        }

        #[test]
        fn blocks_ipv6_site_local_deprecated() {
            assert!(is_blocked_ip(IpAddr::V6("fec0::1".parse().unwrap())));
        }

        #[test]
        fn blocks_ipv6_multicast_and_documentation_and_discard() {
            assert!(is_blocked_ip(IpAddr::V6("ff01::1".parse().unwrap())));
            assert!(is_blocked_ip(IpAddr::V6("ff02::1".parse().unwrap())));
            assert!(is_blocked_ip(IpAddr::V6("2001:db8::1".parse().unwrap())));
            assert!(is_blocked_ip(IpAddr::V6("100::1".parse().unwrap())));
        }

        #[test]
        fn allows_public_ipv6() {
            assert!(!is_blocked_ip(IpAddr::V6(
                "2606:4700:4700::1111".parse().unwrap()
            ))); // Cloudflare
            assert!(!is_blocked_ip(IpAddr::V6(
                "2001:4860:4860::8888".parse().unwrap()
            ))); // Google
        }
    }

    // SEC-018 — `http_max_response_bytes` honours the env override.
    #[test]
    fn http_max_response_bytes_default_50_mib() {
        // The OnceLock caches the first read. We can't safely set an env
        // and then expect it to be read by the global helper, but we can
        // assert the parser logic by reading directly.
        let from_env = std::env::var("SOLI_HTTP_MAX_RESPONSE_BYTES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(50 * 1024 * 1024);
        assert_eq!(super::http_max_response_bytes(), from_env);
    }

    #[test]
    fn ssrf_validate_ignores_app_env_test() {
        // The blocklist is bypassed only by the explicit test-mode flag — never
        // by `APP_ENV` (which `validate_url_for_ssrf` doesn't read). Passing the
        // flag directly keeps this race-free with tests that toggle the
        // process-global `SSRF_TEST_MODE`.
        let err = super::validate_url_for_ssrf_impl("http://127.0.0.1/", false).unwrap_err();
        assert!(
            err.contains("private/localhost"),
            "blocklist must fire when test-mode is off; got: {}",
            err
        );
        // With the flag on, loopback is allowed.
        assert!(super::validate_url_for_ssrf_impl("http://127.0.0.1/", true).is_ok());
    }

    // SEC-087 — IPv6 literals must be parsed correctly so the SSRF blocklist
    // sees the real address, not the `[` byte the previous string-splitting
    // parser handed it.

    // SEC-087 — IPv6 literals must be parsed correctly so the SSRF blocklist
    // sees the real address, not the `[` byte the previous string-splitting
    // parser handed it. These pass `false` for `test_mode` so the blocklist is
    // forced active without touching the process-global `SSRF_TEST_MODE`.

    #[test]
    fn ssrf_rejects_ipv6_loopback_literal() {
        for url in ["http://[::1]/", "http://[::1]:8080/path"] {
            let err = super::validate_url_for_ssrf_impl(url, false)
                .unwrap_err_or_else_else_panic_with_url(url);
            assert!(
                err.contains("private/localhost"),
                "expected loopback rejection for {}, got: {}",
                url,
                err
            );
        }
    }

    #[test]
    fn ssrf_rejects_ipv4_mapped_ipv6_loopback() {
        // `[::ffff:127.0.0.1]` — IPv4-mapped IPv6 form of 127.0.0.1.
        // is_blocked_ip's v6_to_ipv4_mapped branch handles this; the
        // SEC-087 fix is making sure the URL parser hands the right
        // address to that branch in the first place.
        let err = super::validate_url_for_ssrf_impl("http://[::ffff:127.0.0.1]/", false)
            .expect_err("IPv4-mapped loopback must be rejected");
        assert!(err.contains("private/localhost"), "{}", err);
    }

    #[test]
    fn ssrf_rejects_ipv6_ula_literal() {
        let err = super::validate_url_for_ssrf_impl("http://[fd00::1]/", false)
            .expect_err("ULA fd00::/8 must be rejected");
        assert!(err.contains("private/localhost"), "{}", err);
    }

    #[test]
    fn ssrf_rejects_ipv6_link_local_literal() {
        let err = super::validate_url_for_ssrf_impl("http://[fe80::1]/", false)
            .expect_err("link-local fe80::/10 must be rejected");
        assert!(err.contains("private/localhost"), "{}", err);
    }

    #[test]
    fn ssrf_rejects_ipv6_documentation_prefix() {
        let err = super::validate_url_for_ssrf_impl("http://[2001:db8::1]/", false)
            .expect_err("RFC3849 docs prefix must be rejected");
        assert!(err.contains("private/localhost"), "{}", err);
    }

    #[test]
    fn ssrf_allows_public_ipv6_literal() {
        // Cloudflare DNS is the canonical "public reachable IPv6" — make
        // sure the new parser-based path doesn't accidentally over-block.
        super::validate_url_for_ssrf_impl("http://[2606:4700:4700::1111]/", false)
            .expect("public IPv6 literal must be allowed");
    }

    #[test]
    fn ssrf_existing_ipv4_and_hostname_behaviour_unchanged() {
        // SEC-087 regression coverage: the parser swap must not change
        // the behaviour for the two cases that already worked.
        let err = super::validate_url_for_ssrf_impl("http://127.0.0.1/", false)
            .expect_err("IPv4 loopback must still be rejected");
        assert!(err.contains("private/localhost"), "{}", err);
        super::validate_url_for_ssrf_impl("https://example.com/", false)
            .expect("public hostname must remain allowed");
        let err = super::validate_url_for_ssrf_impl("file:///etc/passwd", false)
            .expect_err("file:// must remain rejected");
        assert!(err.contains("not allowed"), "{}", err);
    }

    /// Tiny extension trait so the test loop above can produce a useful
    /// panic message when `validate_url_for_ssrf` unexpectedly succeeds.
    trait UnwrapErrPanic<T> {
        fn unwrap_err_or_else_else_panic_with_url(self, url: &str) -> T;
    }
    impl<T, E: std::fmt::Display> UnwrapErrPanic<E> for Result<T, E> {
        fn unwrap_err_or_else_else_panic_with_url(self, url: &str) -> E {
            match self {
                Ok(_) => panic!("expected error for {}, got Ok", url),
                Err(e) => e,
            }
        }
    }
}

#[cfg(test)]
mod per_call_timeout_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn opts(pairs: Vec<(&str, Value)>) -> Value {
        let mut h = HashPairs::default();
        for (k, v) in pairs {
            h.insert(HashKey::String(k.into()), v);
        }
        Value::Hash(Rc::new(RefCell::new(h)))
    }

    #[test]
    fn extract_timeout_parses_int_and_float_seconds() {
        let h = opts(vec![("timeout", Value::Int(5))]);
        assert_eq!(
            extract_timeout(Some(&h)).unwrap(),
            Some(std::time::Duration::from_secs(5))
        );

        let h = opts(vec![("timeout", Value::Float(0.25))]);
        assert_eq!(
            extract_timeout(Some(&h)).unwrap(),
            Some(std::time::Duration::from_millis(250))
        );
    }

    #[test]
    fn extract_timeout_absent_or_null_is_none() {
        // No options hash at all.
        assert_eq!(extract_timeout(None).unwrap(), None);
        // Hash present but no `timeout` key.
        let h = opts(vec![("headers", opts(vec![]))]);
        assert_eq!(extract_timeout(Some(&h)).unwrap(), None);
        // Explicit null falls back to the client default.
        let h = opts(vec![("timeout", Value::Null)]);
        assert_eq!(extract_timeout(Some(&h)).unwrap(), None);
        // A non-hash arg is ignored, not an error.
        assert_eq!(
            extract_timeout(Some(&Value::String("x".into()))).unwrap(),
            None
        );
    }

    #[test]
    fn extract_timeout_rejects_bad_values() {
        for bad in [Value::Int(0), Value::Int(-3), Value::Float(-1.0)] {
            let h = opts(vec![("timeout", bad)]);
            assert!(
                extract_timeout(Some(&h)).is_err(),
                "non-positive timeout should error"
            );
        }
        let h = opts(vec![("timeout", Value::String("nope".into()))]);
        assert!(
            extract_timeout(Some(&h)).is_err(),
            "non-numeric timeout should error"
        );
    }

    /// A server that accepts the connection and reads the request but never
    /// replies, so the only way the client returns is via the per-call
    /// timeout. The handler thread sleeps well past the test's timeout and is
    /// reaped when the process exits.
    fn spawn_stalling_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                thread::spawn(move || {
                    let mut s = stream;
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf);
                    thread::sleep(std::time::Duration::from_secs(30));
                    let _ = s.write_all(b"");
                });
            }
        });
        port
    }

    #[test]
    fn short_timeout_aborts_slow_request() {
        let port = spawn_stalling_server();
        let url = format!("http://127.0.0.1:{}/", port);
        let started = std::time::Instant::now();
        let results = run_parallel_gets(vec![url], Some(std::time::Duration::from_millis(300)));
        let elapsed = started.elapsed();

        assert_eq!(results.len(), 1);
        results[0]
            .as_ref()
            .expect_err("a short per-call timeout should error out");
        // The connect succeeds (the server accepts), so the request can only
        // return via the per-call deadline. Returning around the configured
        // 300ms — and far under the client-wide 30s default — proves the
        // per-call timeout fired rather than a connect refusal or the client
        // default. (reqwest's top-level error string is generic, so we assert
        // on timing rather than message text.)
        assert!(
            elapsed >= std::time::Duration::from_millis(200)
                && elapsed < std::time::Duration::from_secs(5),
            "expected the request to abort near the 300ms per-call timeout, took {:?}",
            elapsed
        );
    }
}
