//! PostgreSQL document backend (Phase 1).
//!
//! Each model collection is a table with `_key TEXT PRIMARY KEY` and
//! `doc JSONB` holding the full Soli document (including system fields).

use super::registry::{active_connection_name, active_spec};
use super::sql_compile::{
    compile_aggregate_d, compile_count_d, compile_delete_all_d, compile_exists_d,
    compile_group_by_d, compile_select_by_keys_d, compile_select_d, compile_select_json_text_in_d,
    compile_update_all_d, create_table_sql_d, drop_table_sql_d, migrations_table_sql_d, Dialect,
    GroupAgg, ListQuery, SqlAgg, SqlBind,
};
use postgres::types::{ToSql, Type};
use r2d2::Pool;
use r2d2_postgres::{postgres::NoTls, PostgresConnectionManager};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type PgPool = Pool<PostgresConnectionManager<NoTls>>;
type PgPooled = r2d2::PooledConnection<PostgresConnectionManager<NoTls>>;

static POOLS: OnceLock<Mutex<HashMap<String, PgPool>>> = OnceLock::new();

/// Open SQL transaction on this thread: holds one pool connection until
/// commit/rollback. Nested `Model.transaction` increments `nest` only.
struct TxState {
    conn: PgPooled,
    nest: u32,
    /// Connection name the tx was begun on. Ops on OTHER postgres connections
    /// must not reuse this connection — that would execute them on the wrong
    /// database (silent cross-database write).
    name: String,
}

thread_local! {
    static TX: RefCell<Option<TxState>> = const { RefCell::new(None) };
    /// Fast flag so `has_active_tx` never needs to borrow `TX` (avoids RefCell
    /// conflicts while `with_conn` temporarily takes the connection).
    static TX_ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// Re-entrancy for `with_conn` (e.g. `insert` → `ensure_table` →
    /// `with_conn`): the live client plus the connection name it belongs to,
    /// so a nested op on a DIFFERENT named connection never borrows it.
    static ACTIVE_CLIENT: RefCell<Option<(*mut postgres::Client, String)>> =
        const { RefCell::new(None) };
}

/// Panic-safe reset for `ACTIVE_CLIENT`: if `f` unwinds, the pointer must not
/// dangle into the next `with_conn` on this (reused worker) thread.
struct ActiveClientGuard;

impl ActiveClientGuard {
    fn set(client: &mut postgres::Client, name: String) -> Self {
        ACTIVE_CLIENT.with(|c| *c.borrow_mut() = Some((client as *mut postgres::Client, name)));
        ActiveClientGuard
    }
}

impl Drop for ActiveClientGuard {
    fn drop(&mut self) {
        ACTIVE_CLIENT.with(|c| *c.borrow_mut() = None);
    }
}

fn pools() -> &'static Mutex<HashMap<String, PgPool>> {
    POOLS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pool_for_active() -> Result<PgPool, String> {
    let name = active_connection_name();
    let spec = active_spec()?;
    if spec.adapter != super::Adapter::Postgres {
        return Err(format!(
            "connection {:?} is {}, not postgres",
            name,
            spec.adapter.as_str()
        ));
    }
    let url = spec
        .url
        .clone()
        .ok_or_else(|| format!("connection {name:?}: url required for postgres"))?;
    let mut map = pools().lock().unwrap();
    if let Some(p) = map.get(&name) {
        return Ok(p.clone());
    }
    let manager = PostgresConnectionManager::new(
        url.parse::<postgres::Config>()
            .map_err(|e| format!("invalid DATABASE_URL ({name}): {e}"))?,
        NoTls,
    );
    let max = spec.pool_size.unwrap_or(10).max(1);
    let pool = Pool::builder()
        .max_size(max as u32)
        .connection_timeout(Duration::from_secs(10))
        .build(manager)
        .map_err(|e| format!("postgres pool ({name}): {e}"))?;
    map.insert(name, pool.clone());
    Ok(pool)
}

pub fn ensure_connected() -> Result<(), String> {
    let _ = pool_for_active()?;
    Ok(())
}

pub fn has_active_tx() -> bool {
    TX_ACTIVE.get()
}

fn map_isolation(level: Option<&str>) -> Result<&'static str, String> {
    match level
        .unwrap_or("read_committed")
        .to_ascii_lowercase()
        .as_str()
    {
        "read_committed" | "read committed" => Ok("READ COMMITTED"),
        "repeatable_read" | "repeatable read" => Ok("REPEATABLE READ"),
        "serializable" => Ok("SERIALIZABLE"),
        "read_uncommitted" | "read uncommitted" => Ok("READ UNCOMMITTED"),
        other => Err(format!(
            "unsupported isolation level {other:?} for postgres \
             (use read_committed, repeatable_read, serializable)"
        )),
    }
}

/// Begin a transaction on a checked-out pool connection (held until commit/rollback).
pub fn begin_transaction(isolation_level: Option<&str>) -> Result<String, String> {
    TX.with(|cell| {
        let mut slot = cell.borrow_mut();
        let name = active_connection_name();
        if let Some(tx) = slot.as_mut() {
            if tx.name != name {
                return Err(format!(
                    "a transaction is already open on connection {:?}; cannot begin one \
                     on {:?} from the same block (one SQL transaction per thread)",
                    tx.name, name
                ));
            }
            tx.nest += 1;
            return Ok(format!("sql-pg-nested-{}", tx.nest));
        }
        let iso = map_isolation(isolation_level)?;
        let pool = pool_for_active()?;
        let mut conn = pool
            .get()
            .map_err(|e| format!("postgres transaction checkout: {e}"))?;
        conn.batch_execute(&format!("BEGIN ISOLATION LEVEL {iso}"))
            .map_err(|e| format!("postgres BEGIN: {e}"))?;
        *slot = Some(TxState {
            conn,
            nest: 0,
            name,
        });
        TX_ACTIVE.set(true);
        Ok("sql-pg".into())
    })
}

pub fn commit_transaction() -> Result<(), String> {
    TX.with(|cell| {
        let mut slot = cell.borrow_mut();
        let tx = slot
            .as_mut()
            .ok_or_else(|| "No active postgres transaction".to_string())?;
        if tx.nest > 0 {
            tx.nest -= 1;
            return Ok(());
        }
        let mut state = slot.take().expect("tx present");
        TX_ACTIVE.set(false);
        state
            .conn
            .batch_execute("COMMIT")
            .map_err(|e| format!("postgres COMMIT: {e}"))?;
        Ok(())
    })
}

pub fn rollback_transaction() -> Result<(), String> {
    TX.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(tx) = slot.as_mut() else {
            return Ok(());
        };
        if tx.nest > 0 {
            tx.nest -= 1;
            return Ok(());
        }
        let mut state = slot.take().expect("tx present");
        TX_ACTIVE.set(false);
        state
            .conn
            .batch_execute("ROLLBACK")
            .map_err(|e| format!("postgres ROLLBACK: {e}"))
    })
}

/// Drop transaction state, rolling back if still open (defensive for worker reuse).
pub fn clear_transaction() {
    TX.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(mut state) = slot.take() {
            TX_ACTIVE.set(false);
            let _ = state.conn.batch_execute("ROLLBACK");
        }
    });
}

