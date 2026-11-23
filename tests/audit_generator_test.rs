//! `soli generate audit` — the `Audited` concern and the `Audit` model: what
//! it writes, that it is fmt/lint clean, and a served app (the fixture in
//! `tests/fixtures/_audit_app`, on SQLite, migrated) whose changes are logged
//! with their author, with secret values withheld, in a chain that notices
//! an edited entry.

use std::fs;
use std::path::Path;

use serde_json::{json, Value as Json};
use solilang::scaffold::create_audit;

const AUDIT_FILES: [&str; 2] = ["app/models/concerns/audited.sl", "app/models/audit.sl"];

fn make_app_skeleton(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
}

fn migration(root: &Path) -> String {
    let entry = fs::read_dir(root.join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().contains("create_audits"))
        .expect("no create_audits migration");
    fs::read_to_string(entry.path()).unwrap()
}

#[test]
fn audit_writes_the_concern_the_model_and_a_migration() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    create_audit(dir.path().to_str().unwrap(), false).expect("generate audit");
    for rel in AUDIT_FILES {
        assert!(dir.path().join(rel).exists(), "{rel} not created");
    }
    let model = fs::read_to_string(dir.path().join("app/models/audit.sl")).unwrap();
    assert!(model.contains("const AUDIT_CHAINED = false"), "{model}");
    let plain = migration(dir.path());
    assert!(plain.contains("idx_audit_record"), "{plain}");
    assert!(!plain.contains("idx_audit_seq"), "{plain}");

    // A second run changes nothing, whatever the flag.
    fs::write(dir.path().join("app/models/audit.sl"), "# mine\n").unwrap();
    create_audit(dir.path().to_str().unwrap(), true).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("app/models/audit.sl")).unwrap(),
        "# mine\n"
    );
    assert_eq!(
        fs::read_dir(dir.path().join("db/migrations"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn the_chain_flag_sets_the_constant_and_the_unique_index() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    create_audit(dir.path().to_str().unwrap(), true).expect("generate audit --chain");
    let model = fs::read_to_string(dir.path().join("app/models/audit.sl")).unwrap();
    assert!(model.contains("const AUDIT_CHAINED = true"), "{model}");
    let chained = migration(dir.path());
    assert!(
        chained.contains(r#"["seq"], {"unique": true}"#),
        "{chained}"
    );
}

fn soli(dir: &Path, args: &[&str], env: &[(&str, String)]) -> (bool, String) {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .args(args)
        .current_dir(dir)
        .env_remove("SOLI_ENGINE");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("run soli");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn cli_generates_audit_and_generated_code_passes_fmt_lint_and_check() {
    for args in [
        &["generate", "audit"][..],
        &["generate", "audit", "--chain", "."][..],
        &["generate", "audit", ".", "--chain"][..],
    ] {
        let dir = tempfile::tempdir().unwrap();
        make_app_skeleton(dir.path());
        let (ok, output) = soli(dir.path(), args, &[]);
        assert!(ok, "{args:?}: {output}");
        assert!(output.contains("include Audited"), "{output}");
        let chained = args.contains(&"--chain");
        assert_eq!(output.contains("Audit.verify"), chained, "{output}");

        let (formatted, output) = soli(dir.path(), &["fmt", "--check", "."], &[]);
        assert!(formatted, "soli fmt --check:\n{output}");
        let (clean, output) = soli(dir.path(), &["lint", "."], &[]);
        assert!(clean, "soli lint:\n{output}");
        let (checked, output) = soli(dir.path(), &["check", "."], &[]);
        assert!(checked, "soli check:\n{output}");
    }
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let (ok, output) = soli(dir.path(), &["generate", "audit", "--chained"], &[]);
    assert!(!ok);
    assert!(output.contains("unknown option --chained"), "{output}");
}

// ---------------------------------------------------------------------------
// Served end-to-end
// ---------------------------------------------------------------------------

struct Server(std::process::Child, u16);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn database_env(root: &Path) -> Vec<(&'static str, String)> {
    vec![
        ("SOLI_DB_ADAPTER", "sqlite".to_string()),
        (
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        ),
        (
            "SOLI_ENCRYPTION_KEY",
            "audit-test-key-0123456789abcdef-0123456789".to_string(),
        ),
    ]
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Everything SQLite has on disk for the app: the database and, since recent
/// writes sit there until a checkpoint, its write-ahead log.
fn database_bytes(root: &Path) -> String {
    let mut bytes = fs::read(root.join("app.sqlite3")).unwrap_or_default();
    bytes.extend(fs::read(root.join("app.sqlite3-wal")).unwrap_or_default());
    assert!(!bytes.is_empty(), "no database on disk");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The fixture app with the audit scaffold generated into it and migrated.
fn audited_app(chained: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_audit_app");
    copy_dir(&fixture, dir.path());
    create_audit(dir.path().to_str().unwrap(), chained).expect("generate audit");
    let (ok, output) = soli(
        dir.path(),
        &["db:migrate", "up", "."],
        &database_env(dir.path()),
    );
    assert!(ok, "db:migrate up:\n{output}");
    dir
}

fn serve(root: &Path) -> Server {
    serve_with(root, &[])
}

fn serve_with(root: &Path, extra: &[(&str, &str)]) -> Server {
    let port = port_not_given_before();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .arg("serve")
        .arg(root)
        .args(["--port", &port.to_string(), "--workers", "1"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (key, value) in database_env(root) {
        command.env(key, value);
    }
    command.env_remove("AUDIT_CHAIN_KEY");
    for (key, value) in extra {
        command.env(key, value);
    }
    let server = Server(command.spawn().expect("spawn soli serve"), port);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/audit/verify"))
            .call()
            .is_ok()
        {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("soli serve did not answer on {port}");
}

/// A request as `member` (an email, or "" for nobody); the JSON it answers.
fn call(server: &Server, method: &str, path: &str, body: Option<Json>, member: &str) -> Json {
    let origin = format!("http://127.0.0.1:{}", server.1);
    let mut request =
        ureq::request(method, &format!("{origin}{path}")).set("X-Request-Id", "req-42");
    if method != "GET" {
        request = request.set("Origin", &origin);
    }
    if !member.is_empty() {
        request = request.set("X-Member", member);
    }
    let response = match body {
        Some(body) => request
            .set("Content-Type", "application/json")
            .send_string(&body.to_string()),
        None => request.call(),
    };
    let text = match response {
        Ok(r) => r.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(code, r)) => {
            panic!(
                "{method} {path}: {code} {}",
                r.into_string().unwrap_or_default()
            )
        }
        Err(e) => panic!("{method} {path}: {e}"),
    };
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("{method} {path}: not JSON: {text}"))
}

const ADA: &str = "ada@example.com";

#[test]
fn served_app_logs_who_changed_what_and_notices_an_edited_entry() {
    let dir = audited_app(true);
    let server = serve(dir.path());
    let ada = call(&server, "POST", "/members", Some(json!({"email": ADA})), "");

    // Create, as Ada. `iban` is declared with `encrypts`, `secret_note` is in
    // the model's audit_redacted, `views` in its audit_ignored.
    let invoice = call(
        &server,
        "POST",
        "/invoices",
        Some(json!({"number": "F-1", "total": 100, "iban": "FR76-SECRET", "secret_note": "shh"})),
        ADA,
    );
    let key = invoice["key"].as_str().unwrap().to_string();
    // Update as Ada, then touch only an ignored field, then delete as nobody.
    let updated = call(
        &server,
        "POST",
        &format!("/invoices/{key}/update"),
        Some(json!({"total": 120, "iban": "FR76-OTHER", "views": 5})),
        ADA,
    );
    assert_eq!(updated, json!({"audits": 2}));
    let ignored = call(
        &server,
        "POST",
        &format!("/invoices/{key}/update"),
        Some(json!({"views": 9})),
        ADA,
    );
    assert_eq!(
        ignored,
        json!({"audits": 2}),
        "an ignored field alone is no entry"
    );
    call(
        &server,
        "POST",
        &format!("/invoices/{key}/delete"),
        Some(json!({})),
        "",
    );

    let audits = call(&server, "GET", &format!("/invoices/{key}/audits"), None, "");
    let audits = audits.as_array().unwrap();
    assert_eq!(audits.len(), 3, "{audits:?}");
    let redacted = json!(["[redacted]", "[redacted]"]);

    let created = &audits[0];
    assert_eq!(created["action"], json!("create"));
    assert_eq!(created["record_type"], json!("Invoice"));
    assert_eq!(created["record_key"], json!(key));
    assert_eq!(
        created["changes"],
        json!({"number": [null, "F-1"], "total": [null, 100],
               "iban": redacted, "secret_note": redacted})
    );
    assert_eq!(created["actor_type"], json!("Member"));
    assert_eq!(created["actor_key"], ada["key"]);
    assert_eq!(created["actor_label"], json!(ADA));
    assert_eq!(created["request_id"], json!("req-42"));
    assert_eq!(created["ip"], json!("127.0.0.1"));
    assert_eq!(
        (created["seq"].clone(), created["prev_hash"].clone()),
        (json!(1), json!(""))
    );

    let update = &audits[1];
    assert_eq!(update["action"], json!("update"));
    assert_eq!(
        update["changes"],
        json!({"total": [100, 120], "iban": redacted})
    );
    assert_eq!(update["prev_hash"], created["entry_hash"]);

    let deleted = &audits[2];
    assert_eq!(deleted["action"], json!("delete"));
    assert_eq!(
        deleted["changes"],
        json!({"number": ["F-1", null], "total": [120, null],
               "iban": redacted, "secret_note": redacted})
    );
    assert_eq!(deleted["actor_type"], json!("system"));
    assert_eq!(deleted["actor_key"], Json::Null);
    assert_eq!(deleted["seq"], json!(3));

    // The secrets are not in the log, and the encrypted field is nowhere on
    // disk in plaintext. (`secret_note` is only withheld from the log: the
    // model stores it as it is.)
    let logged = serde_json::to_string(audits).unwrap();
    for secret in ["FR76-SECRET", "FR76-OTHER", "shh"] {
        assert!(!logged.contains(secret), "{secret} is in the log");
    }
    let stored = database_bytes(dir.path());
    assert!(stored.contains("F-1"), "the data is not where this looks");
    for secret in ["FR76-SECRET", "FR76-OTHER"] {
        assert!(!stored.contains(secret), "{secret} is on disk in plaintext");
    }

    // Reading by actor and by period.
    let by_ada = call(
        &server,
        "GET",
        &format!("/audit/actor?email={ADA}"),
        None,
        "",
    );
    assert_eq!(by_ada, json!({"count": 2, "period": 3, "before": 0}));

    // The chain is intact; editing an entry is noticed, at that entry.
    assert_eq!(
        call(&server, "GET", "/audit/verify", None, ""),
        json!({"broken": null, "count": 3})
    );
    call(
        &server,
        "POST",
        "/audit/tamper",
        Some(json!({"seq": 2})),
        "",
    );
    assert_eq!(
        call(&server, "GET", "/audit/verify", None, ""),
        json!({"broken": 2, "count": 3})
    );

    // Retention is a decision, and it removes the head of the chain.
    assert_eq!(
        call(&server, "POST", "/audit/purge", Some(json!({})), ""),
        json!({"purged": 3, "left": 0})
    );
}

#[test]
fn an_unchained_log_has_no_sequence_and_takes_every_entry() {
    let dir = audited_app(false);
    let server = serve(dir.path());
    let mut keys = Vec::new();
    for number in ["F-1", "F-2", "F-3"] {
        let invoice = call(
            &server,
            "POST",
            "/invoices",
            Some(json!({"number": number, "total": 1})),
            "",
        );
        keys.push(invoice["key"].as_str().unwrap().to_string());
    }
    let audits = call(
        &server,
        "GET",
        &format!("/invoices/{}/audits", keys[2]),
        None,
        "",
    );
    let entry = &audits.as_array().unwrap()[0];
    assert_eq!(entry["changes"]["number"], json!([null, "F-3"]));
    assert!(
        entry.get("seq").is_none() && entry.get("entry_hash").is_none(),
        "{entry}"
    );
    assert_eq!(
        call(&server, "GET", "/audit/verify", None, "")["count"],
        json!(3)
    );
}

/// Three entries in a chained log: a creation, an update, a deletion.
fn three_entries(server: &Server) {
    call(server, "POST", "/members", Some(json!({"email": ADA})), "");
    let invoice = call(
        server,
        "POST",
        "/invoices",
        Some(json!({"number": "F-1", "total": 100})),
        ADA,
    );
    let key = invoice["key"].as_str().unwrap().to_string();
    call(
        server,
        "POST",
        &format!("/invoices/{key}/update"),
        Some(json!({"total": 120})),
        ADA,
    );
    call(
        server,
        "POST",
        &format!("/invoices/{key}/delete"),
        Some(json!({})),
        "",
    );
    assert_eq!(
        call(server, "GET", "/audit/verify", None, ""),
        json!({"broken": null, "count": 3})
    );
}

/// Removing the newest entries leaves a shorter chain that is intact: only a
/// head kept elsewhere notices the missing tail.
#[test]
fn a_log_cut_short_is_noticed_against_a_head_kept_elsewhere() {
    let dir = audited_app(true);
    let server = serve(dir.path());
    three_entries(&server);
    let head = call(&server, "GET", "/audit/head", None, "");
    assert_eq!(head["seq"], json!(3), "{head}");
    let against_head = format!(
        "/audit/verify_head?seq=3&hash={}",
        head["entry_hash"].as_str().unwrap()
    );
    assert_eq!(
        call(&server, "GET", &against_head, None, ""),
        json!({"broken": null})
    );

    assert_eq!(
        call(&server, "POST", "/audit/cut", Some(json!({})), ""),
        json!({"count": 2})
    );
    assert_eq!(
        call(&server, "GET", "/audit/verify", None, ""),
        json!({"broken": null, "count": 2}),
        "a shorter chain is intact on its own"
    );
    assert_eq!(
        call(&server, "GET", &against_head, None, ""),
        json!({"broken": 3})
    );
}

/// Someone who can write to the table can edit an entry and redo every hash
/// after it. With plain hashes the chain then verifies; with AUDIT_CHAIN_KEY
/// they would need the key, and the edit is noticed.
#[test]
fn a_redone_chain_passes_without_a_key_and_fails_with_one() {
    let redo = |key: Option<&str>| {
        let dir = audited_app(true);
        let extra: Vec<(&str, &str)> = key
            .map(|k| vec![("AUDIT_CHAIN_KEY", k)])
            .unwrap_or_default();
        let server = serve_with(dir.path(), &extra);
        three_entries(&server);
        call(&server, "POST", "/audit/redo", Some(json!({"seq": 2})), "");
        call(&server, "GET", "/audit/verify", None, "")["broken"].clone()
    };
    assert_eq!(
        redo(None),
        json!(null),
        "an unkeyed chain cannot notice this"
    );
    assert_eq!(
        redo(Some("an-audit-chain-key-of-32-characters!")),
        json!(1),
        "a keyed chain was redone without its key, unnoticed"
    );
}

/// A free port no other test of this binary was given: the tests run side
/// by side, and a port released by one could be handed to another while the
/// first server was still going down.
fn port_not_given_before() -> u16 {
    static TAKEN: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|addr| addr.port())
            .expect("a free port");
        let mut taken = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
        if !taken.contains(&port) {
            taken.push(port);
            return port;
        }
    }
}
