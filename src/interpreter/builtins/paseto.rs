//! PASETO v4 tokens (`Paseto.*`) — the versioned, non-negotiable alternative
//! to JWT.
//!
//! Two purposes, four operations:
//!
//! - `v4.local`  — `Paseto.encrypt` / `Paseto.decrypt`, XChaCha20 + BLAKE2b
//!   over a 32-byte symmetric key. The payload is *encrypted*, so nothing in
//!   it is readable by the bearer.
//! - `v4.public` — `Paseto.sign` / `Paseto.verify`, Ed25519. The payload is
//!   signed but readable, which is what you want for a token a third party
//!   must validate without holding a secret.
//!
//! Why this exists next to `jwt_sign`/`jwt_verify`: a JWT names its own
//! algorithm in an attacker-controlled header, which is the root of the
//! `alg: none` and RS256→HS256 confusion attacks (`jwt_verify` has to defend
//! against that explicitly — see SEC-091 in `jwt.rs`). A PASETO's version and
//! purpose are the first two segments of the token and pin the algorithm
//! before any key is touched: `Paseto.verify` can only ever Ed25519-verify a
//! `v4.public` token, so there is no negotiation to attack.
//!
//! Only v4 is exposed. v1/v2 are deprecated by the spec and v3 exists for
//! NIST-algorithm shops (P-384 / AES-CTR); offering a version *choice* would
//! reintroduce exactly the agility PASETO set out to remove.
//!
//! Verification failures **raise** rather than returning an error hash. The
//! `jwt_verify` contract (`{error: true, message: ...}`) is truthy on failure,
//! so `if jwt_verify(...)` treats a rejected token as authenticated — a
//! footgun documented at length in `jwt.rs`. Raising makes the failure path
//! fail closed, and Soli's postfix `rescue` keeps it a one-liner:
//! `claims = Paseto.verify(token, key) rescue nil`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::rc::Rc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use pasetors::claims::{Claims, ClaimsValidationRules};
use pasetors::errors::{ClaimValidationError, Error as PasetoError};
use pasetors::footer::Footer;
use pasetors::keys::{
    AsymmetricKeyPair, AsymmetricPublicKey, AsymmetricSecretKey, Generate, SymmetricKey,
};
use pasetors::paserk::{FormatAsPaserk, Id};
use pasetors::token::{TrustedToken, UntrustedToken};
use pasetors::version4::V4;
use pasetors::{local, public, Local, Public};
use serde_json::Value as JsonValue;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{
    json_to_value, value_to_json, Class, HashKey, HashPairs, NativeFunction, Value,
};

/// Claims PASETO reserves at the top level. A caller may set these through
/// the payload or the options hash, but they go through the typed setters
/// (which enforce the spec's string/RFC 3339 shapes) rather than being
/// written as free-form additional claims.
const REGISTERED_CLAIMS: [&str; 7] = ["iss", "sub", "aud", "exp", "nbf", "iat", "jti"];

/// Option keys accepted by `Paseto.encrypt` / `Paseto.sign`.
const MINT_OPTIONS: [&str; 12] = [
    "expires_in",
    "non_expiring",
    "exp",
    "nbf",
    "iat",
    "sub",
    "aud",
    "iss",
    "jti",
    "footer",
    "kid",
    "implicit",
];

/// Option keys accepted by `Paseto.decrypt` / `Paseto.verify`.
const READ_OPTIONS: [&str; 8] = [
    "audience",
    "issuer",
    "subject",
    "jti",
    "allow_non_expiring",
    "skip_valid_at",
    "footer",
    "implicit",
];

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// Serialize a key to its PASERK form (`k4.local.…`, `k4.secret.…`,
/// `k4.public.…`). PASERK strings carry the key's version *and* purpose, so a
/// local key can never be silently handed to the signing path.
fn to_paserk<K: FormatAsPaserk>(key: &K) -> Result<String, String> {
    let mut out = String::new();
    key.fmt(&mut out)
        .map_err(|_| "failed to serialize key as PASERK".to_string())?;
    Ok(out)
}

/// Decode a hex string into bytes, or `None` when it isn't clean hex.
fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// Read the key argument as a string.
fn key_arg(args: &[Value], index: usize, func: &str, what: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(format!(
            "{}() expects a string {}, got {}",
            func,
            what,
            other.type_name()
        )),
        None => Err(format!("{}() requires a {}", func, what)),
    }
}

/// Parse a symmetric (`v4.local`) key: a `k4.local.` PASERK string, or 64 hex
/// characters for the raw 32 bytes.
fn parse_local_key(raw: &str, func: &str) -> Result<SymmetricKey<V4>, String> {
    if raw.starts_with("k4.local.") {
        return SymmetricKey::<V4>::try_from(raw).map_err(|_| {
            format!(
                "{}(): invalid k4.local key — the PASERK string is malformed",
                func
            )
        });
    }
    match hex_to_bytes(raw) {
        Some(bytes) if bytes.len() == 32 => SymmetricKey::<V4>::from(&bytes)
            .map_err(|_| format!("{}(): invalid 32-byte local key", func)),
        _ => Err(format!(
            "{}(): key must be a `k4.local.` PASERK string (see Paseto.generate_local_key()) \
             or 64 hex characters for the raw 32 bytes",
            func
        )),
    }
}

/// An Ed25519 seed of all zeroes makes `ed25519-compact` **panic** ("All-zero
/// seed") rather than return an error, and pasetors validates a secret key by
/// deriving its key pair — so a key that came in over the wire, out of a
/// misconfigured `.env`, or from a zero-filled file would take down the worker
/// thread. Screen the seed ourselves and report it like any other bad key.
fn reject_zero_seed(bytes: &[u8], func: &str) -> Result<(), String> {
    if bytes.len() >= 32 && bytes[..32].iter().all(|byte| *byte == 0) {
        return Err(format!(
            "{}(): the secret key's seed is all zeroes — that is not a usable Ed25519 key \
             (generate one with Paseto.generate_key_pair())",
            func
        ));
    }
    Ok(())
}

