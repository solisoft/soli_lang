//! Server-side passkeys (`WebAuthn.*`) — the relying-party half of the
//! WebAuthn Level 3 registration and authentication ceremonies.
//!
//! The class is **stateless**: it builds the options a browser hands to
//! `navigator.credentials.create()` / `.get()`, and it verifies what the
//! browser sends back. Storing the challenge between the two calls (single
//! use, short TTL, bound to the session) and storing the credential are the
//! application's job — `soli generate auth --passkeys` scaffolds both.
//!
//! Four static methods:
//!
//! - `WebAuthn.registration_options(opts)` → `{options, challenge}`
//! - `WebAuthn.verify_registration(credential, expected)` → credential record
//! - `WebAuthn.authentication_options(opts)` → `{options, challenge}`
//! - `WebAuthn.verify_authentication(credential, expected)` → assertion result
//!
//! `credential` is the browser's `PublicKeyCredential.toJSON()` hash, so a
//! controller passes `req["json"]` straight through.
//!
//! Every failed check **raises** (`WebAuthn.verify_authentication(): origin
//! mismatch …`); nothing fails by returning `false`, so a forgotten check at
//! the call site cannot turn a rejection into a login.
//!
//! The checks follow WebAuthn L3 §7.1 (registration) and §7.2
//! (authentication) and are numbered below as the spec numbers them where it
//! helps. Algorithms: ES256 (COSE -7, P-256 via `p256`), EdDSA (COSE -8,
//! Ed25519 per RFC 8032 on `curve25519-dalek`) and RS256 (COSE -257, the
//! in-house PKCS#1 v1.5 verifier in `rsa_key.rs`).
//!
//! Attestation is never *trusted* against a vendor root: the options always
//! ask for `"none"`. A `"packed"` statement with no certificate chain (self
//! attestation) has its signature checked against the credential key, and
//! is reported as `attestation_type: "self"`; any other format, or a packed
//! statement carrying `x5c`, is reported in `attestation_format` with
//! `attestation_type: "unverified"`. That is the right ceiling for consumer
//! passkeys (synced passkeys carry no attestation anyway); it is not enough
//! for an enterprise policy that pins authenticator models by AAGUID.
//!
//! CBOR is decoded by a small strict decoder here rather than a crate: the
//! two structures WebAuthn needs (the attestation object and a COSE key) use a
//! handful of major types, definite lengths only, and the decoder has to
//! report where an item ends inside `authData` — which a general-purpose
//! deserializer hides.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature as EcSignature, VerifyingKey as EcVerifyingKey};
use rand_core::{OsRng, RngCore};
use serde_json::{json, Map as JsonMap, Value as Json};
use sha2::{Digest, Sha256};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value, value_to_json, Class, NativeFunction, Value};

/// COSE algorithm identifiers this verifier accepts.
pub const ALG_ES256: i64 = -7;
pub const ALG_RS256: i64 = -257;
pub const ALG_EDDSA: i64 = -8;
/// Also the default `pubKeyCredParams`, in order of preference.
const SUPPORTED_ALGORITHMS: [i64; 3] = [ALG_ES256, ALG_EDDSA, ALG_RS256];

/// authenticatorData flag bits (WebAuthn L3 §6.1).
const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_BE: u8 = 0x08;
const FLAG_BS: u8 = 0x10;
const FLAG_AT: u8 = 0x40;
const FLAG_ED: u8 = 0x80;

/// Bytes of challenge entropy (the spec asks for at least 16).
const CHALLENGE_BYTES: usize = 32;
/// Default ceremony timeout handed to the browser, in milliseconds. Matches
/// the five-minute challenge TTL the scaffold enforces server-side.
const DEFAULT_TIMEOUT_MS: i64 = 300_000;
/// The spec caps a credential id at 1023 bytes.
const MAX_CREDENTIAL_ID: usize = 1023;
/// A user handle is at most 64 bytes.
const MAX_USER_HANDLE: usize = 64;
/// RSA moduli shorter than this are refused.
const MIN_RSA_MODULUS_BYTES: usize = 256;
/// CBOR nesting the decoder follows before giving up.
const MAX_CBOR_DEPTH: usize = 16;
/// Entries one CBOR map may hold. WebAuthn's maps (attestation object,
/// attStmt, COSE key, extensions) have a handful; the cap keeps the
/// duplicate-key check, which is quadratic in a map's size, cheap on a
/// forged map — the extensions map is parsed before any signature check.
const MAX_CBOR_MAP_ENTRIES: usize = 64;
/// Elements one CBOR array may hold (an attestation certificate chain).
const MAX_CBOR_ARRAY_ITEMS: usize = 256;

// ---------------------------------------------------------------------------
// CBOR (RFC 8949), the strict subset WebAuthn uses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cbor {
    Uint(u64),
    /// A negative integer, stored as its value (`-1 - n`).
    Nint(i128),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    Map(Vec<(Cbor, Cbor)>),
    Tag(u64, Box<Cbor>),
    Bool(bool),
    Null,
    Undefined,
    Float(f64),
}

impl Cbor {
    fn as_int(&self) -> Option<i128> {
        match self {
            Cbor::Uint(n) => Some(*n as i128),
            Cbor::Nint(n) => Some(*n),
            _ => None,
        }
    }

