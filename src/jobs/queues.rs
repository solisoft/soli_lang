//! Background jobs on the edge build: Cloudflare Queues.
//!
//! A Worker has no thread to poll a `_jobs` table with, but Cloudflare runs a
//! Worker's `queue()` handler for each batch of messages a
//! [Queue](https://developers.cloudflare.com/queues/) receives. So on the edge
//! an enqueue (`perform_later`, `Job.enqueue*`, `Webhook.enqueue*`) sends the
//! job's row — the same [`JobDoc`] the native engine would insert — as a
//! message, through the host's `soli_queue_send` import, and the Worker's
//! `queue()` handler runs each message's job with `serve::edge::run_job`.
//! Cloudflare keeps the messages, delivers them at least once and redelivers
//! a retried one; [`outcome`] decides between done, retry and dead with the
//! native engine's rule and backoff.
//!
//! What the edge cannot do with them: a delay past Queues' 12 hours, and the
//! row operations (`Job.list`, `cancel`, `retry`, `queues`) — Cloudflare does
//! not hand messages back.

use super::JobDoc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

/// The longest delay Cloudflare Queues accepts on a message.
pub const MAX_DELAY_SECS: i64 = 43_200;

/// How a message reaches the queue.
pub trait QueueSender: Send + Sync {
    fn send(
        &self,
        binding: &str,
        body: &serde_json::Value,
        delay_seconds: u32,
    ) -> Result<(), String>;
}

static SENDER: RwLock<Option<Arc<dyn QueueSender>>> = RwLock::new(None);
/// Set with the sender, so a native enqueue pays an atomic load to learn that
/// it writes a row as usual.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Send enqueues through `sender` instead of the job table (tests).
pub fn set_sender(sender: Arc<dyn QueueSender>) {
    *SENDER.write().unwrap_or_else(|e| e.into_inner()) = Some(sender);
    INSTALLED.store(true, Ordering::Release);
}

/// Whether enqueues go to a Cloudflare Queue: always on the edge build;
/// natively only once a test has installed a sender.
pub fn routes() -> bool {
    cfg!(target_arch = "wasm32") || INSTALLED.load(Ordering::Acquire)
}

fn sender() -> Result<Arc<dyn QueueSender>, String> {
    if let Some(sender) = SENDER.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return Ok(sender);
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(Arc::new(host::Binding))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err("no Cloudflare Queue to send to outside a Worker".to_string())
    }
}

/// The producer binding in `wrangler.toml`: every Soli queue name travels in
/// the message, through one Cloudflare Queue.
fn binding() -> String {
    crate::platform::env::var("SOLI_QUEUE_BINDING")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "JOBS".to_string())
}

/// Send `doc` to the queue, delayed until its `run_at`. Returns the job id.
pub fn enqueue(doc: &JobDoc) -> Result<String, String> {
    let delay = super::parse_iso_secs(&doc.run_at)
        .map(|at| at - super::unix_now())
        .unwrap_or(0)
        .max(0);
    if delay > MAX_DELAY_SECS {
        return Err(format!(
            "{} is due in {delay}s, but Cloudflare Queues delay a message by at most \
             12 hours ({MAX_DELAY_SECS}s)",
            doc.handler
        ));
    }
    sender()?.send(&binding(), &doc.to_json()?, delay as u32)?;
    Ok(doc.key.clone())
}

/// What the Worker does with a message once its job has run.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Acknowledge it.
    Done,
    /// Redeliver it after this many seconds.
    Retry(u32),
    /// Acknowledge it: the retry budget is spent.
    Dead(String),
}

/// The native engine's rule: a failure is retried while the attempts so far
/// (Cloudflare's `message.attempts`, 1 on the first delivery) stay within the
/// job's `max_retries`, after the same backoff.
pub fn outcome(doc: &JobDoc, attempts: u32, error: Option<String>) -> Outcome {
    match error {
        None => Outcome::Done,
        Some(_) if i64::from(attempts) <= doc.max_retries => {
            let delay = super::backoff_secs(i64::from(attempts), &doc.key);
            Outcome::Retry(delay.clamp(0, MAX_DELAY_SECS) as u32)
        }
        Some(error) => Outcome::Dead(error),
    }
}

/// The producer on the edge build: the host's `soli_queue_send` import
/// (`edge/js/jspi.js`), which the stack suspends on until Cloudflare has the
/// message.
#[cfg(target_arch = "wasm32")]
mod host {
    use super::QueueSender;

    #[link(wasm_import_module = "./jspi.js")]
    extern "C" {
        /// Takes the JSON request at `ptr`/`len` (`{binding, body,
        /// delay_seconds}`); returns a buffer the host allocated with
        /// `soli_alloc`: a little-endian `u32` length, then that many bytes of
        /// JSON (`{ok}` or `{ok: false, error}`). Ownership passes back to Rust.
        fn soli_queue_send(ptr: *const u8, len: usize) -> *mut u8;
    }

    pub(super) struct Binding;

    impl QueueSender for Binding {
        fn send(
            &self,
            binding: &str,
            body: &serde_json::Value,
            delay_seconds: u32,
        ) -> Result<(), String> {
            let request = serde_json::json!({
                "binding": binding,
                "body": body,
                "delay_seconds": delay_seconds,
            })
            .to_string();
            // SAFETY: the host reads `len` bytes at `ptr` and returns a buffer
            // from `soli_alloc(4 + n)`, whose first 4 bytes are `n`.
            let response = unsafe {
                let out = soli_queue_send(request.as_ptr(), request.len());
                if out.is_null() {
                    return Err("the host returned no response".to_string());
                }
                let mut len = [0u8; 4];
                std::ptr::copy_nonoverlapping(out, len.as_mut_ptr(), 4);
                let total = 4 + u32::from_le_bytes(len) as usize;
                let buffer = Vec::from_raw_parts(out, total, total);
                buffer[4..].to_vec()
            };
            let json: serde_json::Value = serde_json::from_slice(&response)
                .map_err(|e| format!("malformed response from the host: {e}"))?;
            if json["ok"].as_bool() == Some(true) {
                Ok(())
            } else {
                Err(json["error"]
                    .as_str()
                    .unwrap_or("the queue refused the message")
                    .to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(max_retries: i64) -> JobDoc {
        let mut doc = JobDoc::new(
            "MailJob",
            serde_json::json!({}),
            "default",
            super::super::now_iso(),
        );
        doc.max_retries = max_retries;
        doc
    }

    #[test]
    fn a_failure_is_retried_with_backoff_until_the_budget_is_spent() {
        let doc = job(2);
        assert_eq!(outcome(&doc, 1, None), Outcome::Done);
        assert_eq!(
            outcome(&doc, 1, Some("boom".into())),
            Outcome::Retry(super::super::backoff_secs(1, &doc.key) as u32)
        );
        assert!(matches!(
            outcome(&doc, 2, Some("boom".into())),
            Outcome::Retry(_)
        ));
        assert_eq!(
            outcome(&doc, 3, Some("boom".into())),
            Outcome::Dead("boom".into())
        );
    }
}
