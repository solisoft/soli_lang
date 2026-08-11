//! Database backend selection, multi-connection registry, and SQL adapters.
//!
//! SoliDB remains the default full-featured backend. Named connections live in
//! `config/database.toml` (or a single env-derived `primary`). Models opt into
//! a connection via class-body `connection "name"`.
//!
//! Design: `docs/sql-adapter-design.md`.

mod adapter;
mod caps;
mod error;
pub mod import;
pub mod mysql;
pub mod postgres;
pub mod registry;
pub mod sql;
pub mod sql_compile;

pub use adapter::{parse_adapter, Adapter, AdapterConfig};
pub use caps::BackendCaps;
pub use error::DbError;
pub use postgres::ListQueryParts;
pub use registry::{
    active_connection_name, active_spec, clear_registry_override, init_from_app_path, registry,
    set_registry_for_tests, with_connection, ConnectionRegistry, ConnectionSpec,
};
pub use sql_compile::{GroupAgg, ListQuery, SoftDeleteMode as SqlSoftDeleteMode, SqlAgg};

use std::sync::OnceLock;

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

/// True when the active connection is SoliDB.
pub fn is_solidb() -> bool {
    !is_sql()
}

/// Ensure all SQL connections in the registry can connect (lazy solidb ok).
pub fn ensure_runtime_ready() -> Result<(), DbError> {
    let reg = registry();
    for (name, spec) in &reg.connections {
        if !spec.adapter.is_sql() {
            continue;
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
    use registry::{set_registry_for_tests, clear_registry_override, ConnectionRegistry, ConnectionSpec};
    use std::collections::HashMap;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_registry_for_tests(reg);
        f();
        clear_registry_override();
    }

    #[test]
    fn solidb_default_is_ready() {
        with_registry(solidb_primary(), || {
            assert!(ensure_runtime_ready().is_ok());
            assert!(is_solidb());
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
                assert!(matches!(err, DbError::MissingDatabaseUrl { .. }));
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
                assert!(matches!(err, DbError::MissingDatabaseUrl { .. }));
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
}
