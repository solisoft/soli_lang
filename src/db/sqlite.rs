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
    compile_group_by_d, compile_select_by_keys_d, compile_select_d, compile_select_json_text_in_d,
    compile_update_all_d, create_table_sql_d, drop_table_sql_d, migrations_table_sql_d, Dialect,
    GroupAgg, ListQuery, SqlAgg, SqlBind,
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
            .map_err(|e| format!("sqlite BEGIN: {e}"))?;
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
            .map_err(|e| format!("sqlite COMMIT: {e}"))
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
            .map_err(|e| format!("sqlite ROLLBACK: {e}"))
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
        .map_err(|e| format!("sqlite BEGIN IMMEDIATE: {e}"))?;
    match f(conn) {
        Ok(value) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("sqlite COMMIT: {e}"))?;
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
    let row: Option<String> = conn
        .query_row(&sql, [key], |r| r.get(0))
        .optional()
        .map_err(|e| format!("sqlite get: {e}"))?;
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
    with_conn(|conn| {
        conn.execute(&sql, rusqlite::params![&key, &doc_str])
            .map_err(|e| format!("sqlite insert: {e}"))?;
        get_on(conn, table, &key)?.ok_or_else(|| "sqlite insert: row missing after write".into())
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
    with_conn(|conn| {
        conn.execute(&sql, rusqlite::params![key, &doc_str])
            .map_err(|e| format!("sqlite update: {e}"))?;
        get_on(conn, table, key)?.ok_or_else(|| "sqlite update: row missing".into())
    })
}

pub fn delete(table: &str, key: &str) -> Result<(), String> {
    if !table_exists(table)? {
        return Ok(());
    }
    let table_q = Dialect::Sqlite.quote_ident(table)?;
    let sql = format!("DELETE FROM {table_q} WHERE _key = ?");
    with_conn(|conn| {
        conn.execute(&sql, [key])
            .map_err(|e| format!("sqlite delete: {e}"))?;
        Ok(())
    })
}

pub fn select(q: &ListQuery) -> Result<Vec<serde_json::Value>, String> {
    if !table_exists(&q.table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_d(Dialect::Sqlite, q)?;
    query_docs(&compiled.sql, &compiled.params)
}

pub fn select_by_keys(table: &str, keys: &[String]) -> Result<Vec<serde_json::Value>, String> {
    if keys.is_empty() || !table_exists(table)? {
        return Ok(Vec::new());
    }
    let compiled = compile_select_by_keys_d(Dialect::Sqlite, table, keys)?;
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
            .map_err(|e| format!("sqlite group_by prepare: {e}"))?;
        let mut rows = stmt
            .query(params_from_iter(params.iter()))
            .map_err(|e| format!("sqlite group_by: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("sqlite group_by row: {e}"))?
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
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n: Option<i64> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| format!("sqlite count: {e}"))?;
        Ok(n.unwrap_or(0))
    })
}

pub fn exists(q: &ListQuery) -> Result<bool, String> {
    if !table_exists(&q.table)? {
        return Ok(false);
    }
    let compiled = compile_exists_d(Dialect::Sqlite, q)?;
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let hit: Option<i64> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| format!("sqlite exists: {e}"))?;
        Ok(hit.is_some())
    })
}

pub fn aggregate(q: &ListQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    if !table_exists(&q.table)? {
        return Ok(serde_json::Value::Null);
    }
    let compiled = compile_aggregate_d(Dialect::Sqlite, q, func, field)?;
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let value: Option<serde_json::Value> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| {
                Ok(r.get_ref(0).map(value_to_json).unwrap_or_default())
            })
            .optional()
            .map_err(|e| format!("sqlite aggregate: {e}"))?;
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
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n = conn
            .execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| format!("sqlite delete_all: {e}"))?;
        Ok(n as u64)
    })
}

pub fn update_all(q: &ListQuery, patch: serde_json::Value) -> Result<u64, String> {
    if !table_exists(&q.table)? {
        return Ok(0);
    }
    let compiled = compile_update_all_d(Dialect::Sqlite, q, &patch)?;
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n = conn
            .execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| format!("sqlite update_all: {e}"))?;
        Ok(n as u64)
    })
}

fn query_docs(sql: &str, params: &[SqlBind]) -> Result<Vec<serde_json::Value>, String> {
    with_conn(|conn| {
        let params = to_sqlite_params(params);
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| format!("sqlite prepare: {e}"))?;
        let mut rows = stmt
            .query(params_from_iter(params.iter()))
            .map_err(|e| format!("sqlite query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| format!("sqlite row: {e}"))? {
            let text: String = row.get(0).map_err(|e| format!("sqlite row doc: {e}"))?;
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
            .map_err(|e| format!("sqlite ensure_table: {e}"))
    })
}

pub fn drop_table(table: &str) -> Result<(), String> {
    let ddl = drop_table_sql_d(Dialect::Sqlite, table)?;
    with_conn(|conn| {
        conn.execute_batch(&ddl)
            .map_err(|e| format!("sqlite drop_table: {e}"))
    })
}

pub fn ensure_migrations_table() -> Result<(), String> {
    with_conn(|conn| {
        conn.execute_batch(migrations_table_sql_d(Dialect::Sqlite))
            .map_err(|e| format!("sqlite migrations table: {e}"))
    })
}

pub fn list_applied_migrations() -> Result<Vec<(String, String)>, String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT version, name FROM \"_migrations\" ORDER BY version")
            .map_err(|e| format!("sqlite list migrations: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| format!("sqlite list migrations: {e}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("sqlite list migrations row: {e}"))
    })
}

