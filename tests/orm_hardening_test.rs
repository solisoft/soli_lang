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
