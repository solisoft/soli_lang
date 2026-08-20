//! Named database connections (`config/database.toml` + env fallback).
//!
//! See multi-database plan: default connection + per-model `connection "name"`.

use super::adapter::{parse_adapter, Adapter, AdapterConfig};
use super::error::DbError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// One named connection after parsing / env expansion.
#[derive(Clone, Debug)]
pub struct ConnectionSpec {
    pub name: String,
    pub adapter: Adapter,
    /// SQL: DATABASE_URL-style connection string.
    pub url: Option<String>,
    /// SoliDB host (e.g. http://localhost:6745).
    pub solidb_host: Option<String>,
    /// SoliDB database name.
    pub solidb_database: Option<String>,
    pub solidb_username: Option<String>,
    pub solidb_password: Option<String>,
    pub solidb_api_key: Option<String>,
    pub pool_size: Option<usize>,
}

impl ConnectionSpec {
    pub fn is_sql(&self) -> bool {
        self.adapter.is_sql()
    }

    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.adapter.as_str())
    }
}

/// Process-wide registry of named connections.
#[derive(Clone, Debug)]
pub struct ConnectionRegistry {
    pub default: String,
    pub connections: HashMap<String, ConnectionSpec>,
    /// True when loaded from `config/database.toml` (vs env-only primary).
    pub from_file: bool,
}

impl ConnectionRegistry {
    pub fn get(&self, name: &str) -> Option<&ConnectionSpec> {
        self.connections.get(name)
    }

    pub fn default_spec(&self) -> &ConnectionSpec {
        self.connections
            .get(&self.default)
            .expect("default connection must exist")
    }

    pub fn resolve(&self, name: Option<&str>) -> Result<&ConnectionSpec, String> {
        let key = name.unwrap_or(self.default.as_str());
        self.connections.get(key).ok_or_else(|| {
            let known: Vec<_> = self.connections.keys().cloned().collect();
            format!(
                "Unknown database connection {key:?}. Known: {}. \
                 See config/database.toml.",
                known.join(", ")
            )
        })
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<_> = self.connections.keys().cloned().collect();
        v.sort();
        v
    }
}

static REGISTRY: OnceLock<ConnectionRegistry> = OnceLock::new();
static REGISTRY_OVERRIDE: Mutex<Option<ConnectionRegistry>> = Mutex::new(None);

/// Install a registry for tests (or reload).
pub fn set_registry_for_tests(reg: ConnectionRegistry) {
    *REGISTRY_OVERRIDE.lock().unwrap() = Some(reg);
}

/// Serializes tests that install a registry override. The override is
/// process-global, so module-local mutexes cannot exclude each other — a
/// postgres integration test would see another module's solidb "primary"
/// mid-flight. Every `set_registry_for_tests` caller must hold THIS lock
/// (poison-tolerantly) for the whole override window.
pub fn registry_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn clear_registry_override() {
    *REGISTRY_OVERRIDE.lock().unwrap() = None;
}

/// Process registry: test override, else OnceLock from env/file (first call).
pub fn registry() -> ConnectionRegistry {
    if let Some(r) = REGISTRY_OVERRIDE.lock().unwrap().clone() {
        return r;
    }
    REGISTRY
        .get_or_init(|| {
            load_registry(None).unwrap_or_else(|e| {
                eprintln!("[WARN] database config: {}", e.message());
                env_only_primary().expect("solidb default always works")
            })
        })
        .clone()
}

/// Force-load registry from an app path (e.g. serve boot). Safe to call once.
pub fn init_from_app_path(app: &Path) -> Result<ConnectionRegistry, DbError> {
    let reg = load_registry(Some(app))?;
    // Prefer explicit init over empty OnceLock; store override for this process
    // when OnceLock already set from tests/early access.
    if REGISTRY.get().is_none() {
        let _ = REGISTRY.set(reg.clone());
    } else {
        *REGISTRY_OVERRIDE.lock().unwrap() = Some(reg.clone());
    }
    Ok(reg)
}

