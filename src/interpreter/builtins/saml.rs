//! SAML 2.0 Web Browser SSO (`Saml.*`) — the service-provider half.
//!
//! The class is **stateless**, like `WebAuthn`: it builds the request that
//! sends a browser to the identity provider, and it verifies the response the
//! browser posts back. Keeping the request id between the two (single use,
//! bound to the session) is the application's job — `soli generate saml`
//! scaffolds it.
//!
//! Five static methods:
//!
//! - `Saml.auth_request(opts)` → `{id, url, xml}` (HTTP-Redirect binding)
//! - `Saml.verify_response(saml_response, expected)` → the verified assertion
//! - `Saml.metadata(opts)` → the service provider's metadata XML
//! - `Saml.parse_metadata(xml)` → `{entity_id, sso_url, certificates}`
//! - `Saml.test_response(opts)` → a signed response, for specs
//!
//! Every failed check **raises**; nothing fails by returning `false`, so a
//! forgotten check at the call site cannot turn a rejection into a login.
//!
//! ## What `verify_response` trusts
//!
//! XML signature wrapping (XSW) is the classic SAML break: the signature
//! verifies over one element while the application reads another. Here the
//! two cannot diverge, by construction:
//!
//! - a signature is only looked for as a **direct child** of the `Response`
//!   or of its single direct-child `Assertion`;
//! - its one `Reference` must name the `ID` of **that parent** — never an
//!   element found by searching the document for the id;
//! - the digest is computed over the very node the claims are then read from,
//!   and an `ID` that appears twice in the document is refused;
//! - the key is the certificate the caller configured, never the `KeyInfo`
//!   the message carries.
//!
//! Text is read with comments already dropped by the parser and every text
//! node concatenated, so `user@example.com<!---->.evil.com` reads as what
//! was signed.
//!
//! Not supported, and refused by name: encrypted assertions, `EncryptedID`,
//! inclusive canonicalization, ECDSA and RSA-PSS signatures, SHA-384. SHA-1
//! is refused unless `allow_sha1: true`. Requests are not signed, and there
//! is no single logout.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rand_core::{OsRng, RngCore};
use serde_json::{json, Map as JsonMap, Value as Json};
use sha2::{Digest, Sha256, Sha512};

use super::xml::c14n::{
    canonicalize, element_ns_uri, parse_document, root_in_scope, Element, Node,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value, value_to_json, Class, NativeFunction, Value};

const SAMLP_NS: &str = "urn:oasis:names:tc:SAML:2.0:protocol";
const SAML_NS: &str = "urn:oasis:names:tc:SAML:2.0:assertion";
const MD_NS: &str = "urn:oasis:names:tc:SAML:2.0:metadata";
const DSIG_NS: &str = "http://www.w3.org/2000/09/xmldsig#";
const EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";
const EXC_C14N_COMMENTS: &str = "http://www.w3.org/2001/10/xml-exc-c14n#WithComments";
const ENVELOPED: &str = "http://www.w3.org/2000/09/xmldsig#enveloped-signature";
const STATUS_SUCCESS: &str = "urn:oasis:names:tc:SAML:2.0:status:Success";
const BEARER: &str = "urn:oasis:names:tc:SAML:2.0:cm:bearer";
const BINDING_REDIRECT: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
const BINDING_POST: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST";
const NAMEID_EMAIL: &str = "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress";

/// A response is a few kilobytes; this is unauthenticated input.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_CLOCK_SKEW: i64 = 120;
const MAX_CLOCK_SKEW: i64 = 600;

type Scope = BTreeMap<String, String>;

/// An element together with the namespace context it inherits, which is what
/// both name resolution and canonicalization need.
#[derive(Clone)]
struct Scoped<'a> {
    el: &'a Element,
    inherited: Rc<Scope>,
}

impl<'a> Scoped<'a> {
    fn root(el: &'a Element) -> Self {
        Scoped {
            el,
            inherited: Rc::new(root_in_scope()),
        }
    }

    fn is(&self, ns: &str, local: &str) -> bool {
        self.el.local == local && element_ns_uri(self.el, &self.inherited) == ns
    }

    fn children(&self) -> Vec<Scoped<'a>> {
        let mut scope = (*self.inherited).clone();
        for (prefix, uri) in &self.el.ns_decls {
            scope.insert(prefix.clone(), uri.clone());
        }
        let scope = Rc::new(scope);
        self.el
            .children
            .iter()
            .filter_map(|node| match node {
                Node::Element(el) => Some(Scoped {
                    el,
                    inherited: Rc::clone(&scope),
                }),
                _ => None,
            })
            .collect()
    }

    fn named(&self, ns: &str, local: &str) -> Vec<Scoped<'a>> {
        self.children()
            .into_iter()
            .filter(|child| child.is(ns, local))
            .collect()
    }

    /// The one child with this name; an error names a missing or repeated one.
    fn only(&self, ns: &str, local: &str) -> Result<Scoped<'a>, String> {
        let mut found = self.named(ns, local);
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err(format!("<{}> has no <{}>", self.el.local, local)),
            n => Err(format!("<{}> has {} <{}>", self.el.local, n, local)),
        }
    }

    fn first(&self, ns: &str, local: &str) -> Option<Scoped<'a>> {
        self.named(ns, local).into_iter().next()
    }

    /// An attribute in no namespace, which is where SAML and XML-DSig put theirs.
    fn attr(&self, name: &str) -> Option<&'a str> {
        self.el
            .attrs
            .iter()
            .find(|a| a.prefix.is_empty() && a.local == name)
            .map(|a| a.value.as_str())
    }

    /// Every text node of the element, concatenated and trimmed. Comments are
    /// gone by now, so a value split by one reads whole.
    fn text(&self) -> String {
        let mut out = String::new();
        for node in &self.el.children {
            if let Node::Text(text) = node {
                out.push_str(text);
            }
        }
        out.trim().to_string()
    }

    fn canonical(&self, prefixes: &BTreeSet<String>, skip: Option<*const Element>) -> String {
        let mut out = String::new();
        canonicalize(
            self.el,
            &self.inherited,
            &BTreeMap::new(),
            prefixes,
            false,
            skip,
            &mut out,
        );
        out
    }
}

fn collect_ids(el: &Element, seen: &mut BTreeSet<String>) -> Result<(), String> {
    if let Some(id) = &el.id {
        if !seen.insert(id.clone()) {
            return Err(format!("the id '{}' appears more than once", id));
        }
    }
    for node in &el.children {
        if let Node::Element(child) = node {
            collect_ids(child, seen)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Expectations
// ---------------------------------------------------------------------------

/// What the caller holds a response to.
pub(crate) struct Expected {
    /// RSA public keys `(n, e)` of the identity provider's signing certificates.
    keys: Vec<(Vec<u8>, Vec<u8>)>,
    /// The service provider's entity id.
    audience: String,
    /// The assertion consumer service URL the response must be addressed to.
    acs_url: String,
    /// The id of the request this answers; `None` only with `allow_unsolicited`.
    request_id: Option<String>,
    idp_entity_id: Option<String>,
    now: i64,
    clock_skew: i64,
    allow_sha1: bool,
}

fn opt_str(obj: &Json, key: &str) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
        Some(Json::String(_)) => Ok(None),
        Some(_) => Err(format!("{} must be a String", key)),
    }
}

fn req_str(obj: &Json, key: &str) -> Result<String, String> {
    opt_str(obj, key)?.ok_or_else(|| format!("{} is required", key))
}

fn opt_bool(obj: &Json, key: &str) -> Result<bool, String> {
    match obj.get(key) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{} must be true or false", key)),
    }
}