fn with_conn<T>(f: impl FnOnce(&mut postgres::Client) -> Result<T, String>) -> Result<T, String> {
    let name = active_connection_name();

    // Re-entrant (insert → ensure_table → with_conn) — reuse the outer client
    // only when the nested op targets the SAME named connection.
    let reentrant = ACTIVE_CLIENT.with(|c| match &*c.borrow() {
        Some((ptr, n)) if *n == name => Some(*ptr),
        _ => None,
    });
    if let Some(ptr) = reentrant {
        // SAFETY: pointer set only for the duration of an outer with_conn on
        // this thread; the guard clears it even on unwind.
        let client = unsafe { &mut *ptr };
        return f(client);
    }

    // Prefer the open transaction connection — but only for ops on the SAME
    // named connection: handing it to another connection's op would execute
    // that op on the wrong database. Take it out of the RefCell so nested
    // helpers (has_active_tx, begin nest) don't fight borrow_mut.
    struct RestoreTx(Option<TxState>);
    impl Drop for RestoreTx {
        fn drop(&mut self) {
            if let Some(state) = self.0.take() {
                TX.with(|c| *c.borrow_mut() = Some(state));
            }
        }
    }

    let tx_matches = TX.with(|c| c.borrow().as_ref().is_some_and(|t| t.name == name));
    if tx_matches {
        let mut restore = RestoreTx(TX.with(|c| c.borrow_mut().take()));
        if let Some(ref mut state) = restore.0 {
            let client: &mut postgres::Client = &mut state.conn;
            let _guard = ActiveClientGuard::set(client, name);
            return f(client);
        }
    }

    let pool = pool_for_active()?;
    let mut conn = pool.get().map_err(|e| format!("postgres checkout: {e}"))?;
    let client: &mut postgres::Client = &mut conn;
    let _guard = ActiveClientGuard::set(client, name);
    f(client)
}

// ---------- document CRUD ----------

pub fn insert(
    table: &str,
    key: Option<&str>,
    mut document: serde_json::Value,
) -> Result<serde_json::Value, String> {
    ensure_table(table)?;
    let key = resolve_key(key, &mut document)?;
    if let Some(obj) = document.as_object_mut() {
        obj.insert("_key".to_string(), serde_json::json!(key));
    }
    let table_q = Dialect::Postgres.quote_ident(table)?;
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES ($1, $2) \
         ON CONFLICT (_key) DO UPDATE SET doc = EXCLUDED.doc \
         RETURNING doc"
    );
    with_conn(|client| {
        let row = client
            .query_one(&sql, &[&key, &document])
            .map_err(|e| format!("postgres insert: {e}"))?;
        let doc: serde_json::Value = row.get(0);
        Ok(doc)
    })
}

pub fn get(table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    if !table_exists(table)? {
        return Ok(None);
    }
    let table_q = Dialect::Postgres.quote_ident(table)?;
    let sql = format!("SELECT doc FROM {table_q} WHERE _key = $1");
    with_conn(|client| {
        let rows = client
            .query(&sql, &[&key])
            .map_err(|e| format!("postgres get: {e}"))?;
        Ok(rows.first().map(|r| r.get(0)))
    })
}

/// Update a document. When `merge` is true (Model.update / soft_delete /
/// touch), patch fields into the existing JSONB with `doc || patch` so
/// unspecified keys are preserved. When false, replace the whole document.
pub fn update(
    table: &str,
    key: &str,
    mut document: serde_json::Value,
    merge: bool,
) -> Result<serde_json::Value, String> {
    ensure_table(table)?;
    if let Some(obj) = document.as_object_mut() {
        obj.insert("_key".to_string(), serde_json::json!(key));
    }
    let table_q = Dialect::Postgres.quote_ident(table)?;
    if merge {
        // JSONB `||` is right-biased: keys in the patch overwrite; others stay.
        // If the row is missing, fall back to a full insert of the patch.
        let sql = format!(
            "UPDATE {table_q} SET doc = COALESCE(doc, '{{}}'::jsonb) || $2::jsonb \
             WHERE _key = $1 RETURNING doc"
        );
        return with_conn(|client| {
            let rows = client
                .query(&sql, &[&key, &document])
                .map_err(|e| format!("postgres update (merge): {e}"))?;
            if let Some(row) = rows.first() {
                return Ok(row.get(0));
            }
            // No existing row — insert the patch as the full document.
            let insert_sql =
                format!("INSERT INTO {table_q} (_key, doc) VALUES ($1, $2) RETURNING doc");
            let row = client
                .query_one(&insert_sql, &[&key, &document])
                .map_err(|e| format!("postgres update (merge insert): {e}"))?;
            Ok(row.get(0))
        });
    }
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES ($1, $2) \
         ON CONFLICT (_key) DO UPDATE SET doc = EXCLUDED.doc \
         RETURNING doc"
    );
    with_conn(|client| {
        let row = client
            .query_one(&sql, &[&key, &document])
            .map_err(|e| format!("postgres update: {e}"))?;
        Ok(row.get(0))
    })
}

pub fn delete(table: &str, key: &str) -> Result<(), String> {
    if !table_exists(table)? {
        return Ok(());
    }
    let table_q = Dialect::Postgres.quote_ident(table)?;
    let sql = format!("DELETE FROM {table_q} WHERE _key = $1");
    with_conn(|client| {
        client
            .execute(&sql, &[&key])
            .map_err(|e| format!("postgres delete: {e}"))?;
        Ok(())
    })
}

pub fn select(q: &ListQuery) -> Result<Vec<serde_json::Value>, String> {
    if !table_exists(&q.table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_d(Dialect::Postgres, q)?;
    query_docs(&compiled.sql, &compiled.params)
}

/// Batch-fetch documents by primary key (includes belongs_to).
pub fn select_by_keys(table: &str, keys: &[String]) -> Result<Vec<serde_json::Value>, String> {
    if keys.is_empty() || !table_exists(table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_by_keys_d(Dialect::Postgres, table, keys)?;
    query_docs(&compiled.sql, &compiled.params)
}

/// Batch-fetch where a JSON text field is in `values` (includes has_many/has_one).
pub fn select_json_text_in(
    table: &str,
    field: &str,
    values: &[String],
) -> Result<Vec<serde_json::Value>, String> {
    if values.is_empty() || !table_exists(table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_json_text_in_d(Dialect::Postgres, table, field, values)?;
    query_docs(&compiled.sql, &compiled.params)
}

/// Multi-row GROUP BY returning plain objects keyed by group fields + aliases.
pub fn group_by(
    q: &ListQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    if !table_exists(&q.table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_group_by_d(Dialect::Postgres, q, group_fields, aggs)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let rows = client
            .query(&compiled.sql, &refs)
            .map_err(|e| format!("postgres group_by: {e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut map = serde_json::Map::new();
            let cols = row.columns();
            for (i, col) in cols.iter().enumerate() {
                let name = col.name();
                let v = pg_cell_to_json(&row, i);
                map.insert(name.to_string(), v);
            }
            out.push(serde_json::Value::Object(map));
        }
        Ok(out)
    })
}

fn pg_cell_to_json(row: &postgres::Row, i: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<_, i64>(i) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<_, f64>(i) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<_, Option<String>>(i) {
        return match v {
            Some(s) => serde_json::Value::String(s),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<_, Option<bool>>(i) {
        return match v {
            Some(b) => serde_json::json!(b),
            None => serde_json::Value::Null,
        };
    }
    serde_json::Value::Null
}

pub fn count(q: &ListQuery) -> Result<i64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_count_d(Dialect::Postgres, q)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let row = client
            .query_one(&compiled.sql, &refs)
            .map_err(|e| format!("postgres count: {e}"))?;
        let n: i64 = row.get(0);
        Ok(n)
    })
}

pub fn exists(q: &ListQuery) -> Result<bool, String> {
    if !table_exists(&q.table)? {
        return Ok(false);
    }
    let compiled = compile_exists_d(Dialect::Postgres, q)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let rows = client
            .query(&compiled.sql, &refs)
            .map_err(|e| format!("postgres exists: {e}"))?;
        Ok(!rows.is_empty())
    })
}

