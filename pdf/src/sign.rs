//! PAdES digital signatures — the *byte-mechanics* half (no crypto lives here).
//!
//! Signing a PDF means: reserve a signature dictionary whose `/Contents` is a
//! zeroed placeholder and whose `/ByteRange` spans the whole file *except* that
//! placeholder; digest the ByteRange; build a detached CMS SignedData over that
//! digest; splice the CMS into the placeholder. This module owns steps 1 and 4;
//! the host crate (`solilang`) owns the crypto (steps 2-3) so this crate stays
//! free of asymmetric-crypto/ASN.1 dependencies — the same split every other
//! post-pass here follows (`attachments`, `encrypt`, `facturx`).
//!
//! ```text
//!   let prepared = sign::prepare_signature(pdf, &meta, DEFAULT_PLACEHOLDER_LEN)?;
//!   let digest   = sha256(prepared.signed_bytes());       // host
//!   let cms_der  = pades::build_cms(&digest, cert, key, …) // host
//!   let signed   = sign::embed_cms(prepared, &cms_der)?;
//! ```
//!
//! **Single signature only.** We fully re-serialize once (via lopdf, like every
//! other pass), then do only *length-preserving* in-place byte patches — the
//! `/ByteRange` integers (space-padded) and the `/Contents` hex (zero-padded) —
//! so no byte offset ever shifts. A second signature would need an incremental
//! (append-only) update; that's out of scope.

use lopdf::{Dictionary, Document, Object, StringFormat};

use crate::error::{PdfError, Result};

/// Default reserved size in bytes for the CMS blob (the hex placeholder is
/// twice this). 16 KiB comfortably holds a signer certificate plus a couple of
/// intermediates; oversizing only wastes zero-padding, never a real problem.
pub const DEFAULT_PLACEHOLDER_LEN: usize = 16384;

/// Wide sentinel written into each `/ByteRange` slot before we know the real
/// offsets: ten digits, so it out-sizes any offset in a file up to ~9.9 GB and
/// the real value always fits when we space-pad the patch.
const BYTE_RANGE_SENTINEL: i64 = 9_999_999_999;

/// Human-facing signature metadata written into the signature dictionary. All
/// fields are optional; whatever is set shows in a reader's signature panel.
#[derive(Debug, Clone, Default)]
pub struct SignMeta {
    pub reason: Option<String>,
    pub location: Option<String>,
    pub name: Option<String>,
    pub contact: Option<String>,
    /// PDF date string for `/M`, e.g. `"D:20260703120000+00'00'"`. The
    /// authoritative signing time also goes into the CMS signed attributes.
    pub signing_time: Option<String>,
}

/// A serialized PDF carrying a reserved, ByteRange-finalized signature slot,
/// ready for the CMS DER to be spliced into its `/Contents` placeholder.
pub struct PreparedSignature {
    /// PDF bytes with a zeroed `/Contents` and a concrete `/ByteRange`.
    pub pdf: Vec<u8>,
    /// The four `/ByteRange` integers `[start1, len1, start2, len2]`.
    pub byte_range: [usize; 4],
    /// Byte offset of the first hex digit inside the `<…>` `/Contents` string.
    pub contents_hex_offset: usize,
    /// Number of hex digits reserved (`placeholder_len * 2`).
    pub contents_hex_len: usize,
}

impl PreparedSignature {
    /// The exact bytes covered by the signature: the two `/ByteRange` spans
    /// concatenated (everything but the `/Contents` hex). Digest *this*.
    pub fn signed_bytes(&self) -> Vec<u8> {
        let [s1, l1, s2, l2] = self.byte_range;
        let mut out = Vec::with_capacity(l1 + l2);
        out.extend_from_slice(&self.pdf[s1..s1 + l1]);
        out.extend_from_slice(&self.pdf[s2..s2 + l2]);
        out
    }
}

