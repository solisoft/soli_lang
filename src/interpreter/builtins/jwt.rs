//! JWT (JSON Web Token) support for Solilang.
//!
//! Provides functions for signing and verifying JWTs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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
use crate::platform::unix_now_secs as current_timestamp;

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

/// Register JWT builtins in the given environment.
/// Does this string look like a key, rather than a shared secret?
///
/// PEM armour and OpenSSH keys are unmistakable, and nobody types either as an
/// HMAC password.
///
/// So is the body of a PEM with its armour stripped: base64 that decodes to
/// one DER `SEQUENCE` spanning every byte, whose first element is a
/// `SEQUENCE` (SubjectPublicKeyInfo, a certificate) or an `INTEGER` (PKCS#1,
/// PKCS#8). DER starts with `0x30`, so its base64 starts with `M` — which a
/// hex secret never does, and a random base64 one rarely does *and* parses.
fn looks_like_key_material(secret: &str) -> bool {
    let trimmed = secret.trim_start();
    trimmed.starts_with("-----BEGIN") || trimmed.starts_with("ssh-") || is_base64_der_key(trimmed)
}

fn is_base64_der_key(text: &str) -> bool {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
    use base64::Engine as _;

    if !text.starts_with('M') {
        return false;
    }
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(der) = STANDARD
        .decode(compact.as_bytes())
        .ok()
        .or_else(|| STANDARD_NO_PAD.decode(compact.as_bytes()).ok())
    else {
        return false;
    };
    // (tag, start of content, end of content) of the DER element at `at`.
    let element = |at: usize| -> Option<(u8, usize, usize)> {
        let tag = *der.get(at)?;
        let first = *der.get(at + 1)? as usize;
        let (length, start) = if first < 0x80 {
            (first, at + 2)
        } else {
            let count = first & 0x7f;
            if count == 0 || count > 3 {
                return None;
            }
            let bytes = der.get(at + 2..at + 2 + count)?;
            if bytes[0] == 0 {
                return None;
            }
            let length = bytes.iter().fold(0usize, |acc, b| (acc << 8) | *b as usize);
            if length < 0x80 {
                return None;
            }
            (length, at + 2 + count)
        };
        let end = start.checked_add(length)?;
        (end <= der.len()).then_some((tag, start, end))
    };
    match element(0) {
        Some((0x30, start, end)) if end == der.len() && der.len() >= 32 => {
            matches!(element(start), Some((0x30 | 0x02, _, inner_end)) if inner_end <= end)
        }
        _ => false,
    }
}

/// The most leeway `jwt_verify` grants, in seconds: a year, which is what
/// accepting an expired OpenID Connect logout hint takes (the generated OIDC
/// provider passes exactly that). Unbounded, a leeway of 10^9 accepted tokens
/// expired decades ago, and one past the current time made `now - leeway`
/// wrap inside the verifier.
const MAX_LEEWAY: i64 = 365 * 86_400;

/// The options `jwt_verify` reads.
const VERIFY_OPTIONS: &str = "algorithm, key, audience, issuer, subject, authorized_party, leeway";

/// The error for an option `jwt_verify` does not know. jwt_sign's claim
/// names are the likely slip, and each has a verifier counterpart.
fn unknown_verify_option(name: &str) -> String {
    let hint = match name {
        "aud" => " (did you mean `audience`?)",
        "iss" => " (did you mean `issuer`?)",
        "sub" => " (did you mean `subject`?)",
        "azp" => " (did you mean `authorized_party`?)",
        _ => "",
    };
    format!(
        "jwt_verify() unknown option `{}`{}; valid options: {}",
        name, hint, VERIFY_OPTIONS
    )
}

/// Refuse a token issued further in the future than the clock leeway: a
/// clock that far ahead, or a token minted to be used later.
fn check_issued_at(iat: Option<u64>, leeway: u64) -> Result<(), String> {
    match iat {
        Some(issued) if issued > current_timestamp().saturating_add(leeway) => {
            Err("InvalidIssuedAt: the token was issued in the future".to_string())
        }
        _ => Ok(()),
    }
}

