//! Dev-only capture of outbound mail — the store behind `/__soli/inbox`.
//!
//! Local development rarely has an SMTP server, so mail either fails to send or
//! vanishes into a log line. Every `deliver_now` / `deliver_later` under `--dev`
//! records the fully-rendered message here (headers, both bodies, attachment
//! metadata, raw MIME) and the dev inbox renders it — the Soli equivalent of
//! MailCatcher / letter_opener, with no extra process to run.
//!
//! Process-wide rather than thread-local: workers are threads in one process
//! and any of them may deliver, but the inbox must show all of it. Everything
//! stored is plain owned data (`String`/`Vec`), so it crosses threads freely —
//! unlike `Value`, which is `!Send`.
//!
//! Nothing writes here unless the server runs with `--dev` (the capture call
//! sites check `template::is_dev_mode()` first), so production and `soli test`
//! pay nothing.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Max messages retained; the oldest is evicted past this. A dev session sends
/// far fewer than this before a restart clears the store anyway.
const CAP: usize = 100;

/// Don't retain a raw MIME blob past this size — a mail with big attachments
/// would otherwise pin megabytes per message. The message itself is still
/// captured; only its "Raw" tab goes missing.
const MIME_MAX_BYTES: usize = 1024 * 1024;

/// What happened to a captured message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Handed to an SMTP server, which accepted it.
    Sent,
    /// Never left the process: no SMTP host configured, or a `test` / `logger`
    /// delivery method. The inbox is the only place this message exists.
    Captured,
    /// Delivery was attempted and failed; carries the error.
    Failed(String),
}

impl Status {
    /// Short lowercase label for the inbox listing.
    pub fn label(&self) -> &'static str {
        match self {
            Status::Sent => "sent",
            Status::Captured => "captured",
            Status::Failed(_) => "failed",
        }
    }
}

/// One attachment's metadata. The bytes themselves aren't retained — the inbox
/// lists attachments, it doesn't serve them.
#[derive(Clone, Debug)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub size: usize,
}

/// A delivered (or attempted) message, as the inbox displays it.
#[derive(Clone, Debug)]
pub struct CapturedMail {
    /// Monotonic decimal id, unique for the life of the process.
    pub id: String,
    /// Local wall-clock capture time, `YYYY-MM-DD HH:MM:SS`.
    pub at: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub reply_to: Option<String>,
    pub subject: String,
    pub html: Option<String>,
    pub text: Option<String>,
    pub attachments: Vec<Attachment>,
    pub status: Status,
    /// The RFC 5322 bytes SMTP would carry. `None` when the message couldn't be
    /// built (a validation failure is itself worth seeing in the inbox) or when
    /// it exceeded [`MIME_MAX_BYTES`].
    pub mime: Option<String>,
}

impl CapturedMail {
    /// Every recipient, in header order — what the listing shows as "to".
    pub fn recipients(&self) -> Vec<&str> {
        self.to
            .iter()
            .chain(self.cc.iter())
            .chain(self.bcc.iter())
            .map(String::as_str)
            .collect()
    }

    /// Case-insensitive match of `needle` against the addresses, subject, and
    /// both bodies. An empty needle matches everything.
    pub fn matches(&self, needle: &str) -> bool {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        let hit = |s: &str| s.to_lowercase().contains(&needle);
        hit(&self.subject)
            || hit(&self.from)
            || self.recipients().iter().any(|r| hit(r))
            || self.reply_to.as_deref().is_some_and(hit)
            || self.text.as_deref().is_some_and(hit)
            || self.html.as_deref().is_some_and(hit)
            || self.attachments.iter().any(|a| hit(&a.filename))
    }
}

fn store() -> &'static Mutex<VecDeque<CapturedMail>> {
    static STORE: OnceLock<Mutex<VecDeque<CapturedMail>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// The next message id. Never reused, so a link to a cleared message 404s
/// rather than silently resolving to a different mail.
pub fn next_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed).to_string()
}

/// Drop a raw MIME blob that's too big to retain (see [`MIME_MAX_BYTES`]).
pub fn retainable_mime(mime: Option<String>) -> Option<String> {
    mime.filter(|m| m.len() <= MIME_MAX_BYTES)
}

/// Record a message, evicting the oldest once at capacity.
pub fn record(mail: CapturedMail) {
    let Ok(mut queue) = store().lock() else {
        return;
    };
    if queue.len() >= CAP {
        queue.pop_front();
    }
    queue.push_back(mail);
}

/// Every captured message, newest first.
pub fn all() -> Vec<CapturedMail> {
    let Ok(queue) = store().lock() else {
        return Vec::new();
    };
    queue.iter().rev().cloned().collect()
}

/// Captured messages matching `needle`, newest first.
pub fn search(needle: &str) -> Vec<CapturedMail> {
    let Ok(queue) = store().lock() else {
        return Vec::new();
    };
    queue
        .iter()
        .rev()
        .filter(|m| m.matches(needle))
        .cloned()
        .collect()
}

/// One message by id, or `None` if unknown / evicted.
pub fn get(id: &str) -> Option<CapturedMail> {
    let queue = store().lock().ok()?;
    queue.iter().rev().find(|m| m.id == id).cloned()
}

/// How many messages are held.
pub fn count() -> usize {
    store().lock().map(|q| q.len()).unwrap_or(0)
}

/// The newest message's id, or 0 when empty. The baseline for the inbox's
/// "new mail" badge — global, so a filtered or paged view doesn't mistake mail
/// it isn't showing for mail that just arrived. Messages are appended in id
/// order, so the back of the queue is the newest.
pub fn latest_id() -> u64 {
    store()
        .lock()
        .ok()
        .and_then(|q| q.back().and_then(|m| m.id.parse().ok()))
        .unwrap_or(0)
}

/// Empty the inbox.
pub fn clear() {
    if let Ok(mut queue) = store().lock() {
        queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, subject: &str, to: &str) -> CapturedMail {
        CapturedMail {
            id: id.to_string(),
            at: "2026-07-29 10:00:00".to_string(),
            from: "app@example.com".to_string(),
            to: vec![to.to_string()],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: None,
            subject: subject.to_string(),
            html: Some("<b>Bonjour</b>".to_string()),
            text: Some("Bonjour".to_string()),
            attachments: Vec::new(),
            status: Status::Captured,
            mime: None,
        }
    }

    #[test]
    fn matches_subject_address_and_body_case_insensitively() {
        let mail = sample("1", "Welcome aboard", "alice@example.com");
        assert!(mail.matches("welcome"));
        assert!(mail.matches("ALICE@"));
        assert!(mail.matches("bonjour"));
        assert!(!mail.matches("nope"));
    }

    #[test]
    fn empty_needle_matches_everything() {
        assert!(sample("1", "x", "a@b.io").matches("   "));
    }

    #[test]
    fn recipients_span_to_cc_and_bcc() {
        let mut mail = sample("1", "x", "to@x.io");
        mail.cc = vec!["cc@x.io".to_string()];
        mail.bcc = vec!["bcc@x.io".to_string()];
        assert_eq!(mail.recipients(), vec!["to@x.io", "cc@x.io", "bcc@x.io"]);
        assert!(mail.matches("bcc@"));
    }

    #[test]
    fn ids_are_unique_and_increasing() {
        let first: u64 = next_id().parse().unwrap();
        let second: u64 = next_id().parse().unwrap();
        assert!(second > first);
    }

    #[test]
    fn oversized_mime_is_not_retained() {
        assert!(retainable_mime(Some("small".to_string())).is_some());
        assert!(retainable_mime(Some("x".repeat(MIME_MAX_BYTES + 1))).is_none());
    }
}
