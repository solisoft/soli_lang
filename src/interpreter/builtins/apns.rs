//! Apple Push Notification service — delivery to a *closed* app.
//!
//! The [native bridge](super::native) reaches a client that has the app open.
//! This reaches one that does not: APNs holds the connection, and macOS/iOS
//! display the notification whether or not the app is running.
//!
//! ```soli
//! Apns.send(device_token, { "title": "New ping", "body": "Ana replied" }, {
//!   "key":     File.read("AuthKey_ABC123.p8"),
//!   "key_id":  "ABC123DEFG",
//!   "team_id": "1A2B3C4D5E",
//!   "topic":   "net.example.myapp"
//! })
//! ```
//!
//! # What this costs, before you build on it
//!
//! Unlike the native bridge, APNs is **not** free of setup. Receiving requires
//! the `aps-environment` entitlement, which comes from a provisioning profile,
//! which requires a **paid Apple Developer account**. An ad-hoc signed app
//! cannot receive a push no matter how correct this sender is.
//!
//! # Authentication
//!
//! Token-based (JWT), not certificate-based: one `.p8` key works for every app
//! under a team and never expires, where certificates are per-app and expire
//! annually. The JWT is ES256 over `{iss: team, iat: now}` with the key id in
//! the header — the same curve and construction as VAPID, so this reuses the
//! machinery in [`super::vapid`] rather than adding a JWT dependency.
//!
//! Apple rate-limits token *minting*, not use: a token is valid for an hour and
//! reissuing it more than once every 20 minutes earns a `TooManyProviderTokenUpdates`.
//! So tokens are cached per (key id, team) and reused until they age out.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

/// Production and development gateways. The wrong one is the single most
/// common cause of `BadDeviceToken`: a token minted by a development build is
/// meaningless to the production gateway and vice versa.
const HOST_PRODUCTION: &str = "https://api.push.apple.com";
const HOST_SANDBOX: &str = "https://api.sandbox.push.apple.com";

/// How long a provider token is reused. Apple accepts them for an hour and
/// refuses *minting* more often than every 20 minutes, so 45 minutes leaves
/// room on both sides.
const TOKEN_REUSE_SECS: u64 = 45 * 60;

/// A device token is 32 bytes hex-encoded, but Apple has widened it before;
/// bound it loosely rather than pin an exact length that a future device breaks.
const MAX_DEVICE_TOKEN_LEN: usize = 200;

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Provider tokens, keyed by (key id, team id), each with the time it was
/// minted. One process may legitimately send for several apps under several
/// teams, so this is a map rather than a single slot.
type TokenCache = Mutex<HashMap<(String, String), (String, u64)>>;

fn token_cache() -> &'static TokenCache {
    static CACHE: OnceLock<TokenCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Mint (or reuse) the ES256 provider token.
///
/// `key_pem` is the contents of the `.p8` file Apple hands out, PKCS#8 PEM.
pub fn provider_token(key_pem: &str, key_id: &str, team_id: &str) -> Result<String, String> {
    let cache_key = (key_id.to_string(), team_id.to_string());
    let now = now_unix_secs();

    if let Ok(cache) = token_cache().lock() {
        if let Some((token, issued)) = cache.get(&cache_key) {
            if now.saturating_sub(*issued) < TOKEN_REUSE_SECS {
                return Ok(token.clone());
            }
        }
    }

    let signing_key = SigningKey::from_pkcs8_pem(key_pem.trim()).map_err(|e| {
        format!(
            "Apns: the auth key is not a valid PKCS#8 EC private key ({}). \
             Pass the contents of the .p8 file Apple issued, including the \
             BEGIN/END PRIVATE KEY lines.",
            e
        )
    })?;

    let header = format!(
        r#"{{"alg":"ES256","kid":"{}"}}"#,
        json_escape_ascii(key_id)?
    );
    let claims = format!(
        r#"{{"iss":"{}","iat":{}}}"#,
        json_escape_ascii(team_id)?,
        now
    );
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.as_bytes()),
        URL_SAFE_NO_PAD.encode(claims.as_bytes())
    );

    let signature: Signature = signing_key.sign(signing_input.as_bytes());
    let token = format!(
        "{}.{}",
        signing_input,
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    );

    if let Ok(mut cache) = token_cache().lock() {
        cache.insert(cache_key, (token.clone(), now));
    }
    Ok(token)
}

