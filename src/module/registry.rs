//! HTTP client for the Soli package registry.
//!
//! Communicates with the registry API to resolve versions,
//! download packages, and publish new packages.

use std::fs;
use std::io;
use std::path::Path;

/// Default registry URL.
pub const DEFAULT_REGISTRY: &str = "https://ilos.solisoft.net";

/// Version metadata returned by the registry.
#[derive(Debug)]
pub struct VersionInfo {
    /// Download URL for the package tarball
    pub download_url: String,
}

/// Resolve a package version from the registry.
///
/// GET {registry}/api/packages/{name}/{version}
pub fn resolve_version(
    registry_url: &str,
    name: &str,
    version: &str,
) -> Result<VersionInfo, String> {
    let api_url = format!("{}/api/packages/{}/{}", registry_url, name, version);

    // SEC-007a carve-out: package registries may redirect to a CDN, and
    // this code path runs only from the developer-driven `soli install`
    // CLI command (no request-level SSRF surface). Use raw `ureq::get`
    // so redirect-following keeps working.
    let response = ureq::get(&api_url)
        .set("User-Agent", "soli-package-manager")
        .call()
        .map_err(|e| {
            format!(
                "Failed to resolve '{}@{}' from registry: {}",
                name, version, e
            )
        })?;

    let body: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("Failed to parse registry response: {}", e))?;

    let download_url = body["download_url"]
        .as_str()
        .ok_or_else(|| {
            format!(
                "Registry response missing 'download_url' for '{}@{}'",
                name, version
            )
        })?
        .to_string();

    Ok(VersionInfo { download_url })
}

/// Download and extract a package tarball.
///
/// Downloads from the given URL and extracts to dest_dir.
/// Registry tarballs are flat (no top-level directory to strip).
pub fn download_package(url: &str, dest: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;

    // See `resolve_version` for the rationale: registry CDNs redirect,
    // CLI-trust context, no request-level SSRF surface.
    let response = ureq::get(url)
        .set("User-Agent", "soli-package-manager")
        .call()
        .map_err(|e| format!("Failed to download package: {}", e))?;

    let reader = response.into_reader();
    let decoder = GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);

    // Create destination directory
    fs::create_dir_all(dest).map_err(|e| format!("Failed to create cache directory: {}", e))?;

    extract_archive(&mut archive, dest)
}

