//! The Cloudflare D1 adapter, run against a real SQLite.
//!
//! D1 is SQLite behind a Worker binding. Natively there is no binding, so this
//! installs a `D1Executor` that hands each statement to an in-memory SQLite the
//! way D1's `prepare(sql).bind(...).all()` would, then drives the adapter from
//! Soli code: the SQL the edge build sends to D1 is SQL that actually ran.
#![cfg(feature = "sqlite")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::types::{Value as SqlValue, ValueRef};
use solilang::db::d1::{set_executor, D1Executor, D1Rows, D1Statement};
use solilang::db::registry::registry_test_lock;
use solilang::db::{
    clear_registry_override, set_registry_for_tests, Adapter, ConnectionRegistry, ConnectionSpec,
};

/// What D1 would do with a statement, on SQLite; remembers what it was sent.
struct SqliteAsD1 {
    conn: Mutex<rusqlite::Connection>,
    seen: Mutex<Vec<(String, String)>>,
    /// One entry per round trip: `run` is 1, a `batch()` its statement count.
    trips: Mutex<Vec<usize>>,
    /// Answer every `batch()` with an error, as D1 does when one statement fails.
    refuse_batches: AtomicBool,
}

impl SqliteAsD1 {
    fn new() -> Self {
        Self {
            conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
            seen: Mutex::new(Vec::new()),
            trips: Mutex::new(Vec::new()),
            refuse_batches: AtomicBool::new(false),
        }
    }
}

impl D1Executor for SqliteAsD1 {
    fn run(
        &self,
        binding: &str,
        sql: &str,
        params: &[serde_json::Value],
    ) -> Result<D1Rows, String> {
        self.trips.lock().unwrap().push(1);
        self.statement(binding, sql, params)
    }

    fn batch(&self, binding: &str, statements: &[D1Statement]) -> Result<Vec<D1Rows>, String> {
        self.trips.lock().unwrap().push(statements.len());
        if self.refuse_batches.load(Ordering::SeqCst) {
            return Err("D1_ERROR: the batch was refused".to_string());
        }
        statements
            .iter()
            .map(|s| self.statement(binding, &s.sql, &s.params))
            .collect()
    }
}

impl SqliteAsD1 {
    fn statement(
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
        let columns: Vec<String> = stmt.column_names().map(|c| c.to_string()).collect();
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
    let d1 = Arc::new(SqliteAsD1::new());
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
    set_executor(Arc::new(SqliteAsD1::new()));
    set_registry_for_tests(d1_registry("d1://DB"));
    let err = solilang::db::sql::begin_transaction(None).unwrap_err();
    clear_registry_override();
    assert!(err.contains("no interactive transactions"), "{err}");
}

const GROUPED_MODELS: &str = r##"
class Fruit < Model
end

class Crate < Model
end

def check(label, got, want)
  throw "#{label}: got #{got}, want #{want}" unless got == want
end
"##;

const GROUPED_READS: &str = r##"
apple = Fruit.find_by("title", "apple")
listed = nil
total = nil
top = nil
by_color = nil
found = nil
cherry = nil
crates = nil
grouped(fn() {
  listed = Fruit.order("title", "asc").all
  total = Fruit.count
  top = Fruit.order("views", "desc").first
  by_color = Fruit.group_by("color").all
  found = Fruit.find(apple._key)
  cherry = Fruit.find_by("title", "cherry")
  crates = Crate.count
})
check("all", listed.map { |f| f.title }.join(","), "apple,banana,cherry")
check("count", total, 3)
check("first", top.title, "banana")
check("group_by", by_color.length, 2)
check("find", found.title, "apple")
check("find_by", cherry.views, 7)
check("other model", crates, 1)
"##;

/// Seed two tables and remember them, so the block's reads need no table check.
fn seed_fruit(d1: &SqliteAsD1) {
    let seed = format!(
        "{GROUPED_MODELS}\n{}",
        r##"
Fruit.create({"title": "apple", "views": 3, "color": "red"})
Fruit.create({"title": "banana", "views": 10, "color": "yellow"})
Fruit.create({"title": "cherry", "views": 7, "color": "red"})
Crate.create({"label": "a"})
check("seeded", Fruit.count + Crate.count, 4)
"##
    );
    run(&seed).expect("seeding failed");
    d1.trips.lock().unwrap().clear();
}

#[test]
fn reads_in_a_grouped_block_reach_d1_in_one_batch() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let d1 = Arc::new(SqliteAsD1::new());
    set_executor(d1.clone());
    set_registry_for_tests(d1_registry("d1://DB"));
    seed_fruit(&d1);

    let result = run(&format!("{GROUPED_MODELS}\n{GROUPED_READS}"));
    let trips = d1.trips.lock().unwrap().clone();
    clear_registry_override();
    result.expect("the grouped reads failed against D1");
    // The find_by before the block, then the seven reads in one batch.
    assert_eq!(trips, vec![1, 7], "round trips: {trips:?}");
}