/// Key ids, team ids and topics are opaque identifiers, not free text. Rejecting
/// anything else keeps them out of the JSON without an escaping dance.
fn json_escape_ascii(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 128 {
        return Err(format!("Apns: '{}' is not a plausible identifier", value));
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(format!(
            "Apns: '{}' contains characters that are not valid in an Apple identifier",
            value
        ));
    }
    Ok(value.to_string())
}

/// The JSON body APNs expects for a plain alert.
fn payload_json(payload: &Value) -> Result<String, String> {
    let mut json = crate::interpreter::value::value_to_json(payload)
        .map_err(|e| format!("Apns.send(): payload is not serializable: {}", e))?;

    // A caller who already knows APNs may hand us a full `aps` envelope; anyone
    // else gets the common shape built for them.
    if json.get("aps").is_some() {
        return serde_json::to_string(&json)
            .map_err(|e| format!("Apns.send(): payload serialization failed: {}", e));
    }

    let object = json
        .as_object_mut()
        .ok_or_else(|| "Apns.send(): payload must be a hash".to_string())?;

    let title = object.remove("title").unwrap_or(serde_json::Value::Null);
    let body = object.remove("body").unwrap_or(serde_json::Value::Null);
    let badge = object.remove("badge");
    let sound = object
        .remove("sound")
        .unwrap_or_else(|| serde_json::Value::String("default".to_string()));

    let mut alert = serde_json::Map::new();
    if !title.is_null() {
        alert.insert("title".to_string(), title);
    }
    if !body.is_null() {
        alert.insert("body".to_string(), body);
    }

    let mut aps = serde_json::Map::new();
    aps.insert("alert".to_string(), serde_json::Value::Object(alert));
    aps.insert("sound".to_string(), sound);
    if let Some(badge) = badge {
        aps.insert("badge".to_string(), badge);
    }

    // Everything the caller left over rides along as custom data, which is what
    // the app reads on tap (a `url`, an id).
    let mut root = serde_json::Map::new();
    root.insert("aps".to_string(), serde_json::Value::Object(aps));
    for (key, value) in object.iter() {
        root.insert(key.clone(), value.clone());
    }

    serde_json::to_string(&serde_json::Value::Object(root))
        .map_err(|e| format!("Apns.send(): payload serialization failed: {}", e))
}

struct SendOptions {
    key: String,
    key_id: String,
    team_id: String,
    topic: String,
    sandbox: bool,
    priority: u8,
    push_type: String,
    collapse_id: Option<String>,
    expiration: Option<i64>,
}

