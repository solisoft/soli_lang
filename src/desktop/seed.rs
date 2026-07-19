//! Importing shipped read-only reference data.
//!
//! Reference data and the user's own data share one database, which makes this
//! the most destructive code in the desktop path: importing replaces a
//! collection wholesale, so importing into the wrong one silently destroys the
//! user's work.
//!
//! Two guards, and neither is optional:
//!
//! 1. **Seed collections must be named `ref_*`**, enforced at build time. An
//!    app's own models therefore cannot collide with a seed collection by
//!    accident — the namespaces are disjoint by construction rather than by
//!    review.
//! 2. **Only collections named in this build's seed are touched.** Nothing
//!    walks the database looking for things to clear.
//!
//! Import is skipped when the data has not changed, tracked by a watermark
//! keyed on the content digest. Re-importing on every launch would be slow and
//! would pointlessly churn the storage engine.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::db::DbCredentials;
use super::manifest::DesktopManifest;

/// Required prefix for a seed collection name.
pub const SEED_COLLECTION_PREFIX: &str = "ref_";

/// Reject a seed collection the app's own models could collide with.
///
/// Called at build time so a bad name fails the developer's build rather than
/// the user's data.
pub fn validate_collection_name(name: &str) -> Result<(), String> {
    if !name.starts_with(SEED_COLLECTION_PREFIX) {
        return Err(format!(
            "seed collection '{}' must be named '{}{}' — importing replaces a collection \
             wholesale, and the prefix is what stops it ever overwriting one of your models",
            name, SEED_COLLECTION_PREFIX, name
        ));
    }
    if name.len() <= SEED_COLLECTION_PREFIX.len() {
        return Err(format!(
            "seed collection '{}' needs a name after the '{}' prefix",
            name, SEED_COLLECTION_PREFIX
        ));
    }
    Ok(())
}

fn watermark_path(state_dir: &Path) -> PathBuf {
    state_dir.join("seed.json")
}

/// Whether the shipped reference data differs from what is installed.
///
/// Keyed on the content digest rather than the version string, so rebuilding
/// changed data is detected even if nobody bumped a version.
pub fn needs_import(state_dir: &Path, manifest: &DesktopManifest) -> bool {
    let Some(expected) = manifest.seed_sha256.as_deref() else {
        return false; // nothing shipped
    };
    let Ok(raw) = std::fs::read_to_string(watermark_path(state_dir)) else {
        return true; // never imported
    };
    let installed = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("sha256").and_then(|s| s.as_str()).map(String::from));
    installed.as_deref() != Some(expected)
}

/// Record that this build's reference data is installed.
///
/// Written only after a successful import, so a failure part-way leaves the
/// watermark stale and the next launch retries rather than assuming success.
pub fn record_watermark(state_dir: &Path, manifest: &DesktopManifest) -> Result<(), String> {
    let body = serde_json::json!({
        "seed_version": manifest.seed_version,
        "sha256": manifest.seed_sha256,
    })
    .to_string();
    std::fs::write(watermark_path(state_dir), body)
        .map_err(|e| format!("cannot record seed watermark: {}", e))
}

/// Parse NDJSON into one JSON value per non-blank line.
///
/// Reports the line number on failure: in a file of thousands of records,
/// "invalid JSON" without a location is close to useless.
pub fn parse_ndjson(bytes: &[u8]) -> Result<Vec<serde_json::Value>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("seed data is not UTF-8: {}", e))?;
    let mut docs = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("line {}: {}", index + 1, e))?;
        docs.push(value);
    }
    Ok(docs)
}

/// Import every seed collection, replacing its current contents.
pub fn import(
    host: &str,
    database: &str,
    credentials: &DbCredentials,
    seed: &[(String, Vec<u8>)],
) -> Result<(), String> {
    if seed.is_empty() {
        return Ok(());
    }

    // Re-validate at import time as well as build time. This is the last point
    // before data is destroyed, and the cost is a string comparison.
    for (name, _) in seed {
        validate_collection_name(name)?;
    }

    let token = login(host, credentials)?;

    // Seeding runs before the app has served anything, so the database the
    // model layer would normally auto-create on first use does not exist yet —
    // without this every collection call 404s.
    ensure_database(host, database, &token)?;

    for (name, bytes) in seed {
        let docs = parse_ndjson(bytes).map_err(|e| format!("seed '{}': {}", name, e))?;
        ensure_collection(host, database, &token, name)?;
        truncate(host, database, &token, name)?;
        for (index, doc) in docs.iter().enumerate() {
            insert(host, database, &token, name, doc)
                .map_err(|e| format!("seed '{}' record {}: {}", name, index + 1, e))?;
        }
        println!("  Imported {} ({} records)", name, docs.len());
    }

    Ok(())
}

fn login(host: &str, credentials: &DbCredentials) -> Result<String, String> {
    let response = ureq::post(&format!("{}/auth/login", host))
        .timeout(Duration::from_secs(10))
        .send_json(ureq::json!({
            "username": credentials.username,
            "password": credentials.password,
        }))
        .map_err(|e| format!("cannot authenticate with the database: {}", e))?;

    let body: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("unreadable login response: {}", e))?;
    body.get("token")
        .and_then(|t| t.as_str())
        .map(String::from)
        .ok_or_else(|| "login response carried no token".to_string())
}

