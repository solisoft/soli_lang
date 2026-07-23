//! `Push.deliver` — one call that reaches a user however you can.
//!
//! There are four ways to notify someone, and which applies depends only on
//! *where they are*, not on anything the calling code should have to branch on:
//!
//! | | reaches |
//! |---|---|
//! | the native bridge ([`super::native`]) | an app that is **open** |
//! | Web Push ([`super::vapid`]) | a browser / PWA that is **closed** |
//! | APNs ([`super::apns`]) | a macOS / iOS shell that is **closed** |
//! | FCM ([`super::fcm`]) | an Android shell that is **closed** |
//!
//! `Push.deliver` is the cascade over all four. It tries the bridge first —
//! free, instant, no push service — and only falls through to a push transport
//! for the targets it did not already reach, routing each by its platform.
//!
//! ```soli
//! result = Push.deliver("user:#{str(user.id)}", {
//!   "title": "New ping",
//!   "body":  "Ana replied",
//!   "url":   "/pings/3"
//! }, {
//!   "targets": user.push_targets(),   # [{platform, token|subscription}, ...]
//!   "apns":    { "key": key, "key_id": "…", "team_id": "…", "topic": "…" },
//!   "fcm":     { "service_account": account }
//!   # vapid keys are read from VAPID_* env when not given here
//! })
//! ```
//!
//! The framework cannot own the device list — where a user's tokens live is the
//! app's schema — so the targets are passed in. What the framework owns is the
//! part that is the same in every app: try the bridge, route each target,
//! detect the dead ones. `result["prune"]` is the list of tokens the push
//! service reported as gone (a `410`, an `UNREGISTERED`); delete those so the
//! store does not accumulate corpses.

use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

/// A platform label, normalized. Everything Apple is `apple`; everything
/// Google is `android`; a browser subscription is `web`.
fn normalize_platform(raw: &str) -> &'static str {
    match raw.to_ascii_lowercase().as_str() {
        "ios" | "macos" | "apple" | "darwin" | "apns" => "apple",
        "android" | "fcm" | "google" => "android",
        _ => "web",
    }
}

fn hash_get<'a>(hash: &'a HashPairs, key: &str) -> Option<&'a Value> {
    hash.get(&HashKey::String(key.into()))
}

