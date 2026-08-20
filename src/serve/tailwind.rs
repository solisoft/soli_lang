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

/// Platform-specific Tailwind standalone CLI download URL, version-namespaced
/// cache filename, and SHA-256 checksum, for the requested major version.
///
/// Two majors are pinned because Tailwind v4 is a hard break from v3: v4 is
/// CSS-first (`@import "tailwindcss"`, `@theme`) and the v3 CLI cannot parse
/// it (and vice-versa). `compile_tailwind_css_once` detects the project's
/// major and asks for the matching binary, so a v3 app and a v4 app on the
/// same machine each get a correct compiler.
///
/// The hashes match each version's published `sha256sums.txt` (verified
/// out-of-band when pinned). To bump a version: fetch
/// `https://github.com/tailwindlabs/tailwindcss/releases/download/<vTAG>/sha256sums.txt`,
/// then update the URL version and the matching checksum here in lock-step.
fn tailwind_binary_info(major: u8) -> Option<BinaryAsset> {
    // v3.4.17 (legacy, config-file based)
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let v3 = BinaryAsset {
        url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.4.17/tailwindcss-macos-arm64",
        name: "tailwindcss-macos-arm64",
        sha256: "a1d0c7985759accca0bf12e51ac1dcbf0f6cf2fffb62e6e0f62d091c477a10a3",
    };
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    let v3 = BinaryAsset {
        url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.4.17/tailwindcss-macos-x64",
        name: "tailwindcss-macos-x64",
        sha256: "6cbdad74be776c087ffa5e9a057512c54898f9fe8828d3362212dfe32fc933a3",
    };
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let v3 = BinaryAsset {
        url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.4.17/tailwindcss-linux-x64",
        name: "tailwindcss-linux-x64",
        sha256: "7d24f7fa191d2193b78cd5f5a42a6093e14409521908529f42d80b11fde1f1d4",
    };
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    let v3 = BinaryAsset {
        url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v3.4.17/tailwindcss-linux-arm64",
        name: "tailwindcss-linux-arm64",
        sha256: "69b1378b8133192d7d2feb12a116fa12d035594f58db3eff215879e4ad8cf39b",
    };

    // v4.3.1 (CSS-first). Cache names are version-namespaced (`-v4-`) so the
    // v4 binary never collides with an already-cached v3 binary of the same
    // upstream filename.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let v4 = BinaryAsset {
        url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.1/tailwindcss-macos-arm64",
        name: "tailwindcss-v4-macos-arm64",
        sha256: "a27c43626185953ee19bdace1939c7601e55da654e0b2fc4461e3e29957aa739",
    };
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    let v4 = BinaryAsset {
        url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.1/tailwindcss-macos-x64",
        name: "tailwindcss-v4-macos-x64",
        sha256: "e9e830ceb3e70b7e0775a3dd79eee8ec82c6b31270f08f2fa2857d0077045ac3",
    };
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let v4 = BinaryAsset {
        url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.1/tailwindcss-linux-x64",
        name: "tailwindcss-v4-linux-x64",
        sha256: "2526d063ba03b71f9a3ea7d5cee14f0aec147f117f222d5adc97b1d736d45999",
    };
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    let v4 = BinaryAsset {
        url: "https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.1/tailwindcss-linux-arm64",
        name: "tailwindcss-v4-linux-arm64",
        sha256: "3d662377a86d71c43b549dc06b90db4586b4acd412bf827a3268e951661e5adf",
    };

    #[cfg(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    ))]
    {
        Some(if major >= 4 { v4 } else { v3 })
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
        let _ = major;
        None
    }
}

/// The version this build pins for a given major. Used when the project
/// doesn't name one of its own.
fn pinned_version(major: u8) -> &'static str {
    if major >= 4 {
        "4.3.1"
    } else {
        "3.4.17"
    }
}

/// This platform's slug in the upstream asset name (`tailwindcss-<slug>`).
/// `None` on platforms upstream doesn't publish a standalone binary for.
fn platform_slug() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Some("macos-arm64");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Some("macos-x64");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Some("linux-x64");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Some("linux-arm64");
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    None
}

