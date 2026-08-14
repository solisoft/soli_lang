//! SQLite document backend.
//!
//! Same document model as the other SQL adapters: a `_key` primary key plus a
//! `doc` column holding JSON, queried with the json1 functions. The difference
//! is operational rather than logical — SQLite is a file, not a server, so a
//! connection is an `open()` and a "cluster" is a path.
//!
//! Two SQLite facts shape this module:
//!
//! - **One writer at a time.** WAL mode lets readers run during a write, but
//!   writers serialize. Every connection therefore sets a `busy_timeout`, so a
//!   concurrent writer waits instead of failing with `SQLITE_BUSY`.
//! - **No `SKIP LOCKED`.** The job engine cannot lock individual rows, so
//!   [`claim_jobs`] takes the database write lock with `BEGIN IMMEDIATE` and
//!   does its select-then-update inside that transaction. The claim is short,
//!   and correctness comes from the lock rather than from row-level skipping.

use super::registry::{active_connection_name, active_spec};
use super::sql_compile::{
    compile_aggregate_d, compile_count_d, compile_delete_all_d, compile_exists_d,
    compile_group_by_d, compile_insert_many_d, compile_select_by_keys_d, compile_select_d,
    compile_select_json_text_in_d, compile_update_all_d, create_table_sql_d, drop_table_sql_d,
    migrations_table_sql_d, Dialect, GroupAgg, ListQuery, SqlAgg, SqlBind,
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params_from_iter, OptionalExtension};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type SqPool = Pool<SqliteConnectionManager>;
type SqConn = r2d2::PooledConnection<SqliteConnectionManager>;

static POOLS: OnceLock<Mutex<HashMap<String, SqPool>>> = OnceLock::new();

struct TxState {
    conn: SqConn,
    nest: u32,
    /// Connection name the tx was begun on. Ops on OTHER sqlite connections
    /// must not reuse it — that would run them against the wrong file.
    name: String,
}

thread_local! {
    static TX: RefCell<Option<TxState>> = const { RefCell::new(None) };
    static TX_ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// Re-entrancy for `with_conn`: the live connection plus the connection
    /// name it belongs to, so a nested op on a DIFFERENT named connection
    /// never borrows it.
    static ACTIVE_CONN: RefCell<Option<(*mut SqConn, String)>> = const { RefCell::new(None) };
}

/// Panic-safe reset for `ACTIVE_CONN`: if `f` unwinds, the pointer must not
/// dangle into the next `with_conn` on this (reused worker) thread.
struct ActiveConnGuard;

impl ActiveConnGuard {
    fn set(conn: &mut SqConn, name: String) -> Self {
        ACTIVE_CONN.with(|c| *c.borrow_mut() = Some((conn as *mut SqConn, name)));
        ActiveConnGuard
    }
}

impl Drop for ActiveConnGuard {
    fn drop(&mut self) {
        ACTIVE_CONN.with(|c| *c.borrow_mut() = None);
    }
}

/// Where a connection's data lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A database file on disk.
    File(PathBuf),
    /// A private in-memory database — one connection, gone at exit. Useful for
    /// tests and throwaway scripts.
    Memory,
}

/// Resolve a connection URL to a SQLite target.
///
/// Accepted forms: `sqlite://relative/app.db`, `sqlite:///absolute/app.db`,
/// `sqlite:app.db`, `sqlite::memory:`, `:memory:`, or a bare filesystem path.
/// A `?query` suffix is ignored — SQLite takes its options from pragmas here,
/// not from the URL.
pub(crate) fn parse_target(url: &str) -> Target {
    let raw = url.trim();
    let rest = ["sqlite3://", "sqlite://", "sqlite3:", "sqlite:"]
        .iter()
        .find_map(|p| raw.strip_prefix(p))
        .unwrap_or(raw);
    let rest = rest.split('?').next().unwrap_or("").trim();
    if rest.is_empty()
        || rest == ":memory:"
        || rest == "/:memory:"
        || rest.eq_ignore_ascii_case("memory")
    {
        return Target::Memory;
    }
    Target::File(PathBuf::from(rest))
}

/// Render a driver error, and classify a constraint violation by SQLite's
/// extended result code. SQLite names `table.column` in the message, which is
/// the most precise of the three adapters.
fn lite_error(context: &str, e: &rusqlite::Error) -> String {
    let text = format!("{context}: {e}");
    let rusqlite::Error::SqliteFailure(code, message) = e else {
        return text;
    };
    let kind = match code.extended_code {
        2067 | 1555 => Some(super::error::ConstraintKind::Unique),
        787 => Some(super::error::ConstraintKind::ForeignKey),
        1299 => Some(super::error::ConstraintKind::NotNull),
        275 => Some(super::error::ConstraintKind::Check),
        _ => None,
    };
    let Some(kind) = kind else {
        return text;
    };
    // Two shapes, depending on what was violated:
    //   "UNIQUE constraint failed: orders.code"          → a real column
    //   "UNIQUE constraint failed: index 'idx_x_status'"  → an expression index,
    //                                                       which has no column
    let tail = message
        .as_deref()
        .and_then(|m| m.rsplit(": ").next())
        .map(str::trim);
    let index_name = tail
        .and_then(|t| t.strip_prefix("index "))
        .map(|t| t.trim_matches('\'').to_string());
    let column = match index_name {
        Some(_) => None,
        None => tail
            .and_then(|t| t.split(',').next())
            .and_then(|first| first.trim().rsplit('.').next())
            .map(str::to_string)
            .filter(|c| !c.is_empty() && !c.contains(' ')),
    };
    let constraint = super::error::Constraint::new(kind)
        .with_column(column)
        .with_name(index_name);
    format!("{} {text}", constraint.to_marker())
}

fn pools() -> &'static Mutex<HashMap<String, SqPool>> {
    POOLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Per-connection setup. WAL gives concurrent readers during a write, and the
/// busy timeout turns writer contention into a wait rather than an error.
fn init_conn(conn: &mut rusqlite::Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_secs(10))?;
    // A memory database has no journal to switch; ignore the answer either way.
    let _: Result<String, _> = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0));
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")
}

fn pool_for_active() -> Result<SqPool, String> {
    let name = active_connection_name();
    let spec = active_spec()?;
    if spec.adapter != super::Adapter::Sqlite {
        return Err(format!(
            "connection {:?} is {}, not sqlite",
            name,
            spec.adapter.as_str()
        ));
    }
    let url = spec
        .url
        .clone()
        .ok_or_else(|| format!("connection {name:?}: url required for sqlite"))?;
    let mut map = pools().lock().unwrap();
    if let Some(p) = map.get(&name) {
        return Ok(p.clone());
    }
    let target = parse_target(&url);
    let (manager, max) = match &target {
        Target::File(path) => {
            // A missing parent directory is the common first-run failure
            // (`sqlite://db/app.sqlite3`); create it rather than erroring.
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(parent);
            }
            let max = spec.pool_size.unwrap_or(5).max(1) as u32;
            (SqliteConnectionManager::file(path), max)
        }
        // Each memory connection would be its OWN empty database, so the pool
        // must hold exactly one and never recycle it.
        Target::Memory => (SqliteConnectionManager::memory(), 1),
    };
    let manager = manager.with_init(init_conn);
    let pool = Pool::builder()
        .max_size(max)
        .min_idle(Some(1))
        .idle_timeout(None)
        .max_lifetime(None)
        .connection_timeout(Duration::from_secs(10))
        .build(manager)
        .map_err(|e| format!("sqlite pool ({name}): {e}"))?;
    map.insert(name, pool.clone());
    Ok(pool)
}

pub fn ensure_connected() -> Result<(), String> {
    let pool = pool_for_active()?;
    pool.get()
        .map(|_| ())
        .map_err(|e| format!("sqlite connect: {e}"))
}

pub fn has_active_tx() -> bool {
    TX_ACTIVE.get()
}

/// SQLite has exactly one isolation level: serializable. Accept the names the
/// other adapters take so the same Soli code runs unchanged, and reject a name
/// that isn't an isolation level at all.
fn check_isolation(level: Option<&str>) -> Result<(), String> {
    match level
        .unwrap_or("serializable")
        .to_ascii_lowercase()
        .as_str()
    {
        "read_committed" | "read committed" | "repeatable_read" | "repeatable read"
        | "serializable" | "read_uncommitted" | "read uncommitted" => Ok(()),
        other => Err(format!(
            "unsupported isolation level {other:?} for sqlite \
             (SQLite transactions are always serializable)"
        )),
    }
}

pub fn begin_transaction(isolation_level: Option<&str>) -> Result<String, String> {
    check_isolation(isolation_level)?;
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
            return Ok(format!("sql-sqlite-nested-{}", tx.nest));
        }
        let pool = pool_for_active()?;
        let conn = pool
            .get()
            .map_err(|e| format!("sqlite transaction checkout: {e}"))?;
        // IMMEDIATE takes the write lock up front. A DEFERRED transaction that
        // reads first and writes later can fail to upgrade and abort with
        // SQLITE_BUSY even with a busy timeout — a lost transaction rather than
        // a wait.
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| lite_error("sqlite BEGIN", &e))?;
        *slot = Some(TxState {
            conn,
            nest: 0,
            name,
        });
        TX_ACTIVE.set(true);
        Ok("sql-sqlite".into())
    })
}

pub fn commit_transaction() -> Result<(), String> {
    TX.with(|cell| {
        let mut slot = cell.borrow_mut();
        let tx = slot
            .as_mut()
            .ok_or_else(|| "No active sqlite transaction".to_string())?;
        if tx.nest > 0 {
            tx.nest -= 1;
            return Ok(());
        }
        let state = slot.take().expect("tx present");
        TX_ACTIVE.set(false);
        state
            .conn
            .execute_batch("COMMIT")
            .map_err(|e| lite_error("sqlite COMMIT", &e))
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
        let state = slot.take().expect("tx present");
        TX_ACTIVE.set(false);
        state
            .conn
            .execute_batch("ROLLBACK")
            .map_err(|e| lite_error("sqlite ROLLBACK", &e))
    })
}

pub fn clear_transaction() {
    TX.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(state) = slot.take() {
            TX_ACTIVE.set(false);
            let _ = state.conn.execute_batch("ROLLBACK");
        }
    });
}

fn with_conn<T>(f: impl FnOnce(&mut SqConn) -> Result<T, String>) -> Result<T, String> {
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
    // named connection: handing it to another connection's op would run that op
    // against the wrong database file.
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
            let conn: &mut SqConn = &mut state.conn;
            let _guard = ActiveConnGuard::set(conn, name);
            return f(conn);
        }
    }

    let pool = pool_for_active()?;
    let mut conn = pool.get().map_err(|e| format!("sqlite checkout: {e}"))?;
    let _guard = ActiveConnGuard::set(&mut conn, name);
    f(&mut conn)
}

/// Run `f` holding SQLite's write lock, unless a transaction is already open on
/// this connection (then `f` simply joins it).
fn in_write_tx<T>(
    conn: &mut SqConn,
    f: impl FnOnce(&mut SqConn) -> Result<T, String>,
) -> Result<T, String> {
    if !conn.is_autocommit() {
        return f(conn);
    }
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| lite_error("sqlite BEGIN IMMEDIATE", &e))?;
    match f(conn) {
        Ok(value) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| lite_error("sqlite COMMIT", &e))?;
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn to_sqlite_params(params: &[SqlBind]) -> Vec<SqlValue> {
    params
        .iter()
        .map(|p| match p {
            SqlBind::Text(s) => SqlValue::Text(s.clone()),
            SqlBind::I64(n) => SqlValue::Integer(*n),
            SqlBind::F64(f) => SqlValue::Real(*f),
            // SQLite has no boolean type; 0/1 is the documented representation.
            SqlBind::Bool(b) => SqlValue::Integer(i64::from(*b)),
            SqlBind::Json(j) => SqlValue::Text(j.to_string()),
        })
        .collect()
}

/// Convert one column of a result row to JSON, driven by the stored value's own
/// type. SQLite is dynamically typed, so this reads what is actually there.
fn value_to_json(v: ValueRef<'_>) -> serde_json::Value {
    match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(n) => serde_json::json!(n),
        ValueRef::Real(f) => serde_json::json!(f),
        ValueRef::Text(bytes) => {
            serde_json::Value::String(String::from_utf8_lossy(bytes).into_owned())
        }
        // A blob has no JSON form; report its size rather than mangled bytes.
        ValueRef::Blob(b) => serde_json::json!(format!("<blob {} bytes>", b.len())),
    }
}

// ---------- CRUD ----------

