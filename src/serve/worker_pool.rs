//! Worker Pool Management
//!
//! This module provides worker pool management structures including:
//! - Hot reload version counters (shared between file watcher and workers)
//! - Per-worker request queues for lock-free request distribution
//!

use crossbeam::channel;
use std::sync::atomic::AtomicU64;

use crate::serve::RequestData;

/// Hot reload version counters - shared between file watcher and workers.
/// Workers periodically check if versions changed and reload accordingly.
pub(crate) struct HotReloadVersions {
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
}

impl HotReloadVersions {
    pub(crate) fn new() -> Self {
        Self {
            controllers: AtomicU64::new(0),
            middleware: AtomicU64::new(0),
            views: AtomicU64::new(0),
            static_files: AtomicU64::new(0),
            routes: AtomicU64::new(0),
            helpers: AtomicU64::new(0),
        }
    }
}

/// Per-worker queue for distributing requests without contention.
/// Each worker has its own dedicated channel, eliminating receiver contention.
pub(crate) struct WorkerQueues {
    senders: Vec<channel::Sender<RequestData>>,
    receivers: Vec<channel::Receiver<RequestData>>,
    /// Shared round-robin counter across all senders for even distribution.
    next_worker: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl WorkerQueues {
    pub(crate) fn new(num_workers: usize, capacity_per_worker: usize) -> Self {
        let mut senders = Vec::with_capacity(num_workers);
        let mut receivers = Vec::with_capacity(num_workers);

        for _ in 0..num_workers {
            let (tx, rx) = channel::bounded(capacity_per_worker);
            senders.push(tx);
            receivers.push(rx);
        }

        Self {
            senders,
            receivers,
            next_worker: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Get a sender that round-robins across workers (lock-free).
    /// All senders share the same counter for even distribution across connections.
    pub(crate) fn get_sender(&self) -> WorkerSender {
        WorkerSender {
            senders: self.senders.clone(),
            next_worker: self.next_worker.clone(),
        }
    }

    /// Get the receiver for a specific worker
    pub(crate) fn get_receiver(&self, worker_id: usize) -> channel::Receiver<RequestData> {
        self.receivers[worker_id].clone()
    }
}

/// A sender that distributes requests across workers using round-robin
#[derive(Clone)]
pub(crate) struct WorkerSender {
    senders: Vec<channel::Sender<RequestData>>,
    next_worker: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl WorkerSender {
    /// Non-blocking try_send - returns the data back if the target queue is full.
    /// Used from async context to avoid blocking tokio worker threads.
    #[allow(clippy::result_large_err)]
    pub(crate) fn try_send(
        &self,
        data: RequestData,
    ) -> Result<(), channel::TrySendError<RequestData>> {
        let worker = self
            .next_worker
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.senders.len();
        self.senders[worker].try_send(data)
    }
}