    /// Value under an integer key of a CBOR map (COSE keys are int-keyed).
    fn int_key(&self, key: i128) -> Option<&Cbor> {
        match self {
            Cbor::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| k.as_int() == Some(key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// Value under a text key of a CBOR map (the attestation object).
    fn text_key(&self, key: &str) -> Option<&Cbor> {
        match self {
            Cbor::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| matches!(k, Cbor::Text(t) if t == key))
                .map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Decode one CBOR item starting at `*pos`, advancing `*pos` past it.
pub(crate) fn cbor_decode_at(input: &[u8], pos: &mut usize) -> Result<Cbor, String> {
    decode_item(input, pos, 0)
}

/// Decode exactly one CBOR item spanning all of `input`; trailing bytes are an
/// error.
pub(crate) fn cbor_decode_exact(input: &[u8]) -> Result<Cbor, String> {
    let mut pos = 0;
    let item = decode_item(input, &mut pos, 0)?;
    if pos != input.len() {
        return Err("trailing bytes after the CBOR item".to_string());
    }
    Ok(item)
}

fn take<'a>(input: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], String> {
    let end = pos
        .checked_add(n)
        .filter(|end| *end <= input.len())
        .ok_or_else(|| "truncated CBOR".to_string())?;
    let slice = &input[*pos..end];
    *pos = end;
    Ok(slice)
}

/// Read the argument of an initial byte: the value itself for additional info
/// 0–23, or the following 1/2/4/8 bytes. Indefinite lengths (31) and the
/// reserved values (28–30) are refused.
fn read_argument(input: &[u8], pos: &mut usize, info: u8) -> Result<u64, String> {
    match info {
        0..=23 => Ok(info as u64),
        24 => Ok(take(input, pos, 1)?[0] as u64),
        25 => {
            let b = take(input, pos, 2)?;
            Ok(u16::from_be_bytes([b[0], b[1]]) as u64)
        }
        26 => {
            let b = take(input, pos, 4)?;
            Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64)
        }
        27 => {
            let b = take(input, pos, 8)?;
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            Ok(u64::from_be_bytes(a))
        }
        31 => Err("indefinite-length CBOR is not allowed".to_string()),
        _ => Err("malformed CBOR (reserved additional information)".to_string()),
    }
}

/// A length argument, bounded by what is left of the input so a forged
/// length cannot drive a huge allocation.
fn read_length(input: &[u8], pos: &mut usize, info: u8) -> Result<usize, String> {
    let n = read_argument(input, pos, info)?;
    let remaining = (input.len() - *pos) as u64;
    if n > remaining {
        return Err("truncated CBOR".to_string());
    }
    Ok(n as usize)
}

fn decode_item(input: &[u8], pos: &mut usize, depth: usize) -> Result<Cbor, String> {
    if depth > MAX_CBOR_DEPTH {
        return Err("CBOR nested too deeply".to_string());
    }
    let initial = take(input, pos, 1)?[0];
    let major = initial >> 5;
    let info = initial & 0x1f;
    match major {
        0 => Ok(Cbor::Uint(read_argument(input, pos, info)?)),
        1 => Ok(Cbor::Nint(-1 - read_argument(input, pos, info)? as i128)),
        2 => {
            let len = read_length(input, pos, info)?;
            Ok(Cbor::Bytes(take(input, pos, len)?.to_vec()))
        }
        3 => {
            let len = read_length(input, pos, info)?;
            let bytes = take(input, pos, len)?;
            String::from_utf8(bytes.to_vec())
                .map(Cbor::Text)
                .map_err(|_| "CBOR text string is not UTF-8".to_string())
        }
        4 => {
            // Each element takes at least one byte, so the remaining input
            // bounds the count.
            let len = read_length(input, pos, info)?;
            if len > MAX_CBOR_ARRAY_ITEMS {
                return Err("CBOR array is too long".to_string());
            }
            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                items.push(decode_item(input, pos, depth + 1)?);
            }
            Ok(Cbor::Array(items))
        }
        5 => {
            let len = read_length(input, pos, info)?;
            if len > MAX_CBOR_MAP_ENTRIES {
                return Err("CBOR map has too many entries".to_string());
            }
            let mut pairs: Vec<(Cbor, Cbor)> = Vec::with_capacity(len);
            for _ in 0..len {
                let key = decode_item(input, pos, depth + 1)?;
                let value = decode_item(input, pos, depth + 1)?;
                if pairs.iter().any(|(k, _)| *k == key) {
                    return Err("duplicate key in CBOR map".to_string());
                }
                pairs.push((key, value));
            }
            Ok(Cbor::Map(pairs))
        }
        6 => {
            let tag = read_argument(input, pos, info)?;
            Ok(Cbor::Tag(
                tag,
                Box::new(decode_item(input, pos, depth + 1)?),
            ))
        }
        _ => match info {
            20 => Ok(Cbor::Bool(false)),
            21 => Ok(Cbor::Bool(true)),
            22 => Ok(Cbor::Null),
            23 => Ok(Cbor::Undefined),
            25 => {
                let b = take(input, pos, 2)?;
                Ok(Cbor::Float(half_to_f64(u16::from_be_bytes([b[0], b[1]]))))
            }
            26 => {
                let b = take(input, pos, 4)?;
                Ok(Cbor::Float(
                    f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64
                ))
            }
            27 => {
                let b = take(input, pos, 8)?;
                let mut a = [0u8; 8];
                a.copy_from_slice(b);
                Ok(Cbor::Float(f64::from_be_bytes(a)))
            }
            _ => Err("unsupported CBOR simple value".to_string()),
        },
    }
}

fn half_to_f64(bits: u16) -> f64 {
    let exp = (bits >> 10) & 0x1f;
    let mant = (bits & 0x3ff) as f64;
    let value = match exp {
        0 => mant * 2f64.powi(-24),
        31 if mant == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (mant + 1024.0) * 2f64.powi(exp as i32 - 25),
    };
    if bits & 0x8000 != 0 {
        -value
    } else {
        value
    }
}

// ---------------------------------------------------------------------------
// authenticatorData (§6.1) and COSE keys
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct AttestedCredential {
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    /// The COSE_Key exactly as encoded in authData — what gets stored.
    pub cose_key: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct AuthData {
    pub rp_id_hash: [u8; 32],
    pub flags: u8,
    pub sign_count: u32,
    pub attested: Option<AttestedCredential>,
}

impl AuthData {
    fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

/// Parse authenticatorData: rpIdHash, flags, signCount, then the attested
/// credential data when AT is set and the extensions map when ED is set.
/// Every byte must be accounted for.
pub(crate) fn parse_auth_data(data: &[u8]) -> Result<AuthData, String> {
    if data.len() < 37 {
        return Err("authenticator data is too short".to_string());
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&data[..32]);
    let flags = data[32];
    let sign_count = u32::from_be_bytes([data[33], data[34], data[35], data[36]]);
    let mut pos = 37;

    let attested = if flags & FLAG_AT != 0 {
        if data.len() < pos + 18 {
            return Err("attested credential data is truncated".to_string());
        }
        let mut aaguid = [0u8; 16];
        aaguid.copy_from_slice(&data[pos..pos + 16]);
        pos += 16;
        let id_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if id_len == 0 || id_len > MAX_CREDENTIAL_ID {
            return Err("credential id length is out of range".to_string());
        }
        if data.len() < pos + id_len {
            return Err("credential id is truncated".to_string());
        }
        let credential_id = data[pos..pos + id_len].to_vec();
        pos += id_len;
        let key_start = pos;
        cbor_decode_at(data, &mut pos).map_err(|e| format!("credential public key: {}", e))?;
        Some(AttestedCredential {
            aaguid,
            credential_id,
            cose_key: data[key_start..pos].to_vec(),
        })
    } else {
        None
    };

    if flags & FLAG_ED != 0 {
        match cbor_decode_at(data, &mut pos).map_err(|e| format!("extensions: {}", e))? {
            Cbor::Map(_) => {}
            _ => return Err("extensions are not a CBOR map".to_string()),
        }
    }
    if pos != data.len() {
        return Err("trailing bytes in authenticator data".to_string());
    }
    Ok(AuthData {
        rp_id_hash,
        flags,
        sign_count,
        attested,
    })
}

/// A credential public key, parsed from its COSE_Key form.
pub(crate) enum CredentialKey {
    Es256(EcVerifyingKey),
    Rs256 {
        n: Vec<u8>,
        e: Vec<u8>,
    },
    /// The public key as encoded (`A`), and decompressed.
    Ed25519 {
        encoded: [u8; 32],
        point: curve25519_dalek::edwards::EdwardsPoint,
    },
}

impl CredentialKey {
    pub fn algorithm(&self) -> i64 {
        match self {
            CredentialKey::Es256(_) => ALG_ES256,
            CredentialKey::Rs256 { .. } => ALG_RS256,
            CredentialKey::Ed25519 { .. } => ALG_EDDSA,
        }
    }

    /// Verify `signature` over `message` (authData || SHA-256(clientDataJSON)).
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        match self {
            CredentialKey::Es256(key) => match EcSignature::from_der(signature) {
                Ok(sig) => key.verify(message, &sig).is_ok(),
                Err(_) => false,
            },
            CredentialKey::Rs256 { n, e } => {
                super::rsa_key::verify_pkcs1_v15_components(n, e, message, signature, "sha256")
                    .unwrap_or(false)
            }
            CredentialKey::Ed25519 { encoded, point } => {
                ed25519_verify(encoded, point, message, signature)
            }
        }
    }
}

/// RFC 8032 §5.1.7 Ed25519 verification (PureEdDSA): with `S` canonical
/// (`S < L`) and `k = SHA-512(R || A || M) mod L`, accept when
/// `[S]B - [k]A` encodes to the signature's `R`. The cofactorless equation,
/// as most libraries check it.
fn ed25519_verify(
    encoded: &[u8; 32],
    point: &curve25519_dalek::edwards::EdwardsPoint,
    message: &[u8],
    signature: &[u8],
) -> bool {
    use curve25519_dalek::edwards::EdwardsPoint;
    use curve25519_dalek::scalar::Scalar;
    use sha2::Sha512;

    let Ok(signature) = <[u8; 64]>::try_from(signature) else {
        return false;
    };
    let mut r = [0u8; 32];
    r.copy_from_slice(&signature[..32]);
    let mut s_bytes = [0u8; 32];
    s_bytes.copy_from_slice(&signature[32..]);
    let Some(s) = Option::<Scalar>::from(Scalar::from_canonical_bytes(s_bytes)) else {
        return false;
    };
    let mut hasher = Sha512::new();
    hasher.update(r);
    hasher.update(encoded);
    hasher.update(message);
    let mut wide = [0u8; 64];
    wide.copy_from_slice(&hasher.finalize());
    let k = Scalar::from_bytes_mod_order_wide(&wide);
    let expected_r = EdwardsPoint::vartime_double_scalar_mul_basepoint(&(-k), point, &s);
    expected_r.compress().to_bytes() == r
}

fn cose_bytes<'a>(key: &'a Cbor, label: i128, what: &str) -> Result<&'a [u8], String> {
    match key.int_key(label) {
        Some(Cbor::Bytes(b)) => Ok(b),
        Some(_) => Err(format!("COSE key: {} is not a byte string", what)),
        None => Err(format!("COSE key: {} is missing", what)),
    }
}

/// Parse a COSE_Key (RFC 9052 §7, RFC 9053) into a verifying key. Only the
/// two algorithms in `SUPPORTED_ALGORITHMS` are accepted, and `kty`/`crv`
/// must agree with `alg`.
pub(crate) fn parse_cose_key(bytes: &[u8]) -> Result<CredentialKey, String> {
    let key = cbor_decode_exact(bytes).map_err(|e| format!("COSE key: {}", e))?;
    if !matches!(key, Cbor::Map(_)) {
        return Err("COSE key is not a CBOR map".to_string());
    }
    let kty = key
        .int_key(1)
        .and_then(Cbor::as_int)
        .ok_or("COSE key: kty is missing")?;
    let alg = key
        .int_key(3)
        .and_then(Cbor::as_int)
        .ok_or("COSE key: alg is missing")?;
    match (kty, alg) {
        (2, -7) => {
            let crv = key.int_key(-1).and_then(Cbor::as_int);
            if crv != Some(1) {
                return Err("COSE key: ES256 requires the P-256 curve (crv 1)".to_string());
            }
            let x = cose_bytes(&key, -2, "x")?;
            let y = cose_bytes(&key, -3, "y")?;
            if x.len() != 32 || y.len() != 32 {
                return Err("COSE key: P-256 coordinates must be 32 bytes".to_string());
            }
            let mut point = Vec::with_capacity(65);
            point.push(0x04);
            point.extend_from_slice(x);
            point.extend_from_slice(y);
            let verifying = EcVerifyingKey::from_sec1_bytes(&point)
                .map_err(|_| "COSE key: not a valid P-256 point".to_string())?;
            Ok(CredentialKey::Es256(verifying))
        }
        (3, -257) => {
            let n = cose_bytes(&key, -1, "n")?;
            let e = cose_bytes(&key, -2, "e")?;
            let n: Vec<u8> = n.iter().skip_while(|b| **b == 0).copied().collect();
            let e: Vec<u8> = e.iter().skip_while(|b| **b == 0).copied().collect();
            if n.len() < MIN_RSA_MODULUS_BYTES {
                return Err("COSE key: RSA modulus is shorter than 2048 bits".to_string());
            }
            if e.is_empty() || e.len() > 8 {
                return Err("COSE key: RSA exponent is out of range".to_string());
            }
            Ok(CredentialKey::Rs256 { n, e })
        }
        (1, -8) => {
            // OKP key, Ed25519 curve (RFC 9053 §7.2).
            if key.int_key(-1).and_then(Cbor::as_int) != Some(6) {
                return Err("COSE key: EdDSA requires the Ed25519 curve (crv 6)".to_string());
            }
            let x = cose_bytes(&key, -2, "x")?;
            let encoded: [u8; 32] = x
                .try_into()
                .map_err(|_| "COSE key: an Ed25519 key must be 32 bytes".to_string())?;
            let point = curve25519_dalek::edwards::CompressedEdwardsY(encoded)
                .decompress()
                .ok_or("COSE key: not a valid Ed25519 point")?;
            // A small-order key verifies signatures it never made.
            if point.is_small_order() {
                return Err("COSE key: Ed25519 key of small order".to_string());
            }
            Ok(CredentialKey::Ed25519 { encoded, point })
        }
        (1, _) | (2, _) | (3, _) => Err(format!(
            "COSE key: kty {} does not match algorithm {}",
            kty, alg
        )),
        (_, other) => Err(format!("unsupported algorithm {}", other)),
    }
}

// ---------------------------------------------------------------------------
// Ceremony verification
// ---------------------------------------------------------------------------

/// Base64url, unpadded output; tolerant decoding (padding and the standard
/// alphabet are both accepted, since some clients send either).
pub(crate) fn b64url_encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn b64url_decode(text: &str) -> Result<Vec<u8>, String> {
    let normalized: String = text
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect();
    URL_SAFE_NO_PAD
        .decode(normalized.as_bytes())
        .map_err(|_| "invalid base64url".to_string())
}

fn json_str<'a>(obj: &'a Json, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Json::as_str)
}

