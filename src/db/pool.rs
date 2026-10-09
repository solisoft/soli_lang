//! Connection pool for the SQL adapters.
//!
//! It replaces r2d2, whose defaults cost a network round trip per query:
//! `test_on_check_out` validated every checkout (`simple_query("")` on
//! Postgres, `ping` on MySQL), and `r2d2_mysql` pinged again on every
//! check-in. A query that should take one round trip took two on Postgres
//! and three on MySQL.
//!
//! Here a connection is validated only when there is a reason to doubt it:
//!
//! * it sat idle longer than the manager's `ping_after` window (the HikariCP
//!   rule — a connection used half a second ago is still there), or
//! * the last operation on it failed (`Pooled::mark_suspect`).
//!
//! Idle connections are handed out LIFO, so a busy pool keeps reusing the
//! warmest ones and never pays for a ping at all.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use web_time::{Duration, Instant};

/// How long a returned network connection is trusted without a ping.
/// HikariCP's figure: a connection used this recently is still there.
pub const PING_AFTER: Duration = Duration::from_millis(500);

/// How a pool opens and checks the connections of one driver.
pub trait Manager: Send + Sync + 'static {
    type Conn: Send + 'static;

    fn connect(&self) -> Result<Self::Conn, String>;

    /// A round trip that proves the connection still answers.
    fn ping(&self, conn: &mut Self::Conn) -> bool;

    /// A local check, no I/O: true when the driver already knows the
    /// connection is dead. Runs on every check-in, so it must be free.
    fn is_broken(&self, _conn: &mut Self::Conn) -> bool {
        false
    }
}

/// An idle connection, with the time it was last returned. `None` means it
/// must be pinged before its next use.
type Idle<C> = (C, Option<Instant>);

struct State<C> {
    idle: Vec<Idle<C>>,
    /// Connections in existence: idle, checked out, or being opened.
    open: usize,
}

struct Shared<M: Manager> {
    manager: M,
    max: usize,
    timeout: Duration,
    /// `None`: never ping (a local SQLite handle does not go stale).
    ping_after: Option<Duration>,
    state: Mutex<State<M::Conn>>,
    returned: Condvar,
}

impl<M: Manager> Shared<M> {
    fn lock(&self) -> MutexGuard<'_, State<M::Conn>> {
        // A panic while holding the lock leaves the counters consistent: every
        // critical section below is a push, a pop or an increment.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn forget_one(&self) {
        self.lock().open -= 1;
        self.returned.notify_one();
    }
}

pub struct Pool<M: Manager>(Arc<Shared<M>>);

impl<M: Manager> Clone for Pool<M> {
    fn clone(&self) -> Self {
        Pool(Arc::clone(&self.0))
    }
}

impl<M: Manager> Pool<M> {
    /// A pool of at most `max` connections, opened on demand. `get` waits up
    /// to `timeout` for one to come back when all of them are checked out.
    pub fn new(manager: M, max: usize, timeout: Duration, ping_after: Option<Duration>) -> Self {
        Pool(Arc::new(Shared {
            manager,
            max: max.max(1),
            timeout,
            ping_after,
            state: Mutex::new(State {
                idle: Vec::new(),
                open: 0,
            }),
            returned: Condvar::new(),
        }))
    }

    pub fn get(&self) -> Result<Pooled<M>, String> {
        let shared = &self.0;
        let deadline = Instant::now() + shared.timeout;
        let mut state = shared.lock();
        loop {
            if let Some((mut conn, returned_at)) = state.idle.pop() {
                let fresh = match (shared.ping_after, returned_at) {
                    (None, _) => true,
                    (Some(window), Some(at)) => at.elapsed() < window,
                    (Some(_), None) => false,
                };
                if fresh {
                    return Ok(self.wrap(conn));
                }
                drop(state);
                if shared.manager.ping(&mut conn) {
                    return Ok(self.wrap(conn));
                }
                drop(conn);
                state = shared.lock();
                state.open -= 1;
                continue;
            }
            if state.open < shared.max {
                state.open += 1;
                drop(state);
                // Connect outside the lock: a slow handshake must not stall
                // the threads returning connections.
                return match shared.manager.connect() {
                    Ok(conn) => Ok(self.wrap(conn)),
                    Err(e) => {
                        shared.forget_one();
                        Err(e)
                    }
                };
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "timed out after {}s waiting for a connection (all {} in use)",
                    shared.timeout.as_secs(),
                    shared.max
                ));
            }
            state = shared
                .returned
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn wrap(&self, conn: M::Conn) -> Pooled<M> {
        Pooled {
            conn: Some(conn),
            suspect: false,
            pool: Arc::clone(&self.0),
        }
    }
}

