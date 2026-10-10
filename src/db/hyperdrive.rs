//! PostgreSQL on the edge build: the SQL of the native adapter, run by a
//! JavaScript driver in the Worker.
//!
//! A Worker has no socket a Rust client could open, so the `postgres` crate is
//! not in the wasm build. A JavaScript driver can: `pg` connects through the
//! Worker's TCP sockets, and [Hyperdrive](https://developers.cloudflare.com/hyperdrive/)
//! keeps a pool of connections next to the database for it. So this module
//! compiles the SQL the native adapter sends — the same `sql_compile`
//! generators in [`Dialect::Postgres`], the same `_key` + `doc` tables, the
//! same column-aware statements — and hands each statement to the host's
//! `soli_sql` import, which the wasm stack suspends on like a D1 query.
//!
//! The connection URL picks the database: `hyperdrive://HYPERDRIVE` (the
//! default) names a Hyperdrive binding, and a `postgres://` URL is connected to
//! directly. The host opens one client per connection per request and closes it
//! when the request ends, so every statement of a request — a transaction's
//! included — runs on the same connection.
//!
//! MySQL is not offered on the edge: its driver alone weighs 410 KB compressed,
//! more than the room left under the free plan's 3 MB.
//!
//! Natively nothing routes here, unless a test installs a [`SqlExecutor`] and
//! the connection's URL is a `hyperdrive://` one.

use super::introspect::{ColType, RawColumns, TableSchema};
use super::registry::{active_connection_name, active_spec};
use super::sql_columns_compile as cols;
use super::sql_compile::{
    compile_aggregate_d, compile_count_d, compile_delete_all_d, compile_exists_d,
    compile_group_by_d, compile_insert_many_d, compile_select_by_keys_d, compile_select_d,
    compile_select_json_text_in_d, compile_update_all_d, create_table_sql_d, drop_table_sql_d,
    migrations_table_sql_d, Dialect, GroupAgg, ListQuery, SqlAgg, SqlBind,
};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

const PG: Dialect = Dialect::Postgres;

/// What one statement returned, flattened to positional rows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SqlRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Rows written.
    pub changes: u64,
}

/// One statement for the host: `target` is the connection URL
/// (`hyperdrive://BINDING` or a `postgres://` URL), `params` what the
/// JavaScript driver binds — strings, numbers, booleans and null.
pub struct SqlRequest<'a> {
    pub target: &'a str,
    pub sql: &'a str,
    pub params: &'a [serde_json::Value],
}

/// How a statement reaches the database.
pub trait SqlExecutor: Send + Sync {
    fn run(&self, request: &SqlRequest<'_>) -> Result<SqlRows, String>;
}

static EXECUTOR: RwLock<Option<Arc<dyn SqlExecutor>>> = RwLock::new(None);
/// Set with the executor, so the native adapter's hot path pays an atomic load
/// to learn that nothing routes here.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Route `hyperdrive://` connections through `executor` (tests).
pub fn set_executor(executor: Arc<dyn SqlExecutor>) {
    *EXECUTOR.write().unwrap_or_else(|e| e.into_inner()) = Some(executor);
    INSTALLED.store(true, Ordering::Release);
}

/// Whether the active Postgres connection runs here: always on the edge build,
/// which has no native client; natively only for a `hyperdrive://` connection
/// once a test has installed an executor.
pub fn routes() -> bool {
    if cfg!(target_arch = "wasm32") {
        return true;
    }
    INSTALLED.load(Ordering::Acquire)
        && active_spec()
            .ok()
            .and_then(|spec| spec.url.clone())
            .is_some_and(|url| url.starts_with("hyperdrive://"))
}

fn executor() -> Result<Arc<dyn SqlExecutor>, String> {
    if let Some(executor) = EXECUTOR.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return Ok(executor);
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(Arc::new(host::Driver))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(
            "a hyperdrive:// connection is reached through a Cloudflare Worker's binding: \
             run the app with `soli edge build` and `wrangler`, or use a postgres:// url \
             under `soli serve`"
                .to_string(),
        )
    }
}

/// The default URL on the edge build: the binding `wrangler hyperdrive create`
/// documents.
pub const DEFAULT_URL: &str = "hyperdrive://HYPERDRIVE";

/// The active connection's target, its `hyperdrive://` binding name checked.
fn target() -> Result<String, String> {
    let spec = active_spec()?;
    let url = spec.url.clone().unwrap_or_else(|| DEFAULT_URL.to_string());
    if let Some(name) = url.strip_prefix("hyperdrive://") {
        let name = name.trim_matches('/');
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!(
                "connection {:?}: a Hyperdrive url names the Worker binding, like \
                 hyperdrive://HYPERDRIVE (got {url:?})",
                spec.name
            ));
        }
        return Ok(format!("hyperdrive://{name}"));
    }
    Ok(url)
}

