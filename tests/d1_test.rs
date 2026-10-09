//! The Cloudflare D1 adapter, run against a real SQLite.
//!
//! D1 is SQLite behind a Worker binding. Natively there is no binding, so this
//! installs a `D1Executor` that hands each statement to an in-memory SQLite the
//! way D1's `prepare(sql).bind(...).all()` would, then drives the adapter from
//! Soli code: the SQL the edge build sends to D1 is SQL that actually ran.
#![cfg(feature = "sqlite")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::types::{Value as SqlValue, ValueRef};
use solilang::db::d1::{set_executor, D1Executor, D1Rows};
use solilang::db::registry::registry_test_lock;
use solilang::db::{
    clear_registry_override, set_registry_for_tests, Adapter, ConnectionRegistry, ConnectionSpec,
};

/// What D1 would do with a statement, on SQLite; remembers what it was sent.
struct SqliteAsD1 {
    conn: Mutex<rusqlite::Connection>,
    seen: Mutex<Vec<(String, String)>>,
}

impl D1Executor for SqliteAsD1 {
    fn run(
        &self,
        binding: &str,
        sql: &str,
        params: &[serde_json::Value],
    ) -> Result<D1Rows, String> {
        self.seen
            .lock()
            .unwrap()
            .push((binding.to_string(), sql.to_string()));
        // D1's own limit, so a statement this adapter builds too big fails here too.
        if params.len() > 100 {
            return Err(format!(
                "D1_ERROR: too many SQL variables ({})",
                params.len()
            ));
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(sql).map_err(|e| format!("D1_ERROR: {e}"))?;
        let columns: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
        // D1's bind() takes strings, numbers and null — nothing else arrives.
        let bound: Vec<SqlValue> = params
            .iter()
            .map(|p| match p {
                serde_json::Value::Null => SqlValue::Null,
                serde_json::Value::String(s) => SqlValue::Text(s.clone()),
                serde_json::Value::Number(n) => match n.as_i64() {
                    Some(i) => SqlValue::Integer(i),
                    None => SqlValue::Real(n.as_f64().unwrap()),
                },
                other => panic!("D1 cannot bind {other}"),
            })
            .collect();
        let mut rows = Vec::new();
        let mut cursor = stmt
            .query(rusqlite::params_from_iter(bound.iter()))
            .map_err(|e| format!("D1_ERROR: {e}"))?;
        while let Some(row) = cursor.next().map_err(|e| format!("D1_ERROR: {e}"))? {
            let cells = (0..columns.len())
                .map(|i| match row.get_ref(i).unwrap() {
                    ValueRef::Null => serde_json::Value::Null,
                    ValueRef::Integer(n) => serde_json::json!(n),
                    ValueRef::Real(f) => serde_json::json!(f),
                    ValueRef::Text(t) => serde_json::json!(String::from_utf8_lossy(t)),
                    ValueRef::Blob(_) => serde_json::Value::Null,
                })
                .collect();
            rows.push(cells);
        }
        drop(cursor);
        Ok(D1Rows {
            columns,
            rows,
            changes: conn.changes(),
        })
    }
}

fn d1_registry(url: &str) -> ConnectionRegistry {
    let mut connections = HashMap::new();
    connections.insert(
        "primary".to_string(),
        ConnectionSpec {
            name: "primary".to_string(),
            adapter: Adapter::D1,
            url: Some(url.to_string()),
            solidb_host: None,
            solidb_database: None,
            solidb_username: None,
            solidb_password: None,
            solidb_api_key: None,
            pool_size: None,
        },
    );
    ConnectionRegistry {
        default: "primary".to_string(),
        connections,
        from_file: false,
    }
}

fn run(source: &str) -> Result<(), String> {
    let (_, result) = solilang::run_with_path_and_coverage(source, None, false, None, None, &[]);
    result.map_err(|e| e.to_string())
}

#[test]
fn models_work_on_d1() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let d1 = Arc::new(SqliteAsD1 {
        conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        seen: Mutex::new(Vec::new()),
    });
    set_executor(d1.clone());
    set_registry_for_tests(d1_registry("d1://NOTES_DB"));

    let result = run(r##"
class Note < Model
end

def check(label, got, want)
  throw "#{label}: got #{got}, want #{want}" unless got == want
end

a = Note.create({"title": "apple", "views": 3, "tags": ["red"]})
b = Note.create({"title": "banana", "views": 10})
Note.create({"title": "cherry", "views": 7})
check("key", a._key.length > 0, true)
check("count", Note.count, 3)

found = Note.find(a._key)
check("find", found.title, "apple")
check("nested json", found.tags[0], "red")

check("where", Note.where({"views": {"gte": 7}}).order("title", "asc").all.map { |n| n.title }.join(","), "banana,cherry")
check("first", Note.order("views", "desc").first.title, "banana")
check("exists", Note.where({"title": "cherry"}).count, 1)

b.views = 11
b.save
check("save", Note.find(b._key).views, 11)
Note.update(a._key, {"views": 4})
check("merge update", Note.find(a._key).views, 4)
check("merge kept the rest", Note.find(a._key).tags[0], "red")

total = Note.aggregate({"total": ["sum", "views"]}).first
check("sum", total["total"], 22)

Note.where({"title": "cherry"}).update_all({"views": 0})
check("update_all", Note.find_by("title", "cherry").views, 0)

a.delete
check("delete", Note.count, 2)
check("find_by miss", Note.find_by("title", "apple"), nil)

miss = Note.find("nope") rescue "raised"
check("find miss raises", miss, "raised")

Note.where({"views": {"lt": 5}}).delete_all
check("delete_all", Note.count, 1)

# 120 rows = 240 parameters: past D1's 100, so the adapter must chunk.
created = Note.create_many((0..120).map { |i| {"title": "bulk #{i}", "views": i} })
check("create_many", created["created"], 120)
check("count after bulk", Note.count, 121)

refused = Note.transaction(fn() { Note.create({"title": "in tx"}) }) rescue "refused"
check("transaction", refused, "refused")
"##);
    clear_registry_override();
    result.expect("the Soli script failed against D1");

    // Every statement went to the binding the URL names.
    let seen = d1.seen.lock().unwrap();
    assert!(
        seen.iter().all(|(binding, _)| binding == "NOTES_DB"),
        "{seen:?}"
    );
    // Writes come back in one statement (RETURNING), not insert-then-select.
    assert!(
        seen.iter().any(|(_, sql)| sql.contains("RETURNING doc")),
        "{seen:?}"
    );
    // D1 has no interactive transactions: no BEGIN ever reaches it.
    assert!(
        !seen.iter().any(|(_, sql)| sql.contains("BEGIN")),
        "{seen:?}"
    );
}

#[test]
fn transactions_are_refused_with_a_reason() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    set_executor(Arc::new(SqliteAsD1 {
        conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        seen: Mutex::new(Vec::new()),
    }));
    set_registry_for_tests(d1_registry("d1://DB"));
    let err = solilang::db::sql::begin_transaction(None).unwrap_err();
    clear_registry_override();
    assert!(err.contains("no interactive transactions"), "{err}");
}
