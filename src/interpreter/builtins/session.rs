//! Session management for Solilang.
//!
//! Provides session storage with pluggable backends (in-memory, disk, SolidB, SoliKV).
//!
//! Default backend is in-memory. Use session.configure() to switch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::interpreter::value::HashPairs;

use lazy_static::lazy_static;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SessionDriver {
    #[default]
    InMemory,
    Disk,
    Solidb,
    Solikv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SameSite {
    #[default]
    Lax,
    Strict,
    None,
}

impl std::fmt::Display for SameSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SameSite::Lax => write!(f, "Lax"),
            SameSite::Strict => write!(f, "Strict"),
            SameSite::None => write!(f, "None"),
        }
    }
}

impl std::str::FromStr for SameSite {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "strict" => Ok(SameSite::Strict),
            "none" => Ok(SameSite::None),
            "lax" | "" => Ok(SameSite::Lax),
            _ => Err(format!("Unknown SameSite value: {}", s)),
        }
    }
}

impl std::fmt::Display for SessionDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionDriver::InMemory => write!(f, "in_memory"),
            SessionDriver::Disk => write!(f, "disk"),
            SessionDriver::Solidb => write!(f, "solidb"),
            SessionDriver::Solikv => write!(f, "solikv"),
        }
    }
}

impl std::str::FromStr for SessionDriver {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "in_memory" | "inmemory" | "memory" => Ok(SessionDriver::InMemory),
            "disk" | "file" => Ok(SessionDriver::Disk),
            "solidb" | "soliddb" | "db" => Ok(SessionDriver::Solidb),
            "solikv" | "kv" | "redis" => Ok(SessionDriver::Solikv),
            _ => Err(format!("Unknown session driver: {}", s)),
        }
    }
}

pub trait SessionStore: Send + Sync {
    fn get_or_create(&self, session_id: &str) -> String;
    fn create_session(&self) -> String;
    fn get(&self, session_id: &str, key: &str) -> Option<JsonValue>;
    fn set(&self, session_id: &str, key: &str, value: JsonValue);
    fn delete(&self, session_id: &str, key: &str) -> Option<JsonValue>;
    fn destroy(&self, session_id: &str);
    fn regenerate(&self, old_id: &str) -> String;
    fn cleanup(&self);
    fn driver_name(&self) -> &'static str;

    /// Open a connection to the backing store ahead of the first request,
    /// so request handlers don't pay a cold-start. Defaults to a no-op for
    /// stores with no network backend (in-memory, disk); the SoliDB store
    /// overrides it with a cheap read-only round-trip. Must never write or
    /// mutate any session state — see `SolidbSessionStore::warm`.
    fn warm(&self) {}
}

// SEC-038a: the previous `SessionStoreManager` cached one Arc<dyn SessionStore>
// at first access via a lazy_static. `configure_session` updated CURRENT_STORE
// but the manager's cached pointer never moved, so a runtime call like
// `session_configure({"driver": "disk"})` was a silent no-op for dispatch.
// Helpers now go straight through `get_current_store()` per call, which reads
// the live `RwLock` and picks up backend swaps. The extra `RwLock::read +
// Arc::clone` per session op is negligible compared to the underlying store
// work (HashMap lookup or RESP/HTTP round-trip).

#[derive(Clone)]
pub struct SessionConfig {
    pub driver: SessionDriver,
    pub path: Option<String>,
    pub solidb_host: Option<String>,
    pub solidb_database: Option<String>,
    pub solidb_collection: Option<String>,
    /// SEC-025: API key passed to the SoliDB session backend so worker
    /// → SoliDB calls authenticate. Reads `SOLI_SOLIDB_API_KEY`, falling
    /// back to `SOLIDB_API_KEY` (the same key the Model layer uses) when
    /// the session-specific override isn't set — common-case operators
    /// only need to configure it once.
    pub solidb_api_key: Option<String>,
    /// SEC-025: basic-auth username for SoliDB sessions. Reads
    /// `SOLI_SOLIDB_USERNAME`, falling back to `SOLIDB_USERNAME`.
    pub solidb_username: Option<String>,
    /// SEC-025: basic-auth password for SoliDB sessions. Reads
    /// `SOLI_SOLIDB_PASSWORD`, falling back to `SOLIDB_PASSWORD`.
    pub solidb_password: Option<String>,
    pub solikv_host: Option<String>,
    pub solikv_port: Option<u16>,
    pub solikv_token: Option<String>,
    pub ttl: u64,
    pub same_site: SameSite,
    pub host_prefix: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        let driver = std::env::var("SOLI_SESSION_DRIVER")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(SessionDriver::InMemory);

        let path = std::env::var("SOLI_SESSION_PATH").ok();
        let solidb_host = std::env::var("SOLI_SOLIDB_HOST").ok();
        let solidb_database = std::env::var("SOLI_SOLIDB_DATABASE").ok();
        let solidb_collection = std::env::var("SOLI_SOLIDB_COLLECTION").ok();
        // SEC-025: prefer session-specific overrides, fall back to the
        // generic SOLIDB_* credentials the Model layer already reads.
        let solidb_api_key = std::env::var("SOLI_SOLIDB_API_KEY")
            .ok()
            .or_else(|| std::env::var("SOLIDB_API_KEY").ok())
            .filter(|t| !t.is_empty());
        let solidb_username = std::env::var("SOLI_SOLIDB_USERNAME")
            .ok()
            .or_else(|| std::env::var("SOLIDB_USERNAME").ok())
            .filter(|t| !t.is_empty());
        let solidb_password = std::env::var("SOLI_SOLIDB_PASSWORD")
            .ok()
            .or_else(|| std::env::var("SOLIDB_PASSWORD").ok())
            .filter(|t| !t.is_empty());
        let solikv_host = std::env::var("SOLI_SOLIKV_HOST").ok();
        let solikv_port = std::env::var("SOLI_SOLIKV_PORT")
            .ok()
            .and_then(|p| p.parse().ok());
        let solikv_token = std::env::var("SOLI_SOLIKV_TOKEN")
            .ok()
            .filter(|t| !t.is_empty());
        let ttl = std::env::var("SOLI_SESSION_TTL")
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(86400);
        let same_site = std::env::var("SOLI_SESSION_SAMESITE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(SameSite::Lax);
        let host_prefix = std::env::var("SOLI_SESSION_HOST_PREFIX")
            .ok()
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        Self {
            driver,
            path,
            solidb_host,
            solidb_database,
            solidb_collection,
            solidb_api_key,
            solidb_username,
            solidb_password,
            solikv_host,
            solikv_port,
            solikv_token,
            ttl,
            same_site,
            host_prefix,
        }
    }
}

