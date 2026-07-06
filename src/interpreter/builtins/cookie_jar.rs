//! Signed and encrypted cookie jar (Rails-style `cookies.signed` /
//! `cookies.encrypted`, expressed through `set_cookie` options and
//! `read_cookie`).
//!
//! Not to be confused with `secure_cookies.rs`, which is the SEC-028
//! force-`Secure`-attribute gate and has nothing to do with value sealing.
//!
//! Wire formats (both base64url, no padding):
//!
//! - encrypted: `enc.v1.<b64( nonce[12] ‖ AES-256-GCM(payload, aad) )>`
//! - signed:    `sig.v1.<b64(payload)>.<b64( HMAC-SHA256(mac_input) )>`
//!
//! where payload is `{"iat": secs, "exp": secs?, "val": <json>}` — `exp`
//! mirrors the `max_age` cookie option so a captured cookie can't be replayed
//! past its intended lifetime (browser expiry is advisory). The cookie NAME is
//! bound into the GCM AAD / MAC input, so a validly-sealed value for cookie A
//! reads as absent when replayed under cookie B (purpose binding).
//!
//! Keys are HKDF-SHA256-derived from `SOLI_SESSION_SECRET` with info labels
//! distinct from each other and from the cookie session driver's
//! (`soli.session.cookie.v1`) — the three formats can rotate independently.
//!
//! Failure semantics mirror the cookie session driver: configuration problems
//! (missing/short secret) raise so the operator hears about them; anything an
//! attacker controls (tamper, expiry, wrong name, garbage) makes `read_cookie`
//! return `null`, indistinguishable from an absent cookie.

use std::cell::RefCell;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::Sha256;

use super::crypto::{
    aes_decrypt_bytes_aad, aes_encrypt_bytes_aad, do_secure_compare, hmac_sha256_bytes,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value_ref, HashKey, NativeFunction, Value};

const ENC_PREFIX: &str = "enc.v1.";
const SIG_PREFIX: &str = "sig.v1.";
/// HKDF info labels — the domain separation that keeps the two jar keys apart
/// from each other and from the session driver's `soli.session.cookie.v1`.
const ENC_INFO: &[u8] = b"soli.cookie.encrypted.v1";
const SIG_INFO: &[u8] = b"soli.cookie.signed.v1";
/// Same floor as the cookie session driver: a short secret caps the derived
/// key's entropy no matter how strong the KDF is.
const MIN_SECRET_LEN: usize = 32;
/// Same ceiling as the cookie session driver (~4096-byte Set-Cookie line,
/// minus headroom for name + attributes). Oversize raises at seal time.
const MAX_SEALED_LEN: usize = 4000;

/// What travels inside a sealed cookie value.
#[derive(Serialize, Deserialize)]
struct JarPayload {
    iat: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    exp: Option<u64>,
    val: JsonValue,
}

/// How `set_cookie` should treat the value, parsed from its options hash.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SealMode {
    Plain,
    Encrypted,
    Signed,
}