fn bind_value(bind: &SqlBind) -> serde_json::Value {
    match bind {
        SqlBind::Text(s) => serde_json::Value::String(s.clone()),
        SqlBind::I64(n) => serde_json::json!(n),
        SqlBind::F64(f) => serde_json::json!(f),
        SqlBind::Bool(b) => serde_json::json!(b),
        // JSON travels as text, as the native adapter binds it; the server
        // infers `jsonb` from the column or the cast.
        SqlBind::Json(j) => serde_json::Value::String(j.to_string()),
    }
}

/// Run one statement on the active connection.
fn exec(context: &str, sql: &str, params: &[SqlBind]) -> Result<SqlRows, String> {
    let _trace = super::trace::start(sql, params);
    let bound: Vec<serde_json::Value> = params.iter().map(bind_value).collect();
    executor()?
        .run(&SqlRequest {
            target: &target()?,
            sql,
            params: &bound,
        })
        .map_err(|e| format!("postgres {context}: {e}"))
}

/// Run a script: Postgres takes several statements in one simple query, which
/// also keeps `$$`-quoted function bodies whole.
fn exec_script(context: &str, script: &str) -> Result<(), String> {
    if !script.trim().is_empty() {
        exec(context, script, &[])?;
    }
    Ok(())
}

fn first_cell(rows: &SqlRows) -> Option<&serde_json::Value> {
    rows.rows.first().and_then(|row| row.first())
}

fn as_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|f| f as i64))
        .or_else(|| value.as_str().and_then(super::parse_counter))
}

/// A cell as the text the native adapter reads numbers and timestamps as.
fn cell_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// A `doc` cell: the driver parses `jsonb`, but a JSON value computed by an
/// expression can still arrive as text.
fn doc_value(context: &str, cell: serde_json::Value) -> Result<serde_json::Value, String> {
    match cell {
        serde_json::Value::String(text) => {
            serde_json::from_str(&text).map_err(|e| format!("{context} json: {e}"))
        }
        other => Ok(other),
    }
}

fn docs(context: &str, rows: SqlRows) -> Result<Vec<serde_json::Value>, String> {
    rows.rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .map(|cell| doc_value(context, cell))
        .collect()
}

fn one_doc(context: &str, rows: SqlRows) -> Result<Option<serde_json::Value>, String> {
    Ok(docs(context, rows)?.into_iter().next())
}

fn unsupported(what: &str) -> String {
    format!("{what} is not available on the edge (Cloudflare Workers) build")
}

fn table_exists(table: &str) -> Result<bool, String> {
    // `current_schema()`, as the native adapter resolves it.
    let rows = exec(
        "table_exists",
        "SELECT 1 FROM information_schema.tables \
         WHERE table_schema = current_schema() AND table_name = $1",
        &[SqlBind::Text(table.to_string())],
    )?;
    Ok(!rows.rows.is_empty())
}

