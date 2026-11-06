//! Built-in spam defences for public forms: what `spam_check(req)` decides
//! with, and the quarantine the `/__soli/spam` page reads.
//!
//! Four layers, cheapest first:
//!
//! * a **honeypot** field and a **signed render timestamp**, which
//!   `csrf_field()` puts in every POST form. A bot fills the field, or posts
//!   without ever loading the page (no stamp), or answers within seconds;
//! * an optional **proof of work** (`pow_field()`, solved in the browser by
//!   `/__soli/pow.js`): a fraction of a second for one visitor, real CPU for a
//!   thousand posts;
//! * a **naive Bayes score** learnt from this application's own traffic;
//! * a **quarantine** of what was refused, and of what was accepted, so a
//!   mistake is seen and corrected on `/__soli/spam` — which is also what
//!   trains the score.
//!
//! Signing keys are derived (HKDF) from the session secret, one per purpose.
//! Without a secret — a bare `soli serve --dev` — a key is drawn once per
//! process: forms rendered before a restart then come back as "token", which
//! in development is a page reload.
//!
//! The quarantine and the model live in the app's own database
//! (`_soli_spam`, `_soli_spam_model`), written by a background writer so a
//! check never waits on the database. On by default; `SOLI_SPAM=off` turns the
//! recording off, and it stays off under `APP_ENV=test` unless `SOLI_SPAM=on`.
//! The checks themselves and the in-memory model work either way.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use web_time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

use crate::interpreter::builtins::crypto::{do_secure_compare, hmac_sha256_bytes};

use super::internal_store;
use super::tenant::{TenantId, TenantValue};
pub(crate) use super::tenant_writer::StatsSnapshot;
use super::tenant_writer::{self, Stats, Writers};

/// The honeypot: hidden from people, filled by bots that fill every field.
pub(crate) const HONEYPOT_FIELD: &str = "_soli_url";
/// The signed render timestamp.
pub(crate) const STAMP_FIELD: &str = "_soli_ts";
/// The proof-of-work solution, `<challenge>:<nonce>`.
pub(crate) const POW_FIELD: &str = "_soli_pow";

/// Default seconds a person needs at least between the page and the post.
pub(crate) const DEFAULT_MIN_SECONDS: u64 = 3;
/// Default age after which a form is too old to post (two days).
pub(crate) const DEFAULT_MAX_AGE: u64 = 2 * 24 * 3600;
/// Default number of links a message may hold; one more is spam.
pub(crate) const DEFAULT_MAX_LINKS: usize = 2;
/// Default score from which the model calls a message spam.
pub(crate) const DEFAULT_MAX_SCORE: f64 = 0.9;
/// Default proof-of-work difficulty: about 130 000 hashes, a fraction of a
/// second in a browser.
pub(crate) const DEFAULT_POW_BITS: u32 = 17;
/// Highest difficulty accepted: past this a phone would take minutes.
pub(crate) const MAX_POW_BITS: u32 = 24;

/// Refusal reasons a person can act on (reload, wait, enable JavaScript), as
/// opposed to the ones only a bot triggers, which get a fake success.
pub(crate) const SHOWN_REASONS: [&str; 4] = ["token", "expired", "pow", "rate"];

const FORM_INFO: &[u8] = b"soli.spam.form.v1";
const POW_INFO: &[u8] = b"soli.spam.pow.v1";
const MIN_SECRET_LEN: usize = 32;
const MAX_TOKEN_LEN: usize = 256;

/// Whether a refusal for `reason` should be shown to the visitor.
pub(crate) fn shown(reason: &str) -> bool {
    SHOWN_REASONS.contains(&reason)
}

// --- keys --------------------------------------------------------------------

fn process_secret() -> &'static [u8; 32] {
    static SECRET: OnceLock<[u8; 32]> = OnceLock::new();
    SECRET.get_or_init(|| {
        let mut out = [0u8; 32];
        out[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        out[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        out
    })
}

fn derive_key(secret: &[u8], info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, secret);
    let mut key = [0u8; 32];
    // 32 bytes is far below HKDF-SHA256's 8160-byte limit: this cannot fail.
    let _ = hk.expand(info, &mut key);
    key
}

