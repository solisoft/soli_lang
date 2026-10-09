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
//!
//! Two more read what a certificate *says* rather than what key it carries —
//! enough to watch expirations without shelling out to `openssl`:
//!
//! ```text
//! info = X509.info(pem)                      # subject, issuer, not_before, not_after, days_left, …
//! peer = X509.peer_certificate("example.com") # the same, for the cert a server presents
//! ```
//!
//! `peer_certificate` completes a TLS handshake WITHOUT validating the chain:
//! a probe has to be able to report an expired or self-signed certificate,
//! which a validating client would refuse before showing it. It only reads —
//! nothing is sent after the handshake — and it goes through the same SSRF
//! guard as `HTTP`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use der::{Decode, Encode, Tag, Tagged};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::name::Name;
use x509_cert::spki::ObjectIdentifier;
use x509_cert::Certificate;

#[cfg(not(target_arch = "wasm32"))]
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
#[cfg(not(target_arch = "wasm32"))]
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
#[cfg(not(target_arch = "wasm32"))]
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
#[cfg(not(target_arch = "wasm32"))]
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, NativeFunction, Value};

use super::hex::{encode as bytes_to_hex, HexError};

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    super::hex::decode(hex).map_err(|e| match e {
        HexError::OddLength => "invalid hex: odd length".to_string(),
        HexError::InvalidPair(_) => "invalid hex digit".to_string(),
    })
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

