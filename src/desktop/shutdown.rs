//! Ordered shutdown for a desktop app.
//!
//! Two things must happen when the app is asked to stop, and neither happens
//! today:
//!
//! 1. **The database gets a clean close.** Killed abruptly, RocksDB replays its
//!    write-ahead log on the next open — slow, and it reads to a user like
//!    corruption.
//! 2. **The decrypted application tree is removed.** It lives in RAM-backed
//!    `/dev/shm`, but it survives until something deletes it.
//!
//! The existing handler calls `process::exit(0)` directly from a signal
//! handler, which runs `atexit` hooks that take a mutex. `Mutex::lock` is not
//! async-signal-safe: if the signal lands while that lock is held, shutdown
//! deadlocks. So the handler here does the minimum a signal handler may do —
//! store to an atomic — and a dedicated thread does the real work.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Set by the signal handler; polled by the shutdown thread.
static REQUESTED: AtomicBool = AtomicBool::new(false);
/// Database process id, or 0 when none is running.
static DB_PID: AtomicU32 = AtomicU32::new(0);
/// Ensures the sequence runs exactly once even if several signals arrive.
static RAN: AtomicBool = AtomicBool::new(false);

fn extraction_dir() -> &'static Mutex<Option<PathBuf>> {
    static DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    DIR.get_or_init(|| Mutex::new(None))
}

/// Register what shutdown is responsible for.
pub fn register(db_pid: u32, dir: &Path) {
    DB_PID.store(db_pid, Ordering::SeqCst);
    if let Ok(mut slot) = extraction_dir().lock() {
        *slot = Some(dir.to_path_buf());
    }
}

/// Whether a stop has been requested.
pub fn is_requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Request shutdown. Safe to call from anywhere, including a signal handler:
/// it only stores to an atomic.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Start the watcher thread and install signal handlers.
pub fn install() {
    install_signal_handlers();

    std::thread::Builder::new()
        .name("soli-shutdown".to_string())
        .spawn(|| loop {
            if is_requested() {
                run();
                std::process::exit(0);
            }
            // Polling rather than parking: `Thread::unpark` is not documented
            // async-signal-safe, and a 100ms shutdown latency is irrelevant
            // next to the risk of an unsafe call in a signal handler.
            std::thread::sleep(Duration::from_millis(100));
        })
        .ok();
}

/// Run the ordered shutdown. Idempotent.
///
/// Order is deliberate. The decrypted tree goes first because it is the
/// confidentiality-relevant step and the process may be killed at any moment
/// once a stop is pending — on Windows a close event allows only a few seconds
/// before the OS terminates the process regardless. Losing a clean database
/// close is recoverable; leaving plaintext behind is not.
pub fn run() {
    if RAN.swap(true, Ordering::SeqCst) {
        return;
    }

    remove_extraction_dir();
    stop_database(Duration::from_secs(10));
}

fn remove_extraction_dir() {
    if let Ok(mut slot) = extraction_dir().lock() {
        if let Some(dir) = slot.take() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Ask the database to stop, escalating if it will not.
fn stop_database(grace: Duration) {
    let pid = DB_PID.swap(0, Ordering::SeqCst);
    if pid == 0 {
        return;
    }

    #[cfg(unix)]
    {
        // SAFETY: signalling a pid we spawned. If it has already exited the
        // call simply fails, which is fine.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }

        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            // Reap rather than probe. `kill(pid, 0)` is the obvious check and
            // is wrong here: an exited child becomes a zombie until its parent
            // reaps it, and signalling a zombie still succeeds — so the probe
            // reports "alive" forever, and every shutdown waits out the full
            // grace and then SIGKILLs a database that already exited cleanly.
            // `waitpid` both detects the exit and clears the zombie.
            let mut status: libc::c_int = 0;
            // SAFETY: waiting on a child we spawned; WNOHANG makes it
            // non-blocking.
            let reaped = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
            if reaped != 0 {
                // >0: reaped. <0: not our child or already gone. Either way
                // there is nothing left to wait for.
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        // It ignored SIGTERM. A WAL replay next launch beats hanging here.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }

    #[cfg(not(unix))]
    {
        let _ = grace; // Windows termination arrives with the job object.
    }
}

#[cfg(unix)]
fn install_signal_handlers() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // The handler stores to an atomic and returns — the only kind of work
        // that is unambiguously safe here. Everything else runs on the watcher
        // thread.
        extern "C" fn handle(_sig: i32) {
            REQUESTED.store(true, Ordering::SeqCst);
        }

        let action = nix::sys::signal::SigAction::new(
            nix::sys::signal::SigHandler::Handler(handle),
            nix::sys::signal::SaFlags::empty(),
            nix::sys::signal::SigSet::empty(),
        );
        // Best-effort: a failed install leaves the previous behavior.
        unsafe {
            let _ = nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGTERM, &action);
            let _ = nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGINT, &action);
        }
    });
}

#[cfg(not(unix))]
fn install_signal_handlers() {
    // Windows needs SetConsoleCtrlHandler, and a GUI-hosted process needs
    // WM_QUERYENDSESSION instead — neither is reachable until the Windows port
    // lands and the shell technology is settled.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_the_extraction_dir_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("soli_shutdown_test_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("app")).expect("scratch");
        std::fs::write(dir.join("app/x.sl"), b"secret").expect("write");

        register(0, &dir);
        remove_extraction_dir();
        assert!(!dir.exists(), "decrypted tree must be gone");

        // A second pass must not panic — shutdown can be reached twice (a
        // signal and an explicit quit racing).
        remove_extraction_dir();
    }

    #[test]
    fn stopping_with_no_database_is_a_no_op() {
        DB_PID.store(0, Ordering::SeqCst);
        stop_database(Duration::from_millis(10));
    }

    #[test]
    fn request_is_observable() {
        REQUESTED.store(false, Ordering::SeqCst);
        assert!(!is_requested());
        request();
        assert!(is_requested());
        REQUESTED.store(false, Ordering::SeqCst);
    }
}
