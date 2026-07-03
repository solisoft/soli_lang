//! Cryptographic built-in functions and Crypto class.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use aes_gcm::{aead::Aead, Aes256Gcm, Nonce};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::Engine as _;
use curve25519_dalek::edwards::EdwardsPoint;
use curve25519_dalek::scalar::Scalar;
use hmac::{Hmac, Mac};
use md5::Md5;
use num_bigint::BigUint;
use rand_core::RngCore;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, NativeFunction, Value};

const X25519_PRIVATE_KEY_LENGTH: usize = 32;
const X25519_PUBLIC_KEY_LENGTH: usize = 32;

/// The X25519 basepoint (9 in Montgomery form)
const X25519_BASEPOINT_BYTES: [u8; 32] = [
    9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Helper to convert bytes to hex string
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Helper to convert hex string to bytes
fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("Invalid hex string: odd length".to_string());
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let chunk = &hex[i..i + 2];
        let byte =
            u8::from_str_radix(chunk, 16).map_err(|_| format!("Invalid hex byte: {}", chunk))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

/// Helper to convert Value to bytes
fn value_to_bytes(value: &Value) -> Result<Vec<u8>, String> {
    match value {
        Value::String(s) => {
            if s.len() == X25519_PRIVATE_KEY_LENGTH * 2 && s.chars().all(|c| c.is_ascii_hexdigit())
            {
                hex_to_bytes(s)
            } else {
                Ok(s.as_bytes().to_vec())
            }
        }
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                Value::Int(n) => Err(format!("byte value {} out of range", n)),
                other => Err(format!("expected byte, got {}", other.type_name())),
            })
            .collect(),
        other => Err(format!(
            "expected string or array, got {}",
            other.type_name()
        )),
    }
}

/// Helper to convert bytes to Value (returns hex string)
fn bytes_to_value(bytes: &[u8]) -> Value {
    Value::String(bytes_to_hex(bytes).into())
}

/// Perform X25519 scalar multiplication (Montgomery curve).
///
/// SEC-088: previously this routed through `MontgomeryPoint::to_edwards(0).unwrap()`,
/// which panicked for any 32-byte point that didn't decode to an Edwards
/// point with sign bit 0 — including small-order, low-bit, and
/// adversarial inputs. The unwind would crash the request-handling
/// thread, giving any controller that does X25519 key agreement against
/// untrusted clients a cheap DoS primitive. Multiplying directly on the
/// Montgomery curve via `MontgomeryPoint * Scalar` is total over all
/// 32-byte points: no panic for any input. Callers that derive a shared
/// secret should additionally reject the all-zero output (small-order
/// peer keys) — that check lives at the call site so public-key
/// derivation against the standard basepoint stays unrestricted.
fn x25519_scalar_mult(scalar: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    use curve25519_dalek::montgomery::MontgomeryPoint;

    let scalar_val = Scalar::from_bytes_mod_order(*scalar);
    let mont_point = MontgomeryPoint(*point);
    (mont_point * scalar_val).0
}

/// SEC-088: per RFC 7748 §6.1 / §6.2, an X25519 shared secret of all
/// zeros means the peer public key was a small-order point — accepting
/// it lets an attacker downgrade the shared secret to a known constant.
/// Used by the shared-secret entry points; not by public-key derivation
/// (which always multiplies by the well-formed standard basepoint and
/// can't produce an all-zero result for any clamped scalar).
fn x25519_is_small_order_output(shared: &[u8; 32]) -> bool {
    shared.iter().all(|&b| b == 0)
}

// ============================================================================
// Hash Functions Implementation
// ============================================================================

fn do_sha256(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    bytes_to_hex(&hasher.finalize())
}

fn do_sha512(data: &str) -> String {
    let mut hasher = Sha512::new();
    hasher.update(data.as_bytes());
    bytes_to_hex(&hasher.finalize())
}

fn do_md5(data: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(data.as_bytes());
    bytes_to_hex(&hasher.finalize())
}

fn do_hmac_sha256(message: &str, key: &str) -> Result<String, String> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(key.as_bytes()).map_err(|e| format!("HMAC error: {}", e))?;
    mac.update(message.as_bytes());
    Ok(bytes_to_hex(&mac.finalize().into_bytes()))
}

/// Constant-time byte comparison. Returns false immediately for unequal-length
/// inputs (length is not secret); for equal-length inputs the running time
/// depends only on the length, not on the position of any differing byte.
pub(crate) fn do_secure_compare(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in ab.iter().zip(bb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn do_argon2_hash(password: &[u8]) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password, &salt)
        .map_err(|e| format!("Failed to hash password: {}", e))?;
    Ok(hash.to_string())
}

fn do_argon2_verify(password: &[u8], hash_str: &str) -> Result<bool, String> {
    let parsed_hash =
        PasswordHash::new(hash_str).map_err(|e| format!("Invalid hash format: {}", e))?;
    let argon2 = Argon2::default();
    Ok(argon2.verify_password(password, &parsed_hash).is_ok())
}

fn do_x25519_keypair() -> (String, String) {
    let mut private_key = [0u8; X25519_PRIVATE_KEY_LENGTH];
    OsRng.fill_bytes(&mut private_key);
    private_key[0] &= 248;
    private_key[31] &= 127;
    private_key[31] |= 64;
    let public_key = x25519_scalar_mult(&private_key, &X25519_BASEPOINT_BYTES);
    (bytes_to_hex(&private_key), bytes_to_hex(&public_key))
}

fn do_ed25519_keypair() -> (String, String) {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let scalar = Scalar::from_bytes_mod_order(seed);
    let public_key = EdwardsPoint::mul_base(&scalar).compress().to_bytes();
    (bytes_to_hex(&seed), bytes_to_hex(&public_key))
}

const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32_decode(input: &str) -> Result<Vec<u8>, String> {
    let input = input.to_uppercase().replace('=', "");
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let mut output = Vec::with_capacity(input.len() * 5 / 8);
    let mut buffer: u64 = 0;
    let mut bits_in_buffer = 0;

    for c in input.chars() {
        let value = BASE32_ALPHABET
            .iter()
            .position(|&x| x == c as u8)
            .ok_or_else(|| format!("Invalid Base32 character: {}", c))?;
        buffer = (buffer << 5) | (value as u64);
        bits_in_buffer += 5;
        if bits_in_buffer >= 8 {
            bits_in_buffer -= 8;
            output.push((buffer >> bits_in_buffer) as u8);
            buffer &= (1 << bits_in_buffer) - 1;
        }
    }
    Ok(output)
}

fn do_totp_generate(secret: &str, time: u64, period: u64) -> Result<String, String> {
    let secret_bytes = base32_decode(secret)?;
    if secret_bytes.is_empty() {
        return Err("Secret cannot be empty".to_string());
    }
    let counter = time / period;
    let counter_bytes = counter.to_be_bytes();

    type HmacSha1 = hmac::Hmac<Sha1>;
    let mut mac =
        HmacSha1::new_from_slice(&secret_bytes).map_err(|e| format!("HMAC error: {}", e))?;
    mac.update(&counter_bytes);
    let result = mac.finalize().into_bytes();

    let offset = (result[19] & 0x0f) as usize;
    let code = ((result[offset] & 0x7f) as u32) << 24
        | (result[offset + 1] as u32) << 16
        | (result[offset + 2] as u32) << 8
        | (result[offset + 3] as u32);
    let code = code % 1_000_000;
    Ok(format!("{:06}", code))
}

