//! JWT (JSON Web Token) support for Solilang.
//!
//! Provides functions for signing and verifying JWTs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Algorithm, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// JWT claims structure.
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    /// Subject (usually user ID)
    #[serde(skip_serializing_if = "Option::is_none")]
    sub: Option<String>,
    /// Expiration time (Unix timestamp)
    #[serde(skip_serializing_if = "Option::is_none")]
    exp: Option<u64>,
    /// Issued at (Unix timestamp)
    #[serde(skip_serializing_if = "Option::is_none")]
    iat: Option<u64>,
    /// Custom payload data
    #[serde(flatten)]
    data: HashMap<String, JsonValue>,
}

// Use centralized conversion functions from value module
use crate::interpreter::value::{json_to_value, value_to_json};

/// Get current Unix timestamp.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Register JWT builtins in the given environment.
pub fn register_jwt_builtins(env: &mut Environment) {
    // jwt_sign(payload, secret, options?) -> token string
    env.define(
        "jwt_sign".to_string(),
        Value::NativeFunction(NativeFunction::new("jwt_sign", None, |args| {
            if args.len() < 2 {
                return Err("jwt_sign() requires at least payload and secret".to_string());
            }

            let payload = &args[0];
            let secret = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "jwt_sign() expects string secret, got {}",
                        other.type_name()
                    ))
                }
            };

            // Parse options
            let mut expires_in: Option<u64> = None;
            let mut algorithm = Algorithm::HS256;

            if args.len() > 2 {
                if let Value::Hash(opts) = &args[2] {
                    for (k, v) in opts.borrow().iter() {
                        if let Value::String(key) = k {
                            match key.as_str() {
                                "expires_in" => {
                                    if let Value::Int(secs) = v {
                                        expires_in = Some(*secs as u64);
                                    }
                                }
                                "algorithm" => {
                                    if let Value::String(alg) = v {
                                        algorithm = match alg.as_str() {
                                            "HS256" => Algorithm::HS256,
                                            "HS384" => Algorithm::HS384,
                                            "HS512" => Algorithm::HS512,
                                            _ => {
                                                return Err(format!(
                                                    "Unsupported algorithm: {}",
                                                    alg
                                                ))
                                            }
                                        };
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            // Build claims
            let mut data = HashMap::new();
            if let Value::Hash(hash) = payload {
                for (k, v) in hash.borrow().iter() {
                    if let Value::String(key) = k {
                        // Skip reserved claims
                        if key != "exp" && key != "iat" && key != "sub" {
                            data.insert(key.clone(), value_to_json(v)?);
                        }
                    }
                }
            }

            let now = current_timestamp();
            let claims = Claims {
                sub: extract_string_claim(payload, "sub"),
                exp: expires_in.map(|secs| now + secs),
                iat: Some(now),
                data,
            };

            // Create token
            let header = Header::new(algorithm);
            let token = encode(&header, &claims, &EncodingKey::from_secret(secret.as_bytes()))
                .map_err(|e| format!("Failed to create JWT: {}", e))?;

            Ok(Value::String(token))
        })),
    );

    // jwt_verify(token, secret) -> payload hash or error hash
    env.define(
        "jwt_verify".to_string(),
        Value::NativeFunction(NativeFunction::new("jwt_verify", Some(2), |args| {
            let token = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "jwt_verify() expects string token, got {}",
                        other.type_name()
                    ))
                }
            };

            let secret = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "jwt_verify() expects string secret, got {}",
                        other.type_name()
                    ))
                }
            };

            // Try to decode and verify the token
            let mut validation = Validation::default();
            validation.validate_exp = true;

            match decode::<Claims>(
                &token,
                &DecodingKey::from_secret(secret.as_bytes()),
                &validation,
            ) {
                Ok(token_data) => {
                    // Convert claims to Soli Value
                    let claims = token_data.claims;
                    let mut pairs: Vec<(Value, Value)> = Vec::new();

                    if let Some(sub) = claims.sub {
                        pairs.push((Value::String("sub".to_string()), Value::String(sub)));
                    }
                    if let Some(exp) = claims.exp {
                        pairs.push((Value::String("exp".to_string()), Value::Int(exp as i64)));
                    }
                    if let Some(iat) = claims.iat {
                        pairs.push((Value::String("iat".to_string()), Value::Int(iat as i64)));
                    }

                    // Add custom data
                    for (key, value) in claims.data {
                        pairs.push((Value::String(key), json_to_value(&value)?));
                    }

                    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
                }
                Err(e) => {
                    // Return error hash instead of throwing
                    let error_pairs: Vec<(Value, Value)> = vec![
                        (Value::String("error".to_string()), Value::Bool(true)),
                        (
                            Value::String("message".to_string()),
                            Value::String(format!("{}", e)),
                        ),
                    ];
                    Ok(Value::Hash(Rc::new(RefCell::new(error_pairs))))
                }
            }
        })),
    );

    // jwt_decode(token) -> payload hash (without verification)
    env.define(
        "jwt_decode".to_string(),
        Value::NativeFunction(NativeFunction::new("jwt_decode", Some(1), |args| {
            let token = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "jwt_decode() expects string token, got {}",
                        other.type_name()
                    ))
                }
            };

            // Decode without verification
            let mut validation = Validation::default();
            validation.insecure_disable_signature_validation();
            validation.validate_exp = false;

            match decode::<Claims>(
                &token,
                &DecodingKey::from_secret(&[]), // Empty key since we're not verifying
                &validation,
            ) {
                Ok(token_data) => {
                    let claims = token_data.claims;
                    let mut pairs: Vec<(Value, Value)> = Vec::new();

                    if let Some(sub) = claims.sub {
                        pairs.push((Value::String("sub".to_string()), Value::String(sub)));
                    }
                    if let Some(exp) = claims.exp {
                        pairs.push((Value::String("exp".to_string()), Value::Int(exp as i64)));
                    }
                    if let Some(iat) = claims.iat {
                        pairs.push((Value::String("iat".to_string()), Value::Int(iat as i64)));
                    }

                    for (key, value) in claims.data {
                        pairs.push((Value::String(key), json_to_value(&value)?));
                    }

                    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
                }
                Err(e) => {
                    let error_pairs: Vec<(Value, Value)> = vec![
                        (Value::String("error".to_string()), Value::Bool(true)),
                        (
                            Value::String("message".to_string()),
                            Value::String(format!("{}", e)),
                        ),
                    ];
                    Ok(Value::Hash(Rc::new(RefCell::new(error_pairs))))
                }
            }
        })),
    );
}

/// Extract a string claim from a payload hash.
fn extract_string_claim(payload: &Value, key: &str) -> Option<String> {
    if let Value::Hash(hash) = payload {
        for (k, v) in hash.borrow().iter() {
            if let Value::String(k_str) = k {
                if k_str == key {
                    if let Value::String(s) = v {
                        return Some(s.clone());
                    }
                }
            }
        }
    }
    None
}
