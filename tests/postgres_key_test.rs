//! A key Postgres cannot read — `/orders/not-a-uuid` on a `uuid` key — names
//! no row: a miss for `find` and for the static delete (which the generated
//! `delete` action calls with no `find` before it), not a 500 any visitor
//! can cause. A value Postgres cannot store is not a miss: the row is there,
//! and the write raises. Needs `PG_DATABASE_URL` (or a postgres
//! `DATABASE_URL`); skipped without a server, failed instead under
//! `SOLI_REQUIRE_DB=1`.
#![cfg(feature = "postgres")]

fn skip(reason: &str) {
    if std::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1") {
        panic!("SOLI_REQUIRE_DB=1 but this test would have been skipped: {reason}");
    }
    eprintln!("skip: {reason}");
}

const SCRIPT: &str = r#"
def check(label, action)
  try
    print(label, action())
  catch e
    print(label, "raised:", e)
  end
end

class KeyProbe < Model
  table "soli_key_probes"
end
row = KeyProbe.create({"id": "3f2b8c1e-4d5a-4e6f-8a9b-0c1d2e3f4a5b", "title": "kept"})
check("find", fn() { KeyProbe.find("not-a-uuid") })
check("delete", fn() { type(KeyProbe.delete("not-a-uuid")) })
check("value", fn() { KeyProbe.update("3f2b8c1e-4d5a-4e6f-8a9b-0c1d2e3f4a5b", {"due_on": "tomorrow"}) })
check("kept", fn() { KeyProbe.find("3f2b8c1e-4d5a-4e6f-8a9b-0c1d2e3f4a5b").title })
"#;

#[test]
fn a_malformed_uuid_key_is_a_miss_and_a_malformed_date_value_raises() {
    let url = std::env::var("PG_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .filter(|u| u.starts_with("postgres"))
        .unwrap_or_else(|| "postgres://soli@localhost:5432/soli_test".into());
    let mut admin = match postgres::Client::connect(&url, postgres::NoTls) {
        Ok(client) => client,
        Err(e) => {
            skip(&format!("postgres not reachable at {url}: {e}"));
            return;
        }
    };
    admin
        .batch_execute(
            "DROP TABLE IF EXISTS soli_key_probes; \
             CREATE TABLE soli_key_probes (id uuid PRIMARY KEY, title text NOT NULL, due_on date);",
        )
        .expect("create the table");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("script.sl"), SCRIPT).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("script.sl")
        .current_dir(dir.path())
        .env("SOLI_DB_ADAPTER", "postgres")
        .env("DATABASE_URL", &url)
        .output()
        .expect("run soli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let _ = admin.batch_execute("DROP TABLE IF EXISTS soli_key_probes;");

    let line = |label: &str| {
        stdout
            .lines()
            .find(|line| line.starts_with(label))
            .unwrap_or_else(|| panic!("no line for {label}: {stdout}{stderr}"))
            .to_string()
    };
    // A miss, as a missing row is: RecordNotFound from `find`…
    assert!(
        line("find").contains("raised:") && line("find").contains("not found"),
        "{}",
        line("find")
    );
    // …and the in-band "not there" from the static delete.
    assert_eq!(line("delete"), "delete string");
    // A value the column refuses: raised, and not as a missing row.
    assert!(
        line("value").contains("raised:") && !line("value").contains("not found"),
        "{}",
        line("value")
    );
    assert_eq!(line("kept"), "kept kept");
}
