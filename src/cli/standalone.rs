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
//! Darwin is the exception. A Mach-O may not carry data past `__LINKEDIT` —
//! `codesign` rejects such a file outright ("main executable failed strict
//! validation") and rewriting signers silently drop the trailing bytes, which
//! throws the payload away. So for darwin the inherited signature is stripped,
//! the payload appended, and `__LINKEDIT` grown to cover it (see `cli::macho`).
//! Signing then appends the signature last, which pushes the footer off EOF:
//!
//! ```text
//! [ mach-o, __LINKEDIT grown over everything below ]
//! [ payload ] [ footer ] [ code signature — added by codesign, last ]
//! ```
//!
//! `codesign` starts that signature on a 16-byte boundary, so the appended
//! region is padded to one — otherwise the gap it inserts sits between the
//! footer and the signature, and the boot-side lookup below reads padding
//! instead of the magic (which lands the user in the REPL, not their app).
//!
//! At boot, `boot_if_standalone()` (the first statement of `cli::run()`)
//! reads the last 16 bytes of `current_exe()`, and on a signed Mach-O falls
//! back to the 16 bytes before the signature — then, for artifacts built
//! before the padding fix, walks back over the alignment gap. No footer →
//! normal soli CLI.
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
//! Cross-target builds (`--target linux-amd64|linux-arm64|darwin-amd64|darwin-arm64|windows-amd64`)
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

/// `codesign` starts the code signature blob on a 16-byte boundary. Appending a
/// payload whose end is not itself 16-byte aligned therefore leaves up to 15
/// bytes of padding between our footer and the signature — so the appended
/// region is padded to this alignment at build time, and the boot-side lookup
/// walks back over the padding on artifacts built before that.
const SIGNATURE_ALIGN: u64 = 16;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The GitHub repo releases are published from (same as `soli update`).
const RELEASE_REPO: &str = "solisoft/soli_lang";

/// Release artifact names, exactly as CI publishes them.
const SUPPORTED_TARGETS: &[&str] = &[
    "linux-amd64",
    "linux-arm64",
    "darwin-amd64",
    "darwin-arm64",
    "windows-amd64",
];

// ---------------------------------------------------------------------------
// Boot side
// ---------------------------------------------------------------------------

/// Where the appended region ends, and how long the payload is.
struct Footer {
    payload_len: u64,
    /// Offset one past the footer's last byte.
    end: u64,
}

/// Read the footer of `file` and return the payload length when the magic
/// matches. `file_len` is passed in so the caller can bound-check.
///
/// The footer sits at EOF on ELF and PE, and on any darwin artifact that was
/// never signed. On a *signed* Mach-O the code signature is necessarily the
/// last thing in the file, so the appended region ends where that signature
/// begins — try EOF first, then that anchor.
///
/// The anchor is not always exact. `codesign` aligns the signature blob to
/// [`SIGNATURE_ALIGN`], so on an artifact whose appended region ends off that
/// boundary it inserts padding the anchor then points past. Builds now pad to
/// the boundary themselves, but artifacts produced before that fix are already
/// in the wild — and a payload found 10 bytes early is the difference between
/// an app that boots and one that drops the user into the soli REPL.
fn read_footer(file: &mut std::fs::File, file_len: u64) -> Option<Footer> {
    if let Some(footer) = read_footer_at(file, file_len, file_len) {
        return Some(footer);
    }
    let anchor = macho_footer_anchor(file)?;
    if let Some(footer) = read_footer_at(file, file_len, anchor) {
        return Some(footer);
    }
    read_footer_behind_padding(file, file_len, anchor)
}

/// Walk back from `anchor` over signature alignment padding, looking for the
/// footer. Bounded to the padding a 16-byte alignment can produce: a wider
/// scan would risk matching the magic inside the payload itself.
fn read_footer_behind_padding(
    file: &mut std::fs::File,
    file_len: u64,
    anchor: u64,
) -> Option<Footer> {
    for back in 1..SIGNATURE_ALIGN {
        let end = anchor.checked_sub(back)?;
        if let Some(footer) = read_footer_at(file, file_len, end) {
            // The magic alone is 8 bytes of a payload that may be tens of
            // megabytes; confirm the bundle it points at before trusting it,
            // because a wrong hit here is a hard boot failure rather than a
            // fall-through to the CLI.
            if payload_magic_is_valid(file, &footer) {
                return Some(footer);
            }
        }
    }
    None
}

