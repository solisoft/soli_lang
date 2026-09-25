//! One background writer per application, fed over a bounded channel.
//!
//! The error tracker, the slow-query tracker and the notifier all record
//! something on the request path and write it somewhere slow. Each needs the
//! same guarantees, so they share this: recording never blocks a worker (a full
//! queue drops the item and counts it), each application gets its own thread
//! with its own tenant bound, and a writer found dead is replaced on the next
//! item instead of every later item being reported as a queue overflow.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};

use super::tenant::TenantId;

/// What went wrong with recording, per application, since the process
/// started. Kept apart from the writer so a restarted writer keeps the counts.
#[derive(Default)]
pub(crate) struct Stats {
    /// Queue full: items arrived faster than the writer could store them.
    pub(crate) dropped: AtomicU64,
    /// Items whose write failed (a database error, or a panic in the writer
    /// caught around that write).
    pub(crate) failed: AtomicU64,
    /// Times the writer was found gone and started again.
    pub(crate) restarts: AtomicU64,
    /// Items lost because no writer could take them.
    pub(crate) lost: AtomicU64,
    /// Items of a new group counted in (or refused for) the overflow.
    pub(crate) overflowed: AtomicU64,
}

/// A copy of [`Stats`] for a dashboard.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatsSnapshot {
    pub dropped: u64,
    pub failed: u64,
    pub restarts: u64,
    pub lost: u64,
    pub overflowed: u64,
}

impl Stats {
    pub(crate) fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            dropped: self.dropped.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            restarts: self.restarts.load(Ordering::Relaxed),
            lost: self.lost.load(Ordering::Relaxed),
            overflowed: self.overflowed.load(Ordering::Relaxed),
        }
    }
}

/// One application's writer: its channel, a generation that tells a stale
/// sender from its replacement, and the counters that outlive both.
struct Writer<T> {
    sender: Option<SyncSender<T>>,
    generation: u64,
    stats: Arc<Stats>,
}

/// Starts a writer thread for `tenant` reading `receiver`; `false` when it
/// could not be started.
pub(crate) type Spawn<'a, T> = &'a dyn Fn(TenantId, Receiver<T>, Arc<Stats>) -> bool;

/// The writers of one kind, one per application.
pub(crate) struct Writers<T> {
    /// `[errors]`, `[slow-queries]`: how this kind names itself on stderr.
    label: &'static str,
    queue_cap: usize,
    registry: OnceLock<Mutex<HashMap<TenantId, Writer<T>>>>,
}

impl<T> Writers<T> {
    pub(crate) const fn new(label: &'static str, queue_cap: usize) -> Self {
        Writers {
            label,
            queue_cap,
            registry: OnceLock::new(),
        }
    }