#[derive(Clone)]
pub(crate) struct CookieJar {
    enc_key: [u8; 32],
    sig_key: [u8; 32],
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The purpose-binding bytes: `info ‖ NUL ‖ cookie name`. NUL is unambiguous —
/// `validate_cookie_pair` forbids control characters in names.
fn purpose_aad(info: &[u8], name: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(info.len() + 1 + name.len());
    aad.extend_from_slice(info);
    aad.push(0);
    aad.extend_from_slice(name.as_bytes());
    aad
}

/// MAC input for the signed format: `info ‖ NUL ‖ name ‖ NUL ‖ b64(payload)`.
/// The MAC covers the *encoded* payload, so verification needs no decode.
fn mac_input(name: &str, payload_b64: &str) -> Vec<u8> {
    let mut input = purpose_aad(SIG_INFO, name);
    input.push(0);
    input.extend_from_slice(payload_b64.as_bytes());
    input
}

/// Bounds the length and alphabet of attacker-controlled input before it
/// reaches base64/crypto — same guard the cookie session driver applies.
fn is_plausible_sealed(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SEALED_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn check_sealed_len(sealed: &str) -> Result<(), String> {
    if sealed.len() > MAX_SEALED_LEN {
        return Err(format!(
            "set_cookie() sealed value exceeds the ~4KB cookie limit ({} bytes); \
             store less in the cookie or keep the data server-side (session)",
            sealed.len()
        ));
    }
    Ok(())
}

impl CookieJar {
    /// Derive both jar keys from the operator secret. Rejects short secrets
    /// outright rather than degrade — sealed cookies carry trusted values, so
    /// a guessable key equals forgery.
    pub(crate) fn new(secret: &str) -> Result<Self, String> {
        if secret.len() < MIN_SECRET_LEN {
            return Err(format!(
                "signed/encrypted cookies require a session secret of at least {} characters (got {}); \
                 set SOLI_SESSION_SECRET or call session_configure({{\"secret\": ...}})",
                MIN_SECRET_LEN,
                secret.len()
            ));
        }
        let hk = Hkdf::<Sha256>::new(None, secret.as_bytes());
        let mut enc_key = [0u8; 32];
        hk.expand(ENC_INFO, &mut enc_key)
            .map_err(|e| format!("cookie key derivation failed: {}", e))?;
        let mut sig_key = [0u8; 32];
        hk.expand(SIG_INFO, &mut sig_key)
            .map_err(|e| format!("cookie key derivation failed: {}", e))?;
        Ok(Self { enc_key, sig_key })
    }

    fn payload_bytes(val: &JsonValue, max_age: Option<u64>) -> Result<Vec<u8>, String> {
        let now = now_unix_secs();
        let payload = JarPayload {
            iat: now,
            exp: max_age.map(|m| now.saturating_add(m)),
            val: val.clone(),
        };
        serde_json::to_vec(&payload).map_err(|e| format!("cookie serialize failed: {}", e))
    }

    fn open_payload(bytes: &[u8]) -> Result<JsonValue, String> {
        let payload: JarPayload =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid cookie payload: {}", e))?;
        if let Some(exp) = payload.exp {
            if now_unix_secs() > exp {
                return Err("cookie expired".to_string());
            }
        }
        Ok(payload.val)
    }

    pub(crate) fn seal_encrypted(
        &self,
        name: &str,
        val: &JsonValue,
        max_age: Option<u64>,
    ) -> Result<String, String> {
        let plaintext = Self::payload_bytes(val, max_age)?;
        let sealed = format!(
            "{}{}",
            ENC_PREFIX,
            URL_SAFE_NO_PAD.encode(aes_encrypt_bytes_aad(
                &plaintext,
                &self.enc_key,
                &purpose_aad(ENC_INFO, name),
            )?)
        );
        check_sealed_len(&sealed)?;
        Ok(sealed)
    }

    pub(crate) fn open_encrypted(&self, name: &str, sealed: &str) -> Result<JsonValue, String> {
        if !is_plausible_sealed(sealed) {
            return Err("implausible cookie value".to_string());
        }
        let encoded = sealed
            .strip_prefix(ENC_PREFIX)
            .ok_or_else(|| "not an encrypted cookie".to_string())?;
        let raw = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|e| format!("invalid base64: {}", e))?;
        let plaintext = aes_decrypt_bytes_aad(&raw, &self.enc_key, &purpose_aad(ENC_INFO, name))?;
        Self::open_payload(&plaintext)
    }

    pub(crate) fn seal_signed(
        &self,
        name: &str,
        val: &JsonValue,
        max_age: Option<u64>,
    ) -> Result<String, String> {
        let payload_b64 = URL_SAFE_NO_PAD.encode(Self::payload_bytes(val, max_age)?);
        let mac = hmac_sha256_bytes(&mac_input(name, &payload_b64), &self.sig_key);
        let sealed = format!(
            "{}{}.{}",
            SIG_PREFIX,
            payload_b64,
            URL_SAFE_NO_PAD.encode(mac)
        );
        check_sealed_len(&sealed)?;
        Ok(sealed)
    }

    pub(crate) fn open_signed(&self, name: &str, sealed: &str) -> Result<JsonValue, String> {
        if !is_plausible_sealed(sealed) {
            return Err("implausible cookie value".to_string());
        }
        let rest = sealed
            .strip_prefix(SIG_PREFIX)
            .ok_or_else(|| "not a signed cookie".to_string())?;
        let (payload_b64, mac_b64) = rest
            .rsplit_once('.')
            .ok_or_else(|| "malformed signed cookie".to_string())?;
        let expected = URL_SAFE_NO_PAD.encode(hmac_sha256_bytes(
            &mac_input(name, payload_b64),
            &self.sig_key,
        ));
        if !do_secure_compare(&expected, mac_b64) {
            return Err("bad cookie signature".to_string());
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(payload_b64)
            .map_err(|e| format!("invalid base64: {}", e))?;
        Self::open_payload(&bytes)
    }
}

// Derived jar, cached against the secret string it came from — NOT a OnceLock,
// because `session_configure({"secret": ...})` and per-test secrets can change
// the secret at runtime and the jar must follow.
static JAR_CACHE: RwLock<Option<(String, CookieJar)>> = RwLock::new(None);

/// The jar for the currently-configured session secret, deriving (and
/// caching) it on first use. Errs when no secret is configured.
pub(crate) fn current_jar() -> Result<CookieJar, String> {
    let secret = super::session::get_session_config().secret.ok_or_else(|| {
        "signed/encrypted cookies require a secret: set SOLI_SESSION_SECRET (32+ chars) \
         or call session_configure({\"secret\": ...})"
            .to_string()
    })?;
    jar_for_secret(&secret)
}

fn jar_for_secret(secret: &str) -> Result<CookieJar, String> {
    if let Some((cached_secret, jar)) = JAR_CACHE.read().unwrap().as_ref() {
        if cached_secret == secret {
            return Ok(jar.clone());
        }
    }
    let jar = CookieJar::new(secret)?;
    *JAR_CACHE.write().unwrap() = Some((secret.to_string(), jar.clone()));
    Ok(jar)
}

// The raw Cookie header of the request being handled on this worker thread.
// Stored as the unparsed string (one clone per request); parsed lazily only
// when `read_cookie` is actually called.
thread_local! {
    static REQUEST_COOKIE_HEADER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Install the incoming Cookie header for this request. Called at the top of
/// every request in the serve loop; passing `None` doubles as the per-request
/// clear, so a worker thread can never leak cookies across requests.
pub fn install_request_cookie_header(header: Option<&str>) {
    REQUEST_COOKIE_HEADER.with(|h| *h.borrow_mut() = header.map(|s| s.to_string()));
}

fn incoming_cookie(name: &str) -> Option<String> {
    REQUEST_COOKIE_HEADER.with(|h| {
        h.borrow()
            .as_deref()
            .and_then(|header| super::session::parse_cookies_from_header(Some(header)).remove(name))
    })
}

fn read_mode_from_options(options: &Value) -> Result<SealMode, String> {
    let Value::Hash(hash) = options else {
        return Err(format!(
            "read_cookie() options must be a hash, got {}",
            options.type_name()
        ));
    };
    let mut mode = SealMode::Plain;
    for (k, v) in hash.borrow().iter() {
        let HashKey::String(key) = k else {
            return Err("read_cookie() option keys must be strings".to_string());
        };
        let wanted = match key.as_ref() {
            "encrypted" => SealMode::Encrypted,
            "signed" => SealMode::Signed,
            other => {
                return Err(format!(
                    "read_cookie() unknown option \"{}\" (encrypted, signed)",
                    other
                ))
            }
        };
        match v {
            Value::Bool(true) => {
                if mode != SealMode::Plain {
                    return Err(
                        "read_cookie() options \"encrypted\" and \"signed\" are mutually exclusive"
                            .to_string(),
                    );
                }
                mode = wanted;
            }
            Value::Bool(false) => {}
            other => {
                return Err(format!(
                    "read_cookie() {} must be a boolean, got {}",
                    key,
                    other.type_name()
                ))
            }
        }
    }
    Ok(mode)
}

/// Register `read_cookie(name, options?)`.
pub fn register_cookie_jar_builtins(env: &mut Environment) {
    env.define(
        "read_cookie".to_string(),
        Value::NativeFunction(NativeFunction::new("read_cookie", None, |args| {
            let name = match args.first() {
                Some(Value::String(s)) => s.to_string(),
                Some(other) => {
                    return Err(format!(
                        "read_cookie() expects string name, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("read_cookie() requires a name".to_string()),
            };
            let mode = match args.get(1) {
                None => SealMode::Plain,
                Some(options) => read_mode_from_options(options)?,
            };
            // Same-request read-your-write first (a set_cookie earlier in this
            // request), then the incoming Cookie header.
            let raw =
                super::session::peek_response_cookie(&name).or_else(|| incoming_cookie(&name));
            let Some(raw) = raw else {
                return Ok(Value::Null);
            };
            match mode {
                SealMode::Plain => Ok(Value::String(raw.into())),
                SealMode::Encrypted => match current_jar()?.open_encrypted(&name, &raw) {
                    Ok(json) => json_to_value_ref(&json),
                    // Attacker-controlled failure: absent, not an error.
                    Err(_) => Ok(Value::Null),
                },
                SealMode::Signed => match current_jar()?.open_signed(&name, &raw) {
                    Ok(json) => json_to_value_ref(&json),
                    Err(_) => Ok(Value::Null),
                },
            }
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn jar() -> CookieJar {
        CookieJar::new(SECRET).unwrap()
    }

    #[test]
    fn rejects_short_secret() {
        // No unwrap_err(): the jar deliberately doesn't derive Debug so key
        // material can't end up in logs via `{:?}`.
        let err = match CookieJar::new("too-short") {
            Err(e) => e,
            Ok(_) => panic!("short secret must be rejected"),
        };
        assert!(err.contains("at least 32"), "unexpected error: {}", err);
    }

    #[test]
    fn encrypted_round_trip_across_value_shapes() {
        let jar = jar();
        for val in [
            json!({"theme": "dark", "cols": [1, 2]}),
            json!([1, "two", null]),
            json!(42),
            json!("héllo — ünïcode ✓"),
            json!(null),
        ] {
            let sealed = jar.seal_encrypted("prefs", &val, None).unwrap();
            assert!(sealed.starts_with(ENC_PREFIX));
            assert!(is_plausible_sealed(&sealed));
            assert_eq!(jar.open_encrypted("prefs", &sealed).unwrap(), val);
        }
    }

    #[test]
    fn signed_round_trip_across_value_shapes() {
        let jar = jar();
        for val in [json!({"uid": 7}), json!([true, false]), json!("plain")] {
            let sealed = jar.seal_signed("uid", &val, None).unwrap();
            assert!(sealed.starts_with(SIG_PREFIX));
            assert!(is_plausible_sealed(&sealed));
            assert_eq!(jar.open_signed("uid", &sealed).unwrap(), val);
        }
    }

    #[test]
    fn signed_payload_is_readable_but_tamperproof() {
        let jar = jar();
        let sealed = jar.seal_signed("uid", &json!(42), None).unwrap();

        // The middle segment is plain base64url JSON — anyone can read it.
        let rest = sealed.strip_prefix(SIG_PREFIX).unwrap();
        let (payload_b64, _mac) = rest.rsplit_once('.').unwrap();
        let payload: JarPayload =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload_b64).unwrap()).unwrap();
        assert_eq!(payload.val, json!(42));

        // ...but swapping the value without re-signing fails verification.
        let forged_payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&JarPayload {
                iat: payload.iat,
                exp: None,
                val: json!(9999),
            })
            .unwrap(),
        );
        let mac = rest.rsplit_once('.').unwrap().1;
        let forged = format!("{}{}.{}", SIG_PREFIX, forged_payload, mac);
        assert!(jar.open_signed("uid", &forged).is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let jar = jar();
        let sealed = jar.seal_encrypted("prefs", &json!({"a": 1}), None).unwrap();
        let mut bytes = sealed.into_bytes();
        let mid = bytes.len() / 2;
        bytes[mid] = if bytes[mid] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(jar.open_encrypted("prefs", &tampered).is_err());
    }

    #[test]
    fn name_swap_is_rejected_for_both_variants() {
        let jar = jar();
        let enc = jar.seal_encrypted("cookie_a", &json!(1), None).unwrap();
        assert!(jar.open_encrypted("cookie_b", &enc).is_err());
        let sig = jar.seal_signed("cookie_a", &json!(1), None).unwrap();
        assert!(jar.open_signed("cookie_b", &sig).is_err());
    }

    #[test]
    fn cross_variant_open_is_rejected() {
        let jar = jar();
        let enc = jar.seal_encrypted("x", &json!(1), None).unwrap();
        assert!(jar.open_signed("x", &enc).is_err());
        let sig = jar.seal_signed("x", &json!(1), None).unwrap();
        assert!(jar.open_encrypted("x", &sig).is_err());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let jar_a = jar();
        let jar_b = CookieJar::new("ffffffffffffffffffffffffffffffff").unwrap();
        let enc = jar_a.seal_encrypted("x", &json!(1), None).unwrap();
        assert!(jar_b.open_encrypted("x", &enc).is_err());
        let sig = jar_a.seal_signed("x", &json!(1), None).unwrap();
        assert!(jar_b.open_signed("x", &sig).is_err());
    }

    #[test]
    fn expiry_is_embedded_and_enforced() {
        // max_age far in the future: opens fine.
        let jar = jar();
        let sealed = jar.seal_encrypted("x", &json!(1), Some(3600)).unwrap();
        assert_eq!(jar.open_encrypted("x", &sealed).unwrap(), json!(1));

        // Payload with exp in the past is rejected; absent exp never expires.
        let expired = serde_json::to_vec(&JarPayload {
            iat: 0,
            exp: Some(1),
            val: json!(1),
        })
        .unwrap();
        assert!(CookieJar::open_payload(&expired).is_err());
        let eternal = serde_json::to_vec(&JarPayload {
            iat: 0,
            exp: None,
            val: json!(1),
        })
        .unwrap();
        assert_eq!(CookieJar::open_payload(&eternal).unwrap(), json!(1));
    }

    #[test]
    fn oversized_value_raises_at_seal_time() {
        let jar = jar();
        let big = json!("x".repeat(5000));
        let err = match jar.seal_encrypted("x", &big, None) {
            Err(e) => e,
            Ok(_) => panic!("oversized value must be refused"),
        };
        assert!(err.contains("4KB"), "unexpected error: {}", err);
        assert!(jar.seal_signed("x", &big, None).is_err());
    }

    #[test]
    fn implausible_input_is_rejected_before_decode() {
        let jar = jar();
        assert!(jar.open_encrypted("x", "").is_err());
        assert!(jar.open_encrypted("x", "enc.v1.not base64!!").is_err());
        assert!(jar.open_signed("x", "sig.v1.<script>").is_err());
        // A bare attacker-set cookie value is not a sealed value at all.
        assert!(jar.open_encrypted("x", "42").is_err());
        assert!(jar.open_signed("x", "42").is_err());
    }

    #[test]
    fn jar_cache_rederives_when_secret_changes() {
        let a = jar_for_secret(SECRET).unwrap();
        let sealed = a.seal_encrypted("x", &json!(1), None).unwrap();
        // Same secret: cached jar still opens it.
        assert!(jar_for_secret(SECRET)
            .unwrap()
            .open_encrypted("x", &sealed)
            .is_ok());
        // Changed secret: a different key, so the old blob no longer opens.
        let b = jar_for_secret("ffffffffffffffffffffffffffffffff").unwrap();
        assert!(b.open_encrypted("x", &sealed).is_err());
        // And back: re-derivation must follow every change.
        assert!(jar_for_secret(SECRET)
            .unwrap()
            .open_encrypted("x", &sealed)
            .is_ok());
    }
}
