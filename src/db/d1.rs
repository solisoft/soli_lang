//! Cloudflare D1 backend: the SQLite document model, through a Worker's binding.
//!
//! D1 is SQLite, so the SQL is the SQLite adapter's: the same `sql_compile`
//! generators in [`Dialect::Sqlite`], the same `_key` + JSON `doc` tables. What
//! differs is the connection. There is none to open: each statement goes to the
//! Worker's D1 binding (`env.DB.prepare(sql).bind(...).all()`), and every one is
//! a network round trip — so writes use `RETURNING doc` instead of reading the
//! row back, one statement instead of two.
//!
//! Statements reach D1 through a [`D1Executor`]. On the edge build it is the
//! host's `soli_d1` import, which the stack suspends on like any `fetch`
//! (`platform::jspi`). Natively there is no binding to call; tests install an
//! executor backed by a real SQLite, so the SQL sent to D1 is SQL that ran.
//!
//! D1 has no interactive transactions (only batches), so `transaction` is
//! refused, and neither the job engine's row claims nor column-aware tables
//! are supported yet.

use super::introspect::TableSchema;
use super::registry::active_spec;
use super::sql_columns_compile as cols;
use super::sql_compile::{
    compile_aggregate_d, compile_count_d, compile_delete_all_d, compile_exists_d,
    compile_group_by_d, compile_insert_many_d, compile_select_by_keys_d, compile_select_d,
    compile_select_json_text_in_d, compile_update_all_d, create_table_sql_d, drop_table_sql_d,
    migrations_table_sql_d, Dialect, GroupAgg, ListQuery, SqlAgg, SqlBind,
};
use std::sync::{Arc, RwLock};

/// What one statement returned: D1's `.all()`, flattened to positional rows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct D1Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Rows written (D1's `meta.changes`).
    pub changes: u64,
}

/// How a statement reaches D1. `params` are what D1's `bind()` takes: strings,
/// numbers and null.
pub trait D1Executor: Send + Sync {
    fn run(&self, binding: &str, sql: &str, params: &[serde_json::Value])
        -> Result<D1Rows, String>;
}

static EXECUTOR: RwLock<Option<Arc<dyn D1Executor>>> = RwLock::new(None);

/// Route D1 statements through `executor` (tests; a native D1 client later).
pub fn set_executor(executor: Arc<dyn D1Executor>) {
    *EXECUTOR.write().unwrap_or_else(|e| e.into_inner()) = Some(executor);
}

fn executor() -> Result<Arc<dyn D1Executor>, String> {
    if let Some(executor) = EXECUTOR.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return Ok(executor);
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(Arc::new(host::Binding))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(
            "a D1 database is reached through a Cloudflare Worker's binding: \
             run the app with `soli edge build` and `wrangler`"
                .to_string(),
        )
    }
}

/// The Worker binding the active connection names: `d1://DB` (or plain `DB`).
fn binding() -> Result<String, String> {
    let spec = active_spec()?;
    let url = spec.url.as_deref().unwrap_or("d1://DB");
    let name = url.strip_prefix("d1://").unwrap_or(url).trim_matches('/');
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "connection {:?}: a D1 url names the Worker binding, like d1://DB (got {url:?})",
            spec.name
        ));
    }
    Ok(name.to_string())
}

/// D1 refuses a statement with more than 100 bound parameters, where SQLite
/// takes 32,766: multi-row writes and key lists go in chunks under it.
const MAX_PARAMS: usize = 100;

fn bind_value(bind: &SqlBind) -> serde_json::Value {
    match bind {
        SqlBind::Text(s) => serde_json::Value::String(s.clone()),
        SqlBind::I64(n) => serde_json::json!(n),
        SqlBind::F64(f) => serde_json::json!(f),
        // SQLite has no boolean type; 0/1 is the documented representation.
        SqlBind::Bool(b) => serde_json::json!(i64::from(*b)),
        SqlBind::Json(j) => serde_json::Value::String(j.to_string()),
    }
}

/// Run one statement on the active connection's binding.
fn exec(context: &str, sql: &str, params: &[SqlBind]) -> Result<D1Rows, String> {
    let _trace = super::trace::start(sql, params);
    let bound: Vec<serde_json::Value> = params.iter().map(bind_value).collect();
    executor()?
        .run(&binding()?, sql, &bound)
        .map_err(|e| format!("d1 {context}: {e}"))
}

