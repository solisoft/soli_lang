//! The job poller: claims due work, hands it to the worker pool, keeps leases
//! fresh, and ticks the cron scheduler.
//!
//! One poller thread runs per `soli serve` process. It never executes job code
//! itself — Soli code runs on the background pool's interpreters
//! (`serve::background_jobs`), which report each outcome back through
//! [`store::complete`] / [`store::fail`]. The one exception is the built-in
//! `__WebhookDelivery` job, which is pure HTTP and runs inline on the poller.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::{store, JobDoc, WebhookSpec, WEBHOOK_HANDLER};
use crate::interpreter::builtins::hex::encode as hex_encode;

/// Jobs currently executing on this process, by id. The poller renews their
/// leases each tick so a long job is not reclaimed mid-flight.
static IN_FLIGHT: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
/// Worker slots not currently occupied. The poller never claims more than this,
/// so a claimed job always starts immediately and burns no lease time queued.
static IDLE_SLOTS: OnceLock<Arc<AtomicUsize>> = OnceLock::new();

fn in_flight() -> &'static Arc<Mutex<Vec<String>>> {
    IN_FLIGHT.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

fn idle_slots() -> &'static Arc<AtomicUsize> {
    IDLE_SLOTS.get_or_init(|| Arc::new(AtomicUsize::new(0)))
}

/// Publish the pool size so the poller knows how many jobs it may claim.
pub fn set_capacity(slots: usize) {
    idle_slots().store(slots, Ordering::SeqCst);
}

/// Mark a job as started on this process (pool worker calls this).
pub fn mark_started(job_id: &str) {
    idle_slots().fetch_sub(1, Ordering::SeqCst);
    if let Ok(mut list) = in_flight().lock() {
        list.push(job_id.to_string());
    }
}

/// Mark a job as finished on this process (pool worker calls this).
pub fn mark_finished(job_id: &str) {
    idle_slots().fetch_add(1, Ordering::SeqCst);
    if let Ok(mut list) = in_flight().lock() {
        list.retain(|id| id != job_id);
    }
}

/// Report the outcome of a job that ran on the pool. `error` is `None` on
/// success. Keeps all state transitions in one place.
pub fn report_outcome(job: &JobDoc, error: Option<&str>) {
    let result = match error {
        None => store::complete(&job.key).map(|_| ()),
        Some(err) => store::fail(job, err).map(|retrying| {
            if retrying {
                println!(
                    "[jobs] {} failed (attempt {}/{}), retrying: {}",
                    job.handler, job.attempts, job.max_retries, err
                );
            } else {
                eprintln!(
                    "[jobs] {} died after {} attempt(s): {}",
                    job.handler, job.attempts, err
                );
            }
        }),
    };
    if let Err(e) = result {
        // The job ran; only bookkeeping failed. Say so loudly — the lease will
        // expire and another worker will retry it.
        eprintln!(
            "[jobs] could not record outcome for {} ({}): {e}",
            job.handler, job.key
        );
    }
}

/// Whether `soli serve` should start the engine at boot: the app has at least
/// one job class to run (`app/jobs/**/*_job.sl`, the files the loader reads —
/// which is also where `static cron` is declared), or a mailer is configured
/// (`deliver_later` enqueues the built-in delivery job).
///
/// An empty `app/jobs/` — what `soli new` scaffolds — is not a reason: such an
/// app has nothing to run, and on an app with no database the poller only
/// produced a failed claim per tick, forever. An app that enqueues anyway
/// (`Webhook.enqueue`, `Cron.schedule`) starts the engine on that first
/// enqueue, through [`start_deferred`]. A directory that cannot be read keeps
/// the old behaviour and starts it.
pub fn wanted_at_boot(jobs_dir: &std::path::Path, mailer_configured: bool) -> bool {
    mailer_configured
        || crate::serve::app_loader::scan_jobs(jobs_dir)
            .map(|files| !files.is_empty())
            .unwrap_or(true)
}

/// A start that `soli serve` set aside because the app had nothing to run.
type DeferredStart = Box<dyn FnOnce() + Send>;

static DEFERRED_START: Mutex<Option<DeferredStart>> = Mutex::new(None);

/// Keep `starter` for the first enqueue: an app with no job files can still
/// enqueue a webhook or declare a cron, and that work needs a poller.
pub fn defer_start(starter: DeferredStart) {
    if let Ok(mut slot) = DEFERRED_START.lock() {
        *slot = Some(starter);
    }
}

