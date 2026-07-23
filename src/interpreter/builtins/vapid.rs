//! VAPID (Voluntary Application Server Identification) for Web Push.
//!
//! Native Soli builtins that replace the `web-push` Node module:
//!   * `vapid_generate_keys()` — fresh P-256 key pair (base64url).
//!   * `vapid_sign(priv, aud, sub, exp?)` — ES256 JWT for the `Authorization: vapid` header.
//!   * `vapid_encrypt(payload, subscription, public_key, private_key)` — RFC 8291 aes128gcm
//!     payload encryption.
//!   * `vapid_send(subscription, payload, priv, pub, subject, options?)` — sign + encrypt + POST
//!     to the endpoint.
//!
//! Subscription shape (matches `PushSubscription.toJSON()` in the browser):
//! ```ignore
//! {"endpoint": "https://...", "keys": {"p256dh": "<b64url>", "auth": "<b64url>"}}
//! ```
//!
//! The encryption keys used for the push payload are always *ephemeral* —
//! generated fresh inside `vapid_encrypt` per RFC 8291 §3.4 — so the VAPID
//! identity keys are never reused for ECDH. The `public_key`/`private_key`
//! parameters on `vapid_encrypt` are accepted for API symmetry with
//! `vapid_send` but are intentionally not consumed by the encryption step.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes128Gcm, Nonce,
};
use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
use hkdf::Hkdf;
use p256::{
    ecdh::diffie_hellman,
    ecdsa::{signature::Signer, Signature, SigningKey},
    elliptic_curve::sec1::FromEncodedPoint,
    EncodedPoint, PublicKey, SecretKey,
};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};

/// Default record size (rs) for aes128gcm. 4096 is the canonical value
/// used by every browser push service; tightening it just wastes a byte
/// per record without affecting interop.
const RECORD_SIZE: u32 = 4096;

/// Default VAPID JWT lifetime when the caller doesn't pass `expiry_seconds`.
/// 12 h matches the upper end web-push libraries use; RFC 8292 caps it at 24 h.
const DEFAULT_EXP_SECONDS: u64 = 12 * 3600;

/// Default `TTL` push-service header when not overridden via the
/// `vapid_send` options. 60 s is a safe "deliver now or drop" value.
const DEFAULT_TTL_SECONDS: i64 = 60;

