//! MySQL / MariaDB document backend (Phase 2).
//!
//! Same document model as PostgreSQL: `_key` PK + `doc` JSON column.

use super::registry::{active_connection_name, active_spec};
use super::sql_compile::{
    compile_aggregate_d, compile_count_d, compile_delete_all_d, compile_exists_d,
    compile_group_by_d, compile_select_by_keys_d, compile_select_d, compile_select_json_text_in_d,
    compile_update_all_d, create_table_sql_d, drop_table_sql_d, migrations_table_sql_d, Dialect,
    GroupAgg, ListQuery, SqlAgg, SqlBind,
};
use mysql::prelude::*;
use mysql::{Opts, OptsBuilder, Value as MysqlValue};
use r2d2::Pool;
use r2d2_mysql::MySqlConnectionManager;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type MyPool = Pool<MySqlConnectionManager>;
type MyConn = r2d2::PooledConnection<MySqlConnectionManager>;

static POOLS: OnceLock<Mutex<HashMap<String, MyPool>>> = OnceLock::new();

struct TxState {
    conn: MyConn,
    nest: u32,
    /// Connection name the tx was begun on. Ops on OTHER mysql connections
    /// must not reuse this connection — that would execute them on the wrong
    /// database (silent cross-database write).
    name: String,
}

thread_local! {
    static TX: RefCell<Option<TxState>> = const { RefCell::new(None) };
    static TX_ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// Re-entrancy for `with_conn`: the live connection plus the connection
    /// name it belongs to, so a nested op on a DIFFERENT named connection
    /// never borrows it.
    static ACTIVE_CONN: RefCell<Option<(*mut MyConn, String)>> = const { RefCell::new(None) };
}

/// Panic-safe reset for `ACTIVE_CONN`: if `f` unwinds, the pointer must not
/// dangle into the next `with_conn` on this (reused worker) thread.
struct ActiveConnGuard;

impl ActiveConnGuard {
    fn set(conn: &mut MyConn, name: String) -> Self {
        ACTIVE_CONN.with(|c| *c.borrow_mut() = Some((conn as *mut MyConn, name)));
        ActiveConnGuard
    }
}

impl Drop for ActiveConnGuard {
    fn drop(&mut self) {
        ACTIVE_CONN.with(|c| *c.borrow_mut() = None);
    }
}

fn pools() -> &'static Mutex<HashMap<String, MyPool>> {
    POOLS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pool_for_active() -> Result<MyPool, String> {
    let name = active_connection_name();
    let spec = active_spec()?;
    if spec.adapter != super::Adapter::Mysql {
        return Err(format!(
            "connection {:?} is {}, not mysql",
            name,
            spec.adapter.as_str()
        ));
    }
    let url = spec
        .url
        .clone()
        .ok_or_else(|| format!("connection {name:?}: url required for mysql"))?;
    let mut map = pools().lock().unwrap();
    if let Some(p) = map.get(&name) {
        return Ok(p.clone());
    }
    let opts = Opts::from_url(&url).map_err(|e| format!("invalid DATABASE_URL ({name}): {e}"))?;
    let builder = OptsBuilder::from_opts(opts);
    let manager = MySqlConnectionManager::new(builder);
    let max = spec.pool_size.unwrap_or(10).max(1);
    let pool = Pool::builder()
        .max_size(max as u32)
        .connection_timeout(Duration::from_secs(10))
        .build(manager)
        .map_err(|e| format!("mysql pool ({name}): {e}"))?;
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
            "unsupported isolation level {other:?} for mysql \
             (use read_committed, repeatable_read, serializable)"
        )),
    }
}

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
            return Ok(format!("sql-mysql-nested-{}", tx.nest));
        }
        let iso = map_isolation(isolation_level)?;
        let pool = pool_for_active()?;
        let mut conn = pool
            .get()
            .map_err(|e| format!("mysql transaction checkout: {e}"))?;
        // SET TRANSACTION must run before START TRANSACTION for isolation.
        conn.query_drop(format!("SET TRANSACTION ISOLATION LEVEL {iso}"))
            .map_err(|e| format!("mysql SET TRANSACTION: {e}"))?;
        conn.query_drop("START TRANSACTION")
            .map_err(|e| format!("mysql START TRANSACTION: {e}"))?;
        *slot = Some(TxState {
            conn,
            nest: 0,
            name,
        });
        TX_ACTIVE.set(true);
        Ok("sql-mysql".into())
    })
}

pub fn commit_transaction() -> Result<(), String> {
    TX.with(|cell| {
        let mut slot = cell.borrow_mut();
        let tx = slot
            .as_mut()
            .ok_or_else(|| "No active mysql transaction".to_string())?;
        if tx.nest > 0 {
            tx.nest -= 1;
            return Ok(());
        }
        let mut state = slot.take().expect("tx present");
        TX_ACTIVE.set(false);
        state
            .conn
            .query_drop("COMMIT")
            .map_err(|e| format!("mysql COMMIT: {e}"))?;
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
            .query_drop("ROLLBACK")
            .map_err(|e| format!("mysql ROLLBACK: {e}"))
    })
}

