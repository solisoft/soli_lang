//! Firebase Cloud Messaging — push to a closed Android app.
//!
//! The counterpart to [`super::apns`] on the other platform, and the only way
//! to reach an Android app that is not running: the OS kills long-lived
//! connections within minutes of the screen going off, which is precisely why
//! FCM exists.
//!
//! ```soli
//! Fcm.send(device_token, { "title": "New ping", "body": "Ana replied" }, {
//!   "service_account": File.read("service-account.json")
//! })
//! ```
//!
//! # Why this is more involved than APNs
//!
//! APNs authenticates with the request itself — one ES256 JWT per call. FCM's
//! HTTP v1 API takes an **OAuth2 access token**, which means a second round
//! trip: sign a service-account assertion (RS256), exchange it at Google's
//! token endpoint, then send. Access tokens last an hour, so they are cached
//! per service account and the exchange happens roughly once.
//!
//! The legacy `key=AAAA...` server-key API needed none of this and is not an
//! option: Google shut it down in 2024.
//!
//! # Data values are strings
//!
//! FCM rejects a `data` payload whose values are not strings — numbers and
//! booleans included. Rather than fail a send over a `{"count": 3}`, non-string
//! values are stringified here, which is what every FCM client library does.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

/// The scope an access token must carry to send a message.
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// Access tokens are valid for an hour; refresh a little early so a send never
/// races the expiry.
const TOKEN_REUSE_SECS: u64 = 55 * 60;

/// The assertion's own lifetime. Google caps it at an hour.
const ASSERTION_TTL_SECS: u64 = 3600;

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The fields of a service-account JSON file this needs. Everything else in
/// that file (`type`, `client_id`, the cert URLs) is irrelevant to sending.
#[derive(Deserialize, Debug)]
struct ServiceAccount {
    client_email: String,
    private_key: String,
    project_id: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

#[derive(Serialize)]
struct Assertion<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: u64,
    iat: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Cached access tokens, keyed by service-account email, with the time each was
/// obtained.
type TokenCache = Mutex<HashMap<String, (String, u64)>>;

fn token_cache() -> &'static TokenCache {
    static CACHE: OnceLock<TokenCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse_service_account(json: &str) -> Result<ServiceAccount, String> {
    serde_json::from_str::<ServiceAccount>(json).map_err(|e| {
        format!(
            "Fcm: service_account is not a valid service-account JSON ({}). \
             Pass the contents of the file downloaded from Firebase console → \
             Project settings → Service accounts.",
            e
        )
    })
}

/// Sign the assertion and exchange it for an access token.
///
/// Exposed so `Fcm.access_token` can hand it to a caller driving the HTTP
/// themselves, and so the signing half is testable without the network.
pub fn build_assertion(account: &ServiceAccountView<'_>) -> Result<String, String> {
    let now = now_unix_secs();
    let claims = Assertion {
        iss: account.client_email,
        scope: SCOPE,
        aud: account.token_uri,
        exp: now.saturating_add(ASSERTION_TTL_SECS),
        iat: now,
    };
    let key = EncodingKey::from_rsa_pem(account.private_key.as_bytes()).map_err(|e| {
        format!(
            "Fcm: the service account's private_key is not a valid RSA PEM ({}). \
             It is the `private_key` field of the JSON, newlines included.",
            e
        )
    })?;
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|e| format!("Fcm: could not sign the service-account assertion: {}", e))
}

/// The borrowed view of a service account the signing step needs.
pub struct ServiceAccountView<'a> {
    pub client_email: &'a str,
    pub private_key: &'a str,
    pub token_uri: &'a str,
}

fn access_token(account: &ServiceAccount) -> Result<String, String> {
    let now = now_unix_secs();
    if let Ok(cache) = token_cache().lock() {
        if let Some((token, obtained)) = cache.get(&account.client_email) {
            if now.saturating_sub(*obtained) < TOKEN_REUSE_SECS {
                return Ok(token.clone());
            }
        }
    }

    let assertion = build_assertion(&ServiceAccountView {
        client_email: &account.client_email,
        private_key: &account.private_key,
        token_uri: &account.token_uri,
    })?;

    let client = reqwest::blocking::Client::new();
    let response = client
        .post(&account.token_uri)
        .form(&[
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:jwt-bearer".to_string(),
            ),
            ("assertion", assertion),
        ])
        .send()
        .map_err(|e| format!("Fcm: token exchange failed: {}", e))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        // Google's error body names the cause ("invalid_grant" for a clock skew
        // or a revoked key), so pass it through rather than flattening it.
        return Err(format!(
            "Fcm: token exchange refused with {} — {}",
            status.as_u16(),
            body.trim()
        ));
    }

    let token: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Fcm: token response was not the expected JSON: {}", e))?;

    if let Ok(mut cache) = token_cache().lock() {
        cache.insert(
            account.client_email.clone(),
            (token.access_token.clone(), now),
        );
    }
    Ok(token.access_token)
}

