//! End-to-end tests for the per-project version pin
//! (`soli.toml` → `[package] soli_version = "=X.Y.Z"`).
//!
//! None of these touch the network. `SOLI_RELEASE_BASE_URL` points at a local
//! mock server publishing a tiny fake "toolchain" — a `#!/bin/sh` script that
//! echoes a sentinel, its argv and the guard variable — so the tests can assert
//! that the pinned binary really replaced the process, and what it was handed.
//! That makes the file Unix-only; Windows has no `exec` and a different switch
//! path, covered by the unit tests in `src/cli/toolchain.rs`.
//!
//! `XDG_CACHE_HOME` is redirected into a tempdir throughout, so a test run
//! never reads or writes the developer's real toolchain cache.

#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

fn soli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_soli"))
}

/// The version the tests pin to. Deliberately absurd so it can never collide
/// with the running binary's real version and accidentally short-circuit.
const PINNED: &str = "9.9.9";

/// Printed by the fake toolchain, so a test can prove the switch happened
/// rather than inferring it from an exit code.
const SENTINEL: &str = "zz_pinned_toolchain_ran_zz";

/// A stand-in for a published soli: it reports that it ran, what argv it got,
/// and what the shim put in the guard variable.
fn fake_toolchain() -> Vec<u8> {
    format!(
        "#!/bin/sh\n\
         echo {SENTINEL}\n\
         echo \"argv: $*\"\n\
         echo \"guard: $SOLI_PINNED_EXEC\"\n"
    )
    .into_bytes()
}

fn make_tarball(runtime: &[u8]) -> Vec<u8> {
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

/// What the mock release server should do with the `.sha256` sibling.
#[derive(Clone, Copy)]
enum Checksum {
    /// Publish the real hash.
    Correct,
    /// Publish a hash that does not match the tarball.
    Wrong,
    /// 404 the checksum, as pre-SEC-041 releases do.
    Missing,
}

fn spawn_mock_release_server(tarball: Vec<u8>, checksum: Checksum) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let sha = match checksum {
        Checksum::Correct => sha256_hex(&tarball),
        Checksum::Wrong => "0".repeat(64),
        Checksum::Missing => String::new(),
    };
    thread::spawn(move || {
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let path = String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();

            let (status, body): (&str, Vec<u8>) = if path.ends_with(".sha256") {
                match checksum {
                    Checksum::Missing => ("404 Not Found", Vec::new()),
                    _ => ("200 OK", sha.clone().into_bytes()),
                }
            } else if path.ends_with(".tar.gz") {
                ("200 OK", tarball.clone())
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

/// A project directory whose manifest pins `version`.
fn write_pinned_project(dir: &Path, version: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("soli.toml"),
        format!(
            "[package]\nname = \"pinned\"\nversion = \"0.1.0\"\nmain = \"app.sl\"\nsoli_version = \"={version}\"\n"
        ),
    )
    .unwrap();
}

struct Run {
    stdout: String,
    stderr: String,
    ok: bool,
}

/// Run the real `soli` inside `cwd`, with the cache and release base redirected.
fn run_soli(
    cwd: &Path,
    cache: &Path,
    base_url: Option<&str>,
    args: &[&str],
    env: &[(&str, &str)],
) -> Run {
    let mut cmd = Command::new(soli_binary());
    cmd.current_dir(cwd)
        .args(args)
        .env("XDG_CACHE_HOME", cache)
        // Keep the test off any developer-configured pin behaviour.
        .env_remove("SOLI_NO_PIN")
        .env_remove("SOLI_PINNED_EXEC");
    if let Some(url) = base_url {
        cmd.env("SOLI_RELEASE_BASE_URL", url);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("soli should run");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        ok: out.status.success(),
    }
}

/// The test that proves the feature: a pinned project runs the pinned binary,
/// and that binary receives the arguments the user typed.
#[test]
fn a_pinned_project_switches_to_the_pinned_toolchain() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);

    let port = spawn_mock_release_server(make_tarball(&fake_toolchain()), Checksum::Correct);
    let base = format!("http://127.0.0.1:{port}");
    let cache = dir.path().join("cache");

    let run = run_soli(
        &project,
        &cache,
        Some(&base),
        &["routes", "-g", "posts"],
        &[],
    );

    assert!(run.ok, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains(SENTINEL),
        "the pinned toolchain did not run.\nstdout: {}\nstderr: {}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("argv: routes -g posts"),
        "argv was not forwarded verbatim: {}",
        run.stdout
    );
    // The fetch must announce itself — this is the disclosure that the
    // interpreter is now repo-selected.
    assert!(
        run.stdout.contains("Fetching soli 9.9.9 pinned by"),
        "the fetch was silent: {}",
        run.stdout
    );
}

/// The loop-prevention regression test. Without the guard, a toolchain whose
/// compiled version disagrees with its release tag re-execs itself forever.
#[test]
fn the_switch_sets_the_guard_variable_for_the_child() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);

    let port = spawn_mock_release_server(make_tarball(&fake_toolchain()), Checksum::Correct);
    let base = format!("http://127.0.0.1:{port}");
    let cache = dir.path().join("cache");

    let run = run_soli(&project, &cache, Some(&base), &["routes"], &[]);

    assert!(
        run.stdout.contains(&format!("guard: {PINNED}")),
        "the child did not receive SOLI_PINNED_EXEC: {}",
        run.stdout
    );
}