/// Run the start [`defer_start`] kept, once. A no-op when nothing was
/// deferred — the engine already runs, `SOLI_JOB_WORKERS=0` disabled it, or
/// this is not a `soli serve` process.
pub fn start_deferred() {
    let starter = DEFERRED_START.lock().ok().and_then(|mut slot| slot.take());
    if let Some(starter) = starter {
        starter();
    }
}

/// Start the poller thread. Returns immediately; the thread runs until the
/// process shuts down. `runtime_handle` is the server's tokio handle — the
/// poller's DB and HTTP calls need it (the getter is thread-local, so the
/// handle has to be carried across the thread boundary). `dev_mode` slows the
/// tick to [`super::DEV_POLL_MS`] so a laptop running several dev servers is
/// not paying a claim round-trip per app per second.
pub fn start(pool_slots: usize, runtime_handle: tokio::runtime::Handle, dev_mode: bool) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return; // already running
    }
    set_capacity(pool_slots);

    let mut cfg = super::config().clone();
    if dev_mode {
        cfg.poll_ms = super::dev_poll_ms();
    }
    let poll_ms = cfg.poll_ms;
    let lease_secs = cfg.lease_secs;
    let builder = std::thread::Builder::new().name("jobs-poller".to_string());
    // The poller reads and writes the queue of the application that started
    // it; bound for the life of the thread, since thread-locals do not cross
    // `spawn`.
    let tenant = crate::serve::tenant::current_id();
    // Subscribe before the poller starts, so its first sleep can already be the
    // long one. This runs on the server runtime, never on the poller thread.
    super::push::start(&runtime_handle);

    let idle_ms = super::idle_poll_ms();
    let spawned = builder.spawn(move || {
        crate::serve::tenant::bind_current(tenant);
        crate::serve::set_tokio_handle(runtime_handle);
        // Claims, leases and cron ticks: the queue's own traffic, which no app
        // can act on. The jobs themselves run elsewhere and are counted.
        crate::serve::query_stats::ignore_this_thread();
        run_poller(cfg)
    });
    match spawned {
        Ok(_) => println!(
            "Job engine: {} worker slot(s), {}s lease, polling every {}ms \
             (backstop {}ms when the changefeed is connected)",
            pool_slots, lease_secs, poll_ms, idle_ms
        ),
        Err(e) => eprintln!("Failed to spawn job poller: {e}"),
    }
}

/// Create `_jobs` and `_cron_jobs`, and index them, on the SoliDB backend.
///
/// The collections would otherwise appear on the first enqueue, which for most
/// apps is never — and the changefeed refuses a subscription to a collection
/// that does not exist, so the app that needs the cheap idle path the most was
/// the one that could not get it.
///
/// The indexes matter just as much. `store::ensure_sql_indexes` only ever ran
/// in the SQL branch of `enqueue`, so a SoliDB app indexed nothing at all and
/// the claim query — which filters on `state` and orders by `priority`, on
/// every poll — scanned the entire collection. Measured against a real queue
/// that had grown to 128k rows: 738ms per claim, versus 0.08ms for the
/// server's own dispatch query, which does have its index.
///
/// Runs on the poller thread, where blocking database calls belong, and is
/// best-effort: a failure costs performance, not correctness. The one failure
/// it returns is an unreachable store, so the poller can report it once and try
/// again when the store answers, rather than print a line per collection.
fn ensure_queue_collections() -> Result<(), String> {
    if crate::db::is_sql() {
        // The SQL backends create and index their tables in `store::enqueue`,
        // and have no changefeed to satisfy.
        return Ok(());
    }
    use crate::interpreter::builtins::model::crud;

    for (collection, fields) in [
        (super::JOBS_COLLECTION, &["state", "run_at", "priority"][..]),
        (super::CRON_COLLECTION, &["next_run_at", "enabled"][..]),
    ] {
        if let Err(e) = crud::ensure_collection(collection) {
            if is_connection_error(&e) {
                return Err(e);
            }
            eprintln!("[jobs] could not ensure {collection}: {e}");
            continue;
        }
        for field in fields {
            if let Err(e) = crud::ensure_index(collection, field) {
                eprintln!("[jobs] could not index {collection}.{field}: {e}");
            }
        }
    }
    // The claim filters a range on run_at and sorts by priority then
    // run_at: the hash indexes above only serve the state equality, so
    // every poll read every pending, scheduled and failed row. From SoliDB
    // 2.6.3 this index serves the FILTER as one bounded range per state
    // (older servers ignore it); the SORT then runs over the due rows only.
    // The SQL backends build theirs in `store::enqueue`.
    if let Err(e) = crud::ensure_sorted_index(
        super::JOBS_COLLECTION,
        "idx_jobs_claim",
        &["state", "run_at", "priority"],
    ) {
        eprintln!(
            "[jobs] could not index the claim on {}: {e}",
            super::JOBS_COLLECTION
        );
    }
    Ok(())
}

