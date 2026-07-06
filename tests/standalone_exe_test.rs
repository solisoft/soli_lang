//! End-to-end tests for self-executing bundles (`soli build --standalone`).
//!
//! A standalone executable is the soli runtime + appended bundle + a 16-byte
//! footer. NOTE ON SIZE: each artifact embeds the full test-profile soli
//! binary (hundreds of MB in debug), so the two shared fixtures (plain +
//! protected) are built ONCE per test binary in a `OnceLock` and reused;
//! tests that must mutate or relocate an artifact hard-link it (falling back
//! to copy) instead of rebuilding.
//!
//! Cross-target (`--target`) tests never touch the network: they point
//! `SOLI_RELEASE_BASE_URL` at a local mock server that publishes a tiny fake
//! runtime tarball + `.sha256`, exercising download, checksum verification,
//! and caching. The produced cross-target artifact is never executed — the
//! append logic is shared with the host path, which the other tests run for
//! real.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const KEY: &str = "standalone-test-key-material";
/// Unique string planted in the fixture app so leak checks can't
/// false-positive against the runtime's own rodata.
const LEAK_MARKER: &str = "zz_soli_leak_marker_zz";

const FOOTER_MAGIC: &[u8; 8] = b"SOLIXEC1";

fn pick_port() -> u16 {
    static FALLBACK: AtomicU16 = AtomicU16::new(28900);
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
        format!(
            "def index(req)\n  marker = \"{}\"\n  \
             render(\"home/index\", {{\"marker\": marker}})\nend\n",
            LEAK_MARKER
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("app/views/home/index.html.erb"),
        "<h1>Standalone</h1>\n<p><%= marker %></p>\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app/views/layouts/application.html.erb"),
        "<html><body><%= yield %></body></html>\n",
    )
    .unwrap();
}

/// Run `soli build --standalone [extra args]` and return (status, stderr).
fn build_standalone(
    app_dir: &Path,
    output: Option<&Path>,
    extra: &[&str],
    envs: &[(&str, &str)],
    cwd: Option<&Path>,
) -> (std::process::ExitStatus, String) {
    let mut cmd = Command::new(soli_binary());
    cmd.arg("build")
        .arg(app_dir)
        .arg("--standalone")
        .env("SOLI_BUNDLE_KEY", KEY)
        .env_remove("SOLI_BUNDLE_AUTH_URL")
        .env_remove("SOLI_RELEASE_BASE_URL");
    if let Some(out) = output {
        cmd.arg("-o").arg(out);
    }
    for a in extra {
        cmd.arg(a);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().expect("run soli build");
    (out.status, String::from_utf8_lossy(&out.stderr).to_string())
}

struct Fixtures {
    _dir: tempfile::TempDir,
    plain_exe: PathBuf,
    protected_exe: PathBuf,
}

static FIXTURES: OnceLock<Fixtures> = OnceLock::new();

fn fixtures() -> &'static Fixtures {
    FIXTURES.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let app_dir = dir.path().join("app_src");
        std::fs::create_dir_all(&app_dir).unwrap();
        write_fixture_app(&app_dir);

        let plain_exe = dir.path().join("app_plain");
        let (status, stderr) = build_standalone(&app_dir, Some(&plain_exe), &[], &[], None);
        assert!(status.success(), "plain build failed: {stderr}");

        let protected_exe = dir.path().join("app_protected");
        let (status, stderr) =
            build_standalone(&app_dir, Some(&protected_exe), &["--protect"], &[], None);
        assert!(status.success(), "protected build failed: {stderr}");

        Fixtures {
            _dir: dir,
            plain_exe,
            protected_exe,
        }
    })
}

/// Hard-link `src` into `dst` (same tmpfs), falling back to a full copy.
fn link_or_copy(src: &Path, dst: &Path) {
    if std::fs::hard_link(src, dst).is_err() {
        std::fs::copy(src, dst).unwrap();
    }
}

struct StandaloneServer {
    child: Child,
    port: u16,
}

