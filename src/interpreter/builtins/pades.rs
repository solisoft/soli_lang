//! PAdES-B-B CMS assembly — the *crypto* half of PDF signing.
//!
//! The `soli-pdf` crate reserves a signature placeholder and hands us the bytes
//! to digest ([`soli_pdf::prepare_signature`]); this module turns that digest
//! into a detached **CMS SignedData** (RFC 5652 / ETSI EN 319 122 baseline),
//! which the pdf crate then splices back in ([`soli_pdf::embed_cms`]). Keeping
//! all asymmetric crypto here means the pdf crate never depends on ASN.1/RSA.
//!
//! The signed attributes are the PAdES-B-B set: content-type (id-data),
//! message-digest, signing-time, and the ESS signing-certificate-v2 that binds
//! the signature to *this* certificate. The signature covers the DER of that
//! attribute set (re-tagged as a `SET OF`, per RFC 5652 §5.4).
//!
//! RSA signing reuses the in-house `modexp` + PKCS#1 v1.5 primitives from
//! [`crypto`] — the project deliberately avoids the `rsa` crate
//! (RUSTSEC-2023-0071). ECDSA (P-256) uses `p256` directly, as `vapid` does.

use der::asn1::{ObjectIdentifier, OctetString, SetOfVec, UtcTime};
use der::{Any, Decode, Encode};
use time::OffsetDateTime;
use x509_cert::attr::Attribute;
use x509_cert::spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};

use super::crypto::{do_modexp, do_pkcs1_pad};

// --- OIDs ------------------------------------------------------------------

const OID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const OID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const OID_SIGNING_CERT_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
pub(crate) const OID_SHA256: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const OID_EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");

/// The fixed DER prefix of a SHA-256 `DigestInfo` (`SEQUENCE { sha256+NULL,
/// OCTET STRING }`) — the 32-byte digest is appended. Used for RSA PKCS#1 v1.5.
const SHA256_DIGESTINFO_PREFIX: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

// --- key material ----------------------------------------------------------

/// A parsed signing key. RSA is stored as raw big-endian `(n, d)` octets so it
/// flows straight into the in-house `modexp`; EC is a ready `p256` signer.
pub enum SigningKey {
    /// `n` is the modulus with any DER sign byte stripped (so `n.len()` is the
    /// true key size in octets); `d` is the private exponent.
    Rsa {
        n: Vec<u8>,
        d: Vec<u8>,
    },
    Ec(Box<p256::ecdsa::SigningKey>),
}

/// Everything needed to produce one signer's CMS: the signer certificate, any
/// intermediate certificates to embed, and the private key.
pub struct SignerMaterial {
    pub cert_der: Vec<u8>,
    pub chain_der: Vec<Vec<u8>>,
    pub key: SigningKey,
}

/// Decode a PEM (`-----BEGIN …-----`) or bare-base64 / DER byte string into its
/// label (empty for non-PEM) and DER bytes.
fn pem_to_der(input: &str) -> Result<(String, Vec<u8>), String> {
    use base64::Engine;
    if input.contains("-----BEGIN") {
        let begin = "-----BEGIN ";
        let start = input.find(begin).ok_or("missing PEM header")?;
        let after = &input[start + begin.len()..];
        let label = after[..after.find("-----").ok_or("malformed PEM header")?]
            .trim()
            .to_string();
        let body: String = input
            .lines()
            .skip_while(|l| !l.contains("-----BEGIN"))
            .skip(1)
            .take_while(|l| !l.contains("-----END"))
            .flat_map(|l| l.chars())
            .filter(|c| !c.is_whitespace())
            .collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(body.as_bytes())
            .map_err(|e| format!("invalid PEM base64: {e}"))?;
        Ok((label, der))
    } else {
        // Bare base64 (whitespace tolerated) — no label.
        let stripped: String = input.chars().filter(|c| !c.is_whitespace()).collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(stripped.as_bytes())
            .map_err(|e| format!("certificate/key is not PEM or base64: {e}"))?;
        Ok((String::new(), der))
    }
}