fn opt_int(obj: &Json, key: &str) -> Result<Option<i64>, String> {
    match obj.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("{} must be an Int", key)),
    }
}

fn unix_now() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn certificate_key(text: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let der = super::x509::to_der(&Value::String(text.into()))?;
    let cert = super::x509::parse_certificate(&der)?;
    super::x509::rsa_public_key(&cert).map_err(|e| match e {
        super::x509::RsaKeyError::NotRsa => {
            "the certificate's key is not RSA (ECDSA signatures are not supported)".to_string()
        }
        super::x509::RsaKeyError::Malformed(reason) => {
            format!("the certificate's RSA key is malformed: {}", reason)
        }
    })
}

impl Expected {
    pub(crate) fn from_json(obj: &Json) -> Result<Self, String> {
        let mut certificates: Vec<String> = Vec::new();
        for key in ["certificate", "certificates"] {
            match obj.get(key) {
                None | Some(Json::Null) => {}
                Some(Json::String(s)) => certificates.push(s.clone()),
                Some(Json::Array(items)) => {
                    for item in items {
                        match item {
                            Json::String(s) => certificates.push(s.clone()),
                            _ => return Err(format!("{} must hold Strings", key)),
                        }
                    }
                }
                Some(_) => return Err(format!("{} must be a String or an Array", key)),
            }
        }
        if certificates.is_empty() {
            return Err(
                "expected certificates is required (the identity provider's signing certificate)"
                    .to_string(),
            );
        }
        let keys = certificates
            .iter()
            .map(|text| certificate_key(text))
            .collect::<Result<Vec<_>, _>>()?;

        let request_id = opt_str(obj, "request_id")?;
        if request_id.is_none() && !opt_bool(obj, "allow_unsolicited")? {
            return Err(
                "expected request_id is required (the id Saml.auth_request returned); \
                 pass allow_unsolicited: true to accept a response nobody asked for"
                    .to_string(),
            );
        }
        let clock_skew = opt_int(obj, "clock_skew")?.unwrap_or(DEFAULT_CLOCK_SKEW);
        if !(0..=MAX_CLOCK_SKEW).contains(&clock_skew) {
            return Err(format!(
                "clock_skew must be between 0 and {} seconds",
                MAX_CLOCK_SKEW
            ));
        }
        Ok(Expected {
            keys,
            audience: req_str(obj, "audience")
                .map_err(|e| format!("expected {} (the service provider's entity id)", e))?,
            acs_url: req_str(obj, "acs_url").map_err(|e| format!("expected {}", e))?,
            request_id,
            idp_entity_id: opt_str(obj, "idp_entity_id")?,
            now: opt_int(obj, "now")?.unwrap_or_else(unix_now),
            clock_skew,
            allow_sha1: opt_bool(obj, "allow_sha1")?,
        })
    }
}

// ---------------------------------------------------------------------------
// XML signature
// ---------------------------------------------------------------------------

fn digest(algorithm: &str, bytes: &[u8]) -> Vec<u8> {
    use sha1::Sha1;
    match algorithm {
        "sha1" => Sha1::digest(bytes).to_vec(),
        "sha512" => Sha512::digest(bytes).to_vec(),
        _ => Sha256::digest(bytes).to_vec(),
    }
}

fn hash_of(uri: &str, what: &str, allow_sha1: bool) -> Result<&'static str, String> {
    match uri {
        "http://www.w3.org/2001/04/xmlenc#sha256"
        | "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256" => Ok("sha256"),
        "http://www.w3.org/2001/04/xmlenc#sha512"
        | "http://www.w3.org/2001/04/xmldsig-more#rsa-sha512" => Ok("sha512"),
        "http://www.w3.org/2000/09/xmldsig#sha1" | "http://www.w3.org/2000/09/xmldsig#rsa-sha1" => {
            if allow_sha1 {
                Ok("sha1")
            } else {
                Err(format!(
                    "the {} uses SHA-1; switch the identity provider to SHA-256, \
                     or pass allow_sha1: true",
                    what
                ))
            }
        }
        other => Err(format!("unsupported {} algorithm {}", what, other)),
    }
}

fn algorithm_of<'a>(el: &Scoped<'a>, what: &str) -> Result<&'a str, String> {
    el.attr("Algorithm")
        .ok_or_else(|| format!("<{}> has no Algorithm", what))
}

fn exclusive_c14n(uri: &str, what: &str) -> Result<(), String> {
    if uri == EXC_C14N || uri == EXC_C14N_COMMENTS {
        Ok(())
    } else {
        Err(format!(
            "unsupported {} {} (only exclusive canonicalization)",
            what, uri
        ))
    }
}

/// The `PrefixList` of an `<ec:InclusiveNamespaces>` child, if there is one.
fn inclusive_prefixes(el: &Scoped) -> BTreeSet<String> {
    let mut prefixes = BTreeSet::new();
    if let Some(list) = el
        .first(EXC_C14N, "InclusiveNamespaces")
        .and_then(|inclusive| inclusive.attr("PrefixList"))
    {
        for prefix in list.split_whitespace() {
            prefixes.insert(if prefix == "#default" {
                String::new()
            } else {
                prefix.to_string()
            });
        }
    }
    prefixes
}

fn decode_base64(text: &str, what: &str) -> Result<Vec<u8>, String> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| format!("{} is not valid base64", what))
}

