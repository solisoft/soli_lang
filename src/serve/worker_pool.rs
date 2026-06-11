//! Worker Pool Management
//!
//! This module provides worker pool management structures including:
//! - Hot reload version counters (shared between file watcher and workers)
//! - A single shared request queue drained by all workers

use crossbeam::channel;
use std::sync::atomic::AtomicU64;

use crate::serve::RequestData;

/// Hot reload version counters - shared between file watcher and workers.
/// Workers periodically check if versions changed and reload accordingly.
pub(crate) struct HotReloadVersions {
    /// Bumped (after the per-kind counters, with Release ordering) whenever
    /// ANY of the counters below changes. Workers check this one counter
    /// per loop tick and only scan the individual versions when it moved,
    /// instead of eight Acquire loads per tick.
    pub generation: AtomicU64,
    /// Incremented when controllers change
    pub controllers: AtomicU64,
    /// Incremented when middleware changes
    pub middleware: AtomicU64,
    /// Incremented when views change
    pub views: AtomicU64,
    /// Incremented when static files (CSS, JS) change
    pub static_files: AtomicU64,
    /// Incremented when routes.sl changes
    pub routes: AtomicU64,
    /// Incremented when view helpers change
    pub helpers: AtomicU64,
    /// Incremented when models change
    pub models: AtomicU64,
    /// Incremented when app/jobs/*_job.sl files change
    pub jobs: AtomicU64,
}

impl HotReloadVersions {
    pub(crate) fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            controllers: AtomicU64::new(0),
            middleware: AtomicU64::new(0),
            views: AtomicU64::new(0),
            static_files: AtomicU64::new(0),
            routes: AtomicU64::new(0),
            helpers: AtomicU64::new(0),
            models: AtomicU64::new(0),
            jobs: AtomicU64::new(0),
        }
    }
}

/// A single shared request queue drained by every worker.
///
/// crossbeam's bounded channel is multi-producer / multi-consumer: each worker
/// clones the same receiver and competes to pull the next request. Whichever
/// worker is free grabs it, so a request is never stranded behind a worker
/// parked in a slow handler while siblings sit idle.
///
/// This replaces an earlier per-worker design (one private channel per worker,
/// round-robin sends across them). That design avoided receiver contention but
/// caused head-of-line blocking: a request was committed to worker
/// `counter % N` at enqueue time regardless of whether that worker was busy, so
/// it could wait seconds in a busy worker's private queue even when other
/// workers were idle. The in-handler access-log timer only starts once a worker
/// dequeues, so that queue wait was invisible in the logs (fast "Nms" line, slow
/// wall-clock). A single shared queue removes the stranding entirely; receiver
/// contention on crossbeam's bounded MPMC is negligible at our request rates.
pub(crate) struct WorkerQueues {
    sender: channel::Sender<RequestData>,
    receiver: channel::Receiver<RequestData>,
}

impl WorkerQueues {
    pub(crate) fn new(num_workers: usize, capacity_per_worker: usize) -> Self {
        // Keep the same total backpressure capacity as the old per-worker
        // queues (num_workers * capacity_per_worker), just in one shared buffer.
        let (sender, receiver) = channel::bounded(num_workers * capacity_per_worker);
        Self { sender, receiver }
    }

    /// Get a sender into the shared queue. Cloning a crossbeam `Sender` is cheap.
    pub(crate) fn get_sender(&self) -> WorkerSender {
        WorkerSender {
            sender: self.sender.clone(),
        }
    }

    /// Get a receiver for a worker. Every worker shares the one queue, so the
    /// returned receivers are clones of the same channel end; `worker_id` is
    /// accepted for call-site symmetry but does not select a private queue.
    pub(crate) fn get_receiver(&self, _worker_id: usize) -> channel::Receiver<RequestData> {
        self.receiver.clone()
    }
}

/// A sender into the shared worker queue.
#[derive(Clone)]
pub(crate) struct WorkerSender {
    sender: channel::Sender<RequestData>,
}

impl WorkerSender {
    /// Non-blocking try_send - returns the data back if the queue is full.
    /// Used from async context to avoid blocking tokio worker threads.
    #[allow(clippy::result_large_err)]
    pub(crate) fn try_send(
        &self,
        data: RequestData,
    ) -> Result<(), channel::TrySendError<RequestData>> {
        self.sender.try_send(data)
    }
}