/// A DER INTEGER carries a leading `0x00` when its high bit is set; RSA `n` is
/// unsigned, and we need its true octet length for the PKCS#1 block size.
fn strip_leading_zeros(bytes: &[u8]) -> Vec<u8> {
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    if first >= bytes.len() {
        vec![0]
    } else {
        bytes[first..].to_vec()
    }
}

/// Parse a certificate (PEM or DER) to raw DER bytes, validating it decodes.
pub fn cert_to_der(input: &str) -> Result<Vec<u8>, String> {
    let (_, der) = pem_to_der(input)?;
    Certificate::from_der(&der).map_err(|e| format!("invalid certificate: {e}"))?;
    Ok(der)
}

/// Parse a private key PEM/DER (RSA PKCS#1 or PKCS#8, or EC PKCS#8) into a
/// [`SigningKey`].
pub fn parse_private_key(input: &str) -> Result<SigningKey, String> {
    use p256::pkcs8::DecodePrivateKey;
    use pkcs1::RsaPrivateKey as Pkcs1;
    use pkcs8::PrivateKeyInfo;

    let (label, der) = pem_to_der(input)?;

    if label.contains("RSA PRIVATE KEY") {
        let key = Pkcs1::from_der(&der).map_err(|e| format!("RSA PKCS#1 parse error: {e}"))?;
        return Ok(SigningKey::Rsa {
            n: strip_leading_zeros(key.modulus.as_bytes()),
            d: key.private_exponent.as_bytes().to_vec(),
        });
    }

    // PKCS#8: dispatch on the inner algorithm.
    let pki = PrivateKeyInfo::from_der(&der).map_err(|e| format!("PKCS#8 parse error: {e}"))?;
    if pki.algorithm.oid == OID_RSA_ENCRYPTION {
        let key =
            Pkcs1::from_der(pki.private_key).map_err(|e| format!("RSA PKCS#1 parse error: {e}"))?;
        Ok(SigningKey::Rsa {
            n: strip_leading_zeros(key.modulus.as_bytes()),
            d: key.private_exponent.as_bytes().to_vec(),
        })
    } else if pki.algorithm.oid == OID_EC_PUBLIC_KEY {
        let sk = p256::ecdsa::SigningKey::from_pkcs8_der(&der)
            .map_err(|e| format!("EC PKCS#8 parse error (only P-256 is supported): {e}"))?;
        Ok(SigningKey::Ec(Box::new(sk)))
    } else {
        Err(format!(
            "unsupported private key algorithm {} (expected RSA or EC P-256)",
            pki.algorithm.oid
        ))
    }
}

// --- CMS assembly ----------------------------------------------------------

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).into()
}

fn alg(oid: ObjectIdentifier, params: Option<Any>) -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned {
        oid,
        parameters: params,
    }
}

fn der_null() -> Result<Any, String> {
    Any::encode_from(&der::asn1::Null).map_err(|e| format!("encode NULL: {e}"))
}

fn attr(oid: ObjectIdentifier, value: Any) -> Result<Attribute, String> {
    let values = SetOfVec::try_from(vec![value]).map_err(|e| format!("attr set: {e}"))?;
    Ok(Attribute { oid, values })
}

/// ESS `SigningCertificateV2` with a single `ESSCertIDv2` holding the SHA-256 of
/// the signer certificate (hashAlgorithm defaults to sha256 → omitted;
/// issuerSerial omitted). All lengths are fixed because the hash is 32 bytes, so
/// this hand-built DER is deterministic.
fn signing_certificate_v2(cert_sha256: &[u8; 32]) -> Vec<u8> {
    let mut der = Vec::with_capacity(40);
    der.extend_from_slice(&[0x30, 0x26]); // SigningCertificateV2 SEQUENCE
    der.extend_from_slice(&[0x30, 0x24]); //   certs SEQUENCE OF
    der.extend_from_slice(&[0x30, 0x22]); //     ESSCertIDv2 SEQUENCE
    der.extend_from_slice(&[0x04, 0x20]); //       certHash OCTET STRING (32)
    der.extend_from_slice(cert_sha256);
    der
}