pub fn clear_transaction() {
    TX.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(mut state) = slot.take() {
            TX_ACTIVE.set(false);
            let _ = state.conn.query_drop("ROLLBACK");
        }
    });
}

fn with_conn<T>(f: impl FnOnce(&mut MyConn) -> Result<T, String>) -> Result<T, String> {
    let name = active_connection_name();

    // Re-entrant — reuse the outer connection only when the nested op targets
    // the SAME named connection.
    let reentrant = ACTIVE_CONN.with(|c| match &*c.borrow() {
        Some((ptr, n)) if *n == name => Some(*ptr),
        _ => None,
    });
    if let Some(ptr) = reentrant {
        // SAFETY: pointer set only for the duration of an outer with_conn on
        // this thread; the guard clears it even on unwind.
        let conn = unsafe { &mut *ptr };
        return f(conn);
    }

    // Prefer the open transaction connection — but only for ops on the SAME
    // named connection: handing it to another connection's op would execute
    // that op on the wrong database.
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
            let conn: &mut MyConn = &mut state.conn;
            let _guard = ActiveConnGuard::set(conn, name);
            return f(conn);
        }
    }

    let pool = pool_for_active()?;
    let mut conn = pool.get().map_err(|e| format!("mysql checkout: {e}"))?;
    let _guard = ActiveConnGuard::set(&mut conn, name);
    f(&mut conn)
}

fn to_mysql_params(params: &[SqlBind]) -> Vec<MysqlValue> {
    params
        .iter()
        .map(|p| match p {
            SqlBind::Text(s) => MysqlValue::from(s.as_str()),
            SqlBind::I64(n) => MysqlValue::from(*n),
            SqlBind::F64(f) => MysqlValue::from(*f),
            SqlBind::Bool(b) => MysqlValue::from(*b),
            SqlBind::Json(j) => MysqlValue::from(j.to_string()),
        })
        .collect()
}

// ---------- CRUD ----------

/// SELECT one doc on an already-checked-out connection. Write paths use this
/// instead of `get` — a second pool checkout while holding a connection
/// stalls (and with pool_size=1, deadlocks) under concurrency.
fn get_on(conn: &mut MyConn, table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    let table_q = Dialect::Mysql.quote_ident(table)?;
    let sql = format!("SELECT doc FROM {table_q} WHERE _key = ?");
    let row: Option<String> = conn
        .exec_first(&sql, (key,))
        .map_err(|e| format!("mysql get: {e}"))?;
    match row {
        Some(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| format!("mysql get json: {e}")),
        None => Ok(None),
    }
}

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
    let table_q = Dialect::Mysql.quote_ident(table)?;
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES (?, CAST(? AS JSON)) \
         ON DUPLICATE KEY UPDATE doc = VALUES(doc)"
    );
    let doc_str = document.to_string();
    with_conn(|conn| {
        conn.exec_drop(&sql, (&key, &doc_str))
            .map_err(|e| format!("mysql insert: {e}"))?;
        get_on(conn, table, &key)?.ok_or_else(|| "mysql insert: row missing after write".into())
    })
}

pub fn get(table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    if !table_exists(table)? {
        return Ok(None);
    }
    with_conn(|conn| get_on(conn, table, key))
}

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
    let table_q = Dialect::Mysql.quote_ident(table)?;
    let doc_str = document.to_string();
    if merge {
        let sql = format!(
            "UPDATE {table_q} SET doc = JSON_MERGE_PATCH(COALESCE(doc, '{{}}'), CAST(? AS JSON)) \
             WHERE _key = ?"
        );
        return with_conn(|conn| {
            conn.exec_drop(&sql, (&doc_str, key))
                .map_err(|e| format!("mysql update merge: {e}"))?;
            // affected_rows() can't tell "row missing" from "no-op merge"
            // (CLIENT_FOUND_ROWS is off, so it reports CHANGED rows) — read
            // back instead, and only insert when the row truly isn't there.
            if let Some(doc) = get_on(conn, table, key)? {
                return Ok(doc);
            }
            // Insert patch as full document; merge on a concurrent insert.
            let ins = format!(
                "INSERT INTO {table_q} (_key, doc) VALUES (?, CAST(? AS JSON)) \
                 ON DUPLICATE KEY UPDATE \
                 doc = JSON_MERGE_PATCH(COALESCE(doc, '{{}}'), CAST(? AS JSON))"
            );
            conn.exec_drop(&ins, (key, &doc_str, &doc_str))
                .map_err(|e| format!("mysql update merge insert: {e}"))?;
            get_on(conn, table, key)?.ok_or_else(|| "mysql update: row missing".into())
        });
    }
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES (?, CAST(? AS JSON)) \
         ON DUPLICATE KEY UPDATE doc = VALUES(doc)"
    );
    with_conn(|conn| {
        conn.exec_drop(&sql, (key, &doc_str))
            .map_err(|e| format!("mysql update: {e}"))?;
        get_on(conn, table, key)?.ok_or_else(|| "mysql update: row missing".into())
    })
}