impl StandaloneServer {
    fn spawn(exe: &Path, envs: &[(&str, &str)]) -> Self {
        let port = pick_port();
        let mut cmd = Command::new(exe);
        cmd.arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
            .env("SOLI_BUNDLE_ALLOW_DISK", "1")
            .env_remove("SOLI_BUNDLE_KEY")
            .env_remove("SOLI_BUNDLE_AUTH_URL")
            .env_remove("SOLI_BUNDLE_API_KEY")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("spawn standalone exe");
        let server = StandaloneServer { child, port };
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
                panic!("standalone server on port {} never became ready", self.port);
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

impl Drop for StandaloneServer {
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

/// Parse the footer of a standalone artifact: (payload_offset, payload_len).
fn payload_region(exe_bytes: &[u8]) -> (usize, usize) {
    assert!(exe_bytes.len() > 16, "artifact too small for a footer");
    let footer = &exe_bytes[exe_bytes.len() - 16..];
    assert_eq!(&footer[..8], FOOTER_MAGIC, "missing SOLIXEC1 footer");
    let payload_len = u64::from_le_bytes(footer[8..16].try_into().unwrap()) as usize;
    let payload_off = exe_bytes.len() - 16 - payload_len;
    (payload_off, payload_len)
}

// ---------------------------------------------------------------------------
// Host-platform artifacts (built and RUN for real)
// ---------------------------------------------------------------------------

#[test]
fn plain_standalone_serves_http() {
    let fx = fixtures();
    let bytes = std::fs::read(&fx.plain_exe).unwrap();

    // Structure: bigger than the runtime alone, footer at EOF, SOLB payload.
    assert!(bytes.len() as u64 > std::fs::metadata(soli_binary()).unwrap().len());
    let (off, len) = payload_region(&bytes);
    assert_eq!(&bytes[off..off + 4], b"SOLB", "plain payload magic");
    assert!(len > 0);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&fx.plain_exe)
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "artifact must be executable");
    }

    let server = StandaloneServer::spawn(&fx.plain_exe, &[]);
    let body = server.get_root();
    assert!(body.contains("<h1>Standalone</h1>"), "got: {body}");
    assert!(body.contains(LEAK_MARKER), "got: {body}");
}

#[test]
fn protected_standalone_serves_with_env_key_and_leaks_no_source() {
    let fx = fixtures();
    let bytes = std::fs::read(&fx.protected_exe).unwrap();

    let (off, _) = payload_region(&bytes);
    assert_eq!(
        &bytes[off..off + 4],
        b"SOLE",
        "protected payload is encrypted"
    );
    // The marker must not appear anywhere: not in the encrypted payload and
    // not in the runtime portion.
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(
        !haystack.contains(LEAK_MARKER),
        "app source leaked into the artifact"
    );

    let server = StandaloneServer::spawn(&fx.protected_exe, &[("SOLI_BUNDLE_KEY", KEY)]);
    assert!(server.get_root().contains(LEAK_MARKER));
}

#[test]
fn encrypted_standalone_reads_dotenv_beside_exe() {
    let fx = fixtures();
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("app");
    link_or_copy(&fx.protected_exe, &exe);
    // The key lives ONLY in a .env next to the executable.
    std::fs::write(
        dir.path().join(".env"),
        format!("SOLI_BUNDLE_KEY={}\n", KEY),
    )
    .unwrap();

    let server = StandaloneServer::spawn(&exe, &[]);
    assert!(server.get_root().contains(LEAK_MARKER));
}