fn do_totp_verify(secret: &str, code: &str, time: u64, period: u64) -> Result<bool, String> {
    let code_str = code.trim();
    if code_str.len() != 6 || !code_str.chars().all(|c| c.is_ascii_digit()) {
        return Err("Code must be 6 digits".to_string());
    }
    let windows = [time.saturating_sub(period), time, time + period];
    for window_time in windows {
        let expected = do_totp_generate(secret, window_time, period)?;
        if do_secure_compare(&expected, code_str) {
            return Ok(true);
        }
    }
    Ok(false)
}

// ============================================================================
// RSA primitives: modular exponentiation + PKCS#1 v1.5 padding
//
// These are the low-level building blocks for RSA signing/verification and,
// together with exclusive XML C14N (see `xml_c14n.rs`), for XML-DSig /
// WS-Security. They operate on octet strings (big-endian byte arrays), the
// representation RFC 8017 (PKCS#1) calls I2OSP/OS2IP.
// ============================================================================

/// Convert a Soli value to big-endian octets for the bignum / RSA primitives.
///
/// Unlike [`value_to_bytes`], a string is **always** interpreted as hex (with
/// an optional `0x` prefix), never as raw UTF-8. These primitives work on
/// octet strings — silently treating a hex-encoded 2048-bit modulus as UTF-8
/// bytes would be a dangerous, hard-to-debug surprise.
fn value_to_octets(value: &Value, what: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::String(s) => {
            let trimmed = s
                .strip_prefix("0x")
                .or_else(|| s.strip_prefix("0X"))
                .unwrap_or(s);
            hex_to_bytes(trimmed).map_err(|e| format!("{}: {}", what, e))
        }
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                Value::Int(n) => Err(format!("{}: byte value {} out of range 0-255", what, n)),
                other => Err(format!(
                    "{}: expected byte (Int 0-255), got {}",
                    what,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{}: expected hex string or byte array, got {}",
            what,
            other.type_name()
        )),
    }
}

/// Modular exponentiation `base^exp mod modulus` over big-endian octet strings.
///
/// The result is left-padded with zero octets to the modulus length `k`
/// (`k = ceil(bits(modulus) / 8)`), matching the RSA convention where a
/// signature / ciphertext is always `k` octets wide. This makes the output
/// directly composable with the PKCS#1 padding helpers.
fn do_modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, String> {
    let m = BigUint::from_bytes_be(modulus);
    if m == BigUint::from(0u32) {
        return Err("modulus must be non-zero".to_string());
    }
    let b = BigUint::from_bytes_be(base);
    let e = BigUint::from_bytes_be(exp);
    let result = b.modpow(&e, &m);

    let k = (m.bits().div_ceil(8) as usize).max(1);
    let raw = result.to_bytes_be();
    if raw.len() < k {
        let mut padded = vec![0u8; k - raw.len()];
        padded.extend_from_slice(&raw);
        Ok(padded)
    } else {
        // `result < modulus`, so `raw` can never be wider than `k`.
        Ok(raw)
    }
}

/// PKCS#1 v1.5 padding (RFC 8017 §7.2.1 / §9.2): `EM = 0x00 || BT || PS || 0x00 || data`.
///
/// `block_type` 1 (signatures) fills `PS` with `0xFF`; block type 2
/// (encryption) fills it with random non-zero octets. `PS` is always at least
/// 8 octets, so `data` must be at most `k - 11` octets long.
fn do_pkcs1_pad(data: &[u8], k: usize, block_type: u8) -> Result<Vec<u8>, String> {
    if k < 11 {
        return Err(format!(
            "key size {} too small for PKCS#1 v1.5 padding (need >= 11)",
            k
        ));
    }
    if data.len() > k - 11 {
        return Err(format!(
            "data length {} too long for key size {} (max {} octets)",
            data.len(),
            k,
            k - 11
        ));
    }
    let ps_len = k - 3 - data.len(); // >= 8, guaranteed by the check above
    let mut em = Vec::with_capacity(k);
    em.push(0x00);
    em.push(block_type);
    match block_type {
        1 => em.resize(em.len() + ps_len, 0xFFu8),
        2 => {
            // Random *non-zero* padding so the 0x00 separator is unambiguous.
            let mut written = 0;
            while written < ps_len {
                let mut b = [0u8; 1];
                OsRng.fill_bytes(&mut b);
                if b[0] != 0 {
                    em.push(b[0]);
                    written += 1;
                }
            }
        }
        other => {
            return Err(format!(
                "unsupported block type {} (use 1 for signatures, 2 for encryption)",
                other
            ))
        }
    }
    em.push(0x00);
    em.extend_from_slice(data);
    Ok(em)
}

/// Strip PKCS#1 v1.5 padding, returning the embedded data octets. Validates the
/// `0x00 || BT` prefix, the minimum 8-octet padding string, and (for block
/// type 1) that every padding octet is `0xFF`.
fn do_pkcs1_unpad(em: &[u8]) -> Result<Vec<u8>, String> {
    if em.len() < 11 {
        return Err("encoded message too short (need >= 11 octets)".to_string());
    }
    if em[0] != 0x00 {
        return Err(format!("first octet is 0x{:02x}, expected 0x00", em[0]));
    }
    let block_type = em[1];
    if block_type != 0x01 && block_type != 0x02 {
        return Err(format!(
            "unsupported block type 0x{:02x} (expected 0x01 or 0x02)",
            block_type
        ));
    }
    let mut sep = None;
    for (i, &b) in em.iter().enumerate().skip(2) {
        if b == 0x00 {
            sep = Some(i);
            break;
        }
        if block_type == 0x01 && b != 0xFF {
            return Err("block type 1 padding octet is not 0xFF".to_string());
        }
    }
    let sep = sep.ok_or("missing 0x00 separator after padding string")?;
    // PS must be >= 8 octets: separator index >= 2 + 8 = 10.
    if sep < 10 {
        return Err("padding string shorter than 8 octets".to_string());
    }
    Ok(em[sep + 1..].to_vec())
}

// ---------------------------------------------------------------------------
// Symmetric encryption (AES-256-GCM) — Crypto.encrypt/decrypt + model field
// encryption (see model `encrypts`). Output is base64(nonce ‖ ciphertext+tag).
// ---------------------------------------------------------------------------

/// Normalize arbitrary key material to a 32-byte AES key by SHA-256. Accepts a
/// hex/base64/raw high-entropy key or a passphrase.
pub(crate) fn derive_aes_key(material: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(material);
    hasher.finalize().into()
}