pub fn delete(table: &str, key: &str) -> Result<(), String> {
    if !table_exists(table)? {
        return Ok(());
    }
    let table_q = Dialect::Mysql.quote_ident(table)?;
    let sql = format!("DELETE FROM {table_q} WHERE _key = ?");
    with_conn(|conn| {
        conn.exec_drop(&sql, (key,))
            .map_err(|e| format!("mysql delete: {e}"))?;
        Ok(())
    })
}

pub fn select(q: &ListQuery) -> Result<Vec<serde_json::Value>, String> {
    if !table_exists(&q.table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_d(Dialect::Mysql, q)?;
    query_docs(&compiled.sql, &compiled.params)
}

pub fn select_by_keys(table: &str, keys: &[String]) -> Result<Vec<serde_json::Value>, String> {
    if keys.is_empty() || !table_exists(table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_by_keys_d(Dialect::Mysql, table, keys)?;
    query_docs(&compiled.sql, &compiled.params)
}

pub fn select_json_text_in(
    table: &str,
    field: &str,
    values: &[String],
) -> Result<Vec<serde_json::Value>, String> {
    if values.is_empty() || !table_exists(table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_json_text_in_d(Dialect::Mysql, table, field, values)?;
    query_docs(&compiled.sql, &compiled.params)
}

pub fn group_by(
    q: &ListQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    if !table_exists(&q.table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_group_by_d(Dialect::Mysql, q, group_fields, aggs)?;
    // Build expected column order: group fields then agg aliases (or "n").
    let mut col_names: Vec<String> = group_fields.to_vec();
    if aggs.is_empty() {
        col_names.push("n".into());
    } else {
        for a in aggs {
            col_names.push(a.alias.clone());
        }
    }
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        let rows: Vec<mysql::Row> = conn
            .exec(&compiled.sql, params)
            .map_err(|e| format!("mysql group_by: {e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut map = serde_json::Map::new();
            for (i, name) in col_names.iter().enumerate() {
                let v: Option<MysqlValue> = row.get(i);
                map.insert(name.clone(), mysql_value_to_json(v));
            }
            out.push(serde_json::Value::Object(map));
        }
        Ok(out)
    })
}

fn mysql_value_to_json(v: Option<MysqlValue>) -> serde_json::Value {
    match v {
        None | Some(MysqlValue::NULL) => serde_json::Value::Null,
        Some(MysqlValue::Int(n)) => serde_json::json!(n),
        Some(MysqlValue::UInt(n)) => serde_json::json!(n),
        Some(MysqlValue::Float(n)) => serde_json::json!(n),
        Some(MysqlValue::Double(n)) => serde_json::json!(n),
        Some(MysqlValue::Bytes(b)) => {
            let s = String::from_utf8_lossy(&b).into_owned();
            if let Ok(n) = s.parse::<i64>() {
                serde_json::json!(n)
            } else if let Ok(f) = s.parse::<f64>() {
                serde_json::json!(f)
            } else {
                serde_json::Value::String(s)
            }
        }
        Some(other) => serde_json::Value::String(format!("{other:?}")),
    }
}

pub fn count(q: &ListQuery) -> Result<i64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_count_d(Dialect::Mysql, q)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        let n: Option<i64> = conn
            .exec_first(&compiled.sql, params)
            .map_err(|e| format!("mysql count: {e}"))?;
        Ok(n.unwrap_or(0))
    })
}

pub fn exists(q: &ListQuery) -> Result<bool, String> {
    if !table_exists(&q.table)? {
        return Ok(false);
    }
    let compiled = compile_exists_d(Dialect::Mysql, q)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        let row: Option<i64> = conn
            .exec_first(&compiled.sql, params)
            .map_err(|e| format!("mysql exists: {e}"))?;
        Ok(row.is_some())
    })
}

pub fn aggregate(q: &ListQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    if !table_exists(&q.table)? {
        return Ok(serde_json::Value::Null);
    }
    let compiled = compile_aggregate_d(Dialect::Mysql, q, func, field)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        if matches!(func, SqlAgg::Count) {
            let n: Option<i64> = conn
                .exec_first(&compiled.sql, params)
                .map_err(|e| format!("mysql aggregate: {e}"))?;
            return Ok(serde_json::json!(n.unwrap_or(0)));
        }
        // SUM/AVG may come back as Decimal string or f64
        let row: Option<MysqlValue> = conn
            .exec_first(&compiled.sql, params)
            .map_err(|e| format!("mysql aggregate: {e}"))?;
        Ok(match row {
            None | Some(MysqlValue::NULL) => serde_json::Value::Null,
            Some(v) => {
                let s = match v {
                    MysqlValue::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
                    MysqlValue::Int(n) => n.to_string(),
                    MysqlValue::UInt(n) => n.to_string(),
                    MysqlValue::Float(n) => n.to_string(),
                    MysqlValue::Double(n) => n.to_string(),
                    other => format!("{other:?}"),
                };
                if let Ok(f) = s.parse::<f64>() {
                    serde_json::json!(f)
                } else {
                    serde_json::Value::String(s)
                }
            }
        })
    })
}

