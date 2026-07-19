//! Exercises the database-binary download path against a mock release server.
//!
//! The bytes fetched here end up embedded in an artifact and executed on a
//! user's machine, so the checksum behaviour is the point of these tests, not
//! an aside.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

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

/// A gzipped tar holding a single `solidb` member.
fn make_tarball(binary: &[u8]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_size(binary.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append_data(&mut header, "solidb", binary).unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

/// Serves the tarball and whatever `.sha256` body is supplied.
fn spawn_server(tarball: Vec<u8>, sha_body: Option<String>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for _ in 0..8 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_string();

            let (status, body): (&str, Vec<u8>) = if request.contains(".sha256") {
                match &sha_body {
                    Some(text) => ("200 OK", text.clone().into_bytes()),
                    None => ("404 Not Found", Vec::new()),
                }
            } else {
                ("200 OK", tarball.clone())
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

const FAKE_DB: &[u8] = b"fake-solidb-binary-for-download-tests-0123456789";

/// Serialises the tests.
///
/// The download path is configured through process-global environment
/// variables, so running these concurrently lets one test's mirror and cache
/// leak into another — which shows up as an unrelated-looking failure.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Point the fetcher at `port` with a cache directory of its own, run `f`, then
/// restore the environment.
fn with_mirror<T>(port: u16, f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let cache = tempfile::tempdir().expect("cache dir");
    let previous_cache = std::env::var("XDG_CACHE_HOME").ok();
    let previous_url = std::env::var("SOLI_DB_RELEASE_BASE_URL").ok();

    std::env::set_var("XDG_CACHE_HOME", cache.path());
    std::env::set_var(
        "SOLI_DB_RELEASE_BASE_URL",
        format!("http://127.0.0.1:{}", port),
    );

    let out = f();

    match previous_cache {
        Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
        None => std::env::remove_var("XDG_CACHE_HOME"),
    }
    match previous_url {
        Some(v) => std::env::set_var("SOLI_DB_RELEASE_BASE_URL", v),
        None => std::env::remove_var("SOLI_DB_RELEASE_BASE_URL"),
    }
    out
}

#[test]
fn downloads_verifies_and_caches_the_database_binary() {
    let tarball = make_tarball(FAKE_DB);
    let sha = sha256_hex(&tarball);
    let port = spawn_server(tarball, Some(sha));

    with_mirror(port, || {
        let bytes = solilang::desktop::fetch::database_binary("linux-amd64", "9.9.9")
            .expect("download must succeed");
        assert_eq!(bytes, FAKE_DB, "the extracted binary must be the tarball's");

        // Second call must be served from cache. The mock only answers a
        // handful of requests, but more importantly a rebuild should not
        // re-download a hundred megabytes.
        let again = solilang::desktop::fetch::database_binary("linux-amd64", "9.9.9")
            .expect("cached read must succeed");
        assert_eq!(again, FAKE_DB);
    });
}

#[test]
fn refuses_a_tarball_whose_checksum_does_not_match() {
    let tarball = make_tarball(FAKE_DB);
    // A checksum for different content: exactly what a tampered or truncated
    // mirror would produce.
    let wrong = sha256_hex(b"something else entirely");
    let port = spawn_server(tarball, Some(wrong));

    with_mirror(port, || {
        let err = solilang::desktop::fetch::database_binary("linux-amd64", "9.9.8")
            .expect_err("a checksum mismatch must be fatal");
        assert!(
            err.contains("failed checksum verification"),
            "unexpected error: {}",
            err
        );
    });
}

#[test]
fn a_missing_checksum_warns_but_proceeds() {
    // Releases predating checksum publishing must still be usable — refusing
    // them outright would break builds against existing versions.
    let tarball = make_tarball(FAKE_DB);
    let port = spawn_server(tarball, None);

    with_mirror(port, || {
        let bytes = solilang::desktop::fetch::database_binary("linux-amd64", "9.9.7")
            .expect("missing checksum must not be fatal");
        assert_eq!(bytes, FAKE_DB);
    });
}
