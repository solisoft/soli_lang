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
    pub host: String,
}

impl DbConfig {
    fn from_env() -> Self {
        let host =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        let host = host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();
        Self { host }
    }
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
            match ureq::post(&login_url)
                .set("Content-Type", "application/json")
                .send_string(&payload.to_string())
            {
                Ok(resp) => match resp.into_string() {
                    Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                        Ok(json) => json
                            .get("token")
                            .and_then(|t| t.as_str())
                            .map(|t| t.to_string()),
                        Err(_) => None,
                    },
                    Err(_) => None,
                },
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
        let host =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        let host = host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();
        let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());
        let cursor_url = format!("http://{}/_api/database/{}/cursor", host, database);
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
        return format!("http://{}/_api/database/{}/cursor", DB_CONFIG.host, name);
    }
    get_db_config().cursor_url.clone()
}

pub fn get_api_key() -> Option<&'static str> {
    get_db_config().api_key.as_deref()
}

pub fn get_basic_auth() -> Option<&'static str> {
    get_db_config().basic_auth.as_deref()
}