pub fn delete_all(q: &ListQuery) -> Result<u64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_delete_all_d(Dialect::Mysql, q)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        let result = conn
            .exec_iter(&compiled.sql, params)
            .map_err(|e| format!("mysql delete_all: {e}"))?;
        Ok(result.affected_rows())
    })
}

pub fn update_all(q: &ListQuery, patch: serde_json::Value) -> Result<u64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_update_all_d(Dialect::Mysql, q, &patch)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        let result = conn
            .exec_iter(&compiled.sql, params)
            .map_err(|e| format!("mysql update_all: {e}"))?;
        Ok(result.affected_rows())
    })
}

fn query_docs(sql: &str, params: &[SqlBind]) -> Result<Vec<serde_json::Value>, String> {
    with_conn(|conn| {
        let params = to_mysql_params(params);
        let rows: Vec<String> = conn
            .exec(sql, params)
            .map_err(|e| format!("mysql query: {e}"))?;
        rows.into_iter()
            .map(|s| serde_json::from_str(&s).map_err(|e| format!("mysql row json: {e}")))
            .collect()
    })
}

// ---------- schema ----------

pub fn ensure_table(table: &str) -> Result<(), String> {
    let ddl = create_table_sql_d(Dialect::Mysql, table)?;
    with_conn(|conn| {
        conn.query_drop(&ddl)
            .map_err(|e| format!("mysql ensure_table: {e}"))
    })
}

pub fn drop_table(table: &str) -> Result<(), String> {
    let ddl = drop_table_sql_d(Dialect::Mysql, table)?;
    with_conn(|conn| {
        conn.query_drop(&ddl)
            .map_err(|e| format!("mysql drop_table: {e}"))
    })
}

/// Add `delta` to a numeric JSON field **in one statement**, returning the new
/// value. See the Postgres twin for why this is not read-modify-write.
pub fn increment_field(
    table: &str,
    key: &str,
    field: &str,
    delta: i64,
) -> Result<Option<i64>, String> {
    if !table_exists(table)? {
        return Ok(None);
    }
    Dialect::Mysql.quote_ident(field)?;
    let table_q = Dialect::Mysql.quote_ident(table)?;
    let update = format!(
        "UPDATE {table_q} SET doc = JSON_SET(doc, '$.{field}', \
             COALESCE(JSON_EXTRACT(doc, '$.{field}'), 0) + ?) WHERE _key = ?"
    );
    // MySQL has no RETURNING, so the read-back rides the same connection —
    // still inside the statement's own atomicity for the write itself.
    let select = format!(
        "SELECT JSON_UNQUOTE(JSON_EXTRACT(doc, '$.{field}')) FROM {table_q} WHERE _key = ?"
    );
    with_conn(|conn| {
        conn.exec_drop(&update, (delta, key))
            .map_err(|e| format!("mysql increment: {e}"))?;
        let text: Option<Option<String>> = conn
            .exec_first(&select, (key,))
            .map_err(|e| format!("mysql increment read-back: {e}"))?;
        Ok(text.flatten().and_then(|t| super::parse_counter(&t)))
    })
}

/// Index names on `table`.
pub fn list_index_names(table: &str) -> Result<Vec<String>, String> {
    with_conn(|conn| {
        let rows: Vec<String> = conn
            .exec(
                "SELECT DISTINCT INDEX_NAME FROM information_schema.STATISTICS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
                (table,),
            )
            .map_err(|e| format!("mysql list indexes: {e}"))?;
        Ok(rows)
    })
}

/// Column names on `table` — used to skip a generated column that already
/// exists, since MySQL has no `ADD COLUMN IF NOT EXISTS`.
fn list_column_names(table: &str) -> Result<Vec<String>, String> {
    with_conn(|conn| {
        let rows: Vec<String> = conn
            .exec(
                "SELECT COLUMN_NAME FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
                (table,),
            )
            .map_err(|e| format!("mysql list columns: {e}"))?;
        Ok(rows)
    })
}

