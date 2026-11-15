//! Four ORM gaps met while porting a Rails app, on SQLite and on both engines:
//!
//! * `create`, `create_many` and `find_or_create_by` write a field's literal
//!   default, and `create` / `new` / `update` / `save` hand back `value_field`s
//!   as value objects — what a `find` of the same record gives (#909);
//! * `.order` adds a criterion instead of replacing the last one, takes a hash
//!   of them, and `.reorder` starts over — on a query and on an array (#908);
//! * a relation whose singular loses an `e` (`order_remises` → `OrderRemis`)
//!   reaches the class that exists, `OrderRemise`, and one to a class ending
//!   in `s` (`buses` → `Bus`) reads the collection that class writes (#907).

use std::path::Path;

fn sqlite_script(dir: &Path, engine: &str, script: &str) -> Vec<String> {
    std::fs::write(dir.join("script.sl"), script).unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    // The default engine is the VM; a strict `--vm` refuses a script that
    // declares models.
    if engine == "--tree" {
        command.arg(engine);
    }
    let output = command
        .arg("script.sl")
        .current_dir(dir)
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", dir.display()),
        )
        .output()
        .expect("run soli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "{engine}: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout.lines().map(str::to_string).collect()
}

fn on_both_engines(script: &str, expected: &[&str]) {
    for engine in ["default", "--tree"] {
        let dir = tempfile::tempdir().unwrap();
        let lines = sqlite_script(dir.path(), engine, script);
        assert_eq!(lines, expected, "engine {engine}");
    }
}

#[test]
fn create_writes_literal_defaults_and_builds_value_objects() {
    on_both_engines(
        r##"
class Label < ValueObject
  fr: String
  en: String

  def text
    @fr
  end
end

class Order < Model
  status: String = "draft"
  qty: Int = 1
  note: String
  value_field :label, Label
end

order = Order.create({"label": {"fr": "bonjour", "en": "hello"}})
print("created", order.status, order.qty, order.note.nil?, type(order.label), order.label.text)
stored = Order.find(order._key)
print("stored", stored.status, stored.qty, stored.label.text)

paid = Order.create({"status": "paid"})
print("explicit", paid.status, Order.find(paid._key).status, paid.qty)

order.update({"label": {"fr": "salut", "en": "hi"}})
print("update", type(order.label), order.label.text)
order.label = {"fr": "coucou", "en": "hey"}
order.save
print("save", type(order.label), order.label.text, Order.find(order._key).label.text)

built = Order.new({"label": {"fr": "neuf", "en": "new"}})
print("new", type(built.label), built.label.text, built.status)

Order.create_many([{"note": "bulk"}])
bulk = Order.find_by("note", "bulk")
print("create_many", bulk.status, bulk.qty)
found = Order.find_or_create_by("note", "foc")
print("find_or_create_by", found.status, Order.find(found._key).qty)
kept = Order.find_or_create_by("note", "foc2", {"status": "held"})
print("find_or_create_by given", kept.status, kept.qty)
"##,
        &[
            "created draft 1 true Label bonjour",
            "stored draft 1 bonjour",
            "explicit paid paid 1",
            "update Label salut",
            "save Label coucou coucou",
            "new Label neuf draft",
            "create_many draft 1",
            "find_or_create_by draft 1",
            "find_or_create_by given held 1",
        ],
    );
}

#[test]
fn order_adds_criteria_takes_a_hash_and_reorder_starts_over() {
    on_both_engines(
        r##"
class Line < Model
end

Line.create({"n": 2, "made": 2})
Line.create({"n": 2, "made": 1})
Line.create({"n": 1, "made": 5})
Line.create({"n": 3, "made": 0})

def pairs(rows)
  rows.map { |line| "#{line.n}/#{line.made}" }.join(" ")
end

print("chained", pairs(Line.order("n").order("made").all))
print("chained desc", pairs(Line.order("n", "desc").order("made", "desc").all))
print("hash", pairs(Line.order({"n": "desc", "made": "asc"}).all))
print("after where", pairs(Line.where({"n": 2}).order("n").order("made", "desc").all))
print("reorder", pairs(Line.order("n").reorder("made").all))
print("reorder hash", pairs(Line.order("n").reorder({"made": "desc"}).all))
print("to_query", Line.order("n").order("made", "desc").to_query)
print("reorder none", Line.order("n").reorder.to_query)
rows = Line.all
print("array hash", pairs(rows.order({"n": "asc", "made": "desc"})))
print("array one", pairs(rows.order("made", "desc")))
print("bad direction", (Line.order({"n": "sideways"}).all rescue "refused"))
print("bad field", (Line.order({"n; DROP": "asc"}).all rescue "refused"))
"##,
        &[
            "chained 1/5 2/1 2/2 3/0",
            "chained desc 3/0 2/2 2/1 1/5",
            "hash 3/0 2/1 2/2 1/5",
            "after where 2/2 2/1",
            "reorder 3/0 2/1 2/2 1/5",
            "reorder hash 1/5 2/2 2/1 3/0",
            "to_query FOR doc IN lines SORT doc.n ASC, doc.made DESC RETURN doc",
            "reorder none FOR doc IN lines RETURN doc",
            "array hash 1/5 2/2 2/1 3/0",
            "array one 1/5 2/2 2/1 3/0",
            "bad direction refused",
            "bad field refused",
        ],
    );
}

#[test]
fn a_relation_reaches_the_class_whose_singular_ends_in_e() {
    on_both_engines(
        r##"
class Order < Model
  has_many("order_remises", {"dependent": "delete"})
  has_many("caches", {"dependent": "delete"})
  has_many("buses", {"dependent": "delete"})
end
class OrderRemise < Model
  belongs_to("order")
end
class Cache < Model
  belongs_to("order")
end
class Bus < Model
  belongs_to("order")
end

order = Order.create({"name": "one"})
other = Order.create({"name": "two"})
OrderRemise.create({"order_id": order._key, "rate": 5})
OrderRemise.create({"order_id": other._key, "rate": 7})
Cache.create({"order_id": order._key})
Bus.create({"order_id": order._key})

print("read", order.order_remises.length, type(order.order_remises.first))
print("s class", order.buses.length, Bus.all.first.order.name)
print("included", Order.includes("order_remises").where({"name": "one"}).first.order_remises.length)
order.delete
print("cascade", OrderRemise.count, Cache.count, Bus.count)
"##,
        &[
            "read 1 OrderRemise",
            "s class 1 one",
            "included 1",
            "cascade 1 0 0",
        ],
    );
}