fn required_str<'a>(obj: &'a Json, key: &str, what: &str) -> Result<&'a str, String> {
    match obj.get(key) {
        Some(Json::String(s)) if !s.is_empty() => Ok(s),
        Some(Json::String(_)) | None | Some(Json::Null) => Err(format!("{} is missing", what)),
        Some(_) => Err(format!("{} must be a string", what)),
    }
}

fn required_b64(obj: &Json, key: &str, what: &str) -> Result<Vec<u8>, String> {
    let text = required_str(obj, key, what)?;
    b64url_decode(text).map_err(|_| format!("{} is not valid base64url", what))
}

fn bool_opt(obj: &Json, key: &str, what: &str) -> Result<bool, String> {
    match obj.get(key) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{} must be true or false", what)),
    }
}

/// What the relying party expects of a ceremony response.
pub(crate) struct Expected {
    pub challenge: String,
    pub origins: Vec<String>,
    pub rp_id: String,
    pub require_user_verification: bool,
    pub algorithms: Vec<i64>,
}

impl Expected {
    pub fn from_json(expected: &Json) -> Result<Self, String> {
        if !expected.is_object() {
            return Err("expected must be a Hash".to_string());
        }
        let challenge = required_str(expected, "challenge", "expected challenge")?.to_string();
        let origins = match expected.get("origins").or_else(|| expected.get("origin")) {
            Some(Json::String(s)) if !s.is_empty() => vec![s.clone()],
            Some(Json::Array(items)) if !items.is_empty() => items
                .iter()
                .map(|item| match item {
                    Json::String(s) if !s.is_empty() => Ok(s.clone()),
                    _ => Err("expected origins must be non-empty strings".to_string()),
                })
                .collect::<Result<_, _>>()?,
            _ => {
                return Err(
                    "expected origins is missing (a String or an Array of origins)".to_string(),
                )
            }
        };
        let rp_id = required_str(expected, "rp_id", "expected rp_id")?.to_string();
        let require_user_verification = bool_opt(
            expected,
            "require_user_verification",
            "require_user_verification",
        )?;
        let algorithms = algorithms_opt(expected)?;
        Ok(Expected {
            challenge,
            origins,
            rp_id,
            require_user_verification,
            algorithms,
        })
    }
}

fn algorithms_opt(obj: &Json) -> Result<Vec<i64>, String> {
    match obj.get("algorithms") {
        None | Some(Json::Null) => Ok(SUPPORTED_ALGORITHMS.to_vec()),
        Some(Json::Array(items)) if !items.is_empty() => items
            .iter()
            .map(|item| match item.as_i64() {
                Some(alg) if SUPPORTED_ALGORITHMS.contains(&alg) => Ok(alg),
                Some(alg) => Err(format!(
                    "algorithm {} is not supported (use -7 for ES256, -8 for EdDSA or -257 for RS256)",
                    alg
                )),
                None => Err("algorithms must be COSE algorithm integers".to_string()),
            })
            .collect(),
        Some(_) => Err("algorithms must be a non-empty Array of integers".to_string()),
    }
}

/// The credential envelope common to both ceremonies: type, id/rawId and the
/// `response` object.
fn credential_envelope(credential: &Json) -> Result<(Vec<u8>, String, &Json), String> {
    if !credential.is_object() {
        return Err("credential must be a Hash (PublicKeyCredential.toJSON())".to_string());
    }
    if json_str(credential, "type") != Some("public-key") {
        return Err("credential type is not \"public-key\"".to_string());
    }
    let id_text = required_str(credential, "id", "credential id")?;
    let id = b64url_decode(id_text).map_err(|_| "credential id is not valid base64url")?;
    if id.is_empty() {
        return Err("credential id is empty".to_string());
    }
    if let Some(raw) = json_str(credential, "rawId") {
        let raw = b64url_decode(raw).map_err(|_| "credential rawId is not valid base64url")?;
        if raw != id {
            return Err("credential id and rawId differ".to_string());
        }
    }
    let response = credential
        .get("response")
        .filter(|r| r.is_object())
        .ok_or("credential response is missing")?;
    let id_text = b64url_encode(&id);
    Ok((id, id_text, response))
}

/// Steps shared by both ceremonies over clientDataJSON (§7.1 steps 5–10,
/// §7.2 steps 10–14). Returns SHA-256(clientDataJSON).
fn verify_client_data(
    client_data_json: &[u8],
    expected_type: &str,
    expected: &Expected,
) -> Result<[u8; 32], String> {
    let client_data: Json = serde_json::from_slice(client_data_json)
        .map_err(|_| "clientDataJSON is not valid JSON".to_string())?;
    // type
    match json_str(&client_data, "type") {
        Some(t) if t == expected_type => {}
        Some(t) => {
            return Err(format!(
                "clientData type mismatch (expected {}, got {})",
                expected_type, t
            ))
        }
        None => return Err("clientData type is missing".to_string()),
    }
    // challenge — compared as unpadded base64url, in constant time
    let received = json_str(&client_data, "challenge").ok_or("clientData challenge is missing")?;
    let received = received.trim_end_matches('=');
    let wanted = expected.challenge.trim_end_matches('=');
    if !super::crypto::do_secure_compare(received, wanted) {
        return Err("challenge mismatch".to_string());
    }
    // origin — exact match against the allow-list
    let origin = json_str(&client_data, "origin").ok_or("clientData origin is missing")?;
    if !expected.origins.iter().any(|o| o == origin) {
        return Err(format!(
            "origin mismatch (expected {}, got {})",
            expected.origins.join(" or "),
            origin
        ));
    }
    // cross-origin iframes are not supported
    if client_data.get("crossOrigin") == Some(&Json::Bool(true)) {
        return Err("cross-origin ceremonies are not allowed".to_string());
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&Sha256::digest(client_data_json));
    Ok(hash)
}

/// Flag and rpIdHash checks shared by both ceremonies.
fn verify_auth_data_common(auth: &AuthData, expected: &Expected) -> Result<(), String> {
    let rp_hash = Sha256::digest(expected.rp_id.as_bytes());
    if rp_hash[..] != auth.rp_id_hash[..] {
        return Err(format!(
            "rpIdHash mismatch (the credential is not scoped to {})",
            expected.rp_id
        ));
    }
    if !auth.has(FLAG_UP) {
        return Err("user presence flag is not set".to_string());
    }
    if expected.require_user_verification && !auth.has(FLAG_UV) {
        return Err("user verification is required but was not performed".to_string());
    }
    if auth.has(FLAG_BS) && !auth.has(FLAG_BE) {
        return Err("backup state is set without backup eligibility".to_string());
    }
    Ok(())
}

fn format_aaguid(aaguid: &[u8; 16]) -> String {
    let hex: String = aaguid.iter().map(|b| format!("{:02x}", b)).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Verify a registration response (§7.1). Returns the credential record to
/// store.
pub(crate) fn verify_registration(credential: &Json, expected: &Expected) -> Result<Json, String> {
    let (id, id_text, response) = credential_envelope(credential)?;
    let client_data_json = required_b64(response, "clientDataJSON", "response.clientDataJSON")?;
    let client_hash = verify_client_data(&client_data_json, "webauthn.create", expected)?;

    let attestation = required_b64(response, "attestationObject", "response.attestationObject")?;
    let attestation =
        cbor_decode_exact(&attestation).map_err(|e| format!("attestationObject: {}", e))?;
    let fmt = match attestation.text_key("fmt") {
        Some(Cbor::Text(fmt)) if !fmt.is_empty() => fmt.clone(),
        _ => return Err("attestationObject: fmt is missing".to_string()),
    };
    let att_stmt = match attestation.text_key("attStmt") {
        Some(stmt @ Cbor::Map(_)) => stmt,
        _ => return Err("attestationObject: attStmt is missing".to_string()),
    };
    if fmt == "none" && !matches!(att_stmt, Cbor::Map(pairs) if pairs.is_empty()) {
        return Err("attestationObject: \"none\" attestation carries a statement".to_string());
    }
    let auth_data_bytes = match attestation.text_key("authData") {
        Some(Cbor::Bytes(bytes)) => bytes,
        _ => return Err("attestationObject: authData is missing".to_string()),
    };
    let auth = parse_auth_data(auth_data_bytes)?;
    verify_auth_data_common(&auth, expected)?;

    let attested = auth
        .attested
        .as_ref()
        .ok_or("attested credential data is missing (AT flag not set)")?;
    if attested.credential_id != id {
        return Err("credential id does not match the attested credential data".to_string());
    }
    let key = parse_cose_key(&attested.cose_key)?;
    if !expected.algorithms.contains(&key.algorithm()) {
        return Err(format!(
            "algorithm {} is not in the allowed list",
            key.algorithm()
        ));
    }
    let attestation_type =
        verify_attestation_statement(&fmt, att_stmt, auth_data_bytes, &client_hash, &key)?;

    let transports: Vec<Json> = match response.get("transports") {
        Some(Json::Array(items)) => items
            .iter()
            .filter_map(|t| t.as_str().map(|s| Json::String(s.to_string())))
            .collect(),
        _ => Vec::new(),
    };

    Ok(json!({
        "credential_id": id_text,
        "public_key": b64url_encode(&attested.cose_key),
        "algorithm": key.algorithm(),
        "sign_count": auth.sign_count,
        "transports": transports,
        "aaguid": format_aaguid(&attested.aaguid),
        "user_verified": auth.has(FLAG_UV),
        "backup_eligible": auth.has(FLAG_BE),
        "backed_up": auth.has(FLAG_BS),
        "attestation_format": fmt,
        "attestation_type": attestation_type,
    }))
}

/// Check what can be checked of an attestation statement, and say how far
/// it went: `"none"`, `"self"` (a packed statement signed by the credential
/// key itself, §8.2 with no `x5c`), or `"unverified"` (a certificate chain or
/// another format — reported, never trusted).
fn verify_attestation_statement(
    fmt: &str,
    att_stmt: &Cbor,
    auth_data: &[u8],
    client_hash: &[u8; 32],
    key: &CredentialKey,
) -> Result<&'static str, String> {
    match fmt {
        "none" => Ok("none"),
        "packed" => {
            if att_stmt.text_key("x5c").is_some() {
                return Ok("unverified");
            }
            let alg = att_stmt
                .text_key("alg")
                .and_then(Cbor::as_int)
                .ok_or("packed attestation: alg is missing")?;
            if alg != key.algorithm() as i128 {
                return Err(format!(
                    "packed attestation: alg {} does not match the credential key's {}",
                    alg,
                    key.algorithm()
                ));
            }
            let sig = match att_stmt.text_key("sig") {
                Some(Cbor::Bytes(sig)) => sig,
                _ => return Err("packed attestation: sig is missing".to_string()),
            };
            let mut signed = auth_data.to_vec();
            signed.extend_from_slice(client_hash);
            if !key.verify(&signed, sig) {
                return Err("packed attestation: self-attestation signature is invalid".to_string());
            }
            Ok("self")
        }
        _ => Ok("unverified"),
    }
}

