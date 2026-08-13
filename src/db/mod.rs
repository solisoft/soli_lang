//! Database backend selection, multi-connection registry, and SQL adapters.
//!
//! SoliDB remains the default full-featured backend. Named connections live in
//! `config/database.toml` (or a single env-derived `primary`). Models opt into
//! a connection via class-body `connection "name"`.
//!
//! SQL backends and their client crates are Cargo features (`postgres`,
//! `mysql`, `sqlite`; all on by default). Drop them at build time for a smaller
//! binary.
//!
//! Design: `docs/sql-adapter-design.md`.

mod adapter;
mod caps;
pub mod columns;
pub mod ddl;
mod error;
pub mod import;
pub mod introspect;
#[cfg(feature = "mysql")]
pub mod mysql;
#[cfg(feature = "postgres")]
pub mod postgres;
pub mod registry;
pub mod sql;
pub mod sql_columns_compile;
pub mod sql_compile;
#[cfg(feature = "sqlite")]
pub mod sqlite;

pub use adapter::{parse_adapter, Adapter, AdapterConfig};
pub use caps::BackendCaps;
pub use error::DbError;
pub use registry::{
    active_connection_name, active_spec, clear_registry_override, init_from_app_path, registry,
    set_registry_for_tests, with_connection, ConnectionRegistry, ConnectionSpec,
};
pub use sql_compile::{
    GroupAgg, ListQuery, ListQueryParts, SoftDeleteMode as SqlSoftDeleteMode, SqlAgg,
};

use std::sync::OnceLock;

/// Read a counter value the database rendered as text.
///
/// A JSON number can come back as `7`, `7.0`, or (on an exact-numeric cast)
/// `7.000`; a counter is an integer either way.
pub fn parse_counter(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if let Ok(n) = trimmed.parse::<i64>() {
        return Some(n);
    }
    trimmed.parse::<f64>().ok().map(|f| f.round() as i64)
}

/// Whether this binary was built with the given SQL adapter's client code.
pub fn adapter_feature_enabled(adapter: Adapter) -> bool {
    match adapter {
        Adapter::Solidb => true,
        Adapter::Postgres => cfg!(feature = "postgres"),
        Adapter::Mysql => cfg!(feature = "mysql"),
        Adapter::Sqlite => cfg!(feature = "sqlite"),
    }
}

/// Process-wide adapter config for the **default** connection (compat).
pub fn config() -> &'static AdapterConfig {
    static CFG: OnceLock<AdapterConfig> = OnceLock::new();
    CFG.get_or_init(|| {
        let reg = registry();
        let spec = reg.default_spec();
        AdapterConfig {
            adapter: spec.adapter,
            database_url: spec.url.clone(),
            pool_size: spec.pool_size,
        }
    })
}

/// Capabilities of the **active** (or default) connection.
pub fn caps() -> BackendCaps {
    active_spec()
        .map(|s| s.adapter.caps())
        .unwrap_or_else(|_| BackendCaps::solidb())
}

/// True when the active connection is a SQL document backend.
pub fn is_sql() -> bool {
    active_spec().map(|s| s.is_sql()).unwrap_or(false)
}

/// True when the active connection is PostgreSQL.
pub fn is_postgres() -> bool {
    active_spec()
        .map(|s| s.adapter == Adapter::Postgres)
        .unwrap_or(false)
}

/// True when the active connection is MySQL.
pub fn is_mysql() -> bool {
    active_spec()
        .map(|s| s.adapter == Adapter::Mysql)
        .unwrap_or(false)
}

/// True when the active connection is SQLite.
pub fn is_sqlite() -> bool {
    active_spec()
        .map(|s| s.adapter == Adapter::Sqlite)
        .unwrap_or(false)
}

/// True when the active connection is SoliDB.
pub fn is_solidb() -> bool {
    !is_sql()
}

/// Ensure all SQL connections in the registry can connect (lazy solidb ok).
pub fn ensure_runtime_ready() -> Result<(), DbError> {
    let reg = registry();
    for (name, spec) in &reg.connections {
        if !spec.adapter.is_sql() {
            // Per-connection SoliDB routing is not implemented: every SoliDB
            // request builds its URL from SOLIDB_HOST/SOLIDB_DATABASE env. A
            // spec declaring a different target would validate here yet send
            // ALL of its traffic to the env default — silent cross-database
            // corruption. Refuse to boot instead.
            let env_host = std::env::var("SOLIDB_HOST")
                .unwrap_or_else(|_| "http://localhost:6745".to_string());
            let env_db = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());
            let host_mismatch = spec
                .solidb_host
                .as_deref()
                .is_some_and(|h| h.trim_end_matches('/') != env_host.trim_end_matches('/'));
            let db_mismatch = spec.solidb_database.as_deref().is_some_and(|d| d != env_db);
            if host_mismatch || db_mismatch {
                return Err(DbError::Backend(format!(
                    "connection {name:?}: per-connection SoliDB hosts/databases are not \
                     supported yet — SoliDB requests always target SOLIDB_HOST/\
                     SOLIDB_DATABASE ({env_host}, database {env_db:?}). Point this \
                     connection at the same host and database (or configure it via env), \
                     or use a SQL adapter (postgres/mysql) for secondary connections."
                )));
            }
            continue;
        }
        if !adapter_feature_enabled(spec.adapter) {
            return Err(DbError::FeatureNotCompiled {
                adapter: spec.adapter,
            });
        }
        if spec.url.is_none() {
            return Err(DbError::MissingDatabaseUrl {
                adapter: spec.adapter,
            });
        }
        with_connection(name, || sql::ensure_connected().map_err(DbError::Backend))?;
    }
    Ok(())
}