impl SessionConfig {
    pub fn create_store(&self) -> Result<Arc<dyn SessionStore>, String> {
        // SEC-038: every store honors `SOLI_SESSION_TTL`. Previously
        // each backend hardcoded 24h, so operators who set
        // SOLI_SESSION_TTL=300 silently got 24h sessions.
        let max_age = Duration::from_secs(self.ttl);
        match self.driver {
            SessionDriver::InMemory => {
                Ok(Arc::new(InMemorySessionStore::new().with_max_age(max_age)))
            }
            SessionDriver::Disk => {
                let path = self
                    .path
                    .clone()
                    .unwrap_or_else(|| "./sessions".to_string());
                let store = crate::interpreter::builtins::session_disk::DiskSessionStore::new(
                    std::path::PathBuf::from(path),
                )
                .map_err(|e| format!("Failed to create disk session store: {}", e))?
                .with_max_age(max_age);
                Ok(Arc::new(store))
            }
            SessionDriver::Solidb => {
                let host = self
                    .solidb_host
                    .clone()
                    .unwrap_or_else(|| "localhost:8080".to_string());
                let database = self
                    .solidb_database
                    .clone()
                    .unwrap_or_else(|| "solidb".to_string());

                // SEC-025: refuse plaintext-HTTP non-loopback session storage,
                // and require auth. Sessions hold authenticated `user_id`s,
                // so anyone on the network path between the app and SoliDB
                // could otherwise read or forge user identity.
                let in_test = crate::interpreter::builtins::http_class::ssrf_test_mode();
                let allow_insecure = session_allow_insecure_http();
                let is_https = host.starts_with("https://");
                let is_loopback = is_loopback_session_host(&host);
                if !in_test && !is_https && !is_loopback && !allow_insecure {
                    return Err(format!(
                        "SoliDB session storage refuses plaintext HTTP for non-loopback host '{}'. \
                         Use https:// or set SOLI_SESSION_ALLOW_INSECURE_HTTP=1 (only when the network path is trusted).",
                        host
                    ));
                }
                let has_auth = self.solidb_api_key.is_some()
                    || (self.solidb_username.is_some() && self.solidb_password.is_some());
                if !in_test && !is_loopback && !has_auth && !allow_insecure {
                    return Err(format!(
                        "SoliDB session storage requires authentication for non-loopback host '{}'. \
                         Set SOLI_SOLIDB_API_KEY or SOLI_SOLIDB_USERNAME / SOLI_SOLIDB_PASSWORD, or set SOLI_SESSION_ALLOW_INSECURE_HTTP=1.",
                        host
                    ));
                }

                let mut store =
                    crate::interpreter::builtins::session_solidb::SolidbSessionStore::new(
                        host, database,
                    );
                if let Some(collection) = self.solidb_collection.clone() {
                    store = store.with_collection(collection);
                }
                let store = store
                    .with_auth(
                        self.solidb_api_key.clone(),
                        self.solidb_username.clone(),
                        self.solidb_password.clone(),
                    )
                    .with_max_age(max_age);
                Ok(Arc::new(store))
            }
            SessionDriver::Solikv => {
                let host = self
                    .solikv_host
                    .clone()
                    .unwrap_or_else(|| "localhost".to_string());
                let port = self.solikv_port.unwrap_or(6380);

                // SEC-026: SoliKV's RESP transport is plaintext TCP — the
                // `auth_token` is sent as a `AUTH` command in the clear,
                // so a passive sniff equals full session takeover. There
                // is no rustls path on `RespPool`; refuse non-loopback
                // hosts unless the operator explicitly opts in (same
                // `SOLI_SESSION_ALLOW_INSECURE_HTTP` knob as SEC-025).
                let in_test = crate::interpreter::builtins::http_class::ssrf_test_mode();
                let allow_insecure = session_allow_insecure_http();
                if !in_test && !is_loopback_session_host(&host) && !allow_insecure {
                    return Err(format!(
                        "SoliKV session storage is plaintext TCP and refuses non-loopback host '{}'. \
                         Move SoliKV to loopback or set SOLI_SESSION_ALLOW_INSECURE_HTTP=1 (only when the network path is trusted).",
                        host
                    ));
                }

                let store = crate::interpreter::builtins::session_solikv::SolikvSessionStore::new(
                    host,
                    port,
                    self.solikv_token.clone(),
                )
                .with_ttl(self.ttl);
                Ok(Arc::new(store))
            }
        }
    }
}

/// SEC-025: returns true when the operator has explicitly opted into
/// plaintext / no-auth session storage via `SOLI_SESSION_ALLOW_INSECURE_HTTP=1`.
/// Use sparingly — this disables the same-VPC TLS/auth requirement.
fn session_allow_insecure_http() -> bool {
    std::env::var("SOLI_SESSION_ALLOW_INSECURE_HTTP")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// SEC-025: detect loopback-only session storage hosts. Loopback has no
/// network path so plaintext / no-auth SoliDB is acceptable there.
/// Accepts the same set of names `is_blocked_host` does in the SSRF
/// guard so the two surfaces agree on what counts as loopback.
fn is_loopback_session_host(host: &str) -> bool {
    let host = host.trim_end_matches('/');
    let host = host
        .strip_prefix("http://")
        .or_else(|| host.strip_prefix("https://"))
        .unwrap_or(host);
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    // Extract just the hostname, handling bracketed `[::1]:8080`,
    // bare IPv6 like `::1`, and `host:port`.
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

lazy_static! {
    static ref SESSION_CONFIG: RwLock<SessionConfig> = RwLock::new(SessionConfig::default());
    // SEC-038: build the startup store from the env-derived config so
    // SOLI_SESSION_TTL is honored from the first request, not just after
    // an explicit `session_configure` call. Falls back to a TTL-aware
    // in-memory store if the configured backend errors at boot.
    static ref CURRENT_STORE: RwLock<Arc<dyn SessionStore>> = {
        let cfg = SessionConfig::default();
        let store = cfg.create_store().unwrap_or_else(|_| {
            Arc::new(InMemorySessionStore::new().with_max_age(Duration::from_secs(cfg.ttl)))
        });
        RwLock::new(store)
    };
}

pub fn get_session_config() -> SessionConfig {
    SESSION_CONFIG.read().unwrap().clone()
}

pub fn configure_session(config: SessionConfig) -> Result<(), String> {
    let store = config.create_store()?;
    let mut current = CURRENT_STORE.write().map_err(|e| e.to_string())?;
    *current = store;
    let mut cfg = SESSION_CONFIG.write().map_err(|e| e.to_string())?;
    *cfg = config;
    Ok(())
}

pub fn get_current_store() -> Arc<dyn SessionStore> {
    CURRENT_STORE.read().unwrap().clone()
}

/// Session data with expiration.
/// Stores data as JSON values for thread safety.
#[derive(Clone)]
struct Session {
    data: HashMap<String, JsonValue>,
    #[allow(dead_code)]
    created_at: Instant,
    last_accessed: Instant,
}

impl Session {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            data: HashMap::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }

    fn is_expired(&self, max_age: Duration) -> bool {
        self.last_accessed.elapsed() > max_age
    }
}

/// Thread-safe in-memory session store.
///
/// In production (the common case when apps do not call `session_configure`),
/// this store must not grow unbounded. We therefore clean aggressively on both
/// a request counter (legacy) and a wall-time basis so that low-traffic or
/// bursty workloads still reclaim memory from expired sessions.
pub struct InMemorySessionStore {
    sessions: RwLock<HashMap<String, Session>>,
    max_age: Duration,
    request_counter: AtomicU64,
    /// Last time we ran a full expiry sweep. Protected by a mutex because
    /// we only mutate it inside the (rare) cleanup path.
    last_cleanup: std::sync::Mutex<Instant>,
}

impl InMemorySessionStore {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_age: Duration::from_secs(24 * 60 * 60),
            request_counter: AtomicU64::new(0),
            last_cleanup: std::sync::Mutex::new(now),
        }
    }

    /// SEC-038: thread `SOLI_SESSION_TTL` into the in-memory store so
    /// short-session apps don't silently fall back to 24h.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }
}

