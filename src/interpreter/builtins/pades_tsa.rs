//! RFC 3161 trusted timestamping — the "-T" in PAdES-B-T.
//!
//! After the signer's signature is computed, we hash it, ask a Time-Stamp
//! Authority (TSA) to timestamp that hash, and embed the returned token as the
//! `id-aa-timeStampToken` **unsigned** attribute on the `SignerInfo`. Because it
//! is unsigned, adding it does not disturb the already-computed signature — it
//! just proves the signature existed at the TSA's asserted time.
//!
//! Kept separate from [`super::pades`] so the CMS-assembly module stays free of
//! HTTP; the only network call in the signing path lives here.

use der::asn1::{Null, OctetString};
use der::{Any, Encode, Sequence};
use x509_cert::spki::AlgorithmIdentifierOwned;

use super::pades::OID_SHA256;

/// `id-aa-timeStampToken` (RFC 3161 §3.3.2) — the unsigned SignerInfo attribute
/// that carries the timestamp token.
pub const OID_TIMESTAMP_TOKEN: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");

/// `MessageImprint ::= SEQUENCE { hashAlgorithm AlgorithmIdentifier, hashedMessage OCTET STRING }`.
#[derive(Sequence)]
struct MessageImprint {
    hash_algorithm: AlgorithmIdentifierOwned,
    hashed_message: OctetString,
}

/// `TimeStampReq` (RFC 3161 §2.4.1). We send version 1, a SHA-256 imprint, and
/// `certReq = TRUE` so the TSA embeds its own certificate in the token (needed
/// for later verification). Policy/nonce/extensions are omitted (all OPTIONAL).
#[derive(Sequence)]
struct TimeStampReq {
    version: u8,
    message_imprint: MessageImprint,
    cert_req: bool,
}

/// Build the DER `TimeStampReq` for a SHA-256 `imprint` (the hash of the
/// signature value being timestamped).
fn build_request(imprint: &[u8]) -> Result<Vec<u8>, String> {
    let req = TimeStampReq {
        version: 1,
        message_imprint: MessageImprint {
            // SHA-256 with explicit NULL parameters — the encoding TSA servers
            // most reliably accept for the message imprint.
            hash_algorithm: AlgorithmIdentifierOwned {
                oid: OID_SHA256,
                parameters: Some(Any::encode_from(&Null).map_err(|e| format!("tsa: null: {e}"))?),
            },
            hashed_message: OctetString::new(imprint.to_vec())
                .map_err(|e| format!("tsa: imprint: {e}"))?,
        },
        cert_req: true,
    };
    req.to_der().map_err(|e| format!("tsa: encode request: {e}"))
}

/// Request a timestamp token for `imprint` from the TSA at `url`. Returns the
/// DER of the `timeStampToken` (a CMS `ContentInfo`), ready to drop into an
/// unsigned attribute.
pub fn fetch_timestamp_token(url: &str, imprint: &[u8]) -> Result<Vec<u8>, String> {
    let body = build_request(imprint)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("tsa: HTTP client: {e}"))?;
    let resp = client
        .post(url)
        .header("Content-Type", "application/timestamp-query")
        .header("Accept", "application/timestamp-reply")
        .body(body)
        .send()
        .map_err(|e| format!("tsa: request to {url} failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("tsa: {url} returned HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .map_err(|e| format!("tsa: reading response: {e}"))?;
    extract_token(&bytes)
}

/// Read one DER length starting at `i`, returning `(length, header_end_index)`.
fn read_len(b: &[u8], mut i: usize) -> Result<(usize, usize), String> {
    let first = *b.get(i).ok_or("tsa: truncated length")?;
    i += 1;
    if first < 0x80 {
        Ok((first as usize, i))
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return Err("tsa: unsupported length form".into());
        }
        let mut len = 0usize;
        for _ in 0..n {
            len = (len << 8) | *b.get(i).ok_or("tsa: truncated length")? as usize;
            i += 1;
        }
        Ok((len, i))
    }
}

/// Read one TLV at `i`, returning `(tag, content_start, content_len, total_end)`.
fn read_tlv(b: &[u8], i: usize) -> Result<(u8, usize, usize, usize), String> {
    let tag = *b.get(i).ok_or("tsa: truncated element")?;
    let (len, hdr_end) = read_len(b, i + 1)?;
    let end = hdr_end + len;
    if end > b.len() {
        return Err("tsa: element overruns response".into());
    }
    Ok((tag, hdr_end, len, end))
}

/// Parse a `TimeStampResp`, check the PKIStatus, and return the raw DER of the
/// `timeStampToken` ContentInfo.
///
/// ```text
/// TimeStampResp ::= SEQUENCE { status PKIStatusInfo, timeStampToken ContentInfo OPTIONAL }
/// PKIStatusInfo ::= SEQUENCE { status INTEGER, statusString .. OPTIONAL, failInfo .. OPTIONAL }
/// ```
fn extract_token(resp: &[u8]) -> Result<Vec<u8>, String> {
    let (tag, outer_start, _, _) = read_tlv(resp, 0)?;
    if tag != 0x30 {
        return Err("tsa: response is not a SEQUENCE".into());
    }
    // PKIStatusInfo
    let (t_status, status_start, _, status_end) = read_tlv(resp, outer_start)?;
    if t_status != 0x30 {
        return Err("tsa: missing PKIStatusInfo".into());
    }
    // status INTEGER (first field of PKIStatusInfo)
    let (t_int, int_start, int_len, _) = read_tlv(resp, status_start)?;
    if t_int != 0x02 {
        return Err("tsa: missing status integer".into());
    }
    // 0 = granted, 1 = grantedWithMods; anything else is a rejection.
    let status = *resp.get(int_start + int_len - 1).unwrap_or(&0xFF);
    if int_len != 1 || (status != 0 && status != 1) {
        return Err(format!("tsa: request rejected (PKIStatus {status})"));
    }
    // timeStampToken = the element right after PKIStatusInfo.
    let (t_token, _, _, token_end) = read_tlv(resp, status_end)?;
    if t_token != 0x30 {
        return Err("tsa: response contained no timeStampToken".into());
    }
    Ok(resp[status_end..token_end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_well_formed_der() {
        let der = build_request(&[0x11u8; 32]).unwrap();
        // Parses back as a SEQUENCE that contains our imprint.
        let (tag, _, _, end) = read_tlv(&der, 0).unwrap();
        assert_eq!(tag, 0x30);
        assert_eq!(end, der.len(), "no trailing bytes");
        assert!(
            der.windows(32).any(|w| w == [0x11u8; 32]),
            "imprint present in the request"
        );
    }

    #[test]
    fn rejects_a_denied_status() {
        // TimeStampResp { status = 2 (rejection) }, no token.
        // SEQUENCE { SEQUENCE { INTEGER 2 } }
        let resp = [0x30, 0x05, 0x30, 0x03, 0x02, 0x01, 0x02];
        assert!(extract_token(&resp).unwrap_err().contains("rejected"));
    }

    #[test]
    fn extracts_the_token_after_status() {
        // SEQUENCE {
        //   SEQUENCE { INTEGER 0 },              -- granted
        //   SEQUENCE { INTEGER 42 }              -- stand-in "timeStampToken"
        // }
        let resp = [
            0x30, 0x0a, 0x30, 0x03, 0x02, 0x01, 0x00, 0x30, 0x03, 0x02, 0x01, 0x2a,
        ];
        let token = extract_token(&resp).unwrap();
        assert_eq!(token, [0x30, 0x03, 0x02, 0x01, 0x2a]);
    }
}
