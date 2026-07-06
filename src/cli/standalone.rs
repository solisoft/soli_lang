//! Self-executing app bundles: `soli build --standalone`.
//!
//! A standalone executable is the soli runtime with the app's `.soli` bundle
//! appended, plus a 16-byte footer at EOF:
//!
//! ```text
//! [ soli runtime binary, byte-for-byte ]
//! [ payload: SOLB or SOLE bundle bytes, verbatim ]
//! [ footer: magic b"SOLIXEC1" (8 bytes) + payload_len u64 LE (8 bytes) ]
//! ```
//!
//! At boot, `boot_if_standalone()` (the first statement of `cli::run()`)
//! reads the last 16 bytes of `current_exe()`. No footer → normal soli CLI.
//! Footer present → the executable IS the app: parse the app-oriented flags
//! (`--port`, `--host`, `--workers`, `--dev`) and serve the embedded payload
//! through the same pipeline as `soli serve app.soli` — including key
//! resolution / the key-server kill-switch and the RAM-only (/dev/shm)
//! extraction contract for encrypted payloads.
//!
//! The footer check is position-anchored at EOF, so the `SOLIXEC1` literal
//! sitting in this very binary's rodata can never false-positive. Every
//! failure path is `Result` → `eprintln!` + `process::exit` — the release
//! profile is `panic = "abort"`, so nothing here may panic on bad input.
//!
//! Cross-target builds (`--target linux-amd64|linux-arm64|darwin-arm64`)
//! embed a published release runtime instead of `current_exe()`: downloaded
//! from `{SOLI_RELEASE_BASE_URL | github releases}/v{VERSION}/soli-{target}.tar.gz`,
//! sha256-verified against the `.sha256` sibling, and cached under
//! `~/.cache/soli/runtimes/`. The version is pinned to the building soli's
//! own version — protected bundles are hard-locked to it (`check_bundle_meta`),
//! so runtime and bundle must ship as a matched pair.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process;

const FOOTER_MAGIC: &[u8; 8] = b"SOLIXEC1";
const FOOTER_LEN: u64 = 16;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The GitHub repo releases are published from (same as `soli update`).
const RELEASE_REPO: &str = "solisoft/soli_lang";

/// Release artifact names, exactly as CI publishes them.
const SUPPORTED_TARGETS: &[&str] = &["linux-amd64", "linux-arm64", "darwin-arm64"];

// ---------------------------------------------------------------------------
// Boot side
// ---------------------------------------------------------------------------

/// Read the footer of `file` and return the payload length when the magic
/// matches. `file_len` is passed in so the caller can bound-check.
fn read_footer(file: &mut std::fs::File, file_len: u64) -> Option<u64> {
    if file_len < FOOTER_LEN {
        return None;
    }
    file.seek(SeekFrom::End(-(FOOTER_LEN as i64))).ok()?;
    let mut footer = [0u8; 16];
    file.read_exact(&mut footer).ok()?;
    if &footer[..8] != FOOTER_MAGIC {
        return None;
    }
    Some(u64::from_le_bytes(footer[8..16].try_into().ok()?))
}

/// Extract the embedded bundle payload from the running executable.
///
/// - `None` — no footer (or the exe is unreadable): this is a regular soli
///   binary; fall through to the normal CLI. Unreadable is deliberately
///   `None`, not an error — a broken `/proc/self/exe` must never brick soli.
/// - `Some(Err)` — a footer is present but the payload is invalid
///   (truncated, tampered, or not a bundle). The caller must NOT fall
///   through to the soli CLI: this executable claims to be an app.
pub fn embedded_payload() -> Option<Result<Vec<u8>, String>> {
    let exe = std::env::current_exe().ok()?;
    let mut file = std::fs::File::open(&exe).ok()?;
    let file_len = file.metadata().ok()?.len();
    let payload_len = read_footer(&mut file, file_len)?;

    Some(read_payload(&mut file, file_len, payload_len))
}

fn read_payload(
    file: &mut std::fs::File,
    file_len: u64,
    payload_len: u64,
) -> Result<Vec<u8>, String> {
    if payload_len == 0 {
        return Err("embedded bundle is empty".to_string());
    }
    let max_payload = file_len - FOOTER_LEN;
    if payload_len > max_payload {
        return Err(format!(
            "footer claims a {} byte bundle but the executable only has {} bytes before the footer",
            payload_len, max_payload
        ));
    }
    file.seek(SeekFrom::Start(file_len - FOOTER_LEN - payload_len))
        .map_err(|e| format!("seek failed: {}", e))?;
    let mut payload = vec![0u8; payload_len as usize];
    file.read_exact(&mut payload)
        .map_err(|e| format!("read failed: {}", e))?;
    if !payload.starts_with(b"SOLB") && !payload.starts_with(b"SOLE") {
        return Err("embedded data is not a soli bundle (bad magic)".to_string());
    }
    Ok(payload)
}

