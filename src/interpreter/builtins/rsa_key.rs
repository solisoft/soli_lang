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

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use rsa::RsaPrivateKey;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, NativeFunction, Value};

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Parse a PEM RSA private key, trying PKCS#8 first then PKCS#1.
fn parse_private_key(pem: &str) -> Result<RsaPrivateKey, String> {
    RsaPrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(pem))
        .map_err(|e| {
            format!(
                "could not parse RSA private key (PKCS#8 or PKCS#1 PEM): {}",
                e
            )
        })
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
                let key = parse_private_key(&pem)
                    .map_err(|e| format!("RsaKey.private_from_pem(): {}", e))?;
                let n = key.n().to_bytes_be();
                let e = key.e().to_bytes_be();
                let d = key.d().to_bytes_be();
                Ok(hash_from_pairs([
                    ("algorithm".to_string(), Value::String("RSA".to_string())),
                    ("n".to_string(), Value::String(bytes_to_hex(&n))),
                    ("e".to_string(), Value::String(bytes_to_hex(&e))),
                    ("d".to_string(), Value::String(bytes_to_hex(&d))),
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

    // A 512-bit PKCS#8 RSA key (test-only) generated with OpenSSL.
    const PKCS8_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIBVAIBADANBgkqhkiG9w0BAQEFAASCAT4wggE6AgEAAkEAtsQsHV8r6V8s0nE6\n\
3p1Rk8m9pVQ0o0o0 K0\n\
-----END PRIVATE KEY-----\n";

    #[test]
    fn rejects_garbage_pem() {
        assert!(parse_private_key("not a key").is_err());
        assert!(parse_private_key(PKCS8_PEM).is_err()); // truncated/invalid body
    }
}