#[test]
fn tampered_payload_refuses_cleanly() {
    let fx = fixtures();
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("app_tampered");
    // Full copy (NOT hard link): we mutate the bytes.
    std::fs::copy(&fx.plain_exe, &exe).unwrap();

    let mut bytes = std::fs::read(&exe).unwrap();
    let (off, _) = payload_region(&bytes);
    bytes[off + 1] ^= 0xFF; // corrupt the payload magic
    std::fs::write(&exe, &bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let out = Command::new(&exe)
        .arg("--port")
        .arg(pick_port().to_string())
        .output()
        .expect("run tampered exe");
    assert!(!out.status.success(), "tampered artifact must not boot");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid"), "got: {stderr}");
    assert!(
        !stderr.contains("panicked"),
        "must fail cleanly, not panic: {stderr}"
    );
}

#[test]
fn standalone_help_and_version_do_not_boot() {
    let fx = fixtures();

    let out = Command::new(&fx.plain_exe).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--port"), "got: {stdout}");
    assert!(stdout.contains("SOLI_BUNDLE_KEY"), "got: {stdout}");

    let out = Command::new(&fx.plain_exe)
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")) && stdout.contains("plain bundle"),
        "got: {stdout}"
    );
}

#[test]
fn standalone_missing_key_fails_with_env_var_names() {
    let fx = fixtures();
    let out = Command::new(&fx.protected_exe)
        .arg("--port")
        .arg(pick_port().to_string())
        .env("SOLI_BUNDLE_ALLOW_DISK", "1")
        .env_remove("SOLI_BUNDLE_KEY")
        .env_remove("SOLI_BUNDLE_AUTH_URL")
        .env_remove("SOLI_BUNDLE_API_KEY")
        .output()
        .expect("run protected exe without key");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("SOLI_BUNDLE_KEY"), "got: {stderr}");
    assert!(stderr.contains("SOLI_BUNDLE_AUTH_URL"), "got: {stderr}");
}

#[test]
fn build_standalone_refuses_directory_output() {
    let dir = tempfile::tempdir().unwrap();
    let app_dir = dir.path().join("my_app");
    std::fs::create_dir_all(&app_dir).unwrap();
    write_fixture_app(&app_dir);

    // Building from the parent dir: the default output "my_app" collides
    // with the source directory itself.
    let (status, stderr) = build_standalone(&app_dir, None, &[], &[], Some(dir.path()));
    assert!(!status.success(), "build must refuse a directory output");
    assert!(stderr.contains("is a directory"), "got: {stderr}");
}

// ---------------------------------------------------------------------------
// Cross-target builds (mock release server; artifact never executed)
// ---------------------------------------------------------------------------