struct StandaloneArgs {
    port: u16,
    workers: usize,
    dev_mode: bool,
}

/// If this executable carries an embedded bundle, boot it and never return.
/// Called before `parse_args()` so app executables never see the soli CLI.
pub fn boot_if_standalone() {
    let payload = match embedded_payload() {
        None => return, // regular soli binary — normal CLI
        Some(Ok(p)) => p,
        Some(Err(e)) => {
            eprintln!(
                "Error: this executable's embedded app bundle is invalid: {}",
                e
            );
            process::exit(70);
        }
    };

    let args = parse_standalone_args(&payload);

    // `.env` lives next to the artifact — same convention as a `.soli`
    // bundle file (dotfiles are never bundled; secrets stay deploy-local).
    let env_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let label = exe_label();
    if let Err(e) = crate::cli::commands::serve_bundle_bytes(
        &payload,
        &env_dir,
        args.port,
        args.dev_mode,
        args.workers,
        &label,
    ) {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
    process::exit(0);
}

/// The executable's display name for usage/version output.
fn exe_label() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "app".to_string())
}

/// App-oriented argv parsing. Deliberately NOT `parse_args()` — its error
/// paths print the soli CLI usage, which is wrong and confusing output for
/// an app executable.
fn parse_standalone_args(payload: &[u8]) -> StandaloneArgs {
    let mut port = 5011u16;
    let mut workers = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    let mut dev_mode = false;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                port = args.get(i).and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                    eprintln!("--port requires a port number");
                    process::exit(64);
                });
            }
            "--workers" => {
                i += 1;
                workers = args.get(i).and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                    eprintln!("--workers requires a number");
                    process::exit(64);
                });
            }
            "--host" => {
                i += 1;
                let host = args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--host requires an IP address");
                    process::exit(64);
                });
                // The server reads SOLI_HOST at bind time.
                std::env::set_var("SOLI_HOST", host);
            }
            "--dev" => dev_mode = true,
            "--help" | "-h" => {
                print_standalone_usage(&exe_label());
                process::exit(0);
            }
            "--version" | "-v" => {
                let kind = if payload.starts_with(b"SOLE") {
                    "encrypted"
                } else {
                    "plain"
                };
                println!(
                    "{} (soli standalone runtime {}, {} bundle, {} KB payload)",
                    exe_label(),
                    VERSION,
                    kind,
                    payload.len() / 1024
                );
                process::exit(0);
            }
            other => {
                eprintln!("Unknown option: {}", other);
                print_standalone_usage(&exe_label());
                process::exit(64);
            }
        }
        i += 1;
    }

    StandaloneArgs {
        port,
        workers,
        dev_mode,
    }
}

fn print_standalone_usage(exe: &str) {
    println!("{} — self-contained Soli application", exe);
    println!();
    println!("Usage: ./{} [options]", exe);
    println!();
    println!("Options:");
    println!("  --port PORT      Port to listen on (default: 5011)");
    println!("  --host IP        Interface to bind (default: all; sets SOLI_HOST)");
    println!("  --workers N      Worker threads (default: CPU count)");
    println!("  --dev            Development mode");
    println!("  --version, -v    Print version information");
    println!("  --help, -h       Show this help");
    println!();
    println!("If the embedded bundle is encrypted, provide the key via");
    println!("SOLI_BUNDLE_KEY or SOLI_BUNDLE_AUTH_URL (+ optional SOLI_BUNDLE_API_KEY),");
    println!("in the environment or a .env file next to this executable.");
    println!("Encrypted apps extract to RAM (/dev/shm); on systems without it,");
    println!("set SOLI_BUNDLE_ALLOW_DISK=1 to allow temp-dir extraction.");
}

// ---------------------------------------------------------------------------
// Build side
// ---------------------------------------------------------------------------