/// Build a detached CMS SignedData (DER) over `message_digest` — the SHA-256 of
/// the PDF's signed byte range.
///
/// When `tsa_url` is `Some`, the signature is additionally timestamped: the
/// signature value is hashed and sent to that RFC 3161 Time-Stamp Authority, and
/// the returned token is embedded as the `id-aa-timeStampToken` unsigned
/// attribute — upgrading the result from PAdES-B-B to **PAdES-B-T**. This makes
/// a network call at sign time.
pub fn build_cms(
    message_digest: &[u8],
    signer: &SignerMaterial,
    signing_time: OffsetDateTime,
    tsa_url: Option<&str>,
) -> Result<Vec<u8>, String> {
    let cert =
        Certificate::from_der(&signer.cert_der).map_err(|e| format!("signer certificate: {e}"))?;

    let sha256_alg = alg(OID_SHA256, None);
    let cert_hash = sha256(&signer.cert_der);

    // Signed attributes (order is irrelevant — SetOfVec re-sorts to DER SET OF).
    let content_type = attr(
        OID_CONTENT_TYPE,
        Any::encode_from(&OID_DATA).map_err(|e| format!("content-type: {e}"))?,
    )?;
    let message_digest_attr = attr(
        OID_MESSAGE_DIGEST,
        Any::encode_from(
            &OctetString::new(message_digest.to_vec())
                .map_err(|e| format!("digest octets: {e}"))?,
        )
        .map_err(|e| format!("message-digest: {e}"))?,
    )?;
    let secs = signing_time.unix_timestamp().max(0) as u64;
    let utc = UtcTime::from_unix_duration(core::time::Duration::from_secs(secs))
        .map_err(|e| format!("signing-time: {e}"))?;
    let signing_time_attr = attr(
        OID_SIGNING_TIME,
        Any::encode_from(&utc).map_err(|e| format!("signing-time: {e}"))?,
    )?;
    let scv2 = signing_certificate_v2(&cert_hash);
    let signing_cert_attr = attr(
        OID_SIGNING_CERT_V2,
        Any::from_der(&scv2).map_err(|e| format!("signing-cert-v2: {e}"))?,
    )?;

    let signed_attrs = SetOfVec::try_from(vec![
        content_type,
        message_digest_attr,
        signing_time_attr,
        signing_cert_attr,
    ])
    .map_err(|e| format!("signed attributes: {e}"))?;

    // The signature covers the SET OF encoding (tag 0x31) of the attributes.
    let signed_attrs_der = signed_attrs
        .to_der()
        .map_err(|e| format!("encode signed attrs: {e}"))?;

    let (signature, sig_alg) = match &signer.key {
        SigningKey::Rsa { n, d } => {
            let digest = sha256(&signed_attrs_der);
            let mut digest_info = SHA256_DIGESTINFO_PREFIX.to_vec();
            digest_info.extend_from_slice(&digest);
            let em =
                do_pkcs1_pad(&digest_info, n.len(), 1).map_err(|e| format!("PKCS#1 pad: {e}"))?;
            let sig = do_modexp(&em, d, n).map_err(|e| format!("RSA sign: {e}"))?;
            (sig, alg(OID_RSA_ENCRYPTION, Some(der_null()?)))
        }
        SigningKey::Ec(sk) => {
            use p256::ecdsa::signature::Signer;
            let sig: p256::ecdsa::Signature = sk.sign(&signed_attrs_der);
            (
                sig.to_der().as_bytes().to_vec(),
                alg(OID_ECDSA_WITH_SHA256, None),
            )
        }
    };

    // PAdES-B-T: timestamp the signature value and carry the token as an
    // unsigned attribute. Unsigned attrs are outside the signature, so adding
    // this does not disturb the signature computed above.
    let unsigned_attrs = match tsa_url {
        Some(url) => {
            let imprint = sha256(&signature);
            let token = super::pades_tsa::fetch_timestamp_token(url, &imprint)?;
            let ts_attr = attr(
                super::pades_tsa::OID_TIMESTAMP_TOKEN,
                Any::from_der(&token).map_err(|e| format!("timestamp token: {e}"))?,
            )?;
            Some(SetOfVec::try_from(vec![ts_attr]).map_err(|e| format!("unsigned attrs: {e}"))?)
        }
        None => None,
    };

    let signer_info = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: cert.tbs_certificate.issuer.clone(),
            serial_number: cert.tbs_certificate.serial_number.clone(),
        }),
        digest_alg: sha256_alg.clone(),
        signed_attrs: Some(signed_attrs),
        signature_algorithm: sig_alg,
        signature: OctetString::new(signature).map_err(|e| format!("signature octets: {e}"))?,
        unsigned_attrs,
    };

    // certificates: signer first, then any intermediates.
    let mut cert_choices = vec![CertificateChoices::Certificate(cert)];
    for der in &signer.chain_der {
        let c = Certificate::from_der(der).map_err(|e| format!("chain certificate: {e}"))?;
        cert_choices.push(CertificateChoices::Certificate(c));
    }
    let certificates =
        CertificateSet::try_from(cert_choices).map_err(|e| format!("certificate set: {e}"))?;

    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![sha256_alg])
            .map_err(|e| format!("digest algs: {e}"))?,
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: OID_DATA,
            econtent: None, // detached
        },
        certificates: Some(certificates),
        crls: None,
        signer_infos: SignerInfos::try_from(vec![signer_info])
            .map_err(|e| format!("signer infos: {e}"))?,
    };

    let content_info = ContentInfo {
        content_type: OID_SIGNED_DATA,
        content: Any::encode_from(&signed_data).map_err(|e| format!("wrap signed data: {e}"))?,
    };
    content_info
        .to_der()
        .map_err(|e| format!("encode CMS: {e}"))
}

