//! What a served action does before an error is not repeated on every failing
//! request (Bonfire #643).
//!
//! In production an action that fails on the VM is re-run on the
//! tree-walker, which answers the request — unless it wrote first: a re-run
//! would write again. Two holes let the writes repeat on *every* failing
//! request for the worker's life, once an action failing the same way on both
//! engines stopped being demoted:
//!
//! * the tripwire (`crud::had_durable_commit`) saw model saves and bulk
//!   inserts, not an `increment`, a job enqueue, a raw write query or a mail;
//! * nothing remembered that the failure was the app's own, so the next
//!   failing request was re-run again, side effects included.
//!
//! These count the side effects across three failing requests to a real
//! `soli serve` on SQLite.

use std::path::Path;

fn write(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

fn app(root: &Path) {
    write(
        root,
        "config/routes.sl",
        r#"get("/up", "effects#up")
get("/seed", "effects#seed")
get("/bump", "effects#bump")
get("/enqueue", "effects#enqueue")
get("/append", "effects#append")
get("/counts", "effects#counts")
"#,
    );
    write(
        root,
        "app/models/counter.sl",
        "class Counter < Model\nend\n",
    );
    write(
        root,
        "app/jobs/later_job.sl",
        r#"class LaterJob
  static def perform(args)
    nil
  end
end
"#,
    );
    write(
        root,
        "app/controllers/effects_controller.sl",
        r#"class EffectsController < Controller
  # GET /up
  def up(req)
    "ok"
  end

  # GET /seed
  def seed(req)
    Counter.create({"hits": 0}, {"key": "c"})
    File.write("effects.log", "")
    "seeded"
  end

  # GET /bump — a write, then an error both engines raise
  def bump(req)
    Counter.find("c").increment("hits")
    throw "bad input"
  end

  # GET /enqueue — a job enqueued, then an error both engines raise
  def enqueue(req)
    LaterJob.perform_in("60 minutes", {"n": 1})
    throw "bad input"
  end

  # GET /append — an effect no tripwire sees, then an error both engines raise
  def append(req)
    File.append("effects.log", "x")
    throw "bad input"
  end

  # GET /counts
  def counts(req)
    render_json({
      "hits": Counter.find("c").hits,
      "jobs": Job.list().length,
      "appends": File.read("effects.log").length
    })
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
        .unwrap_or(28743);
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .current_dir(root)
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        // A refusal or an engine divergence stops the server: the counts
        // below are about the app's own error, on the VM.
        .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = Server(child, port);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/up"))
            .call()
            .is_ok()
        {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("soli serve did not answer on {port}");
}

fn get(server: &Server, path: &str) -> (u16, String) {
    let response = match ureq::get(&format!("http://127.0.0.1:{}{path}", server.1)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("GET {path}: {e} — did the server stop on an engine divergence?"),
    };
    (
        response.status(),
        response.into_string().unwrap_or_default(),
    )
}

#[test]
fn a_failing_action_does_not_repeat_its_side_effects_on_every_request() {
    let dir = tempfile::tempdir().unwrap();
    app(dir.path());
    let server = serve(dir.path());

    let (status, body) = get(&server, "/seed");
    assert_eq!(status, 200, "{body}");
    for path in ["/bump", "/enqueue", "/append"] {
        for _ in 0..3 {
            let (status, body) = get(&server, path);
            assert_eq!(status, 500, "{path}: {body}");
        }
    }

    let (status, body) = get(&server, "/counts");
    assert_eq!(status, 200, "{body}");
    let counts: serde_json::Value = serde_json::from_str(&body).unwrap();
    // A write before the error: the first request answers with the VM's
    // error and is not re-run (the action leaves the VM); the next two run
    // on the tree-walker alone. It was 6: each request wrote on both engines.
    assert_eq!(counts["hits"], 3, "{body}");
    // An enqueue is a write too: one job per request, not two.
    assert_eq!(counts["jobs"], 3, "{body}");
    // An effect the tripwire cannot see: the first failure is re-run, which
    // settles the error as the app's own; the next two are answered from the
    // VM without a re-run. It was 6.
    assert_eq!(counts["appends"], 4, "{body}");
}