/// How long to keep pruning apart, regardless of how often the loop wakes.
const PRUNE_EVERY: Duration = Duration::from_secs(600);

fn run_poller(cfg: super::EngineConfig) {
    let mut last_prune = web_time::Instant::now();
    let mut health = StoreHealth::default();
    let mut cron_errors = ErrorThrottle::default();
    let mut poll_errors = ErrorThrottle::default();
    let base = Duration::from_millis(cfg.poll_ms);

    // Before the first sleep: the changefeed refuses a subscription to a
    // collection that does not exist yet.
    let first = ensure_queue_collections();
    let mut queue_ready = first.is_ok();
    report_health(&mut health, first);

    loop {
        // While the store is unreachable the poller backs off on its own
        // schedule; the queue's deadlines mean nothing until it answers.
        let sleep = health.backoff(base).unwrap_or_else(|| next_sleep(&cfg));
        super::wake::wait(sleep);

        // Renew leases first: in-flight work must not be reclaimed just because
        // this tick is slow or the queue is busy.
        let renewed = renew_leases(cfg.lease_secs);

        if crate::serve::shutdown::is_draining() {
            // Stop taking new work, but keep the loop alive so in-flight leases
            // stay renewed until the drain completes.
            report_health(&mut health, renewed);
            continue;
        }

        let tick = renewed.and_then(|()| {
            if !queue_ready {
                ensure_queue_collections()?;
                queue_ready = true;
            }
            poll_once(&mut cron_errors, &mut poll_errors)
        });
        report_health(&mut health, tick);
        if health.is_down() {
            continue;
        }

        // Prune completed rows about every ten minutes. Timed rather than
        // counted in ticks: the loop no longer wakes on a fixed interval, so a
        // tick count says nothing about elapsed time.
        if cfg.retention_secs > 0 && last_prune.elapsed() >= PRUNE_EVERY {
            last_prune = web_time::Instant::now();
            let cutoff = super::iso_from_unix(super::unix_now() - cfg.retention_secs);
            if let Err(e) = store::prune_done(&cutoff) {
                eprintln!("[jobs] prune failed: {e}");
            }
        }
    }
}

/// One cron tick and one claim. Returns `Err` only when the store could not be
/// reached; any other failure is logged here, at most once a minute per kind,
/// and the tick carries on.
fn poll_once(
    cron_errors: &mut ErrorThrottle,
    poll_errors: &mut ErrorThrottle,
) -> Result<(), String> {
    if let Err(e) = tick_cron() {
        if is_connection_error(&e) {
            return Err(e);
        }
        if let Some(line) = cron_errors.report(web_time::Instant::now(), "[cron] tick failed", &e) {
            eprintln!("{line}");
        }
    }
    if let Err(e) = dispatch_due() {
        if is_connection_error(&e) {
            return Err(e);
        }
        if let Some(line) = poll_errors.report(web_time::Instant::now(), "[jobs] poll failed", &e) {
            eprintln!("{line}");
        }
    }
    Ok(())
}

/// Feed a tick's outcome to `health` and print what it says is worth saying.
fn report_health(health: &mut StoreHealth, outcome: Result<(), String>) {
    match outcome {
        Ok(()) => {
            if let Some(line) = health.reachable(&store_label()) {
                println!("{line}");
            }
        }
        Err(e) => {
            if let Some(line) = health.unreachable(&store_label(), &e) {
                eprintln!("{line}");
            }
        }
    }
}

/// Where the queue lives, for a log line: the SoliDB host, or the SQL
/// connection's name and adapter (never its URL, which carries credentials).
fn store_label() -> String {
    if crate::db::is_sql() {
        crate::db::adapter_label()
    } else {
        crate::interpreter::builtins::model::db_config::db_host()
    }
}

/// Longest the poller waits between two attempts while the store is down.
const MAX_OUTAGE_BACKOFF: Duration = Duration::from_secs(300);

/// Shortest gap between two log lines for the same kind of query error.
const QUERY_ERROR_LOG_EVERY: Duration = Duration::from_secs(60);