/// Verify `signature`, a direct child of `signed`, over `signed` itself.
fn verify_signature(
    signed: &Scoped,
    signature: &Scoped,
    expected: &Expected,
) -> Result<(), String> {
    let id = signed
        .attr("ID")
        .filter(|id| !id.is_empty())
        .ok_or_else(|| format!("the signed <{}> has no ID", signed.el.local))?;

    let signed_info = signature.only(DSIG_NS, "SignedInfo")?;
    let method = signed_info.only(DSIG_NS, "CanonicalizationMethod")?;
    exclusive_c14n(
        algorithm_of(&method, "CanonicalizationMethod")?,
        "canonicalization",
    )?;
    let signature_hash = hash_of(
        algorithm_of(
            &signed_info.only(DSIG_NS, "SignatureMethod")?,
            "SignatureMethod",
        )?,
        "signature",
        expected.allow_sha1,
    )?;

    // One reference, to the element that carries the signature. Anything else
    // is how a wrapped assertion gets a valid signature it does not own.
    let reference = signed_info.only(DSIG_NS, "Reference")?;
    if reference.attr("URI") != Some(format!("#{}", id).as_str()) {
        return Err(format!(
            "the signature does not reference the <{}> that carries it",
            signed.el.local
        ));
    }
    let mut prefixes = BTreeSet::new();
    if let Some(transforms) = reference.first(DSIG_NS, "Transforms") {
        let list = transforms.named(DSIG_NS, "Transform");
        if list.len() > 2 {
            return Err("the signature has more than two transforms".to_string());
        }
        for transform in &list {
            let uri = algorithm_of(transform, "Transform")?;
            if uri == ENVELOPED {
                continue;
            }
            exclusive_c14n(uri, "transform")?;
            prefixes = inclusive_prefixes(transform);
        }
    }
    let digest_hash = hash_of(
        algorithm_of(&reference.only(DSIG_NS, "DigestMethod")?, "DigestMethod")?,
        "digest",
        expected.allow_sha1,
    )?;
    let digest_value = decode_base64(
        &reference.only(DSIG_NS, "DigestValue")?.text(),
        "DigestValue",
    )?;
    let canonical = signed.canonical(&prefixes, Some(signature.el as *const Element));
    if digest(digest_hash, canonical.as_bytes()) != digest_value {
        return Err(format!(
            "the <{}> was modified after it was signed (digest mismatch)",
            signed.el.local
        ));
    }

    let signed_bytes = signed_info.canonical(&inclusive_prefixes(&method), None);
    let signature_value = decode_base64(
        &signature.only(DSIG_NS, "SignatureValue")?.text(),
        "SignatureValue",
    )?;
    for (n, e) in &expected.keys {
        if super::rsa_key::verify_pkcs1_v15_components(
            n,
            e,
            signed_bytes.as_bytes(),
            &signature_value,
            signature_hash,
        )? {
            return Ok(());
        }
    }
    Err(format!(
        "the signature on the <{}> does not verify with the identity provider's certificate",
        signed.el.local
    ))
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

fn parse_instant(text: &str, what: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(text.trim())
        .map(|time| time.timestamp())
        .map_err(|_| format!("{} is not a timestamp: {}", what, text))
}

fn optional_instant(el: &Scoped, name: &str) -> Result<Option<i64>, String> {
    el.attr(name)
        .map(|text| parse_instant(text, name))
        .transpose()
}

/// `InResponseTo` against the request the caller made: equal when one was
/// made, absent when none was.
fn check_in_response_to(found: Option<&str>, expected: &Expected, on: &str) -> Result<(), String> {
    match (&expected.request_id, found) {
        (Some(want), Some(got)) if want == got => Ok(()),
        (Some(_), Some(_)) => Err(format!("{} answers another request", on)),
        (Some(_), None) => Ok(()),
        (None, Some(_)) => Err(format!("{} answers a request, and none was expected", on)),
        (None, None) => Ok(()),
    }
}

/// The bearer confirmation that makes the assertion ours to accept.
fn check_subject_confirmation(subject: &Scoped, expected: &Expected) -> Result<(), String> {
    let mut reason = "the subject has no bearer confirmation".to_string();
    for confirmation in subject.named(SAML_NS, "SubjectConfirmation") {
        if confirmation.attr("Method") != Some(BEARER) {
            continue;
        }
        let Some(data) = confirmation.first(SAML_NS, "SubjectConfirmationData") else {
            reason = "the bearer confirmation has no SubjectConfirmationData".to_string();
            continue;
        };
        let checked = (|| -> Result<(), String> {
            if data.attr("Recipient") != Some(expected.acs_url.as_str()) {
                return Err("the assertion is addressed to another recipient".to_string());
            }
            let not_on_or_after = optional_instant(&data, "NotOnOrAfter")?
                .ok_or("the bearer confirmation has no NotOnOrAfter")?;
            if expected.now - expected.clock_skew >= not_on_or_after {
                return Err("the assertion has expired".to_string());
            }
            if let Some(not_before) = optional_instant(&data, "NotBefore")? {
                if expected.now + expected.clock_skew < not_before {
                    return Err("the assertion is not valid yet".to_string());
                }
            }
            match (&expected.request_id, data.attr("InResponseTo")) {
                (Some(want), Some(got)) if want == got => Ok(()),
                (Some(_), _) => Err("the assertion answers another request".to_string()),
                (None, Some(_)) => {
                    Err("the assertion answers a request, and none was expected".to_string())
                }
                (None, None) => Ok(()),
            }
        })();
        match checked {
            Ok(()) => return Ok(()),
            Err(why) => reason = why,
        }
    }
    Err(reason)
}

fn check_conditions(assertion: &Scoped, expected: &Expected) -> Result<Option<i64>, String> {
    let conditions = assertion.only(SAML_NS, "Conditions")?;
    if let Some(not_before) = optional_instant(&conditions, "NotBefore")? {
        if expected.now + expected.clock_skew < not_before {
            return Err("the assertion is not valid yet".to_string());
        }
    }
    let not_on_or_after = optional_instant(&conditions, "NotOnOrAfter")?;
    if let Some(limit) = not_on_or_after {
        if expected.now - expected.clock_skew >= limit {
            return Err("the assertion has expired".to_string());
        }
    }
    let restrictions = conditions.named(SAML_NS, "AudienceRestriction");
    if restrictions.is_empty() {
        return Err("the assertion names no audience".to_string());
    }
    for restriction in &restrictions {
        let ours = restriction
            .named(SAML_NS, "Audience")
            .iter()
            .any(|audience| audience.text() == expected.audience);
        if !ours {
            return Err("the assertion was issued for another audience".to_string());
        }
    }
    Ok(not_on_or_after)
}

/// Verify a base64 `SAMLResponse` and return what its assertion says.
pub(crate) fn verify_response(encoded: &str, expected: &Expected) -> Result<Json, String> {
    if encoded.len() > MAX_RESPONSE_BYTES * 2 {
        return Err("the response is too large".to_string());
    }
    let bytes = decode_base64(encoded, "the response")?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("the response is too large".to_string());
    }
    let xml = String::from_utf8(bytes).map_err(|_| "the response is not UTF-8".to_string())?;
    let document = parse_document(&xml).map_err(|e| format!("the response is not XML: {}", e))?;
    collect_ids(&document, &mut BTreeSet::new())?;

    let response = Scoped::root(&document);
    if !response.is(SAMLP_NS, "Response") {
        return Err("the document is not a SAML Response".to_string());
    }
    if response.attr("Version") != Some("2.0") {
        return Err("the response is not SAML 2.0".to_string());
    }

    let status = response
        .only(SAMLP_NS, "Status")?
        .only(SAMLP_NS, "StatusCode")?;
    let code = status.attr("Value").unwrap_or_default();
    if code != STATUS_SUCCESS {
        let detail = status
            .first(SAMLP_NS, "StatusCode")
            .and_then(|inner| inner.attr("Value"))
            .map(|inner| format!(" ({})", inner))
            .unwrap_or_default();
        return Err(format!("the identity provider answered {}{}", code, detail));
    }

    if !response.named(SAML_NS, "EncryptedAssertion").is_empty() {
        return Err(
            "encrypted assertions are not supported: turn assertion encryption off \
             at the identity provider"
                .to_string(),
        );
    }
    let assertion = response.only(SAML_NS, "Assertion")?;
    if assertion.attr("Version") != Some("2.0") {
        return Err("the assertion is not SAML 2.0".to_string());
    }

    // Every signature present must verify; at least one must be present.
    let response_signatures = response.named(DSIG_NS, "Signature");
    let assertion_signatures = assertion.named(DSIG_NS, "Signature");
    if response_signatures.len() > 1 || assertion_signatures.len() > 1 {
        return Err("an element carries more than one signature".to_string());
    }
    let signed = match (response_signatures.first(), assertion_signatures.first()) {
        (None, None) => {
            return Err("neither the response nor the assertion is signed".to_string());
        }
        (Some(outer), None) => {
            verify_signature(&response, outer, expected)?;
            "response"
        }
        (None, Some(inner)) => {
            verify_signature(&assertion, inner, expected)?;
            "assertion"
        }
        (Some(outer), Some(inner)) => {
            verify_signature(&response, outer, expected)?;
            verify_signature(&assertion, inner, expected)?;
            "both"
        }
    };

    // The response's own addressing. A signed response states its destination.
    match response.attr("Destination") {
        Some(destination) if destination == expected.acs_url => {}
        Some(_) => return Err("the response is addressed to another destination".to_string()),
        None if signed != "assertion" => {
            return Err("the signed response has no Destination".to_string());
        }
        None => {}
    }
    check_in_response_to(response.attr("InResponseTo"), expected, "the response")?;

    // From here on, only the assertion node the signature covers is read.
    let issuer = assertion.only(SAML_NS, "Issuer")?.text();
    if let Some(want) = &expected.idp_entity_id {
        if &issuer != want {
            return Err("the assertion comes from another identity provider".to_string());
        }
    }
    let subject = assertion.only(SAML_NS, "Subject")?;
    if !subject.named(SAML_NS, "EncryptedID").is_empty() {
        return Err("an encrypted NameID is not supported".to_string());
    }
    let name_id = subject.only(SAML_NS, "NameID")?;
    let name = name_id.text();
    if name.is_empty() {
        return Err("the NameID is empty".to_string());
    }
    check_subject_confirmation(&subject, expected)?;
    let expires_at = check_conditions(&assertion, expected)?;

    let mut session_index = Json::Null;
    let mut session_expires_at = Json::Null;
    let mut authn_context = Json::Null;
    if let Some(statement) = assertion.first(SAML_NS, "AuthnStatement") {
        if let Some(index) = statement.attr("SessionIndex") {
            session_index = json!(index);
        }
        if let Some(limit) = optional_instant(&statement, "SessionNotOnOrAfter")? {
            if expected.now - expected.clock_skew >= limit {
                return Err("the identity provider's session has expired".to_string());
            }
            session_expires_at = json!(limit);
        }
        if let Some(class) = statement
            .first(SAML_NS, "AuthnContext")
            .and_then(|context| context.first(SAML_NS, "AuthnContextClassRef"))
        {
            authn_context = json!(class.text());
        }
    }

    let mut attributes = JsonMap::new();
    for statement in assertion.named(SAML_NS, "AttributeStatement") {
        for attribute in statement.named(SAML_NS, "Attribute") {
            let Some(name) = attribute.attr("Name") else {
                continue;
            };
            let values: Vec<Json> = attribute
                .named(SAML_NS, "AttributeValue")
                .iter()
                .map(|value| json!(value.text()))
                .collect();
            match attributes.get_mut(name) {
                Some(Json::Array(existing)) => existing.extend(values),
                _ => {
                    attributes.insert(name.to_string(), Json::Array(values));
                }
            }
        }
    }

    Ok(json!({
        "name_id": name,
        "name_id_format": name_id.attr("Format"),
        "issuer": issuer,
        "attributes": attributes,
        "session_index": session_index,
        "session_expires_at": session_expires_at,
        "authn_context": authn_context,
        "assertion_id": assertion.attr("ID"),
        "response_id": response.attr("ID"),
        "in_response_to": expected.request_id,
        "expires_at": expires_at,
        "signed": signed,
    }))
}

