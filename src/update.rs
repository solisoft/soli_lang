//! Auto-update for standalone & desktop artifacts.
//!
//! A `soli build --standalone` / `soli desktop build` artifact is otherwise a
//! frozen binary. Built with `--update-url`, it can check a release channel and
//! replace itself with a newer version — the same download → verify → swap flow
//! `soli update` uses for the CLI, pointed at the *developer's* channel instead
//! of soli's GitHub.
//!
//! # Trust
//!
//! The new binary is fetched from the developer's own URL, so the sha256 in the
//! manifest is not enough on its own — a host that can serve a bad binary can
//! serve a matching sha256. So the manifest is **signed** (P-256 ECDSA, the
//! same curve APNs/VAPID already use) and verified against a public key embedded
//! at build time. An unsigned update is accepted only when no key was embedded,
//! and then only with a loud warning. Downgrades are refused
//! ([`crate::module::compare_versions`]). The swap is atomic: a failed or
//! tampered download never touches the installed binary.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Embedded in the bundle as `__soli_update__` at build time. Tells a running
/// artifact where to look and what to trust.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDescriptor {
    /// The app's own version, from its `soli.toml` `[package] version`.
    pub app_version: String,
    /// Base URL the manifest is published under.
    pub update_url: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    /// Base64 SEC1 P-256 public key the manifest signature is checked against.
    /// Absent means unsigned updates (dev/testing only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
}

fn default_channel() -> String {
    "stable".to_string()
}

/// The bundle entry key for [`UpdateDescriptor`].
pub const UPDATE_ENTRY: &str = "__soli_update__";

/// The descriptor of the artifact currently being served, if any. Set once at
/// serve boot from the bundle's `__soli_update__` entry, so the `Updater`
/// builtin can drive an in-app update UI without re-reading the payload.
static ACTIVE_DESCRIPTOR: OnceLock<UpdateDescriptor> = OnceLock::new();

/// Stash the update descriptor found in a bundle's entries (no-op if absent or
/// already set). Called from the serve boot path.
pub fn stash_active_descriptor(entries: &[(String, &[u8])]) {
    if ACTIVE_DESCRIPTOR.get().is_some() {
        return;
    }
    if let Some(desc) = entries
        .iter()
        .find(|(p, _)| p == UPDATE_ENTRY)
        .and_then(|(_, data)| serde_json::from_slice::<UpdateDescriptor>(data).ok())
    {
        let _ = ACTIVE_DESCRIPTOR.set(desc);
    }
}

/// The descriptor of the artifact being served, for the `Updater` builtin.
pub fn active_descriptor() -> Option<&'static UpdateDescriptor> {
    ACTIVE_DESCRIPTOR.get()
}

/// One published artifact in a manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactEntry {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

/// What the developer publishes at `<update_url>/<channel>/latest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    /// `target -> artifact`, e.g. `"darwin-arm64"`.
    pub artifacts: HashMap<String, ArtifactEntry>,
    /// Base64 P-256 ECDSA signature over the canonicalized manifest with this
    /// field removed. Absent on an unsigned manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// The result of a check, for the CLI and the `Updater` builtin.
pub struct CheckResult {
    pub available: bool,
    pub current: String,
    pub latest: String,
    pub notes: String,
}

/// This host's published-artifact target name (`linux-amd64`, `darwin-arm64`, …).
pub fn host_target() -> Result<String, String> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        other => return Err(format!("auto-update: unsupported OS '{}'", other)),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        other => return Err(format!("auto-update: unsupported architecture '{}'", other)),
    };
    Ok(format!("{}-{}", os, arch))
}

#[cfg(not(target_arch = "wasm32"))]
fn http_client() -> Result<reqwest::blocking::Client, String> {
    // Same TLS-1.2 floor as `run_self_update` and the runtime clients.
    reqwest::blocking::Client::builder()
        .user_agent("soli-artifact-updater")
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .build()
        .map_err(|e| format!("auto-update: HTTP client init failed: {}", e))
}

