use super::resp::{RespPool, RespValue};
use crate::serve::tenant::TenantValue;

const DEFAULT_TTL_SECONDS: u64 = 3600;
const DEFAULT_RESP_PORT: u16 = 6380;

pub(crate) struct SolikvConfig {
    pub prefix: String,
    pub default_ttl: u64,
    resp_host: String,
    resp_port: u16,
    auth_token: Option<String>,
}

/// SoliKV / Redis connection settings for the application on this thread.
///
/// Per application, not per process: `SOLIKV_RESP_HOST` and `SOLIKV_TOKEN` name
/// one app's cache, and `Cache.configure` / `KV.configure` repoint it at
/// runtime. Shared, one application's `Cache.clear()` would reach into the
/// other's store — and `solikv_configure` in one would silently redirect the
/// other's traffic to a host it never named.
static SOLIKV_CONFIG: TenantValue<SolikvConfig> = TenantValue::new(solikv_config_from_env);

/// Connections to that application's SoliKV, and so per application too: a
/// shared pool would hold sockets to whichever host was configured first.
static RESP_POOL: TenantValue<RespPool> = TenantValue::new(resp_pool_from_config);

fn solikv_config_from_env() -> SolikvConfig {
    let resp_host = std::env::var("SOLIKV_RESP_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".to_string());
    let resp_port = std::env::var("SOLIKV_RESP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_RESP_PORT);
    let auth_token = std::env::var("SOLIKV_TOKEN").ok().filter(|t| !t.is_empty());

    SolikvConfig {
        prefix: "soli:cache:".to_string(),
        default_ttl: DEFAULT_TTL_SECONDS,
        resp_host,
        resp_port,
        auth_token,
    }
}

fn resp_pool_from_config() -> RespPool {
    // Reads a different `TenantValue`, so no lock is held twice.
    let (host, port, token) =
        with_solikv_config(|cfg| (cfg.resp_host.clone(), cfg.resp_port, cfg.auth_token.clone()));
    RespPool::new(host, port, token)
}

/// Read this application's SoliKV settings.
///
/// Replaces the `&'static RwLock<SolikvConfig>` this used to hand out: a
/// per-application value lives inside a map inside a lock, which stable Rust
/// cannot lend a guard into. Keep the closure short — it runs under a read
/// lock.
pub(crate) fn with_solikv_config<R>(f: impl FnOnce(&SolikvConfig) -> R) -> R {
    SOLIKV_CONFIG.read(f)
}

/// Execute a RESP command and return the raw RespValue.
pub(crate) fn resp_cmd(args: &[&str]) -> Result<RespValue, String> {
    // Fast path when neither the dev bar nor the prod KV log is collecting:
    // a single relaxed atomic load, then straight through to the pool.
    if !super::kv_log::is_enabled() {
        return RESP_POOL.read(|pool| pool.execute(args));
    }

    let start = std::time::Instant::now();
    let result = RESP_POOL.read(|pool| pool.execute(args));
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Record the verb + key only — never the value (args[2..]), which may
    // hold secrets or large cached blobs.
    let command = args.first().map(|c| c.to_uppercase()).unwrap_or_default();
    let key = args.get(1).map(|k| k.to_string()).unwrap_or_default();
    let error = result.as_ref().err().cloned();
    super::kv_log::record(command, key, duration_ms, error);

    result
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
    let port = SOLIKV_CONFIG.write(|cfg| {
        cfg.resp_host = host.to_string();
        cfg.auth_token = token.clone();
        cfg.resp_port
    });

    // Replace the pool with a new one pointing to the new host. Read outside
    // the write below: they are different locks, but nesting them would fix an
    // order this file would then have to keep.
    RESP_POOL.write(|pool| *pool = RespPool::new(host.to_string(), port, token));
}