fn lit(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

/// Reserve a signature dictionary in `pdf` and finalize its `/ByteRange`.
///
/// Adds an (invisible) signature field + widget on the first page, an
/// `/AcroForm` with `/SigFlags 3`, and a `/Sig` value dictionary whose
/// `/Contents` is `placeholder_len` zero bytes. Returns the re-serialized bytes
/// with the `/ByteRange` patched to the real offsets, plus the locations the
/// host needs to digest and later splice.
pub fn prepare_signature(
    pdf: &[u8],
    meta: &SignMeta,
    placeholder_len: usize,
) -> Result<PreparedSignature> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("sign: could not parse the render: {e}")))?;

    // 1. The signature *value* dictionary. Insert /ByteRange before /Contents
    //    so the range placeholder sits ahead of the hex placeholder in the
    //    serialized bytes (either order is correct, but this keeps the patch
    //    regions from overlapping conceptually).
    let mut sig = Dictionary::new();
    sig.set("Type", Object::Name(b"Sig".to_vec()));
    sig.set("Filter", Object::Name(b"Adobe.PPKLite".to_vec()));
    // PAdES: detached CMS. (Adobe's older /adbe.pkcs7.detached is the non-PAdES
    // equivalent; ETSI.CAdES.detached is the ETSI EN 319 142 baseline.)
    sig.set("SubFilter", Object::Name(b"ETSI.CAdES.detached".to_vec()));
    sig.set(
        "ByteRange",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(BYTE_RANGE_SENTINEL),
            Object::Integer(BYTE_RANGE_SENTINEL),
            Object::Integer(BYTE_RANGE_SENTINEL),
        ]),
    );
    sig.set(
        "Contents",
        Object::String(vec![0u8; placeholder_len], StringFormat::Hexadecimal),
    );
    if let Some(r) = &meta.reason {
        sig.set("Reason", lit(r));
    }
    if let Some(l) = &meta.location {
        sig.set("Location", lit(l));
    }
    if let Some(n) = &meta.name {
        sig.set("Name", lit(n));
    }
    if let Some(c) = &meta.contact {
        sig.set("ContactInfo", lit(c));
    }
    if let Some(m) = &meta.signing_time {
        sig.set("M", lit(m));
    }
    let sig_id = doc.add_object(Object::Dictionary(sig));

    // 2. A signature field that is also its own widget annotation on page 1.
    //    Invisible (/Rect [0 0 0 0]); /F = Print(4) | Locked(128) = 132.
    let page_id = *doc
        .get_pages()
        .values()
        .next()
        .ok_or_else(|| PdfError::Backend("sign: document has no pages".into()))?;

    let mut field = Dictionary::new();
    field.set("Type", Object::Name(b"Annot".to_vec()));
    field.set("Subtype", Object::Name(b"Widget".to_vec()));
    field.set("FT", Object::Name(b"Sig".to_vec()));
    field.set(
        "Rect",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(0),
        ]),
    );
    field.set("T", lit("Signature1"));
    field.set("V", Object::Reference(sig_id));
    field.set("P", Object::Reference(page_id));
    field.set("F", Object::Integer(132));
    let field_id = doc.add_object(Object::Dictionary(field));

    // Append the widget to the page's /Annots (creating it if absent).
    {
        let page = doc
            .get_object_mut(page_id)
            .map_err(|e| PdfError::Backend(format!("sign: no page object: {e}")))?
            .as_dict_mut()
            .map_err(|e| PdfError::Backend(format!("sign: page is not a dict: {e}")))?;
        let mut annots = page
            .get(b"Annots")
            .ok()
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default();
        annots.push(Object::Reference(field_id));
        page.set("Annots", Object::Array(annots));
    }

    // 3. /AcroForm on the catalog with the field and /SigFlags 3
    //    (SignaturesExist | AppendOnly).
    {
        let catalog = doc
            .catalog_mut()
            .map_err(|e| PdfError::Backend(format!("sign: no catalog: {e}")))?;
        let mut acro = catalog
            .get(b"AcroForm")
            .ok()
            .and_then(|o| o.as_dict().ok())
            .cloned()
            .unwrap_or_default();
        let mut fields = acro
            .get(b"Fields")
            .ok()
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default();
        fields.push(Object::Reference(field_id));
        acro.set("Fields", Object::Array(fields));
        acro.set("SigFlags", Object::Integer(3));
        catalog.set("AcroForm", Object::Dictionary(acro));
    }

    // 4. Serialize once. Everything below is length-preserving in-place surgery.
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|e| PdfError::Backend(format!("sign: could not save: {e}")))?;

    // 5. Locate the zeroed /Contents hex run and finalize the /ByteRange.
    let (open_lt, close_gt) = locate_contents(&bytes, placeholder_len)?;
    let contents_hex_offset = open_lt + 1;
    let contents_hex_len = close_gt - contents_hex_offset;
    // Signed ranges include the `<` and `>`, exclude only the hex between them.
    let byte_range = [0usize, open_lt + 1, close_gt, bytes.len() - close_gt];
    patch_byte_range(&mut bytes, byte_range)?;

    Ok(PreparedSignature {
        pdf: bytes,
        byte_range,
        contents_hex_offset,
        contents_hex_len,
    })
}

