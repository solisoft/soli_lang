//! Read replicas through `soli serve`: a GET reads the replica, a write goes to
//! the primary and leaves the `soli_db_primary` cookie, the client's next GET
//! reads the primary, and a GET that writes reads the primary afterwards.
//!
//! A second database on the same Postgres server stands in for the replica:
//! the same key holds a different title in each, so every answer says which
//! database gave it. Needs `PG_DATABASE_URL` (or a postgres `DATABASE_URL`);
//! skipped without a server, failed instead under `SOLI_REQUIRE_DB=1`.
#![cfg(feature = "postgres")]

use std::collections::HashMap;
use std::path::Path;

use solilang::db::registry::registry_test_lock;
use solilang::db::{
    clear_registry_override, set_registry_for_tests, Adapter, ConnectionRegistry, ConnectionSpec,
};

const TABLE: &str = "notes";

fn spec(name: &str, url: &str) -> ConnectionSpec {
    ConnectionSpec {
        name: name.to_string(),
        adapter: Adapter::Postgres,
        url: Some(url.to_string()),
        solidb_host: None,
        solidb_database: None,
        solidb_username: None,
        solidb_password: None,
        solidb_api_key: None,
        pool_size: Some(2),
        replicas: Vec::new(),
        replica_delay: None,
    }
}

/// The server's URL and the stand-in replica's, the replica database created.
fn databases() -> Option<(String, String)> {
    let url = std::env::var("PG_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .filter(|u| u.starts_with("postgres"))
        .unwrap_or_else(|| "postgres://soli@localhost:5432/soli_test".into());
    let (head, tail) = url.rsplit_once('/')?;
    let (database, query) = match tail.split_once('?') {
        Some((db, q)) => (db.to_string(), format!("?{q}")),
        None => (tail.to_string(), String::new()),
    };
    let maintenance = format!("{head}/postgres{query}");
    let mut admin = match postgres::Client::connect(&maintenance, postgres::NoTls) {
        Ok(client) => client,
        Err(e) => {
            skip(&format!("postgres not reachable at {url}: {e}"));
            return None;
        }
    };
    let replica_db = format!("{database}_replica");
    let _ = admin.batch_execute(&format!("CREATE DATABASE \"{replica_db}\""));
    Some((url.clone(), format!("{head}/{replica_db}{query}")))
}

fn skip(reason: &str) {
    if std::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1") {
        panic!("SOLI_REQUIRE_DB=1 but this test would have been skipped: {reason}");
    }
    eprintln!("skip: {reason}");
}

/// `k1` says "primary" on the primary and "replica" on the replica.
fn seed(primary: &str, replica: &str) {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let connections: HashMap<_, _> = [
        ("primary".to_string(), spec("primary", primary)),
        ("standin".to_string(), spec("standin", replica)),
    ]
    .into_iter()
    .collect();
    set_registry_for_tests(ConnectionRegistry {
        default: "primary".to_string(),
        connections,
        from_file: false,
    });
    for (connection, title) in [("primary", "primary"), ("standin", "replica")] {
        solilang::db::with_connection(connection, || {
            let _ = solilang::db::sql::delete(TABLE, "k1");
            solilang::db::sql::insert(TABLE, Some("k1"), serde_json::json!({ "title": title }))
                .expect("seed");
        });
    }
    clear_registry_override();
}