#[test]
fn a_refused_batch_falls_back_to_one_statement_each() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let d1 = Arc::new(SqliteAsD1::new());
    set_executor(d1.clone());
    set_registry_for_tests(d1_registry("d1://DB"));
    seed_fruit(&d1);
    d1.refuse_batches.store(true, Ordering::SeqCst);

    let result = run(&format!("{GROUPED_MODELS}\n{GROUPED_READS}"));
    let trips = d1.trips.lock().unwrap().clone();
    clear_registry_override();
    // Same answers: every read ran on its own after D1 refused the batch.
    result.expect("the grouped reads failed after a refused batch");
    assert_eq!(
        trips,
        vec![1, 7, 1, 1, 1, 1, 1, 1, 1],
        "round trips: {trips:?}"
    );
}

#[test]
fn a_read_that_needs_another_inside_the_block_still_gets_its_rows() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let d1 = Arc::new(SqliteAsD1::new());
    set_executor(d1.clone());
    set_registry_for_tests(d1_registry("d1://DB"));
    seed_fruit(&d1);

    let result = run(&format!(
        "{GROUPED_MODELS}\n{}",
        r##"
again = nil
total = nil
grouped(fn() {
  total = Fruit.count
  apple = Fruit.find_by("title", "apple")
  # Reading `apple` here flushes what is pending, then this read runs.
  again = Fruit.find(apple._key)
})
check("dependent find", again.title, "apple")
check("count", total, 3)
"##
    ));
    let trips = d1.trips.lock().unwrap().clone();
    clear_registry_override();
    result.expect("the dependent read failed");
    // count + find_by in one batch, then the find on its own.
    assert_eq!(trips, vec![2, 1], "round trips: {trips:?}");
}

#[test]
fn a_tenant_template_names_each_sites_binding() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let d1 = Arc::new(SqliteAsD1::new());
    set_executor(d1.clone());
    let mut registry = d1_registry("d1://MAIN");
    let mut site = registry.connections["primary"].clone();
    site.name = "site".to_string();
    site.url = Some("d1://SITE_{tenant}".to_string());
    registry.connections.insert("site".to_string(), site);
    set_registry_for_tests(registry);

    // A name of its own: the model registry is process-wide, and a `Note` on
    // the `site` connection would follow `models_work_on_d1`'s `Note` there.
    let result = run(r##"
class Account < Model
end

class SiteNote < Model
  connection "site"
end

Account.create({"name": "main"})
switch_tenant("acme")
SiteNote.create({"title": "for acme"})
switch_tenant("globex")
SiteNote.create({"title": "for globex"})
switch_tenant(nil)
"##);
    solilang::db::tenancy::clear();
    clear_registry_override();
    result.expect("the Soli script failed against D1");

    let seen = d1.seen.lock().unwrap();
    let bindings: std::collections::BTreeSet<_> = seen.iter().map(|(b, _)| b.as_str()).collect();
    assert_eq!(
        bindings,
        ["MAIN", "SITE_acme", "SITE_globex"].into_iter().collect(),
        "{seen:?}"
    );
    // Each site note went to its site's binding, the account to the main one.
    assert!(seen
        .iter()
        .any(|(b, sql)| b == "SITE_acme" && sql.contains("INSERT INTO \"site_notes\"")));
    assert!(seen
        .iter()
        .any(|(b, sql)| b == "MAIN" && sql.contains("INSERT INTO \"accounts\"")));
    assert!(!seen
        .iter()
        .any(|(b, sql)| b == "MAIN" && sql.contains("\"site_notes\"")));
}

#[test]
fn a_durable_object_url_names_one_object_per_tenant() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let d1 = Arc::new(SqliteAsD1::new());
    set_executor(d1.clone());
    let mut registry = d1_registry("d1://MAIN");
    let mut site = registry.connections["primary"].clone();
    site.name = "site".to_string();
    site.url = Some("do://SITES/{tenant}".to_string());
    registry.connections.insert("site".to_string(), site);
    set_registry_for_tests(registry);

    // Model names of their own: the model registry is process-wide.
    let result = run(r##"
class DoAccount < Model
end

class DoNote < Model
  connection "site"
end

def check(label, got, want)
  throw "#{label}: got #{got}, want #{want}" unless got == want
end

DoAccount.create({"name": "main"})
switch_tenant("acme")
DoNote.create({"title": "for acme"})
switch_tenant("globex")
DoNote.create({"title": "for globex"})

# An explicit url still has to name a binding and a well-formed object.
bad = with_tenant("odd", {"url": "do://SITES/a.b"}) do
  DoNote.count
end rescue "refused"
check("an object name with a dot", bad, "refused")
missing = with_tenant("odd", {"url": "do://SITES"}) do
  DoNote.count
end rescue "refused"
check("no object", missing, "refused")
switch_tenant(nil)
"##);
    solilang::db::tenancy::clear();
    clear_registry_override();
    result.expect("the Soli script failed");

    let seen = d1.seen.lock().unwrap();
    let targets: std::collections::BTreeSet<_> = seen.iter().map(|(b, _)| b.as_str()).collect();
    assert_eq!(
        targets,
        ["MAIN", "do://SITES/acme", "do://SITES/globex"]
            .into_iter()
            .collect(),
        "{seen:?}"
    );
    assert!(seen
        .iter()
        .any(|(b, sql)| b == "do://SITES/acme" && sql.contains("INSERT INTO \"do_notes\"")));
}
