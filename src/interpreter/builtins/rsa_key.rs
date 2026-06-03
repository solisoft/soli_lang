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
use pkcs1::RsaPrivateKey as Pkcs1PrivateKey;
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

    #[test]
    fn rejects_garbage_pem() {
        assert!(parse_components("not a key").is_err());
        assert!(
            parse_components("-----BEGIN PRIVATE KEY-----\nZm9v\n-----END PRIVATE KEY-----")
                .is_err()
        );
    }
}