// --- CMS verification (the read side, for `pdf_verify`) --------------------

const OID_SHA256_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const OID_COMMON_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");

/// The result of verifying one detached CMS signature.
pub struct VerifyOutcome {
    /// The CMS signature verifies against the embedded certificate AND the
    /// signed content's digest matches the signature's message-digest attribute.
    pub valid: bool,
    /// The signer certificate's Common Name, if present.
    pub signer: Option<String>,
}

/// Verify a detached CMS `SignedData` over `signed_content`: check the
/// message-digest attribute equals `SHA-256(signed_content)`, then verify the
/// signature over the signed attributes with the embedded signer certificate's
/// public key (RSA PKCS#1 v1.5 or ECDSA P-256). Does **not** assert certificate
/// trust — only cryptographic integrity + authenticity against the embedded cert.
pub fn verify_cms(cms_der: &[u8], signed_content: &[u8]) -> Result<VerifyOutcome, String> {
    let ci = ContentInfo::from_der(cms_der).map_err(|e| format!("CMS parse: {e}"))?;
    if ci.content_type != OID_SIGNED_DATA {
        return Err("not a CMS SignedData".to_string());
    }
    let sd: SignedData = ci
        .content
        .decode_as()
        .map_err(|e| format!("SignedData: {e}"))?;
    let si = sd
        .signer_infos
        .0
        .as_slice()
        .first()
        .ok_or("no signer info")?;
    let attrs = si.signed_attrs.as_ref().ok_or("no signed attributes")?;
    let attrs_der = attrs.to_der().map_err(|e| format!("attrs der: {e}"))?;

    // 1. The message-digest attribute must equal SHA-256 of the signed content.
    let md = attrs
        .as_slice()
        .iter()
        .find(|a| a.oid == OID_MESSAGE_DIGEST)
        .and_then(|a| a.values.as_slice().first())
        .map(|v| v.value().to_vec())
        .ok_or("no message-digest attribute")?;
    let digest_ok = md == sha256(signed_content);

    // 2. The signer certificate (embedded), matched by issuer + serial.
    let cert = find_signer_cert(&sd, si).ok_or("signer certificate not embedded")?;
    let signer = cert_common_name(&cert);

    // 3. Verify the signature over the signed attributes.
    let sig = si.signature.as_bytes();
    let alg = si.signature_algorithm.oid;
    let sig_ok = if alg == OID_RSA_ENCRYPTION || alg == OID_SHA256_WITH_RSA {
        verify_rsa(&cert, &attrs_der, sig)
    } else if alg == OID_ECDSA_WITH_SHA256 {
        verify_ecdsa(&cert, &attrs_der, sig)
    } else {
        false
    };

    Ok(VerifyOutcome {
        valid: digest_ok && sig_ok,
        signer,
    })
}

