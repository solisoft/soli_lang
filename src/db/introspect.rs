//! Schema introspection for column-aware models.
//!
//! A model that declares `table "orders"` maps to an **existing** relational
//! table with real columns, rather than the `_key` + `doc` document layout the
//! rest of the SQL backend uses. To read and write such a table, Soli needs to
//! know its columns, their types, and its primary key — so it asks the database
//! once (`information_schema`, or `PRAGMA table_info` on SQLite) and caches the
//! answer.
//!
//! Column mode never issues DDL. The schema is owned by whoever created the
//! table; Soli only reads its shape.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// A column's type, reduced to the set Soli can round-trip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColType {
    Int,
    Float,
    /// Exact numeric (`numeric`/`decimal`). Read as Float — see the docs for
    /// the precision caveat.
    Decimal,
    Bool,
    Text,
    Uuid,
    Date,
    DateTime,
    Json,
    /// A type Soli has no mapping for (arrays, `bytea`, geometry, …). Reads as
    /// null; writing or filtering on it is an error naming the column.
    Unknown,
}

impl ColType {
    /// Whether an aggregate like `SUM`/`AVG` is meaningful on this column.
    pub fn is_numeric(self) -> bool {
        matches!(self, ColType::Int | ColType::Float | ColType::Decimal)
    }