/// SELECT one doc on an already-checked-out connection. Write paths use this
/// instead of `get` — a second pool checkout while holding a connection stalls
/// (and on a memory database, which has a single connection, deadlocks).
fn get_on(conn: &mut SqConn, table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    let table_q = Dialect::Sqlite.quote_ident(table)?;
    let sql = format!("SELECT doc FROM {table_q} WHERE _key = ?");
    // Traced here rather than in `get`: this is the per-key lookup a loop turns
    // into an N+1, and write paths read back through it too.
    let _trace = super::trace::start(&sql, &[SqlBind::Text(key.to_string())]);
    let row: Option<String> = conn
        .query_row(&sql, [key], |r| r.get(0))
        .optional()
        .map_err(|e| lite_error("sqlite get", &e))?;
    match row {
        Some(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| format!("sqlite get json: {e}")),
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
    let table_q = Dialect::Sqlite.quote_ident(table)?;
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES (?, json(?)) \
         ON CONFLICT(_key) DO UPDATE SET doc = excluded.doc"
    );
    let doc_str = document.to_string();
    let _trace = super::trace::start_plain(&sql);
    with_conn(|conn| {
        conn.execute(&sql, rusqlite::params![&key, &doc_str])
            .map_err(|e| lite_error("sqlite insert", &e))?;
        get_on(conn, table, &key)?.ok_or_else(|| "sqlite insert: row missing after write".into())
    })
}

/// Insert many documents in one statement per chunk.
pub fn insert_many(table: &str, rows: &[(String, serde_json::Value)]) -> Result<u64, String> {
    ensure_table(table)?;
    let compiled = compile_insert_many_d(Dialect::Sqlite, table, rows)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n = conn
            .execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite insert_many", &e))?;
        Ok(n as u64)
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
    let table_q = Dialect::Sqlite.quote_ident(table)?;
    let doc_str = document.to_string();
    // json_patch is SQLite's RFC 7396 merge: the same semantics as Postgres
    // `||` and MySQL JSON_MERGE_PATCH for a flat patch. `excluded.doc` holds
    // the patch, so one statement covers both the update and the insert.
    let set = if merge {
        "doc = json_patch(COALESCE(doc, '{}'), excluded.doc)"
    } else {
        "doc = excluded.doc"
    };
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES (?, json(?)) \
         ON CONFLICT(_key) DO UPDATE SET {set}"
    );
    let _trace = super::trace::start_plain(&sql);
    with_conn(|conn| {
        conn.execute(&sql, rusqlite::params![key, &doc_str])
            .map_err(|e| lite_error("sqlite update", &e))?;
        get_on(conn, table, key)?.ok_or_else(|| "sqlite update: row missing".into())
    })
}

pub fn delete(table: &str, key: &str) -> Result<(), String> {
    if !table_exists(table)? {
        return Ok(());
    }
    let table_q = Dialect::Sqlite.quote_ident(table)?;
    let sql = format!("DELETE FROM {table_q} WHERE _key = ?");
    let _trace = super::trace::start_plain(&sql);
    with_conn(|conn| {
        conn.execute(&sql, [key])
            .map_err(|e| lite_error("sqlite delete", &e))?;
        Ok(())
    })
}

pub fn select(q: &ListQuery) -> Result<Vec<serde_json::Value>, String> {
    if !table_exists(&q.table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_d(Dialect::Sqlite, q)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    query_docs(&compiled.sql, &compiled.params)
}

pub fn select_by_keys(table: &str, keys: &[String]) -> Result<Vec<serde_json::Value>, String> {
    if keys.is_empty() || !table_exists(table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_by_keys_d(Dialect::Sqlite, table, keys)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
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
    let compiled = compile_select_json_text_in_d(Dialect::Sqlite, table, field, values)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
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
    let compiled = compile_group_by_d(Dialect::Sqlite, q, group_fields, aggs)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    // Column order: group fields, then agg aliases (or "n" when there are none).
    let mut col_names: Vec<String> = group_fields.to_vec();
    if aggs.is_empty() {
        col_names.push("n".into());
    } else {
        for a in aggs {
            col_names.push(a.alias.clone());
        }
    }
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let mut stmt = conn
            .prepare(&compiled.sql)
            .map_err(|e| lite_error("sqlite group_by prepare", &e))?;
        let mut rows = stmt
            .query(params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite group_by", &e))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| lite_error("sqlite group_by row", &e))?
        {
            let mut map = serde_json::Map::new();
            for (i, name) in col_names.iter().enumerate() {
                let value = row.get_ref(i).map(value_to_json).unwrap_or_default();
                map.insert(name.clone(), value);
            }
            out.push(serde_json::Value::Object(map));
        }
        Ok(out)
    })
}

pub fn count(q: &ListQuery) -> Result<i64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_count_d(Dialect::Sqlite, q)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n: Option<i64> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| lite_error("sqlite count", &e))?;
        Ok(n.unwrap_or(0))
    })
}

pub fn exists(q: &ListQuery) -> Result<bool, String> {
    if !table_exists(&q.table)? {
        return Ok(false);
    }
    let compiled = compile_exists_d(Dialect::Sqlite, q)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let hit: Option<i64> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| lite_error("sqlite exists", &e))?;
        Ok(hit.is_some())
    })
}

pub fn aggregate(q: &ListQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    if !table_exists(&q.table)? {
        return Ok(serde_json::Value::Null);
    }
    let compiled = compile_aggregate_d(Dialect::Sqlite, q, func, field)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let value: Option<serde_json::Value> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| {
                Ok(r.get_ref(0).map(value_to_json).unwrap_or_default())
            })
            .optional()
            .map_err(|e| lite_error("sqlite aggregate", &e))?;
        // COUNT of nothing is 0; SUM of nothing is null, and the compiler's
        // aggregate already returns SQL NULL there.
        Ok(match value {
            None if matches!(func, SqlAgg::Count) => serde_json::json!(0),
            None => serde_json::Value::Null,
            Some(v) => v,
        })
    })
}

pub fn delete_all(q: &ListQuery) -> Result<u64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_delete_all_d(Dialect::Sqlite, q)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n = conn
            .execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite delete_all", &e))?;
        Ok(n as u64)
    })
}

pub fn update_all(q: &ListQuery, patch: serde_json::Value) -> Result<u64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_update_all_d(Dialect::Sqlite, q, &patch)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n = conn
            .execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite update_all", &e))?;
        Ok(n as u64)
    })
}

fn query_docs(sql: &str, params: &[SqlBind]) -> Result<Vec<serde_json::Value>, String> {
    with_conn(|conn| {
        let params = to_sqlite_params(params);
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| lite_error("sqlite prepare", &e))?;
        let mut rows = stmt
            .query(params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite query", &e))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| lite_error("sqlite row", &e))? {
            let text: String = row.get(0).map_err(|e| lite_error("sqlite row doc", &e))?;
            out.push(serde_json::from_str(&text).map_err(|e| format!("sqlite row json: {e}"))?);
        }
        Ok(out)
    })
}

// ---------- schema ----------

pub fn ensure_table(table: &str) -> Result<(), String> {
    let ddl = create_table_sql_d(Dialect::Sqlite, table)?;
    with_conn(|conn| {
        conn.execute_batch(&ddl)
            .map_err(|e| lite_error("sqlite ensure_table", &e))
    })
}

pub fn drop_table(table: &str) -> Result<(), String> {
    let ddl = drop_table_sql_d(Dialect::Sqlite, table)?;
    with_conn(|conn| {
        conn.execute_batch(&ddl)
            .map_err(|e| lite_error("sqlite drop_table", &e))
    })
}

pub fn ensure_migrations_table() -> Result<(), String> {
    with_conn(|conn| {
        conn.execute_batch(migrations_table_sql_d(Dialect::Sqlite))
            .map_err(|e| lite_error("sqlite migrations table", &e))
    })
}

pub fn list_applied_migrations() -> Result<Vec<(String, String)>, String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT version, name FROM \"_migrations\" ORDER BY version")
            .map_err(|e| lite_error("sqlite list migrations", &e))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| lite_error("sqlite list migrations", &e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| lite_error("sqlite list migrations row", &e))
    })
}

pub fn record_migration(version: &str, name: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO \"_migrations\" (version, name) VALUES (?, ?)",
            [version, name],
        )
        .map_err(|e| lite_error("sqlite record migration", &e))?;
        Ok(())
    })
}

pub fn remove_migration(version: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        conn.execute("DELETE FROM \"_migrations\" WHERE version = ?", [version])
            .map_err(|e| lite_error("sqlite remove migration", &e))?;
        Ok(())
    })
}

/// Create or drop the database file the connection URL names.
///
/// A SQLite database is a file, so "create" is touching it (the pool would do
/// that anyway) and "drop" is deleting it — along with the `-wal` and `-shm`
/// sidecars, which would otherwise resurrect committed data into the next file
/// of the same name.
pub fn create_or_drop_database(drop: bool) -> Result<String, String> {
    let spec = active_spec()?;
    let url = spec
        .url
        .clone()
        .ok_or_else(|| "connection has no url".to_string())?;
    let Target::File(path) = parse_target(&url) else {
        return Ok("sqlite::memory: needs no file — nothing to do".to_string());
    };

    if drop {
        // Drop the pool first: deleting a file out from under open connections
        // leaves them writing to an unlinked inode.
        if let Ok(mut map) = pools().lock() {
            map.remove(&active_connection_name());
        }
        let mut removed = false;
        for suffix in ["", "-wal", "-shm"] {
            let target = if suffix.is_empty() {
                path.clone()
            } else {
                std::path::PathBuf::from(format!("{}{suffix}", path.display()))
            };
            if target.exists() {
                std::fs::remove_file(&target)
                    .map_err(|e| format!("could not remove {}: {e}", target.display()))?;
                removed |= suffix.is_empty();
            }
        }
        return Ok(if removed {
            format!("dropped sqlite database {}", path.display())
        } else {
            format!("sqlite database {} did not exist", path.display())
        });
    }

    if path.exists() {
        return Ok(format!("sqlite database {} already exists", path.display()));
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    // Opening through the pool creates the file with the right pragmas.
    ensure_connected()?;
    Ok(format!("created sqlite database {}", path.display()))
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
    Dialect::Sqlite.quote_ident(field)?;
    let table_q = Dialect::Sqlite.quote_ident(table)?;
    let sql = format!(
        "UPDATE {table_q} SET doc = json_set(doc, '$.{field}', \
             COALESCE(doc ->> '$.{field}', 0) + ?1) \
         WHERE _key = ?2 RETURNING CAST((doc ->> '$.{field}') AS TEXT)"
    );
    let _trace = super::trace::start_plain(&sql);
    with_conn(|conn| {
        let value: Option<Option<String>> = conn
            .query_row(&sql, rusqlite::params![delta, key], |r| r.get(0))
            .optional()
            .map_err(|e| lite_error("sqlite increment", &e))?;
        Ok(value.flatten().and_then(|t| super::parse_counter(&t)))
    })
}

/// Index names on `table`.
pub fn list_index_names(table: &str) -> Result<Vec<String>, String> {
    with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = ?")
            .map_err(|e| lite_error("sqlite list indexes", &e))?;
        let rows = stmt
            .query_map([table], |r| r.get::<_, String>(0))
            .map_err(|e| lite_error("sqlite list indexes", &e))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| lite_error("sqlite list indexes row", &e))
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
    for sql in super::ddl::doc_index_sql(Dialect::Sqlite, table, fields, name, unique)? {
        execute_ddl(&sql)?;
    }
    Ok(true)
}

/// Full `CREATE` text for user tables and indexes, plus applied versions.
pub fn dump_schema() -> Result<String, String> {
    let versions = list_applied_migrations().unwrap_or_default();
    let stmts = with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT sql FROM sqlite_master \
                 WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
                 ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, name",
            )
            .map_err(|e| format!("sqlite dump: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("sqlite dump: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            let sql = row.map_err(|e| format!("sqlite dump: {e}"))?;
            if !sql.trim().is_empty() {
                out.push(sql);
            }
        }
        Ok(out)
    })?;
    Ok(super::schema_dump::format_dump("sqlite", &versions, &stmts))
}

/// Run compiled DDL (used by migrations and by the column-mode test harness).
pub fn execute_ddl(sql: &str) -> Result<(), String> {
    with_conn(|conn| {
        conn.execute_batch(sql)
            .map_err(|e| lite_error("sqlite ddl", &e))
    })
}

/// Undo session-level changes a raw `execute` may have left (ATTACH, pragma).
fn reset_sqlite_session(conn: &rusqlite::Connection) {
    let _ = conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA writable_schema=OFF;");
    let attached: Vec<String> = conn
        .prepare("PRAGMA database_list")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .ok()
                .map(|rows| rows.filter_map(|n| n.ok()).collect())
        })
        .unwrap_or_default();
    for name in attached {
        if name == "main" || name == "temp" {
            continue;
        }
        if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            let _ = conn.execute_batch(&format!("DETACH DATABASE \"{name}\""));
        }
    }
}

/// `db.execute`: a dedicated file connection so ATTACH / PRAGMA cannot poison
/// the pool. `:memory:` *is* the pool (a second connection is a second empty
/// database), so that path resets the session afterwards instead.
pub fn execute_raw(sql: &str) -> Result<(), String> {
    let spec = active_spec()?;
    let url = spec
        .url
        .as_deref()
        .ok_or_else(|| format!("connection {:?}: url required for sqlite", spec.name))?;
    match parse_target(url) {
        Target::Memory => with_conn(|conn| {
            let result = conn
                .execute_batch(sql)
                .map_err(|e| lite_error("sqlite execute", &e));
            reset_sqlite_session(conn);
            result
        }),
        Target::File(path) => {
            let mut conn = rusqlite::Connection::open(&path)
                .map_err(|e| lite_error("sqlite execute open", &e))?;
            init_conn(&mut conn).map_err(|e| lite_error("sqlite execute init", &e))?;
            conn.execute_batch(sql)
                .map_err(|e| lite_error("sqlite execute", &e))
        }
    }
}