/// Parse an Ed25519 secret key: a `k4.secret.` PASERK string, or 128 hex
/// characters for the raw 64 bytes (seed ‖ public key, as Ed25519 stores it).
fn parse_secret_key(raw: &str, func: &str) -> Result<AsymmetricSecretKey<V4>, String> {
    if let Some(encoded) = raw.strip_prefix("k4.secret.") {
        // Decoded up-front purely to screen the seed: `try_from` would reach the
        // panicking derivation before we ever see the bytes. A decode failure
        // here is left to `try_from` to report as a malformed key.
        if let Ok(bytes) = URL_SAFE_NO_PAD.decode(encoded) {
            reject_zero_seed(&bytes, func)?;
        }
        return AsymmetricSecretKey::<V4>::try_from(raw).map_err(|_| {
            format!(
                "{}(): invalid k4.secret key — the PASERK string is malformed",
                func
            )
        });
    }
    if raw.starts_with("k4.public.") {
        return Err(format!(
            "{}(): got a public key — signing needs the `k4.secret.` half of the pair",
            func
        ));
    }
    match hex_to_bytes(raw) {
        Some(bytes) if bytes.len() == 64 => {
            reject_zero_seed(&bytes, func)?;
            AsymmetricSecretKey::<V4>::from(&bytes).map_err(|_| {
                format!(
                    "{}(): invalid Ed25519 secret key — the public half does not match the seed",
                    func
                )
            })
        }
        _ => Err(format!(
            "{}(): key must be a `k4.secret.` PASERK string (see Paseto.generate_key_pair()) \
             or 128 hex characters for the raw 64 bytes (seed + public key)",
            func
        )),
    }
}

/// Parse an Ed25519 public key: a `k4.public.` PASERK string, or 64 hex
/// characters for the raw 32 bytes.
fn parse_public_key(raw: &str, func: &str) -> Result<AsymmetricPublicKey<V4>, String> {
    if raw.starts_with("k4.public.") {
        return AsymmetricPublicKey::<V4>::try_from(raw).map_err(|_| {
            format!(
                "{}(): invalid k4.public key — the PASERK string is malformed",
                func
            )
        });
    }
    if raw.starts_with("k4.secret.") {
        // Verifying with the secret key would work cryptographically (the
        // public half is embedded), but it means the secret has been shipped
        // to whatever service is doing the verifying. Refuse it loudly.
        return Err(format!(
            "{}(): got a secret key — verify with the `k4.public.` half so the signing key \
             never leaves the issuer (Paseto.public_key(secret) derives it)",
            func
        ));
    }
    match hex_to_bytes(raw) {
        Some(bytes) if bytes.len() == 32 => AsymmetricPublicKey::<V4>::from(&bytes)
            .map_err(|_| format!("{}(): invalid Ed25519 public key", func)),
        _ => Err(format!(
            "{}(): key must be a `k4.public.` PASERK string (see Paseto.generate_key_pair()) \
             or 64 hex characters for the raw 32 bytes",
            func
        )),
    }
}

// ---------------------------------------------------------------------------
// Claims / options
// ---------------------------------------------------------------------------

/// Convert a Unix timestamp to the RFC 3339 form PASETO requires for
/// `exp`/`nbf`/`iat` (the spec defines them as ISO 8601 strings, unlike JWT's
/// numeric dates).
fn unix_to_rfc3339(ts: i64, func: &str, claim: &str) -> Result<String, String> {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .ok_or_else(|| format!("{}(): `{}` is not a valid Unix timestamp", func, claim))
}

/// Read a time-valued option, accepting either a Unix timestamp (`Int`, the
/// JWT habit) or an RFC 3339 string (what the token actually carries).
fn time_option(value: &Value, func: &str, claim: &str) -> Result<String, String> {
    match value {
        Value::Int(ts) => unix_to_rfc3339(*ts, func, claim),
        Value::String(s) => Ok(s.to_string()),
        other => Err(format!(
            "{}(): `{}` expects a Unix timestamp Int or an RFC 3339 String, got {}",
            func,
            claim,
            other.type_name()
        )),
    }
}

fn string_option(value: &Value, func: &str, option: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.to_string()),
        other => Err(format!(
            "{}(): `{}` expects a String, got {}",
            func,
            option,
            other.type_name()
        )),
    }
}

fn bool_option(value: &Value, func: &str, option: &str) -> Result<bool, String> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(format!(
            "{}(): `{}` expects a Bool, got {}",
            func,
            option,
            other.type_name()
        )),
    }
}

/// Reject option keys we don't understand. `jwt_sign` silently ignores them,
/// which turns a typo like `"audiance"` into a check the caller believes is
/// enforced and isn't. Security options fail loudly here.
fn reject_unknown_options(
    options: &Value,
    allowed: &[&str],
    func: &str,
) -> Result<Rc<RefCell<HashPairs>>, String> {
    let hash = match options {
        Value::Hash(h) => h.clone(),
        Value::Null => return Ok(Rc::new(RefCell::new(HashPairs::default()))),
        other => {
            return Err(format!(
                "{}() options must be a Hash, got {}",
                func,
                other.type_name()
            ))
        }
    };
    for (key, _) in hash.borrow().iter() {
        if let HashKey::String(name) = key {
            if !allowed.contains(&name.as_ref()) {
                return Err(format!(
                    "{}(): unknown option `{}` (accepted: {})",
                    func,
                    name,
                    allowed.join(", ")
                ));
            }
        }
    }
    Ok(hash)
}

/// Set a registered claim through its typed setter.
fn set_registered(
    claims: &mut Claims,
    claim: &str,
    value: &Value,
    func: &str,
) -> Result<(), String> {
    let result = match claim {
        "iss" => claims.issuer(&string_option(value, func, "iss")?),
        "sub" => claims.subject(&string_option(value, func, "sub")?),
        // PASETO's `aud` is a single string — the JWT array form has no
        // equivalent here, so a caller passing an Array gets a clear error
        // from `string_option` rather than a silently dropped audience.
        "aud" => claims.audience(&string_option(value, func, "aud")?),
        "jti" => claims.token_identifier(&string_option(value, func, "jti")?),
        "exp" => claims.expiration(&time_option(value, func, "exp")?),
        "nbf" => claims.not_before(&time_option(value, func, "nbf")?),
        "iat" => claims.issued_at(&time_option(value, func, "iat")?),
        other => return Err(format!("{}(): unknown registered claim `{}`", func, other)),
    };
    result.map_err(|_| {
        format!(
            "{}(): `{}` is not a valid value for that claim",
            func, claim
        )
    })
}