fn resolve_key(key: Option<&str>, document: &serde_json::Value) -> String {
    key.map(str::to_string)
        .or_else(|| {
            document
                .get("_key")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            document
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

// ---------- connection and transactions ----------

/// The open transaction: which connection holds it and how deeply it is
/// nested (a nested `transaction` joins the outer one, as natively).
struct TxState {
    name: String,
    nest: u32,
}

thread_local! {
    static TX: RefCell<Option<TxState>> = const { RefCell::new(None) };
}

pub fn ensure_connected() -> Result<(), String> {
    exec("connect", "SELECT 1", &[]).map(|_| ())
}

pub fn has_active_tx() -> bool {
    TX.with(|tx| tx.borrow().is_some())
}

/// Whether the transaction is open on the connection this op runs against.
fn tx_open_on_active_connection() -> bool {
    let name = active_connection_name();
    TX.with(|tx| tx.borrow().as_ref().is_some_and(|t| t.name == name))
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

pub fn begin_transaction(isolation_level: Option<&str>) -> Result<String, String> {
    let name = active_connection_name();
    let nested = TX.with(|tx| -> Result<Option<u32>, String> {
        let mut slot = tx.borrow_mut();
        match slot.as_mut() {
            Some(state) if state.name != name => Err(format!(
                "a transaction is already open on connection {:?}; cannot begin one on \
                 {name:?} from the same block (one SQL transaction per thread)",
                state.name
            )),
            Some(state) => {
                state.nest += 1;
                Ok(Some(state.nest))
            }
            None => Ok(None),
        }
    })?;
    if let Some(nest) = nested {
        return Ok(format!("sql-hyperdrive-nested-{nest}"));
    }
    let iso = map_isolation(isolation_level)?;
    exec("BEGIN", &format!("BEGIN ISOLATION LEVEL {iso}"), &[])?;
    TX.with(|tx| *tx.borrow_mut() = Some(TxState { name, nest: 0 }));
    Ok("sql-hyperdrive".into())
}

/// Close the transaction with `verb` (`COMMIT` / `ROLLBACK`), or leave a nested
/// level. `None` when no transaction is open.
fn end_transaction(verb: &str) -> Option<Result<(), String>> {
    let outer = TX.with(|tx| {
        let mut slot = tx.borrow_mut();
        let state = slot.as_mut()?;
        if state.nest > 0 {
            state.nest -= 1;
            return Some(None);
        }
        Some(slot.take().map(|state| state.name))
    })?;
    let name = outer?;
    Some(super::registry::with_connection(&name, || {
        exec(verb, verb, &[]).map(|_| ())
    }))
}

pub fn commit_transaction() -> Result<(), String> {
    end_transaction("COMMIT").unwrap_or_else(|| Err("No active SQL transaction".into()))
}

pub fn rollback_transaction() -> Result<(), String> {
    end_transaction("ROLLBACK").unwrap_or(Ok(()))
}

/// Forget the transaction. The host closes the request's connections when it
/// ends, so the database rolls back whatever was left open.
pub fn clear_transaction() {
    TX.with(|tx| tx.borrow_mut().take());
}

pub fn claim_jobs(
    _now_iso: &str,
    _worker_id: &str,
    _locked_until_iso: &str,
    _batch: usize,
) -> Result<Vec<serde_json::Value>, String> {
    Err(unsupported("the job queue"))
}

pub fn claim_cron_slot(
    _key: &str,
    _expected_next_run_at: &str,
    _patch: serde_json::Value,
) -> Result<bool, String> {
    Err(unsupported("cron"))
}

// ---------- CRUD ----------

pub fn insert(
    table: &str,
    key: Option<&str>,
    document: serde_json::Value,
) -> Result<serde_json::Value, String> {
    super::ensured::write_with_table(table, has_active_tx(), ensure_table, |_| {
        write(table, key, document.clone(), false, "insert")
    })
}

pub fn update(
    table: &str,
    key: &str,
    document: serde_json::Value,
    merge: bool,
) -> Result<serde_json::Value, String> {
    super::ensured::write_with_table(table, has_active_tx(), ensure_table, |_| {
        write(table, Some(key), document.clone(), merge, "update")
    })
}

/// An insert, a replace or (with `merge`) an RFC 7396 patch — the statements
/// of the native adapter.
fn write(
    table: &str,
    key: Option<&str>,
    mut document: serde_json::Value,
    merge: bool,
    context: &str,
) -> Result<serde_json::Value, String> {
    let key = resolve_key(key, &document);
    if let Some(obj) = document.as_object_mut() {
        obj.insert("_key".to_string(), serde_json::json!(key));
    }
    let table_q = PG.quote_ident(table)?;
    if merge {
        return merge_into(&table_q, &key, &document);
    }
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES ($1, $2::jsonb) \
         ON CONFLICT (_key) DO UPDATE SET doc = EXCLUDED.doc RETURNING doc"
    );
    let rows = exec(
        context,
        &sql,
        &[SqlBind::Text(key), SqlBind::Text(document.to_string())],
    )?;
    one_doc(context, rows)?.ok_or_else(|| format!("{context}: row missing after write"))
}

/// The native adapter's merge: one statement when the patch has no nested
/// object (`||`, then drop the keys it nulls), otherwise a locked
/// read-merge-write — in a savepoint when a transaction is already open.
fn merge_into(
    table_q: &str,
    key: &str,
    document: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut fresh = serde_json::json!({});
    super::merge::merge_patch(&mut fresh, document);
    let insert_sql =
        format!("INSERT INTO {table_q} (_key, doc) VALUES ($1, $2::jsonb) RETURNING doc");
    let key_bind = || SqlBind::Text(key.to_string());

    if !super::merge::needs_recursive_merge(document) {
        let sql = format!(
            "UPDATE {table_q} SET doc = (COALESCE(doc, '{{}}'::jsonb) || $2::jsonb) - (\
                 SELECT COALESCE(array_agg(key), ARRAY[]::text[]) \
                 FROM jsonb_each($2::jsonb) WHERE value = 'null'::jsonb\
             ) WHERE _key = $1 RETURNING doc"
        );
        let rows = exec(
            "update (merge)",
            &sql,
            &[key_bind(), SqlBind::Text(document.to_string())],
        )?;
        if let Some(doc) = one_doc("update (merge)", rows)? {
            return Ok(doc);
        }
        let rows = exec(
            "update (merge insert)",
            &insert_sql,
            &[key_bind(), SqlBind::Text(fresh.to_string())],
        )?;
        return one_doc("update (merge insert)", rows)?
            .ok_or_else(|| "update (merge insert): row missing".to_string());
    }

    let (open, commit, undo) = if tx_open_on_active_connection() {
        (
            "SAVEPOINT soli_merge",
            "RELEASE SAVEPOINT soli_merge",
            "ROLLBACK TO SAVEPOINT soli_merge",
        )
    } else {
        ("BEGIN", "COMMIT", "ROLLBACK")
    };
    exec("update (merge begin)", open, &[])?;
    let result = (|| -> Result<serde_json::Value, String> {
        let select = format!("SELECT doc FROM {table_q} WHERE _key = $1 FOR UPDATE");
        let existing = one_doc(
            "update (merge select)",
            exec("update (merge select)", &select, &[key_bind()])?,
        )?;
        let (sql, doc) = match existing {
            Some(mut doc) => {
                super::merge::merge_patch(&mut doc, document);
                (
                    format!("UPDATE {table_q} SET doc = $2::jsonb WHERE _key = $1 RETURNING doc"),
                    doc,
                )
            }
            None => (insert_sql.clone(), fresh.clone()),
        };
        let rows = exec(
            "update (merge write)",
            &sql,
            &[key_bind(), SqlBind::Text(doc.to_string())],
        )?;
        one_doc("update (merge write)", rows)?
            .ok_or_else(|| "update (merge write): row missing".to_string())
    })();
    match result {
        Ok(doc) => {
            exec("update (merge commit)", commit, &[])?;
            Ok(doc)
        }
        Err(e) => {
            let _ = exec("update (merge rollback)", undo, &[]);
            Err(e)
        }
    }
}

pub fn insert_many(table: &str, rows: &[(String, serde_json::Value)]) -> Result<u64, String> {
    super::ensured::write_with_table(table, has_active_tx(), ensure_table, |_| {
        let compiled = compile_insert_many_d(PG, table, rows)?;
        Ok(exec("insert_many", &compiled.sql, &compiled.params)?.changes)
    })
}

pub fn get(table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    super::ensured::read_with_table(
        table,
        has_active_tx(),
        table_exists,
        || None,
        || {
            let table_q = PG.quote_ident(table)?;
            let sql = format!("SELECT doc FROM {table_q} WHERE _key = $1");
            one_doc("get", exec("get", &sql, &[SqlBind::Text(key.to_string())])?)
        },
    )
}

pub fn delete(table: &str, key: &str) -> Result<(), String> {
    super::ensured::read_with_table(
        table,
        has_active_tx(),
        table_exists,
        || (),
        || {
            let table_q = PG.quote_ident(table)?;
            let sql = format!("DELETE FROM {table_q} WHERE _key = $1");
            exec("delete", &sql, &[SqlBind::Text(key.to_string())]).map(|_| ())
        },
    )
}

pub fn select(q: &ListQuery) -> Result<Vec<serde_json::Value>, String> {
    super::ensured::read_with_table(&q.table, has_active_tx(), table_exists, Vec::new, || {
        let compiled = compile_select_d(PG, q)?;
        docs("select", exec("select", &compiled.sql, &compiled.params)?)
    })
}

pub fn select_by_keys(table: &str, keys: &[String]) -> Result<Vec<serde_json::Value>, String> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    super::ensured::read_with_table(table, has_active_tx(), table_exists, Vec::new, || {
        let compiled = compile_select_by_keys_d(PG, table, keys)?;
        docs("select", exec("select", &compiled.sql, &compiled.params)?)
    })
}

pub fn select_json_text_in(
    table: &str,
    field: &str,
    values: &[String],
) -> Result<Vec<serde_json::Value>, String> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    super::ensured::read_with_table(table, has_active_tx(), table_exists, Vec::new, || {
        let compiled = compile_select_json_text_in_d(PG, table, field, values)?;
        docs("select", exec("select", &compiled.sql, &compiled.params)?)
    })
}

/// An aggregate cell: a number, or the text a `numeric` can come back as.
fn aggregate_value(cell: &serde_json::Value) -> serde_json::Value {
    match cell {
        serde_json::Value::String(s) => super::columns::parse_aggregate_text(s.clone()),
        other => other.clone(),
    }
}

pub fn group_by(
    q: &ListQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    super::ensured::read_with_table(&q.table, has_active_tx(), table_exists, Vec::new, || {
        let compiled = compile_group_by_d(PG, q, group_fields, aggs)?;
        let mut names: Vec<String> = group_fields.to_vec();
        if aggs.is_empty() {
            names.push("n".into());
        } else {
            names.extend(aggs.iter().map(|a| a.alias.clone()));
        }
        let rows = exec("group_by", &compiled.sql, &compiled.params)?;
        Ok(rows
            .rows
            .into_iter()
            .map(|row| {
                // Group keys keep what the column held; aggregates are numbers.
                let map = names
                    .iter()
                    .cloned()
                    .zip(row)
                    .enumerate()
                    .map(|(i, (name, cell))| {
                        let value = if i >= group_fields.len() {
                            aggregate_value(&cell)
                        } else {
                            cell
                        };
                        (name, value)
                    })
                    .collect();
                serde_json::Value::Object(map)
            })
            .collect())
    })
}

pub fn count(q: &ListQuery) -> Result<i64, String> {
    super::ensured::read_with_table(
        &q.table,
        has_active_tx(),
        table_exists,
        || 0,
        || {
            let compiled = compile_count_d(PG, q)?;
            let rows = exec("count", &compiled.sql, &compiled.params)?;
            Ok(first_cell(&rows).and_then(as_i64).unwrap_or(0))
        },
    )
}

pub fn exists(q: &ListQuery) -> Result<bool, String> {
    super::ensured::read_with_table(
        &q.table,
        has_active_tx(),
        table_exists,
        || false,
        || {
            let compiled = compile_exists_d(PG, q)?;
            Ok(!exec("exists", &compiled.sql, &compiled.params)?
                .rows
                .is_empty())
        },
    )
}

pub fn aggregate(q: &ListQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    super::ensured::read_with_table(
        &q.table,
        has_active_tx(),
        table_exists,
        || serde_json::Value::Null,
        || {
            let compiled = compile_aggregate_d(PG, q, func, field)?;
            let rows = exec("aggregate", &compiled.sql, &compiled.params)?;
            if matches!(func, SqlAgg::Count) {
                return Ok(serde_json::json!(first_cell(&rows)
                    .and_then(as_i64)
                    .unwrap_or(0)));
            }
            Ok(match first_cell(&rows) {
                None | Some(serde_json::Value::Null) => serde_json::Value::Null,
                Some(cell) => match cell.as_f64() {
                    Some(f) => serde_json::json!(f),
                    None => aggregate_value(cell),
                },
            })
        },
    )
}

pub fn delete_all(q: &ListQuery) -> Result<u64, String> {
    super::ensured::read_with_table(
        &q.table,
        has_active_tx(),
        table_exists,
        || 0,
        || {
            let compiled = compile_delete_all_d(PG, q)?;
            Ok(exec("delete_all", &compiled.sql, &compiled.params)?.changes)
        },
    )
}

pub fn update_all(q: &ListQuery, patch: serde_json::Value) -> Result<u64, String> {
    super::ensured::read_with_table(
        &q.table,
        has_active_tx(),
        table_exists,
        || 0,
        || {
            let compiled = compile_update_all_d(PG, q, &patch)?;
            Ok(exec("update_all", &compiled.sql, &compiled.params)?.changes)
        },
    )
}

/// Add `delta` to a numeric JSON field in one statement, returning the new value.
pub fn increment_field(
    table: &str,
    key: &str,
    field: &str,
    delta: i64,
) -> Result<Option<i64>, String> {
    super::ensured::read_with_table(
        table,
        has_active_tx(),
        table_exists,
        || None,
        || {
            // Validated as an identifier before it reaches a JSON path literal.
            PG.quote_ident(field)?;
            let table_q = PG.quote_ident(table)?;
            let sql = format!(
                "UPDATE {table_q} SET doc = jsonb_set(doc, '{{{field}}}', \
                     to_jsonb(COALESCE((doc->>'{field}')::numeric, 0) + $1::bigint)) \
                 WHERE _key = $2 RETURNING (doc->>'{field}')"
            );
            let rows = exec(
                "increment",
                &sql,
                &[SqlBind::I64(delta), SqlBind::Text(key.to_string())],
            )?;
            Ok(first_cell(&rows)
                .and_then(cell_text)
                .and_then(|t| super::parse_counter(&t)))
        },
    )
}

/// `db.query(sql, params)` / `Model.find_by_sql`.
pub fn query_raw(sql: &str, params: &[SqlBind]) -> Result<Vec<serde_json::Value>, String> {
    let rows = exec("raw query", sql, params)?;
    if rows.columns.len() == 1 && rows.columns[0] == "doc" {
        return docs("raw query", rows);
    }
    let columns = rows.columns;
    Ok(rows
        .rows
        .into_iter()
        .map(|row| {
            let map = columns
                .iter()
                .cloned()
                .zip(row)
                .map(|(name, cell)| {
                    // As natively: text that reads as a number becomes one.
                    let value = match cell {
                        serde_json::Value::String(s) => super::raw_cell(Some(s)),
                        other => other,
                    };
                    (name, value)
                })
                .collect();
            serde_json::Value::Object(map)
        })
        .collect())
}

// ---------- schema and migrations ----------

pub fn ensure_table(table: &str) -> Result<(), String> {
    exec_script("ensure_table", &create_table_sql_d(PG, table)?)
}

pub fn drop_table(table: &str) -> Result<(), String> {
    // Tables may be gone after this: stop skipping their `ensure_table`.
    super::ensured::forget_all();
    exec_script("drop_table", &drop_table_sql_d(PG, table)?)
}

pub fn ensure_migrations_table() -> Result<(), String> {
    exec_script("migrations table", migrations_table_sql_d(PG))
}

pub fn list_applied_migrations() -> Result<Vec<(String, String)>, String> {
    ensure_migrations_table()?;
    let rows = exec(
        "list migrations",
        "SELECT version, name FROM \"_migrations\" ORDER BY version",
        &[],
    )?;
    Ok(rows
        .rows
        .into_iter()
        .map(|row| {
            let text = |i: usize| row.get(i).and_then(cell_text).unwrap_or_default();
            (text(0), text(1))
        })
        .collect())
}

pub fn record_migration(version: &str, name: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    exec(
        "record migration",
        "INSERT INTO \"_migrations\" (version, name) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[SqlBind::Text(version.into()), SqlBind::Text(name.into())],
    )
    .map(|_| ())
}

pub fn remove_migration(version: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    exec(
        "remove migration",
        "DELETE FROM \"_migrations\" WHERE version = $1",
        &[SqlBind::Text(version.into())],
    )
    .map(|_| ())
}

pub fn create_or_drop_database(_drop: bool) -> Result<String, String> {
    Err(
        "create or drop the database from `soli serve` or the database's own tools: \
         a Worker only connects to one"
            .to_string(),
    )
}

pub fn list_index_names(table: &str) -> Result<Vec<String>, String> {
    let rows = exec(
        "list indexes",
        "SELECT indexname FROM pg_indexes WHERE schemaname = ANY (current_schemas(false)) \
         AND tablename = $1",
        &[SqlBind::Text(table.to_string())],
    )?;
    Ok(rows
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .filter_map(|v| cell_text(&v))
        .collect())
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
    for sql in super::ddl::doc_index_sql(PG, table, fields, name, unique)? {
        execute_ddl(&sql)?;
    }
    Ok(true)
}

pub fn dump_schema() -> Result<String, String> {
    Err("dump the schema with `soli db:schema:dump` under `soli serve`".to_string())
}

/// Run compiled DDL (migrations).
pub fn execute_ddl(sql: &str) -> Result<(), String> {
    super::ensured::forget_all();
    exec_script("ddl", sql)
}

/// `db.execute(sql)`. The connection closes when the request ends, so a `SET`
/// cannot outlive it.
pub fn execute_raw(sql: &str) -> Result<(), String> {
    super::ensured::forget_all();
    exec_script("execute", sql)
}

// ---------- column-aware models ----------

/// The shape of an existing table, from `information_schema` as natively.
pub fn introspect_table(table: &str) -> Result<RawColumns, String> {
    let table_bind = || [SqlBind::Text(table.to_string())];
    let rows = exec(
        "introspect columns",
        "SELECT column_name, udt_name, is_nullable, \
                (column_default LIKE 'nextval(%' OR is_identity = 'YES') AS is_auto \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 \
         ORDER BY ordinal_position",
        &table_bind(),
    )?;
    let columns = rows
        .rows
        .iter()
        .map(|row| {
            let text = |i: usize| row.get(i).and_then(cell_text).unwrap_or_default();
            (
                text(0),
                text(1),
                String::new(),
                text(2).eq_ignore_ascii_case("YES"),
                row.get(3).is_some_and(truthy),
            )
        })
        .collect();
    let pk = exec(
        "introspect primary key",
        "SELECT kcu.column_name \
         FROM information_schema.table_constraints tc \
         JOIN information_schema.key_column_usage kcu \
           ON kcu.constraint_name = tc.constraint_name \
          AND kcu.table_schema = tc.table_schema \
         WHERE tc.table_schema = current_schema() AND tc.table_name = $1 \
           AND tc.constraint_type = 'PRIMARY KEY' \
         ORDER BY kcu.ordinal_position",
        &table_bind(),
    )?
    .rows
    .iter()
    .filter_map(|row| row.first().and_then(cell_text))
    .collect();
    Ok(RawColumns { columns, pk })
}

fn truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        serde_json::Value::String(s) => matches!(s.as_str(), "t" | "true" | "1"),
        _ => false,
    }
}

