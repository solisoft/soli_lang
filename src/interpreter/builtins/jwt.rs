//! JWT (JSON Web Token) support for Solilang.
//!
//! Provides functions for signing and verifying JWTs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{
    decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
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

const MIN_SECRET_BYTES: usize = 32;

/// Read an option that accepts either a single string or an array of strings
/// (`aud` and `iss` are both defined that way in RFC 7519).
fn string_list_option(value: &Value, func: &str, option: &str) -> Result<Vec<String>, String> {
    match value {
        Value::String(s) => Ok(vec![s.to_string()]),
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.to_string()),
                other => Err(format!(
                    "{}() `{}` array expects strings, got {}",
                    func,
                    option,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{}() `{}` expects string or array of strings, got {}",
            func,
            option,
            other.type_name()
        )),
    }
}

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
            let mut pem_key: Option<String> = None;
            let mut kid: Option<String> = None;
            let mut typ: Option<String> = None;
            let mut absolute_exp: Option<i64> = None;
            let mut not_before: Option<i64> = None;
            let mut audience: Option<JsonValue> = None;
            let mut issuer: Option<String> = None;
            let mut jwt_id: Option<String> = None;

            if args.len() > 2 {
                if let Value::Hash(opts) = &args[2] {
                    for (k, v) in opts.borrow().iter() {
                        if let HashKey::String(key) = k {
                            match key.as_ref() {
                                "expires_in" => {
                                    if let Value::Int(secs) = v {
                                        expires_in = Some(*secs as u64);
                                    }
                                }
                                "algorithm" => {
                                    if let Value::String(alg) = v {
                                        algorithm = match alg.as_ref() {
                                            "HS256" => Algorithm::HS256,
                                            "HS384" => Algorithm::HS384,
                                            "HS512" => Algorithm::HS512,
                                            "RS256" => Algorithm::RS256,
                                            "EdDSA" => Algorithm::EdDSA,
                                            _ => {
                                                return Err(format!(
                                                    "Unsupported algorithm: {}",
                                                    alg
                                                ))
                                            }
                                        };
                                    }
                                }
                                "key" => {
                                    if let Value::String(k) = v {
                                        pem_key = Some(k.clone().to_string());
                                    }
                                }
                                // Header parameters. `kid` lets a verifier pick
                                // the right key out of a JWKS, which is what
                                // makes key rotation possible at all.
                                "kid" => {
                                    if let Value::String(k) = v {
                                        kid = Some(k.to_string());
                                    }
                                }
                                "typ" => {
                                    if let Value::String(t) = v {
                                        typ = Some(t.to_string());
                                    }
                                }
                                // Registered claims. `exp` here is an absolute
                                // Unix timestamp, unlike the relative `expires_in`.
                                "exp" => {
                                    if let Value::Int(ts) = v {
                                        absolute_exp = Some(*ts);
                                    }
                                }
                                "nbf" => {
                                    if let Value::Int(ts) = v {
                                        not_before = Some(*ts);
                                    }
                                }
                                // RFC 7519 §4.1.3 allows `aud` to be a single
                                // string or an array of them.
                                "aud" => {
                                    let auds = string_list_option(v, "jwt_sign", "aud")?;
                                    audience = Some(match auds.len() {
                                        1 => JsonValue::String(auds[0].clone()),
                                        _ => JsonValue::Array(
                                            auds.into_iter().map(JsonValue::String).collect(),
                                        ),
                                    });
                                }
                                "iss" => {
                                    if let Value::String(s) = v {
                                        issuer = Some(s.to_string());
                                    }
                                }
                                "jti" => {
                                    if let Value::String(s) = v {
                                        jwt_id = Some(s.to_string());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            if absolute_exp.is_some() && expires_in.is_some() {
                return Err("jwt_sign() accepts either `exp` (absolute timestamp) or \
                            `expires_in` (seconds from now), not both"
                    .to_string());
            }

            // SEC-054: enforce minimum secret length for HMAC algorithms.
            // Asymmetric algorithms (RS256, EdDSA) use PEM keys and are exempt.
            let is_asymmetric = matches!(algorithm, Algorithm::RS256 | Algorithm::EdDSA);
            if !is_asymmetric && secret.len() < MIN_SECRET_BYTES {
                return Err(format!(
                    "jwt_sign() secret must be at least {} bytes for security (got {}); \
                     load a high-entropy value from .env, e.g. `JWT_SECRET=$(openssl rand -hex 32)`",
                    MIN_SECRET_BYTES,
                    secret.len()
                ));
            }

            // Build claims as a raw JSON object rather than a typed struct, so
            // `aud` can serialize as either a string or an array. Custom claims
            // still come from the payload and registered ones from the options,
            // which keeps the split unambiguous.
            let mut claims = serde_json::Map::new();
            if let Value::Hash(hash) = payload {
                for (k, v) in hash.borrow().iter() {
                    if let HashKey::String(key) = k {
                        // Skip reserved claims
                        if **key != *"exp" && **key != *"iat" && **key != *"sub" {
                            claims.insert(key.to_string(), value_to_json(v)?);
                        }
                    }
                }
            }

            let now = current_timestamp();
            if let Some(sub) = extract_string_claim(payload, "sub") {
                claims.insert("sub".to_string(), JsonValue::String(sub));
            }
            claims.insert("iat".to_string(), JsonValue::from(now));
            if let Some(exp) = absolute_exp {
                claims.insert("exp".to_string(), JsonValue::from(exp));
            } else if let Some(secs) = expires_in {
                claims.insert("exp".to_string(), JsonValue::from(now + secs));
            }
            if let Some(nbf) = not_before {
                claims.insert("nbf".to_string(), JsonValue::from(nbf));
            }
            // Options win over any same-named key carried in the payload.
            if let Some(aud) = audience {
                claims.insert("aud".to_string(), aud);
            }
            if let Some(iss) = issuer {
                claims.insert("iss".to_string(), JsonValue::String(iss));
            }
            if let Some(jti) = jwt_id {
                claims.insert("jti".to_string(), JsonValue::String(jti));
            }

            // Create token
            let mut header = Header::new(algorithm);
            header.kid = kid;
            if let Some(t) = typ {
                header.typ = Some(t);
            }
            let encoding_key = build_encoding_key(&algorithm, &secret, pem_key.as_deref())?;
            let token = encode(&header, &claims, &encoding_key)
                .map_err(|e| format!("Failed to create JWT: {}", e))?;

            Ok(Value::String(token.into()))
        })),
    );

    // jwt_verify(token, secret, options?) -> payload hash or error hash.
    // Arity is variable so the optional `options` hash (`{key: ..., algorithm: ...}`)
    // is reachable — the previous `Some(2)` made the options path dead code.
    env.define(
        "jwt_verify".to_string(),
        Value::NativeFunction(NativeFunction::new("jwt_verify", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "jwt_verify() expects 2 or 3 arguments (token, secret, options?), got {}",
                    args.len()
                ));
            }
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

            // Pull out the optional PEM key + explicit expected algorithm.
            // SEC-091: the verifier — not the token — picks which algorithm
            // is acceptable. Without that, an attacker who knew the verifier's
            // RSA public key could sign an HS256 token using the public key
            // bytes as an HMAC secret and have it verified.
            let mut pem_key: Option<String> = None;
            let mut expected_algorithm: Option<Algorithm> = None;
            let mut expected_audience: Option<Vec<String>> = None;
            let mut expected_issuer: Option<Vec<String>> = None;
            let mut expected_subject: Option<String> = None;
            let mut leeway: Option<u64> = None;
            if args.len() > 2 {
                if let Value::Hash(opts) = &args[2] {
                    for (k, v) in opts.borrow().iter() {
                        if let HashKey::String(key) = k {
                            match key.as_ref() {
                                "key" => {
                                    if let Value::String(k) = v {
                                        pem_key = Some(k.clone().to_string());
                                    }
                                }
                                "audience" => {
                                    expected_audience =
                                        Some(string_list_option(v, "jwt_verify", "audience")?);
                                }
                                "issuer" => {
                                    expected_issuer =
                                        Some(string_list_option(v, "jwt_verify", "issuer")?);
                                }
                                "subject" => {
                                    if let Value::String(s) = v {
                                        expected_subject = Some(s.to_string());
                                    }
                                }
                                "leeway" => {
                                    if let Value::Int(secs) = v {
                                        leeway = Some((*secs).max(0) as u64);
                                    }
                                }
                                "algorithm" => {
                                    if let Value::String(alg) = v {
                                        expected_algorithm = Some(match alg.as_ref() {
                                            "HS256" => Algorithm::HS256,
                                            "HS384" => Algorithm::HS384,
                                            "HS512" => Algorithm::HS512,
                                            "RS256" => Algorithm::RS256,
                                            "EdDSA" => Algorithm::EdDSA,
                                            _ => {
                                                return Err(format!(
                                                    "jwt_verify() unsupported algorithm: {}",
                                                    alg
                                                ))
                                            }
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            // SEC-091: decide the allowed algorithm set up-front, never from
            // the token header.
            // - explicit `algorithm` option → only that algorithm.
            // - PEM key provided, no explicit alg → asymmetric only
            //   (RS256 / EdDSA). This is the case the algorithm-confusion
            //   attack exploited: caller meant "verify with this RSA public
            //   key", attacker switched to HS256 against the same bytes.
            // - 2-arg form, no PEM, no explicit alg → HMAC only (back-compat
            //   for the common `jwt_verify(token, secret)` callers).
            let allowed_algorithms: Vec<Algorithm> = match (expected_algorithm, &pem_key) {
                (Some(alg), _) => vec![alg],
                (None, Some(_)) => vec![Algorithm::RS256, Algorithm::EdDSA],
                (None, None) => {
                    vec![Algorithm::HS256, Algorithm::HS384, Algorithm::HS512]
                }
            };

            let header_carries_hmac = |algs: &[Algorithm]| {
                algs.iter()
                    .any(|a| !matches!(a, Algorithm::RS256 | Algorithm::EdDSA))
            };

            // SEC-054: enforce the HMAC secret floor when an HMAC algorithm
            // is in the allowed set. A weak secret must be a hard reject
            // regardless of how structurally valid the token is — otherwise
            // a junk token would mask a misconfigured production secret.
            if header_carries_hmac(&allowed_algorithms) && secret.len() < MIN_SECRET_BYTES {
                return Err(format!(
                    "jwt_verify() secret must be at least {} bytes for security (got {})",
                    MIN_SECRET_BYTES,
                    secret.len()
                ));
            }

            // Decode the header — but only to enforce a match against the
            // allow-list. The header's `alg` never picks the validation
            // algorithm on its own.
            let token_header =
                decode_header(&token).map_err(|e| format!("Failed to parse JWT header: {}", e))?;
            let token_alg = token_header.alg;

            if !allowed_algorithms.contains(&token_alg) {
                let expected = allowed_algorithms
                    .iter()
                    .map(|a| format!("{:?}", a))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "jwt_verify(): token algorithm {:?} does not match expected ({})",
                    token_alg, expected
                ));
            }

            // Configure validation pinned to the (now-trusted) algorithm.
            let mut validation = Validation::new(token_alg);
            validation.validate_exp = true;
            // SEC: also reject tokens presented before their `nbf` (not-before)
            // instant. Without this a token minted for future use is accepted
            // early.
            validation.validate_nbf = true;

            // `aud` is only checked when the caller says what it expects.
            //
            // jsonwebtoken defaults `validate_aud` to true, which makes it
            // reject *any* token carrying an `aud` claim when no expected
            // audience was configured — so every OIDC id_token failed here
            // with `InvalidAudience`. Audience is a caller-supplied policy,
            // exactly like `iss`, and is opt-in for the same reason.
            //
            // Note: with several expected audiences jsonwebtoken requires the
            // token to carry *all* of them (subset semantics), not any one.
            match &expected_audience {
                Some(auds) => {
                    validation.set_audience(auds);
                    validation.required_spec_claims.insert("aud".to_string());
                }
                None => validation.validate_aud = false,
            }
            if let Some(issuers) = &expected_issuer {
                validation.set_issuer(issuers);
                validation.required_spec_claims.insert("iss".to_string());
            }
            if let Some(subject) = expected_subject {
                validation.sub = Some(subject);
            }
            if let Some(secs) = leeway {
                validation.leeway = secs;
            }

            // Try to decode and verify the token.
            let decoding_key = build_decoding_key(&token_alg, &secret, pem_key.as_deref())?;
            match decode::<Claims>(&token, &decoding_key, &validation) {
                Ok(token_data) => {
                    // Convert claims to Soli Value
                    let claims = token_data.claims;
                    let mut pairs: HashPairs = HashPairs::default();

                    if let Some(sub) = claims.sub {
                        pairs.insert(HashKey::String("sub".into()), Value::String(sub.into()));
                    }
                    if let Some(exp) = claims.exp {
                        pairs.insert(HashKey::String("exp".into()), Value::Int(exp as i64));
                    }
                    if let Some(iat) = claims.iat {
                        pairs.insert(HashKey::String("iat".into()), Value::Int(iat as i64));
                    }

                    // Add custom data
                    for (key, value) in claims.data {
                        pairs.insert(HashKey::String(key.into()), json_to_value(value)?);
                    }

                    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
                }
                Err(e) => {
                    // Return error hash instead of throwing (deliberate, tested,
                    // documented contract). CALLERS MUST branch on
                    // `result["error"]` — do NOT write `if jwt_verify(...)`: the
                    // returned hash is truthy on failure too, so a bare truthiness
                    // check would treat a *failed* verification as authenticated.
                    // See www/docs/authentication.md for the correct pattern.
                    let mut error_pairs: HashPairs = HashPairs::default();
                    error_pairs.insert(HashKey::String("error".into()), Value::Bool(true));
                    error_pairs.insert(
                        HashKey::String("message".into()),
                        Value::String(format!("{}", e).into()),
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
            // Likewise, `Validation::default()` validates `aud`, which made
            // this helper reject every token carrying an audience — including
            // every OIDC id_token. Checking audience inside an explicitly
            // unverified inspection helper is meaningless anyway.
            validation.validate_aud = false;

            match decode::<Claims>(
                &token,
                &DecodingKey::from_secret(&[]), // Empty key since we're not verifying
                &validation,
            ) {
                Ok(token_data) => {
                    let claims = token_data.claims;
                    let mut claims_pairs: HashPairs = HashPairs::default();

                    if let Some(sub) = claims.sub {
                        claims_pairs
                            .insert(HashKey::String("sub".into()), Value::String(sub.into()));
                    }
                    if let Some(exp) = claims.exp {
                        claims_pairs.insert(HashKey::String("exp".into()), Value::Int(exp as i64));
                    }
                    if let Some(iat) = claims.iat {
                        claims_pairs.insert(HashKey::String("iat".into()), Value::Int(iat as i64));
                    }

                    for (key, value) in claims.data {
                        claims_pairs.insert(HashKey::String(key.into()), json_to_value(value)?);
                    }

                    // Wrap the claims in an outer hash that names them as
                    // unverified. Code that mistakenly does
                    // `result["sub"]` now reads `null`, not a forged claim.
                    let mut pairs: HashPairs = HashPairs::default();
                    pairs.insert(HashKey::String("unverified".into()), Value::Bool(true));
                    pairs.insert(
                        HashKey::String("claims".into()),
                        Value::Hash(Rc::new(RefCell::new(claims_pairs))),
                    );
                    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
                }
                Err(e) => {
                    let mut error_pairs: HashPairs = HashPairs::default();
                    error_pairs.insert(HashKey::String("error".into()), Value::Bool(true));
                    error_pairs.insert(
                        HashKey::String("message".into()),
                        Value::String(format!("{}", e).into()),
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

/// Build an EncodingKey for the given algorithm. RS256 expects an RSA PEM,
/// EdDSA expects an Ed25519 PEM. Falling back to `from_secret` would silently
/// downgrade asymmetric crypto to HMAC against the PEM bytes — explicitly
/// surface a parse error instead.
fn build_encoding_key(
    algorithm: &Algorithm,
    secret: &str,
    pem_key: Option<&str>,
) -> Result<EncodingKey, String> {
    match algorithm {
        Algorithm::RS256 => {
            let pem = pem_key.unwrap_or(secret);
            EncodingKey::from_rsa_pem(pem.as_bytes())
                .map_err(|e| format!("RS256 requires a valid RSA private key in PEM form: {}", e))
        }
        Algorithm::EdDSA => {
            let pem = pem_key.unwrap_or(secret);
            EncodingKey::from_ed_pem(pem.as_bytes()).map_err(|e| {
                format!(
                    "EdDSA requires a valid Ed25519 private key in PEM form: {}",
                    e
                )
            })
        }
        _ => Ok(EncodingKey::from_secret(secret.as_bytes())),
    }
}

/// Build a DecodingKey for the given algorithm. See `build_encoding_key`.
fn build_decoding_key(
    algorithm: &Algorithm,
    secret: &str,
    pem_key: Option<&str>,
) -> Result<DecodingKey, String> {
    match algorithm {
        Algorithm::RS256 => {
            let pem = pem_key.unwrap_or(secret);
            DecodingKey::from_rsa_pem(pem.as_bytes())
                .map_err(|e| format!("RS256 requires a valid RSA public key in PEM form: {}", e))
        }
        Algorithm::EdDSA => {
            let pem = pem_key.unwrap_or(secret);
            DecodingKey::from_ed_pem(pem.as_bytes()).map_err(|e| {
                format!(
                    "EdDSA requires a valid Ed25519 public key in PEM form: {}",
                    e
                )
            })
        }
        _ => Ok(DecodingKey::from_secret(secret.as_bytes())),
    }
}

/// Extract a string claim from a payload hash.
fn extract_string_claim(payload: &Value, key: &str) -> Option<String> {
    if let Value::Hash(hash) = payload {
        for (k, v) in hash.borrow().iter() {
            if let HashKey::String(k_str) = k {
                if **k_str == *key {
                    if let Value::String(s) = v {
                        return Some(s.clone().to_string());
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
        payload.insert(HashKey::String("sub".into()), Value::String("alice".into()));
        payload.insert(
            HashKey::String("role".into()),
            Value::String("admin".into()),
        );
        let payload_hash = Value::Hash(Rc::new(RefCell::new(payload)));
        // SEC-054: secret must be ≥ 32 bytes; the prior fixture was
        // only 23 chars and would now fail the minimum-length check.
        let token = (sign.func)(vec![
            payload_hash,
            Value::String("0123456789abcdef0123456789abcdef".into()),
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
            outer_borrow.get(&HashKey::String("unverified".into())),
            Some(Value::Bool(true))
        ));
        let claims = match outer_borrow.get(&HashKey::String("claims".into())) {
            Some(Value::Hash(c)) => c.clone(),
            other => panic!("expected nested claims hash, got {other:?}"),
        };
        let claims_borrow = claims.borrow();
        // Claims are reachable via the wrapper but NOT at the top level.
        assert!(matches!(
            claims_borrow.get(&HashKey::String("sub".into())),
            Some(Value::String(s)) if **s == *"alice"
        ));
        assert!(outer_borrow.get(&HashKey::String("sub".into())).is_none());
    }

    /// SEC-054: jwt_sign and jwt_verify reject secrets shorter than 32
    /// bytes. The prior 16-char minimum let `aaaaaaaaaaaaaaaa` and
    /// other low-entropy secrets through, weaker than the HS256 digest.
    #[test]
    fn jwt_sign_rejects_secret_under_32_bytes() {
        let env = fresh_env();
        let sign = jwt_fn(&env, "jwt_sign");

        let payload = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));
        let weak = "a".repeat(31);
        let err = (sign.func)(vec![payload, Value::String(weak.into())]).unwrap_err();
        assert!(
            err.contains("at least 32") && err.contains("openssl rand"),
            "expected 32-byte minimum + .env hint, got: {err}"
        );
    }

    #[test]
    fn jwt_verify_rejects_secret_under_32_bytes() {
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");

        // Even a structurally invalid token must trip the length gate
        // first — the key check fires before signature verification.
        let weak = "a".repeat(31);
        let err = (verify.func)(vec![
            Value::String("dummy.token.value".into()),
            Value::String(weak.into()),
        ])
        .unwrap_err();
        assert!(
            err.contains("at least 32"),
            "expected 32-byte minimum, got: {err}"
        );
    }

    /// SEC-029: the bare `jwt_decode` builtin is removed and points
    /// callers at the safe alternatives.
    #[test]
    fn jwt_decode_returns_migration_error() {
        let env = fresh_env();
        let decode = jwt_fn(&env, "jwt_decode");

        let err = (decode.func)(vec![Value::String("anything".into())]).unwrap_err();
        assert!(
            err.contains("SEC-029")
                && err.contains("jwt_decode_unsafe")
                && err.contains("jwt_verify"),
            "expected SEC-029 migration error pointing at both alternatives, got: {}",
            err
        );
    }

    // SEC-091 — verifier picks the algorithm, not the token.

    /// Sign an HMAC token (HS256) with the given secret and return the
    /// JWS string. Always sets `expires_in: 3600` so the resulting token
    /// has the `exp` claim that `Validation::new` requires by default
    /// — without it, jwt_verify rejects the token as malformed and
    /// algorithm-mismatch tests can't reach the SEC-091 check they want
    /// to exercise.
    fn sign_hs256(secret: &str, sub: &str) -> String {
        let mut payload: HashPairs = HashPairs::default();
        payload.insert(
            HashKey::String("sub".into()),
            Value::String(sub.to_string().into()),
        );
        let mut sign_opts: HashPairs = HashPairs::default();
        sign_opts.insert(HashKey::String("expires_in".into()), Value::Int(3600));
        let env = fresh_env();
        let sign = jwt_fn(&env, "jwt_sign");
        let token = (sign.func)(vec![
            Value::Hash(Rc::new(RefCell::new(payload))),
            Value::String(secret.to_string().into()),
            Value::Hash(Rc::new(RefCell::new(sign_opts))),
        ])
        .unwrap();
        match token {
            Value::String(s) => s.to_string(),
            other => panic!("expected token string, got {:?}", other),
        }
    }

    fn opts(pairs: &[(&str, Value)]) -> Value {
        let mut h: HashPairs = HashPairs::default();
        for (k, v) in pairs {
            h.insert(HashKey::String((*k).to_string().into()), v.clone());
        }
        Value::Hash(Rc::new(RefCell::new(h)))
    }

    const TEST_SECRET: &str = "0123456789abcdef0123456789abcdef";

    /// Sign `payload_pairs` with `sign_opts`, returning the token string.
    fn sign_with(payload_pairs: &[(&str, Value)], sign_opts: &[(&str, Value)]) -> String {
        let env = fresh_env();
        let sign = jwt_fn(&env, "jwt_sign");
        let token = (sign.func)(vec![
            opts(payload_pairs),
            Value::String(TEST_SECRET.into()),
            opts(sign_opts),
        ])
        .unwrap();
        match token {
            Value::String(s) => s.to_string(),
            other => panic!("expected token string, got {:?}", other),
        }
    }

    /// Verify `token` and read one claim (or the `message` of an error hash).
    fn verify_claim(token: &str, verify_opts: &[(&str, Value)], claim: &str) -> Option<Value> {
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");
        let result = (verify.func)(vec![
            Value::String(token.into()),
            Value::String(TEST_SECRET.into()),
            opts(verify_opts),
        ])
        .unwrap();
        match result {
            Value::Hash(h) => h.borrow().get(&HashKey::String(claim.into())).cloned(),
            other => panic!("expected hash result, got {other:?}"),
        }
    }

    /// Regression: jsonwebtoken defaults `validate_aud` to true, so the 2-arg
    /// form used to reject *every* token carrying an audience — which meant no
    /// OIDC id_token could be verified at all.
    #[test]
    fn jwt_verify_accepts_aud_bearing_token_without_audience_option() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                ("aud", Value::String("client1".into())),
            ],
        );

        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");
        let result = (verify.func)(vec![
            Value::String(token.into()),
            Value::String(TEST_SECRET.into()),
        ])
        .unwrap();
        let claims = match result {
            Value::Hash(h) => h,
            other => panic!("expected hash result, got {other:?}"),
        };
        let borrowed = claims.borrow();
        assert!(
            borrowed.get(&HashKey::String("error".into())).is_none(),
            "2-arg verify must not reject an aud-bearing token: {:?}",
            borrowed
        );
        assert_eq!(
            borrowed.get(&HashKey::String("aud".into())),
            Some(&Value::String("client1".into()))
        );
    }

    #[test]
    fn jwt_verify_matches_and_rejects_audience() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                ("aud", Value::String("client1".into())),
            ],
        );

        assert_eq!(
            verify_claim(
                &token,
                &[("audience", Value::String("client1".into()))],
                "aud"
            ),
            Some(Value::String("client1".into()))
        );
        assert_eq!(
            verify_claim(
                &token,
                &[("audience", Value::String("other".into()))],
                "message"
            ),
            Some(Value::String("InvalidAudience".into()))
        );
    }

    /// Asking for an audience makes `aud` mandatory, so a token without one
    /// cannot slip through a check the caller believed was enforced.
    #[test]
    fn jwt_verify_requires_aud_when_audience_requested() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[("expires_in", Value::Int(600))],
        );

        let message = verify_claim(
            &token,
            &[("audience", Value::String("client1".into()))],
            "message",
        );
        match message {
            Some(Value::String(s)) => assert!(s.contains("aud"), "{s}"),
            other => panic!("expected a missing-claim error, got {other:?}"),
        }
    }

    #[test]
    fn jwt_verify_matches_and_rejects_issuer() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                ("iss", Value::String("https://op.test".into())),
            ],
        );

        assert_eq!(
            verify_claim(
                &token,
                &[("issuer", Value::String("https://op.test".into()))],
                "iss"
            ),
            Some(Value::String("https://op.test".into()))
        );
        assert_eq!(
            verify_claim(
                &token,
                &[("issuer", Value::String("https://evil.test".into()))],
                "message"
            ),
            Some(Value::String("InvalidIssuer".into()))
        );
    }

    /// `jwt_decode_unsafe` inspects without verifying, so an audience it was
    /// never told to expect must not make the token unreadable.
    #[test]
    fn jwt_decode_unsafe_reads_aud_bearing_token() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                ("aud", Value::String("client1".into())),
            ],
        );

        let env = fresh_env();
        let decode = jwt_fn(&env, "jwt_decode_unsafe");
        let result = (decode.func)(vec![Value::String(token.into())]).unwrap();
        let outer = match result {
            Value::Hash(h) => h,
            other => panic!("expected hash result, got {other:?}"),
        };
        let claims = match outer.borrow().get(&HashKey::String("claims".into())) {
            Some(Value::Hash(h)) => h.clone(),
            other => panic!("expected claims hash, got {other:?}"),
        };
        let claims_ref = claims.borrow();
        assert_eq!(
            claims_ref.get(&HashKey::String("aud".into())),
            Some(&Value::String("client1".into()))
        );
    }

    /// Without `kid` a relying party cannot pick a key out of a JWKS, so key
    /// rotation is impossible — assert it reaches the header verbatim.
    #[test]
    fn jwt_sign_sets_kid_and_typ_header() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                ("kid", Value::String("key-1".into())),
                ("typ", Value::String("at+jwt".into())),
            ],
        );

        let header = decode_header(&token).expect("header must parse");
        assert_eq!(header.kid.as_deref(), Some("key-1"));
        assert_eq!(header.typ.as_deref(), Some("at+jwt"));
    }

    /// `aud` is a string or an array per RFC 7519 §4.1.3; both must survive
    /// the round trip.
    #[test]
    fn jwt_sign_supports_array_audience() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                (
                    "aud",
                    Value::Array(Rc::new(RefCell::new(vec![
                        Value::String("a".into()),
                        Value::String("b".into()),
                    ]))),
                ),
            ],
        );

        // Subset semantics: the token carries both, so both must be expected.
        assert!(verify_claim(
            &token,
            &[(
                "audience",
                Value::Array(Rc::new(RefCell::new(vec![
                    Value::String("a".into()),
                    Value::String("b".into()),
                ])))
            )],
            "aud"
        )
        .is_some());
    }

    #[test]
    fn jwt_sign_accepts_absolute_exp_and_nbf() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("exp", Value::Int(4_102_444_800)),
                ("nbf", Value::Int(1000)),
            ],
        );

        assert_eq!(
            verify_claim(&token, &[], "exp"),
            Some(Value::Int(4_102_444_800))
        );
        assert_eq!(verify_claim(&token, &[], "nbf"), Some(Value::Int(1000)));
    }

    /// The two are different units (absolute vs. relative); silently letting
    /// one win would produce a token expiring at a time the caller never meant.
    #[test]
    fn jwt_sign_rejects_exp_with_expires_in() {
        let env = fresh_env();
        let sign = jwt_fn(&env, "jwt_sign");
        let err = (sign.func)(vec![
            opts(&[("sub", Value::String("u1".into()))]),
            Value::String(TEST_SECRET.into()),
            opts(&[("exp", Value::Int(123)), ("expires_in", Value::Int(60))]),
        ])
        .expect_err("exp + expires_in must be rejected");
        assert!(err.contains("not both"), "{}", err);
    }

    /// SEC-091 core attack: an attacker who knows the verifier's RSA
    /// public-key bytes signs an HS256 token using those bytes as the
    /// HMAC secret. The previous code picked the algorithm from the
    /// token header and would have verified the signature. With the fix,
    /// `jwt_verify(token, public_key, {algorithm: "RS256"})` must reject
    /// the algorithm mismatch before any signature verification runs.
    #[test]
    fn jwt_verify_rejects_hmac_token_when_rs256_expected() {
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");

        // The "RSA public key" the verifier thinks it's using. In the
        // attack, the attacker treats the bytes as an HMAC secret and
        // signs with HS256.
        let pretend_pub_key = "0123456789abcdef0123456789abcdef0123456789abcdef";
        let attacker_token = sign_hs256(pretend_pub_key, "alice");

        let result = (verify.func)(vec![
            Value::String(attacker_token.into()),
            Value::String(pretend_pub_key.to_string().into()),
            opts(&[("algorithm", Value::String("RS256".into()))]),
        ])
        .expect_err("HS256 token must be rejected when RS256 is expected");
        assert!(
            result.contains("does not match expected"),
            "expected algorithm-mismatch rejection, got: {}",
            result
        );
    }

    /// Asymmetric default: when the caller passes a `key` option (the
    /// asymmetric pattern) but no explicit `algorithm`, the allow list
    /// is RS256 / EdDSA only. An HS256 token must be rejected.
    #[test]
    fn jwt_verify_with_pem_key_rejects_hmac_tokens_by_default() {
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");

        let pretend_pub_key = "0123456789abcdef0123456789abcdef0123456789abcdef";
        let attacker_token = sign_hs256(pretend_pub_key, "alice");

        let err = (verify.func)(vec![
            Value::String(attacker_token.into()),
            Value::String(pretend_pub_key.to_string().into()),
            opts(&[("key", Value::String(pretend_pub_key.to_string().into()))]),
        ])
        .expect_err("HMAC token must be rejected when a PEM key is provided");
        assert!(err.contains("does not match expected"), "{}", err);
    }

    /// Round-trip the standard 2-arg HMAC path (no options): the legacy
    /// shape used by every existing caller still works end-to-end. SEC-091
    /// keeps this path on HS256/HS384/HS512 only.
    #[test]
    fn jwt_verify_two_arg_form_still_round_trips_hmac() {
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");

        let secret = "0123456789abcdef0123456789abcdef".to_string();
        let token = sign_hs256(&secret, "alice");

        let result = (verify.func)(vec![
            Value::String(token.into()),
            Value::String(secret.into()),
        ])
        .unwrap();
        let h = match result {
            Value::Hash(h) => h,
            other => panic!("expected verified-claims hash, got {:?}", other),
        };
        let h = h.borrow();
        // Verified path returns claims at the top level (distinct from
        // jwt_decode_unsafe which wraps them in `{unverified, claims}`).
        let sub = h.get(&HashKey::String("sub".into()));
        assert!(
            matches!(sub, Some(Value::String(s)) if **s == *"alice"),
            "{:?}",
            sub
        );
    }

    /// Explicit-algorithm pin: caller specifies `algorithm: "HS256"` and
    /// the token's header alg matches → verify succeeds. Caller specifies
    /// `algorithm: "HS512"` and the token's header is HS256 → reject.
    #[test]
    fn jwt_verify_explicit_algorithm_pin_strict_match() {
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");

        let secret = "0123456789abcdef0123456789abcdef".to_string();
        let token_hs256 = sign_hs256(&secret, "alice");

        // Match → success.
        let ok = (verify.func)(vec![
            Value::String(token_hs256.clone().into()),
            Value::String(secret.clone().into()),
            opts(&[("algorithm", Value::String("HS256".into()))]),
        ])
        .unwrap();
        assert!(matches!(ok, Value::Hash(_)));

        // Mismatch → error.
        let err = (verify.func)(vec![
            Value::String(token_hs256.into()),
            Value::String(secret.into()),
            opts(&[("algorithm", Value::String("HS512".into()))]),
        ])
        .expect_err("HS256 token must be rejected when HS512 is pinned");
        assert!(err.contains("does not match expected"), "{}", err);
    }

    #[test]
    fn jwt_verify_unknown_algorithm_in_options_errors() {
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");
        let err = (verify.func)(vec![
            Value::String("unused.token.value".into()),
            Value::String("0123456789abcdef0123456789abcdef".into()),
            opts(&[("algorithm", Value::String("none".into()))]),
        ])
        .expect_err("unsupported algorithm name must error");
        assert!(err.contains("unsupported algorithm"), "{}", err);
    }

    #[test]
    fn jwt_verify_too_many_args_errors() {
        // Arity gate: previously `Some(2)` made the options path dead;
        // now we accept 2 or 3 args and reject 4+.
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");
        let err = (verify.func)(vec![
            Value::String("t".into()),
            Value::String("s".into()),
            Value::Hash(Rc::new(RefCell::new(HashPairs::default()))),
            Value::String("extra".into()),
        ])
        .expect_err("4 args must be rejected");
        assert!(err.contains("expects 2 or 3 arguments"), "{}", err);
    }
}
