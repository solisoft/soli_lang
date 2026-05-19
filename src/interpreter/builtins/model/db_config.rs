use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use lazy_static::lazy_static;

thread_local! {
    /// Per-thread DB name override. When set, replaces the cached
    /// `SOLIDB_DATABASE` env value for the current thread. Used by the
    /// parallel test runner so each worker writes to its own database.
    static DB_OVERRIDE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Install a per-thread DB name. Subsequent `get_database_name()` /
/// `get_cursor_url()` calls on this thread will use `name` instead of
/// the value cached from `SOLIDB_DATABASE`.
pub fn set_database_override(name: String) {
    DB_OVERRIDE.with(|o| *o.borrow_mut() = Some(name));
}

/// Clear the per-thread DB override.
pub fn clear_database_override() {
    DB_OVERRIDE.with(|o| *o.borrow_mut() = None);
}

fn override_database() -> Option<String> {
    DB_OVERRIDE.with(|o| o.borrow().clone())
}

/// Cached database configuration to avoid repeated env::var() lookups.
pub struct DbConfig {
    /// Scheme to use when building DB URLs. SEC-027: preserved from
    /// `SOLIDB_HOST` if explicit; otherwise defaults to `https://` for
    /// remote hosts and `http://` for loopback. Previously the scheme
    /// was stripped and forced to `http://` regardless of operator
    /// intent, making TLS impossible for the model layer.
    pub scheme: String,
    pub host: String,
}

impl DbConfig {
    fn from_env() -> Self {
        let raw =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        let (scheme, host) = parse_solidb_host(&raw);
        Self { scheme, host }
    }
}

/// SEC-027: split `SOLIDB_HOST` into `(scheme, host)`. Preserves an
/// explicit `http://` / `https://` prefix; for unscheme'd values picks
/// `http://` for loopback and `https://` for everything else so a
/// remote DB is TLS by default.
pub(super) fn parse_solidb_host(raw: &str) -> (String, String) {
    let raw = raw.trim();
    let trimmed = raw.trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("https://") {
        return ("https://".to_string(), rest.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("http://") {
        return ("http://".to_string(), rest.to_string());
    }
    let scheme = if is_loopback_db_host(trimmed) {
        "http://"
    } else {
        "https://"
    };
    (scheme.to_string(), trimmed.to_string())
}

/// SEC-027: detect loopback hosts so `parse_solidb_host` can default an
/// unscheme'd `localhost:6745` to `http://` instead of breaking the
/// common `soli new` / dev-loop deployment.
fn is_loopback_db_host(host: &str) -> bool {
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let hostname = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if host.matches(':').count() >= 2 {
        host
    } else {
        host.split(':').next().unwrap_or(host)
    };
    let lower = hostname.to_ascii_lowercase();
    if lower == "localhost" || lower.starts_with("localhost.") {
        return true;
    }
    if let Ok(ip) = lower.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

/// Cached JWT for the SolidB model layer.
///
/// SolidB issues 24h-TTL JWTs via `/auth/login` with no refresh hint or
/// retry-on-401 path. The original implementation cached the token in a
/// `OnceLock<Option<String>>`, freezing the first response for the lifetime
/// of the process — so any worker still up past 24h would 401 on every
/// query until it was restarted. Now we store the decoded `exp` claim
/// alongside the token and pre-emptively re-login when within
/// `JWT_REFRESH_LEEWAY_SECS` of expiry. `force_refresh_jwt_token()` is the
/// escape hatch for the request-time 401 retry path in `crud.rs`.
#[derive(Clone)]
struct CachedJwt {
    token: String,
    /// Epoch seconds when the token expires. `0` means we couldn't decode
    /// an `exp` claim (unusual token shape) — in that case we don't
    /// pre-emptively refresh; the 401-retry path is the safety net.
    exp_epoch: u64,
}

static JWT_CACHE: Mutex<Option<CachedJwt>> = Mutex::new(None);

/// Refresh `JWT_REFRESH_LEEWAY_SECS` seconds before the token expires, so a
/// long request that crosses the boundary doesn't 401 mid-flight.
const JWT_REFRESH_LEEWAY_SECS: u64 = 60;

/// Cached DB config - initialized on first use.
static CACHED_DB_CONFIG: OnceLock<CachedDbConfig> = OnceLock::new();

struct CachedDbConfig {
    cursor_url: String,
    database: String,
    api_key: Option<String>,
    basic_auth: Option<String>,
}

lazy_static! {
    /// Cached DB configuration (for username/password which are less likely to change).
    pub static ref DB_CONFIG: DbConfig = DbConfig::from_env();
}

pub fn init_jwt_token() {
    let _ = get_jwt_token();
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Extract the `exp` claim (epoch seconds) from a JWT without verifying its
/// signature. SolidB does the real verification on every request — here we
/// only need the expiry so we can pre-emptively refresh. Returns `0` if the
/// token is shaped wrong or has no `exp` claim, which disables pre-emptive
/// refresh for that token (the 401-retry path still covers it).
pub(super) fn jwt_exp(token: &str) -> u64 {
    let mut parts = token.splitn(3, '.');
    let _header = parts.next();
    let Some(payload_b64) = parts.next() else {
        return 0;
    };
    // JWTs use URL-safe base64 without padding.
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    let bytes = match URL_SAFE_NO_PAD.decode(payload_b64) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(j) => j,
        Err(_) => return 0,
    };
    json.get("exp").and_then(|v| v.as_u64()).unwrap_or(0)
}

/// Hit `/auth/login` and return the fresh token + decoded exp. Returns
/// `None` if credentials are missing or the login attempt fails — the
/// caller decides what to do with any previously-cached token in that
/// case.
fn login_for_token() -> Option<CachedJwt> {
    let (username, password) = match (
        std::env::var("SOLIDB_USERNAME").ok(),
        std::env::var("SOLIDB_PASSWORD").ok(),
    ) {
        (Some(u), Some(p)) => (u, p),
        _ => return None,
    };
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
    let login_url = format!("{}/auth/login", host);
    let payload = serde_json::json!({
        "username": username,
        "password": password,
    });
    // SEC-007a: route through the redirect-disabled shared agent for
    // consistency with the rest of the HTTP layer. SOLIDB_HOST is
    // operator-configured (not user input) so this is hardening, not
    // closing an exploit.
    let resp = match crate::interpreter::builtins::http_class::ureq_agent()
        .post(&login_url)
        .set("Content-Type", "application/json")
        .send_string(&payload.to_string())
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Warning: JWT login failed ({}), falling back", e);
            return None;
        }
    };
    let body = crate::interpreter::builtins::http_class::read_capped_text_sync(resp).ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let token = json.get("token").and_then(|t| t.as_str())?.to_string();
    let exp_epoch = jwt_exp(&token);
    Some(CachedJwt { token, exp_epoch })
}

/// Pure decision function: does this cache entry need a refresh right
/// now? Split out from `get_jwt_token` so it can be unit-tested without
/// touching the network. Rules:
/// - no cached entry → must refresh
/// - cached entry with `exp == 0` (couldn't decode exp claim) → don't
///   refresh pre-emptively; the 401-retry path is the safety net
/// - otherwise → refresh once we're within `JWT_REFRESH_LEEWAY_SECS` of
///   expiry
fn needs_jwt_refresh(cache: Option<&CachedJwt>, now: u64) -> bool {
    match cache {
        None => true,
        Some(entry) if entry.exp_epoch == 0 => false,
        Some(entry) => now + JWT_REFRESH_LEEWAY_SECS >= entry.exp_epoch,
    }
}

/// Returns a valid JWT for SolidB, pre-emptively refreshing when within
/// `JWT_REFRESH_LEEWAY_SECS` of expiry. `None` means no credentials are
/// configured (or the login call failed and there is no prior token to
/// fall back to) — callers in `crud.rs` then drop to API key or basic
/// auth.
pub fn get_jwt_token() -> Option<String> {
    let mut cache = JWT_CACHE.lock().ok()?;
    if needs_jwt_refresh(cache.as_ref(), now_epoch()) {
        if let Some(fresh) = login_for_token() {
            *cache = Some(fresh);
        } else if cache.is_none() {
            // No prior token and login failed → tell the caller to fall
            // back. We don't cache the failure: the next call will retry,
            // matching the behaviour of a transient network blip rather
            // than the original "cache None forever" footgun.
            return None;
        }
        // Else: we have an old token and the refresh failed. Keep
        // serving the old one; the 401-retry path will trigger
        // `force_refresh_jwt_token()` on the next failed request.
    }
    cache.as_ref().map(|e| e.token.clone())
}

/// Drop the cached JWT so the next `get_jwt_token()` call re-logs in.
/// Called from the 401-retry path in `crud.rs::send_with_db_auth_retry`
/// when a request comes back unauthorised despite carrying what we
/// believed was a valid token (clock skew, server-side revocation, etc.).
pub fn force_refresh_jwt_token() {
    if let Ok(mut cache) = JWT_CACHE.lock() {
        *cache = None;
    }
}

pub fn init_db_config() {
    let _ = get_db_config();
}

fn get_db_config() -> &'static CachedDbConfig {
    CACHED_DB_CONFIG.get_or_init(|| {
        let raw =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        let (scheme, host) = parse_solidb_host(&raw);
        let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());
        // SEC-027: build the cursor URL with the scheme `parse_solidb_host`
        // chose (preserves operator-set https://, defaults remote hosts
        // to https). Was forced to http:// regardless of intent.
        let cursor_url = format!("{}{}/_api/database/{}/cursor", scheme, host, database);
        let api_key = std::env::var("SOLIDB_API_KEY").ok();
        let basic_auth = match (
            std::env::var("SOLIDB_USERNAME").ok(),
            std::env::var("SOLIDB_PASSWORD").ok(),
        ) {
            (Some(u), Some(p)) => {
                use base64::Engine;
                Some(format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", u, p))
                ))
            }
            _ => None,
        };
        CachedDbConfig {
            cursor_url,
            database,
            api_key,
            basic_auth,
        }
    })
}