// ---------------------------------------------------------------------------
// Request and metadata
// ---------------------------------------------------------------------------

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn new_id() -> String {
    let mut bytes = [0u8; 20];
    OsRng.fill_bytes(&mut bytes);
    // An xs:ID cannot start with a digit.
    format!("_{}", super::hex::encode(&bytes))
}

fn instant(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|time| time.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

fn http_url(obj: &Json, key: &str) -> Result<String, String> {
    let url = req_str(obj, key)?;
    if url.starts_with("https://") || url.starts_with("http://") {
        Ok(url)
    } else {
        Err(format!("{} must be an http(s) URL", key))
    }
}

/// The request that sends a browser to the identity provider, as the URL of
/// the HTTP-Redirect binding.
pub(crate) fn auth_request(opts: &Json) -> Result<Json, String> {
    let sso_url = http_url(opts, "idp_sso_url")?;
    let acs_url = http_url(opts, "acs_url")?;
    let entity_id = req_str(opts, "sp_entity_id")?;
    let id = new_id();

    let mut xml = format!(
        "<samlp:AuthnRequest xmlns:samlp=\"{SAMLP_NS}\" xmlns:saml=\"{SAML_NS}\" ID=\"{id}\" \
         Version=\"2.0\" IssueInstant=\"{}\" Destination=\"{}\" ProtocolBinding=\"{BINDING_POST}\" \
         AssertionConsumerServiceURL=\"{}\"",
        instant(unix_now()),
        escape(&sso_url),
        escape(&acs_url),
    );
    if opt_bool(opts, "force_authn")? {
        xml.push_str(" ForceAuthn=\"true\"");
    }
    xml.push_str(&format!(
        "><saml:Issuer>{}</saml:Issuer>",
        escape(&entity_id)
    ));
    if let Some(format) = opt_str(opts, "name_id_format")? {
        xml.push_str(&format!(
            "<samlp:NameIDPolicy Format=\"{}\" AllowCreate=\"true\"/>",
            escape(&format)
        ));
    }
    xml.push_str("</samlp:AuthnRequest>");

    let deflated = super::deflate::do_deflate(xml.as_bytes())?;
    let mut url = sso_url.clone();
    url.push(if sso_url.contains('?') { '&' } else { '?' });
    url.push_str("SAMLRequest=");
    url.push_str(&urlencoding::encode(&STANDARD.encode(deflated)));
    if let Some(relay_state) = opt_str(opts, "relay_state")? {
        if relay_state.len() > 80 {
            return Err("relay_state is longer than the 80 bytes SAML allows".to_string());
        }
        url.push_str("&RelayState=");
        url.push_str(&urlencoding::encode(&relay_state));
    }
    Ok(json!({"id": id, "url": url, "xml": xml}))
}

/// The service provider's metadata, to hand to the identity provider.
pub(crate) fn metadata(opts: &Json) -> Result<Json, String> {
    let entity_id = req_str(opts, "sp_entity_id")?;
    let acs_url = http_url(opts, "acs_url")?;
    let format = opt_str(opts, "name_id_format")?.unwrap_or_else(|| NAMEID_EMAIL.to_string());
    Ok(Json::String(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <md:EntityDescriptor xmlns:md=\"{MD_NS}\" entityID=\"{}\">\
         <md:SPSSODescriptor AuthnRequestsSigned=\"false\" WantAssertionsSigned=\"true\" \
         protocolSupportEnumeration=\"{SAMLP_NS}\">\
         <md:NameIDFormat>{}</md:NameIDFormat>\
         <md:AssertionConsumerService Binding=\"{BINDING_POST}\" Location=\"{}\" index=\"0\" \
         isDefault=\"true\"/>\
         </md:SPSSODescriptor></md:EntityDescriptor>\n",
        escape(&entity_id),
        escape(&format),
        escape(&acs_url),
    )))
}

fn identity_providers<'a>(el: &Scoped<'a>, out: &mut Vec<(Scoped<'a>, Scoped<'a>)>) {
    if el.is(MD_NS, "EntityDescriptor") {
        if let Some(idp) = el.first(MD_NS, "IDPSSODescriptor") {
            out.push((el.clone(), idp));
        }
    } else if el.is(MD_NS, "EntitiesDescriptor") {
        for child in el.children() {
            identity_providers(&child, out);
        }
    }
}

