//! JWT (JSON Web Token) support for Solilang.
//!
//! Provides functions for signing and verifying JWTs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};

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

            // Enforce minimum secret length for security
            const MIN_SECRET_LENGTH: usize = 16;
            if secret.len() < MIN_SECRET_LENGTH {
                return Err(format!(
                    "jwt_sign() secret must be at least {} characters for security (got {})",
                    MIN_SECRET_LENGTH,
                    secret.len()
                ));
            }

            // Parse options
            let mut expires_in: Option<u64> = None;
            let mut algorithm = Algorithm::HS256;

            if args.len() > 2 {
                if let Value::Hash(opts) = &args[2] {
                    for (k, v) in opts.borrow().iter() {
                        if let HashKey::String(key) = k {
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
                    if let HashKey::String(key) = k {
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
            let token = encode(
                &header,
                &claims,
                &EncodingKey::from_secret(secret.as_bytes()),
            )
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

            // Enforce minimum secret length for security
            const MIN_SECRET_LENGTH: usize = 16;
            if secret.len() < MIN_SECRET_LENGTH {
                return Err(format!(
                    "jwt_verify() secret must be at least {} characters for security (got {})",
                    MIN_SECRET_LENGTH,
                    secret.len()
                ));
            }

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
                    let mut pairs: HashPairs = HashPairs::default();

                    if let Some(sub) = claims.sub {
                        pairs.insert(HashKey::String("sub".to_string()), Value::String(sub));
                    }
                    if let Some(exp) = claims.exp {
                        pairs.insert(HashKey::String("exp".to_string()), Value::Int(exp as i64));
                    }
                    if let Some(iat) = claims.iat {
                        pairs.insert(HashKey::String("iat".to_string()), Value::Int(iat as i64));
                    }

                    // Add custom data
                    for (key, value) in claims.data {
                        pairs.insert(HashKey::String(key), json_to_value(value)?);
                    }

                    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
                }
                Err(e) => {
                    // Return error hash instead of throwing
                    let mut error_pairs: HashPairs = HashPairs::default();
                    error_pairs.insert(HashKey::String("error".to_string()), Value::Bool(true));
                    error_pairs.insert(
                        HashKey::String("message".to_string()),
                        Value::String(format!("{}", e)),
                    );
                    Ok(Value::Hash(Rc::new(RefCell::new(error_pairs))))
                }
            }
        })),
    );

    // SEC-029: `jwt_decode_unsafe` — decode WITHOUT verification.
    //
    // Returns `{unverified: true, claims: {...}}` so the caller cannot
    // pattern-match on `result["sub"]` and accidentally trust an
    // attacker-forged claim. The previous `jwt_decode` returned the same
    // shape as a verified `jwt_verify`, which is a silent footgun: any
    // controller that did `let claims = jwt_decode(token); user_id =
    // claims["sub"]` was fully bypassable.
    //
    // Use this only for inspection / debugging. For auth, use
    // `jwt_verify(token, secret)`.
    env.define(
        "jwt_decode_unsafe".to_string(),
        Value::NativeFunction(NativeFunction::new("jwt_decode_unsafe", Some(1), |args| {
            let token = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "jwt_decode_unsafe() expects string token, got {}",
                        other.type_name()
                    ))
                }
            };

            // Decode without verification
            let mut validation = Validation::default();
            validation.insecure_disable_signature_validation();
            validation.validate_exp = false;
            // SEC-029: `Validation::default()` requires `exp` to be present
            // even when `validate_exp = false`. For an inspection helper we
            // accept tokens without `exp` too — otherwise tokens minted by
            // `jwt_sign(..., {expires_in: 0})` or by other libraries would
            // be unreadable here.
            validation.required_spec_claims.clear();

            match decode::<Claims>(
                &token,
                &DecodingKey::from_secret(&[]), // Empty key since we're not verifying
                &validation,
            ) {
                Ok(token_data) => {
                    let claims = token_data.claims;
                    let mut claims_pairs: HashPairs = HashPairs::default();

                    if let Some(sub) = claims.sub {
                        claims_pairs.insert(HashKey::String("sub".to_string()), Value::String(sub));
                    }
                    if let Some(exp) = claims.exp {
                        claims_pairs
                            .insert(HashKey::String("exp".to_string()), Value::Int(exp as i64));
                    }
                    if let Some(iat) = claims.iat {
                        claims_pairs
                            .insert(HashKey::String("iat".to_string()), Value::Int(iat as i64));
                    }

                    for (key, value) in claims.data {
                        claims_pairs.insert(HashKey::String(key), json_to_value(value)?);
                    }

                    // Wrap the claims in an outer hash that names them as
                    // unverified. Code that mistakenly does
                    // `result["sub"]` now reads `null`, not a forged claim.
                    let mut pairs: HashPairs = HashPairs::default();
                    pairs.insert(HashKey::String("unverified".to_string()), Value::Bool(true));
                    pairs.insert(
                        HashKey::String("claims".to_string()),
                        Value::Hash(Rc::new(RefCell::new(claims_pairs))),
                    );
                    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
                }
                Err(e) => {
                    let mut error_pairs: HashPairs = HashPairs::default();
                    error_pairs.insert(HashKey::String("error".to_string()), Value::Bool(true));
                    error_pairs.insert(
                        HashKey::String("message".to_string()),
                        Value::String(format!("{}", e)),
                    );
                    Ok(Value::Hash(Rc::new(RefCell::new(error_pairs))))
                }
            }
        })),
    );

    // SEC-029: `jwt_decode` is removed. The old shape was identical to a
    // verified `jwt_verify` result, which made `let claims =
    // jwt_decode(token); user_id = claims["sub"]` a silent
    // authentication bypass. Existing callers must migrate to
    // `jwt_verify(token, secret)` (verified path) or
    // `jwt_decode_unsafe(token)` (returns `{unverified: true, claims}`).
    env.define(
        "jwt_decode".to_string(),
        Value::NativeFunction(NativeFunction::new("jwt_decode", None, |_args| {
            Err(
                "jwt_decode() has been removed (SEC-029). It returned the same shape as a verified jwt_verify(), \
                 making `claims[\"sub\"]` a silent auth bypass. Use jwt_verify(token, secret) for authenticated reads, \
                 or jwt_decode_unsafe(token) for inspection (returns `{unverified: true, claims: {...}}`)."
                    .to_string(),
            )
        })),
    );
}

