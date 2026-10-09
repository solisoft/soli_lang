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
//!
//! **Kept in the app's database too**, in the framework collection
//! `_soli_mail_inbox` (like `_soli_errors`): a restart of `soli serve --dev`
//! emptied the inbox, and with it the confirmation link you were about to
//! click. Memory stays the working copy — every page reads it, so the inbox
//! adds no query to a request's dev bar — and the database is its durable
//! copy: each capture is written to both, the first access in a process loads
//! what the database holds, and **Clear inbox** empties both. Without a
//! reachable database the inbox works as before, in memory only.

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
    STORE.get_or_init(|| Mutex::new(persisted::load()))
}

/// The next message id. Never reused — not even across restarts, now that the
/// inbox outlives the process: ids start from the clock (milliseconds) and
/// only move forward, so a link to a cleared message 404s rather than
/// resolving to a different mail, and a mail captured after a restart sorts
/// after the ones loaded from the database.
pub fn next_id() -> String {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut previous = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(previous + 1);
        match LAST.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next.to_string(),
            Err(actual) => previous = actual,
        }
    }
}

/// Drop a raw MIME blob that's too big to retain (see [`MIME_MAX_BYTES`]).
pub fn retainable_mime(mime: Option<String>) -> Option<String> {
    mime.filter(|m| m.len() <= MIME_MAX_BYTES)
}

