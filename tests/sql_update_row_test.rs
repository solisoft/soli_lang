//! A record deleted while a request still holds it is not written back — on
//! Postgres and MySQL, where an update used to be an upsert. The SQLite case
//! is in `orm_hardening_test.rs`; this is the same script against the two
//! servers. Needs `PG_DATABASE_URL` / `MYSQL_URL`; skipped without a server,
//! failed instead under `SOLI_REQUIRE_DB=1`.

fn skip(reason: &str) {
    if std::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1") {
        panic!("SOLI_REQUIRE_DB=1 but this test would have been skipped: {reason}");
    }
    eprintln!("skip: {reason}");
}

const SCRIPT: &str = r##"
class RowProbe < Model
end

def answer(label, action)
  said = "raised"
  try
    said = action()
  catch error
    said = "raised"
  end
  print(label, said, RowProbe.where({"suite": "update_row"}).count)
end

RowProbe.where({"suite": "update_row"}).delete_all

# A request reads the record, its owner deletes it, and the request writes.
issued = RowProbe.create({"suite": "update_row", "owner": "ada"})
held = RowProbe.find(issued._key)
key = issued._key
issued.delete
held.uses = 1
answer("update", fn() { held.update })
held.owner = "eve"
answer("save", fn() { held.save })
answer("touch", fn() { held.touch })
answer("static", fn() { type(RowProbe.update(key, {"owner": "eve"})) })
# A patch with a nested object takes the read-merge-write path.
answer("nested", fn() { type(RowProbe.update(key, {"prefs": {"theme": "dark"}})) })
# `upsert` is the one that creates — and merges into a row that is there.
answer("upsert", fn() { RowProbe.upsert("row-probe-chosen", {"suite": "update_row", "owner": "linus"}).owner })
answer("upsert again", fn() { RowProbe.upsert("row-probe-chosen", {"uses": 2}).owner })
RowProbe.where({"_key": "row-probe-chosen"}).delete_all

# A row that is there is written as before, nested objects included.
kept = RowProbe.create({"suite": "update_row", "owner": "ada", "prefs": {"theme": "dark"}})
kept.owner = "grace"
answer("there", fn() { kept.update })
answer("merged", fn() { RowProbe.update(kept._key, {"prefs": {"lang": "fr"}}).prefs.keys.sort.join(",") })
# A patch that changes nothing is still a row that is there (MySQL counts
# the rows an UPDATE changed, not the ones it matched).
answer("no change", fn() { RowProbe.update(kept._key, {"owner": "grace"}).owner })
answer("no change, instance", fn() { RowProbe.find(kept._key).update })

RowProbe.where({"suite": "update_row"}).delete_all
"##;

const EXPECTED: [&str; 11] = [
    "update false 0",
    "save false 0",
    "touch raised 0",
    "static string 0",
    "nested string 0",
    "upsert linus 1",
    "upsert again linus 1",
    "there true 1",
    "merged lang,theme 1",
    "no change grace 1",
    "no change, instance true 1",
];

fn run_against(adapter: &str, url: &str) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("script.sl"), SCRIPT).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("script.sl")
        .current_dir(dir.path())
        .env("SOLI_DB_ADAPTER", adapter)
        .env("DATABASE_URL", url)
        .output()
        .expect("run soli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let all = format!("{stdout}{stderr}").to_lowercase();
        // A server that is not there, said by the pool — not any failure
        // that happens to mention a connection.
        if all.contains("pool (primary)")
            && (all.contains("could not connect") || all.contains("error connecting"))
        {
            skip(&format!("{adapter} not reachable at {url}: {stderr}"));
            return;
        }
        panic!("{adapter}: {stdout}{stderr}");
    }
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, EXPECTED, "{adapter}: {stdout}{stderr}");
}

#[cfg(feature = "postgres")]
#[test]
fn a_record_deleted_meanwhile_is_not_written_back_on_postgres() {
    let url = std::env::var("PG_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .filter(|u| u.starts_with("postgres"))
        .unwrap_or_else(|| "postgres://soli@localhost:5432/soli_test".into());
    run_against("postgres", &url);
}

#[cfg(feature = "mysql")]
#[test]
fn a_record_deleted_meanwhile_is_not_written_back_on_mysql() {
    let url = std::env::var("MYSQL_URL")
        .ok()
        .filter(|u| u.starts_with("mysql"))
        .unwrap_or_else(|| "mysql://root@127.0.0.1:3306/soli_test".into());
    run_against("mysql", &url);
}
