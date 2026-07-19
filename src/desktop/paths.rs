//! Per-OS directory layout for a packaged desktop app.
//!
//! Three roots, because they have genuinely different lifetimes:
//!
//! | | `data` (must persist) | `state` | `cache` (safe to delete) |
//! |---|---|---|---|
//! | Linux | `$XDG_DATA_HOME/<app_id>/db` | `.../state` | `$XDG_CACHE_HOME/<app_id>/bin` |
//! | macOS | `~/Library/Application Support/<app_id>/db` | `.../state` | `~/Library/Caches/<app_id>/bin` |
//! | Windows | `%LOCALAPPDATA%\<app_id>\db` | `...\state` | `%LOCALAPPDATA%\<app_id>\cache\bin` |
//!
//! `data` holds the database — losing it loses the user's work. `cache` holds
//! the extracted database binary, which is re-derivable from the bundle, so a
//! disk cleaner may remove it freely.
//!
//! Windows uses the *local* app-data root deliberately: the roaming profile
//! (`%APPDATA%`) is synced between machines, and a multi-hundred-megabyte
//! RocksDB directory must never be.

use std::path::{Path, PathBuf};

/// Resolved directories for one application identity.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// Read-write database directory. Must survive across launches.
    pub data: PathBuf,
    /// Small mutable files: credentials, seed watermark, pidfile, lock.
    pub state: PathBuf,
    /// Re-derivable artifacts, chiefly the extracted database binary.
    pub cache: PathBuf,
}

impl AppPaths {
    /// The single-instance lock file. See `platform::lock`.
    pub fn instance_lock(&self) -> PathBuf {
        self.state.join("instance.lock")
    }

    /// Create `data` and `state`. `cache` is created lazily by whoever
    /// populates it, so a run that never needs the binary doesn't make dirs.
    pub fn ensure(&self) -> Result<(), String> {
        for dir in [&self.data, &self.state] {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
        }
        Ok(())
    }
}

/// Reject an `app_id` that could escape its parent directory or produce a
/// surprising path.
///
/// The id is baked in at build time rather than user-supplied, so this is a
/// build-time misconfiguration guard rather than a defence against attack —
/// but it is cheap, and an `app_id` of `../..` would otherwise silently point
/// the database somewhere alarming.
pub fn validate_app_id(app_id: &str) -> Result<(), String> {
    if app_id.is_empty() {
        return Err("app_id must not be empty".to_string());
    }
    if app_id.len() > 128 {
        return Err(format!(
            "app_id is too long ({} chars, max 128)",
            app_id.len()
        ));
    }
    // Reverse-DNS shape: alphanumerics, dot, dash, underscore. Notably excludes
    // both separators and `..`, so traversal is impossible by construction.
    if let Some(bad) = app_id
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_'))
    {
        return Err(format!(
            "app_id contains an invalid character {:?} — use reverse-DNS form, e.g. com.example.app",
            bad
        ));
    }
    if app_id.starts_with('.') || app_id.ends_with('.') {
        return Err("app_id must not start or end with '.'".to_string());
    }
    Ok(())
}

/// Resolve the directory layout for `app_id` (reverse-DNS, e.g.
/// `com.example.app`).
pub fn for_app(app_id: &str) -> Result<AppPaths, String> {
    validate_app_id(app_id)?;

    // `data_local_dir` is machine-local on every platform — on Windows that is
    // %LOCALAPPDATA% rather than the roaming %APPDATA%.
    let data_root = dirs::data_local_dir()
        .ok_or_else(|| "cannot determine the local application-data directory".to_string())?;
    let cache_root =
        dirs::cache_dir().ok_or_else(|| "cannot determine the cache directory".to_string())?;

    Ok(build(app_id, &data_root, &cache_root))
}

/// Layout construction, split out so tests can pin the shape without depending
/// on the host's real home directory.
fn build(app_id: &str, data_root: &Path, cache_root: &Path) -> AppPaths {
    let base = data_root.join(app_id);
    AppPaths {
        data: base.join("db"),
        state: base.join("state"),
        cache: cache_root.join(app_id).join("bin"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_separates_data_state_and_cache() {
        let paths = build(
            "com.example.app",
            Path::new("/data-root"),
            Path::new("/cache-root"),
        );
        assert_eq!(paths.data, Path::new("/data-root/com.example.app/db"));
        assert_eq!(paths.state, Path::new("/data-root/com.example.app/state"));
        assert_eq!(paths.cache, Path::new("/cache-root/com.example.app/bin"));
        assert_eq!(
            paths.instance_lock(),
            Path::new("/data-root/com.example.app/state/instance.lock")
        );
    }

    #[test]
    fn cache_is_not_under_data() {
        // The cache is deletable by the OS; the database is not. If a future
        // refactor nests one in the other, a disk cleaner could take the
        // user's data with it.
        let paths = build("app", Path::new("/data-root"), Path::new("/cache-root"));
        assert!(
            !paths.cache.starts_with(&paths.data),
            "cache must never live inside the persistent data directory"
        );
    }

    #[test]
    fn rejects_app_ids_that_would_escape_the_root() {
        for bad in ["../../etc", "a/b", "a\\b", "", "."] {
            assert!(
                validate_app_id(bad).is_err(),
                "app_id {:?} should be rejected",
                bad
            );
        }
    }

    #[test]
    fn accepts_reverse_dns_ids() {
        for good in ["com.example.app", "app", "com.example.my-app_2"] {
            assert!(
                validate_app_id(good).is_ok(),
                "app_id {:?} should be accepted",
                good
            );
        }
    }
}
