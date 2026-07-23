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

    // X509.spki_pin(cert) -> String ("sha256/<base64>")
    //
    // The public-key pin used by certificate pinning: base64 of SHA-256 over
    // the certificate's SubjectPublicKeyInfo. It pins the *key*, not the cert,
    // so it survives a certificate renewal that reuses the key — which is the
    // only way pinning does not brick the client every ~90 days. The `sha256/`
    // prefix is the form Android's Network Security Config and every HPKP-style
    // pin-set expects, so the output drops straight into a pin-set.
    methods.insert(
        "spki_pin".to_string(),
        Rc::new(NativeFunction::new("X509.spki_pin", Some(1), |args| {
            let der = to_der(&args[0]).map_err(|e| format!("X509.spki_pin(): {}", e))?;
            let (_, cert) = X509Certificate::from_der(&der)
                .map_err(|e| format!("X509.spki_pin(): invalid certificate: {}", e))?;
            // The raw DER of the SubjectPublicKeyInfo — pinning this and not the
            // whole certificate is the entire point.
            let spki = cert.public_key().raw;
            let digest = Sha256::digest(spki);
            let pin = base64::engine::general_purpose::STANDARD.encode(digest);
            Ok(Value::String(format!("sha256/{}", pin).into()))
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

    /// The property that makes SPKI pinning usable: two certificates issued from
    /// the SAME key — a renewal — produce the SAME pin, so a 90-day cert rotation
    /// does not brick a pinned client. A DIFFERENT key produces a different pin.
    #[test]
    fn spki_pin_is_stable_across_renewal_and_changes_with_the_key() {
        // Two certs, one key (a renewal). Different validity, different serial.
        let cert1 = "-----BEGIN CERTIFICATE-----\nMIIBgTCCASegAwIBAgIUfRx6UUfAxAc/7KnSxMRyrRCwqVswCgYIKoZIzj0EAwIw\nFjEUMBIGA1UEAwwLZXhhbXBsZS5jb20wHhcNMjYwNzIzMDc0NDA3WhcNMjYwODIy\nMDc0NDA3WjAWMRQwEgYDVQQDDAtleGFtcGxlLmNvbTBZMBMGByqGSM49AgEGCCqG\nSM49AwEHA0IABGzbcZZRYvhhLwk6iNlmYpmJYFDraCR7j9rNeYv3FLD1shSy/oIz\nZsFvEu1FgV00QGsa/WcgSl7sugEJUvb2N7ejUzBRMB0GA1UdDgQWBBSJYpREDiiF\nD9aZ/5bkaeHdYryb9DAfBgNVHSMEGDAWgBSJYpREDiiFD9aZ/5bkaeHdYryb9DAP\nBgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0gAMEUCIQDzgcU3umi5dhgn004P\n2Ql5dY2VpwLZ52brEWxuQ56WEwIgXqXOJrvvo4nqcCkMepFOsNP86GuJv+18iFty\nuOWHyBw=\n-----END CERTIFICATE-----";
        let cert2 = "-----BEGIN CERTIFICATE-----
MIIBgTCCASegAwIBAgIUBhrf1zzlaVoaAqrsyohR0+AzV78wCgYIKoZIzj0EAwIw
FjEUMBIGA1UEAwwLZXhhbXBsZS5jb20wHhcNMjYwNzIzMDc0NDA3WhcNMjcwNzIz
MDc0NDA3WjAWMRQwEgYDVQQDDAtleGFtcGxlLmNvbTBZMBMGByqGSM49AgEGCCqG
SM49AwEHA0IABGzbcZZRYvhhLwk6iNlmYpmJYFDraCR7j9rNeYv3FLD1shSy/oIz
ZsFvEu1FgV00QGsa/WcgSl7sugEJUvb2N7ejUzBRMB0GA1UdDgQWBBSJYpREDiiF
D9aZ/5bkaeHdYryb9DAfBgNVHSMEGDAWgBSJYpREDiiFD9aZ/5bkaeHdYryb9DAP
BgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0gAMEUCIQC6h6Nt/V46ycmNNSG5
T98qJTfTvm1nKb4aEPvWb/THVwIgQEPKW7I6xy7kHDKaamTVAG21RODfBgk/agLQ
3ZFPIpU=
-----END CERTIFICATE-----";
        let cert3 = "-----BEGIN CERTIFICATE-----
MIIBgjCCASegAwIBAgIUDy1GrRA9x5FdBcwbb/30lFjMAzkwCgYIKoZIzj0EAwIw
FjEUMBIGA1UEAwwLZXhhbXBsZS5jb20wHhcNMjYwNzIzMDc0NDA3WhcNMjYwODIy
MDc0NDA3WjAWMRQwEgYDVQQDDAtleGFtcGxlLmNvbTBZMBMGByqGSM49AgEGCCqG
SM49AwEHA0IABFXP+T6OsIxdD4spFdFwJOYUfhK9dmVVxwTN9hP8m69cUZdNnOKw
aUWyIoOcP58Uc3wh0NcYILeIm4Xl6MK18/6jUzBRMB0GA1UdDgQWBBQD1AVrRXrJ
aXdcHxIiAoVWt33sNjAfBgNVHSMEGDAWgBQD1AVrRXrJaXdcHxIiAoVWt33sNjAP
BgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0kAMEYCIQDmAhNSNXFAg+SuPtDf
qbpLWgutn/Xw5zPsQZUxy6X82AIhALn4uFQe9r08hcPYD+BOXw0eeLvK+JqPcqGS
IZV30Dp1
-----END CERTIFICATE-----"; // a different key entirely

        let pin = |pem: &str| -> String {
            let der = to_der(&Value::String(pem.into())).unwrap();
            let (_, cert) = X509Certificate::from_der(&der).unwrap();
            let digest = Sha256::digest(cert.public_key().raw);
            format!(
                "sha256/{}",
                base64::engine::general_purpose::STANDARD.encode(digest)
            )
        };

        // Matches `openssl x509 -pubkey | openssl pkey -pubin -outform der
        //          | openssl dgst -sha256 -binary | base64`.
        assert_eq!(
            pin(cert1),
            "sha256/UKm/R6MKhCiukXKhnWjBQSRBSWRwGQBLCCa/8w27Dxs="
        );
        // A renewal reusing the key keeps the pin — the whole point.
        assert_eq!(pin(cert1), pin(cert2));
        // A new key changes it.
        assert_ne!(pin(cert1), pin(cert3));
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex_to_bytes("00ff10").unwrap(), vec![0x00, 0xff, 0x10]);
        assert_eq!(bytes_to_hex(&[0x00, 0xff, 0x10]), "00ff10");
        assert!(hex_to_bytes("abc").is_err());
    }
}