/// Create a JSON-field index on a document table if it is absent.
///
/// MySQL cannot index a JSON extract, so each field first gets a generated
/// `STORED` column. Both steps are skipped individually when they already
/// exist — MySQL supports `IF NOT EXISTS` on neither.
pub fn ensure_doc_index(
    table: &str,
    fields: &[String],
    name: &str,
    unique: bool,
) -> Result<bool, String> {
    if list_index_names(table)?.iter().any(|n| n == name) {
        return Ok(false);
    }
    let existing_columns = list_column_names(table)?;
    let statements = super::ddl::doc_index_sql(Dialect::Mysql, table, fields, name, unique)?;
    for (field, sql) in fields.iter().zip(statements.iter()) {
        let generated = super::ddl::generated_column_name(field);
        if existing_columns.contains(&generated) {
            continue;
        }
        execute_ddl(sql)?;
    }
    // The last statement is the index itself.
    if let Some(create_index) = statements.last() {
        execute_ddl(create_index)?;
    }
    Ok(true)
}

/// Run compiled DDL (migrations' column-table helpers).
pub fn execute_ddl(sql: &str) -> Result<(), String> {
    with_conn(|conn| conn.query_drop(sql).map_err(|e| format!("mysql ddl: {e}")))
}

/// `db.execute`: a dedicated connection, dropped afterwards, so
/// `SET FOREIGN_KEY_CHECKS=0` cannot leak into the pool.
pub fn execute_raw(sql: &str) -> Result<(), String> {
    let spec = active_spec()?;
    let url = spec
        .url
        .as_deref()
        .ok_or_else(|| format!("connection {:?}: url required for mysql", spec.name))?;
    let opts = Opts::from_url(url).map_err(|e| format!("mysql execute: {e}"))?;
    let mut conn = mysql::Conn::new(opts).map_err(|e| format!("mysql execute: {e}"))?;
    conn.query_drop(sql)
        .map_err(|e| format!("mysql execute: {e}"))
}

pub fn ensure_migrations_table() -> Result<(), String> {
    with_conn(|conn| {
        conn.query_drop(migrations_table_sql_d(Dialect::Mysql))
            .map_err(|e| format!("mysql migrations table: {e}"))
    })
}

pub fn list_applied_migrations() -> Result<Vec<(String, String)>, String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        let rows: Vec<(String, String)> = conn
            .query("SELECT version, name FROM `_migrations` ORDER BY version")
            .map_err(|e| format!("mysql list migrations: {e}"))?;
        Ok(rows)
    })
}

pub fn record_migration(version: &str, name: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        conn.exec_drop(
            "INSERT IGNORE INTO `_migrations` (version, name) VALUES (?, ?)",
            (version, name),
        )
        .map_err(|e| format!("mysql record migration: {e}"))?;
        Ok(())
    })
}

pub fn remove_migration(version: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        conn.exec_drop("DELETE FROM `_migrations` WHERE version = ?", (version,))
            .map_err(|e| format!("mysql remove migration: {e}"))?;
        Ok(())
    })
}

// ---------- column-aware model execution ----------

use super::introspect::{ColType, TableSchema};
use super::sql_columns_compile as cols;

/// Convert one driver row into a JSON object keyed by column name, so
/// downstream hydration matches the document path.
fn row_to_json(schema: &TableSchema, row: &mysql::Row) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    let mut idx = 0usize;
    for col in &schema.columns {
        // Unreadable columns are never selected, so they hold no position.
        if col.ty == ColType::Unknown {
            continue;
        }
        let value = match col.ty {
            // Numbers arrive as text (see ColType::reads_as_text) so neither
            // backend has to match an exact SQL width.
            ColType::Int => row
                .get_opt::<Option<String>, usize>(idx)
                .and_then(Result::ok)
                .flatten()
                .and_then(|s| s.trim().parse::<i64>().ok())
                .map(|n| serde_json::json!(n))
                .unwrap_or(serde_json::Value::Null),
            ColType::Float => row
                .get_opt::<Option<String>, usize>(idx)
                .and_then(Result::ok)
                .flatten()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .map(|f| serde_json::json!(f))
                .unwrap_or(serde_json::Value::Null),
            // MySQL has no native bool: tinyint(1) arrives as 0/1.
            ColType::Bool => row
                .get_opt::<Option<i64>, usize>(idx)
                .and_then(Result::ok)
                .flatten()
                .map(|n| serde_json::json!(n != 0))
                .unwrap_or(serde_json::Value::Null),
            ColType::Json => row
                .get_opt::<Option<String>, usize>(idx)
                .and_then(Result::ok)
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::Value::Null),
            _ => row
                .get_opt::<Option<String>, usize>(idx)
                .and_then(Result::ok)
                .flatten()
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

/// MySQL renders DATETIME as `2026-08-11 10:00:00`; re-emit the RFC 3339 form
/// Soli's DateTime parses. MySQL stores no offset, so UTC is assumed — the
/// documented convention for column-aware models.
fn normalize_text(ty: ColType, raw: &str) -> String {
    if ty != ColType::DateTime {
        return raw.to_string();
    }
    let mut out = raw.replacen(' ', "T", 1);
    if !out.ends_with('Z') && !out.contains('+') {
        out.push('Z');
    }
    out
}

/// Re-read a row by primary key on the connection already held, so an insert or
/// update can return the stored row (MySQL has no RETURNING).
fn read_back(
    conn: &mut MyConn,
    schema: &TableSchema,
    pk: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let compiled = cols::compile_get_cols(Dialect::Mysql, schema, pk)?;
    let params = to_mysql_params(&compiled.params);
    let row: Option<mysql::Row> = conn
        .exec_first(&compiled.sql, params)
        .map_err(|e| format!("mysql column read-back: {e}"))?;
    Ok(row.map(|r| row_to_json(schema, &r)))
}

pub fn col_get(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    with_conn(|conn| read_back(conn, schema, pk))
}

pub fn col_insert(
    schema: &std::sync::Arc<TableSchema>,
    doc: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_insert_cols(Dialect::Mysql, schema, doc)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        conn.exec_drop(&compiled.sql, params)
            .map_err(|e| format!("mysql column insert: {e}"))?;
        // A generated key comes from LAST_INSERT_ID(); an explicit one is
        // whatever the caller supplied. Both must read back on THIS connection.
        let key = if schema.pk_auto && doc.get(&schema.pk).is_none_or(|v| v.is_null()) {
            serde_json::json!(conn.last_insert_id())
        } else {
            doc.get(&schema.pk)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        };
        read_back(conn, schema, &key)?.ok_or_else(|| {
            format!(
                "mysql column insert: row missing after write in {:?}",
                schema.table
            )
        })
    })
}