fn key_for(info: &[u8]) -> [u8; 32] {
    match crate::interpreter::builtins::session::get_session_config().secret {
        Some(secret) if secret.len() >= MIN_SECRET_LEN => derive_key(secret.as_bytes(), info),
        _ => derive_key(process_secret(), info),
    }
}

fn mac(key: &[u8; 32], message: &str) -> String {
    URL_SAFE_NO_PAD.encode(hmac_sha256_bytes(message.as_bytes(), key))
}

// --- the render timestamp ----------------------------------------------------

/// `<unix seconds>.<mac>`: when the form was rendered, signed.
pub(crate) fn form_stamp(now: u64) -> String {
    stamp_with(&key_for(FORM_INFO), now)
}

fn stamp_with(key: &[u8; 32], now: u64) -> String {
    format!("{now}.{}", mac(key, &format!("form:{now}")))
}

/// `Ok` when `stamp` is ours and its age is within `[min_secs, max_secs]`;
/// otherwise the refusal reason: `token` (missing or forged), `fast`,
/// `expired`.
pub(crate) fn check_stamp(
    stamp: &str,
    now: u64,
    min_secs: u64,
    max_secs: u64,
) -> Result<(), &'static str> {
    check_stamp_with(&key_for(FORM_INFO), stamp, now, min_secs, max_secs)
}

fn check_stamp_with(
    key: &[u8; 32],
    stamp: &str,
    now: u64,
    min_secs: u64,
    max_secs: u64,
) -> Result<(), &'static str> {
    if stamp.is_empty() || stamp.len() > MAX_TOKEN_LEN {
        return Err("token");
    }
    let Some((at, signature)) = stamp.split_once('.') else {
        return Err("token");
    };
    let Ok(at) = at.parse::<u64>() else {
        return Err("token");
    };
    // Constant-time: the MAC must not leak one position at a time.
    if !do_secure_compare(&mac(key, &format!("form:{at}")), signature) {
        return Err("token");
    }
    // A stamp from the future is a clock skew between slots, not a bot.
    let age = now.saturating_sub(at);
    if age < min_secs {
        return Err("fast");
    }
    if age > max_secs {
        return Err("expired");
    }
    Ok(())
}

// --- proof of work -----------------------------------------------------------

/// Challenges already solved, until they expire: a solution is spent once.
static SPENT_CHALLENGES: TenantValue<HashMap<String, u64>> = TenantValue::new(HashMap::new);

/// More than this many spent challenges and the expired ones are swept.
const SPENT_SWEEP_AT: usize = 10_000;

/// `<salt>.<bits>.<expiry>.<mac>`: find a nonce such that
/// `SHA-256("<challenge>:<nonce>")` starts with `bits` zero bits.
pub(crate) fn pow_challenge(bits: u32, now: u64) -> String {
    pow_challenge_with(&key_for(POW_INFO), bits, now)
}

fn pow_challenge_with(key: &[u8; 32], bits: u32, now: u64) -> String {
    let bits = bits.clamp(1, MAX_POW_BITS);
    let salt = URL_SAFE_NO_PAD.encode(&uuid::Uuid::new_v4().as_bytes()[..12]);
    let payload = format!("{salt}.{bits}.{}", now.saturating_add(DEFAULT_MAX_AGE));
    format!("{payload}.{}", mac(key, &format!("pow:{payload}")))
}

