//! Cryptographic built-in functions.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

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
}