/// Verify an authentication response (§7.2) against the stored credential.
pub(crate) fn verify_authentication(
    credential: &Json,
    expected: &Expected,
    expected_json: &Json,
) -> Result<Json, String> {
    let (id, id_text, response) = credential_envelope(credential)?;

    // The stored credential the app looked up for this response.
    let public_key_text = required_str(expected_json, "public_key", "expected public_key")?;
    let public_key = b64url_decode(public_key_text)
        .map_err(|_| "expected public_key is not valid base64url".to_string())?;
    let stored_count = match expected_json.get("sign_count") {
        None | Some(Json::Null) => 0u64,
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= u32::MAX as u64)
            .ok_or("expected sign_count must be a non-negative integer")?,
    };
    if let Some(stored_id) = expected_json.get("credential_id") {
        let stored_id = stored_id
            .as_str()
            .ok_or("expected credential_id must be a string")?;
        let stored_id = b64url_decode(stored_id)
            .map_err(|_| "expected credential_id is not valid base64url".to_string())?;
        if stored_id != id {
            return Err("credential id does not match the stored credential".to_string());
        }
    }

    let client_data_json = required_b64(response, "clientDataJSON", "response.clientDataJSON")?;
    let auth_data_bytes =
        required_b64(response, "authenticatorData", "response.authenticatorData")?;
    let signature = required_b64(response, "signature", "response.signature")?;

    let client_hash = verify_client_data(&client_data_json, "webauthn.get", expected)?;
    let auth = parse_auth_data(&auth_data_bytes)?;
    verify_auth_data_common(&auth, expected)?;
    if auth.attested.is_some() {
        return Err("unexpected attested credential data in an assertion".to_string());
    }

    let key = parse_cose_key(&public_key)?;
    if !expected.algorithms.contains(&key.algorithm()) {
        return Err(format!(
            "algorithm {} is not in the allowed list",
            key.algorithm()
        ));
    }
    let mut signed = auth_data_bytes.clone();
    signed.extend_from_slice(&client_hash);
    if !key.verify(&signed, &signature) {
        return Err("signature is invalid".to_string());
    }

    let user_handle = match response.get("userHandle") {
        None | Some(Json::Null) => Json::Null,
        Some(Json::String(s)) if s.is_empty() => Json::Null,
        Some(Json::String(s)) => {
            let bytes =
                b64url_decode(s).map_err(|_| "userHandle is not valid base64url".to_string())?;
            if bytes.len() > MAX_USER_HANDLE {
                return Err("userHandle is longer than 64 bytes".to_string());
            }
            Json::String(String::from_utf8(bytes).map_err(|_| "userHandle is not a UTF-8 string")?)
        }
        Some(_) => return Err("userHandle must be a string".to_string()),
    };

    // §7.2 step 22: a counter that does not move forward may mean a cloned
    // authenticator. Both zero means the authenticator has no counter.
    let new_count = auth.sign_count as u64;
    let counter_regressed = !(new_count == 0 && stored_count == 0) && new_count <= stored_count;

    Ok(json!({
        "credential_id": id_text,
        "user_handle": user_handle,
        "sign_count": new_count,
        "user_verified": auth.has(FLAG_UV),
        "backup_eligible": auth.has(FLAG_BE),
        "backed_up": auth.has(FLAG_BS),
        "counter_regressed": counter_regressed,
    }))
}

// ---------------------------------------------------------------------------
// Options builders
// ---------------------------------------------------------------------------

fn new_challenge() -> String {
    let mut bytes = [0u8; CHALLENGE_BYTES];
    OsRng.fill_bytes(&mut bytes);
    b64url_encode(&bytes)
}

fn one_of(obj: &Json, key: &str, allowed: &[&str], default: &str) -> Result<String, String> {
    match obj.get(key) {
        None | Some(Json::Null) => Ok(default.to_string()),
        Some(Json::String(s)) if allowed.contains(&s.as_str()) => Ok(s.clone()),
        Some(_) => Err(format!("{} must be one of {}", key, allowed.join(", "))),
    }
}

fn timeout_opt(obj: &Json) -> Result<i64, String> {
    match obj.get("timeout") {
        None | Some(Json::Null) => Ok(DEFAULT_TIMEOUT_MS),
        Some(value) => value
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or_else(|| "timeout must be a positive integer (milliseconds)".to_string()),
    }
}

/// `exclude_credentials` / `allow_credentials`: an Array of credential id
/// strings, or of hashes `{id, transports}` (a stored credential row works).
fn credential_descriptors(obj: &Json, key: &str) -> Result<Vec<Json>, String> {
    let items = match obj.get(key) {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::Array(items)) => items,
        Some(_) => return Err(format!("{} must be an Array", key)),
    };
    items
        .iter()
        .map(|item| {
            let (id, transports) = match item {
                Json::String(id) => (id.as_str(), None),
                Json::Object(map) => {
                    let id = map
                        .get("id")
                        .or_else(|| map.get("credential_id"))
                        .and_then(Json::as_str)
                        .ok_or_else(|| format!("{}: each entry needs an id", key))?;
                    (id, map.get("transports"))
                }
                _ => return Err(format!("{}: entries must be ids or hashes", key)),
            };
            b64url_decode(id).map_err(|_| format!("{}: {} is not valid base64url", key, id))?;
            let mut descriptor = JsonMap::new();
            descriptor.insert("type".into(), json!("public-key"));
            descriptor.insert("id".into(), json!(id.trim_end_matches('=')));
            if let Some(Json::Array(list)) = transports {
                if !list.is_empty() {
                    descriptor.insert("transports".into(), Json::Array(list.clone()));
                }
            }
            Ok(Json::Object(descriptor))
        })
        .collect()
}

pub(crate) fn registration_options(opts: &Json) -> Result<Json, String> {
    if !opts.is_object() {
        return Err("expects an options Hash".to_string());
    }
    let rp_id = required_str(opts, "rp_id", "rp_id")?;
    let rp_name = json_str(opts, "rp_name")
        .filter(|s| !s.is_empty())
        .unwrap_or(rp_id);
    let user_id = match opts.get("user_id") {
        Some(Json::String(s)) if !s.is_empty() => s.clone(),
        Some(Json::Number(n)) => n.to_string(),
        _ => return Err("user_id is missing".to_string()),
    };
    if user_id.len() > MAX_USER_HANDLE {
        return Err("user_id must be at most 64 bytes".to_string());
    }
    let user_name = required_str(opts, "user_name", "user_name")?;
    let display_name = json_str(opts, "display_name")
        .filter(|s| !s.is_empty())
        .unwrap_or(user_name);
    let user_verification = one_of(
        opts,
        "user_verification",
        &["required", "preferred", "discouraged"],
        "preferred",
    )?;
    let resident_key = one_of(
        opts,
        "resident_key",
        &["required", "preferred", "discouraged"],
        "required",
    )?;
    let algorithms = algorithms_opt(opts)?;
    let exclude = credential_descriptors(opts, "exclude_credentials")?;
    let timeout = timeout_opt(opts)?;
    let challenge = new_challenge();

    let mut selection = JsonMap::new();
    selection.insert("residentKey".into(), json!(resident_key));
    selection.insert(
        "requireResidentKey".into(),
        json!(resident_key == "required"),
    );
    selection.insert("userVerification".into(), json!(user_verification));
    match opts.get("authenticator_attachment") {
        None | Some(Json::Null) => {}
        Some(Json::String(s)) if s == "platform" || s == "cross-platform" => {
            selection.insert("authenticatorAttachment".into(), json!(s));
        }
        Some(_) => {
            return Err("authenticator_attachment must be platform or cross-platform".to_string())
        }
    }

    let params: Vec<Json> = algorithms
        .iter()
        .map(|alg| json!({"type": "public-key", "alg": alg}))
        .collect();
    let options = json!({
        "challenge": challenge,
        "rp": {"id": rp_id, "name": rp_name},
        "user": {
            "id": b64url_encode(user_id.as_bytes()),
            "name": user_name,
            "displayName": display_name,
        },
        "pubKeyCredParams": params,
        "timeout": timeout,
        "attestation": "none",
        "excludeCredentials": exclude,
        "authenticatorSelection": Json::Object(selection),
    });
    Ok(json!({"options": options, "challenge": challenge}))
}