/// Fetch and parse the channel manifest.
#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_manifest(descriptor: &UpdateDescriptor) -> Result<UpdateManifest, String> {
    let base = descriptor.update_url.trim_end_matches('/');
    require_transport(base, "update_url")?;
    let url = format!("{}/{}/latest.json", base, descriptor.channel);
    let body = http_client()?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("auto-update: could not fetch {}: {}", url, e))?
        .text()
        .map_err(|e| format!("auto-update: could not read manifest: {}", e))?;

    let manifest: UpdateManifest = serde_json::from_str(&body)
        .map_err(|e| format!("auto-update: manifest is not valid JSON: {}", e))?;

    verify_signature(&manifest, descriptor.pubkey.as_deref())?;
    Ok(manifest)
}
#[cfg(target_arch = "wasm32")]
pub fn fetch_manifest(_descriptor: &UpdateDescriptor) -> Result<UpdateManifest, String> {
    Err(crate::platform::unsupported_on_edge("fetch_manifest"))
}

/// Canonical bytes a signature covers: the manifest as sorted-key JSON with the
/// `signature` field removed, so signer and verifier agree byte-for-byte
/// regardless of field order.
pub fn canonical_bytes(manifest: &UpdateManifest) -> Result<Vec<u8>, String> {
    let mut value = serde_json::to_value(manifest)
        .map_err(|e| format!("auto-update: canonicalization failed: {}", e))?;
    if let Some(object) = value.as_object_mut() {
        object.remove("signature");
    }
    Ok(canonical_json(&value).into_bytes())
}

/// Deterministic JSON: object keys sorted, recursively. serde_json does not
/// guarantee key order, so a signature could not otherwise be reproduced.
fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<&String, &serde_json::Value> = map.iter().collect();
            let inner: Vec<String> = sorted
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical_json(v)
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// The largest artifact an update may be.
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// An update travels over https — or over http to this machine, for a test
/// or a local mirror. Over plain http to anywhere else, whoever sits on the
/// path serves the manifest and the binary.
fn require_transport(url: &str, what: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = lower.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = match authority.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => authority.split(':').next().unwrap_or(""),
        };
        if matches!(host, "localhost" | "127.0.0.1" | "::1") {
            return Ok(());
        }
    }
    Err(format!(
        "auto-update: {what} must be https:// (plain http only to this machine), got '{url}'"
    ))
}