fn write(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

fn served_app(root: &Path, primary: &str, replica: &str) {
    write(
        root,
        "config/database.toml",
        &format!(
            r#"default = "primary"

[connections.primary]
adapter = "postgres"
url = "{primary}"
replicas = ["{replica}"]
replica_delay = 30
"#
        ),
    );
    write(
        root,
        "config/routes.sl",
        r#"get("/note", "notes#show")
post("/note", "notes#update")
get("/bump", "notes#bump")
get("/report", "notes#report")
"#,
    );
    write(root, "app/models/note.sl", "class Note < Model\nend\n");
    write(
        root,
        "app/controllers/notes_controller.sl",
        r#"class NotesController < Controller
  # GET /note
  def show(req)
    render_json({"title": Note.find("k1").title, "role": current_role()})
  end

  # POST /note
  def update(req)
    Note.find("k1").update({"title": params["title"]})
    render_json({"role": current_role()})
  end

  # GET /bump — a GET that writes, then reads again
  def bump(req)
    before = Note.find("k1").title
    Note.find("k1").update({"views": 1})
    after = Note.find("k1").title
    render_json({"before": before, "after": after})
  end

  # GET /report — a forced primary read inside a reading request
  def report(req)
    fresh = with_role("writing") do
      Note.find("k1").title
    end
    render_json({"fresh": fresh, "role": current_role()})
  end
end
"#,
    );
}

struct Server(std::process::Child, u16);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(root: &Path) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(28741);
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .current_dir(root)
        .env(
            "SOLI_SESSION_SECRET",
            "replica-test-secret-0123456789abcdef",
        )
        .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = Server(child, port);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/note"))
            .call()
            .is_ok()
        {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("soli serve did not answer on {port}");
}

/// The JSON answer and the `soli_db_primary` cookie, when one was set.
fn call(
    server: &Server,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<&str>,
) -> (serde_json::Value, Option<String>) {
    let mut request = ureq::request(method, &format!("http://127.0.0.1:{}{path}", server.1));
    if let Some(cookie) = cookie {
        request = request.set("Cookie", cookie);
    }
    let response = match body {
        Some(body) => request
            .set("Content-Type", "application/json")
            .send_string(body),
        None => request.call(),
    };
    let response = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(status, r)) => {
            panic!(
                "{method} {path}: {status} {}",
                r.into_string().unwrap_or_default()
            )
        }
        Err(e) => panic!("{method} {path}: {e}"),
    };
    let sticky = response
        .all("set-cookie")
        .into_iter()
        .find(|c| c.starts_with("soli_db_primary="))
        .map(|c| c.split(';').next().unwrap_or_default().to_string());
    let text = response.into_string().unwrap_or_default();
    (
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
        sticky,
    )
}

#[test]
fn a_served_app_reads_the_replica_and_its_own_writes() {
    let Some((primary, replica)) = databases() else {
        return;
    };
    seed(&primary, &replica);
    let dir = tempfile::tempdir().expect("tempdir");
    served_app(dir.path(), &primary, &replica);
    let server = serve(dir.path());

    // A GET reads the replica, and sets no cookie.
    let (body, sticky) = call(&server, "GET", "/note", None, None);
    assert_eq!(body["title"], "replica", "{body}");
    assert_eq!(body["role"], "reading");
    assert!(sticky.is_none());

    // A GET can still ask for the primary.
    let (body, _) = call(&server, "GET", "/report", None, None);
    assert_eq!(body["fresh"], "primary", "{body}");
    assert_eq!(body["role"], "reading", "with_role puts the role back");

    // A write goes to the primary and leaves the cookie.
    let (body, sticky) = call(
        &server,
        "POST",
        "/note",
        None,
        Some(r#"{"title": "fresh"}"#),
    );
    assert_eq!(body["role"], "writing", "{body}");
    let sticky = sticky.expect("a request that wrote sets soli_db_primary");

    // The same client's next GET reads the primary and sees its write.
    let (body, _) = call(&server, "GET", "/note", Some(&sticky), None);
    assert_eq!(body["title"], "fresh", "{body}");
    assert_eq!(body["role"], "writing");

    // Another client still reads the replica.
    let (body, _) = call(&server, "GET", "/note", None, None);
    assert_eq!(body["title"], "replica", "{body}");

    // A GET that writes reads the primary after its write, and sets the cookie.
    let (body, sticky) = call(&server, "GET", "/bump", None, None);
    assert_eq!(body["before"], "replica", "{body}");
    assert_eq!(body["after"], "fresh", "{body}");
    assert!(sticky.is_some());

    // That write sent only what changed: the replica's older title did not
    // overwrite the primary's.
    let (body, _) = call(&server, "GET", "/report", None, None);
    assert_eq!(body["fresh"], "fresh", "{body}");
}
