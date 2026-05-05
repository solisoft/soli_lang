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

    // SEC-017: the test runner (`soli test`) signals child processes
    // (test-server `soli serve` instances) that they should permit
    // loopback / private-IP outbound requests by setting
    // `SOLI_INTERNAL_TEST_RUNNER=1`. Translate that env signal *once*
    // here, into the in-process `AtomicBool` the SSRF blocklist
    // consults — after this point env vars are never trusted again
    // for that decision. The variable name is undocumented for
    // operator use; setting it manually in production is equivalent
    // to disabling the SSRF guardrail and should never be done.
    if std::env::var("SOLI_INTERNAL_TEST_RUNNER").as_deref() == Ok("1") {
        solilang::interpreter::builtins::http_class::enable_ssrf_test_mode();
    }

    cli::run();
}