/// How often the in-memory session store will voluntarily run expiry cleanup,
/// even if the request counter has not yet hit the old 1000-request threshold.
/// This is critical for low-traffic prod deployments (or "just testing" a
/// complex app) that use the default in_memory driver and would otherwise
/// accumulate sessions for a very long time.
const IN_MEMORY_SESSION_CLEANUP_INTERVAL: Duration = Duration::from_secs(30);

impl SessionStore for InMemorySessionStore {
    fn get_or_create(&self, session_id: &str) -> String {
        let count = self.request_counter.fetch_add(1, Ordering::Relaxed);
        let should_cleanup_by_count = count.is_multiple_of(1000);

        // Time-based cleanup: ensures that even very low traffic or bursty
        // production workloads (the exact scenario reported: 1 worker, prod
        // mode, default in_memory, complex app) will eventually reclaim memory
        // from expired sessions instead of growing for hours.
        let should_cleanup_by_time = {
            if let Ok(last) = self.last_cleanup.lock() {
                last.elapsed() >= IN_MEMORY_SESSION_CLEANUP_INTERVAL
            } else {
                false
            }
        };

        if should_cleanup_by_count || should_cleanup_by_time {
            self.cleanup();
        }

        {
            let sessions = self.sessions.read().unwrap();
            if sessions.contains_key(session_id) {
                return session_id.to_string();
            }
        }

        let mut sessions = self.sessions.write().unwrap();

        if sessions.contains_key(session_id) {
            return session_id.to_string();
        }

        let new_id = Uuid::new_v4().to_string();
        sessions.insert(new_id.clone(), Session::new());
        new_id
    }

    fn create_session(&self) -> String {
        let mut sessions = self.sessions.write().unwrap();
        let session_id = Uuid::new_v4().to_string();
        sessions.insert(session_id.clone(), Session::new());
        session_id
    }

    fn get(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        let sessions = self.sessions.read().unwrap();
        sessions
            .get(session_id)
            .and_then(|s| s.data.get(key).cloned())
    }

    fn set(&self, session_id: &str, key: &str, value: JsonValue) {
        let mut sessions = self.sessions.write().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.touch();
            session.data.insert(key.to_string(), value);
        }
    }

    fn delete(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        let mut sessions = self.sessions.write().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.touch();
            return session.data.remove(key);
        }
        None
    }

    fn destroy(&self, session_id: &str) {
        let mut sessions = self.sessions.write().unwrap();
        sessions.remove(session_id);
    }

    fn regenerate(&self, old_id: &str) -> String {
        let mut sessions = self.sessions.write().unwrap();
        let new_id = Uuid::new_v4().to_string();

        if let Some(session) = sessions.remove(old_id) {
            sessions.insert(new_id.clone(), session);
        } else {
            sessions.insert(new_id.clone(), Session::new());
        }

        new_id
    }

    fn cleanup(&self) {
        {
            let mut sessions = self.sessions.write().unwrap();
            sessions.retain(|_, session| !session.is_expired(self.max_age));
        }

        // Record that we just did a sweep so the time-based trigger doesn't
        // fire again immediately.
        if let Ok(mut last) = self.last_cleanup.lock() {
            *last = Instant::now();
        }
    }

    fn driver_name(&self) -> &'static str {
        "in_memory"
    }
}