/// Postgres renders `timestamptz` as `2026-08-11 10:00:00+00`: re-emit the RFC
/// 3339 form Soli's DateTime parses, as the native adapter does.
fn normalize_datetime(raw: &str) -> String {
    let mut out = raw.replacen(' ', "T", 1);
    if let Some(t) = out.find('T') {
        if let Some(rel) = out[t..].rfind(['+', '-']) {
            if out.len() - (t + rel) == 3 {
                out.push_str(":00");
            }
        }
    }
    out
}

/// One row as a JSON object keyed by column name, typed by the declared
/// columns, as the native adapter builds it.
fn row_to_json(
    schema: &TableSchema,
    columns: &[String],
    row: &[serde_json::Value],
) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    let mut idx = 0usize;
    for name in columns {
        let Some(col) = schema.column(name) else {
            continue;
        };
        let cell = row.get(idx).unwrap_or(&serde_json::Value::Null);
        let value = match col.ty {
            ColType::Int => cell_text(cell)
                .and_then(|s| s.trim().parse::<i64>().ok())
                .map(|n| serde_json::json!(n))
                .unwrap_or(serde_json::Value::Null),
            ColType::Float => cell_text(cell)
                .and_then(|s| s.trim().parse::<f64>().ok())
                .map(|f| serde_json::json!(f))
                .unwrap_or(serde_json::Value::Null),
            ColType::Bool => match cell {
                serde_json::Value::Null => serde_json::Value::Null,
                other => serde_json::json!(truthy(other)),
            },
            ColType::Json => match cell {
                serde_json::Value::String(s) => {
                    serde_json::from_str(s).unwrap_or(serde_json::Value::Null)
                }
                other => other.clone(),
            },
            ColType::DateTime => cell_text(cell)
                .map(|s| serde_json::json!(normalize_datetime(&s)))
                .unwrap_or(serde_json::Value::Null),
            _ => cell_text(cell)
                .map(|s| serde_json::json!(s))
                .unwrap_or(serde_json::Value::Null),
        };
        out.insert(col.name.clone(), value);
        idx += 1;
    }
    // Mirror the key under `_key` when the table has no such column.
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

