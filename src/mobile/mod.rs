//! Building and distributing the native shells `soli generate client` writes.
//!
//! `soli mobile build android|ios` turns `clients/<platform>/` into an
//! installable, versioned APK or IPA; `soli mobile publish` uploads it to the
//! app's own `/__soli/mobile` page (see `serve::dev_mobile`), which lists the
//! builds and hands out install links and QR codes.
//!
//! What lives here is what both the CLI and the server need, or what a test
//! can check without a device: the config file, version stamping, the build
//! staging, the iOS install manifest and the QR rendering. Running the
//! platform tool chains is in [`build`] too, so the CLI stays a thin layer of
//! argument parsing and printing.

pub mod build;
pub mod config;
pub mod ios_manifest;
pub mod publish;
pub mod qr;
pub mod stamp;

use std::io::Read;
use std::path::Path;

/// The two platforms a build can be for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Android,
    Ios,
}

impl Platform {
    pub fn parse(raw: &str) -> Option<Platform> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "android" => Some(Platform::Android),
            "ios" => Some(Platform::Ios),
            _ => None,
        }
    }

    /// The platform an artifact's file name implies.
    pub fn from_file_name(name: &str) -> Option<Platform> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".apk") {
            Some(Platform::Android)
        } else if lower.ends_with(".ipa") {
            Some(Platform::Ios)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Android => "android",
            Platform::Ios => "ios",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Platform::Android => "apk",
            Platform::Ios => "ipa",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Platform::Android => "application/vnd.android.package-archive",
            Platform::Ios => "application/octet-stream",
        }
    }
}

/// SHA-256 of a file, read in chunks: an IPA can be hundreds of megabytes.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Escape text for an XML element or a double-quoted attribute.
pub(crate) fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_comes_from_the_name_or_the_extension() {
        assert_eq!(Platform::parse("Android"), Some(Platform::Android));
        assert_eq!(Platform::parse("ios"), Some(Platform::Ios));
        assert_eq!(Platform::parse("windows"), None);
        assert_eq!(
            Platform::from_file_name("shop-1.0-3.APK"),
            Some(Platform::Android)
        );
        assert_eq!(Platform::from_file_name("shop.ipa"), Some(Platform::Ios));
        assert_eq!(Platform::from_file_name("shop.zip"), None);
    }

    #[test]
    fn sha256_of_a_file_matches_the_known_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