/// Run a script statement by statement: `prepare` takes one at a time, and
/// D1's `exec` wants each on a single line, which compiled DDL is not.
fn exec_script(context: &str, script: &str) -> Result<(), String> {
    for statement in split_statements(script) {
        exec(context, &statement, &[])?;
    }
    Ok(())
}

/// Split on `;` outside quotes, identifiers and comments.
fn split_statements(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = script.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                current.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => {
                    quote = Some(c);
                    current.push(c);
                }
                '-' if chars.peek() == Some(&'-') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                    current.push('\n');
                }
                ';' => {
                    if !current.trim().is_empty() {
                        out.push(current.trim().to_string());
                    }
                    current.clear();
                }
                _ => current.push(c),
            },
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

fn first_cell(rows: &D1Rows) -> Option<&serde_json::Value> {
    rows.rows.first().and_then(|row| row.first())
}

fn as_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|f| f as i64))
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// Documents from a `SELECT doc …`.
fn docs(context: &str, rows: D1Rows) -> Result<Vec<serde_json::Value>, String> {
    rows.rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .map(|cell| match cell {
            serde_json::Value::String(text) => {
                serde_json::from_str(&text).map_err(|e| format!("d1 {context} json: {e}"))
            }
            serde_json::Value::Null => Ok(serde_json::Value::Null),
            other => Ok(other),
        })
        .collect()
}

fn one_doc(context: &str, rows: D1Rows) -> Result<Option<serde_json::Value>, String> {
    Ok(docs(context, rows)?.into_iter().next())
}

fn unsupported(what: &str) -> String {
    format!("{what} is not supported on Cloudflare D1")
}

fn table_exists(table: &str) -> Result<bool, String> {
    let rows = exec(
        "table_exists",
        "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?",
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

pub fn ensure_connected() -> Result<(), String> {
    exec("connect", "SELECT 1", &[]).map(|_| ())
}

pub fn has_active_tx() -> bool {
    false
}

pub fn begin_transaction(_isolation_level: Option<&str>) -> Result<String, String> {
    Err(
        "Cloudflare D1 has no interactive transactions (it only runs batches): \
         `transaction` is not available on a D1 connection"
            .to_string(),
    )
}

pub fn commit_transaction() -> Result<(), String> {
    Err("no transaction is open (Cloudflare D1 has none)".to_string())
}

pub fn rollback_transaction() -> Result<(), String> {
    Err("no transaction is open (Cloudflare D1 has none)".to_string())
}

pub fn clear_transaction() {}

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
    super::ensured::write_with_table(table, false, ensure_table, |_| {
        upsert(table, key, document.clone(), false, "insert")
    })
}

pub fn update(
    table: &str,
    key: &str,
    document: serde_json::Value,
    merge: bool,
) -> Result<serde_json::Value, String> {
    super::ensured::write_with_table(table, false, ensure_table, |_| {
        upsert(table, Some(key), document.clone(), merge, "update")
    })
}

/// One `INSERT … ON CONFLICT … RETURNING doc`: an insert, a replace, or (with
/// `merge`) an RFC 7396 patch via `json_patch`, as in the SQLite adapter.
fn upsert(
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
    let table_q = Dialect::Sqlite.quote_ident(table)?;
    let set = if merge {
        "doc = json_patch(COALESCE(doc, '{}'), excluded.doc)"
    } else {
        "doc = excluded.doc"
    };
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES (?, json(?)) \
         ON CONFLICT(_key) DO UPDATE SET {set} RETURNING doc"
    );
    let rows = exec(
        context,
        &sql,
        &[SqlBind::Text(key), SqlBind::Text(document.to_string())],
    )?;
    one_doc(context, rows)?.ok_or_else(|| format!("d1 {context}: row missing after write"))
}

pub fn insert_many(table: &str, rows: &[(String, serde_json::Value)]) -> Result<u64, String> {
    super::ensured::write_with_table(table, false, ensure_table, |_| {
        // Two parameters a row (`_key`, `doc`).
        let mut written = 0;
        for chunk in rows.chunks(MAX_PARAMS / 2) {
            let compiled = compile_insert_many_d(Dialect::Sqlite, table, chunk)?;
            written += exec("insert_many", &compiled.sql, &compiled.params)?.changes;
        }
        Ok(written)
    })
}