/// ISO-8601 UTC, second precision: `2026-09-25T00:00:00Z`.
fn iso_utc(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

fn parse_certificate(der: &[u8]) -> Result<Certificate, String> {
    Certificate::from_der(der).map_err(|e| format!("invalid certificate: {}", e))
}

const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const SUBJECT_ALT_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.17");

/// Why a certificate's RSA key could not be read.
enum RsaKeyError {
    NotRsa,
    Malformed(String),
}

/// The RSA modulus and public exponent, as unsigned big-endian bytes.
fn rsa_public_key(cert: &Certificate) -> Result<(Vec<u8>, Vec<u8>), RsaKeyError> {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid != RSA_ENCRYPTION {
        return Err(RsaKeyError::NotRsa);
    }
    let key = pkcs1::RsaPublicKey::from_der(spki.subject_public_key.raw_bytes())
        .map_err(|e| RsaKeyError::Malformed(e.to_string()))?;
    Ok((
        strip_leading_zeros(key.modulus.as_bytes()).to_vec(),
        strip_leading_zeros(key.public_exponent.as_bytes()).to_vec(),
    ))
}

/// The DER of the SubjectPublicKeyInfo — what an SPKI pin hashes.
fn spki_der(cert: &Certificate) -> Result<Vec<u8>, String> {
    cert.tbs_certificate
        .subject_public_key_info
        .to_der()
        .map_err(|e| format!("cannot encode the public key: {}", e))
}

/// The DNS names of the subjectAltName extension; none when it is absent or
/// unreadable.
fn dns_names(cert: &Certificate) -> Vec<String> {
    let mut names = Vec::new();
    for extension in cert.tbs_certificate.extensions.iter().flatten() {
        if extension.extn_id != SUBJECT_ALT_NAME {
            continue;
        }
        if let Ok(san) = SubjectAltName::from_der(extension.extn_value.as_bytes()) {
            for name in san.0 {
                if let GeneralName::DnsName(dns) = name {
                    names.push(dns.to_string());
                }
            }
        }
    }
    names
}

/// The serial number as colon-separated lowercase hex of its encoded bytes
/// (`00:a3:…` — the sign byte included), as `openssl x509 -serial` groups it.
fn serial_string(cert: &Certificate) -> String {
    cert.tbs_certificate
        .serial_number
        .as_bytes()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<Vec<_>>()
        .join(":")
}

/// Short names for attribute types, in the form `X509.info` has always
/// printed them (`CN=example.com, O=Acme`): the usual abbreviations first,
/// then the X.500 attribute names.
const ATTRIBUTE_NAMES: &[(&str, &str)] = &[
    ("2.5.4.3", "CN"),
    ("2.5.4.6", "C"),
    ("2.5.4.7", "L"),
    ("2.5.4.8", "ST"),
    ("2.5.4.10", "O"),
    ("2.5.4.11", "OU"),
    ("0.9.2342.19200300.100.1.25", "DC"),
    ("1.2.840.113549.1.9.1", "Email"),
    ("0.9.2342.19200300.100.1.1", "uid"),
    ("0.9.2342.19200300.100.1.25", "domainComponent"),
    ("1.3.6.1.4.1.311.60.2.1.1", "msJurisdictionLocality"),
    ("1.3.6.1.4.1.311.60.2.1.2", "msJurisdictionStateOrProvince"),
    ("1.3.6.1.4.1.311.60.2.1.3", "msJurisdictionCountry"),
    ("2.5.4.0", "objectClass"),
    ("2.5.4.1", "aliasedEntryName"),
    ("2.5.4.2", "knowledgeInformation"),
    ("2.5.4.3", "commonName"),
    ("2.5.4.4", "surname"),
    ("2.5.4.5", "serialNumber"),
    ("2.5.4.6", "countryName"),
    ("2.5.4.7", "localityName"),
    ("2.5.4.8", "stateOrProvinceName"),
    ("2.5.4.9", "streetAddress"),
    ("2.5.4.10", "organizationName"),
    ("2.5.4.11", "organizationalUnit"),
    ("2.5.4.12", "title"),
    ("2.5.4.13", "description"),
    ("2.5.4.14", "searchGuide"),
    ("2.5.4.15", "businessCategory"),
    ("2.5.4.16", "postalAddress"),
    ("2.5.4.17", "postalCode"),
    ("2.5.4.41", "name"),
    ("2.5.4.42", "givenName"),
    ("2.5.4.43", "initials"),
    ("2.5.4.44", "generationQualifier"),
    ("2.5.4.45", "uniqueIdentifier"),
    ("2.5.4.46", "dnQualifier"),
];

/// A distinguished name in RDN order, `TYPE=value` joined by `, ` (and `+`
/// inside a multi-valued RDN). String values print as text; anything else as
/// uppercase hex of its content.
fn name_to_string(name: &Name) -> String {
    let mut out = String::new();
    for rdn in name.0.iter() {
        if !out.is_empty() {
            out.push_str(", ");
        }
        for (i, attribute) in rdn.0.iter().enumerate() {
            if i > 0 {
                out.push_str(" + ");
            }
            let oid = attribute.oid.to_string();
            match ATTRIBUTE_NAMES.iter().find(|(known, _)| *known == oid) {
                Some((_, short)) => out.push_str(short),
                None => {
                    out.push_str("OID(");
                    out.push_str(&oid);
                    out.push(')');
                }
            }
            out.push('=');
            let Some(value) = attribute_value(attribute.value.tag(), attribute.value.value())
            else {
                return "<X509Error: Invalid X.509 name>".to_string();
            };
            out.push_str(&value);
        }
    }
    out
}

fn attribute_value(tag: Tag, content: &[u8]) -> Option<String> {
    match tag {
        Tag::Utf8String
        | Tag::PrintableString
        | Tag::Ia5String
        | Tag::TeletexString
        | Tag::VideotexString
        | Tag::NumericString
        | Tag::VisibleString => std::str::from_utf8(content).ok().map(str::to_string),
        Tag::BmpString => {
            if !content.len().is_multiple_of(2) {
                return None;
            }
            let units: Vec<u16> = content
                .chunks(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&units).ok()
        }
        _ => Some(content.iter().map(|b| format!("{:02X}", b)).collect()),
    }
}

/// DER → PEM, 64 columns, the way `openssl x509` writes it.
fn der_to_pem(der: &[u8]) -> String {
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

/// What a certificate says, as `(key, value)` pairs, judged against `now`
/// (unix seconds) — a parameter so tests do not depend on the clock.
///
/// `days_left` is whole days until `not_after`, floored, negative once the
/// certificate has expired: a probe compares it to a threshold, and "zero"
/// must not mean both "expires today" and "expired yesterday".
fn cert_info_pairs(der: &[u8], now: i64) -> Result<Vec<(String, Value)>, String> {
    let cert = parse_certificate(der)?;
    let validity = &cert.tbs_certificate.validity;
    let not_before = validity.not_before.to_unix_duration().as_secs() as i64;
    let not_after = validity.not_after.to_unix_duration().as_secs() as i64;
    let dns_names: Vec<Value> = dns_names(&cert)
        .into_iter()
        .map(|name| Value::String(name.into()))
        .collect();

    Ok(vec![
        (
            "subject".to_string(),
            Value::String(name_to_string(&cert.tbs_certificate.subject).into()),
        ),
        (
            "issuer".to_string(),
            Value::String(name_to_string(&cert.tbs_certificate.issuer).into()),
        ),
        (
            "serial".to_string(),
            Value::String(serial_string(&cert).into()),
        ),
        (
            "not_before".to_string(),
            Value::String(iso_utc(not_before).into()),
        ),
        (
            "not_after".to_string(),
            Value::String(iso_utc(not_after).into()),
        ),
        ("not_before_unix".to_string(), Value::Int(not_before)),
        ("not_after_unix".to_string(), Value::Int(not_after)),
        (
            "days_left".to_string(),
            Value::Int((not_after - now).div_euclid(86_400)),
        ),
        ("expired".to_string(), Value::Bool(now > not_after)),
        ("not_yet_valid".to_string(), Value::Bool(now < not_before)),
        (
            "dns_names".to_string(),
            Value::Array(Rc::new(RefCell::new(dns_names))),
        ),
        ("pem".to_string(), Value::String(der_to_pem(der).into())),
    ])
}

/// A verifier that accepts any certificate but still checks the handshake
/// signatures: the peer must hold the key of the certificate it presents,
/// otherwise we would be reporting a certificate it merely replayed.
#[derive(Debug)]
#[cfg(not(target_arch = "wasm32"))]
struct RecordOnly {
    provider: Arc<CryptoProvider>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ServerCertVerifier for RecordOnly {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Complete a TLS handshake with `host:port` and return the presented chain
/// (leaf first), as DER. No application data is exchanged. `check_ssrf`
/// is off only in unit tests, which talk to a server on loopback.
#[cfg(not(target_arch = "wasm32"))]
fn fetch_peer_chain(
    host: &str,
    port: u16,
    timeout: Duration,
    check_ssrf: bool,
) -> Result<Vec<Vec<u8>>, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("host cannot be empty".to_string());
    }
    if check_ssrf {
        let authority = if host.contains(':') {
            format!("[{}]", host.trim_start_matches('[').trim_end_matches(']'))
        } else {
            host.to_string()
        };
        crate::interpreter::builtins::http_class::validate_url_for_ssrf(&format!(
            "https://{}:{}/",
            authority, port
        ))?;
    }

    let bare = host.trim_start_matches('[').trim_end_matches(']');
    // Every resolved address in turn: `localhost` answers `::1` first, and a
    // host whose IPv6 route is broken must still be read over IPv4.
    let addrs: Vec<_> = (bare, port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {}:{}: {}", bare, port, e))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("no address found for {}:{}", bare, port));
    }
    let mut last_error = String::new();
    let mut connected = None;
    for addr in &addrs {
        match TcpStream::connect_timeout(addr, timeout) {
            Ok(stream) => {
                connected = Some(stream);
                break;
            }
            Err(e) => last_error = e.to_string(),
        }
    }
    let mut tcp =
        connected.ok_or_else(|| format!("connect to {}:{} failed: {}", bare, port, last_error))?;
    let _ = tcp.set_read_timeout(Some(timeout));
    let _ = tcp.set_write_timeout(Some(timeout));

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS init failed: {}", e))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(RecordOnly { provider }))
        .with_no_client_auth();
    let server_name = ServerName::try_from(bare.to_string())
        .map_err(|_| format!("invalid TLS server name: {}", bare))?;
    let mut conn = ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| format!("TLS setup failed: {}", e))?;
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp)
            .map_err(|e| format!("TLS handshake with {}:{} failed: {}", bare, port, e))?;
    }
    let chain: Vec<Vec<u8>> = conn
        .peer_certificates()
        .ok_or_else(|| format!("{}:{} presented no certificate", bare, port))?
        .iter()
        .map(|c| c.as_ref().to_vec())
        .collect();
    conn.send_close_notify();
    let _ = conn.complete_io(&mut tcp);
    if chain.is_empty() {
        return Err(format!("{}:{} presented no certificate", bare, port));
    }
    Ok(chain)
}
#[cfg(target_arch = "wasm32")]
fn fetch_peer_chain(
    _host: &str,
    _port: u16,
    _timeout: Duration,
    _check_ssrf: bool,
) -> Result<Vec<Vec<u8>>, String> {
    Err(crate::platform::unsupported_on_edge("fetch_peer_chain"))
}