// Thread-local current session ID (set per-request from cookie).
thread_local! {
    static CURRENT_SESSION_ID: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Set the current session ID for this request.
pub fn set_current_session_id(session_id: Option<String>) {
    CURRENT_SESSION_ID.with(|id| {
        *id.borrow_mut() = session_id;
    });
}

/// Get the current session ID for this request.
pub fn get_current_session_id() -> Option<String> {
    CURRENT_SESSION_ID.with(|id| id.borrow().clone())
}

// Response cookies accumulated during request handling via set_cookie().
thread_local! {
    static RESPONSE_COOKIES: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Clear any response cookies accumulated from a previous request.
pub fn clear_response_cookies() {
    RESPONSE_COOKIES.with(|c| c.borrow_mut().clear());
}

/// Store a response cookie to be emitted as a Set-Cookie header.
pub fn set_response_cookie(name: &str, value: &str) {
    // Trip the response cache dirty flag so a stale cached body
    // can't be returned without the new Set-Cookie header.
    crate::template::response_cache::mark_response_dirty();
    RESPONSE_COOKIES.with(|c| c.borrow_mut().push((name.to_string(), value.to_string())));
}

/// Drain all accumulated response cookies (called in finalize_response).
pub fn take_response_cookies() -> Vec<(String, String)> {
    RESPONSE_COOKIES.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Parse a Cookie header string into a HashMap of name -> value.
pub fn parse_cookies_from_header(cookie_header: Option<&str>) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    let header = match cookie_header {
        Some(h) => h,
        None => return cookies,
    };
    for cookie in header.split(';') {
        let cookie = cookie.trim();
        if cookie.is_empty() {
            continue;
        }
        if let Some(pos) = cookie.find('=') {
            let name = cookie[..pos].trim().to_string();
            let value = cookie[pos + 1..].trim().to_string();
            if !name.is_empty() {
                cookies.insert(name, value);
            }
        }
    }
    cookies
}

/// Parse a Cookie header directly into Soli `HashPairs`, ready to drop into
/// the request hash. Same parsing rules as `parse_cookies_from_header`
/// (split on `;`, trim name and value, last duplicate wins) without the
/// intermediate `HashMap` — the request path parses the header exactly once.
pub fn parse_cookie_pairs(cookie_header: Option<&str>) -> HashPairs {
    use crate::interpreter::value::HashKey;
    let mut pairs = HashPairs::default();
    let header = match cookie_header {
        Some(h) => h,
        None => return pairs,
    };
    for cookie in header.split(';') {
        let cookie = cookie.trim();
        if cookie.is_empty() {
            continue;
        }
        if let Some(pos) = cookie.find('=') {
            let name = cookie[..pos].trim();
            let value = cookie[pos + 1..].trim();
            if !name.is_empty() {
                pairs.insert(HashKey::String(name.into()), Value::String(value.into()));
            }
        }
    }
    pairs
}

/// Derive the session ID from an already-parsed cookie map (the single-parse
/// request path). SEC-077 precedence preserved: `__Host-session_id` wins over
/// plain `session_id` when both are present.
pub fn session_id_from_cookie_pairs(cookies: &HashPairs) -> Option<String> {
    use crate::interpreter::value::StrKey;
    let read = |name: &str| match cookies.get(&StrKey(name)) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    read("__Host-session_id")
        .or_else(|| read("session_id"))
        .map(|s| s.to_string())
}

/// Get or create a session for the given cookie value.
/// Returns the session ID to use (may be new if expired or invalid).
///
/// SEC-053: validates the cookie value against the UUID-v4 hyphenated
/// shape we generate before passing it to any backend. Without this
/// check, an attacker-controlled cookie could push very long strings
/// (memory amplification on the in-memory / SoliKV stores) or values
/// containing control bytes (log injection in `eprintln!` paths) into
/// the backend layer. Only the disk store had its own input
/// validation; centralising it here closes the gap for every
/// implementor.
pub fn ensure_session(cookie_session_id: Option<&str>) -> String {
    let store = get_current_store();
    match cookie_session_id {
        Some(id) if is_valid_session_id(id) => store.get_or_create(id),
        _ => store.create_session(),
    }
}

/// SEC-053: a session ID must be a UUID-v4 in 36-char hyphenated form
/// (the exact shape every store mints via `Uuid::new_v4().to_string()`).
/// Anything else — too long, wrong alphabet, embedded control bytes,
/// path-traversal — gets rejected here so the backends only ever see
/// well-formed IDs.
pub fn is_valid_session_id(id: &str) -> bool {
    id.len() == 36 && Uuid::parse_str(id).is_ok()
}

/// Extract session ID from Cookie header. SEC-077: when the deployment runs
/// with `SOLI_SESSION_HOST_PREFIX=1`, the server emits the cookie under the
/// `__Host-session_id` name; this read path now accepts both that prefixed
/// form and the plain `session_id` form so requests are still recognised.
/// `__Host-session_id` takes precedence when both are present (the prefixed
/// cookie carries the `__Host-` browser guarantees, so prefer it over a
/// plain replay smuggled in by an attacker on the same host).
pub fn extract_session_id_from_cookie(cookie_header: Option<&str>) -> Option<String> {
    let cookies = cookie_header?;
    let mut host_prefixed: Option<String> = None;
    let mut plain: Option<String> = None;
    for cookie in cookies.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix("__Host-session_id=") {
            host_prefixed = Some(value.to_string());
        } else if let Some(value) = cookie.strip_prefix("session_id=") {
            plain = Some(value.to_string());
        }
    }
    host_prefixed.or(plain)
}

/// Create Set-Cookie header value for session.
///
/// SEC-038: `Max-Age` reflects the configured `SOLI_SESSION_TTL` so the
/// browser drops the cookie on the same schedule the server expires the
/// stored session. The previous hardcoded 86400 silently kept cookies
/// alive for 24h regardless of the operator's TTL setting.
///
/// SEC-079: when the configured `SameSite` is `None`, the `Secure` flag is
/// forced on regardless of the caller-detected request scheme. Browsers
/// reject `SameSite=None` cookies that lack `Secure`, so the previous
/// behaviour silently emitted a cookie the browser would refuse — leaving
/// session-bearing cross-site flows broken under HTTP and tempting
/// operators into insecure workarounds. Self-correcting (always Secure on
/// SameSite=None) is preferable to hard-failing startup; the docs reflect
/// the implicit pairing.
pub fn create_session_cookie(session_id: &str, secure: bool) -> String {
    let cfg = SESSION_CONFIG.read().ok();
    let max_age = cfg.as_ref().map(|c| c.ttl).unwrap_or(24 * 60 * 60);
    let same_site = cfg.as_ref().map(|c| c.same_site).unwrap_or(SameSite::Lax);
    let host_prefix = cfg.as_ref().map(|c| c.host_prefix).unwrap_or(false);
    // SEC-079: SameSite=None requires Secure per browser policy.
    let secure = secure || same_site == SameSite::None;
    let secure_flag = if secure { "; Secure" } else { "" };
    let cookie_name = if host_prefix && secure {
        "__Host-session_id"
    } else {
        "session_id"
    };
    format!(
        "{}={}; Path=/; HttpOnly; SameSite={}; Max-Age={}{}",
        cookie_name, session_id, same_site, max_age, secure_flag
    )
}

/// Return a Set-Cookie header value iff the active session ID differs from
/// the one the client sent — i.e. we created a session lazily, the cookie
/// was expired/invalid and `ensure_session` minted a replacement, or the
/// controller called `session_regenerate`. Returns None when no cookie
/// refresh is needed.
pub fn session_cookie_if_changed(
    current: Option<&str>,
    cookie: Option<&str>,
    secure: bool,
) -> Option<String> {
    match current {
        Some(sid) if Some(sid) != cookie => Some(create_session_cookie(sid, secure)),
        _ => None,
    }
}

/// Convert a Soli Value to JSON for storage.
fn value_to_json(value: &Value) -> Result<JsonValue, String> {
    crate::interpreter::value::value_to_json(value)
}

/// Convert a JSON value back to a Soli Value.
fn json_to_value(json: &JsonValue) -> Value {
    crate::interpreter::value::json_to_value_ref(json).unwrap_or(Value::Null)
}

/// Register session builtins in the given environment.
pub fn register_session_builtins(env: &mut Environment) {
    // session_get(key) -> Value or null
    env.define(
        "session_get".to_string(),
        Value::NativeFunction(NativeFunction::new("session_get", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "session_get() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let session_id = get_current_session_id();
            match session_id {
                Some(id) => Ok(get_current_store()
                    .get(&id, &key)
                    .map(|json| json_to_value(&json))
                    .unwrap_or(Value::Null)),
                None => Ok(Value::Null),
            }
        })),
    );

    // session_set(key, value) -> null
    env.define(
        "session_set".to_string(),
        Value::NativeFunction(NativeFunction::new("session_set", Some(2), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "session_set() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let value = &args[1];
            let json_value = value_to_json(value)?;

            // Lazily create a session on first write so first-time visitors
            // (no Cookie header) still get a persisted session + Set-Cookie.
            let id = match get_current_session_id() {
                Some(id) => id,
                None => {
                    let id = get_current_store().create_session();
                    set_current_session_id(Some(id.clone()));
                    id
                }
            };
            get_current_store().set(&id, &key, json_value);

            Ok(Value::Null)
        })),
    );

    // session_delete(key) -> deleted value or null
    env.define(
        "session_delete".to_string(),
        Value::NativeFunction(NativeFunction::new("session_delete", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "session_delete() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            if let Some(id) = get_current_session_id() {
                return Ok(get_current_store()
                    .delete(&id, &key)
                    .map(|json| json_to_value(&json))
                    .unwrap_or(Value::Null));
            }

            Ok(Value::Null)
        })),
    );

    // session_destroy() -> null
    env.define(
        "session_destroy".to_string(),
        Value::NativeFunction(NativeFunction::new("session_destroy", Some(0), |_args| {
            if let Some(id) = get_current_session_id() {
                get_current_store().destroy(&id);
            }
            Ok(Value::Null)
        })),
    );

    // session_id() -> String
    env.define(
        "session_id".to_string(),
        Value::NativeFunction(NativeFunction::new("session_id", Some(0), |_args| {
            Ok(get_current_session_id()
                .map(|s| Value::String(s.into()))
                .unwrap_or(Value::Null))
        })),
    );

    // session_regenerate() -> String (new ID)
    env.define(
        "session_regenerate".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "session_regenerate",
            Some(0),
            |_args| {
                let new_id = match get_current_session_id() {
                    Some(old_id) => get_current_store().regenerate(&old_id),
                    None => get_current_store().create_session(),
                };
                set_current_session_id(Some(new_id.clone()));
                Ok(Value::String(new_id.into()))
            },
        )),
    );

    // session_has(key) -> Bool
    env.define(
        "session_has".to_string(),
        Value::NativeFunction(NativeFunction::new("session_has", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "session_has() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            if let Some(id) = get_current_session_id() {
                return Ok(Value::Bool(get_current_store().get(&id, &key).is_some()));
            }

            Ok(Value::Bool(false))
        })),
    );

    // session_driver() -> String
    env.define(
        "session_driver".to_string(),
        Value::NativeFunction(NativeFunction::new("session_driver", Some(0), |_args| {
            Ok(Value::String(
                get_current_store().driver_name().to_string().into(),
            ))
        })),
    );

    // session_configure(options) -> Bool
    env.define(
        "session_configure".to_string(),
        Value::NativeFunction(NativeFunction::new("session_configure", Some(1), |args| {
            let options = match &args[0] {
                Value::Hash(h) => h.borrow().clone(),
                other => {
                    return Err(format!(
                        "session_configure() expects hash options, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut config = get_session_config();

            for (k, v) in options.iter() {
                let key = match k {
                    crate::interpreter::value::HashKey::String(s) => s.clone(),
                    _ => continue,
                };

                match key.as_ref() {
                    "driver" => {
                        if let Value::String(s) = v {
                            config.driver = s.parse().map_err(|e: String| e)?;
                        }
                    }
                    "path" => {
                        if let Value::String(s) = v {
                            config.path = Some(s.clone().to_string());
                        }
                    }
                    "solidb_host" | "solidb_addr" => {
                        if let Value::String(s) = v {
                            config.solidb_host = Some(s.clone().to_string());
                        }
                    }
                    "solidb_database" | "database" => {
                        if let Value::String(s) = v {
                            config.solidb_database = Some(s.clone().to_string());
                        }
                    }
                    "solidb_collection" | "collection" => {
                        if let Value::String(s) = v {
                            config.solidb_collection = Some(s.clone().to_string());
                        }
                    }
                    "solidb_api_key" | "api_key" => {
                        if let Value::String(s) = v {
                            config.solidb_api_key = Some(s.clone().to_string());
                        }
                    }
                    "solidb_username" | "username" => {
                        if let Value::String(s) = v {
                            config.solidb_username = Some(s.clone().to_string());
                        }
                    }
                    "solidb_password" | "password" => {
                        if let Value::String(s) = v {
                            config.solidb_password = Some(s.clone().to_string());
                        }
                    }
                    "solikv_host" => {
                        if let Value::String(s) = v {
                            config.solikv_host = Some(s.clone().to_string());
                        }
                    }
                    "solikv_port" | "port" => {
                        if let Value::Int(i) = v {
                            config.solikv_port = Some(*i as u16);
                        }
                    }
                    "solikv_token" | "token" => {
                        if let Value::String(s) = v {
                            config.solikv_token = Some(s.clone().to_string());
                        }
                    }
                    "ttl" => {
                        if let Value::Int(i) = v {
                            config.ttl = *i as u64;
                        }
                    }
                    _ => {}
                }
            }

            configure_session(config)?;
            Ok(Value::Bool(true))
        })),
    );

    // session_config() -> Hash
    env.define(
        "session_config".to_string(),
        Value::NativeFunction(NativeFunction::new("session_config", Some(0), |_args| {
            let config = get_session_config();
            let mut hash: HashPairs = HashPairs::default();

            hash.insert(
                crate::interpreter::value::HashKey::String("driver".into()),
                Value::String(config.driver.to_string().into()),
            );

            if let Some(ref path) = config.path {
                hash.insert(
                    crate::interpreter::value::HashKey::String("path".into()),
                    Value::String(path.clone().into()),
                );
            }
            if let Some(ref host) = config.solidb_host {
                hash.insert(
                    crate::interpreter::value::HashKey::String("solidb_host".into()),
                    Value::String(host.clone().into()),
                );
            }
            if let Some(ref db) = config.solidb_database {
                hash.insert(
                    crate::interpreter::value::HashKey::String("solidb_database".into()),
                    Value::String(db.clone().into()),
                );
            }
            if let Some(ref col) = config.solidb_collection {
                hash.insert(
                    crate::interpreter::value::HashKey::String("solidb_collection".into()),
                    Value::String(col.clone().into()),
                );
            }
            if let Some(ref host) = config.solikv_host {
                hash.insert(
                    crate::interpreter::value::HashKey::String("solikv_host".into()),
                    Value::String(host.clone().into()),
                );
            }
            if let Some(port) = config.solikv_port {
                hash.insert(
                    crate::interpreter::value::HashKey::String("solikv_port".into()),
                    Value::Int(port as i64),
                );
            }
            if let Some(ref token) = config.solikv_token {
                hash.insert(
                    crate::interpreter::value::HashKey::String("solikv_token".into()),
                    Value::String(token.clone().into()),
                );
            }
            hash.insert(
                crate::interpreter::value::HashKey::String("ttl".into()),
                Value::Int(config.ttl as i64),
            );

            Ok(Value::Hash(Rc::new(RefCell::new(hash))))
        })),
    );
}

/// Register cookie-related builtins (set_cookie) that are available in all
/// contexts (controllers, views, middleware).
pub fn register_cookie_builtins(env: &mut Environment) {
    // set_cookie(name, value) -> Null
    env.define(
        "set_cookie".to_string(),
        Value::NativeFunction(NativeFunction::new("set_cookie", Some(2), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_cookie() expects string name, got {}",
                        other.type_name()
                    ))
                }
            };
            let value = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_cookie() expects string value, got {}",
                        other.type_name()
                    ))
                }
            };
            set_response_cookie(&name, &value);
            Ok(Value::Null)
        })),
    );
}