pub fn get(table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    super::ensured::read_with_table(
        table,
        false,
        table_exists,
        || None,
        || {
            let table_q = Dialect::Sqlite.quote_ident(table)?;
            let sql = format!("SELECT doc FROM {table_q} WHERE _key = ?");
            let rows = exec("get", &sql, &[SqlBind::Text(key.to_string())])?;
            one_doc("get", rows)
        },
    )
}

pub fn delete(table: &str, key: &str) -> Result<(), String> {
    super::ensured::read_with_table(
        table,
        false,
        table_exists,
        || (),
        || {
            let table_q = Dialect::Sqlite.quote_ident(table)?;
            let sql = format!("DELETE FROM {table_q} WHERE _key = ?");
            exec("delete", &sql, &[SqlBind::Text(key.to_string())]).map(|_| ())
        },
    )
}

pub fn select(q: &ListQuery) -> Result<Vec<serde_json::Value>, String> {
    super::ensured::read_with_table(&q.table, false, table_exists, Vec::new, || {
        let compiled = compile_select_d(Dialect::Sqlite, q)?;
        docs("select", exec("select", &compiled.sql, &compiled.params)?)
    })
}

pub fn select_by_keys(table: &str, keys: &[String]) -> Result<Vec<serde_json::Value>, String> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    super::ensured::read_with_table(table, false, table_exists, Vec::new, || {
        let mut out = Vec::with_capacity(keys.len());
        for chunk in keys.chunks(MAX_PARAMS) {
            let compiled = compile_select_by_keys_d(Dialect::Sqlite, table, chunk)?;
            out.extend(docs(
                "select",
                exec("select", &compiled.sql, &compiled.params)?,
            )?);
        }
        Ok(out)
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
    super::ensured::read_with_table(table, false, table_exists, Vec::new, || {
        let mut out = Vec::new();
        for chunk in values.chunks(MAX_PARAMS) {
            let compiled = compile_select_json_text_in_d(Dialect::Sqlite, table, field, chunk)?;
            out.extend(docs(
                "select",
                exec("select", &compiled.sql, &compiled.params)?,
            )?);
        }
        Ok(out)
    })
}

pub fn group_by(
    q: &ListQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    super::ensured::read_with_table(&q.table, false, table_exists, Vec::new, || {
        let compiled = compile_group_by_d(Dialect::Sqlite, q, group_fields, aggs)?;
        // Column order: group fields, then agg aliases (or "n" when there are none).
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
                let map = names.iter().cloned().zip(row).collect();
                serde_json::Value::Object(map)
            })
            .collect())
    })
}

pub fn count(q: &ListQuery) -> Result<i64, String> {
    super::ensured::read_with_table(
        &q.table,
        false,
        table_exists,
        || 0,
        || {
            let compiled = compile_count_d(Dialect::Sqlite, q)?;
            let rows = exec("count", &compiled.sql, &compiled.params)?;
            Ok(first_cell(&rows).and_then(as_i64).unwrap_or(0))
        },
    )
}

pub fn exists(q: &ListQuery) -> Result<bool, String> {
    super::ensured::read_with_table(
        &q.table,
        false,
        table_exists,
        || false,
        || {
            let compiled = compile_exists_d(Dialect::Sqlite, q)?;
            Ok(!exec("exists", &compiled.sql, &compiled.params)?
                .rows
                .is_empty())
        },
    )
}

pub fn aggregate(q: &ListQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    super::ensured::read_with_table(
        &q.table,
        false,
        table_exists,
        || serde_json::Value::Null,
        || {
            let compiled = compile_aggregate_d(Dialect::Sqlite, q, func, field)?;
            let rows = exec("aggregate", &compiled.sql, &compiled.params)?;
            // COUNT of nothing is 0; SUM of nothing is null.
            Ok(match first_cell(&rows) {
                None if matches!(func, SqlAgg::Count) => serde_json::json!(0),
                None => serde_json::Value::Null,
                Some(v) => v.clone(),
            })
        },
    )
}

pub fn delete_all(q: &ListQuery) -> Result<u64, String> {
    super::ensured::read_with_table(
        &q.table,
        false,
        table_exists,
        || 0,
        || {
            let compiled = compile_delete_all_d(Dialect::Sqlite, q)?;
            Ok(exec("delete_all", &compiled.sql, &compiled.params)?.changes)
        },
    )
}