/// Build the `message` object FCM v1 expects.
///
/// `title`/`body` become a `notification` (which is what makes the OS display
/// it without the app running); everything else becomes `data`, stringified,
/// for the app to read on tap.
pub fn message_json(device_token: &str, payload: &Value) -> Result<serde_json::Value, String> {
    let mut json = crate::interpreter::value::value_to_json(payload)
        .map_err(|e| format!("Fcm.send(): payload is not serializable: {}", e))?;

    // A caller who already speaks FCM can hand over a full message body.
    if json.get("message").is_some() {
        return Ok(json);
    }

    let object = json
        .as_object_mut()
        .ok_or_else(|| "Fcm.send(): payload must be a hash".to_string())?;

    let title = object.remove("title");
    let body = object.remove("body");

    let mut notification = serde_json::Map::new();
    if let Some(title) = title {
        notification.insert("title".to_string(), title);
    }
    if let Some(body) = body {
        notification.insert("body".to_string(), body);
    }

    // FCM refuses non-string data values outright, so stringify rather than let
    // a `{"count": 3}` fail the whole send.
    let mut data = serde_json::Map::new();
    for (key, value) in object.iter() {
        let as_string = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        data.insert(key.clone(), serde_json::Value::String(as_string));
    }

    let mut message = serde_json::Map::new();
    message.insert(
        "token".to_string(),
        serde_json::Value::String(device_token.to_string()),
    );
    if !notification.is_empty() {
        message.insert(
            "notification".to_string(),
            serde_json::Value::Object(notification),
        );
    }
    if !data.is_empty() {
        message.insert("data".to_string(), serde_json::Value::Object(data));
    }
    // High priority: a notification the user should see now is exactly the case
    // Doze would otherwise defer.
    message.insert(
        "android".to_string(),
        serde_json::json!({ "priority": "high" }),
    );

    Ok(serde_json::json!({ "message": message }))
}

fn send(device_token: &str, payload: &Value, options: &Value) -> Result<Value, String> {
    if device_token.is_empty() || device_token.len() > 512 {
        return Err("Fcm.send(): implausible device token".to_string());
    }

    let Value::Hash(hash) = options else {
        return Err("Fcm.send(): options must be a hash".to_string());
    };
    let service_account_json = match hash
        .borrow()
        .get(&HashKey::String("service_account".into()))
    {
        Some(Value::String(s)) => s.to_string(),
        _ => {
            return Err(
                "Fcm.send(): options must include 'service_account' — the contents of the \
                 service-account JSON from the Firebase console."
                    .to_string(),
            )
        }
    };

    let account = parse_service_account(&service_account_json)?;
    let token = access_token(&account)?;
    let message = message_json(device_token, payload)?;

    let url = format!(
        "https://fcm.googleapis.com/v1/projects/{}/messages:send",
        account.project_id
    );

    let client = reqwest::blocking::Client::new();
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&message)
        .send()
        .map_err(|e| format!("Fcm.send(): HTTP request failed: {}", e))?;

    let status = response.status().as_u16();
    let text = response.text().unwrap_or_default();

    // Success is a `{"name": "projects/.../messages/..."}`; failure carries
    // `error.status`, e.g. UNREGISTERED for a token to prune.
    let reason = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("status"))
                .and_then(|s| s.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_default();

    let mut result = HashPairs::default();
    result.insert(HashKey::String("status".into()), Value::Int(status as i64));
    result.insert(
        HashKey::String("reason".into()),
        Value::String(reason.into()),
    );
    Ok(Value::Hash(Rc::new(std::cell::RefCell::new(result))))
}