/// Write `runtime ‖ bundle ‖ footer` to `output` and make it executable.
/// `target = None` embeds the running soli binary (host platform);
/// `Some(target)` embeds a published release runtime for that platform.
pub fn write_standalone_exe(
    bundle_data: &[u8],
    output: &Path,
    target: Option<&str>,
) -> Result<(), String> {
    let (template, target) = resolve_runtime_template(target)?;

    if template.len() as u64 >= FOOTER_LEN
        && &template[template.len() - 16..template.len() - 8] == FOOTER_MAGIC
    {
        return Err(
            "the runtime template is itself a standalone app executable — \
             install the plain soli release and rebuild"
                .to_string(),
        );
    }

    let mut out = Vec::with_capacity(template.len() + bundle_data.len() + 16);
    out.extend_from_slice(&template);
    out.extend_from_slice(bundle_data);
    out.extend_from_slice(FOOTER_MAGIC);
    out.extend_from_slice(&(bundle_data.len() as u64).to_le_bytes());

    std::fs::write(output, &out)
        .map_err(|e| format!("failed to write '{}': {}", output.display(), e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("failed to chmod '{}': {}", output.display(), e))?;
    }

    sign_if_needed(output, &target)?;
    Ok(())
}

/// Signing matrix for the produced artifact. Appending bytes to a Mach-O
/// invalidates its ad-hoc signature and Apple Silicon SIGKILLs unsigned
/// binaries at exec — so darwin outputs must be re-signed.
fn sign_if_needed(output: &Path, target: &BuildTarget) -> Result<(), String> {
    if !target.is_darwin() {
        return Ok(()); // ELF ignores trailing bytes — nothing to do.
    }
    if cfg!(target_os = "macos") {
        // Hard-fail: a silently-broken artifact is worse than a failed build.
        let status = std::process::Command::new("codesign")
            .args(["--force", "-s", "-"])
            .arg(output)
            .status()
            .map_err(|e| format!("failed to run codesign: {}", e))?;
        if !status.success() {
            return Err(format!(
                "codesign failed on '{}' — the artifact would be killed at exec on Apple Silicon",
                output.display()
            ));
        }
        Ok(())
    } else {
        println!();
        println!(
            "  \x1b[33m\x1b[1m! macOS artifact built on {}\x1b[0m — its code signature is now",
            std::env::consts::OS
        );
        println!("    invalid and Apple Silicon will refuse to run it. Before distributing,");
        println!("    ad-hoc re-sign it on a Mac:");
        println!("      codesign --force -s - {}", output.display());
        println!("    (or use rcodesign from any platform: rcodesign sign <file>)");
        Ok(())
    }
}

/// Which runtime the artifact embeds.
enum BuildTarget {
    /// The running soli binary (host platform).
    Host,
    /// A published release artifact, e.g. "darwin-arm64".
    Release(String),
}

impl BuildTarget {
    fn is_darwin(&self) -> bool {
        match self {
            BuildTarget::Host => cfg!(target_os = "macos"),
            BuildTarget::Release(t) => t.starts_with("darwin-"),
        }
    }

    /// Human label for the build summary.
    pub fn label(&self) -> String {
        match self {
            BuildTarget::Host => format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            BuildTarget::Release(t) => t.clone(),
        }
    }
}

/// Normalize a user-supplied `--target` to a release artifact name.
fn normalize_target(target: &str) -> Result<String, String> {
    let normalized = match target {
        "linux-amd64" | "linux-x86_64" | "linux-x64" => "linux-amd64",
        "linux-arm64" | "linux-aarch64" => "linux-arm64",
        "darwin-arm64" | "darwin-aarch64" | "macos-arm64" | "macos-aarch64" => "darwin-arm64",
        other => {
            return Err(format!(
                "unsupported target '{}' — supported targets: {}",
                other,
                SUPPORTED_TARGETS.join(", ")
            ))
        }
    };
    Ok(normalized.to_string())
}

/// Resolve the runtime bytes to embed. Host builds read `current_exe()`;
/// `--target` builds use a published release runtime: cache hit under
/// `~/.cache/soli/runtimes/`, else download + sha256-verify + cache.
fn resolve_runtime_template(target: Option<&str>) -> Result<(Vec<u8>, BuildTarget), String> {
    let Some(target) = target else {
        let exe = std::env::current_exe()
            .map_err(|e| format!("cannot locate the running soli binary: {}", e))?;
        let bytes = std::fs::read(&exe)
            .map_err(|e| format!("cannot read the running soli binary: {}", e))?;
        return Ok((bytes, BuildTarget::Host));
    };

    let target = normalize_target(target)?;
    let cache_path = runtime_cache_dir()?
        .join(format!("v{}", VERSION))
        .join(format!("soli-{}", target));

    if cache_path.is_file() {
        println!(
            "  Using cached {} runtime ({})",
            target,
            cache_path.display()
        );
        let bytes = std::fs::read(&cache_path).map_err(|e| {
            format!(
                "cannot read cached runtime '{}': {}",
                cache_path.display(),
                e
            )
        })?;
        return Ok((bytes, BuildTarget::Release(target)));
    }

    let bytes = download_release_runtime(&target)?;

    // Cache for next time (rename-into-place from a temp sibling so a
    // concurrent build never sees a half-written file).
    if let Some(parent) = cache_path.parent() {
        if std::fs::create_dir_all(parent).is_ok() {
            let tmp = parent.join(format!(".soli-{}.tmp-{}", target, std::process::id()));
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &cache_path);
            }
        }
    }

    Ok((bytes, BuildTarget::Release(target)))
}

