//! End-to-end tests for encrypted / protected `.soli` bundles.
//!
//! Builds a minimal MVC app in a tempdir, bundles it with `soli build
//! --protect`, and serves the bundle with the key coming from the
//! environment, from a mock key server (`SOLI_BUNDLE_AUTH_URL` +
//! `SOLI_BUNDLE_API_KEY` as `x-api-key`), and from a `.env` next to the
//! bundle. Failure modes (wrong key, no key, revoked key server) are
//! asserted on stderr.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const KEY: &str = "integration-test-key-material";

fn pick_port() -> u16 {
    static FALLBACK: AtomicU16 = AtomicU16::new(28600);
    if let Ok(listener) = TcpListener::bind("127.0.0.1:0") {
        if let Ok(addr) = listener.local_addr() {
            return addr.port();
        }
    }
    FALLBACK.fetch_add(1, Ordering::SeqCst)
}

fn soli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_soli"))
}

/// Write a minimal MVC app and return its directory.
fn write_fixture_app(dir: &Path) {
    std::fs::create_dir_all(dir.join("app/controllers")).unwrap();
    std::fs::create_dir_all(dir.join("app/views/home")).unwrap();
    std::fs::create_dir_all(dir.join("app/views/layouts")).unwrap();
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(
        dir.join("config/routes.sl"),
        "get(\"/\", \"home#index\");\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app/controllers/home_controller.sl"),
        "def index(req)\n  plans = [{\"name\": \"Free\"}, {\"name\": \"Pro\"}]\n  \
         render(\"home/index\", {\"plans\": plans})\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app/views/home/index.html.erb"),
        "<h1>Plans</h1>\n<% for plan in plans %><li><%= plan[\"name\"] %></li><% end %>\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app/views/layouts/application.html.erb"),
        "<html><body><%= yield %></body></html>\n",
    )
    .unwrap();
}

/// Run `soli build --protect` on the fixture with `SOLI_BUNDLE_KEY` set.
/// Returns the bundle path.
fn build_protected_bundle(app_dir: &Path) -> PathBuf {
    let bundle_path = app_dir.join("app.soli");
    let output = Command::new(soli_binary())
        .arg("build")
        .arg(app_dir)
        .arg("-o")
        .arg(&bundle_path)
        .arg("--protect")
        .env("SOLI_BUNDLE_KEY", KEY)
        .env_remove("SOLI_BUNDLE_AUTH_URL")
        .output()
        .expect("run soli build");
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(&bundle_path).unwrap();
    assert_eq!(&bytes[..4], b"SOLE", "bundle must be encrypted");
    // No source strings survive: neither the variable name nor the literal.
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(!haystack.contains("plans"), "source leaked into bundle");
    bundle_path
}

struct BundleServer {
    child: Child,
    port: u16,
}

