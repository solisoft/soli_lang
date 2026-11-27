//! The dev error page's REPL answers with the token the page carries.
//!
//! The page gets the token only when the request named a local `Host`, and it
//! read that header from the request's wire map — which the pipeline had
//! already taken out to build `req["headers"]`. The page therefore embedded an
//! empty token, and whatever one typed in the REPL answered "Forbidden dev REPL
//! request" (2026-11-27). Under `--dev` the headers are now kept.

use std::io::{Read, Write};
use std::net::TcpStream;
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

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn send(port: u16, request: &str) -> String {
    // Empty while the server is still starting: the caller polls.
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return String::new();
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out
}

fn get(port: u16, path: &str) -> String {
    send(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\nAccept: text/html\r\nConnection: close\r\n\r\n"),
    )
}

fn start(app: &Path) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("a free port");
    let child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
        .arg("serve")
        .arg(app)
        .arg("--dev")
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .env("SOLI_SESSION_SECRET", "repl-test-secret-0123456789abcdef")
        .env_remove("SOLI_DEV_REPL_SECRET")
        .env_remove("SOLI_DEV_REPL_ALLOW_REMOTE")
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

fn embedded_token(page: &str) -> String {
    let marker = "const devReplToken = \"";
    let start = page
        .find(marker)
        .unwrap_or_else(|| panic!("no REPL token in the page:\n{page}"))
        + marker.len();
    let len = page[start..].find('"').expect("closing quote");
    page[start..start + len].to_string()
}

#[test]
fn the_error_page_repl_runs_code_with_the_token_it_embeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    write(
        root,
        "config/routes.sl",
        "get(\"/ping\", \"home#ping\")\nget(\"/boom\", \"home#boom\")\n",
    );
    write(
        root,
        "app/controllers/home_controller.sl",
        "class HomeController < Controller\n  def ping(req)\n    render_text(\"ok\")\n  end\n\n  \
         def boom(req)\n    missing = nil\n    missing.explode\n  end\nend\n",
    );
    let server = start(root);

    let page = get(server.port, "/boom");
    assert!(
        page.contains(" 500 "),
        "expected the dev error page:\n{page}"
    );
    let token = embedded_token(&page);
    assert!(!token.is_empty(), "the page embedded an empty REPL token");

    let body = r#"{"code": "20 + 22", "request_data": {}, "breakpoint_env": null}"#;
    let answer = send(
        server.port,
        &format!(
            "POST /__dev/repl HTTP/1.1\r\nHost: localhost:{port}\r\nOrigin: http://localhost:{port}\r\n\
             Content-Type: application/json\r\nX-Soli-Dev-Token: {token}\r\nContent-Length: {len}\r\n\
             Connection: close\r\n\r\n{body}",
            port = server.port,
            len = body.len(),
        ),
    );
    assert!(answer.contains(" 200 "), "REPL refused:\n{answer}");
    assert!(answer.contains("42"), "REPL result missing:\n{answer}");
}