// ---------- column-aware model execution ----------

use super::introspect::{ColType, TableSchema};
use super::sql_columns_compile as cols;

/// Read one row into a JSON object keyed by column name, so downstream
/// hydration is identical to the document path.
fn row_to_json(schema: &TableSchema, columns: &[String], row: &rusqlite::Row) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    let mut idx = 0usize;
    // Walk the columns the SELECT actually returned, in that order — the schema's
    // own order would misalign a projected row.
    for name in columns {
        let Some(col) = schema.column(name) else {
            continue;
        };
        let raw = row.get_ref(idx).unwrap_or(ValueRef::Null);
        let value = match col.ty {
            ColType::Int => match raw {
                ValueRef::Null => serde_json::Value::Null,
                ValueRef::Real(f) => serde_json::json!(f as i64),
                other => match value_to_json(other) {
                    serde_json::Value::String(s) => s
                        .trim()
                        .parse::<i64>()
                        .map(|n| serde_json::json!(n))
                        .unwrap_or(serde_json::Value::Null),
                    v => v,
                },
            },
            ColType::Float => match raw {
                ValueRef::Null => serde_json::Value::Null,
                ValueRef::Integer(n) => serde_json::json!(n as f64),
                other => match value_to_json(other) {
                    serde_json::Value::String(s) => s
                        .trim()
                        .parse::<f64>()
                        .map(|f| serde_json::json!(f))
                        .unwrap_or(serde_json::Value::Null),
                    v => v,
                },
            },
            // SQLite stores booleans as 0/1 integers.
            ColType::Bool => match raw {
                ValueRef::Null => serde_json::Value::Null,
                ValueRef::Integer(n) => serde_json::json!(n != 0),
                ValueRef::Real(f) => serde_json::json!(f != 0.0),
                other => match value_to_json(other) {
                    serde_json::Value::String(s) => {
                        serde_json::json!(matches!(
                            s.trim().to_ascii_lowercase().as_str(),
                            "1" | "true" | "t" | "yes"
                        ))
                    }
                    v => v,
                },
            },
            ColType::Json => match raw {
                ValueRef::Text(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
                other => value_to_json(other),
            },
            // A temporal column may hold text OR a unix timestamp — both are
            // common in SQLite, which enforces neither. Convert a number so
            // `DateTime` fields behave as they do on the other adapters instead
            // of surfacing a bare integer. The select list casts to TEXT, so the
            // digits arrive as a string.
            ColType::Date | ColType::DateTime => match value_to_json(raw) {
                serde_json::Value::String(s) => {
                    let digits = s.trim();
                    match digits.parse::<i64>() {
                        Ok(unix) => serde_json::json!(unix_to_iso(unix)),
                        Err(_) => serde_json::json!(normalize_temporal(col.ty, &s)),
                    }
                }
                serde_json::Value::Number(n) => n
                    .as_i64()
                    .map(|unix| serde_json::json!(unix_to_iso(unix)))
                    .unwrap_or(serde_json::Value::Null),
                v => v,
            },
            // Text, Uuid, and Decimal come through as stored.
            _ => value_to_json(raw),
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

/// SQLite's `CURRENT_TIMESTAMP` writes `2026-08-12 10:00:00` (UTC, no offset);
/// re-emit the RFC 3339 form Soli's DateTime parses. SQLite stores no zone, so
/// UTC is assumed — the documented convention for column-aware models.
fn normalize_temporal(ty: ColType, raw: &str) -> String {
    if ty != ColType::DateTime {
        return raw.to_string();
    }
    let mut out = raw.replacen(' ', "T", 1);
    if !out.ends_with('Z') && !out.contains('+') {
        out.push('Z');
    }
    out
}

/// Unix time (seconds, or milliseconds when the value is too large to be
/// seconds) to RFC 3339.
fn unix_to_iso(value: i64) -> String {
    // 1e11 seconds is the year 5138; a value that big is milliseconds.
    let secs = if value.abs() > 100_000_000_000 {
        value / 1000
    } else {
        value
    };
    crate::jobs::iso_from_unix(secs)
}

/// Run a compiled column query and map its rows.
fn col_rows(
    conn: &mut SqConn,
    schema: &TableSchema,
    columns: &[String],
    sql: &str,
    params: &[SqlBind],
    what: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let params = to_sqlite_params(params);
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| lite_error(&format!("sqlite column {what} prepare"), &e))?;
    let mut rows = stmt
        .query(params_from_iter(params.iter()))
        .map_err(|e| lite_error(&format!("sqlite column {what}"), &e))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| lite_error(&format!("sqlite column {what} row"), &e))?
    {
        out.push(row_to_json(schema, columns, row));
    }
    Ok(out)
}

pub fn col_get(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let compiled = cols::compile_get_cols(Dialect::Sqlite, schema, pk)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let columns = cols::selected_columns(schema, &None)?;
        let rows = col_rows(
            conn,
            schema,
            &columns,
            &compiled.sql,
            &compiled.params,
            "get",
        )?;
        Ok(rows.into_iter().next())
    })
}

pub fn col_insert(
    schema: &std::sync::Arc<TableSchema>,
    doc: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    // SQLite supports RETURNING (3.35+, and this binary bundles a newer one),
    // so the stored row — generated keys and defaults included — comes back
    // from the insert itself.
    let compiled = cols::compile_insert_cols(Dialect::Sqlite, schema, doc)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let columns = cols::selected_columns(schema, &None)?;
        let rows = col_rows(
            conn,
            schema,
            &columns,
            &compiled.sql,
            &compiled.params,
            "insert",
        )?;
        rows.into_iter().next().ok_or_else(|| {
            format!(
                "sqlite column insert: row missing after write in {:?}",
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
    let compiled = cols::compile_update_cols(Dialect::Sqlite, schema, pk, patch)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let columns = cols::selected_columns(schema, &None)?;
        let rows = col_rows(
            conn,
            schema,
            &columns,
            &compiled.sql,
            &compiled.params,
            "update",
        )?;
        rows.into_iter()
            .next()
            .ok_or_else(|| format!("no row in {:?} with {} = {}", schema.table, schema.pk, pk))
    })
}

pub fn col_delete(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<(), String> {
    let compiled = cols::compile_delete_cols(Dialect::Sqlite, schema, pk)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        conn.execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite column delete", &e))?;
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
    let (sql, params) = cols::compile_increment_col(Dialect::Sqlite, schema, pk, column, delta)?;
    with_conn(|conn| {
        let params = to_sqlite_params(&params);
        let value: Option<Option<String>> = conn
            .query_row(&sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| lite_error("sqlite column increment", &e))?;
        Ok(value.flatten().and_then(|t| super::parse_counter(&t)))
    })
}

/// Grouped aggregation over real columns.
pub fn col_group_by(
    q: &cols::ColumnQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    let compiled = cols::compile_group_by_cols(Dialect::Sqlite, q, group_fields, aggs)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    let names = super::columns::group_result_names(group_fields, aggs);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let mut stmt = conn
            .prepare(&compiled.sql)
            .map_err(|e| lite_error("sqlite column group_by prepare", &e))?;
        let mut rows = stmt
            .query(params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite column group_by", &e))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| lite_error("sqlite column group_by row", &e))?
        {
            let texts: Vec<Option<String>> = (0..names.len())
                .map(|i| row.get::<_, Option<String>>(i).ok().flatten())
                .collect();
            out.push(super::columns::group_row_to_json(&names, &texts));
        }
        Ok(out)
    })
}

pub fn col_delete_all(q: &cols::ColumnQuery) -> Result<u64, String> {
    let compiled = cols::compile_delete_all_cols(Dialect::Sqlite, q)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n = conn
            .execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite column delete_all", &e))?;
        Ok(n as u64)
    })
}

pub fn col_update_all(q: &cols::ColumnQuery, patch: &serde_json::Value) -> Result<u64, String> {
    let compiled = cols::compile_update_all_cols(Dialect::Sqlite, q, patch)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n = conn
            .execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| lite_error("sqlite column update_all", &e))?;
        Ok(n as u64)
    })
}

/// Run a caller-supplied `SELECT` and return its rows as JSON. See the Postgres
/// twin for the single-`doc`-column convention.
pub fn query_raw(sql: &str, params: &[SqlBind]) -> Result<Vec<serde_json::Value>, String> {
    let _trace = super::trace::start(sql, params);
    with_conn(|conn| {
        let bound = to_sqlite_params(params);
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| lite_error("sqlite raw query prepare", &e))?;
        let names: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut rows = stmt
            .query(params_from_iter(bound.iter()))
            .map_err(|e| lite_error("sqlite raw query", &e))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| lite_error("sqlite raw query row", &e))?
        {
            if names.len() == 1 && names[0] == "doc" {
                let text: Option<String> = row.get(0).ok();
                out.push(match text {
                    Some(t) => serde_json::from_str(&t).unwrap_or(serde_json::Value::Null),
                    None => serde_json::Value::Null,
                });
                continue;
            }
            let mut map = serde_json::Map::new();
            for (index, name) in names.iter().enumerate() {
                let value = row.get_ref(index).map(value_to_json).unwrap_or_default();
                map.insert(name.clone(), value);
            }
            out.push(serde_json::Value::Object(map));
        }
        Ok(out)
    })
}

pub fn col_select(q: &cols::ColumnQuery) -> Result<Vec<serde_json::Value>, String> {
    let compiled = cols::compile_select_cols(Dialect::Sqlite, q)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    let columns = cols::selected_columns(&q.schema, &q.select_fields)?;
    with_conn(|conn| {
        col_rows(
            conn,
            &q.schema,
            &columns,
            &compiled.sql,
            &compiled.params,
            "select",
        )
    })
}

pub fn col_count(q: &cols::ColumnQuery) -> Result<i64, String> {
    let compiled = cols::compile_count_cols(Dialect::Sqlite, q)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n: Option<i64> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| lite_error("sqlite column count", &e))?;
        Ok(n.unwrap_or(0))
    })
}

pub fn col_exists(q: &cols::ColumnQuery) -> Result<bool, String> {
    let compiled = cols::compile_exists_cols(Dialect::Sqlite, q)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let hit: Option<i64> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| lite_error("sqlite column exists", &e))?;
        Ok(hit.is_some())
    })
}

pub fn col_aggregate(
    q: &cols::ColumnQuery,
    func: SqlAgg,
    field: &str,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_aggregate_cols(Dialect::Sqlite, q, func, field)?;
    let _trace = super::trace::start(&compiled.sql, &compiled.params);
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        if func == SqlAgg::Count {
            let n: Option<i64> = conn
                .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
                .optional()
                .map_err(|e| lite_error("sqlite column aggregate", &e))?;
            return Ok(serde_json::json!(n.unwrap_or(0)));
        }
        let raw: Option<Option<String>> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| lite_error("sqlite column aggregate", &e))?;
        Ok(super::columns::parse_agg_text(raw.flatten()))
    })
}

// ---------- column-aware model introspection ----------

/// Read the shape of an existing table for column mode.
///
/// SQLite keeps no `information_schema`; `PRAGMA table_info` is the equivalent.
/// The declared type is what it reports — SQLite applies type *affinity* rather
/// than enforcing types, so the declaration is the only schema-level signal
/// available. `AUTOINCREMENT` needs a second lookup: only the original DDL in
/// `sqlite_master` records it, and a plain `INTEGER PRIMARY KEY` generates keys
/// too (it aliases the rowid).
pub fn introspect_table(table: &str) -> Result<super::introspect::RawColumns, String> {
    with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?) ORDER BY cid")
            .map_err(|e| lite_error("sqlite introspect prepare", &e))?;
        let rows = stmt
            .query_map([table], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| lite_error("sqlite introspect columns", &e))?;

        let mut columns = Vec::new();
        // `pk` is the 1-based position in the key, so sort by it to keep key
        // order (a composite key is rejected later, but the order is reported
        // in the error).
        let mut pk_ordered: Vec<(i64, String)> = Vec::new();
        for row in rows {
            let (name, declared, notnull, pk_pos) =
                row.map_err(|e| lite_error("sqlite introspect row", &e))?;
            if pk_pos > 0 {
                pk_ordered.push((pk_pos, name.clone()));
            }
            columns.push((name, declared, String::new(), notnull == 0, false));
        }
        pk_ordered.sort_by_key(|(pos, _)| *pos);
        let pk: Vec<String> = pk_ordered.into_iter().map(|(_, name)| name).collect();

        // A single INTEGER primary key IS the rowid, so SQLite generates it —
        // with or without AUTOINCREMENT. Any other key type must be supplied.
        if pk.len() == 1 {
            let is_integer_key = columns
                .iter()
                .find(|(name, ..)| *name == pk[0])
                .map(|(_, declared, ..)| declared.trim().eq_ignore_ascii_case("integer"))
                .unwrap_or(false);
            if is_integer_key {
                for col in columns.iter_mut() {
                    if col.0 == pk[0] {
                        col.4 = true;
                    }
                }
            }
        }

        Ok(super::introspect::RawColumns { columns, pk })
    })
}

