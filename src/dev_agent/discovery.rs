//! `.soli/dev-server.json`: how `soli agent` finds the `soli serve --dev`
//! process of the app it is started in.
//!
//! `serve --dev` writes it once the listener is bound and removes it on
//! shutdown. It holds the pid, the port, the app root and the dev REPL token —
//! the same per-process secret the dev error pages already embed — so the file
//! is created with mode 0600: only the developer who runs the server can read
//! it. A server killed outright leaves the file behind; the pid check below is
//! what tells a stale file from a live one.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Relative to the app root.
pub const DISCOVERY_FILE: &str = ".soli/dev-server.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DevServerInfo {
    pub pid: u32,
    pub port: u16,
    pub app_root: String,
    /// The `X-Soli-Dev-Token` `/__dev/repl` requires. Absent in files written
    /// before the field existed.
    #[serde(default)]
    pub dev_token: Option<String>,
}

/// The file this process wrote, so shutdown removes that one and no other.
static WRITTEN: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Write `<app_root>/.soli/dev-server.json` for this process. Best effort: a
/// read-only checkout must not stop the server, so a failure is reported on
/// stderr and otherwise ignored.
pub fn write(app_root: &Path, port: u16, dev_token: &str) {
    let info = DevServerInfo {
        pid: std::process::id(),
        port,
        app_root: app_root.display().to_string(),
        dev_token: Some(dev_token.to_string()),
    };
    let path = app_root.join(DISCOVERY_FILE);
    match write_info(&path, &info) {
        Ok(()) => {
            if let Ok(mut slot) = WRITTEN.lock() {
                *slot = Some(path);
            }
        }
        Err(e) => eprintln!("soli agent: could not write {}: {}", path.display(), e),
    }
}

fn write_info(path: &Path, info: &DevServerInfo) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp)?;
    let body = serde_json::to_string_pretty(info).map_err(std::io::Error::other)?;
    file.write_all(body.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)
}

/// Remove the file this process wrote, if it still names this process (a
/// second server started in the same app since then owns it now).
pub fn remove() {
    let Some(path) = WRITTEN.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    let ours = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<DevServerInfo>(&s).ok())
        .is_some_and(|info| info.pid == std::process::id());
    if ours {
        let _ = fs::remove_file(&path);
    }
}

/// Read the discovery file under `dir` and check that its process is alive.
/// Every failure is the one message an agent can act on.
pub fn read(dir: &Path) -> Result<DevServerInfo, String> {
    let path = dir.join(DISCOVERY_FILE);
    let not_running = || {
        format!(
            "dev server is not running (no live `soli serve --dev` found via {}; start it from the app root)",
            path.display()
        )
    };
    let raw = fs::read_to_string(&path).map_err(|_| not_running())?;
    let info: DevServerInfo = serde_json::from_str(&raw).map_err(|_| not_running())?;
    if !pid_alive(info.pid) {
        return Err(not_running());
    }
    Ok(info)
}

/// Whether a process with this pid exists.
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // Signal 0 checks existence and permission without sending anything.
    // EPERM means it exists but belongs to someone else.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// No cheap existence check here; the connection attempt decides.
#[cfg(not(unix))]
pub fn pid_alive(pid: u32) -> bool {
    pid != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pid that cannot belong to a live process: spawn one and wait for it.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let pid = child.id();
        child.wait().expect("wait");
        pid
    }

    #[test]
    fn missing_file_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let err = read(dir.path()).unwrap_err();
        assert!(err.starts_with("dev server is not running"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn dead_pid_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let info = DevServerInfo {
            pid: dead_pid(),
            port: 1,
            app_root: dir.path().display().to_string(),
            dev_token: None,
        };
        write_info(&dir.path().join(DISCOVERY_FILE), &info).unwrap();
        let err = read(dir.path()).unwrap_err();
        assert!(err.starts_with("dev server is not running"), "{err}");
    }

    #[test]
    fn live_pid_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let info = DevServerInfo {
            pid: std::process::id(),
            port: 5011,
            app_root: dir.path().display().to_string(),
            dev_token: Some("tok".into()),
        };
        write_info(&dir.path().join(DISCOVERY_FILE), &info).unwrap();
        assert_eq!(read(dir.path()).unwrap(), info);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DISCOVERY_FILE);
        let info = DevServerInfo {
            pid: std::process::id(),
            port: 5011,
            app_root: String::new(),
            dev_token: Some("tok".into()),
        };
        write_info(&path, &info).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