/// Extract a string claim from a payload hash.
fn extract_string_claim(payload: &Value, key: &str) -> Option<String> {
    if let Value::Hash(hash) = payload {
        for (k, v) in hash.borrow().iter() {
            if let HashKey::String(k_str) = k {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt_fn(env: &Environment, name: &str) -> NativeFunction {
        match env.get(name) {
            Some(Value::NativeFunction(f)) => f.clone(),
            other => panic!("expected NativeFunction for {name}, got {other:?}"),
        }
    }

    fn fresh_env() -> Environment {
        let mut env = Environment::new();
        register_jwt_builtins(&mut env);
        env
    }

    /// SEC-029: a token signed with a known secret can be inspected via
    /// `jwt_decode_unsafe` and the result is wrapped as
    /// `{unverified: true, claims: {...}}` — distinct from `jwt_verify`.
    #[test]
    fn jwt_decode_unsafe_returns_wrapped_shape() {
        let env = fresh_env();
        let sign = jwt_fn(&env, "jwt_sign");
        let decode = jwt_fn(&env, "jwt_decode_unsafe");

        // Build a payload {sub: "alice", role: "admin"} and sign it.
        let mut payload: HashPairs = HashPairs::default();
        payload.insert(
            HashKey::String("sub".to_string()),
            Value::String("alice".to_string()),
        );
        payload.insert(
            HashKey::String("role".to_string()),
            Value::String("admin".to_string()),
        );
        let payload_hash = Value::Hash(Rc::new(RefCell::new(payload)));
        let token = (sign.func)(vec![
            payload_hash,
            Value::String("a-very-long-secret-here".to_string()),
        ])
        .unwrap();
        let token_str = match token {
            Value::String(s) => s,
            other => panic!("expected token string, got {other:?}"),
        };

        let result = (decode.func)(vec![Value::String(token_str)]).unwrap();
        let outer = match result {
            Value::Hash(h) => h,
            other => panic!("expected hash result, got {other:?}"),
        };
        let outer_borrow = outer.borrow();

        // Outer shape is `{unverified: true, claims: {...}}`.
        assert!(matches!(
            outer_borrow.get(&HashKey::String("unverified".to_string())),
            Some(Value::Bool(true))
        ));
        let claims = match outer_borrow.get(&HashKey::String("claims".to_string())) {
            Some(Value::Hash(c)) => c.clone(),
            other => panic!("expected nested claims hash, got {other:?}"),
        };
        let claims_borrow = claims.borrow();
        // Claims are reachable via the wrapper but NOT at the top level.
        assert!(matches!(
            claims_borrow.get(&HashKey::String("sub".to_string())),
            Some(Value::String(s)) if s == "alice"
        ));
        assert!(outer_borrow
            .get(&HashKey::String("sub".to_string()))
            .is_none());
    }

    /// SEC-029: the bare `jwt_decode` builtin is removed and points
    /// callers at the safe alternatives.
    #[test]
    fn jwt_decode_returns_migration_error() {
        let env = fresh_env();
        let decode = jwt_fn(&env, "jwt_decode");

        let err = (decode.func)(vec![Value::String("anything".to_string())]).unwrap_err();
        assert!(
            err.contains("SEC-029")
                && err.contains("jwt_decode_unsafe")
                && err.contains("jwt_verify"),
            "expected SEC-029 migration error pointing at both alternatives, got: {}",
            err
        );
    }
}