pub fn aggregate(q: &ListQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    if !table_exists(&q.table)? {
        return Ok(serde_json::Value::Null);
    }
    let compiled = compile_aggregate_d(Dialect::Postgres, q, func, field)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let row = client
            .query_one(&compiled.sql, &refs)
            .map_err(|e| format!("postgres aggregate: {e}"))?;
        // COUNT returns i64; SUM/AVG may be f64 or Decimal as string via float.
        if matches!(func, SqlAgg::Count) {
            let n: i64 = row.get(0);
            return Ok(serde_json::json!(n));
        }
        let v: Option<f64> = row.try_get(0).ok();
        Ok(v.map(|f| serde_json::json!(f))
            .unwrap_or(serde_json::Value::Null))
    })
}

pub fn delete_all(q: &ListQuery) -> Result<u64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_delete_all_d(Dialect::Postgres, q)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let n = client
            .execute(&compiled.sql, &refs)
            .map_err(|e| format!("postgres delete_all: {e}"))?;
        Ok(n)
    })
}

pub fn update_all(q: &ListQuery, patch: serde_json::Value) -> Result<u64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_update_all_d(Dialect::Postgres, q, &patch)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let n = client
            .execute(&compiled.sql, &refs)
            .map_err(|e| format!("postgres update_all: {e}"))?;
        Ok(n)
    })
}

fn query_docs(sql: &str, params: &[SqlBind]) -> Result<Vec<serde_json::Value>, String> {
    with_conn(|client| {
        let owned = bind_owned(params);
        let refs = bind_refs(&owned);
        let rows = client
            .query(sql, &refs)
            .map_err(|e| format!("postgres query: {e}"))?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    })
}

// ---------- schema / migrations ----------

pub fn ensure_table(table: &str) -> Result<(), String> {
    let ddl = create_table_sql_d(Dialect::Postgres, table)?;
    with_conn(|client| {
        client
            .batch_execute(&ddl)
            .map_err(|e| format!("postgres ensure_table: {e}"))
    })
}

pub fn drop_table(table: &str) -> Result<(), String> {
    let ddl = drop_table_sql_d(Dialect::Postgres, table)?;
    with_conn(|client| {
        client
            .batch_execute(&ddl)
            .map_err(|e| format!("postgres drop_table: {e}"))
    })
}

pub fn ensure_migrations_table() -> Result<(), String> {
    with_conn(|client| {
        client
            .batch_execute(migrations_table_sql_d(Dialect::Postgres))
            .map_err(|e| format!("postgres migrations table: {e}"))
    })
}

pub fn list_applied_migrations() -> Result<Vec<(String, String)>, String> {
    ensure_migrations_table()?;
    with_conn(|client| {
        let rows = client
            .query(
                "SELECT version, name FROM _migrations ORDER BY version",
                &[],
            )
            .map_err(|e| format!("postgres list migrations: {e}"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect())
    })
}

pub fn record_migration(version: &str, name: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    with_conn(|client| {
        client
            .execute(
                "INSERT INTO _migrations (version, name) VALUES ($1, $2) \
                 ON CONFLICT (version) DO NOTHING",
                &[&version, &name],
            )
            .map_err(|e| format!("postgres record migration: {e}"))?;
        Ok(())
    })
}

pub fn remove_migration(version: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    with_conn(|client| {
        client
            .execute("DELETE FROM _migrations WHERE version = $1", &[&version])
            .map_err(|e| format!("postgres remove migration: {e}"))?;
        Ok(())
    })
}

/// Add `delta` to a numeric JSON field **in one statement**, returning the new
/// value.
///
/// The read-modify-write this replaces lost concurrent bumps: two requests both
/// read 5, both write 6. One `UPDATE` leaves the arithmetic to the database, so
/// the row's own lock serializes the increments.
pub fn increment_field(
    table: &str,
    key: &str,
    field: &str,
    delta: i64,
) -> Result<Option<i64>, String> {
    if !table_exists(table)? {
        return Ok(None);
    }
    // Validated as an identifier before it reaches a JSON path literal.
    Dialect::Postgres.quote_ident(field)?;
    let table_q = Dialect::Postgres.quote_ident(table)?;
    let sql = format!(
        "UPDATE {table_q} SET doc = jsonb_set(doc, '{{{field}}}', \
             to_jsonb(COALESCE((doc->>'{field}')::numeric, 0) + $1::bigint)) \
         WHERE _key = $2 RETURNING (doc->>'{field}')"
    );
    with_conn(|client| {
        let rows = client
            .query(&sql, &[&delta, &key])
            .map_err(|e| format!("postgres increment: {e}"))?;
        Ok(rows
            .first()
            .and_then(|r| r.get::<_, Option<String>>(0))
            .and_then(|text| super::parse_counter(&text)))
    })
}

/// Index names on `table`.
pub fn list_index_names(table: &str) -> Result<Vec<String>, String> {
    with_conn(|client| {
        let rows = client
            .query(
                "SELECT indexname FROM pg_indexes WHERE schemaname = ANY (current_schemas(false)) \
                 AND tablename = $1",
                &[&table],
            )
            .map_err(|e| format!("postgres list indexes: {e}"))?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    })
}

/// Create a JSON-field index on a document table if it is absent.
pub fn ensure_doc_index(
    table: &str,
    fields: &[String],
    name: &str,
    unique: bool,
) -> Result<bool, String> {
    if list_index_names(table)?.iter().any(|n| n == name) {
        return Ok(false);
    }
    for sql in super::ddl::doc_index_sql(Dialect::Postgres, table, fields, name, unique)? {
        execute_ddl(&sql)?;
    }
    Ok(true)
}

pub fn execute_ddl(sql: &str) -> Result<(), String> {
    with_conn(|client| {
        client
            .batch_execute(sql)
            .map_err(|e| format!("postgres ddl: {e}"))
    })
}

/// `db.execute`: a dedicated connection, dropped afterwards, so `SET ROLE` /
/// `SET search_path` cannot leak into the pool.
pub fn execute_raw(sql: &str) -> Result<(), String> {
    let spec = active_spec()?;
    let url = spec
        .url
        .as_deref()
        .ok_or_else(|| format!("connection {:?}: url required for postgres", spec.name))?;
    let mut client =
        postgres::Client::connect(url, NoTls).map_err(|e| format!("postgres execute: {e}"))?;
    client
        .batch_execute(sql)
        .map_err(|e| format!("postgres execute: {e}"))
}

// ---------- column-aware model execution ----------

use super::introspect::{ColType, TableSchema};
use super::sql_columns_compile as cols;

/// Read one row into a JSON object keyed by column name, so downstream
/// hydration is identical to the document path.
fn row_to_json(schema: &TableSchema, row: &postgres::Row) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    let mut idx = 0usize;
    for col in &schema.columns {
        // Unreadable columns are not selected, so they hold no position.
        if col.ty == ColType::Unknown {
            continue;
        }
        let value = match col.ty {
            // Numbers arrive as text (see ColType::reads_as_text) so the
            // driver never has to match an exact SQL width.
            ColType::Int => row
                .get::<_, Option<String>>(idx)
                .and_then(|s| s.trim().parse::<i64>().ok())
                .map(|n| serde_json::json!(n))
                .unwrap_or(serde_json::Value::Null),
            ColType::Float => row
                .get::<_, Option<String>>(idx)
                .and_then(|s| s.trim().parse::<f64>().ok())
                .map(|f| serde_json::json!(f))
                .unwrap_or(serde_json::Value::Null),
            ColType::Bool => row
                .get::<_, Option<bool>>(idx)
                .map(|b| serde_json::json!(b))
                .unwrap_or(serde_json::Value::Null),
            ColType::Json => row
                .get::<_, Option<serde_json::Value>>(idx)
                .unwrap_or(serde_json::Value::Null),
            // Text-carried types (timestamps, uuid, exact numerics) plus plain text.
            _ => row
                .get::<_, Option<String>>(idx)
                .map(|s| serde_json::json!(normalize_text(col.ty, &s)))
                .unwrap_or(serde_json::Value::Null),
        };
        out.insert(col.name.clone(), value);
        idx += 1;
    }
    // Mirror the key under `_key` when the table has no such column, so the
    // instance plumbing (dirty tracking, delete) keeps working unchanged.
    if !schema.has_column("_key") {
        if let Some(pk) = out.get(&schema.pk) {
            let key = match pk {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.insert("_key".to_string(), serde_json::json!(key));
        }
    }
    serde_json::Value::Object(out)
}

/// Postgres renders `timestamptz` as `2026-08-11 10:00:00+00`; re-emit the
/// RFC 3339 form Soli's DateTime parses.
fn normalize_text(ty: ColType, raw: &str) -> String {
    if ty != ColType::DateTime {
        return raw.to_string();
    }
    let mut out = raw.replacen(' ', "T", 1);
    if let Some(pos) = out.rfind('+') {
        // "+00" -> "+00:00" so the offset is well-formed.
        if out.len() - pos == 3 {
            out.push_str(":00");
        }
    }
    out
}

fn rows_to_json(schema: &TableSchema, rows: &[postgres::Row]) -> Vec<serde_json::Value> {
    rows.iter().map(|r| row_to_json(schema, r)).collect()
}

pub fn col_get(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let compiled = cols::compile_get_cols(Dialect::Postgres, schema, pk)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let rows = client
            .query(&compiled.sql, &refs)
            .map_err(|e| format!("postgres column get: {e}"))?;
        Ok(rows.first().map(|r| row_to_json(schema, r)))
    })
}