/// What an identity provider's metadata says a service provider needs.
pub(crate) fn parse_metadata(xml: &str) -> Result<Json, String> {
    if xml.len() > MAX_RESPONSE_BYTES {
        return Err("the metadata is too large".to_string());
    }
    let document = parse_document(xml).map_err(|e| format!("the metadata is not XML: {}", e))?;
    let mut found = Vec::new();
    identity_providers(&Scoped::root(&document), &mut found);
    let (entity, idp) = match found.len() {
        1 => found.remove(0),
        0 => return Err("the metadata describes no identity provider".to_string()),
        n => {
            return Err(format!(
                "the metadata describes {} identity providers; pass one EntityDescriptor",
                n
            ))
        }
    };
    let entity_id = entity
        .attr("entityID")
        .ok_or("the EntityDescriptor has no entityID")?;

    let mut sso_url = None;
    let mut bindings = Vec::new();
    for service in idp.named(MD_NS, "SingleSignOnService") {
        let binding = service.attr("Binding").unwrap_or_default();
        bindings.push(binding.to_string());
        if binding == BINDING_REDIRECT && sso_url.is_none() {
            sso_url = service.attr("Location");
        }
    }
    let sso_url = sso_url.ok_or_else(|| {
        format!(
            "the identity provider offers no HTTP-Redirect single sign-on service (found: {})",
            bindings.join(", ")
        )
    })?;

    let mut certificates = Vec::new();
    for descriptor in idp.named(MD_NS, "KeyDescriptor") {
        if descriptor.attr("use") == Some("encryption") {
            continue;
        }
        for key_info in descriptor.named(DSIG_NS, "KeyInfo") {
            for data in key_info.named(DSIG_NS, "X509Data") {
                for certificate in data.named(DSIG_NS, "X509Certificate") {
                    let der = decode_base64(&certificate.text(), "X509Certificate")?;
                    super::x509::parse_certificate(&der)?;
                    certificates.push(json!(super::x509::der_to_pem(&der)));
                }
            }
        }
    }
    if certificates.is_empty() {
        return Err("the identity provider publishes no signing certificate".to_string());
    }
    Ok(json!({
        "entity_id": entity_id,
        "sso_url": sso_url,
        "certificates": certificates,
    }))
}

// ---------------------------------------------------------------------------
// A signed response for specs
// ---------------------------------------------------------------------------

/// RSA private key `(n, d)` from a PEM (PKCS#8 or PKCS#1).
fn private_key(pem: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (n, _, d) = super::rsa_key::parse_components(pem)?;
    let n: Vec<u8> = n.iter().skip_while(|b| **b == 0).copied().collect();
    Ok((n, d))
}

fn rsa_sha256_sign(message: &[u8], n: &[u8], d: &[u8]) -> Result<Vec<u8>, String> {
    let info = super::rsa_key::digest_info("sha256", message)?;
    let k = n.len();
    if k < info.len() + 11 {
        return Err("the private key is too short".to_string());
    }
    let mut em = vec![0x00u8, 0x01];
    em.extend(std::iter::repeat_n(0xffu8, k - info.len() - 3));
    em.push(0x00);
    em.extend_from_slice(&info);
    let signature = super::crypto::do_modexp(&em, d, n)?;
    let mut padded = vec![0u8; k.saturating_sub(signature.len())];
    padded.extend_from_slice(&signature);
    Ok(padded)
}

fn find_with_id<'a>(el: Scoped<'a>, id: &str) -> Option<Scoped<'a>> {
    if el.attr("ID") == Some(id) {
        return Some(el);
    }
    el.children()
        .into_iter()
        .find_map(|child| find_with_id(child, id))
}

/// `<ds:Signature>` over the element of `xml` whose ID is `id`.
fn signature_for(xml: &str, id: &str, n: &[u8], d: &[u8]) -> Result<String, String> {
    let document = parse_document(xml)?;
    let target = find_with_id(Scoped::root(&document), id)
        .ok_or_else(|| format!("no element with ID {}", id))?;
    let digest_value = STANDARD.encode(Sha256::digest(
        target.canonical(&BTreeSet::new(), None).as_bytes(),
    ));
    let signed_info = format!(
        "<ds:SignedInfo><ds:CanonicalizationMethod Algorithm=\"{EXC_C14N}\"/>\
         <ds:SignatureMethod Algorithm=\"http://www.w3.org/2001/04/xmldsig-more#rsa-sha256\"/>\
         <ds:Reference URI=\"#{id}\"><ds:Transforms><ds:Transform Algorithm=\"{ENVELOPED}\"/>\
         <ds:Transform Algorithm=\"{EXC_C14N}\"/></ds:Transforms>\
         <ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"/>\
         <ds:DigestValue>{digest_value}</ds:DigestValue></ds:Reference></ds:SignedInfo>"
    );
    let wrapper = format!("<ds:Signature xmlns:ds=\"{DSIG_NS}\">{signed_info}</ds:Signature>");
    let parsed = parse_document(&wrapper)?;
    let canonical = Scoped::root(&parsed)
        .only(DSIG_NS, "SignedInfo")?
        .canonical(&BTreeSet::new(), None);
    let value = STANDARD.encode(rsa_sha256_sign(canonical.as_bytes(), n, d)?);
    Ok(format!(
        "<ds:Signature xmlns:ds=\"{DSIG_NS}\">{signed_info}\
         <ds:SignatureValue>{value}</ds:SignatureValue></ds:Signature>"
    ))
}

const ASSERTION_MARK: &str = "<!--assertion-signature-->";
const RESPONSE_MARK: &str = "<!--response-signature-->";