/// `Ok` when `solution` solves one of our unexpired, unspent challenges.
/// Spends the challenge.
pub(crate) fn verify_pow(solution: &str, now: u64) -> Result<(), &'static str> {
    verify_pow_with(&key_for(POW_INFO), solution, now)?;
    let challenge = solution.split_once(':').map(|(c, _)| c).unwrap_or("");
    let expiry = challenge
        .split('.')
        .nth(2)
        .and_then(|e| e.parse::<u64>().ok())
        .unwrap_or(now);
    SPENT_CHALLENGES.write(|spent| {
        if spent.len() >= SPENT_SWEEP_AT {
            spent.retain(|_, until| *until >= now);
        }
        if spent.contains_key(challenge) {
            return Err("pow");
        }
        spent.insert(challenge.to_string(), expiry);
        Ok(())
    })
}

fn verify_pow_with(key: &[u8; 32], solution: &str, now: u64) -> Result<(), &'static str> {
    if solution.is_empty() || solution.len() > MAX_TOKEN_LEN {
        return Err("pow");
    }
    let Some((challenge, nonce)) = solution.split_once(':') else {
        return Err("pow");
    };
    if nonce.is_empty() || nonce.len() > 20 || !nonce.bytes().all(|b| b.is_ascii_digit()) {
        return Err("pow");
    }
    let mut parts = challenge.split('.');
    let (Some(salt), Some(bits), Some(expiry), Some(signature), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err("pow");
    };
    let payload = format!("{salt}.{bits}.{expiry}");
    if !do_secure_compare(&mac(key, &format!("pow:{payload}")), signature) {
        return Err("pow");
    }
    let (Ok(bits), Ok(expiry)) = (bits.parse::<u32>(), expiry.parse::<u64>()) else {
        return Err("pow");
    };
    if bits == 0 || bits > MAX_POW_BITS || now > expiry {
        return Err("pow");
    }
    let digest = Sha256::digest(solution.as_bytes());
    if leading_zero_bits(&digest) < bits {
        return Err("pow");
    }
    Ok(())
}

fn leading_zero_bits(bytes: &[u8]) -> u32 {
    let mut count = 0;
    for byte in bytes {
        if *byte == 0 {
            count += 8;
        } else {
            return count + byte.leading_zeros();
        }
    }
    count
}

// --- content -----------------------------------------------------------------

/// Links in a text: `http://`, `https://` and BBCode `[url`.
pub(crate) fn count_links(text: &str) -> usize {
    let lower = text.to_lowercase();
    lower.matches("http://").count()
        + lower.matches("https://").count()
        + lower.matches("[url").count()
}

/// Tokens per document, at most: a pasted novel is not more spam than a line.
const MAX_DOC_TOKENS: usize = 400;

/// What the model reads in a text: its words (lowercased, 2 to 24 letters or
/// digits; a run of digits counts as `#num`) and, per link, `url:<host>` and
/// `url:*`. A set: a word repeated ten times is one piece of evidence.
pub(crate) fn tokens(text: &str) -> HashSet<String> {
    let lower = text.to_lowercase();
    let mut out = HashSet::new();
    for scheme in ["http://", "https://"] {
        for (index, _) in lower.match_indices(scheme) {
            let rest = &lower[index + scheme.len()..];
            let host: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '-')
                .take(64)
                .collect();
            let host = host.trim_start_matches("www.").trim_end_matches('.');
            if !host.is_empty() {
                out.insert(format!("url:{host}"));
            }
            out.insert("url:*".to_string());
        }
    }
    for word in lower.split(|c: char| !c.is_alphanumeric()) {
        if out.len() >= MAX_DOC_TOKENS {
            break;
        }
        let length = word.chars().count();
        if !(2..=24).contains(&length) {
            continue;
        }
        if word.chars().all(|c| c.is_ascii_digit()) {
            out.insert("#num".to_string());
        } else {
            out.insert(word.to_string());
        }
    }
    out
}

// --- the model ---------------------------------------------------------------

/// Documents of each kind needed before the score is used to refuse.
pub(crate) const MIN_TRAINED_DOCS: u64 = 5;
/// Words kept. Past this, the rarest are forgotten first.
const MAX_MODEL_TOKENS: usize = 20_000;
/// The most telling words a score is computed from (Graham's fifteen).
const INTERESTING_TOKENS: usize = 15;
/// Robinson's strength of the 0.5 prior for a rarely seen word.
const PRIOR_STRENGTH: f64 = 1.0;

