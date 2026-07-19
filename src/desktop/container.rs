//! Assembling and opening the desktop artifact payload.
//!
//! The payload is an ordinary plain `SOLB` bundle with reserved entries:
//!
//! ```text
//! __desktop__/manifest.json   build-time facts; its presence selects this path
//! __app__.sole                the encrypted application (a SOLE container)
//! __runtime__/solidb          the database binary for this target
//! __seed__/<name>.ndjson      read-only reference data
//! ```
//!
//! Composing existing containers rather than inventing a format means the
//! executable footer, the encryption container and the bundle reader all work
//! unchanged.
//!
//! The database binary and seed data sit *outside* the encrypted payload
//! deliberately. Encrypting a published, publicly downloadable database binary
//! on every launch costs real time and memory and protects nothing; the
//! application source keeps full encryption.

use std::collections::HashMap;

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::bundle::BundleReader;

use super::manifest::{DesktopManifest, APP_ENTRY, DB_BINARY_ENTRY, MANIFEST_ENTRY, SEED_PREFIX};

/// Inputs for assembling a desktop payload.
pub struct ContainerInputs {
    /// The already-encrypted application payload (`SOLE` bytes).
    pub encrypted_app: Vec<u8>,
    /// The database executable for the build target.
    pub db_binary: Vec<u8>,
    /// Reference data, as `(collection_name, ndjson_bytes)`.
    pub seed: Vec<(String, Vec<u8>)>,
    /// Everything except the checksums, which are computed here so they cannot
    /// disagree with the bytes actually embedded.
    pub manifest: DesktopManifest,
}

/// Value of `manifest.db_compression` for a deflated binary.
const DEFLATE: &str = "deflate";

/// Hex-encoded SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// A checksum over the whole seed set.
///
/// Order-independent by construction: entries are sorted by name before
/// hashing, and each contributes its name and length as well as its bytes, so
/// renaming a collection or moving a row between collections changes the digest
/// even when the concatenated content would not.
pub fn seed_digest(seed: &[(String, Vec<u8>)]) -> String {
    let mut sorted: Vec<&(String, Vec<u8>)> = seed.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (name, bytes) in sorted {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Build the payload bytes, ready to be appended to a runtime executable.
///
/// The manifest's checksums are overwritten with digests of the bytes actually
/// embedded, so a caller cannot ship a manifest that disagrees with its
/// content — which would surface at launch as a verification failure the user
/// could do nothing about.
pub fn build(mut inputs: ContainerInputs) -> Result<Vec<u8>, String> {
    if inputs.encrypted_app.is_empty() {
        return Err("desktop container needs an application payload".to_string());
    }
    if inputs.db_binary.is_empty() {
        return Err("desktop container needs a database binary".to_string());
    }

    // The checksum covers the *uncompressed* bytes — what actually gets
    // executed — so it stays meaningful regardless of how they are stored.
    inputs.manifest.solidb_sha256 = sha256_hex(&inputs.db_binary);

    // The database binary dominates artifact size; deflate takes it to roughly
    // a third. Compressing only this entry keeps the outer bundle a plain SOLB
    // that existing readers still parse.
    let stored_db = {
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&inputs.db_binary)
            .and_then(|()| encoder.finish())
            .map_err(|e| format!("cannot compress the database binary: {}", e))?
    };
    inputs.manifest.db_compression = Some(DEFLATE.to_string());
    inputs.manifest.seed_sha256 = if inputs.seed.is_empty() {
        None
    } else {
        Some(seed_digest(&inputs.seed))
    };

    let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
    entries.insert(MANIFEST_ENTRY.to_string(), inputs.manifest.to_json()?);
    entries.insert(APP_ENTRY.to_string(), inputs.encrypted_app);
    entries.insert(DB_BINARY_ENTRY.to_string(), stored_db);

    for (name, bytes) in inputs.seed {
        validate_seed_name(&name)?;
        // Fail the developer's build rather than the user's data: importing
        // replaces a collection wholesale, and the prefix is what keeps seed
        // collections disjoint from the app's own models.
        crate::desktop::seed::validate_collection_name(&name)?;
        entries.insert(format!("{}{}.ndjson", SEED_PREFIX, name), bytes);
    }

    crate::bundle::BundleBuilder::serialize_entries(&entries)
}

/// Seed collection names become bundle paths and, later, database collection
/// names, so keep them to an unambiguous shape.
fn validate_seed_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("seed collection name must not be empty".to_string());
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
    {
        return Err(format!(
            "seed collection '{}' contains an invalid character {:?} — use letters, digits, '_' or '-'",
            name, bad
        ));
    }
    Ok(())
}

