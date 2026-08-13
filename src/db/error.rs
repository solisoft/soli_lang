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

// ---------- constraint violations ----------

/// The kind of constraint a write violated.
///
/// Every adapter reports these differently — Postgres by SQLSTATE, MySQL by
/// error number, SQLite by extended result code — so each one classifies its own
/// driver error and the model layer reads the result through one shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    Unique,
    ForeignKey,
    NotNull,
    Check,
}

impl ConstraintKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ConstraintKind::Unique => "unique",
            ConstraintKind::ForeignKey => "foreign_key",
            ConstraintKind::NotNull => "not_null",
            ConstraintKind::Check => "check",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "unique" => Some(ConstraintKind::Unique),
            "foreign_key" => Some(ConstraintKind::ForeignKey),
            "not_null" => Some(ConstraintKind::NotNull),
            "check" => Some(ConstraintKind::Check),
            _ => None,
        }
    }

    /// The validation message a field error carries for this kind.
    pub fn message(self) -> &'static str {
        match self {
            ConstraintKind::Unique => "has already been taken",
            ConstraintKind::ForeignKey => "must reference an existing record",
            ConstraintKind::NotNull => "can't be blank",
            ConstraintKind::Check => "is invalid",
        }
    }
}

/// A classified constraint violation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraint {
    pub kind: ConstraintKind,
    /// The offending column, when the database names one.
    pub column: Option<String>,
    /// The constraint or index name, which often carries the column name.
    pub name: Option<String>,
}

/// Prefix that carries a classified violation through the `Result<_, String>`
/// boundary the db facade uses.
const MARKER: &str = "soli:constraint=";

impl Constraint {
    pub fn new(kind: ConstraintKind) -> Self {
        Self {
            kind,
            column: None,
            name: None,
        }
    }

    pub fn with_column(mut self, column: Option<String>) -> Self {
        self.column = column.filter(|c| !c.is_empty());
        self
    }

    pub fn with_name(mut self, name: Option<String>) -> Self {
        self.name = name.filter(|n| !n.is_empty());
        self
    }

    /// Render as a machine-readable prefix followed by human text.
    ///
    /// The facade's error type is `String`, so the classification travels in the
    /// message rather than in a type. Parsing our own marker is exact; parsing a
    /// driver's prose would not be.
    pub fn to_marker(&self) -> String {
        let mut out = format!("{MARKER}{};", self.kind.as_str());
        if let Some(column) = &self.column {
            out.push_str(&format!("column={column};"));
        }
        if let Some(name) = &self.name {
            out.push_str(&format!("name={name};"));
        }
        out
    }

    /// Recover a violation from an error string, if one was marked.
    pub fn parse(err: &str) -> Option<Self> {
        let start = err.find(MARKER)? + MARKER.len();
        let rest = &err[start..];
        let end = rest.find(' ').unwrap_or(rest.len());
        let fields = &rest[..end];
        let mut parts = fields.split(';').filter(|p| !p.is_empty());
        let kind = ConstraintKind::parse(parts.next()?)?;
        let mut constraint = Constraint::new(kind);
        for part in parts {
            match part.split_once('=') {
                Some(("column", value)) => constraint.column = Some(value.to_string()),
                Some(("name", value)) => constraint.name = Some(value.to_string()),
                _ => {}
            }
        }
        Some(constraint)
    }

    /// Best guess at the field a validation error should hang off: the column the
    /// database named, else a name pulled out of the constraint/index name.
    ///
    /// Index names are conventionally `idx_<table>_<column>` or
    /// `<table>_<column>_key`, so the last segment is usually the column.
    pub fn field(&self) -> Option<String> {
        if let Some(column) = &self.column {
            return Some(column.clone());
        }
        let name = self.name.as_ref()?;
        let trimmed = name
            .trim_end_matches("_key")
            .trim_end_matches("_idx")
            .trim_end_matches("_unique");
        trimmed
            .rsplit('_')
            .find(|segment| !segment.is_empty())
            .map(|segment| segment.to_string())
    }
}

/// Extract `code` from a Postgres `DETAIL: Key (code)=(x) …` line.
pub fn column_from_key_detail(detail: &str) -> Option<String> {
    let start = detail.find("Key (")? + "Key (".len();
    let rest = &detail[start..];
    let end = rest.find(')')?;
    let first = rest[..end].split(',').next()?.trim();
    // A composite key names several columns; the first is enough to point at.
    if first.is_empty() {
        None
    } else {
        Some(first.trim_matches('"').to_string())
    }
}

#[cfg(test)]
mod constraint_tests {
    use super::*;

    #[test]
    fn a_marker_round_trips() {
        let original = Constraint::new(ConstraintKind::Unique)
            .with_column(Some("code".into()))
            .with_name(Some("orders_code_key".into()));
        let text = format!("{} sqlite insert: UNIQUE failed", original.to_marker());
        let parsed = Constraint::parse(&text).expect("marker parses");
        assert_eq!(parsed.kind, ConstraintKind::Unique);
        assert_eq!(parsed.column.as_deref(), Some("code"));
        assert_eq!(parsed.name.as_deref(), Some("orders_code_key"));
        assert_eq!(parsed.field().as_deref(), Some("code"));
        // An unmarked error stays unclassified rather than being guessed at.
        assert!(Constraint::parse("postgres insert: connection closed").is_none());
    }

    #[test]
    fn a_field_is_recovered_from_an_index_name_when_the_column_is_unnamed() {
        let from_pg =
            Constraint::new(ConstraintKind::Unique).with_name(Some("orders_code_key".into()));
        assert_eq!(from_pg.field().as_deref(), Some("code"));

        let from_mysql =
            Constraint::new(ConstraintKind::Unique).with_name(Some("idx_orders_email".into()));
        assert_eq!(from_mysql.field().as_deref(), Some("email"));

        // Nothing to go on: no field rather than a wrong one.
        assert_eq!(Constraint::new(ConstraintKind::Unique).field(), None);
    }

    #[test]
    fn postgres_detail_names_the_column() {
        assert_eq!(
            column_from_key_detail("Key (code)=(x) already exists.").as_deref(),
            Some("code")
        );
        assert_eq!(
            column_from_key_detail("Key (ref)=(999) is not present in table \"orders\".")
                .as_deref(),
            Some("ref")
        );
        // A composite key points at its first column.
        assert_eq!(
            column_from_key_detail("Key (a, b)=(1, 2) already exists.").as_deref(),
            Some("a")
        );
        assert_eq!(column_from_key_detail("Failing row contains (4, w)."), None);
    }

    #[test]
    fn each_kind_carries_a_validation_message() {
        assert_eq!(ConstraintKind::Unique.message(), "has already been taken");
        assert_eq!(ConstraintKind::NotNull.message(), "can't be blank");
        assert!(ConstraintKind::ForeignKey.message().contains("existing"));
    }
}
