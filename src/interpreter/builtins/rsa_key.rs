//! RSA private-key parsing built-in class for SoliLang.
//!
//! The verification side ([`X509.public_key`]) extracts `(n, e)` from a
//! certificate. To *sign* — produce an XML-DSig enveloped signature — a Service
//! Provider needs its own `(n, d)`. `RsaKey.private_from_pem` parses a PKCS#8
//! (`-----BEGIN PRIVATE KEY-----`) or PKCS#1 (`-----BEGIN RSA PRIVATE KEY-----`)
//! PEM key and returns the components as hex, ready for [`Crypto.modexp`]:
//!
//! ```text
//! key = RsaKey.private_from_pem(sp_private_key_pem)
//! sig = Crypto.modexp(padded_digest_info, key["d"], key["n"])
//! ```
//!
//! We deliberately parse with the lightweight RustCrypto ASN.1 crates
//! (`der` / `pkcs1` / `pkcs8`) instead of the `rsa` crate, whose Marvin-attack
//! advisory (RUSTSEC-2023-0071) has no fixed release. We only read key
//! material here — none of `rsa`'s timing-sensitive operations are used.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use base64::Engine;
use der::Decode;
use pkcs1::{RsaPrivateKey as Pkcs1PrivateKey, RsaPublicKey as Pkcs1PublicKey};
use pkcs8::spki::SubjectPublicKeyInfoRef;
use pkcs8::PrivateKeyInfo;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, NativeFunction, Value};

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Decode a PEM document into its label and DER bytes.
fn pem_to_der(pem: &str) -> Result<(String, Vec<u8>), String> {
    let begin = "-----BEGIN ";
    let start = pem.find(begin).ok_or("missing PEM header")?;
    let after = &pem[start + begin.len()..];
    let label_end = after.find("-----").ok_or("malformed PEM header")?;
    let label = after[..label_end].trim().to_string();

    let body: String = pem
        .lines()
        .skip_while(|l| !l.contains("-----BEGIN"))
        .skip(1)
        .take_while(|l| !l.contains("-----END"))
        .flat_map(|l| l.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.as_bytes())
        .map_err(|e| format!("invalid PEM base64: {}", e))?;
    Ok((label, der))
}