pub fn update_all(q: &ListQuery, patch: serde_json::Value) -> Result<u64, String> {
    super::ensured::read_with_table(
        &q.table,
        false,
        table_exists,
        || 0,
        || {
            let compiled = compile_update_all_d(Dialect::Sqlite, q, &patch)?;
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
        false,
        table_exists,
        || None,
        || {
            Dialect::Sqlite.quote_ident(field)?;
            let table_q = Dialect::Sqlite.quote_ident(table)?;
            let sql = format!(
                "UPDATE {table_q} SET doc = json_set(doc, '$.{field}', \
                 COALESCE(doc ->> '$.{field}', 0) + ?1) \
             WHERE _key = ?2 RETURNING CAST((doc ->> '$.{field}') AS TEXT)"
            );
            let rows = exec(
                "increment",
                &sql,
                &[SqlBind::I64(delta), SqlBind::Text(key.to_string())],
            )?;
            Ok(first_cell(&rows)
                .and_then(|v| v.as_str())
                .and_then(super::parse_counter))
        },
    )
}

/// `db.query(sql, params)` on a SQL connection.
pub fn query_raw(sql: &str, params: &[SqlBind]) -> Result<Vec<serde_json::Value>, String> {
    let rows = exec("raw query", sql, params)?;
    if rows.columns.len() == 1 && rows.columns[0] == "doc" {
        return docs("raw query", rows);
    }
    let columns = rows.columns;
    Ok(rows
        .rows
        .into_iter()
        .map(|row| serde_json::Value::Object(columns.iter().cloned().zip(row).collect()))
        .collect())
}

// ---------- schema and migrations ----------

pub fn ensure_table(table: &str) -> Result<(), String> {
    exec_script("ensure_table", &create_table_sql_d(Dialect::Sqlite, table)?)
}

pub fn drop_table(table: &str) -> Result<(), String> {
    // Tables may be gone after this: stop skipping their `ensure_table`.
    super::ensured::forget_all();
    exec_script("drop_table", &drop_table_sql_d(Dialect::Sqlite, table)?)
}

pub fn ensure_migrations_table() -> Result<(), String> {
    exec_script("migrations table", migrations_table_sql_d(Dialect::Sqlite))
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
            let text = |i: usize| {
                row.get(i)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            (text(0), text(1))
        })
        .collect())
}

pub fn record_migration(version: &str, name: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    exec(
        "record migration",
        "INSERT OR IGNORE INTO \"_migrations\" (version, name) VALUES (?, ?)",
        &[SqlBind::Text(version.into()), SqlBind::Text(name.into())],
    )
    .map(|_| ())
}

pub fn remove_migration(version: &str) -> Result<(), String> {
    ensure_migrations_table()?;
    exec(
        "remove migration",
        "DELETE FROM \"_migrations\" WHERE version = ?",
        &[SqlBind::Text(version.into())],
    )
    .map(|_| ())
}

pub fn create_or_drop_database(_drop: bool) -> Result<String, String> {
    Err("a D1 database is created and deleted with Wrangler: \
         `npx wrangler d1 create <name>` / `npx wrangler d1 delete <name>`"
        .to_string())
}

pub fn list_index_names(table: &str) -> Result<Vec<String>, String> {
    let rows = exec(
        "list indexes",
        "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = ?",
        &[SqlBind::Text(table.to_string())],
    )?;
    Ok(rows
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect())
}

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

/// `CREATE` text for the app's tables and indexes (D1's own `_cf_*` excluded).
pub fn dump_schema() -> Result<String, String> {
    let versions = list_applied_migrations().unwrap_or_default();
    let rows = exec(
        "dump",
        "SELECT sql FROM sqlite_master \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '\\_cf\\_%' ESCAPE '\\' \
         ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, name",
        &[],
    )?;
    let stmts: Vec<String> = rows
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .filter_map(|v| v.as_str().map(str::to_string))
        .filter(|sql| !sql.trim().is_empty())
        .collect();
    Ok(super::schema_dump::format_dump("d1", &versions, &stmts))
}

/// Run compiled DDL (migrations).
pub fn execute_ddl(sql: &str) -> Result<(), String> {
    super::ensured::forget_all();
    exec_script("ddl", sql)
}

/// `db.execute(sql)`.
pub fn execute_raw(sql: &str) -> Result<(), String> {
    super::ensured::forget_all();
    exec_script("execute", sql)
}

// ---------- column-aware models: not yet ----------

