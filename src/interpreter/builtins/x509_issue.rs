//! `X509.issue(options)` — a certificate issued by a CA the app holds.
//!
//! What an e-signature service needs for an *advanced* signature (eIDAS art.
//! 26): a key and a certificate per signer, made at the moment they sign and
//! used once, so the signature is uniquely linked to them. The CA's key is
//! the app's secret; the signer's key is generated here and returned with
//! the certificate, to be handed to `pdf_sign` and dropped.
//!
//! ```text
//! identity = X509.issue({
//!   "issuer_cert": ca_pem, "issuer_key": ca_key_pem,
//!   "subject": {"common_name": "Ada Buyer", "email": "ada@example.com", "country": "FR"},
//!   "days": 1
//! })
//! identity["cert"]   # PEM, signed by the CA (ECDSA P-256 / SHA-256)
//! identity["key"]    # PEM, PKCS#8 P-256 — the subject's private key
//! ```
//!
//! Issuer and subject keys are EC P-256, what `pdf_sign` signs with. Pure
//! RustCrypto, so it runs on the edge (wasm32) build too.

use std::time::Duration;

use base64::Engine;
use der::asn1::{Any, BitString, GeneralizedTime, Ia5String, OctetString, SetOfVec, UtcTime};
use der::oid::AssociatedOid;
use der::{Decode, Encode, Tag};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use x509_cert::attr::AttributeTypeAndValue;
use x509_cert::certificate::{TbsCertificate, Version};
use x509_cert::ext::pkix::certpolicy::{CertificatePolicies, PolicyInformation};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{
    AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage, KeyUsages,
    SubjectAltName, SubjectKeyIdentifier,
};
use x509_cert::ext::Extension;
use x509_cert::name::{Name, RdnSequence, RelativeDistinguishedName};
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::{AlgorithmIdentifierOwned, ObjectIdentifier, SubjectPublicKeyInfoOwned};
use x509_cert::time::{Time, Validity};
use x509_cert::Certificate;

use super::pades::{cert_to_der, parse_private_key, SigningKey};
use crate::interpreter::value::{hash_from_pairs, HashKey, HashPairs, Value};

const OID_ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const OID_COUNTRY: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.6");
const OID_ORGANIZATION: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.10");
const OID_ORG_UNIT: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.11");
const OID_COMMON_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");
const OID_SERIAL_NUMBER: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.5");
const OID_GIVEN_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.42");
const OID_SURNAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.4");
const OID_EMAIL: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.1");
/// id-kp-documentSigning (RFC 9336) and id-kp-emailProtection.
const OID_KP_DOCUMENT_SIGNING: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.36");
const OID_KP_EMAIL_PROTECTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.4");

/// Longest a certificate may last: 30 years, as a root CA might.
const MAX_DAYS: i64 = 30 * 366;

/// What `X509.issue` was asked for, read from its options hash.
struct IssueRequest {
    issuer_cert: Vec<u8>,
    issuer_key: p256::ecdsa::SigningKey,
    subject: Vec<(ObjectIdentifier, String)>,
    email: Option<String>,
    days: i64,
    ca: bool,
    policies: Vec<ObjectIdentifier>,
}

fn get<'a>(hash: &'a HashPairs, key: &str) -> Option<&'a Value> {
    hash.get(&HashKey::String(key.into()))
}

