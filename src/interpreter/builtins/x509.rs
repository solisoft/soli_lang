//! X.509 certificate parsing built-in class for SoliLang.
//!
//! Extracts the RSA public key `(n, e)` from an X.509 certificate so a SAML
//! Service Provider can verify an IdP's signature with [`Crypto.modexp`]:
//! the IdP's signing certificate arrives base64-DER-encoded inside the SAML
//! metadata's `<ds:X509Certificate>` element.
//!
//! ```text
//! key = X509.public_key(metadata_cert_b64)   # { algorithm, n, e, bits }
//! em  = Crypto.modexp(signature, key["e"], key["n"])
//! ```
//!
//! Accepts PEM (`-----BEGIN CERTIFICATE-----`), bare base64, hex, or a raw
//! DER byte array.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use base64::Engine;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_parser::prelude::*;
use x509_parser::public_key::PublicKey;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, NativeFunction, Value};

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("invalid hex: odd length".to_string());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| "invalid hex digit".to_string())
        })
        .collect()
}

/// Decode a certificate argument to raw DER bytes. Handles PEM, bare base64
/// (possibly with embedded whitespace, as in SAML metadata), hex, or a raw
/// DER byte array.
fn to_der(value: &Value) -> Result<Vec<u8>, String> {
    match value {
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                Value::Int(n) => Err(format!("byte value {} out of range 0-255", n)),
                other => Err(format!(
                    "expected byte (Int 0-255), got {}",
                    other.type_name()
                )),
            })
            .collect(),
        Value::String(s) => {
            if s.contains("-----BEGIN") {
                let body: String = s
                    .lines()
                    .filter(|l| !l.contains("-----"))
                    .flat_map(|l| l.chars())
                    .filter(|c| !c.is_whitespace())
                    .collect();
                base64::engine::general_purpose::STANDARD
                    .decode(body.as_bytes())
                    .map_err(|e| format!("invalid PEM base64: {}", e))
            } else {
                let stripped: String = s.chars().filter(|c| !c.is_whitespace()).collect();
                base64::engine::general_purpose::STANDARD
                    .decode(stripped.as_bytes())
                    .or_else(|_| hex_to_bytes(&stripped))
                    .map_err(|_| "certificate is neither valid base64 nor hex".to_string())
            }
        }
        other => Err(format!(
            "expected certificate string or byte array, got {}",
            other.type_name()
        )),
    }
}

/// Strip leading zero octets (a DER INTEGER carries a 0x00 sign byte for
/// values whose high bit is set; RSA `(n, e)` are unsigned).
fn strip_leading_zeros(bytes: &[u8]) -> &[u8] {
    let first_nonzero = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    let trimmed = &bytes[first_nonzero..];
    if trimmed.is_empty() {
        &bytes[bytes.len().saturating_sub(1)..] // keep a single 0 for value 0
    } else {
        trimmed
    }
}

pub fn register_x509_builtins(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // X509.public_key(cert) -> { algorithm, n, e, bits }
    methods.insert(
        "public_key".to_string(),
        Rc::new(NativeFunction::new("X509.public_key", Some(1), |args| {
            let der = to_der(&args[0]).map_err(|e| format!("X509.public_key(): {}", e))?;
            let (_, cert) = X509Certificate::from_der(&der)
                .map_err(|e| format!("X509.public_key(): invalid certificate: {}", e))?;
            match cert.public_key().parsed() {
                Ok(PublicKey::RSA(rsa)) => {
                    let n = strip_leading_zeros(rsa.modulus);
                    let e = strip_leading_zeros(rsa.exponent);
                    Ok(hash_from_pairs([
                        ("algorithm".to_string(), Value::String("RSA".into())),
                        ("n".to_string(), Value::String(bytes_to_hex(n).into())),
                        ("e".to_string(), Value::String(bytes_to_hex(e).into())),
                        ("bits".to_string(), Value::Int((n.len() * 8) as i64)),
                    ]))
                }
                Ok(_) => Err(
                    "X509.public_key(): certificate does not contain an RSA public key".to_string(),
                ),
                Err(e) => Err(format!(
                    "X509.public_key(): could not parse public key: {}",
                    e
                )),
            }
        })),
    );

    // X509.fingerprint(cert, algorithm?) -> String (hex; sha256 default)
    methods.insert(
        "fingerprint".to_string(),
        Rc::new(NativeFunction::new("X509.fingerprint", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "X509.fingerprint() expects 1-2 arguments (cert, algorithm?), got {}",
                    args.len()
                ));
            }
            let der = to_der(&args[0]).map_err(|e| format!("X509.fingerprint(): {}", e))?;
            let algo = if args.len() == 2 {
                match &args[1] {
                    Value::String(s) => s.to_lowercase().to_string(),
                    other => {
                        return Err(format!(
                            "X509.fingerprint() expects string algorithm, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                "sha256".to_string()
            };
            let hex = match algo.as_str() {
                "sha256" => bytes_to_hex(&Sha256::digest(&der)),
                "sha1" => bytes_to_hex(&Sha1::digest(&der)),
                other => {
                    return Err(format!(
                        "X509.fingerprint(): unsupported algorithm '{}' (use sha256 or sha1)",
                        other
                    ))
                }
            };
            Ok(Value::String(hex.into()))
        })),
    );

    let class = Class {
        name: "X509".to_string(),
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
    env.define("X509".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_leading_zeros_removes_der_sign_byte() {
        assert_eq!(strip_leading_zeros(&[0x00, 0xff, 0x10]), &[0xff, 0x10]);
        assert_eq!(strip_leading_zeros(&[0x01, 0x02]), &[0x01, 0x02]);
        assert_eq!(strip_leading_zeros(&[0x00]), &[0x00]);
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex_to_bytes("00ff10").unwrap(), vec![0x00, 0xff, 0x10]);
        assert_eq!(bytes_to_hex(&[0x00, 0xff, 0x10]), "00ff10");
        assert!(hex_to_bytes("abc").is_err());
    }
}