pub fn col_update(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
    patch: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_update_cols(Dialect::Mysql, schema, pk, patch)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        conn.exec_drop(&compiled.sql, params)
            .map_err(|e| format!("mysql column update: {e}"))?;
        // Read back rather than trusting affected_rows: MySQL reports 0 for a
        // no-op update, which would look like a missing row.
        read_back(conn, schema, pk)?
            .ok_or_else(|| format!("no row in {:?} with {} = {}", schema.table, schema.pk, pk))
    })
}

pub fn col_delete(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<(), String> {
    let compiled = cols::compile_delete_cols(Dialect::Mysql, schema, pk)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        conn.exec_drop(&compiled.sql, params)
            .map_err(|e| format!("mysql column delete: {e}"))?;
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
    let (sql, params) = cols::compile_increment_col(Dialect::Mysql, schema, pk, column, delta)?;
    // No RETURNING on MySQL: read back on the same connection.
    let read_back = cols::compile_read_column(Dialect::Mysql, schema, pk, column)?;
    with_conn(|conn| {
        conn.exec_drop(&sql, to_mysql_params(&params))
            .map_err(|e| format!("mysql column increment: {e}"))?;
        let text: Option<Option<String>> = conn
            .exec_first(&read_back.sql, to_mysql_params(&read_back.params))
            .map_err(|e| format!("mysql column increment read-back: {e}"))?;
        Ok(text.flatten().and_then(|t| super::parse_counter(&t)))
    })
}

pub fn col_select(q: &cols::ColumnQuery) -> Result<Vec<serde_json::Value>, String> {
    let compiled = cols::compile_select_cols(Dialect::Mysql, q)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        let rows: Vec<mysql::Row> = conn
            .exec(&compiled.sql, params)
            .map_err(|e| format!("mysql column select: {e}"))?;
        Ok(rows.iter().map(|r| row_to_json(&q.schema, r)).collect())
    })
}

pub fn col_count(q: &cols::ColumnQuery) -> Result<i64, String> {
    let compiled = cols::compile_count_cols(Dialect::Mysql, q)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        let n: Option<i64> = conn
            .exec_first(&compiled.sql, params)
            .map_err(|e| format!("mysql column count: {e}"))?;
        Ok(n.unwrap_or(0))
    })
}

pub fn col_exists(q: &cols::ColumnQuery) -> Result<bool, String> {
    let compiled = cols::compile_exists_cols(Dialect::Mysql, q)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        let hit: Option<i64> = conn
            .exec_first(&compiled.sql, params)
            .map_err(|e| format!("mysql column exists: {e}"))?;
        Ok(hit.is_some())
    })
}

pub fn col_aggregate(
    q: &cols::ColumnQuery,
    func: SqlAgg,
    field: &str,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_aggregate_cols(Dialect::Mysql, q, func, field)?;
    with_conn(|conn| {
        let params = to_mysql_params(&compiled.params);
        if func == SqlAgg::Count {
            let n: Option<i64> = conn
                .exec_first(&compiled.sql, params)
                .map_err(|e| format!("mysql column aggregate: {e}"))?;
            return Ok(serde_json::json!(n.unwrap_or(0)));
        }
        let raw: Option<Option<String>> = conn
            .exec_first(&compiled.sql, params)
            .map_err(|e| format!("mysql column aggregate: {e}"))?;
        Ok(super::columns::parse_agg_text(raw.flatten()))
    })
}

