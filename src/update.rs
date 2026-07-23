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

fn http_client() -> Result<reqwest::blocking::Client, String> {
    // Same TLS-1.2 floor as `run_self_update` and the runtime clients.
    reqwest::blocking::Client::builder()
        .user_agent("soli-artifact-updater")
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .build()
        .map_err(|e| format!("auto-update: HTTP client init failed: {}", e))
}

/// Fetch and parse the channel manifest.
pub fn fetch_manifest(descriptor: &UpdateDescriptor) -> Result<UpdateManifest, String> {
    let base = descriptor.update_url.trim_end_matches('/');
    if !base.starts_with("https://") && !base.starts_with("http://") {
        return Err(format!(
            "auto-update: update_url must be http(s), got '{}'",
            base
        ));
    }
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

fn verify_signature(manifest: &UpdateManifest, pubkey_b64: Option<&str>) -> Result<(), String> {
    let Some(pubkey_b64) = pubkey_b64 else {
        // No key embedded: unsigned updates permitted, loudly.
        eprintln!(
            "  \x1b[33mWarning:\x1b[0m this artifact was built without --update-key, so the \
             update manifest is trusted without a signature. Anyone who can serve the update \
             URL can push a binary. Build with --update-key for production."
        );
        return Ok(());
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

fn download_and_verify(entry: &ArtifactEntry) -> Result<Vec<u8>, String> {
    if !entry.url.starts_with("https://") && !entry.url.starts_with("http://") {
        return Err(format!(
            "auto-update: artifact url must be http(s): {}",
            entry.url
        ));
    }
    let mut response = http_client()?
        .get(&entry.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("auto-update: download failed: {}", e))?;

    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(|e| format!("auto-update: download read failed: {}", e))?;

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

    std::fs::write(&staged, bytes)
        .map_err(|e| format!("auto-update: cannot stage new binary: {}", e))?;

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
// Developer tooling: keygen + manifest signing.
// ---------------------------------------------------------------------------

/// A fresh P-256 keypair: (private PKCS#8 PEM, public base64 SEC1).
pub fn generate_keypair() -> Result<(String, String), String> {
    use p256::pkcs8::EncodePrivateKey;
    let signing = SigningKey::random(&mut rand_core::OsRng);
    let private_pem = signing
        .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
        .map_err(|e| format!("keygen: {}", e))?
        .to_string();
    let verifying = VerifyingKey::from(&signing);
    let public_b64 = B64.encode(verifying.to_encoded_point(false).as_bytes());
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
        let signing = SigningKey::random(&mut rand_core::OsRng);
        let verifying = VerifyingKey::from(&signing);
        let pubkey = B64.encode(verifying.to_encoded_point(false).as_bytes());

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
}