/// Resolve the encryption key: an explicit string, else `SOLI_ENCRYPTION_KEY`.
fn resolve_aes_key(explicit: Option<&str>) -> Result<[u8; 32], String> {
    let material = match explicit {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => std::env::var("SOLI_ENCRYPTION_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "no encryption key: pass one or set SOLI_ENCRYPTION_KEY".to_string())?,
    };
    Ok(derive_aes_key(material.as_bytes()))
}

/// Encrypt raw bytes: returns `nonce[12] ‖ ciphertext+tag`. Binary twin of
/// `aes_encrypt`, shared with the encrypted-bundle container (`src/bundle.rs`).
pub(crate) fn aes_encrypt_bytes(plaintext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    // Fully-qualified: `hmac::Mac` is also in scope and defines new_from_slice.
    let cipher = <Aes256Gcm as aes_gcm::aead::KeyInit>::new_from_slice(key)
        .map_err(|e| format!("bad key: {e}"))?;
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    #[allow(deprecated)]
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("encryption failed: {e}"))?;
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt `nonce[12] ‖ ciphertext+tag`. Binary twin of `aes_decrypt`.
pub(crate) fn aes_decrypt_bytes(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    if data.len() < 12 {
        return Err("ciphertext too short".to_string());
    }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let cipher = <Aes256Gcm as aes_gcm::aead::KeyInit>::new_from_slice(key)
        .map_err(|e| format!("bad key: {e}"))?;
    #[allow(deprecated)]
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "decryption failed (wrong key or corrupt data)".to_string())
}

fn aes_encrypt(plaintext: &str, key: &[u8; 32]) -> Result<String, String> {
    let out = aes_encrypt_bytes(plaintext.as_bytes(), key)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(out))
}

fn aes_decrypt(encoded: &str, key: &[u8; 32]) -> Result<String, String> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| format!("invalid base64: {e}"))?;
    let plaintext = aes_decrypt_bytes(&data, key)?;
    String::from_utf8(plaintext).map_err(|e| format!("decrypted data is not UTF-8: {e}"))
}

/// Encrypt a model field value using `SOLI_ENCRYPTION_KEY`. Used by `encrypts`.
pub fn encrypt_field(plaintext: &str) -> Result<String, String> {
    aes_encrypt(plaintext, &resolve_aes_key(None)?)
}

/// Decrypt a model field value using `SOLI_ENCRYPTION_KEY`. Used by `encrypts`.
pub fn decrypt_field(ciphertext: &str) -> Result<String, String> {
    aes_decrypt(ciphertext, &resolve_aes_key(None)?)
}

