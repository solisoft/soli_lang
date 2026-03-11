use super::resp::{RespPool, RespValue};
use std::sync::{OnceLock, RwLock};

const DEFAULT_TTL_SECONDS: u64 = 3600;
const DEFAULT_RESP_PORT: u16 = 6380;

pub(crate) struct SolikvConfig {
    pub prefix: String,
    pub default_ttl: u64,
    resp_host: String,
    resp_port: u16,
    auth_token: Option<String>,
}

static SOLIKV_CONFIG: OnceLock<RwLock<SolikvConfig>> = OnceLock::new();
static RESP_POOL: OnceLock<RwLock<RespPool>> = OnceLock::new();

pub(crate) fn get_solikv_config() -> &'static RwLock<SolikvConfig> {
    SOLIKV_CONFIG.get_or_init(|| {
        let resp_host = std::env::var("SOLIKV_RESP_HOST")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "localhost".to_string());
        let resp_port = std::env::var("SOLIKV_RESP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_RESP_PORT);
        let auth_token = std::env::var("SOLIKV_TOKEN").ok().filter(|t| !t.is_empty());

        RwLock::new(SolikvConfig {
            prefix: "soli:cache:".to_string(),
            default_ttl: DEFAULT_TTL_SECONDS,
            resp_host,
            resp_port,
            auth_token,
        })
    })
}

fn get_resp_pool() -> &'static RwLock<RespPool> {
    RESP_POOL.get_or_init(|| {
        let cfg = get_solikv_config().read().unwrap();
        RwLock::new(RespPool::new(
            cfg.resp_host.clone(),
            cfg.resp_port,
            cfg.auth_token.clone(),
        ))
    })
}

/// Execute a RESP command and return the raw RespValue.
pub(crate) fn resp_cmd(args: &[&str]) -> Result<RespValue, String> {
    let pool = get_resp_pool().read().map_err(|e| e.to_string())?;
    pool.execute(args)
}

/// Execute a RESP command and convert the result to serde_json::Value.
/// Used by cache.rs and kv.rs for backward compatibility.
pub(crate) fn solikv_cmd(args: &[&str]) -> Result<serde_json::Value, String> {
    let val = resp_cmd(args)?;
    Ok(val.to_json())
}

/// SET key value [EX ttl]
pub(crate) fn solikv_set(key: &str, value: &str, ttl: Option<u64>) -> Result<(), String> {
    let ttl_str;
    let args: Vec<&str> = if let Some(t) = ttl {
        ttl_str = t.to_string();
        vec!["SET", key, value, "EX", &ttl_str]
    } else {
        vec!["SET", key, value]
    };
    resp_cmd(&args)?;
    Ok(())
}

/// GET key → Option<String>
pub(crate) fn solikv_get(key: &str) -> Result<Option<String>, String> {
    let val = resp_cmd(&["GET", key])?;
    match val {
        RespValue::BulkString(s) => Ok(Some(s)),
        RespValue::Null => Ok(None),
        _ => Ok(val.as_str().map(|s| s.to_string())),
    }
}

/// DEL key → number of keys deleted
pub(crate) fn solikv_del(key: &str) -> Result<i64, String> {
    let val = resp_cmd(&["DEL", key])?;
    Ok(val.as_i64().unwrap_or(0))
}

/// Reconfigure the connection. Called by Cache.configure() / KV.configure().
pub(crate) fn solikv_configure(host: &str, token: Option<String>) {
    // Update config
    if let Ok(mut cfg) = get_solikv_config().write() {
        cfg.resp_host = host.to_string();
        cfg.auth_token = token.clone();
    }

    // Replace the pool with a new one pointing to the new host
    if let Ok(mut pool) = get_resp_pool().write() {
        let port = get_solikv_config()
            .read()
            .map(|c| c.resp_port)
            .unwrap_or(DEFAULT_RESP_PORT);
        *pool = RespPool::new(host.to_string(), port, token);
    }
}