/// A gzipped tar containing a single `soli` file with the given bytes.
fn make_runtime_tarball(runtime: &[u8]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_size(runtime.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append_data(&mut header, "soli", runtime).unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Serve `soli-*.tar.gz` and its `.sha256` for a handful of requests.
fn spawn_mock_release_server(tarball: Vec<u8>, sha_hex: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = request
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            let (status, body): (&str, Vec<u8>) = if path.ends_with(".tar.gz") {
                ("200 OK", tarball.clone())
            } else if path.ends_with(".sha256") {
                ("200 OK", sha_hex.clone().into_bytes())
            } else {
                ("404 Not Found", Vec::new())
            };
            let header = format!(
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                status,
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    port
}

const FAKE_RUNTIME: &[u8] = b"\x7fELF-fake-runtime-for-cross-target-tests-0123456789";

#[test]
fn target_build_downloads_verifies_and_appends() {
    let dir = tempfile::tempdir().unwrap();
    let app_dir = dir.path().join("xapp");
    std::fs::create_dir_all(&app_dir).unwrap();
    write_fixture_app(&app_dir);

    let tarball = make_runtime_tarball(FAKE_RUNTIME);
    let sha = sha256_hex(&tarball);
    let port = spawn_mock_release_server(tarball, sha);
    let base_url = format!("http://127.0.0.1:{}", port);
    let cache = dir.path().join("cache");

    let (status, stderr) = build_standalone(
        &app_dir,
        None,
        &["--target", "linux-arm64"],
        &[
            ("SOLI_RELEASE_BASE_URL", base_url.as_str()),
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ],
        Some(dir.path()),
    );
    assert!(status.success(), "cross-target build failed: {stderr}");

    // Default name is <app>-<target>; contents are runtime ‖ payload ‖ footer.
    let artifact = dir.path().join("xapp-linux-arm64");
    let bytes = std::fs::read(&artifact).expect("artifact written");
    assert!(bytes.starts_with(FAKE_RUNTIME), "runtime prefix mismatch");
    let (off, _) = payload_region(&bytes);
    assert_eq!(off, FAKE_RUNTIME.len(), "payload must start after runtime");
    assert_eq!(&bytes[off..off + 4], b"SOLB", "plain payload magic");
}

#[test]
fn target_build_hard_fails_on_sha_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let app_dir = dir.path().join("xapp");
    std::fs::create_dir_all(&app_dir).unwrap();
    write_fixture_app(&app_dir);

    let tarball = make_runtime_tarball(FAKE_RUNTIME);
    let port = spawn_mock_release_server(tarball, "deadbeef".repeat(8));
    let base_url = format!("http://127.0.0.1:{}", port);
    let cache = dir.path().join("cache");

    let (status, stderr) = build_standalone(
        &app_dir,
        None,
        &["--target", "linux-arm64"],
        &[
            ("SOLI_RELEASE_BASE_URL", base_url.as_str()),
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ],
        Some(dir.path()),
    );
    assert!(!status.success(), "tampered download must fail the build");
    assert!(stderr.contains("checksum mismatch"), "got: {stderr}");
    assert!(
        !dir.path().join("xapp-linux-arm64").exists(),
        "no artifact may be written on checksum failure"
    );
}

#[test]
fn target_build_uses_cache_on_second_run() {
    let dir = tempfile::tempdir().unwrap();
    let app_dir = dir.path().join("xapp");
    std::fs::create_dir_all(&app_dir).unwrap();
    write_fixture_app(&app_dir);

    let tarball = make_runtime_tarball(FAKE_RUNTIME);
    let sha = sha256_hex(&tarball);
    let port = spawn_mock_release_server(tarball, sha);
    let base_url = format!("http://127.0.0.1:{}", port);
    let cache = dir.path().join("cache");

    let (status, stderr) = build_standalone(
        &app_dir,
        None,
        &["--target", "linux-arm64"],
        &[
            ("SOLI_RELEASE_BASE_URL", base_url.as_str()),
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ],
        Some(dir.path()),
    );
    assert!(
        status.success(),
        "first cross-target build failed: {stderr}"
    );
    std::fs::remove_file(dir.path().join("xapp-linux-arm64")).unwrap();

    // Second build points at an unreachable base URL: it can only succeed
    // via the cache populated by the first build.
    let (status, stderr) = build_standalone(
        &app_dir,
        None,
        &["--target", "linux-arm64"],
        &[
            ("SOLI_RELEASE_BASE_URL", "http://127.0.0.1:1"),
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ],
        Some(dir.path()),
    );
    assert!(
        status.success(),
        "cached cross-target build failed: {stderr}"
    );
    let bytes = std::fs::read(dir.path().join("xapp-linux-arm64")).unwrap();
    assert!(bytes.starts_with(FAKE_RUNTIME));
}

#[test]
fn target_rejects_unknown_and_requires_standalone() {
    let dir = tempfile::tempdir().unwrap();
    let app_dir = dir.path().join("xapp");
    std::fs::create_dir_all(&app_dir).unwrap();
    write_fixture_app(&app_dir);

    let (status, stderr) = build_standalone(
        &app_dir,
        None,
        &["--target", "windows-amd64"],
        &[],
        Some(dir.path()),
    );
    assert!(!status.success());
    assert!(
        stderr.contains("linux-amd64") && stderr.contains("darwin-arm64"),
        "error must list supported targets, got: {stderr}"
    );

    // --target without --standalone is a usage error.
    let out = Command::new(soli_binary())
        .arg("build")
        .arg(&app_dir)
        .arg("--target")
        .arg("linux-arm64")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--target only applies to --standalone"),
        "got: {stderr}"
    );
}
