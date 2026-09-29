//! The dev previews (`/__soli/mailers/…`, `/__soli/components/…`) render with
//! the app's view helpers.
//!
//! `/__soli/*` pages are answered in the async handler, before any worker, and
//! workers are where `app/helpers/*.sl` gets loaded. A mailer view calling an
//! app helper — a translation helper, in every localised email — therefore
//! rendered "Cannot call non-function value" in the gallery while the real
//! email rendered fine (every email of a GRC app, 29/09/2026). The preview now
//! loads the helpers on its own thread first.

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

fn get(port: u16, path: &str) -> String {
    // Empty while the server is still starting: the caller polls.
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return String::new();
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out
}

fn start(app: &Path) -> Server {
    let port = free_port();
    let child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
        .arg("serve")
        .arg(app)
        .arg("--dev")
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .env(
            "SOLI_SESSION_SECRET",
            "preview-test-secret-0123456789abcdef",
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = Server { child, port };
    let deadline = Instant::now() + Duration::from_secs(30);
    while !get(port, "/ping").contains(" 200 ") {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(200));
    }
    server
}

fn app_with_a_helper() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    write(root, "config/routes.sl", "get(\"/ping\", \"home#ping\")\n");
    write(
        root,
        "app/controllers/home_controller.sl",
        "class HomeController < Controller\n  def ping(req)\n    return render_text(\"ok\")\n  end\nend\n",
    );
    write(
        root,
        "app/helpers/greeting_helper.sl",
        "def greeting_for(name)\n  return \"Bonjour \" + name.upcase()\nend\n",
    );
    write(
        root,
        "app/views/welcome_mailer/hello.html.slv",
        "<%# preview: { \"user\": { \"name\": \"ada\" } } %>\n<p><%= greeting_for(user[\"name\"]) %></p>\n",
    );
    write(
        root,
        "app/views/components/badge.html.slv",
        "<%# preview: { \"label\": \"ada\" } %>\n<span><%= greeting_for(label) %></span>\n",
    );
    dir
}

#[test]
fn a_mailer_preview_calls_an_app_helper() {
    let app = app_with_a_helper();
    let server = start(app.path());
    let page = get(server.port, "/__soli/mailers/welcome_mailer/hello");
    assert!(!page.contains("render error"), "preview failed:\n{page}");
    assert!(
        page.contains("Bonjour ADA"),
        "helper output missing:\n{page}"
    );
}

#[test]
fn a_component_preview_calls_an_app_helper() {
    let app = app_with_a_helper();
    let server = start(app.path());
    let page = get(server.port, "/__soli/components/badge");
    assert!(!page.contains("render error"), "preview failed:\n{page}");
    assert!(
        page.contains("Bonjour ADA"),
        "helper output missing:\n{page}"
    );
}
