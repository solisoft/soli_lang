//! Soli CLI: Execute files or run the REPL.

mod cli;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Install a SIGTERM/SIGINT handler that exits via `process::exit(0)` so atexit
/// handlers run on graceful shutdown — including the LLVM coverage profile
/// flush used by `cargo llvm-cov`. Without this, coverage data from the
/// `soli serve` subprocess is lost when integration tests kill it.
#[cfg(unix)]
fn install_graceful_shutdown_handler() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Once;

    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};

        static EXITING: AtomicBool = AtomicBool::new(false);

        extern "C" fn handle_signal(_sig: i32) {
            if EXITING.swap(true, Ordering::SeqCst) {
                return;
            }
            std::process::exit(0);
        }

        let action = SigAction::new(
            SigHandler::Handler(handle_signal),
            SaFlags::empty(),
            SigSet::empty(),
        );
        // Best-effort: ignore failures — production behavior is unchanged
        // if the install fails for any reason.
        unsafe {
            let _ = sigaction(Signal::SIGTERM, &action);
            let _ = sigaction(Signal::SIGINT, &action);
        }
    });
}

#[cfg(not(unix))]
fn install_graceful_shutdown_handler() {}

fn main() {
    install_graceful_shutdown_handler();
    cli::run();
}