fn string_opt(options: &HashPairs, key: &str) -> Option<String> {
    match options.get(&HashKey::String(key.into())) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn parse_options(value: &Value) -> Result<SendOptions, String> {
    let Value::Hash(hash) = value else {
        return Err("Apns.send(): options must be a hash".to_string());
    };
    let options = hash.borrow();

    let required = |name: &str| -> Result<String, String> {
        string_opt(&options, name).ok_or_else(|| {
            format!(
                "Apns.send(): options is missing '{}'. Required: key (the .p8 contents), \
                 key_id, team_id, topic (your bundle id).",
                name
            )
        })
    };

    let key_id = required("key_id")?;
    let team_id = required("team_id")?;
    let topic = required("topic")?;
    json_escape_ascii(&key_id)?;
    json_escape_ascii(&team_id)?;
    json_escape_ascii(&topic)?;

    Ok(SendOptions {
        key: required("key")?,
        key_id,
        team_id,
        topic,
        sandbox: matches!(
            options.get(&HashKey::String("sandbox".into())),
            Some(Value::Bool(true))
        ),
        priority: match options.get(&HashKey::String("priority".into())) {
            Some(Value::Int(n)) if *n == 5 || *n == 10 => *n as u8,
            _ => 10,
        },
        push_type: string_opt(&options, "push_type").unwrap_or_else(|| "alert".to_string()),
        collapse_id: string_opt(&options, "collapse_id"),
        expiration: match options.get(&HashKey::String("expiration".into())) {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        },
    })
}

/// POST one notification. Returns `{"status": Int, "reason": String}` rather
/// than throwing: a dead device token is an ordinary outcome to be handled
/// (prune it), not an exception.
pub fn send(device_token: &str, payload: &Value, options: &Value) -> Result<Value, String> {
    if device_token.is_empty() || device_token.len() > MAX_DEVICE_TOKEN_LEN {
        return Err("Apns.send(): implausible device token".to_string());
    }
    if !device_token.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("Apns.send(): device token must be hex, as the device reports it".to_string());
    }

    let options = parse_options(options)?;
    let token = provider_token(&options.key, &options.key_id, &options.team_id)?;
    let body = payload_json(payload)?;

    let host = if options.sandbox {
        HOST_SANDBOX
    } else {
        HOST_PRODUCTION
    };
    let url = format!("{}/3/device/{}", host, device_token);

    // Blocking client, as `vapid_send` uses. HTTP/2 is negotiated over ALPN;
    // APNs speaks nothing else, which is why the `http2` feature is enabled.
    let client = reqwest::blocking::Client::new();
    let mut request = client
        .post(&url)
        .header("authorization", format!("bearer {}", token))
        .header("apns-topic", &options.topic)
        .header("apns-push-type", &options.push_type)
        .header("apns-priority", options.priority.to_string())
        .header("content-type", "application/json")
        .body(body);

    if let Some(collapse_id) = &options.collapse_id {
        request = request.header("apns-collapse-id", collapse_id);
    }
    if let Some(expiration) = options.expiration {
        request = request.header("apns-expiration", expiration.to_string());
    }

    let response = request
        .send()
        .map_err(|e| format!("Apns.send(): HTTP request failed: {}", e))?;

    let status = response.status().as_u16();
    let text = response.text().unwrap_or_default();

    // Apple answers a failure with `{"reason": "..."}`; success is 200, empty.
    let reason = if text.trim().is_empty() {
        String::new()
    } else {
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("reason")
                    .and_then(|r| r.as_str().map(|s| s.to_string()))
            })
            .unwrap_or(text)
    };

    let mut result = HashPairs::default();
    result.insert(HashKey::String("status".into()), Value::Int(status as i64));
    result.insert(
        HashKey::String("reason".into()),
        Value::String(reason.into()),
    );
    Ok(Value::Hash(Rc::new(std::cell::RefCell::new(result))))
}