fn spki_bytes(cert: &Certificate) -> &[u8] {
    cert.tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .raw_bytes()
}

fn verify_rsa(cert: &Certificate, attrs_der: &[u8], sig: &[u8]) -> bool {
    let Ok(pubkey) = pkcs1::RsaPublicKey::from_der(spki_bytes(cert)) else {
        return false;
    };
    let n = strip_leading_zeros(pubkey.modulus.as_bytes());
    let e = pubkey.public_exponent.as_bytes().to_vec();
    let Ok(em) = do_modexp(sig, &e, &n) else {
        return false;
    };
    // The recovered EM must end with the SHA-256 DigestInfo of the signed attrs.
    let mut expected = SHA256_DIGESTINFO_PREFIX.to_vec();
    expected.extend_from_slice(&sha256(attrs_der));
    em.ends_with(&expected)
}

fn verify_ecdsa(cert: &Certificate, attrs_der: &[u8], sig: &[u8]) -> bool {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    let Ok(vk) = VerifyingKey::from_sec1_bytes(spki_bytes(cert)) else {
        return false;
    };
    let Ok(s) = Signature::from_der(sig) else {
        return false;
    };
    vk.verify(attrs_der, &s).is_ok()
}

/// Find the embedded certificate matching the signer's issuer + serial (falling
/// back to the first certificate).
fn find_signer_cert(sd: &SignedData, si: &SignerInfo) -> Option<Certificate> {
    let certs = sd.certificates.as_ref()?;
    let cert_iter = || {
        certs.0.iter().filter_map(|c| match c {
            CertificateChoices::Certificate(cert) => Some(cert),
            _ => None,
        })
    };
    if let SignerIdentifier::IssuerAndSerialNumber(ias) = &si.sid {
        if let Some(c) = cert_iter().find(|c| {
            c.tbs_certificate.serial_number == ias.serial_number
                && c.tbs_certificate.issuer == ias.issuer
        }) {
            return Some(c.clone());
        }
    }
    cert_iter().next().cloned()
}

