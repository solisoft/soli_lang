//! What a hash of attributes, a list of field names or a bind map may not do
//! to the ORM, on a SQL adapter (SQLite): choose another row's key, re-type a
//! record, or put text into a query.

use std::path::Path;

fn sqlite_script(dir: &Path, script: &str) -> Vec<String> {
    std::fs::write(dir.join("script.sl"), script).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
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
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout.lines().map(str::to_string).collect()
}

const CHECK: &str = r#"
def check(label, action)
  try
    print(label, action())
  catch e
    print(label, "raised:", e)
  end
end
"#;

/// An insert is an upsert on the SQL document adapters, and the key used to
/// be taken from an `id` field of the document: `Note.create(params)` with
/// `{"id": "<someone's key>"}` replaced that row, owner and all.
#[test]
fn attributes_never_choose_another_rows_key() {
    let dir = tempfile::tempdir().unwrap();
    let script = r#"
class Note < Model
end

victim = Note.create({"title": "mine", "owner": "ada"})
key = victim._key

Note.create({"title": "x", "owner": "eve", "id": key})
print("create", Note.count, Note.find(key).owner)

Note.create_many([
  {"title": "y", "owner": "eve", "id": key},
  {"title": "z", "owner": "eve", "_key": key}
])
print("create_many", Note.count, Note.find(key).owner)

Note.find_or_create_by("title", "new", {"_key": key, "id": key, "owner": "eve"})
print("find_or_create_by", Note.count, Note.find(key).owner)

Note.where({"owner": "eve"}).update_all({"_key": key, "reviewed": true})
print("update_all", Note.count, Note.find(key).owner, Note.where({"reviewed": true}).count)

chosen = Note.create({"title": "picked"}, {"key": "picked-key"})
print("chosen", chosen._key)
"#;
    let lines = sqlite_script(dir.path(), script);
    assert_eq!(
        lines,
        [
            "create 2 ada",
            "create_many 4 ada",
            "find_or_create_by 5 ada",
            "update_all 5 ada 4",
            "chosen picked-key",
        ],
        "{lines:#?}"
    );
}

/// The `fields` of an `includes` are written into the query: each is held to
/// the rule `select` and `pluck` apply, and a value that is not a name is an
/// error rather than silently dropped.
#[test]
fn include_fields_and_filters_are_validated() {
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        "{CHECK}{}",
        r#"
class Author < Model
  has_many("books")
end
class Book < Model
  belongs_to("author")
end

ada = Author.create({"name": "ada"})
Book.create({"title": "shown", "published": true, "author_id": ada._key})
Book.create({"title": "hidden", "published": false, "author_id": ada._key})

check("injection", fn() {
  Author.includes("books", {"fields": ["x: (FOR u IN users RETURN u), y: \""]}).all.length
})
check("not a name", fn() { Author.includes("books", {"fields": [1]}).all.length })
check("not a list", fn() { Author.includes("books", {"fields": "title"}).all.length })
check("names", fn() { Author.includes("books", {"fields": ["title"]}).all.length })
check("reserved bind", fn() {
  Author.where("name == @name", {"name": "ada", "__soli_through_fk": "x"}).all.length
})
"#
    );
    let lines = sqlite_script(dir.path(), &script);
    assert!(lines[0].starts_with("injection raised:"), "{lines:#?}");
    assert!(lines[1].starts_with("not a name raised:"), "{lines:#?}");
    assert!(lines[2].starts_with("not a list raised:"), "{lines:#?}");
    assert_eq!(lines[3], "names 1", "{lines:#?}");
    // (On SQLite a string filter is refused before its binds are read; the
    // reserved-name check is exercised by the SDBQL specs.)
    assert!(lines[4].starts_with("reserved bind raised:"), "{lines:#?}");
}

