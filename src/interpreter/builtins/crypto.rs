//! Cryptographic built-in functions and Crypto class.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use curve25519_dalek::edwards::EdwardsPoint;
use curve25519_dalek::scalar::Scalar;
use hmac::{Hmac, Mac};
use md5::Md5;
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
    Value::String(bytes_to_hex(bytes))
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

/// Register cryptographic functions and Crypto class in the given environment.
pub fn register_crypto_builtins(env: &mut Environment) {
    // Build Crypto static methods
    let mut crypto_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

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
            Ok(Value::String(do_sha256(&data)))
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
            Ok(Value::String(do_sha512(&data)))
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
            Ok(Value::String(do_md5(&data)))
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
            Ok(Value::String(result))
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
            Ok(Value::String(hash))
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
                Ok(Value::String(hash))
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
                    ("private".to_string(), Value::String(private)),
                    ("public".to_string(), Value::String(public)),
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
                    ("private".to_string(), Value::String(private)),
                    ("public".to_string(), Value::String(public)),
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
            Ok(Value::String(code))
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

            Ok(Value::String(uri))
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
            Ok(Value::String(do_sha256(&data)))
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
            Ok(Value::String(do_sha512(&data)))
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
            Ok(Value::String(do_md5(&data)))
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
            Ok(Value::String(result))
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
            Ok(Value::String(hash))
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
            Ok(Value::String(hash))
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
                ("private".to_string(), Value::String(private)),
                ("public".to_string(), Value::String(public)),
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
                ("private".to_string(), Value::String(private)),
                ("public".to_string(), Value::String(public)),
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
            Value::String("JBSWY3DPEHPK3PXP".to_string()),
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
            Value::String("JBSWY3DPEHPK3PXP".to_string()),
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
            Value::String("JBSWY3DPEHPK3PXP".to_string()),
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
            Value::String("JBSWY3DPEHPK3PXP".to_string()),
            Value::String("000000".to_string()),
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
            Value::String("JBSWY3DPEHPK3PXP".to_string()),
            Value::String("000000".to_string()),
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
        let secret = Value::String("JBSWY3DPEHPK3PXP".to_string());
        let time = Value::Int(1_700_000_000);
        let code = match (gen.func)(vec![secret.clone(), time.clone()]).unwrap() {
            Value::String(s) => s,
            other => panic!("expected String code, got {:?}", other),
        };
        let valid = (ver.func)(vec![secret, Value::String(code), time]).unwrap();
        assert!(matches!(valid, Value::Bool(true)));
    }
}