impl BundleServer {
    fn spawn(bundle: &Path, envs: &[(&str, &str)], remove: &[&str]) -> Self {
        let port = pick_port();
        let mut cmd = Command::new(soli_binary());
        cmd.arg("serve")
            .arg(bundle)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
            // Portability off-Linux (macOS has no /dev/shm).
            .env("SOLI_BUNDLE_ALLOW_DISK", "1")
            .env_remove("SOLI_BUNDLE_KEY")
            .env_remove("SOLI_BUNDLE_AUTH_URL")
            .env_remove("SOLI_BUNDLE_API_KEY")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        for k in remove {
            cmd.env_remove(k);
        }
        let child = cmd.spawn().expect("spawn soli serve");
        let server = BundleServer { child, port };
        server.wait_ready();
        server
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if ureq::get(&format!("http://127.0.0.1:{}/", self.port))
                .timeout(Duration::from_millis(500))
                .call()
                .is_ok()
            {
                return;
            }
            if Instant::now() >= deadline {
                panic!("bundle server on port {} never became ready", self.port);
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn get_root(&self) -> String {
        let resp = ureq::get(&format!("http://127.0.0.1:{}/", self.port))
            .timeout(Duration::from_secs(5))
            .call()
            .expect("GET /");
        let mut body = String::new();
        resp.into_reader().read_to_string(&mut body).unwrap();
        body
    }
}

impl Drop for BundleServer {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            let _ = kill(Pid::from_raw(self.child.id() as i32), Signal::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() >= deadline => break,
                _ => thread::sleep(Duration::from_millis(50)),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `soli serve` expecting a boot FAILURE; returns stderr.
fn serve_expect_failure(bundle: &Path, envs: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(soli_binary());
    cmd.arg("serve")
        .arg(bundle)
        .arg("--port")
        .arg(pick_port().to_string())
        .env("SOLI_BUNDLE_ALLOW_DISK", "1")
        .env_remove("SOLI_BUNDLE_KEY")
        .env_remove("SOLI_BUNDLE_AUTH_URL")
        .env_remove("SOLI_BUNDLE_API_KEY");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run soli serve");
    assert!(
        !output.status.success(),
        "serve unexpectedly succeeded; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Minimal single-request mock key server. Asserts the `x-api-key` header
/// and responds with the key material.
fn spawn_mock_key_server(expected_api_key: &'static str) -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        // Serve a handful of requests (boot may retry); exit when the
        // listener would block past the test's lifetime.
        listener.set_nonblocking(false).expect("blocking listener");
        for _ in 0..4 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_lowercase();
            let (status, body) = if request.contains(&format!("x-api-key: {}", expected_api_key)) {
                ("200 OK", KEY)
            } else {
                ("403 Forbidden", "")
            };
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (port, handle)
}

#[test]
fn protected_bundle_serves_with_env_key() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_app(dir.path());
    let bundle = build_protected_bundle(dir.path());

    let server = BundleServer::spawn(&bundle, &[("SOLI_BUNDLE_KEY", KEY)], &[]);
    let body = server.get_root();
    assert!(body.contains("<h1>Plans</h1>"), "got: {body}");
    assert!(body.contains("<li>Free</li>"), "got: {body}");
    assert!(body.contains("<li>Pro</li>"), "got: {body}");
}

#[test]
fn protected_bundle_serves_with_key_from_key_server() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_app(dir.path());
    let bundle = build_protected_bundle(dir.path());

    let (key_port, _handle) = spawn_mock_key_server("sekrit-api-key");
    let auth_url = format!("http://127.0.0.1:{}/app-key", key_port);
    let server = BundleServer::spawn(
        &bundle,
        &[
            ("SOLI_BUNDLE_AUTH_URL", auth_url.as_str()),
            ("SOLI_BUNDLE_API_KEY", "sekrit-api-key"),
        ],
        &[],
    );
    assert!(server.get_root().contains("<li>Free</li>"));
}

#[test]
fn key_server_rejecting_api_key_blocks_boot() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_app(dir.path());
    let bundle = build_protected_bundle(dir.path());

    let (key_port, _handle) = spawn_mock_key_server("the-real-key");
    let auth_url = format!("http://127.0.0.1:{}/app-key", key_port);
    let stderr = serve_expect_failure(
        &bundle,
        &[
            ("SOLI_BUNDLE_AUTH_URL", auth_url.as_str()),
            ("SOLI_BUNDLE_API_KEY", "a-revoked-key"),
        ],
    );
    assert!(
        stderr.contains("rejected the API key") && stderr.contains("revoked"),
        "got: {stderr}"
    );
}

#[test]
fn wrong_key_fails_with_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_app(dir.path());
    let bundle = build_protected_bundle(dir.path());

    let stderr = serve_expect_failure(&bundle, &[("SOLI_BUNDLE_KEY", "not-the-key")]);
    assert!(stderr.contains("wrong or rotated key"), "got: {stderr}");
}

#[test]
fn missing_key_config_names_the_env_vars() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_app(dir.path());
    let bundle = build_protected_bundle(dir.path());

    let stderr = serve_expect_failure(&bundle, &[]);
    assert!(stderr.contains("SOLI_BUNDLE_KEY"), "got: {stderr}");
    assert!(stderr.contains("SOLI_BUNDLE_AUTH_URL"), "got: {stderr}");
}

#[test]
fn key_config_in_dotenv_next_to_bundle_is_used() {
    let dir = tempfile::tempdir().unwrap();
    write_fixture_app(dir.path());
    let bundle = build_protected_bundle(dir.path());

    // The key lives ONLY in a .env next to the .soli — nothing in the
    // process environment.
    std::fs::write(
        bundle.parent().unwrap().join(".env"),
        format!("SOLI_BUNDLE_KEY={}\n", KEY),
    )
    .unwrap();

    let server = BundleServer::spawn(&bundle, &[], &[]);
    assert!(server.get_root().contains("<li>Free</li>"));
}

// NOTE: the protected-bundle soli-version lock is covered deterministically
// by the `check_bundle_meta_rejects_version_mismatch` unit test in
// src/bundle.rs — an e2e variant would have to forge a version-mismatched
// meta with a byte-length-preserving edit, which is brittle.