/// Load from `app/config/database.toml` if present, else env-only `primary`.
pub fn load_registry(app: Option<&Path>) -> Result<ConnectionRegistry, DbError> {
    let toml_path = app.map(|a| a.join("config/database.toml"));
    if let Some(path) = toml_path {
        if path.is_file() {
            return load_from_toml(&path);
        }
    }
    // Also try CWD when no app path.
    if app.is_none() {
        let cwd = PathBuf::from("config/database.toml");
        if cwd.is_file() {
            return load_from_toml(&cwd);
        }
    }
    env_only_primary()
}

fn env_only_primary() -> Result<ConnectionRegistry, DbError> {
    let cfg = AdapterConfig::from_env()?;
    let mut connections = HashMap::new();
    let mut spec = ConnectionSpec {
        name: "primary".into(),
        adapter: cfg.adapter,
        url: cfg.database_url.clone(),
        solidb_host: std::env::var("SOLIDB_HOST").ok(),
        solidb_database: std::env::var("SOLIDB_DATABASE").ok(),
        solidb_username: std::env::var("SOLIDB_USERNAME").ok(),
        solidb_password: std::env::var("SOLIDB_PASSWORD").ok(),
        solidb_api_key: std::env::var("SOLIDB_API_KEY").ok(),
        pool_size: cfg.pool_size,
    };
    if spec.adapter.is_sql() && spec.url.is_none() {
        return Err(DbError::MissingDatabaseUrl {
            adapter: spec.adapter,
        });
    }
    // Normalize solidb defaults for label/boot.
    if !spec.adapter.is_sql() {
        if spec.solidb_host.is_none() {
            spec.solidb_host = Some("http://localhost:6745".into());
        }
        if spec.solidb_database.is_none() {
            spec.solidb_database = Some("default".into());
        }
    }
    connections.insert("primary".into(), spec);
    Ok(ConnectionRegistry {
        default: "primary".into(),
        connections,
        from_file: false,
    })
}

fn load_from_toml(path: &Path) -> Result<ConnectionRegistry, DbError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| DbError::Backend(format!("read {}: {e}", path.display())))?;
    let expanded = expand_env(&raw);
    let value: toml::Value = expanded
        .parse()
        .map_err(|e| DbError::Backend(format!("parse {}: {e}", path.display())))?;

    let default = value
        .get("default")
        .and_then(|v| v.as_str())
        .unwrap_or("primary")
        .to_string();

    let conns_tbl = value
        .get("connections")
        .and_then(|v| v.as_table())
        .ok_or_else(|| {
            DbError::Backend(format!(
                "{}: missing [connections.*] tables",
                path.display()
            ))
        })?;

    let mut connections = HashMap::new();
    for (name, node) in conns_tbl {
        let table = node.as_table().ok_or_else(|| {
            DbError::Backend(format!(
                "{}: connections.{} must be a table",
                path.display(),
                name
            ))
        })?;
        let adapter_raw = table
            .get("adapter")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                DbError::Backend(format!(
                    "{}: connections.{} missing adapter",
                    path.display(),
                    name
                ))
            })?;
        let adapter = parse_adapter(Some(adapter_raw))?;
        let url = table
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let pool_size = table
            .get("pool")
            .and_then(|v| v.as_integer())
            .map(|n| n as usize)
            .filter(|&n| n > 0);
        let solidb_host = table
            .get("host")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let solidb_database = table
            .get("database")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let solidb_username = table
            .get("username")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let solidb_password = table
            .get("password")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let solidb_api_key = table
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());

        if adapter.is_sql() && url.is_none() {
            return Err(DbError::Backend(format!(
                "{}: connections.{} (adapter={}) requires url = \"…\"",
                path.display(),
                name,
                adapter.as_str()
            )));
        }

        connections.insert(
            name.clone(),
            ConnectionSpec {
                name: name.clone(),
                adapter,
                url,
                solidb_host,
                solidb_database,
                solidb_username,
                solidb_password,
                solidb_api_key,
                pool_size,
            },
        );
    }

    if connections.is_empty() {
        return Err(DbError::Backend(format!(
            "{}: no connections defined",
            path.display()
        )));
    }
    if !connections.contains_key(&default) {
        return Err(DbError::Backend(format!(
            "{}: default connection {:?} not found in [connections]",
            path.display(),
            default
        )));
    }

    Ok(ConnectionRegistry {
        default,
        connections,
        from_file: true,
    })
}