pub fn register_x509_builtins(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // X509.public_key(cert) -> { algorithm, n, e, bits }
    methods.insert(
        "public_key".to_string(),
        Rc::new(NativeFunction::new("X509.public_key", Some(1), |args| {
            let der = to_der(&args[0]).map_err(|e| format!("X509.public_key(): {}", e))?;
            let cert = parse_certificate(&der).map_err(|e| format!("X509.public_key(): {}", e))?;
            match rsa_public_key(&cert) {
                Ok((n, e)) => Ok(hash_from_pairs([
                    ("algorithm".to_string(), Value::String("RSA".into())),
                    ("n".to_string(), Value::String(bytes_to_hex(&n).into())),
                    ("e".to_string(), Value::String(bytes_to_hex(&e).into())),
                    ("bits".to_string(), Value::Int((n.len() * 8) as i64)),
                ])),
                Err(RsaKeyError::NotRsa) => Err(
                    "X509.public_key(): certificate does not contain an RSA public key".to_string(),
                ),
                Err(RsaKeyError::Malformed(e)) => Err(format!(
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
            let cert = parse_certificate(&der).map_err(|e| format!("X509.spki_pin(): {}", e))?;
            // The DER of the SubjectPublicKeyInfo — pinning this and not the
            // whole certificate is the entire point.
            let spki = spki_der(&cert).map_err(|e| format!("X509.spki_pin(): {}", e))?;
            let digest = Sha256::digest(&spki);
            let pin = base64::engine::general_purpose::STANDARD.encode(digest);
            Ok(Value::String(format!("sha256/{}", pin).into()))
        })),
    );

    // X509.info(cert) -> { subject, issuer, serial, not_before, not_after,
    //                      not_before_unix, not_after_unix, days_left,
    //                      expired, not_yet_valid, dns_names, pem }
    methods.insert(
        "info".to_string(),
        Rc::new(NativeFunction::new("X509.info", Some(1), |args| {
            let der = to_der(&args[0]).map_err(|e| format!("X509.info(): {}", e))?;
            let now = chrono::Utc::now().timestamp();
            let pairs = cert_info_pairs(&der, now).map_err(|e| format!("X509.info(): {}", e))?;
            Ok(hash_from_pairs(pairs))
        })),
    );

    // X509.peer_certificate(host, port = 443, timeout_seconds = 10)
    //   -> X509.info of the leaf, plus host, port and chain_length.
    methods.insert(
        "peer_certificate".to_string(),
        Rc::new(NativeFunction::new("X509.peer_certificate", None, |args| {
            if args.is_empty() || args.len() > 3 {
                return Err(format!(
                    "X509.peer_certificate() expects 1-3 arguments (host, port?, timeout?), got {}",
                    args.len()
                ));
            }
            let host = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "X509.peer_certificate() expects a string host, got {}",
                        other.type_name()
                    ))
                }
            };
            let port = match args.get(1) {
                None | Some(Value::Null) => 443,
                Some(Value::Int(n)) if (1..=65535).contains(n) => *n as u16,
                Some(other) => {
                    return Err(format!(
                        "X509.peer_certificate(): port must be an Int 1-65535, got {}",
                        other.type_name()
                    ))
                }
            };
            let timeout = match args.get(2) {
                None | Some(Value::Null) => Duration::from_secs(10),
                Some(Value::Int(n)) if *n > 0 && *n <= 60 => Duration::from_secs(*n as u64),
                Some(Value::Float(f)) if *f > 0.0 && *f <= 60.0 => Duration::from_secs_f64(*f),
                Some(_) => {
                    return Err("X509.peer_certificate(): timeout must be 0-60 seconds".to_string())
                }
            };
            let chain = fetch_peer_chain(&host, port, timeout, true)
                .map_err(|e| format!("X509.peer_certificate(): {}", e))?;
            let now = chrono::Utc::now().timestamp();
            let mut pairs = cert_info_pairs(&chain[0], now)
                .map_err(|e| format!("X509.peer_certificate(): {}", e))?;
            pairs.push(("host".to_string(), Value::String(host.into())));
            pairs.push(("port".to_string(), Value::Int(port as i64)));
            pairs.push(("chain_length".to_string(), Value::Int(chain.len() as i64)));
            Ok(hash_from_pairs(pairs))
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
            let cert = parse_certificate(&der).unwrap();
            let digest = Sha256::digest(spki_der(&cert).unwrap());
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

    // Two self-signed P-256 certificates with known dates, generated once with
    // `openssl req -x509 -not_before … -not_after …` (tests/fixtures/x509/).
    const EXPIRED_CERT: &str = include_str!("../../../tests/fixtures/x509/expire.pem");
    const EXPIRED_KEY: &str = include_str!("../../../tests/fixtures/x509/expire.key");
    const VALID_CERT: &str = include_str!("../../../tests/fixtures/x509/valide.pem");

    // 2026-01-01T00:00:00Z
    const NEW_YEAR_2026: i64 = 1_767_225_600;

    fn field(pairs: &[(String, Value)], key: &str) -> Value {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("no field {}", key))
    }

    fn text(v: Value) -> String {
        match v {
            Value::String(s) => s.to_string(),
            other => panic!("expected a string, got {}", other.type_name()),
        }
    }

    #[test]
    fn info_reads_the_validity_dates_and_names() {
        let der = to_der(&Value::String(EXPIRED_CERT.into())).unwrap();
        let pairs = cert_info_pairs(&der, NEW_YEAR_2026).unwrap();
        assert_eq!(text(field(&pairs, "not_before")), "2020-01-01T00:00:00Z");
        assert_eq!(text(field(&pairs, "not_after")), "2021-01-01T00:00:00Z");
        assert!(text(field(&pairs, "subject")).contains("expire.test"));
        assert!(matches!(field(&pairs, "expired"), Value::Bool(true)));
        // 2021-01-01 → 2026-01-01 is 1826 days; expired, so negative.
        assert!(matches!(field(&pairs, "days_left"), Value::Int(-1826)));
        match field(&pairs, "dns_names") {
            Value::Array(a) => {
                let names: Vec<String> = a.borrow().iter().cloned().map(text).collect();
                assert_eq!(
                    names,
                    vec!["expire.test".to_string(), "localhost".to_string()]
                );
            }
            other => panic!("dns_names is {}", other.type_name()),
        }
        // The PEM we hand back parses to the same certificate.
        let again = to_der(&field(&pairs, "pem")).unwrap();
        assert_eq!(again, der);
    }

    #[test]
    fn days_left_counts_whole_days_until_expiry() {
        let der = to_der(&Value::String(VALID_CERT.into())).unwrap();
        let pairs = cert_info_pairs(&der, NEW_YEAR_2026).unwrap();
        // 2026-01-01 → 2036-01-01: ten years, two of them leap (2028, 2032).
        assert!(matches!(field(&pairs, "days_left"), Value::Int(3652)));
        assert!(matches!(field(&pairs, "expired"), Value::Bool(false)));
        // One second before expiry is day 0, not day 1; one second after is -1.
        let not_after = 2_082_758_400; // 2036-01-01T00:00:00Z
        let last = cert_info_pairs(&der, not_after - 1).unwrap();
        assert!(matches!(field(&last, "days_left"), Value::Int(0)));
        let past = cert_info_pairs(&der, not_after + 1).unwrap();
        assert!(matches!(field(&past, "days_left"), Value::Int(-1)));
        assert!(matches!(field(&past, "expired"), Value::Bool(true)));
    }

    /// The reason this function exists: a server presenting an EXPIRED
    /// certificate must be read, not refused — a validating client would fail
    /// the handshake and the probe would report an error instead of a date.
    #[test]
    fn peer_certificate_reads_an_expired_certificate_from_a_live_server() {
        use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
        use std::net::TcpListener;

        let cert_der = to_der(&Value::String(EXPIRED_CERT.into())).unwrap();
        let key_der = to_der(&Value::String(EXPIRED_KEY.into())).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server_config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert_der.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut conn = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
            while conn.is_handshaking() {
                if conn.complete_io(&mut socket).is_err() {
                    return;
                }
            }
            let _ = conn.complete_io(&mut socket);
        });

        let chain = fetch_peer_chain("localhost", port, Duration::from_secs(5), false).unwrap();
        server.join().unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0], cert_der);
        let pairs = cert_info_pairs(&chain[0], NEW_YEAR_2026).unwrap();
        assert!(matches!(field(&pairs, "expired"), Value::Bool(true)));
        assert_eq!(text(field(&pairs, "not_after")), "2021-01-01T00:00:00Z");
    }

    #[test]
    fn peer_certificate_reports_a_closed_port_as_an_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = fetch_peer_chain("127.0.0.1", port, Duration::from_secs(2), false).unwrap_err();
        assert!(err.contains("failed"), "{}", err);
    }

    /// Loopback is refused outside the test runner, like `HTTP`. The flag is
    /// process-global and other tests turn it on, so the assertion only runs
    /// while it is off.
    #[test]
    fn peer_certificate_goes_through_the_ssrf_guard() {
        if crate::interpreter::builtins::http_class::ssrf_test_mode() {
            return;
        }
        let err = fetch_peer_chain("127.0.0.1", 443, Duration::from_secs(1), true).unwrap_err();
        assert!(err.contains("not allowed"), "{}", err);
    }
}