/// Whether `message` says the store could not be reached at all — refused,
/// reset, timed out, no such host — as opposed to a query the store answered
/// with an error. Matched on text, since the SoliDB and SQL drivers each word
/// it their own way and only the message reaches the poller.
pub(crate) fn is_connection_error(message: &str) -> bool {
    const MARKERS: [&str; 16] = [
        "http error",
        "error sending request",
        "connection refused",
        "connection reset",
        "connection closed",
        "connection failed",
        "connect error",
        "tcp connect",
        "timed out",
        "dns error",
        "failed to lookup",
        "name or service not known",
        "nodename nor servname",
        "network is unreachable",
        "no route to host",
        "broken pipe",
    ];
    let lower = message.to_ascii_lowercase();
    MARKERS.iter().any(|marker| lower.contains(marker))
}

/// The poller's view of whether the store answers, and what to say about it.
///
/// Kept free of I/O and clocks so the log and backoff rules are testable: an
/// outage is reported once when it starts and once when it ends, and the
/// poller waits longer between attempts the longer it lasts.
#[derive(Debug, Default)]
struct StoreHealth {
    /// Consecutive ticks that could not reach the store.
    failed_ticks: u32,
}

impl StoreHealth {
    fn is_down(&self) -> bool {
        self.failed_ticks > 0
    }

    /// A tick could not reach the store. Returns the line to log, which only
    /// the first failure of an outage has.
    fn unreachable(&mut self, host: &str, error: &str) -> Option<String> {
        self.failed_ticks = self.failed_ticks.saturating_add(1);
        (self.failed_ticks == 1).then(|| {
            format!(
                "[jobs] database unreachable at {host}: {error}; retrying with backoff \
                 (up to {}s between attempts)",
                MAX_OUTAGE_BACKOFF.as_secs()
            )
        })
    }

    /// A tick reached the store. Returns the line to log when that ends an
    /// outage.
    fn reachable(&mut self, host: &str) -> Option<String> {
        let failed = std::mem::take(&mut self.failed_ticks);
        (failed > 0).then(|| {
            format!("[jobs] database reachable again at {host} after {failed} failed attempt(s)")
        })
    }

    /// How long to wait before the next attempt, while down: the poll interval
    /// doubled per failed attempt, capped at [`MAX_OUTAGE_BACKOFF`].
    fn backoff(&self, base: Duration) -> Option<Duration> {
        if self.failed_ticks == 0 {
            return None;
        }
        let factor = 1u32 << self.failed_ticks.min(16);
        Some(base.saturating_mul(factor).min(MAX_OUTAGE_BACKOFF))
    }
}

/// Rate-limits a recurring error to one line per [`QUERY_ERROR_LOG_EVERY`],
/// counting what it swallowed in between.
#[derive(Debug, Default)]
struct ErrorThrottle {
    last_logged: Option<web_time::Instant>,
    suppressed: u32,
}

impl ErrorThrottle {
    fn report(&mut self, now: web_time::Instant, what: &str, error: &str) -> Option<String> {
        let due = self
            .last_logged
            .is_none_or(|at| now.saturating_duration_since(at) >= QUERY_ERROR_LOG_EVERY);
        if !due {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.last_logged = Some(now);
        let suppressed = std::mem::take(&mut self.suppressed);
        Some(if suppressed > 0 {
            format!("{what}: {error} ({suppressed} similar error(s) not shown)")
        } else {
            format!("{what}: {error}")
        })
    }
}

#[cfg(test)]
mod health_tests {
    use super::*;

    #[test]
    fn an_outage_is_logged_once_and_its_end_once() {
        let mut health = StoreHealth::default();
        let first = health.unreachable("localhost:6745", "HTTP error: connection refused");
        let line = first.expect("the first failure is reported");
        assert!(
            line.contains("database unreachable at localhost:6745"),
            "{line}"
        );
        assert!(line.contains("retrying with backoff"), "{line}");
        for _ in 0..50 {
            assert_eq!(
                health.unreachable("localhost:6745", "HTTP error: refused"),
                None
            );
        }
        let back = health
            .reachable("localhost:6745")
            .expect("recovery is reported");
        assert!(back.contains("reachable again"), "{back}");
        assert!(back.contains("51 failed attempt(s)"), "{back}");
        assert_eq!(
            health.reachable("localhost:6745"),
            None,
            "a healthy tick says nothing"
        );
        assert!(
            health.unreachable("h", "e").is_some(),
            "a new outage is reported again"
        );
    }