/// Map a SQLite declared type to a [`ColType`] using SQLite's own affinity
/// rules, refined for the names Soli can round-trip more precisely.
///
/// Affinity is substring-based in SQLite (`"POINT"` really does get INTEGER
/// affinity because it contains `"INT"`), so the order of these checks matters.
/// The Soli-specific names — BOOLEAN, DATE/DATETIME, JSON, UUID — are tested
/// first, because affinity alone would hide them.
pub fn sqlite_coltype(declared: &str) -> ColType {
    let ty = declared.trim().to_ascii_uppercase();
    // No declared type at all: the column holds whatever was written. Text is
    // the tolerant choice — the reader reports each value by its real type.
    if ty.is_empty() {
        return ColType::Text;
    }
    if ty.contains("BOOL") {
        return ColType::Bool;
    }
    if ty.contains("DATETIME") || ty.contains("TIMESTAMP") {
        return ColType::DateTime;
    }
    if ty.contains("DATE") {
        return ColType::Date;
    }
    if ty.contains("JSON") {
        return ColType::Json;
    }
    if ty.contains("UUID") || ty.contains("GUID") {
        return ColType::Uuid;
    }
    // SQLite's documented affinity order from here on.
    if ty.contains("INT") {
        return ColType::Int;
    }
    if ty.contains("CHAR") || ty.contains("CLOB") || ty.contains("TEXT") {
        return ColType::Text;
    }
    if ty.contains("BLOB") {
        return ColType::Unknown;
    }
    if ty.contains("REAL") || ty.contains("FLOA") || ty.contains("DOUB") {
        return ColType::Float;
    }
    if ty.contains("DEC") || ty.contains("NUMERIC") || ty.contains("MONEY") {
        return ColType::Decimal;
    }
    // NUMERIC affinity is SQLite's fallback, but the value could be text just
    // as easily; read it as stored rather than forcing a number.
    ColType::Text
}

// ---------- Soli job engine ----------

/// Atomically claim up to `batch` due jobs from `_jobs`.
///
/// SQLite has no `SKIP LOCKED`, so exclusivity comes from the database write
/// lock: `BEGIN IMMEDIATE` blocks every other claimer for the (very short)
/// duration of the select-then-update. The lease-reclaim clause recovers jobs
/// whose worker died holding a lease.
pub fn claim_jobs(
    now_iso: &str,
    worker_id: &str,
    locked_until_iso: &str,
    batch: usize,
) -> Result<Vec<serde_json::Value>, String> {
    if batch == 0 || !table_exists("_jobs")? {
        return Ok(Vec::new());
    }
    with_conn(|conn| {
        in_write_tx(conn, |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT _key FROM \"_jobs\" \
                     WHERE ((doc ->> '$.state') IN ('pending','scheduled','failed') \
                            AND (doc ->> '$.run_at') <= ?1) \
                        OR ((doc ->> '$.state') = 'running' \
                            AND (doc ->> '$.locked_until') < ?1) \
                     ORDER BY COALESCE(doc ->> '$.priority', 0) DESC, \
                              (doc ->> '$.run_at') ASC \
                     LIMIT ?2",
                )
                .map_err(|e| lite_error("sqlite claim_jobs prepare", &e))?;
            let keys: Vec<String> = stmt
                .query_map(rusqlite::params![now_iso, batch as i64], |r| r.get(0))
                .map_err(|e| lite_error("sqlite claim_jobs select", &e))?
                .collect::<rusqlite::Result<Vec<String>>>()
                .map_err(|e| lite_error("sqlite claim_jobs row", &e))?;
            drop(stmt);

            let mut claimed = Vec::with_capacity(keys.len());
            for key in keys {
                let doc: Option<String> = conn
                    .query_row(
                        "UPDATE \"_jobs\" SET doc = json_set(doc, \
                             '$.state', 'running', \
                             '$.locked_by', ?2, \
                             '$.locked_until', ?3, \
                             '$.attempts', COALESCE(doc ->> '$.attempts', 0) + 1) \
                         WHERE _key = ?1 RETURNING doc",
                        rusqlite::params![&key, worker_id, locked_until_iso],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(|e| lite_error("sqlite claim_jobs update", &e))?;
                if let Some(text) = doc {
                    claimed.push(
                        serde_json::from_str(&text)
                            .map_err(|e| format!("sqlite claim_jobs json: {e}"))?,
                    );
                }
            }
            Ok(claimed)
        })
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
        let changed = conn
            .execute(
                "UPDATE \"_cron_jobs\" SET doc = json_patch(doc, json(?2)) \
                 WHERE _key = ?1 AND (doc ->> '$.next_run_at') = ?3",
                rusqlite::params![key, &patch_str, expected_next_run_at],
            )
            .map_err(|e| lite_error("sqlite claim_cron_slot", &e))?;
        Ok(changed == 1)
    })
}