// ---------- column-aware model introspection ----------

/// Read the shape of an existing table for column mode. `COLUMN_TYPE` is
/// carried alongside `DATA_TYPE` because only the former distinguishes
/// `tinyint(1)` (the MySQL bool convention) from a wider tinyint, and shows
/// `unsigned`.
pub fn introspect_table(table: &str) -> Result<super::introspect::RawColumns, String> {
    with_conn(|conn| {
        let rows: Vec<(String, String, String, String, String, String)> = conn
            .exec(
                "SELECT COLUMN_NAME, DATA_TYPE, COLUMN_TYPE, IS_NULLABLE, \
                        COALESCE(COLUMN_KEY, ''), COALESCE(EXTRA, '') \
                 FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? \
                 ORDER BY ORDINAL_POSITION",
                (table,),
            )
            .map_err(|e| format!("mysql introspect columns: {e}"))?;

        let mut columns = Vec::with_capacity(rows.len());
        let mut pk = Vec::new();
        for (name, data_type, column_type, nullable, key, extra) in rows {
            if key.eq_ignore_ascii_case("PRI") {
                pk.push(name.clone());
            }
            let is_auto = extra.to_ascii_lowercase().contains("auto_increment");
            columns.push((
                name,
                data_type,
                column_type,
                nullable.eq_ignore_ascii_case("YES"),
                is_auto,
            ));
        }

        Ok(super::introspect::RawColumns { columns, pk })
    })
}

// ---------- Soli job engine ----------

/// Atomically claim up to `batch` due jobs from `_jobs` for the Soli job
/// engine. MySQL forbids self-referencing subqueries in UPDATE, so this uses
/// the token-claim pattern: one atomic `UPDATE … ORDER BY … LIMIT n` stamps a
/// unique claim token into `locked_by`, then a SELECT retrieves exactly the
/// rows this call won. The lease-reclaim clause recovers jobs whose worker
/// died holding a lease.
pub fn claim_jobs(
    now_iso: &str,
    worker_id: &str,
    locked_until_iso: &str,
    batch: usize,
) -> Result<Vec<serde_json::Value>, String> {
    if !table_exists("_jobs")? {
        return Ok(Vec::new());
    }
    let token = format!("{}#{}", worker_id, uuid::Uuid::new_v4());
    let update = "UPDATE `_jobs` SET doc = JSON_SET(doc, \
                      '$.state', 'running', '$.locked_by', ?, '$.locked_until', ?, \
                      '$.attempts', COALESCE(JSON_EXTRACT(doc, '$.attempts'), 0) + 1) \
                  WHERE ((JSON_UNQUOTE(JSON_EXTRACT(doc, '$.state')) IN ('pending','scheduled','failed') \
                          AND JSON_UNQUOTE(JSON_EXTRACT(doc, '$.run_at')) <= ?) \
                     OR (JSON_UNQUOTE(JSON_EXTRACT(doc, '$.state')) = 'running' \
                         AND JSON_UNQUOTE(JSON_EXTRACT(doc, '$.locked_until')) < ?)) \
                  ORDER BY COALESCE(JSON_EXTRACT(doc, '$.priority'), 0) DESC, \
                           JSON_UNQUOTE(JSON_EXTRACT(doc, '$.run_at')) ASC \
                  LIMIT ?";
    with_conn(|conn| {
        conn.exec_drop(
            update,
            (&token, locked_until_iso, now_iso, now_iso, batch as u64),
        )
        .map_err(|e| format!("mysql claim_jobs: {e}"))?;
        let rows: Vec<String> = conn
            .exec(
                "SELECT doc FROM `_jobs` \
                 WHERE JSON_UNQUOTE(JSON_EXTRACT(doc, '$.locked_by')) = ?",
                (&token,),
            )
            .map_err(|e| format!("mysql claim_jobs select: {e}"))?;
        rows.into_iter()
            .map(|s| serde_json::from_str(&s).map_err(|e| format!("mysql claim_jobs json: {e}")))
            .collect()
    })
}

/// Compare-and-swap a cron slot: merge `patch` into the `_cron_jobs` row only
/// when its stored `next_run_at` still equals `expected_next_run_at`. Returns
/// true when this process won the slot.
pub fn claim_cron_slot(
    key: &str,
    expected_next_run_at: &str,
    patch: serde_json::Value,
) -> Result<bool, String> {
    if !table_exists("_cron_jobs")? {
        return Ok(false);
    }
    let patch_str = patch.to_string();
    with_conn(|conn| {
        let affected = conn
            .exec_iter(
                "UPDATE `_cron_jobs` \
                 SET doc = JSON_MERGE_PATCH(doc, CAST(? AS JSON)) \
                 WHERE _key = ? AND JSON_UNQUOTE(JSON_EXTRACT(doc, '$.next_run_at')) = ?",
                (&patch_str, key, expected_next_run_at),
            )
            .map_err(|e| format!("mysql claim_cron_slot: {e}"))?
            .affected_rows();
        Ok(affected == 1)
    })
}