/// RSA components `(n, e, d)` as big-endian octets.
type RsaComponents = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Parse a PEM RSA private key (PKCS#8 or PKCS#1) and return `(n, e, d)` as
/// big-endian octets.
fn parse_components(pem: &str) -> Result<RsaComponents, String> {
    let (label, der) = pem_to_der(pem)?;

    // PKCS#8 wraps the PKCS#1 RSAPrivateKey in a PrivateKeyInfo; unwrap it.
    // Otherwise the DER is the RSAPrivateKey directly.
    let inner_der: Vec<u8> = if label.contains("RSA PRIVATE KEY") {
        der.clone()
    } else {
        let pki =
            PrivateKeyInfo::from_der(&der).map_err(|e| format!("PKCS#8 parse error: {}", e))?;
        pki.private_key.to_vec()
    };

    let key = Pkcs1PrivateKey::from_der(&inner_der)
        .map_err(|e| format!("RSA private key (PKCS#1) parse error: {}", e))?;
    Ok((
        key.modulus.as_bytes().to_vec(),
        key.public_exponent.as_bytes().to_vec(),
        key.private_exponent.as_bytes().to_vec(),
    ))
}

/// Parse a PEM RSA **public** key — SPKI (`-----BEGIN PUBLIC KEY-----`) or
/// PKCS#1 (`-----BEGIN RSA PUBLIC KEY-----`) — and return `(n, e)` as
/// big-endian octets.
///
/// `X509.public_key` covers the certificate case; this covers a bare public
/// key, which is what an OIDC provider publishes in its JWKS and what a
/// relying party is handed for verification.
fn parse_public_components(pem: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (label, der) = pem_to_der(pem)?;

    // SPKI wraps the PKCS#1 RSAPublicKey in a SubjectPublicKeyInfo; unwrap it.
    let inner_der: Vec<u8> = if label.contains("RSA PUBLIC KEY") {
        der.clone()
    } else {
        let spki = SubjectPublicKeyInfoRef::from_der(&der)
            .map_err(|e| format!("SPKI parse error: {}", e))?;
        spki.subject_public_key
            .as_bytes()
            .ok_or("SPKI public key is not byte-aligned")?
            .to_vec()
    };

    let key = Pkcs1PublicKey::from_der(&inner_der)
        .map_err(|e| format!("RSA public key (PKCS#1) parse error: {}", e))?;
    Ok((
        key.modulus.as_bytes().to_vec(),
        key.public_exponent.as_bytes().to_vec(),
    ))
}

/// DER `DigestInfo` prefix for each digest accepted by `RsaKey.verify`
/// (RFC 8017 section 9.2, note 1), and the digest of `message`.
fn digest_info(algorithm: &str, message: &[u8]) -> Result<Vec<u8>, String> {
    use sha1::Sha1;
    use sha2::{Digest, Sha256, Sha512};
    let (prefix, digest): (&[u8], Vec<u8>) = match algorithm {
        "sha1" => (
            &[
                0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04,
                0x14,
            ],
            Sha1::digest(message).to_vec(),
        ),
        "sha256" => (
            &[
                0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x01, 0x05, 0x00, 0x04, 0x20,
            ],
            Sha256::digest(message).to_vec(),
        ),
        "sha512" => (
            &[
                0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x03, 0x05, 0x00, 0x04, 0x40,
            ],
            Sha512::digest(message).to_vec(),
        ),
        other => {
            return Err(format!(
                "unsupported digest '{}' (expected sha1, sha256 or sha512)",
                other
            ))
        }
    };
    let mut out = prefix.to_vec();
    out.extend_from_slice(&digest);
    Ok(out)
}

/// RSASSA-PKCS1-v1_5 verification (RFC 8017 section 8.2.2): rebuild the
/// expected encoded message and compare it in constant time with
/// `signature^e mod n`. Any malformed input is simply "not valid".
fn verify_pkcs1_v15(
    pem: &str,
    message: &[u8],
    signature: &[u8],
    algorithm: &str,
) -> Result<bool, String> {
    let (n, e) = parse_public_components(pem)?;
    let n: Vec<u8> = n.iter().skip_while(|b| **b == 0).copied().collect();
    let k = n.len();
    let t = digest_info(algorithm, message)?;
    if signature.len() != k || k < t.len() + 11 {
        return Ok(false);
    }
    // The signature representative must be smaller than the modulus.
    if num_bigint::BigUint::from_bytes_be(signature) >= num_bigint::BigUint::from_bytes_be(&n) {
        return Ok(false);
    }
    let em = crate::interpreter::builtins::crypto::do_modexp(signature, &e, &n)?;
    let mut expected = vec![0x00u8, 0x01];
    expected.extend(std::iter::repeat_n(0xffu8, k - t.len() - 3));
    expected.push(0x00);
    expected.extend_from_slice(&t);
    if em.len() != expected.len() {
        return Ok(false);
    }
    let diff = em
        .iter()
        .zip(expected.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    Ok(diff == 0)
}

/// Bytes of a `RsaKey.verify` argument. A message string is its UTF-8 bytes;
/// a signature string is base64 (standard or URL-safe alphabet). Arrays are
/// raw bytes.
fn arg_bytes(value: &Value, what: &str, base64_string: bool) -> Result<Vec<u8>, String> {
    match value {
        Value::String(s) if base64_string => {
            let text: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            base64::engine::general_purpose::STANDARD
                .decode(text.as_bytes())
                .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(text.as_bytes()))
                .or_else(|_| {
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.as_bytes())
                })
                .map_err(|e| format!("{}: invalid base64: {}", what, e))
        }
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                other => Err(format!(
                    "{}: expected byte (Int 0-255), got {}",
                    what,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{}: expected string or byte array, got {}",
            what,
            other.type_name()
        )),
    }
}