/// Create the database, treating "already exists" as success.
///
/// Creation is a POST to the singular `/_api/database`; the plural
/// `/_api/databases` is list-only.
fn ensure_database(host: &str, db: &str, token: &str) -> Result<(), String> {
    let result = ureq::post(&format!("{}/_api/database", host))
        .set("Authorization", &format!("Bearer {}", token))
        .timeout(Duration::from_secs(10))
        .send_json(ureq::json!({ "name": db }));

    match result {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(409, _)) | Err(ureq::Error::Status(400, _)) => Ok(()),
        Err(e) => Err(format!("cannot create database '{}': {}", db, e)),
    }
}

/// Create the collection, treating "already exists" as success.
fn ensure_collection(host: &str, db: &str, token: &str, name: &str) -> Result<(), String> {
    let result = ureq::post(&format!("{}/_api/database/{}/collection", host, db))
        .set("Authorization", &format!("Bearer {}", token))
        .timeout(Duration::from_secs(10))
        .send_json(ureq::json!({ "name": name }));

    match result {
        Ok(_) => Ok(()),
        // A collection that already exists is the normal case on re-import.
        Err(ureq::Error::Status(409, _)) | Err(ureq::Error::Status(400, _)) => Ok(()),
        Err(e) => Err(format!("cannot create collection '{}': {}", name, e)),
    }
}

fn truncate(host: &str, db: &str, token: &str, name: &str) -> Result<(), String> {
    ureq::put(&format!(
        "{}/_api/database/{}/collection/{}/truncate",
        host, db, name
    ))
    .set("Authorization", &format!("Bearer {}", token))
    .timeout(Duration::from_secs(30))
    .call()
    .map(|_| ())
    .map_err(|e| format!("cannot clear collection '{}': {}", name, e))
}

fn insert(
    host: &str,
    db: &str,
    token: &str,
    name: &str,
    doc: &serde_json::Value,
) -> Result<(), String> {
    ureq::post(&format!("{}/_api/database/{}/document/{}", host, db, name))
        .set("Authorization", &format!("Bearer {}", token))
        .timeout(Duration::from_secs(10))
        .send_json(doc.clone())
        .map(|_| ())
        .map_err(|e| format!("{}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::manifest::MANIFEST_VERSION;

    fn manifest_with(sha: Option<&str>) -> DesktopManifest {
        DesktopManifest {
            manifest_version: MANIFEST_VERSION,
            app_id: "com.example.app".to_string(),
            app_name: "Example".to_string(),
            soli_version: "1.22.0".to_string(),
            solidb_version: "0.31.0".to_string(),
            solidb_sha256: "ab".repeat(32),
            db_compression: None,
            seed_version: Some("v1".to_string()),
            seed_sha256: sha.map(String::from),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("soli_seed_test_{}_{}", std::process::id(), name));
        std::fs::create_dir_all(&p).expect("scratch dir");
        p
    }

    #[test]
    fn collection_names_must_carry_the_reference_prefix() {
        // The prefix is the only thing standing between a seed import and the
        // user's own data, so anything without it must fail the build.
        assert!(validate_collection_name("ref_countries").is_ok());
        for bad in ["users", "countries", "ref_", "myref_x"] {
            assert!(
                validate_collection_name(bad).is_err(),
                "'{}' should be rejected",
                bad
            );
        }
    }

    #[test]
    fn the_rejection_explains_why_the_prefix_exists() {
        let err = validate_collection_name("users").expect_err("rejected");
        assert!(
            err.contains("overwriting one of your models"),
            "error should say what the prefix protects, got: {}",
            err
        );
    }

    #[test]
    fn import_is_needed_only_when_the_content_changed() {
        let dir = scratch("watermark");
        let manifest = manifest_with(Some("aa".repeat(32).as_str()));

        // Never imported.
        assert!(needs_import(&dir, &manifest));

        record_watermark(&dir, &manifest).expect("record");
        assert!(
            !needs_import(&dir, &manifest),
            "unchanged data must be skipped"
        );

        // Rebuilt data with the same version string must still be detected —
        // which is why the watermark keys on the digest, not the version.
        let mut changed = manifest.clone();
        changed.seed_sha256 = Some("bb".repeat(32));
        assert!(needs_import(&dir, &changed));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_app_without_seed_data_never_imports() {
        let dir = scratch("noseed");
        assert!(!needs_import(&dir, &manifest_with(None)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_ndjson_skipping_blank_lines() {
        let docs = parse_ndjson(b"{\"a\":1}\n\n  \n{\"b\":2}\n").expect("parse");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0]["a"], 1);
        assert_eq!(docs[1]["b"], 2);
    }

    #[test]
    fn a_bad_record_names_its_line() {
        // Locating one broken record in a file of thousands is the whole point.
        let err = parse_ndjson(b"{\"a\":1}\n{oops}\n").expect_err("must fail");
        assert!(err.starts_with("line 2:"), "unexpected error: {}", err);
    }

    #[test]
    fn importing_nothing_succeeds_without_touching_the_database() {
        // No host is supplied, so this would fail loudly if it tried to connect.
        assert!(import(
            "http://127.0.0.1:1",
            "db",
            &DbCredentials {
                username: "admin".to_string(),
                password: "x".to_string(),
            },
            &[]
        )
        .is_ok());
    }

    #[test]
    fn import_refuses_an_unprefixed_collection_before_connecting() {
        // Validation must happen before any network call: the check exists to
        // prevent destruction, so it cannot run after truncation starts.
        let err = import(
            "http://127.0.0.1:1",
            "db",
            &DbCredentials {
                username: "admin".to_string(),
                password: "x".to_string(),
            },
            &[("users".to_string(), b"{}\n".to_vec())],
        )
        .expect_err("must refuse");
        assert!(err.contains("must be named"), "unexpected error: {}", err);
    }
}