pub fn col_insert(
    schema: &std::sync::Arc<TableSchema>,
    doc: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_insert_cols(Dialect::Postgres, schema, doc)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let row = client
            .query_one(&compiled.sql, &refs)
            .map_err(|e| format!("postgres column insert: {e}"))?;
        Ok(row_to_json(schema, &row))
    })
}

pub fn col_update(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
    patch: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_update_cols(Dialect::Postgres, schema, pk, patch)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let rows = client
            .query(&compiled.sql, &refs)
            .map_err(|e| format!("postgres column update: {e}"))?;
        match rows.first() {
            Some(row) => Ok(row_to_json(schema, row)),
            None => Err(format!(
                "no row in {:?} with {} = {}",
                schema.table, schema.pk, pk
            )),
        }
    })
}

pub fn col_delete(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<(), String> {
    let compiled = cols::compile_delete_cols(Dialect::Postgres, schema, pk)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        client
            .execute(&compiled.sql, &refs)
            .map_err(|e| format!("postgres column delete: {e}"))?;
        Ok(())
    })
}

/// Add `delta` to a numeric **column** of one row, atomically.
///
/// Column mode owns no schema, so the column must already be numeric; a
/// non-numeric one is refused by name rather than by a driver error.
pub fn col_increment(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
    column: &str,
    delta: i64,
) -> Result<Option<i64>, String> {
    let (sql, params) = cols::compile_increment_col(Dialect::Postgres, schema, pk, column, delta)?;
    with_conn(|client| {
        let owned = bind_owned(&params);
        let refs = bind_refs(&owned);
        let rows = client
            .query(&sql, &refs)
            .map_err(|e| format!("postgres column increment: {e}"))?;
        Ok(rows
            .first()
            .and_then(|r| r.get::<_, Option<String>>(0))
            .and_then(|text| super::parse_counter(&text)))
    })
}

pub fn col_select(q: &cols::ColumnQuery) -> Result<Vec<serde_json::Value>, String> {
    let compiled = cols::compile_select_cols(Dialect::Postgres, q)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let rows = client
            .query(&compiled.sql, &refs)
            .map_err(|e| format!("postgres column select: {e}"))?;
        Ok(rows_to_json(&q.schema, &rows))
    })
}

pub fn col_count(q: &cols::ColumnQuery) -> Result<i64, String> {
    let compiled = cols::compile_count_cols(Dialect::Postgres, q)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let row = client
            .query_one(&compiled.sql, &refs)
            .map_err(|e| format!("postgres column count: {e}"))?;
        Ok(row.get(0))
    })
}

pub fn col_exists(q: &cols::ColumnQuery) -> Result<bool, String> {
    let compiled = cols::compile_exists_cols(Dialect::Postgres, q)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let rows = client
            .query(&compiled.sql, &refs)
            .map_err(|e| format!("postgres column exists: {e}"))?;
        Ok(!rows.is_empty())
    })
}

pub fn col_aggregate(
    q: &cols::ColumnQuery,
    func: SqlAgg,
    field: &str,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_aggregate_cols(Dialect::Postgres, q, func, field)?;
    with_conn(|client| {
        let owned = bind_owned(&compiled.params);
        let refs = bind_refs(&owned);
        let row = client
            .query_one(&compiled.sql, &refs)
            .map_err(|e| format!("postgres column aggregate: {e}"))?;
        if func == SqlAgg::Count {
            let n: i64 = row.get(0);
            return Ok(serde_json::json!(n));
        }
        let raw: Option<String> = row.get(0);
        Ok(super::columns::parse_agg_text(raw))
    })
}

// ---------- column-aware model introspection ----------

/// Read the shape of an existing table for column mode: columns in declaration
/// order (with type, nullability, and whether the database generates the value)
/// plus the primary-key columns in key order.
pub fn introspect_table(table: &str) -> Result<super::introspect::RawColumns, String> {
    with_conn(|client| {
        let column_rows = client
            .query(
                "SELECT column_name, udt_name, is_nullable, \
                        (column_default LIKE 'nextval(%' OR is_identity = 'YES') AS is_auto \
                 FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = $1 \
                 ORDER BY ordinal_position",
                &[&table],
            )
            .map_err(|e| format!("postgres introspect columns: {e}"))?;

        let mut columns = Vec::with_capacity(column_rows.len());
        for row in &column_rows {
            let name: String = row.get(0);
            let udt: String = row.get(1);
            let nullable: String = row.get(2);
            let is_auto: Option<bool> = row.get(3);
            columns.push((
                name,
                udt,
                String::new(),
                nullable.eq_ignore_ascii_case("YES"),
                is_auto.unwrap_or(false),
            ));
        }

        let pk_rows = client
            .query(
                "SELECT kcu.column_name \
                 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON kcu.constraint_name = tc.constraint_name \
                  AND kcu.table_schema = tc.table_schema \
                 WHERE tc.table_schema = current_schema() AND tc.table_name = $1 \
                   AND tc.constraint_type = 'PRIMARY KEY' \
                 ORDER BY kcu.ordinal_position",
                &[&table],
            )
            .map_err(|e| format!("postgres introspect primary key: {e}"))?;
        let pk = pk_rows.iter().map(|r| r.get::<_, String>(0)).collect();

        Ok(super::introspect::RawColumns { columns, pk })
    })
}

