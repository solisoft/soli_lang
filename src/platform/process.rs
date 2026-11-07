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

/// Have the kernel SIGKILL the child `command` spawns when this process dies,
/// so a helper (a browser, a database, a test server) never outlives a runner
/// that was killed outright. Linux only (`PR_SET_PDEATHSIG`); a no-op
/// elsewhere.
#[cfg(target_os = "linux")]
pub fn arm_parent_death(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: `prctl` is async-signal-safe and this closure runs in the child
    // between fork and exec, where only such calls are permitted.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub fn arm_parent_death(_command: &mut std::process::Command) {}

/// Children to SIGKILL when this process exits, whatever the path out.
///
/// An owning guard's `Drop` covers a normal return and an unwinding panic,
/// but not `std::process::exit` — which the SIGINT/SIGTERM handler calls, as
/// do early-exit paths such as a coverage threshold miss — and
/// `PR_SET_PDEATHSIG` ([`arm_parent_death`]) exists only on Linux. `soli
/// test | head` on macOS left `soli serve --workers 1` test servers running
/// after the runner was gone (#679). `exit` runs `atexit` handlers, so one
/// registered here reaches every one of those paths.
///
/// Fixed slots of atomics rather than a `Mutex<Vec<_>>`: the handler can run
/// from inside a signal handler's `exit`, where taking a lock another thread
/// holds would deadlock. 0 is an empty slot.
#[cfg(unix)]
static EXIT_KILL_PIDS: [std::sync::atomic::AtomicI32; 64] =
    [const { std::sync::atomic::AtomicI32::new(0) }; 64];

#[cfg(unix)]
extern "C" fn kill_registered_children() {
    use std::sync::atomic::Ordering;
    for slot in &EXIT_KILL_PIDS {
        let pid = slot.swap(0, Ordering::SeqCst);
        if pid > 0 {
            // SAFETY: `kill` is async-signal-safe; the pid is a child this
            // process spawned and has not reaped (reaping clears the slot).
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

/// SIGKILL `pid` when this process exits, unless [`forget_exit_kill`] runs
/// first. Returns false when every slot is taken (the child is then covered
/// only by its owner's `Drop`).
#[cfg(unix)]
pub fn kill_on_exit(pid: u32) -> bool {
    use std::sync::atomic::Ordering;
    use std::sync::Once;
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // SAFETY: registering a plain `extern "C" fn()` with `atexit`.
        unsafe {
            libc::atexit(kill_registered_children);
        }
    });
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    EXIT_KILL_PIDS.iter().any(|slot| {
        slot.compare_exchange(0, pid, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    })
}

/// Stop tracking `pid` — call once it has been reaped, so a recycled id is
/// never signalled.
#[cfg(unix)]
pub fn forget_exit_kill(pid: u32) {
    use std::sync::atomic::Ordering;
    let Ok(pid) = i32::try_from(pid) else { return };
    for slot in &EXIT_KILL_PIDS {
        let _ = slot.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

#[cfg(not(unix))]
pub fn kill_on_exit(_pid: u32) -> bool {
    false
}

#[cfg(not(unix))]
pub fn forget_exit_kill(_pid: u32) {}

/// `value` as one POSIX shell word: single-quoted, with each `'` written
/// `'\''`.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, not two: the slots are process-global and the exit handler
    /// clears all of them, so two tests touching them would race.
    #[test]
    #[cfg(unix)]
    fn registered_children_are_killed_by_the_exit_handler_and_forgotten_ones_are_not() {
        use std::sync::atomic::Ordering;
        let tracked = |pid: i32| {
            EXIT_KILL_PIDS
                .iter()
                .any(|s| s.load(Ordering::SeqCst) == pid)
        };

        // A pid no process can have: registering and forgetting it must leave
        // no slot behind.
        let fake = i32::MAX as u32;
        assert!(kill_on_exit(fake));
        assert!(tracked(i32::MAX));
        forget_exit_kill(fake);
        assert!(!tracked(i32::MAX));

        // The handler `exit` runs: a registered child is SIGKILLed and its
        // slot cleared.
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = sleeper.id();
        assert!(kill_on_exit(pid));
        kill_registered_children();
        let status = sleeper.wait().expect("reap");
        assert!(!status.success(), "the child should have been killed");
        assert!(!tracked(pid as i32));
    }

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
