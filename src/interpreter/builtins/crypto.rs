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

/// Perform X25519 scalar multiplication (Montgomery curve)
fn x25519_scalar_mult(scalar: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    use curve25519_dalek::montgomery::MontgomeryPoint;
    use curve25519_dalek::traits::MultiscalarMul;

    let scalar_val = Scalar::from_bytes_mod_order(*scalar);
    let mont_point = MontgomeryPoint(*point);
    let ed_point = mont_point.to_edwards(0).unwrap();
    let result = EdwardsPoint::multiscalar_mul([scalar_val], [ed_point]);
    result.to_montgomery().0
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
    let expected = do_totp_generate(secret, time, period)?;
    Ok(expected == code_str)
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
                    Value::Int(p) => *p as u64,
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
                    Value::Int(p) => *p as u64,
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
        methods: HashMap::new(),
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