/// Record a message, evicting the oldest once at capacity — in memory and in
/// the database.
pub fn record(mail: CapturedMail) {
    let Ok(mut queue) = store().lock() else {
        return;
    };
    persisted::insert(&mail);
    if queue.len() >= CAP {
        if let Some(evicted) = queue.pop_front() {
            persisted::delete(&evicted.id);
        }
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

/// Empty the inbox, in memory and in the database.
pub fn clear() {
    if let Ok(mut queue) = store().lock() {
        for mail in queue.iter() {
            persisted::delete(&mail.id);
        }
        queue.clear();
    }
}

/// The durable copy, in the app's database. Every call is best-effort: a
/// database that is down, absent or refuses a write leaves the in-memory inbox
/// working, and says so once on stderr.
mod persisted {
    use super::{Attachment, CapturedMail, Status, CAP};
    use crate::serve::internal_store;
    use serde_json::{json, Value};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub(super) const COLLECTION: &str = "_soli_mail_inbox";

    /// Report the first failure only: a dev server without a database would
    /// otherwise print one line per captured mail.
    fn warn_once(what: &str, error: &str) {
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            eprintln!("[inbox] {what} {COLLECTION}: {error} — the inbox stays in memory only");
        }
    }

    fn fenced<R>(write: impl FnOnce() -> Result<R, String>) -> Result<R, String> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(write)) {
            Ok(result) => result,
            Err(_) => Err("the database call panicked".to_string()),
        }
    }

    fn ensured() -> bool {
        static READY: AtomicBool = AtomicBool::new(false);
        if READY.load(Ordering::Relaxed) {
            return true;
        }
        // A restart finds the collection and its index already there, and
        // SoliDB answers the index creation with "already exists": ready.
        let prepared = fenced(|| internal_store::ensure(COLLECTION, "seq")).or_else(|e| {
            if e.contains("already exists") {
                Ok(())
            } else {
                Err(e)
            }
        });
        match prepared {
            Ok(()) => {
                READY.store(true, Ordering::Relaxed);
                true
            }
            Err(e) => {
                warn_once("could not prepare", &e);
                false
            }
        }
    }

    /// Every stored message, oldest first, at most [`CAP`].
    pub(super) fn load() -> VecDeque<CapturedMail> {
        if !ensured() {
            return VecDeque::new();
        }
        let rows = fenced(|| internal_store::list(COLLECTION, None, "seq", true, CAP, &["mail"]));
        match rows {
            Ok(rows) => {
                let mut mails: Vec<CapturedMail> = rows
                    .iter()
                    .filter_map(|row| row.get("mail").and_then(from_json))
                    .collect();
                mails.reverse();
                mails.into_iter().collect()
            }
            Err(e) => {
                warn_once("could not read", &e);
                VecDeque::new()
            }
        }
    }

    pub(super) fn insert(mail: &CapturedMail) {
        if !ensured() {
            return;
        }
        let seq: u64 = mail.id.parse().unwrap_or(0);
        let doc = json!({ "seq": seq, "mail": to_json(mail) });
        if let Err(e) = fenced(|| internal_store::insert(COLLECTION, &mail.id, doc)) {
            warn_once("could not write to", &e);
        }
    }

    pub(super) fn delete(id: &str) {
        if !ensured() {
            return;
        }
        if let Err(e) = fenced(|| internal_store::delete(COLLECTION, id)) {
            warn_once("could not delete from", &e);
        }
    }

    pub(super) fn to_json(mail: &CapturedMail) -> Value {
        let (status, error) = match &mail.status {
            Status::Failed(e) => ("failed", Some(e.clone())),
            other => (other.label(), None),
        };
        json!({
            "id": mail.id,
            "at": mail.at,
            "from": mail.from,
            "to": mail.to,
            "cc": mail.cc,
            "bcc": mail.bcc,
            "reply_to": mail.reply_to,
            "subject": mail.subject,
            "html": mail.html,
            "text": mail.text,
            "attachments": mail.attachments.iter().map(|a| json!({
                "filename": a.filename,
                "content_type": a.content_type,
                "size": a.size,
            })).collect::<Vec<_>>(),
            "status": status,
            "error": error,
            "mime": mail.mime,
        })
    }

    pub(super) fn from_json(value: &Value) -> Option<CapturedMail> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        let list = |key: &str| -> Vec<String> {
            value
                .get(key)
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let status = match text("status").as_deref() {
            Some("sent") => Status::Sent,
            Some("failed") => Status::Failed(text("error").unwrap_or_default()),
            _ => Status::Captured,
        };
        let attachments = value
            .get("attachments")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|a| Attachment {
                        filename: a
                            .get("filename")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        content_type: a
                            .get("content_type")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        size: a.get("size").and_then(Value::as_u64).unwrap_or(0) as usize,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(CapturedMail {
            id: text("id")?,
            at: text("at").unwrap_or_default(),
            from: text("from").unwrap_or_default(),
            to: list("to"),
            cc: list("cc"),
            bcc: list("bcc"),
            reply_to: text("reply_to"),
            subject: text("subject").unwrap_or_default(),
            html: text("html"),
            text: text("text"),
            attachments,
            status,
            mime: text("mime"),
        })
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
    fn a_stored_message_reads_back_unchanged() {
        let mut mail = sample("1759000000000", "Confirmez", "claire@exemple.fr");
        mail.cc = vec!["cc@x.io".to_string()];
        mail.reply_to = Some("support@x.io".to_string());
        mail.attachments = vec![Attachment {
            filename: "bon.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            size: 1234,
        }];
        mail.status = Status::Failed("550 no such user".to_string());
        mail.mime = Some("MIME-Version: 1.0".to_string());
        let back = persisted::from_json(&persisted::to_json(&mail)).expect("reads back");
        assert_eq!(back.id, mail.id);
        assert_eq!(back.cc, mail.cc);
        assert_eq!(back.reply_to, mail.reply_to);
        assert_eq!(back.html, mail.html);
        assert_eq!(back.status, mail.status);
        assert_eq!(back.mime, mail.mime);
        assert_eq!(back.attachments[0].filename, "bon.pdf");
        assert_eq!(back.attachments[0].size, 1234);
    }

    #[test]
    fn ids_start_from_the_clock_so_a_restart_sorts_after_stored_mail() {
        let id: u64 = next_id().parse().unwrap();
        assert!(
            id > 1_700_000_000_000,
            "id {id} does not come from the clock"
        );
    }

    #[test]
    fn oversized_mime_is_not_retained() {
        assert!(retainable_mime(Some("small".to_string())).is_some());
        assert!(retainable_mime(Some("x".repeat(MIME_MAX_BYTES + 1))).is_none());
    }
}