    fn registry(&self) -> &Mutex<HashMap<TenantId, Writer<T>>> {
        self.registry.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// This application's counters, since the process started.
    pub(crate) fn stats(&self, tenant: TenantId) -> StatsSnapshot {
        let writers = self.registry().lock().unwrap_or_else(|e| e.into_inner());
        writers
            .get(&tenant)
            .map(|w| w.stats.snapshot())
            .unwrap_or_default()
    }

    /// Hand an item to the application's writer, starting it if needed.
    ///
    /// A full queue drops the item and counts it as `dropped`. A writer that
    /// is gone (its thread died) is a different fault: its sender is forgotten,
    /// a new writer is started and the item retried once.
    pub(crate) fn deliver(&self, tenant: TenantId, item: T, spawn: Spawn<'_, T>) {
        let mut item = item;
        for attempt in 0..2 {
            let Some((sender, generation, stats)) = self.writer_for(tenant, spawn) else {
                return;
            };
            match sender.try_send(item) {
                Ok(()) => return,
                Err(TrySendError::Full(_)) => {
                    stats.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(TrySendError::Disconnected(back)) => {
                    self.forget(tenant, generation);
                    stats.restarts.fetch_add(1, Ordering::Relaxed);
                    eprintln!("[{}] the writer had stopped; starting it again", self.label);
                    if attempt == 1 {
                        stats.lost.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    item = back;
                }
            }
        }
    }

    /// The live sender for `tenant`, starting a writer when there is none.
    fn writer_for(
        &self,
        tenant: TenantId,
        spawn: Spawn<'_, T>,
    ) -> Option<(SyncSender<T>, u64, Arc<Stats>)> {
        let mut writers = self.registry().lock().unwrap_or_else(|e| e.into_inner());
        let writer = writers.entry(tenant).or_insert_with(|| Writer {
            sender: None,
            generation: 0,
            stats: Arc::new(Stats::default()),
        });
        if let Some(sender) = &writer.sender {
            return Some((sender.clone(), writer.generation, writer.stats.clone()));
        }
        let (sender, receiver) = mpsc::sync_channel(self.queue_cap);
        if !spawn(tenant, receiver, writer.stats.clone()) {
            writer.stats.lost.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        writer.generation += 1;
        writer.sender = Some(sender.clone());
        Some((sender, writer.generation, writer.stats.clone()))
    }

    /// Drop a dead writer's sender, unless another thread already replaced it.
    fn forget(&self, tenant: TenantId, generation: u64) {
        let mut writers = self.registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(writer) = writers.get_mut(&tenant) {
            if writer.generation == generation {
                writer.sender = None;
            }
        }
    }
}

/// Start a named writer thread that runs with `tenant` and the server's
/// runtime bound — a writer's database and HTTP calls need both, and neither
/// crosses `spawn` on its own.
pub(crate) fn spawn_thread(
    label: &'static str,
    tenant: TenantId,
    body: impl FnOnce() + Send + 'static,
) -> bool {
    let Some(handle) = super::get_tokio_handle() else {
        return false;
    };
    let spawned = std::thread::Builder::new()
        .name(label.to_string())
        .spawn(move || {
            super::tenant::bind_current(tenant);
            super::set_tokio_handle(handle);
            body();
        });
    match spawned {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[{label}] could not start the writer: {e}");
            false
        }
    }
}

/// Run `write`, turning a panic into an error, so one bad write costs its own
/// items and never the writer thread.
pub(crate) fn fenced<R>(write: impl FnOnce() -> Result<R, String>) -> Result<R, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(write)) {
        Ok(result) => result,
        Err(panic) => {
            let what = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_string());
            Err(format!("the writer panicked: {what}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_dead_writer_is_replaced_not_reported_as_overflow() {
        static WRITERS: Writers<&'static str> = Writers::new("test", 8);
        let tenant = TenantId(0xE77_0001);
        let spawned = AtomicUsize::new(0);
        let kept: Mutex<Vec<Receiver<&'static str>>> = Mutex::new(Vec::new());
        let spawn = |_: TenantId, rx: Receiver<&'static str>, _: Arc<Stats>| {
            // The first writer dies at once; the second stays up.
            if spawned.fetch_add(1, Ordering::SeqCst) > 0 {
                kept.lock().unwrap().push(rx);
            }
            true
        };
        WRITERS.deliver(tenant, "a", &spawn);
        WRITERS.deliver(tenant, "b", &spawn);
        assert_eq!(
            spawned.load(Ordering::SeqCst),
            2,
            "respawned once, then reused"
        );
        let received: Vec<&str> = kept.lock().unwrap()[0].try_iter().collect();
        assert_eq!(received, vec!["a", "b"], "the retried item arrived");
        let stats = WRITERS.stats(tenant);
        assert_eq!(stats.restarts, 1);
        assert_eq!(stats.dropped, 0);
        assert_eq!(stats.lost, 0);
    }

    #[test]
    fn a_full_queue_drops_and_counts() {
        static WRITERS: Writers<u32> = Writers::new("test", 1);
        let tenant = TenantId(0xE77_0002);
        let kept: Mutex<Vec<Receiver<u32>>> = Mutex::new(Vec::new());
        let spawn = |_: TenantId, rx: Receiver<u32>, _: Arc<Stats>| {
            kept.lock().unwrap().push(rx);
            true
        };
        WRITERS.deliver(tenant, 1, &spawn);
        WRITERS.deliver(tenant, 2, &spawn);
        assert_eq!(WRITERS.stats(tenant).dropped, 1);
    }

    #[test]
    fn a_panic_is_an_error() {
        let result: Result<(), String> = fenced(|| panic!("boom"));
        assert_eq!(result.unwrap_err(), "the writer panicked: boom");
    }
}
