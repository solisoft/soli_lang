//! Spam defences for form posts, as Soli sees them.
//!
//! - `spam_fields()` — the honeypot and the signed render time. `csrf_field()`
//!   (and so `form_with` and `button_to`) already includes them.
//! - `pow_field(bits = 17)` — a proof-of-work challenge, solved in the browser
//!   by `/__soli/pow.js`. `form_with(..., {"pow": true})` adds it.
//! - `spam_check(req, options = {})` — `nil` when the post may go through,
//!   otherwise `{"reason", "show", "score"}`.
//! - `client_ip(req)` — the client's address, behind a trusted proxy too.
//! - `SpamFilter` — the naive Bayes model `spam_check` scores with:
//!   `train(text, "spam"|"ham")`, `untrain`, `score(text)`, `trained?()`,
//!   `stats()`, `reset()`.
//!
//! The decisions live in `crate::serve::spam`; this file reads `req` and the
//! options and builds the answers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};
use crate::platform::unix_now_secs;
use crate::serve::spam;

/// Form fields whose value is never read as message text nor stored: secrets
/// and payment data a form may carry next to the message.
const PRIVATE_FIELD_PARTS: [&str; 8] = [
    "pass", "token", "secret", "card", "cvv", "cvc", "iban", "csrf",
];

/// Characters kept per stored field, and for the text of a submission.
const MAX_FIELD_CHARS: usize = 2000;
const MAX_TEXT_CHARS: usize = 5000;

pub fn register_spam_builtins(env: &mut Environment) {
    // spam_fields() -> String
    env.define(
        "spam_fields".to_string(),
        Value::NativeFunction(NativeFunction::new("spam_fields", Some(0), |_args| {
            Ok(Value::String(spam_fields_html().into()))
        })),
    );

    // pow_field(bits = 17) -> String
    env.define(
        "pow_field".to_string(),
        Value::NativeFunction(NativeFunction::new("pow_field", None, |args| {
            let bits = match args.first() {
                None | Some(Value::Null) => spam::DEFAULT_POW_BITS,
                Some(Value::Int(n)) if *n >= 1 => (*n as u32).min(spam::MAX_POW_BITS),
                Some(other) => {
                    return Err(format!(
                        "pow_field() expects a difficulty in bits (1-{}), got {}",
                        spam::MAX_POW_BITS,
                        other.type_name()
                    ))
                }
            };
            crate::template::response_cache::mark_response_dirty();
            let challenge = spam::pow_challenge(bits, unix_now_secs());
            Ok(Value::String(
                format!(
                    "<input type=\"hidden\" name=\"{}\" value=\"\" data-soli-pow=\"{}\">",
                    spam::POW_FIELD,
                    challenge
                )
                .into(),
            ))
        })),
    );

    // client_ip(req) -> String | nil
    env.define(
        "client_ip".to_string(),
        Value::NativeFunction(NativeFunction::new("client_ip", Some(1), |args| {
            Ok(super::rate_limit::extract_client_ip(&args[0])
                .map(|ip| Value::String(ip.into()))
                .unwrap_or(Value::Null))
        })),
    );

    // spam_check(req, options = {}) -> nil | {"reason", "show", "score"}
    env.define(
        "spam_check".to_string(),
        Value::NativeFunction(NativeFunction::new("spam_check", None, |args| {
            let Some(req) = args.first() else {
                return Err("spam_check(req, options = {}) expects the request".to_string());
            };
            let options = Options::read(args.get(1))?;
            Ok(spam_check(req, &options))
        })),
    );

    register_spam_filter_class(env);
}

