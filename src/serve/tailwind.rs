//! Tailwind CSS Integration
//!
//! This module handles Tailwind CSS compilation for development.
//! Uses the standalone Tailwind CSS CLI binary, downloading it automatically if needed.

use std::path::{Path, PathBuf};

/// One platform's standalone Tailwind binary: download URL, on-disk
/// filename, and the pinned SHA-256 checksum from the upstream release's
/// `sha256sums.txt`. SEC-081: every download is hash-verified before the
/// binary is renamed into place and made executable, so a compromised
/// release asset, CDN problem, or in-flight tampering can't slip an
/// arbitrary executable past `cargo run` / `soli serve --dev`.
struct BinaryAsset {
    url: &'static str,
    name: &'static str,
    sha256: &'static str,
}

/// Platform-specific Tailwind standalone CLI download URL, filename, and
/// SHA-256 checksum. The hashes match v3.4.17's published `sha256sums.txt`
/// (verified out-of-band when the version was pinned). To bump Tailwind:
/// fetch `https://github.com/tailwindlabs/tailwindcss/releases/download/<vTAG>/sha256sums.txt`,
/// update both the URL version and the matching checksum here in lock-step.
fn tailwind_binary_info() -> Option<BinaryAsset> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some(BinaryAsset {
            url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.4.17/tailwindcss-macos-arm64",
            name: "tailwindcss-macos-arm64",
            sha256: "a1d0c7985759accca0bf12e51ac1dcbf0f6cf2fffb62e6e0f62d091c477a10a3",
        })
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Some(BinaryAsset {
            url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.4.17/tailwindcss-macos-x64",
            name: "tailwindcss-macos-x64",
            sha256: "6cbdad74be776c087ffa5e9a057512c54898f9fe8828d3362212dfe32fc933a3",
        })
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some(BinaryAsset {
            url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.4.17/tailwindcss-linux-x64",
            name: "tailwindcss-linux-x64",
            sha256: "7d24f7fa191d2193b78cd5f5a42a6093e14409521908529f42d80b11fde1f1d4",
        })
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Some(BinaryAsset {
            url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.4.17/tailwindcss-linux-arm64",
            name: "tailwindcss-linux-arm64",
            sha256: "69b1378b8133192d7d2feb12a116fa12d035594f58db3eff215879e4ad8cf39b",
        })
    }
    // No pinned hash for other platforms — refuse to download an
    // unverified binary rather than fall back to the previous trust-on-
    // first-use pattern.
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        None
    }
}

/// Get the path where the standalone Tailwind binary is cached.
/// Stored in ~/.soli/bin/
fn tailwind_cache_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".soli").join("bin"))
}

/// Compute the SHA-256 of `path` as a lowercase hex string.
fn sha256_hex_of_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{:02x}", b);
    }
    Ok(out)
}

/// Verify a freshly-downloaded file against the pinned SHA-256. Returns
/// `Ok(())` on match, `Err(message)` on read failure or mismatch. The
/// caller is responsible for cleaning up the file on `Err`.
fn verify_sha256(path: &Path, expected_hex: &str) -> Result<(), String> {
    let actual = sha256_hex_of_file(path)
        .map_err(|e| format!("failed to read {} for hashing: {}", path.display(), e))?;
    if !actual.eq_ignore_ascii_case(expected_hex) {
        return Err(format!(
            "SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            expected_hex,
            actual
        ));
    }
    Ok(())
}

/// Download the standalone Tailwind CSS CLI binary, verify its checksum,
/// then atomically rename into place and mark it executable. SEC-081:
/// the binary is written to a `.partial` sibling first; only after
/// `verify_sha256` returns Ok does it get renamed to `dest`. Mismatches
/// fail closed with the partial file deleted, so a hostile or corrupted
/// download never leaves an executable cached file behind.
fn download_tailwind_binary(dest: &Path) -> bool {
    let asset = match tailwind_binary_info() {
        Some(a) => a,
        None => {
            eprintln!("   ✗ No standalone Tailwind binary available for this platform");
            return false;
        }
    };

    println!("   Downloading Tailwind CSS standalone CLI...");

    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("   ✗ Failed to create directory: {}", e);
            return false;
        }
    }

    // Stage download to `<dest>.partial` so a failed verification (or a
    // crashed download mid-flight) can never leave an executable cached
    // file at `dest`.
    let partial = dest.with_extension("partial");

    let curl_result = std::process::Command::new("curl")
        .args(["-sL", "--fail", "-o"])
        .arg(&partial)
        .arg(asset.url)
        .output();

    match curl_result {
        Ok(result) if result.status.success() => {}
        Ok(result) => {
            let _ = std::fs::remove_file(&partial);
            let stderr = String::from_utf8_lossy(&result.stderr);
            eprintln!("   ✗ Download failed: {}", stderr);
            return false;
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            eprintln!("   ✗ Failed to run curl: {}", e);
            return false;
        }
    }

    if let Err(e) = verify_sha256(&partial, asset.sha256) {
        let _ = std::fs::remove_file(&partial);
        eprintln!(
            "   ✗ {}\n     Refusing to install an unverified Tailwind binary. \
             Delete {} (if anything is left), check your network path, and re-run.",
            e,
            partial.display()
        );
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755)) {
            let _ = std::fs::remove_file(&partial);
            eprintln!("   ✗ Failed to chmod cached binary: {}", e);
            return false;
        }
    }

    if let Err(e) = std::fs::rename(&partial, dest) {
        let _ = std::fs::remove_file(&partial);
        eprintln!(
            "   ✗ Failed to install verified binary at {}: {}",
            dest.display(),
            e
        );
        return false;
    }

    println!(
        "   ✓ Tailwind CSS CLI downloaded and verified ({})",
        asset.name
    );
    true
}

