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
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type MyPool = Pool<MySqlConnectionManager>;
type MyConn = r2d2::PooledConnection<MySqlConnectionManager>;

static POOLS: OnceLock<Mutex<HashMap<String, MyPool>>> = OnceLock::new();

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

fn with_conn<T>(f: impl FnOnce(&mut MyConn) -> Result<T, String>) -> Result<T, String> {
    let pool = pool_for_active()?;
    let mut conn = pool.get().map_err(|e| format!("mysql checkout: {e}"))?;
    f(&mut conn)
}

fn to_mysql_params(params: &[SqlBind]) -> Vec<MysqlValue> {
    params
        .iter()
        .map(|p| match p {
            SqlBind::Text(s) => MysqlValue::from(s.as_str()),
            SqlBind::I64(n) => MysqlValue::from(*n),
            SqlBind::Json(j) => MysqlValue::from(j.to_string()),
        })
        .collect()
}

// ---------- CRUD ----------

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
        get(table, &key)?.ok_or_else(|| "mysql insert: row missing after write".into())
    })
}

pub fn get(table: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    if !table_exists(table)? {
        return Ok(None);
    }
    let table_q = Dialect::Mysql.quote_ident(table)?;
    let sql = format!("SELECT doc FROM {table_q} WHERE _key = ?");
    with_conn(|conn| {
        let row: Option<String> = conn
            .exec_first(&sql, (key,))
            .map_err(|e| format!("mysql get: {e}"))?;
        match row {
            Some(s) => {
                let v: serde_json::Value =
                    serde_json::from_str(&s).map_err(|e| format!("mysql get json: {e}"))?;
                Ok(Some(v))
            }
            None => Ok(None),
        }
    })
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
            let affected = conn
                .exec_iter(&sql, (&doc_str, key))
                .map_err(|e| format!("mysql update merge: {e}"))?
                .affected_rows();
            if affected == 0 {
                // Insert patch as full document.
                let ins = format!("INSERT INTO {table_q} (_key, doc) VALUES (?, CAST(? AS JSON))");
                conn.exec_drop(&ins, (key, &doc_str))
                    .map_err(|e| format!("mysql update merge insert: {e}"))?;
            }
            get(table, key)?.ok_or_else(|| "mysql update: row missing".into())
        });
    }
    let sql = format!(
        "INSERT INTO {table_q} (_key, doc) VALUES (?, CAST(? AS JSON)) \
         ON DUPLICATE KEY UPDATE doc = VALUES(doc)"
    );
    with_conn(|conn| {
        conn.exec_drop(&sql, (key, &doc_str))
            .map_err(|e| format!("mysql update: {e}"))?;
        get(table, key)?.ok_or_else(|| "mysql update: row missing".into())
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
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    fn with_mysql(f: impl FnOnce()) {
        let _g = LOCK.lock().unwrap();
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
        f();
        clear_registry_override();
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