crate::request_local! {
    /// Numbers the honeypots a page renders, so each form's has an id of its
    /// own (a page renders on one thread). Restarted with every request, so
    /// the same page renders the same ids.
    static HONEYPOT_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Restart the honeypot ids — at the top of every request, with the response
/// cache's per-request state. Without it two identical pages differed, which
/// an app comparing two answers (a "does not reveal whether the address
/// exists" check) reads as a leak.
pub(crate) fn reset_honeypot_ids() {
    HONEYPOT_SEQ.with(|seq| seq.set(0));
}

/// The honeypot (in a `hidden` block, so no stylesheet or CSP is involved) and
/// the signed render time. The honeypot has a `<label for>`: it is outside the
/// accessibility tree, but an accessibility check that wants every text field
/// labelled (an app's spec, an HTML linter) no longer flags every form.
pub(crate) fn spam_fields_html() -> String {
    // The stamp changes on every render: the page must not be cached.
    crate::template::response_cache::mark_response_dirty();
    let id = HONEYPOT_SEQ.with(|seq| {
        seq.set(seq.get().wrapping_add(1));
        format!("{}_{}", spam::HONEYPOT_FIELD, seq.get())
    });
    format!(
        "<div hidden aria-hidden=\"true\"><label for=\"{id}\">Website</label>\
<input type=\"text\" id=\"{id}\" name=\"{}\" value=\"\" tabindex=\"-1\" autocomplete=\"off\"></div>\
<input type=\"hidden\" name=\"{}\" value=\"{}\">",
        spam::HONEYPOT_FIELD,
        spam::STAMP_FIELD,
        spam::form_stamp(unix_now_secs())
    )
}

// --- spam_check ---------------------------------------------------------------

struct Options {
    min_seconds: u64,
    max_age: u64,
    max_links: usize,
    max_score: f64,
    stamp: bool,
    pow: bool,
    rate: Option<(usize, u64)>,
    fields: Option<Vec<String>>,
    record: bool,
    learn: bool,
}

impl Options {
    fn read(value: Option<&Value>) -> Result<Options, String> {
        let mut options = Options {
            min_seconds: spam::DEFAULT_MIN_SECONDS,
            max_age: spam::DEFAULT_MAX_AGE,
            max_links: spam::DEFAULT_MAX_LINKS,
            max_score: spam::DEFAULT_MAX_SCORE,
            stamp: true,
            pow: false,
            rate: None,
            fields: None,
            record: true,
            learn: true,
        };
        let hash = match value {
            None | Some(Value::Null) => return Ok(options),
            Some(Value::Hash(hash)) => hash.borrow(),
            Some(other) => {
                return Err(format!(
                    "spam_check(req, options) expects a Hash of options, got {}",
                    other.type_name()
                ))
            }
        };
        for (key, value) in hash.iter() {
            // `{"pow": true}` and `{pow: true}` alike.
            let (HashKey::String(name) | HashKey::Symbol(name)) = key else {
                continue;
            };
            let name: &str = name;
            match (name, value) {
                ("min_seconds", Value::Int(n)) => options.min_seconds = (*n).max(0) as u64,
                ("max_age", Value::Int(n)) => options.max_age = (*n).max(0) as u64,
                ("max_links", Value::Int(n)) => options.max_links = (*n).max(0) as usize,
                ("max_score", Value::Float(f)) => options.max_score = *f,
                ("max_score", Value::Int(n)) => options.max_score = *n as f64,
                ("stamp", Value::Bool(b)) => options.stamp = *b,
                ("pow", Value::Bool(b)) => options.pow = *b,
                ("record", Value::Bool(b)) => options.record = *b,
                ("learn", Value::Bool(b)) => options.learn = *b,
                ("rate", Value::Null) => options.rate = None,
                ("rate", Value::Hash(rate)) => {
                    let rate = rate.borrow();
                    let int = |field: &str| {
                        let found = rate
                            .get(&HashKey::String(field.into()))
                            .or_else(|| rate.get(&HashKey::Symbol(field.into())));
                        match found {
                            Some(Value::Int(n)) if *n > 0 => Some(*n),
                            _ => None,
                        }
                    };
                    let (Some(limit), Some(window)) = (int("limit"), int("window")) else {
                        return Err(
                            "spam_check(): \"rate\" expects {\"limit\": n, \"window\": seconds}"
                                .to_string(),
                        );
                    };
                    options.rate = Some((limit as usize, window as u64));
                }
                ("fields", Value::Array(names)) => {
                    options.fields = Some(
                        names
                            .borrow()
                            .iter()
                            .filter_map(|n| match n {
                                Value::String(s) => Some(s.to_string()),
                                _ => None,
                            })
                            .collect(),
                    );
                }
                (other, value) => {
                    return Err(format!(
                        "spam_check(): unknown option {other:?} (or a {} where it expects \
another type); options are min_seconds, max_age, max_links, max_score, stamp, pow, rate, \
fields, record, learn",
                        value.type_name()
                    ))
                }
            }
        }
        Ok(options)
    }
}

/// The posted fields, flattened to `name` / `parent[child]` paths.
fn posted_fields(req: &Value) -> Vec<(String, String)> {
    let Value::Hash(request) = req else {
        return Vec::new();
    };
    let request = request.borrow();
    let source = ["form", "json", "all"].iter().find_map(|key| {
        match request.get(&HashKey::String((*key).into())) {
            Some(value @ Value::Hash(_)) => Some(value.clone()),
            _ => None,
        }
    });
    let mut out = Vec::new();
    if let Some(source) = source {
        flatten("", &source, &mut out, 0);
    }
    out
}

fn flatten(prefix: &str, value: &Value, out: &mut Vec<(String, String)>, depth: usize) {
    if depth > 8 || out.len() > 200 {
        return;
    }
    match value {
        Value::Hash(hash) => {
            for (key, child) in hash.borrow().iter() {
                let name = match key {
                    HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
                    other => format!("{other}"),
                };
                let path = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}[{name}]")
                };
                flatten(&path, child, out, depth + 1);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.borrow().iter().enumerate() {
                flatten(&format!("{prefix}[{index}]"), child, out, depth + 1);
            }
        }
        Value::String(s) => out.push((prefix.to_string(), s.to_string())),
        Value::Int(_) | Value::Float(_) | Value::Bool(_) => {
            out.push((prefix.to_string(), format!("{value}")))
        }
        _ => {}
    }
}