/// A read or a bulk write the database failed is an error, not an answer.
/// `.first` used to say nil ("no such row"), `.exists.first` a truthy
/// string, `update_all` / `delete_all` a string nobody looked at: a database
/// error read as "nothing forbids this" or "it is done".
#[test]
fn a_failed_read_or_bulk_write_raises() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.sqlite3"),
        "this file is not a database, and is long enough to be read as a header ........................................",
    )
    .unwrap();
    let script = format!(
        r#"{CHECK}
class Note < Model
end
check("first", fn() {{ Note.where({{"owner": "a"}}).first }})
check("exists", fn() {{ Note.where({{"owner": "a"}}).exists.first }})
check("update_all", fn() {{ Note.where({{"owner": "a"}}).update_all({{"seen": true}}) }})
check("delete_all", fn() {{ Note.where({{"owner": "a"}}).delete_all }})
"#
    );
    let lines = sqlite_script(dir.path(), &script);
    for label in ["first", "exists", "update_all", "delete_all"] {
        let line = lines
            .iter()
            .find(|line| line.starts_with(label))
            .unwrap_or_else(|| panic!("no line for {label}: {lines:?}"));
        assert!(
            line.contains("raised:") && line.contains("not a database"),
            "{label} did not raise: {line}"
        );
    }
}

/// On SoliDB, `find_by` answered nil when the read failed — and the
/// second-factor gate asks it whether an account has two-factor on. With no
/// database answering, it raises.
#[test]
fn find_by_raises_when_solidb_does_not_answer() {
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        r#"{CHECK}
class Note < Model
end
check("find_by", fn() {{ Note.find_by("owner", "a") }})
check("first_by", fn() {{ Note.first_by("owner", "a") }})
"#
    );
    std::fs::write(dir.path().join("script.sl"), script).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("script.sl")
        .current_dir(dir.path())
        .env_remove("SOLI_DB_ADAPTER")
        .env_remove("DATABASE_URL")
        // A port nothing listens on.
        .env("SOLIDB_HOST", "http://127.0.0.1:9")
        .env("SOLIDB_DATABASE", "none")
        .output()
        .expect("run soli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    for label in ["find_by", "first_by"] {
        let line = stdout
            .lines()
            .find(|line| line.starts_with(label))
            .unwrap_or_else(|| panic!("no line for {label}: {stdout}"));
        assert!(line.contains("raised:"), "{label} did not raise: {line}");
    }
}

/// A database that stops answering inside `grouped`: no read of the block is
/// taken for nil. One read where nothing can raise (a condition) is raised
/// by the block's end; one read rescued leaves the other still failing, not
/// nil.
#[test]
fn a_database_failure_inside_grouped_is_never_read_as_nil() {
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        r#"{CHECK}
class Note < Model
end
check("condition", fn() {{
  grouped(fn() {{
    row = Note.where({{"owner": "a"}}).first
    label = row ? "found" : "nothing forbids it"
    print("condition read as", label)
  }})
  "the block finished"
}})
check("sibling", fn() {{
  let seen = "unset"
  grouped(fn() {{
    one = Note.where({{"owner": "a"}}).first
    two = Note.where({{"owner": "b"}}).count
    title = (one.title rescue "rescued")
    seen = two.nil? ? "sibling read as nil" : "sibling has a value"
  }})
  seen
}})
"#
    );
    std::fs::write(dir.path().join("script.sl"), script).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("script.sl")
        .current_dir(dir.path())
        .env_remove("SOLI_DB_ADAPTER")
        .env_remove("DATABASE_URL")
        .env("APP_ENV", "production")
        // A port nothing listens on.
        .env("SOLIDB_HOST", "http://127.0.0.1:9")
        .env("SOLIDB_DATABASE", "none")
        .output()
        .expect("run soli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    for label in ["condition", "sibling"] {
        let line = stdout
            .lines()
            .find(|line| line.starts_with(label) && !line.starts_with("condition read as"))
            .unwrap_or_else(|| panic!("no line for {label}: {stdout}"));
        assert!(line.contains("raised:"), "{label} did not raise: {stdout}");
    }
}