// ---------- Soli job engine ----------

/// Atomically claim up to `batch` due jobs from `_jobs` for the Soli job
/// engine. `FOR UPDATE SKIP LOCKED` keeps concurrent pollers (multi-process
/// deploys) from double-claiming; the lease-reclaim clause recovers jobs
/// whose worker died holding a lease. Timestamps are fixed-width ISO-8601
/// strings, so text comparison is chronological.
pub fn claim_jobs(
    now_iso: &str,
    worker_id: &str,
    locked_until_iso: &str,
    batch: usize,
) -> Result<Vec<serde_json::Value>, String> {
    if !table_exists("_jobs")? {
        return Ok(Vec::new());
    }
    let sql = "UPDATE _jobs SET doc = doc || jsonb_build_object(\
                   'state', 'running', 'locked_by', $1::text, 'locked_until', $2::text, \
                   'attempts', COALESCE((doc->>'attempts')::bigint, 0) + 1) \
               WHERE _key IN (\
                   SELECT _key FROM _jobs \
                   WHERE ((doc->>'state') IN ('pending','scheduled','failed') \
                          AND (doc->>'run_at') <= $3) \
                      OR ((doc->>'state') = 'running' AND (doc->>'locked_until') < $3) \
                   ORDER BY COALESCE((doc->>'priority')::bigint, 0) DESC, (doc->>'run_at') ASC \
                   LIMIT $4 FOR UPDATE SKIP LOCKED) \
               RETURNING doc";
    with_conn(|client| {
        let rows = client
            .query(
                sql,
                &[&worker_id, &locked_until_iso, &now_iso, &(batch as i64)],
            )
            .map_err(|e| format!("postgres claim_jobs: {e}"))?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    })
}

/// Compare-and-swap a cron slot: merge `patch` into the `_cron_jobs` row only
/// when its stored `next_run_at` still equals `expected_next_run_at`. Returns
/// true when this process won the slot (exactly one winner across processes).
pub fn claim_cron_slot(
    key: &str,
    expected_next_run_at: &str,
    patch: serde_json::Value,
) -> Result<bool, String> {
    if !table_exists("_cron_jobs")? {
        return Ok(false);
    }
    with_conn(|client| {
        let n = client
            .execute(
                "UPDATE _cron_jobs SET doc = doc || $1 \
                 WHERE _key = $2 AND (doc->>'next_run_at') = $3",
                &[&patch, &key, &expected_next_run_at],
            )
            .map_err(|e| format!("postgres claim_cron_slot: {e}"))?;
        Ok(n == 1)
    })
}

// ---------- helpers ----------

fn table_exists(table: &str) -> Result<bool, String> {
    with_conn(|client| {
        let row = client
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM information_schema.tables
                    WHERE table_schema = 'public' AND table_name = $1
                )",
                &[&table],
            )
            .map_err(|e| format!("postgres table_exists: {e}"))?;
        Ok(row.get(0))
    })
}

fn resolve_key(key: Option<&str>, document: &mut serde_json::Value) -> Result<String, String> {
    if let Some(k) = key {
        return Ok(k.to_string());
    }
    if let Some(k) = document.get("_key").and_then(|v| v.as_str()) {
        return Ok(k.to_string());
    }
    if let Some(k) = document.get("id").and_then(|v| v.as_str()) {
        return Ok(k.to_string());
    }
    Ok(uuid::Uuid::new_v4().to_string())
}

/// Owned bind values so we can build `&dyn ToSql` slices.
#[derive(Debug)]
enum OwnedParam {
    I64(i64),
    F64(f64),
    Bool(bool),
    Text(String),
    Json(serde_json::Value),
}

impl ToSql for OwnedParam {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            OwnedParam::I64(n) => (*n).to_sql(ty, out),
            OwnedParam::F64(f) => (*f).to_sql(ty, out),
            OwnedParam::Bool(b) => (*b).to_sql(ty, out),
            OwnedParam::Text(s) => s.to_sql(ty, out),
            OwnedParam::Json(j) => j.to_sql(ty, out),
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(
            *ty,
            Type::BOOL
                | Type::INT2
                | Type::INT4
                | Type::INT8
                | Type::FLOAT4
                | Type::FLOAT8
                | Type::NUMERIC
                | Type::TEXT
                | Type::VARCHAR
                | Type::BPCHAR
                | Type::JSON
                | Type::JSONB
                | Type::UNKNOWN
        ) || <serde_json::Value as ToSql>::accepts(ty)
    }

    postgres::types::to_sql_checked!();
}

fn bind_owned(params: &[SqlBind]) -> Vec<OwnedParam> {
    params
        .iter()
        .map(|v| match v {
            // JSON null binds as 'null'::jsonb, NOT SQL NULL — `(doc->'f') = NULL`
            // is never true, which would make {f: null} filters match nothing.
            SqlBind::Json(j) => OwnedParam::Json(j.clone()),
            SqlBind::I64(n) => OwnedParam::I64(*n),
            SqlBind::F64(f) => OwnedParam::F64(*f),
            SqlBind::Bool(b) => OwnedParam::Bool(*b),
            SqlBind::Text(s) => OwnedParam::Text(s.clone()),
        })
        .collect()
}

fn bind_refs(owned: &[OwnedParam]) -> Vec<&(dyn ToSql + Sync)> {
    owned.iter().map(|p| p as &(dyn ToSql + Sync)).collect()
}

#[cfg(test)]
mod integration_tests {
    use super::super::sql_compile::SoftDeleteMode;
    use super::*;
    use std::collections::BTreeMap;

    fn with_pg(f: impl FnOnce()) {
        with_pg_conns(&["primary"], f)
    }