/// Set the footer's `kid`. PASETO reserves this claim and the spec's Key-ID
/// guidance is to publish a PASERK key *id* (`Paseto.key_id(key)`) rather than
/// a name of your own choosing — the id is derived from the key, so it can
/// never leak key material and never carries attacker-chosen text.
fn set_footer_kid(footer: &mut Footer, value: &Value, func: &str) -> Result<(), String> {
    let raw = string_option(value, func, "kid")?;
    let id = Id::try_from(raw.as_str()).map_err(|_| {
        format!(
            "{}(): `kid` must be a PASERK key id from Paseto.key_id(key) (`k4.lid.…` for a local \
             key, `k4.pid.…` for a public one) — for a label of your own, use any other footer key",
            func
        )
    })?;
    footer.key_id(&id);
    Ok(())
}

/// Build the footer from a Hash of string values. The footer is *not*
/// encrypted (even for `v4.local`) but it *is* covered by the
/// authentication tag / signature, so it can be trusted after a successful
/// verify and tampering with it invalidates the token.
fn build_footer(value: &Value, func: &str) -> Result<Footer, String> {
    let hash = match value {
        Value::Hash(h) => h.clone(),
        other => {
            return Err(format!(
                "{}(): `footer` expects a Hash of string values, got {}",
                func,
                other.type_name()
            ))
        }
    };
    let mut footer = Footer::new();
    for (key, val) in hash.borrow().iter() {
        let name = match key {
            HashKey::String(name) => name.to_string(),
            other => {
                return Err(format!(
                    "{}(): `footer` keys must be strings, got {:?}",
                    func, other
                ))
            }
        };
        if name == "kid" {
            set_footer_kid(&mut footer, val, func)?;
            continue;
        }
        let text = match val {
            Value::String(s) => s.to_string(),
            other => {
                return Err(format!(
                    "{}(): footer value for `{}` must be a String, got {}",
                    func,
                    name,
                    other.type_name()
                ))
            }
        };
        // `add_additional` refuses values that look like a serialized key
        // (`k4.local.`, `k4.secret.`, …). The footer travels in cleartext, so
        // that guard is load-bearing — don't route around it.
        footer.add_additional(&name, &text).map_err(|_| {
            format!(
                "{}(): `{}` cannot be used as a footer key, or its value looks like a \
                 serialized key (the footer is not encrypted — never put key material in it)",
                func, name
            )
        })?;
    }
    Ok(footer)
}

/// What `Paseto.encrypt` / `Paseto.sign` need beyond the claims themselves.
struct MintExtras {
    footer: Option<Footer>,
    implicit: Option<Vec<u8>>,
}

/// Assemble the claims, footer and implicit assertion for a mint operation.
///
/// Claims come from two places: the payload hash (where custom claims live,
/// and where a caller naturally writes `{"sub": "alice"}`) and the options
/// hash (registered claims plus token settings). Options win on a collision,
/// matching `jwt_sign`.
fn build_mint(
    payload: &Value,
    options: Option<&Value>,
    func: &str,
) -> Result<(Claims, MintExtras), String> {
    // `Claims::new()` stamps `iat`/`nbf` at now and `exp` an hour out. An
    // expiring-by-default token is the right failure mode: forgetting
    // `expires_in` yields a short-lived token, not an eternal one.
    let mut claims = Claims::new().map_err(|_| format!("{}(): failed to build claims", func))?;

    match payload {
        Value::Hash(hash) => {
            for (key, value) in hash.borrow().iter() {
                let name = match key {
                    HashKey::String(name) => name.to_string(),
                    other => {
                        return Err(format!(
                            "{}(): payload keys must be strings, got {:?}",
                            func, other
                        ))
                    }
                };
                if REGISTERED_CLAIMS.contains(&name.as_str()) {
                    set_registered(&mut claims, &name, value, func)?;
                } else {
                    claims
                        .add_additional(&name, value_to_json(value)?)
                        .map_err(|_| format!("{}(): cannot add claim `{}`", func, name))?;
                }
            }
        }
        Value::Null => {}
        other => {
            return Err(format!(
                "{}() expects a Hash payload, got {}",
                func,
                other.type_name()
            ))
        }
    }

    let mut extras = MintExtras {
        footer: None,
        implicit: None,
    };
    let Some(options) = options else {
        return Ok((claims, extras));
    };
    let options = reject_unknown_options(options, &MINT_OPTIONS, func)?;
    let options = options.borrow();

    let has = |name: &str| options.get(&HashKey::String(name.into())).is_some();
    let non_expiring = match options.get(&HashKey::String("non_expiring".into())) {
        Some(value) => bool_option(value, func, "non_expiring")?,
        None => false,
    };
    // Two units for one instant, or an instant *and* "never": silently
    // letting one win would mint a token expiring at a time the caller never
    // asked for.
    if has("exp") && has("expires_in") {
        return Err(format!(
            "{}(): pass either `exp` (an instant) or `expires_in` (seconds from now), not both",
            func
        ));
    }
    if non_expiring && (has("exp") || has("expires_in")) {
        return Err(format!(
            "{}(): `non_expiring` cannot be combined with `exp` or `expires_in`",
            func
        ));
    }

    // The footer is built before the loop so a `kid` option can extend it
    // whichever order the two keys appear in the hash.
    if let Some(value) = options.get(&HashKey::String("footer".into())) {
        extras.footer = Some(build_footer(value, func)?);
    }

    for (key, value) in options.iter() {
        let HashKey::String(name) = key else { continue };
        match name.as_ref() {
            "expires_in" => {
                let secs = match value {
                    Value::Int(secs) if *secs > 0 => *secs as u64,
                    Value::Int(_) => {
                        return Err(format!(
                            "{}(): `expires_in` must be a positive number of seconds \
                             (use {{\"non_expiring\": true}} for a token that never expires)",
                            func
                        ))
                    }
                    other => {
                        return Err(format!(
                            "{}(): `expires_in` expects an Int, got {}",
                            func,
                            other.type_name()
                        ))
                    }
                };
                claims
                    .set_expires_in(&std::time::Duration::from_secs(secs))
                    .map_err(|_| format!("{}(): `expires_in` is out of range", func))?;
            }
            "non_expiring" => {
                if non_expiring {
                    claims.non_expiring();
                }
            }
            // Handled above the loop.
            "footer" => {}
            "kid" => {
                // Shorthand for the one footer claim almost every rotating
                // deployment needs: the key id a verifier uses to pick the
                // right key. Equivalent to `{"footer": {"kid": ...}}`.
                let mut footer = extras.footer.take().unwrap_or_default();
                set_footer_kid(&mut footer, value, func)?;
                extras.footer = Some(footer);
            }
            "implicit" => {
                extras.implicit = Some(string_option(value, func, "implicit")?.into_bytes())
            }
            claim => set_registered(&mut claims, claim, value, func)?,
        }
    }

    Ok((claims, extras))
}