fn no_column_tables() -> String {
    unsupported("a column-aware model (one that declares its columns)")
}

pub fn col_get(
    _schema: &Arc<TableSchema>,
    _pk: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    Err(no_column_tables())
}

pub fn col_insert(
    _schema: &Arc<TableSchema>,
    _doc: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    Err(no_column_tables())
}

pub fn col_update(
    _schema: &Arc<TableSchema>,
    _pk: &serde_json::Value,
    _patch: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    Err(no_column_tables())
}

pub fn col_delete(_schema: &Arc<TableSchema>, _pk: &serde_json::Value) -> Result<(), String> {
    Err(no_column_tables())
}

pub fn col_increment(
    _schema: &Arc<TableSchema>,
    _pk: &serde_json::Value,
    _column: &str,
    _delta: i64,
) -> Result<Option<i64>, String> {
    Err(no_column_tables())
}

pub fn col_group_by(
    _q: &cols::ColumnQuery,
    _group_fields: &[String],
    _aggs: &[GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    Err(no_column_tables())
}

pub fn col_delete_all(_q: &cols::ColumnQuery) -> Result<u64, String> {
    Err(no_column_tables())
}

pub fn col_update_all(_q: &cols::ColumnQuery, _patch: &serde_json::Value) -> Result<u64, String> {
    Err(no_column_tables())
}

pub fn col_select(_q: &cols::ColumnQuery) -> Result<Vec<serde_json::Value>, String> {
    Err(no_column_tables())
}

pub fn col_count(_q: &cols::ColumnQuery) -> Result<i64, String> {
    Err(no_column_tables())
}

pub fn col_exists(_q: &cols::ColumnQuery) -> Result<bool, String> {
    Err(no_column_tables())
}

pub fn col_aggregate(
    _q: &cols::ColumnQuery,
    _func: SqlAgg,
    _field: &str,
) -> Result<serde_json::Value, String> {
    Err(no_column_tables())
}

/// The edge build's executor: the host's `soli_d1` import (`edge/js/jspi.js`),
/// wrapped in `WebAssembly.Suspending` — the stack suspends until D1 answers.
#[cfg(target_arch = "wasm32")]
mod host {
    use super::{D1Executor, D1Rows};

    #[link(wasm_import_module = "./jspi.js")]
    extern "C" {
        /// Takes the JSON request at `ptr`/`len`; returns a buffer the host
        /// allocated with `soli_alloc`: a little-endian `u32` length, then that
        /// many bytes of JSON response. Ownership passes back to Rust.
        fn soli_d1(ptr: *const u8, len: usize) -> *mut u8;
    }

    pub(super) struct Binding;

    impl D1Executor for Binding {
        fn run(
            &self,
            binding: &str,
            sql: &str,
            params: &[serde_json::Value],
        ) -> Result<D1Rows, String> {
            let request =
                serde_json::json!({ "binding": binding, "sql": sql, "params": params }).to_string();
            // SAFETY: the host reads `len` bytes at `ptr` and returns a buffer
            // from `soli_alloc(4 + n)`, whose first 4 bytes are `n`.
            let response = unsafe {
                let out = soli_d1(request.as_ptr(), request.len());
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
                    .unwrap_or("unknown D1 error")
                    .to_string());
            }
            Ok(D1Rows {
                columns: serde_json::from_value(json["columns"].clone()).unwrap_or_default(),
                rows: serde_json::from_value(json["rows"].clone()).unwrap_or_default(),
                changes: json["changes"].as_u64().unwrap_or(0),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_script_into_statements() {
        let script = "CREATE TABLE \"a;b\" (x TEXT DEFAULT ';');\n-- a comment; here\nCREATE INDEX i ON t (x);  ";
        assert_eq!(
            split_statements(script),
            vec![
                "CREATE TABLE \"a;b\" (x TEXT DEFAULT ';')".to_string(),
                "CREATE INDEX i ON t (x)".to_string(),
            ]
        );
    }

    #[test]
    fn binds_what_d1_accepts() {
        assert_eq!(bind_value(&SqlBind::Bool(true)), serde_json::json!(1));
        assert_eq!(
            bind_value(&SqlBind::Json(serde_json::json!({"a": 1}))),
            serde_json::json!("{\"a\":1}")
        );
        assert_eq!(bind_value(&SqlBind::I64(7)), serde_json::json!(7));
    }
}