fn table_exists(table: &str) -> Result<bool, String> {
    with_conn(|conn| {
        let n: Option<i64> = conn
            .exec_first(
                "SELECT COUNT(*) FROM information_schema.tables \
                 WHERE table_schema = DATABASE() AND table_name = ?",
                (table,),
            )
            .map_err(|e| format!("mysql table_exists: {e}"))?;
        Ok(n.unwrap_or(0) > 0)
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

#[cfg(test)]
mod integration_tests {
    use super::*;

    fn with_mysql(f: impl FnOnce()) {
        // Cross-module lock: the registry override is process-global, so all
        // override-installing test modules must serialize on the same mutex.
        let _g = crate::db::registry::registry_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let url = match std::env::var("MYSQL_URL").or_else(|_| std::env::var("DATABASE_URL")) {
            Ok(u) if u.starts_with("mysql") => u,
            _ => {
                eprintln!("skip: no MYSQL_URL / mysql DATABASE_URL");
                return;
            }
        };
        if mysql::Pool::new(Opts::from_url(&url).expect("url")).is_err() {
            eprintln!("skip: mysql not reachable at {url}");
            return;
        }
        use crate::db::registry::{
            clear_registry_override, set_registry_for_tests, ConnectionRegistry, ConnectionSpec,
        };
        use crate::db::Adapter;
        use std::collections::HashMap;
        let mut connections = HashMap::new();
        connections.insert(
            "primary".into(),
            ConnectionSpec {
                name: "primary".into(),
                adapter: Adapter::Mysql,
                url: Some(url.clone()),
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

    #[test]
    fn crud_roundtrip_when_mysql_available() {
        with_mysql(|| {
            if ensure_connected().is_err() {
                return;
            }
            let table = "soli_mysql_crud_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }
            let doc = serde_json::json!({"_key": "k1", "name": "Ada", "status": "up"});
            let inserted = insert(table, Some("k1"), doc).expect("insert");
            assert_eq!(inserted["name"], "Ada");
            let patched =
                update(table, "k1", serde_json::json!({"status": "late"}), true).expect("merge");
            assert_eq!(patched["status"], "late");
            assert_eq!(patched["name"], "Ada");
            delete(table, "k1").expect("delete");
            assert!(get(table, "k1").unwrap().is_none());
            let _ = drop_table(table);
        });
    }

    #[test]
    fn aggregate_and_bulk_when_mysql_available() {
        use super::super::sql_compile::SoftDeleteMode;
        use std::collections::BTreeMap;
        with_mysql(|| {
            if ensure_connected().is_err() {
                return;
            }
            let table = "soli_mysql_agg_test";
            let _ = drop_table(table);
            if ensure_table(table).is_err() {
                return;
            }
            insert(
                table,
                Some("a"),
                serde_json::json!({"_key": "a", "amount": 10, "status": "open"}),
            )
            .unwrap();
            insert(
                table,
                Some("b"),
                serde_json::json!({"_key": "b", "amount": 5, "status": "open"}),
            )
            .unwrap();
            insert(
                table,
                Some("c"),
                serde_json::json!({"_key": "c", "amount": 99, "status": "closed"}),
            )
            .unwrap();
            let mut eq = BTreeMap::new();
            eq.insert("status".into(), serde_json::json!("open"));
            let q = ListQuery {
                table: table.into(),
                eq_filters: eq,
                filter_sdbql: Some("doc.status == @status".into()),
                soft_delete: SoftDeleteMode::Default,
                is_soft_delete_model: false,
                order_field: None,
                order_desc: false,
                limit: None,
                offset: None,
            };
            let sum = aggregate(&q, SqlAgg::Sum, "amount").expect("sum");
            assert_eq!(sum.as_f64().unwrap_or(0.0), 15.0);
            let n = update_all(&q, serde_json::json!({"tag": "x"})).expect("update_all");
            assert_eq!(n, 2);
            let deleted = delete_all(&q).expect("delete_all");
            assert_eq!(deleted, 2);
            assert!(get(table, "c").unwrap().is_some());
            let _ = drop_table(table);
        });
    }

    #[test]
    fn migrations_when_mysql_available() {
        with_mysql(|| {
            if ensure_connected().is_err() {
                return;
            }
            if ensure_migrations_table().is_err() {
                return;
            }
            let _ = remove_migration("20990101000001");
            record_migration("20990101000001", "mysql_test").expect("record");
            let applied = list_applied_migrations().expect("list");
            assert!(applied.iter().any(|(v, _)| v == "20990101000001"));
            remove_migration("20990101000001").expect("remove");
        });
    }
}