    /// Whether values of this type are carried as **text** through the driver
    /// and parsed in Rust.
    ///
    /// Numbers are included deliberately. `information_schema` collapses
    /// int2/int4/int8 (and float4/float8) into one Soli type, so the reader
    /// cannot know the exact width — and the Postgres driver refuses to
    /// deserialize an `int4` into an `i64`. Reading text and parsing sidesteps
    /// width entirely and gives both backends one canonical form.
    pub fn reads_as_text(self) -> bool {
        matches!(
            self,
            ColType::Uuid
                | ColType::Date
                | ColType::DateTime
                | ColType::Decimal
                | ColType::Int
                | ColType::Float
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ColType::Int => "Int",
            ColType::Float => "Float",
            ColType::Decimal => "Decimal",
            ColType::Bool => "Bool",
            ColType::Text => "Text",
            ColType::Uuid => "Uuid",
            ColType::Date => "Date",
            ColType::DateTime => "DateTime",
            ColType::Json => "Json",
            ColType::Unknown => "unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnDef {
    pub name: String,
    pub ty: ColType,
    pub nullable: bool,
}

/// The shape of one existing table.
#[derive(Clone, Debug)]
pub struct TableSchema {
    pub connection: String,
    pub table: String,
    /// Primary-key column name.
    pub pk: String,
    pub pk_type: ColType,
    /// True when the database generates the key (serial / identity /
    /// auto_increment), so inserts must omit it.
    pub pk_auto: bool,
    /// Columns in declaration order.
    pub columns: Vec<ColumnDef>,
    pub has_created_at: bool,
    pub has_updated_at: bool,
}

impl TableSchema {
    pub fn column(&self, name: &str) -> Option<&ColumnDef> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn has_column(&self, name: &str) -> bool {
        self.column(name).is_some()
    }

    /// Column names, for error messages that tell the user what *is* available.
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }
}

/// Raw introspection result from a backend, before validation.
#[derive(Clone, Debug, Default)]
pub struct RawColumns {
    /// `(name, db_type, second_type_hint, nullable, is_auto)`. The second hint
    /// carries MySQL's `COLUMN_TYPE` (needed to see `tinyint(1)` as bool);
    /// Postgres leaves it empty.
    pub columns: Vec<(String, String, String, bool, bool)>,
    /// Primary-key columns in key order. More than one means a composite PK.
    pub pk: Vec<String>,
}

/// Map a PostgreSQL `udt_name` to a [`ColType`].
pub fn pg_coltype(udt_name: &str) -> ColType {
    match udt_name {
        "int2" | "int4" | "int8" => ColType::Int,
        "float4" | "float8" => ColType::Float,
        "numeric" => ColType::Decimal,
        "bool" => ColType::Bool,
        "text" | "varchar" | "bpchar" | "citext" | "name" => ColType::Text,
        "uuid" => ColType::Uuid,
        "date" => ColType::Date,
        "timestamp" | "timestamptz" => ColType::DateTime,
        "json" | "jsonb" => ColType::Json,
        _ => ColType::Unknown,
    }
}

/// Map a MySQL `DATA_TYPE` (+ `COLUMN_TYPE`) to a [`ColType`].
pub fn mysql_coltype(data_type: &str, column_type: &str) -> ColType {
    let lower = data_type.to_ascii_lowercase();
    let full = column_type.to_ascii_lowercase();
    // `tinyint(1)` is the conventional MySQL bool; wider tinyints are integers.
    if lower == "tinyint" {
        return if full.starts_with("tinyint(1)") {
            ColType::Bool
        } else {
            ColType::Int
        };
    }
    if lower == "bit" {
        return if full.starts_with("bit(1)") {
            ColType::Bool
        } else {
            ColType::Unknown
        };
    }
    match lower.as_str() {
        // Even unsigned, these all fit in i64.
        "smallint" | "mediumint" | "int" | "integer" => ColType::Int,
        "bigint" => {
            // An unsigned BIGINT can exceed i64::MAX; refuse it at
            // introspection rather than silently wrapping a value.
            if full.contains("unsigned") {
                ColType::Unknown
            } else {
                ColType::Int
            }
        }
        "float" | "double" | "real" => ColType::Float,
        "decimal" | "numeric" => ColType::Decimal,
        "boolean" => ColType::Bool,
        "char" | "varchar" | "text" | "tinytext" | "mediumtext" | "longtext" | "enum" | "set" => {
            ColType::Text
        }
        "date" => ColType::Date,
        "datetime" | "timestamp" => ColType::DateTime,
        "json" => ColType::Json,
        _ => ColType::Unknown,
    }
}

/// Build a validated [`TableSchema`] from raw introspection output.
pub fn build_schema(
    connection: &str,
    table: &str,
    raw: RawColumns,
    map_type: impl Fn(&str, &str) -> ColType,
) -> Result<TableSchema, String> {
    if raw.columns.is_empty() {
        return Err(format!(
            "column-aware model: table {table:?} not found on connection {connection:?}. \
             Column mode never creates or alters tables — create it first, or fix the \
             `table` declaration."
        ));
    }
    if raw.pk.is_empty() {
        return Err(format!(
            "column-aware model: table {table:?} on connection {connection:?} has no \
             primary key. `find`, `update`, and `delete` need one."
        ));
    }
    if raw.pk.len() > 1 {
        return Err(format!(
            "column-aware model: table {table:?} on connection {connection:?} has a \
             composite primary key ({}); composite keys are not supported yet.",
            raw.pk.join(", ")
        ));
    }

    let pk = raw.pk[0].clone();
    let mut columns = Vec::with_capacity(raw.columns.len());
    let mut pk_type = ColType::Unknown;
    let mut pk_auto = false;
    for (name, db_type, hint, nullable, is_auto) in raw.columns {
        let ty = map_type(&db_type, &hint);
        if name == pk {
            pk_type = ty;
            pk_auto = is_auto;
        }
        columns.push(ColumnDef { name, ty, nullable });
    }

    let has_created_at = columns.iter().any(|c| c.name == "created_at");
    let has_updated_at = columns.iter().any(|c| c.name == "updated_at");

    Ok(TableSchema {
        connection: connection.to_string(),
        table: table.to_string(),
        pk,
        pk_type,
        pk_auto,
        columns,
        has_created_at,
        has_updated_at,
    })
}

// ---------- cache ----------

type Cache = RwLock<HashMap<(String, String), Arc<TableSchema>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Schema for `table` on the **active** connection, introspecting on first use.
///
/// Introspection happens outside the cache lock — holding a lock across a pool
/// checkout would serialize every column-mode model behind one query (and could
/// deadlock a single-connection pool).
pub fn get_schema(table: &str) -> Result<Arc<TableSchema>, String> {
    let connection = super::registry::active_connection_name();
    let key = (connection.clone(), table.to_string());

    if let Ok(map) = cache().read() {
        if let Some(found) = map.get(&key) {
            return Ok(found.clone());
        }
    }

    let schema = Arc::new(introspect(&connection, table)?);
    if let Ok(mut map) = cache().write() {
        map.insert(key, schema.clone());
    }
    Ok(schema)
}

/// Drop one table's cached schema, so the next use re-introspects it.
///
/// The escape hatch for an `ALTER TABLE` applied while the server runs: the
/// column path calls this when a query mentions a column the cached schema does
/// not have, then retries once. Without it, a newly added column stayed invisible
/// until restart.
pub fn invalidate_schema(table: &str) {
    let connection = super::registry::active_connection_name();
    if let Ok(mut map) = cache().write() {
        map.remove(&(connection, table.to_string()));
    }
}

/// Drop every cached schema. Called on hot reload and when tests swap the
/// connection registry, so a changed table (or a different database) is
/// re-introspected rather than answered from a stale entry.
pub fn clear_schema_cache() {
    if let Ok(mut map) = cache().write() {
        map.clear();
    }
}

/// Introspect on the active connection, dispatching per adapter.
fn introspect(connection: &str, table: &str) -> Result<TableSchema, String> {
    let spec = super::registry::active_spec()?;
    match spec.adapter {
        super::Adapter::Solidb => Err(format!(
            "column-aware model: table {table:?} requires a SQL connection, but \
             {connection:?} is solidb. Declare `connection \"<postgres-or-mysql>\"` \
             on the model, or drop the `table` declaration to use document storage."
        )),
        super::Adapter::Postgres => {
            #[cfg(feature = "postgres")]
            {
                let raw = super::postgres::introspect_table(table)?;
                build_schema(connection, table, raw, |t, _| pg_coltype(t))
            }
            #[cfg(not(feature = "postgres"))]
            {
                Err(feature_missing("postgres", table))
            }
        }
        super::Adapter::Mysql => {
            #[cfg(feature = "mysql")]
            {
                let raw = super::mysql::introspect_table(table)?;
                build_schema(connection, table, raw, mysql_coltype)
            }
            #[cfg(not(feature = "mysql"))]
            {
                Err(feature_missing("mysql", table))
            }
        }
        super::Adapter::Sqlite => {
            #[cfg(feature = "sqlite")]
            {
                let raw = super::sqlite::introspect_table(table)?;
                build_schema(connection, table, raw, |t, _| {
                    super::sqlite::sqlite_coltype(t)
                })
            }
            #[cfg(not(feature = "sqlite"))]
            {
                Err(feature_missing("sqlite", table))
            }
        }
    }
}

#[cfg(any(
    not(feature = "postgres"),
    not(feature = "mysql"),
    not(feature = "sqlite")
))]
fn feature_missing(adapter: &str, table: &str) -> String {
    format!(
        "column-aware model for table {table:?} needs the `{adapter}` adapter, which is \
         not compiled into this soli binary. Rebuild with `--features {adapter}`."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(columns: &[(&str, &str, &str, bool, bool)], pk: &[&str]) -> RawColumns {
        RawColumns {
            columns: columns
                .iter()
                .map(|(n, t, h, null, auto)| {
                    (n.to_string(), t.to_string(), h.to_string(), *null, *auto)
                })
                .collect(),
            pk: pk.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn postgres_types_map_to_soli_types() {
        assert_eq!(pg_coltype("int4"), ColType::Int);
        assert_eq!(pg_coltype("int8"), ColType::Int);
        assert_eq!(pg_coltype("float8"), ColType::Float);
        assert_eq!(pg_coltype("numeric"), ColType::Decimal);
        assert_eq!(pg_coltype("bool"), ColType::Bool);
        assert_eq!(pg_coltype("varchar"), ColType::Text);
        assert_eq!(pg_coltype("uuid"), ColType::Uuid);
        assert_eq!(pg_coltype("timestamptz"), ColType::DateTime);
        assert_eq!(pg_coltype("date"), ColType::Date);
        assert_eq!(pg_coltype("jsonb"), ColType::Json);
        // Unmapped types are explicit, not silently text.
        assert_eq!(pg_coltype("bytea"), ColType::Unknown);
        assert_eq!(pg_coltype("_int4"), ColType::Unknown); // int[] array
    }

    #[test]
    fn mysql_tinyint_1_is_bool_but_wider_tinyints_are_ints() {
        // The classic MySQL bool convention; getting this wrong turns every
        // boolean column into 0/1 integers.
        assert_eq!(mysql_coltype("tinyint", "tinyint(1)"), ColType::Bool);
        assert_eq!(mysql_coltype("tinyint", "tinyint(4)"), ColType::Int);
        assert_eq!(mysql_coltype("boolean", "tinyint(1)"), ColType::Bool);
    }

    #[test]
    fn mysql_unsigned_bigint_is_refused_rather_than_wrapped() {
        // Values above i64::MAX would wrap; better to error at introspection.
        assert_eq!(
            mysql_coltype("bigint", "bigint(20) unsigned"),
            ColType::Unknown
        );
        assert_eq!(mysql_coltype("bigint", "bigint(20)"), ColType::Int);
    }

    #[test]
    fn mysql_common_types_map() {
        assert_eq!(mysql_coltype("varchar", "varchar(255)"), ColType::Text);
        assert_eq!(mysql_coltype("decimal", "decimal(10,2)"), ColType::Decimal);
        assert_eq!(mysql_coltype("datetime", "datetime"), ColType::DateTime);
        assert_eq!(mysql_coltype("json", "json"), ColType::Json);
        assert_eq!(mysql_coltype("blob", "blob"), ColType::Unknown);
        assert_eq!(mysql_coltype("enum", "enum('a','b')"), ColType::Text);
    }

    #[test]
    fn build_schema_detects_pk_type_auto_and_timestamps() {
        let schema = build_schema(
            "legacy",
            "orders",
            raw(
                &[
                    ("id", "int8", "", false, true),
                    ("name", "text", "", false, false),
                    ("total", "numeric", "", true, false),
                    ("created_at", "timestamptz", "", true, false),
                    ("updated_at", "timestamptz", "", true, false),
                ],
                &["id"],
            ),
            |t, _| pg_coltype(t),
        )
        .expect("schema");

        assert_eq!(schema.pk, "id");
        assert_eq!(schema.pk_type, ColType::Int);
        assert!(schema.pk_auto, "BIGSERIAL must be detected as generated");
        assert!(schema.has_created_at && schema.has_updated_at);
        assert_eq!(schema.columns.len(), 5);
        assert_eq!(schema.column("total").unwrap().ty, ColType::Decimal);
        assert!(schema.column("total").unwrap().nullable);
        assert!(!schema.column("name").unwrap().nullable);
        assert!(schema.has_column("name") && !schema.has_column("nope"));
    }

    #[test]
    fn missing_table_error_says_column_mode_never_creates_tables() {
        let err = build_schema("legacy", "ghosts", raw(&[], &[]), |t, _| pg_coltype(t))
            .expect_err("must error");
        assert!(err.contains("not found"), "{err}");
        assert!(err.contains("never creates or alters"), "{err}");
        assert!(err.contains("ghosts") && err.contains("legacy"), "{err}");
    }

    #[test]
    fn composite_and_missing_primary_keys_are_refused_clearly() {
        let composite = build_schema(
            "legacy",
            "order_items",
            raw(
                &[
                    ("order_id", "int8", "", false, false),
                    ("item_id", "int8", "", false, false),
                ],
                &["order_id", "item_id"],
            ),
            |t, _| pg_coltype(t),
        )
        .expect_err("composite PK must error");
        assert!(composite.contains("composite primary key"), "{composite}");
        assert!(composite.contains("order_id, item_id"), "{composite}");

        let none = build_schema(
            "legacy",
            "logs",
            raw(&[("message", "text", "", true, false)], &[]),
            |t, _| pg_coltype(t),
        )
        .expect_err("missing PK must error");
        assert!(
            none.contains("no \nprimary key") || none.contains("no primary key"),
            "{none}"
        );
    }

    #[test]
    fn numeric_and_text_read_classifications() {
        assert!(ColType::Int.is_numeric() && ColType::Decimal.is_numeric());
        assert!(!ColType::Text.is_numeric() && !ColType::Bool.is_numeric());
        // Types the driver hands back as text and Rust parses. Numbers are
        // included: the exact SQL width is unknown, and the Postgres driver
        // refuses int4 -> i64.
        assert!(ColType::DateTime.reads_as_text() && ColType::Uuid.reads_as_text());
        assert!(ColType::Int.reads_as_text() && ColType::Float.reads_as_text());
        assert!(!ColType::Json.reads_as_text() && !ColType::Bool.reads_as_text());
    }
}
