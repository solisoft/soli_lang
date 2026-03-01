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
pub fn resolve_version(registry_url: &str, name: &str, version: &str) -> Result<VersionInfo, String> {
    let api_url = format!("{}/api/packages/{}/{}", registry_url, name, version);

    let response = ureq::get(&api_url)
        .set("User-Agent", "soli-package-manager")
        .call()
        .map_err(|e| format!("Failed to resolve '{}@{}' from registry: {}", name, version, e))?;

    let body: serde_json::Value = response
        .into_json()
        .map_err(|e| format!("Failed to parse registry response: {}", e))?;

    let download_url = body["download_url"]
        .as_str()
        .ok_or_else(|| format!("Registry response missing 'download_url' for '{}@{}'", name, version))?
        .to_string();

    Ok(VersionInfo { download_url })
}

/// Download and extract a package tarball.
///
/// Downloads from the given URL and extracts to dest_dir.
/// Registry tarballs are flat (no top-level directory to strip).
pub fn download_package(url: &str, dest: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;

    let response = ureq::get(url)
        .set("User-Agent", "soli-package-manager")
        .call()
        .map_err(|e| format!("Failed to download package: {}", e))?;

    let reader = response.into_reader();
    let decoder = GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);

    // Create destination directory
    fs::create_dir_all(dest)
        .map_err(|e| format!("Failed to create cache directory: {}", e))?;

    // Extract directly — registry tarballs are flat (no top-level dir to strip)
    for entry in archive
        .entries()
        .map_err(|e| format!("Failed to read archive entries: {}", e))?
    {
        let mut entry = entry.map_err(|e| format!("Failed to read archive entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("Failed to read entry path: {}", e))?
            .into_owned();

        let out_path = dest.join(&path);

        if entry.header().entry_type() == tar::EntryType::Directory {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directory: {}", e))?;
            }
            let mut out_file = fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create file: {}", e))?;
            io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
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
