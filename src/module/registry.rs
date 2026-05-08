//! HTTP client for the Soli package registry.
//!
//! Communicates with the registry API to resolve versions,
//! download packages, and publish new packages.

use std::fs;
use std::net::IpAddr;
use std::path::Path;

use super::tar_extract;
use crate::interpreter::builtins::http_class::is_blocked_ip;

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
///
/// SEC-093: the URL is registry-controlled (returned in the JSON response
/// from `resolve_version`). It is validated up-front against
/// [`validate_download_url`] before any network call: a compromised
/// registry must not be able to point `soli install` at metadata
/// endpoints, RFC1918 services, or other hosts reachable from the
/// developer's machine. `registry_url` is the operator-configured registry
/// origin and is consulted to permit local-dev workflows where a registry
/// running on `localhost` legitimately serves tarballs over `http`.
pub fn download_package(registry_url: &str, url: &str, dest: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;

    validate_download_url(url, registry_url)?;

    // See `resolve_version` for the rationale on redirect-following:
    // registry CDNs redirect, CLI-trust context, no request-level SSRF
    // surface. The up-front validation above closes the
    // attacker-controlled-URL hole.
    let response = ureq::get(url)
        .set("User-Agent", "soli-package-manager")
        .call()
        .map_err(|e| format!("Failed to download package: {}", e))?;

    let reader = response.into_reader();
    let decoder = GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);

    // Create destination directory
    fs::create_dir_all(dest).map_err(|e| format!("Failed to create cache directory: {}", e))?;

    // Registry tarballs are flat — no top-level directory to strip.
    tar_extract::extract_archive(&mut archive, dest, false)
}

/// Validate a registry-returned tarball URL before opening a connection.
///
/// SEC-093 — policy:
/// - Only `http://` and `https://` are accepted (no `file:`, `ftp:`, etc.).
/// - `http://` is only accepted when the configured registry itself is
///   `http://`. The default registry is https; any plaintext download URL
///   from an https registry is treated as a downgrade and refused.
/// - IP-literal hosts and the `localhost` name are rejected unless the
///   configured registry itself targets a private/loopback host (i.e. the
///   operator explicitly opted in to a local registry). Bare hostnames
///   are not DNS-resolved here; the realistic registry-supplied SSRF
///   payload is an IP literal, and resolving DNS up-front would add a
///   TOCTOU window between this check and the fetch.
fn validate_download_url(url: &str, registry_url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| format!("registry returned an invalid download URL '{}': {}", url, e))?;
    let registry = reqwest::Url::parse(registry_url)
        .map_err(|e| format!("invalid registry URL '{}': {}", registry_url, e))?;

    let scheme = parsed.scheme().to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!(
            "registry download URL must use http or https, got '{}://': {}",
            scheme, url
        ));
    }
    if scheme == "http" && !registry.scheme().eq_ignore_ascii_case("http") {
        return Err(format!(
            "registry is https but returned an http download URL; refusing: {}",
            url
        ));
    }

    let host = parsed
        .host()
        .ok_or_else(|| format!("registry download URL has no host: {}", url))?;

    let registry_is_private = registry
        .host()
        .map(|h| host_is_private(&h))
        .unwrap_or(false);

    if host_is_private(&host) && !registry_is_private {
        return Err(format!(
            "registry download URL targets a private/localhost address; refusing: {}",
            url
        ));
    }

    Ok(())
}

fn host_is_private(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Ipv4(v4) => is_blocked_ip(IpAddr::V4(*v4)),
        url::Host::Ipv6(v6) => is_blocked_ip(IpAddr::V6(*v6)),
        url::Host::Domain(d) => {
            let lower = d.to_lowercase();
            lower == "localhost" || lower == "localhost." || lower.starts_with("localhost.")
        }
    }
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

// Extraction tests live in `src/module/tar_extract.rs`; this module just
// delegates to the shared helper.

#[cfg(test)]
mod tests {
    use super::validate_download_url;

    const PUBLIC_REGISTRY: &str = "https://ilos.solisoft.net";
    const LOCAL_REGISTRY: &str = "http://localhost:8080";

    #[test]
    fn accepts_https_cdn_for_public_registry() {
        validate_download_url(
            "https://cdn.example.com/pkg/foo-1.0.0.tar.gz",
            PUBLIC_REGISTRY,
        )
        .expect("public https CDN should be allowed");
    }

    #[test]
    fn rejects_non_http_schemes() {
        let err = validate_download_url("file:///etc/passwd", PUBLIC_REGISTRY).unwrap_err();
        assert!(err.contains("must use http or https"), "got: {}", err);

        let err =
            validate_download_url("ftp://example.com/foo.tar.gz", PUBLIC_REGISTRY).unwrap_err();
        assert!(err.contains("must use http or https"), "got: {}", err);
    }

    #[test]
    fn rejects_http_when_registry_is_https() {
        let err = validate_download_url("http://cdn.example.com/foo.tar.gz", PUBLIC_REGISTRY)
            .unwrap_err();
        assert!(err.contains("https but returned an http"), "got: {}", err);
    }

    #[test]
    fn rejects_loopback_ip_literal_for_public_registry() {
        let err =
            validate_download_url("http://127.0.0.1:8080/foo.tar.gz", PUBLIC_REGISTRY).unwrap_err();
        // Either the http-downgrade message or the private-address message
        // is acceptable; both block the request.
        assert!(err.contains("https but returned an http") || err.contains("private/localhost"));

        let err =
            validate_download_url("https://127.0.0.1/foo.tar.gz", PUBLIC_REGISTRY).unwrap_err();
        assert!(err.contains("private/localhost"), "got: {}", err);
    }

    #[test]
    fn rejects_localhost_name_for_public_registry() {
        let err =
            validate_download_url("https://localhost/foo.tar.gz", PUBLIC_REGISTRY).unwrap_err();
        assert!(err.contains("private/localhost"), "got: {}", err);
    }

    #[test]
    fn rejects_rfc1918_for_public_registry() {
        for url in [
            "https://10.0.0.1/foo.tar.gz",
            "https://192.168.1.1/foo.tar.gz",
            "https://172.16.0.1/foo.tar.gz",
        ] {
            let err = validate_download_url(url, PUBLIC_REGISTRY).unwrap_err();
            assert!(err.contains("private/localhost"), "url={} err={}", url, err);
        }
    }

    #[test]
    fn rejects_ipv6_loopback_for_public_registry() {
        let err = validate_download_url("https://[::1]/foo.tar.gz", PUBLIC_REGISTRY).unwrap_err();
        assert!(err.contains("private/localhost"), "got: {}", err);

        // IPv4-mapped loopback must be rejected too.
        let err = validate_download_url("https://[::ffff:127.0.0.1]/foo.tar.gz", PUBLIC_REGISTRY)
            .unwrap_err();
        assert!(err.contains("private/localhost"), "got: {}", err);
    }

    #[test]
    fn allows_loopback_when_registry_is_local() {
        validate_download_url("http://localhost:8080/pkg/foo-1.0.0.tar.gz", LOCAL_REGISTRY)
            .expect("local registry must be able to serve local download URLs");

        validate_download_url("http://127.0.0.1:8080/foo.tar.gz", LOCAL_REGISTRY)
            .expect("local registry must be able to serve loopback IP URLs");
    }

    #[test]
    fn rejects_garbage_url() {
        let err = validate_download_url("not a url", PUBLIC_REGISTRY).unwrap_err();
        assert!(err.contains("invalid download URL"), "got: {}", err);
    }
}
