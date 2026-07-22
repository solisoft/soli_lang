//! `Native.*` — reaching the shell an app is being viewed in.
//!
//! A Soli app packaged with `soli desktop build`, or wrapped in a WebView on a
//! phone, renders inside an embedded web view — and neither `WKWebView` nor
//! Android's `WebView` implements the Push API or the Notifications API. Both
//! platforms reserve those for the browser proper. So an app that ships web
//! push (see [`super::vapid`]) reaches browsers and installed PWAs, and
//! silently reaches nothing at all inside its own native shell.
//!
//! This is the missing channel: server-side Soli code addressing the client
//! that is *currently looking at the page*.
//!
//! ```soli
//! Native.notify("user:42", { "title": "New ping", "body": "…", "url": "/pings/3" })
//! ```
//!
//! Delivery rides the existing SSE fan-out ([`super::streaming::broadcast_sse`]),
//! so there is no new transport here — and no push service, no certificates and
//! no VAPID keys. The trade is deliberate and worth stating plainly: **this only
//! reaches a client that has the app open.** `notify` returns how many it
//! reached, so an app can fall through to real push when that is zero:
//!
//! ```soli
//! WebPush.deliver_to_user(user_id, title, body, url) if Native.notify(channel, payload) == 0
//! ```
//!
//! For a desktop app carrying its own database that fallback rarely matters —
//! nothing happens while the app is closed — but for a shared remote database
//! it is the difference between a notification and silence.
//!
//! # Channel tokens
//!
//! Subscribing is a `GET` from the browser, so the channel cannot simply be a
//! query parameter: anyone could listen to `user:42`. Pages emit a signed token
//! instead (`native_channel(channel)` in a view), and the stream endpoint
//! verifies it before subscribing. The key is HKDF-SHA256-derived from the
//! session secret with its own info label, domain-separated from the cookie jar
//! and the session driver.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use sha2::Sha256;

use crate::interpreter::builtins::crypto::{do_secure_compare, hmac_sha256_bytes};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

/// SSE topic prefix. Namespaced so an app's own `sse_broadcast` topics can
/// never collide with — or be reachable through — a native channel token.
const TOPIC_PREFIX: &str = "__soli_native:";

/// HKDF info label, domain-separating this key from `soli.cookie.signed.v1`,
/// `soli.cookie.encrypted.v1` and `soli.session.cookie.v1`.
const CHANNEL_INFO: &[u8] = b"soli.native.channel.v1";

/// Same floor as the cookie jar: a short secret caps the derived key's entropy
/// however strong the KDF is.
const MIN_SECRET_LEN: usize = 32;

/// How long a channel token stays valid. Long enough to outlive a working day
/// with the page open (the client re-reads the token when it reconnects, and a
/// navigation mints a fresh one), short enough that a leaked URL goes stale.
const TOKEN_TTL_SECS: u64 = 12 * 60 * 60;

/// Bounds an attacker-controlled token before it reaches base64 or the MAC.
const MAX_TOKEN_LEN: usize = 512;

/// Bounds a channel name. Long channels are a mistake, not a use case.
const MAX_CHANNEL_LEN: usize = 128;

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The SSE topic backing a channel.
pub fn topic_for(channel: &str) -> String {
    format!("{}{}", TOPIC_PREFIX, channel)
}