#[cfg(test)]
mod tests {
    //! End-to-end integration tests for the session layer.
    //!
    //! These exercise the actual native-function closures that the interpreter
    //! invokes, driving the same get_current_store() + CURRENT_SESSION_ID thread-local
    //! that a real HTTP request uses. They simulate the request lifecycle
    //! (resolve cookie → run handler → diff IDs for Set-Cookie) without
    //! standing up a full interpreter + router.
    //!
    //! Regression coverage:
    //! - session_set on a request with no Cookie header must persist and
    //!   cause a Set-Cookie to be emitted (previously a silent no-op).
    //! - session_regenerate must emit a Set-Cookie carrying the new ID,
    //!   otherwise the client's cookie points at a deleted session after
    //!   login (previously the new ID was dropped on the floor).
    use super::*;
    use serde_json::json;

    /// Look up a registered native function by name and invoke it.
    fn call_fn(env: &Environment, name: &str, args: Vec<Value>) -> Result<Value, String> {
        match env.get(name) {
            Some(Value::NativeFunction(f)) => (f.func)(args),
            other => panic!("expected NativeFunction for {name}, got {other:?}"),
        }
    }

    fn fresh_env() -> Environment {
        // Reset the thread-local so tests sharing a thread don't leak state.
        set_current_session_id(None);
        let mut env = Environment::new();
        register_session_builtins(&mut env);
        env
    }

