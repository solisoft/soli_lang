//! PostgreSQL document backend (Phase 1).
//!
//! Each model collection is a table with `_key TEXT PRIMARY KEY` and
//! `doc JSONB` holding the full Soli document (including system fields).

use super::registry::{active_connection_name, active_spec};
use super::sql_compile::{
    compile_aggregate_d, compile_count_d, compile_delete_all_d, compile_exists_d,
    compile_group_by_d, compile_select_by_keys_d, compile_select_d, compile_select_json_text_in_d,
    compile_update_all_d, create_table_sql_d, drop_table_sql_d, migrations_table_sql_d, Dialect,
    GroupAgg, ListQuery, SoftDeleteMode, SqlAgg, SqlBind,
};
use postgres::types::{ToSql, Type};
use r2d2::Pool;
use r2d2_postgres::{postgres::NoTls, PostgresConnectionManager};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type PgPool = Pool<PostgresConnectionManager<NoTls>>;

static POOLS: OnceLock<Mutex<HashMap<String, PgPool>>> = OnceLock::new();

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

fn with_conn<T>(f: impl FnOnce(&mut postgres::Client) -> Result<T, String>) -> Result<T, String> {
    let pool = pool_for_active()?;
    let mut conn = pool.get().map_err(|e| format!("postgres checkout: {e}"))?;
    f(&mut conn)
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

pub fn execute_ddl(sql: &str) -> Result<(), String> {
    with_conn(|client| {
        client
            .batch_execute(sql)
            .map_err(|e| format!("postgres ddl: {e}"))
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
    Null,
    I64(i64),
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
            OwnedParam::Null => Ok(postgres::types::IsNull::Yes),
            OwnedParam::I64(n) => (*n).to_sql(ty, out),
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
            SqlBind::Json(j) => match j {
                serde_json::Value::Null => OwnedParam::Null,
                other => OwnedParam::Json(other.clone()),
            },
            SqlBind::I64(n) => OwnedParam::I64(*n),
            SqlBind::Text(s) => OwnedParam::Text(s.clone()),
        })
        .collect()
}

fn bind_refs(owned: &[OwnedParam]) -> Vec<&(dyn ToSql + Sync)> {
    owned.iter().map(|p| p as &(dyn ToSql + Sync)).collect()
}

/// Inputs for [`list_query_from_parts`].
pub struct ListQueryParts {
    pub table: String,
    pub filter_sdbql: Option<String>,
    pub bind_vars: HashMap<String, serde_json::Value>,
    pub soft_delete: SoftDeleteMode,
    pub is_soft_delete_model: bool,
    pub order_field: Option<String>,
    pub order_desc: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Build a ListQuery IR (dialect-agnostic filters); validates with `dialect`.
pub fn list_query_from_parts(parts: ListQueryParts, dialect: Dialect) -> Result<ListQuery, String> {
    let mut eq_filters = BTreeMap::new();
    if let Some(ref f) = parts.filter_sdbql {
        if !f.trim().is_empty() {
            for (k, v) in &parts.bind_vars {
                if k.starts_with("__soli_") {
                    continue;
                }
                eq_filters.insert(k.clone(), v.clone());
            }
        }
    }
    let q = ListQuery {
        table: parts.table,
        eq_filters,
        filter_sdbql: parts.filter_sdbql,
        soft_delete: parts.soft_delete,
        is_soft_delete_model: parts.is_soft_delete_model,
        order_field: parts.order_field,
        order_desc: parts.order_desc,
        limit: parts.limit,
        offset: parts.offset,
    };
    let _ = compile_select_d(dialect, &q)?;
    Ok(q)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    fn with_pg(f: impl FnOnce()) {
        let _g = LOCK.lock().unwrap();
        let url = std::env::var("PG_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()
            .filter(|u| u.starts_with("postgres"))
            .unwrap_or_else(|| "postgres://soli@localhost:5432/soli_test".into());
        if postgres::Client::connect(&url, NoTls).is_err() {
            eprintln!("skip: postgres not reachable at {url}");
            return;
        }
        use crate::db::registry::{set_registry_for_tests, clear_registry_override, ConnectionRegistry, ConnectionSpec};
        use crate::db::Adapter;
        use std::collections::HashMap;
        let mut connections = HashMap::new();
        connections.insert(
            "primary".into(),
            ConnectionSpec {
                name: "primary".into(),
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
        set_registry_for_tests(ConnectionRegistry {
            default: "primary".into(),
            connections,
            from_file: false,
        });
        f();
        clear_registry_override();
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
