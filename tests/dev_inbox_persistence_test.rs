//! The dev inbox (`/__soli/inbox`) outlives a restart of `soli serve --dev`.
//!
//! It lived in memory only: restarting the dev server — which `--dev` asks for
//! whenever a `before_action` changes — emptied it, and with it the
//! confirmation or reset link you had just asked the app to send. It is now
//! kept in the app's database too (`_soli_mail_inbox`).
//!
//! Needs a SoliDB: `SOLIDB_DRIVER_TEST_HOST=127.0.0.1:6745` (credentials from
//! `SOLIDB_USERNAME` / `SOLIDB_PASSWORD`, `admin`/`admin` by default). Without
//! one the test skips — or fails under `SOLI_REQUIRE_DB=1`, as the driver test.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn solidb_host() -> Option<String> {
    match std::env::var("SOLIDB_DRIVER_TEST_HOST") {
        Ok(host) if !host.is_empty() => Some(host),
        _ => {
            assert!(
                !std::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1"),
                "SOLI_REQUIRE_DB=1 but SOLIDB_DRIVER_TEST_HOST is unset: \
                 the inbox persistence test would have been skipped"
            );
            eprintln!("skip: SOLIDB_DRIVER_TEST_HOST unset (no SoliDB to keep the inbox in)");
            None
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("free port")
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// Empty while the server is still starting: the caller polls.
fn request(port: u16, method: &str, path: &str) -> String {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return String::new();
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out
}

fn start(app: &Path, host: &str, database: &str) -> Server {
    let port = free_port();
    let child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
        .arg("serve")
        .arg(app)
        .arg("--dev")
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .env("SOLI_SESSION_SECRET", "inbox-test-secret-0123456789abcdef")
        .env("SOLIDB_HOST", format!("http://{host}"))
        .env("SOLIDB_DATABASE", database)
        .env(
            "SOLIDB_USERNAME",
            std::env::var("SOLIDB_USERNAME").unwrap_or_else(|_| "admin".into()),
        )
        .env(
            "SOLIDB_PASSWORD",
            std::env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "admin".into()),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = Server { child, port };
    let deadline = Instant::now() + Duration::from_secs(30);
    while !request(port, "GET", "/ping").contains(" 200 ") {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(200));
    }
    server
}

fn app_that_sends_mail(subject: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    write(
        root,
        "config/routes.sl",
        "get(\"/ping\", \"home#ping\")\nget(\"/deliver\", \"home#deliver\")\n",
    );
    write(
        root,
        "app/controllers/home_controller.sl",
        "class HomeController < Controller\n  def ping(req)\n    return render_text(\"ok\")\n  end\n\n  \
         def deliver(req)\n    WelcomeMailer.hello(\"ada@example.test\").deliver_now\n    \
         return render_text(\"sent\")\n  end\nend\n",
    );
    write(
        root,
        "app/mailers/welcome_mailer.sl",
        "class WelcomeMailer < Mailer\n  def hello(address)\n    \
         this.mail(to: address, subject: \"SUBJECT\", html: \"<p>Bonjour</p>\")\n  end\nend\n"
            .replace("SUBJECT", subject)
            .as_str(),
    );
    dir
}

#[test]
fn the_inbox_keeps_its_mail_across_a_restart() {
    let Some(host) = solidb_host() else {
        return;
    };
    // One database for every run (no pile of test databases), and a subject
    // of this run's own, so mail an earlier run left behind proves nothing.
    let database = "soli_inbox_persistence_test";
    let subject = format!(
        "Survives a restart {}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let app = app_that_sends_mail(&subject);

    {
        let server = start(app.path(), &host, database);
        let sent = request(server.port, "GET", "/deliver");
        assert!(sent.contains("sent"), "the mail was not sent:\n{sent}");
        let inbox = request(server.port, "GET", "/__soli/inbox");
        assert!(inbox.contains(&subject), "not captured:\n{inbox}");
    }

    let server = start(app.path(), &host, database);
    let inbox = request(server.port, "GET", "/__soli/inbox");
    assert!(
        inbox.contains(&subject),
        "the restart emptied the inbox:\n{inbox}"
    );

    // Clear empties the durable copy too: a third start finds nothing.
    let cleared = request(server.port, "POST", "/__soli/inbox/clear");
    assert!(!cleared.is_empty(), "no answer to clear");
    drop(server);
    let server = start(app.path(), &host, database);
    let inbox = request(server.port, "GET", "/__soli/inbox");
    assert!(
        !inbox.contains(&subject),
        "clear left the mail in the database:\n{inbox}"
    );
}
