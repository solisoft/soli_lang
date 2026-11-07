//! An update validates the record it writes, identity included (card #674).
//!
//! The instance update paths (`record.update(attrs)`, `save` on a loaded
//! record) built the record handed to validators from its non-`_` fields, so a
//! custom validator saw no `_key`: "unique except me" (`taken._key != @_key`)
//! refused every update of a record that already held the value. Every update
//! path — and the built-in `uniqueness` rule — now sees the persisted identity
//! and the merged current values.

use std::path::Path;

fn sqlite_script(dir: &Path, engine: &str, script: &str) -> Vec<String> {
    std::fs::write(dir.join("script.sl"), script).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args([engine, "script.sl"])
        .current_dir(dir)
        .env_remove("SOLI_ENGINE")
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

const DOCUMENT_SCRIPT: &str = r#"
class Site < Model
  validates("slug", {"custom": "_slug_free"})
  validates("host", {"custom": fn(value, record) {
    taken = Site.find_by("host", value)
    taken.nil? || taken._key == record["_key"] || "is taken"
  }})
  validates("code", {"uniqueness": true})
  validates("title", {"on": "update", "custom": fn(value, record) { record["id"] == record["_key"] || "no id" }})

  private

  def _slug_free(slug)
    taken = Site.find_by("slug", slug)
    return true if taken.nil? || taken._key == @_key

    "is taken"
  end
end

def fields(errors)
  (errors ?? []).map { |e| e["field"] }.sort
end

a = Site.create({"slug": "a", "host": "a.test", "code": "A", "title": "A"})
b = Site.create({"slug": "b", "host": "b.test", "code": "B", "title": "B"})
print("created", fields(a._errors), fields(b._errors))

loaded = Site.find(a._key)
print("instance update", loaded.update({"title": "A2"}), fields(loaded._errors))
loaded.title = "A3"
print("save", loaded.save, fields(loaded._errors))
print("static update", fields(Site.update(a._key, {"title": "A4"})["_errors"]))
print("stored", Site.find(a._key).title)

clash = Site.find(a._key)
print("instance clash", clash.update({"slug": "b", "host": "b.test", "code": "B"}), fields(clash._errors))
print("static clash", fields(Site.update(a._key, {"slug": "b", "host": "b.test", "code": "B"})["_errors"]))
again = Site.find(a._key)
again.slug = "b"
print("save clash", again.save, fields(again._errors))
print("create clash", fields(Site.create({"slug": "a", "host": "a.test", "code": "A", "title": "C"})._errors))
"#;

// The tree-walker only: under `--vm` no class-body DSL (`validates`, …)
// receives its class (see vm_model_scope_test.rs), while `soli serve` loads
// model files on the tree-walker — and validators run there on every engine.
#[test]
fn every_update_path_hands_validators_the_record_identity() {
    {
        let engine = "--tree";
        let dir = tempfile::tempdir().unwrap();
        let lines = sqlite_script(dir.path(), engine, DOCUMENT_SCRIPT);
        assert_eq!(
            lines,
            vec![
                "created [] []",
                "instance update true []",
                "save true []",
                "static update []",
                "stored A4",
                "instance clash false [code, host, slug]",
                "static clash [code, host, slug]",
                "save clash false [slug]",
                "create clash [code, host, slug]",
            ],
            "engine {engine}"
        );
    }
}

#[test]
fn a_column_mode_row_keeps_its_own_id() {
    let dir = tempfile::tempdir().unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("app.sqlite3")).unwrap();
    conn.execute_batch(
        "CREATE TABLE monitors (id INTEGER PRIMARY KEY AUTOINCREMENT, url TEXT NOT NULL, \
         label TEXT);",
    )
    .unwrap();
    drop(conn);
    let lines = sqlite_script(
        dir.path(),
        "--tree",
        r#"class Monitor < Model
  table "monitors"
  validates("url", {"custom": "_url_free"})

  private

  def _url_free(url)
    taken = Monitor.find_by("url", url)
    taken.nil? || taken.id == @id || "is taken"
  end
end
first = Monitor.create({"url": "https://a.test"})
Monitor.create({"url": "https://b.test"})
loaded = Monitor.find(first.id)
print(loaded.update({"label": "home"}), type(loaded.id))
loaded.url = "https://b.test"
print(loaded.save, loaded._errors.map { |e| e["field"] })
"#,
    );
    assert_eq!(lines, vec!["true int", "false [url]"]);
}