/// OpenID Connect Core 3.1.3.7: when `azp` is present it must be the client
/// verifying the token, and a token for several audiences must carry it.
fn check_authorized_party(
    claims: &HashMap<String, JsonValue>,
    expected: &str,
) -> Result<(), String> {
    match claims.get("azp") {
        Some(JsonValue::String(azp)) if azp == expected => Ok(()),
        Some(_) => Err("InvalidAuthorizedParty".to_string()),
        None => match claims.get("aud") {
            Some(JsonValue::Array(auds)) if auds.len() > 1 => {
                Err("Missing required claim: azp (the token names several audiences)".to_string())
            }
            _ => Ok(()),
        },
    }
}

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
            // Signed: a negative `expires_in` signs a token that is already
            // expired (how a spec builds one). As `u64` it wrapped to a huge
            // value and `now + secs` overflowed — a panic in debug builds,
            // and the right answer only by accident of wrapping in release.
            let mut expires_in: Option<i64> = None;
            let mut algorithm = Algorithm::HS256;
            let mut pem_key: Option<String> = None;
            let mut kid: Option<String> = None;
            let mut typ: Option<String> = None;
            let mut absolute_exp: Option<i64> = None;
            let mut not_before: Option<i64> = None;
            let mut issued_at: Option<i64> = None;
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
                                        expires_in = Some(*secs);
                                    }
                                }
                                "algorithm" => {
                                    if let Value::String(alg) = v {
                                        algorithm = match alg.as_ref() {
                                            "HS256" => Algorithm::HS256,
                                            "HS384" => Algorithm::HS384,
                                            "HS512" => Algorithm::HS512,
                                            "RS256" => Algorithm::RS256,
                                            "ES256" => Algorithm::ES256,
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
                                // Absolute Unix timestamp; defaults to now.
                                // Strict on type: a new option has no
                                // callers relying on a silent skip.
                                "iat" => match v {
                                    Value::Int(ts) => issued_at = Some(*ts),
                                    other => {
                                        return Err(format!(
                                            "jwt_sign() option `iat` must be an Int \
                                             (Unix timestamp), got {}",
                                            other.type_name()
                                        ))
                                    }
                                },
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
            let is_asymmetric = matches!(
                algorithm,
                Algorithm::RS256 | Algorithm::ES256 | Algorithm::EdDSA
            );
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
            //
            // `exp` and `iat` in the payload used to be dropped silently, so
            // a caller who put them there believed the token carried them.
            // They raise now and name the option that does the job.
            let mut claims = serde_json::Map::new();
            if let Value::Hash(hash) = payload {
                for (k, v) in hash.borrow().iter() {
                    if let HashKey::String(key) = k {
                        match key.as_ref() {
                            "exp" => {
                                return Err("jwt_sign() takes `exp` from the options, \
                                            not the payload: pass {\"exp\": <timestamp>} \
                                            or {\"expires_in\": <seconds>} as the third \
                                            argument"
                                    .to_string())
                            }
                            "iat" => {
                                return Err("jwt_sign() takes `iat` from the options, \
                                            not the payload: pass {\"iat\": <timestamp>} \
                                            as the third argument (it defaults to now)"
                                    .to_string())
                            }
                            // Read separately below: only a string is kept.
                            "sub" => {}
                            _ => {
                                claims.insert(key.to_string(), value_to_json(v)?);
                            }
                        }
                    }
                }
            }

            let now = i64::try_from(current_timestamp()).unwrap_or(i64::MAX);
            let iat = issued_at.unwrap_or(now);
            if let Some(sub) = extract_string_claim(payload, "sub") {
                claims.insert("sub".to_string(), JsonValue::String(sub));
            }
            claims.insert("iat".to_string(), JsonValue::from(iat));
            if let Some(exp) = absolute_exp {
                claims.insert("exp".to_string(), JsonValue::from(exp));
            } else if let Some(secs) = expires_in {
                let exp = now.saturating_add(secs);
                claims.insert("exp".to_string(), JsonValue::from(exp));
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
            let mut expected_authorized_party: Option<String> = None;
            let mut leeway: Option<u64> = None;
            // Strict: an option the verifier does not know, or a value of the
            // wrong type, used to be skipped — so `{"aud": …, "iss": …}`
            // (jwt_sign's names) verified a token with no audience or issuer
            // check at all. A check the caller believes is enforced must be.
            match args.get(2) {
                None | Some(Value::Null) => {}
                Some(Value::Hash(opts)) => {
                    for (k, v) in opts.borrow().iter() {
                        let key = match k {
                            HashKey::String(key) => key.to_string(),
                            other => {
                                return Err(format!(
                                    "jwt_verify() option names must be strings, got {:?}",
                                    other
                                ))
                            }
                        };
                        let string_option = |v: &Value| match v {
                            Value::String(s) => Ok(s.to_string()),
                            other => Err(format!(
                                "jwt_verify() option `{}` must be a String, got {}",
                                key,
                                other.type_name()
                            )),
                        };
                        match key.as_str() {
                            "key" => pem_key = Some(string_option(v)?),
                            "audience" => {
                                expected_audience =
                                    Some(string_list_option(v, "jwt_verify", "audience")?);
                            }
                            "issuer" => {
                                expected_issuer =
                                    Some(string_list_option(v, "jwt_verify", "issuer")?);
                            }
                            "subject" => expected_subject = Some(string_option(v)?),
                            "authorized_party" => {
                                expected_authorized_party = Some(string_option(v)?)
                            }
                            "leeway" => match v {
                                Value::Int(secs) if (0..=MAX_LEEWAY).contains(secs) => {
                                    leeway = Some(*secs as u64)
                                }
                                Value::Int(secs) if *secs > MAX_LEEWAY => {
                                    return Err(format!(
                                        "jwt_verify() option `leeway` is at most {} seconds \
                                         (a year), got {}",
                                        MAX_LEEWAY, secs
                                    ))
                                }
                                other => {
                                    return Err(format!(
                                        "jwt_verify() option `leeway` must be a non-negative \
                                         Int (seconds), got {}",
                                        match other {
                                            Value::Int(secs) => secs.to_string(),
                                            other => other.type_name().to_string(),
                                        }
                                    ))
                                }
                            },
                            "algorithm" => {
                                let alg = string_option(v)?;
                                expected_algorithm = Some(match alg.as_str() {
                                    "HS256" => Algorithm::HS256,
                                    "HS384" => Algorithm::HS384,
                                    "HS512" => Algorithm::HS512,
                                    "RS256" => Algorithm::RS256,
                                    "ES256" => Algorithm::ES256,
                                    "EdDSA" => Algorithm::EdDSA,
                                    _ => {
                                        return Err(format!(
                                            "jwt_verify() unsupported algorithm: {}",
                                            alg
                                        ))
                                    }
                                });
                            }
                            other => return Err(unknown_verify_option(other)),
                        }
                    }
                }
                Some(other) => {
                    return Err(format!(
                        "jwt_verify() options must be a Hash, got {}",
                        other.type_name()
                    ))
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
                (None, Some(_)) => vec![Algorithm::RS256, Algorithm::ES256, Algorithm::EdDSA],
                (None, None) => {
                    vec![Algorithm::HS256, Algorithm::HS384, Algorithm::HS512]
                }
            };

            let header_carries_hmac = |algs: &[Algorithm]| {
                algs.iter()
                    .any(|a| !matches!(a, Algorithm::RS256 | Algorithm::ES256 | Algorithm::EdDSA))
            };

            // Key material passed where a shared secret belongs is refused.
            //
            // SEC-091 pinned the algorithm from the caller's *options*, which
            // closes the confusion attack when a `key` is supplied. It leaves
            // one step: `jwt_verify(token, <RSA public PEM>)` — the pre-SEC-091
            // habit, and an easy misreading of the docs — falls into the
            // HMAC-only branch and keys an HMAC verifier with public bytes. Any
            // PEM is well over the 32-byte floor, so nothing else catches it,
            // and an attacker who has the public key (it is public) can forge an
            // HS256 token that verifies. Legitimate RS256 tokens fail against
            // such a call anyway, so refusing costs no working code.
            if header_carries_hmac(&allowed_algorithms) && looks_like_key_material(&secret) {
                return Err(
                    "jwt_verify(): the second argument looks like key material, not a shared \
                     secret. Verifying an asymmetric token needs the algorithm named: \
                     jwt_verify(token, {\"algorithm\": \"RS256\", \"key\": pem}). Passing a \
                     public key as an HMAC secret would let anyone holding that key forge a \
                     token."
                        .to_string(),
                );
            }

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
            // With several expected audiences the token passes when its `aud`
            // names *any one* of them (jsonwebtoken intersects the two sets).
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
            let leeway_secs = validation.leeway;
            let decoded = decode::<Claims>(&token, &decoding_key, &validation)
                .map_err(|e| e.to_string())
                .and_then(|token_data| {
                    check_issued_at(token_data.claims.iat, leeway_secs)?;
                    if let Some(party) = &expected_authorized_party {
                        check_authorized_party(&token_data.claims.data, party)?;
                    }
                    Ok(token_data)
                });
            match decoded {
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
                    error_pairs.insert(HashKey::String("message".into()), Value::String(e.into()));
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
        Algorithm::ES256 => {
            let pem = pem_key.unwrap_or(secret);
            EncodingKey::from_ec_pem(pem.as_bytes()).map_err(|e| {
                format!(
                    "ES256 requires a valid P-256 private key in PKCS#8 PEM form: {}",
                    e
                )
            })
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
        Algorithm::ES256 => {
            let pem = pem_key.unwrap_or(secret);
            DecodingKey::from_ec_pem(pem.as_bytes())
                .map_err(|e| format!("ES256 requires a valid P-256 public key in PEM form: {}", e))
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
        let token = (sign.func)(&[
            payload_hash,
            Value::String("0123456789abcdef0123456789abcdef".into()),
        ])
        .unwrap();
        let token_str = match token {
            Value::String(s) => s,
            other => panic!("expected token string, got {other:?}"),
        };

        let result = (decode.func)(&[Value::String(token_str)]).unwrap();
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
        let err = (sign.func)(&[payload, Value::String(weak.into())]).unwrap_err();
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
        let err = (verify.func)(&[
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

        let err = (decode.func)(&[Value::String("anything".into())]).unwrap_err();
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
        let token = (sign.func)(&[
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
        let token = (sign.func)(&[
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
        let result = (verify.func)(&[
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
        let result = (verify.func)(&[
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
        let result = (decode.func)(&[Value::String(token.into())]).unwrap();
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

    /// A negative `expires_in` signs an already-expired token: `exp` sits that
    /// many seconds before `iat`, with no overflow (it panicked in debug
    /// builds, reached from grc's SSO spec).
    #[test]
    fn jwt_sign_negative_expires_in_is_already_expired() {
        let env = fresh_env();
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[("expires_in", Value::Int(-3600))],
        );
        let decode = jwt_fn(&env, "jwt_decode_unsafe");
        // The unverified claims sit under `claims`, by design (SEC-029).
        let claims = match (decode.func)(&[Value::String(token.into())]).unwrap() {
            Value::Hash(h) => match h.borrow().get(&HashKey::String("claims".into())) {
                Some(Value::Hash(inner)) => inner.borrow().clone(),
                other => panic!("expected a claims hash, got {other:?}"),
            },
            other => panic!("expected a hash, got {other:?}"),
        };
        let read = |name: &str| match claims.get(&HashKey::String(name.into())) {
            Some(Value::Int(n)) => *n,
            other => panic!("expected int {name}, got {other:?}"),
        };
        assert_eq!(read("exp"), read("iat") - 3600);
    }

    /// The two are different units (absolute vs. relative); silently letting
    /// one win would produce a token expiring at a time the caller never meant.
    #[test]
    fn jwt_sign_rejects_exp_with_expires_in() {
        let env = fresh_env();
        let sign = jwt_fn(&env, "jwt_sign");
        let err = (sign.func)(&[
            opts(&[("sub", Value::String("u1".into()))]),
            Value::String(TEST_SECRET.into()),
            opts(&[("exp", Value::Int(123)), ("expires_in", Value::Int(60))]),
        ])
        .expect_err("exp + expires_in must be rejected");
        assert!(err.contains("not both"), "{}", err);
    }

    /// `iat` comes from the options and replaces the default `now`.
    #[test]
    fn jwt_sign_accepts_iat_option() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("iat", Value::Int(1_700_000_000)),
                ("exp", Value::Int(4_102_444_800)),
            ],
        );
        assert_eq!(
            verify_claim(&token, &[], "iat"),
            Some(Value::Int(1_700_000_000))
        );
    }

    #[test]
    fn jwt_sign_rejects_non_int_iat_option() {
        let env = fresh_env();
        let sign = jwt_fn(&env, "jwt_sign");
        let err = (sign.func)(&[
            opts(&[("sub", Value::String("u1".into()))]),
            Value::String(TEST_SECRET.into()),
            opts(&[("iat", Value::String("yesterday".into()))]),
        ])
        .expect_err("a string iat must be rejected");
        assert!(err.contains("`iat` must be an Int"), "{}", err);
    }

    /// `exp` / `iat` in the payload were dropped silently; they raise now
    /// and name the option to use instead.
    #[test]
    fn jwt_sign_rejects_exp_and_iat_in_payload() {
        let env = fresh_env();
        let sign = jwt_fn(&env, "jwt_sign");
        for (claim, hint) in [("exp", "expires_in"), ("iat", "{\"iat\":")] {
            let err = (sign.func)(&[
                opts(&[
                    ("sub", Value::String("u1".into())),
                    (claim, Value::Int(1000)),
                ]),
                Value::String(TEST_SECRET.into()),
            ])
            .expect_err("a registered time claim in the payload must be rejected");
            assert!(
                err.contains(&format!("takes `{}` from the options", claim)),
                "{}",
                err
            );
            assert!(err.contains(hint), "{}", err);
        }
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

        let result = (verify.func)(&[
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

        let err = (verify.func)(&[
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

        let result =
            (verify.func)(&[Value::String(token.into()), Value::String(secret.into())]).unwrap();
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
        let ok = (verify.func)(&[
            Value::String(token_hs256.clone().into()),
            Value::String(secret.clone().into()),
            opts(&[("algorithm", Value::String("HS256".into()))]),
        ])
        .unwrap();
        assert!(matches!(ok, Value::Hash(_)));

        // Mismatch → error.
        let err = (verify.func)(&[
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
        let err = (verify.func)(&[
            Value::String("unused.token.value".into()),
            Value::String("0123456789abcdef0123456789abcdef".into()),
            opts(&[("algorithm", Value::String("none".into()))]),
        ])
        .expect_err("unsupported algorithm name must error");
        assert!(err.contains("unsupported algorithm"), "{}", err);
    }

    fn verify_err(token: &str, verify_opts: Value) -> String {
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");
        (verify.func)(&[
            Value::String(token.into()),
            Value::String(TEST_SECRET.into()),
            verify_opts,
        ])
        .expect_err("expected jwt_verify to raise")
    }

    /// `{"aud": …, "iss": …}` are jwt_sign's names; jwt_verify skipped them,
    /// so the token verified with no audience or issuer check at all.
    #[test]
    fn jwt_verify_refuses_unknown_options_and_wrong_types() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                ("aud", Value::String("client1".into())),
            ],
        );
        for (name, hint) in [
            ("aud", "audience"),
            ("iss", "issuer"),
            ("sub", "subject"),
            ("azp", "authorized_party"),
        ] {
            let err = verify_err(&token, opts(&[(name, Value::String("x".into()))]));
            assert!(
                err.contains(&format!("unknown option `{name}`"))
                    && err.contains(&format!("did you mean `{hint}`"))
                    && err.contains("valid options: algorithm, key, audience"),
                "{err}"
            );
        }
        let err = verify_err(&token, opts(&[("audiance", Value::String("x".into()))]));
        assert!(err.contains("unknown option `audiance`"), "{err}");

        for (name, value, want) in [
            ("subject", Value::Int(1), "`subject` must be a String"),
            ("key", Value::Int(1), "`key` must be a String"),
            (
                "algorithm",
                Value::Bool(true),
                "`algorithm` must be a String",
            ),
            (
                "authorized_party",
                Value::Null,
                "`authorized_party` must be a String",
            ),
            (
                "leeway",
                Value::String("60".into()),
                "`leeway` must be a non-negative Int",
            ),
            (
                "leeway",
                Value::Int(-1),
                "`leeway` must be a non-negative Int",
            ),
            (
                "leeway",
                Value::Int(31_536_001),
                "`leeway` is at most 31536000 seconds",
            ),
            (
                "leeway",
                Value::Int(1_000_000_000),
                "`leeway` is at most 31536000 seconds",
            ),
            (
                "audience",
                Value::Int(1),
                "`audience` expects string or array",
            ),
        ] {
            let err = verify_err(&token, opts(&[(name, value)]));
            assert!(err.contains(want), "{name}: {err}");
        }
        let err = verify_err(&token, Value::String("audience".into()));
        assert!(err.contains("options must be a Hash"), "{err}");
    }

    /// Several expected audiences: the token passes naming any one of them.
    #[test]
    fn jwt_verify_audience_list_is_any_of() {
        let token = sign_with(
            &[("sub", Value::String("u1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                ("aud", Value::String("client1".into())),
            ],
        );
        let either = Value::Array(Rc::new(RefCell::new(vec![
            Value::String("other".into()),
            Value::String("client1".into()),
        ])));
        assert_eq!(
            verify_claim(&token, &[("audience", either)], "aud"),
            Some(Value::String("client1".into()))
        );
    }

    #[test]
    fn jwt_verify_checks_authorized_party() {
        let two_audiences = Value::Array(Rc::new(RefCell::new(vec![
            Value::String("client1".into()),
            Value::String("api".into()),
        ])));
        let party = |p: &str| ("authorized_party", Value::String(p.to_string().into()));

        let with_azp = sign_with(
            &[("azp", Value::String("client1".into()))],
            &[
                ("expires_in", Value::Int(600)),
                ("aud", two_audiences.clone()),
            ],
        );
        assert_eq!(
            verify_claim(&with_azp, &[party("client1")], "azp"),
            Some(Value::String("client1".into()))
        );
        assert_eq!(
            verify_claim(&with_azp, &[party("client2")], "message"),
            Some(Value::String("InvalidAuthorizedParty".into()))
        );

        // Several audiences and no azp: refused once a party is expected.
        let without_azp = sign_with(
            &[],
            &[("expires_in", Value::Int(600)), ("aud", two_audiences)],
        );
        match verify_claim(&without_azp, &[party("client1")], "message") {
            Some(Value::String(s)) => assert!(s.contains("azp"), "{s}"),
            other => panic!("expected a missing-azp error, got {other:?}"),
        }
        assert!(verify_claim(&without_azp, &[], "error").is_none());

        // One audience and no azp: nothing to check.
        let single = sign_with(
            &[],
            &[
                ("expires_in", Value::Int(600)),
                ("aud", Value::String("client1".into())),
            ],
        );
        assert!(verify_claim(&single, &[party("client1")], "error").is_none());
    }

    #[test]
    fn jwt_verify_refuses_iat_in_the_future_beyond_the_leeway() {
        let now = current_timestamp() as i64;
        let future = sign_with(
            &[],
            &[
                ("expires_in", Value::Int(7200)),
                ("iat", Value::Int(now + 3600)),
            ],
        );
        match verify_claim(&future, &[], "message") {
            Some(Value::String(s)) => assert!(s.contains("InvalidIssuedAt"), "{s}"),
            other => panic!("expected an iat error, got {other:?}"),
        }
        // Inside the leeway (default 60s): a clock slightly behind the issuer's.
        let near = sign_with(
            &[],
            &[
                ("expires_in", Value::Int(600)),
                ("iat", Value::Int(now + 20)),
            ],
        );
        assert!(verify_claim(&near, &[], "error").is_none());
        match verify_claim(&near, &[("leeway", Value::Int(0))], "message") {
            Some(Value::String(s)) => assert!(s.contains("InvalidIssuedAt"), "{s}"),
            other => panic!("expected an iat error, got {other:?}"),
        }
    }

    const RSA_SPKI: &str = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEArNSGQQ1YInbNRnm7Rxtk
VL3iOwXNl5LsOJizsDc8GSVjmloPCCP2fReTbPOBe2OHQW7ApS/vRakJ7ADd4+Ol
7buTb0OpfVn9zQyiMo6iqml28QNLrwzwbleHzpBW0suM7jEHqpaKvwQGNeQh1Y9x
zSfw5j36jZsrymt1UzA+xrR9nkBrr0qOZbvjZENEmTScECbWzBfBs3HIA5RsMDYe
tVo4ukqSzfo3A3LUYu0JGZ7gPthgWII6BcCioj3Uq+9TAKje3b5O0yaKpVTGyR8W
9ztL89hSiDUh0urG3j8OjzSFvVY7zTVcurYroiGXgxYNtT6OzNgmaim0kewesp8+
TQIDAQAB";
    const EC_SPKI: &str = "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEHrltsgZjCoqnCKZ1LsI8KPysLi8/NL3dxQRrGe+fmnr6if6p1pex3HKKLdTtsd03dW1gq+36yLJNjX4CGWNkkw==";
    const ED25519_SPKI: &str = "MCowBQYDK2VwAyEAlEq2AHbiMejyudC0qJO3zeEDTsabg23pWJPkWcEfYUs=";

    /// A public key with its PEM armour stripped is still a public key: as
    /// an HMAC secret it lets anyone who holds it forge a token.
    #[test]
    fn bare_base64_keys_are_refused_as_hmac_secrets() {
        let certificate: String = include_str!("../../../tests/fixtures/saml/idp.crt")
            .lines()
            .filter(|line| !line.contains("-----"))
            .collect();
        let single_line_rsa: String = RSA_SPKI.lines().collect();
        for key in [
            RSA_SPKI,
            single_line_rsa.as_str(),
            EC_SPKI,
            ED25519_SPKI,
            certificate.as_str(),
        ] {
            assert!(looks_like_key_material(key), "{key}");
            let env = fresh_env();
            let verify = jwt_fn(&env, "jwt_verify");
            let err = (verify.func)(&[
                Value::String(sign_hs256(key, "alice").into()),
                Value::String(key.to_string().into()),
            ])
            .expect_err("a bare public key must not key an HMAC verifier");
            assert!(err.contains("looks like key material"), "{err}");
        }
        // Secrets as people generate them stay secrets.
        for secret in [
            TEST_SECRET,
            "4f1c2a9e8b7d6c5f4e3d2c1b0a9f8e7d6c5b4a3f2e1d0c9b8a7f6e5d4c3b2a1f",
            "Mz8xYjQ0ZTdkNjVhYzE0ZWJmNjE4NDQ4YTZiZjFiZjA5OGE2MmY2Mzc0ZDE=",
            "MyApplicationSecretThatIsLongEnoughToPass1234",
        ] {
            assert!(!looks_like_key_material(secret), "{secret}");
        }
    }

    #[test]
    fn jwt_verify_too_many_args_errors() {
        // Arity gate: previously `Some(2)` made the options path dead;
        // now we accept 2 or 3 args and reject 4+.
        let env = fresh_env();
        let verify = jwt_fn(&env, "jwt_verify");
        let err = (verify.func)(&[
            Value::String("t".into()),
            Value::String("s".into()),
            Value::Hash(Rc::new(RefCell::new(HashPairs::default()))),
            Value::String("extra".into()),
        ])
        .expect_err("4 args must be rejected");
        assert!(err.contains("expects 2 or 3 arguments"), "{}", err);
    }
}
