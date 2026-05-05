use std::cell::RefCell;
use std::sync::OnceLock;

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

/// Cached JWT token obtained by logging in with Basic auth credentials.
static CACHED_JWT: OnceLock<Option<String>> = OnceLock::new();

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

pub fn get_jwt_token() -> Option<&'static str> {
    CACHED_JWT
        .get_or_init(|| {
            let (username, password) = match (
                std::env::var("SOLIDB_USERNAME").ok(),
                std::env::var("SOLIDB_PASSWORD").ok(),
            ) {
                (Some(u), Some(p)) => (u, p),
                _ => return None,
            };
            let host = std::env::var("SOLIDB_HOST")
                .unwrap_or_else(|_| "http://localhost:6745".to_string());
            let login_url = format!("{}/auth/login", host);
            let payload = serde_json::json!({
                "username": username,
                "password": password,
            });
            // SEC-007a: route through the redirect-disabled shared agent
            // for consistency with the rest of the HTTP layer. SOLIDB_HOST
            // is operator-configured (not user input) so this is hardening,
            // not closing an exploit.
            match crate::interpreter::builtins::http_class::ureq_agent()
                .post(&login_url)
                .set("Content-Type", "application/json")
                .send_string(&payload.to_string())
            {
                Ok(resp) => {
                    match crate::interpreter::builtins::http_class::read_capped_text_sync(resp) {
                        Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                            Ok(json) => json
                                .get("token")
                                .and_then(|t| t.as_str())
                                .map(|t| t.to_string()),
                            Err(_) => None,
                        },
                        Err(_) => None,
                    }
                }
                Err(e) => {
                    eprintln!("Warning: JWT login failed ({}), falling back", e);
                    None
                }
            }
        })
        .as_deref()
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
}
