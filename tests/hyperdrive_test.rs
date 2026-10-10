//! The edge build's Postgres adapter, run against a real server.
//!
//! On a Worker each statement goes to the JavaScript driver `pg` through the
//! host's `soli_sql` import. Natively this installs a `SqlExecutor` that does
//! what `pg` does with a statement — parameters sent untyped, as text the
//! server infers a type for — on a real connection, then drives models from
//! Soli code: the SQL the edge build sends is SQL that ran on Postgres.
//!
//! Like the native adapter tests, it skips without a server
//! (`PG_DATABASE_URL`), and fails instead with `SOLI_REQUIRE_DB=1`.
#![cfg(feature = "postgres")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use solilang::db::hyperdrive::{set_executor, SqlExecutor, SqlRequest, SqlRows};
use solilang::db::registry::registry_test_lock;
use solilang::db::{
    clear_registry_override, set_registry_for_tests, Adapter, ConnectionRegistry, ConnectionSpec,
};

/// Skip without a server, unless CI said one must be there.
fn server_url(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(url) if !url.is_empty() => Some(url),
        _ => {
            if std::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1") {
                panic!("SOLI_REQUIRE_DB=1 but {var} is not set");
            }
            eprintln!("skip: {var} is not set");
            None
        }
    }
}

/// A parameter as the drivers send it: text, or NULL.
fn param_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Replace each `$n` outside quotes with the quoted literal of its parameter:
/// an untyped string the server infers a type for, as it does for the text
/// parameters `pg` sends.
fn inline(sql: &str, params: &[serde_json::Value]) -> String {
    let literal = |i: usize| match params.get(i).and_then(param_text) {
        Some(text) => format!("'{}'", text.replace('\'', "''")),
        None => "NULL".to_string(),
    };
    let mut out = String::new();
    let mut chars = sql.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                out.push(c);
            }
            '$' if chars.peek().is_some_and(|d| d.is_ascii_digit()) => {
                let mut digits = String::new();
                while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                    digits.push(d);
                    chars.next();
                }
                out.push_str(&literal(digits.parse::<usize>().unwrap() - 1));
            }
            _ => out.push(c),
        }
    }
    out
}

struct PgAsWorker {
    client: Mutex<postgres::Client>,
    seen: Mutex<Vec<String>>,
}

impl SqlExecutor for PgAsWorker {
    fn run(&self, request: &SqlRequest<'_>) -> Result<SqlRows, String> {
        assert_eq!(request.target, "hyperdrive://HYPERDRIVE");
        self.seen.lock().unwrap().push(request.sql.to_string());
        let sql = inline(request.sql, request.params);
        let messages = self
            .client
            .lock()
            .unwrap()
            .simple_query(&sql)
            .map_err(|e| format!("{e:?}"))?;
        let mut out = SqlRows::default();
        for message in messages {
            match message {
                postgres::SimpleQueryMessage::RowDescription(columns) => {
                    // A new statement's result: keep only the last, as pg does.
                    out.columns = columns.iter().map(|c| c.name().to_string()).collect();
                    out.rows.clear();
                }
                postgres::SimpleQueryMessage::Row(row) => out.rows.push(
                    (0..row.len())
                        .map(|i| row.get(i).map_or(serde_json::Value::Null, |t| t.into()))
                        .collect(),
                ),
                postgres::SimpleQueryMessage::CommandComplete(n) => out.changes = n,
                _ => {}
            }
        }
        Ok(out)
    }
}