pub(crate) fn authentication_options(opts: &Json) -> Result<Json, String> {
    if !opts.is_object() {
        return Err("expects an options Hash".to_string());
    }
    let rp_id = required_str(opts, "rp_id", "rp_id")?;
    let user_verification = one_of(
        opts,
        "user_verification",
        &["required", "preferred", "discouraged"],
        "preferred",
    )?;
    let allow = credential_descriptors(opts, "allow_credentials")?;
    let timeout = timeout_opt(opts)?;
    let challenge = new_challenge();
    let options = json!({
        "challenge": challenge,
        "rpId": rp_id,
        "allowCredentials": allow,
        "userVerification": user_verification,
        "timeout": timeout,
    });
    Ok(json!({"options": options, "challenge": challenge}))
}

// ---------------------------------------------------------------------------
// The `WebAuthn` class
// ---------------------------------------------------------------------------

fn hash_arg(args: &[Value], index: usize, func: &str, what: &str) -> Result<Json, String> {
    match args.get(index) {
        Some(value @ Value::Hash(_)) => {
            value_to_json(value).map_err(|e| format!("{}(): {}", func, e))
        }
        Some(other) => Err(format!(
            "{}(): {} must be a Hash, got {}",
            func,
            what,
            other.type_name()
        )),
        None => Err(format!("{}(): {} is required", func, what)),
    }
}