/// Require a capability; return an operator-facing error if missing.
pub fn require_cap(cap: &str, available: bool) -> Result<(), String> {
    if available {
        return Ok(());
    }
    Err(format!(
        "{cap} is SoliDB-only (current connection: {}). \
         See docs/sql-adapter-design.md.",
        adapter_label()
    ))
}

/// Human one-liner for boot banners / CLI.
pub fn adapter_label() -> String {
    active_spec()
        .map(|s| s.label())
        .unwrap_or_else(|_| "primary (solidb)".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use registry::{
        clear_registry_override, set_registry_for_tests, ConnectionRegistry, ConnectionSpec,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        // Shared with the postgres/mysql integration tests — the registry
        // override is process-global, so one mutex must cover every module.
        registry::registry_test_lock()
    }

    fn solidb_primary() -> ConnectionRegistry {
        let mut connections = HashMap::new();
        connections.insert(
            "primary".into(),
            ConnectionSpec {
                name: "primary".into(),
                adapter: Adapter::Solidb,
                url: None,
                solidb_host: Some("http://localhost:6745".into()),
                solidb_database: Some("default".into()),
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: None,
            },
        );
        ConnectionRegistry {
            default: "primary".into(),
            connections,
            from_file: false,
        }
    }

    fn with_registry(reg: ConnectionRegistry, f: impl FnOnce()) {
        let _g = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Clear on unwind too — a panicking test must not leak its override
        // into whichever test takes the (poison-recovered) lock next.
        struct ClearOnDrop;
        impl Drop for ClearOnDrop {
            fn drop(&mut self) {
                clear_registry_override();
            }
        }
        set_registry_for_tests(reg);
        let _clear = ClearOnDrop;
        f();
    }

    #[test]
    fn solidb_default_is_ready() {
        with_registry(solidb_primary(), || {
            assert!(ensure_runtime_ready().is_ok());
            assert!(is_solidb());
        });
    }

    #[test]
    fn solidb_connection_pointing_elsewhere_errors() {
        // A named SoliDB connection targeting a host/database other than the
        // env-configured one must fail at boot — SoliDB traffic is routed via
        // SOLIDB_HOST/SOLIDB_DATABASE, so it would silently hit the wrong DB.
        let mut reg = solidb_primary();
        reg.connections.insert(
            "analytics".into(),
            ConnectionSpec {
                name: "analytics".into(),
                adapter: Adapter::Solidb,
                url: None,
                solidb_host: Some("http://db2:6745".into()),
                solidb_database: Some("metrics".into()),
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: None,
            },
        );
        with_registry(reg, || {
            let err = ensure_runtime_ready().unwrap_err();
            assert!(
                err.message().contains("per-connection SoliDB"),
                "{}",
                err.message()
            );
        });
    }

    #[test]
    fn postgres_without_url_errors() {
        let mut connections = HashMap::new();
        connections.insert(
            "primary".into(),
            ConnectionSpec {
                name: "primary".into(),
                adapter: Adapter::Postgres,
                url: None,
                solidb_host: None,
                solidb_database: None,
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: None,
            },
        );
        with_registry(
            ConnectionRegistry {
                default: "primary".into(),
                connections,
                from_file: false,
            },
            || {
                let err = ensure_runtime_ready().unwrap_err();
                if cfg!(feature = "postgres") {
                    assert!(matches!(err, DbError::MissingDatabaseUrl { .. }));
                } else {
                    assert!(matches!(err, DbError::FeatureNotCompiled { .. }));
                }
            },
        );
    }

    #[test]
    fn mysql_without_url_errors() {
        let mut connections = HashMap::new();
        connections.insert(
            "primary".into(),
            ConnectionSpec {
                name: "primary".into(),
                adapter: Adapter::Mysql,
                url: None,
                solidb_host: None,
                solidb_database: None,
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: None,
            },
        );
        with_registry(
            ConnectionRegistry {
                default: "primary".into(),
                connections,
                from_file: false,
            },
            || {
                let err = ensure_runtime_ready().unwrap_err();
                if cfg!(feature = "mysql") {
                    assert!(matches!(err, DbError::MissingDatabaseUrl { .. }));
                } else {
                    assert!(matches!(err, DbError::FeatureNotCompiled { .. }));
                }
            },
        );
    }

    #[test]
    fn require_cap_names_adapter() {
        let mut connections = HashMap::new();
        connections.insert(
            "primary".into(),
            ConnectionSpec {
                name: "primary".into(),
                adapter: Adapter::Postgres,
                url: Some("postgres://localhost/x".into()),
                solidb_host: None,
                solidb_database: None,
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: None,
            },
        );
        with_registry(
            ConnectionRegistry {
                default: "primary".into(),
                connections,
                from_file: false,
            },
            || {
                let err = require_cap("Graph traversal", false).unwrap_err();
                assert!(err.contains("SoliDB-only"));
                assert!(err.contains("postgres") || err.contains("primary"));
            },
        );
    }

    #[test]
    fn adapter_feature_flags_match_cfg() {
        assert!(adapter_feature_enabled(Adapter::Solidb));
        assert_eq!(
            adapter_feature_enabled(Adapter::Postgres),
            cfg!(feature = "postgres")
        );
        assert_eq!(
            adapter_feature_enabled(Adapter::Mysql),
            cfg!(feature = "mysql")
        );
    }
}