/// Register cryptographic functions and Crypto class in the given environment.
pub fn register_crypto_builtins(env: &mut Environment) {
    // Build Crypto static methods
    let mut crypto_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Crypto.encrypt(plaintext, key?) -> String (base64 nonce‖ciphertext)
    crypto_static_methods.insert(
        "encrypt".to_string(),
        Rc::new(NativeFunction::new("Crypto.encrypt", None, |args| {
            let plaintext = match args.first() {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err("Crypto.encrypt(plaintext, key?) expects a string".to_string()),
            };
            let key = match args.get(1) {
                Some(Value::String(s)) => Some(s.to_string()),
                None | Some(Value::Null) => None,
                Some(other) => {
                    return Err(format!(
                        "Crypto.encrypt key must be a string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(
                aes_encrypt(&plaintext, &resolve_aes_key(key.as_deref())?)?.into(),
            ))
        })),
    );

    // Crypto.decrypt(ciphertext, key?) -> String
    crypto_static_methods.insert(
        "decrypt".to_string(),
        Rc::new(NativeFunction::new("Crypto.decrypt", None, |args| {
            let ciphertext = match args.first() {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err("Crypto.decrypt(ciphertext, key?) expects a string".to_string()),
            };
            let key = match args.get(1) {
                Some(Value::String(s)) => Some(s.to_string()),
                None | Some(Value::Null) => None,
                Some(other) => {
                    return Err(format!(
                        "Crypto.decrypt key must be a string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(
                aes_decrypt(&ciphertext, &resolve_aes_key(key.as_deref())?)?.into(),
            ))
        })),
    );

    // Crypto.sha256(data) -> String
    crypto_static_methods.insert(
        "sha256".to_string(),
        Rc::new(NativeFunction::new("Crypto.sha256", Some(1), |args| {
            let data = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.sha256() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(do_sha256(&data).into()))
        })),
    );

    // Crypto.sha512(data) -> String
    crypto_static_methods.insert(
        "sha512".to_string(),
        Rc::new(NativeFunction::new("Crypto.sha512", Some(1), |args| {
            let data = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.sha512() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(do_sha512(&data).into()))
        })),
    );

    // Crypto.md5(data) -> String
    crypto_static_methods.insert(
        "md5".to_string(),
        Rc::new(NativeFunction::new("Crypto.md5", Some(1), |args| {
            let data = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.md5() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(do_md5(&data).into()))
        })),
    );

    // Crypto.hmac(message, key) -> String (uses SHA256)
    crypto_static_methods.insert(
        "hmac".to_string(),
        Rc::new(NativeFunction::new("Crypto.hmac", Some(2), |args| {
            let message = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.hmac() expects string message, got {}",
                        other.type_name()
                    ))
                }
            };
            let key = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.hmac() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };
            let result = do_hmac_sha256(&message, &key)?;
            Ok(Value::String(result.into()))
        })),
    );

    // Crypto.secure_compare(a, b) -> Bool — constant-time string equality
    crypto_static_methods.insert(
        "secure_compare".to_string(),
        Rc::new(NativeFunction::new(
            "Crypto.secure_compare",
            Some(2),
            |args| {
                let a = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Crypto.secure_compare() expects string, got {}",
                            other.type_name()
                        ))
                    }
                };
                let b = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Crypto.secure_compare() expects string, got {}",
                            other.type_name()
                        ))
                    }
                };
                Ok(Value::Bool(do_secure_compare(&a, &b)))
            },
        )),
    );

    // Crypto.argon2_hash(password) -> String
    crypto_static_methods.insert(
        "argon2_hash".to_string(),
        Rc::new(NativeFunction::new("Crypto.argon2_hash", Some(1), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes().to_vec(),
                other => {
                    return Err(format!(
                        "Crypto.argon2_hash() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let hash = do_argon2_hash(&password)?;
            Ok(Value::String(hash.into()))
        })),
    );

    // Crypto.argon2_verify(password, hash) -> Bool
    crypto_static_methods.insert(
        "argon2_verify".to_string(),
        Rc::new(NativeFunction::new(
            "Crypto.argon2_verify",
            Some(2),
            |args| {
                let password = match &args[0] {
                    Value::String(s) => s.as_bytes().to_vec(),
                    other => {
                        return Err(format!(
                            "Crypto.argon2_verify() expects string password, got {}",
                            other.type_name()
                        ))
                    }
                };
                let hash = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Crypto.argon2_verify() expects string hash, got {}",
                            other.type_name()
                        ))
                    }
                };
                let is_valid = do_argon2_verify(&password, &hash)?;
                Ok(Value::Bool(is_valid))
            },
        )),
    );

    // Crypto.password_hash(password) -> String (alias for argon2_hash)
    crypto_static_methods.insert(
        "password_hash".to_string(),
        Rc::new(NativeFunction::new(
            "Crypto.password_hash",
            Some(1),
            |args| {
                let password = match &args[0] {
                    Value::String(s) => s.as_bytes().to_vec(),
                    other => {
                        return Err(format!(
                            "Crypto.password_hash() expects string, got {}",
                            other.type_name()
                        ))
                    }
                };
                let hash = do_argon2_hash(&password)?;
                Ok(Value::String(hash.into()))
            },
        )),
    );

    // Crypto.password_verify(password, hash) -> Bool (alias for argon2_verify)
    crypto_static_methods.insert(
        "password_verify".to_string(),
        Rc::new(NativeFunction::new(
            "Crypto.password_verify",
            Some(2),
            |args| {
                let password = match &args[0] {
                    Value::String(s) => s.as_bytes().to_vec(),
                    other => {
                        return Err(format!(
                            "Crypto.password_verify() expects string password, got {}",
                            other.type_name()
                        ))
                    }
                };
                let hash = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Crypto.password_verify() expects string hash, got {}",
                            other.type_name()
                        ))
                    }
                };
                let is_valid = do_argon2_verify(&password, &hash)?;
                Ok(Value::Bool(is_valid))
            },
        )),
    );

    // Crypto.x25519_keypair() -> {private: String, public: String}
    crypto_static_methods.insert(
        "x25519_keypair".to_string(),
        Rc::new(NativeFunction::new(
            "Crypto.x25519_keypair",
            Some(0),
            |_args| {
                let (private, public) = do_x25519_keypair();
                Ok(hash_from_pairs([
                    ("private".to_string(), Value::String(private.into())),
                    ("public".to_string(), Value::String(public.into())),
                ]))
            },
        )),
    );

    // Crypto.x25519_shared_secret(private_key, public_key) -> String
    crypto_static_methods.insert(
        "x25519_shared_secret".to_string(),
        Rc::new(NativeFunction::new(
            "Crypto.x25519_shared_secret",
            Some(2),
            |args| {
                let private_bytes = value_to_bytes(&args[0])
                    .map_err(|e| format!("Crypto.x25519_shared_secret(): {}", e))?;
                if private_bytes.len() != X25519_PRIVATE_KEY_LENGTH {
                    return Err(format!(
                        "Crypto.x25519_shared_secret(): private key must be {} bytes, got {}",
                        X25519_PRIVATE_KEY_LENGTH,
                        private_bytes.len()
                    ));
                }
                let public_bytes = value_to_bytes(&args[1])
                    .map_err(|e| format!("Crypto.x25519_shared_secret(): {}", e))?;
                if public_bytes.len() != X25519_PUBLIC_KEY_LENGTH {
                    return Err(format!(
                        "Crypto.x25519_shared_secret(): public key must be {} bytes, got {}",
                        X25519_PUBLIC_KEY_LENGTH,
                        public_bytes.len()
                    ));
                }
                let mut private_key = [0u8; X25519_PRIVATE_KEY_LENGTH];
                private_key.copy_from_slice(&private_bytes[..X25519_PRIVATE_KEY_LENGTH]);
                private_key[0] &= 248;
                private_key[31] &= 127;
                private_key[31] |= 64;
                let mut public_array = [0u8; 32];
                public_array.copy_from_slice(&public_bytes[..32]);
                let shared = x25519_scalar_mult(&private_key, &public_array);
                if x25519_is_small_order_output(&shared) {
                    return Err("Crypto.x25519_shared_secret(): invalid X25519 public key \
                         (small-order point produced an all-zero shared secret)"
                        .to_string());
                }
                Ok(bytes_to_value(&shared))
            },
        )),
    );

    // Crypto.x25519_public_key(private_key) -> String
    crypto_static_methods.insert(
        "x25519_public_key".to_string(),
        Rc::new(NativeFunction::new(
            "Crypto.x25519_public_key",
            Some(1),
            |args| {
                let private_bytes = value_to_bytes(&args[0])
                    .map_err(|e| format!("Crypto.x25519_public_key(): {}", e))?;
                if private_bytes.len() != X25519_PRIVATE_KEY_LENGTH {
                    return Err(format!(
                        "Crypto.x25519_public_key(): private key must be {} bytes, got {}",
                        X25519_PRIVATE_KEY_LENGTH,
                        private_bytes.len()
                    ));
                }
                let mut private_key = [0u8; X25519_PRIVATE_KEY_LENGTH];
                private_key.copy_from_slice(&private_bytes[..X25519_PRIVATE_KEY_LENGTH]);
                private_key[0] &= 248;
                private_key[31] &= 127;
                private_key[31] |= 64;
                let public_key = x25519_scalar_mult(&private_key, &X25519_BASEPOINT_BYTES);
                Ok(bytes_to_value(&public_key))
            },
        )),
    );

    // Crypto.ed25519_keypair() -> {private: String, public: String}
    crypto_static_methods.insert(
        "ed25519_keypair".to_string(),
        Rc::new(NativeFunction::new(
            "Crypto.ed25519_keypair",
            Some(0),
            |_args| {
                let (private, public) = do_ed25519_keypair();
                Ok(hash_from_pairs([
                    ("private".to_string(), Value::String(private.into())),
                    ("public".to_string(), Value::String(public.into())),
                ]))
            },
        )),
    );

    // Crypto.totp_generate(secret, time?, period?) -> String
    crypto_static_methods.insert(
        "totp_generate".to_string(),
        Rc::new(NativeFunction::new("Crypto.totp_generate", None, |args| {
            if args.is_empty() || args.len() > 3 {
                return Err(format!(
                    "Crypto.totp_generate() expects 1-3 arguments (secret, time?, period?), got {}",
                    args.len()
                ));
            }
            let secret = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.totp_generate() expects string secret, got {}",
                        other.type_name()
                    ))
                }
            };
            let time = if args.len() > 1 {
                match &args[1] {
                    Value::Int(t) => *t as u64,
                    other => {
                        return Err(format!(
                            "Crypto.totp_generate() expects optional Int time, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs()
            };
            let period = if args.len() > 2 {
                match &args[2] {
                    Value::Int(p) => {
                        // SEC-089: reject zero / negative periods. The previous
                        // `*p as u64` cast turned 0 into a divide-by-zero panic
                        // in `do_totp_generate`, and any negative value into a
                        // huge positive `u64` via wrap-around.
                        if *p <= 0 {
                            return Err(format!(
                                "Crypto.totp_generate() period must be positive, got {}",
                                p
                            ));
                        }
                        *p as u64
                    }
                    other => {
                        return Err(format!(
                            "Crypto.totp_generate() expects optional Int period, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                30
            };
            let code = do_totp_generate(&secret, time, period)?;
            Ok(Value::String(code.into()))
        })),
    );

    // Crypto.totp_verify(secret, code, time?, period?) -> Bool
    crypto_static_methods.insert(
        "totp_verify".to_string(),
        Rc::new(NativeFunction::new("Crypto.totp_verify", None, |args| {
            if args.len() < 2 || args.len() > 4 {
                return Err(format!(
                    "Crypto.totp_verify() expects 2-4 arguments (secret, code, time?, period?), got {}",
                    args.len()
                ));
            }
            let secret = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.totp_verify() expects string secret, got {}",
                        other.type_name()
                    ))
                }
            };
            let code = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.totp_verify() expects string code, got {}",
                        other.type_name()
                    ))
                }
            };
            let time = if args.len() > 2 {
                match &args[2] {
                    Value::Int(t) => *t as u64,
                    other => {
                        return Err(format!(
                            "Crypto.totp_verify() expects optional Int time, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs()
            };
            let period = if args.len() > 3 {
                match &args[3] {
                    Value::Int(p) => {
                        // SEC-089: reject zero / negative periods (see the
                        // matching guard in Crypto.totp_generate above).
                        if *p <= 0 {
                            return Err(format!(
                                "Crypto.totp_verify() period must be positive, got {}",
                                p
                            ));
                        }
                        *p as u64
                    }
                    other => {
                        return Err(format!(
                            "Crypto.totp_verify() expects optional Int period, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                30
            };
            let valid = do_totp_verify(&secret, &code, time, period)?;
            Ok(Value::Bool(valid))
        })),
    );

    // Crypto.totp_uri(secret, account_name?, issuer?, period?) -> String
    crypto_static_methods.insert(
        "totp_uri".to_string(),
        Rc::new(NativeFunction::new("Crypto.totp_uri", None, |args| {
            if args.is_empty() || args.len() > 4 {
                return Err(format!(
                    "Crypto.totp_uri() expects 1-4 arguments (secret, account_name?, issuer?, period?), got {}",
                    args.len()
                ));
            }
            let secret = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Crypto.totp_uri() expects string secret, got {}",
                        other.type_name()
                    ))
                }
            };
            let account_name = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => Some(s.clone()),
                    other if other.type_name() == "Null" => None,
                    other => {
                        return Err(format!(
                            "Crypto.totp_uri() expects optional string account_name, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                None
            };
            let issuer = if args.len() > 2 {
                match &args[2] {
                    Value::String(s) => Some(s.clone()),
                    other if other.type_name() == "Null" => None,
                    other => {
                        return Err(format!(
                            "Crypto.totp_uri() expects optional string issuer, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                None
            };
            let period = if args.len() > 3 {
                match &args[3] {
                    Value::Int(p) => *p as u32,
                    other => {
                        return Err(format!(
                            "Crypto.totp_uri() expects optional Int period, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                30
            };

            // Build otpauth:// URI
            // Format: otpauth://totp/ISSUER:ACCOUNT?secret=SECRET&issuer=ISSUER&algorithm=SHA1&digits=6&period=30
            let mut uri = String::from("otpauth://totp/");

            if let (Some(i), Some(a)) = (&issuer, &account_name) {
                uri.push_str(&urlencoding::encode(i));
                uri.push(':');
                uri.push_str(&urlencoding::encode(a));
            } else if let Some(a) = &account_name {
                uri.push_str(&urlencoding::encode(a));
            } else if let Some(i) = &issuer {
                uri.push_str(&urlencoding::encode(i));
            }

            uri.push_str("?secret=");
            uri.push_str(&secret);
            uri.push_str("&algorithm=SHA1&digits=6&period=");
            uri.push_str(&period.to_string());

            if let Some(i) = &issuer {
                uri.push_str("&issuer=");
                uri.push_str(&urlencoding::encode(i));
            }

            Ok(Value::String(uri.into()))
        })),
    );

    // Crypto.modexp(base, exp, modulus) -> String (big-endian hex, k octets wide)
    crypto_static_methods.insert(
        "modexp".to_string(),
        Rc::new(NativeFunction::new("Crypto.modexp", Some(3), |args| {
            let base = value_to_octets(&args[0], "Crypto.modexp() base")?;
            let exp = value_to_octets(&args[1], "Crypto.modexp() exponent")?;
            let modulus = value_to_octets(&args[2], "Crypto.modexp() modulus")?;
            let result =
                do_modexp(&base, &exp, &modulus).map_err(|e| format!("Crypto.modexp(): {}", e))?;
            Ok(Value::String(bytes_to_hex(&result).into()))
        })),
    );

    // Crypto.pkcs1_pad(data, key_size, block_type?) -> String (hex)
    crypto_static_methods.insert(
        "pkcs1_pad".to_string(),
        Rc::new(NativeFunction::new("Crypto.pkcs1_pad", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "Crypto.pkcs1_pad() expects 2-3 arguments (data, key_size, block_type?), got {}",
                    args.len()
                ));
            }
            let data = value_to_octets(&args[0], "Crypto.pkcs1_pad() data")?;
            let key_size = match &args[1] {
                Value::Int(n) if *n > 0 => *n as usize,
                Value::Int(n) => {
                    return Err(format!(
                        "Crypto.pkcs1_pad() key_size must be positive, got {}",
                        n
                    ))
                }
                other => {
                    return Err(format!(
                        "Crypto.pkcs1_pad() expects Int key_size, got {}",
                        other.type_name()
                    ))
                }
            };
            let block_type = if args.len() == 3 {
                match &args[2] {
                    Value::Int(t @ (1 | 2)) => *t as u8,
                    Value::Int(t) => {
                        return Err(format!(
                            "Crypto.pkcs1_pad() block_type must be 1 or 2, got {}",
                            t
                        ))
                    }
                    other => {
                        return Err(format!(
                            "Crypto.pkcs1_pad() expects Int block_type, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                1
            };
            let em = do_pkcs1_pad(&data, key_size, block_type)
                .map_err(|e| format!("Crypto.pkcs1_pad(): {}", e))?;
            Ok(Value::String(bytes_to_hex(&em).into()))
        })),
    );

    // Crypto.pkcs1_unpad(encoded_message) -> String (hex of the embedded data)
    crypto_static_methods.insert(
        "pkcs1_unpad".to_string(),
        Rc::new(NativeFunction::new("Crypto.pkcs1_unpad", Some(1), |args| {
            let em = value_to_octets(&args[0], "Crypto.pkcs1_unpad() encoded message")?;
            let data = do_pkcs1_unpad(&em).map_err(|e| format!("Crypto.pkcs1_unpad(): {}", e))?;
            Ok(Value::String(bytes_to_hex(&data).into()))
        })),
    );

    // Create and register the Crypto class
    let crypto_class = Class {
        name: "Crypto".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: crypto_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("Crypto".to_string(), Value::Class(Rc::new(crypto_class)));

    // ========================================================================
    // Standalone functions for backward compatibility
    // ========================================================================

    // sha256(data) -> String
    env.define(
        "sha256".to_string(),
        Value::NativeFunction(NativeFunction::new("sha256", Some(1), |args| {
            let data = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "sha256() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(do_sha256(&data).into()))
        })),
    );

    // sha512(data) -> String
    env.define(
        "sha512".to_string(),
        Value::NativeFunction(NativeFunction::new("sha512", Some(1), |args| {
            let data = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "sha512() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(do_sha512(&data).into()))
        })),
    );

    // md5(data) -> String
    env.define(
        "md5".to_string(),
        Value::NativeFunction(NativeFunction::new("md5", Some(1), |args| {
            let data = match &args[0] {
                Value::String(s) => s.clone(),
                other => return Err(format!("md5() expects string, got {}", other.type_name())),
            };
            Ok(Value::String(do_md5(&data).into()))
        })),
    );

    // hmac(message, key) -> String
    env.define(
        "hmac".to_string(),
        Value::NativeFunction(NativeFunction::new("hmac", Some(2), |args| {
            let message = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "hmac() expects string message, got {}",
                        other.type_name()
                    ))
                }
            };
            let key = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "hmac() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };
            let result = do_hmac_sha256(&message, &key)?;
            Ok(Value::String(result.into()))
        })),
    );

    // secure_compare(a, b) -> Bool — constant-time string equality
    env.define(
        "secure_compare".to_string(),
        Value::NativeFunction(NativeFunction::new("secure_compare", Some(2), |args| {
            let a = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "secure_compare() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let b = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "secure_compare() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::Bool(do_secure_compare(&a, &b)))
        })),
    );

    // argon2_hash(password) -> String
    env.define(
        "argon2_hash".to_string(),
        Value::NativeFunction(NativeFunction::new("argon2_hash", Some(1), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes().to_vec(),
                other => {
                    return Err(format!(
                        "argon2_hash() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let hash = do_argon2_hash(&password)?;
            Ok(Value::String(hash.into()))
        })),
    );

    // argon2_verify(password, hash) -> Bool
    env.define(
        "argon2_verify".to_string(),
        Value::NativeFunction(NativeFunction::new("argon2_verify", Some(2), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes().to_vec(),
                other => {
                    return Err(format!(
                        "argon2_verify() expects string password, got {}",
                        other.type_name()
                    ))
                }
            };
            let hash = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "argon2_verify() expects string hash, got {}",
                        other.type_name()
                    ))
                }
            };
            let is_valid = do_argon2_verify(&password, &hash)?;
            Ok(Value::Bool(is_valid))
        })),
    );

    // password_hash(password) -> String
    env.define(
        "password_hash".to_string(),
        Value::NativeFunction(NativeFunction::new("password_hash", Some(1), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes().to_vec(),
                other => {
                    return Err(format!(
                        "password_hash() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let hash = do_argon2_hash(&password)?;
            Ok(Value::String(hash.into()))
        })),
    );

    // password_verify(password, hash) -> Bool
    env.define(
        "password_verify".to_string(),
        Value::NativeFunction(NativeFunction::new("password_verify", Some(2), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes().to_vec(),
                other => {
                    return Err(format!(
                        "password_verify() expects string password, got {}",
                        other.type_name()
                    ))
                }
            };
            let hash = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "password_verify() expects string hash, got {}",
                        other.type_name()
                    ))
                }
            };
            let is_valid = do_argon2_verify(&password, &hash)?;
            Ok(Value::Bool(is_valid))
        })),
    );

    // x25519_keypair() -> {private: String, public: String}
    env.define(
        "x25519_keypair".to_string(),
        Value::NativeFunction(NativeFunction::new("x25519_keypair", Some(0), |_args| {
            let (private, public) = do_x25519_keypair();
            Ok(hash_from_pairs([
                ("private".to_string(), Value::String(private.into())),
                ("public".to_string(), Value::String(public.into())),
            ]))
        })),
    );

    // x25519_shared_secret(private_key, public_key) -> String
    env.define(
        "x25519_shared_secret".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "x25519_shared_secret",
            Some(2),
            |args| {
                let private_bytes = value_to_bytes(&args[0])
                    .map_err(|e| format!("x25519_shared_secret(): {}", e))?;
                if private_bytes.len() != X25519_PRIVATE_KEY_LENGTH {
                    return Err(format!(
                        "x25519_shared_secret(): private key must be {} bytes, got {}",
                        X25519_PRIVATE_KEY_LENGTH,
                        private_bytes.len()
                    ));
                }
                let public_bytes = value_to_bytes(&args[1])
                    .map_err(|e| format!("x25519_shared_secret(): {}", e))?;
                if public_bytes.len() != X25519_PUBLIC_KEY_LENGTH {
                    return Err(format!(
                        "x25519_shared_secret(): public key must be {} bytes, got {}",
                        X25519_PUBLIC_KEY_LENGTH,
                        public_bytes.len()
                    ));
                }
                let mut private_key = [0u8; X25519_PRIVATE_KEY_LENGTH];
                private_key.copy_from_slice(&private_bytes[..X25519_PRIVATE_KEY_LENGTH]);
                private_key[0] &= 248;
                private_key[31] &= 127;
                private_key[31] |= 64;
                let mut public_array = [0u8; 32];
                public_array.copy_from_slice(&public_bytes[..32]);
                let shared = x25519_scalar_mult(&private_key, &public_array);
                if x25519_is_small_order_output(&shared) {
                    return Err("x25519_shared_secret(): invalid X25519 public key \
                         (small-order point produced an all-zero shared secret)"
                        .to_string());
                }
                Ok(bytes_to_value(&shared))
            },
        )),
    );

    // x25519_public_key(private_key) -> String
    env.define(
        "x25519_public_key".to_string(),
        Value::NativeFunction(NativeFunction::new("x25519_public_key", Some(1), |args| {
            let private_bytes =
                value_to_bytes(&args[0]).map_err(|e| format!("x25519_public_key(): {}", e))?;
            if private_bytes.len() != X25519_PRIVATE_KEY_LENGTH {
                return Err(format!(
                    "x25519_public_key(): private key must be {} bytes, got {}",
                    X25519_PRIVATE_KEY_LENGTH,
                    private_bytes.len()
                ));
            }
            let mut private_key = [0u8; X25519_PRIVATE_KEY_LENGTH];
            private_key.copy_from_slice(&private_bytes[..X25519_PRIVATE_KEY_LENGTH]);
            private_key[0] &= 248;
            private_key[31] &= 127;
            private_key[31] |= 64;
            let public_key = x25519_scalar_mult(&private_key, &X25519_BASEPOINT_BYTES);
            Ok(bytes_to_value(&public_key))
        })),
    );

    // x25519(basepoint, scalar) -> String
    env.define(
        "x25519".to_string(),
        Value::NativeFunction(NativeFunction::new("x25519", Some(2), |args| {
            let basepoint_bytes =
                value_to_bytes(&args[0]).map_err(|e| format!("x25519(): {}", e))?;
            if basepoint_bytes.len() != X25519_PUBLIC_KEY_LENGTH {
                return Err(format!(
                    "x25519(): basepoint must be {} bytes, got {}",
                    X25519_PUBLIC_KEY_LENGTH,
                    basepoint_bytes.len()
                ));
            }
            let scalar_bytes = value_to_bytes(&args[1]).map_err(|e| format!("x25519(): {}", e))?;
            if scalar_bytes.len() != X25519_PRIVATE_KEY_LENGTH {
                return Err(format!(
                    "x25519(): scalar must be {} bytes, got {}",
                    X25519_PRIVATE_KEY_LENGTH,
                    scalar_bytes.len()
                ));
            }
            let mut basepoint_array = [0u8; 32];
            basepoint_array.copy_from_slice(&basepoint_bytes[..32]);
            let mut scalar_array = [0u8; 32];
            scalar_array.copy_from_slice(&scalar_bytes[..32]);
            let result = x25519_scalar_mult(&scalar_array, &basepoint_array);
            // SEC-088: same all-zero rejection as the shared-secret
            // helpers — a caller passing a small-order basepoint would
            // otherwise get a known-constant result they could confuse
            // with a real DH shared secret.
            if x25519_is_small_order_output(&result) {
                return Err(
                    "x25519(): invalid basepoint (small-order point produced an all-zero result)"
                        .to_string(),
                );
            }
            Ok(bytes_to_value(&result))
        })),
    );

    // ed25519_keypair() -> {private: String, public: String}
    env.define(
        "ed25519_keypair".to_string(),
        Value::NativeFunction(NativeFunction::new("ed25519_keypair", Some(0), |_args| {
            let (private, public) = do_ed25519_keypair();
            Ok(hash_from_pairs([
                ("private".to_string(), Value::String(private.into())),
                ("public".to_string(), Value::String(public.into())),
            ]))
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_compare_equal_strings() {
        assert!(do_secure_compare("hello", "hello"));
        assert!(do_secure_compare("", ""));
        let hash = do_hmac_sha256("payload", "key").unwrap();
        assert!(do_secure_compare(&hash, &hash));
    }

    #[test]
    fn secure_compare_unequal_strings() {
        assert!(!do_secure_compare("hello", "world"));
        assert!(!do_secure_compare("hello", "Hello"));
        assert!(!do_secure_compare("foo", "foobar"));
        assert!(!do_secure_compare("foobar", "foo"));
    }

    #[test]
    fn secure_compare_unicode_safe() {
        assert!(do_secure_compare("café", "café"));
        assert!(!do_secure_compare("café", "cafe"));
    }

    // SEC-088 — X25519 must never panic on arbitrary 32-byte input.

    fn clamped_scalar(seed: u8) -> [u8; 32] {
        let mut s = [seed; 32];
        s[0] &= 248;
        s[31] &= 127;
        s[31] |= 64;
        s
    }

    #[test]
    fn x25519_scalar_mult_does_not_panic_on_arbitrary_inputs() {
        // Sweep a handful of "would have panicked" candidates: the
        // previous `to_edwards(0).unwrap()` failed for any 32-byte point
        // that didn't decode to an Edwards point with sign bit 0.
        // Every byte pattern below now produces a well-formed result
        // — the only contract we assert here is "no panic".
        let scalar = clamped_scalar(0x42);
        for point in [
            [0u8; 32],  // all-zeros (small-order)
            [0xff; 32], // all-ones (would unwind to_edwards on the old impl)
            [1u8; 32],  // smooth pattern
            [0x80; 32], // high bit on
            {
                // The standard X25519 basepoint (u=9) — should always work.
                let mut p = [0u8; 32];
                p[0] = 9;
                p
            },
        ] {
            let _ = x25519_scalar_mult(&scalar, &point);
        }
    }

    #[test]
    fn x25519_small_order_output_detected_for_zero_point() {
        // The all-zero point is a canonical small-order generator: any
        // scalar multiplication produces the all-zero output, which the
        // SEC-088 small-order guard rejects at the call site.
        let scalar = clamped_scalar(0x55);
        let zero_point = [0u8; 32];
        let shared = x25519_scalar_mult(&scalar, &zero_point);
        assert!(
            x25519_is_small_order_output(&shared),
            "all-zero point must produce all-zero shared secret"
        );
    }

    #[test]
    fn x25519_small_order_output_passes_for_real_keypair() {
        // Generate two real keypairs, derive both shared secrets, confirm
        // (a) they agree, (b) they're not all-zero. This is the
        // round-trip sanity check from the SEC-088 acceptance criteria.
        let (priv_a, pub_a) = do_x25519_keypair();
        let (priv_b, pub_b) = do_x25519_keypair();
        let priv_a_bytes = hex_to_bytes(&priv_a).unwrap();
        let priv_b_bytes = hex_to_bytes(&priv_b).unwrap();
        let pub_a_bytes = hex_to_bytes(&pub_a).unwrap();
        let pub_b_bytes = hex_to_bytes(&pub_b).unwrap();
        let mut sa = [0u8; 32];
        sa.copy_from_slice(&priv_a_bytes);
        let mut sb = [0u8; 32];
        sb.copy_from_slice(&priv_b_bytes);
        let mut pa = [0u8; 32];
        pa.copy_from_slice(&pub_a_bytes);
        let mut pb = [0u8; 32];
        pb.copy_from_slice(&pub_b_bytes);
        let secret_ab = x25519_scalar_mult(&sa, &pb);
        let secret_ba = x25519_scalar_mult(&sb, &pa);
        assert_eq!(secret_ab, secret_ba, "shared secret must be symmetric");
        assert!(
            !x25519_is_small_order_output(&secret_ab),
            "real keypair must not produce all-zero shared secret"
        );
    }

    // SEC-089 — TOTP helpers must not panic on zero / negative periods.

    fn crypto_static(name: &str) -> Rc<NativeFunction> {
        let mut env = Environment::new();
        register_crypto_builtins(&mut env);
        let crypto = env
            .get("Crypto")
            .unwrap_or_else(|| panic!("Crypto class not registered"));
        let class = match &crypto {
            Value::Class(c) => c.clone(),
            other => panic!("Crypto is not a Class: {:?}", other),
        };
        class
            .native_static_methods
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("Crypto.{} not registered", name))
    }

    #[test]
    fn totp_generate_rejects_zero_period() {
        let f = crypto_static("totp_generate");
        let err = (f.func)(vec![
            Value::String("JBSWY3DPEHPK3PXP".into()),
            Value::Int(1_700_000_000),
            Value::Int(0),
        ])
        .expect_err("period=0 must error, not panic with divide-by-zero");
        assert!(err.contains("period must be positive"), "{}", err);
    }

    #[test]
    fn totp_generate_rejects_negative_period() {
        // The previous `*p as u64` cast turned -30 into a huge positive
        // period value, producing wildly wrong codes silently.
        let f = crypto_static("totp_generate");
        let err = (f.func)(vec![
            Value::String("JBSWY3DPEHPK3PXP".into()),
            Value::Int(1_700_000_000),
            Value::Int(-30),
        ])
        .expect_err("negative period must error");
        assert!(err.contains("period must be positive"), "{}", err);
    }

    #[test]
    fn totp_generate_default_period_still_works() {
        // The 30-second default path (no period arg) must remain
        // unchanged.
        let f = crypto_static("totp_generate");
        let result = (f.func)(vec![
            Value::String("JBSWY3DPEHPK3PXP".into()),
            Value::Int(1_700_000_000),
        ])
        .expect("default 30s period must still produce a code");
        match result {
            Value::String(code) => assert_eq!(code.len(), 6, "expected 6-digit code, got {}", code),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn totp_verify_rejects_zero_period() {
        let f = crypto_static("totp_verify");
        let err = (f.func)(vec![
            Value::String("JBSWY3DPEHPK3PXP".into()),
            Value::String("000000".into()),
            Value::Int(1_700_000_000),
            Value::Int(0),
        ])
        .expect_err("period=0 must error before reaching the divide");
        assert!(err.contains("period must be positive"), "{}", err);
    }

    #[test]
    fn totp_verify_rejects_negative_period() {
        let f = crypto_static("totp_verify");
        let err = (f.func)(vec![
            Value::String("JBSWY3DPEHPK3PXP".into()),
            Value::String("000000".into()),
            Value::Int(1_700_000_000),
            Value::Int(-1),
        ])
        .expect_err("negative period must error");
        assert!(err.contains("period must be positive"), "{}", err);
    }

    #[test]
    fn totp_generate_then_verify_round_trip() {
        // Acceptance criterion: existing valid TOTP behaviour unchanged.
        let gen = crypto_static("totp_generate");
        let ver = crypto_static("totp_verify");
        let secret = Value::String("JBSWY3DPEHPK3PXP".into());
        let time = Value::Int(1_700_000_000);
        let code = match (gen.func)(vec![secret.clone(), time.clone()]).unwrap() {
            Value::String(s) => s,
            other => panic!("expected String code, got {:?}", other),
        };
        let valid = (ver.func)(vec![secret, Value::String(code), time]).unwrap();
        assert!(matches!(valid, Value::Bool(true)));
    }

    // ---- RSA primitives: modexp + PKCS#1 v1.5 ----

    #[test]
    fn modexp_small_known_value() {
        // 4^13 mod 497 = 445 (classic textbook modexp example).
        // 497 = 0x01F1, 445 = 0x01BD; modulus needs 9 bits -> k = 2 octets.
        let out = do_modexp(&[4], &[13], &[0x01, 0xF1]).unwrap();
        assert_eq!(out, vec![0x01, 0xBD]);
    }

    #[test]
    fn modexp_rejects_zero_modulus() {
        let err = do_modexp(&[2], &[3], &[0]).unwrap_err();
        assert!(err.contains("non-zero"), "{}", err);
    }

    #[test]
    fn modexp_rsa_roundtrip() {
        // Tiny RSA: p=61, q=53, n=3233, e=17, d=2753.
        // Message 65 -> cipher 65^17 mod 3233 = 2790 -> 2790^2753 mod 3233 = 65.
        let n = 3233u32.to_be_bytes();
        let cipher = do_modexp(&[65], &[17], &n[2..]).unwrap();
        let plain = do_modexp(&cipher, &2753u32.to_be_bytes()[2..], &n[2..]).unwrap();
        // k = ceil(12/8) = 2 octets.
        assert_eq!(plain, vec![0x00, 0x41]); // 0x41 == 65
    }

    #[test]
    fn pkcs1_type1_pad_unpad_roundtrip() {
        let data = b"hello";
        let em = do_pkcs1_pad(data, 64, 1).unwrap();
        assert_eq!(em.len(), 64);
        assert_eq!(em[0], 0x00);
        assert_eq!(em[1], 0x01);
        // All padding octets are 0xFF up to the separator.
        assert!(em[2..64 - data.len() - 1].iter().all(|&b| b == 0xFF));
        assert_eq!(em[64 - data.len() - 1], 0x00);
        let recovered = do_pkcs1_unpad(&em).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn pkcs1_type2_pad_is_random_nonzero_and_unpads() {
        let data = b"secret";
        let em = do_pkcs1_pad(data, 64, 2).unwrap();
        assert_eq!(em[1], 0x02);
        // Padding string (between BT and separator) must be all non-zero.
        let sep = em.iter().skip(2).position(|&b| b == 0).unwrap() + 2;
        assert!(sep >= 10, "PS must be >= 8 octets");
        assert!(em[2..sep].iter().all(|&b| b != 0));
        assert_eq!(do_pkcs1_unpad(&em).unwrap(), data);
    }

    #[test]
    fn pkcs1_pad_rejects_overlong_data() {
        // k=11 leaves room for 0 data octets (k-11); 1 octet must fail.
        let err = do_pkcs1_pad(b"x", 11, 1).unwrap_err();
        assert!(err.contains("too long"), "{}", err);
    }

    #[test]
    fn pkcs1_unpad_rejects_bad_prefix() {
        let mut em = do_pkcs1_pad(b"data", 32, 1).unwrap();
        em[0] = 0x01; // corrupt leading octet
        assert!(do_pkcs1_unpad(&em).is_err());
    }

    #[test]
    fn pkcs1_unpad_rejects_short_padding() {
        // 0x00 01 FF 00 <data...> — only 1 padding octet, below the 8 minimum.
        let em = [0x00, 0x01, 0xFF, 0x00, 0xAA, 0xBB];
        assert!(do_pkcs1_unpad(&em).is_err());
    }

    #[test]
    fn value_to_octets_treats_string_as_hex() {
        let v = Value::String("0x00ff10".into());
        assert_eq!(value_to_octets(&v, "test").unwrap(), vec![0x00, 0xff, 0x10]);
    }

    #[test]
    fn aes_bytes_round_trip_non_utf8() {
        let key = derive_aes_key(b"bundle-test-key");
        // Deliberately invalid UTF-8 — the bytes API must not care.
        let payload = vec![0u8, 159, 146, 150, 255, 0, 42];
        let sealed = aes_encrypt_bytes(&payload, &key).unwrap();
        assert_ne!(sealed[12..], payload[..]);
        assert_eq!(aes_decrypt_bytes(&sealed, &key).unwrap(), payload);
    }

    #[test]
    fn aes_bytes_wrong_key_fails() {
        let sealed = aes_encrypt_bytes(b"secret", &derive_aes_key(b"key-a")).unwrap();
        assert!(aes_decrypt_bytes(&sealed, &derive_aes_key(b"key-b")).is_err());
    }

    #[test]
    fn aes_string_delegates_to_bytes_container() {
        // The string API must still produce base64(nonce ‖ ct+tag) decodable
        // by the bytes API — the refactor must not have changed the wire.
        let key = derive_aes_key(b"shared");
        let encoded = aes_encrypt("hello", &key).unwrap();
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .unwrap();
        assert_eq!(aes_decrypt_bytes(&raw, &key).unwrap(), b"hello");
        assert_eq!(aes_decrypt(&encoded, &key).unwrap(), "hello");
    }
}