/// An already-set guard means "you are the child" — never switch again,
/// whatever the manifest says.
#[test]
fn an_existing_guard_prevents_a_second_switch() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let cache = dir.path().join("cache");

    // An unreachable base URL: if the shim tried to fetch, this would fail
    // loudly instead of falling through to the host binary.
    let run = run_soli(
        &project,
        &cache,
        Some("http://127.0.0.1:1"),
        &["--help"],
        &[("SOLI_PINNED_EXEC", PINNED)],
    );

    assert!(!run.stdout.contains(SENTINEL));
    assert!(!run.stderr.contains("Fetching"));
}

/// The cache is the point: a second run must not touch the network at all.
#[test]
fn a_cached_toolchain_is_reused_without_the_network() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let cache = dir.path().join("cache");

    let port = spawn_mock_release_server(make_tarball(&fake_toolchain()), Checksum::Correct);
    let base = format!("http://127.0.0.1:{port}");
    let first = run_soli(&project, &cache, Some(&base), &["routes"], &[]);
    assert!(first.stdout.contains(SENTINEL), "stderr: {}", first.stderr);

    // Point at a dead port. A cache miss would now fail.
    let second = run_soli(
        &project,
        &cache,
        Some("http://127.0.0.1:1"),
        &["routes"],
        &[],
    );

    assert!(
        second.stdout.contains(SENTINEL),
        "stderr: {}",
        second.stderr
    );
    assert!(
        !second.stdout.contains("Fetching"),
        "the second run refetched: {}",
        second.stdout
    );
}

#[test]
fn a_checksum_mismatch_fails_hard_and_caches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let cache = dir.path().join("cache");

    let port = spawn_mock_release_server(make_tarball(&fake_toolchain()), Checksum::Wrong);
    let base = format!("http://127.0.0.1:{port}");

    let run = run_soli(&project, &cache, Some(&base), &["routes"], &[]);

    assert!(!run.ok);
    assert!(
        run.stderr.contains("checksum mismatch"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stdout.contains(SENTINEL));
    assert!(
        !cache
            .join("soli/runtimes")
            .join(format!("v{PINNED}"))
            .exists(),
        "a rejected download must leave nothing behind"
    );
}

/// The one place the pin is deliberately stricter than `soli build --target`:
/// a release with no published checksum. Those bytes are about to be executed.
#[test]
fn a_missing_checksum_is_refused_for_a_pin() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let cache = dir.path().join("cache");

    let port = spawn_mock_release_server(make_tarball(&fake_toolchain()), Checksum::Missing);
    let base = format!("http://127.0.0.1:{port}");

    let run = run_soli(&project, &cache, Some(&base), &["routes"], &[]);

    assert!(!run.ok);
    assert!(
        run.stderr
            .contains("refusing to run an unverified interpreter"),
        "stderr: {}",
        run.stderr
    );
}