/// Register VAPID builtins in the given environment.
pub fn register_vapid_builtins(env: &mut Environment) {
    env.define(
        "vapid_generate_keys".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "vapid_generate_keys",
            Some(0),
            |_args| Ok(generate_keys()),
        )),
    );

    env.define(
        "vapid_sign".to_string(),
        Value::NativeFunction(NativeFunction::new("vapid_sign", None, |args| {
            if args.len() < 3 || args.len() > 4 {
                return Err(format!(
                    "vapid_sign() expects 3 or 4 arguments (private_key, audience, subject, expiry_seconds?), got {}",
                    args.len()
                ));
            }
            let private_key = arg_string(&args[0], "vapid_sign", "private_key")?;
            let audience = arg_string(&args[1], "vapid_sign", "audience")?;
            let subject = arg_string(&args[2], "vapid_sign", "subject")?;
            let exp = if args.len() == 4 {
                match &args[3] {
                    Value::Int(n) if *n > 0 => *n as u64,
                    Value::Int(n) => {
                        return Err(format!(
                            "vapid_sign() expiry_seconds must be positive, got {}",
                            n
                        ))
                    }
                    other => {
                        return Err(format!(
                            "vapid_sign() expects integer expiry_seconds, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                DEFAULT_EXP_SECONDS
            };
            sign_jwt(&private_key, &audience, &subject, exp).map(|s| Value::String(s.into()))
        })),
    );

    env.define(
        "vapid_encrypt".to_string(),
        Value::NativeFunction(NativeFunction::new("vapid_encrypt", None, |args| {
            // The trailing `private_key` / `public_key` args are accepted
            // to mirror the call sites in `vapid_send`, but per RFC 8291
            // the encryption uses fresh ephemeral keys generated inside
            // this function — VAPID identity keys are never reused for
            // ECDH.
            if !(2..=4).contains(&args.len()) {
                return Err(format!(
                    "vapid_encrypt() expects 2..=4 arguments (payload, subscription, public_key?, private_key?), got {}",
                    args.len()
                ));
            }
            let payload = arg_string(&args[0], "vapid_encrypt", "payload")?;
            let subscription = arg_subscription_keys(&args[1], "vapid_encrypt")?;
            encrypt_payload(payload.as_bytes(), &subscription.p256dh, &subscription.auth)
                .map(encrypted_to_hash)
        })),
    );

    env.define(
        "vapid_send".to_string(),
        Value::NativeFunction(NativeFunction::new("vapid_send", None, |args| {
            if !(5..=6).contains(&args.len()) {
                return Err(format!(
                    "vapid_send() expects 5 or 6 arguments (subscription, payload, private_key, public_key, subject, options?), got {}",
                    args.len()
                ));
            }
            let endpoint = arg_subscription_endpoint(&args[0], "vapid_send")?;
            let sub_keys = arg_subscription_keys(&args[0], "vapid_send")?;
            let payload = arg_string(&args[1], "vapid_send", "payload")?;
            let private_key = arg_string(&args[2], "vapid_send", "private_key")?;
            let public_key = arg_string(&args[3], "vapid_send", "public_key")?;
            let subject = arg_string(&args[4], "vapid_send", "subject")?;
            let options = if args.len() == 6 {
                Some(&args[5])
            } else {
                None
            };
            send_push(
                &endpoint,
                &sub_keys.p256dh,
                &sub_keys.auth,
                payload.as_bytes(),
                &private_key,
                &public_key,
                &subject,
                options,
            )
        })),
    );
}

/// Send a Web Push to a browser subscription, for callers composing transports
/// (see [`super::push`]) rather than calling `vapid_send` from Soli.
///
/// `subscription` is the `{endpoint, keys: {p256dh, auth}}` a browser produces;
/// `payload` is the already-serialized notification JSON.
pub fn send_to_subscription(
    subscription: &Value,
    payload: &str,
    private_key_b64: &str,
    public_key_b64: &str,
    subject: &str,
    options: Option<&Value>,
) -> Result<Value, String> {
    let endpoint = arg_subscription_endpoint(subscription, "Push")?;
    let keys = arg_subscription_keys(subscription, "Push")?;
    send_push(
        &endpoint,
        &keys.p256dh,
        &keys.auth,
        payload.as_bytes(),
        private_key_b64,
        public_key_b64,
        subject,
        options,
    )
}

// ---------- core crypto ---------------------------------------------------

fn generate_keys() -> Value {
    let secret = SecretKey::random(&mut OsRng);
    let public = secret.public_key();
    let public_b64 = URL_SAFE_NO_PAD.encode(public.to_sec1_bytes());
    let private_b64 = URL_SAFE_NO_PAD.encode(secret.to_bytes());

    let mut pairs: HashPairs = HashPairs::default();
    pairs.insert(
        HashKey::String("public_key".into()),
        Value::String(public_b64.into()),
    );
    pairs.insert(
        HashKey::String("private_key".into()),
        Value::String(private_b64.into()),
    );
    Value::Hash(Rc::new(RefCell::new(pairs)))
}

fn sign_jwt(
    private_key_b64: &str,
    audience: &str,
    subject: &str,
    exp_seconds: u64,
) -> Result<String, String> {
    let priv_bytes = decode_b64url(private_key_b64)
        .map_err(|e| format!("vapid_sign(): private_key is not valid base64url: {}", e))?;
    if priv_bytes.len() != 32 {
        return Err(format!(
            "vapid_sign(): private_key must decode to 32 bytes (P-256 scalar), got {}",
            priv_bytes.len()
        ));
    }

    // SEC: forbid relative `aud`. Browsers compute `aud` from the push
    // endpoint's origin, so it must be a real http(s) URL.
    if !audience.starts_with("http://") && !audience.starts_with("https://") {
        return Err(format!(
            "vapid_sign(): audience must be an http(s) origin, got '{}'",
            audience
        ));
    }

    let signing_key = SigningKey::from_bytes(priv_bytes.as_slice().into())
        .map_err(|e| format!("vapid_sign(): invalid P-256 private key: {}", e))?;

    let exp = current_timestamp().saturating_add(exp_seconds);

    // Build header / claims manually so we don't pull in another JWT
    // crate — they're tiny, fixed-shape JSON objects.
    let header_b64 = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = format!(
        r#"{{"aud":"{}","exp":{},"sub":"{}"}}"#,
        json_escape(audience),
        exp,
        json_escape(subject),
    );
    let claims_b64 = URL_SAFE_NO_PAD.encode(claims.as_bytes());

    let signing_input = format!("{}.{}", header_b64, claims_b64);
    let signature: Signature = signing_key.sign(signing_input.as_bytes());
    // ES256 = ECDSA P-256 / SHA-256 with the JWS "raw r||s" encoding.
    // `Signature::to_bytes` already gives that fixed-size 64-byte form.
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

    Ok(format!("{}.{}", signing_input, sig_b64))
}

struct EncryptedPayload {
    body: Vec<u8>,
    salt: [u8; 16],
    server_public: Vec<u8>,
}

fn encrypted_to_hash(enc: EncryptedPayload) -> Value {
    let mut pairs: HashPairs = HashPairs::default();
    // `ciphertext` is the full RFC 8291 body (salt || rs || idlen ||
    // keyid || encrypted-record). Callers POST this directly with
    // `Content-Encoding: aes128gcm`.
    pairs.insert(
        HashKey::String("ciphertext".into()),
        Value::String(URL_SAFE_NO_PAD.encode(&enc.body).into()),
    );
    pairs.insert(
        HashKey::String("salt".into()),
        Value::String(URL_SAFE_NO_PAD.encode(enc.salt).into()),
    );
    pairs.insert(
        HashKey::String("server_public_key".into()),
        Value::String(URL_SAFE_NO_PAD.encode(&enc.server_public).into()),
    );
    Value::Hash(Rc::new(RefCell::new(pairs)))
}

fn encrypt_payload(
    payload: &[u8],
    p256dh_b64: &str,
    auth_b64: &str,
) -> Result<EncryptedPayload, String> {
    let p256dh = decode_b64url(p256dh_b64).map_err(|e| {
        format!(
            "vapid_encrypt(): subscription p256dh is not valid base64url: {}",
            e
        )
    })?;
    let auth = decode_b64url(auth_b64).map_err(|e| {
        format!(
            "vapid_encrypt(): subscription auth is not valid base64url: {}",
            e
        )
    })?;

    if p256dh.len() != 65 {
        return Err(format!(
            "vapid_encrypt(): p256dh must be a 65-byte uncompressed P-256 point, got {}",
            p256dh.len()
        ));
    }

    let ua_point = EncodedPoint::from_bytes(&p256dh)
        .map_err(|e| format!("vapid_encrypt(): malformed p256dh point: {}", e))?;
    let ua_public_opt = PublicKey::from_encoded_point(&ua_point);
    if ua_public_opt.is_none().into() {
        return Err("vapid_encrypt(): p256dh is not a valid P-256 public key".to_string());
    }
    let ua_public = ua_public_opt.unwrap();

    // Ephemeral keypair for this push — generated fresh per RFC 8291.
    let as_secret = SecretKey::random(&mut OsRng);
    let as_public_bytes = as_secret.public_key().to_sec1_bytes().to_vec();
    if as_public_bytes.len() != 65 {
        return Err(format!(
            "vapid_encrypt(): ephemeral server key encoded to {} bytes, expected 65",
            as_public_bytes.len()
        ));
    }

    // Fresh salt per record (16 bytes).
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);

    // ECDH(server_priv, user_agent_pub).
    let shared = diffie_hellman(as_secret.to_nonzero_scalar(), ua_public.as_affine());
    let shared_secret = shared.raw_secret_bytes();

    // Step 1: IKM = HKDF-Expand(HKDF-Extract(auth_secret, ecdh_secret),
    //                           "WebPush: info\0" || ua_pub || as_pub,
    //                           32)
    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(&p256dh);
    key_info.extend_from_slice(&as_public_bytes);
    let ikm = hkdf_extract_expand(&auth, shared_secret.as_ref(), &key_info, 32)?;

    // Step 2: CEK + nonce keyed off the per-record salt.
    let cek = hkdf_extract_expand(&salt, &ikm, b"Content-Encoding: aes128gcm\0", 16)?;
    let nonce_bytes = hkdf_extract_expand(&salt, &ikm, b"Content-Encoding: nonce\0", 12)?;

    // Pad: append the 0x02 "last record" marker to the cleartext.
    let mut plaintext = Vec::with_capacity(payload.len() + 1);
    plaintext.extend_from_slice(payload);
    plaintext.push(0x02);

    let cipher = Aes128Gcm::new_from_slice(&cek)
        .map_err(|e| format!("vapid_encrypt(): AES-128-GCM key error: {}", e))?;
    #[allow(deprecated)]
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: &plaintext,
                aad: &[],
            },
        )
        .map_err(|e| format!("vapid_encrypt(): AES-128-GCM encrypt failed: {}", e))?;

    // Encrypted record must fit within rs after subtracting the 16-byte
    // GCM tag, otherwise the push service will reject it.
    if ciphertext.len() as u32 > RECORD_SIZE {
        return Err(format!(
            "vapid_encrypt(): encrypted record {} bytes exceeds record size {}",
            ciphertext.len(),
            RECORD_SIZE
        ));
    }

    // Assemble RFC 8291 body:
    //   salt(16) || rs(4 BE) || idlen(1) || keyid (as_public, 65) || encrypted-record
    let mut body = Vec::with_capacity(16 + 4 + 1 + 65 + ciphertext.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(as_public_bytes.len() as u8);
    body.extend_from_slice(&as_public_bytes);
    body.extend_from_slice(&ciphertext);

    Ok(EncryptedPayload {
        body,
        salt,
        server_public: as_public_bytes,
    })
}

#[allow(clippy::too_many_arguments)]
fn send_push(
    endpoint: &str,
    p256dh_b64: &str,
    auth_b64: &str,
    payload: &[u8],
    private_key_b64: &str,
    public_key_b64: &str,
    subject: &str,
    options: Option<&Value>,
) -> Result<Value, String> {
    let mut ttl = DEFAULT_TTL_SECONDS;
    let mut urgency: Option<String> = None;
    let mut topic: Option<String> = None;
    let mut exp_seconds = DEFAULT_EXP_SECONDS;
    if let Some(Value::Hash(opts)) = options {
        for (k, v) in opts.borrow().iter() {
            if let HashKey::String(key) = k {
                match key.as_ref() {
                    "ttl" => {
                        if let Value::Int(n) = v {
                            ttl = *n;
                        }
                    }
                    "urgency" => {
                        if let Value::String(s) = v {
                            urgency = Some(s.clone().to_string());
                        }
                    }
                    "topic" => {
                        if let Value::String(s) = v {
                            topic = Some(s.clone().to_string());
                        }
                    }
                    "expiry_seconds" => {
                        if let Value::Int(n) = v {
                            if *n > 0 {
                                exp_seconds = *n as u64;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let audience = origin_of(endpoint).ok_or_else(|| {
        format!(
            "vapid_send(): subscription endpoint must be an absolute http(s) URL, got '{}'",
            endpoint
        )
    })?;

    let jwt = sign_jwt(private_key_b64, &audience, subject, exp_seconds)?;
    let encrypted = encrypt_payload(payload, p256dh_b64, auth_b64)?;

    // SEC: avoid an obviously invalid public_key value silently
    // producing a 401 from the push service. Decode it up-front so the
    // error surfaces here.
    let pub_bytes = decode_b64url(public_key_b64)
        .map_err(|e| format!("vapid_send(): public_key is not valid base64url: {}", e))?;
    if pub_bytes.len() != 65 {
        return Err(format!(
            "vapid_send(): public_key must decode to 65 bytes (uncompressed P-256 point), got {}",
            pub_bytes.len()
        ));
    }

    let authorization = format!("vapid t={}, k={}", jwt, public_key_b64);

    let client = reqwest::blocking::Client::new();
    let mut req = client
        .post(endpoint)
        .header("Authorization", authorization)
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("TTL", ttl.to_string())
        .body(encrypted.body);
    if let Some(u) = urgency {
        req = req.header("Urgency", u);
    }
    if let Some(t) = topic {
        req = req.header("Topic", t);
    }

    let resp = req
        .send()
        .map_err(|e| format!("vapid_send(): HTTP request failed: {}", e))?;

    let status = resp.status().as_u16() as i64;
    let body = resp.text().unwrap_or_default();

    let mut pairs: HashPairs = HashPairs::default();
    pairs.insert(HashKey::String("status".into()), Value::Int(status));
    pairs.insert(HashKey::String("body".into()), Value::String(body.into()));
    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
}

// ---------- helpers --------------------------------------------------------

fn arg_string(value: &Value, fn_name: &str, arg_name: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone().to_string()),
        other => Err(format!(
            "{}(): expects string {}, got {}",
            fn_name,
            arg_name,
            other.type_name()
        )),
    }
}

struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

fn arg_subscription_endpoint(value: &Value, fn_name: &str) -> Result<String, String> {
    match value {
        Value::Hash(hash) => {
            for (k, v) in hash.borrow().iter() {
                if let HashKey::String(key) = k {
                    if **key == *"endpoint" {
                        if let Value::String(s) = v {
                            return Ok(s.clone().to_string());
                        }
                    }
                }
            }
            Err(format!(
                "{}(): subscription is missing string 'endpoint'",
                fn_name
            ))
        }
        other => Err(format!(
            "{}(): expects subscription hash, got {}",
            fn_name,
            other.type_name()
        )),
    }
}

fn arg_subscription_keys(value: &Value, fn_name: &str) -> Result<SubscriptionKeys, String> {
    let hash = match value {
        Value::Hash(h) => h.clone(),
        other => {
            return Err(format!(
                "{}(): expects subscription hash, got {}",
                fn_name,
                other.type_name()
            ))
        }
    };
    let borrow = hash.borrow();
    let keys = borrow.get(&HashKey::String("keys".into())).ok_or_else(|| {
        format!(
            "{}(): subscription is missing 'keys' hash with p256dh/auth",
            fn_name
        )
    })?;
    let keys_hash = match keys {
        Value::Hash(h) => h.borrow(),
        other => {
            return Err(format!(
                "{}(): subscription 'keys' must be a hash, got {}",
                fn_name,
                other.type_name()
            ))
        }
    };
    let p256dh = match keys_hash.get(&HashKey::String("p256dh".into())) {
        Some(Value::String(s)) => s.clone(),
        _ => {
            return Err(format!(
                "{}(): subscription keys.p256dh missing or not a string",
                fn_name
            ))
        }
    };
    let auth = match keys_hash.get(&HashKey::String("auth".into())) {
        Some(Value::String(s)) => s.clone(),
        _ => {
            return Err(format!(
                "{}(): subscription keys.auth missing or not a string",
                fn_name
            ))
        }
    };
    Ok(SubscriptionKeys {
        p256dh: p256dh.to_string(),
        auth: auth.to_string(),
    })
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Decode a base64url string, tolerating optional padding. Browser
/// subscription payloads sometimes carry `=` padding and sometimes don't.
fn decode_b64url(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let trimmed = input.trim();
    if trimmed.contains('=') {
        URL_SAFE.decode(trimmed)
    } else {
        URL_SAFE_NO_PAD.decode(trimmed)
    }
}

fn hkdf_extract_expand(
    salt: &[u8],
    ikm: &[u8],
    info: &[u8],
    out_len: usize,
) -> Result<Vec<u8>, String> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut out = vec![0u8; out_len];
    hk.expand(info, &mut out)
        .map_err(|e| format!("HKDF expand failed ({} bytes requested): {}", out_len, e))?;
    Ok(out)
}

/// Extract the `scheme://host[:port]` origin from a URL — VAPID `aud` per
/// RFC 8292 §2. Returns `None` if the URL doesn't parse or lacks a scheme/host.
fn origin_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    if let Some(port) = parsed.port() {
        Some(format!("{}://{}:{}", scheme, host, port))
    } else {
        Some(format!("{}://{}", scheme, host))
    }
}

/// Minimal JSON string escaper for the JWT claims we control. We never
/// pass attacker-controlled content into `aud`, `sub`, or `exp`, but
/// escape `"` and `\\` defensively so a mailto/URL with an embedded quote
/// can't break the JSON envelope.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_env() -> Environment {
        let mut env = Environment::new();
        register_vapid_builtins(&mut env);
        env
    }

    fn vapid_fn(env: &Environment, name: &str) -> NativeFunction {
        match env.get(name) {
            Some(Value::NativeFunction(f)) => f.clone(),
            other => panic!("expected NativeFunction for {name}, got {other:?}"),
        }
    }

    fn hash_get<'a>(hash: &'a HashPairs, key: &str) -> Option<&'a Value> {
        hash.get(&HashKey::String(key.to_string().into()))
    }

    #[test]
    fn vapid_generate_keys_returns_b64url_pair() {
        let env = fresh_env();
        let gen = vapid_fn(&env, "vapid_generate_keys");
        let result = (gen.func)(vec![]).unwrap();
        let hash = match result {
            Value::Hash(h) => h,
            other => panic!("expected hash, got {other:?}"),
        };
        let hash = hash.borrow();
        let pub_b64 = match hash_get(&hash, "public_key") {
            Some(Value::String(s)) => s.clone(),
            other => panic!("expected public_key string, got {other:?}"),
        };
        let priv_b64 = match hash_get(&hash, "private_key") {
            Some(Value::String(s)) => s.clone(),
            other => panic!("expected private_key string, got {other:?}"),
        };
        // 32-byte scalar → 43 chars, 65-byte point → 87 chars (no padding).
        assert_eq!(URL_SAFE_NO_PAD.decode(&*priv_b64).unwrap().len(), 32);
        assert_eq!(URL_SAFE_NO_PAD.decode(&*pub_b64).unwrap().len(), 65);
    }

    #[test]
    fn vapid_sign_produces_three_segment_jwt() {
        let env = fresh_env();
        let gen = vapid_fn(&env, "vapid_generate_keys");
        let sign = vapid_fn(&env, "vapid_sign");

        let keys = (gen.func)(vec![]).unwrap();
        let priv_key = match keys {
            Value::Hash(h) => match hash_get(&h.borrow(), "private_key") {
                Some(Value::String(s)) => s.clone(),
                other => panic!("expected priv string, got {other:?}"),
            },
            other => panic!("expected hash, got {other:?}"),
        };
        let token = (sign.func)(vec![
            Value::String(priv_key),
            Value::String("https://fcm.googleapis.com".into()),
            Value::String("mailto:dev@example.com".into()),
        ])
        .unwrap();
        let token_str = match token {
            Value::String(s) => s,
            other => panic!("expected token string, got {other:?}"),
        };
        // ES256 JWT: header.payload.signature — exactly two dots.
        assert_eq!(token_str.matches('.').count(), 2);
        for segment in token_str.split('.') {
            URL_SAFE_NO_PAD
                .decode(segment)
                .expect("each JWT segment must be valid base64url");
        }
    }

    #[test]
    fn vapid_sign_rejects_relative_audience() {
        let env = fresh_env();
        let gen = vapid_fn(&env, "vapid_generate_keys");
        let sign = vapid_fn(&env, "vapid_sign");
        let keys = (gen.func)(vec![]).unwrap();
        let priv_key = match keys {
            Value::Hash(h) => match hash_get(&h.borrow(), "private_key") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("private_key missing"),
            },
            _ => panic!("hash missing"),
        };
        let err = (sign.func)(vec![
            Value::String(priv_key),
            Value::String("fcm.googleapis.com".into()),
            Value::String("mailto:dev@example.com".into()),
        ])
        .unwrap_err();
        assert!(err.contains("audience"), "got: {err}");
    }

    #[test]
    fn vapid_encrypt_round_trips_against_generated_subscription() {
        let env = fresh_env();
        let gen = vapid_fn(&env, "vapid_generate_keys");
        let encrypt = vapid_fn(&env, "vapid_encrypt");

        // Fake a subscriber by generating a P-256 keypair: their p256dh
        // is the uncompressed public point, auth is a 16-byte secret.
        let ua_secret = SecretKey::random(&mut OsRng);
        let ua_pub_b64 = URL_SAFE_NO_PAD.encode(ua_secret.public_key().to_sec1_bytes());
        let mut auth = [0u8; 16];
        OsRng.fill_bytes(&mut auth);
        let auth_b64 = URL_SAFE_NO_PAD.encode(auth);

        let mut sub_keys: HashPairs = HashPairs::default();
        sub_keys.insert(
            HashKey::String("p256dh".into()),
            Value::String(ua_pub_b64.into()),
        );
        sub_keys.insert(
            HashKey::String("auth".into()),
            Value::String(auth_b64.into()),
        );
        let mut sub: HashPairs = HashPairs::default();
        sub.insert(
            HashKey::String("endpoint".into()),
            Value::String("https://example.invalid/push/abc".into()),
        );
        sub.insert(
            HashKey::String("keys".into()),
            Value::Hash(Rc::new(RefCell::new(sub_keys))),
        );

        // Generate VAPID keys just to fill the trailing args.
        let vapid_keys = (gen.func)(vec![]).unwrap();
        let (pub_key, priv_key) = match vapid_keys {
            Value::Hash(h) => {
                let h = h.borrow();
                let p = match hash_get(&h, "public_key") {
                    Some(Value::String(s)) => s.clone(),
                    _ => panic!(),
                };
                let s = match hash_get(&h, "private_key") {
                    Some(Value::String(s)) => s.clone(),
                    _ => panic!(),
                };
                (p, s)
            }
            _ => panic!(),
        };

        let result = (encrypt.func)(vec![
            Value::String("{\"title\":\"Hi\"}".into()),
            Value::Hash(Rc::new(RefCell::new(sub))),
            Value::String(pub_key),
            Value::String(priv_key),
        ])
        .unwrap();
        let hash = match result {
            Value::Hash(h) => h,
            other => panic!("expected hash, got {other:?}"),
        };
        let hash = hash.borrow();
        let ciphertext = match hash_get(&hash, "ciphertext") {
            Some(Value::String(s)) => s.clone(),
            other => panic!("expected ciphertext string, got {other:?}"),
        };
        let salt = match hash_get(&hash, "salt") {
            Some(Value::String(s)) => s.clone(),
            other => panic!("expected salt string, got {other:?}"),
        };
        let server_pub = match hash_get(&hash, "server_public_key") {
            Some(Value::String(s)) => s.clone(),
            other => panic!("expected server_public_key string, got {other:?}"),
        };
        assert!(!ciphertext.is_empty());
        assert_eq!(URL_SAFE_NO_PAD.decode(&*salt).unwrap().len(), 16);
        assert_eq!(URL_SAFE_NO_PAD.decode(&*server_pub).unwrap().len(), 65);
    }

    #[test]
    fn origin_of_strips_path_and_query() {
        assert_eq!(
            origin_of("https://fcm.googleapis.com/fcm/send/abc?x=1").as_deref(),
            Some("https://fcm.googleapis.com"),
        );
        assert_eq!(
            origin_of("https://push.example.com:8443/p").as_deref(),
            Some("https://push.example.com:8443"),
        );
        assert!(origin_of("file:///etc/passwd").is_none());
    }
}