    /// Like [`with_pg`], but registers every name in `names` as a postgres
    /// connection on the same URL (first name is the default). Lets tests
    /// exercise multi-connection routing against a single server.
    fn with_pg_conns(names: &[&str], f: impl FnOnce()) {
        // Cross-module lock: the registry override is process-global, so all
        // override-installing test modules must serialize on the same mutex.
        let _g = crate::db::registry::registry_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let url = std::env::var("PG_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()
            .filter(|u| u.starts_with("postgres"))
            .unwrap_or_else(|| "postgres://soli@localhost:5432/soli_test".into());
        if postgres::Client::connect(&url, NoTls).is_err() {
            eprintln!("skip: postgres not reachable at {url}");
            return;
        }
        use crate::db::registry::{
            clear_registry_override, set_registry_for_tests, ConnectionRegistry, ConnectionSpec,
        };
        use crate::db::Adapter;
        use std::collections::HashMap;
        let mut connections = HashMap::new();
        for name in names {
            connections.insert(
                (*name).to_string(),
                ConnectionSpec {
                    name: (*name).to_string(),
                    adapter: Adapter::Postgres,
                    url: Some(url.clone()),
                    solidb_host: None,
                    solidb_database: None,
                    solidb_username: None,
                    solidb_password: None,
                    solidb_api_key: None,
                    pool_size: Some(5),
                },
            );
        }
        set_registry_for_tests(ConnectionRegistry {
            default: names[0].to_string(),
            connections,
            from_file: false,
        });
        // Clear on unwind too — a panicking test must not leak its override
        // into whichever test takes the (poison-recovered) lock next.
        struct ClearOnDrop;
        impl Drop for ClearOnDrop {
            fn drop(&mut self) {
                clear_registry_override();
            }
        }
        let _clear = ClearOnDrop;
        f();
    }

    /// The Postgres twin of the SQLite concurrency test: one statement per bump,
    /// so parallel increments cannot overwrite each other.
    #[test]
    fn concurrent_increments_do_not_lose_counts_when_pg_available() {
        const THREADS: i64 = 8;
        const PER_THREAD: i64 = 25;

        with_pg(|| {
            if ensure_connected().is_err() {
                return;
            }
            let table = "soli_pg_increment_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }
            insert(table, Some("hits"), serde_json::json!({ "views": 0 })).expect("seed");

            use crate::interpreter::builtins::model::crud::cas_field_delta;
            std::thread::scope(|scope| {
                for _ in 0..THREADS {
                    scope.spawn(|| {
                        for _ in 0..PER_THREAD {
                            cas_field_delta(table, "hits", "views", 1).expect("increment");
                        }
                    });
                }
            });

            let doc = get(table, "hits").unwrap().expect("row");
            assert_eq!(doc["views"].as_i64().unwrap(), THREADS * PER_THREAD);

            // A missing field starts at 0; a decrement is a negative delta.
            assert_eq!(increment_field(table, "hits", "other", 2).unwrap(), Some(2));
            assert_eq!(
                increment_field(table, "hits", "other", -5).unwrap(),
                Some(-3)
            );
            assert_eq!(increment_field(table, "nope", "views", 1).unwrap(), None);
            let _ = drop_table(table);
        });
    }

    #[test]
    fn crud_roundtrip_when_pg_available() {
        with_pg(|| {
            if ensure_connected().is_err() {
                eprintln!("skip: pool init failed (pool may already be solidb from other tests)");
                return;
            }
            let table = "soli_pg_crud_test";
            let _ = drop_table(table);
            if let Err(e) = ensure_table(table) {
                eprintln!("skip: ensure_table failed: {e}");
                return;
            }
            let doc = serde_json::json!({
                "_key": "k1",
                "name": "Ada",
                "status": "up"
            });
            let inserted = insert(table, Some("k1"), doc).expect("insert");
            assert_eq!(inserted["name"], "Ada");
            let got = get(table, "k1").expect("get").expect("row");
            assert_eq!(got["status"], "up");
            let updated = update(
                table,
                "k1",
                serde_json::json!({ "_key": "k1", "name": "Ada", "status": "late" }),
                false,
            )
            .expect("update");
            assert_eq!(updated["status"], "late");
            let mut eq = BTreeMap::new();
            eq.insert("status".into(), serde_json::json!("late"));
            let q = ListQuery {
                table: table.into(),
                eq_filters: eq,
                filter_sdbql: Some("doc.status == @status".into()),
                soft_delete: SoftDeleteMode::Default,
                is_soft_delete_model: false,
                order_field: None,
                order_desc: false,
                limit: Some(10),
                offset: None,
            };
            let rows = select(&q).expect("select");
            assert_eq!(rows.len(), 1);
            assert_eq!(count(&q).unwrap(), 1);
            assert!(exists(&q).unwrap());
            delete(table, "k1").expect("delete");
            assert!(get(table, "k1").unwrap().is_none());
            let _ = drop_table(table);
        });
    }

    /// `where({field: null})` must match documents whose field is JSON null.
    /// Binding SQL NULL would compile to `(doc->'f') = NULL`, which is never
    /// true — the filter would silently return 0 rows.
    #[test]
    fn null_filter_matches_json_null() {
        with_pg(|| {
            if ensure_connected().is_err() {
                return;
            }
            let table = "soli_pg_null_filter_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }
            insert(
                table,
                Some("t1"),
                serde_json::json!({ "_key": "t1", "assignee_id": null }),
            )
            .expect("insert");
            insert(
                table,
                Some("t2"),
                serde_json::json!({ "_key": "t2", "assignee_id": "u9" }),
            )
            .expect("insert");
            let mut eq = BTreeMap::new();
            eq.insert("assignee_id".into(), serde_json::Value::Null);
            let q = ListQuery {
                table: table.into(),
                eq_filters: eq,
                filter_sdbql: Some("doc.assignee_id == @assignee_id".into()),
                soft_delete: SoftDeleteMode::Default,
                is_soft_delete_model: false,
                order_field: None,
                order_desc: false,
                limit: None,
                offset: None,
            };
            let rows = select(&q).expect("select");
            assert_eq!(rows.len(), 1, "null filter must match the JSON-null row");
            assert_eq!(rows[0]["_key"], "t1");
            assert_eq!(count(&q).unwrap(), 1);
            let _ = drop_table(table);
        });
    }

    /// Model.update / soft_delete pass merge=true with a partial payload.
    /// Replacing the whole JSONB would wipe sibling fields — this must merge.
    #[test]
    fn partial_merge_preserves_sibling_fields() {
        with_pg(|| {
            if ensure_connected().is_err() {
                return;
            }
            let table = "soli_pg_merge_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }
            insert(
                table,
                Some("u1"),
                serde_json::json!({
                    "_key": "u1",
                    "name": "Ada",
                    "status": "up",
                    "age": 36
                }),
            )
            .expect("insert");
            // Partial update — only status (same shape as Model.update(id, {status: …}))
            let patched = update(table, "u1", serde_json::json!({ "status": "late" }), true)
                .expect("merge update");
            assert_eq!(patched["status"], "late", "status should change");
            assert_eq!(patched["name"], "Ada", "name must survive merge");
            assert_eq!(patched["age"], 36, "age must survive merge");
            // soft_delete-shaped patch: only deleted_at
            let soft = update(
                table,
                "u1",
                serde_json::json!({ "deleted_at": "2026-08-10T00:00:00Z" }),
                true,
            )
            .expect("soft delete patch");
            assert_eq!(soft["deleted_at"], "2026-08-10T00:00:00Z");
            assert_eq!(soft["name"], "Ada");
            assert_eq!(soft["status"], "late");
            // restore-shaped: set deleted_at to null via merge
            let restored = update(
                table,
                "u1",
                serde_json::json!({ "deleted_at": serde_json::Value::Null }),
                true,
            )
            .expect("restore");
            assert!(restored
                .get("deleted_at")
                .map(|v| v.is_null())
                .unwrap_or(false));
            assert_eq!(restored["name"], "Ada");
            let _ = drop_table(table);
        });
    }

    #[test]
    fn sum_aggregate_when_pg_available() {
        with_pg(|| {
            if ensure_connected().is_err() {
                return;
            }
            let table = "soli_pg_agg_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }
            insert(
                table,
                Some("a"),
                serde_json::json!({"_key": "a", "amount": 10}),
            )
            .unwrap();
            insert(
                table,
                Some("b"),
                serde_json::json!({"_key": "b", "amount": 5}),
            )
            .unwrap();
            let q = ListQuery {
                table: table.into(),
                eq_filters: BTreeMap::new(),
                filter_sdbql: None,
                soft_delete: SoftDeleteMode::Default,
                is_soft_delete_model: false,
                order_field: None,
                order_desc: false,
                limit: None,
                offset: None,
            };
            let sum = aggregate(&q, SqlAgg::Sum, "amount").expect("sum");
            assert_eq!(sum.as_f64().unwrap_or(0.0), 15.0);
            let cnt = aggregate(&q, SqlAgg::Count, "").expect("count");
            assert_eq!(cnt.as_i64().unwrap_or(0), 2);
            let _ = drop_table(table);
        });
    }

    #[test]
    fn migrations_table_roundtrip_when_pg_available() {
        with_pg(|| {
            if ensure_connected().is_err() {
                return;
            }
            if ensure_migrations_table().is_err() {
                return;
            }
            let _ = remove_migration("20990101000000");
            record_migration("20990101000000", "test_mig").expect("record");
            let applied = list_applied_migrations().expect("list");
            assert!(applied.iter().any(|(v, _)| v == "20990101000000"));
            remove_migration("20990101000000").expect("remove");
        });
    }

    #[test]
    fn transaction_commit_and_rollback_when_pg_available() {
        with_pg(|| {
            if ensure_connected().is_err() {
                return;
            }
            clear_transaction();
            let table = "soli_pg_tx_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }

            // Commit path: row visible after commit.
            begin_transaction(None).expect("begin");
            assert!(has_active_tx());
            insert(
                table,
                Some("commit-me"),
                serde_json::json!({"_key": "commit-me", "n": 1}),
            )
            .expect("insert in tx");
            commit_transaction().expect("commit");
            assert!(!has_active_tx());
            assert!(get(table, "commit-me").unwrap().is_some());

            // Rollback path: row never lands.
            begin_transaction(None).expect("begin2");
            insert(
                table,
                Some("rollback-me"),
                serde_json::json!({"_key": "rollback-me", "n": 2}),
            )
            .expect("insert in tx2");
            rollback_transaction().expect("rollback");
            assert!(!has_active_tx());
            assert!(get(table, "rollback-me").unwrap().is_none());

            let _ = drop_table(table);
        });
    }

    /// A transaction on one named connection must NOT capture operations on a
    /// different connection of the same adapter — those would execute on the
    /// wrong database. Both names point at the same server here; routing
    /// correctness is still observable: if the "secondary" write wrongly
    /// joined the primary tx, the rollback would erase it.
    #[test]
    fn transaction_does_not_capture_other_connections() {
        with_pg_conns(&["primary", "secondary"], || {
            if ensure_connected().is_err() {
                return;
            }
            clear_transaction();
            let table = "soli_pg_tx_route_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }

            begin_transaction(None).expect("begin on primary");

            // Same-connection write joins the tx.
            insert(table, Some("inside"), serde_json::json!({"_key": "inside"}))
                .expect("insert inside tx");

            // Other-connection write must run on its own pool connection.
            crate::db::with_connection("secondary", || {
                insert(
                    table,
                    Some("outside"),
                    serde_json::json!({"_key": "outside"}),
                )
                .expect("insert outside tx")
            });

            // A nested begin on a different connection is refused, not joined.
            let err = crate::db::with_connection("secondary", || begin_transaction(None))
                .expect_err("cross-connection begin must fail");
            assert!(err.contains("already open"), "{err}");

            rollback_transaction().expect("rollback");
            assert!(
                get(table, "inside").unwrap().is_none(),
                "tx write must roll back"
            );
            assert!(
                get(table, "outside").unwrap().is_some(),
                "other-connection write must persist through the rollback"
            );

            let _ = drop_table(table);
        });
    }

    #[test]
    fn group_by_and_batch_keys_when_pg_available() {
        with_pg(|| {
            if ensure_connected().is_err() {
                return;
            }
            let table = "soli_pg_group_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }
            insert(
                table,
                Some("1"),
                serde_json::json!({"_key": "1", "country": "FR", "amount": 10}),
            )
            .unwrap();
            insert(
                table,
                Some("2"),
                serde_json::json!({"_key": "2", "country": "FR", "amount": 5}),
            )
            .unwrap();
            insert(
                table,
                Some("3"),
                serde_json::json!({"_key": "3", "country": "US", "amount": 7}),
            )
            .unwrap();
            let q = ListQuery {
                table: table.into(),
                eq_filters: BTreeMap::new(),
                filter_sdbql: None,
                soft_delete: SoftDeleteMode::Default,
                is_soft_delete_model: false,
                order_field: Some("country".into()),
                order_desc: false,
                limit: None,
                offset: None,
            };
            let aggs = vec![GroupAgg {
                alias: "total".into(),
                func: SqlAgg::Sum,
                field: "amount".into(),
            }];
            let rows = group_by(&q, &["country".into()], &aggs).expect("group_by");
            assert_eq!(rows.len(), 2);
            let by_keys = select_by_keys(table, &["1".into(), "3".into()]).expect("keys");
            assert_eq!(by_keys.len(), 2);
            let kids = select_json_text_in(table, "country", &["FR".into()]).expect("in");
            assert_eq!(kids.len(), 2);
            let _ = drop_table(table);
        });
    }
}

/// Integration tests for column-aware models: a real table with real columns,
/// exercised end to end. Skips when Postgres is unreachable.
#[cfg(test)]
mod column_integration_tests {
    use super::*;
    use crate::db::introspect::{clear_schema_cache, ColType};
    use crate::db::registry::{
        clear_registry_override, registry_test_lock, set_registry_for_tests, ConnectionRegistry,
        ConnectionSpec,
    };
    use crate::db::sql_columns_compile::ColumnQuery;
    use std::collections::HashMap;
    use std::sync::Arc;

