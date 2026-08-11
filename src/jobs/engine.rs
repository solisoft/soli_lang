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

/// Start the poller thread. Returns immediately; the thread runs until the
/// process shuts down. `runtime_handle` is the server's tokio handle — the
/// poller's DB and HTTP calls need it (the getter is thread-local, so the
/// handle has to be carried across the thread boundary).
pub fn start(pool_slots: usize, runtime_handle: tokio::runtime::Handle) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return; // already running
    }
    set_capacity(pool_slots);

    let cfg = super::config().clone();
    let builder = std::thread::Builder::new().name("jobs-poller".to_string());
    let spawned = builder.spawn(move || {
        crate::serve::set_tokio_handle(runtime_handle);
        run_poller(cfg)
    });
    match spawned {
        Ok(_) => println!(
            "Job engine: polling every {}ms, {} worker slot(s), {}s lease",
            super::config().poll_ms,
            pool_slots,
            super::config().lease_secs
        ),
        Err(e) => eprintln!("Failed to spawn job poller: {e}"),
    }
}

fn run_poller(cfg: super::EngineConfig) {
    let mut ticks: u64 = 0;
    loop {
        std::thread::sleep(Duration::from_millis(cfg.poll_ms));
        ticks = ticks.wrapping_add(1);

        // Renew leases first: in-flight work must not be reclaimed just because
        // this tick is slow or the queue is busy.
        renew_leases(cfg.lease_secs);

        if crate::serve::shutdown::is_draining() {
            // Stop taking new work, but keep the loop alive so in-flight leases
            // stay renewed until the drain completes.
            continue;
        }

        if let Err(e) = tick_cron() {
            eprintln!("[cron] tick failed: {e}");
        }

        if let Err(e) = dispatch_due() {
            eprintln!("[jobs] poll failed: {e}");
        }

        // Prune completed rows about once every 10 minutes of ticks.
        let prune_every = (600_000 / cfg.poll_ms).max(1);
        if cfg.retention_secs > 0 && ticks.is_multiple_of(prune_every) {
            let cutoff = super::iso_from_unix(super::unix_now() - cfg.retention_secs);
            if let Err(e) = store::prune_done(&cutoff) {
                eprintln!("[jobs] prune failed: {e}");
            }
        }
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
            // Pure HTTP: no interpreter needed, so run it here.
            let error = deliver_webhook(&job).err();
            report_outcome(&job, error.as_deref());
            continue;
        }
        dispatch_to_pool(job);
    }
    Ok(())
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

fn renew_leases(lease_secs: i64) {
    let ids: Vec<String> = match in_flight().lock() {
        Ok(list) => list.clone(),
        Err(_) => return,
    };
    for id in ids {
        if let Err(e) = store::renew_lease(&id, lease_secs) {
            eprintln!("[jobs] lease renewal failed for {id}: {e}");
        }
    }
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
fn deliver_webhook(job: &JobDoc) -> Result<(), String> {
    let Some(spec) = job.webhook.as_ref() else {
        return Err("webhook job has no webhook spec".to_string());
    };
    let body = job.args.to_string();
    let secret = spec
        .secret
        .clone()
        .or_else(|| std::env::var("SOLI_WEBHOOK_SECRET").ok())
        .filter(|s| !s.is_empty());

    let client = crate::interpreter::builtins::http_class::db_http_client();
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

/// Run a future on the shared DB/HTTP runtime for this thread. `block_on_db`
/// reuses the server handle when present and otherwise the per-thread runtime,
/// so the poller never spins up a reactor of its own.
fn run_blocking<F>(future: F) -> F::Output
where
    F: std::future::Future + 'static,
{
    crate::interpreter::builtins::http_class::block_on_db(future)
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
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
