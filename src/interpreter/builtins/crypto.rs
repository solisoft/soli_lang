//! Cryptographic built-in functions.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use curve25519_dalek::edwards::EdwardsPoint;
use curve25519_dalek::scalar::Scalar;
use rand_core::RngCore;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

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
    // Convert Edwards point to Montgomery form
    result.to_montgomery().0
}

/// Register cryptographic functions in the given environment.
pub fn register_crypto_builtins(env: &mut Environment) {
    // argon2_hash(password) -> String
    // Hashes a password using Argon2id (recommended variant)
    // Returns the hash in PHC string format (includes salt and parameters)
    env.define(
        "argon2_hash".to_string(),
        Value::NativeFunction(NativeFunction::new("argon2_hash", Some(1), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes(),
                other => {
                    return Err(format!(
                        "argon2_hash() expects string password, got {}",
                        other.type_name()
                    ))
                }
            };

            // Generate a random salt
            let salt = SaltString::generate(&mut OsRng);

            // Hash the password using Argon2id (default, recommended variant)
            let argon2 = Argon2::default();
            let password_hash = argon2
                .hash_password(password, &salt)
                .map_err(|e| format!("Failed to hash password: {}", e))?;

            Ok(Value::String(password_hash.to_string()))
        })),
    );

    // argon2_verify(password, hash) -> Bool
    // Verifies a password against an Argon2 hash
    // Returns true if the password matches, false otherwise
    env.define(
        "argon2_verify".to_string(),
        Value::NativeFunction(NativeFunction::new("argon2_verify", Some(2), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes(),
                other => {
                    return Err(format!(
                        "argon2_verify() expects string password as first argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let hash_str = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "argon2_verify() expects string hash as second argument, got {}",
                        other.type_name()
                    ))
                }
            };

            // Parse the hash string
            let parsed_hash = match PasswordHash::new(&hash_str) {
                Ok(h) => h,
                Err(e) => {
                    return Err(format!("Invalid hash format: {}", e));
                }
            };

            // Verify the password
            let argon2 = Argon2::default();
            let is_valid = argon2.verify_password(password, &parsed_hash).is_ok();

            Ok(Value::Bool(is_valid))
        })),
    );

    // password_hash(plain) -> hashed string
    // Alias for argon2_hash
    env.define(
        "password_hash".to_string(),
        Value::NativeFunction(NativeFunction::new("password_hash", Some(1), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes(),
                other => {
                    return Err(format!(
                        "password_hash() expects string password, got {}",
                        other.type_name()
                    ))
                }
            };

            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            let password_hash = argon2
                .hash_password(password, &salt)
                .map_err(|e| format!("Failed to hash password: {}", e))?;

            Ok(Value::String(password_hash.to_string()))
        })),
    );

    // password_verify(plain, hashed) -> bool
    // Alias for argon2_verify
    env.define(
        "password_verify".to_string(),
        Value::NativeFunction(NativeFunction::new("password_verify", Some(2), |args| {
            let password = match &args[0] {
                Value::String(s) => s.as_bytes(),
                other => {
                    return Err(format!(
                        "password_verify() expects string password as first argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let hash_str = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "password_verify() expects string hash as second argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let parsed_hash = match PasswordHash::new(&hash_str) {
                Ok(h) => h,
                Err(e) => {
                    return Err(format!("Invalid hash format: {}", e));
                }
            };

            let argon2 = Argon2::default();
            let is_valid = argon2.verify_password(password, &parsed_hash).is_ok();

            Ok(Value::Bool(is_valid))
        })),
    );

    // x25519_keypair() -> {private: String, public: String}
    // Generates a new X25519 key pair
    // Returns a hash with 'private' and 'public' keys as hex strings
    env.define(
        "x25519_keypair".to_string(),
        Value::NativeFunction(NativeFunction::new("x25519_keypair", Some(0), |_args| {
            let mut private_key = [0u8; X25519_PRIVATE_KEY_LENGTH];
            OsRng.fill_bytes(&mut private_key);

            // Clamp the private key (set bits 0, 1, 2 and 255 according to spec)
            private_key[0] &= 248;
            private_key[31] &= 127;
            private_key[31] |= 64;

            // Derive the public key by scalar multiplication with basepoint
            let public_key = x25519_scalar_mult(&private_key, &X25519_BASEPOINT_BYTES);

            let result = vec![
                (
                    Value::String("private".to_string()),
                    Value::String(bytes_to_hex(&private_key)),
                ),
                (
                    Value::String("public".to_string()),
                    Value::String(bytes_to_hex(&public_key)),
                ),
            ];

            Ok(Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
                result,
            ))))
        })),
    );

    // x25519_shared_secret(private_key, public_key) -> String
    // Computes a shared secret from a private key and a public key
    // private_key can be a 32-byte array or 64-char hex string
    // public_key can be a 32-byte array or 64-char hex string
    // Returns the shared secret as a hex string (32 bytes)
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

                // Clamp the private key
                let mut private_key = [0u8; X25519_PRIVATE_KEY_LENGTH];
                private_key.copy_from_slice(&private_bytes[..X25519_PRIVATE_KEY_LENGTH]);
                private_key[0] &= 248;
                private_key[31] &= 127;
                private_key[31] |= 64;

                // Compute shared secret using x25519 scalar multiplication
                let mut public_array = [0u8; 32];
                public_array.copy_from_slice(&public_bytes[..32]);
                let shared = x25519_scalar_mult(&private_key, &public_array);

                Ok(bytes_to_value(&shared))
            },
        )),
    );

    // x25519_public_key(private_key) -> String
    // Derives the public key from a private key
    // private_key can be a 32-byte array or 64-char hex string
    // Returns the public key as a hex string (32 bytes)
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

            // Clamp the private key
            let mut private_key = [0u8; X25519_PRIVATE_KEY_LENGTH];
            private_key.copy_from_slice(&private_bytes[..X25519_PRIVATE_KEY_LENGTH]);
            private_key[0] &= 248;
            private_key[31] &= 127;
            private_key[31] |= 64;

            // Derive the public key using X25519 basepoint
            let public_key = x25519_scalar_mult(&private_key, &X25519_BASEPOINT_BYTES);

            Ok(bytes_to_value(&public_key))
        })),
    );

    // x25519(basepoint, scalar) -> String
    // Performs X25519 scalar multiplication (for advanced use)
    // basepoint: 32-byte point as hex string or array
    // scalar: 32-byte scalar as hex string or array
    // Returns the result as a hex string
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
    // Generates a new Ed25519 key pair for digital signatures
    // Returns a hash with 'private' and 'public' keys as hex strings
    env.define(
        "ed25519_keypair".to_string(),
        Value::NativeFunction(NativeFunction::new("ed25519_keypair", Some(0), |_args| {
            let mut seed = [0u8; 32];
            OsRng.fill_bytes(&mut seed);
            let scalar = Scalar::from_bytes_mod_order(seed);
            let public_key = EdwardsPoint::mul_base(&scalar).compress().to_bytes();

            let result = vec![
                (
                    Value::String("private".to_string()),
                    Value::String(bytes_to_hex(&seed)),
                ),
                (
                    Value::String("public".to_string()),
                    Value::String(bytes_to_hex(&public_key)),
                ),
            ];

            Ok(Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
                result,
            ))))
        })),
    );
}