/// May a manifest without a signature be acted on? Only when whoever runs
/// the app says so (`SOLI_ALLOW_UNSIGNED_UPDATES=1`).
fn unsigned_updates_allowed() -> bool {
    crate::platform::env::var("SOLI_ALLOW_UNSIGNED_UPDATES")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn verify_signature(manifest: &UpdateManifest, pubkey_b64: Option<&str>) -> Result<(), String> {
    let Some(pubkey_b64) = pubkey_b64 else {
        // No key embedded. The manifest used to be trusted with a warning:
        // anyone able to answer for the update URL — its host, its DNS, a
        // hop on the way — then chose the binary every installed copy runs.
        if unsigned_updates_allowed() {
            eprintln!(
                "  \x1b[33mWarning:\x1b[0m this artifact was built without --update-key and \
                 SOLI_ALLOW_UNSIGNED_UPDATES is set: the update manifest is trusted without a \
                 signature."
            );
            return Ok(());
        }
        return Err(
            "auto-update: this artifact was built without --update-key, so its updates cannot \
             be verified. Rebuild with --update-key (soli update keygen), or set \
             SOLI_ALLOW_UNSIGNED_UPDATES=1 to accept unsigned updates."
                .to_string(),
        );
    };
    let signature_b64 = manifest.signature.as_deref().ok_or_else(|| {
        "auto-update: the artifact requires a signed manifest but this one is unsigned".to_string()
    })?;

    let key_bytes = B64
        .decode(pubkey_b64.trim())
        .map_err(|_| "auto-update: embedded update key is not valid base64".to_string())?;
    let verifying = VerifyingKey::from_sec1_bytes(&key_bytes)
        .map_err(|e| format!("auto-update: embedded update key is invalid: {}", e))?;
    let signature = Signature::from_slice(
        &B64.decode(signature_b64.trim())
            .map_err(|_| "auto-update: manifest signature is not valid base64".to_string())?,
    )
    .map_err(|_| "auto-update: manifest signature is malformed".to_string())?;

    let message = canonical_bytes(manifest)?;
    verifying
        .verify(&message, &signature)
        .map_err(|_| "auto-update: manifest signature does not verify".to_string())
}

/// Compare the manifest version against what this artifact carries.
pub fn check(descriptor: &UpdateDescriptor) -> Result<CheckResult, String> {
    let manifest = fetch_manifest(descriptor)?;
    let available = matches!(
        crate::module::compare_versions(&manifest.version, &descriptor.app_version),
        std::cmp::Ordering::Greater
    );
    Ok(CheckResult {
        available,
        current: descriptor.app_version.clone(),
        latest: manifest.version.clone(),
        notes: manifest.notes.clone(),
    })
}

/// Download the newer artifact, verify it, and replace this executable.
pub fn apply(descriptor: &UpdateDescriptor) -> Result<String, String> {
    let manifest = fetch_manifest(descriptor)?;
    if !matches!(
        crate::module::compare_versions(&manifest.version, &descriptor.app_version),
        std::cmp::Ordering::Greater
    ) {
        return Ok(format!("already up to date (v{})", descriptor.app_version));
    }

    let target = host_target()?;
    let entry = manifest.artifacts.get(&target).ok_or_else(|| {
        format!(
            "auto-update: v{} has no artifact for this platform ({})",
            manifest.version, target
        )
    })?;

    let bytes = download_and_verify(entry)?;
    let exe = std::env::current_exe()
        .map_err(|e| format!("auto-update: cannot locate current executable: {}", e))?;
    replace_executable(&exe, &bytes)?;
    Ok(format!("updated to v{}", manifest.version))
}

#[cfg(not(target_arch = "wasm32"))]
fn download_and_verify(entry: &ArtifactEntry) -> Result<Vec<u8>, String> {
    require_transport(&entry.url, "artifact url")?;
    let mut response = http_client()?
        .get(&entry.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("auto-update: download failed: {}", e))?;

    // No more than the manifest announced (or the ceiling, when it announced
    // nothing): the download was read to its end, whatever its size, before
    // the digest could refuse it.
    let limit = if entry.size > 0 {
        entry.size
    } else {
        MAX_ARTIFACT_BYTES
    }
    .min(MAX_ARTIFACT_BYTES);
    let mut bytes = Vec::new();
    (&mut response)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("auto-update: download read failed: {}", e))?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "auto-update: the artifact is larger than the {limit} bytes the manifest allows. Refusing to install."
        ));
    }

    let digest = format!("{:x}", Sha256::digest(&bytes));
    if !digest.eq_ignore_ascii_case(entry.sha256.trim()) {
        return Err(format!(
            "auto-update: sha256 mismatch — expected {}, got {}. Refusing to install.",
            entry.sha256.trim(),
            digest
        ));
    }
    Ok(bytes)
}
#[cfg(target_arch = "wasm32")]
fn download_and_verify(_entry: &ArtifactEntry) -> Result<Vec<u8>, String> {
    Err(crate::platform::unsupported_on_edge("download_and_verify"))
}

