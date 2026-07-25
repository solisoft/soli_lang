//! Lifecycle state for the HTTP server: readiness, graceful drain, and the
//! in-flight connection count that drain waits on.
//!
//! Before this module, `SIGTERM` went straight to `std::process::exit(0)` in
//! `main.rs`, so every rolling deploy, container restart, or `systemctl restart`
//! truncated whatever was mid-flight. The drain sequence is now:
//!
//! 1. `begin_drain()` — `/_ready` starts answering `503` so a load balancer
//!    takes this instance out of rotation, and newly accepted requests get
//!    `503` too.
//! 2. the accept loop stops taking new connections.
//! 3. we wait for [`in_flight`] to reach zero, or for [`grace_period`] to
//!    elapse, whichever comes first.
//! 4. `std::process::exit(0)` — deliberately `exit`, not `abort`, so `atexit`
//!    handlers still run (notably the `cargo llvm-cov` profile flush that
//!    `main.rs` documents).
//!
//! State is process-global rather than threaded through as `Arc`s because the
//! readiness probe is answered deep inside `handle_hyper_request`, far from
//! where the server owns its state.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

// The server's fault isolation is `std::panic::catch_unwind`, which is a no-op
// under `panic = "abort"` — the process aborts at the panic site instead. That
// silently turns the per-request guard in `dispatch_http_request`, the
// web-worker restart loop, and the background-job pool restart loop into dead
// code, so one panic in one worker takes down every worker in the process.
//
// This shipped that way until it was caught in review: `[profile.release]` set
// `panic = "abort"`, so none of those three nets existed in any released binary.
// Failing the build is the only check that cannot itself be forgotten.
#[cfg(panic = "abort")]
compile_error!(
    "Soli's server requires unwinding panics: `catch_unwind` is a no-op under \
     `panic = \"abort\"`, which disables per-request panic containment and both \
     worker-restart supervisors. Remove `panic = \"abort\"` from the active \
     cargo profile."
);

/// Set once the worker pool has booted and the listener is bound.
static READY: AtomicBool = AtomicBool::new(false);

/// Set when a shutdown signal arrives. One-way — nothing clears it.
static DRAINING: AtomicBool = AtomicBool::new(false);

/// Connections currently being served. Maintained by [`ConnectionGuard`].
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Default seconds to wait for in-flight connections before exiting anyway.
/// Comfortably under Kubernetes' default 30s `terminationGracePeriodSeconds`
/// so the orchestrator sees a clean exit rather than a `SIGKILL`.
const DEFAULT_GRACE_SECS: u64 = 25;

/// Mark the server ready to receive traffic. Called once the worker pool is up.
pub fn mark_ready() {
    READY.store(true, Ordering::Release);
}

/// Whether the server should accept new traffic: booted, and not draining.
///
/// This is the readiness signal (`/_ready`), not liveness — during a drain the
/// process is perfectly healthy, it just wants to stop being routed to.
pub fn is_ready() -> bool {
    READY.load(Ordering::Acquire) && !is_draining()
}

/// Whether a shutdown is in progress.
pub fn is_draining() -> bool {
    DRAINING.load(Ordering::Acquire)
}

/// Enter the draining state. Returns `false` if a drain was already underway,
/// which is how a second `SIGTERM` is recognised as "stop waiting, exit now".
pub fn begin_drain() -> bool {
    !DRAINING.swap(true, Ordering::AcqRel)
}

/// Connections currently being served.
pub fn in_flight() -> usize {
    IN_FLIGHT.load(Ordering::Acquire)
}

/// How long to wait for in-flight connections before exiting anyway.
/// Override with `SOLI_SHUTDOWN_GRACE_SECS`; `0` exits immediately.
pub fn grace_period() -> Duration {
    let secs = std::env::var("SOLI_SHUTDOWN_GRACE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_GRACE_SECS);
    Duration::from_secs(secs)
}

/// RAII counter for one in-flight connection. Held for the lifetime of a
/// `serve_connection_with_upgrades` future, so the count falls back to zero
/// even when a connection ends by error or panic.
pub struct ConnectionGuard;

impl ConnectionGuard {
    pub fn new() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Default for ConnectionGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three states a readiness probe must distinguish. Uses the real
    /// globals, so it must not run concurrently with the drain test below —
    /// both are in this one test to keep them ordered.
    #[test]
    fn readiness_reflects_boot_then_drain() {
        // Before boot: not ready, even though nothing is wrong.
        assert!(!is_ready(), "must not be ready before mark_ready()");

        mark_ready();
        assert!(is_ready(), "ready once booted");
        assert!(!is_draining());

        // First drain wins; a second signal reports "already draining" so the
        // caller can treat it as an immediate-exit request.
        assert!(begin_drain(), "first drain returns true");
        assert!(!begin_drain(), "second drain returns false");

        // Draining is not ready — this is what pulls the instance out of the
        // load balancer — but it is still live and serving what it has.
        assert!(is_draining());
        assert!(!is_ready(), "draining must fail readiness");
    }

    #[test]
    fn connection_guard_counts_and_releases() {
        let before = in_flight();
        {
            let _a = ConnectionGuard::new();
            assert_eq!(in_flight(), before + 1);
            {
                let _b = ConnectionGuard::new();
                assert_eq!(in_flight(), before + 2);
            }
            assert_eq!(in_flight(), before + 1, "inner guard released on drop");
        }
        assert_eq!(in_flight(), before, "all guards released");
    }

    #[test]
    fn grace_period_defaults_and_parses() {
        // No env var set in the test process → the default.
        assert_eq!(grace_period(), Duration::from_secs(DEFAULT_GRACE_SECS));
    }
}