    #[test]
    fn backoff_doubles_and_caps_at_five_minutes() {
        let base = Duration::from_millis(1000);
        let mut health = StoreHealth::default();
        assert_eq!(
            health.backoff(base),
            None,
            "a healthy store keeps the normal schedule"
        );
        let mut seen = Vec::new();
        for _ in 0..12 {
            health.unreachable("h", "e");
            seen.push(health.backoff(base).expect("down").as_secs());
        }
        assert_eq!(&seen[..5], &[2, 4, 8, 16, 32]);
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(*seen.last().expect("some"), 300);
        for _ in 0..100 {
            health.unreachable("h", "e");
        }
        assert_eq!(health.backoff(base), Some(MAX_OUTAGE_BACKOFF));
        health.reachable("h");
        assert_eq!(health.backoff(base), None);
    }

    #[test]
    fn connection_failures_are_told_from_query_errors() {
        for message in [
            "HTTP error: error sending request for url (http://localhost:6745/_api/database/x/cursor)",
            "HTTP error : tcp connect error: Connection refused (os error 61)",
            "error communicating with database: Connection refused (os error 111)",
            "pool timed out while waiting for an open connection",
            "dns error: failed to lookup address information",
        ] {
            assert!(is_connection_error(message), "{message}");
        }
        for message in [
            "Query failed (400): collection not found",
            "syntax error at or near \"FOR\"",
            "invalid cron expression",
        ] {
            assert!(!is_connection_error(message), "{message}");
        }
    }