fn string_field(hash: &HashPairs, key: &str) -> Option<String> {
    match hash_get(hash, key) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn status_of(result: &Value) -> i64 {
    if let Value::Hash(h) = result {
        if let Some(Value::Int(n)) = hash_get(&h.borrow(), "status") {
            return *n;
        }
    }
    0
}

fn reason_of(result: &Value) -> String {
    if let Value::Hash(h) = result {
        match hash_get(&h.borrow(), "reason") {
            Some(Value::String(s)) => return s.to_string(),
            // vapid returns `body`, not `reason`.
            _ => {
                if let Some(Value::String(s)) = hash_get(&h.borrow(), "body") {
                    return s.to_string();
                }
            }
        }
    }
    String::new()
}

/// Whether a send outcome means the target is gone for good and should be
/// pruned — as opposed to a transient failure worth retrying.
///
/// Deliberately conservative: a `400 BadDeviceToken` is *not* here, because its
/// usual cause is the wrong APNs gateway (a sandbox token sent to production),
/// and pruning it would delete a token that is actually fine.
fn is_dead(status: i64, reason: &str) -> bool {
    matches!(status, 404 | 410)
        || reason.contains("Unregistered")
        || reason.contains("UNREGISTERED")
        || reason.contains("NOT_FOUND")
}

struct Vapid {
    public_key: String,
    private_key: String,
    subject: String,
}

/// VAPID keys from the options hash, falling back to the environment — the same
/// `VAPID_*` names `WebPush` uses, so an app that already ships web push needs
/// to configure nothing new.
fn resolve_vapid(options: &HashPairs) -> Option<Vapid> {
    let from_opts = |field: &str| {
        hash_get(options, "vapid").and_then(|v| match v {
            Value::Hash(h) => string_field(&h.borrow(), field),
            _ => None,
        })
    };
    let public_key = from_opts("public_key")
        .or_else(|| std::env::var("VAPID_PUBLIC_KEY").ok())
        .filter(|s| !s.is_empty())?;
    let private_key = from_opts("private_key")
        .or_else(|| std::env::var("VAPID_PRIVATE_KEY").ok())
        .filter(|s| !s.is_empty())?;
    let subject = from_opts("subject")
        .or_else(|| std::env::var("VAPID_SUBJECT").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "mailto:admin@localhost".to_string());
    Some(Vapid {
        public_key,
        private_key,
        subject,
    })
}

fn record(pairs: &mut HashPairs, key: &str, value: Value) {
    pairs.insert(HashKey::String(key.into()), value);
}

fn string_value(s: impl Into<String>) -> Value {
    Value::String(s.into().into())
}

/// The result accumulator, so each target's outcome lands in the right bucket.
#[derive(Default)]
struct Outcome {
    sent: Vec<Value>,
    failed: Vec<Value>,
    prune: Vec<Value>,
}

impl Outcome {
    fn sent(&mut self, platform: &str, id: &str) {
        let mut entry = HashPairs::default();
        record(&mut entry, "platform", string_value(platform));
        record(&mut entry, "target", string_value(id));
        self.sent
            .push(Value::Hash(Rc::new(std::cell::RefCell::new(entry))));
    }

    fn failed(&mut self, platform: &str, id: &str, error: &str, dead: bool) {
        let mut entry = HashPairs::default();
        record(&mut entry, "platform", string_value(platform));
        record(&mut entry, "target", string_value(id));
        record(&mut entry, "error", string_value(error));
        self.failed
            .push(Value::Hash(Rc::new(std::cell::RefCell::new(entry))));
        if dead {
            self.prune.push(string_value(id));
        }
    }
}

/// Route one target to its transport and record the outcome.
fn deliver_to_target(
    target: &Value,
    payload: &Value,
    payload_json: &str,
    options: &HashPairs,
    vapid: &Option<Vapid>,
    out: &mut Outcome,
) {
    let Value::Hash(target_hash) = target else {
        out.failed("web", "", "target is not a hash", false);
        return;
    };
    let target_hash = target_hash.borrow();
    let platform = normalize_platform(
        &string_field(&target_hash, "platform").unwrap_or_else(|| "web".to_string()),
    );

    match platform {
        "apple" => {
            let token = string_field(&target_hash, "token").unwrap_or_default();
            let Some(Value::Hash(apns_opts)) = hash_get(options, "apns") else {
                out.failed("apple", &token, "no apns credentials in options", false);
                return;
            };
            let apns_value = Value::Hash(apns_opts.clone());
            match super::apns::send(&token, payload, &apns_value) {
                Ok(result) => {
                    let status = status_of(&result);
                    let reason = reason_of(&result);
                    if (200..300).contains(&status) {
                        out.sent("apple", &token);
                    } else {
                        out.failed("apple", &token, &reason, is_dead(status, &reason));
                    }
                }
                Err(e) => out.failed("apple", &token, &e, false),
            }
        }
        "android" => {
            let token = string_field(&target_hash, "token").unwrap_or_default();
            let Some(Value::Hash(fcm_opts)) = hash_get(options, "fcm") else {
                out.failed("android", &token, "no fcm credentials in options", false);
                return;
            };
            let fcm_value = Value::Hash(fcm_opts.clone());
            match super::fcm::send(&token, payload, &fcm_value) {
                Ok(result) => {
                    let status = status_of(&result);
                    let reason = reason_of(&result);
                    if (200..300).contains(&status) {
                        out.sent("android", &token);
                    } else {
                        out.failed("android", &token, &reason, is_dead(status, &reason));
                    }
                }
                Err(e) => out.failed("android", &token, &e, false),
            }
        }
        _ => {
            // web: a browser push subscription.
            let subscription = match hash_get(&target_hash, "subscription") {
                Some(sub) => sub.clone(),
                None => target.clone(), // the target may itself be the subscription
            };
            let endpoint = string_field(&target_hash, "endpoint").unwrap_or_else(|| {
                if let Value::Hash(s) = &subscription {
                    string_field(&s.borrow(), "endpoint").unwrap_or_default()
                } else {
                    String::new()
                }
            });
            let Some(vapid) = vapid else {
                out.failed(
                    "web",
                    &endpoint,
                    "no VAPID keys (set VAPID_PUBLIC_KEY / VAPID_PRIVATE_KEY or pass options.vapid)",
                    false,
                );
                return;
            };
            match super::vapid::send_to_subscription(
                &subscription,
                payload_json,
                &vapid.private_key,
                &vapid.public_key,
                &vapid.subject,
                None,
            ) {
                Ok(result) => {
                    let status = status_of(&result);
                    if (200..300).contains(&status) {
                        out.sent("web", &endpoint);
                    } else {
                        out.failed("web", &endpoint, &reason_of(&result), is_dead(status, ""));
                    }
                }
                Err(e) => out.failed("web", &endpoint, &e, false),
            }
        }
    }
}

fn deliver(channel: &str, payload: &Value, options: &Value) -> Result<Value, String> {
    let options = match options {
        Value::Hash(h) => h.borrow().clone(),
        Value::Null => HashPairs::default(),
        other => {
            return Err(format!(
                "Push.deliver(): options must be a hash, got {}",
                other.type_name()
            ))
        }
    };

    // The bridge first — a client with the app open needs no push service.
    let reached_live = super::native::notify(channel, payload)?;

    let always = matches!(hash_get(&options, "always"), Some(Value::Bool(true)));
    let mut out = Outcome::default();

    // Skip the push cascade when the bridge already reached someone, unless the
    // caller insists (a to-all announcement may want every transport).
    let run_push = reached_live == 0 || always;
    if run_push {
        let payload_json = crate::interpreter::value::value_to_json(payload)
            .ok()
            .and_then(|j| serde_json::to_string(&j).ok())
            .unwrap_or_else(|| "{}".to_string());
        let vapid = resolve_vapid(&options);

        if let Some(Value::Array(targets)) = hash_get(&options, "targets") {
            for target in targets.borrow().iter() {
                deliver_to_target(target, payload, &payload_json, &options, &vapid, &mut out);
            }
        }
    }

    let mut result = HashPairs::default();
    record(&mut result, "reached_live", Value::Int(reached_live));
    record(
        &mut result,
        "transport",
        string_value(if reached_live > 0 && !always {
            "native"
        } else if !out.sent.is_empty() {
            "push"
        } else {
            "none"
        }),
    );
    record(
        &mut result,
        "sent",
        Value::Array(Rc::new(std::cell::RefCell::new(out.sent))),
    );
    record(
        &mut result,
        "failed",
        Value::Array(Rc::new(std::cell::RefCell::new(out.failed))),
    );
    record(
        &mut result,
        "prune",
        Value::Array(Rc::new(std::cell::RefCell::new(out.prune))),
    );
    Ok(Value::Hash(Rc::new(std::cell::RefCell::new(result))))
}

pub fn register_push_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    statics.insert(
        "deliver".to_string(),
        Rc::new(NativeFunction::new("Push.deliver", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err("Push.deliver() expects (channel, payload, options?)".to_string());
            }
            let channel = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "Push.deliver() expects a string channel, got {}",
                        other.type_name()
                    ))
                }
            };
            let options = args.get(2).cloned().unwrap_or(Value::Null);
            deliver(&channel, &args[1], &options)
        })),
    );

    let class = Rc::new(Class {
        name: "Push".to_string(),
        superclass: None,
        methods: Rc::new(std::cell::RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: statics,
        native_methods: HashMap::new(),
        static_fields: Rc::new(std::cell::RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(std::cell::RefCell::new(HashMap::new())),
        ..Default::default()
    });

    env.define("Push".to_string(), Value::Class(class));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_labels_normalize() {
        for apple in ["ios", "iOS", "macos", "apple", "darwin", "APNS"] {
            assert_eq!(normalize_platform(apple), "apple", "{}", apple);
        }
        for android in ["android", "Android", "fcm", "google"] {
            assert_eq!(normalize_platform(android), "android", "{}", android);
        }
        for web in ["web", "browser", "pwa", ""] {
            assert_eq!(normalize_platform(web), "web", "{}", web);
        }
    }

    /// The pruning rule is the load-bearing decision: prune what is gone, keep
    /// what might just be misconfigured.
    #[test]
    fn dead_targets_are_recognized_and_wrong_gateway_is_not() {
        assert!(is_dead(410, ""), "410 Gone");
        assert!(is_dead(404, ""), "404 Not Found");
        assert!(is_dead(0, "Unregistered"), "APNs Unregistered");
        assert!(is_dead(0, "UNREGISTERED"), "FCM UNREGISTERED");
        assert!(is_dead(0, "NOT_FOUND"), "FCM NOT_FOUND");

        // A sandbox token sent to production: fix the gateway, do NOT delete it.
        assert!(!is_dead(400, "BadDeviceToken"));
        assert!(!is_dead(200, ""));
        assert!(!is_dead(503, "UNAVAILABLE"));
    }

    #[test]
    fn a_sent_entry_carries_its_platform_and_target() {
        let mut out = Outcome::default();
        out.sent("apple", "abc123");
        assert_eq!(out.sent.len(), 1);
        if let Value::Hash(h) = &out.sent[0] {
            assert_eq!(
                string_field(&h.borrow(), "platform").as_deref(),
                Some("apple")
            );
            assert_eq!(
                string_field(&h.borrow(), "target").as_deref(),
                Some("abc123")
            );
        } else {
            panic!("expected a hash");
        }
    }

    /// A dead target lands in both `failed` and `prune`; a live failure lands
    /// only in `failed`.
    #[test]
    fn only_dead_failures_are_queued_for_pruning() {
        let mut out = Outcome::default();
        out.failed("android", "gone", "UNREGISTERED", true);
        out.failed("android", "flaky", "UNAVAILABLE", false);
        assert_eq!(out.failed.len(), 2);
        assert_eq!(out.prune.len(), 1);
        assert_eq!(
            match &out.prune[0] {
                Value::String(s) => s.to_string(),
                _ => String::new(),
            },
            "gone"
        );
    }
}
