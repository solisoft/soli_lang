//! Best-effort removal of sensitive scratch directories on process exit.
//!
//! Used by encrypted-bundle serving: the decrypted app tree lives in a
//! private tmpfs directory that must not outlive the server. Registered
//! directories are removed by a `libc::atexit` hook, which fires on normal
//! exit AND on SIGINT/SIGTERM because the graceful-shutdown handler in
//! `main.rs` exits via `process::exit(0)`. Limits (documented, accepted):
//! `kill -9` and power loss skip atexit — mitigated by the stale-dir sweep
//! at next boot and by tmpfs vanishing on reboot.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

static CLEANUP_DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static INSTALL: Once = Once::new();

extern "C" fn run_cleanup_extern() {
    if let Ok(dirs) = CLEANUP_DIRS.lock() {
        for dir in dirs.iter() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Register a directory for removal at process exit. Installs the atexit
/// hook on first use.
pub fn register_cleanup_dir(path: &Path) {
    INSTALL.call_once(|| unsafe {
        libc::atexit(run_cleanup_extern);
    });
    if let Ok(mut dirs) = CLEANUP_DIRS.lock() {
        dirs.push(path.to_path_buf());
    }
}