    const TABLE: &str = "soli_col_orders";

    fn with_pg(f: impl FnOnce()) {
        let _g = registry_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let url = std::env::var("PG_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()
            .filter(|u| u.starts_with("postgres"))
            .unwrap_or_else(|| "postgres://soli@localhost:5432/soli_test".into());
        let mut connections = HashMap::new();
        connections.insert(
            "primary".into(),
            ConnectionSpec {
                name: "primary".into(),
                adapter: crate::db::Adapter::Postgres,
                url: Some(url),
                solidb_host: None,
                solidb_database: None,
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: Some(5),
            },
        );
        set_registry_for_tests(ConnectionRegistry {
            default: "primary".into(),
            connections,
            from_file: false,
        });
        struct ClearOnDrop;
        impl Drop for ClearOnDrop {
            fn drop(&mut self) {
                clear_registry_override();
                // Schemas are cached per (connection, table); the next test
                // must not answer from this test's table.
                clear_schema_cache();
            }
        }
        let _clear = ClearOnDrop;
        clear_schema_cache();
        f();
    }

    /// Create a table with the column types a real legacy schema mixes.
    fn setup_table() -> bool {
        if ensure_connected().is_err() {
            eprintln!("skip: postgres not reachable");
            return false;
        }
        let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {TABLE}"));
        execute_ddl(&format!(
            "CREATE TABLE {TABLE} (
                 id BIGSERIAL PRIMARY KEY,
                 name TEXT NOT NULL,
                 qty INT,
                 price NUMERIC(10,2),
                 active BOOLEAN,
                 meta JSONB,
                 created_at TIMESTAMPTZ,
                 updated_at TIMESTAMPTZ
             )"
        ))
        .expect("create table");
        true
    }

    fn schema() -> Arc<crate::db::introspect::TableSchema> {
        crate::db::introspect::get_schema(TABLE).expect("introspect")
    }

    #[test]
    fn introspection_reads_columns_types_and_the_generated_key() {
        with_pg(|| {
            if !setup_table() {
                return;
            }
            let s = schema();
            assert_eq!(s.pk, "id");
            assert_eq!(s.pk_type, ColType::Int);
            assert!(s.pk_auto, "BIGSERIAL is database-generated");
            assert_eq!(s.column("name").unwrap().ty, ColType::Text);
            assert!(!s.column("name").unwrap().nullable);
            assert_eq!(s.column("qty").unwrap().ty, ColType::Int);
            assert_eq!(s.column("price").unwrap().ty, ColType::Decimal);
            assert_eq!(s.column("active").unwrap().ty, ColType::Bool);
            assert_eq!(s.column("meta").unwrap().ty, ColType::Json);
            assert_eq!(s.column("created_at").unwrap().ty, ColType::DateTime);
            assert!(s.has_created_at && s.has_updated_at);
            let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {TABLE}"));
        });
    }