/// Expand `${VAR}` and `${VAR:-default}` in config text.
pub fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = input[i + 2..].find('}') {
                let body = &input[i + 2..i + 2 + end];
                let (var, default) = if let Some((v, d)) = body.split_once(":-") {
                    (v, Some(d))
                } else {
                    (body, None)
                };
                let val = std::env::var(var).unwrap_or_else(|_| default.unwrap_or("").to_string());
                out.push_str(&val);
                i = i + 2 + end + 1;
                continue;
            }
        }
        // Step by whole characters: `bytes[i] as char` reinterprets each UTF-8
        // byte as a codepoint, so a password containing `é` came back as `Ã©`.
        let ch = input[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

// ---------- active connection (thread-local) ----------

thread_local! {
    static ACTIVE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Restores the previously active connection on drop.
///
/// A plain post-call restore is not enough: a panic inside the adapter unwinds
/// straight past it, and `serve::run_caught` catches the panic and keeps the
/// worker alive. The override then leaked for the rest of that worker's life,
/// so every later request on it read *and wrote* the secondary database.
struct ActiveGuard(Option<String>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let prev = self.0.take();
        ACTIVE.with(|c| *c.borrow_mut() = prev);
    }
}

/// Run `f` with the given connection name as the active SQL/SoliDB target.
pub fn with_connection<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let prev = ACTIVE.with(|c| c.replace(Some(name.to_string())));
    let _guard = ActiveGuard(prev);
    f()
}

/// Active connection name, or registry default.
pub fn active_connection_name() -> String {
    ACTIVE.with(|c| {
        c.borrow()
            .clone()
            .unwrap_or_else(|| registry().default.clone())
    })
}

/// Spec for the active (or default) connection.
pub fn active_spec() -> Result<ConnectionSpec, String> {
    let reg = registry();
    let name = active_connection_name();
    reg.resolve(Some(&name)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn expand_env_default_and_set() {
        std::env::set_var("SOLI_TEST_EXPAND_A", "hello");
        assert_eq!(expand_env("x=${SOLI_TEST_EXPAND_A}y"), "x=helloy");
        assert_eq!(
            expand_env("x=${SOLI_TEST_EXPAND_MISSING:-fallback}y"),
            "x=fallbacky"
        );
        std::env::remove_var("SOLI_TEST_EXPAND_A");
    }

    #[test]
    fn parse_toml_multi() {
        let dir = tempfile_dir();
        let path = dir.join("database.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            r#"
default = "primary"
[connections.primary]
adapter = "solidb"
host = "http://localhost:6745"
database = "app"
[connections.legacy]
adapter = "postgres"
url = "postgres://u:p@localhost/legacy"
pool = 4
"#
        )
        .unwrap();
        let reg = load_from_toml(&path).unwrap();
        assert_eq!(reg.default, "primary");
        assert!(reg.from_file);
        assert_eq!(reg.get("primary").unwrap().adapter, Adapter::Solidb);
        assert_eq!(reg.get("legacy").unwrap().adapter, Adapter::Postgres);
        assert_eq!(
            reg.get("legacy").unwrap().url.as_deref(),
            Some("postgres://u:p@localhost/legacy")
        );
        assert_eq!(reg.get("legacy").unwrap().pool_size, Some(4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_default_errors() {
        let dir = tempfile_dir();
        let path = dir.join("database.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            r#"
default = "nope"
[connections.primary]
adapter = "solidb"
"#
        )
        .unwrap();
        let err = load_from_toml(&path).unwrap_err();
        assert!(err.message().contains("default"), "{}", err.message());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tempfile_dir() -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("soli_db_reg_{}", std::process::id()));
        d.push(format!("{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