/// An unpublished version must say which version and which URL, so the fix is
/// obvious from the message alone.
#[test]
fn an_unpublished_pin_names_the_version_and_the_url() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let cache = dir.path().join("cache");

    // A server that 404s everything.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for _ in 0..4 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let base = format!("http://127.0.0.1:{port}");

    let run = run_soli(&project, &cache, Some(&base), &["routes"], &[]);

    assert!(!run.ok);
    assert!(run.stderr.contains(PINNED), "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("soli-linux-amd64.tar.gz")
            || run.stderr.contains("no published soli for this platform"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("SOLI_NO_PIN"),
        "the error must offer the escape hatch: {}",
        run.stderr
    );
}

#[test]
fn soli_no_pin_skips_the_switch() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let cache = dir.path().join("cache");

    // Unreachable base URL: any attempt to fetch would fail loudly.
    let run = run_soli(
        &project,
        &cache,
        Some("http://127.0.0.1:1"),
        &["--help"],
        &[("SOLI_NO_PIN", "1")],
    );

    assert!(!run.stdout.contains(SENTINEL));
    assert!(!run.stdout.contains("Fetching"));
}

/// `soli update` must replace the *installed* binary. Redirected into a pinned
/// toolchain it would overwrite a cache entry with a different version.
#[test]
fn update_and_probes_bypass_the_pin() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let cache = dir.path().join("cache");

    for args in [vec!["--version"], vec!["--help"], vec!["which"]] {
        let run = run_soli(&project, &cache, Some("http://127.0.0.1:1"), &args, &[]);
        assert!(
            !run.stdout.contains(SENTINEL) && !run.stdout.contains("Fetching"),
            "{args:?} should not have switched: {}",
            run.stdout
        );
    }
}

/// A pin that could escape the cache directory or the release URL is ignored,
/// and the project keeps working on the invoked soli.
#[test]
fn a_malformed_pin_never_becomes_a_path() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("soli.toml"),
        "[package]\nname = \"pinned\"\nversion = \"0.1.0\"\nsoli_version = \"=../../../../evil\"\n",
    )
    .unwrap();
    let cache = dir.path().join("cache");

    let run = run_soli(
        &project,
        &cache,
        Some("http://127.0.0.1:1"),
        &["which"],
        &[],
    );

    assert!(run.ok, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("pinned   no"),
        "a malformed pin must read as no pin: {}",
        run.stdout
    );
    assert!(!dir.path().join("evil").exists());
}

/// `soli which` is the diagnostic: it reports the resolution instead of
/// following it, and names the manifest responsible.
#[test]
fn which_reports_the_pin_without_following_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let cache = dir.path().join("cache");

    let run = run_soli(
        &project,
        &cache,
        Some("http://127.0.0.1:1"),
        &["which"],
        &[],
    );

    assert!(run.ok, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains(&format!("soli {PINNED}")),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("not downloaded yet"), "{}", run.stdout);
    assert!(
        run.stdout.contains("soli.toml"),
        "which must name the manifest: {}",
        run.stdout
    );
}

/// The upward walk: you run soli from a subdirectory, and the manifest is
/// several levels up.
#[test]
fn the_pin_is_found_from_a_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    write_pinned_project(&project, PINNED);
    let nested = project.join("app").join("models");
    std::fs::create_dir_all(&nested).unwrap();
    let cache = dir.path().join("cache");

    let run = run_soli(&nested, &cache, Some("http://127.0.0.1:1"), &["which"], &[]);

    assert!(
        run.stdout.contains(&format!("soli {PINNED}")),
        "{}",
        run.stdout
    );
}

/// A directory with no manifest must behave exactly as before: no pin, no
/// extra work, the invoked binary runs.
#[test]
fn an_unpinned_directory_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");

    let run = run_soli(
        dir.path(),
        &cache,
        Some("http://127.0.0.1:1"),
        &["which"],
        &[],
    );

    assert!(run.ok, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("pinned   no"), "{}", run.stdout);
    assert!(!run.stdout.contains(SENTINEL));
}
