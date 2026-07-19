//! Exclusive, OS-released advisory file locks.
//!
//! Used to enforce single-instance semantics. The property that matters is that
//! the lock is released by the *kernel* when the holding process dies —
//! including on `SIGKILL`, a panic, or a power-loss-adjacent crash. A lockfile
//! containing a PID cannot offer that: it survives the process, so every reader
//! has to guess whether the PID is stale, and PID reuse makes that guess wrong
//! in the dangerous direction.

use std::fs::{File, OpenOptions};
use std::path::Path;

/// An acquired exclusive lock. Releasing happens on drop — implicitly, by
/// closing the file descriptor, which is also what the kernel does for us if
/// the process dies without unwinding.
#[derive(Debug)]
pub struct InstanceLock {
    // Held purely for its lifetime: dropping closes the fd and drops the lock.
    _file: File,
}

/// Try to take an exclusive lock on `path`, creating it if absent.
///
/// Returns `Ok(None)` when another live process already holds it — that is the
/// expected "app is already running" answer, not an error. `Err` is reserved
/// for a lock that could not be evaluated at all (unreadable path, permissions,
/// unsupported platform).
pub fn try_acquire(path: &Path) -> Result<Option<InstanceLock>, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create lock directory {}: {}", parent.display(), e))?;
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("cannot open lock file {}: {}", path.display(), e))?;

    lock_exclusive_nonblocking(&file, path).map(|acquired| {
        if acquired {
            Some(InstanceLock { _file: file })
        } else {
            None
        }
    })
}

/// `Ok(true)` = acquired, `Ok(false)` = held by someone else.
#[cfg(unix)]
fn lock_exclusive_nonblocking(file: &File, path: &Path) -> Result<bool, String> {
    use std::os::unix::io::AsRawFd;

    // SAFETY: `flock` takes a valid fd, which `file` owns for this call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        // Both spellings mean "another process holds it"; they are the same
        // value on Linux but distinct on some BSDs, so match each.
        Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => Ok(false),
        _ => Err(format!("cannot lock {}: {}", path.display(), err)),
    }
}

#[cfg(not(unix))]
fn lock_exclusive_nonblocking(_file: &File, path: &Path) -> Result<bool, String> {
    // Windows wants LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY),
    // which needs an explicit windows-sys dependency this crate does not carry
    // yet. Refusing loudly beats a silently absent lock, which would let two
    // instances open the same database directory.
    Err(format!(
        "single-instance locking is not implemented on this platform (lock file: {})",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("soli_lock_test_{}_{}", std::process::id(), name));
        p
    }

    #[test]
    #[cfg(unix)]
    fn acquires_when_free_and_refuses_while_held() {
        let path = scratch("held");
        let _ = std::fs::remove_file(&path);

        let first = try_acquire(&path).expect("first acquire must evaluate");
        assert!(first.is_some(), "lock should be free initially");

        // A second attempt from this same process must also be refused: flock
        // is per-open-file-description, and we deliberately open a new one.
        let second = try_acquire(&path).expect("second acquire must evaluate");
        assert!(
            second.is_none(),
            "a held lock must report as taken, not error"
        );

        drop(first);
        let third = try_acquire(&path).expect("third acquire must evaluate");
        assert!(third.is_some(), "lock must be released on drop");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn creates_missing_parent_directories() {
        let mut path = scratch("nested");
        path.push("deeper");
        path.push("instance.lock");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());

        let lock = try_acquire(&path).expect("must create parents");
        assert!(lock.is_some());
        assert!(path.exists());

        drop(lock);
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