fn get_str(hash: &HashPairs, key: &str) -> Option<String> {
    match get(hash, key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

fn read_request(options: &Value) -> Result<IssueRequest, String> {
    let Value::Hash(options) = options else {
        return Err("expects an options hash".into());
    };
    let options = options.borrow();

    let cert_pem = get_str(&options, "issuer_cert").ok_or("`issuer_cert` (PEM) is required")?;
    let key_pem = get_str(&options, "issuer_key").ok_or("`issuer_key` (PEM) is required")?;
    let issuer_cert = cert_to_der(&cert_pem).map_err(|e| format!("issuer_cert: {e}"))?;
    if key_pem.contains("BEGIN EC PRIVATE KEY") {
        return Err(
            "issuer_key is a SEC1 key: convert it to PKCS#8 with `openssl pkcs8 -topk8 -nocrypt -in ca.key`"
                .into(),
        );
    }
    let issuer_key = match parse_private_key(&key_pem).map_err(|e| format!("issuer_key: {e}"))? {
        SigningKey::Ec(key) => *key,
        SigningKey::Rsa { .. } => {
            return Err("issuer_key: only EC P-256 keys can issue certificates".into())
        }
    };

    let Some(Value::Hash(subject_hash)) = get(&options, "subject") else {
        return Err("`subject` (a hash with at least common_name) is required".into());
    };
    let subject_hash = subject_hash.borrow();
    // RFC 5280 order, broadest first.
    let fields = [
        ("country", OID_COUNTRY),
        ("organization", OID_ORGANIZATION),
        ("organizational_unit", OID_ORG_UNIT),
        ("surname", OID_SURNAME),
        ("given_name", OID_GIVEN_NAME),
        ("serial_number", OID_SERIAL_NUMBER),
        ("common_name", OID_COMMON_NAME),
        ("email", OID_EMAIL),
    ];
    let mut subject = Vec::new();
    for (name, oid) in fields {
        if let Some(value) = get_str(&subject_hash, name) {
            if name == "country" && (value.len() != 2 || !value.is_ascii()) {
                return Err(format!(
                    "subject.country must be two letters, got {value:?}"
                ));
            }
            if value.chars().count() > 128 {
                return Err(format!("subject.{name} is longer than 128 characters"));
            }
            subject.push((oid, value));
        }
    }
    if !subject.iter().any(|(oid, _)| *oid == OID_COMMON_NAME) {
        return Err("subject.common_name is required".into());
    }
    let email = get_str(&subject_hash, "email");
    if let Some(email) = &email {
        if !email.is_ascii() || !email.contains('@') {
            return Err(format!("subject.email is not an address: {email:?}"));
        }
    }

    let days = match get(&options, "days") {
        None | Some(Value::Null) => 1,
        Some(Value::Int(n)) if (1..=MAX_DAYS).contains(n) => *n,
        Some(other) => {
            return Err(format!(
                "`days` must be an integer from 1 to {MAX_DAYS}, got {other}"
            ))
        }
    };
    let ca = matches!(get(&options, "ca"), Some(Value::Bool(true)));
    let mut policies = Vec::new();
    if let Some(Value::Array(items)) = get(&options, "policies") {
        for item in items.borrow().iter() {
            let Value::String(text) = item else {
                return Err("`policies` holds OID strings".into());
            };
            policies.push(
                ObjectIdentifier::new(text.trim()).map_err(|e| format!("policy {text:?}: {e}"))?,
            );
        }
    }

    Ok(IssueRequest {
        issuer_cert,
        issuer_key,
        subject,
        email,
        days,
        ca,
        policies,
    })
}

fn rdn(oid: ObjectIdentifier, value: &str) -> Result<RelativeDistinguishedName, String> {
    // Country is a PrintableString and email an IA5String by the standards;
    // everything else UTF8String, so accented names survive.
    let any = if oid == OID_COUNTRY {
        Any::new(Tag::PrintableString, value.as_bytes())
    } else if oid == OID_EMAIL {
        Any::new(Tag::Ia5String, value.as_bytes())
    } else {
        Any::new(Tag::Utf8String, value.as_bytes())
    }
    .map_err(|e| format!("subject value: {e}"))?;
    let set = SetOfVec::try_from(vec![AttributeTypeAndValue { oid, value: any }])
        .map_err(|e| format!("subject RDN: {e}"))?;
    Ok(RelativeDistinguishedName(set))
}

/// A time as RFC 5280 wants it: UTCTime through 2049, GeneralizedTime after.
fn x509_time(unix: i64) -> Result<Time, String> {
    let duration = Duration::from_secs(unix.max(0) as u64);
    if unix < 2_524_608_000 {
        UtcTime::from_unix_duration(duration).map(Time::UtcTime)
    } else {
        GeneralizedTime::from_unix_duration(duration).map(Time::GeneralTime)
    }
    .map_err(|e| format!("time: {e}"))
}

fn extension<T: Encode + AssociatedOid>(value: &T, critical: bool) -> Result<Extension, String> {
    let bytes = value
        .to_der()
        .map_err(|e| format!("extension {}: {e}", T::OID))?;
    Ok(Extension {
        extn_id: T::OID,
        critical,
        extn_value: OctetString::new(bytes).map_err(|e| format!("extension: {e}"))?,
    })
}

/// RFC 7093 method 1: the leftmost 160 bits of the SHA-256 of the key bits.
fn key_identifier(spki: &SubjectPublicKeyInfoOwned) -> Vec<u8> {
    Sha256::digest(spki.subject_public_key.raw_bytes())[..20].to_vec()
}

fn pem(label: &str, der: &[u8]) -> String {
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn new_signing_key() -> Result<p256::ecdsa::SigningKey, String> {
    // A random scalar is valid but for a 2^-128 chance; draw again then.
    for _ in 0..8 {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        if let Ok(key) = p256::ecdsa::SigningKey::from_slice(&bytes) {
            return Ok(key);
        }
    }
    Err("could not generate a key".into())
}

fn spki_of(key: &p256::ecdsa::SigningKey) -> Result<SubjectPublicKeyInfoOwned, String> {
    use p256::pkcs8::EncodePublicKey;
    let der = key
        .verifying_key()
        .to_public_key_der()
        .map_err(|e| format!("public key: {e}"))?;
    SubjectPublicKeyInfoOwned::from_der(der.as_bytes()).map_err(|e| format!("public key: {e}"))
}

/// Issue the certificate `request` describes at `now` (Unix seconds).
/// Returns (certificate DER, subject key PKCS#8 DER, serial hex, not_after).
fn issue(request: IssueRequest, now: i64) -> Result<(Vec<u8>, Vec<u8>, String, i64), String> {
    let issuer =
        Certificate::from_der(&request.issuer_cert).map_err(|e| format!("issuer_cert: {e}"))?;
    let issuer_spki = spki_of(&request.issuer_key)?;
    if issuer_spki != issuer.tbs_certificate.subject_public_key_info {
        return Err("issuer_key does not belong to issuer_cert".into());
    }
    let issuer_is_ca = issuer
        .tbs_certificate
        .get::<BasicConstraints>()
        .ok()
        .flatten()
        .is_some_and(|(_, constraints)| constraints.ca);
    if !issuer_is_ca {
        return Err("issuer_cert is not a CA certificate (basicConstraints CA:TRUE)".into());
    }
    let issuer_key_id = issuer
        .tbs_certificate
        .get::<SubjectKeyIdentifier>()
        .ok()
        .flatten()
        .map(|(_, ski)| ski.0.as_bytes().to_vec())
        .unwrap_or_else(|| key_identifier(&issuer_spki));

    let subject_key = new_signing_key()?;
    let subject_spki = spki_of(&subject_key)?;

    let mut serial = [0u8; 16];
    OsRng.fill_bytes(&mut serial);
    serial[0] = (serial[0] & 0x7f) | 0x01; // positive, and 16 bytes long
    let serial_hex: String = serial.iter().map(|b| format!("{b:02x}")).collect();

    // A minute of slack for a verifier whose clock runs a little behind.
    let not_before = now - 60;
    let not_after = now + request.days * 86_400;
    let mut not_after_time = not_after;
    // A certificate never outlives the CA that vouches for it.
    let issuer_end = issuer
        .tbs_certificate
        .validity
        .not_after
        .to_unix_duration()
        .as_secs() as i64;
    if not_after_time > issuer_end {
        not_after_time = issuer_end;
    }

    let subject = Name::from(RdnSequence(
        request
            .subject
            .iter()
            .map(|(oid, value)| rdn(*oid, value))
            .collect::<Result<Vec<_>, _>>()?,
    ));

    let mut extensions = vec![
        extension(
            &BasicConstraints {
                ca: request.ca,
                path_len_constraint: request.ca.then_some(0),
            },
            true,
        )?,
        extension(
            &KeyUsage(if request.ca {
                KeyUsages::KeyCertSign | KeyUsages::CRLSign
            } else {
                KeyUsages::DigitalSignature | KeyUsages::NonRepudiation
            }),
            true,
        )?,
        extension(
            &SubjectKeyIdentifier(
                OctetString::new(key_identifier(&subject_spki)).map_err(|e| e.to_string())?,
            ),
            false,
        )?,
        extension(
            &AuthorityKeyIdentifier {
                key_identifier: Some(OctetString::new(issuer_key_id).map_err(|e| e.to_string())?),
                authority_cert_issuer: None,
                authority_cert_serial_number: None,
            },
            false,
        )?,
    ];
    if !request.ca {
        extensions.push(extension(
            &ExtendedKeyUsage(vec![OID_KP_DOCUMENT_SIGNING, OID_KP_EMAIL_PROTECTION]),
            false,
        )?);
    }
    if let Some(email) = &request.email {
        let address = Ia5String::new(email).map_err(|e| format!("email: {e}"))?;
        extensions.push(extension(
            &SubjectAltName(vec![GeneralName::Rfc822Name(address)]),
            false,
        )?);
    }
    if !request.policies.is_empty() {
        let policies = request
            .policies
            .iter()
            .map(|oid| PolicyInformation {
                policy_identifier: *oid,
                policy_qualifiers: None,
            })
            .collect();
        extensions.push(extension(&CertificatePolicies(policies), false)?);
    }

    let signature_algorithm = AlgorithmIdentifierOwned {
        oid: OID_ECDSA_WITH_SHA256,
        parameters: None,
    };
    let tbs = TbsCertificate {
        version: Version::V3,
        serial_number: SerialNumber::new(&serial).map_err(|e| format!("serial: {e}"))?,
        signature: signature_algorithm.clone(),
        issuer: issuer.tbs_certificate.subject.clone(),
        validity: Validity {
            not_before: x509_time(not_before)?,
            not_after: x509_time(not_after_time)?,
        },
        subject,
        subject_public_key_info: subject_spki,
        issuer_unique_id: None,
        subject_unique_id: None,
        extensions: Some(extensions),
    };
    let tbs_der = tbs.to_der().map_err(|e| format!("certificate: {e}"))?;
    let signature: p256::ecdsa::Signature = {
        use p256::ecdsa::signature::Signer;
        request.issuer_key.sign(&tbs_der)
    };
    let certificate = Certificate {
        tbs_certificate: tbs,
        signature_algorithm,
        signature: BitString::from_bytes(signature.to_der().as_bytes())
            .map_err(|e| format!("signature: {e}"))?,
    };
    let cert_der = certificate
        .to_der()
        .map_err(|e| format!("certificate: {e}"))?;

    let key_der = {
        use p256::pkcs8::EncodePrivateKey;
        subject_key
            .to_pkcs8_der()
            .map_err(|e| format!("private key: {e}"))?
    };
    Ok((
        cert_der,
        key_der.as_bytes().to_vec(),
        serial_hex,
        not_after_time,
    ))
}

/// `X509.issue(options)` → {"cert", "key", "serial", "not_after_unix"}.
pub fn issue_builtin(options: &Value) -> Result<Value, String> {
    let request = read_request(options).map_err(|e| format!("X509.issue(): {e}"))?;
    let now = crate::platform::unix_now_secs() as i64;
    let (cert, key, serial, not_after) =
        issue(request, now).map_err(|e| format!("X509.issue(): {e}"))?;
    Ok(hash_from_pairs([
        (
            "cert".to_string(),
            Value::String(pem("CERTIFICATE", &cert).into()),
        ),
        (
            "key".to_string(),
            Value::String(pem("PRIVATE KEY", &key).into()),
        ),
        ("serial".to_string(), Value::String(serial.into())),
        ("not_after_unix".to_string(), Value::Int(not_after)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn hash(pairs: Vec<(&str, Value)>) -> Value {
        Value::Hash(Rc::new(RefCell::new(
            pairs
                .into_iter()
                .map(|(k, v)| (HashKey::String(k.into()), v))
                .collect(),
        )))
    }

    fn string(s: &str) -> Value {
        Value::String(s.into())
    }

    fn get_s(value: &Value, key: &str) -> String {
        let Value::Hash(h) = value else {
            panic!("not a hash")
        };
        match h.borrow().get(&HashKey::String(key.into())) {
            Some(Value::String(s)) => s.to_string(),
            other => panic!("{key}: {other:?}"),
        }
    }

    /// A root CA issued by `X509.issue` itself, from a self-signed seed.
    fn root() -> (String, String) {
        // Self-sign: issue a CA certificate whose issuer is a throwaway
        // self-signed certificate made with the same key.
        let key = new_signing_key().unwrap();
        let spki = spki_of(&key).unwrap();
        let name = Name::from(RdnSequence(
            vec![rdn(OID_COMMON_NAME, "Spec Root").unwrap()],
        ));
        let alg = AlgorithmIdentifierOwned {
            oid: OID_ECDSA_WITH_SHA256,
            parameters: None,
        };
        let tbs = TbsCertificate {
            version: Version::V3,
            serial_number: SerialNumber::new(&[1]).unwrap(),
            signature: alg.clone(),
            issuer: name.clone(),
            validity: Validity {
                not_before: x509_time(0).unwrap(),
                not_after: x509_time(4_000_000_000).unwrap(),
            },
            subject: name,
            subject_public_key_info: spki,
            issuer_unique_id: None,
            subject_unique_id: None,
            extensions: Some(vec![extension(
                &BasicConstraints {
                    ca: true,
                    path_len_constraint: None,
                },
                true,
            )
            .unwrap()]),
        };
        let sig: p256::ecdsa::Signature = {
            use p256::ecdsa::signature::Signer;
            key.sign(&tbs.to_der().unwrap())
        };
        let cert = Certificate {
            tbs_certificate: tbs,
            signature_algorithm: alg,
            signature: BitString::from_bytes(sig.to_der().as_bytes()).unwrap(),
        };
        let key_der = {
            use p256::pkcs8::EncodePrivateKey;
            key.to_pkcs8_der().unwrap()
        };
        (
            pem("CERTIFICATE", &cert.to_der().unwrap()),
            pem("PRIVATE KEY", key_der.as_bytes()),
        )
    }

    fn verify(cert_pem: &str, issuer_pem: &str) -> bool {
        use p256::ecdsa::signature::Verifier;
        use p256::pkcs8::DecodePublicKey;
        let cert = Certificate::from_der(&cert_to_der(cert_pem).unwrap()).unwrap();
        let issuer = Certificate::from_der(&cert_to_der(issuer_pem).unwrap()).unwrap();
        let key = p256::ecdsa::VerifyingKey::from_public_key_der(
            &issuer
                .tbs_certificate
                .subject_public_key_info
                .to_der()
                .unwrap(),
        )
        .unwrap();
        let sig = p256::ecdsa::Signature::from_der(cert.signature.raw_bytes()).unwrap();
        key.verify(&cert.tbs_certificate.to_der().unwrap(), &sig)
            .is_ok()
    }

    #[test]
    fn issues_a_signer_certificate_the_ca_signed() {
        let (ca_cert, ca_key) = root();
        let out = issue_builtin(&hash(vec![
            ("issuer_cert", string(&ca_cert)),
            ("issuer_key", string(&ca_key)),
            (
                "subject",
                hash(vec![
                    ("common_name", string("Adèle Buyer")),
                    ("email", string("ada@example.com")),
                    ("country", string("FR")),
                ]),
            ),
            ("days", Value::Int(1)),
            (
                "policies",
                Value::Array(Rc::new(RefCell::new(vec![string("1.3.6.1.4.1.99999.1")]))),
            ),
        ]))
        .unwrap();
        let cert = get_s(&out, "cert");
        assert!(verify(&cert, &ca_cert));
        // The key it returns is the certificate's own, and pdf_sign reads it.
        let key = parse_private_key(&get_s(&out, "key")).unwrap();
        let SigningKey::Ec(key) = key else {
            panic!("not EC")
        };
        let parsed = Certificate::from_der(&cert_to_der(&cert).unwrap()).unwrap();
        assert_eq!(
            spki_of(&key).unwrap(),
            parsed.tbs_certificate.subject_public_key_info
        );
        let (_, constraints) = parsed
            .tbs_certificate
            .get::<BasicConstraints>()
            .unwrap()
            .unwrap();
        assert!(!constraints.ca);
        let (_, usage) = parsed.tbs_certificate.get::<KeyUsage>().unwrap().unwrap();
        assert!(usage.digital_signature() && usage.non_repudiation());
        let subject = parsed.tbs_certificate.subject.to_string();
        assert!(subject.contains("CN=Adèle Buyer"), "{subject}");
        assert_eq!(get_s(&out, "serial").len(), 32);
    }

    #[test]
    fn issues_an_intermediate_ca_that_can_issue_in_turn() {
        let (root_cert, root_key) = root();
        let intermediate = issue_builtin(&hash(vec![
            ("issuer_cert", string(&root_cert)),
            ("issuer_key", string(&root_key)),
            (
                "subject",
                hash(vec![("common_name", string("Spec Signers CA"))]),
            ),
            ("days", Value::Int(3650)),
            ("ca", Value::Bool(true)),
        ]))
        .unwrap();
        let leaf = issue_builtin(&hash(vec![
            ("issuer_cert", string(&get_s(&intermediate, "cert"))),
            ("issuer_key", string(&get_s(&intermediate, "key"))),
            ("subject", hash(vec![("common_name", string("Bob"))])),
        ]))
        .unwrap();
        assert!(verify(&get_s(&leaf, "cert"), &get_s(&intermediate, "cert")));
    }

    #[test]
    fn refuses_a_key_from_another_ca_a_leaf_issuer_and_a_missing_name() {
        let (ca_cert, _) = root();
        let (_, other_key) = root();
        let mismatched = issue_builtin(&hash(vec![
            ("issuer_cert", string(&ca_cert)),
            ("issuer_key", string(&other_key)),
            ("subject", hash(vec![("common_name", string("X"))])),
        ]));
        assert!(mismatched.unwrap_err().contains("does not belong"));

        let (ca_cert, ca_key) = root();
        let leaf = issue_builtin(&hash(vec![
            ("issuer_cert", string(&ca_cert)),
            ("issuer_key", string(&ca_key)),
            ("subject", hash(vec![("common_name", string("Leaf"))])),
        ]))
        .unwrap();
        let from_leaf = issue_builtin(&hash(vec![
            ("issuer_cert", string(&get_s(&leaf, "cert"))),
            ("issuer_key", string(&get_s(&leaf, "key"))),
            ("subject", hash(vec![("common_name", string("Y"))])),
        ]));
        assert!(from_leaf.unwrap_err().contains("not a CA"));

        let nameless = issue_builtin(&hash(vec![
            ("issuer_cert", string(&ca_cert)),
            ("issuer_key", string(&ca_key)),
            ("subject", hash(vec![("email", string("a@b.c"))])),
        ]));
        assert!(nameless.unwrap_err().contains("common_name"));
    }
}