/// Find or download the Tailwind CSS binary.
/// Priority: local node_modules > cached standalone > download standalone
fn find_tailwind_binary(folder: &Path) -> Option<PathBuf> {
    // 1. Check local node_modules/.bin/tailwindcss
    let local_bin = folder.join("node_modules/.bin/tailwindcss");
    if local_bin.exists() {
        return Some(local_bin);
    }

    // 2. Check cached standalone binary in ~/.soli/bin/
    let asset = tailwind_binary_info()?;
    let cache_dir = tailwind_cache_dir()?;
    let cached = cache_dir.join(asset.name);
    if cached.exists() {
        return Some(cached);
    }

    // 3. Download standalone binary (verifies SHA-256 before installing).
    if download_tailwind_binary(&cached) {
        return Some(cached);
    }

    None
}

/// Compile all CSS files in app/assets/css/ to public/css/.
/// Each `app/assets/css/foo.css` is compiled to `public/css/foo.css`.
/// Returns true if all compilations were successful.
pub(crate) fn compile_tailwind_css_once(folder: &Path) -> bool {
    let tailwind_config = folder.join("tailwind.config.js");
    if !tailwind_config.exists() {
        return false;
    }

    let assets_dir = folder.join("app/assets/css");
    if !assets_dir.exists() {
        return false;
    }

    // Collect all .css files in app/assets/css/
    let css_files: Vec<_> = match std::fs::read_dir(&assets_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "css")
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return false,
    };

    if css_files.is_empty() {
        return false;
    }

    let output_dir = folder.join("public/css");
    let _ = std::fs::create_dir_all(&output_dir);

    println!("Compiling Tailwind CSS...");

    let tailwind_bin = match find_tailwind_binary(folder) {
        Some(bin) => bin,
        None => {
            eprintln!("   ✗ Tailwind CSS CLI not found. Run 'npm install' in your project or check your internet connection.");
            return false;
        }
    };

    let mut all_ok = true;
    for entry in &css_files {
        let input = entry.path();
        let filename = input.file_name().unwrap();
        let output = output_dir.join(filename);

        let result = std::process::Command::new(&tailwind_bin)
            .arg("-i")
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .current_dir(folder)
            .output();

        match result {
            Ok(r) if r.status.success() => {
                println!("   ✓ {}", filename.to_string_lossy());
            }
            Ok(r) => {
                let stderr = String::from_utf8_lossy(&r.stderr);
                eprintln!("   ✗ {} failed: {}", filename.to_string_lossy(), stderr);
                all_ok = false;
            }
            Err(e) => {
                eprintln!("   ✗ {} failed: {}", filename.to_string_lossy(), e);
                all_ok = false;
            }
        }
    }

    all_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256 of "abc" — RFC 6234 test vector.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("abc.bin");
        write(&p, b"abc");
        let hex = sha256_hex_of_file(&p).unwrap();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verify_sha256_passes_on_match() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("abc.bin");
        write(&p, b"abc");
        verify_sha256(
            &p,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .expect("matching hash must verify");
    }

    #[test]
    fn verify_sha256_passes_uppercase_expected() {
        // Pinned hashes are commonly written lowercase, but tolerate
        // accidental uppercase entries too — `eq_ignore_ascii_case`.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("abc.bin");
        write(&p, b"abc");
        verify_sha256(
            &p,
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
        )
        .expect("case-insensitive expected hash must verify");
    }

    #[test]
    fn verify_sha256_fails_on_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("abc.bin");
        write(&p, b"abc");
        let err = verify_sha256(&p, &"0".repeat(64)).unwrap_err();
        assert!(err.contains("SHA-256 mismatch"), "{}", err);
        assert!(err.contains(&"0".repeat(64)), "{}", err);
    }

    #[test]
    fn verify_sha256_fails_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("never.bin");
        let err = verify_sha256(&p, &"0".repeat(64)).unwrap_err();
        assert!(err.contains("failed to read"), "{}", err);
    }
}
