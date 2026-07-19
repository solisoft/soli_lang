//! The manifest that marks a bundle as a desktop application.
//!
//! Its presence is how boot distinguishes a desktop artifact from an ordinary
//! standalone one, so it is also the compatibility boundary: an older runtime
//! meeting a newer manifest must refuse rather than guess.

use serde::{Deserialize, Serialize};

/// Bundle entry holding the manifest. Its presence selects the desktop boot
/// path.
pub const MANIFEST_ENTRY: &str = "__desktop__/manifest.json";
/// Bundle entry holding the encrypted application payload (a `SOLE` container).
pub const APP_ENTRY: &str = "__app__.sole";
/// Bundle entry holding the database executable for this target.
pub const DB_BINARY_ENTRY: &str = "__runtime__/solidb";
/// Prefix for read-only reference data entries.
pub const SEED_PREFIX: &str = "__seed__/";

/// Manifest format version. Bumped only for a breaking layout change.
pub const MANIFEST_VERSION: u32 = 1;

/// Build-time facts a desktop artifact carries about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopManifest {
    /// Layout version. A runtime that does not recognise it must refuse.
    pub manifest_version: u32,
    /// Reverse-DNS application identity. Determines the per-OS data directory,
    /// so changing it between releases orphans the user's existing data.
    pub app_id: String,
    /// Human-readable name, for window titles and error messages.
    pub app_name: String,
    /// The soli version that built this artifact.
    pub soli_version: String,
    /// Version of the embedded database binary, for diagnostics.
    pub solidb_version: String,
    /// SHA-256 of the embedded database binary, hex-encoded.
    ///
    /// Checked before the binary is executed. The binary is extracted to a
    /// cache directory and reused across launches, so it lives somewhere the
    /// user can write — verifying on every launch is what makes reuse safe.
    pub solidb_sha256: String,
    /// How the embedded database binary is stored, if compressed.
    ///
    /// `None` means stored verbatim — the shape older artifacts have, so
    /// reading one still works. `Some("deflate")` roughly thirds the binary,
    /// which dominates artifact size.
    #[serde(default)]
    pub db_compression: Option<String>,
    /// Identifier for the shipped reference data, used to decide whether an
    /// installed copy is current. `None` when the app ships no seed data.
    pub seed_version: Option<String>,
    /// SHA-256 over the whole seed set. Re-importing is driven by this
    /// changing, not by the version string, so a rebuilt seed with an unchanged
    /// version is still detected.
    pub seed_sha256: Option<String>,
}

impl DesktopManifest {
    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec_pretty(self).map_err(|e| format!("cannot serialize manifest: {}", e))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let manifest: DesktopManifest = serde_json::from_slice(bytes)
            .map_err(|e| format!("invalid desktop manifest: {}", e))?;
        if manifest.manifest_version > MANIFEST_VERSION {
            return Err(format!(
                "this application was built for desktop manifest version {}, but this runtime \
                 understands up to version {} — the runtime is older than the app",
                manifest.manifest_version, MANIFEST_VERSION
            ));
        }
        if manifest.app_id.is_empty() {
            return Err("desktop manifest has an empty app_id".to_string());
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DesktopManifest {
        DesktopManifest {
            manifest_version: MANIFEST_VERSION,
            app_id: "com.example.app".to_string(),
            app_name: "Example".to_string(),
            soli_version: "1.22.0".to_string(),
            solidb_version: "0.31.0".to_string(),
            solidb_sha256: "ab".repeat(32),
            db_compression: Some("deflate".to_string()),
            seed_version: Some("2026-07-19".to_string()),
            seed_sha256: Some("cd".repeat(32)),
        }
    }

    #[test]
    fn round_trips() {
        let manifest = sample();
        let json = manifest.to_json().expect("serialize");
        assert_eq!(DesktopManifest::from_json(&json).expect("parse"), manifest);
    }

    #[test]
    fn refuses_a_manifest_from_a_newer_runtime() {
        // Forward compatibility cannot be guessed at: a newer layout may move
        // entries this runtime would then silently fail to find.
        let mut manifest = sample();
        manifest.manifest_version = MANIFEST_VERSION + 1;
        let json = manifest.to_json().expect("serialize");
        let err = DesktopManifest::from_json(&json).expect_err("must refuse");
        assert!(
            err.contains("older than the app"),
            "error should explain which side is out of date, got: {}",
            err
        );
    }

    #[test]
    fn rejects_an_empty_app_id() {
        let mut manifest = sample();
        manifest.app_id = String::new();
        let json = manifest.to_json().expect("serialize");
        assert!(DesktopManifest::from_json(&json).is_err());
    }

    #[test]
    fn a_manifest_without_compression_still_parses() {
        // Artifacts built before compression existed store the binary verbatim
        // and carry no such field; they must keep working.
        let json = br#"{
            "manifest_version": 1,
            "app_id": "com.example.app",
            "app_name": "Example",
            "soli_version": "1.22.0",
            "solidb_version": "0.31.0",
            "solidb_sha256": "aa",
            "seed_version": null,
            "seed_sha256": null
        }"#;
        let manifest = DesktopManifest::from_json(json).expect("parse");
        assert_eq!(manifest.db_compression, None);
    }

    #[test]
    fn seed_fields_are_optional() {
        let mut manifest = sample();
        manifest.seed_version = None;
        manifest.seed_sha256 = None;
        let json = manifest.to_json().expect("serialize");
        assert_eq!(DesktopManifest::from_json(&json).expect("parse"), manifest);
    }
}
