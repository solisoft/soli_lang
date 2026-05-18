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

/// SEC-084: the test runner mints a fresh UUID v4 per run and hands it
/// to each child server via `SOLI_INTERNAL_TEST_RUNNER`. The previous
/// gate accepted the literal `"1"`, which any deploy-script env leak
/// (`export SOLI_INTERNAL_TEST_RUNNER=1` on a build machine, a CI step
/// that re-uses a parent shell's environment, ...) would silently turn
/// into "SSRF guardrail off". Requiring a well-formed UUID raises the
/// bar — operators don't accidentally generate UUIDs by mistake.
///
/// This is a configuration foot-gun reduction, not a cryptographic
/// gate: a determined attacker who can write to the server process's
/// environment already owns the box. The goal is to refuse the easy
/// accidental misconfigurations.
fn is_internal_test_runner_token(value: &str) -> bool {
    match uuid::Uuid::parse_str(value.trim()) {
        Ok(uuid) => uuid.get_version_num() == 4,
        Err(_) => false,
    }
}

fn main() {
    install_graceful_shutdown_handler();

    // SEC-017 / SEC-084: the test runner (`soli test`) signals child
    // processes (test-server `soli serve` instances) that they should
    // permit loopback / private-IP outbound requests by setting
    // `SOLI_INTERNAL_TEST_RUNNER=<uuid-v4>`. Translate that env signal
    // *once* here, into the in-process `AtomicBool` the SSRF blocklist
    // consults — after this point env vars are never trusted again for
    // that decision. The value must be a well-formed UUID v4 minted by
    // the test runner; legacy `=1` payloads (and any other shape) are
    // rejected so an accidental env leak in production cannot disable
    // the SSRF guardrail.
    if let Ok(value) = std::env::var("SOLI_INTERNAL_TEST_RUNNER") {
        if is_internal_test_runner_token(&value) {
            solilang::interpreter::builtins::http_class::enable_ssrf_test_mode();
        }
    }

    cli::run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_token_accepts_uuid_v4() {
        // A real UUID v4 — what the test runner actually mints.
        let token = uuid::Uuid::new_v4().to_string();
        assert!(
            is_internal_test_runner_token(&token),
            "expected UUID v4 to be accepted: {}",
            token
        );
        // Tolerate surrounding whitespace from accidental env-var quoting.
        assert!(is_internal_test_runner_token(&format!("  {}  ", token)));
    }

    #[test]
    fn test_runner_token_rejects_legacy_and_garbage_values() {
        // SEC-084: the previous gate accepted "1" — that's the exact
        // foot-gun this refactor closes. Anything other than a v4 UUID
        // must be rejected outright.
        for v in [
            "",
            "1",
            "true",
            "yes",
            "on",
            // UUID v1 / v3 / v5 — wrong version.
            "00000000-0000-0000-0000-000000000000",
            "550e8400-e29b-11d4-a716-446655440000", // v1
            // Plausible-looking strings that aren't UUIDs at all.
            "abcdef0123456789",
            "test-runner",
            "550e8400-e29b-41d4-a716-44665544", // truncated
        ] {
            assert!(
                !is_internal_test_runner_token(v),
                "expected {:?} to be rejected",
                v
            );
        }
    }
}
