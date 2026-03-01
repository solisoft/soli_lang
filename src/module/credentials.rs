//! Credentials management for the Soli package registry.
//!
//! Stores API tokens in `~/.soli/credentials` for authenticating
//! with the package registry (publish, etc.).

use std::fs;
use std::path::PathBuf;

/// Registry credentials.
#[derive(Debug, Clone)]
pub struct Credentials {
    /// Registry URL
    pub url: String,
    /// API token
    pub token: String,
}

/// Path to the credentials file (~/.soli/credentials).
pub fn credentials_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".soli")
        .join("credentials")
}

/// Load credentials from disk.
pub fn load_credentials() -> Option<Credentials> {
    let path = credentials_path();
    let content = fs::read_to_string(&path).ok()?;

    let mut url = None;
    let mut token = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            match key {
                "url" => url = Some(value.to_string()),
                "token" => token = Some(value.to_string()),
                _ => {}
            }
        }
    }

    Some(Credentials {
        url: url?,
        token: token?,
    })
}

/// Save credentials to disk.
pub fn save_credentials(creds: &Credentials) -> Result<(), String> {
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create ~/.soli directory: {}", e))?;
    }

    let content = format!(
        "[registry]\nurl = \"{}\"\ntoken = \"{}\"\n",
        creds.url, creds.token
    );

    fs::write(&path, content).map_err(|e| format!("Failed to write credentials: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_path() {
        let path = credentials_path();
        assert!(path.to_string_lossy().contains(".soli/credentials"));
    }
}