/// A signed `SAMLResponse`, as an identity provider would post it: the
/// stand-in a spec signs in with. Not an identity provider — it signs
/// whatever it is told with a key the caller supplies.
pub(crate) fn test_response(opts: &Json) -> Result<Json, String> {
    let (n, d) = private_key(&req_str(opts, "private_key")?)?;
    let issuer = req_str(opts, "issuer")?;
    let acs_url = req_str(opts, "acs_url")?;
    let audience = req_str(opts, "audience")?;
    let name_id = req_str(opts, "name_id")?;
    let format = opt_str(opts, "name_id_format")?.unwrap_or_else(|| NAMEID_EMAIL.to_string());
    let request_id = opt_str(opts, "request_id")?;
    let now = opt_int(opts, "now")?.unwrap_or_else(unix_now);
    let lifetime = opt_int(opts, "lifetime")?.unwrap_or(300);
    let sign = opt_str(opts, "sign")?.unwrap_or_else(|| "assertion".to_string());
    if !["assertion", "response", "both", "none"].contains(&sign.as_str()) {
        return Err("sign must be assertion, response, both or none".to_string());
    }
    let response_id = opt_str(opts, "response_id")?.unwrap_or_else(new_id);
    let assertion_id = opt_str(opts, "assertion_id")?.unwrap_or_else(new_id);
    let in_response_to = request_id
        .as_ref()
        .map(|id| format!(" InResponseTo=\"{}\"", escape(id)))
        .unwrap_or_default();
    let issued = instant(now);
    let expires = instant(now + lifetime);

    let mut attributes = String::new();
    if let Some(Json::Object(map)) = opts.get("attributes") {
        for (name, value) in map {
            let values: Vec<String> = match value {
                Json::Array(items) => items
                    .iter()
                    .map(|item| item.as_str().map(str::to_string))
                    .collect::<Option<_>>()
                    .ok_or("attributes must hold Strings")?,
                Json::String(text) => vec![text.clone()],
                _ => return Err("attributes must hold Strings".to_string()),
            };
            attributes.push_str(&format!("<saml:Attribute Name=\"{}\">", escape(name)));
            for value in values {
                attributes.push_str(&format!(
                    "<saml:AttributeValue>{}</saml:AttributeValue>",
                    escape(&value)
                ));
            }
            attributes.push_str("</saml:Attribute>");
        }
    }
    if !attributes.is_empty() {
        attributes = format!("<saml:AttributeStatement>{attributes}</saml:AttributeStatement>");
    }
    let session_index = opt_str(opts, "session_index")?.unwrap_or_else(new_id);

    let mut xml = format!(
        "<samlp:Response xmlns:samlp=\"{SAMLP_NS}\" xmlns:saml=\"{SAML_NS}\" ID=\"{}\" \
         Version=\"2.0\" IssueInstant=\"{issued}\" Destination=\"{}\"{in_response_to}>\
         <saml:Issuer>{}</saml:Issuer>{RESPONSE_MARK}\
         <samlp:Status><samlp:StatusCode Value=\"{}\"/></samlp:Status>\
         <saml:Assertion ID=\"{}\" Version=\"2.0\" IssueInstant=\"{issued}\">\
         <saml:Issuer>{}</saml:Issuer>{ASSERTION_MARK}\
         <saml:Subject><saml:NameID Format=\"{}\">{}</saml:NameID>\
         <saml:SubjectConfirmation Method=\"{BEARER}\">\
         <saml:SubjectConfirmationData NotOnOrAfter=\"{expires}\" Recipient=\"{}\"{in_response_to}/>\
         </saml:SubjectConfirmation></saml:Subject>\
         <saml:Conditions NotBefore=\"{issued}\" NotOnOrAfter=\"{expires}\">\
         <saml:AudienceRestriction><saml:Audience>{}</saml:Audience></saml:AudienceRestriction>\
         </saml:Conditions>\
         <saml:AuthnStatement AuthnInstant=\"{issued}\" SessionIndex=\"{}\">\
         <saml:AuthnContext><saml:AuthnContextClassRef>\
         urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport\
         </saml:AuthnContextClassRef></saml:AuthnContext></saml:AuthnStatement>\
         {attributes}</saml:Assertion></samlp:Response>",
        escape(&response_id),
        escape(&acs_url),
        escape(&issuer),
        escape(&opt_str(opts, "status")?.unwrap_or_else(|| STATUS_SUCCESS.to_string())),
        escape(&assertion_id),
        escape(&issuer),
        escape(&format),
        escape(&name_id),
        escape(&acs_url),
        escape(&audience),
        escape(&session_index),
    );

    // The assertion first: the response's digest covers the assertion's signature.
    if sign == "assertion" || sign == "both" {
        let signature = signature_for(&xml, &assertion_id, &n, &d)?;
        xml = xml.replacen(ASSERTION_MARK, &signature, 1);
    }
    if sign == "response" || sign == "both" {
        let signature = signature_for(&xml, &response_id, &n, &d)?;
        xml = xml.replacen(RESPONSE_MARK, &signature, 1);
    }
    xml = xml.replace(ASSERTION_MARK, "").replace(RESPONSE_MARK, "");
    Ok(Json::String(STANDARD.encode(xml)))
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

fn hash_arg(args: &[Value], index: usize, func: &str, what: &str) -> Result<Json, String> {
    match args.get(index) {
        Some(value @ Value::Hash(_)) => {
            value_to_json(value).map_err(|e| format!("{}(): {}", func, e))
        }
        Some(other) => Err(format!(
            "{}(): {} must be a Hash, got {}",
            func,
            what,
            other.type_name()
        )),
        None => Err(format!("{}(): {} is required", func, what)),
    }
}

fn string_arg(args: &[Value], index: usize, func: &str, what: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(text)) => Ok(text.to_string()),
        Some(other) => Err(format!(
            "{}(): {} must be a String, got {}",
            func,
            what,
            other.type_name()
        )),
        None => Err(format!("{}(): {} is required", func, what)),
    }
}

