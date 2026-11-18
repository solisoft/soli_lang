//! Rules every server-side session store applies the same way.
//!
//! - **One writer per session id at a time.** A store that writes a session by
//!   loading it, changing it and saving it back (disk, SoliDB, SoliKV) raced
//!   `destroy` and `regenerate`: a write that loaded the session before the
//!   delete and saved it after brought the whole session back, `user_id`
//!   included, so a logout did not log out a client that kept writing. Each
//!   of those steps now holds the lock of its id ([`id_lock`]), and a write
//!   whose session is gone does nothing. The locks are per process: several
//!   processes sharing one store still need the store's own guarantees.
//! - **An absolute lifetime** ([`max_lifetime_secs`]): the TTL slides, so a
//!   session kept busy never expired, and neither did a stolen id kept busy.
//!   `SOLI_SESSION_MAX_LIFETIME` (seconds, default 30 days, `0` = none) caps
//!   it counted from creation; `session_regenerate()` starts a new session.
//!   The cookie driver already had it.
//! - **Reads keep a session alive too** ([`needs_refresh`]): the TTL moved
//!   only on writes, so a user who only read pages was signed out mid-visit.
//!   A read refreshes the stamp once a quarter of the TTL has passed, not on
//!   every request.
//! - **Expired sessions are swept from session creation** ([`cleanup_due`]):
//!   the sweep ran only from a path the server never takes, so expired
//!   sessions piled up for good, and any public page that writes the session
//!   let an anonymous client fill the disk or the collection.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

const LOCK_STRIPES: usize = 256;

static ID_LOCKS: LazyLock<Vec<Mutex<()>>> =
    LazyLock::new(|| (0..LOCK_STRIPES).map(|_| Mutex::new(())).collect());

/// The lock of `session_id`'s stripe, held for one load-change-save or one
/// delete. Two ids may share a stripe; nothing holds two at once, so that
/// costs some waiting and never a deadlock.
pub(crate) fn id_lock(session_id: &str) -> MutexGuard<'static, ()> {
    let hash = session_id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |acc, byte| {
            (acc ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    let stripe = &ID_LOCKS[(hash as usize) % LOCK_STRIPES];
    // A panic while holding the lock leaves nothing half-done in the lock
    // itself (it guards no data), so a poisoned one is still usable.
    stripe
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `SOLI_SESSION_MAX_LIFETIME` in seconds (`0` = no absolute lifetime), the
/// same setting and default as the cookie driver's.
pub(crate) fn max_lifetime_secs() -> u64 {
    static CACHED: LazyLock<u64> = LazyLock::new(|| {
        super::session_cookie::parse_max_lifetime(
            crate::platform::env::var("SOLI_SESSION_MAX_LIFETIME")
                .ok()
                .as_deref(),
        )
    });
    *CACHED
}

/// True when a session created at `created_secs` has outlived the absolute
/// lifetime at `now_secs`.
pub(crate) fn outlived(created_secs: u64, now_secs: u64, max_lifetime: u64) -> bool {
    max_lifetime > 0 && now_secs > created_secs.saturating_add(max_lifetime)
}

/// True when a read should refresh the session's stamp: a quarter of the TTL
/// has passed since it was last refreshed.
pub(crate) fn needs_refresh(last_secs: u64, now_secs: u64, ttl_secs: u64) -> bool {
    now_secs.saturating_sub(last_secs) > (ttl_secs / 4).max(1)
}

/// The shortest time between two sweeps of expired sessions.
const CLEANUP_INTERVAL_SECS: u64 = 60;

/// True (and the stamp moved) when the store's last sweep, kept in `last`,
/// is older than [`CLEANUP_INTERVAL_SECS`]. One caller wins a given window.
pub(crate) fn cleanup_due(last: &AtomicU64, now_secs: u64) -> bool {
    let previous = last.load(Ordering::Relaxed);
    if now_secs.saturating_sub(previous) < CLEANUP_INTERVAL_SECS {
        return false;
    }
    last.compare_exchange(previous, now_secs, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
}

pub(crate) fn now_secs() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_absolute_lifetime_counts_from_creation_and_zero_turns_it_off() {
        assert!(!outlived(1_000, 1_000 + 3_600, 3_600));
        assert!(outlived(1_000, 1_000 + 3_601, 3_600));
        assert!(!outlived(0, u64::MAX, 0));
    }

    #[test]
    fn a_read_refreshes_after_a_quarter_of_the_ttl() {
        assert!(!needs_refresh(1_000, 1_000 + 25, 100));
        assert!(needs_refresh(1_000, 1_000 + 26, 100));
        // A clock that went back never asks for a refresh.
        assert!(!needs_refresh(1_000, 900, 100));
    }

    #[test]
    fn one_sweep_per_interval() {
        let last = AtomicU64::new(0);
        assert!(cleanup_due(&last, 10_000));
        assert!(!cleanup_due(&last, 10_000 + CLEANUP_INTERVAL_SECS - 1));
        assert!(cleanup_due(&last, 10_000 + CLEANUP_INTERVAL_SECS));
    }

    #[test]
    fn the_same_id_always_takes_the_same_stripe() {
        let first = id_lock("1f0e2d3c-4b5a-6978-8796-a5b4c3d2e1f0");
        drop(first);
        let again = id_lock("1f0e2d3c-4b5a-6978-8796-a5b4c3d2e1f0");
        drop(again);
    }
}