    #[test]
    fn a_recurring_query_error_is_logged_once_a_minute() {
        let start = web_time::Instant::now();
        let mut throttle = ErrorThrottle::default();
        assert_eq!(
            throttle
                .report(start, "[jobs] poll failed", "boom")
                .as_deref(),
            Some("[jobs] poll failed: boom")
        );
        for second in 1..60 {
            let at = start + Duration::from_secs(second);
            assert_eq!(throttle.report(at, "[jobs] poll failed", "boom"), None);
        }
        let line = throttle
            .report(
                start + Duration::from_secs(60),
                "[jobs] poll failed",
                "boom",
            )
            .expect("a minute later it logs again");
        assert!(line.contains("59 similar error(s) not shown"), "{line}");
    }
}

/// How long the poller may sleep before it must look at the queue again.
///
/// A live changefeed subscription is what earns the long backstop: without one
/// nothing would tell us about a job another process enqueued, so the interval
/// stays what it always was.
fn next_sleep(cfg: &super::EngineConfig) -> Duration {
    let backstop_ms = if super::push::is_connected() {
        super::idle_poll_ms().max(cfg.poll_ms)
    } else {
        cfg.poll_ms
    };
    let holding_work = in_flight().lock().map(|l| !l.is_empty()).unwrap_or(false);
    let now = super::unix_now();
    let deadlines = [
        super::scheduler::next_due_unix(),
        super::next_future_run_at(now),
    ];

    soonest(backstop_ms, cfg.lease_secs, holding_work, &deadlines, now)
}

/// The backstop, pulled in by whatever falls due sooner.
///
/// Split out from [`next_sleep`] so the arithmetic can be tested without the
/// process-wide state the real deadlines come from.
fn soonest(
    backstop_ms: u64,
    lease_secs: i64,
    holding_work: bool,
    deadlines: &[Option<i64>],
    now: i64,
) -> Duration {
    let mut sleep = Duration::from_millis(backstop_ms);

    // Work already running must have its lease renewed well inside the lease
    // window, whatever else is or is not due.
    if holding_work {
        let renew_every = Duration::from_secs((lease_secs.max(2) / 2) as u64);
        sleep = sleep.min(renew_every);
    }

    for deadline in deadlines.iter().flatten() {
        let until = Duration::from_secs(deadline.saturating_sub(now).max(0) as u64);
        sleep = sleep.min(until);
    }

    // Never spin: a deadline that has already passed still yields one pass, and
    // the pass itself is what clears it.
    sleep.max(Duration::from_millis(50))
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn with_nothing_due_the_poller_sleeps_the_whole_backstop() {
        assert_eq!(
            soonest(30_000, 60, false, &[None, None], NOW),
            Duration::from_millis(30_000)
        );
    }

    #[test]
    fn a_nearer_deadline_pulls_the_sleep_in() {
        // A job due in 5s must not wait out a 30s backstop.
        let sleep = soonest(30_000, 60, false, &[None, Some(NOW + 5)], NOW);
        assert_eq!(sleep, Duration::from_secs(5));
    }

    #[test]
    fn the_soonest_of_several_deadlines_wins() {
        let sleep = soonest(30_000, 60, false, &[Some(NOW + 20), Some(NOW + 3)], NOW);
        assert_eq!(sleep, Duration::from_secs(3));
    }

    #[test]
    fn a_deadline_beyond_the_backstop_does_not_extend_it() {
        let sleep = soonest(30_000, 60, false, &[Some(NOW + 9_000)], NOW);
        assert_eq!(sleep, Duration::from_millis(30_000));
    }

    #[test]
    fn in_flight_work_forces_a_wake_inside_the_lease_window() {
        // Sleeping the full backstop would let a 20s lease expire under a
        // running job and another process would reclaim it mid-flight.
        let sleep = soonest(30_000, 20, true, &[None], NOW);
        assert_eq!(sleep, Duration::from_secs(10));
        assert!(sleep < Duration::from_secs(20), "must renew before expiry");
    }

    #[test]
    fn an_overdue_deadline_yields_one_pass_not_a_spin() {
        let sleep = soonest(30_000, 60, false, &[Some(NOW - 500)], NOW);
        assert_eq!(
            sleep,
            Duration::from_millis(50),
            "a passed deadline must still sleep a little, or the loop spins"
        );
    }

    #[test]
    fn a_tiny_lease_cannot_produce_a_zero_sleep() {
        let sleep = soonest(30_000, 1, true, &[None], NOW);
        assert!(sleep >= Duration::from_millis(50));
    }
}

/// Claim as many due jobs as there are free worker slots and dispatch them.
fn dispatch_due() -> Result<(), String> {
    let slots = idle_slots().load(Ordering::SeqCst);
    if slots == 0 {
        return Ok(());
    }
    let jobs = super::claim::claim(slots)?;
    for job in jobs {
        if job.handler == WEBHOOK_HANDLER {
            // Pure HTTP, so no interpreter is needed — but it must not run *on
            // the poller*. A black-holed host made one tick block for the whole
            // connect timeout per due webhook, which outlives the lease and also
            // stalls `renew_leases` and the cron tick; a second poller then
            // re-claimed the in-flight job through the expired-lease branch and
            // ran `perform` a second time, concurrently.
            //
            // Its own thread, bracketed by `mark_started`/`mark_finished`, keeps
            // the slot accounting honest (so the next tick does not over-claim)
            // and puts the job on the in-flight list that `renew_leases` walks.
            spawn_webhook_delivery(job);
            continue;
        }
        dispatch_to_pool(job);
    }
    Ok(())
}

/// Deliver one webhook off the poller thread. Holds a worker slot for its
/// duration and stays on the in-flight list so its lease keeps being renewed.
fn spawn_webhook_delivery(job: JobDoc) {
    mark_started(&job.key);
    // `job` moves into the closure, and a failed `spawn` drops it with the
    // closure — so keep a copy for the failure path below.
    let unspawned = job.clone();
    let tenant = crate::serve::tenant::current_id();
    let spawned = std::thread::Builder::new()
        .name("soli-webhook".to_string())
        .spawn(move || {
            crate::serve::tenant::bind_current(tenant);
            let error = deliver_webhook(&job).err();
            report_outcome(&job, error.as_deref());
            mark_finished(&job.key);
        });
    if let Err(e) = spawned {
        // Could not spawn: undo the claim so the job is retried rather than
        // stranded in `running`. `mark_finished` is what does the undoing —
        // returning the slot alone would leave the id on the in-flight list,
        // where `renew_leases` extends its lease forever and nothing ever
        // reclaims it.
        eprintln!("[jobs] could not spawn webhook thread: {e}");
        mark_finished(&unspawned.key);
        if let Err(e) = release(&unspawned) {
            eprintln!("[jobs] failed to release {}: {e}", unspawned.key);
        }
    }
}

/// Hand a job to the background pool. If the pool is not running (or its
/// channel is closed) the claim is released so the job is retried rather than
/// silently stranded in `running` until its lease expires.
fn dispatch_to_pool(job: JobDoc) {
    let args_json = job.args.to_string();
    let accepted =
        crate::serve::background_jobs::enqueue_with_id(job.handler.clone(), args_json, job.clone());
    if !accepted {
        eprintln!(
            "[jobs] no worker pool available for {}; releasing claim",
            job.handler
        );
        if let Err(e) = release(&job) {
            eprintln!("[jobs] failed to release {}: {e}", job.key);
        }
    }
}

/// Put a claimed job back on the queue immediately (undo the claim).
fn release(job: &JobDoc) -> Result<(), String> {
    store::renew_lease(&job.key, -1)?; // expire the lease at once
    Ok(())
}

/// Extend the lease of every job running here. Stops at, and returns, the
/// first failure to reach the store: the poller reports that once, rather than
/// a line per in-flight job per tick.
fn renew_leases(lease_secs: i64) -> Result<(), String> {
    let ids: Vec<String> = match in_flight().lock() {
        Ok(list) => list.clone(),
        Err(_) => return Ok(()),
    };
    for id in ids {
        if let Err(e) = store::renew_lease(&id, lease_secs) {
            if is_connection_error(&e) {
                return Err(e);
            }
            eprintln!("[jobs] lease renewal failed for {id}: {e}");
        }
    }
    Ok(())
}

fn tick_cron() -> Result<(), String> {
    super::scheduler::tick().map(|fired| {
        if fired > 0 {
            println!("[cron] enqueued {fired} job(s)");
        }
    })
}

// ---------- built-in outbound webhook job ----------

/// POST a webhook job's payload to its URL, signing the body with HMAC-SHA256
/// when a secret is configured. Mirrors the headers SolidB used to send, so
/// existing receivers keep verifying successfully.
pub(crate) fn deliver_webhook(job: &JobDoc) -> Result<(), String> {
    let Some(spec) = job.webhook.as_ref() else {
        return Err("webhook job has no webhook spec".to_string());
    };
    let body = job.args.to_string();
    let secret = spec
        .secret
        .clone()
        .or_else(|| crate::platform::env::var("SOLI_WEBHOOK_SECRET").ok())
        .filter(|s| !s.is_empty());

    // Webhook URLs are the archetypal user-supplied destination ("paste your
    // endpoint"), and delivery used to run on `db_http_client()` — the client
    // deliberately exempted from the SSRF blocklist because it normally only
    // talks to SOLIDB_HOST. That combination pointed an authenticated,
    // retrying, signature-adding POST at anything reachable from the server,
    // with the status code echoed back through `last_error`. Validate here and
    // send on the user-facing client, which filters DNS at connect time and
    // re-checks every redirect hop.
    validate_webhook_url(&spec.url)?;

    let client = crate::interpreter::builtins::http_class::get_user_http_client().clone();
    let mut request = client
        .post(&spec.url)
        .header("Content-Type", "application/json")
        .header("X-Webhook-Event", "job")
        .header("X-Webhook-Delivery", job.key.clone());

    if let Some(key) = &secret {
        let mac = crate::interpreter::builtins::crypto::hmac_sha256_bytes(
            body.as_bytes(),
            key.as_bytes(),
        );
        request = request.header("X-Webhook-Signature", hex_encode(&mac));
    }
    if let Some(headers) = spec.headers.as_ref().and_then(|h| h.as_object()) {
        for (name, value) in headers {
            if let Some(v) = value.as_str() {
                // Reserved names are refused rather than merged: `Host` retargets
                // virtual-host routing, `Authorization`/`Cookie` would forward the
                // app's own credentials to the caller's destination, and the
                // framing headers desync the connection.
                if is_reserved_webhook_header(name) {
                    return Err(format!(
                        "webhook header {name:?} is reserved and cannot be set"
                    ));
                }
                request = request.header(name.as_str(), v);
            }
        }
    }

    let response = run_blocking(async move { request.body(body).send().await })
        .map_err(|e| format!("webhook POST {}: {e}", spec.url))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    Err(format!(
        "webhook POST {} returned HTTP {}",
        spec.url,
        status.as_u16()
    ))
}

/// Header names a webhook spec may not set. `Content-Length` and
/// `Transfer-Encoding` are derived from the body (a mismatch desyncs the
/// upstream connection); the rest would either retarget the request or leak the
/// app's own credentials to a destination the caller chose.
const RESERVED_WEBHOOK_HEADERS: [&str; 7] = [
    "host",
    "authorization",
    "cookie",
    "content-length",
    "transfer-encoding",
    "proxy-authorization",
    "proxy-authenticate",
];

pub(crate) fn is_reserved_webhook_header(name: &str) -> bool {
    RESERVED_WEBHOOK_HEADERS
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

/// Gate a webhook destination through the SSRF blocklist. Called at enqueue
/// (so a bad URL fails loudly where the developer can see it) and again at
/// delivery (so a row written directly to `_jobs`, or a DNS record that moved
/// in between, cannot slip past).
pub(crate) fn validate_webhook_url(url: &str) -> Result<(), String> {
    crate::interpreter::builtins::http_class::validate_url_for_ssrf(url)
        .map_err(|e| format!("webhook URL {url:?}: {e}"))
}

/// Run a future on the shared DB/HTTP runtime for this thread. `block_on_db`
/// reuses the server handle when present and otherwise the per-thread runtime,
/// so the poller never spins up a reactor of its own.
fn run_blocking<F>(future: F) -> F::Output
where
    F: std::future::Future + 'static,
{
    crate::interpreter::builtins::http_class::block_on_db(future)
}

/// Build the `__WebhookDelivery` job for `Webhook.enqueue*`.
pub fn webhook_job(
    url: &str,
    payload: serde_json::Value,
    queue: &str,
    run_at: String,
    spec_extras: (Option<String>, Option<serde_json::Value>),
) -> JobDoc {
    let (secret, headers) = spec_extras;
    let mut job = JobDoc::new(WEBHOOK_HANDLER, payload, queue, run_at);
    job.webhook = Some(WebhookSpec {
        url: url.to_string(),
        secret,
        headers,
    });
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_jobs_directory_does_not_start_the_engine() {
        let dir = tempfile::tempdir().expect("tempdir");
        let jobs = dir.path().join("jobs");
        std::fs::create_dir_all(&jobs).expect("jobs dir");
        assert!(!wanted_at_boot(&jobs, false), "what `soli new` scaffolds");
        // A file the loader does not read as a job is not one either.
        std::fs::write(jobs.join("README.md"), "notes").expect("readme");
        std::fs::write(jobs.join("helpers.sl"), "def x\n  1\nend\n").expect("helper");
        assert!(!wanted_at_boot(&jobs, false));
        assert!(!wanted_at_boot(&dir.path().join("missing"), false));
    }

    #[test]
    fn a_job_file_or_a_mailer_starts_the_engine() {
        let dir = tempfile::tempdir().expect("tempdir");
        let jobs = dir.path().join("jobs");
        std::fs::create_dir_all(jobs.join("billing")).expect("nested dir");
        assert!(wanted_at_boot(&jobs, true), "deliver_later needs a poller");
        assert!(wanted_at_boot(&dir.path().join("missing"), true));
        std::fs::write(
            jobs.join("billing/invoice_job.sl"),
            "class InvoiceJob\nend\n",
        )
        .expect("job file");
        assert!(wanted_at_boot(&jobs, false), "found in a subdirectory");
    }

    #[test]
    fn a_deferred_start_runs_once() {
        use std::sync::atomic::AtomicUsize;
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        start_deferred(); // nothing deferred: a no-op
        defer_start(Box::new(|| {
            RUNS.fetch_add(1, Ordering::SeqCst);
        }));
        start_deferred();
        start_deferred();
        assert_eq!(RUNS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn capacity_tracks_started_and_finished_jobs() {
        set_capacity(2);
        assert_eq!(idle_slots().load(Ordering::SeqCst), 2);
        mark_started("a");
        assert_eq!(idle_slots().load(Ordering::SeqCst), 1);
        mark_started("b");
        assert_eq!(idle_slots().load(Ordering::SeqCst), 0);
        // A full pool claims nothing — that's what keeps leases off queued work.
        assert!(dispatch_due().is_ok());
        mark_finished("a");
        assert_eq!(idle_slots().load(Ordering::SeqCst), 1);
        mark_finished("b");
        assert_eq!(idle_slots().load(Ordering::SeqCst), 2);
        assert!(in_flight().lock().unwrap().is_empty());
    }

    #[test]
    fn hex_encoding_is_lowercase_and_zero_padded() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn webhook_job_carries_url_secret_and_headers() {
        let job = webhook_job(
            "https://example.test/hook",
            serde_json::json!({"id": 1}),
            "default",
            super::super::now_iso(),
            (
                Some("s3cret".to_string()),
                Some(serde_json::json!({"X-Custom": "1"})),
            ),
        );
        assert_eq!(job.handler, WEBHOOK_HANDLER);
        let spec = job.webhook.expect("spec");
        assert_eq!(spec.url, "https://example.test/hook");
        assert_eq!(spec.secret.as_deref(), Some("s3cret"));
        assert_eq!(spec.headers.unwrap()["X-Custom"], "1");
    }

    #[test]
    fn webhook_delivery_without_a_spec_is_an_error_not_a_panic() {
        let job = JobDoc::new(
            WEBHOOK_HANDLER,
            serde_json::json!({}),
            "default",
            super::super::now_iso(),
        );
        let err = deliver_webhook(&job).expect_err("missing spec must error");
        assert!(err.contains("no webhook spec"), "{err}");
    }
}
