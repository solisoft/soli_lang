//! A wall-clock budget for one unit of interpreted work.
//!
//! A worker thread is one of a small pool, and it runs application code with
//! request data in hand. `RESPONSE_WAIT_TIMEOUT_SECS` only makes hyper give up
//! and answer 504: the worker itself stays in the handler forever, so a request
//! that steers a handler into a long loop (a big `range().map`, a quadratic
//! string scan, an accidental `while true`) removes a worker permanently, and a
//! handful of them take the server down. There was no watchdog anywhere.
//!
//! Both engines check this deadline on their backward jumps and function calls
//! — the only places a program can spend unbounded time without returning — and
//! raise an ordinary catchable error when it passes. Checking a thread-local
//! `Instant` costs a few nanoseconds and only every Nth iteration, so the hot
//! path is unaffected.
//!
//! Deliberately *not* a CPU-time limit: a handler that spends thirty seconds
//! waiting on a slow database is doing exactly what it should, and the DB and
//! HTTP clients have their own timeouts. This bounds time spent *executing*.

use std::cell::Cell;
use std::time::{Duration, Instant};

thread_local! {
    /// When the current unit of work must stop, if it is bounded at all.
    static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Countdown to the next real clock read.
    static COUNTDOWN: Cell<u32> = const { Cell::new(CHECK_INTERVAL) };
}

/// How many loop iterations pass between clock reads.
///
/// `Instant::now` is cheap but not free, and this sits on the hottest path in
/// the interpreter. A few thousand iterations of even the slowest opcode is far
/// under a millisecond, so the deadline is still honoured promptly.
const CHECK_INTERVAL: u32 = 4096;

/// The default budget for one request handler, in seconds.
///
/// Generous on purpose: this is a runaway-loop backstop, not a performance
/// policy, and a legitimate slow handler (a report, a large export) must not
/// trip it. `SOLI_HANDLER_TIMEOUT_SECS=0` disables it.
pub fn default_budget_secs() -> u64 {
    std::env::var("SOLI_HANDLER_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(30)
}

/// Start a budget on this thread, returning a guard that restores the previous
/// one when dropped. Nested units of work therefore cannot extend an outer
/// budget, and a handler cannot leak its deadline to whatever the worker runs
/// next.
pub fn enter(budget: Duration) -> DeadlineGuard {
    let previous = DEADLINE.with(|d| d.replace(Some(Instant::now() + budget)));
    COUNTDOWN.with(|c| c.set(CHECK_INTERVAL));
    DeadlineGuard { previous }
}

/// Start the default request budget, or nothing when it is disabled.
pub fn enter_default() -> DeadlineGuard {
    match default_budget_secs() {
        0 => DeadlineGuard {
            previous: DEADLINE.with(|d| d.get()),
        },
        secs => enter(Duration::from_secs(secs)),
    }
}

/// Restores the enclosing deadline on drop, on every path including a panic.
pub struct DeadlineGuard {
    previous: Option<Instant>,
}

impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        DEADLINE.with(|d| d.set(self.previous));
        COUNTDOWN.with(|c| c.set(CHECK_INTERVAL));
    }
}

/// Has the current unit of work run out of time?
///
/// Reads the clock only every [`CHECK_INTERVAL`] calls, so callers can put this
/// directly on a dispatch loop.
#[inline]
pub fn expired() -> bool {
    let remaining = COUNTDOWN.with(|c| {
        let next = c.get().wrapping_sub(1);
        c.set(next);
        next
    });
    if remaining != 0 {
        return false;
    }
    COUNTDOWN.with(|c| c.set(CHECK_INTERVAL));
    DEADLINE.with(|d| match d.get() {
        Some(deadline) => Instant::now() >= deadline,
        None => false,
    })
}

/// The message raised when a budget runs out.
pub fn timeout_message() -> String {
    format!(
        "execution exceeded the {}s handler budget — a loop that never finishes, \
         or work that belongs in a background job (SOLI_HANDLER_TIMEOUT_SECS \
         raises or disables it)",
        default_budget_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no budget entered, nothing ever expires — scripts, the REPL and
    /// tests must be unaffected.
    #[test]
    fn without_a_budget_nothing_expires() {
        for _ in 0..(CHECK_INTERVAL * 2) {
            assert!(!expired());
        }
    }

    /// A budget that has already elapsed reports expiry once the countdown
    /// reaches a clock read.
    #[test]
    fn an_elapsed_budget_expires() {
        let _guard = enter(Duration::from_nanos(1));
        std::thread::sleep(Duration::from_millis(2));
        let mut hit = false;
        for _ in 0..(CHECK_INTERVAL + 1) {
            if expired() {
                hit = true;
                break;
            }
        }
        assert!(hit, "an elapsed deadline must be reported");
    }

    /// A live budget does not fire.
    #[test]
    fn a_live_budget_does_not_expire() {
        let _guard = enter(Duration::from_secs(60));
        for _ in 0..(CHECK_INTERVAL * 2) {
            assert!(!expired());
        }
    }

    /// The guard must restore the enclosing budget, so a nested unit of work
    /// cannot leave the thread bounded (or unbounded) afterwards.
    #[test]
    fn the_guard_restores_the_previous_budget() {
        {
            let _outer = enter(Duration::from_secs(60));
            {
                let _inner = enter(Duration::from_nanos(1));
            }
            // The inner guard dropped; the outer budget is live again.
            std::thread::sleep(Duration::from_millis(2));
            for _ in 0..(CHECK_INTERVAL + 1) {
                assert!(!expired(), "the outer budget must be restored");
            }
        }
        for _ in 0..(CHECK_INTERVAL + 1) {
            assert!(!expired(), "no budget outside the guard");
        }
    }
}