/// The last segment of `contact[text]`: `text`.
fn leaf(name: &str) -> &str {
    name.rsplit('[')
        .next()
        .map(|s| s.trim_end_matches(']'))
        .unwrap_or(name)
}

fn is_private(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with('_') || PRIVATE_FIELD_PARTS.iter().any(|part| lower.contains(part))
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn hash_value(pairs: Vec<(&str, Value)>) -> Value {
    let mut out = HashPairs::default();
    for (key, value) in pairs {
        out.insert(HashKey::String(key.into()), value);
    }
    Value::Hash(Rc::new(RefCell::new(out)))
}

fn spam_check(req: &Value, options: &Options) -> Value {
    let fields = posted_fields(req);
    let now = unix_now_secs();

    // What a person wrote: the chosen fields, or every field that is neither
    // the framework's nor private.
    let analysed: Vec<&(String, String)> = fields
        .iter()
        .filter(|(name, _)| match &options.fields {
            Some(wanted) => wanted.iter().any(|w| w == name || w == leaf(name)),
            None => !is_private(name) && !is_private(leaf(name)),
        })
        .collect();
    let text = truncate(
        &analysed
            .iter()
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        MAX_TEXT_CHARS,
    );

    let ip = super::rate_limit::extract_client_ip(req).unwrap_or_default();
    let path = match req {
        Value::Hash(h) => super::args::hash_str(&h.borrow(), "path").unwrap_or_default(),
        _ => String::new(),
    };

    let score = spam::with_model(|model| model.score(&text));
    let trained = spam::with_model(|model| model.trained());

    let reason: Option<&'static str> = (|| {
        if !field(&fields, spam::HONEYPOT_FIELD)
            .unwrap_or("")
            .is_empty()
        {
            return Some("honeypot");
        }
        if options.stamp {
            let stamp = field(&fields, spam::STAMP_FIELD).unwrap_or("");
            if let Err(reason) = spam::check_stamp(stamp, now, options.min_seconds, options.max_age)
            {
                return Some(reason);
            }
        }
        if let Some((limit, window)) = options.rate {
            let key = format!("spam:{path}:{}", super::rate_limit::rate_limit_ip_key(&ip));
            if !super::rate_limit::allow(&key, limit, window) {
                return Some("rate");
            }
        }
        let solution = field(&fields, spam::POW_FIELD).unwrap_or("");
        if options.pow || !solution.is_empty() {
            if let Err(reason) = spam::verify_pow(solution, now) {
                return Some(reason);
            }
        }
        if spam::count_links(&text) > options.max_links {
            return Some("links");
        }
        if trained && score >= options.max_score {
            return Some("score");
        }
        None
    })();

    // Learn from the certain cases only: a bot that filled the honeypot or
    // answered within seconds is spam; an accepted post is ham until someone
    // says otherwise on /__soli/spam.
    let learnt = match reason {
        _ if !options.learn || text.is_empty() => None,
        Some("honeypot") | Some("fast") => Some("spam"),
        None => Some("ham"),
        Some(_) => None,
    };
    if let Some(label) = learnt {
        spam::update_model(|model| model.train(&text, label == "spam"));
    }

    if options.record {
        let stored: serde_json::Map<String, serde_json::Value> = fields
            .iter()
            .filter(|(name, _)| !is_private(name) && !is_private(leaf(name)))
            .map(|(name, value)| (name.clone(), truncate(value, MAX_FIELD_CHARS).into()))
            .collect();
        spam::record(serde_json::json!({
            "created_at": crate::jobs::now_iso(),
            "path": path,
            "ip": ip,
            "reason": reason.unwrap_or(""),
            "score": (score * 1000.0).round() / 1000.0,
            "status": if reason.is_some() { "spam" } else { "ham" },
            "trained": learnt,
            "fields": stored,
            "text": text,
        }));
    }

    match reason {
        None => Value::Null,
        Some(reason) => hash_value(vec![
            ("reason", Value::String(reason.into())),
            ("show", Value::Bool(spam::shown(reason))),
            ("score", Value::Float(score)),
        ]),
    }
}