/// A parsed desktop payload.
#[derive(Debug)]
pub struct DesktopContainer<'a> {
    pub manifest: DesktopManifest,
    pub encrypted_app: &'a [u8],
    /// Decompressed and verified, so callers never see the stored form.
    pub db_binary: Vec<u8>,
    /// Reference data as `(collection_name, ndjson_bytes)`.
    pub seed: Vec<(String, &'a [u8])>,
}

/// Whether a plain bundle carries a desktop manifest.
///
/// Returns `false` rather than erroring for anything unparseable, because the
/// caller's next move is to try the ordinary standalone path, which will
/// produce a better-targeted error.
pub fn is_desktop_payload(payload: &[u8]) -> bool {
    BundleReader::new(payload)
        .map(|reader| reader.get(MANIFEST_ENTRY).is_some())
        .unwrap_or(false)
}

/// Parse a desktop payload, verifying the embedded database binary.
pub fn open(payload: &[u8]) -> Result<DesktopContainer<'_>, String> {
    let reader = BundleReader::new(payload)?;

    let manifest_bytes = reader
        .get(MANIFEST_ENTRY)
        .ok_or_else(|| "not a desktop application: no manifest".to_string())?;
    let manifest = DesktopManifest::from_json(manifest_bytes)?;

    let encrypted_app = reader
        .get(APP_ENTRY)
        .ok_or_else(|| format!("desktop application is missing its payload ({})", APP_ENTRY))?;
    let db_binary = reader.get(DB_BINARY_ENTRY).ok_or_else(|| {
        format!(
            "desktop application is missing its database binary ({})",
            DB_BINARY_ENTRY
        )
    })?;

    // Restore the stored form before verifying: the checksum is over the bytes
    // that will be executed, not over however they happen to be packed.
    let db_binary = match manifest.db_compression.as_deref() {
        None => db_binary.to_vec(),
        Some(DEFLATE) => {
            use std::io::Read;
            let mut out = Vec::new();
            ZlibDecoder::new(db_binary)
                .read_to_end(&mut out)
                .map_err(|e| format!("cannot decompress the database binary: {}", e))?;
            out
        }
        Some(other) => {
            return Err(format!(
                "this application stores its database binary as '{}', which this runtime \
                 does not understand — the runtime is older than the app",
                other
            ))
        }
    };

    // Verify before anything executes these bytes. A mismatch means the
    // artifact was truncated or altered after it was built.
    let actual = sha256_hex(&db_binary);
    if actual != manifest.solidb_sha256 {
        return Err(format!(
            "embedded database binary failed verification: manifest expects {}, found {} \
             — this application has been modified since it was built",
            manifest.solidb_sha256, actual
        ));
    }

    let mut seed: Vec<(String, &[u8])> = Vec::new();
    for (path, bytes) in reader.entries() {
        if let Some(rest) = path.strip_prefix(SEED_PREFIX) {
            let name = rest.strip_suffix(".ndjson").unwrap_or(rest);
            seed.push((name.to_string(), bytes));
        }
    }
    seed.sort_by(|a, b| a.0.cmp(&b.0));

    if !seed.is_empty() {
        let owned: Vec<(String, Vec<u8>)> = seed
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.to_vec()))
            .collect();
        let actual_seed = seed_digest(&owned);
        match manifest.seed_sha256.as_deref() {
            Some(expected) if expected == actual_seed => {}
            Some(expected) => {
                return Err(format!(
                    "embedded reference data failed verification: manifest expects {}, found {}",
                    expected, actual_seed
                ))
            }
            None => {
                return Err(
                    "desktop application carries reference data but no checksum for it".to_string(),
                )
            }
        }
    }

    Ok(DesktopContainer {
        manifest,
        encrypted_app,
        db_binary,
        seed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::manifest::MANIFEST_VERSION;

    fn manifest() -> DesktopManifest {
        DesktopManifest {
            manifest_version: MANIFEST_VERSION,
            app_id: "com.example.app".to_string(),
            app_name: "Example".to_string(),
            soli_version: "1.22.0".to_string(),
            solidb_version: "0.31.0".to_string(),
            // Deliberately wrong: `build` must overwrite it.
            solidb_sha256: "00".repeat(32),
            db_compression: None,
            seed_version: Some("v1".to_string()),
            seed_sha256: None,
        }
    }

    fn inputs() -> ContainerInputs {
        ContainerInputs {
            encrypted_app: b"SOLE\x01 pretend ciphertext".to_vec(),
            db_binary: b"#!/bin/sh\necho pretend-database\n".to_vec(),
            seed: vec![
                ("ref_countries".to_string(), b"{\"code\":\"FR\"}\n".to_vec()),
                (
                    "ref_currencies".to_string(),
                    b"{\"code\":\"EUR\"}\n".to_vec(),
                ),
            ],
            manifest: manifest(),
        }
    }

    #[test]
    fn round_trips_through_build_and_open() {
        let original = inputs();
        let expected_app = original.encrypted_app.clone();
        let expected_db = original.db_binary.clone();

        let payload = build(original).expect("build");
        assert!(is_desktop_payload(&payload));

        let container = open(&payload).expect("open");
        assert_eq!(container.encrypted_app, &expected_app[..]);
        assert_eq!(container.db_binary, &expected_db[..]);
        assert_eq!(container.manifest.app_id, "com.example.app");
        assert_eq!(
            container
                .seed
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            vec!["ref_countries", "ref_currencies"]
        );
    }

    #[test]
    fn build_overwrites_checksums_with_the_embedded_bytes() {
        // The caller supplied a bogus digest; shipping it would fail
        // verification at launch, where the user can do nothing about it.
        let payload = build(inputs()).expect("build");
        let container = open(&payload).expect("open");
        assert_eq!(
            container.manifest.solidb_sha256,
            sha256_hex(b"#!/bin/sh\necho pretend-database\n")
        );
        assert!(container.manifest.seed_sha256.is_some());
    }

    #[test]
    fn open_rejects_a_tampered_database_binary() {
        // Rebuild the payload with the stored binary altered but the manifest's
        // checksum left alone — what a modified artifact looks like. The binary
        // is extracted to a user-writable cache and executed, so this check is
        // what makes reusing it across launches safe.
        let payload = build(inputs()).expect("build");
        let reader = crate::bundle::BundleReader::new(&payload).expect("read");

        let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
        for (path, bytes) in reader.entries() {
            let mut content = bytes.to_vec();
            if path == DB_BINARY_ENTRY {
                // Flip a byte inside the stored (compressed) binary.
                let at = content.len() / 2;
                content[at] ^= 0xff;
            }
            entries.insert(path.clone(), content);
        }
        let tampered =
            crate::bundle::BundleBuilder::serialize_entries(&entries).expect("re-serialize");

        let err = open(&tampered).expect_err("must refuse a modified binary");
        // Either the compressed stream no longer decodes, or it decodes to
        // something whose digest does not match. Both are correct refusals;
        // what matters is that neither is silently accepted.
        assert!(
            err.contains("failed verification") || err.contains("cannot decompress"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn compression_shrinks_the_stored_binary_but_not_what_is_returned() {
        // The binary dominates artifact size, so it is stored deflated; callers
        // must still receive the exact bytes that will be executed.
        let mut i = inputs();
        // Repetitive content so compression is unambiguous.
        i.db_binary = b"AAAABBBBCCCC".repeat(500).to_vec();
        let original = i.db_binary.clone();

        let payload = build(i).expect("build");
        let reader = crate::bundle::BundleReader::new(&payload).expect("read");
        let stored = reader.get(DB_BINARY_ENTRY).expect("db entry");
        assert!(
            stored.len() < original.len(),
            "stored {} bytes vs original {} — compression did not apply",
            stored.len(),
            original.len()
        );

        let container = open(&payload).expect("open");
        assert_eq!(container.db_binary, original, "must round-trip exactly");
        assert_eq!(
            container.manifest.db_compression.as_deref(),
            Some("deflate")
        );
    }

    #[test]
    fn open_refuses_an_unknown_compression_scheme() {
        // A newer artifact may store the binary some other way. Guessing would
        // mean executing bytes we did not decode correctly.
        let payload = build(inputs()).expect("build");
        let reader = crate::bundle::BundleReader::new(&payload).expect("read");

        let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
        for (path, bytes) in reader.entries() {
            entries.insert(path.clone(), bytes.to_vec());
        }
        let mut manifest =
            DesktopManifest::from_json(entries[MANIFEST_ENTRY].as_slice()).expect("parse manifest");
        manifest.db_compression = Some("zstd-from-the-future".to_string());
        entries.insert(
            MANIFEST_ENTRY.to_string(),
            manifest.to_json().expect("json"),
        );
        let future =
            crate::bundle::BundleBuilder::serialize_entries(&entries).expect("re-serialize");

        let err = open(&future).expect_err("must refuse");
        assert!(
            err.contains("older than the app"),
            "error should say which side is out of date, got: {}",
            err
        );
    }

    #[test]
    fn seed_digest_is_order_independent_but_name_sensitive() {
        let a = vec![
            ("one".to_string(), b"x".to_vec()),
            ("two".to_string(), b"y".to_vec()),
        ];
        let reordered = vec![
            ("two".to_string(), b"y".to_vec()),
            ("one".to_string(), b"x".to_vec()),
        ];
        assert_eq!(seed_digest(&a), seed_digest(&reordered));

        // Moving content between collections must change the digest, or a
        // reshuffled seed would not trigger a re-import.
        let moved = vec![
            ("one".to_string(), b"y".to_vec()),
            ("two".to_string(), b"x".to_vec()),
        ];
        assert_ne!(seed_digest(&a), seed_digest(&moved));
    }

    #[test]
    fn rejects_seed_collections_without_the_reference_prefix() {
        // A seed named after one of the app's own models would wipe it.
        let mut i = inputs();
        i.seed = vec![("users".to_string(), b"{}\n".to_vec())];
        let err = build(i).expect_err("must refuse");
        assert!(err.contains("must be named"), "unexpected error: {}", err);
    }

    #[test]
    fn rejects_seed_names_that_are_not_plain_identifiers() {
        for bad in ["../escape", "with/slash", "", "dot.name"] {
            let mut i = inputs();
            i.seed = vec![(bad.to_string(), b"{}\n".to_vec())];
            assert!(build(i).is_err(), "seed name {:?} should be rejected", bad);
        }
    }

    #[test]
    fn requires_both_an_app_and_a_database() {
        let mut no_app = inputs();
        no_app.encrypted_app = Vec::new();
        assert!(build(no_app).is_err());

        let mut no_db = inputs();
        no_db.db_binary = Vec::new();
        assert!(build(no_db).is_err());
    }

    #[test]
    fn a_plain_bundle_is_not_a_desktop_payload() {
        let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
        entries.insert(
            "app/controllers/home.sl".to_string(),
            b"def index {}".to_vec(),
        );
        let plain = crate::bundle::BundleBuilder::serialize_entries(&entries).expect("serialize");
        assert!(!is_desktop_payload(&plain));
        assert!(open(&plain).is_err());
    }
}
