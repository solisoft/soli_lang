//! Shared SQL document-backend facade (Postgres, MySQL, SQLite).
//!
//! Routes through the **active** named connection (`registry::with_connection`).
//! Each backend is feature-gated (`postgres` / `mysql` / `sqlite`); calling a
//! disabled adapter yields a rebuild hint rather than a link error at app boot.

// When no SQL client is linked, every dispatch arm returns `feature_missing`
// without reading the call args — silence that noise.
#![cfg_attr(
    not(any(feature = "postgres", feature = "mysql", feature = "sqlite")),
    allow(unused_variables)
)]

use super::registry::active_spec;
pub use super::sql_compile::ListQueryParts;
use super::sql_compile::{list_query_from_parts as build_list_query, GroupAgg, ListQuery, SqlAgg};
use super::Adapter;

/// Only referenced when one of the SQL adapters is off.
#[cfg(any(
    not(feature = "postgres"),
    not(feature = "mysql"),
    not(feature = "sqlite")
))]
fn feature_missing(adapter: &str) -> String {
    format!(
        "SQL adapter `{adapter}` is not compiled into this soli binary. \
         Rebuild with `--features {adapter}` (or `sql` for every SQL backend). \
         Example: cargo install --path . --locked --no-default-features \
         --features embedding,llm,codegraph,{adapter}"
    )
}

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

pub fn is_sqlite() -> bool {
    active_spec()
        .map(|s| s.adapter == Adapter::Sqlite)
        .unwrap_or(false)
}

/// Dispatch a SQL op to the active backend, or a clear missing-feature error.
macro_rules! route_sql {
    ($pg:expr, $my:expr, $lite:expr) => {{
        match active_spec()?.adapter {
            Adapter::Mysql => {
                #[cfg(feature = "mysql")]
                {
                    $my
                }
                #[cfg(not(feature = "mysql"))]
                {
                    Err(feature_missing("mysql"))
                }
            }
            Adapter::Postgres => {
                #[cfg(feature = "postgres")]
                {
                    $pg
                }
                #[cfg(not(feature = "postgres"))]
                {
                    Err(feature_missing("postgres"))
                }
            }
            Adapter::Sqlite => {
                #[cfg(feature = "sqlite")]
                {
                    $lite
                }
                #[cfg(not(feature = "sqlite"))]
                {
                    Err(feature_missing("sqlite"))
                }
            }
            Adapter::Solidb => {
                Err("SQL facade used on a solidb connection (internal error; report this)".into())
            }
        }
    }};
}

pub fn ensure_connected() -> Result<(), String> {
    route_sql!(
        super::postgres::ensure_connected(),
        super::mysql::ensure_connected(),
        super::sqlite::ensure_connected()
    )
}

/// True when this thread holds an open SQL transaction (Postgres or MySQL).
pub fn has_active_tx() -> bool {
    #[cfg(feature = "postgres")]
    if super::postgres::has_active_tx() {
        return true;
    }
    #[cfg(feature = "mysql")]
    if super::mysql::has_active_tx() {
        return true;
    }
    #[cfg(feature = "sqlite")]
    if super::sqlite::has_active_tx() {
        return true;
    }
    false
}

pub fn begin_transaction(isolation_level: Option<&str>) -> Result<String, String> {
    route_sql!(
        super::postgres::begin_transaction(isolation_level),
        super::mysql::begin_transaction(isolation_level),
        super::sqlite::begin_transaction(isolation_level)
    )
}

pub fn commit_transaction() -> Result<(), String> {
    // Route by whichever adapter HOLDS the tx, not the active spec — the tx
    // may live on a named connection while the ambient default is something
    // else entirely (even solidb).
    #[cfg(feature = "postgres")]
    if super::postgres::has_active_tx() {
        return super::postgres::commit_transaction();
    }
    #[cfg(feature = "mysql")]
    if super::mysql::has_active_tx() {
        return super::mysql::commit_transaction();
    }
    #[cfg(feature = "sqlite")]
    if super::sqlite::has_active_tx() {
        return super::sqlite::commit_transaction();
    }
    Err("No active SQL transaction".into())
}

pub fn rollback_transaction() -> Result<(), String> {
    // Holder-routed like commit; no-op success when nothing is open
    // (mirrors defensive rollback paths).
    #[cfg(feature = "postgres")]
    if super::postgres::has_active_tx() {
        return super::postgres::rollback_transaction();
    }
    #[cfg(feature = "mysql")]
    if super::mysql::has_active_tx() {
        return super::mysql::rollback_transaction();
    }
    #[cfg(feature = "sqlite")]
    if super::sqlite::has_active_tx() {
        return super::sqlite::rollback_transaction();
    }
    Ok(())
}

pub fn clear_transaction() {
    #[cfg(feature = "postgres")]
    super::postgres::clear_transaction();
    #[cfg(feature = "mysql")]
    super::mysql::clear_transaction();
    #[cfg(feature = "sqlite")]
    super::sqlite::clear_transaction();
}

/// Atomically claim up to `batch` due jobs for the Soli job engine.
pub fn claim_jobs(
    now_iso: &str,
    worker_id: &str,
    locked_until_iso: &str,
    batch: usize,
) -> Result<Vec<serde_json::Value>, String> {
    route_sql!(
        super::postgres::claim_jobs(now_iso, worker_id, locked_until_iso, batch),
        super::mysql::claim_jobs(now_iso, worker_id, locked_until_iso, batch),
        super::sqlite::claim_jobs(now_iso, worker_id, locked_until_iso, batch)
    )
}