/// Extract a tarball into `dest`, refusing any entry whose path contains
/// non-`Normal` components (absolute roots, `..`, prefixes), and refusing
/// symlink / hardlink entries outright. SEC-075: a malicious or compromised
/// registry tarball would otherwise overwrite arbitrary files outside the
/// package cache when `soli install` runs.
fn extract_archive<R: io::Read>(archive: &mut tar::Archive<R>, dest: &Path) -> Result<(), String> {
    for entry in archive
        .entries()
        .map_err(|e| format!("Failed to read archive entries: {}", e))?
    {
        let mut entry = entry.map_err(|e| format!("Failed to read archive entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("Failed to read entry path: {}", e))?
            .into_owned();

        if path.iter().count() == 0 {
            continue;
        }

        validate_archive_entry_path(&path)?;

        let entry_type = entry.header().entry_type();
        // Symlinks and hardlinks can point outside `dest` regardless of how
        // safe the entry path itself looks. Refuse them — the registry does
        // not need to ship link entries.
        if entry_type == tar::EntryType::Symlink || entry_type == tar::EntryType::Link {
            return Err(format!(
                "Refusing to extract link entry from package tarball: {}",
                path.display()
            ));
        }

        let mut out_path = dest.to_path_buf();
        for component in path.iter() {
            out_path.push(component);
        }

        if entry_type == tar::EntryType::Directory {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                // Defence in depth: even after rejecting `..`/root components
                // above, re-canonicalise and confirm the resolved parent is
                // still under `dest`. The only way to reach this branch with
                // an unsafe parent is via a pre-existing symlink in the
                // ancestor chain — also worth refusing.
                if let (Ok(canon_parent), Ok(canon_root)) =
                    (fs::canonicalize(parent), fs::canonicalize(dest))
                {
                    if !canon_parent.starts_with(&canon_root) {
                        return Err(format!(
                            "Refusing to extract package entry that resolves outside the cache directory: {}",
                            path.display()
                        ));
                    }
                }
            }
            let mut out_file =
                fs::File::create(&out_path).map_err(|e| format!("Failed to create file: {}", e))?;
            io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
        }
    }

    Ok(())
}

/// Reject tarball entry paths that contain anything other than plain
/// `Normal(_)` segments. Mirrors the SEC-011 guard in
/// `src/scaffold/app_generator.rs`: refuses absolute roots, `..` segments,
/// and any non-normal path component before the loop joins the entry onto
/// the destination.
fn validate_archive_entry_path(path: &Path) -> Result<(), String> {
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "Refusing to extract package entry with unsafe path: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

/// Publish a package to the registry.
///
/// POST {registry}/api/packages with multipart form data.
pub fn publish_package(
    registry_url: &str,
    token: &str,
    name: &str,
    version: &str,
    description: &str,
    tarball_path: &Path,
) -> Result<(), String> {
    let api_url = format!("{}/api/packages", registry_url);

    let form = reqwest::blocking::multipart::Form::new()
        .text("name", name.to_string())
        .text("version", version.to_string())
        .text("description", description.to_string())
        .file("tarball", tarball_path)
        .map_err(|e| format!("Failed to read tarball: {}", e))?;

    let client = reqwest::blocking::Client::new();
    let response = client
        .post(&api_url)
        .header("Authorization", format!("Bearer {}", token))
        .multipart(form)
        .send()
        .map_err(|e| format!("Failed to publish package: {}", e))?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = response.text().unwrap_or_default();
        Err(format!(
            "Registry returned {} when publishing '{}@{}': {}",
            status, name, version, body
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn validator_accepts_normal_relative_paths() {
        assert!(validate_archive_entry_path(Path::new("src/lib.sl")).is_ok());
        assert!(validate_archive_entry_path(Path::new("README.md")).is_ok());
        assert!(validate_archive_entry_path(Path::new("a/b/c/d/e")).is_ok());
    }

    #[test]
    fn validator_rejects_parent_dir_segment() {
        let err = validate_archive_entry_path(Path::new("../.bashrc")).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn validator_rejects_buried_parent_dir_segment() {
        let err = validate_archive_entry_path(Path::new("a/../../pwned")).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn validator_rejects_absolute_path() {
        let err = validate_archive_entry_path(Path::new("/tmp/pwned")).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    /// Append a regular file entry whose name is written directly into the
    /// header buffer, bypassing `tar::Header::set_path`'s built-in rejection
    /// of `..` and absolute roots — we *want* to ship malicious entries here
    /// so we can prove the extractor refuses them on its own.
    fn append_raw_entry(builder: &mut tar::Builder<Vec<u8>>, name: &[u8], body: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        let buf = &mut header.as_old_mut().name;
        for slot in buf.iter_mut() {
            *slot = 0;
        }
        let n = name.len().min(buf.len());
        buf[..n].copy_from_slice(&name[..n]);
        header.set_cksum();
        builder.append(&header, body).unwrap();
    }

    fn build_safe_tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, body) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, path, &body[..]).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn extract_accepts_safe_nested_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = build_safe_tarball(&[("pkg/lib.sl", b"hello")]);
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        extract_archive(&mut archive, tmp.path()).unwrap();
        let extracted = std::fs::read(tmp.path().join("pkg/lib.sl")).unwrap();
        assert_eq!(extracted, b"hello");
    }

    #[test]
    fn extract_rejects_parent_traversal_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        append_raw_entry(&mut builder, b"../pwned", b"escape");
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path()).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
        assert!(!tmp.path().parent().unwrap().join("pwned").exists());
    }

    #[test]
    fn extract_rejects_buried_traversal_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        append_raw_entry(&mut builder, b"a/../../pwned", b"escape");
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path()).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn extract_rejects_absolute_path_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        append_raw_entry(&mut builder, b"/tmp/pwned", b"escape");
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path()).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn extract_rejects_symlink_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("/etc/passwd").expect("symlink target");
        header.set_cksum();
        builder.append_data(&mut header, "passwd", &[][..]).unwrap();
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path()).unwrap_err();
        assert!(err.contains("link entry"), "{}", err);
    }
}