pub fn record_migration(version: &str, name: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO \"_migrations\" (version, name) VALUES (?, ?)",
            [version, name],
        )
        .map_err(|e| format!("sqlite record migration: {e}"))?;
        Ok(())
    })
}

pub fn remove_migration(version: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    with_conn(|conn| {
        conn.execute("DELETE FROM \"_migrations\" WHERE version = ?", [version])
            .map_err(|e| format!("sqlite remove migration: {e}"))?;
        Ok(())
    })
}

/// Run raw DDL (used by migrations and by the column-mode test harness).
pub fn execute_ddl(sql: &str) -> Result<(), String> {
    with_conn(|conn| {
        conn.execute_batch(sql)
            .map_err(|e| format!("sqlite ddl: {e}"))
    })
}

// ---------- column-aware model execution ----------

use super::introspect::{ColType, TableSchema};
use super::sql_columns_compile as cols;

/// Read one row into a JSON object keyed by column name, so downstream
/// hydration is identical to the document path.
fn row_to_json(schema: &TableSchema, row: &rusqlite::Row) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    let mut idx = 0usize;
    for col in &schema.columns {
        // Unreadable columns are not selected, so they hold no position.
        if col.ty == ColType::Unknown {
            continue;
        }
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
    sql: &str,
    params: &[SqlBind],
    what: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let params = to_sqlite_params(params);
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("sqlite column {what} prepare: {e}"))?;
    let mut rows = stmt
        .query(params_from_iter(params.iter()))
        .map_err(|e| format!("sqlite column {what}: {e}"))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| format!("sqlite column {what} row: {e}"))?
    {
        out.push(row_to_json(schema, row));
    }
    Ok(out)
}

pub fn col_get(
    schema: &std::sync::Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let compiled = cols::compile_get_cols(Dialect::Sqlite, schema, pk)?;
    with_conn(|conn| {
        let rows = col_rows(conn, schema, &compiled.sql, &compiled.params, "get")?;
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
    with_conn(|conn| {
        let rows = col_rows(conn, schema, &compiled.sql, &compiled.params, "insert")?;
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
    with_conn(|conn| {
        let rows = col_rows(conn, schema, &compiled.sql, &compiled.params, "update")?;
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
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        conn.execute(&compiled.sql, params_from_iter(params.iter()))
            .map_err(|e| format!("sqlite column delete: {e}"))?;
        Ok(())
    })
}

pub fn col_select(q: &cols::ColumnQuery) -> Result<Vec<serde_json::Value>, String> {
    let compiled = cols::compile_select_cols(Dialect::Sqlite, q)?;
    with_conn(|conn| col_rows(conn, &q.schema, &compiled.sql, &compiled.params, "select"))
}

pub fn col_count(q: &cols::ColumnQuery) -> Result<i64, String> {
    let compiled = cols::compile_count_cols(Dialect::Sqlite, q)?;
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let n: Option<i64> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| format!("sqlite column count: {e}"))?;
        Ok(n.unwrap_or(0))
    })
}

pub fn col_exists(q: &cols::ColumnQuery) -> Result<bool, String> {
    let compiled = cols::compile_exists_cols(Dialect::Sqlite, q)?;
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        let hit: Option<i64> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| format!("sqlite column exists: {e}"))?;
        Ok(hit.is_some())
    })
}

pub fn col_aggregate(
    q: &cols::ColumnQuery,
    func: SqlAgg,
    field: &str,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_aggregate_cols(Dialect::Sqlite, q, func, field)?;
    with_conn(|conn| {
        let params = to_sqlite_params(&compiled.params);
        if func == SqlAgg::Count {
            let n: Option<i64> = conn
                .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
                .optional()
                .map_err(|e| format!("sqlite column aggregate: {e}"))?;
            return Ok(serde_json::json!(n.unwrap_or(0)));
        }
        let raw: Option<Option<String>> = conn
            .query_row(&compiled.sql, params_from_iter(params.iter()), |r| r.get(0))
            .optional()
            .map_err(|e| format!("sqlite column aggregate: {e}"))?;
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
            .map_err(|e| format!("sqlite introspect prepare: {e}"))?;
        let rows = stmt
            .query_map([table], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| format!("sqlite introspect columns: {e}"))?;

        let mut columns = Vec::new();
        // `pk` is the 1-based position in the key, so sort by it to keep key
        // order (a composite key is rejected later, but the order is reported
        // in the error).
        let mut pk_ordered: Vec<(i64, String)> = Vec::new();
        for row in rows {
            let (name, declared, notnull, pk_pos) =
                row.map_err(|e| format!("sqlite introspect row: {e}"))?;
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
                .map_err(|e| format!("sqlite claim_jobs prepare: {e}"))?;
            let keys: Vec<String> = stmt
                .query_map(rusqlite::params![now_iso, batch as i64], |r| r.get(0))
                .map_err(|e| format!("sqlite claim_jobs select: {e}"))?
                .collect::<rusqlite::Result<Vec<String>>>()
                .map_err(|e| format!("sqlite claim_jobs row: {e}"))?;
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
                    .map_err(|e| format!("sqlite claim_jobs update: {e}"))?;
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
            .map_err(|e| format!("sqlite claim_cron_slot: {e}"))?;
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
            .map_err(|e| format!("sqlite table_exists: {e}"))?;
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
            filter_sdbql: if clauses.is_empty() {
                None
            } else {
                Some(clauses.join(" AND "))
            },
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