    #[test]
    fn crud_round_trips_every_column_type() {
        with_pg(|| {
            if !setup_table() {
                return;
            }
            let s = schema();

            // INSERT: the generated key is assigned by the database.
            let doc = serde_json::json!({
                "name": "Ada",
                "qty": 3,
                "price": "19.99",
                "active": true,
                "meta": { "tier": "gold" },
                "created_at": "2026-08-11T10:00:00Z"
            });
            let inserted = super::super::columns::insert_row(&s, &doc).expect("insert");
            let id = inserted["id"].as_i64().expect("generated id");
            assert!(id > 0, "the database assigned the key");
            assert_eq!(inserted["name"], "Ada");
            assert_eq!(inserted["qty"], 3);
            assert_eq!(inserted["active"], true);
            assert_eq!(inserted["meta"]["tier"], "gold");
            // Exact numerics keep their scale by travelling as text.
            assert_eq!(inserted["price"], "19.99");
            // Timestamps come back RFC 3339 so Soli's DateTime can parse them.
            let created = inserted["created_at"].as_str().expect("created_at");
            assert!(created.contains('T'), "{created}");
            // The key is mirrored for the instance plumbing.
            assert_eq!(inserted["_key"], id.to_string());

            // GET by primary key.
            let fetched = super::super::columns::get_row(&s, &serde_json::json!(id))
                .expect("get")
                .expect("row");
            assert_eq!(fetched["name"], "Ada");

            // UPDATE patches only the named columns.
            let updated = super::super::columns::update_row(
                &s,
                &serde_json::json!(id),
                &serde_json::json!({ "qty": 10 }),
            )
            .expect("update");
            assert_eq!(updated["qty"], 10);
            assert_eq!(updated["name"], "Ada", "unnamed columns are preserved");

            // DELETE.
            super::super::columns::delete_row(&s, &serde_json::json!(id)).expect("delete");
            assert!(super::super::columns::get_row(&s, &serde_json::json!(id))
                .expect("get")
                .is_none());

            let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {TABLE}"));
        });
    }

    #[test]
    fn filters_order_count_exists_and_aggregates() {
        with_pg(|| {
            if !setup_table() {
                return;
            }
            let s = schema();
            for (name, qty, price, active) in [
                ("a", Some(1), "10.00", true),
                ("b", Some(5), "20.50", true),
                ("c", None, "30.00", false),
            ] {
                let mut doc = serde_json::json!({
                    "name": name, "price": price, "active": active
                });
                if let Some(q) = qty {
                    doc["qty"] = serde_json::json!(q);
                }
                super::super::columns::insert_row(&s, &doc).expect("insert");
            }

            // Equality filter on a bool column.
            let mut q = ColumnQuery::new(s.clone());
            q.eq_filters
                .insert("active".into(), serde_json::json!(true));
            assert_eq!(super::super::columns::count(&q).expect("count"), 2);
            assert!(super::super::columns::exists(&q).expect("exists"));

            // NULL filter must match the row whose qty is NULL — the bug that
            // `col = NULL` would silently produce zero rows.
            let mut null_q = ColumnQuery::new(s.clone());
            null_q
                .eq_filters
                .insert("qty".into(), serde_json::Value::Null);
            let null_rows = super::super::columns::select_rows(&null_q).expect("select");
            assert_eq!(null_rows.len(), 1);
            assert_eq!(null_rows[0]["name"], "c");

            // Order + limit.
            let mut ordered = ColumnQuery::new(s.clone());
            ordered.order_field = Some("name".into());
            ordered.order_desc = true;
            ordered.limit = Some(2);
            let rows = super::super::columns::select_rows(&ordered).expect("select");
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0]["name"], "c");
            assert_eq!(rows[1]["name"], "b");

            // Aggregates over real numeric columns.
            let all = ColumnQuery::new(s.clone());
            let sum = super::super::columns::aggregate(&all, SqlAgg::Sum, "price").expect("sum");
            assert_eq!(sum.as_f64().expect("numeric sum"), 60.50);
            let max = super::super::columns::aggregate(&all, SqlAgg::Max, "qty").expect("max");
            assert_eq!(max.as_i64(), Some(5));
            let count = super::super::columns::aggregate(&all, SqlAgg::Count, "id").expect("count");
            assert_eq!(count.as_i64(), Some(3));

            // A non-numeric aggregate is refused, not silently zero.
            let err = super::super::columns::aggregate(&all, SqlAgg::Sum, "name")
                .expect_err("sum over text must error");
            assert!(err.contains("not numeric"), "{err}");

            let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {TABLE}"));
        });
    }

    #[test]
    fn column_writes_participate_in_transactions() {
        with_pg(|| {
            if !setup_table() {
                return;
            }
            clear_transaction();
            let s = schema();

            // Committed insert survives.
            begin_transaction(None).expect("begin");
            let kept =
                super::super::columns::insert_row(&s, &serde_json::json!({ "name": "keep" }))
                    .expect("insert in tx");
            commit_transaction().expect("commit");
            let id = kept["id"].as_i64().unwrap();
            assert!(super::super::columns::get_row(&s, &serde_json::json!(id))
                .expect("get")
                .is_some());

            // Rolled-back insert vanishes — proof the column path reuses the
            // transaction's held connection rather than its own.
            begin_transaction(None).expect("begin2");
            let dropped =
                super::super::columns::insert_row(&s, &serde_json::json!({ "name": "drop" }))
                    .expect("insert in tx2");
            let dropped_id = dropped["id"].as_i64().unwrap();
            rollback_transaction().expect("rollback");
            assert!(
                super::super::columns::get_row(&s, &serde_json::json!(dropped_id))
                    .expect("get")
                    .is_none(),
                "a rolled-back column write must not persist"
            );

            let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {TABLE}"));
        });
    }

    #[test]
    fn composite_and_missing_primary_keys_fail_introspection() {
        with_pg(|| {
            if ensure_connected().is_err() {
                return;
            }
            let composite = "soli_col_composite";
            let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {composite}"));
            execute_ddl(&format!(
                "CREATE TABLE {composite} (
                     order_id BIGINT NOT NULL,
                     item_id BIGINT NOT NULL,
                     PRIMARY KEY (order_id, item_id)
                 )"
            ))
            .expect("create composite table");
            let err = crate::db::introspect::get_schema(composite)
                .expect_err("composite PK must be refused");
            assert!(err.contains("composite primary key"), "{err}");
            assert!(err.contains("order_id, item_id"), "{err}");

            let keyless = "soli_col_keyless";
            let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {keyless}"));
            execute_ddl(&format!("CREATE TABLE {keyless} (message TEXT)"))
                .expect("create keyless table");
            let err =
                crate::db::introspect::get_schema(keyless).expect_err("missing PK must be refused");
            assert!(err.contains("no primary key"), "{err}");

            // A table that does not exist names the connection and the table.
            let err = crate::db::introspect::get_schema("soli_col_ghost")
                .expect_err("missing table must be refused");
            assert!(err.contains("not found"), "{err}");
            assert!(err.contains("never creates or alters"), "{err}");

            let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {composite}"));
            let _ = execute_ddl(&format!("DROP TABLE IF EXISTS {keyless}"));
        });
    }
}