fn registry(adapter: Adapter) -> ConnectionRegistry {
    let mut connections = HashMap::new();
    connections.insert(
        "primary".to_string(),
        ConnectionSpec {
            name: "primary".to_string(),
            adapter,
            url: Some("hyperdrive://HYPERDRIVE".to_string()),
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

/// The same model code on both dialects: documents, merges, aggregates,
/// transactions, raw SQL and a column-aware table.
const SCRIPT: &str = r##"
class HdNote < Model
end

class HdPerson < Model
  table "hd_people"
end

def check(label, got, want)
  throw "#{label}: got #{got}, want #{want}" unless got == want
end

a = HdNote.create({"title": "apple", "views": 3, "tags": ["red"], "meta": {"a": 1, "b": 2}})
b = HdNote.create({"title": "banana", "views": 10})
HdNote.create({"title": "cherry", "views": 7})
check("key", a._key.length > 0, true)
check("count", HdNote.count, 3)

found = HdNote.find(a._key)
check("find", found.title, "apple")
check("nested json", found.tags[0], "red")
check("where", HdNote.where({"views": {"gte": 7}}).order("title", "asc").all.map { |n| n.title }.join(","), "banana,cherry")
check("title order", HdNote.order("title", "desc").first.title, "cherry")
# A numeric field sorts as a number: 10 before 7, not "7" before "10".
check("numeric order", HdNote.order("views", "desc").first.title, "banana")
check("numeric order asc", HdNote.order("views", "asc").all.map { |n| n.views }.join(","), "3,7,10")

b.views = 11
b.save
check("save", HdNote.find(b._key).views, 11)
HdNote.update(a._key, {"views": 4})
check("merge update", HdNote.find(a._key).views, 4)
check("merge kept the rest", HdNote.find(a._key).tags[0], "red")
HdNote.update(a._key, {"meta": {"b": 3}})
check("deep merge", HdNote.find(a._key).meta.a, 1)
check("deep merge value", HdNote.find(a._key).meta.b, 3)

total = HdNote.aggregate({"total": ["sum", "views"]}).first
check("sum", total["total"], 22)
check("max", HdNote.aggregate({"top": ["max", "views"]}).first["top"], 11)

HdNote.where({"title": "cherry"}).update_all({"views": 0})
check("update_all", HdNote.find_by("title", "cherry").views, 0)

a.delete
check("delete", HdNote.count, 2)
miss = HdNote.find("nope") rescue "raised"
check("find miss raises", miss, "raised")

created = HdNote.create_many((0..20).map { |i| {"title": "bulk #{i}", "views": i} })
check("create_many", created["created"], 20)
check("count after bulk", HdNote.count, 22)

HdNote.transaction(fn() { HdNote.create({"title": "kept"}) })
check("commit", HdNote.where({"title": "kept"}).count, 1)
gone = HdNote.transaction(fn() {
  HdNote.create({"title": "rolled back"})
  throw "boom"
}) rescue "rolled"
check("rollback", gone, "rolled")
check("rolled back", HdNote.where({"title": "rolled back"}).count, 0)

HdNote.delete_all()
check("delete_all", HdNote.count, 0)

p = HdPerson.create({"name": "Ada", "age": 36, "active": true})
check("column insert", p.name, "Ada")
check("column key", p.id > 0, true)
again = HdPerson.find(p.id)
check("column find", again.age, 36)
check("column bool", again.active, true)
again.age = 37
again.save
check("column update", HdPerson.find(p.id).age, 37)
HdPerson.create({"name": "Grace", "age": 45, "active": false})
check("column where", HdPerson.where({"active": true}).count, 1)
check("column order", HdPerson.order("age", "desc").first.name, "Grace")
totals = HdPerson.aggregate({"total": ["sum", "age"], "n": ["count"]}).first
check("column aggregate", totals["total"], 82)
check("column aggregate count", totals["n"], 2)
p.delete
check("column delete", HdPerson.count, 1)
"##;

#[test]
fn models_work_on_postgres_through_the_worker_driver() {
    let Some(url) = server_url("PG_DATABASE_URL") else {
        return;
    };
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut client = postgres::Client::connect(&url, postgres::NoTls).expect("connect");
    client
        .batch_execute(
            "DROP TABLE IF EXISTS hd_notes; DROP TABLE IF EXISTS hd_people; \
             CREATE TABLE hd_people (id serial PRIMARY KEY, name text NOT NULL, \
             age integer, active boolean)",
        )
        .unwrap();
    let pg = Arc::new(PgAsWorker {
        client: Mutex::new(client),
        seen: Mutex::new(Vec::new()),
    });
    set_executor(pg.clone());
    set_registry_for_tests(registry(Adapter::Postgres));
    let result = run(SCRIPT);
    clear_registry_override();
    result.expect("the Soli script failed against Postgres");

    let seen = pg.seen.lock().unwrap();
    assert!(
        seen.iter().any(|sql| sql.contains("RETURNING doc")),
        "{seen:?}"
    );
    assert!(seen.iter().any(|sql| sql.starts_with("BEGIN")), "{seen:?}");
    assert!(seen.iter().any(|sql| sql == "ROLLBACK"), "{seen:?}");
}

/// A Hyperdrive url names a binding: anything else in it is refused by name.
#[test]
fn a_hyperdrive_url_must_name_a_binding() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    set_executor(Arc::new(PgRefuses));
    let mut reg = registry(Adapter::Postgres);
    reg.connections.get_mut("primary").unwrap().url = Some("hyperdrive://my-db!".to_string());
    set_registry_for_tests(reg);
    let err = solilang::db::sql::ensure_connected().unwrap_err();
    clear_registry_override();
    assert!(err.contains("names the Worker binding"), "{err}");
}

struct PgRefuses;

impl SqlExecutor for PgRefuses {
    fn run(&self, _request: &SqlRequest<'_>) -> Result<SqlRows, String> {
        Err("unreachable".to_string())
    }
}