fn table_exists(table: &str) -> Result<bool, String> {
    with_conn(|conn| {
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?",
                [table],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| lite_error("sqlite table_exists", &e))?;
        Ok(found.is_some())
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
mod tests {
    use super::*;

    #[test]
    fn parses_every_url_form() {
        assert_eq!(
            parse_target("sqlite://db/app.sqlite3"),
            Target::File("db/app.sqlite3".into())
        );
        assert_eq!(
            parse_target("sqlite:///var/data/app.db"),
            Target::File("/var/data/app.db".into())
        );
        assert_eq!(parse_target("sqlite:app.db"), Target::File("app.db".into()));
        assert_eq!(
            parse_target("sqlite3://app.db"),
            Target::File("app.db".into())
        );
        assert_eq!(parse_target("./app.db"), Target::File("./app.db".into()));
        // A query suffix is not part of the path.
        assert_eq!(
            parse_target("sqlite://app.db?mode=rwc"),
            Target::File("app.db".into())
        );
        assert_eq!(parse_target("sqlite::memory:"), Target::Memory);
        assert_eq!(parse_target(":memory:"), Target::Memory);
        assert_eq!(parse_target("sqlite://"), Target::Memory);
    }

    #[test]
    fn affinity_rules_map_declared_types() {
        assert_eq!(sqlite_coltype("INTEGER"), ColType::Int);
        assert_eq!(sqlite_coltype("bigint"), ColType::Int);
        assert_eq!(sqlite_coltype("TEXT"), ColType::Text);
        assert_eq!(sqlite_coltype("VARCHAR(255)"), ColType::Text);
        assert_eq!(sqlite_coltype("REAL"), ColType::Float);
        assert_eq!(sqlite_coltype("DOUBLE PRECISION"), ColType::Float);
        assert_eq!(sqlite_coltype("DECIMAL(10,2)"), ColType::Decimal);
        assert_eq!(sqlite_coltype("BLOB"), ColType::Unknown);
        // Names SQLite would resolve by affinity alone, which Soli reads more
        // precisely.
        assert_eq!(sqlite_coltype("BOOLEAN"), ColType::Bool);
        assert_eq!(sqlite_coltype("DATETIME"), ColType::DateTime);
        assert_eq!(sqlite_coltype("TIMESTAMP"), ColType::DateTime);
        assert_eq!(sqlite_coltype("DATE"), ColType::Date);
        assert_eq!(sqlite_coltype("JSON"), ColType::Json);
        assert_eq!(sqlite_coltype("uuid"), ColType::Uuid);
        // A column declared with no type holds anything.
        assert_eq!(sqlite_coltype(""), ColType::Text);
    }

    #[test]
    fn integer_timestamps_convert_to_rfc3339() {
        assert_eq!(unix_to_iso(0), "1970-01-01T00:00:00Z");
        // Milliseconds are recognised by magnitude.
        assert_eq!(unix_to_iso(1_000_000_000), unix_to_iso(1_000_000_000_000));
    }

    /// Close and forget the pool for `name`, so the next test that reuses the
    /// name opens its own file rather than inheriting this one's connections.
    fn drop_pool_for_tests(name: &str) {
        if let Ok(mut map) = pools().lock() {
            map.remove(name);
        }
    }

    /// Run `f` against a private SQLite file. Unlike the Postgres and MySQL
    /// suites, these tests need no server — SQLite is a file, so they always
    /// run.
    ///
    /// Each test gets its own connection NAME as well as its own file: pools are
    /// cached per connection name, so sharing a name would hand the second test
    /// the first test's (deleted) database.
    fn with_sqlite(name: &str, f: impl FnOnce()) {
        use crate::db::registry::{
            clear_registry_override, registry_test_lock, set_registry_for_tests,
            ConnectionRegistry, ConnectionSpec,
        };
        use crate::db::Adapter;

        // The registry override is process-global, so every module that
        // installs one serializes on this mutex.
        let _guard = registry_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!("soli-sqlite-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("app.db");

        let mut connections = HashMap::new();
        connections.insert(
            name.to_string(),
            ConnectionSpec {
                name: name.to_string(),
                adapter: Adapter::Sqlite,
                url: Some(format!("sqlite://{}", path.display())),
                solidb_host: None,
                solidb_database: None,
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: Some(3),
            },
        );
        set_registry_for_tests(ConnectionRegistry {
            default: name.to_string(),
            connections,
            from_file: false,
        });

        // Clear on unwind too — a panicking test must not leak its override or
        // its pool into whichever test takes the lock next.
        struct Cleanup {
            name: String,
            dir: std::path::PathBuf,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                clear_registry_override();
                crate::db::introspect::clear_schema_cache();
                drop_pool_for_tests(&self.name);
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
        let _cleanup = Cleanup {
            name: name.to_string(),
            dir: dir.clone(),
        };

        f();
    }

    fn list_query(table: &str, eq: &[(&str, serde_json::Value)]) -> ListQuery {
        use super::super::sql_compile::SoftDeleteMode;
        use std::collections::BTreeMap;
        let mut eq_filters = BTreeMap::new();
        let mut clauses = Vec::new();
        for (field, value) in eq {
            eq_filters.insert((*field).to_string(), value.clone());
            clauses.push(format!("doc.{field} == @{field}"));
        }
        ListQuery {
            table: table.into(),
            eq_filters,
            hash_filter: None,
            filter_sdbql: if clauses.is_empty() {
                None
            } else {
                Some(clauses.join(" AND "))
            },
            having: None,
            exists_filters: Vec::new(),
            soft_delete: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            order_field: None,
            order_desc: false,
            limit: None,
            offset: None,
        }
    }

    #[test]
    fn document_crud_roundtrip() {
        with_sqlite("crud", || {
            ensure_connected().expect("connect");
            let table = "people";
            // A read before the first write must not fail on a missing table.
            assert!(get(table, "k1").unwrap().is_none());
            assert_eq!(select(&list_query(table, &[])).unwrap().len(), 0);

            let inserted = insert(
                table,
                Some("k1"),
                serde_json::json!({"name": "Ada", "status": "up"}),
            )
            .expect("insert");
            assert_eq!(inserted["name"], "Ada");
            assert_eq!(inserted["_key"], "k1");

            // Merge keeps the untouched fields.
            let merged =
                update(table, "k1", serde_json::json!({"status": "late"}), true).expect("merge");
            assert_eq!(merged["status"], "late");
            assert_eq!(merged["name"], "Ada");

            // Replace drops them.
            let replaced =
                update(table, "k1", serde_json::json!({"name": "Grace"}), false).expect("replace");
            assert!(replaced.get("status").is_none());
            assert_eq!(replaced["name"], "Grace");

            // A merge on an absent key inserts the patch.
            let created =
                update(table, "k2", serde_json::json!({"name": "Alan"}), true).expect("upsert");
            assert_eq!(created["name"], "Alan");

            delete(table, "k1").expect("delete");
            assert!(get(table, "k1").unwrap().is_none());
            assert!(get(table, "k2").unwrap().is_some());
        });
    }

    #[test]
    fn filters_aggregates_and_bulk_writes() {
        with_sqlite("query", || {
            ensure_connected().expect("connect");
            let table = "orders";
            for (key, amount, status) in [("a", 10, "open"), ("b", 5, "open"), ("c", 99, "closed")]
            {
                insert(
                    table,
                    Some(key),
                    serde_json::json!({"amount": amount, "status": status}),
                )
                .expect("insert");
            }

            let open = list_query(table, &[("status", serde_json::json!("open"))]);
            assert_eq!(count(&open).unwrap(), 2);
            assert!(exists(&open).unwrap());
            assert_eq!(select(&open).unwrap().len(), 2);
            assert_eq!(
                aggregate(&open, SqlAgg::Sum, "amount")
                    .unwrap()
                    .as_f64()
                    .unwrap(),
                15.0
            );
            assert_eq!(
                aggregate(&open, SqlAgg::Count, "amount").unwrap(),
                serde_json::json!(2)
            );

            // A JSON null must match `IS NULL`, not compare equal to a value.
            insert(table, Some("d"), serde_json::json!({"status": null})).expect("insert null");
            let nulls = list_query(table, &[("status", serde_json::Value::Null)]);
            assert_eq!(count(&nulls).unwrap(), 1);

            assert_eq!(
                update_all(&open, serde_json::json!({"tag": "x"})).unwrap(),
                2
            );
            let tagged = select(&open).unwrap();
            assert!(tagged.iter().all(|d| d["tag"] == "x"));
            // The merge patch must not have replaced the rest of the document.
            assert!(tagged.iter().all(|d| d["amount"].is_number()));

            assert_eq!(delete_all(&open).unwrap(), 2);
            assert!(get(table, "c").unwrap().is_some());

            // An empty result set is null for SUM, and 0 for COUNT.
            let gone = list_query(table, &[("status", serde_json::json!("open"))]);
            assert!(aggregate(&gone, SqlAgg::Sum, "amount").unwrap().is_null());
            assert_eq!(
                aggregate(&gone, SqlAgg::Count, "amount").unwrap(),
                serde_json::json!(0)
            );
        });
    }

    #[test]
    fn transactions_commit_and_roll_back() {
        with_sqlite("tx", || {
            ensure_connected().expect("connect");
            let table = "notes";
            ensure_table(table).expect("table");

            begin_transaction(None).expect("begin");
            insert(table, Some("keep"), serde_json::json!({"n": 1})).expect("insert");
            commit_transaction().expect("commit");
            assert!(get(table, "keep").unwrap().is_some());

            begin_transaction(Some("serializable")).expect("begin");
            insert(table, Some("drop"), serde_json::json!({"n": 2})).expect("insert");
            rollback_transaction().expect("rollback");
            assert!(
                get(table, "drop").unwrap().is_none(),
                "rolled-back write must not survive"
            );

            // Nesting is counted, so the outer commit is the real one.
            begin_transaction(None).expect("begin");
            begin_transaction(None).expect("nested begin");
            insert(table, Some("nested"), serde_json::json!({"n": 3})).expect("insert");
            commit_transaction().expect("inner commit");
            assert!(has_active_tx(), "inner commit only unwinds the nesting");
            commit_transaction().expect("outer commit");
            assert!(!has_active_tx());
            assert!(get(table, "nested").unwrap().is_some());

            assert!(begin_transaction(Some("snapshot")).is_err());
        });
    }

    #[test]
    fn migrations_are_recorded_once() {
        with_sqlite("migrations", || {
            ensure_connected().expect("connect");
            ensure_migrations_table().expect("table");
            record_migration("20990101000001", "sqlite_test").expect("record");
            // Recording twice must not fail or duplicate.
            record_migration("20990101000001", "sqlite_test").expect("record again");
            let applied = list_applied_migrations().expect("list");
            assert_eq!(
                applied
                    .iter()
                    .filter(|(v, _)| v == "20990101000001")
                    .count(),
                1
            );
            remove_migration("20990101000001").expect("remove");
            assert!(list_applied_migrations()
                .unwrap()
                .iter()
                .all(|(v, _)| v != "20990101000001"));
        });
    }

    // ---------- column mode ----------

    const ORDERS_DDL: &str = "CREATE TABLE orders (\
         id INTEGER PRIMARY KEY, \
         code TEXT NOT NULL, \
         qty INTEGER, \
         ratio REAL, \
         amount DECIMAL(10,2), \
         paid BOOLEAN, \
         meta JSON, \
         note TEXT, \
         shipped_at DATETIME, \
         created_at DATETIME, \
         updated_at DATETIME)";

    fn orders_schema() -> std::sync::Arc<TableSchema> {
        let raw = introspect_table("orders").expect("introspect");
        std::sync::Arc::new(
            super::super::introspect::build_schema("primary", "orders", raw, |t, _| {
                sqlite_coltype(t)
            })
            .expect("schema"),
        )
    }

    #[test]
    fn introspection_reads_the_real_schema() {
        with_sqlite("introspect", || {
            execute_ddl(ORDERS_DDL).expect("ddl");
            let schema = orders_schema();
            assert_eq!(schema.pk, "id");
            assert_eq!(schema.pk_type, ColType::Int);
            // `INTEGER PRIMARY KEY` aliases the rowid, so SQLite generates it.
            assert!(schema.pk_auto);
            assert!(schema.has_created_at && schema.has_updated_at);
            assert_eq!(schema.column("code").unwrap().ty, ColType::Text);
            assert!(!schema.column("code").unwrap().nullable);
            assert!(schema.column("note").unwrap().nullable);
            assert_eq!(schema.column("qty").unwrap().ty, ColType::Int);
            assert_eq!(schema.column("ratio").unwrap().ty, ColType::Float);
            assert_eq!(schema.column("amount").unwrap().ty, ColType::Decimal);
            assert_eq!(schema.column("paid").unwrap().ty, ColType::Bool);
            assert_eq!(schema.column("meta").unwrap().ty, ColType::Json);
            assert_eq!(schema.column("shipped_at").unwrap().ty, ColType::DateTime);

            // A text primary key is NOT generated — the caller must supply it.
            execute_ddl("CREATE TABLE tokens (token TEXT PRIMARY KEY, used BOOLEAN)").expect("ddl");
            let raw = introspect_table("tokens").expect("introspect");
            let tokens =
                super::super::introspect::build_schema("primary", "tokens", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect("schema");
            assert_eq!(tokens.pk, "token");
            assert!(!tokens.pk_auto);

            // A composite key is refused with a clear message rather than
            // silently using the first column.
            execute_ddl("CREATE TABLE memberships (a INTEGER, b INTEGER, PRIMARY KEY (a, b))")
                .expect("ddl");
            let raw = introspect_table("memberships").expect("introspect");
            let err =
                super::super::introspect::build_schema("primary", "memberships", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect_err("composite key must be rejected");
            assert!(err.contains("composite"), "{err}");

            // A missing table names the table rather than failing later.
            let raw = introspect_table("nope").expect("introspect");
            let err = super::super::introspect::build_schema("primary", "nope", raw, |t, _| {
                sqlite_coltype(t)
            })
            .expect_err("missing table must be rejected");
            assert!(err.contains("not found"), "{err}");
        });
    }

    #[test]
    fn column_mode_crud_and_types_roundtrip() {
        with_sqlite("columns", || {
            execute_ddl(ORDERS_DDL).expect("ddl");
            let schema = orders_schema();

            let mut doc = serde_json::json!({
                "code": "A-1",
                "qty": 3,
                "ratio": 0.5,
                "amount": "19.99",
                "paid": true,
                "meta": {"gift": true, "tags": ["x"]},
                "shipped_at": "2026-08-12T10:00:00Z"
            });
            super::super::columns::apply_timestamps(&schema, &mut doc, true);
            let row = col_insert(&schema, &doc).expect("insert");

            // The generated key comes back from RETURNING, not a second query.
            assert_eq!(row["id"], serde_json::json!(1));
            assert_eq!(row["code"], "A-1");
            assert_eq!(row["qty"], serde_json::json!(3));
            assert_eq!(row["ratio"], serde_json::json!(0.5));
            // An exact numeric keeps its scale: it travels as text.
            assert_eq!(row["amount"], "19.99");
            assert_eq!(row["paid"], serde_json::json!(true));
            assert_eq!(row["meta"]["gift"], serde_json::json!(true));
            assert_eq!(row["shipped_at"], "2026-08-12T10:00:00Z");
            assert!(row["note"].is_null());
            // The key is mirrored as a string for the instance plumbing.
            assert_eq!(row["_key"], "1");

            let fetched = col_get(&schema, &serde_json::json!(1))
                .expect("get")
                .expect("row");
            assert_eq!(fetched["code"], "A-1");
            assert!(col_get(&schema, &serde_json::json!(999)).unwrap().is_none());

            let updated = col_update(
                &schema,
                &serde_json::json!(1),
                &serde_json::json!({"note": "packed", "paid": false}),
            )
            .expect("update");
            assert_eq!(updated["note"], "packed");
            assert_eq!(updated["paid"], serde_json::json!(false));
            // The primary key is identity, never a field to rewrite.
            assert_eq!(updated["id"], serde_json::json!(1));

            col_delete(&schema, &serde_json::json!(1)).expect("delete");
            assert!(col_get(&schema, &serde_json::json!(1)).unwrap().is_none());
        });
    }

    #[test]
    fn column_mode_queries_filter_order_and_aggregate() {
        with_sqlite("column-queries", || {
            execute_ddl(ORDERS_DDL).expect("ddl");
            let schema = orders_schema();
            for (code, qty, amount, paid, note) in [
                ("A", 1, "10.00", true, Some("first")),
                ("B", 2, "5.50", true, None),
                ("C", 3, "99.00", false, Some("third")),
            ] {
                let doc = serde_json::json!({
                    "code": code, "qty": qty, "amount": amount,
                    "paid": paid, "note": note
                });
                col_insert(&schema, &doc).expect("insert");
            }

            let mut paid_query = cols::ColumnQuery::new(schema.clone());
            paid_query
                .eq_filters
                .insert("paid".into(), serde_json::json!(true));
            assert_eq!(col_count(&paid_query).unwrap(), 2);
            assert!(col_exists(&paid_query).unwrap());
            let rows = col_select(&paid_query).expect("select");
            assert_eq!(rows.len(), 2);

            // Exact numerics aggregate without an f64 detour.
            assert_eq!(
                col_aggregate(&paid_query, SqlAgg::Sum, "amount")
                    .unwrap()
                    .as_f64()
                    .unwrap(),
                15.5
            );
            assert_eq!(
                col_aggregate(&paid_query, SqlAgg::Count, "amount").unwrap(),
                serde_json::json!(2)
            );

            // A null filter is `IS NULL`, and `id` aliases the primary key.
            let mut null_note = cols::ColumnQuery::new(schema.clone());
            null_note
                .eq_filters
                .insert("note".into(), serde_json::Value::Null);
            assert_eq!(col_count(&null_note).unwrap(), 1);
            assert_eq!(col_select(&null_note).unwrap()[0]["code"], "B");

            // Order, limit, and offset.
            let mut ordered = cols::ColumnQuery::new(schema.clone());
            ordered.order_field = Some("qty".into());
            ordered.order_desc = true;
            ordered.limit = Some(2);
            let desc = col_select(&ordered).expect("ordered");
            assert_eq!(desc.len(), 2);
            assert_eq!(desc[0]["code"], "C");
            assert_eq!(desc[1]["code"], "B");

            // Offset with no limit is `LIMIT -1 OFFSET n` on SQLite — no
            // invented row ceiling.
            let mut skipped = cols::ColumnQuery::new(schema.clone());
            skipped.order_field = Some("qty".into());
            skipped.offset = Some(1);
            let tail = col_select(&skipped).expect("offset");
            assert_eq!(tail.len(), 2);
            assert_eq!(tail[0]["code"], "B");

            // An unknown field is refused before any SQL is built, and the
            // error lists what is available.
            let mut bogus = cols::ColumnQuery::new(schema.clone());
            bogus.eq_filters.insert("nope".into(), serde_json::json!(1));
            let err = col_select(&bogus).expect_err("unknown column");
            assert!(err.contains("nope") && err.contains("code"), "{err}");

            // An aggregate over a text column is a named error, not SQL noise.
            let err = col_aggregate(&paid_query, SqlAgg::Avg, "code").expect_err("text avg");
            assert!(err.contains("code"), "{err}");
        });
    }

    #[test]
    fn column_mode_writes_join_an_open_transaction() {
        with_sqlite("column-tx", || {
            execute_ddl(ORDERS_DDL).expect("ddl");
            let schema = orders_schema();

            begin_transaction(None).expect("begin");
            col_insert(&schema, &serde_json::json!({"code": "rolled-back"})).expect("insert");
            rollback_transaction().expect("rollback");

            let all = cols::ColumnQuery::new(schema.clone());
            assert_eq!(
                col_count(&all).unwrap(),
                0,
                "a column write inside a rolled-back transaction must not persist"
            );
        });
    }

    #[test]
    fn integer_and_text_timestamp_columns_both_read_as_dates() {
        with_sqlite("temporal", || {
            // SQLite enforces no types, so a DATETIME column may hold text OR a
            // unix timestamp. Both must read back as a parseable date.
            execute_ddl("CREATE TABLE events (id INTEGER PRIMARY KEY, at DATETIME, on_day DATE)")
                .expect("ddl");
            execute_ddl(
                "INSERT INTO events (id, at, on_day) VALUES \
                 (1, '2026-08-12 10:00:00', '2026-08-12'), \
                 (2, 1000000000, '2026-08-13')",
            )
            .expect("insert");
            let raw = introspect_table("events").expect("introspect");
            let schema = std::sync::Arc::new(
                super::super::introspect::build_schema("primary", "events", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect("schema"),
            );
            let mut ordered = cols::ColumnQuery::new(schema);
            ordered.order_field = Some("id".into());
            let rows = col_select(&ordered).expect("select");
            // Stored text gains the UTC marker Soli's DateTime parses.
            assert_eq!(rows[0]["at"], "2026-08-12T10:00:00Z");
            assert_eq!(rows[0]["on_day"], "2026-08-12");
            // A stored integer is read as unix time.
            assert_eq!(rows[1]["at"], "2001-09-09T01:46:40Z");
        });
    }

    /// A table created by the portable DDL compiler must be one column mode can
    /// map — that is the contract that makes migrations and column-aware models
    /// two halves of the same feature.
    #[test]
    fn a_migration_created_table_round_trips_through_column_mode() {
        use super::super::ddl;

        with_sqlite("ddl-roundtrip", || {
            let spec = ddl::parse_table_spec(
                "invoices",
                &[
                    ("id".to_string(), serde_json::json!("pk")),
                    (
                        "code".to_string(),
                        serde_json::json!({ "type": "string", "limit": 32, "null": false }),
                    ),
                    ("total".to_string(), serde_json::json!("decimal(10,2)")),
                    ("qty".to_string(), serde_json::json!("integer")),
                    (
                        "paid".to_string(),
                        serde_json::json!({ "type": "boolean", "default": false }),
                    ),
                    ("meta".to_string(), serde_json::json!("json")),
                    ("due_on".to_string(), serde_json::json!("date")),
                    ("timestamps".to_string(), serde_json::json!(true)),
                ],
            )
            .expect("spec");
            let sql = ddl::create_table_sql(Dialect::Sqlite, &spec).expect("ddl");
            execute_ddl(&sql).expect("create");
            // Creating twice is a no-op, so a re-run cannot fail a migration.
            execute_ddl(&sql).expect("idempotent");

            let raw = introspect_table("invoices").expect("introspect");
            let schema = std::sync::Arc::new(
                super::super::introspect::build_schema("primary", "invoices", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect("schema"),
            );

            // Every declared type came back as the Soli type it was written as.
            assert_eq!(schema.pk, "id");
            assert!(schema.pk_auto, "an INTEGER PRIMARY KEY is generated");
            assert_eq!(schema.column("code").unwrap().ty, ColType::Text);
            assert!(!schema.column("code").unwrap().nullable);
            assert_eq!(schema.column("total").unwrap().ty, ColType::Decimal);
            assert_eq!(schema.column("qty").unwrap().ty, ColType::Int);
            assert_eq!(schema.column("paid").unwrap().ty, ColType::Bool);
            assert_eq!(schema.column("meta").unwrap().ty, ColType::Json);
            assert_eq!(schema.column("due_on").unwrap().ty, ColType::Date);
            assert!(schema.has_created_at && schema.has_updated_at);

            // And the table is writable through the column path.
            let row = col_insert(
                &schema,
                &serde_json::json!({
                    "code": "INV-1", "total": "99.90", "qty": 2,
                    "paid": true, "meta": {"po": "A"}, "due_on": "2026-09-01"
                }),
            )
            .expect("insert");
            assert_eq!(row["id"], serde_json::json!(1));
            // "99.90" comes back as "99.9": a SQLite DECIMAL column has NUMERIC
            // *affinity*, not an exact numeric type, so the value is stored as a
            // REAL and the trailing zero is not part of it. Postgres `numeric`
            // and MySQL `decimal` do keep the scale. Declare the column TEXT if
            // exact decimal text matters on SQLite.
            assert_eq!(row["total"], "99.9");
            assert_eq!(row["paid"], serde_json::json!(true));
            assert_eq!(row["meta"]["po"], "A");
            // The database default filled the timestamps the migration declared.
            assert!(row["created_at"].is_string(), "{row}");

            // A column added later is visible after the cache is dropped, the
            // way a re-boot or a dev reload picks up an altered table.
            let note = ddl::parse_column("note", &serde_json::json!({ "type": "text" })).unwrap();
            execute_ddl(&ddl::add_column_sql(Dialect::Sqlite, "invoices", &note).unwrap())
                .expect("add column");
            execute_ddl(
                &ddl::add_index_sql(
                    Dialect::Sqlite,
                    "invoices",
                    &["code".to_string()],
                    None,
                    true,
                )
                .unwrap(),
            )
            .expect("add index");
            super::super::introspect::clear_schema_cache();

            let raw = introspect_table("invoices").expect("introspect");
            let schema =
                super::super::introspect::build_schema("primary", "invoices", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect("schema");
            assert_eq!(schema.column("note").unwrap().ty, ColType::Text);
            assert!(schema.column("note").unwrap().nullable);
        });
    }

    #[test]
    fn execute_raw_does_not_leave_an_attachment_on_the_pool() {
        with_sqlite("raw-attach", || {
            let extra = std::env::temp_dir().join(format!(
                "soli-attach-{}-{}.db",
                std::process::id(),
                "stolen"
            ));
            let _ = std::fs::remove_file(&extra);
            execute_raw(&format!("ATTACH DATABASE '{}' AS stolen", extra.display()))
                .expect("attach");
            with_conn(|conn| {
                let names: Vec<String> = conn
                    .prepare("PRAGMA database_list")
                    .unwrap()
                    .query_map([], |row| row.get(1))
                    .unwrap()
                    .filter_map(|n| n.ok())
                    .collect();
                assert!(
                    !names.iter().any(|n| n == "stolen"),
                    "ATTACH must not leak onto the pool: {names:?}"
                );
                Ok(())
            })
            .unwrap();
            let _ = std::fs::remove_file(&extra);
        });
    }

    /// Run `EXPLAIN QUERY PLAN` and return its joined detail column.
    fn query_plan(sql: &str, params: &[SqlBind]) -> String {
        with_conn(|conn| {
            let params = to_sqlite_params(params);
            let mut stmt = conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .map_err(|e| format!("explain prepare: {e}"))?;
            let rows = stmt
                .query_map(params_from_iter(params.iter()), |r| r.get::<_, String>(3))
                .map_err(|e| format!("explain: {e}"))?;
            Ok(rows
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| format!("explain row: {e}"))?
                .join(" | "))
        })
        .expect("explain")
    }

    /// An index the planner never uses is dead weight, so assert on the plan —
    /// not just on the index existing.
    #[test]
    fn a_document_index_is_created_once_and_actually_used() {
        with_sqlite("doc-index", || {
            let table = "posts";
            ensure_table(table).expect("table");
            for (key, status) in [("a", "open"), ("b", "open"), ("c", "closed")] {
                insert(table, Some(key), serde_json::json!({ "status": status })).expect("insert");
            }

            let filter = list_query(table, &[("status", serde_json::json!("open"))]);
            let compiled = compile_select_d(Dialect::Sqlite, &filter).expect("compile");

            // Before: a full scan of the table.
            let before = query_plan(&compiled.sql, &compiled.params);
            assert!(
                before.contains("SCAN"),
                "expected a scan before indexing, got: {before}"
            );

            assert!(
                ensure_doc_index(table, &["status".to_string()], "idx_posts_status", false)
                    .expect("create index"),
                "the first call creates the index"
            );
            // Idempotent: a second boot must not fail or duplicate.
            assert!(
                !ensure_doc_index(table, &["status".to_string()], "idx_posts_status", false)
                    .expect("second call"),
                "the index already existed"
            );
            assert!(list_index_names(table)
                .unwrap()
                .contains(&"idx_posts_status".to_string()));

            // After: the planner uses it. This is what makes the expression in
            // `ddl::doc_index_sql` match `compile_where`'s string equality.
            let after = query_plan(&compiled.sql, &compiled.params);
            assert!(
                after.contains("USING INDEX idx_posts_status"),
                "expected the index to be used, got: {after}"
            );

            // …and the rows are still correct.
            assert_eq!(select(&filter).unwrap().len(), 2);
            assert_eq!(count(&filter).unwrap(), 2);
        });
    }

    /// The queue's own bootstrap must index itself: the claim query runs every
    /// poll tick, and unindexed it scans every job ever enqueued.
    #[test]
    fn enqueuing_a_job_indexes_the_queue_table() {
        with_sqlite("jobs-index", || {
            let job = crate::jobs::JobDoc::new(
                "TestJob",
                serde_json::json!({}),
                "default",
                crate::jobs::now_iso(),
            );
            crate::jobs::store::enqueue(&job).expect("enqueue");

            let indexes = list_index_names("_jobs").expect("indexes");
            for field in ["state", "run_at", "priority"] {
                let expected = format!("idx__jobs_{field}");
                assert!(
                    indexes.contains(&expected),
                    "the claim query filters/orders on {field}; indexes: {indexes:?}"
                );
            }

            // The claim predicate itself must reach an index rather than scan.
            let plan = query_plan(
                "SELECT _key FROM \"_jobs\" WHERE (doc ->> '$.state') = 'pending' \
                 ORDER BY (doc ->> '$.run_at') ASC",
                &[],
            );
            assert!(plan.contains("USING INDEX"), "plan was: {plan}");
        });
    }

    #[test]
    fn a_unique_document_index_rejects_a_duplicate() {
        with_sqlite("doc-index-unique", || {
            let table = "accounts";
            ensure_table(table).expect("table");
            insert(table, Some("a"), serde_json::json!({ "email": "a@b.c" })).expect("insert");
            ensure_doc_index(table, &["email".to_string()], "idx_accounts_email", true)
                .expect("index");

            let err = insert(table, Some("b"), serde_json::json!({ "email": "a@b.c" }))
                .expect_err("a duplicate must be refused by the database");
            assert!(err.to_lowercase().contains("unique"), "{err}");

            // A different value still inserts.
            insert(table, Some("c"), serde_json::json!({ "email": "c@b.c" })).expect("insert");
        });
    }

    /// Concurrency is the whole point of doing the arithmetic in SQL, so the
    /// test has to be concurrent: with the old read-modify-write, threads read
    /// the same value and overwrite each other's bumps.
    #[test]
    fn concurrent_increments_do_not_lose_counts() {
        const THREADS: i64 = 8;
        const PER_THREAD: i64 = 25;

        with_sqlite("increment", || {
            let table = "counters";
            ensure_table(table).expect("table");
            insert(table, Some("hits"), serde_json::json!({ "views": 0 })).expect("seed");

            // Through the model-level entry point, which is what
            // `instance.increment`, `decrement`, and counter caches all call.
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
            assert_eq!(
                doc["views"].as_i64().unwrap(),
                THREADS * PER_THREAD,
                "every increment must survive"
            );
        });
    }

    #[test]
    fn increment_creates_a_missing_field_and_counts_down() {
        with_sqlite("increment-edges", || {
            let table = "counters";
            ensure_table(table).expect("table");
            insert(table, Some("k"), serde_json::json!({ "name": "x" })).expect("seed");

            // A missing field starts at 0, so a parent needs no schema prep.
            assert_eq!(increment_field(table, "k", "views", 1).unwrap(), Some(1));
            assert_eq!(increment_field(table, "k", "views", 5).unwrap(), Some(6));
            // Decrement is the same call with a negative delta, and may go below 0.
            assert_eq!(increment_field(table, "k", "views", -7).unwrap(), Some(-1));
            // The rest of the document is untouched.
            let doc = get(table, "k").unwrap().expect("row");
            assert_eq!(doc["name"], "x");
            assert_eq!(doc["views"].as_i64().unwrap(), -1);

            // A missing row reports nothing rather than inventing one.
            assert_eq!(increment_field(table, "nope", "views", 1).unwrap(), None);
            // A missing table is not an error either (nothing to count yet).
            assert_eq!(increment_field("absent", "k", "views", 1).unwrap(), None);
        });
    }

    #[test]
    fn a_column_counter_increments_atomically_too() {
        with_sqlite("increment-column", || {
            execute_ddl("CREATE TABLE meters (id INTEGER PRIMARY KEY, hits INTEGER, label TEXT)")
                .expect("ddl");
            execute_ddl("INSERT INTO meters (id, hits, label) VALUES (1, 0, 'a'), (2, NULL, 'b')")
                .expect("seed");
            let raw = introspect_table("meters").expect("introspect");
            let schema = std::sync::Arc::new(
                super::super::introspect::build_schema("primary", "meters", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect("schema"),
            );

            let one = serde_json::json!(1);
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    scope.spawn(|| {
                        for _ in 0..25 {
                            col_increment(&schema, &one, "hits", 1).expect("increment");
                        }
                    });
                }
            });
            assert_eq!(
                col_get(&schema, &one).unwrap().unwrap()["hits"],
                serde_json::json!(100)
            );

            // NULL counts as 0, so a fresh row needs no backfill.
            assert_eq!(
                col_increment(&schema, &serde_json::json!(2), "hits", 3).unwrap(),
                Some(3)
            );
            // A non-numeric column is refused by name, not by a driver error.
            let err = col_increment(&schema, &one, "label", 1).expect_err("text column");
            assert!(err.contains("label") && err.contains("numeric"), "{err}");
            // A missing row reports nothing.
            assert_eq!(
                col_increment(&schema, &serde_json::json!(99), "hits", 1).unwrap(),
                None
            );
        });
    }

    /// A constraint violation must arrive as a field error, not as driver prose.
    #[test]
    fn a_violation_is_classified_with_its_column() {
        use crate::db::error::{Constraint, ConstraintKind};

        with_sqlite("constraint-errors", || {
            execute_ddl("CREATE TABLE parents (id INTEGER PRIMARY KEY, code TEXT NOT NULL UNIQUE)")
                .expect("ddl");
            execute_ddl(
                "CREATE TABLE kids (id INTEGER PRIMARY KEY, parent_id BIGINT \
                 REFERENCES parents(id), note TEXT NOT NULL)",
            )
            .expect("ddl");
            let schema = |table: &str| {
                let raw = introspect_table(table).expect("introspect");
                std::sync::Arc::new(
                    super::super::introspect::build_schema("primary", table, raw, |t, _| {
                        sqlite_coltype(t)
                    })
                    .expect("schema"),
                )
            };
            let parents = schema("parents");
            let kids = schema("kids");
            col_insert(&parents, &serde_json::json!({ "code": "A" })).expect("insert");

            // Unique: the column comes from "UNIQUE constraint failed: parents.code".
            let err =
                col_insert(&parents, &serde_json::json!({ "code": "A" })).expect_err("duplicate");
            let violation = Constraint::parse(&err).expect("classified");
            assert_eq!(violation.kind, ConstraintKind::Unique);
            assert_eq!(violation.field().as_deref(), Some("code"));

            // NOT NULL names its column too.
            let err =
                col_insert(&parents, &serde_json::json!({ "code": null })).expect_err("null code");
            let violation = Constraint::parse(&err).expect("classified");
            assert_eq!(violation.kind, ConstraintKind::NotNull);
            assert_eq!(violation.field().as_deref(), Some("code"));

            // Foreign keys are enforced (PRAGMA foreign_keys=ON) and classified.
            let err = col_insert(&kids, &serde_json::json!({ "parent_id": 999, "note": "x" }))
                .expect_err("dangling fk");
            let violation = Constraint::parse(&err).expect("classified");
            assert_eq!(violation.kind, ConstraintKind::ForeignKey);

            // An unrelated failure stays unclassified rather than being guessed.
            let err = col_insert(&parents, &serde_json::json!({ "nope": 1 }))
                .expect_err("unknown column");
            assert!(Constraint::parse(&err).is_none(), "{err}");
        });
    }

    /// The document path (`_key` + `doc`) classifies the same way, and the model
    /// layer turns it into `{field, message}`.
    #[test]
    fn a_duplicate_reads_as_a_field_error_not_driver_text() {
        with_sqlite("constraint-model", || {
            let table = "accounts";
            ensure_table(table).expect("table");
            ensure_doc_index(table, &["email".to_string()], "idx_accounts_email", true)
                .expect("index");
            insert(table, Some("a"), serde_json::json!({ "email": "a@b.c" })).expect("insert");

            let err = insert(table, Some("b"), serde_json::json!({ "email": "a@b.c" }))
                .expect_err("duplicate");

            // What the model layer reports for that error.
            use crate::interpreter::builtins::model::validation::{
                build_constraint_errors, is_unique_violation,
            };
            assert!(is_unique_violation(&err), "{err}");
            let errors = build_constraint_errors(&err).expect("field errors");
            assert_eq!(errors.len(), 1);
            // The index is named after the column, which is how the field is
            // recovered when the database names only the index.
            assert_eq!(errors[0].field, "email");
            assert_eq!(errors[0].message, "has already been taken");
        });
    }

    /// The dev bar, `dev_queries()`, the N+1 detector and `--fail-on-n1` all read
    /// the query log, which no SQL adapter wrote to. This asserts the whole chain
    /// from a Model-level query down to a logged statement with its binds.
    #[test]
    fn queries_reach_the_dev_query_log() {
        use crate::interpreter::builtins::model::query_log;

        with_sqlite("query-log", || {
            let table = "posts";
            ensure_table(table).expect("table");
            insert(table, Some("a"), serde_json::json!({ "status": "open" })).expect("insert");

            query_log::set_enabled(true);
            query_log::clear();

            let filter = list_query(table, &[("status", serde_json::json!("open"))]);
            select(&filter).expect("select");
            count(&filter).expect("count");

            let logged = query_log::snapshot();
            query_log::set_enabled(false);

            assert!(
                logged.len() >= 2,
                "both statements should be logged, got {}",
                logged.len()
            );
            let select_entry = logged
                .iter()
                .find(|entry| entry.query.starts_with("SELECT doc"))
                .expect("the SELECT is logged with its real SQL");
            // The panel shows the SQL a developer can paste into a client…
            assert!(
                select_entry.query.contains("(doc ->> '$.status')"),
                "{}",
                select_entry.query
            );
            // …with the binds numbered like the placeholders…
            let binds = select_entry.bind_vars.as_ref().expect("binds are captured");
            assert_eq!(binds["1"], serde_json::json!("open"));
            // …and a duration, which is what the dev bar sorts by.
            assert!(select_entry.duration_ms >= 0.0);
            assert!(logged.iter().any(|e| e.query.contains("COUNT(*)")));
        });
    }

    /// An N+1 is a repeated statement shape. With the log fed, the detector the
    /// dev bar and `soli test --fail-on-n1` share can finally see one on SQL.
    #[test]
    fn a_repeated_query_is_detectable_as_an_n_plus_one() {
        use crate::interpreter::builtins::model::query_log;

        with_sqlite("query-log-n1", || {
            let table = "posts";
            ensure_table(table).expect("table");
            for key in ["a", "b", "c", "d"] {
                insert(table, Some(key), serde_json::json!({ "status": "open" })).expect("insert");
            }

            query_log::set_enabled(true);
            query_log::clear();
            // The shape of an N+1: one lookup per parent.
            for key in ["a", "b", "c", "d"] {
                get(table, key).expect("get");
            }
            let logged = query_log::snapshot();
            query_log::set_enabled(false);

            let groups = crate::serve::dev_bar::detect_n_plus_one(&logged, 3);
            assert!(
                !groups.is_empty(),
                "four identical lookups must register as an N+1; logged: {:?}",
                logged.iter().map(|e| &e.query).collect::<Vec<_>>()
            );
            let (_, count, _total_us) = &groups[0];
            assert!(*count >= 4, "expected at least 4 repeats, got {count}");
        });
    }

    /// Column-mode parity: batched eager loading, grouping, and bulk writes over
    /// real columns. These are compiler-level assertions on the SQL the column
    /// path emits, plus the round trip through a real database.
    #[test]
    fn column_mode_groups_and_bulk_writes() {
        with_sqlite("column-parity", || {
            execute_ddl(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, kind TEXT, qty INTEGER, \
                 note TEXT, updated_at DATETIME)",
            )
            .expect("ddl");
            execute_ddl(
                "INSERT INTO items (kind, qty) VALUES ('a', 1), ('a', 2), ('b', 5), ('b', 7)",
            )
            .expect("seed");
            let raw = introspect_table("items").expect("introspect");
            let schema = std::sync::Arc::new(
                super::super::introspect::build_schema("primary", "items", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect("schema"),
            );

            // group_by with an explicit aggregate.
            let query = cols::ColumnQuery::new(schema.clone());
            let aggs = vec![super::super::sql_compile::GroupAgg {
                alias: "total".into(),
                func: SqlAgg::Sum,
                field: "qty".into(),
            }];
            let rows = col_group_by(&query, &["kind".to_string()], &aggs).expect("group_by");
            assert_eq!(rows.len(), 2);
            let by_kind: std::collections::HashMap<String, f64> = rows
                .iter()
                .map(|row| {
                    (
                        row["kind"].as_str().unwrap_or_default().to_string(),
                        row["total"].as_f64().unwrap_or_default(),
                    )
                })
                .collect();
            assert_eq!(by_kind["a"], 3.0);
            assert_eq!(by_kind["b"], 12.0);

            // No aggregate declared → a row count aliased `n`, as on the
            // document path.
            let counted = col_group_by(&query, &["kind".to_string()], &[]).expect("counts");
            assert!(counted.iter().all(|row| row["n"].as_i64() == Some(2)));

            // An aggregate over a text column is refused by name.
            let bad = vec![super::super::sql_compile::GroupAgg {
                alias: "x".into(),
                func: SqlAgg::Sum,
                field: "note".into(),
            }];
            let err = col_group_by(&query, &["kind".to_string()], &bad).expect_err("text sum");
            assert!(err.contains("note") && err.contains("numeric"), "{err}");

            // Bulk update touches only the matched rows, and never the key.
            let mut only_a = cols::ColumnQuery::new(schema.clone());
            only_a
                .eq_filters
                .insert("kind".into(), serde_json::json!("a"));
            let changed =
                col_update_all(&only_a, &serde_json::json!({ "note": "seen", "id": 999 }))
                    .expect("update_all");
            assert_eq!(changed, 2);
            let rows = col_select(&only_a).expect("select");
            assert!(rows.iter().all(|r| r["note"] == "seen"));
            // The `id` in the patch was ignored rather than rewriting identities.
            let mut ids: Vec<i64> = rows.iter().filter_map(|r| r["id"].as_i64()).collect();
            ids.sort();
            assert_eq!(ids, vec![1, 2]);

            // Bulk delete is scoped the same way.
            let deleted = col_delete_all(&only_a).expect("delete_all");
            assert_eq!(deleted, 2);
            let all = cols::ColumnQuery::new(schema.clone());
            assert_eq!(col_count(&all).unwrap(), 2, "only kind 'b' remains");
        });
    }

    /// Soft delete needs a real column; with one, the scope is an ordinary filter.
    #[test]
    fn column_mode_soft_delete_uses_the_real_column() {
        with_sqlite("column-soft-delete", || {
            execute_ddl(
                "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT, deleted_at DATETIME)",
            )
            .expect("ddl");
            execute_ddl(
                "INSERT INTO notes (body, deleted_at) VALUES ('live', NULL), \
                 ('gone', '2026-01-01T00:00:00Z')",
            )
            .expect("seed");
            let raw = introspect_table("notes").expect("introspect");
            let schema = std::sync::Arc::new(
                super::super::introspect::build_schema("primary", "notes", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect("schema"),
            );

            // Default scope: deleted_at IS NULL.
            let mut live = cols::ColumnQuery::new(schema.clone());
            live.eq_filters
                .insert("deleted_at".into(), serde_json::Value::Null);
            let rows = col_select(&live).expect("select");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["body"], "live");

            // only_deleted: deleted_at IS NOT NULL.
            let mut deleted = cols::ColumnQuery::new(schema.clone());
            deleted.not_null_filters.push("deleted_at".into());
            let rows = col_select(&deleted).expect("select");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["body"], "gone");

            // with_deleted: no scope at all.
            let all = cols::ColumnQuery::new(schema);
            assert_eq!(col_count(&all).unwrap(), 2);
        });
    }

    /// `create_many` used to cost one statement (and on SQLite one transaction)
    /// per row. The query log makes the improvement checkable rather than
    /// assumed.
    #[test]
    fn bulk_insert_is_one_statement_and_upserts() {
        use crate::interpreter::builtins::model::query_log;

        with_sqlite("insert-many", || {
            let table = "widgets";
            ensure_table(table).expect("table");
            let rows: Vec<(String, serde_json::Value)> = (0..25)
                .map(|i| {
                    (
                        format!("k{i}"),
                        serde_json::json!({ "_key": format!("k{i}"), "n": i }),
                    )
                })
                .collect();

            query_log::set_enabled(true);
            query_log::clear();
            let inserted = crate::db::sql::insert_many(table, &rows).expect("insert_many");
            let logged = query_log::snapshot();
            query_log::set_enabled(false);

            assert_eq!(inserted, 25);
            let inserts = logged
                .iter()
                .filter(|entry| entry.query.starts_with("INSERT INTO"))
                .count();
            assert_eq!(
                inserts,
                1,
                "25 rows must be one statement, not 25; logged: {}",
                logged.len()
            );
            assert_eq!(count(&list_query(table, &[])).unwrap(), 25);

            // Re-running upserts rather than failing, like single-row insert.
            let again = crate::db::sql::insert_many(table, &rows).expect("re-run");
            assert_eq!(again, 25);
            assert_eq!(count(&list_query(table, &[])).unwrap(), 25);

            // An empty batch is a no-op, not an error.
            assert_eq!(crate::db::sql::insert_many(table, &[]).unwrap(), 0);
        });
    }

    /// `find_by_sql` is the escape hatch for what the portable surface cannot
    /// express. A single `doc` column hydrates as a document; anything else
    /// becomes a hash keyed by column name.
    #[test]
    fn raw_queries_return_documents_or_projections() {
        with_sqlite("find-by-sql", || {
            let table = "widgets";
            ensure_table(table).expect("table");
            for (key, n) in [("a", 1), ("b", 5), ("c", 9)] {
                insert(table, Some(key), serde_json::json!({ "n": n })).expect("insert");
            }

            // One `doc` column → the documents themselves.
            let docs = crate::db::sql::query_raw(
                "SELECT doc FROM widgets WHERE (doc ->> '$.n') > ? ORDER BY (doc ->> '$.n')",
                &[SqlBind::I64(1)],
            )
            .expect("raw select");
            assert_eq!(docs.len(), 2);
            assert_eq!(docs[0]["n"], serde_json::json!(5));

            // Any other shape → a hash per row, numbers parsed as numbers.
            let rows = crate::db::sql::query_raw(
                "SELECT _key AS id, (doc ->> '$.n') AS n FROM widgets ORDER BY n DESC LIMIT 1",
                &[],
            )
            .expect("projection");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["id"], "c");
            assert_eq!(rows[0]["n"], serde_json::json!(9));

            // Binds are passed, never interpolated: a value that looks like SQL
            // stays a value.
            let none = crate::db::sql::query_raw(
                "SELECT doc FROM widgets WHERE _key = ?",
                &[SqlBind::Text("a' OR 1=1 --".into())],
            )
            .expect("bound");
            assert!(none.is_empty(), "the injection attempt matched nothing");
        });
    }

    /// A projection fetches only the named columns — on a wide table that is the
    /// difference between reading two columns and reading all of them.
    #[test]
    fn a_projection_selects_only_what_was_asked_for() {
        with_sqlite("projection", || {
            execute_ddl("CREATE TABLE wide (id INTEGER PRIMARY KEY, a TEXT, b TEXT, c TEXT)")
                .expect("ddl");
            execute_ddl("INSERT INTO wide (a, b, c) VALUES ('1', '2', '3')").expect("seed");
            let raw = introspect_table("wide").expect("introspect");
            let schema = std::sync::Arc::new(
                super::super::introspect::build_schema("primary", "wide", raw, |t, _| {
                    sqlite_coltype(t)
                })
                .expect("schema"),
            );

            let mut q = cols::ColumnQuery::new(schema.clone());
            q.select_fields = Some(vec!["a".to_string()]);
            let compiled = cols::compile_select_cols(Dialect::Sqlite, &q).expect("compile");
            // Only `a` and the key, not b/c.
            assert!(compiled.sql.contains("\"a\""), "{}", compiled.sql);
            assert!(!compiled.sql.contains("\"b\""), "{}", compiled.sql);
            assert!(compiled.sql.contains("\"id\""), "the key stays available");

            let rows = col_select(&q).expect("select");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["a"], "1");
            // The unprojected columns are absent rather than null.
            assert!(rows[0].get("b").is_none(), "{:?}", rows[0]);
            assert_eq!(rows[0]["id"], serde_json::json!(1));

            // An unknown field is still refused before any SQL is built.
            let mut bad = cols::ColumnQuery::new(schema);
            bad.select_fields = Some(vec!["nope".to_string()]);
            assert!(cols::compile_select_cols(Dialect::Sqlite, &bad).is_err());
        });
    }

    /// `.join` keeps parents that have a matching child, without duplicating them
    /// the way a real join would.
    #[test]
    fn an_exists_filter_selects_parents_with_children() {
        use super::super::sql_compile::ExistsFilter;

        with_sqlite("exists-filter", || {
            ensure_table("posts").expect("table");
            ensure_table("comments").expect("table");
            insert("posts", Some("p1"), serde_json::json!({ "title": "one" })).expect("insert");
            insert("posts", Some("p2"), serde_json::json!({ "title": "two" })).expect("insert");
            // Two comments on p1, so a real join would return p1 twice.
            insert(
                "comments",
                Some("c1"),
                serde_json::json!({ "post_id": "p1", "approved": "yes" }),
            )
            .expect("insert");
            insert(
                "comments",
                Some("c2"),
                serde_json::json!({ "post_id": "p1", "approved": "no" }),
            )
            .expect("insert");

            let mut q = list_query("posts", &[]);
            q.exists_filters = vec![ExistsFilter {
                table: "comments".into(),
                foreign_key: "post_id".into(),
                eq_filters: std::collections::BTreeMap::new(),
                hash_filter: None,
            }];
            let rows = select(&q).expect("select");
            assert_eq!(rows.len(), 1, "p1 once, not twice");
            assert_eq!(rows[0]["_key"], "p1");
            assert_eq!(count(&q).unwrap(), 1);

            // A filter on the child narrows which parents qualify.
            q.exists_filters[0]
                .eq_filters
                .insert("approved".into(), serde_json::json!("no"));
            assert_eq!(count(&q).unwrap(), 1);
            q.exists_filters[0]
                .eq_filters
                .insert("approved".into(), serde_json::json!("maybe"));
            assert_eq!(count(&q).unwrap(), 0);
        });
    }

    /// `.having` filters groups after aggregation.
    #[test]
    fn having_filters_groups() {
        with_sqlite("having", || {
            let table = "orders";
            ensure_table(table).expect("table");
            for (key, status) in [("a", "open"), ("b", "open"), ("c", "closed")] {
                insert(table, Some(key), serde_json::json!({ "status": status })).expect("insert");
            }

            let mut q = list_query(table, &[]);
            q.having = Some("n > 1".into());
            let rows = group_by(&q, &["status".to_string()], &[]).expect("group_by");
            assert_eq!(rows.len(), 1, "only the group of two survives");
            assert_eq!(rows[0]["status"], "open");
            assert_eq!(rows[0]["n"], serde_json::json!(2));

            // Without it, both groups come back.
            q.having = None;
            assert_eq!(group_by(&q, &["status".to_string()], &[]).unwrap().len(), 2);
        });
    }

    #[test]
    fn hash_where_compiles_comparisons_in_like_and_or() {
        use crate::db::hash_filter::HashFilter;

        with_sqlite("hash-where", || {
            let table = "orders";
            ensure_table(table).expect("table");
            insert(
                table,
                Some("a"),
                serde_json::json!({"status": "open", "total": 50, "email": "a@x.com"}),
            )
            .expect("insert");
            insert(
                table,
                Some("b"),
                serde_json::json!({"status": "draft", "total": 5, "email": "b@y.com"}),
            )
            .expect("insert");

            let pred = HashFilter::from_json_map(
                serde_json::json!({"total": {"gte": 10}})
                    .as_object()
                    .unwrap(),
                "where",
            )
            .unwrap();
            let mut q = list_query(table, &[]);
            q.hash_filter = Some(pred);
            q.filter_sdbql = None;
            q.eq_filters.clear();
            let rows = select(&q).expect("select");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["_key"], "a");

            q.hash_filter = Some(
                HashFilter::from_json_map(
                    serde_json::json!({"email": {"like": "%@x.com"}})
                        .as_object()
                        .unwrap(),
                    "where",
                )
                .unwrap(),
            );
            assert_eq!(count(&q).unwrap(), 1);

            q.hash_filter = Some(
                HashFilter::from_json_map(
                    serde_json::json!({"status": ["open", "paid"]})
                        .as_object()
                        .unwrap(),
                    "where",
                )
                .unwrap(),
            );
            assert_eq!(count(&q).unwrap(), 1);

            q.hash_filter = Some(
                HashFilter::from_json_map(
                    serde_json::json!({"or": [{"status": "draft"}, {"status": "paid"}]})
                        .as_object()
                        .unwrap(),
                    "where",
                )
                .unwrap(),
            );
            assert_eq!(count(&q).unwrap(), 1);
        });
    }

    #[test]
    fn schema_dump_round_trips_through_load() {
        with_sqlite("schema-dump", || {
            ensure_connected().expect("connect");
            ensure_table("people").expect("table");
            insert("people", Some("k1"), serde_json::json!({"name": "Ada"})).expect("insert");
            ensure_migrations_table().expect("migrations");
            record_migration("20260813000001", "create_people").expect("record");

            let dump = dump_schema().expect("dump");
            assert!(dump.contains("-- adapter: sqlite"), "{dump}");
            assert!(dump.contains("20260813000001_create_people"), "{dump}");
            assert!(dump.to_lowercase().contains("create table"), "{dump}");

            execute_raw("DROP TABLE IF EXISTS people; DROP TABLE IF EXISTS `_migrations`;")
                .expect("wipe");
            crate::db::sql::load_schema(&dump).expect("load");
            assert!(table_exists("people").expect("exists"));
            let versions = list_applied_migrations().expect("versions");
            assert!(
                versions
                    .iter()
                    .any(|(v, n)| v == "20260813000001" && n == "create_people"),
                "{versions:?}"
            );
        });
    }

    // ---------- job engine ----------

    fn enqueue_job(
        key: &str,
        state: &str,
        run_at: &str,
        priority: i64,
        locked_until: Option<&str>,
    ) {
        insert(
            "_jobs",
            Some(key),
            serde_json::json!({
                "handler": "TestJob",
                "args": {},
                "queue": "default",
                "priority": priority,
                "state": state,
                "attempts": 0,
                "max_retries": 3,
                "run_at": run_at,
                "locked_by": null,
                "locked_until": locked_until,
            }),
        )
        .expect("enqueue");
    }

    #[test]
    fn claim_is_exclusive_and_priority_ordered() {
        with_sqlite("claim", || {
            ensure_connected().expect("connect");
            let past = "2020-01-01T00:00:00Z";
            let now = "2026-08-12T12:00:00Z";
            let lease = "2026-08-12T12:01:00Z";

            // No table yet: claiming must be a quiet no-op, not an error.
            assert!(claim_jobs(now, "w1", lease, 5).unwrap().is_empty());

            enqueue_job("low", "pending", past, 0, None);
            enqueue_job("high", "pending", past, 5, None);
            enqueue_job("future", "scheduled", "2099-01-01T00:00:00Z", 9, None);

            // Priority wins, and a job that is not due yet is left alone.
            let first = claim_jobs(now, "w1", lease, 1).expect("claim");
            assert_eq!(first.len(), 1);
            assert_eq!(first[0]["handler"], "TestJob");
            assert_eq!(first[0]["state"], "running");
            assert_eq!(first[0]["locked_by"], "w1");
            assert_eq!(first[0]["attempts"], serde_json::json!(1));
            assert_eq!(first[0]["_key"], "high");

            // A second claimer never sees the row the first one took.
            let second = claim_jobs(now, "w2", lease, 5).expect("claim");
            assert_eq!(second.len(), 1);
            assert_eq!(second[0]["_key"], "low");

            // Everything due is now held; the scheduled job stays put.
            assert!(claim_jobs(now, "w3", lease, 5).unwrap().is_empty());
        });
    }

    #[test]
    fn an_expired_lease_is_reclaimed() {
        with_sqlite("lease", || {
            ensure_connected().expect("connect");
            // A worker died holding this job: state `running`, lease in the past.
            enqueue_job(
                "orphan",
                "running",
                "2020-01-01T00:00:00Z",
                0,
                Some("2020-01-01T00:00:10Z"),
            );
            let now = "2026-08-12T12:00:00Z";
            let claimed = claim_jobs(now, "w-new", "2026-08-12T12:01:00Z", 5).expect("claim");
            assert_eq!(claimed.len(), 1);
            assert_eq!(claimed[0]["_key"], "orphan");
            assert_eq!(claimed[0]["locked_by"], "w-new");
            // The attempt counter advances, so retries stay bounded.
            assert_eq!(claimed[0]["attempts"], serde_json::json!(1));

            // A live lease is NOT reclaimed.
            let still_held = claim_jobs(now, "w-other", "2026-08-12T12:02:00Z", 5).expect("claim");
            assert!(still_held.is_empty());
        });
    }

    #[test]
    fn a_cron_slot_has_exactly_one_winner() {
        with_sqlite("cron", || {
            ensure_connected().expect("connect");
            // No table yet: nobody wins, and nothing raises.
            assert!(!claim_cron_slot("nightly", "x", serde_json::json!({})).unwrap());

            insert(
                "_cron_jobs",
                Some("nightly"),
                serde_json::json!({
                    "name": "nightly",
                    "cron_expression": "0 0 3 * * *",
                    "handler": "ReportJob",
                    "enabled": true,
                    "last_run_at": null,
                    "next_run_at": "2026-08-12T03:00:00Z",
                }),
            )
            .expect("insert");

            let patch = serde_json::json!({
                "last_run_at": "2026-08-12T03:00:00Z",
                "next_run_at": "2026-08-13T03:00:00Z",
            });
            assert!(claim_cron_slot("nightly", "2026-08-12T03:00:00Z", patch.clone()).unwrap());
            // The loser sees the moved slot and claims nothing.
            assert!(!claim_cron_slot("nightly", "2026-08-12T03:00:00Z", patch).unwrap());

            let row = get("_cron_jobs", "nightly").unwrap().expect("row");
            assert_eq!(row["next_run_at"], "2026-08-13T03:00:00Z");
            // The merge must leave the rest of the definition intact.
            assert_eq!(row["handler"], "ReportJob");
            assert_eq!(row["cron_expression"], "0 0 3 * * *");
        });
    }

    #[test]
    fn text_timestamps_gain_a_zone() {
        assert_eq!(
            normalize_temporal(ColType::DateTime, "2026-08-12 10:00:00"),
            "2026-08-12T10:00:00Z"
        );
        // An offset already present is left alone, and a DATE is never touched.
        assert_eq!(
            normalize_temporal(ColType::DateTime, "2026-08-12T10:00:00+02:00"),
            "2026-08-12T10:00:00+02:00"
        );
        assert_eq!(
            normalize_temporal(ColType::Date, "2026-08-12"),
            "2026-08-12"
        );
    }
}