/// The first row of a statement whose RETURNING / SELECT list is the full row.
fn first_full_row(
    schema: &TableSchema,
    rows: &SqlRows,
) -> Result<Option<serde_json::Value>, String> {
    let columns = cols::selected_columns(schema, &None)?;
    Ok(rows
        .rows
        .first()
        .map(|row| row_to_json(schema, &columns, row)))
}

pub fn col_get(
    schema: &Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let compiled = cols::compile_get_cols(PG, schema, pk)?;
    first_full_row(
        schema,
        &exec("column get", &compiled.sql, &compiled.params)?,
    )
}

pub fn col_insert(
    schema: &Arc<TableSchema>,
    doc: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_insert_cols(PG, schema, doc)?;
    first_full_row(
        schema,
        &exec("column insert", &compiled.sql, &compiled.params)?,
    )?
    .ok_or_else(|| format!("column insert: no row returned for {:?}", schema.table))
}

pub fn col_update(
    schema: &Arc<TableSchema>,
    pk: &serde_json::Value,
    patch: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_update_cols(PG, schema, pk, patch)?;
    first_full_row(
        schema,
        &exec("column update", &compiled.sql, &compiled.params)?,
    )?
    .ok_or_else(|| format!("no row in {:?} with {} = {}", schema.table, schema.pk, pk))
}