/// A checked-out connection; it goes back to the pool when dropped.
pub struct Pooled<M: Manager> {
    conn: Option<M::Conn>,
    suspect: bool,
    pool: Arc<Shared<M>>,
}

impl<M: Manager> Pooled<M> {
    /// Ping this connection before it is next handed out. Called after a
    /// failed operation, which may have been the connection dying.
    pub fn mark_suspect(&mut self) {
        self.suspect = true;
    }
}

impl<M: Manager> Deref for Pooled<M> {
    type Target = M::Conn;

    fn deref(&self) -> &M::Conn {
        self.conn
            .as_ref()
            .expect("pooled connection already returned")
    }
}

impl<M: Manager> DerefMut for Pooled<M> {
    fn deref_mut(&mut self) -> &mut M::Conn {
        self.conn
            .as_mut()
            .expect("pooled connection already returned")
    }
}

impl<M: Manager> Drop for Pooled<M> {
    fn drop(&mut self) {
        let Some(mut conn) = self.conn.take() else {
            return;
        };
        let shared = &self.pool;
        if shared.manager.is_broken(&mut conn) {
            drop(conn);
            shared.forget_one();
            return;
        }
        let returned_at = (!self.suspect).then(Instant::now);
        shared.lock().idle.push((conn, returned_at));
        shared.returned.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counting {
        connects: AtomicUsize,
        pings: AtomicUsize,
        fail_connect: bool,
        ping_ok: bool,
    }

    impl Manager for Arc<Counting> {
        type Conn = usize;

        fn connect(&self) -> Result<usize, String> {
            if self.fail_connect {
                return Err("refused".into());
            }
            Ok(self.connects.fetch_add(1, Ordering::SeqCst))
        }

        fn ping(&self, _conn: &mut usize) -> bool {
            self.pings.fetch_add(1, Ordering::SeqCst);
            self.ping_ok
        }
    }

    fn counting(ping_ok: bool) -> Arc<Counting> {
        Arc::new(Counting {
            ping_ok,
            ..Default::default()
        })
    }

    const LONG: Duration = Duration::from_secs(3600);

    #[test]
    fn a_recently_used_connection_is_reused_without_a_ping() {
        let manager = counting(true);
        let pool = Pool::new(Arc::clone(&manager), 4, LONG, Some(LONG));
        for _ in 0..100 {
            let conn = pool.get().unwrap();
            assert_eq!(*conn, 0);
        }
        assert_eq!(manager.connects.load(Ordering::SeqCst), 1);
        assert_eq!(manager.pings.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn an_idle_connection_is_pinged_and_a_dead_one_replaced() {
        let manager = counting(false);
        let pool = Pool::new(Arc::clone(&manager), 4, LONG, Some(Duration::ZERO));
        drop(pool.get().unwrap());
        let conn = pool.get().unwrap();
        assert_eq!(manager.pings.load(Ordering::SeqCst), 1);
        assert_eq!(
            *conn, 1,
            "the dead connection was dropped and a new one opened"
        );
    }

    #[test]
    fn a_suspect_connection_is_pinged_even_inside_the_window() {
        let manager = counting(true);
        let pool = Pool::new(Arc::clone(&manager), 4, LONG, Some(LONG));
        let mut conn = pool.get().unwrap();
        conn.mark_suspect();
        drop(conn);
        let conn = pool.get().unwrap();
        assert_eq!(*conn, 0);
        assert_eq!(manager.pings.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failed_connect_reports_its_own_error_and_frees_the_slot() {
        let manager = Arc::new(Counting {
            fail_connect: true,
            ..Default::default()
        });
        let pool = Pool::new(manager, 1, LONG, None);
        assert_eq!(pool.get().err().unwrap(), "refused");
        assert_eq!(
            pool.get().err().unwrap(),
            "refused",
            "the slot was released"
        );
    }

    #[test]
    fn a_full_pool_times_out_then_serves_a_returned_connection() {
        let pool = Pool::new(counting(true), 1, Duration::from_millis(20), None);
        let held = pool.get().unwrap();
        assert!(pool.get().err().unwrap().contains("timed out"));

        let waiter = {
            let pool = Pool::new(counting(true), 1, LONG, None);
            let first = pool.get().unwrap();
            let other = pool.clone();
            let handle = std::thread::spawn(move || *other.get().unwrap());
            std::thread::sleep(Duration::from_millis(20));
            drop(first);
            handle.join().unwrap()
        };
        assert_eq!(waiter, 0, "the waiter got the returned connection");
        drop(held);
    }
}