fn native(
    name: &'static str,
    arity: usize,
    body: fn(&[Value], &'static str) -> Result<Json, String>,
) -> Rc<NativeFunction> {
    Rc::new(NativeFunction::new(name, Some(arity), move |args| {
        let result = body(args, name).map_err(|e| {
            if e.starts_with(name) {
                e
            } else {
                format!("{}(): {}", name, e)
            }
        })?;
        json_to_value(result)
    }))
}

pub fn register_webauthn_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    statics.insert(
        "registration_options".to_string(),
        native("WebAuthn.registration_options", 1, |args, func| {
            registration_options(&hash_arg(args, 0, func, "options")?)
        }),
    );
    statics.insert(
        "authentication_options".to_string(),
        native("WebAuthn.authentication_options", 1, |args, func| {
            authentication_options(&hash_arg(args, 0, func, "options")?)
        }),
    );
    statics.insert(
        "verify_registration".to_string(),
        native("WebAuthn.verify_registration", 2, |args, func| {
            let credential = hash_arg(args, 0, func, "credential")?;
            let expected = Expected::from_json(&hash_arg(args, 1, func, "expected")?)?;
            verify_registration(&credential, &expected)
        }),
    );
    statics.insert(
        "verify_authentication".to_string(),
        native("WebAuthn.verify_authentication", 2, |args, func| {
            let credential = hash_arg(args, 0, func, "credential")?;
            let expected_json = hash_arg(args, 1, func, "expected")?;
            let expected = Expected::from_json(&expected_json)?;
            verify_authentication(&credential, &expected, &expected_json)
        }),
    );

    let class = Class {
        name: "WebAuthn".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: statics,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("WebAuthn".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Signer;
    use p256::ecdsa::SigningKey;

    const VECTORS: &str = include_str!("../../../tests/fixtures/webauthn/vectors.json");
    const RP_ID: &str = "example.com";
    const ORIGIN: &str = "https://example.com";

    fn vectors() -> Json {
        serde_json::from_str(VECTORS).unwrap()
    }

    // -- a minimal CBOR encoder, enough to synthesize credentials ----------

    fn head(major: u8, value: u64) -> Vec<u8> {
        let m = major << 5;
        match value {
            0..=23 => vec![m | value as u8],
            24..=0xff => vec![m | 24, value as u8],
            0x100..=0xffff => {
                let mut v = vec![m | 25];
                v.extend_from_slice(&(value as u16).to_be_bytes());
                v
            }
            _ => {
                let mut v = vec![m | 26];
                v.extend_from_slice(&(value as u32).to_be_bytes());
                v
            }
        }
    }

    fn int(n: i64) -> Vec<u8> {
        if n >= 0 {
            head(0, n as u64)
        } else {
            head(1, (-1 - n) as u64)
        }
    }

    fn bytes(b: &[u8]) -> Vec<u8> {
        let mut v = head(2, b.len() as u64);
        v.extend_from_slice(b);
        v
    }

    fn text(s: &str) -> Vec<u8> {
        let mut v = head(3, s.len() as u64);
        v.extend_from_slice(s.as_bytes());
        v
    }

    fn map(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut v = head(5, pairs.len() as u64);
        for (k, val) in pairs {
            v.extend_from_slice(k);
            v.extend_from_slice(val);
        }
        v
    }

    // -- a synthetic ES256 authenticator ----------------------------------

    struct Authenticator {
        key: SigningKey,
        credential_id: Vec<u8>,
    }

    fn authenticator() -> Authenticator {
        Authenticator {
            key: SigningKey::from_slice(&[7u8; 32]).unwrap(),
            credential_id: vec![0xc1; 16],
        }
    }

    impl Authenticator {
        fn cose_key(&self) -> Vec<u8> {
            let point = self.key.verifying_key().to_sec1_point(false);
            let point = point.as_bytes();
            map(&[
                (int(1), int(2)),
                (int(3), int(-7)),
                (int(-1), int(1)),
                (int(-2), bytes(&point[1..33])),
                (int(-3), bytes(&point[33..65])),
            ])
        }

        fn auth_data(&self, rp_id: &str, flags: u8, count: u32, attested: bool) -> Vec<u8> {
            let mut data = Sha256::digest(rp_id.as_bytes()).to_vec();
            data.push(flags);
            data.extend_from_slice(&count.to_be_bytes());
            if attested {
                data.extend_from_slice(&[0u8; 16]);
                data.extend_from_slice(&(self.credential_id.len() as u16).to_be_bytes());
                data.extend_from_slice(&self.credential_id);
                data.extend_from_slice(&self.cose_key());
            }
            data
        }

        fn sign(&self, message: &[u8]) -> Vec<u8> {
            let sig: EcSignature = self.key.sign(message);
            sig.to_der().as_bytes().to_vec()
        }
    }

    fn client_data(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "type": kind,
            "challenge": challenge,
            "origin": origin,
            "crossOrigin": false,
        }))
        .unwrap()
    }

    fn registration_credential(id: &[u8], client: &[u8], attestation: &[u8]) -> Json {
        json!({
            "id": b64url_encode(id),
            "rawId": b64url_encode(id),
            "type": "public-key",
            "response": {
                "clientDataJSON": b64url_encode(client),
                "attestationObject": b64url_encode(attestation),
                "transports": ["internal"],
            },
        })
    }

    fn attestation_none(auth_data: &[u8]) -> Vec<u8> {
        map(&[
            (text("fmt"), text("none")),
            (text("attStmt"), map(&[])),
            (text("authData"), bytes(auth_data)),
        ])
    }

    fn expected(challenge: &str) -> Expected {
        Expected {
            challenge: challenge.to_string(),
            origins: vec![ORIGIN.to_string()],
            rp_id: RP_ID.to_string(),
            require_user_verification: false,
            algorithms: SUPPORTED_ALGORITHMS.to_vec(),
        }
    }

    const CHALLENGE: &str = "c2VydmVyLWNoYWxsZW5nZS0xMjM0NTY3ODkwMTIzNDU2";
    const UP_UV: u8 = FLAG_UP | FLAG_UV;

    /// A registration of the synthetic authenticator with one knob turned.
    fn register_with(
        flags: u8,
        kind: &str,
        challenge: &str,
        origin: &str,
        rp_id: &str,
    ) -> Result<Json, String> {
        let a = authenticator();
        let auth = a.auth_data(rp_id, flags, 0, flags & FLAG_AT != 0);
        let client = client_data(kind, challenge, origin);
        let credential =
            registration_credential(&a.credential_id, &client, &attestation_none(&auth));
        verify_registration(&credential, &expected(CHALLENGE))
    }

    fn register(flags: u8) -> Result<Json, String> {
        register_with(flags, "webauthn.create", CHALLENGE, ORIGIN, RP_ID)
    }

    /// An assertion signed by the synthetic authenticator, verified against
    /// its stored key with `stored_count`.
    fn authenticate_with(
        flags: u8,
        count: u32,
        stored_count: u64,
        tamper: impl Fn(&mut Vec<u8>, &mut Vec<u8>, &mut Vec<u8>),
    ) -> Result<Json, String> {
        let a = authenticator();
        let mut auth = a.auth_data(RP_ID, flags, count, false);
        let mut client = client_data("webauthn.get", CHALLENGE, ORIGIN);
        let mut signed = auth.clone();
        signed.extend_from_slice(&Sha256::digest(&client));
        let mut signature = a.sign(&signed);
        tamper(&mut auth, &mut client, &mut signature);
        let credential = json!({
            "id": b64url_encode(&a.credential_id),
            "rawId": b64url_encode(&a.credential_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": b64url_encode(&client),
                "authenticatorData": b64url_encode(&auth),
                "signature": b64url_encode(&signature),
                "userHandle": b64url_encode(b"user-1"),
            },
        });
        let expected_json = json!({
            "public_key": b64url_encode(&a.cose_key()),
            "sign_count": stored_count,
        });
        verify_authentication(&credential, &expected(CHALLENGE), &expected_json)
    }

    fn authenticate(flags: u8, count: u32, stored: u64) -> Result<Json, String> {
        authenticate_with(flags, count, stored, |_, _, _| {})
    }

    fn assert_err(result: Result<Json, String>, needle: &str) {
        match result {
            Ok(value) => panic!("expected an error containing {needle:?}, got {value}"),
            Err(e) => assert!(e.contains(needle), "expected {needle:?} in {e:?}"),
        }
    }

    // -- CBOR -------------------------------------------------------------

    #[test]
    fn cbor_round_trips_the_types_webauthn_uses() {
        let encoded = map(&[(int(-7), bytes(b"ab")), (text("k"), int(300))]);
        let decoded = cbor_decode_exact(&encoded).unwrap();
        assert_eq!(decoded.int_key(-7), Some(&Cbor::Bytes(b"ab".to_vec())));
        assert_eq!(decoded.text_key("k"), Some(&Cbor::Uint(300)));
    }

    #[test]
    fn cbor_rejects_trailing_bytes_truncation_and_indefinite_lengths() {
        let mut encoded = int(5);
        encoded.push(0);
        assert!(cbor_decode_exact(&encoded)
            .unwrap_err()
            .contains("trailing"));
        assert!(cbor_decode_exact(&[0x42, 0x01])
            .unwrap_err()
            .contains("truncated"));
        assert!(cbor_decode_exact(&[0x5f, 0x41, 0x00, 0xff])
            .unwrap_err()
            .contains("indefinite"));
        // A forged huge length must not allocate.
        assert!(
            cbor_decode_exact(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])
                .unwrap_err()
                .contains("truncated")
        );
        assert!(
            cbor_decode_exact(&map(&[(int(1), int(1)), (int(1), int(2))]))
                .unwrap_err()
                .contains("duplicate")
        );
        // Container sizes are bounded, so a forged map cannot make the
        // quadratic duplicate-key check expensive.
        let big_map: Vec<(Vec<u8>, Vec<u8>)> = (0..65).map(|i| (int(i), int(0))).collect();
        assert!(cbor_decode_exact(&map(&big_map))
            .unwrap_err()
            .contains("too many entries"));
        let ok_map: Vec<(Vec<u8>, Vec<u8>)> = (0..64).map(|i| (int(i), int(0))).collect();
        assert!(cbor_decode_exact(&map(&ok_map)).is_ok());
        let mut big_array = head(4, 257);
        big_array.extend(std::iter::repeat_n(0u8, 257));
        assert!(cbor_decode_exact(&big_array)
            .unwrap_err()
            .contains("too long"));
        // Deep nesting is bounded.
        let deep = vec![0x81u8; 64];
        assert!(cbor_decode_exact(&deep).is_err());
    }

    // -- vectors signed by OpenSSL ------------------------------------------

    fn vector_registration(name: &str) -> Result<Json, String> {
        let v = vectors();
        let v = &v[name];
        let challenge = v["registration_challenge"].as_str().unwrap();
        verify_registration(&v["registration"], &expected(challenge))
    }

    fn vector_authentication(name: &str, credential: &Json) -> Result<Json, String> {
        let v = vectors();
        let v = &v[name];
        let challenge = v["authentication_challenge"].as_str().unwrap();
        let expected_json = json!({
            "public_key": v["public_key"],
            "sign_count": 0,
            "credential_id": v["credential_id"],
        });
        verify_authentication(credential, &expected(challenge), &expected_json)
    }

    #[test]
    fn openssl_vectors_register_and_authenticate() {
        let v = vectors();
        for (name, alg) in [
            ("es256", ALG_ES256),
            ("rs256", ALG_RS256),
            ("eddsa", ALG_EDDSA),
        ] {
            let record = vector_registration(name).unwrap();
            assert_eq!(record["attestation_type"], json!("none"), "{name}");
            assert_eq!(record["algorithm"], json!(alg), "{name}");
            assert_eq!(record["credential_id"], v[name]["credential_id"]);
            assert_eq!(record["public_key"], v[name]["public_key"]);
            assert_eq!(record["sign_count"], json!(0));
            assert_eq!(record["user_verified"], json!(true));
            assert_eq!(record["backup_eligible"], json!(true));
            assert_eq!(record["backed_up"], json!(false));
            assert_eq!(record["attestation_format"], json!("none"));
            assert_eq!(
                record["aaguid"],
                json!("adce0002-35bc-c60a-648b-0b25f1f05503")
            );
            assert_eq!(record["transports"], json!(["internal", "hybrid"]));

            let result = vector_authentication(name, &v[name]["authentication"]).unwrap();
            assert_eq!(result["user_handle"], json!("user-1"), "{name}");
            assert_eq!(result["sign_count"], json!(1));
            assert_eq!(result["counter_regressed"], json!(false));
        }
    }

    #[test]
    fn openssl_vectors_reject_a_tampered_signature() {
        let v = vectors();
        for name in ["es256", "rs256", "eddsa"] {
            let mut credential = v[name]["authentication"].clone();
            let sig = b64url_decode(credential["response"]["signature"].as_str().unwrap()).unwrap();
            let mut sig = sig;
            let last = sig.len() - 1;
            sig[last] ^= 1;
            credential["response"]["signature"] = json!(b64url_encode(&sig));
            assert_err(
                vector_authentication(name, &credential),
                "signature is invalid",
            );
        }
    }

    #[test]
    fn a_key_of_one_algorithm_cannot_verify_the_other() {
        let v = vectors();
        let credential = v["es256"]["authentication"].clone();
        let expected_json = json!({"public_key": v["rs256"]["public_key"]});
        let challenge = v["es256"]["authentication_challenge"].as_str().unwrap();
        assert_err(
            verify_authentication(&credential, &expected(challenge), &expected_json),
            "signature is invalid",
        );
    }

    #[test]
    fn eddsa_rejects_another_message_and_a_non_canonical_s() {
        let v = vectors();
        let key =
            parse_cose_key(&b64url_decode(v["eddsa"]["public_key"].as_str().unwrap()).unwrap())
                .unwrap();
        let response = &v["eddsa"]["authentication"]["response"];
        let auth = b64url_decode(response["authenticatorData"].as_str().unwrap()).unwrap();
        let client = b64url_decode(response["clientDataJSON"].as_str().unwrap()).unwrap();
        let sig = b64url_decode(response["signature"].as_str().unwrap()).unwrap();
        let mut signed = auth.clone();
        signed.extend_from_slice(&Sha256::digest(&client));
        assert!(key.verify(&signed, &sig));
        let mut other = signed.clone();
        other[0] ^= 1;
        assert!(!key.verify(&other, &sig));
        // S + L is the same scalar, non-canonically encoded: refused.
        let l: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9,
            0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
        ];
        let mut bumped = sig.clone();
        let mut carry = 0u16;
        for i in 0..32 {
            let sum = bumped[32 + i] as u16 + l[i] as u16 + carry;
            bumped[32 + i] = sum as u8;
            carry = sum >> 8;
        }
        assert!(!key.verify(&signed, &bumped));
        assert!(!key.verify(&signed, &sig[..63]));
    }

    /// A vector's packed registration with its statement rebuilt by `edit`.
    fn packed_registration(name: &str, stmt: Vec<u8>) -> Result<Json, String> {
        let v = vectors();
        let mut credential = v[name]["packed_registration"].clone();
        let att = b64url_decode(
            credential["response"]["attestationObject"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let Some(Cbor::Bytes(auth)) = cbor_decode_exact(&att)
            .unwrap()
            .text_key("authData")
            .cloned()
        else {
            panic!("authData")
        };
        let rebuilt = map(&[
            (text("fmt"), text("packed")),
            (text("attStmt"), stmt),
            (text("authData"), bytes(&auth)),
        ]);
        credential["response"]["attestationObject"] = json!(b64url_encode(&rebuilt));
        let challenge = v[name]["registration_challenge"].as_str().unwrap();
        verify_registration(&credential, &expected(challenge))
    }

    fn packed_sig(name: &str) -> Vec<u8> {
        let v = vectors();
        let att = b64url_decode(
            v[name]["packed_registration"]["response"]["attestationObject"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let att = cbor_decode_exact(&att).unwrap();
        match att.text_key("attStmt").and_then(|s| s.text_key("sig")) {
            Some(Cbor::Bytes(sig)) => sig.clone(),
            _ => panic!("sig"),
        }
    }

    #[test]
    fn packed_self_attestation_is_verified() {
        let v = vectors();
        for name in ["es256", "rs256", "eddsa"] {
            let challenge = v[name]["registration_challenge"].as_str().unwrap();
            let record =
                verify_registration(&v[name]["packed_registration"], &expected(challenge)).unwrap();
            assert_eq!(record["attestation_format"], json!("packed"), "{name}");
            assert_eq!(record["attestation_type"], json!("self"), "{name}");
        }
    }

    #[test]
    fn packed_self_attestation_rejects_a_bad_signature_or_alg() {
        for (name, alg) in [("es256", -7), ("rs256", -257), ("eddsa", -8)] {
            let mut sig = packed_sig(name);
            let last = sig.len() - 1;
            sig[last] ^= 1;
            assert_err(
                packed_registration(
                    name,
                    map(&[(text("alg"), int(alg)), (text("sig"), bytes(&sig))]),
                ),
                "self-attestation signature is invalid",
            );
            let other = if alg == -7 { -257 } else { -7 };
            assert_err(
                packed_registration(
                    name,
                    map(&[
                        (text("alg"), int(other)),
                        (text("sig"), bytes(&packed_sig(name))),
                    ]),
                ),
                "does not match the credential key",
            );
            assert_err(
                packed_registration(name, map(&[(text("alg"), int(alg))])),
                "sig is missing",
            );
        }
    }

    #[test]
    fn a_certificate_chain_or_another_format_is_reported_unverified() {
        let chain = map(&[
            (text("alg"), int(-7)),
            (text("sig"), bytes(&[1, 2, 3])),
            (text("x5c"), {
                let mut array = head(4, 1);
                array.extend(bytes(&[0x30, 0x00]));
                array
            }),
        ]);
        let record = packed_registration("es256", chain).unwrap();
        assert_eq!(record["attestation_type"], json!("unverified"));
    }

    /// Random input never panics a parser: it is refused, or (rarely) read.
    #[test]
    fn the_parsers_survive_random_bytes() {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let v = vectors();
        let seeds: Vec<Vec<u8>> = ["es256", "rs256", "eddsa"]
            .iter()
            .flat_map(|name| {
                let att = b64url_decode(
                    v[*name]["packed_registration"]["response"]["attestationObject"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                let key = b64url_decode(v[*name]["public_key"].as_str().unwrap()).unwrap();
                [att, key]
            })
            .collect();
        for round in 0..20_000 {
            let mut input = if round % 2 == 0 {
                // A real structure, a few bytes flipped or cut.
                let mut seed = seeds[(next() as usize) % seeds.len()].clone();
                for _ in 0..1 + next() % 4 {
                    let at = (next() as usize) % seed.len();
                    seed[at] = next() as u8;
                }
                seed.truncate(1 + (next() as usize) % seed.len());
                seed
            } else {
                (0..(next() % 300)).map(|_| next() as u8).collect()
            };
            let _ = cbor_decode_exact(&input);
            let _ = parse_auth_data(&input);
            let _ = parse_cose_key(&input);
            input.push(0);
            let _ = parse_auth_data(&input);
        }
    }

    // -- registration, one negative test per check ----------------------------

    #[test]
    fn registration_happy_path() {
        let record = register(UP_UV | FLAG_AT).unwrap();
        assert_eq!(record["algorithm"], json!(-7));
        assert_eq!(record["user_verified"], json!(true));
        assert_eq!(
            record["aaguid"],
            json!("00000000-0000-0000-0000-000000000000")
        );
    }

    #[test]
    fn registration_rejects_the_wrong_client_data_type() {
        assert_err(
            register_with(UP_UV | FLAG_AT, "webauthn.get", CHALLENGE, ORIGIN, RP_ID),
            "clientData type mismatch",
        );
    }

    #[test]
    fn registration_rejects_another_challenge() {
        assert_err(
            register_with(UP_UV | FLAG_AT, "webauthn.create", "b3RoZXI", ORIGIN, RP_ID),
            "challenge mismatch",
        );
    }

    #[test]
    fn registration_rejects_another_origin() {
        assert_err(
            register_with(
                UP_UV | FLAG_AT,
                "webauthn.create",
                CHALLENGE,
                "https://evil.example",
                RP_ID,
            ),
            "origin mismatch (expected https://example.com, got https://evil.example)",
        );
        // Exact match only: a subdomain or another port is another origin.
        assert_err(
            register_with(
                UP_UV | FLAG_AT,
                "webauthn.create",
                CHALLENGE,
                "https://example.com:8443",
                RP_ID,
            ),
            "origin mismatch",
        );
    }

    #[test]
    fn registration_rejects_another_rp_id() {
        assert_err(
            register_with(
                UP_UV | FLAG_AT,
                "webauthn.create",
                CHALLENGE,
                ORIGIN,
                "evil.example",
            ),
            "rpIdHash mismatch",
        );
    }

    #[test]
    fn registration_requires_user_presence() {
        assert_err(register(FLAG_UV | FLAG_AT), "user presence flag is not set");
    }

    #[test]
    fn registration_enforces_user_verification_when_required() {
        let a = authenticator();
        let auth = a.auth_data(RP_ID, FLAG_UP | FLAG_AT, 0, true);
        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let credential =
            registration_credential(&a.credential_id, &client, &attestation_none(&auth));
        let mut wanted = expected(CHALLENGE);
        assert!(verify_registration(&credential, &wanted).is_ok());
        wanted.require_user_verification = true;
        assert_err(
            verify_registration(&credential, &wanted),
            "user verification is required",
        );
    }

    #[test]
    fn registration_requires_attested_credential_data() {
        assert_err(register(UP_UV), "AT flag not set");
    }

    #[test]
    fn registration_rejects_backup_state_without_eligibility() {
        assert_err(
            register(UP_UV | FLAG_AT | FLAG_BS),
            "backup state is set without backup eligibility",
        );
        let record = register(UP_UV | FLAG_AT | FLAG_BE | FLAG_BS).unwrap();
        assert_eq!(record["backed_up"], json!(true));
    }

    #[test]
    fn registration_rejects_trailing_bytes_in_the_attestation_object() {
        let a = authenticator();
        let auth = a.auth_data(RP_ID, UP_UV | FLAG_AT, 0, true);
        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let mut attestation = attestation_none(&auth);
        attestation.push(0x00);
        let credential = registration_credential(&a.credential_id, &client, &attestation);
        assert_err(
            verify_registration(&credential, &expected(CHALLENGE)),
            "trailing",
        );
    }

    #[test]
    fn registration_rejects_trailing_bytes_in_auth_data() {
        let a = authenticator();
        let mut auth = a.auth_data(RP_ID, UP_UV | FLAG_AT, 0, true);
        auth.push(0x00);
        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let credential =
            registration_credential(&a.credential_id, &client, &attestation_none(&auth));
        assert_err(
            verify_registration(&credential, &expected(CHALLENGE)),
            "trailing bytes in authenticator data",
        );
    }

    #[test]
    fn registration_skips_extension_data() {
        let a = authenticator();
        let mut auth = a.auth_data(RP_ID, UP_UV | FLAG_AT | FLAG_ED, 0, true);
        auth.extend_from_slice(&map(&[(text("credProtect"), int(2))]));
        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let credential =
            registration_credential(&a.credential_id, &client, &attestation_none(&auth));
        assert!(verify_registration(&credential, &expected(CHALLENGE)).is_ok());
    }

    #[test]
    fn registration_rejects_a_credential_id_that_differs_from_auth_data() {
        let a = authenticator();
        let auth = a.auth_data(RP_ID, UP_UV | FLAG_AT, 0, true);
        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let credential = registration_credential(&[0xaa; 16], &client, &attestation_none(&auth));
        assert_err(
            verify_registration(&credential, &expected(CHALLENGE)),
            "credential id does not match",
        );
    }

    #[test]
    fn registration_rejects_a_disallowed_algorithm() {
        let a = authenticator();
        let auth = a.auth_data(RP_ID, UP_UV | FLAG_AT, 0, true);
        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let credential =
            registration_credential(&a.credential_id, &client, &attestation_none(&auth));
        let mut wanted = expected(CHALLENGE);
        wanted.algorithms = vec![ALG_RS256];
        assert_err(
            verify_registration(&credential, &wanted),
            "algorithm -7 is not in the allowed list",
        );
    }

    #[test]
    fn registration_rejects_a_statement_on_none_attestation() {
        let a = authenticator();
        let auth = a.auth_data(RP_ID, UP_UV | FLAG_AT, 0, true);
        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let attestation = map(&[
            (text("fmt"), text("none")),
            (text("attStmt"), map(&[(text("sig"), bytes(b"x"))])),
            (text("authData"), bytes(&auth)),
        ]);
        let credential = registration_credential(&a.credential_id, &client, &attestation);
        assert_err(
            verify_registration(&credential, &expected(CHALLENGE)),
            "carries a statement",
        );
    }

    #[test]
    fn registration_reports_but_does_not_trust_other_formats() {
        let a = authenticator();
        let auth = a.auth_data(RP_ID, UP_UV | FLAG_AT, 0, true);
        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let attestation = map(&[
            (text("fmt"), text("tpm")),
            (text("attStmt"), map(&[(text("alg"), int(-7))])),
            (text("authData"), bytes(&auth)),
        ]);
        let credential = registration_credential(&a.credential_id, &client, &attestation);
        let record = verify_registration(&credential, &expected(CHALLENGE)).unwrap();
        assert_eq!(record["attestation_format"], json!("tpm"));
        assert_eq!(record["attestation_type"], json!("unverified"));
    }

    #[test]
    fn registration_rejects_cross_origin_and_mismatched_ids() {
        let a = authenticator();
        let auth = a.auth_data(RP_ID, UP_UV | FLAG_AT, 0, true);
        let client = serde_json::to_vec(&json!({
            "type": "webauthn.create", "challenge": CHALLENGE,
            "origin": ORIGIN, "crossOrigin": true,
        }))
        .unwrap();
        let credential =
            registration_credential(&a.credential_id, &client, &attestation_none(&auth));
        assert_err(
            verify_registration(&credential, &expected(CHALLENGE)),
            "cross-origin",
        );

        let client = client_data("webauthn.create", CHALLENGE, ORIGIN);
        let mut credential =
            registration_credential(&a.credential_id, &client, &attestation_none(&auth));
        credential["rawId"] = json!(b64url_encode(b"other"));
        assert_err(
            verify_registration(&credential, &expected(CHALLENGE)),
            "id and rawId differ",
        );
        credential["type"] = json!("password");
        assert_err(
            verify_registration(&credential, &expected(CHALLENGE)),
            "not \"public-key\"",
        );
    }

    // -- COSE keys --------------------------------------------------------------

    #[test]
    fn cose_key_parsing_rejects_malformed_keys() {
        let a = authenticator();
        let point = a.key.verifying_key().to_sec1_point(false);
        let point = point.as_bytes();
        let x = bytes(&point[1..33]);
        let y = bytes(&point[33..65]);
        let wrong_curve = map(&[
            (int(1), int(2)),
            (int(3), int(-7)),
            (int(-1), int(2)),
            (int(-2), x.clone()),
            (int(-3), y.clone()),
        ]);
        assert!(parse_cose_key(&wrong_curve)
            .err()
            .unwrap()
            .contains("P-256 curve"));
        let missing_y = map(&[
            (int(1), int(2)),
            (int(3), int(-7)),
            (int(-1), int(1)),
            (int(-2), x.clone()),
        ]);
        assert!(parse_cose_key(&missing_y)
            .err()
            .unwrap()
            .contains("y is missing"));
        let off_curve = map(&[
            (int(1), int(2)),
            (int(3), int(-7)),
            (int(-1), int(1)),
            (int(-2), x.clone()),
            (int(-3), bytes(&[1u8; 32])),
        ]);
        assert!(parse_cose_key(&off_curve)
            .err()
            .unwrap()
            .contains("not a valid P-256 point"));
        // y = 0 decodes to a point of order 4: refused.
        let small_order = map(&[
            (int(1), int(1)),
            (int(3), int(-8)),
            (int(-1), int(6)),
            (int(-2), bytes(&[0; 32])),
        ]);
        assert!(parse_cose_key(&small_order).is_err());
        let x448 = map(&[
            (int(1), int(1)),
            (int(3), int(-8)),
            (int(-1), int(7)),
            (int(-2), bytes(&[9; 32])),
        ]);
        assert!(parse_cose_key(&x448)
            .err()
            .unwrap()
            .contains("Ed25519 curve"));
        let short = map(&[
            (int(1), int(1)),
            (int(3), int(-8)),
            (int(-1), int(6)),
            (int(-2), bytes(&[9; 31])),
        ]);
        assert!(parse_cose_key(&short).err().unwrap().contains("32 bytes"));
        let mismatched = map(&[(int(1), int(3)), (int(3), int(-7))]);
        assert!(parse_cose_key(&mismatched)
            .err()
            .unwrap()
            .contains("does not match"));
        let short_rsa = map(&[
            (int(1), int(3)),
            (int(3), int(-257)),
            (int(-1), bytes(&[0xc5; 128])),
            (int(-2), bytes(&[1, 0, 1])),
        ]);
        assert!(parse_cose_key(&short_rsa).err().unwrap().contains("2048"));
    }

    // -- authentication, one negative test per check ------------------------------

    #[test]
    fn authentication_happy_path_and_counter_rule() {
        let result = authenticate(UP_UV, 5, 4).unwrap();
        assert_eq!(result["user_handle"], json!("user-1"));
        assert_eq!(result["sign_count"], json!(5));
        assert_eq!(result["counter_regressed"], json!(false));
        // Both zero: the authenticator keeps no counter (synced passkeys).
        assert_eq!(
            authenticate(UP_UV, 0, 0).unwrap()["counter_regressed"],
            json!(false)
        );
        // Equal or lower than stored: flagged, not rejected.
        assert_eq!(
            authenticate(UP_UV, 4, 4).unwrap()["counter_regressed"],
            json!(true)
        );
        assert_eq!(
            authenticate(UP_UV, 0, 9).unwrap()["counter_regressed"],
            json!(true)
        );
    }

    #[test]
    fn authentication_requires_user_presence() {
        assert_err(authenticate(FLAG_UV, 1, 0), "user presence flag is not set");
    }

    #[test]
    fn authentication_rejects_a_bad_signature() {
        assert_err(
            authenticate_with(UP_UV, 1, 0, |_, _, sig| {
                let last = sig.len() - 1;
                sig[last] ^= 1;
            }),
            "signature is invalid",
        );
        assert_err(
            authenticate_with(UP_UV, 1, 0, |_, _, sig| sig.truncate(3)),
            "signature is invalid",
        );
    }

    #[test]
    fn authentication_rejects_modified_auth_data_or_client_data() {
        // Raising the counter after signing breaks the signature.
        assert_err(
            authenticate_with(UP_UV, 1, 0, |auth, _, _| auth[36] = 9),
            "signature is invalid",
        );
        // Swapping the challenge is caught before the signature.
        assert_err(
            authenticate_with(UP_UV, 1, 0, |_, client, _| {
                *client = client_data("webauthn.get", "b3RoZXI", ORIGIN)
            }),
            "challenge mismatch",
        );
        assert_err(
            authenticate_with(UP_UV, 1, 0, |_, client, _| {
                *client = client_data("webauthn.create", CHALLENGE, ORIGIN)
            }),
            "clientData type mismatch",
        );
    }

    #[test]
    fn authentication_rejects_a_credential_that_is_not_the_stored_one() {
        let a = authenticator();
        let auth = a.auth_data(RP_ID, UP_UV, 1, false);
        let client = client_data("webauthn.get", CHALLENGE, ORIGIN);
        let mut signed = auth.clone();
        signed.extend_from_slice(&Sha256::digest(&client));
        let credential = json!({
            "id": b64url_encode(&a.credential_id),
            "type": "public-key",
            "response": {
                "clientDataJSON": b64url_encode(&client),
                "authenticatorData": b64url_encode(&auth),
                "signature": b64url_encode(&a.sign(&signed)),
            },
        });
        let expected_json = json!({
            "public_key": b64url_encode(&a.cose_key()),
            "credential_id": b64url_encode(b"another-credential"),
        });
        assert_err(
            verify_authentication(&credential, &expected(CHALLENGE), &expected_json),
            "does not match the stored credential",
        );
    }

    // -- options -----------------------------------------------------------------

    #[test]
    fn registration_options_shape() {
        let out = registration_options(&json!({
            "rp_id": RP_ID,
            "rp_name": "Example",
            "user_id": "user-1",
            "user_name": "alice@example.com",
            "exclude_credentials": ["AAEC", {"credential_id": "AwQF", "transports": ["usb"]}],
        }))
        .unwrap();
        let options = &out["options"];
        assert_eq!(options["challenge"], out["challenge"]);
        assert_eq!(
            b64url_decode(out["challenge"].as_str().unwrap())
                .unwrap()
                .len(),
            32
        );
        assert_eq!(options["rp"], json!({"id": RP_ID, "name": "Example"}));
        assert_eq!(options["user"]["id"], json!(b64url_encode(b"user-1")));
        assert_eq!(options["user"]["displayName"], json!("alice@example.com"));
        assert_eq!(options["attestation"], json!("none"));
        assert_eq!(
            options["pubKeyCredParams"],
            json!([
                {"type": "public-key", "alg": -7},
                {"type": "public-key", "alg": -8},
                {"type": "public-key", "alg": -257}
            ])
        );
        assert_eq!(
            options["authenticatorSelection"]["residentKey"],
            json!("required")
        );
        assert_eq!(
            options["authenticatorSelection"]["requireResidentKey"],
            json!(true)
        );
        assert_eq!(
            options["authenticatorSelection"]["userVerification"],
            json!("preferred")
        );
        assert_eq!(
            options["excludeCredentials"],
            json!([
                {"type": "public-key", "id": "AAEC"},
                {"type": "public-key", "id": "AwQF", "transports": ["usb"]}
            ])
        );
        // Two calls never share a challenge.
        let again =
            registration_options(&json!({"rp_id": RP_ID, "user_id": "u", "user_name": "n"}))
                .unwrap();
        assert_ne!(again["challenge"], out["challenge"]);
    }

    #[test]
    fn options_reject_bad_input() {
        assert!(
            registration_options(&json!({"user_id": "u", "user_name": "n"}))
                .unwrap_err()
                .contains("rp_id")
        );
        assert!(
            registration_options(&json!({"rp_id": RP_ID, "user_name": "n"}))
                .unwrap_err()
                .contains("user_id")
        );
        assert!(registration_options(
            &json!({"rp_id": RP_ID, "user_id": "u", "user_name": "n", "algorithms": [-36]})
        )
        .unwrap_err()
        .contains("-36"));
        assert!(registration_options(&json!({"rp_id": RP_ID, "user_id": "u", "user_name": "n", "user_verification": "always"}))
            .unwrap_err()
            .contains("user_verification"));
        assert!(authentication_options(&json!({}))
            .unwrap_err()
            .contains("rp_id"));
    }

    #[test]
    fn authentication_options_shape() {
        let out = authentication_options(&json!({"rp_id": RP_ID, "user_verification": "required"}))
            .unwrap();
        assert_eq!(out["options"]["rpId"], json!(RP_ID));
        assert_eq!(out["options"]["allowCredentials"], json!([]));
        assert_eq!(out["options"]["userVerification"], json!("required"));
        assert_eq!(out["options"]["timeout"], json!(DEFAULT_TIMEOUT_MS));
    }

    #[test]
    fn expected_requires_challenge_origin_and_rp_id() {
        assert!(
            Expected::from_json(&json!({"origins": ORIGIN, "rp_id": RP_ID}))
                .err()
                .unwrap()
                .contains("challenge")
        );
        assert!(
            Expected::from_json(&json!({"challenge": "x", "rp_id": RP_ID}))
                .err()
                .unwrap()
                .contains("origins")
        );
        assert!(
            Expected::from_json(&json!({"challenge": "x", "origins": [ORIGIN]}))
                .err()
                .unwrap()
                .contains("rp_id")
        );
        let e = Expected::from_json(&json!({"challenge": "x", "origin": ORIGIN, "rp_id": RP_ID}))
            .unwrap();
        assert_eq!(e.origins, vec![ORIGIN.to_string()]);
    }

    #[test]
    fn registered_class_raises_with_the_method_name() {
        let mut env = Environment::new();
        register_webauthn_builtins(&mut env);
        let Some(Value::Class(class)) = env.get("WebAuthn") else {
            panic!("WebAuthn is not registered")
        };
        let verify = class
            .native_static_methods
            .get("verify_registration")
            .unwrap();
        let err = (verify.func)(&[Value::Int(1), Value::Int(2)]).unwrap_err();
        assert_eq!(
            err,
            "WebAuthn.verify_registration(): credential must be a Hash, got int"
        );
    }
}