pub fn col_delete(schema: &Arc<TableSchema>, pk: &serde_json::Value) -> Result<(), String> {
    let compiled = cols::compile_delete_cols(PG, schema, pk)?;
    exec("column delete", &compiled.sql, &compiled.params).map(|_| ())
}

pub fn col_increment(
    schema: &Arc<TableSchema>,
    pk: &serde_json::Value,
    column: &str,
    delta: i64,
) -> Result<Option<i64>, String> {
    let (sql, params) = cols::compile_increment_col(PG, schema, pk, column, delta)?;
    let rows = exec("column increment", &sql, &params)?;
    Ok(first_cell(&rows)
        .and_then(cell_text)
        .and_then(|t| super::parse_counter(&t)))
}

pub fn col_group_by(
    q: &cols::ColumnQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    let compiled = cols::compile_group_by_cols(PG, q, group_fields, aggs)?;
    let names = super::columns::group_result_names(group_fields, aggs);
    let numeric = super::columns::group_key_numeric_flags(&q.schema, group_fields);
    let rows = exec("column group_by", &compiled.sql, &compiled.params)?;
    Ok(rows
        .rows
        .iter()
        .map(|row| {
            let texts: Vec<Option<String>> = (0..names.len())
                .map(|i| row.get(i).and_then(cell_text))
                .collect();
            super::columns::group_row_to_json(&names, &texts, group_fields.len(), &numeric)
        })
        .collect())
}