/// Splice the DER-encoded CMS SignedData into the reserved `/Contents`
/// placeholder. The hex overwrites the leading zeros in place; the trailing
/// zeros stay (a fixed-length hex string longer than the DER is standard — the
/// verifier reads the DER length header and ignores the padding). Length never
/// changes, so the `/ByteRange` remains valid.
pub fn embed_cms(mut prepared: PreparedSignature, cms_der: &[u8]) -> Result<Vec<u8>> {
    let hex = to_hex(cms_der);
    if hex.len() > prepared.contents_hex_len {
        return Err(PdfError::Backend(format!(
            "sign: CMS signature is {} hex chars but only {} are reserved; \
             raise the placeholder size",
            hex.len(),
            prepared.contents_hex_len
        )));
    }
    let start = prepared.contents_hex_offset;
    prepared.pdf[start..start + hex.len()].copy_from_slice(hex.as_bytes());
    Ok(prepared.pdf)
}

/// Find the `<` … `>` delimiting the zeroed `/Contents` hex string by matching
/// its unique long run of ASCII `'0'`s (there is no other kilobyte-scale run of
/// zeros in a rendered PDF). Returns the byte indices of `<` and `>`.
fn locate_contents(bytes: &[u8], placeholder_len: usize) -> Result<(usize, usize)> {
    let zeros = placeholder_len * 2;
    // A short unique needle expanded outward is enough and cheap.
    let needle = vec![b'0'; zeros.min(256)];
    let hit = find_subslice(bytes, &needle).ok_or_else(|| {
        PdfError::Backend("sign: could not locate the /Contents placeholder".into())
    })?;
    // Walk left to the '<' and right to the '>' bounding the full zero run.
    let mut open = hit;
    while open > 0 && bytes[open - 1] == b'0' {
        open -= 1;
    }
    let mut close = hit + needle.len();
    while close < bytes.len() && bytes[close] == b'0' {
        close += 1;
    }
    let open_lt = open
        .checked_sub(1)
        .filter(|&i| bytes[i] == b'<')
        .ok_or_else(|| PdfError::Backend("sign: malformed /Contents placeholder (no '<')".into()))?;
    if close >= bytes.len() || bytes[close] != b'>' {
        return Err(PdfError::Backend(
            "sign: malformed /Contents placeholder (no '>')".into(),
        ));
    }
    if close - (open_lt + 1) != zeros {
        return Err(PdfError::Backend(
            "sign: /Contents placeholder length mismatch".into(),
        ));
    }
    Ok((open_lt, close))
}

/// Overwrite the `/ByteRange [ … ]` array in place with the concrete offsets,
/// space-padding to the original span length so no byte offset moves.
fn patch_byte_range(bytes: &mut [u8], range: [usize; 4]) -> Result<()> {
    let key = b"/ByteRange";
    let key_at = find_subslice(bytes, key)
        .ok_or_else(|| PdfError::Backend("sign: /ByteRange key not found".into()))?;
    let open = key_at + key.len()
        + bytes[key_at + key.len()..]
            .iter()
            .position(|&b| b == b'[')
            .ok_or_else(|| PdfError::Backend("sign: /ByteRange has no '['".into()))?;
    let close = open
        + bytes[open..]
            .iter()
            .position(|&b| b == b']')
            .ok_or_else(|| PdfError::Backend("sign: /ByteRange has no ']'".into()))?;
    let span = close - open + 1; // includes both brackets
    let [s1, l1, s2, l2] = range;
    let mut replacement = format!("[{} {} {} {}", s1, l1, s2, l2).into_bytes();
    if replacement.len() + 1 > span {
        return Err(PdfError::Backend(
            "sign: /ByteRange did not fit its placeholder".into(),
        ));
    }
    // Pad with spaces so the closing ']' lands on the original offset.
    while replacement.len() < span - 1 {
        replacement.push(b' ');
    }
    replacement.push(b']');
    bytes[open..=close].copy_from_slice(&replacement);
    Ok(())
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}