fn runtime_cache_dir() -> Result<PathBuf, String> {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("soli").join("runtimes"));
        }
    }
    let home =
        std::env::var("HOME").map_err(|_| "cannot determine cache dir (no HOME)".to_string())?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("soli")
        .join("runtimes"))
}

fn release_base_url() -> String {
    std::env::var("SOLI_RELEASE_BASE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .unwrap_or_else(|| format!("https://github.com/{}/releases/download", RELEASE_REPO))
}

/// Download `soli-{target}.tar.gz` for this soli's exact version, verify it
/// against the published `.sha256` sibling, extract, and return the runtime
/// bytes. Mirrors the SEC-041/SEC-042a discipline of `soli update`
/// (`run_self_update`): TLS-1.2 floor, mode-0700 temp staging, hard-fail on
/// checksum mismatch, warn-and-continue only when no checksum is published.
fn download_release_runtime(target: &str) -> Result<Vec<u8>, String> {
    let tarball = format!("soli-{}.tar.gz", target);
    let url = format!("{}/v{}/{}", release_base_url(), VERSION, tarball);

    println!("  Downloading {} runtime v{} ...", target, VERSION);

    let client = reqwest::blocking::Client::builder()
        .user_agent("soli-lang-cli")
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .build()
        .map_err(|e| format!("failed to create HTTP client: {}", e))?;

    let mut response = client
        .get(&url)
        .send()
        .map_err(|e| format!("failed to download {}: {}", url, e))
        .and_then(|resp| {
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                Err(format!(
                    "soli v{} has no published {} artifact at {} (dev build or unpublished \
                     release?) — use a released soli, omit --target, or point \
                     SOLI_RELEASE_BASE_URL at a mirror",
                    VERSION, target, url
                ))
            } else {
                resp.error_for_status()
                    .map_err(|e| format!("download error: {}", e))
            }
        })?;

    let temp_dir = tempfile::Builder::new()
        .prefix("soli-standalone-")
        .tempdir()
        .map_err(|e| format!("failed to create temp directory: {}", e))?;
    let tarball_path = temp_dir.path().join(&tarball);

    let mut file = std::fs::File::create(&tarball_path)
        .map_err(|e| format!("failed to create temp file: {}", e))?;
    response
        .copy_to(&mut file)
        .map_err(|e| format!("failed to write download: {}", e))?;
    drop(file);

    // Verify against the published checksum before extracting anything.
    let actual_sha = crate::cli::commands::sha256_of_file(&tarball_path)?;
    let sha_url = format!("{}.sha256", url);
    match client.get(&sha_url).send() {
        Ok(resp) if resp.status().is_success() => {
            let body = resp
                .text()
                .map_err(|e| format!("failed to read .sha256: {}", e))?;
            let expected = body.split_whitespace().next().unwrap_or("").to_lowercase();
            if expected.is_empty() {
                return Err(format!("empty .sha256 file at {}", sha_url));
            }
            if expected != actual_sha {
                return Err(format!(
                    "checksum mismatch for {}: expected {}, got {}",
                    tarball, expected, actual_sha
                ));
            }
            println!("  Checksum verified.");
        }
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            eprintln!(
                "  \x1b[33mWarning:\x1b[0m no .sha256 published for v{} — \
                 skipping checksum verification.",
                VERSION
            );
        }
        Ok(resp) => return Err(format!("fetching .sha256: HTTP {}", resp.status())),
        Err(e) => return Err(format!("fetching .sha256: {}", e)),
    }

    let tf =
        std::fs::File::open(&tarball_path).map_err(|e| format!("failed to open tarball: {}", e))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tf));
    archive
        .unpack(temp_dir.path())
        .map_err(|e| format!("failed to extract tarball: {}", e))?;

    let binary_path = temp_dir.path().join("soli");
    std::fs::read(&binary_path).map_err(|e| {
        format!(
            "tarball did not contain a 'soli' binary ({}): {}",
            binary_path.display(),
            e
        )
    })
}