/// A Tailwind binary to obtain: where to fetch it, what to call it in the
/// cache, and what to check it against.
struct ResolvedAsset {
    url: String,
    /// Cache filename. Version-qualified for project-chosen versions so two
    /// projects on different Tailwinds don't fight over one cached file.
    cache_name: String,
    /// The hash this build pins. `None` for a project-chosen version — there
    /// is no constant to pin it to, so it is checked against the release's
    /// own `sha256sums.txt` (or an explicitly configured hash) instead.
    sha256: Option<String>,
    version: String,
    /// The asset's name in the upstream release (and in its
    /// `sha256sums.txt`), which carries no version — unlike `cache_name`.
    upstream_name: String,
    /// True when the version came from the project rather than from us.
    configured: bool,
}

/// Accept only what can safely become a URL path segment and a filename:
/// one to four dot-separated runs of digits (`4`, `4.1`, `4.1.5`, `3.4.17`).
fn is_valid_version(version: &str) -> bool {
    let mut segments = 0;
    for segment in version.split('.') {
        segments += 1;
        if segments > 4 || segment.is_empty() || !segment.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    segments > 0
}

fn is_valid_sha256(hex: &str) -> bool {
    hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// Read a key from the `[assets]` table of the project's `soli.toml`.
fn assets_config_value(folder: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(folder.join("soli.toml")).ok()?;
    let parsed: toml::Value = text.parse().ok()?;
    let value = parsed.get("assets")?.get(key)?.as_str()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// The environment always wins over `soli.toml` — CI and one-off upgrade
/// checks shouldn't require editing a checked-in file.
fn configured_value(folder: &Path, env_key: &str, toml_key: &str) -> Option<String> {
    if let Ok(from_env) = std::env::var(env_key) {
        let trimmed = from_env.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    assets_config_value(folder, toml_key)
}

/// Decide which binary to use for `major`, honouring a project-chosen
/// version from `[assets] tailwind_version` in `soli.toml` (or
/// `SOLI_TAILWIND_VERSION`). Prints and ignores an unusable version rather
/// than failing the build over it.
fn resolve_asset(folder: &Path, major: u8) -> Option<ResolvedAsset> {
    if let Some(version) = configured_value(folder, "SOLI_TAILWIND_VERSION", "tailwind_version") {
        if !is_valid_version(&version) {
            eprintln!(
                "   ✗ Ignoring Tailwind version '{}': expected a numeric version like 4.1.5",
                version
            );
        } else if let Some(slug) = platform_slug() {
            let configured_hash =
                configured_value(folder, "SOLI_TAILWIND_SHA256", "tailwind_sha256").filter(|hex| {
                    let ok = is_valid_sha256(hex);
                    if !ok {
                        eprintln!("   ✗ Ignoring tailwind_sha256: expected 64 hex characters");
                    }
                    ok
                });
            return Some(ResolvedAsset {
                url: format!(
                    "https://github.com/tailwindlabs/tailwindcss/releases/download/v{}/tailwindcss-{}",
                    version, slug
                ),
                cache_name: format!("tailwindcss-{}-{}", version, slug),
                sha256: configured_hash,
                version,
                upstream_name: format!("tailwindcss-{}", slug),
                configured: true,
            });
        }
    }

    let asset = tailwind_binary_info(major)?;
    Some(ResolvedAsset {
        url: asset.url.to_string(),
        cache_name: asset.name.to_string(),
        sha256: Some(asset.sha256.to_string()),
        version: pinned_version(major).to_string(),
        upstream_name: format!("tailwindcss-{}", platform_slug().unwrap_or_default()),
        configured: false,
    })
}

/// The SHA-256 for `asset_name` from a release's own `sha256sums.txt`.
///
/// This is what a project-chosen version is verified against when it hasn't
/// declared `tailwind_sha256`. It is weaker than the pinned constants: it
/// proves the bytes we received are the bytes the release lists, not that
/// the release itself is what it was when someone vetted it. Declaring
/// `tailwind_sha256` restores the stronger guarantee.
fn fetch_release_sha256(version: &str, asset_name: &str) -> Option<String> {
    let url = format!(
        "https://github.com/tailwindlabs/tailwindcss/releases/download/v{}/sha256sums.txt",
        version
    );
    let output = std::process::Command::new("curl")
        .args(["-sL", "--fail"])
        .arg(&url)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let listing = String::from_utf8_lossy(&output.stdout);
    for line in listing.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        let hash = fields[0];
        let name = fields[fields.len() - 1]
            .trim_start_matches("./")
            .trim_start_matches('*');
        if name == asset_name && is_valid_sha256(hash) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
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
fn download_tailwind_binary(dest: &Path, asset: &ResolvedAsset) -> bool {
    println!(
        "   Downloading Tailwind CSS {} standalone CLI...",
        asset.version
    );

    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("   ✗ Failed to create directory: {}", e);
            return false;
        }
    }

    // Stage download to `<dest>.partial` so a failed verification (or a
    // crashed download mid-flight) can never leave an executable cached
    // file at `dest`.
    // Append rather than `with_extension`, which would truncate at the last
    // dot of a version-qualified name — `tailwindcss-4.3.1-macos-arm64` and
    // `…-4.3.5-…` would both stage to `tailwindcss-4.3.partial` and collide.
    let partial = {
        let mut name = dest.file_name().unwrap_or_default().to_os_string();
        name.push(".partial");
        dest.with_file_name(name)
    };

    let curl_result = std::process::Command::new("curl")
        .args(["-sL", "--fail", "-o"])
        .arg(&partial)
        .arg(&asset.url)
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

    // A version we ship has a pinned hash. A project-chosen one is checked
    // against the release's own `sha256sums.txt`, unless the project declared
    // `tailwind_sha256` — and if neither can be had, nothing is installed.
    let expected = match asset.sha256.clone() {
        Some(hex) => hex,
        None => match fetch_release_sha256(&asset.version, &asset.upstream_name) {
            Some(hex) => {
                println!(
                    "   Verifying against the v{} release's sha256sums.txt \
                     (declare tailwind_sha256 in soli.toml to pin it here instead)",
                    asset.version
                );
                hex
            }
            None => {
                let _ = std::fs::remove_file(&partial);
                eprintln!(
                    "   ✗ No checksum available for Tailwind {}: the release's sha256sums.txt \
                     could not be read.\n     Refusing to install an unverified binary. Check the \
                     version exists, or set tailwind_sha256 in soli.toml.",
                    asset.version
                );
                return false;
            }
        },
    };

    if let Err(e) = verify_sha256(&partial, &expected) {
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

    if asset.configured {
        // The project picked this version, so show the hash it resolved to:
        // pasting it into `[assets] tailwind_sha256` upgrades the check from
        // "matches the release listing" to "matches the bytes I vetted".
        match sha256_hex_of_file(dest) {
            Ok(hex) => println!(
                "   ✓ Tailwind CSS {} downloaded and verified\n     tailwind_sha256 = \"{}\"",
                asset.version, hex
            ),
            Err(_) => println!(
                "   ✓ Tailwind CSS {} downloaded and verified",
                asset.version
            ),
        }
    } else {
        println!(
            "   ✓ Tailwind CSS CLI downloaded and verified ({})",
            asset.cache_name
        );
    }
    true
}

/// Find or download the Tailwind CSS binary for the given major version.
/// Priority: local node_modules > cached standalone > download standalone.
///
/// The local `node_modules/.bin/tailwindcss` still wins when it exists:
/// `soli new` no longer creates one, but a project that keeps its own npm
/// toolchain has deliberately installed that CLI, and its version is the one
/// that project means to use. A project with no `node_modules` gets the
/// standalone binary for `[assets] tailwind_version`, or the pinned default.
fn find_tailwind_binary(folder: &Path, major: u8) -> Option<PathBuf> {
    // 1. Check local node_modules/.bin/tailwindcss
    let local_bin = folder.join("node_modules/.bin/tailwindcss");
    if local_bin.exists() {
        return Some(local_bin);
    }

    // 2. Check cached standalone binary in ~/.soli/bin/
    let asset = resolve_asset(folder, major)?;
    let cache_dir = tailwind_cache_dir()?;
    let cached = cache_dir.join(&asset.cache_name);
    if cached.exists() {
        return Some(cached);
    }

    // 3. Download standalone binary (verifies SHA-256 before installing).
    if download_tailwind_binary(&cached, &asset) {
        return Some(cached);
    }

    None
}

/// Tailwind directives that mark a CSS file as a Tailwind entrypoint. Covers
/// both v3 (`@tailwind`) and v4 (`@import "tailwindcss"`, `@theme`, …) so we
/// recognise a Tailwind project without depending on a `tailwind.config.js`
/// (which v4 projects don't have).
fn css_has_tailwind_directive(css: &str) -> bool {
    css.contains("@tailwind")
        || css.contains("tailwindcss")
        || css.contains("@theme")
        || css.contains("@apply")
        || css.contains("@plugin")
        || css.contains("@utility")
}

/// v4-only directives — their presence forces the v4 toolchain regardless of
/// what `package.json` says.
fn css_is_v4(css: &str) -> bool {
    css.contains("@import \"tailwindcss\"")
        || css.contains("@import 'tailwindcss'")
        || css.contains("@theme")
        || css.contains("@plugin")
        || css.contains("@utility")
}

/// Detect the Tailwind major version a project targets. Order of evidence:
/// v4-only CSS directives (authoritative) > the `tailwindcss` /
/// `@tailwindcss/cli` semver in `package.json` > legacy `tailwind.config.js`
/// (implies v3) > default. Tailwind v4 is the modern default when nothing
/// else is conclusive.
fn detect_tailwind_major(folder: &Path, css_sources: &[String]) -> u8 {
    if css_sources.iter().any(|css| css_is_v4(css)) {
        return 4;
    }

    if let Ok(pkg) = std::fs::read_to_string(folder.join("package.json")) {
        // Match the first major digit after a `"tailwindcss"` /
        // `"@tailwindcss/cli"` dependency key, tolerating `^`, `~`, `>=`, etc.
        for key in ["@tailwindcss/cli", "tailwindcss"] {
            if let Some(major) = semver_major_after_key(&pkg, key) {
                return major;
            }
        }
    }

    // A bare `@tailwind base;` entry with no v4 markers and no package.json
    // hint is the classic v3 shape.
    if css_sources.iter().any(|css| css.contains("@tailwind ")) {
        return 3;
    }
    if folder.join("tailwind.config.js").exists() || folder.join("tailwind.config.ts").exists() {
        return 3;
    }

    4
}

/// Pull the major version out of a `package.json` dependency value, e.g.
/// `"@tailwindcss/cli": "^4.3.1"` -> 4. Returns None if the key is absent or
/// the value has no leading numeric major.
fn semver_major_after_key(pkg_json: &str, key: &str) -> Option<u8> {
    let needle = format!("\"{}\"", key);
    let after_key = &pkg_json[pkg_json.find(&needle)? + needle.len()..];
    // Skip past the `:` and opening quote of the version string.
    let colon = after_key.find(':')?;
    let after_colon = &after_key[colon + 1..];
    let quote = after_colon.find('"')?;
    let version = &after_colon[quote + 1..];
    // First run of digits in the version specifier is the major.
    let digits: String = version
        .trim_start_matches(['^', '~', '>', '=', '<', ' ', 'v'])
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Compile all CSS files in app/assets/css/ to public/css/.
/// Each `app/assets/css/foo.css` is compiled to `public/css/foo.css`.
/// Returns true if all compilations were successful.
pub(crate) fn compile_tailwind_css_once(folder: &Path) -> bool {
    // The compiler runs with `current_dir(folder)` so Tailwind's `@source`
    // globs resolve against the project. That makes a *relative* `folder`
    // resolve twice — `soli new blog` would ask for `./blog/app/...` from
    // inside `blog/`. Absolutise once, up front, and use it everywhere.
    let root = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    let folder: &Path = &root;

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

    // Read the entry CSS so we can (a) confirm this is actually a Tailwind
    // project and (b) detect v3 vs v4 from its directives. Tailwind v4 is
    // CSS-first and ships no `tailwind.config.js`, so we must NOT gate on that
    // file — gating on a Tailwind directive in the CSS recognises both v3 and
    // v4 without dragging the standalone download into plain-CSS projects.
    let css_sources: Vec<String> = css_files
        .iter()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .collect();

    if !css_sources
        .iter()
        .any(|css| css_has_tailwind_directive(css))
    {
        return false;
    }

    let major = detect_tailwind_major(folder, &css_sources);

    let output_dir = folder.join("public/css");
    let _ = std::fs::create_dir_all(&output_dir);

    println!("Compiling Tailwind CSS (v{} toolchain)...", major);

    let tailwind_bin = match find_tailwind_binary(folder, major) {
        Some(bin) => bin,
        None => {
            // Don't suggest npm: a scaffolded project has no package.json.
            // Either the download failed, or this platform has no pinned
            // standalone binary (see `platform_slug`).
            eprintln!(
                "   ✗ No Tailwind CSS compiler available.\n     \
                 Check your internet connection, or pin a version with \
                 [assets] tailwind_version in soli.toml.\n     \
                 On a platform with no standalone build, install the CLI \
                 yourself at node_modules/.bin/tailwindcss."
            );
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

    #[test]
    fn both_majors_have_a_pinned_asset_on_this_platform() {
        // On any supported platform, both v3 and v4 must resolve (so a
        // detected major never silently falls through to "no binary").
        if tailwind_binary_info(3).is_some() {
            assert!(tailwind_binary_info(4).is_some());
            // v4 cache name is version-namespaced to avoid colliding with v3.
            assert!(tailwind_binary_info(4).unwrap().name.contains("v4"));
            assert!(tailwind_binary_info(4).unwrap().url.contains("v4.3.1"));
        }
    }

    #[test]
    fn semver_major_parses_common_specifiers() {
        let pkg = r#"{ "devDependencies": { "@tailwindcss/cli": "^4.3.1" } }"#;
        assert_eq!(semver_major_after_key(pkg, "@tailwindcss/cli"), Some(4));

        let pkg3 = r#"{ "devDependencies": { "tailwindcss": "~3.4.17" } }"#;
        assert_eq!(semver_major_after_key(pkg3, "tailwindcss"), Some(3));

        let pkg_plain = r#"{ "dependencies": { "tailwindcss": "4.0.0" } }"#;
        assert_eq!(semver_major_after_key(pkg_plain, "tailwindcss"), Some(4));

        let absent = r#"{ "dependencies": { "react": "18" } }"#;
        assert_eq!(semver_major_after_key(absent, "tailwindcss"), None);
    }

    #[test]
    fn v4_css_directives_force_v4_over_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        // package.json says v3, but the CSS is unmistakably v4.
        write(
            &tmp.path().join("package.json"),
            br#"{ "devDependencies": { "tailwindcss": "^3.4.0" } }"#,
        );
        let css = vec!["@import \"tailwindcss\";\n@theme { --color-x: #fff; }".to_string()];
        assert_eq!(detect_tailwind_major(tmp.path(), &css), 4);
    }

    #[test]
    fn detects_v3_from_legacy_css_and_config() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join("tailwind.config.js"),
            b"module.exports = {}",
        );
        let css = vec!["@tailwind base;\n@tailwind utilities;".to_string()];
        assert_eq!(detect_tailwind_major(tmp.path(), &css), 3);
    }

    #[test]
    fn defaults_to_v4_when_no_evidence() {
        let tmp = tempfile::tempdir().unwrap();
        let css = vec!["@apply text-sm;".to_string()];
        assert_eq!(detect_tailwind_major(tmp.path(), &css), 4);
    }

    #[test]
    fn plain_css_is_not_a_tailwind_project() {
        assert!(!css_has_tailwind_directive("body { color: red; }"));
        assert!(css_has_tailwind_directive("@import \"tailwindcss\";"));
        assert!(css_has_tailwind_directive("@tailwind base;"));
    }
}