/// Swap `bytes` in for the running executable, atomically where possible.
///
/// Unix: write a sibling `<exe>.new` on the same filesystem, chmod 0755, then
/// `rename` over the target — the running process keeps its open inode, and the
/// rename is atomic, so a crash mid-write never leaves a half-written binary.
///
/// Windows: a running `.exe` cannot be overwritten, so the current file is
/// renamed aside and the new one moved into place; the aside copy is swept on
/// next launch.
pub fn replace_executable(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = exe
        .parent()
        .ok_or_else(|| "auto-update: executable has no parent directory".to_string())?;
    let staged: PathBuf = dir.join(format!(
        "{}.new",
        exe.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("soli-app")
    ));

    // A new file, never whatever is already at that path: `fs::write`
    // followed a symbolic link left there and wrote the binary to its target.
    let _ = std::fs::remove_file(&staged);
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|e| format!("auto-update: cannot stage new binary: {}", e))?;
        file.write_all(bytes)
            .map_err(|e| format!("auto-update: cannot stage new binary: {}", e))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("auto-update: chmod failed: {}", e))?;
        std::fs::rename(&staged, exe).map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            format!("auto-update: could not replace the executable: {}", e)
        })?;
    }

    #[cfg(windows)]
    {
        let aside = dir.join(format!(
            "{}.old",
            exe.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("soli-app")
        ));
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(exe, &aside)
            .map_err(|e| format!("auto-update: could not move the running exe aside: {}", e))?;
        std::fs::rename(&staged, exe).map_err(|e| {
            // Roll back so the app still launches.
            let _ = std::fs::rename(&aside, exe);
            format!("auto-update: could not install the new exe: {}", e)
        })?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Soli's own releases: a signature beside the digest.
// ---------------------------------------------------------------------------

/// The public key soli's release tarballs are signed with (base64 SEC1
/// P-256, as `soli update-keygen` prints it), set when this binary was built
/// (`SOLI_RELEASE_PUBKEY`). None on a build that names no key: `soli update`
/// then verifies the published digest only, as it always has.
///
/// The digest sits beside the tarball, on the same release: whoever can
/// replace one can replace the other. A signature cannot be replaced
/// without the private key, which is not on the release host.
pub fn release_pubkey() -> Option<&'static str> {
    option_env!("SOLI_RELEASE_PUBKEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

/// Check `signature_b64` — an ECDSA P-256 signature over the SHA-256 of
/// `bytes`, DER-encoded then base64, which is what
/// `openssl dgst -sha256 -sign key.pem file | openssl base64 -A` writes —
/// against `pubkey_b64`.
pub fn verify_release_signature(
    bytes: &[u8],
    signature_b64: &str,
    pubkey_b64: &str,
) -> Result<(), String> {
    let key_bytes = B64
        .decode(pubkey_b64.trim())
        .map_err(|_| "the release key built into this binary is not valid base64".to_string())?;
    let verifying = VerifyingKey::from_sec1_bytes(&key_bytes)
        .map_err(|e| format!("the release key built into this binary is invalid: {e}"))?;
    let compact: String = signature_b64.split_whitespace().collect();
    let der = B64
        .decode(compact)
        .map_err(|_| "the release signature is not valid base64".to_string())?;
    let signature =
        Signature::from_der(&der).map_err(|_| "the release signature is malformed".to_string())?;
    verifying
        .verify(bytes, &signature)
        .map_err(|_| "the release signature does not verify".to_string())
}

// ---------------------------------------------------------------------------
// Developer tooling: keygen + manifest signing.
// ---------------------------------------------------------------------------

/// A fresh P-256 keypair: (private PKCS#8 PEM, public base64 SEC1).
pub fn generate_keypair() -> Result<(String, String), String> {
    use p256::pkcs8::EncodePrivateKey;
    let signing = <SigningKey as p256::elliptic_curve::Generate>::generate();
    let private_pem = signing
        .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
        .map_err(|e| format!("keygen: {}", e))?
        .to_string();
    let verifying = VerifyingKey::from(&signing);
    let public_b64 = B64.encode(verifying.to_sec1_point(false).as_bytes());
    Ok((private_pem, public_b64))
}

/// Sign a manifest file in place with a PKCS#8 P-256 private key.
pub fn sign_manifest_file(manifest_path: &Path, key_pem: &str) -> Result<(), String> {
    use p256::pkcs8::DecodePrivateKey;
    let raw = std::fs::read_to_string(manifest_path).map_err(|e| {
        format!(
            "sign-update: cannot read {}: {}",
            manifest_path.display(),
            e
        )
    })?;
    let mut manifest: UpdateManifest = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "sign-update: {} is not a valid manifest: {}",
            manifest_path.display(),
            e
        )
    })?;
    manifest.signature = None;

    let signing = SigningKey::from_pkcs8_pem(key_pem.trim())
        .map_err(|e| format!("sign-update: key is not a valid PKCS#8 P-256 key: {}", e))?;
    let message = canonical_bytes(&manifest)?;
    let signature: Signature = signing.sign(&message);
    manifest.signature = Some(B64.encode(signature.to_bytes()));

    let pretty = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("sign-update: serialize failed: {}", e))?;
    std::fs::write(manifest_path, pretty + "\n").map_err(|e| {
        format!(
            "sign-update: cannot write {}: {}",
            manifest_path.display(),
            e
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release tarball's signature: what the key signed verifies, and
    /// nothing else does — other bytes, another key, a signature cut short.
    #[test]
    fn a_release_signature_verifies_only_what_its_key_signed() {
        use p256::pkcs8::DecodePrivateKey;
        let (private_pem, public_b64) = generate_keypair().unwrap();
        let (_, other_public) = generate_keypair().unwrap();
        let signing = SigningKey::from_pkcs8_pem(&private_pem).unwrap();
        let tarball = b"the bytes of soli-linux-amd64.tar.gz";
        let signature: Signature = signing.sign(tarball);
        let sig_b64 = B64.encode(signature.to_der().as_bytes());

        assert!(verify_release_signature(tarball, &sig_b64, &public_b64).is_ok());
        // As a file read back with its trailing newline.
        assert!(verify_release_signature(tarball, &format!("{sig_b64}\n"), &public_b64).is_ok());
        assert!(verify_release_signature(b"other bytes", &sig_b64, &public_b64).is_err());
        assert!(verify_release_signature(tarball, &sig_b64, &other_public).is_err());
        assert!(verify_release_signature(tarball, &sig_b64[..20], &public_b64).is_err());
        assert!(verify_release_signature(tarball, "", &public_b64).is_err());
        assert!(verify_release_signature(tarball, &sig_b64, "not a key").is_err());
    }

    /// The release pipeline signs with
    /// `openssl dgst -sha256 -sign key.pem file | openssl base64 -A`, using
    /// the key `soli update-keygen` printed. That exact output verifies here.
    #[test]
    fn a_signature_made_by_openssl_as_the_release_pipeline_makes_it_verifies() {
        let openssl = std::process::Command::new("openssl")
            .arg("version")
            .output();
        if !openssl.is_ok_and(|out| out.status.success()) {
            eprintln!("skip: no openssl on this machine");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (private_pem, public_b64) = generate_keypair().unwrap();
        let key = dir.path().join("key.pem");
        let tarball = dir.path().join("soli.tar.gz");
        std::fs::write(&key, &private_pem).unwrap();
        let bytes: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&tarball, &bytes).unwrap();

        let signed = std::process::Command::new("openssl")
            .args(["dgst", "-sha256", "-sign"])
            .arg(&key)
            .arg(&tarball)
            .output()
            .unwrap();
        assert!(
            signed.status.success(),
            "{}",
            String::from_utf8_lossy(&signed.stderr)
        );
        let sig_b64 = B64.encode(&signed.stdout);
        verify_release_signature(&bytes, &sig_b64, &public_b64).expect("openssl's signature");
        let mut tampered = bytes.clone();
        tampered[100] ^= 1;
        assert!(verify_release_signature(&tampered, &sig_b64, &public_b64).is_err());
    }

    fn sample_manifest() -> UpdateManifest {
        let mut artifacts = HashMap::new();
        artifacts.insert(
            "linux-amd64".to_string(),
            ArtifactEntry {
                url: "https://example.com/app-1.2.0-linux-amd64".to_string(),
                sha256: "abc123".to_string(),
                size: 100,
            },
        );
        UpdateManifest {
            version: "1.2.0".to_string(),
            notes: "notes".to_string(),
            artifacts,
            signature: None,
        }
    }

    /// Canonical bytes must not depend on field order, or a valid signature
    /// would fail to reproduce.
    #[test]
    fn canonical_bytes_ignore_signature_and_are_stable() {
        let mut a = sample_manifest();
        let base = canonical_bytes(&a).unwrap();
        a.signature = Some("whatever".to_string());
        assert_eq!(
            canonical_bytes(&a).unwrap(),
            base,
            "signature must not be covered"
        );
    }

    /// The load-bearing property: a manifest signed with a key verifies against
    /// that key, and a tampered version does not.
    #[test]
    fn a_signed_manifest_verifies_and_tampering_is_caught() {
        let signing = <SigningKey as p256::elliptic_curve::Generate>::generate();
        let verifying = VerifyingKey::from(&signing);
        let pubkey = B64.encode(verifying.to_sec1_point(false).as_bytes());

        let mut manifest = sample_manifest();
        let signature: Signature = signing.sign(&canonical_bytes(&manifest).unwrap());
        manifest.signature = Some(B64.encode(signature.to_bytes()));

        // Verifies.
        verify_signature(&manifest, Some(&pubkey)).expect("must verify");

        // Tamper the version → signature no longer matches.
        let mut tampered = manifest.clone();
        tampered.version = "9.9.9".to_string();
        assert!(verify_signature(&tampered, Some(&pubkey)).is_err());
    }

    /// An artifact that requires a signature refuses an unsigned manifest.
    #[test]
    fn a_required_key_rejects_an_unsigned_manifest() {
        let (_, pubkey) = generate_keypair().unwrap();
        let manifest = sample_manifest(); // signature: None
        assert!(verify_signature(&manifest, Some(&pubkey)).is_err());
    }

    /// keygen → sign → verify, end to end, through the file signer.
    #[test]
    fn keygen_sign_and_verify_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("latest.json");
        std::fs::write(&path, serde_json::to_string(&sample_manifest()).unwrap()).unwrap();

        // keygen writes PKCS#8 PEM; recover the pubkey the same way keygen does.
        let (private_pem, pubkey) = generate_keypair().unwrap();
        sign_manifest_file(&path, &private_pem).unwrap();

        let signed: UpdateManifest =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(signed.signature.is_some());
        verify_signature(&signed, Some(&pubkey)).expect("signed file must verify");
    }

    #[test]
    fn host_target_is_a_known_shape() {
        let target = host_target().unwrap();
        assert!(
            target.contains('-'),
            "target should be os-arch, got {}",
            target
        );
    }

    #[test]
    fn an_update_travels_over_https_or_to_this_machine() {
        for ok in [
            "https://updates.example.com/app",
            "http://localhost:8080/app",
            "http://127.0.0.1/app",
            "http://[::1]:9000/app",
        ] {
            assert!(require_transport(ok, "update_url").is_ok(), "{ok}");
        }
        for refused in [
            "http://updates.example.com/app",
            "http://localhost.evil.example/app",
            "http://127.0.0.1.evil.example/app",
            "ftp://updates.example.com/app",
            "file:///tmp/app",
        ] {
            assert!(
                require_transport(refused, "update_url").is_err(),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_manifest_without_a_signature_is_refused_by_a_keyless_build() {
        if unsigned_updates_allowed() {
            return;
        }
        let error = verify_signature(&sample_manifest(), None).unwrap_err();
        assert!(error.contains("--update-key"), "{error}");
    }
}