    /// SEC-025: loopback host detection accepts the names that
    /// `is_blocked_host` does in the SSRF guard.
    #[test]
    fn loopback_session_host_accepts_known_loopbacks() {
        for host in [
            "127.0.0.1",
            "localhost",
            "localhost:8080",
            "http://127.0.0.1:6745",
            "https://localhost",
            "user:pass@127.0.0.1:8080",
            "[::1]",
            "[::1]:8080",
            "::1",
            "127.0.0.1/",
            "localhost.local",
        ] {
            assert!(
                is_loopback_session_host(host),
                "expected `{}` to be loopback",
                host
            );
        }
    }

    /// SEC-025: non-loopback hosts must be flagged as remote so the
    /// gate refuses plaintext / no-auth traffic.
    #[test]
    fn loopback_session_host_rejects_remote() {
        for host in [
            "db.internal:8080",
            "10.0.0.1:8080",
            "http://example.com",
            "https://soli.example.com:6745",
            "192.168.1.1",
        ] {
            assert!(
                !is_loopback_session_host(host),
                "expected `{}` to NOT be loopback",
                host
            );
        }
    }

    /// First-time visitor: no Cookie header, handler writes to the session.
    /// session_set must create a session on demand, persist the value, and
    /// leave the thread-local pointing at the new ID so finalize_response
    /// can emit Set-Cookie.
    #[test]
    fn session_set_lazily_creates_session_when_no_cookie() {
        let env = fresh_env();
        let cookie_session_id: Option<String> = None;
        set_current_session_id(cookie_session_id.clone());

        call_fn(
            &env,
            "session_set",
            vec![Value::String("user_id".into()), Value::Int(42)],
        )
        .unwrap();

        let current = get_current_session_id().expect("session should be created lazily");
        assert_eq!(
            get_current_store().get(&current, "user_id"),
            Some(json!(42)),
            "value must persist under the newly created session"
        );

        // Simulate finalize_response: Set-Cookie must carry the new ID.
        let cookie = session_cookie_if_changed(Some(&current), cookie_session_id.as_deref(), false)
            .expect("expected Set-Cookie for lazily created session");
        assert!(cookie.contains(&format!("session_id={current}")));
    }

    /// session_regenerate on login rotates the ID, migrates data, and
    /// destroys the old ID. The response must carry a Set-Cookie for
    /// the new ID, or the browser keeps using the deleted cookie.
    #[test]
    fn session_regenerate_migrates_data_and_emits_new_cookie() {
        let env = fresh_env();

        // Prime a session as if an earlier request had created one.
        let old_id = get_current_store().create_session();
        get_current_store().set(&old_id, "flash", json!("hello"));
        set_current_session_id(Some(old_id.clone()));
        let cookie_session_id = Some(old_id.clone());

        // Login-style flow: regenerate, then write user_id.
        let new_id = match call_fn(&env, "session_regenerate", vec![]).unwrap() {
            Value::String(s) => s,
            other => panic!("expected String session id, got {other:?}"),
        };
        assert_ne!(&*new_id, old_id.as_str(), "regenerate must mint a new ID");

        call_fn(
            &env,
            "session_set",
            vec![Value::String("user_id".into()), Value::Int(42)],
        )
        .unwrap();

        assert!(
            get_current_store().get(&old_id, "flash").is_none(),
            "old session ID must be destroyed after regenerate"
        );
        assert_eq!(
            get_current_store().get(&new_id, "flash"),
            Some(json!("hello")),
            "data must move from old ID to new ID"
        );
        assert_eq!(get_current_store().get(&new_id, "user_id"), Some(json!(42)));
        assert_eq!(get_current_session_id().as_deref(), Some(new_id.as_ref()));

        let cookie = session_cookie_if_changed(
            get_current_session_id().as_deref(),
            cookie_session_id.as_deref(),
            true,
        )
        .expect("expected Set-Cookie carrying the rotated ID");
        assert!(cookie.contains(&format!("session_id={new_id}")));
        assert!(cookie.contains("Secure"), "secure flag must propagate");
    }

    /// session_regenerate with no prior session (e.g. first-request login)
    /// should still produce a usable, cookie-emitted session.
    #[test]
    fn session_regenerate_creates_session_when_none_active() {
        let env = fresh_env();
        set_current_session_id(None);

        let new_id = match call_fn(&env, "session_regenerate", vec![]).unwrap() {
            Value::String(s) => s,
            other => panic!("expected String, got {other:?}"),
        };
        assert_eq!(get_current_session_id().as_deref(), Some(new_id.as_ref()));
        assert!(session_cookie_if_changed(Some(&new_id), None, false).is_some());
    }

