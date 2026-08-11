//! Shared SQL document-backend facade (Postgres + MySQL).
//!
//! Routes through the **active** named connection (`registry::with_connection`).

pub use super::postgres::ListQueryParts;
use super::registry::active_spec;
use super::sql_compile::{GroupAgg, ListQuery, SqlAgg};
use super::{mysql, postgres, Adapter};

pub fn is_sql() -> bool {
    active_spec().map(|s| s.is_sql()).unwrap_or(false)
}

pub fn is_postgres() -> bool {
    active_spec()
        .map(|s| s.adapter == Adapter::Postgres)
        .unwrap_or(false)
}

pub fn is_mysql() -> bool {
    active_spec()
        .map(|s| s.adapter == Adapter::Mysql)
        .unwrap_or(false)
}

pub fn ensure_connected() -> Result<(), String> {
    let spec = active_spec()?;
    match spec.adapter {
        Adapter::Mysql => mysql::ensure_connected(),
        Adapter::Postgres => postgres::ensure_connected(),
        Adapter::Solidb => Err("ensure_connected called for solidb connection".into()),
    }
}

pub fn insert(
    table: &str,
    key: Option<&str>,
    document: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if is_mysql() {
        mysql::insert(table, key, document)
    } else {
        postgres::insert(table, key, document)
    }
}

pub fn get(table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    if is_mysql() {
        mysql::get(table, key)
    } else {
        postgres::get(table, key)
    }
}

pub fn update(
    table: &str,
    key: &str,
    document: serde_json::Value,
    merge: bool,
) -> Result<serde_json::Value, String> {
    if is_mysql() {
        mysql::update(table, key, document, merge)
    } else {
        postgres::update(table, key, document, merge)
    }
}

pub fn delete(table: &str, key: &str) -> Result<(), String> {
    if is_mysql() {
        mysql::delete(table, key)
    } else {
        postgres::delete(table, key)
    }
}

pub fn select(q: &ListQuery) -> Result<Vec<serde_json::Value>, String> {
    if is_mysql() {
        mysql::select(q)
    } else {
        postgres::select(q)
    }
}

pub fn select_by_keys(table: &str, keys: &[String]) -> Result<Vec<serde_json::Value>, String> {
    if is_mysql() {
        mysql::select_by_keys(table, keys)
    } else {
        postgres::select_by_keys(table, keys)
    }
}

pub fn select_json_text_in(
    table: &str,
    field: &str,
    values: &[String],
) -> Result<Vec<serde_json::Value>, String> {
    if is_mysql() {
        mysql::select_json_text_in(table, field, values)
    } else {
        postgres::select_json_text_in(table, field, values)
    }
}

pub fn group_by(
    q: &ListQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    if is_mysql() {
        mysql::group_by(q, group_fields, aggs)
    } else {
        postgres::group_by(q, group_fields, aggs)
    }
}

pub fn count(q: &ListQuery) -> Result<i64, String> {
    if is_mysql() {
        mysql::count(q)
    } else {
        postgres::count(q)
    }
}

pub fn exists(q: &ListQuery) -> Result<bool, String> {
    if is_mysql() {
        mysql::exists(q)
    } else {
        postgres::exists(q)
    }
}

pub fn aggregate(q: &ListQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    if is_mysql() {
        mysql::aggregate(q, func, field)
    } else {
        postgres::aggregate(q, func, field)
    }
}

pub fn delete_all(q: &ListQuery) -> Result<u64, String> {
    if is_mysql() {
        mysql::delete_all(q)
    } else {
        postgres::delete_all(q)
    }
}

pub fn update_all(q: &ListQuery, patch: serde_json::Value) -> Result<u64, String> {
    if is_mysql() {
        mysql::update_all(q, patch)
    } else {
        postgres::update_all(q, patch)
    }
}

pub fn ensure_table(table: &str) -> Result<(), String> {
    if is_mysql() {
        mysql::ensure_table(table)
    } else {
        postgres::ensure_table(table)
    }
}

pub fn drop_table(table: &str) -> Result<(), String> {
    if is_mysql() {
        mysql::drop_table(table)
    } else {
        postgres::drop_table(table)
    }
}

pub fn ensure_migrations_table() -> Result<(), String> {
    if is_mysql() {
        mysql::ensure_migrations_table()
    } else {
        postgres::ensure_migrations_table()
    }
}

pub fn list_applied_migrations() -> Result<Vec<(String, String)>, String> {
    if is_mysql() {
        mysql::list_applied_migrations()
    } else {
        postgres::list_applied_migrations()
    }
}

pub fn record_migration(version: &str, name: &str) -> Result<(), String> {
    if is_mysql() {
        mysql::record_migration(version, name)
    } else {
        postgres::record_migration(version, name)
    }
}

pub fn remove_migration(version: &str) -> Result<(), String> {
    if is_mysql() {
        mysql::remove_migration(version)
    } else {
        postgres::remove_migration(version)
    }
}

pub fn list_query_from_parts(parts: ListQueryParts) -> Result<ListQuery, String> {
    use super::sql_compile::Dialect;
    let dialect = if is_mysql() {
        Dialect::Mysql
    } else {
        Dialect::Postgres
    };
    postgres::list_query_from_parts(parts, dialect)
}
