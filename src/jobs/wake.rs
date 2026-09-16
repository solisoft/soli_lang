//! The poller's sleep, interruptible.
//!
//! [`run_poller`](super::engine) used to `std::thread::sleep` for a fixed
//! interval, which set the floor on how much idle chatter a process made
//! against a shared database: one cron read plus one claim round-trip every
//! `poll_ms`, whether or not anything had happened. Several dev apps open at
//! once made that the dominant load on the database.
//!
//! Waiting on a condvar instead lets the interval be a *backstop* rather than
//! the mechanism. Anything that knows work has appeared — the changefeed
//! subscriber in [`super::push`], an in-process enqueue — calls [`notify`], and
//! the poller wakes at once.
//!
//! The flag is sticky on purpose: a `notify` that lands while the poller is
//! mid-pass is remembered, so the pass already running does not swallow a
//! wake-up for work it had not yet seen.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// An interruptible sleep. One of these is process-global (the poller's); tests
/// make their own so they don't contend over it.
#[derive(Default)]
pub struct Wake {
    /// Set by [`Wake::notify`], cleared by the waiter once it has observed it.
    pending: Mutex<bool>,
    condvar: Condvar,
}

impl Wake {
    /// Record that there may be work now and wake a waiter. Cheap,
    /// non-blocking, safe from any thread or tokio task; redundant calls
    /// collapse into one.
    pub fn notify(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = true;
            self.condvar.notify_all();
        }
    }

    /// Sleep until [`Wake::notify`] is called or `timeout` elapses.
    ///
    /// Returns `true` when a notification ended the wait. Callers use that only
    /// for logging — either way the next thing they do is look for work.
    pub fn wait(&self, timeout: Duration) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            // A poisoned lock would otherwise turn the poller into a busy loop.
            std::thread::sleep(timeout);
            return false;
        };

        // Consume a notification that arrived while the previous pass was
        // running, rather than sleeping through work already waiting.
        if *pending {
            *pending = false;
            return true;
        }

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Ok((guard, _)) = self.condvar.wait_timeout(pending, remaining) else {
                return false;
            };
            pending = guard;
            if *pending {
                *pending = false;
                return true;
            }
            // Spurious wake-up: a condvar may return without `notify` running.
        }
    }
}

static WAKE: OnceLock<Wake> = OnceLock::new();

fn shared() -> &'static Wake {
    WAKE.get_or_init(Wake::default)
}

/// Tell the job poller there may be work now.
pub fn notify() {
    shared().notify();
}

/// Sleep the poller until [`notify`] is called or `timeout` elapses.
pub fn wait(timeout: Duration) -> bool {
    shared().wait(timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_notification_before_the_wait_is_not_lost() {
        let w = Wake::default();
        w.notify();
        let started = Instant::now();
        assert!(w.wait(Duration::from_secs(30)));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "a pending notification must return immediately, not sleep"
        );
    }

    #[test]
    fn the_flag_is_consumed_by_one_wait() {
        let w = Wake::default();
        w.notify();
        assert!(w.wait(Duration::from_secs(30)));
        // The second wait has nothing pending and must honour its timeout.
        let started = Instant::now();
        assert!(!w.wait(Duration::from_millis(80)));
        assert!(started.elapsed() >= Duration::from_millis(60));
    }

    #[test]
    fn repeated_notifications_collapse_into_one() {
        let w = Wake::default();
        w.notify();
        w.notify();
        w.notify();
        assert!(w.wait(Duration::from_secs(30)));
        assert!(!w.wait(Duration::from_millis(80)));
    }

    #[test]
    fn a_notification_during_the_wait_ends_it() {
        let w = Arc::new(Wake::default());
        let signaller = Arc::clone(&w);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            signaller.notify();
        });
        let started = Instant::now();
        assert!(w.wait(Duration::from_secs(30)));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the waiter must be woken by notify, not by the timeout"
        );
    }

    #[test]
    fn the_timeout_is_honoured_with_no_notification() {
        let w = Wake::default();
        let started = Instant::now();
        assert!(!w.wait(Duration::from_millis(120)));
        assert!(started.elapsed() >= Duration::from_millis(100));
    }
}