/// Derive the channel-signing key from an explicit secret.
///
/// Split from [`channel_key`] so the token format can be tested without
/// touching the process-global session config, which every other test in the
/// tree also reads.
fn derive_channel_key(secret: &str) -> Result<[u8; 32], String> {
    if secret.len() < MIN_SECRET_LEN {
        return Err(format!(
            "native channels require a session secret of at least {} characters (got {})",
            MIN_SECRET_LEN,
            secret.len()
        ));
    }
    let hk = Hkdf::<Sha256>::new(None, secret.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(CHANNEL_INFO, &mut key)
        .map_err(|e| format!("native channel key derivation failed: {}", e))?;
    Ok(key)
}

/// The signing key for the configured session secret.
fn channel_key() -> Result<[u8; 32], String> {
    let secret = super::session::get_session_config().secret.ok_or_else(|| {
        "native channels require a session secret: set SOLI_SESSION_SECRET (32+ chars) \
         or call session_configure({\"secret\": ...})"
            .to_string()
    })?;
    derive_channel_key(&secret)
}

/// A channel name must be printable, bounded, and free of the separator the
/// token format uses — otherwise `a.b|c` could be signed as one channel and
/// read back as another.
fn validate_channel(channel: &str) -> Result<(), String> {
    if channel.is_empty() {
        return Err("native channel name cannot be empty".to_string());
    }
    if channel.len() > MAX_CHANNEL_LEN {
        return Err(format!(
            "native channel name exceeds {} characters",
            MAX_CHANNEL_LEN
        ));
    }
    if channel
        .bytes()
        .any(|b| b < 0x20 || b == 0x7f || b == b'.' || b == b'|')
    {
        return Err(
            "native channel name may not contain '.', '|' or control characters".to_string(),
        );
    }
    Ok(())
}

/// Mint a token for `channel`: `b64(channel)` ‖ `.` ‖ `b64(expiry)` ‖ `.` ‖ `b64(mac)`.
///
/// The MAC covers the *encoded* channel and expiry, so verification needs no
/// decode before the signature is known good.
pub fn sign_channel(channel: &str) -> Result<String, String> {
    sign_channel_with(&channel_key()?, channel)
}

fn sign_channel_with(key: &[u8; 32], channel: &str) -> Result<String, String> {
    validate_channel(channel)?;
    let expiry = now_unix_secs().saturating_add(TOKEN_TTL_SECS);
    Ok(token_for(key, channel, expiry))
}

fn token_for(key: &[u8; 32], channel: &str, expiry: u64) -> String {
    let channel_b64 = URL_SAFE_NO_PAD.encode(channel.as_bytes());
    let expiry_b64 = URL_SAFE_NO_PAD.encode(expiry.to_string().as_bytes());
    let mac = hmac_sha256_bytes(mac_input(&channel_b64, &expiry_b64).as_bytes(), key);
    format!(
        "{}.{}.{}",
        channel_b64,
        expiry_b64,
        URL_SAFE_NO_PAD.encode(mac)
    )
}

fn mac_input(channel_b64: &str, expiry_b64: &str) -> String {
    format!("{}.{}", channel_b64, expiry_b64)
}

/// Recover the channel from a token, or say why not.
///
/// Every failure is the same to the caller — a `403` — but the reasons are kept
/// distinct here because "expired" and "forged" mean very different things when
/// someone is debugging a client that stopped receiving events.
pub fn verify_channel(token: &str) -> Result<String, String> {
    verify_channel_with(&channel_key()?, token)
}

fn verify_channel_with(key: &[u8; 32], token: &str) -> Result<String, String> {
    if token.is_empty() || token.len() > MAX_TOKEN_LEN {
        return Err("malformed native channel token".to_string());
    }
    let mut parts = token.split('.');
    let (Some(channel_b64), Some(expiry_b64), Some(mac_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("malformed native channel token".to_string());
    };

    let expected = URL_SAFE_NO_PAD.encode(hmac_sha256_bytes(
        mac_input(channel_b64, expiry_b64).as_bytes(),
        key,
    ));

    // Constant-time: a byte-at-a-time comparison here leaks the MAC one
    // position at a time to anyone willing to time the endpoint.
    if !do_secure_compare(&expected, mac_b64) {
        return Err("native channel token signature does not verify".to_string());
    }

    let expiry: u64 = String::from_utf8(
        URL_SAFE_NO_PAD
            .decode(expiry_b64)
            .map_err(|_| "malformed native channel token".to_string())?,
    )
    .map_err(|_| "malformed native channel token".to_string())?
    .parse()
    .map_err(|_| "malformed native channel token".to_string())?;

    if now_unix_secs() > expiry {
        return Err("native channel token has expired".to_string());
    }

    let channel = String::from_utf8(
        URL_SAFE_NO_PAD
            .decode(channel_b64)
            .map_err(|_| "malformed native channel token".to_string())?,
    )
    .map_err(|_| "malformed native channel token".to_string())?;
    validate_channel(&channel)?;
    Ok(channel)
}

/// Send `payload` to every client listening on `channel`. Returns the number
/// reached, which is the signal an app uses to decide whether real push is
/// needed.
fn notify(channel: &str, payload: &Value) -> Result<i64, String> {
    validate_channel(channel)?;
    let json = crate::interpreter::value::value_to_json(payload)
        .map_err(|e| format!("Native.notify() could not serialize payload: {}", e))?;
    let envelope = serde_json::json!({ "type": "notify", "payload": json });
    let data = serde_json::to_string(&envelope)
        .map_err(|e| format!("Native.notify() could not serialize payload: {}", e))?;
    Ok(super::streaming::broadcast_sse(&topic_for(channel), &data, Some("soli-native")) as i64)
}

pub fn register_native_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Native.notify(channel, payload) -> Int (clients reached)
    statics.insert(
        "notify".to_string(),
        Rc::new(NativeFunction::new("Native.notify", Some(2), |args| {
            let channel = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "Native.notify() expects a string channel, got {}",
                        other.type_name()
                    ))
                }
            };
            notify(&channel, &args[1]).map(Value::Int)
        })),
    );

    // Native.subscribers(channel) -> Int. Lets an app ask "is anyone looking?"
    // without sending anything.
    statics.insert(
        "subscribers".to_string(),
        Rc::new(NativeFunction::new("Native.subscribers", Some(1), |args| {
            let channel = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "Native.subscribers() expects a string channel, got {}",
                        other.type_name()
                    ))
                }
            };
            validate_channel(&channel)?;
            Ok(Value::Int(
                super::streaming::subscriber_count_for(&topic_for(&channel)) as i64,
            ))
        })),
    );

    // Native.channel_token(channel) -> String. The view helper `native_channel`
    // is the usual way in; this is for apps minting a token for a client that
    // is not a rendered page (a native screen, a test).
    statics.insert(
        "channel_token".to_string(),
        Rc::new(NativeFunction::new(
            "Native.channel_token",
            Some(1),
            |args| {
                let channel = match &args[0] {
                    Value::String(s) => s.to_string(),
                    other => {
                        return Err(format!(
                            "Native.channel_token() expects a string channel, got {}",
                            other.type_name()
                        ))
                    }
                };
                sign_channel(&channel).map(|t| Value::String(t.into()))
            },
        )),
    );

    let class = Rc::new(Class {
        name: "Native".to_string(),
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

    env.define("Native".to_string(), Value::Class(class));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";
    const OTHER_SECRET: &str = "ffffffffffffffffffffffffffffffff";

    fn key(secret: &str) -> [u8; 32] {
        derive_channel_key(secret).expect("derives")
    }

    #[test]
    fn token_round_trips() {
        let k = key(SECRET);
        let token = sign_channel_with(&k, "user:42").expect("signs");
        assert_eq!(
            verify_channel_with(&k, &token).expect("verifies"),
            "user:42"
        );
    }

    /// The whole point of signing: a client cannot listen to a channel it was
    /// not handed. Swapping the channel while keeping the MAC must fail.
    #[test]
    fn a_tampered_channel_does_not_verify() {
        let k = key(SECRET);
        let token = sign_channel_with(&k, "user:42").expect("signs");
        let rest = token.split_once('.').unwrap().1;
        let forged = format!("{}.{}", URL_SAFE_NO_PAD.encode("user:1"), rest);
        assert!(verify_channel_with(&k, &forged).is_err());
    }

    /// Rotating the session secret must invalidate outstanding tokens, the same
    /// way it invalidates sealed cookies.
    #[test]
    fn a_token_from_another_secret_does_not_verify() {
        let token = sign_channel_with(&key(SECRET), "user:42").expect("signs");
        assert!(verify_channel_with(&key(OTHER_SECRET), &token).is_err());
    }

    #[test]
    fn an_expired_token_does_not_verify() {
        let k = key(SECRET);
        let expired = token_for(&k, "user:42", now_unix_secs() - 1);
        let err = verify_channel_with(&k, &expired).expect_err("must reject");
        assert!(err.contains("expired"), "unexpected error: {}", err);
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        let k = key(SECRET);
        let too_long = "x".repeat(MAX_TOKEN_LEN + 1);
        for bad in ["", "a", "a.b", "a.b.c.d", "...", too_long.as_str()] {
            assert!(
                verify_channel_with(&k, bad).is_err(),
                "should reject {:?}",
                bad
            );
        }
    }

    /// A short secret caps the key's entropy however good the KDF is, so it is
    /// refused rather than silently accepted.
    #[test]
    fn short_secrets_are_refused() {
        assert!(derive_channel_key("tooshort").is_err());
    }

    /// The separator must not be smuggled into a channel name: `a|b` signed as
    /// one channel and read back as another would defeat the token entirely.
    #[test]
    fn channel_names_reject_separators_and_control_characters() {
        for bad in ["", "a.b", "a|b", "a\nb", "a\u{7f}b"] {
            assert!(validate_channel(bad).is_err(), "should reject {:?}", bad);
        }
        for good in ["user:42", "room-7", "user:42:pings"] {
            assert!(validate_channel(good).is_ok(), "should accept {:?}", good);
        }
    }

    /// Namespacing keeps native channels out of reach of an app's own SSE
    /// topics, in both directions.
    #[test]
    fn topics_are_namespaced() {
        assert_eq!(topic_for("user:42"), "__soli_native:user:42");
    }
}
