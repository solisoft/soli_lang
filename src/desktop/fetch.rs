//! Fetching the database binary for a build target.
//!
//! Mirrors how the soli runtime is obtained for a cross-target standalone
//! build: download a published release tarball, verify it against its `.sha256`
//! sibling, cache it, and reuse. The same discipline applies because the same
//! thing is at stake — these bytes are embedded in an artifact and later
//! executed on a user's machine.

use std::path::PathBuf;
use std::time::Duration;

/// Repository the database publishes releases from.
const DB_RELEASE_REPO: &str = "solisoft/solidb";

/// Base URL for database releases. `SOLI_DB_RELEASE_BASE_URL` overrides it, so
/// tests and mirrors can point elsewhere — the same escape hatch the runtime
/// download has.
fn release_base_url() -> String {
    std::env::var("SOLI_DB_RELEASE_BASE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .unwrap_or_else(|| format!("https://github.com/{}/releases/download", DB_RELEASE_REPO))
}

/// Where downloaded database binaries are cached.
///
/// Deliberately defined here rather than borrowed from the CLI: `src/cli` is
/// the binary crate, and the library must not depend on it.
fn cache_root() -> Result<PathBuf, String> {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("soli").join("solidb"));
        }
    }
    let home =
        std::env::var("HOME").map_err(|_| "cannot determine cache dir (no HOME)".to_string())?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("soli")
        .join("solidb"))
}

/// Download (or reuse a cached) database binary for `target` at `version`.
///
/// The cache is keyed by version and target, so switching either fetches
/// afresh rather than silently embedding the wrong build.
pub fn database_binary(target: &str, version: &str) -> Result<Vec<u8>, String> {
    let cache_dir = cache_root()?.join(format!("v{}", version));
    let cached = cache_dir.join(format!("solidb-{}", target));

    if let Ok(bytes) = std::fs::read(&cached) {
        if !bytes.is_empty() {
            println!("  Using cached database binary ({}, v{})", target, version);
            return Ok(bytes);
        }
    }

    let bytes = download(target, version)?;

    // Cache via a temp file and rename, so a concurrent build never observes a
    // half-written binary and embeds it.
    if std::fs::create_dir_all(&cache_dir).is_ok() {
        let tmp = cache_dir.join(format!("solidb-{}.partial", target));
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &cached);
        }
    }
    Ok(bytes)
}

fn download(target: &str, version: &str) -> Result<Vec<u8>, String> {
    let tarball = format!("solidb-{}.tar.gz", target);
    let url = format!("{}/v{}/{}", release_base_url(), version, tarball);

    println!("  Downloading database {} v{} ...", target, version);

    let client = reqwest::blocking::Client::builder()
        .user_agent("soli-lang-cli")
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("failed to create HTTP client: {}", e))?;

    let mut response = client
        .get(&url)
        .send()
        .map_err(|e| format!("failed to download {}: {}", url, e))
        .and_then(|resp| {
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                Err(format!(
                    "no published solidb {} artifact for v{} at {} — pass --solidb <path> \
                     to embed a locally built binary, or point SOLI_DB_RELEASE_BASE_URL \
                     at a mirror",
                    target, version, url
                ))
            } else {
                resp.error_for_status()
                    .map_err(|e| format!("download error: {}", e))
            }
        })?;

    let temp_dir = tempfile::Builder::new()
        .prefix("soli-solidb-")
        .tempdir()
        .map_err(|e| format!("failed to create temp directory: {}", e))?;
    let tarball_path = temp_dir.path().join(&tarball);
    let mut file = std::fs::File::create(&tarball_path)
        .map_err(|e| format!("failed to create temp file: {}", e))?;
    response
        .copy_to(&mut file)
        .map_err(|e| format!("failed to write download: {}", e))?;
    drop(file);

    verify_checksum(&client, &url, &tarball_path)?;
    extract_binary(&tarball_path, temp_dir.path(), target)
}

/// Verify the tarball against its published `.sha256`.
///
/// A mismatch is fatal — these bytes get executed on a user's machine. A
/// *missing* checksum only warns, matching the runtime download: older releases
/// predate checksum publishing, and refusing them outright would break builds
/// against them.
fn verify_checksum(
    client: &reqwest::blocking::Client,
    url: &str,
    tarball_path: &std::path::Path,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let sha_url = format!("{}.sha256", url);
    let published = match client.get(&sha_url).send() {
        Ok(resp) if resp.status().is_success() => resp
            .text()
            .map_err(|e| format!("reading .sha256: {}", e))?
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_lowercase(),
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            println!(
                "  \x1b[33m! no .sha256 published for this database release\x1b[0m — \
                 embedding without verification"
            );
            return Ok(());
        }
        Ok(resp) => return Err(format!("fetching .sha256: HTTP {}", resp.status())),
        Err(e) => return Err(format!("fetching .sha256: {}", e)),
    };

    let bytes = std::fs::read(tarball_path).map_err(|e| format!("re-reading tarball: {}", e))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();

    if actual != published {
        return Err(format!(
            "database tarball failed checksum verification: published {}, got {} — \
             refusing to embed it",
            published, actual
        ));
    }
    Ok(())
}

fn extract_binary(
    tarball_path: &std::path::Path,
    into: &std::path::Path,
    target: &str,
) -> Result<Vec<u8>, String> {
    let tf =
        std::fs::File::open(tarball_path).map_err(|e| format!("failed to open tarball: {}", e))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tf));
    archive
        .unpack(into)
        .map_err(|e| format!("failed to extract tarball: {}", e))?;

    // Windows tarballs carry solidb.exe; everything else carries solidb. The
    // tarball also ships dump/restore tools, which are not embedded.
    let name = if target.starts_with("windows-") {
        "solidb.exe"
    } else {
        "solidb"
    };
    let path: PathBuf = into.join(name);
    std::fs::read(&path).map_err(|e| {
        format!(
            "tarball did not contain a '{}' binary ({}): {}",
            name,
            path.display(),
            e
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_overridable() {
        std::env::set_var("SOLI_DB_RELEASE_BASE_URL", "http://127.0.0.1:9/mirror/");
        // The trailing slash is trimmed so URL joining cannot double it.
        assert_eq!(release_base_url(), "http://127.0.0.1:9/mirror");
        std::env::remove_var("SOLI_DB_RELEASE_BASE_URL");
        assert!(release_base_url().contains("solisoft/solidb"));
    }

    #[test]
    fn windows_tarballs_hold_an_exe() {
        let dir = tempfile::tempdir().expect("scratch");
        // A tarball whose only member is `solidb` must not satisfy a Windows
        // build: silently embedding the wrong file would produce an artifact
        // that fails at launch on the user's machine.
        std::fs::write(dir.path().join("solidb"), b"unix-binary").expect("write");
        let err = extract_binary(
            &dir.path().join("missing.tar.gz"),
            dir.path(),
            "windows-amd64",
        )
        .expect_err("must fail");
        assert!(err.contains("failed to open tarball"), "got: {}", err);
    }
}