    /// Across two simulated requests, a session written on request #1 must
    /// be readable on request #2 when the client echoes the cookie.
    #[test]
    fn session_persists_across_requests_via_cookie() {
        let env = fresh_env();

        // --- Request 1: no cookie, handler writes user_id.
        set_current_session_id(None);
        call_fn(
            &env,
            "session_set",
            vec![Value::String("user_id".into()), Value::Int(42)],
        )
        .unwrap();
        let issued_id = get_current_session_id().expect("request 1 must create a session");
        set_current_session_id(None); // end of request 1

        // --- Request 2: client sends the cookie back.
        let cookie_session_id = Some(issued_id.clone());
        let resolved = ensure_session(cookie_session_id.as_deref());
        assert_eq!(
            resolved, issued_id,
            "ensure_session must reuse an existing cookie ID"
        );
        set_current_session_id(Some(resolved.clone()));

        let got = call_fn(&env, "session_get", vec![Value::String("user_id".into())]).unwrap();
        match got {
            Value::Int(n) => assert_eq!(n, 42),
            other => panic!("expected stored user_id, got {other:?}"),
        }

        // No Set-Cookie on request 2 since the ID didn't change.
        assert!(session_cookie_if_changed(
            get_current_session_id().as_deref(),
            cookie_session_id.as_deref(),
            false,
        )
        .is_none());
    }

    /// ensure_session mints a replacement when the cookie's ID is unknown
    /// (e.g. after a server restart). finalize_response must notice and
    /// refresh the client's cookie.
    #[test]
    fn unknown_cookie_id_triggers_replacement_and_set_cookie() {
        let _env = fresh_env();
        let stale = "00000000-0000-0000-0000-000000000000".to_string();
        let cookie_session_id = Some(stale.clone());

        let resolved = ensure_session(cookie_session_id.as_deref());
        assert_ne!(resolved, stale, "unknown cookie ID must be replaced");
        set_current_session_id(Some(resolved.clone()));

        let cookie =
            session_cookie_if_changed(Some(&resolved), cookie_session_id.as_deref(), false)
                .expect("Set-Cookie must be emitted when the cookie ID was replaced");
        assert!(cookie.contains(&format!("session_id={resolved}")));
    }

    /// Serialize tests that mutate the process-global SESSION_CONFIG /
    /// CURRENT_STORE. Cargo runs tests in a module in parallel by default,
    /// and these globals are read by every other test in this module.
    static GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// SEC-038: SOLI_SESSION_TTL must drive Set-Cookie's Max-Age and the
    /// in-memory store's expiry, not silently fall back to 24h.
    #[test]
    fn session_ttl_threads_through_to_cookie_and_store() {
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = SESSION_CONFIG.read().unwrap().clone();
        {
            let mut cfg = SESSION_CONFIG.write().unwrap();
            cfg.ttl = 300;
        }
        let cookie = create_session_cookie("abc", false);
        assert!(
            cookie.contains("Max-Age=300"),
            "expected Max-Age=300 from configured TTL, got: {}",
            cookie
        );

        // The in-memory store created via SessionConfig::create_store
        // must honor the TTL — verify by constructing a short-TTL config
        // and checking is_expired sees a near-zero max_age.
        let mut short = prev.clone();
        short.ttl = 0;
        let store = short.create_store().expect("in-memory store");
        // Drive a get_or_create + immediate cleanup; with ttl=0 every
        // session is immediately expired and pruned.
        let id = store.create_session();
        store.cleanup();
        assert!(
            store.get(&id, "anything").is_none(),
            "ttl=0 must expire newly created session on cleanup"
        );

        // Restore.
        let mut cfg = SESSION_CONFIG.write().unwrap();
        *cfg = prev;
    }

    /// SEC-079: SameSite=Lax / Strict cookies follow the caller's request-
    /// scheme detection (no Secure on plain HTTP); SameSite=None always
    /// carries Secure even when the caller passes `secure=false`, because
    /// browsers reject `SameSite=None` without `Secure`.
    #[test]
    fn samesite_lax_omits_secure_on_plain_http() {
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = SESSION_CONFIG.read().unwrap().clone();
        {
            let mut cfg = SESSION_CONFIG.write().unwrap();
            cfg.same_site = SameSite::Lax;
        }
        let cookie = create_session_cookie("abc", false);
        assert!(cookie.contains("SameSite=Lax"), "{}", cookie);
        assert!(!cookie.contains("Secure"), "{}", cookie);
        *SESSION_CONFIG.write().unwrap() = prev;
    }

    #[test]
    fn samesite_strict_omits_secure_on_plain_http() {
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = SESSION_CONFIG.read().unwrap().clone();
        {
            let mut cfg = SESSION_CONFIG.write().unwrap();
            cfg.same_site = SameSite::Strict;
        }
        let cookie = create_session_cookie("abc", false);
        assert!(cookie.contains("SameSite=Strict"), "{}", cookie);
        assert!(!cookie.contains("Secure"), "{}", cookie);
        *SESSION_CONFIG.write().unwrap() = prev;
    }

    #[test]
    fn samesite_none_forces_secure_even_on_plain_http() {
        // SEC-079: a `SameSite=None` cookie without `Secure` is rejected by
        // every modern browser; emit Secure regardless of the caller's
        // scheme detection so the cookie stays useful.
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = SESSION_CONFIG.read().unwrap().clone();
        {
            let mut cfg = SESSION_CONFIG.write().unwrap();
            cfg.same_site = SameSite::None;
        }
        let cookie = create_session_cookie("abc", false);
        assert!(cookie.contains("SameSite=None"), "{}", cookie);
        assert!(
            cookie.contains("; Secure"),
            "SameSite=None must always carry Secure: {}",
            cookie
        );
        *SESSION_CONFIG.write().unwrap() = prev;
    }

    #[test]
    fn samesite_none_secure_idempotent_when_caller_already_secure() {
        // The auto-Secure for SameSite=None must not double up the flag
        // when the caller already detected HTTPS.
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = SESSION_CONFIG.read().unwrap().clone();
        {
            let mut cfg = SESSION_CONFIG.write().unwrap();
            cfg.same_site = SameSite::None;
        }
        let cookie = create_session_cookie("abc", true);
        assert_eq!(
            cookie.matches("Secure").count(),
            1,
            "Secure must appear exactly once: {}",
            cookie
        );
        *SESSION_CONFIG.write().unwrap() = prev;
    }