pub fn register_rsa_key_builtins(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // RsaKey.private_from_pem(pem) -> { algorithm, n, e, d, bits }
    methods.insert(
        "private_from_pem".to_string(),
        Rc::new(NativeFunction::new(
            "RsaKey.private_from_pem",
            Some(1),
            |args| {
                let pem = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "RsaKey.private_from_pem() expects string PEM, got {}",
                            other.type_name()
                        ))
                    }
                };
                let (n, e, d) = parse_components(&pem)
                    .map_err(|err| format!("RsaKey.private_from_pem(): {}", err))?;
                Ok(hash_from_pairs([
                    ("algorithm".to_string(), Value::String("RSA".into())),
                    ("n".to_string(), Value::String(bytes_to_hex(&n).into())),
                    ("e".to_string(), Value::String(bytes_to_hex(&e).into())),
                    ("d".to_string(), Value::String(bytes_to_hex(&d).into())),
                    ("bits".to_string(), Value::Int((n.len() * 8) as i64)),
                ]))
            },
        )),
    );

    // RsaKey.public_from_pem(pem) -> { algorithm, n, e, bits }
    methods.insert(
        "public_from_pem".to_string(),
        Rc::new(NativeFunction::new(
            "RsaKey.public_from_pem",
            Some(1),
            |args| {
                let pem = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "RsaKey.public_from_pem() expects string PEM, got {}",
                            other.type_name()
                        ))
                    }
                };
                let (n, e) = parse_public_components(&pem)
                    .map_err(|err| format!("RsaKey.public_from_pem(): {}", err))?;
                Ok(hash_from_pairs([
                    ("algorithm".to_string(), Value::String("RSA".into())),
                    ("n".to_string(), Value::String(bytes_to_hex(&n).into())),
                    ("e".to_string(), Value::String(bytes_to_hex(&e).into())),
                    ("bits".to_string(), Value::Int((n.len() * 8) as i64)),
                ]))
            },
        )),
    );

    // RsaKey.verify(public_pem, message, signature, algorithm?) -> Bool
    // RSASSA-PKCS1-v1_5; `signature` is base64 or a byte array; `algorithm`
    // is "sha256" (default), "sha1" or "sha512". A bad signature returns
    // false; only an unreadable key or an unknown algorithm raises.
    methods.insert(
        "verify".to_string(),
        Rc::new(NativeFunction::new("RsaKey.verify", None, |args| {
            if args.len() < 3 || args.len() > 4 {
                return Err(format!(
                    "RsaKey.verify() expects 3-4 arguments (public_pem, message, signature, algorithm?), got {}",
                    args.len()
                ));
            }
            let pem = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "RsaKey.verify() expects string PEM, got {}",
                        other.type_name()
                    ))
                }
            };
            let message = arg_bytes(&args[1], "RsaKey.verify() message", false)?;
            let signature = match arg_bytes(&args[2], "RsaKey.verify() signature", true) {
                Ok(bytes) => bytes,
                Err(_) => return Ok(Value::Bool(false)),
            };
            let algorithm = match args.get(3) {
                None | Some(Value::Null) => "sha256".to_string(),
                Some(Value::String(s)) => s.to_lowercase().to_string(),
                Some(other) => {
                    return Err(format!(
                        "RsaKey.verify() expects string algorithm, got {}",
                        other.type_name()
                    ))
                }
            };
            let valid = verify_pkcs1_v15(&pem, &message, &signature, &algorithm)
                .map_err(|err| format!("RsaKey.verify(): {}", err))?;
            Ok(Value::Bool(valid))
        })),
    );

    let class = Class {
        name: "RsaKey".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("RsaKey".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQCVNJCMOElvuFlwioV+yJrePWcB
f297VcdPey6eTprEaqwVln0QnSix6+8SZ+Lmhp/reqSQbbSU7CFjq2hE4ihycI9K
L4owJCcZguPsS8BfQ5N+oebbEECMJFy8gPSh5gNjeZBuv06XCGRttcKrdRnJ1Suu
RhwGmTE/QZENd9lsuQIDAQAB
-----END PUBLIC KEY-----";
    // `openssl dgst -sha1 -sign` / `-sha256 -sign` of TEST_MESSAGE with the
    // matching private key.
    const TEST_MESSAGE: &[u8] = b"amount=2500&reference=abc";
    const TEST_SIG_SHA1: &str = "DoxNElGwl2BVmyEqHYRymZzP/bgBE+fhzBOll7/uJxpM3RWdVFsc8FXC+fpnU9fu3/7ven5SXRGD1E9aquLFky9Dh7YYq3nKtFwajkafDBb3JbtBBlG8p8U4bRTdNaGtk+2aAjqCwj1f+ZDnRmQ5U/WbapkqHbLWXr3jNvO8jZk=";
    const TEST_SIG_SHA256: &str = "HLvC3i1aCsLBCV9ugJ7uhzJ2TCzJxAnEsPrwnZGlXGEvwFpXj8NJ45ce70Lp7L6cqNlYD5HS8BY6ZRALn0a1mKJCZUur9dg51bqriNFzr4+UxJV1pWb+qPXG3tW3BNU37lLUP0Zk5Z1jlQ8Uz/RE4p+Y3Ib+PS4m420VTpLABvA=";

    fn sig(b64: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap()
    }

    #[test]
    fn verify_accepts_openssl_signatures() {
        assert!(
            verify_pkcs1_v15(TEST_PUBLIC_PEM, TEST_MESSAGE, &sig(TEST_SIG_SHA1), "sha1").unwrap()
        );
        assert!(verify_pkcs1_v15(
            TEST_PUBLIC_PEM,
            TEST_MESSAGE,
            &sig(TEST_SIG_SHA256),
            "sha256"
        )
        .unwrap());
    }

    #[test]
    fn verify_rejects_tampering() {
        // Other message, other digest, flipped bit, truncated signature.
        assert!(!verify_pkcs1_v15(
            TEST_PUBLIC_PEM,
            b"amount=2600&reference=abc",
            &sig(TEST_SIG_SHA1),
            "sha1"
        )
        .unwrap());
        assert!(
            !verify_pkcs1_v15(TEST_PUBLIC_PEM, TEST_MESSAGE, &sig(TEST_SIG_SHA1), "sha256")
                .unwrap()
        );
        let mut flipped = sig(TEST_SIG_SHA1);
        flipped[10] ^= 1;
        assert!(!verify_pkcs1_v15(TEST_PUBLIC_PEM, TEST_MESSAGE, &flipped, "sha1").unwrap());
        assert!(!verify_pkcs1_v15(
            TEST_PUBLIC_PEM,
            TEST_MESSAGE,
            &sig(TEST_SIG_SHA1)[1..],
            "sha1"
        )
        .unwrap());
        assert!(
            verify_pkcs1_v15(TEST_PUBLIC_PEM, TEST_MESSAGE, &sig(TEST_SIG_SHA1), "md5").is_err()
        );
    }

    #[test]
    fn verify_signature_argument_accepts_url_safe_base64() {
        let url_safe = TEST_SIG_SHA1.replace('+', "-").replace('/', "_");
        let bytes = arg_bytes(&Value::String(url_safe.into()), "sig", true).unwrap();
        assert!(verify_pkcs1_v15(TEST_PUBLIC_PEM, TEST_MESSAGE, &bytes, "sha1").unwrap());
    }

    #[test]
    fn rejects_garbage_pem() {
        assert!(parse_components("not a key").is_err());
        assert!(
            parse_components("-----BEGIN PRIVATE KEY-----\nZm9v\n-----END PRIVATE KEY-----")
                .is_err()
        );
    }
}