/// Whether the bytes the footer points at actually begin a soli bundle.
fn payload_magic_is_valid(file: &mut std::fs::File, footer: &Footer) -> bool {
    let Some(start) = footer
        .end
        .checked_sub(FOOTER_LEN)
        .and_then(|e| e.checked_sub(footer.payload_len))
    else {
        return false;
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return false;
    }
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && (&magic == b"SOLB" || &magic == b"SOLE")
}

/// Read a footer whose last byte is at `end - 1`.
fn read_footer_at(file: &mut std::fs::File, file_len: u64, end: u64) -> Option<Footer> {
    if end < FOOTER_LEN || end > file_len {
        return None;
    }
    file.seek(SeekFrom::Start(end - FOOTER_LEN)).ok()?;
    let mut footer = [0u8; 16];
    file.read_exact(&mut footer).ok()?;
    if &footer[..8] != FOOTER_MAGIC {
        return None;
    }
    Some(Footer {
        payload_len: u64::from_le_bytes(footer[8..16].try_into().ok()?),
        end,
    })
}

/// Offset of the code signature blob, for a signed Mach-O. Reads only the
/// header and load commands — the payload itself may be hundreds of megabytes.
fn macho_footer_anchor(file: &mut std::fs::File) -> Option<u64> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut head = [0u8; 32];
    file.read_exact(&mut head).ok()?;
    if !super::macho::is_macho64(&head) {
        return None;
    }
    let sizeofcmds = u32::from_le_bytes(head[20..24].try_into().ok()?) as usize;
    // Guards a corrupt header from driving a huge allocation.
    if sizeofcmds > 4 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; 32 + sizeofcmds];
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_exact(&mut buf).ok()?;
    super::macho::find_footer_anchor(&buf)
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
    let footer = read_footer(&mut file, file_len)?;

    Some(read_payload(&mut file, &footer))
}