/// Per word, in how many spam and ham documents it appeared.
#[derive(Default, Clone, Debug, PartialEq)]
pub(crate) struct Model {
    pub(crate) spam_docs: u64,
    pub(crate) ham_docs: u64,
    pub(crate) words: HashMap<String, [u64; 2]>,
}

impl Model {
    /// Learn `text` as spam (`true`) or ham.
    pub(crate) fn train(&mut self, text: &str, spam: bool) {
        let side = usize::from(!spam);
        if spam {
            self.spam_docs += 1;
        } else {
            self.ham_docs += 1;
        }
        for token in tokens(text) {
            self.words.entry(token).or_insert([0, 0])[side] += 1;
        }
        self.prune();
    }

    /// Take back a `train(text, spam)`: how a correction moves a document
    /// from one side to the other.
    pub(crate) fn untrain(&mut self, text: &str, spam: bool) {
        let side = usize::from(!spam);
        if spam {
            self.spam_docs = self.spam_docs.saturating_sub(1);
        } else {
            self.ham_docs = self.ham_docs.saturating_sub(1);
        }
        for token in tokens(text) {
            if let Some(counts) = self.words.get_mut(&token) {
                counts[side] = counts[side].saturating_sub(1);
                if counts[0] == 0 && counts[1] == 0 {
                    self.words.remove(&token);
                }
            }
        }
    }

    /// Enough of both kinds for the score to mean something.
    pub(crate) fn trained(&self) -> bool {
        self.spam_docs >= MIN_TRAINED_DOCS && self.ham_docs >= MIN_TRAINED_DOCS
    }