pub fn register_apns_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    statics.insert(
        "send".to_string(),
        Rc::new(NativeFunction::new("Apns.send", Some(3), |args| {
            let device_token = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "Apns.send() expects a string device token, got {}",
                        other.type_name()
                    ))
                }
            };
            send(&device_token, &args[1], &args[2])
        })),
    );

    // Apns.token(key, key_id, team_id) — the provider JWT, for callers driving
    // the HTTP themselves (a batch sender, a proxy).
    statics.insert(
        "token".to_string(),
        Rc::new(NativeFunction::new("Apns.token", Some(3), |args| {
            let strings: Result<Vec<String>, String> = args
                .iter()
                .map(|arg| match arg {
                    Value::String(s) => Ok(s.to_string()),
                    other => Err(format!(
                        "Apns.token() expects strings (key, key_id, team_id), got {}",
                        other.type_name()
                    )),
                })
                .collect();
            let strings = strings?;
            provider_token(&strings[0], &strings[1], &strings[2])
                .map(|token| Value::String(token.into()))
        })),
    );

    let class = Rc::new(Class {
        name: "Apns".to_string(),
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

    env.define("Apns".to_string(), Value::Class(class));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway P-256 key in the shape Apple issues (PKCS#8 PEM).
    fn test_key_pem() -> String {
        use p256::pkcs8::EncodePrivateKey;
        let key = SigningKey::random(&mut rand_core::OsRng);
        key.to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .unwrap()
            .to_string()
    }

    #[test]
    fn provider_token_has_three_parts_and_the_key_id_in_its_header() {
        let pem = test_key_pem();
        let token = provider_token(&pem, "ABC123DEFG", "1A2B3C4D5E").expect("mints");
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "a JWT has three parts");

        let header = String::from_utf8(URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        assert!(header.contains("\"alg\":\"ES256\""), "header: {}", header);
        assert!(
            header.contains("\"kid\":\"ABC123DEFG\""),
            "header: {}",
            header
        );

        let claims = String::from_utf8(URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert!(
            claims.contains("\"iss\":\"1A2B3C4D5E\""),
            "claims: {}",
            claims
        );
    }

    /// The load-bearing property: Apple verifies this signature with the public
    /// half of the key. A structurally perfect JWT that does not verify is
    /// rejected with `InvalidProviderToken`, so check the signature itself.
    #[test]
    fn the_provider_token_signature_verifies_against_the_key() {
        use p256::ecdsa::signature::Verifier;
        use p256::ecdsa::VerifyingKey;
        use p256::pkcs8::EncodePrivateKey;

        let key = SigningKey::random(&mut rand_core::OsRng);
        let pem = key
            .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .unwrap()
            .to_string();
        let verifying = VerifyingKey::from(&key);

        let token = provider_token(&pem, "VERIFYKEY1", "VERIFYTEAM").expect("mints");
        let (signing_input, signature_b64) = token.rsplit_once('.').unwrap();
        let signature =
            Signature::from_slice(&URL_SAFE_NO_PAD.decode(signature_b64).unwrap()).unwrap();

        verifying
            .verify(signing_input.as_bytes(), &signature)
            .expect("signature must verify against the issuing key");
    }

    /// Apple refuses tokens minted more often than every 20 minutes, so reuse
    /// is not an optimization — it is what keeps the sender working.
    #[test]
    fn provider_tokens_are_reused_within_their_window() {
        let pem = test_key_pem();
        let first = provider_token(&pem, "REUSEKEY01", "REUSETEAM1").unwrap();
        let second = provider_token(&pem, "REUSEKEY01", "REUSETEAM1").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn a_key_that_is_not_pkcs8_is_rejected_with_advice() {
        let err = provider_token("not-a-key", "ABC123DEFG", "1A2B3C4D5E").expect_err("rejects");
        assert!(err.contains(".p8"), "error should name the file: {}", err);
    }

    #[test]
    fn identifiers_reject_anything_that_is_not_an_apple_identifier() {
        for bad in ["", "has space", "quote\"inside", "brace{"] {
            assert!(json_escape_ascii(bad).is_err(), "should reject {:?}", bad);
        }
        for good in ["ABC123DEFG", "net.example.app", "team-id_1"] {
            assert!(json_escape_ascii(good).is_ok(), "should accept {:?}", good);
        }
    }

    /// The common case: a caller passes title/body and gets a correct `aps`
    /// envelope without having to know the format.
    #[test]
    fn payload_is_wrapped_in_an_aps_envelope() {
        let mut pairs = HashPairs::default();
        pairs.insert(HashKey::String("title".into()), Value::String("Hi".into()));
        pairs.insert(
            HashKey::String("body".into()),
            Value::String("There".into()),
        );
        pairs.insert(HashKey::String("url".into()), Value::String("/x".into()));
        let value = Value::Hash(Rc::new(std::cell::RefCell::new(pairs)));

        let json: serde_json::Value = serde_json::from_str(&payload_json(&value).unwrap()).unwrap();
        assert_eq!(json["aps"]["alert"]["title"], "Hi");
        assert_eq!(json["aps"]["alert"]["body"], "There");
        assert_eq!(json["aps"]["sound"], "default");
        // Custom keys survive alongside `aps` — that is what the app reads on tap.
        assert_eq!(json["url"], "/x");
    }

    /// A badge sets the icon count on a closed app — the iOS/macOS analogue of
    /// Android's notification_count — and lands in `aps.badge`, not as custom
    /// data.
    #[test]
    fn a_badge_lands_in_the_aps_envelope() {
        let mut pairs = HashPairs::default();
        pairs.insert(HashKey::String("title".into()), Value::String("Hi".into()));
        pairs.insert(HashKey::String("badge".into()), Value::Int(9));
        let value = Value::Hash(Rc::new(std::cell::RefCell::new(pairs)));

        let json: serde_json::Value = serde_json::from_str(&payload_json(&value).unwrap()).unwrap();
        assert_eq!(json["aps"]["badge"], 9);
        assert!(
            json.get("badge").is_none(),
            "badge must not also ship as custom data"
        );
    }

    /// A caller who already speaks APNs keeps full control.
    #[test]
    fn an_explicit_aps_envelope_is_passed_through() {
        let mut aps = HashPairs::default();
        aps.insert(HashKey::String("badge".into()), Value::Int(7));
        let mut pairs = HashPairs::default();
        pairs.insert(
            HashKey::String("aps".into()),
            Value::Hash(Rc::new(std::cell::RefCell::new(aps))),
        );
        let value = Value::Hash(Rc::new(std::cell::RefCell::new(pairs)));

        let json: serde_json::Value = serde_json::from_str(&payload_json(&value).unwrap()).unwrap();
        assert_eq!(json["aps"]["badge"], 7);
        assert!(json["aps"].get("alert").is_none(), "must not be rewritten");
    }
}