pub fn col_delete_all(q: &cols::ColumnQuery) -> Result<u64, String> {
    let compiled = cols::compile_delete_all_cols(PG, q)?;
    Ok(exec("column delete_all", &compiled.sql, &compiled.params)?.changes)
}

pub fn col_update_all(q: &cols::ColumnQuery, patch: &serde_json::Value) -> Result<u64, String> {
    let compiled = cols::compile_update_all_cols(PG, q, patch)?;
    Ok(exec("column update_all", &compiled.sql, &compiled.params)?.changes)
}

pub fn col_select(q: &cols::ColumnQuery) -> Result<Vec<serde_json::Value>, String> {
    let compiled = cols::compile_select_cols(PG, q)?;
    let columns = cols::selected_columns(&q.schema, &q.select_fields)?;
    let rows = exec("column select", &compiled.sql, &compiled.params)?;
    Ok(rows
        .rows
        .iter()
        .map(|row| row_to_json(&q.schema, &columns, row))
        .collect())
}

pub fn col_count(q: &cols::ColumnQuery) -> Result<i64, String> {
    let compiled = cols::compile_count_cols(PG, q)?;
    let rows = exec("column count", &compiled.sql, &compiled.params)?;
    Ok(first_cell(&rows).and_then(as_i64).unwrap_or(0))
}