/// Extract the Common Name from a certificate's subject.
fn cert_common_name(cert: &Certificate) -> Option<String> {
    cert.tbs_certificate
        .subject
        .0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find(|atv| atv.oid == OID_COMMON_NAME)
        .map(|atv| String::from_utf8_lossy(atv.value.value()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(name: &str) -> String {
        std::fs::read_to_string(format!("tests/fixtures/pades/{name}"))
            .unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
    }

    fn material(cert: &str, key: &str) -> SignerMaterial {
        SignerMaterial {
            cert_der: cert_to_der(&load(cert)).expect("cert"),
            chain_der: vec![],
            key: parse_private_key(&load(key)).expect("key"),
        }
    }

    /// Re-parse a CMS blob and return `(signed_attrs_der, signature, message_digest)`.
    fn dissect(cms: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let ci = ContentInfo::from_der(cms).expect("ContentInfo");
        assert_eq!(ci.content_type, OID_SIGNED_DATA);
        let sd: SignedData = ci.content.decode_as().expect("SignedData");
        let si = sd.signer_infos.0.as_slice().first().expect("one signer");
        let attrs = si.signed_attrs.as_ref().expect("signed attrs");
        let attrs_der = attrs.to_der().expect("attrs der");
        // Extract the message-digest attribute value.
        let md = attrs
            .as_slice()
            .iter()
            .find(|a| a.oid == OID_MESSAGE_DIGEST)
            .and_then(|a| a.values.as_slice().first())
            .map(|v| v.value().to_vec())
            .expect("message-digest attr");
        (attrs_der, si.signature.as_bytes().to_vec(), md)
    }

    #[test]
    fn rsa_cms_signature_verifies_against_the_cert() {
        let signer = material("rsa_cert.pem", "rsa_key.pem");
        let digest = [0x11u8; 32];
        let cms = build_cms(&digest, &signer, OffsetDateTime::UNIX_EPOCH, None).expect("build");

        let (attrs_der, signature, md) = dissect(&cms);
        // `Any::value()` already unwraps the OCTET STRING, so `md` is the digest.
        assert_eq!(md, digest, "message-digest attr matches input");

        // Verify: EM = sig^e mod n must equal PKCS#1(DigestInfo(sha256(attrs))).
        let cert = Certificate::from_der(&signer.cert_der).unwrap();
        let spki = cert
            .tbs_certificate
            .subject_public_key_info
            .subject_public_key
            .raw_bytes();
        let pubkey = pkcs1::RsaPublicKey::from_der(spki).expect("rsa pubkey");
        let n = strip_leading_zeros(pubkey.modulus.as_bytes());
        let e = pubkey.public_exponent.as_bytes().to_vec();
        let em = do_modexp(&signature, &e, &n).expect("verify modexp");
        let mut expected = SHA256_DIGESTINFO_PREFIX.to_vec();
        expected.extend_from_slice(&sha256(&attrs_der));
        assert!(
            em.ends_with(&expected),
            "recovered EM must end with the SHA-256 DigestInfo"
        );
    }

    #[test]
    fn ec_cms_signature_verifies_against_the_cert() {
        use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};

        let signer = material("ec_cert.pem", "ec_key.pem");
        let digest = [0x22u8; 32];
        let cms = build_cms(&digest, &signer, OffsetDateTime::UNIX_EPOCH, None).expect("build");

        let (attrs_der, signature, md) = dissect(&cms);
        assert_eq!(md, digest);

        let cert = Certificate::from_der(&signer.cert_der).unwrap();
        let point = cert
            .tbs_certificate
            .subject_public_key_info
            .subject_public_key
            .raw_bytes();
        let vk = VerifyingKey::from_sec1_bytes(point).expect("ec pubkey");
        let sig = Signature::from_der(&signature).expect("ecdsa sig");
        vk.verify(&attrs_der, &sig).expect("ecdsa verify");
    }

    #[test]
    fn verify_cms_accepts_valid_and_rejects_tampering() {
        for (cert, key, cn) in [
            ("rsa_cert.pem", "rsa_key.pem", "Soli Test Signer"),
            ("ec_cert.pem", "ec_key.pem", "Soli EC Signer"),
        ] {
            let signer = material(cert, key);
            let content = b"the exact bytes that were signed";
            let digest = sha256(content);
            let cms = build_cms(&digest, &signer, OffsetDateTime::UNIX_EPOCH, None).expect("build");

            let ok = verify_cms(&cms, content).expect("verify");
            assert!(ok.valid, "{cert}: a valid signature verifies");
            assert_eq!(
                ok.signer.as_deref(),
                Some(cn),
                "{cert}: signer CN from cert"
            );

            // Tampered content → message-digest mismatch → invalid.
            let bad = verify_cms(&cms, b"different bytes entirely").expect("verify");
            assert!(!bad.valid, "{cert}: tampered content fails");
        }
    }

    #[test]
    fn cms_parses_as_valid_der() {
        let signer = material("rsa_cert.pem", "rsa_key.pem");
        let cms =
            build_cms(&[0x33u8; 32], &signer, OffsetDateTime::UNIX_EPOCH, None).expect("build");
        // A second decode round-trips — structurally valid DER.
        ContentInfo::from_der(&cms).expect("re-decode");
    }

    fn byte_range(pdf: &[u8]) -> [usize; 4] {
        let key = b"/ByteRange";
        let at = pdf.windows(key.len()).position(|w| w == key).unwrap();
        let open = at
            + key.len()
            + pdf[at + key.len()..]
                .iter()
                .position(|&b| b == b'[')
                .unwrap();
        let close = open + pdf[open..].iter().position(|&b| b == b']').unwrap();
        let nums: Vec<usize> = std::str::from_utf8(&pdf[open + 1..close])
            .unwrap()
            .split_whitespace()
            .map(|s| s.parse().unwrap())
            .collect();
        [nums[0], nums[1], nums[2], nums[3]]
    }

    /// Pull the CMS DER back out of the `/Contents` hex, trimming trailing
    /// zero-padding to the DER's own declared length.
    fn extract_cms(pdf: &[u8]) -> Vec<u8> {
        let key = b"/Contents";
        // Find the `/Contents` whose value is a hex string `<…>` (the signature),
        // not the page's `/Contents N 0 R` reference or a `<<` dict.
        let mut search = 0;
        let open = loop {
            let rel = pdf[search..]
                .windows(key.len())
                .position(|w| w == key)
                .expect("signature /Contents");
            let at = search + rel;
            let mut i = at + key.len();
            while i < pdf.len() && pdf[i].is_ascii_whitespace() {
                i += 1;
            }
            if pdf.get(i) == Some(&b'<') && pdf.get(i + 1) != Some(&b'<') {
                break i;
            }
            search = at + key.len();
        };
        let close = open + pdf[open..].iter().position(|&b| b == b'>').unwrap();
        let hex = &pdf[open + 1..close];
        let bytes: Vec<u8> = hex
            .chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect();
        // DER total length from the SEQUENCE header.
        let len_byte = bytes[1];
        let total = if len_byte < 0x80 {
            2 + len_byte as usize
        } else {
            let n = (len_byte & 0x7f) as usize;
            let l = bytes[2..2 + n]
                .iter()
                .fold(0usize, |a, &b| (a << 8) | b as usize);
            2 + n + l
        };
        bytes[..total].to_vec()
    }

    /// The complete pipeline as a Soli caller runs it, then verified exactly as
    /// an external validator would: re-hash the PDF's own `/ByteRange` and check
    /// it equals the CMS message-digest. Also writes the signed PDF to a temp
    /// path for optional `pdfsig` inspection.
    #[test]
    fn end_to_end_signed_pdf_byte_range_matches_cms() {
        let opts = soli_pdf::RenderOptions {
            fetch_images: false,
            font_dirs: vec!["font".into()],
            ..Default::default()
        };
        let tmpl = br#"{ "fonts": ["titillium"], "content": [
            { "type": "paragraph", "value": "Signed invoice #INV-2026-001" }
        ] }"#;
        let pdf = soli_pdf::render_to_bytes(tmpl, b"{}", &opts).expect("render");

        let signer = material("rsa_cert.pem", "rsa_key.pem");
        let meta = soli_pdf::SignMeta {
            reason: Some("Invoice issued".into()),
            name: Some("Soli Test Signer".into()),
            signing_time: Some("D:20260703120000+00'00'".into()),
            ..Default::default()
        };
        let prepared = soli_pdf::prepare_signature(&pdf, &meta, signer.cert_der.len() + 4096)
            .expect("prepare");
        let digest = sha256(&prepared.signed_bytes());
        let cms = build_cms(&digest, &signer, OffsetDateTime::UNIX_EPOCH, None).expect("cms");
        let signed = soli_pdf::embed_cms(prepared, &cms).expect("embed");

        // Verify like a reader: digest the two /ByteRange spans of the FINAL
        // file and compare to the message-digest baked into the CMS.
        let br = byte_range(&signed);
        let mut content = signed[br[0]..br[0] + br[1]].to_vec();
        content.extend_from_slice(&signed[br[2]..br[2] + br[3]]);
        let recomputed = sha256(&content);
        let (_, _, md) = dissect(&extract_cms(&signed));
        assert_eq!(md, recomputed.to_vec(), "ByteRange digest matches CMS");

        let out = std::env::temp_dir().join("soli_signed_test.pdf");
        std::fs::write(&out, &signed).unwrap();
        eprintln!("wrote signed PDF: {}", out.display());
    }
}