fn read_payload(file: &mut std::fs::File, footer: &Footer) -> Result<Vec<u8>, String> {
    let payload_len = footer.payload_len;
    if payload_len == 0 {
        return Err("embedded bundle is empty".to_string());
    }
    let max_payload = footer.end - FOOTER_LEN;
    if payload_len > max_payload {
        return Err(format!(
            "footer claims a {} byte bundle but the executable only has {} bytes before the footer",
            payload_len, max_payload
        ));
    }
    file.seek(SeekFrom::Start(footer.end - FOOTER_LEN - payload_len))
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

    // On darwin the payload cannot simply trail the image: it would sit past
    // __LINKEDIT and past the inherited signature, which is precisely what
    // `codesign` rejects as "main executable failed strict validation" (and what
    // makes rewriting signers drop the payload entirely). Drop the inherited
    // signature, append, then grow __LINKEDIT over what we appended so the file
    // stays a well-formed Mach-O with nothing outside a segment.
    let mut out = if target.is_darwin() {
        super::macho::strip_signature(&template)?
    } else {
        let mut v = Vec::with_capacity(template.len() + bundle_data.len() + 16);
        v.extend_from_slice(&template);
        v
    };
    let appended_from = out.len();
    // Pad ahead of the payload so the footer ends on a signature boundary:
    // `codesign` aligns its blob to one, and any gap it has to insert lands
    // between the footer and the signature, leaving the anchor pointing past
    // the footer instead of at it. Padding first keeps the footer last and
    // costs at most 15 bytes; `read_payload` seeks back from the footer by the
    // recorded length, so leading filler is never read.
    if target.is_darwin() {
        let unaligned =
            (out.len() as u64 + bundle_data.len() as u64 + FOOTER_LEN) % SIGNATURE_ALIGN;
        if unaligned != 0 {
            out.resize(out.len() + (SIGNATURE_ALIGN - unaligned) as usize, 0);
        }
    }
    out.extend_from_slice(bundle_data);
    out.extend_from_slice(FOOTER_MAGIC);
    out.extend_from_slice(&(bundle_data.len() as u64).to_le_bytes());
    if target.is_darwin() {
        let appended = (out.len() - appended_from) as u64;
        super::macho::extend_linkedit(&mut out, appended)?;
    }

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
    if target.is_windows() {
        // Appending past the last section works on Windows — the loader maps
        // sections by header and ignores trailing bytes, which is how every
        // self-extracting installer works. Authenticode is the catch: its hash
        // covers the region the payload now occupies, so a signature applied
        // *before* stapling is invalid afterwards. Signing must therefore run
        // on the finished artifact, which is also the right place for it — the
        // artifact is the customer's application and should carry their
        // certificate, not soli's.
        println!();
        println!("  \x1b[33m\x1b[1m! Windows artifact is unsigned\x1b[0m — SmartScreen will show");
        println!("    \"Windows protected your PC\" and hide \"Run anyway\" behind \"More info\".");
        println!("    Sign the finished file (after this step, never before):");
        println!(
            "      signtool sign /fd sha256 /tr <timestamp-url> /td sha256 {}",
            output.display()
        );
        println!("    (or osslsigncode from any platform, for cross-builds)");
        return Ok(());
    }
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
        println!("    Use codesign, not a rewriting signer: the payload lives inside");
        println!("    __LINKEDIT, and a signer that regenerates that segment drops it.");
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

    fn is_windows(&self) -> bool {
        match self {
            BuildTarget::Host => cfg!(target_os = "windows"),
            BuildTarget::Release(t) => t.starts_with("windows-"),
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

/// Append `.exe` when building for Windows.
///
/// Windows refuses to execute a file without the extension, so an artifact
/// cross-built for Windows from Linux must carry it — the host's own
/// conventions are irrelevant here.
pub fn apply_exe_suffix(path: &Path, target: Option<&str>) -> PathBuf {
    let windows = match target {
        Some(t) => t.starts_with("windows-"),
        None => cfg!(target_os = "windows"),
    };
    if !windows {
        return path.to_path_buf();
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return path.to_path_buf();
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".exe");
    path.with_file_name(name)
}

/// Filename a release tarball stores the runtime under, for a target name.
///
/// Windows executables need the extension; nothing else does.
fn runtime_file_name_for(target: &str) -> &'static str {
    if target.starts_with("windows-") {
        "soli.exe"
    } else {
        "soli"
    }
}

/// Normalize a user-supplied `--target` to a release artifact name.
fn normalize_target(target: &str) -> Result<String, String> {
    let normalized = match target {
        "linux-amd64" | "linux-x86_64" | "linux-x64" => "linux-amd64",
        "linux-arm64" | "linux-aarch64" => "linux-arm64",
        "darwin-arm64" | "darwin-aarch64" | "macos-arm64" | "macos-aarch64" => "darwin-arm64",
        "darwin-amd64" | "darwin-x86_64" | "darwin-x64" | "macos-amd64" | "macos-x86_64"
        | "macos-x64" => "darwin-amd64",
        "windows-amd64" | "windows-x86_64" | "windows-x64" | "win-amd64" | "win64" => {
            "windows-amd64"
        }
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
    artifact_cache_dir("runtimes")
}

/// Cache directory for a class of downloaded release artifact.
pub fn artifact_cache_dir(kind: &str) -> Result<PathBuf, String> {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("soli").join(kind));
        }
    }
    let home =
        std::env::var("HOME").map_err(|_| "cannot determine cache dir (no HOME)".to_string())?;
    Ok(PathBuf::from(home).join(".cache").join("soli").join(kind))
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

    // Windows tarballs carry `soli.exe`; every other target carries `soli`.
    let file_name = runtime_file_name_for(target);
    let binary_path = temp_dir.path().join(file_name);
    std::fs::read(&binary_path).map_err(|e| {
        format!(
            "tarball did not contain a '{}' binary ({}): {}",
            file_name,
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
        let footer = read_footer(&mut f, len).expect("footer detected");
        assert_eq!(footer.payload_len, payload.len() as u64);
        let got = read_payload(&mut f, &footer).expect("payload reads");
        assert_eq!(got, payload);
    }

    /// The shape a darwin artifact has *after* signing: payload and footer live
    /// inside __LINKEDIT, with the code signature blob last. The footer is
    /// deliberately not at EOF — that is the whole point of the anchor lookup.
    fn signed_macho_exe(payload: &[u8]) -> Vec<u8> {
        signed_macho_exe_with_padding(payload, 0)
    }

    /// As `signed_macho_exe`, but with `pad` bytes of alignment filler between
    /// the footer and the signature — what `codesign` inserts when the appended
    /// region does not end on a 16-byte boundary.
    fn signed_macho_exe_with_padding(payload: &[u8], pad: usize) -> Vec<u8> {
        const SEG: usize = 72;
        const SIG: usize = 16;
        let linkedit_fileoff: u64 = 128;
        let sig_blob = vec![0x5Au8; 48];
        let sig_dataoff = linkedit_fileoff + payload.len() as u64 + FOOTER_LEN + pad as u64;
        let filesize = payload.len() as u64 + FOOTER_LEN + pad as u64 + sig_blob.len() as u64;

        let mut out = vec![0u8; 32];
        out[0..4].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
        out[16..20].copy_from_slice(&2u32.to_le_bytes());
        out[20..24].copy_from_slice(&((SEG + SIG) as u32).to_le_bytes());

        let mut seg = vec![0u8; SEG];
        seg[0..4].copy_from_slice(&0x19u32.to_le_bytes());
        seg[4..8].copy_from_slice(&(SEG as u32).to_le_bytes());
        seg[8..18].copy_from_slice(b"__LINKEDIT");
        seg[40..48].copy_from_slice(&linkedit_fileoff.to_le_bytes());
        seg[48..56].copy_from_slice(&filesize.to_le_bytes());
        out.extend_from_slice(&seg);

        let mut sig = vec![0u8; SIG];
        sig[0..4].copy_from_slice(&0x1du32.to_le_bytes());
        sig[4..8].copy_from_slice(&(SIG as u32).to_le_bytes());
        sig[8..12].copy_from_slice(&(sig_dataoff as u32).to_le_bytes());
        sig[12..16].copy_from_slice(&(sig_blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&sig);

        out.resize(linkedit_fileoff as usize, 0);
        out.extend_from_slice(payload);
        out.extend_from_slice(FOOTER_MAGIC);
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out.resize(out.len() + pad, 0);
        out.extend_from_slice(&sig_blob);
        out
    }

    #[test]
    fn footer_is_found_behind_a_code_signature() {
        let payload = b"SOLB\x00\x00\x00\x00signed-artifact-bundle";
        let exe = signed_macho_exe(payload);
        // Precondition: EOF holds signature bytes, so the plain check must miss.
        assert_ne!(&exe[exe.len() - 16..exe.len() - 8], FOOTER_MAGIC);

        let (_dir, mut f, len) = scratch_file(&exe);
        let footer = read_footer(&mut f, len).expect("footer found via signature anchor");
        assert_eq!(footer.payload_len, payload.len() as u64);
        assert_eq!(read_payload(&mut f, &footer).unwrap(), payload.to_vec());
    }

    /// The regression that dropped signed macOS apps into the soli REPL:
    /// `codesign` aligns its blob to 16 bytes, so an appended region ending off
    /// that boundary leaves padding, and anchoring the footer at the signature
    /// offset exactly reads that padding instead of the magic.
    #[test]
    fn footer_is_found_behind_signature_alignment_padding() {
        let payload = b"SOLB\x00\x00\x00\x00signed-artifact-bundle";
        for pad in 1..SIGNATURE_ALIGN as usize {
            let exe = signed_macho_exe_with_padding(payload, pad);
            let (_dir, mut f, len) = scratch_file(&exe);
            let footer = read_footer(&mut f, len)
                .unwrap_or_else(|| panic!("footer found with {} bytes of padding", pad));
            assert_eq!(footer.payload_len, payload.len() as u64);
            assert_eq!(read_payload(&mut f, &footer).unwrap(), payload.to_vec());
        }
    }

    /// Padding is only ever alignment slack, so a footer that is not within a
    /// signature boundary of the anchor is not ours to claim.
    #[test]
    fn a_footer_far_behind_the_anchor_is_not_claimed() {
        let payload = b"SOLB\x00\x00\x00\x00signed-artifact-bundle";
        let exe = signed_macho_exe_with_padding(payload, SIGNATURE_ALIGN as usize + 8);
        let (_dir, mut f, len) = scratch_file(&exe);
        assert!(read_footer(&mut f, len).is_none());
    }

    /// The padding walk must not turn a plain signed binary into a broken app.
    /// A binary whose data merely *contains* the magic just behind the
    /// signature has no payload — claiming one would abort at boot with
    /// "not a soli bundle" instead of running the soli CLI.
    #[test]
    fn magic_in_the_padding_window_without_a_bundle_is_not_claimed() {
        // No footer of its own: the "payload" is inert data ending in a
        // sequence that mimics a footer.
        let mut data = vec![0x11u8; 64];
        data.extend_from_slice(FOOTER_MAGIC);
        data.extend_from_slice(&8u64.to_le_bytes());
        let exe = signed_macho_exe_with_padding(&data, 7);

        let (_dir, mut f, len) = scratch_file(&exe);
        assert!(read_footer(&mut f, len).is_none());
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
        let footer = read_footer(&mut f, len).expect("footer magic matches");
        let err = read_payload(&mut f, &footer).unwrap_err();
        assert!(err.contains("only has"), "unexpected error: {}", err);
    }

    #[test]
    fn zero_payload_is_rejected() {
        let exe = make_exe(b"runtime", b"");
        let (_dir, mut f, len) = scratch_file(&exe);
        let footer = read_footer(&mut f, len).unwrap();
        assert!(read_payload(&mut f, &footer).is_err());
    }

    #[test]
    fn non_bundle_payload_is_rejected() {
        let exe = make_exe(b"runtime", b"NOTA bundle at all");
        let (_dir, mut f, len) = scratch_file(&exe);
        let footer = read_footer(&mut f, len).unwrap();
        let err = read_payload(&mut f, &footer).unwrap_err();
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
        let footer = read_footer(&mut f, len).unwrap();
        assert_eq!(footer.payload_len, payload.len() as u64);
        assert_eq!(read_payload(&mut f, &footer).unwrap(), payload);
    }

    #[test]
    fn windows_artifacts_get_an_exe_extension() {
        // Cross-building for Windows from Linux still has to produce a name
        // Windows will execute — the host's conventions are irrelevant.
        assert_eq!(
            apply_exe_suffix(Path::new("myapp"), Some("windows-amd64")),
            Path::new("myapp.exe")
        );
        // Idempotent, and case-insensitive about an extension already present.
        assert_eq!(
            apply_exe_suffix(Path::new("myapp.exe"), Some("windows-amd64")),
            Path::new("myapp.exe")
        );
        assert_eq!(
            apply_exe_suffix(Path::new("myapp.EXE"), Some("windows-amd64")),
            Path::new("myapp.EXE")
        );
        // Directories in the path are preserved.
        assert_eq!(
            apply_exe_suffix(Path::new("dist/myapp"), Some("windows-amd64")),
            Path::new("dist/myapp.exe")
        );
        // Every other target is left alone.
        for target in ["linux-amd64", "darwin-arm64", "darwin-amd64"] {
            assert_eq!(
                apply_exe_suffix(Path::new("myapp"), Some(target)),
                Path::new("myapp"),
                "{} must not gain an extension",
                target
            );
        }
    }

    #[test]
    fn windows_runtime_tarballs_carry_an_exe() {
        // The published tarball stores `soli.exe` for Windows; looking for
        // `soli` would fail extraction with a confusing "not in tarball".
        assert_eq!(runtime_file_name_for("windows-amd64"), "soli.exe");
        assert_eq!(runtime_file_name_for("linux-amd64"), "soli");
        assert_eq!(runtime_file_name_for("darwin-arm64"), "soli");
    }

    #[test]
    fn target_aliases_normalize() {
        assert_eq!(normalize_target("linux-x86_64").unwrap(), "linux-amd64");
        assert_eq!(normalize_target("linux-aarch64").unwrap(), "linux-arm64");
        assert_eq!(normalize_target("macos-arm64").unwrap(), "darwin-arm64");
        assert_eq!(normalize_target("darwin-arm64").unwrap(), "darwin-arm64");
        // Intel Mac — `soli update` already resolves running hosts to this
        // name, so the builder has to accept it too.
        assert_eq!(normalize_target("macos-x86_64").unwrap(), "darwin-amd64");
        assert_eq!(normalize_target("darwin-amd64").unwrap(), "darwin-amd64");

        assert_eq!(normalize_target("win64").unwrap(), "windows-amd64");
        assert_eq!(normalize_target("windows-amd64").unwrap(), "windows-amd64");

        // Windows on ARM is deliberately not published: there is no native CI
        // runner, so it would be cross-compile-only through the whole vendored
        // C chain. Assert against the live list so adding a target doesn't turn
        // this into a tripwire.
        let err = normalize_target("windows-arm64").unwrap_err();
        assert!(err.contains(&SUPPORTED_TARGETS.join(", ")));
        assert!(err.contains("windows-arm64"));
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