/// CAS a `_cron_jobs` slot on its stored `next_run_at`; true = this process won.
pub fn claim_cron_slot(
    key: &str,
    expected_next_run_at: &str,
    patch: serde_json::Value,
) -> Result<bool, String> {
    route_sql!(
        super::postgres::claim_cron_slot(key, expected_next_run_at, patch),
        super::mysql::claim_cron_slot(key, expected_next_run_at, patch),
        super::sqlite::claim_cron_slot(key, expected_next_run_at, patch)
    )
}

pub fn insert(
    table: &str,
    key: Option<&str>,
    document: serde_json::Value,
) -> Result<serde_json::Value, String> {
    route_sql!(
        super::postgres::insert(table, key, document),
        super::mysql::insert(table, key, document),
        super::sqlite::insert(table, key, document)
    )
}

pub fn get(table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    route_sql!(
        super::postgres::get(table, key),
        super::mysql::get(table, key),
        super::sqlite::get(table, key)
    )
}

pub fn update(
    table: &str,
    key: &str,
    document: serde_json::Value,
    merge: bool,
) -> Result<serde_json::Value, String> {
    route_sql!(
        super::postgres::update(table, key, document, merge),
        super::mysql::update(table, key, document, merge),
        super::sqlite::update(table, key, document, merge)
    )
}

pub fn delete(table: &str, key: &str) -> Result<(), String> {
    route_sql!(
        super::postgres::delete(table, key),
        super::mysql::delete(table, key),
        super::sqlite::delete(table, key)
    )
}

pub fn select(q: &ListQuery) -> Result<Vec<serde_json::Value>, String> {
    route_sql!(
        super::postgres::select(q),
        super::mysql::select(q),
        super::sqlite::select(q)
    )
}

pub fn select_by_keys(table: &str, keys: &[String]) -> Result<Vec<serde_json::Value>, String> {
    route_sql!(
        super::postgres::select_by_keys(table, keys),
        super::mysql::select_by_keys(table, keys),
        super::sqlite::select_by_keys(table, keys)
    )
}

pub fn select_json_text_in(
    table: &str,
    field: &str,
    values: &[String],
) -> Result<Vec<serde_json::Value>, String> {
    route_sql!(
        super::postgres::select_json_text_in(table, field, values),
        super::mysql::select_json_text_in(table, field, values),
        super::sqlite::select_json_text_in(table, field, values)
    )
}

pub fn group_by(
    q: &ListQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    route_sql!(
        super::postgres::group_by(q, group_fields, aggs),
        super::mysql::group_by(q, group_fields, aggs),
        super::sqlite::group_by(q, group_fields, aggs)
    )
}

pub fn count(q: &ListQuery) -> Result<i64, String> {
    route_sql!(
        super::postgres::count(q),
        super::mysql::count(q),
        super::sqlite::count(q)
    )
}

pub fn exists(q: &ListQuery) -> Result<bool, String> {
    route_sql!(
        super::postgres::exists(q),
        super::mysql::exists(q),
        super::sqlite::exists(q)
    )
}

pub fn aggregate(q: &ListQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    route_sql!(
        super::postgres::aggregate(q, func, field),
        super::mysql::aggregate(q, func, field),
        super::sqlite::aggregate(q, func, field)
    )
}

pub fn delete_all(q: &ListQuery) -> Result<u64, String> {
    route_sql!(
        super::postgres::delete_all(q),
        super::mysql::delete_all(q),
        super::sqlite::delete_all(q)
    )
}

pub fn update_all(q: &ListQuery, patch: serde_json::Value) -> Result<u64, String> {
    route_sql!(
        super::postgres::update_all(q, patch),
        super::mysql::update_all(q, patch),
        super::sqlite::update_all(q, patch)
    )
}

pub fn ensure_table(table: &str) -> Result<(), String> {
    route_sql!(
        super::postgres::ensure_table(table),
        super::mysql::ensure_table(table),
        super::sqlite::ensure_table(table)
    )
}

pub fn drop_table(table: &str) -> Result<(), String> {
    route_sql!(
        super::postgres::drop_table(table),
        super::mysql::drop_table(table),
        super::sqlite::drop_table(table)
    )
}

pub fn ensure_migrations_table() -> Result<(), String> {
    route_sql!(
        super::postgres::ensure_migrations_table(),
        super::mysql::ensure_migrations_table(),
        super::sqlite::ensure_migrations_table()
    )
}

pub fn list_applied_migrations() -> Result<Vec<(String, String)>, String> {
    route_sql!(
        super::postgres::list_applied_migrations(),
        super::mysql::list_applied_migrations(),
        super::sqlite::list_applied_migrations()
    )
}

pub fn record_migration(version: &str, name: &str) -> Result<(), String> {
    route_sql!(
        super::postgres::record_migration(version, name),
        super::mysql::record_migration(version, name),
        super::sqlite::record_migration(version, name)
    )
}

pub fn remove_migration(version: &str) -> Result<(), String> {
    route_sql!(
        super::postgres::remove_migration(version),
        super::mysql::remove_migration(version),
        super::sqlite::remove_migration(version)
    )
}

pub fn list_query_from_parts(parts: ListQueryParts) -> Result<ListQuery, String> {
    use super::sql_compile::Dialect;
    let dialect = match active_spec()?.adapter {
        Adapter::Mysql => Dialect::Mysql,
        Adapter::Postgres => Dialect::Postgres,
        Adapter::Sqlite => Dialect::Sqlite,
        Adapter::Solidb => {
            return Err("list_query_from_parts on solidb connection".into());
        }
    };
    // Validation only — no live DB; works even if the matching feature is off
    // so unit tests and error paths stay consistent. Real I/O fails at route_sql.
    build_list_query(parts, dialect)
}