/// What `Paseto.decrypt` / `Paseto.verify` need beyond the key.
struct ReadRules {
    rules: ClaimsValidationRules,
    footer: Option<Footer>,
    implicit: Option<Vec<u8>>,
}

/// Assemble the validation rules for a read operation. Defaults are strict:
/// `exp` must be present and unexpired, `nbf`/`iat` must be present and
/// reached. Every relaxation is an explicit opt-in.
fn build_read_rules(options: Option<&Value>, func: &str) -> Result<ReadRules, String> {
    let mut read = ReadRules {
        rules: ClaimsValidationRules::new(),
        footer: None,
        implicit: None,
    };
    let Some(options) = options else {
        return Ok(read);
    };
    let options = reject_unknown_options(options, &READ_OPTIONS, func)?;
    for (key, value) in options.borrow().iter() {
        let HashKey::String(name) = key else { continue };
        match name.as_ref() {
            "audience" => read
                .rules
                .validate_audience_with(&string_option(value, func, "audience")?),
            "issuer" => read
                .rules
                .validate_issuer_with(&string_option(value, func, "issuer")?),
            "subject" => read
                .rules
                .validate_subject_with(&string_option(value, func, "subject")?),
            "jti" => read
                .rules
                .validate_token_identifier_with(&string_option(value, func, "jti")?),
            "allow_non_expiring" => {
                let allowed = bool_option(value, func, "allow_non_expiring")?;
                if allowed {
                    read.rules.allow_non_expiring();
                }
            }
            "skip_valid_at" => {
                let skip = bool_option(value, func, "skip_valid_at")?;
                if skip {
                    read.rules.disable_valid_at();
                }
            }
            "footer" => read.footer = Some(build_footer(value, func)?),
            "implicit" => {
                read.implicit = Some(string_option(value, func, "implicit")?.into_bytes())
            }
            _ => {}
        }
    }
    Ok(read)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Turn a pasetors error into a message that says what to do about it.
/// `Display` on the upstream error prints the bare enum variant
/// (`ClaimValidation(Exp)`), which is not something to put in front of a user.
fn describe(err: &PasetoError, func: &str) -> String {
    let detail = match err {
        PasetoError::ClaimValidation(claim) => match claim {
            ClaimValidationError::Exp => "the token has expired".to_string(),
            ClaimValidationError::Nbf => "the token is not valid yet (`nbf` is in the future)".to_string(),
            ClaimValidationError::Iat => "the token was issued in the future (`iat`)".to_string(),
            ClaimValidationError::Aud => "the `aud` claim does not match the expected audience".to_string(),
            ClaimValidationError::Iss => "the `iss` claim does not match the expected issuer".to_string(),
            ClaimValidationError::Sub => "the `sub` claim does not match the expected subject".to_string(),
            ClaimValidationError::Jti => "the `jti` claim does not match the expected token identifier".to_string(),
            ClaimValidationError::NoExp => "the token has no `exp` claim (pass {\"allow_non_expiring\": true} to accept tokens that never expire)".to_string(),
            ClaimValidationError::NoIat | ClaimValidationError::NoNbf => "the token is missing `iat`/`nbf` (pass {\"skip_valid_at\": true} to accept tokens minted without them)".to_string(),
            ClaimValidationError::NoAud => "the token has no `aud` claim, but an audience was expected".to_string(),
            ClaimValidationError::NoIss => "the token has no `iss` claim, but an issuer was expected".to_string(),
            ClaimValidationError::NoSub => "the token has no `sub` claim, but a subject was expected".to_string(),
            ClaimValidationError::NoJti => "the token has no `jti` claim, but a token identifier was expected".to_string(),
            ClaimValidationError::NoStrExp
            | ClaimValidationError::NoStrIat
            | ClaimValidationError::NoStrNbf
            | ClaimValidationError::ParseExp
            | ClaimValidationError::ParseIat
            | ClaimValidationError::ParseNbf => "a date claim is not an RFC 3339 timestamp".to_string(),
        },
        // One message for "the crypto said no", whatever the reason: a
        // verifier that distinguishes a bad tag from a bad key hands an
        // attacker an oracle.
        PasetoError::TokenValidation | PasetoError::Encryption | PasetoError::Signing => {
            "the token is not authentic (wrong key, or it was tampered with)".to_string()
        }
        PasetoError::TokenFormat | PasetoError::Base64 => {
            "the token is malformed (expected `v4.local.…` or `v4.public.…`)".to_string()
        }
        PasetoError::Key | PasetoError::KeyGeneration | PasetoError::PaserkParsing => {
            "the key could not be read".to_string()
        }
        PasetoError::FooterParsing => {
            "the footer does not match the expected footer".to_string()
        }
        PasetoError::ClaimInvalidJson
        | PasetoError::ClaimInvalidUtf8
        | PasetoError::PayloadInvalidUtf8 => "the payload is not valid JSON".to_string(),
        other => format!("{:?}", other),
    };
    format!("{}(): {}", func, detail)
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// Convert a verified token's claims into a Soli Hash.
///
/// `exp`/`nbf`/`iat` come back as the RFC 3339 strings the token carries —
/// PASETO defines them that way, and rewriting them as Unix ints here would
/// mean the hash no longer matches what was signed. They have already been
/// validated by the time a caller sees them.
fn claims_to_hash(token: &TrustedToken, func: &str) -> Result<Value, String> {
    let claims = token
        .payload_claims()
        .ok_or_else(|| format!("{}(): verified token carried no claims", func))?;
    let json: JsonValue = serde_json::from_str(
        &claims
            .to_string()
            .map_err(|_| format!("{}(): claims could not be read", func))?,
    )
    .map_err(|e| format!("{}(): claims are not valid JSON: {}", func, e))?;
    json_to_value(json)
}

/// Footer bytes → Soli value. A footer built by `Paseto.*` is a JSON object,
/// but a token minted elsewhere may carry an opaque string, so hand back
/// whichever it actually is instead of erroring.
fn footer_to_value(bytes: &[u8]) -> Result<Value, String> {
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let text = String::from_utf8_lossy(bytes).to_string();
    match serde_json::from_str::<JsonValue>(&text) {
        Ok(json @ JsonValue::Object(_)) => json_to_value(json),
        _ => Ok(Value::String(text.into())),
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the `Paseto` class in the given environment.
pub fn register_paseto_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Paseto.generate_local_key() -> "k4.local.…"
    statics.insert(
        "generate_local_key".to_string(),
        Rc::new(NativeFunction::new(
            "Paseto.generate_local_key",
            Some(0),
            |_args| {
                let key = SymmetricKey::<V4>::generate()
                    .map_err(|e| describe(&e, "Paseto.generate_local_key"))?;
                Ok(Value::String(to_paserk(&key)?.into()))
            },
        )),
    );

    // Paseto.generate_key_pair() -> {"secret": "k4.secret.…", "public": "k4.public.…"}
    statics.insert(
        "generate_key_pair".to_string(),
        Rc::new(NativeFunction::new(
            "Paseto.generate_key_pair",
            Some(0),
            |_args| {
                let pair = AsymmetricKeyPair::<V4>::generate()
                    .map_err(|e| describe(&e, "Paseto.generate_key_pair"))?;
                let mut pairs = HashPairs::default();
                pairs.insert(
                    HashKey::String("secret".into()),
                    Value::String(to_paserk(&pair.secret)?.into()),
                );
                pairs.insert(
                    HashKey::String("public".into()),
                    Value::String(to_paserk(&pair.public)?.into()),
                );
                Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
            },
        )),
    );

    // Paseto.public_key(secret) -> "k4.public.…" — so a deployment can store
    // only the secret and hand out the verifying half.
    statics.insert(
        "public_key".to_string(),
        Rc::new(NativeFunction::new("Paseto.public_key", Some(1), |args| {
            let func = "Paseto.public_key";
            let raw = key_arg(args, 0, func, "secret key")?;
            let secret = parse_secret_key(&raw, func)?;
            let public =
                AsymmetricPublicKey::<V4>::try_from(&secret).map_err(|e| describe(&e, func))?;
            Ok(Value::String(to_paserk(&public)?.into()))
        })),
    );

    // Paseto.key_id(key) -> "k4.lid.…" / "k4.sid.…" / "k4.pid.…"
    //
    // A PASERK key id: a hash of the key, safe to publish. Put it in a
    // token's footer as `kid` so a verifier can pick the right key after a
    // rotation without the key itself appearing anywhere.
    statics.insert(
        "key_id".to_string(),
        Rc::new(NativeFunction::new("Paseto.key_id", Some(1), |args| {
            let func = "Paseto.key_id";
            let raw = key_arg(args, 0, func, "key")?;
            let id = if raw.starts_with("k4.local.") {
                Id::from(&parse_local_key(&raw, func)?)
            } else if raw.starts_with("k4.secret.") {
                Id::from(&parse_secret_key(&raw, func)?)
            } else if raw.starts_with("k4.public.") {
                Id::from(&parse_public_key(&raw, func)?)
            } else {
                return Err(format!(
                    "{}(): expects a PASERK key string (`k4.local.`, `k4.secret.` or \
                     `k4.public.`) — a raw hex key carries no purpose to derive an id from",
                    func
                ));
            };
            Ok(Value::String(to_paserk(&id)?.into()))
        })),
    );

    // Paseto.encrypt(payload, key, options?) -> "v4.local.…"
    statics.insert(
        "encrypt".to_string(),
        Rc::new(NativeFunction::new("Paseto.encrypt", None, |args| {
            let func = "Paseto.encrypt";
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "{}() expects 2 or 3 arguments (payload, key, options?), got {}",
                    func,
                    args.len()
                ));
            }
            let key = parse_local_key(&key_arg(args, 1, func, "key")?, func)?;
            let (claims, extras) = build_mint(&args[0], args.get(2), func)?;
            let token = local::encrypt(
                &key,
                &claims,
                extras.footer.as_ref(),
                extras.implicit.as_deref(),
            )
            .map_err(|e| describe(&e, func))?;
            Ok(Value::String(token.into()))
        })),
    );

    // Paseto.decrypt(token, key, options?) -> claims Hash (raises on failure)
    statics.insert(
        "decrypt".to_string(),
        Rc::new(NativeFunction::new("Paseto.decrypt", None, |args| {
            let func = "Paseto.decrypt";
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "{}() expects 2 or 3 arguments (token, key, options?), got {}",
                    func,
                    args.len()
                ));
            }
            let raw_token = key_arg(args, 0, func, "token")?;
            if raw_token.starts_with("v4.public.") {
                return Err(format!(
                    "{}(): got a v4.public token — use Paseto.verify() for signed tokens",
                    func
                ));
            }
            let key = parse_local_key(&key_arg(args, 1, func, "key")?, func)?;
            let read = build_read_rules(args.get(2), func)?;
            let untrusted = UntrustedToken::<Local, V4>::try_from(raw_token.as_str())
                .map_err(|e| describe(&e, func))?;
            let trusted = local::decrypt(
                &key,
                &untrusted,
                &read.rules,
                read.footer.as_ref(),
                read.implicit.as_deref(),
            )
            .map_err(|e| describe(&e, func))?;
            claims_to_hash(&trusted, func)
        })),
    );

    // Paseto.sign(payload, secret_key, options?) -> "v4.public.…"
    statics.insert(
        "sign".to_string(),
        Rc::new(NativeFunction::new("Paseto.sign", None, |args| {
            let func = "Paseto.sign";
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "{}() expects 2 or 3 arguments (payload, secret_key, options?), got {}",
                    func,
                    args.len()
                ));
            }
            let key = parse_secret_key(&key_arg(args, 1, func, "secret key")?, func)?;
            let (claims, extras) = build_mint(&args[0], args.get(2), func)?;
            let token = public::sign(
                &key,
                &claims,
                extras.footer.as_ref(),
                extras.implicit.as_deref(),
            )
            .map_err(|e| describe(&e, func))?;
            Ok(Value::String(token.into()))
        })),
    );

    // Paseto.verify(token, public_key, options?) -> claims Hash (raises on failure)
    statics.insert(
        "verify".to_string(),
        Rc::new(NativeFunction::new("Paseto.verify", None, |args| {
            let func = "Paseto.verify";
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "{}() expects 2 or 3 arguments (token, public_key, options?), got {}",
                    func,
                    args.len()
                ));
            }
            let raw_token = key_arg(args, 0, func, "token")?;
            if raw_token.starts_with("v4.local.") {
                return Err(format!(
                    "{}(): got a v4.local token — use Paseto.decrypt() for encrypted tokens",
                    func
                ));
            }
            let key = parse_public_key(&key_arg(args, 1, func, "public key")?, func)?;
            let read = build_read_rules(args.get(2), func)?;
            let untrusted = UntrustedToken::<Public, V4>::try_from(raw_token.as_str())
                .map_err(|e| describe(&e, func))?;
            let trusted = public::verify(
                &key,
                &untrusted,
                &read.rules,
                read.footer.as_ref(),
                read.implicit.as_deref(),
            )
            .map_err(|e| describe(&e, func))?;
            claims_to_hash(&trusted, func)
        })),
    );

    // Paseto.decode_unsafe(token) -> {unverified: true, version, purpose, claims, footer}
    //
    // Inspection only, and shaped so it cannot be mistaken for a verified
    // read: the claims sit behind `["claims"]`, so code that reaches for
    // `result["sub"]` gets `null` rather than an unauthenticated value. This
    // is the intended way to read a footer's `kid` *before* choosing a key —
    // the footer is covered by the signature, so the following verify still
    // catches a tampered one.
    statics.insert(
        "decode_unsafe".to_string(),
        Rc::new(NativeFunction::new(
            "Paseto.decode_unsafe",
            Some(1),
            |args| {
                let func = "Paseto.decode_unsafe";
                let token = key_arg(args, 0, func, "token")?;
                let mut pairs = HashPairs::default();
                pairs.insert(HashKey::String("unverified".into()), Value::Bool(true));
                pairs.insert(
                    HashKey::String("version".into()),
                    Value::String("v4".into()),
                );

                if token.starts_with("v4.local.") {
                    let untrusted = UntrustedToken::<Local, V4>::try_from(token.as_str())
                        .map_err(|e| describe(&e, func))?;
                    pairs.insert(
                        HashKey::String("purpose".into()),
                        Value::String("local".into()),
                    );
                    // The payload of a local token is ciphertext. There is
                    // nothing to show without the key, which is the point.
                    pairs.insert(HashKey::String("claims".into()), Value::Null);
                    pairs.insert(
                        HashKey::String("footer".into()),
                        footer_to_value(untrusted.untrusted_footer())?,
                    );
                } else if token.starts_with("v4.public.") {
                    let untrusted = UntrustedToken::<Public, V4>::try_from(token.as_str())
                        .map_err(|e| describe(&e, func))?;
                    pairs.insert(
                        HashKey::String("purpose".into()),
                        Value::String("public".into()),
                    );
                    let payload = untrusted.untrusted_payload();
                    let claims = match serde_json::from_slice::<JsonValue>(payload) {
                        Ok(json) => json_to_value(json)?,
                        Err(_) => Value::Null,
                    };
                    pairs.insert(HashKey::String("claims".into()), claims);
                    pairs.insert(
                        HashKey::String("footer".into()),
                        footer_to_value(untrusted.untrusted_footer())?,
                    );
                } else {
                    return Err(format!(
                        "{}(): not a PASETO v4 token (expected `v4.local.…` or `v4.public.…`)",
                        func
                    ));
                }

                Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
            },
        )),
    );

    let paseto_class = Class {
        name: "Paseto".to_string(),
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
    env.define("Paseto".to_string(), Value::Class(Rc::new(paseto_class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paseto_fn(env: &Environment, name: &str) -> Rc<NativeFunction> {
        match env.get("Paseto") {
            Some(Value::Class(class)) => class
                .native_static_methods
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("Paseto.{name} is not registered")),
            other => panic!("expected the Paseto class, got {other:?}"),
        }
    }

    fn fresh_env() -> Environment {
        let mut env = Environment::new();
        register_paseto_builtins(&mut env);
        env
    }

    fn call(name: &str, args: &[Value]) -> Result<Value, String> {
        let env = fresh_env();
        (paseto_fn(&env, name).func)(args)
    }

    fn hash(pairs: &[(&str, Value)]) -> Value {
        let mut h = HashPairs::default();
        for (key, value) in pairs {
            h.insert(HashKey::String((*key).into()), value.clone());
        }
        Value::Hash(Rc::new(RefCell::new(h)))
    }

    fn text(value: &Value) -> String {
        match value {
            Value::String(s) => s.to_string(),
            other => panic!("expected a String, got {other:?}"),
        }
    }

    fn claim(value: &Value, key: &str) -> Option<Value> {
        match value {
            Value::Hash(h) => h.borrow().get(&HashKey::String(key.into())).cloned(),
            other => panic!("expected a Hash, got {other:?}"),
        }
    }

    fn local_key() -> Value {
        call("generate_local_key", &[]).unwrap()
    }

    fn key_pair() -> (Value, Value) {
        let pair = call("generate_key_pair", &[]).unwrap();
        (
            claim(&pair, "secret").unwrap(),
            claim(&pair, "public").unwrap(),
        )
    }

    #[test]
    fn generated_keys_are_paserk_strings() {
        assert!(text(&local_key()).starts_with("k4.local."));
        let (secret, public) = key_pair();
        assert!(text(&secret).starts_with("k4.secret."));
        assert!(text(&public).starts_with("k4.public."));
    }

    #[test]
    fn local_round_trip_carries_custom_and_registered_claims() {
        let key = local_key();
        let token = call(
            "encrypt",
            &[
                hash(&[
                    ("sub", Value::String("alice".into())),
                    ("role", Value::String("admin".into())),
                ]),
                key.clone(),
                hash(&[("expires_in", Value::Int(600))]),
            ],
        )
        .unwrap();
        assert!(text(&token).starts_with("v4.local."));

        let claims = call("decrypt", &[token, key]).unwrap();
        assert_eq!(claim(&claims, "sub"), Some(Value::String("alice".into())));
        assert_eq!(claim(&claims, "role"), Some(Value::String("admin".into())));
        // PASETO dates are RFC 3339 strings, not Unix ints.
        assert!(text(&claim(&claims, "exp").unwrap()).contains('T'));
    }

    #[test]
    fn public_round_trip_verifies_with_the_public_half() {
        let (secret, public) = key_pair();
        let token = call(
            "sign",
            &[
                hash(&[("sub", Value::String("bob".into()))]),
                secret,
                hash(&[("expires_in", Value::Int(600))]),
            ],
        )
        .unwrap();
        assert!(text(&token).starts_with("v4.public."));

        let claims = call("verify", &[token, public]).unwrap();
        assert_eq!(claim(&claims, "sub"), Some(Value::String("bob".into())));
    }

    /// The whole point of PASETO over JWT: purpose and version are part of
    /// the token, so the wrong operation can't be talked into running.
    #[test]
    fn purposes_do_not_cross() {
        let key = local_key();
        let (secret, public) = key_pair();
        let local_token = call(
            "encrypt",
            &[hash(&[("sub", Value::String("a".into()))]), key.clone()],
        )
        .unwrap();
        let public_token = call(
            "sign",
            &[hash(&[("sub", Value::String("a".into()))]), secret],
        )
        .unwrap();

        // A local token handed to the signature path (and vice versa) is
        // rejected on the header alone, before any key is used.
        let err = call("verify", &[local_token, public]).unwrap_err();
        assert!(err.contains("Paseto.decrypt()"), "{err}");
        let err = call("decrypt", &[public_token.clone(), key]).unwrap_err();
        assert!(err.contains("Paseto.verify()"), "{err}");

        // A valid token does not verify under a different key pair.
        let (_, other_public) = key_pair();
        let err = call("verify", &[public_token, other_public]).unwrap_err();
        assert!(err.contains("not authentic"), "{err}");
    }

    #[test]
    fn tampered_token_is_rejected() {
        let (secret, public) = key_pair();
        let token = text(
            &call(
                "sign",
                &[hash(&[("sub", Value::String("alice".into()))]), secret],
            )
            .unwrap(),
        );
        // Flip a character in the payload segment.
        let mut parts: Vec<String> = token.split('.').map(str::to_string).collect();
        let payload = &mut parts[2];
        let first = payload.remove(0);
        payload.insert(0, if first == 'a' { 'b' } else { 'a' });
        let tampered = parts.join(".");

        let err = call("verify", &[Value::String(tampered.into()), public]).unwrap_err();
        assert!(err.contains("not authentic"), "{err}");
    }

    #[test]
    fn wrong_key_is_rejected() {
        let token = call(
            "encrypt",
            &[hash(&[("sub", Value::String("alice".into()))]), local_key()],
        )
        .unwrap();
        let err = call("decrypt", &[token, local_key()]).unwrap_err();
        assert!(err.contains("not authentic"), "{err}");
    }

    #[test]
    fn expired_token_is_rejected_with_a_readable_message() {
        let key = local_key();
        let token = call(
            "encrypt",
            &[
                hash(&[("sub", Value::String("alice".into()))]),
                key.clone(),
                // An `exp` in the past, expressed as a Unix timestamp.
                hash(&[("exp", Value::Int(1_000_000_000))]),
            ],
        )
        .unwrap();
        let err = call("decrypt", &[token, key]).unwrap_err();
        assert!(err.contains("has expired"), "{err}");
    }

    /// A token that never expires must be opted into on both ends, so a
    /// missing `exp` can't quietly become an eternal credential.
    #[test]
    fn non_expiring_requires_opt_in_on_read() {
        let key = local_key();
        let token = call(
            "encrypt",
            &[
                hash(&[("sub", Value::String("alice".into()))]),
                key.clone(),
                hash(&[("non_expiring", Value::Bool(true))]),
            ],
        )
        .unwrap();

        let err = call("decrypt", &[token.clone(), key.clone()]).unwrap_err();
        assert!(err.contains("allow_non_expiring"), "{err}");

        let claims = call(
            "decrypt",
            &[
                token,
                key,
                hash(&[("allow_non_expiring", Value::Bool(true))]),
            ],
        )
        .unwrap();
        assert_eq!(claim(&claims, "sub"), Some(Value::String("alice".into())));
    }

    #[test]
    fn audience_and_issuer_are_checked_when_expected() {
        let (secret, public) = key_pair();
        let token = call(
            "sign",
            &[
                hash(&[("sub", Value::String("alice".into()))]),
                secret,
                hash(&[
                    ("aud", Value::String("api.example.com".into())),
                    ("iss", Value::String("https://issuer.test".into())),
                ]),
            ],
        )
        .unwrap();

        let claims = call(
            "verify",
            &[
                token.clone(),
                public.clone(),
                hash(&[
                    ("audience", Value::String("api.example.com".into())),
                    ("issuer", Value::String("https://issuer.test".into())),
                ]),
            ],
        )
        .unwrap();
        assert_eq!(
            claim(&claims, "aud"),
            Some(Value::String("api.example.com".into()))
        );

        let err = call(
            "verify",
            &[
                token,
                public,
                hash(&[("audience", Value::String("other.example.com".into()))]),
            ],
        )
        .unwrap_err();
        assert!(err.contains("aud"), "{err}");
    }

    /// A typo in a security option must not read as a check that passed.
    #[test]
    fn unknown_options_are_rejected() {
        let (secret, public) = key_pair();
        let err = call(
            "sign",
            &[
                hash(&[("sub", Value::String("alice".into()))]),
                secret.clone(),
                hash(&[("expires", Value::Int(60))]),
            ],
        )
        .unwrap_err();
        assert!(err.contains("unknown option `expires`"), "{err}");

        let token = call(
            "sign",
            &[hash(&[("sub", Value::String("a".into()))]), secret],
        )
        .unwrap();
        let err = call(
            "verify",
            &[
                token,
                public,
                hash(&[("audiance", Value::String("api".into()))]),
            ],
        )
        .unwrap_err();
        assert!(err.contains("unknown option `audiance`"), "{err}");
    }

    #[test]
    fn exp_and_expires_in_together_are_rejected() {
        let err = call(
            "encrypt",
            &[
                hash(&[("sub", Value::String("a".into()))]),
                local_key(),
                hash(&[
                    ("exp", Value::Int(4_102_444_800)),
                    ("expires_in", Value::Int(60)),
                ]),
            ],
        )
        .unwrap_err();
        assert!(err.contains("not both"), "{err}");
    }

    /// The footer is authenticated but unencrypted, which is what makes
    /// `kid`-based key rotation possible: read it before picking a key.
    #[test]
    fn footer_kid_is_readable_before_verification() {
        let (secret, public) = key_pair();
        let kid = text(&call("key_id", std::slice::from_ref(&public)).unwrap());
        assert!(kid.starts_with("k4.pid."));

        let token = call(
            "sign",
            &[
                hash(&[("sub", Value::String("alice".into()))]),
                secret,
                hash(&[("kid", Value::String(kid.clone().into()))]),
            ],
        )
        .unwrap();

        let decoded = call("decode_unsafe", std::slice::from_ref(&token)).unwrap();
        assert_eq!(claim(&decoded, "unverified"), Some(Value::Bool(true)));
        assert_eq!(
            claim(&decoded, "purpose"),
            Some(Value::String("public".into()))
        );
        let footer = claim(&decoded, "footer").unwrap();
        assert_eq!(claim(&footer, "kid"), Some(Value::String(kid.into())));

        // Claims of a *public* token are readable unverified — the wrapper
        // shape is what keeps that from being mistaken for a verified read.
        let claims = claim(&decoded, "claims").unwrap();
        assert_eq!(claim(&claims, "sub"), Some(Value::String("alice".into())));
        assert_eq!(claim(&decoded, "sub"), None);

        // Pinning the footer at verify time still works.
        let verified = call(
            "verify",
            &[
                token,
                public,
                hash(&[("footer", claim(&decoded, "footer").unwrap())]),
            ],
        )
        .unwrap();
        assert_eq!(claim(&verified, "sub"), Some(Value::String("alice".into())));
    }

    /// A local token's payload is encrypted, so inspection reveals nothing
    /// but the footer.
    #[test]
    fn decode_unsafe_hides_local_claims() {
        let token = call(
            "encrypt",
            &[
                hash(&[("secret_note", Value::String("classified".into()))]),
                local_key(),
            ],
        )
        .unwrap();
        let decoded = call("decode_unsafe", &[token]).unwrap();
        assert_eq!(
            claim(&decoded, "purpose"),
            Some(Value::String("local".into()))
        );
        assert_eq!(claim(&decoded, "claims"), Some(Value::Null));
    }

    /// Implicit assertions are bound into the authentication tag without
    /// travelling in the token, so a mismatch has to fail.
    #[test]
    fn implicit_assertion_must_match() {
        let key = local_key();
        let token = call(
            "encrypt",
            &[
                hash(&[("sub", Value::String("alice".into()))]),
                key.clone(),
                hash(&[("implicit", Value::String("session-42".into()))]),
            ],
        )
        .unwrap();

        let claims = call(
            "decrypt",
            &[
                token.clone(),
                key.clone(),
                hash(&[("implicit", Value::String("session-42".into()))]),
            ],
        )
        .unwrap();
        assert_eq!(claim(&claims, "sub"), Some(Value::String("alice".into())));

        let err = call(
            "decrypt",
            &[
                token,
                key,
                hash(&[("implicit", Value::String("session-99".into()))]),
            ],
        )
        .unwrap_err();
        assert!(err.contains("not authentic"), "{err}");
    }

    #[test]
    fn public_key_is_derivable_from_the_secret() {
        let (secret, public) = key_pair();
        let derived = call("public_key", &[secret]).unwrap();
        assert_eq!(derived, public);
    }

    /// Verifying with the secret key would mean the signing key was shipped
    /// to the verifier. Refuse it rather than making it work.
    #[test]
    fn verify_refuses_a_secret_key() {
        let (secret, _) = key_pair();
        let token = call(
            "sign",
            &[hash(&[("sub", Value::String("a".into()))]), secret.clone()],
        )
        .unwrap();
        let err = call("verify", &[token, secret]).unwrap_err();
        assert!(err.contains("public_key"), "{err}");
    }

    /// Raw hex keys are accepted so `openssl rand -hex 32` works, but the
    /// length has to be right.
    #[test]
    fn hex_local_keys_are_accepted_and_length_checked() {
        let hex_key = Value::String("0123456789abcdef".repeat(4).into()); // 32 bytes
        let token = call(
            "encrypt",
            &[
                hash(&[("sub", Value::String("alice".into()))]),
                hex_key.clone(),
            ],
        )
        .unwrap();
        let claims = call("decrypt", &[token, hex_key]).unwrap();
        assert_eq!(claim(&claims, "sub"), Some(Value::String("alice".into())));

        let err = call(
            "encrypt",
            &[
                hash(&[("sub", Value::String("a".into()))]),
                Value::String("too-short".into()),
            ],
        )
        .unwrap_err();
        assert!(err.contains("k4.local."), "{err}");
    }

    /// An all-zero Ed25519 seed makes ed25519-compact panic rather than error,
    /// and a panic in a builtin takes the worker thread with it. Both key forms
    /// have to be screened before pasetors derives the key pair.
    #[test]
    fn all_zero_seed_errors_instead_of_panicking() {
        let hex_zero = Value::String("0".repeat(128).into());
        let err = call(
            "sign",
            &[hash(&[("sub", Value::String("a".into()))]), hex_zero],
        )
        .unwrap_err();
        assert!(err.contains("all zeroes"), "{err}");

        // Same key expressed as PASERK — the path that reaches the panic first.
        let paserk_zero =
            Value::String(format!("k4.secret.{}", URL_SAFE_NO_PAD.encode(vec![0u8; 64])).into());
        let err = call(
            "sign",
            &[hash(&[("sub", Value::String("a".into()))]), paserk_zero],
        )
        .unwrap_err();
        assert!(err.contains("all zeroes"), "{err}");
    }

    /// The symmetric and public paths have no such derivation, so a zero key
    /// there must simply fail to authenticate — not panic, and not verify.
    #[test]
    fn all_zero_local_and_public_keys_fail_cleanly() {
        let zero_32 = Value::String("0".repeat(64).into());
        // A zero symmetric key is structurally valid; it just can't read a token
        // minted under a real one.
        let token = call(
            "encrypt",
            &[hash(&[("sub", Value::String("a".into()))]), local_key()],
        )
        .unwrap();
        let err = call("decrypt", &[token, zero_32.clone()]).unwrap_err();
        assert!(err.contains("not authentic"), "{err}");

        let (secret, _) = key_pair();
        let signed = call(
            "sign",
            &[hash(&[("sub", Value::String("a".into()))]), secret],
        )
        .unwrap();
        let err = call("verify", &[signed, zero_32]).unwrap_err();
        assert!(err.contains("not authentic"), "{err}");
    }

    #[test]
    fn decode_unsafe_rejects_a_non_paseto_string() {
        let err = call(
            "decode_unsafe",
            &[Value::String("eyJhbGciOiJIUzI1NiJ9.e30.sig".into())],
        )
        .unwrap_err();
        assert!(err.contains("not a PASETO v4 token"), "{err}");
    }
}