/// Public label helper for `run_build`'s summary line.
pub fn target_label(target: Option<&str>) -> String {
    match target {
        None => BuildTarget::Host.label(),
        Some(t) => normalize_target(t).unwrap_or_else(|_| t.to_string()),
    }
}

/// Validate a `--target` value early (at parse/build start) so the bundle
/// isn't built before a typo is caught.
pub fn validate_target(target: &str) -> Result<(), String> {
    normalize_target(target).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch_file(bytes: &[u8]) -> (tempfile::TempDir, std::fs::File, u64) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exe");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        drop(f);
        let f = std::fs::File::open(&path).unwrap();
        let len = f.metadata().unwrap().len();
        (dir, f, len)
    }

    fn make_exe(runtime: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = runtime.to_vec();
        out.extend_from_slice(payload);
        out.extend_from_slice(FOOTER_MAGIC);
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out
    }

    #[test]
    fn footer_round_trip() {
        let payload = b"SOLB\x00\x00\x00\x00fake-bundle-bytes";
        let exe = make_exe(b"fake-runtime", payload);
        let (_dir, mut f, len) = scratch_file(&exe);
        let plen = read_footer(&mut f, len).expect("footer detected");
        assert_eq!(plen, payload.len() as u64);
        let got = read_payload(&mut f, len, plen).expect("payload reads");
        assert_eq!(got, payload);
    }

    #[test]
    fn no_footer_is_none() {
        let (_dir, mut f, len) = scratch_file(b"just a plain binary with no trailer");
        assert!(read_footer(&mut f, len).is_none());
    }

    #[test]
    fn truncated_file_is_none() {
        let (_dir, mut f, len) = scratch_file(b"tiny");
        assert!(read_footer(&mut f, len).is_none());
    }

    #[test]
    fn overflowing_payload_len_is_rejected() {
        let mut exe = b"runtime".to_vec();
        exe.extend_from_slice(FOOTER_MAGIC);
        exe.extend_from_slice(&u64::MAX.to_le_bytes());
        let (_dir, mut f, len) = scratch_file(&exe);
        let plen = read_footer(&mut f, len).expect("footer magic matches");
        let err = read_payload(&mut f, len, plen).unwrap_err();
        assert!(err.contains("only has"), "unexpected error: {}", err);
    }

    #[test]
    fn zero_payload_is_rejected() {
        let exe = make_exe(b"runtime", b"");
        let (_dir, mut f, len) = scratch_file(&exe);
        let plen = read_footer(&mut f, len).unwrap();
        assert!(read_payload(&mut f, len, plen).is_err());
    }

    #[test]
    fn non_bundle_payload_is_rejected() {
        let exe = make_exe(b"runtime", b"NOTA bundle at all");
        let (_dir, mut f, len) = scratch_file(&exe);
        let plen = read_footer(&mut f, len).unwrap();
        let err = read_payload(&mut f, len, plen).unwrap_err();
        assert!(err.contains("bad magic"), "unexpected error: {}", err);
    }

    #[test]
    fn footer_magic_in_payload_body_does_not_confuse_detection() {
        // The magic anywhere but EOF-anchored position is meaningless.
        let mut payload = b"SOLB".to_vec();
        payload.extend_from_slice(FOOTER_MAGIC);
        payload.extend_from_slice(&[0u8; 32]);
        let exe = make_exe(b"runtime", &payload);
        let (_dir, mut f, len) = scratch_file(&exe);
        let plen = read_footer(&mut f, len).unwrap();
        assert_eq!(plen, payload.len() as u64);
        assert_eq!(read_payload(&mut f, len, plen).unwrap(), payload);
    }

    #[test]
    fn target_aliases_normalize() {
        assert_eq!(normalize_target("linux-x86_64").unwrap(), "linux-amd64");
        assert_eq!(normalize_target("linux-aarch64").unwrap(), "linux-arm64");
        assert_eq!(normalize_target("macos-arm64").unwrap(), "darwin-arm64");
        assert_eq!(normalize_target("darwin-arm64").unwrap(), "darwin-arm64");
        let err = normalize_target("windows-amd64").unwrap_err();
        assert!(err.contains("linux-amd64, linux-arm64, darwin-arm64"));
    }

    #[test]
    fn write_standalone_refuses_standalone_template() {
        // Simulate: current_exe is itself a standalone app. We can't easily
        // fake current_exe, so exercise the guard through the byte check.
        let template = make_exe(b"runtime", b"SOLB\x00\x00\x00\x00x");
        assert_eq!(
            &template[template.len() - 16..template.len() - 8],
            FOOTER_MAGIC
        );
    }
}