// --- SpamFilter ---------------------------------------------------------------

fn label_arg(args: &[Value], fn_name: &str) -> Result<(String, bool), String> {
    let text = super::args::string_arg(args, 0, fn_name, "text")?;
    let label = super::args::string_arg(args, 1, fn_name, "label")?;
    match label.as_str() {
        "spam" => Ok((text, true)),
        "ham" => Ok((text, false)),
        other => Err(format!(
            "{fn_name}() expects the label \"spam\" or \"ham\", got {other:?}"
        )),
    }
}

fn register_spam_filter_class(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    methods.insert(
        "train".to_string(),
        Rc::new(NativeFunction::new("SpamFilter.train", Some(2), |args| {
            let (text, spam_label) = label_arg(args, "SpamFilter.train")?;
            spam::update_model(|model| model.train(&text, spam_label));
            Ok(Value::Null)
        })),
    );

    methods.insert(
        "untrain".to_string(),
        Rc::new(NativeFunction::new("SpamFilter.untrain", Some(2), |args| {
            let (text, spam_label) = label_arg(args, "SpamFilter.untrain")?;
            spam::update_model(|model| model.untrain(&text, spam_label));
            Ok(Value::Null)
        })),
    );

    methods.insert(
        "score".to_string(),
        Rc::new(NativeFunction::new("SpamFilter.score", Some(1), |args| {
            let text = super::args::string_arg(args, 0, "SpamFilter.score", "text")?;
            Ok(Value::Float(spam::with_model(|model| model.score(&text))))
        })),
    );

    methods.insert(
        "trained?".to_string(),
        Rc::new(NativeFunction::new(
            "SpamFilter.trained?",
            Some(0),
            |_args| Ok(Value::Bool(spam::with_model(|model| model.trained()))),
        )),
    );

    methods.insert(
        "stats".to_string(),
        Rc::new(NativeFunction::new("SpamFilter.stats", Some(0), |_args| {
            let (spam_docs, ham_docs, words, trained) = spam::with_model(|model| {
                (
                    model.spam_docs,
                    model.ham_docs,
                    model.words.len(),
                    model.trained(),
                )
            });
            Ok(hash_value(vec![
                ("spam", Value::Int(spam_docs as i64)),
                ("ham", Value::Int(ham_docs as i64)),
                ("words", Value::Int(words as i64)),
                ("trained", Value::Bool(trained)),
            ]))
        })),
    );

    methods.insert(
        "reset".to_string(),
        Rc::new(NativeFunction::new("SpamFilter.reset", Some(0), |_args| {
            spam::reset_model();
            Ok(Value::Null)
        })),
    );

    let class = Class {
        name: "SpamFilter".to_string(),
        native_static_methods: methods,
        ..Default::default()
    };
    env.define("SpamFilter".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_takes_the_last_bracket_segment() {
        assert_eq!(leaf("contact[text]"), "text");
        assert_eq!(leaf("items[0][sku]"), "sku");
        assert_eq!(leaf("email"), "email");
    }

    #[test]
    fn private_fields_are_neither_read_nor_stored() {
        assert!(is_private("_csrf_token"));
        assert!(is_private("password_confirmation"));
        assert!(is_private("card_number"));
        assert!(!is_private("text"));
        assert!(!is_private("email"));
    }
}