pub fn col_exists(q: &cols::ColumnQuery) -> Result<bool, String> {
    let compiled = cols::compile_exists_cols(PG, q)?;
    Ok(!exec("column exists", &compiled.sql, &compiled.params)?
        .rows
        .is_empty())
}

pub fn col_aggregate(
    q: &cols::ColumnQuery,
    func: SqlAgg,
    field: &str,
) -> Result<serde_json::Value, String> {
    let compiled = cols::compile_aggregate_cols(PG, q, func, field)?;
    let rows = exec("column aggregate", &compiled.sql, &compiled.params)?;
    if func == SqlAgg::Count {
        return Ok(serde_json::json!(first_cell(&rows)
            .and_then(as_i64)
            .unwrap_or(0)));
    }
    Ok(super::columns::parse_agg_text(
        first_cell(&rows).and_then(cell_text),
    ))
}

/// The edge build's executor: the host's `soli_sql` import (`edge/js/jspi.js`),
/// wrapped in `WebAssembly.Suspending` — the stack suspends until the driver
/// answers.
#[cfg(target_arch = "wasm32")]
mod host {
    use super::{SqlExecutor, SqlRequest, SqlRows};

    #[link(wasm_import_module = "./jspi.js")]
    extern "C" {
        /// Takes the JSON request at `ptr`/`len`; returns a buffer the host
        /// allocated with `soli_alloc`: a little-endian `u32` length, then that
        /// many bytes of JSON response. Ownership passes back to Rust.
        fn soli_sql(ptr: *const u8, len: usize) -> *mut u8;
    }

    pub(super) struct Driver;

    impl SqlExecutor for Driver {
        fn run(&self, request: &SqlRequest<'_>) -> Result<SqlRows, String> {
            let body = serde_json::json!({
                "target": request.target,
                "sql": request.sql,
                "params": request.params,
            })
            .to_string();
            // SAFETY: the host reads `len` bytes at `ptr` and returns a buffer
            // from `soli_alloc(4 + n)`, whose first 4 bytes are `n`.
            let response = unsafe {
                let out = soli_sql(body.as_ptr(), body.len());
                if out.is_null() {
                    return Err("the host returned no response".to_string());
                }
                let mut len = [0u8; 4];
                std::ptr::copy_nonoverlapping(out, len.as_mut_ptr(), 4);
                let total = 4 + u32::from_le_bytes(len) as usize;
                let buffer = Vec::from_raw_parts(out, total, total);
                buffer[4..].to_vec()
            };
            let json: serde_json::Value = serde_json::from_slice(&response)
                .map_err(|e| format!("malformed response from the host: {e}"))?;
            if json["ok"].as_bool() != Some(true) {
                return Err(json["error"]
                    .as_str()
                    .unwrap_or("unknown SQL error")
                    .to_string());
            }
            Ok(SqlRows {
                columns: serde_json::from_value(json["columns"].clone()).unwrap_or_default(),
                rows: serde_json::from_value(json["rows"].clone()).unwrap_or_default(),
                changes: json["changes"].as_u64().unwrap_or(0),
            })
        }
    }
}