    /// Probability that `text` is spam, from 0 to 1; 0.5 when nothing in it
    /// was seen before.
    pub(crate) fn score(&self, text: &str) -> f64 {
        let spam_docs = self.spam_docs.max(1) as f64;
        let ham_docs = self.ham_docs.max(1) as f64;
        let mut evidence: Vec<f64> = tokens(text)
            .iter()
            .filter_map(|token| self.words.get(token))
            .filter_map(|[spam, ham]| {
                let in_spam = *spam as f64 / spam_docs;
                let in_ham = *ham as f64 / ham_docs;
                if in_spam + in_ham == 0.0 {
                    return None;
                }
                let seen = (*spam + *ham) as f64;
                let p = in_spam / (in_spam + in_ham);
                let f = (PRIOR_STRENGTH * 0.5 + seen * p) / (PRIOR_STRENGTH + seen);
                Some(f.clamp(0.01, 0.99))
            })
            .filter(|f| (f - 0.5).abs() >= 0.1)
            .collect();
        if evidence.is_empty() {
            return 0.5;
        }
        evidence.sort_by(|a, b| {
            (b - 0.5)
                .abs()
                .partial_cmp(&(a - 0.5).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        evidence.truncate(INTERESTING_TOKENS);
        let log_spam: f64 = evidence.iter().map(|f| f.ln()).sum();
        let log_ham: f64 = evidence.iter().map(|f| (1.0 - f).ln()).sum();
        1.0 / (1.0 + (log_ham - log_spam).exp())
    }

    fn prune(&mut self) {
        if self.words.len() <= MAX_MODEL_TOKENS {
            return;
        }
        self.words.retain(|_, [spam, ham]| *spam + *ham > 1);
        if self.words.len() <= MAX_MODEL_TOKENS {
            return;
        }
        let mut by_count: Vec<(String, u64)> = self
            .words
            .iter()
            .map(|(word, [spam, ham])| (word.clone(), spam + ham))
            .collect();
        by_count.sort_by_key(|(_, count)| *count);
        let excess = self.words.len() - MAX_MODEL_TOKENS;
        for (word, _) in by_count.into_iter().take(excess) {
            self.words.remove(&word);
        }
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "spam_docs": self.spam_docs,
            "ham_docs": self.ham_docs,
            "words": self.words,
            "updated_at": crate::jobs::now_iso(),
        })
    }

    pub(crate) fn from_json(doc: &serde_json::Value) -> Model {
        let words = doc
            .get("words")
            .and_then(|w| w.as_object())
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(word, counts)| {
                        let counts = counts.as_array()?;
                        Some((
                            word.clone(),
                            [counts.first()?.as_u64()?, counts.get(1)?.as_u64()?],
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Model {
            spam_docs: doc.get("spam_docs").and_then(|v| v.as_u64()).unwrap_or(0),
            ham_docs: doc.get("ham_docs").and_then(|v| v.as_u64()).unwrap_or(0),
            words,
        }
    }
}

/// The application's model, and whether it was read from the database yet.
#[derive(Default)]
struct ModelSlot {
    model: Model,
    loaded: bool,
}

static MODEL: TenantValue<ModelSlot> = TenantValue::new(ModelSlot::default);

/// Read the stored model on first use. Without recording (tests, `SOLI_SPAM
/// =off`) the model starts empty and lives in memory.
fn ensure_model_loaded() {
    if MODEL.read(|slot| slot.loaded) {
        return;
    }
    let stored = if enabled() {
        internal_store::get(MODEL_COLLECTION, MODEL_KEY)
            .ok()
            .flatten()
            .map(|doc| Model::from_json(&doc))
    } else {
        None
    };
    MODEL.write(|slot| {
        if !slot.loaded {
            slot.loaded = true;
            if let Some(model) = stored {
                slot.model = model;
            }
        }
    });
}

/// Run `f` on the application's model.
pub(crate) fn with_model<R>(f: impl FnOnce(&Model) -> R) -> R {
    ensure_model_loaded();
    MODEL.read(|slot| f(&slot.model))
}

/// Change the model, then have the writer store it.
pub(crate) fn update_model<R>(f: impl FnOnce(&mut Model) -> R) -> R {
    ensure_model_loaded();
    let result = MODEL.write(|slot| f(&mut slot.model));
    if enabled() {
        WRITERS.deliver(super::tenant::current_id(), Item::SaveModel, &spawn_writer);
    }
    result
}

/// Forget everything learnt, here and in the database.
pub(crate) fn reset_model() {
    MODEL.write(|slot| {
        slot.model = Model::default();
        slot.loaded = true;
    });
    if enabled() {
        WRITERS.deliver(super::tenant::current_id(), Item::SaveModel, &spawn_writer);
    }
}

// --- the quarantine ----------------------------------------------------------

pub(crate) const QUARANTINE_COLLECTION: &str = "_soli_spam";
pub(crate) const MODEL_COLLECTION: &str = "_soli_spam_model";
const MODEL_KEY: &str = "model";

/// Submissions kept. Past this the oldest go first.
pub(crate) const MAX_ENTRIES: u64 = 1000;

/// `spam`: refused, or marked spam by hand. `ham`: accepted, or released.
pub(crate) const STATUSES: [&str; 2] = ["spam", "ham"];

const QUEUE_CAP: usize = 1024;
const FLUSH_EVERY: Duration = Duration::from_secs(1);
/// Inserts between two trims of the collection to [`MAX_ENTRIES`].
const TRIM_EVERY: u64 = 50;

enum Item {
    Entry(String, serde_json::Value),
    SaveModel,
}

static WRITERS: Writers<Item> = Writers::new("spam", QUEUE_CAP);

/// Whether submissions and the model are stored.
pub(crate) fn enabled() -> bool {
    super::error_tracker::env_switch("SOLI_SPAM")
}

/// This application's recording counters, since the process started.
pub(crate) fn stats() -> StatsSnapshot {
    WRITERS.stats(super::tenant::current_id())
}

/// Queue one submission for the quarantine. Never blocks; returns its key,
/// or `None` when recording is off.
pub(crate) fn record(doc: serde_json::Value) -> Option<String> {
    if !enabled() {
        return None;
    }
    let key = uuid::Uuid::new_v4().simple().to_string();
    WRITERS.deliver(
        super::tenant::current_id(),
        Item::Entry(key.clone(), doc),
        &spawn_writer,
    );
    Some(key)
}

fn spawn_writer(tenant: TenantId, receiver: Receiver<Item>, stats: Arc<Stats>) -> bool {
    tenant_writer::spawn_thread("spam", tenant, move || run_writer(receiver, &stats))
}

fn run_writer(receiver: Receiver<Item>, stats: &Stats) {
    let mut ensured = false;
    let mut since_trim = TRIM_EVERY;
    while let Ok(first) = receiver.recv() {
        let mut entries = Vec::new();
        let mut save_model = false;
        let mut take = |item: Item| match item {
            Item::Entry(key, doc) => entries.push((key, doc)),
            Item::SaveModel => save_model = true,
        };
        take(first);
        let deadline = Instant::now() + FLUSH_EVERY;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left) {
                Ok(item) => take(item),
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        if !ensured {
            match tenant_writer::fenced(|| {
                internal_store::ensure(QUARANTINE_COLLECTION, "status")?;
                internal_store::ensure(MODEL_COLLECTION, "updated_at")
            }) {
                Ok(()) => ensured = true,
                Err(e) => {
                    eprintln!("[spam] could not prepare {QUARANTINE_COLLECTION}: {e}");
                    stats.note_failed(entries.len() as u64);
                    continue;
                }
            }
        }

        for (key, doc) in entries {
            match tenant_writer::fenced(|| internal_store::insert(QUARANTINE_COLLECTION, &key, doc))
            {
                Ok(()) => since_trim += 1,
                Err(e) => {
                    eprintln!("[spam] could not store a submission: {e}");
                    stats.note_failed(1);
                }
            }
        }

        if save_model {
            let doc = MODEL.read(|slot| slot.model.to_json());
            if let Err(e) = tenant_writer::fenced(|| store_model(doc)) {
                eprintln!("[spam] could not store the model: {e}");
                stats.note_failed(1);
            }
        }

        if since_trim >= TRIM_EVERY {
            since_trim = 0;
            if let Err(e) = tenant_writer::fenced(trim) {
                eprintln!("[spam] could not trim {QUARANTINE_COLLECTION}: {e}");
            }
        }
    }
}

fn store_model(doc: serde_json::Value) -> Result<(), String> {
    if internal_store::get(MODEL_COLLECTION, MODEL_KEY)?.is_some() {
        internal_store::patch(MODEL_COLLECTION, MODEL_KEY, doc)
    } else {
        internal_store::insert(MODEL_COLLECTION, MODEL_KEY, doc)
    }
}

/// Keep the newest [`MAX_ENTRIES`].
fn trim() -> Result<(), String> {
    let count = internal_store::count(QUARANTINE_COLLECTION)?;
    if count <= MAX_ENTRIES {
        return Ok(());
    }
    let excess = (count - MAX_ENTRIES) as usize;
    let oldest = internal_store::list(
        QUARANTINE_COLLECTION,
        None,
        "created_at",
        false,
        excess,
        &["created_at"],
    )?;
    for row in oldest {
        if let Some(key) = row.get("_key").and_then(|k| k.as_str()) {
            internal_store::delete(QUARANTINE_COLLECTION, key)?;
        }
    }
    Ok(())
}

// --- what the dashboard reads -------------------------------------------------

/// Whether `value` has the shape of a quarantine key.
pub(crate) fn valid_key(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Submissions in `status`, newest first, without their fields.
pub(crate) fn list(status: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
    if !STATUSES.contains(&status) {
        return Err(format!("unknown status {status:?}"));
    }
    internal_store::list(
        QUARANTINE_COLLECTION,
        Some(("status", status)),
        "created_at",
        true,
        limit,
        &[
            "created_at",
            "path",
            "ip",
            "reason",
            "score",
            "status",
            "trained",
            "text",
        ],
    )
}

/// One submission, or `None`.
pub(crate) fn get(key: &str) -> Result<Option<serde_json::Value>, String> {
    internal_store::get(QUARANTINE_COLLECTION, key)
}

/// Forget a submission. `false` when it did not exist.
pub(crate) fn delete(key: &str) -> Result<bool, String> {
    internal_store::delete(QUARANTINE_COLLECTION, key)
}

/// File a submission under `status` and teach the model so: untrain what it
/// was learnt as, train it as what it is. `false` when it did not exist.
pub(crate) fn mark(key: &str, status: &str) -> Result<bool, String> {
    if !STATUSES.contains(&status) {
        return Err(format!("unknown status {status:?}"));
    }
    let Some(doc) = get(key)? else {
        return Ok(false);
    };
    let text = doc
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let was = doc
        .get("trained")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    let spam = status == "spam";
    if was.as_deref() != Some(status) {
        update_model(|model| {
            match was.as_deref() {
                Some("spam") => model.untrain(&text, true),
                Some("ham") => model.untrain(&text, false),
                _ => {}
            }
            model.train(&text, spam);
        });
    }
    internal_store::patch(
        QUARANTINE_COLLECTION,
        key,
        serde_json::json!({ "status": status, "trained": status }),
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    #[test]
    fn a_stamp_round_trips_and_ages() {
        let stamp = stamp_with(&KEY, 1_000);
        assert_eq!(check_stamp_with(&KEY, &stamp, 1_010, 3, 100), Ok(()));
        assert_eq!(check_stamp_with(&KEY, &stamp, 1_001, 3, 100), Err("fast"));
        assert_eq!(
            check_stamp_with(&KEY, &stamp, 1_200, 3, 100),
            Err("expired")
        );
    }

    #[test]
    fn a_missing_or_forged_stamp_is_a_token_refusal() {
        assert_eq!(check_stamp_with(&KEY, "", 10, 3, 100), Err("token"));
        assert_eq!(check_stamp_with(&KEY, "1000", 1_010, 3, 100), Err("token"));
        assert_eq!(
            check_stamp_with(&KEY, "1000.forged", 1_010, 3, 100),
            Err("token")
        );
        // Signed with another key: another app's form.
        let other = stamp_with(&[8u8; 32], 1_000);
        assert_eq!(check_stamp_with(&KEY, &other, 1_010, 3, 100), Err("token"));
        // The time is part of what is signed.
        let stamp = stamp_with(&KEY, 1_000);
        let moved = stamp.replacen("1000", "900", 1);
        assert_eq!(check_stamp_with(&KEY, &moved, 1_010, 3, 100), Err("token"));
    }

    fn solve(challenge: &str) -> String {
        (0u64..)
            .map(|nonce| format!("{challenge}:{nonce}"))
            .find(|candidate| {
                let bits: u32 = challenge.split('.').nth(1).unwrap().parse().unwrap();
                leading_zero_bits(&Sha256::digest(candidate.as_bytes())) >= bits
            })
            .unwrap()
    }

    #[test]
    fn a_solved_challenge_verifies() {
        let challenge = pow_challenge_with(&KEY, 8, 1_000);
        let solution = solve(&challenge);
        assert_eq!(verify_pow_with(&KEY, &solution, 1_010), Ok(()));
    }

    #[test]
    fn an_unsolved_forged_or_expired_challenge_does_not() {
        let challenge = pow_challenge_with(&KEY, 8, 1_000);
        let solution = solve(&challenge);
        // Expired.
        assert_eq!(
            verify_pow_with(&KEY, &solution, 1_000 + DEFAULT_MAX_AGE + 1),
            Err("pow")
        );
        // Another key.
        assert_eq!(verify_pow_with(&[8u8; 32], &solution, 1_010), Err("pow"));
        // A lower difficulty written in: the MAC covers it.
        let easier = solution.replacen(".8.", ".1.", 1);
        assert_eq!(verify_pow_with(&KEY, &easier, 1_010), Err("pow"));
        // Not solved: some nonce below the target fails most of the time, so
        // look for one that does.
        let unsolved = (0u64..)
            .map(|nonce| format!("{challenge}:{nonce}"))
            .find(|candidate| leading_zero_bits(&Sha256::digest(candidate.as_bytes())) < 8)
            .unwrap();
        assert_eq!(verify_pow_with(&KEY, &unsolved, 1_010), Err("pow"));
        assert_eq!(verify_pow_with(&KEY, "", 1_010), Err("pow"));
        assert_eq!(
            verify_pow_with(&KEY, &format!("{challenge}:"), 1_010),
            Err("pow")
        );
    }

    #[test]
    fn difficulty_is_clamped() {
        assert!(pow_challenge_with(&KEY, 99, 0).contains(&format!(".{MAX_POW_BITS}.")));
        assert!(pow_challenge_with(&KEY, 0, 0).contains(".1."));
    }

    #[test]
    fn leading_zero_bits_counts_across_bytes() {
        assert_eq!(leading_zero_bits(&[0x80]), 0);
        assert_eq!(leading_zero_bits(&[0x0f]), 4);
        assert_eq!(leading_zero_bits(&[0x00, 0x01]), 15);
        assert_eq!(leading_zero_bits(&[0x00, 0x00]), 16);
    }

    #[test]
    fn links_are_counted_whatever_the_case() {
        assert_eq!(count_links("voir HTTP://a et https://b, [URL=c]"), 3);
        assert_eq!(count_links("pas de lien"), 0);
    }

    #[test]
    fn tokens_are_words_hosts_and_numbers() {
        let found = tokens("Buy CHEAP pills at https://www.spam.example/x 12345 a");
        assert!(found.contains("buy"));
        assert!(found.contains("cheap"));
        assert!(found.contains("url:spam.example"));
        assert!(found.contains("url:*"));
        assert!(found.contains("#num"));
        assert!(!found.contains("a"), "one-letter words are noise");
    }

    #[test]
    fn the_model_learns_both_sides() {
        let mut model = Model::default();
        assert_eq!(model.score("anything"), 0.5);
        for _ in 0..MIN_TRAINED_DOCS {
            model.train("cheap pills casino https://spam.example", true);
            model.train("bonjour, une question sur le concert de samedi", false);
        }
        assert!(model.trained());
        assert!(model.score("cheap casino pills") > 0.9);
        assert!(model.score("question sur le concert") < 0.1);
        assert_eq!(model.score("zzz qqq"), 0.5, "unseen words say nothing");
    }

    #[test]
    fn untrain_takes_a_document_back() {
        let mut model = Model::default();
        model.train("cheap pills", true);
        model.untrain("cheap pills", true);
        assert_eq!(model, Model::default());
    }

    #[test]
    fn the_model_round_trips_through_json() {
        let mut model = Model::default();
        model.train("cheap pills", true);
        model.train("bonjour", false);
        assert_eq!(Model::from_json(&model.to_json()), model);
    }

    #[test]
    fn pruning_keeps_the_model_bounded() {
        let mut model = Model::default();
        for chunk in 0..(MAX_MODEL_TOKENS / 300 + 2) {
            let text: Vec<String> = (0..300).map(|i| format!("w{chunk}x{i}")).collect();
            model.train(&text.join(" "), chunk % 2 == 0);
        }
        assert!(model.words.len() <= MAX_MODEL_TOKENS);
    }

    #[test]
    fn shown_reasons_are_the_ones_a_person_can_fix() {
        for reason in ["token", "expired", "pow", "rate"] {
            assert!(shown(reason));
        }
        for reason in ["honeypot", "fast", "links", "score"] {
            assert!(!shown(reason));
        }
    }

    #[test]
    fn quarantine_keys_are_32_hex() {
        assert!(valid_key(&uuid::Uuid::new_v4().simple().to_string()));
        assert!(!valid_key("../etc"));
        assert!(!valid_key(""));
    }
}
