//! `SOLI_DB_ADAPTER` / `DATABASE_URL` parsing.

use super::caps::BackendCaps;
use super::error::DbError;

/// Selected storage backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adapter {
    /// SoliDB / SolidB — default, full Model surface.
    Solidb,
    /// PostgreSQL document backend (`_key` + JSONB `doc`).
    Postgres,
    /// MySQL / MariaDB document backend (`_key` + JSON `doc`).
    Mysql,
    /// SQLite document backend (`_key` + JSON `doc`), one file, no server.
    Sqlite,
}

impl Adapter {
    pub fn as_str(self) -> &'static str {
        match self {
            Adapter::Solidb => "solidb",
            Adapter::Postgres => "postgres",
            Adapter::Mysql => "mysql",
            Adapter::Sqlite => "sqlite",
        }
    }

    pub fn caps(self) -> BackendCaps {
        match self {
            Adapter::Solidb => BackendCaps::solidb(),
            Adapter::Postgres => BackendCaps::postgres(),
            Adapter::Mysql => BackendCaps::mysql(),
            Adapter::Sqlite => BackendCaps::sqlite(),
        }
    }

    pub fn is_sql(self) -> bool {
        matches!(self, Adapter::Postgres | Adapter::Mysql | Adapter::Sqlite)
    }
}

/// Parsed adapter configuration from the environment.
#[derive(Clone, Debug)]
pub struct AdapterConfig {
    pub adapter: Adapter,
    /// Present when `DATABASE_URL` is set (required for SQL adapters).
    pub database_url: Option<String>,
    /// Optional pool size for SQL adapters (`SOLI_DB_POOL_SIZE`).
    pub pool_size: Option<usize>,
}

impl AdapterConfig {
    /// Parse adapter settings from the environment. Fails on unknown
    /// `SOLI_DB_ADAPTER` values (does not silently fall back).
    pub fn from_env() -> Result<Self, DbError> {
        let adapter = parse_adapter(std::env::var("SOLI_DB_ADAPTER").ok().as_deref())?;
        let database_url = std::env::var("DATABASE_URL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let pool_size = std::env::var("SOLI_DB_POOL_SIZE")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0);
        Ok(Self {
            adapter,
            database_url,
            pool_size,
        })
    }
}

/// Parse an adapter name. Empty / unset → caller uses Solidb default.
///
/// Accepted aliases:
/// - solidb, solid, sdb
/// - postgres, postgresql, pg
/// - mysql, mariadb
/// - sqlite, sqlite3
pub fn parse_adapter(raw: Option<&str>) -> Result<Adapter, DbError> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Adapter::Solidb);
    };
    match raw.to_ascii_lowercase().as_str() {
        "solidb" | "solid" | "sdb" | "default" => Ok(Adapter::Solidb),
        "postgres" | "postgresql" | "pg" => Ok(Adapter::Postgres),
        "mysql" | "mariadb" => Ok(Adapter::Mysql),
        "sqlite" | "sqlite3" => Ok(Adapter::Sqlite),
        other => Err(DbError::UnknownAdapter {
            value: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases() {
        assert_eq!(parse_adapter(None).unwrap(), Adapter::Solidb);
        assert_eq!(parse_adapter(Some("")).unwrap(), Adapter::Solidb);
        assert_eq!(parse_adapter(Some("solidb")).unwrap(), Adapter::Solidb);
        assert_eq!(parse_adapter(Some("PG")).unwrap(), Adapter::Postgres);
        assert_eq!(
            parse_adapter(Some("PostgreSQL")).unwrap(),
            Adapter::Postgres
        );
        assert_eq!(parse_adapter(Some("mariadb")).unwrap(), Adapter::Mysql);
        assert_eq!(parse_adapter(Some("sqlite")).unwrap(), Adapter::Sqlite);
    }

    #[test]
    fn rejects_unknown() {
        let err = parse_adapter(Some("mongo")).unwrap_err();
        match err {
            DbError::UnknownAdapter { value } => assert_eq!(value, "mongo"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_sqlite_aliases() {
        assert_eq!(parse_adapter(Some("sqlite")).unwrap(), Adapter::Sqlite);
        assert_eq!(parse_adapter(Some("SQLite3")).unwrap(), Adapter::Sqlite);
        assert!(Adapter::Sqlite.is_sql());
    }

    #[test]
    fn solidb_is_not_sql() {
        assert!(!Adapter::Solidb.is_sql());
        assert!(Adapter::Postgres.is_sql());
    }
}
