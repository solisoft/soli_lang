//! Errors for adapter selection and SQL/SoliDB backend operations.

use super::adapter::Adapter;
use std::fmt;

/// Database / adapter error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DbError {
    UnknownAdapter {
        value: String,
    },
    MissingDatabaseUrl {
        adapter: Adapter,
    },
    /// Config asked for an SQL adapter that was not compiled into this binary.
    FeatureNotCompiled {
        adapter: Adapter,
    },
    /// Runtime backend failure (connection, query, etc.).
    Backend(String),
}

impl DbError {
    /// Operator-facing message (stderr / RuntimeError).
    pub fn message(&self) -> String {
        match self {
            DbError::UnknownAdapter { value } => format!(
                "Unknown SOLI_DB_ADAPTER={value:?}. \
                 Use solidb (default), postgres, or mysql. \
                 See docs/sql-adapter-design.md."
            ),
            DbError::MissingDatabaseUrl { adapter } => {
                let example = match adapter {
                    Adapter::Mysql => "mysql://user:pass@localhost:3306/myapp",
                    _ => "postgres://user:pass@localhost:5432/myapp",
                };
                format!(
                    "SOLI_DB_ADAPTER={} requires DATABASE_URL \
                     (e.g. {example}). \
                     See docs/sql-adapter-design.md.",
                    adapter.as_str()
                )
            }
            DbError::FeatureNotCompiled { adapter } => {
                let feature = adapter.as_str();
                format!(
                    "Database adapter `{feature}` is not compiled into this soli binary. \
                     Rebuild with `--features {feature}` (or `sql` for both Postgres and MySQL). \
                     Slim SoliDB-only example: cargo install --path . --locked \
                     --no-default-features --features embedding,llm,codegraph. \
                     See Cargo.toml [features]."
                )
            }
            DbError::Backend(msg) => msg.clone(),
        }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for DbError {}