fn native(
    name: &'static str,
    arity: usize,
    body: fn(&[Value], &'static str) -> Result<Json, String>,
) -> Rc<NativeFunction> {
    Rc::new(NativeFunction::new(name, Some(arity), move |args| {
        let result = body(args, name).map_err(|e| {
            if e.starts_with(name) {
                e
            } else {
                format!("{}(): {}", name, e)
            }
        })?;
        json_to_value(result)
    }))
}

pub fn register_saml_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    statics.insert(
        "auth_request".to_string(),
        native("Saml.auth_request", 1, |args, func| {
            auth_request(&hash_arg(args, 0, func, "options")?)
        }),
    );
    statics.insert(
        "verify_response".to_string(),
        native("Saml.verify_response", 2, |args, func| {
            let response = string_arg(args, 0, func, "the SAMLResponse")?;
            let expected = Expected::from_json(&hash_arg(args, 1, func, "expected")?)?;
            verify_response(&response, &expected)
        }),
    );
    statics.insert(
        "metadata".to_string(),
        native("Saml.metadata", 1, |args, func| {
            metadata(&hash_arg(args, 0, func, "options")?)
        }),
    );
    statics.insert(
        "parse_metadata".to_string(),
        native("Saml.parse_metadata", 1, |args, func| {
            parse_metadata(&string_arg(args, 0, func, "the metadata")?)
        }),
    );
    statics.insert(
        "test_response".to_string(),
        native("Saml.test_response", 1, |args, func| {
            test_response(&hash_arg(args, 0, func, "options")?)
        }),
    );

    let class = Class {
        name: "Saml".to_string(),
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
    env.define("Saml".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDP_KEY: &str = include_str!("../../../tests/fixtures/saml/idp.key");
    const IDP_CERT: &str = include_str!("../../../tests/fixtures/saml/idp.crt");
    const OTHER_KEY: &str = include_str!("../../../tests/fixtures/saml/other.key");
    const OTHER_CERT: &str = include_str!("../../../tests/fixtures/saml/other.crt");

    const ACS: &str = "https://app.test/saml/acs";
    const SP: &str = "https://app.test/saml/metadata";
    const IDP: &str = "https://idp.test/metadata";
    const NOW: i64 = 1_800_000_000;

    /// What the identity provider is told to sign, with overrides.
    fn issue(overrides: Json) -> String {
        let mut opts = json!({
            "private_key": IDP_KEY, "issuer": IDP, "acs_url": ACS, "audience": SP,
            "name_id": "ada@example.com", "request_id": "_req1", "now": NOW,
            "response_id": "_r1", "assertion_id": "_a1",
            "attributes": {"email": "ada@example.com", "groups": ["admin", "staff"]},
        });
        for (key, value) in overrides.as_object().unwrap() {
            opts[key] = value.clone();
        }
        test_response(&opts).unwrap().as_str().unwrap().to_string()
    }

    fn expected(overrides: Json) -> Result<Expected, String> {
        let mut opts = json!({
            "certificate": IDP_CERT, "audience": SP, "acs_url": ACS,
            "request_id": "_req1", "idp_entity_id": IDP, "now": NOW + 10,
        });
        for (key, value) in overrides.as_object().unwrap() {
            opts[key] = value.clone();
        }
        Expected::from_json(&opts)
    }

    fn verify(response: &str) -> Result<Json, String> {
        verify_response(response, &expected(json!({}))?)
    }

    /// Rewrite the XML of a response after it was signed.
    fn tamper(response: &str, edit: impl Fn(String) -> String) -> String {
        let xml = String::from_utf8(STANDARD.decode(response).unwrap()).unwrap();
        STANDARD.encode(edit(xml))
    }

    fn refused(result: Result<Json, String>, why: &str) {
        match result {
            Ok(value) => panic!("accepted, expected '{why}': {value}"),
            Err(error) => assert!(error.contains(why), "expected '{why}', got: {error}"),
        }
    }

    #[test]
    fn a_response_signed_on_the_assertion_the_response_or_both_is_accepted() {
        for sign in ["assertion", "response", "both"] {
            let verified = verify(&issue(json!({"sign": sign}))).unwrap();
            assert_eq!(verified["name_id"], json!("ada@example.com"), "{sign}");
            assert_eq!(verified["issuer"], json!(IDP));
            assert_eq!(verified["signed"], json!(sign));
            assert_eq!(verified["attributes"]["groups"], json!(["admin", "staff"]));
            assert_eq!(verified["attributes"]["email"], json!(["ada@example.com"]));
            assert_eq!(verified["assertion_id"], json!("_a1"));
            assert_eq!(verified["in_response_to"], json!("_req1"));
            assert_eq!(verified["expires_at"], json!(NOW + 300));
            assert!(verified["session_index"].is_string());
        }
    }

    #[test]
    fn an_unsigned_or_foreign_signature_is_refused() {
        refused(verify(&issue(json!({"sign": "none"}))), "is signed");
        for sign in ["assertion", "response", "both"] {
            let forged = issue(json!({"sign": sign, "private_key": OTHER_KEY}));
            refused(verify(&forged), "does not verify");
        }
        // Trusting the other certificate as well accepts it: any listed key.
        let forged = issue(json!({"private_key": OTHER_KEY}));
        let both = expected(json!({"certificates": [IDP_CERT, OTHER_CERT]})).unwrap();
        assert!(verify_response(&forged, &both).is_ok());
        // The certificate a message carries is never the one that is trusted.
        let with_key_info = tamper(&forged, |xml| {
            xml.replacen(
                "</ds:SignatureValue>",
                &format!(
                    "</ds:SignatureValue><ds:KeyInfo><ds:X509Data><ds:X509Certificate>{}\
                     </ds:X509Certificate></ds:X509Data></ds:KeyInfo>",
                    OTHER_CERT
                        .lines()
                        .filter(|l| !l.contains("-----"))
                        .collect::<String>()
                ),
                1,
            )
        });
        refused(verify(&with_key_info), "does not verify");
    }

    #[test]
    fn anything_changed_after_signing_is_refused() {
        for sign in ["assertion", "response", "both"] {
            let edited = tamper(&issue(json!({"sign": sign})), |xml| {
                xml.replace("ada@example.com", "eve@example.com")
            });
            refused(verify(&edited), "modified after it was signed");
        }
        // With only the assertion signed, the response around it is not
        // covered — and its fields are still held to what was expected.
        let readdressed = tamper(&issue(json!({})), |xml| {
            xml.replacen(
                "Destination=\"https://app.test/saml/acs\"",
                "Destination=\"https://evil.test/acs\"",
                1,
            )
        });
        refused(verify(&readdressed), "another destination");
    }

    #[test]
    fn a_comment_does_not_split_what_was_signed() {
        let response = issue(json!({"name_id": "ada@example.com.evil.test"}));
        let split = tamper(&response, |xml| {
            xml.replace(
                "ada@example.com.evil.test</saml:NameID>",
                "ada@example.com<!---->.evil.test</saml:NameID>",
            )
        });
        assert_eq!(
            verify(&split).unwrap()["name_id"],
            json!("ada@example.com.evil.test")
        );
    }

    /// The signed assertion of a response, signature included.
    fn signed_assertion(xml: &str) -> String {
        let start = xml.find("<saml:Assertion").unwrap();
        let end = xml.find("</saml:Assertion>").unwrap() + "</saml:Assertion>".len();
        xml[start..end].to_string()
    }

    fn signature_of(assertion: &str) -> String {
        let start = assertion.find("<ds:Signature").unwrap();
        let end = assertion.find("</ds:Signature>").unwrap() + "</ds:Signature>".len();
        assertion[start..end].to_string()
    }

    /// An assertion the attacker wrote, naming someone else.
    fn evil(assertion: &str, id: &str, signature: &str) -> String {
        assertion
            .replace(&signature_of(assertion), signature)
            .replace("ID=\"_a1\"", &format!("ID=\"{id}\""))
            .replace("ada@example.com", "admin@example.com")
    }

    #[test]
    fn signature_wrapping_is_refused() {
        let response = issue(json!({}));
        let xml = String::from_utf8(STANDARD.decode(&response).unwrap()).unwrap();
        let original = signed_assertion(&xml);
        let signature = signature_of(&original);
        let encode = |xml: String| STANDARD.encode(xml);

        // An unsigned assertion placed before the signed one.
        let attack = xml.replacen(
            &original,
            &format!("{}{original}", evil(&original, "_evil", "")),
            1,
        );
        refused(verify(&encode(attack)), "2 <Assertion>");

        // The signed assertion hidden in an extension, an unsigned one in its place.
        let hidden = format!("<samlp:Extensions>{original}</samlp:Extensions>");
        let attack = xml.replacen(
            &original,
            &format!("{hidden}{}", evil(&original, "_evil", "")),
            1,
        );
        refused(verify(&encode(attack)), "is signed");

        // The same, the forged assertion carrying the real signature: it
        // verifies over the hidden assertion, which is not its parent.
        let attack = xml.replacen(
            &original,
            &format!("{hidden}{}", evil(&original, "_evil", &signature)),
            1,
        );
        refused(verify(&encode(attack)), "does not reference");

        // The forged assertion reusing the signed one's id.
        let attack = xml.replacen(
            &original,
            &format!("{hidden}{}", evil(&original, "_a1", &signature)),
            1,
        );
        refused(verify(&encode(attack)), "more than once");

        // The signed assertion moved inside the forged one.
        let nested = evil(&original, "_evil", "").replacen(
            "</saml:Assertion>",
            &format!("{original}</saml:Assertion>"),
            1,
        );
        refused(
            verify(&encode(xml.replacen(&original, &nested, 1))),
            "is signed",
        );

        // The signed assertion parked inside the signature of the forged one.
        // (Built around a placeholder, so that the parked copy keeps its id.)
        let parked = signature.replacen(
            "</ds:Signature>",
            &format!("<ds:Object>{original}</ds:Object></ds:Signature>"),
            1,
        );
        let forged = evil(&original, "_evil", "<parked/>").replacen("<parked/>", &parked, 1);
        let attack = xml.replacen(&original, &forged, 1);
        refused(verify(&encode(attack)), "does not reference");
    }

    #[test]
    fn the_response_must_be_for_this_application_and_this_request() {
        refused(
            verify(&issue(json!({"audience": "https://other.test"}))),
            "another audience",
        );
        refused(
            verify(&issue(json!({"acs_url": "https://other.test/acs"}))),
            "another destination",
        );
        refused(
            verify(&issue(json!({"issuer": "https://rogue.test"}))),
            "another identity provider",
        );
        refused(
            verify(&issue(json!({"request_id": "_other"}))),
            "another request",
        );
        // A response nobody asked for: refused, unless the caller allows it —
        // and then one that answers a request is the one refused.
        refused(
            verify(&issue(json!({"request_id": null}))),
            "another request",
        );
        let unsolicited = expected(json!({"request_id": null, "allow_unsolicited": true})).unwrap();
        assert!(verify_response(&issue(json!({"request_id": null})), &unsolicited).is_ok());
        refused(
            verify_response(&issue(json!({})), &unsolicited),
            "none was expected",
        );
        assert!(expected(json!({"request_id": null}))
            .err()
            .unwrap()
            .contains("request_id is required"));
    }

    #[test]
    fn the_response_must_be_current_and_successful() {
        let late = expected(json!({"now": NOW + 300 + 120})).unwrap();
        refused(verify_response(&issue(json!({})), &late), "expired");
        // Inside the clock skew on both sides.
        let nearly = expected(json!({"now": NOW + 300 + 119})).unwrap();
        assert!(verify_response(&issue(json!({})), &nearly).is_ok());
        let early = expected(json!({"now": NOW - 121})).unwrap();
        refused(verify_response(&issue(json!({})), &early), "not valid yet");
        let strict = expected(json!({"now": NOW + 300, "clock_skew": 0})).unwrap();
        refused(verify_response(&issue(json!({})), &strict), "expired");

        let denied = issue(json!({"status": "urn:oasis:names:tc:SAML:2.0:status:Responder"}));
        refused(verify(&denied), "status:Responder");
    }

    #[test]
    fn what_is_not_supported_is_refused_by_name() {
        let response = issue(json!({"sign": "response"}));
        let encrypted = tamper(&response, |xml| {
            xml.replacen(
                "<saml:Assertion ",
                "<saml:EncryptedAssertion/><saml:Assertion ",
                1,
            )
        });
        refused(verify(&encrypted), "encrypted assertions are not supported");
        refused(verify("not base64 !"), "not valid base64");
        refused(verify(&STANDARD.encode("<a/>")), "not a SAML Response");
        refused(
            verify(&STANDARD.encode("<!DOCTYPE a [<!ENTITY x \"y\">]><a>&x;</a>")),
            "not XML",
        );
        assert!(hash_of(
            "http://www.w3.org/2000/09/xmldsig#rsa-sha1",
            "signature",
            false
        )
        .unwrap_err()
        .contains("SHA-1"));
        assert_eq!(
            hash_of(
                "http://www.w3.org/2000/09/xmldsig#rsa-sha1",
                "signature",
                true
            ),
            Ok("sha1")
        );
        assert!(hash_of(
            "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256",
            "signature",
            false
        )
        .unwrap_err()
        .contains("unsupported"));
    }

    #[test]
    fn the_request_is_a_deflated_redirect_to_the_identity_provider() {
        let request = auth_request(&json!({
            "idp_sso_url": "https://idp.test/sso?tenant=1", "acs_url": ACS,
            "sp_entity_id": SP, "relay_state": "/reports?x=1&y=2", "force_authn": true,
        }))
        .unwrap();
        let url = request["url"].as_str().unwrap();
        assert!(
            url.starts_with("https://idp.test/sso?tenant=1&SAMLRequest="),
            "{url}"
        );
        assert!(
            url.ends_with("&RelayState=%2Freports%3Fx%3D1%26y%3D2"),
            "{url}"
        );
        let encoded = url
            .split("SAMLRequest=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        let deflated = STANDARD
            .decode(urlencoding::decode(encoded).unwrap().as_bytes())
            .unwrap();
        let xml = String::from_utf8(
            crate::interpreter::builtins::deflate::do_inflate(&deflated).unwrap(),
        )
        .unwrap();
        assert_eq!(request["xml"], json!(xml));
        let id = request["id"].as_str().unwrap();
        assert!(id.starts_with('_') && id.len() == 41, "{id}");
        assert!(xml.contains(&format!("ID=\"{id}\"")), "{xml}");
        assert!(xml.contains("ForceAuthn=\"true\""), "{xml}");
        assert!(
            xml.contains(&format!("<saml:Issuer>{SP}</saml:Issuer>")),
            "{xml}"
        );
        assert!(
            xml.contains(&format!("AssertionConsumerServiceURL=\"{ACS}\"")),
            "{xml}"
        );
        parse_document(&xml).unwrap();

        let other = auth_request(&json!({
            "idp_sso_url": "https://idp.test/sso", "acs_url": ACS, "sp_entity_id": SP,
        }))
        .unwrap();
        assert_ne!(other["id"], request["id"]);
        assert!(auth_request(
            &json!({"idp_sso_url": "javascript:x", "acs_url": ACS, "sp_entity_id": SP})
        )
        .unwrap_err()
        .contains("http(s) URL"));
    }

    #[test]
    fn metadata_is_written_and_read() {
        let ours = metadata(&json!({"sp_entity_id": SP, "acs_url": ACS})).unwrap();
        let ours = ours.as_str().unwrap();
        assert!(ours.contains(&format!("entityID=\"{SP}\"")), "{ours}");
        assert!(ours.contains(&format!("Location=\"{ACS}\"")), "{ours}");
        parse_document(ours).unwrap();

        let certificate: String = IDP_CERT.lines().filter(|l| !l.contains("-----")).collect();
        let theirs = format!(
            "<md:EntityDescriptor xmlns:md=\"{MD_NS}\" xmlns:ds=\"{DSIG_NS}\" entityID=\"{IDP}\">\
             <md:IDPSSODescriptor protocolSupportEnumeration=\"{SAMLP_NS}\">\
             <md:KeyDescriptor use=\"encryption\"><ds:KeyInfo><ds:X509Data>\
             <ds:X509Certificate>AAAA</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor>\
             <md:KeyDescriptor use=\"signing\"><ds:KeyInfo><ds:X509Data>\
             <ds:X509Certificate>{certificate}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor>\
             <md:SingleSignOnService Binding=\"{BINDING_POST}\" Location=\"https://idp.test/post\"/>\
             <md:SingleSignOnService Binding=\"{BINDING_REDIRECT}\" Location=\"https://idp.test/sso\"/>\
             </md:IDPSSODescriptor></md:EntityDescriptor>"
        );
        let parsed = parse_metadata(&theirs).unwrap();
        assert_eq!(parsed["entity_id"], json!(IDP));
        assert_eq!(parsed["sso_url"], json!("https://idp.test/sso"));
        assert_eq!(parsed["certificates"].as_array().unwrap().len(), 1);
        // The certificate it returns is the one verify_response accepts.
        let from_metadata = expected(json!({"certificate": parsed["certificates"][0]})).unwrap();
        assert!(verify_response(&issue(json!({})), &from_metadata).is_ok());

        let wrapped =
            format!("<md:EntitiesDescriptor xmlns:md=\"{MD_NS}\">{theirs}</md:EntitiesDescriptor>");
        assert_eq!(parse_metadata(&wrapped).unwrap()["entity_id"], json!(IDP));
        assert!(
            parse_metadata(&ours.replace("<?xml version=\"1.0\" encoding=\"UTF-8\"?>", ""))
                .unwrap_err()
                .contains("no identity provider")
        );
    }
}
