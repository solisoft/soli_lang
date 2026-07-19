//! Process liveness, without per-call-site `cfg` branches.

/// Whether a process with this id currently exists.
///
/// **The failure direction matters more than the accuracy.** The caller is
/// deciding whether to delete a directory belonging to another instance: a
/// false *alive* leaks a directory that the next sweep will clean up, while a
/// false *dead* deletes a running app's decrypted source out from under it. So
/// everything ambiguous — a permissions error, an unreadable handle — reports
/// alive.
///
/// Both implementations share the pid-reuse caveat: a recycled id reads as
/// alive. That errs in the safe direction, and matching the long-standing Unix
/// behavior is deliberate — a lockfile held open for the process lifetime would
/// be strictly better and is a separate change.
pub fn is_alive(pid: u32) -> bool {
    is_alive_impl(pid)
}

#[cfg(unix)]
fn is_alive_impl(pid: u32) -> bool {
    // Signal 0 checks for existence without delivering anything. ESRCH means
    // gone; EPERM means it exists but belongs to someone else — still alive.
    // SAFETY: `kill` with signal 0 has no effect beyond the existence check.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn is_alive_impl(_pid: u32) -> bool {
    // Windows wants OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION) plus
    // GetExitCodeProcess, which needs an explicit windows-sys dependency this
    // crate does not carry yet. Reporting alive is the safe placeholder: it
    // leaks stale directories rather than deleting live ones.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_process_is_alive() {
        assert!(is_alive(std::process::id()));
    }

    #[test]
    #[cfg(unix)]
    fn a_reaped_child_is_not_alive() {
        // A pid we know is gone, rather than a guessed-unused number: spawn,
        // wait, then ask.
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn /bin/true");
        let pid = child.id();
        child.wait().expect("reap");
        assert!(!is_alive(pid), "a reaped child must report as gone");
    }

    #[test]
    #[cfg(unix)]
    fn pid_one_is_alive_despite_belonging_to_another_user() {
        // init always exists and is not ours: the EPERM path must report
        // alive, not "gone". Getting this backwards would delete a running
        // instance's files.
        assert!(is_alive(1));
    }
}