    /// SEC-038a: configure_session must swap the live store at runtime.
    /// Previously the lazy_static SessionStoreManager cached one Arc clone
    /// at first access, so dispatch kept going to the original store
    /// regardless of subsequent configure_session() calls.
    #[test]
    fn configure_session_swaps_store_at_runtime() {
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        // Snapshot and rebuild the default store so the test starts clean.
        let prev_cfg = SESSION_CONFIG.read().unwrap().clone();
        configure_session(prev_cfg.clone()).expect("reset to default config");

        let store_before = get_current_store();
        let id = store_before.create_session();
        store_before.set(&id, "marker", json!("before-swap"));
        assert_eq!(store_before.get(&id, "marker"), Some(json!("before-swap")));

        // Swap to a fresh in-memory store via configure_session — same
        // driver, but a brand-new instance. After the swap, the old key
        // must be invisible because dispatch goes through the new store.
        let mut next_cfg = prev_cfg.clone();
        next_cfg.driver = SessionDriver::InMemory;
        configure_session(next_cfg).expect("swap to fresh in-memory store");

        let store_after = get_current_store();
        assert!(
            !Arc::ptr_eq(&store_before, &store_after),
            "configure_session must replace the underlying store"
        );
        assert!(
            store_after.get(&id, "marker").is_none(),
            "post-swap dispatch must hit the new store, not the cached old one"
        );

        // Restore.
        configure_session(prev_cfg).expect("restore previous config");
    }

    #[test]
    fn session_cookie_if_changed_respects_matches_and_absence() {
        assert!(session_cookie_if_changed(None, None, false).is_none());
        assert!(session_cookie_if_changed(None, Some("a"), false).is_none());
        assert!(session_cookie_if_changed(Some("a"), Some("a"), false).is_none());
        assert!(session_cookie_if_changed(Some("a"), Some("b"), false).is_some());
        assert!(session_cookie_if_changed(Some("a"), None, false).is_some());
    }

    #[test]
    fn is_valid_session_id_accepts_freshly_minted_uuid() {
        // SEC-053: every store mints IDs via Uuid::new_v4().to_string();
        // the validator must pass them.
        for _ in 0..8 {
            let id = Uuid::new_v4().to_string();
            assert!(
                is_valid_session_id(&id),
                "freshly minted UUID rejected: {id:?}"
            );
        }
    }

    #[test]
    fn is_valid_session_id_rejects_attacker_shapes() {
        // SEC-053: too long, wrong length, control bytes, traversal,
        // empty — all rejected.
        let too_long = "a".repeat(1024);
        for bad in [
            "",
            "abc",
            "../etc/passwd",
            "session\rinjected",
            "session\nlog",
            "00000000-0000-0000-0000-00000000000",   // 35 chars
            "00000000-0000-0000-0000-0000000000000", // 37 chars
            "ZZZZZZZZ-0000-0000-0000-000000000000",  // bad alphabet
            too_long.as_str(),
        ] {
            assert!(!is_valid_session_id(bad), "expected reject: {bad:?}");
        }
    }

    #[test]
    fn ensure_session_mints_new_when_cookie_id_is_invalid() {
        // SEC-053: an invalid cookie value must not flow into the
        // backend — ensure_session should treat it as no cookie and
        // mint a fresh ID.
        set_current_session_id(None);
        let resolved = ensure_session(Some("../etc/passwd\r\n"));
        assert!(
            is_valid_session_id(&resolved),
            "ensure_session returned non-UUID for malformed cookie: {resolved:?}"
        );
    }

    #[test]
    fn extract_session_id_from_cookie_parses_common_shapes() {
        assert_eq!(extract_session_id_from_cookie(None), None);
        assert_eq!(extract_session_id_from_cookie(Some("")), None);
        assert_eq!(
            extract_session_id_from_cookie(Some("session_id=abc")),
            Some("abc".to_string())
        );
        assert_eq!(
            extract_session_id_from_cookie(Some("foo=1; session_id=abc; bar=2")),
            Some("abc".to_string())
        );
        assert_eq!(extract_session_id_from_cookie(Some("foo=1; bar=2")), None);
    }

    #[test]
    fn extract_session_id_from_cookie_reads_host_prefixed_name() {
        // SEC-077: when SOLI_SESSION_HOST_PREFIX=1 the server emits the cookie
        // under `__Host-session_id`; the read path must accept it.
        assert_eq!(
            extract_session_id_from_cookie(Some("__Host-session_id=xyz")),
            Some("xyz".to_string())
        );
        assert_eq!(
            extract_session_id_from_cookie(Some("foo=1; __Host-session_id=xyz; bar=2")),
            Some("xyz".to_string())
        );
    }

    #[test]
    fn extract_session_id_from_cookie_prefers_host_prefixed_over_plain() {
        // SEC-077: if both names are present, the `__Host-` cookie wins
        // because it carries the browser-enforced Secure / Path=/ / no-Domain
        // guarantees that the plain name lacks.
        assert_eq!(
            extract_session_id_from_cookie(Some("session_id=plain; __Host-session_id=prefixed")),
            Some("prefixed".to_string())
        );
        assert_eq!(
            extract_session_id_from_cookie(Some("__Host-session_id=prefixed; session_id=plain")),
            Some("prefixed".to_string())
        );
    }

    #[test]
    fn session_driver_returns_current_driver() {
        let env = fresh_env();
        let result = call_fn(&env, "session_driver", vec![]).unwrap();
        match result {
            Value::String(s) => assert_eq!(s.as_str(), "in_memory"),
            other => panic!("expected String driver name, got {:?}", other),
        }
    }

    #[test]
    fn session_config_returns_hash() {
        let env = fresh_env();
        let result = call_fn(&env, "session_config", vec![]).unwrap();
        match result {
            Value::Hash(_) => {}
            other => panic!("expected Hash config, got {:?}", other),
        }
    }

    #[test]
    fn session_has_returns_true_for_existing_key() {
        let env = fresh_env();
        set_current_session_id(None);

        call_fn(
            &env,
            "session_set",
            vec![Value::String("key".into()), Value::Int(1)],
        )
        .unwrap();

        let result = call_fn(&env, "session_has", vec![Value::String("key".into())]).unwrap();
        match result {
            Value::Bool(b) => assert!(b),
            other => panic!("expected Bool, got {:?}", other),
        }

        let result = call_fn(
            &env,
            "session_has",
            vec![Value::String("nonexistent".into())],
        )
        .unwrap();
        match result {
            Value::Bool(b) => assert!(!b),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn session_delete_returns_previous_value() {
        let env = fresh_env();
        set_current_session_id(None);

        call_fn(
            &env,
            "session_set",
            vec![Value::String("to_delete".into()), Value::Int(42)],
        )
        .unwrap();

        let result = call_fn(
            &env,
            "session_delete",
            vec![Value::String("to_delete".into())],
        )
        .unwrap();

        match result {
            Value::Int(n) => assert_eq!(n, 42),
            other => panic!("expected deleted Int value, got {:?}", other),
        }

        let result = call_fn(
            &env,
            "session_delete",
            vec![Value::String("to_delete".into())],
        )
        .unwrap();
        match result {
            Value::Null => {}
            other => panic!("expected Null for deleted key, got {:?}", other),
        }
    }
}