pub fn get_database_name() -> String {
    if let Some(name) = override_database() {
        return name;
    }
    get_db_config().database.clone()
}

pub fn get_cursor_url() -> String {
    if let Some(name) = override_database() {
        // SEC-027: per-thread DB-name override still uses the same
        // scheme `DbConfig::from_env` picked, not a hard-coded http://.
        return format!(
            "{}{}/_api/database/{}/cursor",
            DB_CONFIG.scheme, DB_CONFIG.host, name
        );
    }
    get_db_config().cursor_url.clone()
}

pub fn get_api_key() -> Option<&'static str> {
    get_db_config().api_key.as_deref()
}

pub fn get_basic_auth() -> Option<&'static str> {
    get_db_config().basic_auth.as_deref()
}

/// SEC-027: build a SoliDB URL using the configured scheme + host.
/// `path` is appended verbatim (e.g. `/_api/database/{db}/cursor`).
/// Use this instead of `format!("http://{}{}", DB_CONFIG.host, path)`,
/// which forces plaintext HTTP regardless of the operator's intent.
pub fn db_url(path: &str) -> String {
    format!("{}{}{}", DB_CONFIG.scheme, DB_CONFIG.host, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SEC-027: explicit schemes survive the parse round-trip.
    #[test]
    fn parse_solidb_host_preserves_explicit_scheme() {
        assert_eq!(
            parse_solidb_host("https://db.example.com:8080"),
            ("https://".to_string(), "db.example.com:8080".to_string())
        );
        assert_eq!(
            parse_solidb_host("http://db.example.com:8080"),
            ("http://".to_string(), "db.example.com:8080".to_string())
        );
        // Trailing slash trimmed.
        assert_eq!(
            parse_solidb_host("https://db.example.com/"),
            ("https://".to_string(), "db.example.com".to_string())
        );
    }

    /// SEC-027: unscheme'd hosts default to https for remote and http
    /// for loopback so the dev loop stays plaintext while remote DBs
    /// upgrade to TLS.
    #[test]
    fn parse_solidb_host_defaults_unscheme_d() {
        // Remote → https.
        assert_eq!(
            parse_solidb_host("db.internal:8080"),
            ("https://".to_string(), "db.internal:8080".to_string())
        );
        assert_eq!(
            parse_solidb_host("10.0.0.1:6745"),
            ("https://".to_string(), "10.0.0.1:6745".to_string())
        );
        // Loopback → http.
        assert_eq!(
            parse_solidb_host("localhost:6745"),
            ("http://".to_string(), "localhost:6745".to_string())
        );
        assert_eq!(
            parse_solidb_host("127.0.0.1:6745"),
            ("http://".to_string(), "127.0.0.1:6745".to_string())
        );
        assert_eq!(
            parse_solidb_host("[::1]:6745"),
            ("http://".to_string(), "[::1]:6745".to_string())
        );
    }

    #[test]
    fn loopback_db_host_basics() {
        assert!(is_loopback_db_host("localhost"));
        assert!(is_loopback_db_host("localhost:6745"));
        assert!(is_loopback_db_host("127.0.0.1"));
        assert!(is_loopback_db_host("127.1.2.3:6745"));
        assert!(is_loopback_db_host("[::1]:6745"));
        assert!(!is_loopback_db_host("db.internal"));
        assert!(!is_loopback_db_host("10.0.0.1:6745"));
    }

    // ---- JWT cache: refresh decision and exp parsing -------------------

    /// A real SolidB JWT (URL-safe base64, no padding) with `exp =
    /// 1779268350`. Captured live from `/auth/login` during the bug
    /// investigation that motivated this refactor.
    const SAMPLE_JWT_EXP_1779268350: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhZG1pbiIsImV4cCI6MTc3OTI2ODM1MCwicm9sZXMiOlsiYWRtaW4iXX0.vlcGI1Vg20pGR8NDBofUsarXN02XHOUKBQRL3Tm1vHE";

    #[test]
    fn jwt_exp_extracts_exp_claim_from_real_token() {
        // The exp claim of the live SolidB token. If SolidB ever
        // changes the claim shape, this test catches it before prod.
        assert_eq!(jwt_exp(SAMPLE_JWT_EXP_1779268350), 1_779_268_350);
    }

    #[test]
    fn jwt_exp_returns_zero_for_malformed_tokens() {
        // Empty / single-segment / two-segment / non-base64 payload /
        // valid-base64-but-not-JSON / valid-JSON-but-no-exp — all
        // collapse to 0 so the cache simply doesn't pre-emptively
        // refresh. The 401-retry path is the safety net.
        assert_eq!(jwt_exp(""), 0);
        assert_eq!(jwt_exp("only-one-segment"), 0);
        assert_eq!(jwt_exp("header.payload"), 0);
        assert_eq!(jwt_exp("header.@@@not-base64@@@.sig"), 0);
        // base64 of `not json`
        assert_eq!(jwt_exp("header.bm90IGpzb24.sig"), 0);
        // base64 of `{"sub":"x"}` — valid JSON, no exp claim
        assert_eq!(jwt_exp("header.eyJzdWIiOiJ4In0.sig"), 0);
    }

    #[test]
    fn jwt_exp_handles_non_integer_exp() {
        // Some JWT libraries serialise exp as a float. Our decoder
        // requires `as_u64`, so a float falls through to 0 — the
        // 401-retry path still covers that token. Documented here so
        // a future change to `as_f64()`-then-`as u64` is intentional.
        // base64 of `{"exp":1779268350.5}`
        let token = "header.eyJleHAiOjE3NzkyNjgzNTAuNX0.sig";
        assert_eq!(jwt_exp(token), 0);
    }

    #[test]
    fn needs_jwt_refresh_no_cache_means_refresh() {
        // First call after process start has nothing cached.
        assert!(needs_jwt_refresh(None, 1_000_000));
    }

    #[test]
    fn needs_jwt_refresh_zero_exp_never_refreshes_pre_emptively() {
        // exp == 0 marks a token we couldn't decode. We keep using it
        // and rely on the 401-retry path; otherwise we'd hammer
        // /auth/login on every request.
        let entry = CachedJwt {
            token: "opaque".to_string(),
            exp_epoch: 0,
        };
        assert!(!needs_jwt_refresh(Some(&entry), u64::MAX));
    }

    #[test]
    fn needs_jwt_refresh_fresh_token_does_not_refresh() {
        // Token still has ~24h left. Don't refresh.
        let entry = CachedJwt {
            token: "fresh".to_string(),
            exp_epoch: 2_000_000,
        };
        assert!(!needs_jwt_refresh(Some(&entry), 2_000_000 - 24 * 3600));
    }

    #[test]
    fn needs_jwt_refresh_inside_leeway_window_refreshes() {
        // Token expires in 30s — inside our 60s leeway. Refresh now so
        // a slow request doesn't cross the expiry boundary mid-flight.
        let entry = CachedJwt {
            token: "near-expiry".to_string(),
            exp_epoch: 1_000_030,
        };
        assert!(needs_jwt_refresh(Some(&entry), 1_000_000));
    }

    #[test]
    fn needs_jwt_refresh_at_exact_boundary_refreshes() {
        // `now + leeway == exp` → also refresh. The `>=` matters: an
        // off-by-one in the leeway window would let a token expire
        // exactly when a long request is in flight.
        let entry = CachedJwt {
            token: "boundary".to_string(),
            exp_epoch: 1_000_000 + JWT_REFRESH_LEEWAY_SECS,
        };
        assert!(needs_jwt_refresh(Some(&entry), 1_000_000));
    }

    #[test]
    fn needs_jwt_refresh_one_second_outside_leeway_does_not_refresh() {
        // Just outside the window — keep using the cached token.
        let entry = CachedJwt {
            token: "safe".to_string(),
            exp_epoch: 1_000_000 + JWT_REFRESH_LEEWAY_SECS + 1,
        };
        assert!(!needs_jwt_refresh(Some(&entry), 1_000_000));
    }

    #[test]
    fn force_refresh_clears_cached_token() {
        // Seed the cache, force-refresh, observe empty. This is the
        // path the 401-retry uses to recover from a server-side
        // revocation that beat the leeway window.
        {
            let mut cache = JWT_CACHE.lock().unwrap();
            *cache = Some(CachedJwt {
                token: "stale".to_string(),
                exp_epoch: u64::MAX,
            });
        }
        force_refresh_jwt_token();
        let cache = JWT_CACHE.lock().unwrap();
        assert!(cache.is_none());
    }

    // ---- End-to-end: cache → expiry → re-login regression --------------
    //
    // These tests touch SOLIDB_* env vars and the process-global
    // JWT_CACHE, so they must run serially. Pattern mirrors
    // `serve::mod::ENV_TEST_LOCK`.
    static JWT_E2E_LOCK: Mutex<()> = Mutex::new(());

    /// Build a JWT-shaped string `header.payload.sig` whose decoded
    /// payload is `{"exp": <exp_epoch>}`. The header and signature are
    /// arbitrary — `jwt_exp` only parses the middle segment.
    fn make_jwt_with_exp(exp_epoch: u64) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let payload = serde_json::json!({ "exp": exp_epoch }).to_string();
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        format!("h.{}.s", payload_b64)
    }

    /// Spawn a stub `/auth/login` server on a random localhost port.
    /// Each incoming POST gets the next token from `tokens` (cycling on
    /// the last). Returns the bound port.
    fn spawn_login_stub(tokens: Vec<String>) -> u16 {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub login");
        let port = listener.local_addr().unwrap().port();
        let tokens = Arc::new(tokens);
        let counter = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tokens = Arc::clone(&tokens);
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || {
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
                        if total.len() > 16 * 1024 {
                            break;
                        }
                    }
                    let idx = counter.fetch_add(1, Ordering::SeqCst).min(tokens.len() - 1);
                    let body = format!("{{\"token\":\"{}\"}}", tokens[idx]);
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(resp.as_bytes());
                    let _ = s.write_all(body.as_bytes());
                });
            }
        });
        port
    }

    /// Regression test for the pre-v1.3.1 bug where the JWT was cached
    /// in a `OnceLock<Option<String>>`, so the very first `/auth/login`
    /// response was reused for the lifetime of the process. SolidB
    /// rotates JWTs after 24h, so any worker that stayed up past the
    /// expiry started 401-ing on every request.
    ///
    /// We give the stub *distinguishable* tokens for the two calls so a
    /// "cache frozen forever" implementation would fail the
    /// `first != second` assertion. With the fix, the second call
    /// observes "near expiry", re-enters `login_for_token`, and the
    /// returned tokens differ.
    #[test]
    fn get_jwt_token_refreshes_when_cached_entry_is_near_expiry() {
        let _guard = JWT_E2E_LOCK.lock().unwrap();

        // Fresh cache for the test, restored on the way out.
        let saved = JWT_CACHE.lock().unwrap().take();

        // Two distinguishable tokens — different exp claims so the JWT
        // strings themselves differ. Both far enough in the future that
        // they wouldn't otherwise trip the leeway check.
        let fresh_exp_a = now_epoch() + 24 * 3600;
        let fresh_exp_b = now_epoch() + 12 * 3600;
        let port = spawn_login_stub(vec![
            make_jwt_with_exp(fresh_exp_a),
            make_jwt_with_exp(fresh_exp_b),
        ]);

        // Save + set env. SOLIDB_HOST is read lazily by login_for_token
        // every call, so both calls go to the same stub.
        let prev_host = std::env::var("SOLIDB_HOST").ok();
        let prev_user = std::env::var("SOLIDB_USERNAME").ok();
        let prev_pass = std::env::var("SOLIDB_PASSWORD").ok();
        std::env::set_var("SOLIDB_HOST", format!("http://127.0.0.1:{}", port));
        std::env::set_var("SOLIDB_USERNAME", "test-user");
        std::env::set_var("SOLIDB_PASSWORD", "test-pass");

        let first = get_jwt_token().expect("first login should succeed via stub");
        let exp_after_first = JWT_CACHE.lock().unwrap().as_ref().unwrap().exp_epoch;

        // Force the cached entry into the leeway window. The fix uses
        // `needs_jwt_refresh` to detect this and re-login; the old
        // `OnceLock` cache had no way to express this state at all.
        {
            let mut cache = JWT_CACHE.lock().unwrap();
            if let Some(entry) = cache.as_mut() {
                entry.exp_epoch = now_epoch() + JWT_REFRESH_LEEWAY_SECS / 2;
            }
        }

        let second = get_jwt_token().expect("second call should re-login via stub");
        let exp_after_second = JWT_CACHE.lock().unwrap().as_ref().unwrap().exp_epoch;

        // Restore env + cache before assertions so test failure doesn't
        // leak state into other tests in the same process.
        match prev_host {
            Some(v) => std::env::set_var("SOLIDB_HOST", v),
            None => std::env::remove_var("SOLIDB_HOST"),
        }
        match prev_user {
            Some(v) => std::env::set_var("SOLIDB_USERNAME", v),
            None => std::env::remove_var("SOLIDB_USERNAME"),
        }
        match prev_pass {
            Some(v) => std::env::set_var("SOLIDB_PASSWORD", v),
            None => std::env::remove_var("SOLIDB_PASSWORD"),
        }
        *JWT_CACHE.lock().unwrap() = saved;

        assert_eq!(
            exp_after_first, fresh_exp_a,
            "first call must cache the stub's first response"
        );
        assert_ne!(
            first, second,
            "near-expiry cache must trigger a real re-login (this fails against the old OnceLock design)"
        );
        assert_eq!(
            exp_after_second, fresh_exp_b,
            "second call must overwrite the cache with the stub's second response — proving the lifecycle is live"
        );
    }

    /// Companion test: when the cache is fresh, `get_jwt_token` must
    /// *not* hit `/auth/login` a second time. Belt-and-braces against a
    /// future regression that refreshes too eagerly and turns every DB
    /// request into a double round-trip.
    #[test]
    fn get_jwt_token_does_not_refresh_when_cache_is_fresh() {
        let _guard = JWT_E2E_LOCK.lock().unwrap();
        let saved = JWT_CACHE.lock().unwrap().take();

        // Stub serves token-1 first, then would serve token-2 — if the
        // second call somehow re-logs in, we'll see the swap.
        let port = spawn_login_stub(vec![
            make_jwt_with_exp(now_epoch() + 24 * 3600),
            make_jwt_with_exp(now_epoch() + 12 * 3600),
        ]);
        let prev_host = std::env::var("SOLIDB_HOST").ok();
        let prev_user = std::env::var("SOLIDB_USERNAME").ok();
        let prev_pass = std::env::var("SOLIDB_PASSWORD").ok();
        std::env::set_var("SOLIDB_HOST", format!("http://127.0.0.1:{}", port));
        std::env::set_var("SOLIDB_USERNAME", "test-user");
        std::env::set_var("SOLIDB_PASSWORD", "test-pass");

        let first = get_jwt_token().expect("first call hits stub");
        let exp_after_first = JWT_CACHE.lock().unwrap().as_ref().unwrap().exp_epoch;
        let second = get_jwt_token().expect("second call should reuse cache");
        let exp_after_second = JWT_CACHE.lock().unwrap().as_ref().unwrap().exp_epoch;

        match prev_host {
            Some(v) => std::env::set_var("SOLIDB_HOST", v),
            None => std::env::remove_var("SOLIDB_HOST"),
        }
        match prev_user {
            Some(v) => std::env::set_var("SOLIDB_USERNAME", v),
            None => std::env::remove_var("SOLIDB_USERNAME"),
        }
        match prev_pass {
            Some(v) => std::env::set_var("SOLIDB_PASSWORD", v),
            None => std::env::remove_var("SOLIDB_PASSWORD"),
        }
        *JWT_CACHE.lock().unwrap() = saved;

        assert_eq!(first, second, "cached call must return identical token");
        assert_eq!(
            exp_after_first, exp_after_second,
            "cache must not have been re-seeded; if exp changed, we hit /auth/login a second time"
        );
    }
}