pub fn register_fcm_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    statics.insert(
        "send".to_string(),
        Rc::new(NativeFunction::new("Fcm.send", Some(3), |args| {
            let device_token = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "Fcm.send() expects a string device token, got {}",
                        other.type_name()
                    ))
                }
            };
            send(&device_token, &args[1], &args[2])
        })),
    );

    // Fcm.access_token(service_account_json) — for a caller driving the HTTP
    // itself, or batching.
    statics.insert(
        "access_token".to_string(),
        Rc::new(NativeFunction::new("Fcm.access_token", Some(1), |args| {
            let json = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "Fcm.access_token() expects the service-account JSON as a string, got {}",
                        other.type_name()
                    ))
                }
            };
            let account = parse_service_account(&json)?;
            access_token(&account).map(|t| Value::String(t.into()))
        })),
    );

    let class = Rc::new(Class {
        name: "Fcm".to_string(),
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

    env.define("Fcm".to_string(), Value::Class(class));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway 2048-bit RSA key, in the PEM shape a service-account file
    /// carries. Fixed rather than generated: key generation costs a second per
    /// run, and this signs nothing that exists.
    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC1R27oZql8YZcY\nOOM+f2rSHE/0Zxyn00zLORVpMerlkjq5o2yDRjI7CCRL7R4Pk+zIOJfQycW5NjPC\nbQQTAS/AMrk2cvR43LZftnpWuFvXooWxFF/q/V7O1I5gR9YOpfB86LZIG53sjc1O\ngY0JXaQ7BRRnseJjy0TY01FyGWNzrey5hGZiqyp5zMImxkTIgKff79ECN4C8KcPr\nW3o8/n3q6GpfaiWk4aoCqHNOEJa/RmURUPOsblnMxVcukFPh5E3lTQUs4UhKiYON\nYhddISRJOrDbdqvdwxdbJrdn5/gmNJ3kxX9j+U9HOrg16vHs0G5eMPECD6uXWSZZ\n7yhTuAT9AgMBAAECggEAAPBnSy3o99t7kGUPDE5Vq9P4uA/mrpniHnVjvoFDMcBB\nrJ+DmuR8syCWK4yFaNg/1EOyyaKZ8x0sRHgTGtQZProkinsq+AtQqItF8/gNhb7y\nSVoNKyb33ENun2IP4lCKv+LmutXlyglsBFmYdPF2vdWHZwgdX534c9UYaWpnQPyl\n7sjrLqcJLa+0SGCkX+tzvupDOn07e+mKEZOXLHnSHndKSY3RlXMvMopOIh05D9Dh\nfcGf+cfY2ps55YyyDk5UGTDxE36HVGY0IDxoTcQuhSme2CtXrzEM1+3NlQRlinyO\npMvYO/uMUiqF4bFxVAWyvVUBQ0qNU1APrzA23ez2EwKBgQDhwDukG79sJWzSFgJX\nHsI0mQSCQC/Hyse4zUNFTtCyaR+iCswpNrNpLtZX4OZkRJUH2y/d0sYP3eZ+bAFo\nhBpYO3/V4x2DvOE74El+Lu8uFOiRkv/McCufRNE7o0EtE5biw0CJHo5QReyey2jt\nB+zOPH03LxJ2XjvpGLJcFhwm7wKBgQDNkbbgAiNLK14TcbesZB/5mRq0XWTeKMm9\nXPANq3MYlLQ7cNBnTY3oDqJ4nMdIscghib3AtrpI1k2wlmB3PdQ2HqYmSa+Ki79C\ncYBAEk5mTFgmOl+bibEUkYK32cfteFbAhRAPqX5fXKmuLY/z9FPIEtBO5xxpu0X1\nzxNIjary0wKBgQCI8sH7iy2z4HxEcj+XNDyiBdW7Yk7aCATi8fqGOArYwHcFKUGz\nGtD51QUIqJF7cDNsYaaHDc9DXtzuAn1UNxd4QRgK281S1qlYVnafCr/kF6ECdseg\n8Mc1xlybriziuIiHJeWniRbSUaj6p/EOIgmhDwbzDCZKEl6LyISi4nLPlwKBgGjo\n4nFz5dso6Lv3nwsFliPldPFzcFTIcByJ36C6TOTQjyJ+snzl4XP6dAQlzrZUtJQZ\nHZPKLUuaws9KDzULgs+T2KtVk5abNyKLli4cqZIfiCUKSVyxaoPatuFo7VVNwshB\noC6+C1ZTjezsJ7kSiedjYpfB7ogvIMcPxQGT+xgtAoGAdvUhXldR7Tank0JGmxZA\n5SnXQWxV8n7mm+lhFDYFRB1h+olsSyDUiN7bI/sYCd9mU3LSwUBd35vPqrDfY1dL\nTMFZlssEVniUrhGm2oFG+SZWavx0oGXU5XSiiV3FlqYS24je9FGrpuxvNTsXQrrB\nYtDD73tj5Wxha8PGiBK3AGA=\n-----END PRIVATE KEY-----";

    fn test_private_key_pem() -> String {
        TEST_PRIVATE_KEY_PEM.to_string()
    }

    fn service_account_json(pem: &str) -> String {
        serde_json::json!({
            "type": "service_account",
            "project_id": "demo-project",
            "client_email": "sender@demo-project.iam.gserviceaccount.com",
            "private_key": pem,
        })
        .to_string()
    }

    #[test]
    fn a_service_account_file_parses_to_what_sending_needs() {
        let account = parse_service_account(&service_account_json("PEM")).expect("parses");
        assert_eq!(account.project_id, "demo-project");
        assert_eq!(
            account.client_email,
            "sender@demo-project.iam.gserviceaccount.com"
        );
        // Defaulted, because real files carry it but nothing should break if one does not.
        assert_eq!(account.token_uri, "https://oauth2.googleapis.com/token");
    }

    #[test]
    fn a_file_that_is_not_a_service_account_is_rejected_with_advice() {
        let err = parse_service_account("{}").expect_err("rejects");
        assert!(
            err.contains("Firebase console"),
            "error should say where to get the file: {}",
            err
        );
    }

    /// The assertion is what Google exchanges for an access token; the scope and
    /// audience are what make it valid for FCM specifically.
    #[test]
    fn the_assertion_carries_the_messaging_scope_and_token_audience() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;

        let pem = test_private_key_pem();
        let assertion = build_assertion(&ServiceAccountView {
            client_email: "sender@demo.iam.gserviceaccount.com",
            private_key: &pem,
            token_uri: "https://oauth2.googleapis.com/token",
        })
        .expect("signs");

        let parts: Vec<&str> = assertion.split('.').collect();
        assert_eq!(parts.len(), 3);

        let header = String::from_utf8(URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        assert!(header.contains("RS256"), "header: {}", header);

        let claims = String::from_utf8(URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert!(claims.contains(SCOPE), "claims: {}", claims);
        assert!(
            claims.contains("https://oauth2.googleapis.com/token"),
            "claims: {}",
            claims
        );
    }

    #[test]
    fn title_and_body_become_a_notification_and_the_rest_becomes_data() {
        let mut pairs = HashPairs::default();
        pairs.insert(HashKey::String("title".into()), Value::String("Hi".into()));
        pairs.insert(
            HashKey::String("body".into()),
            Value::String("There".into()),
        );
        pairs.insert(HashKey::String("url".into()), Value::String("/x".into()));
        let value = Value::Hash(Rc::new(std::cell::RefCell::new(pairs)));

        let message = message_json("device-abc", &value).expect("builds");
        assert_eq!(message["message"]["token"], "device-abc");
        assert_eq!(message["message"]["notification"]["title"], "Hi");
        assert_eq!(message["message"]["notification"]["body"], "There");
        assert_eq!(message["message"]["data"]["url"], "/x");
        // Doze defers normal-priority messages, which is wrong for something the
        // user is meant to see now.
        assert_eq!(message["message"]["android"]["priority"], "high");
    }

    /// FCM rejects non-string data values outright, so a `{"count": 3}` must not
    /// be allowed to fail the whole send.
    #[test]
    fn data_values_are_stringified() {
        let mut pairs = HashPairs::default();
        pairs.insert(HashKey::String("count".into()), Value::Int(3));
        pairs.insert(HashKey::String("urgent".into()), Value::Bool(true));
        let value = Value::Hash(Rc::new(std::cell::RefCell::new(pairs)));

        let message = message_json("device-abc", &value).expect("builds");
        assert_eq!(message["message"]["data"]["count"], "3");
        assert_eq!(message["message"]["data"]["urgent"], "true");
    }

    #[test]
    fn an_explicit_message_body_is_passed_through() {
        let mut inner = HashPairs::default();
        inner.insert(
            HashKey::String("topic".into()),
            Value::String("news".into()),
        );
        let mut pairs = HashPairs::default();
        pairs.insert(
            HashKey::String("message".into()),
            Value::Hash(Rc::new(std::cell::RefCell::new(inner))),
        );
        let value = Value::Hash(Rc::new(std::cell::RefCell::new(pairs)));

        let message = message_json("device-abc", &value).expect("builds");
        assert_eq!(message["message"]["topic"], "news");
        assert!(
            message["message"].get("notification").is_none(),
            "must not be rewritten"
        );
    }
}
