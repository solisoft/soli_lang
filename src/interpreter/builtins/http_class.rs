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
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;

use reqwest::Client;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{
    hash_from_pairs, Class, FutureState, HashKey, HashPairs, HttpFutureKind, NativeFunction, Value,
};
use crate::serve::get_tokio_handle;

const BLOCKED_SCHEMES: &[&str] = &["javascript", "file", "ftp", "ssh", "telnet", "gopher"];

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
    let url = url.trim();

    if url.is_empty() {
        return Err("URL cannot be empty".to_string());
    }

    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_lowercase(), r),
        None => {
            return Err("URL must have a scheme (e.g., http:// or https://)".to_string());
        }
    };

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

    let host = if let Some((h, _)) = rest.split_once('/') {
        if let Some((_, h2)) = h.split_once('@') {
            h2
        } else {
            h
        }
    } else if let Some((_, h)) = rest.split_once('@') {
        h
    } else {
        rest
    };

    let host = if let Some((h, _)) = host.split_once(':') {
        h
    } else {
        host
    };

    if host.is_empty() {
        return Err("URL host cannot be empty".to_string());
    }

    if is_blocked_host(host) {
        // Under the test runner (`APP_ENV=test` is set by `soli test`), allow
        // loopback/private hosts so specs can reach their own test server.
        // Production/dev requests remain blocked.
        if std::env::var("APP_ENV").as_deref() != Ok("test") {
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
fn is_blocked_ip(ip: IpAddr) -> bool {
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
                    let allow_loopback = std::env::var("APP_ENV").as_deref() == Ok("test");
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

/// Internal HTTP client — used by Model queries and SoliDB's HTTP path.
/// URL is operator-configured (`SOLIDB_HOST`, typically loopback in dev
/// and a private hostname behind a VPC in production), so the SSRF
/// blocklist would refuse every connection. Default DNS resolver +
/// SEC-007 redirect policy.
pub fn get_http_client() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .redirect(build_ssrf_redirect_policy())
            .build()
            .expect("Failed to create internal HTTP client")
    })
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
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .redirect(build_ssrf_redirect_policy())
            .dns_resolver(std::sync::Arc::new(SsrfBlockingResolver))
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

fn value_to_json(value: &Value) -> Result<String, String> {
    crate::interpreter::value::stringify_to_string(value)
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
        Rc::new(NativeFunction::new("HTTP.get", Some(1), |args| {
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let resp = send_logged("GET", &url, client.get(&url)).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq_agent().get(&url).call() {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "post".to_string(),
        Rc::new(NativeFunction::new("HTTP.post", Some(2), |args| {
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
                Value::Hash(_) => value_to_json(&args[1])?,
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = client
                            .post(&url)
                            .header("Content-Type", content_type)
                            .body(body);
                        let resp = send_logged("POST", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq_agent()
                        .post(&url)
                        .set("Content-Type", &content_type)
                        .send_string(&body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "put".to_string(),
        Rc::new(NativeFunction::new("HTTP.put", Some(2), |args| {
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
                Value::Hash(_) => value_to_json(&args[1])?,
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = client
                            .put(&url)
                            .header("Content-Type", content_type)
                            .body(body);
                        let resp = send_logged("PUT", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq_agent()
                        .put(&url)
                        .set("Content-Type", &content_type)
                        .send_string(&body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "patch".to_string(),
        Rc::new(NativeFunction::new("HTTP.patch", Some(2), |args| {
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
                Value::Hash(_) => value_to_json(&args[1])?,
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = client
                            .patch(&url)
                            .header("Content-Type", content_type)
                            .body(body);
                        let resp = send_logged("PATCH", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq_agent()
                        .patch(&url)
                        .set("Content-Type", &content_type)
                        .send_string(&body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("HTTP.delete", Some(1), |args| {
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let resp = send_logged("DELETE", &url, client.delete(&url)).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        resp.text().await.map_err(|e| e.to_string())
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq_agent().delete(&url).call() {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "head".to_string(),
        Rc::new(NativeFunction::new("HTTP.head", Some(1), |args| {
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let resp = send_logged("HEAD", &url, client.head(&url)).await?;
                        let status = resp.status().as_u16();
                        Ok(format!(
                            "{} {}",
                            status,
                            resp.status().canonical_reason().unwrap_or("")
                        ))
                    }) {
                        Ok(text) => Ok(Value::String(text)),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_http_future(
                    move || match ureq_agent().head(&url).call() {
                        Ok(response) => {
                            let status = response.status();
                            Ok(format!("{} {}", status, response.status_text()))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::String,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "get_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_json", Some(1), |args| {
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = client.get(&url).header("Accept", "application/json");
                        let resp = send_logged("GET", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = resp.text().await.map_err(|e| e.to_string())?;
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
                    move || match ureq_agent()
                        .get(&url)
                        .set("Accept", "application/json")
                        .call()
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "post_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.post_json", Some(2), |args| {
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = client
                            .post(&url)
                            .header("Content-Type", "application/json")
                            .body(json_body);
                        let resp = send_logged("POST", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = resp.text().await.map_err(|e| e.to_string())?;
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
                    move || match ureq_agent()
                        .post(&url)
                        .set("Content-Type", "application/json")
                        .send_string(&json_body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "put_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.put_json", Some(2), |args| {
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = client
                            .put(&url)
                            .header("Content-Type", "application/json")
                            .body(json_body);
                        let resp = send_logged("PUT", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = resp.text().await.map_err(|e| e.to_string())?;
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
                    move || match ureq_agent()
                        .put(&url)
                        .set("Content-Type", "application/json")
                        .send_string(&json_body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
                    },
                    HttpFutureKind::Json,
                )),
            }
        })),
    );

    http_static_methods.insert(
        "patch_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.patch_json", Some(2), |args| {
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

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match http_block_on(&rt, async move {
                        let req = client
                            .patch(&url)
                            .header("Content-Type", "application/json")
                            .body(json_body);
                        let resp = send_logged("PATCH", &url, req).await?;

                        let status = resp.status();
                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {} error: {}", status.as_u16(), body));
                        }

                        let text = resp.text().await.map_err(|e| e.to_string())?;
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
                    move || match ureq_agent()
                        .patch(&url)
                        .set("Content-Type", "application/json")
                        .send_string(&json_body)
                    {
                        Ok(response) => response
                            .into_string()
                            .map_err(|e| format!("Failed to read response body: {}", e)),
                        Err(ureq::Error::Status(code, response)) => {
                            let body = response.into_string().unwrap_or_default();
                            Err(format!("HTTP {} error: {}", code, body))
                        }
                        Err(e) => Err(format!("HTTP request failed: {}", e)),
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

            let mut headers_vec: Vec<(String, String)> = Vec::new();
            if args.len() > 2 {
                if let Value::Hash(headers) = &args[2] {
                    for (key, value) in headers.borrow().iter() {
                        let key_str = match key {
                            HashKey::String(s) => s.clone(),
                            _ => continue,
                        };
                        let value_str = match value {
                            Value::String(s) => s.clone(),
                            _ => format!("{}", value),
                        };
                        headers_vec.push((key_str, value_str));
                    }
                }
            }

            let body_opt: Option<String> = if args.len() > 3 {
                Some(match &args[3] {
                    Value::String(s) => s.clone(),
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
                            "GET" => client.get(&url),
                            "POST" => client.post(&url),
                            "PUT" => client.put(&url),
                            "DELETE" => client.delete(&url),
                            "PATCH" => client.patch(&url),
                            "HEAD" => client.head(&url),
                            _ => return Err(format!("Unsupported HTTP method: {}", method_clone)),
                        };

                        for (key, value) in &headers_vec_clone {
                            request = request.header(key.as_str(), value.as_str());
                        }

                        if let Some(body) = body_opt_clone {
                            request = request.body(body);
                        }

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

                        let body = resp.text().await.map_err(|e| e.to_string())?;

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
                            let mut request = match method_clone.as_str() {
                                "GET" => ureq_agent().get(&url),
                                "POST" => ureq_agent().post(&url),
                                "PUT" => ureq_agent().put(&url),
                                "DELETE" => ureq_agent().delete(&url),
                                "PATCH" => ureq_agent().patch(&url),
                                "HEAD" => ureq_agent().head(&url),
                                _ => {
                                    return Err(format!(
                                        "Unsupported HTTP method: {}",
                                        method_clone
                                    ))
                                }
                            };

                            for (key, value) in &headers_vec_clone {
                                request = request.set(key, value);
                            }

                            let response = if let Some(body) = body_opt_clone {
                                request.send_string(&body)
                            } else {
                                request.call()
                            };

                            match response {
                                Ok(resp) => {
                                    let status = resp.status();
                                    let status_text = resp.status_text().to_string();

                                    let mut headers_map = serde_json::Map::new();
                                    for name in resp.headers_names() {
                                        if let Some(value) = resp.header(&name) {
                                            headers_map.insert(
                                                name,
                                                serde_json::Value::String(value.to_string()),
                                            );
                                        }
                                    }

                                    let body = resp.into_string().map_err(|e| {
                                        format!("Failed to read response body: {}", e)
                                    })?;

                                    let result = serde_json::json!({
                                        "status": status,
                                        "status_text": status_text,
                                        "headers": headers_map,
                                        "body": body
                                    });

                                    Ok(result.to_string())
                                }
                                Err(ureq::Error::Status(code, resp)) => {
                                    let status_text = resp.status_text().to_string();
                                    let body = resp.into_string().unwrap_or_default();

                                    let result = serde_json::json!({
                                        "status": code,
                                        "status_text": status_text,
                                        "headers": {},
                                        "body": body
                                    });

                                    Ok(result.to_string())
                                }
                                Err(e) => Err(format!("HTTP request failed: {}", e)),
                            }
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
            |args| value_to_json(&args[0]).map(Value::String),
        )),
    );

    http_static_methods.insert(
        "get_all".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_all", Some(1), |args| {
            let urls = match &args[0] {
                Value::Array(arr) => {
                    let mut url_strings = Vec::new();
                    for item in arr.borrow().iter() {
                        match item {
                            Value::String(s) => url_strings.push(s.clone()),
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

            for u in &urls {
                validate_url_for_ssrf(u)?;
            }

            let results = run_parallel_gets(urls);

            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(body) => Value::String(body),
                    Err(e) => hash_from_pairs([("error".to_string(), Value::String(e))]),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    http_static_methods.insert(
        "get_all_json".to_string(),
        Rc::new(NativeFunction::new("HTTP.get_all_json", Some(1), |args| {
            let urls = match &args[0] {
                Value::Array(arr) => {
                    let mut url_strings = Vec::new();
                    for item in arr.borrow().iter() {
                        match item {
                            Value::String(s) => url_strings.push(s.clone()),
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

            for u in &urls {
                validate_url_for_ssrf(u)?;
            }

            let results = run_parallel_gets_json(urls);

            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(value) => value,
                    Err(e) => hash_from_pairs([("error".to_string(), Value::String(e))]),
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

            for c in &requests {
                validate_url_for_ssrf(&c.url)?;
            }

            let results = run_parallel_requests(requests);

            let values: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(response) => response_to_value(response),
                    Err(e) => hash_from_pairs([("error".to_string(), Value::String(e))]),
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
                HashKey::String(k),
                Value::String(v.as_str().unwrap_or("").to_string()),
            )
        })
        .collect();

    let mut result: HashPairs = HashPairs::default();
    result.insert(
        HashKey::String("status".to_string()),
        Value::Int(status as i64),
    );
    result.insert(
        HashKey::String("status_text".to_string()),
        Value::String(status_text),
    );
    result.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(response_headers))),
    );
    result.insert(HashKey::String("body".to_string()), Value::String(body));

    Ok(Value::Hash(Rc::new(RefCell::new(result))))
}

#[derive(Clone)]
struct RequestConfig {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
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
            url: url.clone(),
            headers: vec![],
            body: None,
        }),
        Value::Hash(hash) => {
            let hash = hash.borrow();
            let mut url = None;
            let mut method = "GET".to_string();
            let mut headers = vec![];
            let mut body = None;

            for (k, v) in hash.iter() {
                if let HashKey::String(key) = k {
                    match key.as_str() {
                        "url" => {
                            if let Value::String(s) = v {
                                url = Some(s.clone());
                            }
                        }
                        "method" => {
                            if let Value::String(s) = v {
                                method = s.to_uppercase();
                            }
                        }
                        "headers" => {
                            if let Value::Hash(h) = v {
                                for (hk, hv) in h.borrow().iter() {
                                    if let (HashKey::String(k), Value::String(v)) = (hk, hv) {
                                        headers.push((k.clone(), v.clone()));
                                    }
                                }
                            }
                        }
                        "body" => match v {
                            Value::String(s) => body = Some(s.clone()),
                            Value::Hash(_) => body = Some(value_to_json(v)?),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }

            let url = url.ok_or("Request config must have 'url' field")?;
            Ok(RequestConfig {
                method,
                url,
                headers,
                body,
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

fn run_parallel_gets(urls: Vec<String>) -> Vec<Result<String, String>> {
    let handles: Vec<_> = urls
        .into_iter()
        .map(|url| {
            thread::spawn(move || {
                let start = std::time::Instant::now();
                let (status, body) = match ureq_agent().get(&url).call() {
                    Ok(response) => {
                        let status = response.status();
                        let body = response
                            .into_string()
                            .map_err(|e| format!("Failed to read response: {}", e));
                        (status, body)
                    }
                    Err(ureq::Error::Status(code, response)) => {
                        let body = response.into_string().unwrap_or_default();
                        (code, Err(format!("HTTP {} error: {}", code, body)))
                    }
                    Err(e) => (0, Err(format!("Request failed: {}", e))),
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

    let joined: Vec<_> = handles
        .into_iter()
        .map(|h| {
            h.join().unwrap_or_else(|_| {
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
            })
        })
        .collect();

    let (results, stats): (Vec<_>, Vec<_>) = joined.into_iter().unzip();
    record_parallel_stats(stats);
    results
}

fn run_parallel_gets_json(urls: Vec<String>) -> Vec<Result<Value, String>> {
    // Fetch bodies on worker threads, then parse JSON on the main thread —
    // `Value` is `!Send` (contains Rc), so JSON parsing can't happen inside
    // the spawned threads.
    let handles: Vec<_> = urls
        .into_iter()
        .map(|url| {
            thread::spawn(move || {
                let start = std::time::Instant::now();
                let (status, body) = match ureq_agent()
                    .get(&url)
                    .set("Accept", "application/json")
                    .call()
                {
                    Ok(response) => {
                        let status = response.status();
                        let body = response
                            .into_string()
                            .map_err(|e| format!("Failed to read response: {}", e));
                        (status, body)
                    }
                    Err(ureq::Error::Status(code, response)) => {
                        let body = response.into_string().unwrap_or_default();
                        (code, Err(format!("HTTP {} error: {}", code, body)))
                    }
                    Err(e) => (0, Err(format!("Request failed: {}", e))),
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

    let joined: Vec<_> = handles
        .into_iter()
        .map(|h| {
            h.join().unwrap_or_else(|_| {
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
            })
        })
        .collect();

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
    let handles: Vec<_> = requests
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

    let joined: Vec<_> = handles
        .into_iter()
        .map(|h| {
            h.join().unwrap_or_else(|_| {
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
            })
        })
        .collect();

    let (results, stats): (Vec<_>, Vec<_>) = joined.into_iter().unzip();
    record_parallel_stats(stats);
    results
}

fn execute_request(config: RequestConfig) -> Result<HttpResponse, String> {
    let mut request = match config.method.as_str() {
        "GET" => ureq_agent().get(&config.url),
        "POST" => ureq_agent().post(&config.url),
        "PUT" => ureq_agent().put(&config.url),
        "DELETE" => ureq_agent().delete(&config.url),
        "PATCH" => ureq_agent().patch(&config.url),
        "HEAD" => ureq_agent().head(&config.url),
        _ => return Err(format!("Unsupported HTTP method: {}", config.method)),
    };

    for (key, value) in &config.headers {
        request = request.set(key, value);
    }

    let response = if let Some(body) = config.body {
        request.send_string(&body)
    } else {
        request.call()
    };

    match response {
        Ok(resp) => {
            let status = resp.status();
            let status_text = resp.status_text().to_string();
            let mut headers = vec![];
            for name in resp.headers_names() {
                if let Some(value) = resp.header(&name) {
                    headers.push((name, value.to_string()));
                }
            }
            let body = resp
                .into_string()
                .map_err(|e| format!("Failed to read response: {}", e))?;

            Ok(HttpResponse {
                status,
                status_text,
                headers,
                body,
            })
        }
        Err(ureq::Error::Status(code, resp)) => {
            let status_text = resp.status_text().to_string();
            let body = resp.into_string().unwrap_or_default();
            Ok(HttpResponse {
                status: code,
                status_text,
                headers: vec![],
                body,
            })
        }
        Err(e) => Err(format!("Request failed: {}", e)),
    }
}

fn response_to_value(response: HttpResponse) -> Value {
    let headers: HashPairs = response
        .headers
        .into_iter()
        .map(|(k, v)| (HashKey::String(k), Value::String(v)))
        .collect();

    let mut result: HashPairs = HashPairs::default();
    result.insert(
        HashKey::String("status".to_string()),
        Value::Int(response.status as i64),
    );
    result.insert(
        HashKey::String("status_text".to_string()),
        Value::String(response.status_text),
    );
    result.insert(
        HashKey::String("headers".to_string()),
        Value::Hash(Rc::new(RefCell::new(headers))),
    );
    result.insert(
        HashKey::String("body".to_string()),
        Value::String(response.body),
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
        let err = (f.func)(vec![arr(vec![Value::String(
            "file:///etc/passwd".to_string(),
        )])])
        .expect_err("get_all should reject file:// URLs");
        assert!(err.contains("not allowed"), "got: {}", err);
    }

    #[test]
    fn get_all_json_rejects_blocked_scheme() {
        let f = http_static("get_all_json");
        let err = (f.func)(vec![arr(vec![
            Value::String("https://example.com/ok".to_string()),
            Value::String("gopher://internal/".to_string()),
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
                HashKey::String("url".to_string()),
                Value::String("ftp://internal/etc/passwd".to_string()),
            );
            Value::Hash(Rc::new(RefCell::new(h)))
        };
        let err = (f.func)(vec![arr(vec![cfg])]).expect_err("parallel should reject ftp:// URLs");
        assert!(err.contains("not allowed"), "got: {}", err);
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
        let _ = run_parallel_gets(urls.clone());
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
        let _ = run_parallel_gets_json(urls.clone());
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
            },
            RequestConfig {
                method: "POST".to_string(),
                url: url("/p"),
                headers: vec![],
                body: Some("{}".to_string()),
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
        let _ = run_parallel_gets(vec![format!("http://127.0.0.1:{}/", dead_port)]);
        let snap = http_log::snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].status, 0);
        assert!(snap[0].error.is_some());

        // Disabled mode: same thread, same TLS LOG — recording must
        // short-circuit when neither dev log is enabled.
        http_log::set_enabled(false);
        http_log::clear();
        let _ = run_parallel_gets(vec![url("/d1"), url("/d2")]);
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
}
