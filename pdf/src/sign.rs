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
//! **Two serializations.** An unsigned PDF is re-serialized once (via lopdf,
//! like every other pass). A PDF that already carries a signature — or when
//! [`SignMeta::incremental`] asks for it — gets an **incremental update**
//! instead: the original bytes are kept verbatim and only the new and changed
//! objects are appended, with a fresh xref section whose `/Prev` points back.
//! Every earlier signature's `/ByteRange` therefore still covers exactly the
//! bytes it signed, which is what lets several people sign one document in
//! turn. Either way, what follows is *length-preserving* in-place patching —
//! the `/ByteRange` integers (space-padded) and the `/Contents` hex
//! (zero-padded) — so no byte offset ever shifts.

use std::collections::HashSet;
use std::io::Write as _;

use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, StringFormat};

use crate::error::{PdfError, Result};

/// Default reserved size in bytes for the CMS blob (the hex placeholder is
/// twice this). 16 KiB comfortably holds a signer certificate plus a couple of
/// intermediates; oversizing only wastes zero-padding, never a real problem.
pub const DEFAULT_PLACEHOLDER_LEN: usize = 16384;

/// Wide sentinel written into each `/ByteRange` slot before we know the real
/// offsets: ten digits, so it out-sizes any offset in a file up to ~9.9 GB and
/// the real value always fits when we space-pad the patch.
const BYTE_RANGE_SENTINEL: i64 = 9_999_999_999;

/// Where to draw a *visible* signature block, in PDF points from the page's
/// bottom-left. Without one, the signature is a valid but invisible field.
///
/// When the signature goes into an existing, pre-placed field
/// ([`SignMeta::field`]), the field's own page and rectangle win (the block is
/// drawn there even without an appearance): only `image` is read.
#[derive(Debug, Clone, Default)]
pub struct SignAppearance {
    /// 1-based page to place the block on.
    pub page: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// PNG/JPEG/WebP bytes of a handwritten signature drawn in the block above
    /// the signer's name and date (transparency is kept).
    pub image: Option<Vec<u8>>,
}

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
    /// When set, draw a visible signature block on the page.
    pub appearance: Option<SignAppearance>,
    /// Name of the signature field. When the document already has an
    /// *unsigned* signature field of that name, the signature fills it (its
    /// page and rectangle are used); otherwise a new field of that name is
    /// created. `None` creates `Signature1`, `Signature2`, … — the first
    /// free name.
    pub field: Option<String>,
    /// Append the signature as an incremental update even when the document
    /// has no signature yet. Already-signed documents always are.
    pub incremental: bool,
    /// More places to draw the same signature block (initials on each page,
    /// a second signature line…): `/Stamp` annotations carrying the
    /// appearance, added in the same update as the signature and therefore
    /// covered by it — moving or removing one afterwards breaks the
    /// signature. Only `page`, `x`, `y`, `width`, `height` are read; the
    /// image is `appearance.image`.
    pub stamps: Vec<SignAppearance>,
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

fn backend(msg: impl Into<String>) -> PdfError {
    PdfError::Backend(msg.into())
}

fn escape_appearance(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '(' => o.push_str("\\("),
            ')' => o.push_str("\\)"),
            '\\' => o.push_str("\\\\"),
            c => o.push(c),
        }
    }
    o
}

/// Turn a PDF date `"D:20260703120000+00'00'"` into a readable `"2026-07-03 12:00"`.
fn human_date(pdf: &str) -> String {
    let d = pdf.strip_prefix("D:").unwrap_or(pdf);
    if d.len() >= 12 && d[0..12].bytes().all(|b| b.is_ascii_digit()) {
        format!(
            "{}-{}-{} {}:{}",
            &d[0..4],
            &d[4..6],
            &d[6..8],
            &d[8..10],
            &d[10..12]
        )
    } else {
        pdf.to_string()
    }
}

fn flate(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data)
        .and_then(|_| enc.finish())
        .map_err(|e| backend(format!("sign: could not compress the signature image: {e}")))
}

/// Decode a handwritten-signature image into an `/Image` XObject (RGB, plus an
/// `/SMask` when it has transparency). Returns its id and pixel size.
fn add_image_xobject(doc: &mut Document, bytes: &[u8]) -> Result<(ObjectId, u32, u32)> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| backend(format!("sign: appearance image: {e}")))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(backend("sign: appearance image is empty"));
    }
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    for px in img.pixels() {
        rgb.extend_from_slice(&px.0[..3]);
        alpha.push(px.0[3]);
    }
    let image_dict = |color_space: &[u8], data: Vec<u8>| {
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"XObject".to_vec()));
        d.set("Subtype", Object::Name(b"Image".to_vec()));
        d.set("Width", Object::Integer(w as i64));
        d.set("Height", Object::Integer(h as i64));
        d.set("ColorSpace", Object::Name(color_space.to_vec()));
        d.set("BitsPerComponent", Object::Integer(8));
        d.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        Stream::new(d, data)
    };
    let smask = if alpha.iter().any(|&a| a < 255) {
        Some(doc.add_object(Object::Stream(image_dict(b"DeviceGray", flate(&alpha)?))))
    } else {
        None
    };
    let mut stream = image_dict(b"DeviceRGB", flate(&rgb)?);
    if let Some(id) = smask {
        stream.dict.set("SMask", Object::Reference(id));
    }
    Ok((doc.add_object(Object::Stream(stream)), w, h))
}

/// Build the visible signature block: a bordered box with the signer name,
/// "Digitally signed", the date, and reason/location when present — or, with
/// a handwritten image, the image above a compact name + date caption.
/// Returns the `/AP /N` Form XObject's object id.
fn build_appearance(
    doc: &mut Document,
    meta: &SignMeta,
    w: f32,
    h: f32,
    image: Option<&[u8]>,
) -> Result<ObjectId> {
    let mut font_dict = Dictionary::new();
    font_dict.set("Type", Object::Name(b"Font".to_vec()));
    font_dict.set("Subtype", Object::Name(b"Type1".to_vec()));
    font_dict.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    let font = doc.add_object(Object::Dictionary(font_dict));

    let mut lines: Vec<(f32, String)> = Vec::new();
    let mut c = String::new();
    c.push_str("q\n0.55 0.55 0.55 RG 0.7 w\n");
    c.push_str(&format!("0.5 0.5 {:.2} {:.2} re S\n", w - 1.0, h - 1.0));

    let mut xobjects = Dictionary::new();
    match image {
        Some(bytes) => {
            // Caption: one small line, name — date.
            let mut caption = meta.name.clone().unwrap_or_default();
            if let Some(t) = &meta.signing_time {
                if !caption.is_empty() {
                    caption.push_str(" - ");
                }
                caption.push_str(&human_date(t));
            }
            let caption_h = if caption.is_empty() { 0.0 } else { 11.0 };
            if !caption.is_empty() {
                lines.push((6.5, caption));
            }
            let (img_id, iw, ih) = add_image_xobject(doc, bytes)?;
            xobjects.set("Sig0", Object::Reference(img_id));
            // Fit the image (aspect kept) into the box above the caption.
            let (bx, by) = (4.0, caption_h + 3.0);
            let (bw, bh) = ((w - 8.0).max(1.0), (h - caption_h - 7.0).max(1.0));
            let scale = (bw / iw as f32).min(bh / ih as f32);
            let (dw, dh) = (iw as f32 * scale, ih as f32 * scale);
            let (dx, dy) = (bx + (bw - dw) / 2.0, by + (bh - dh) / 2.0);
            c.push_str(&format!(
                "q {dw:.2} 0 0 {dh:.2} {dx:.2} {dy:.2} cm /Sig0 Do Q\n"
            ));
        }
        None => {
            if let Some(name) = &meta.name {
                lines.push((10.0, name.clone()));
            }
            lines.push((7.5, "Digitally signed".to_string()));
            if let Some(t) = &meta.signing_time {
                lines.push((7.5, format!("Date: {}", human_date(t))));
            }
            if let Some(r) = &meta.reason {
                lines.push((7.5, format!("Reason: {}", r)));
            }
            if let Some(l) = &meta.location {
                lines.push((7.5, format!("Location: {}", l)));
            }
        }
    }

    c.push_str("0.15 0.18 0.22 rg\n");
    // Text block: top-down without an image, a bottom caption with one.
    let mut y = if image.is_some() { 4.0 } else { h - 13.0 };
    for (size, text) in &lines {
        c.push_str("BT\n");
        c.push_str(&format!("/Helv {:.1} Tf\n", size));
        c.push_str(&format!("6 {:.2} Td\n", y));
        c.push_str(&format!("({}) Tj\nET\n", escape_appearance(text)));
        y -= size + 3.0;
    }
    c.push('Q');

    let mut fonts = Dictionary::new();
    fonts.set("Helv", Object::Reference(font));
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));
    if !xobjects.is_empty() {
        res.set("XObject", Object::Dictionary(xobjects));
    }

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Form".to_vec()));
    dict.set(
        "BBox",
        Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(w),
            Object::Real(h),
        ]),
    );
    dict.set("Resources", Object::Dictionary(res));
    Ok(doc.add_object(Object::Stream(Stream::new(dict, c.into_bytes()))))
}

fn lit(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

fn deref_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_dict().ok()),
        _ => None,
    }
}

/// A signature field met while walking `/AcroForm /Fields` (kids included).
struct SigField {
    id: ObjectId,
    name: String,
    signed: bool,
}

/// Every signature field of the document, with its partial name and whether a
/// `/V` signature value is already set. Fields without `/FT /Sig` are skipped,
/// but their names still count when picking a free `SignatureN`.
fn walk_fields(doc: &Document) -> (Vec<SigField>, HashSet<String>) {
    let mut sigs = Vec::new();
    let mut names = HashSet::new();
    let Some(acro) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|o| deref_dict(doc, o))
    else {
        return (sigs, names);
    };
    let roots = match acro.get(b"Fields") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(id)) => doc
            .get_object(*id)
            .and_then(|o| o.as_array().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut stack: Vec<(Object, usize)> = roots.into_iter().map(|o| (o, 0)).collect();
    let mut seen = HashSet::new();
    while let Some((obj, depth)) = stack.pop() {
        let Object::Reference(id) = obj else { continue };
        if depth > 16 || !seen.insert(id) {
            continue;
        }
        let Some(f) = doc.get_object(id).ok().and_then(|o| o.as_dict().ok()) else {
            continue;
        };
        let name = f
            .get(b"T")
            .ok()
            .and_then(|o| o.as_str().ok())
            .map(|s| String::from_utf8_lossy(s).into_owned());
        if let Some(n) = &name {
            names.insert(n.clone());
        }
        if f.get(b"FT").ok().and_then(|o| o.as_name().ok()) == Some(b"Sig") {
            if let Some(name) = name {
                sigs.push(SigField {
                    id,
                    name,
                    signed: f.get(b"V").is_ok(),
                });
            }
        }
        if let Ok(Object::Array(kids)) = f.get(b"Kids") {
            stack.extend(kids.iter().cloned().map(|k| (k, depth + 1)));
        }
    }
    (sigs, names)
}

/// True when the document already carries a filled signature field.
fn is_signed(doc: &Document) -> bool {
    walk_fields(doc).0.iter().any(|f| f.signed)
}

/// Resolve `dict[key]` to a mutable array, wherever it lives (inline in the
/// dict, or an indirect array object), creating an inline one when absent.
/// `owner` is the object holding `dict`.
fn push_to_array(doc: &mut Document, owner: ObjectId, key: &[u8], item: Object) -> Result<()> {
    let target = {
        let dict = doc
            .get_object(owner)
            .and_then(|o| o.as_dict())
            .map_err(|e| backend(format!("sign: {e}")))?;
        match dict.get(key) {
            Ok(Object::Reference(id)) => Some(*id),
            _ => None,
        }
    };
    if let Some(arr_id) = target {
        if let Ok(arr) = doc.get_object_mut(arr_id).and_then(|o| o.as_array_mut()) {
            arr.push(item);
            return Ok(());
        }
    }
    let dict = doc
        .get_object_mut(owner)
        .and_then(|o| o.as_dict_mut())
        .map_err(|e| backend(format!("sign: {e}")))?;
    let mut arr = dict
        .get(key)
        .ok()
        .and_then(|o| o.as_array().ok())
        .cloned()
        .unwrap_or_default();
    arr.push(item);
    dict.set(key.to_vec(), Object::Array(arr));
    Ok(())
}

/// Make sure the catalog's `/AcroForm` exists and has `/SigFlags 3`
/// (SignaturesExist | AppendOnly); returns the id of the object that holds the
/// AcroForm dictionary (the catalog itself when it is inline).
fn ensure_acroform(doc: &mut Document) -> Result<ObjectId> {
    let root = doc
        .trailer
        .get(b"Root")
        .and_then(|o| o.as_reference())
        .map_err(|e| backend(format!("sign: no catalog: {e}")))?;
    let acro_ref = doc
        .get_object(root)
        .and_then(|o| o.as_dict())
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|o| o.as_reference().ok());
    let acro_id = match acro_ref {
        Some(id) => id,
        None => {
            // Inline (or absent): move it into its own object so the
            // incremental update only rewrites what changes.
            let inline = doc
                .get_object(root)
                .and_then(|o| o.as_dict())
                .ok()
                .and_then(|c| c.get(b"AcroForm").ok())
                .and_then(|o| o.as_dict().ok())
                .cloned()
                .unwrap_or_default();
            let id = doc.add_object(Object::Dictionary(inline));
            doc.get_object_mut(root)
                .and_then(|o| o.as_dict_mut())
                .map_err(|e| backend(format!("sign: catalog: {e}")))?
                .set("AcroForm", Object::Reference(id));
            id
        }
    };
    let acro = doc
        .get_object_mut(acro_id)
        .and_then(|o| o.as_dict_mut())
        .map_err(|e| backend(format!("sign: AcroForm: {e}")))?;
    acro.set("SigFlags", Object::Integer(3));
    Ok(acro_id)
}

/// The page a widget sits on: its `/P`, else the page whose `/Annots` lists it.
fn widget_page(doc: &Document, widget: ObjectId) -> Option<ObjectId> {
    if let Some(p) = doc
        .get_object(widget)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"P").ok())
        .and_then(|o| o.as_reference().ok())
    {
        return Some(p);
    }
    doc.get_pages().into_values().find(|&pid| {
        doc.get_object(pid)
            .ok()
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"Annots").ok())
            .and_then(|a| match a {
                Object::Array(a) => Some(a.clone()),
                Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok().cloned(),
                _ => None,
            })
            .is_some_and(|a| a.contains(&Object::Reference(widget)))
    })
}

/// Add the signature value, field/widget and AcroForm entries to `doc`.
fn add_signature_objects(
    doc: &mut Document,
    meta: &SignMeta,
    placeholder_len: usize,
) -> Result<()> {
    // 1. The signature *value* dictionary. Insert /ByteRange before /Contents
    //    so the range placeholder sits ahead of the hex placeholder in the
    //    serialized bytes.
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
    let image = meta.appearance.as_ref().and_then(|a| a.image.as_deref());

    add_stamps(doc, meta, image)?;

    let (fields, taken) = walk_fields(doc);
    let acro_id = ensure_acroform(doc)?;

    // 2a. Fill a pre-placed, unsigned signature field of that name.
    if let Some(name) = &meta.field {
        if let Some(existing) = fields.iter().find(|f| &f.name == name) {
            if existing.signed {
                return Err(backend(format!(
                    "sign: field `{name}` is already signed; pick another field name"
                )));
            }
            return fill_existing_field(doc, existing.id, sig_id, meta, image);
        }
    }

    // 2b. A new signature field that is also its own widget annotation.
    //     Invisible (/Rect [0 0 0 0]) unless a visible appearance was
    //     requested. /F = Print(4) | Locked(128) = 132.
    let name = match &meta.field {
        Some(n) => n.clone(),
        None => (1..)
            .map(|i| format!("Signature{i}"))
            .find(|n| !taken.contains(n))
            .unwrap_or_else(|| "Signature".to_string()),
    };
    let pages = doc.get_pages();
    let page_id = match &meta.appearance {
        Some(a) => *pages
            .get(&a.page.max(1))
            .or_else(|| pages.values().next())
            .ok_or_else(|| backend("sign: document has no pages"))?,
        None => *pages
            .values()
            .next()
            .ok_or_else(|| backend("sign: document has no pages"))?,
    };

    let rect = match &meta.appearance {
        Some(a) => [a.x, a.y, a.x + a.width, a.y + a.height],
        None => [0.0, 0.0, 0.0, 0.0],
    };
    let appearance = match &meta.appearance {
        Some(a) => Some(build_appearance(doc, meta, a.width, a.height, image)?),
        None => None,
    };

    let mut field = Dictionary::new();
    field.set("Type", Object::Name(b"Annot".to_vec()));
    field.set("Subtype", Object::Name(b"Widget".to_vec()));
    field.set("FT", Object::Name(b"Sig".to_vec()));
    field.set(
        "Rect",
        Object::Array(rect.iter().map(|v| Object::Real(*v)).collect()),
    );
    field.set("T", lit(&name));
    field.set("V", Object::Reference(sig_id));
    field.set("P", Object::Reference(page_id));
    field.set("F", Object::Integer(132));
    if let Some(ap_id) = appearance {
        let mut ap = Dictionary::new();
        ap.set("N", Object::Reference(ap_id));
        field.set("AP", Object::Dictionary(ap));
    }
    let field_id = doc.add_object(Object::Dictionary(field));

    // 3. Hook the widget onto its page and the field into the AcroForm.
    push_to_array(doc, page_id, b"Annots", Object::Reference(field_id))?;
    push_to_array(doc, acro_id, b"Fields", Object::Reference(field_id))?;
    Ok(())
}

/// Draw the signature block at each of `meta.stamps` as a locked, printable
/// `/Stamp` annotation on its page. Called while the signature's own objects
/// are being added, so the stamps land in the revision the signature covers.
fn add_stamps(doc: &mut Document, meta: &SignMeta, image: Option<&[u8]>) -> Result<()> {
    if meta.stamps.is_empty() {
        return Ok(());
    }
    let pages = doc.get_pages();
    for stamp in &meta.stamps {
        let page_id = *pages.get(&stamp.page.max(1)).ok_or_else(|| {
            backend(format!(
                "sign: a stamp is on page {}, but the document has {} page(s)",
                stamp.page,
                pages.len()
            ))
        })?;
        if stamp.width <= 1.0 || stamp.height <= 1.0 {
            return Err(backend("sign: a stamp needs a width and a height"));
        }
        let ap_id = build_appearance(doc, meta, stamp.width, stamp.height, image)?;
        let mut ap = Dictionary::new();
        ap.set("N", Object::Reference(ap_id));
        let mut annot = Dictionary::new();
        annot.set("Type", Object::Name(b"Annot".to_vec()));
        annot.set("Subtype", Object::Name(b"Stamp".to_vec()));
        annot.set(
            "Rect",
            Object::Array(
                [
                    stamp.x,
                    stamp.y,
                    stamp.x + stamp.width,
                    stamp.y + stamp.height,
                ]
                .iter()
                .map(|v| Object::Real(*v))
                .collect(),
            ),
        );
        // Print (4) | Locked (128) | LockedContents (512).
        annot.set("F", Object::Integer(644));
        annot.set("P", Object::Reference(page_id));
        annot.set("AP", Object::Dictionary(ap));
        if let Some(name) = &meta.name {
            annot.set("Contents", lit(name));
        }
        let annot_id = doc.add_object(Object::Dictionary(annot));
        push_to_array(doc, page_id, b"Annots", Object::Reference(annot_id))?;
    }
    Ok(())
}

/// Point an existing unsigned signature field at `sig_id`, drawing the visible
/// block into its widget's rectangle when an appearance was requested.
fn fill_existing_field(
    doc: &mut Document,
    field_id: ObjectId,
    sig_id: ObjectId,
    meta: &SignMeta,
    image: Option<&[u8]>,
) -> Result<()> {
    // The widget is the field itself (merged dictionary) or its first kid.
    let widget_id = {
        let f = doc
            .get_object(field_id)
            .and_then(|o| o.as_dict())
            .map_err(|e| backend(format!("sign: field: {e}")))?;
        if f.get(b"Rect").is_ok() {
            field_id
        } else {
            f.get(b"Kids")
                .ok()
                .and_then(|o| o.as_array().ok())
                .and_then(|k| k.first())
                .and_then(|o| o.as_reference().ok())
                .ok_or_else(|| backend("sign: the signature field has no widget"))?
        }
    };

    // A pre-placed field with a real rectangle is meant to be seen: draw the
    // block into it whether or not `appearance` was passed.
    {
        let rect: Vec<f32> = doc
            .get_object(widget_id)
            .ok()
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"Rect").ok())
            .and_then(|o| o.as_array().ok())
            .map(|a| a.iter().filter_map(|v| v.as_float().ok()).collect())
            .unwrap_or_default();
        if rect.len() == 4 {
            let (w, h) = ((rect[2] - rect[0]).abs(), (rect[3] - rect[1]).abs());
            if w > 1.0 && h > 1.0 {
                let ap_id = build_appearance(doc, meta, w, h, image)?;
                let mut ap = Dictionary::new();
                ap.set("N", Object::Reference(ap_id));
                let widget = doc
                    .get_object_mut(widget_id)
                    .and_then(|o| o.as_dict_mut())
                    .map_err(|e| backend(format!("sign: widget: {e}")))?;
                widget.set("AP", Object::Dictionary(ap));
                if widget.get(b"P").is_err() {
                    if let Some(page) = widget_page(doc, widget_id) {
                        doc.get_object_mut(widget_id)
                            .and_then(|o| o.as_dict_mut())
                            .map_err(|e| backend(format!("sign: widget: {e}")))?
                            .set("P", Object::Reference(page));
                    }
                }
            }
        }
    }

    doc.get_object_mut(field_id)
        .and_then(|o| o.as_dict_mut())
        .map_err(|e| backend(format!("sign: field: {e}")))?
        .set("V", Object::Reference(sig_id));
    Ok(())
}

/// Append every object of `doc` that is new or differs from `prev` as an
/// incremental update to `prev_bytes` (which `prev` was loaded from).
fn serialize_incremental(prev_bytes: &[u8], prev: Document, doc: &Document) -> Result<Vec<u8>> {
    let version = prev.version.clone();
    let mut changed = Vec::new();
    for (id, obj) in &doc.objects {
        if prev.objects.get(id) != Some(obj) {
            changed.push((*id, obj.clone()));
        }
    }
    let mut inc = IncrementalDocument::create_from(prev_bytes.to_vec(), prev);
    inc.new_document.objects.extend(changed);
    inc.new_document.max_id = doc.max_id;
    inc.new_document.version = version;
    // The trailer is cloned from the previous revision: drop what described
    // *its* xref (stream keys, the hybrid-file pointer). lopdf re-adds what the
    // new section needs; `/Prev` already points at the previous xref.
    let trailer = &mut inc.new_document.trailer;
    for key in [
        &b"XRefStm"[..],
        b"DecodeParms",
        b"Filter",
        b"Index",
        b"W",
        b"Length",
        b"Type",
    ] {
        trailer.remove(key);
    }
    let mut out = Vec::with_capacity(prev_bytes.len() + 32 * 1024);
    inc.save_to(&mut out)
        .map_err(|e| backend(format!("sign: could not append the update: {e}")))?;
    Ok(out)
}

/// Reserve a signature dictionary in `pdf` and finalize its `/ByteRange`.
///
/// Adds a signature field + widget (invisible unless `meta.appearance`), an
/// `/AcroForm` with `/SigFlags 3`, and a `/Sig` value dictionary whose
/// `/Contents` is `placeholder_len` zero bytes. An already-signed `pdf` is
/// extended by an incremental update, so its signatures stay valid. Returns
/// the bytes with the `/ByteRange` patched to the real offsets, plus the
/// locations the host needs to digest and later splice.
pub fn prepare_signature(
    pdf: &[u8],
    meta: &SignMeta,
    placeholder_len: usize,
) -> Result<PreparedSignature> {
    // lopdf reads offsets relative to the `%PDF-` header; keep ours aligned.
    let head = pdf.windows(5).position(|w| w == b"%PDF-").unwrap_or(0);
    let pdf = &pdf[head..];
    let prev = Document::load_mem(pdf)
        .map_err(|e| backend(format!("sign: could not parse the PDF: {e}")))?;
    if prev.trailer.get(b"Encrypt").is_ok() {
        return Err(backend(
            "sign: the PDF is encrypted; a signed PDF must not be — sign the unencrypted original",
        ));
    }

    let incremental = meta.incremental || is_signed(&prev);
    let mut doc = prev.clone();
    add_signature_objects(&mut doc, meta, placeholder_len)?;

    // Serialize; `from` is where the new signature dictionary can start, so the
    // placeholder search never lands on an earlier signature's zero padding.
    let (mut bytes, from) = if incremental {
        let prev_len = pdf.len();
        (serialize_incremental(pdf, prev, &doc)?, prev_len)
    } else {
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes)
            .map_err(|e| backend(format!("sign: could not save: {e}")))?;
        (bytes, 0)
    };

    // Locate the zeroed /Contents hex run and finalize the /ByteRange.
    let (open_lt, close_gt) = locate_contents(&bytes, from, placeholder_len)?;
    let contents_hex_offset = open_lt + 1;
    let contents_hex_len = close_gt - contents_hex_offset;
    // Signed ranges include the `<` and `>`, exclude only the hex between them.
    let byte_range = [0usize, open_lt + 1, close_gt, bytes.len() - close_gt];
    patch_byte_range(&mut bytes, from, byte_range)?;

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

/// A signature read back out of an existing PDF, ready for CMS verification.
/// The `signed_bytes` are the two `/ByteRange` spans concatenated (what the
/// signature covers); `cms_der` is the detached CMS from `/Contents`.
pub struct ExtractedSignature {
    pub field: String,
    pub signed_bytes: Vec<u8>,
    pub cms_der: Vec<u8>,
    /// True when the signature covers the whole file but its own `/Contents`:
    /// the `/ByteRange` starts at byte 0, ends at EOF, and the one span it
    /// leaves out is exactly this signature's `/Contents` hex string (see
    /// [`covers_whole_file`]).
    pub covers_document: bool,
    pub reason: Option<String>,
    /// The `/M` signing-time PDF date string, if present.
    pub signing_time: Option<String>,
}

/// Trim a hex-decoded `/Contents` blob (CMS + zero padding) to the DER object's
/// own declared length.
fn trim_der(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() < 2 {
        return bytes.to_vec();
    }
    let len_byte = bytes[1];
    let total = if len_byte < 0x80 {
        2 + len_byte as usize
    } else {
        // A length of more octets than a `usize` holds is not a length this
        // blob can have: keep it whole and let the CMS parser refuse it.
        let n = (len_byte & 0x7f) as usize;
        if n > std::mem::size_of::<usize>() || 2 + n > bytes.len() {
            return bytes.to_vec();
        }
        let l = bytes[2..2 + n]
            .iter()
            .fold(0usize, |a, &b| (a << 8) | b as usize);
        match l.checked_add(2 + n) {
            Some(total) => total,
            None => return bytes.to_vec(),
        }
    };
    bytes[..total.min(bytes.len())].to_vec()
}

/// A `/ByteRange` as two in-bounds `(start, len)` spans, in file order.
///
/// `None` for anything that is not: a negative or non-integer entry, a sum
/// that overflows, a span past EOF, or a second span that starts before the
/// first one ends. The entries were cast with `as usize`, so `-1` became
/// `usize::MAX` and `s1 + l1` could wrap (or panic, in a debug build).
fn checked_byte_range(nums: &[i64], file_len: usize) -> Option<[usize; 4]> {
    let [s1, l1, s2, l2] = <[i64; 4]>::try_from(nums).ok()?;
    let [s1, l1, s2, l2] = [s1, l1, s2, l2].map(|n| usize::try_from(n).ok());
    let (s1, l1, s2, l2) = (s1?, l1?, s2?, l2?);
    let end1 = s1.checked_add(l1)?;
    let end2 = s2.checked_add(l2)?;
    (end1 <= s2 && end2 <= file_len).then_some([s1, l1, s2, l2])
}

/// Whether a well-formed `/ByteRange` signs the whole file but this
/// signature's own `/Contents`.
///
/// Reaching EOF is not enough. The first span must start at byte 0, or the
/// bytes before it are unsigned. And the gap between the spans — the only
/// bytes the signature leaves out — must be exactly the hex string of this
/// signature's `/Contents`, right after its `/Contents` key: otherwise a
/// range that ends at EOF can still leave out a page, an annotation, or any
/// object an attacker placed between the spans.
fn covers_whole_file(pdf: &[u8], range: [usize; 4], contents: &[u8]) -> bool {
    let [s1, l1, s2, l2] = range;
    s1 == 0 && s2 + l2 == pdf.len() && gap_is_contents(pdf, l1, s2, contents)
}

/// `pdf[start..end]` is the hex string whose value is `contents`, and the
/// token before it (whitespace aside) is the `/Contents` key.
///
/// Two shapes are accepted, both tight to the string: the gap is `<hex>`,
/// delimiters included (ISO 32000, what Acrobat writes), or the hex digits
/// alone between a `<` and a `>` just outside it (what `prepare_signature`
/// writes).
fn gap_is_contents(pdf: &[u8], start: usize, end: usize, contents: &[u8]) -> bool {
    let Some(gap) = pdf.get(start..end) else {
        return false;
    };
    let (hex, opener) = match (gap.first(), gap.last()) {
        (Some(b'<'), Some(b'>')) if gap.len() >= 2 => (&gap[1..gap.len() - 1], start),
        _ => match (start.checked_sub(1).map(|at| pdf[at]), pdf.get(end)) {
            (Some(b'<'), Some(b'>')) => (gap, start - 1),
            _ => return false,
        },
    };
    let digits: Vec<u8> = hex
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if !digits.iter().all(u8::is_ascii_hexdigit) {
        return false;
    }
    // PDF hex strings: an odd final digit is read as followed by a 0.
    let decoded: Vec<u8> = digits
        .chunks(2)
        .map(|pair| {
            let high = hex_value(pair[0]);
            let low = pair.get(1).map_or(0, |&digit| hex_value(digit));
            (high << 4) | low
        })
        .collect();
    if decoded != contents {
        return false;
    }
    let before = pdf[..opener]
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(&pdf[..0], |last| &pdf[..=last]);
    before.ends_with(b"/Contents")
}

fn hex_value(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        b'A'..=b'F' => digit - b'A' + 10,
        _ => 0,
    }
}

/// Extract every signature field's covered bytes + CMS from an existing PDF, so
/// the CMS can be cryptographically verified (crypto lives in the caller).
pub fn extract_signatures(pdf: &[u8]) -> Result<Vec<ExtractedSignature>> {
    let doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("verify: could not parse the PDF: {e}")))?;
    let mut out = Vec::new();
    let Ok(catalog) = doc.catalog() else {
        return Ok(out);
    };
    let Some(acro) = catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|o| deref_dict(&doc, o))
    else {
        return Ok(out);
    };
    let Ok(fields) = acro.get(b"Fields").and_then(|o| o.as_array()) else {
        return Ok(out);
    };

    for fref in fields {
        let Some(f) = deref_dict(&doc, fref) else {
            continue;
        };
        if f.get(b"FT").ok().and_then(|o| o.as_name().ok()) != Some(b"Sig") {
            continue;
        }
        let name = f
            .get(b"T")
            .ok()
            .and_then(|o| o.as_str().ok())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .unwrap_or_else(|| "Signature".to_string());
        let Some(sig) = f.get(b"V").ok().and_then(|o| deref_dict(&doc, o)) else {
            continue;
        };
        let Some(br) = sig.get(b"ByteRange").ok().and_then(|o| o.as_array().ok()) else {
            continue;
        };
        // Every entry must be an integer: a dropped one would shift the rest.
        let Some(nums) = br
            .iter()
            .map(|o| o.as_i64().ok())
            .collect::<Option<Vec<i64>>>()
        else {
            continue;
        };
        let Some(range) = checked_byte_range(&nums, pdf.len()) else {
            continue;
        };
        let [s1, l1, s2, l2] = range;
        let mut signed = Vec::with_capacity(l1 + l2);
        signed.extend_from_slice(&pdf[s1..s1 + l1]);
        signed.extend_from_slice(&pdf[s2..s2 + l2]);
        let Some(contents) = sig.get(b"Contents").ok().and_then(|o| o.as_str().ok()) else {
            continue;
        };
        let read_str = |key: &[u8]| {
            sig.get(key)
                .ok()
                .and_then(|o| o.as_str().ok())
                .map(|s| String::from_utf8_lossy(s).into_owned())
        };
        out.push(ExtractedSignature {
            field: name,
            signed_bytes: signed,
            cms_der: trim_der(contents),
            covers_document: covers_whole_file(pdf, range, contents),
            reason: read_str(b"Reason"),
            signing_time: read_str(b"M"),
        });
    }
    Ok(out)
}

/// Find the `<` … `>` delimiting the zeroed `/Contents` hex string by matching
/// its unique long run of ASCII `'0'`s (there is no other kilobyte-scale run of
/// zeros in a rendered PDF). Returns the byte indices of `<` and `>`.
fn locate_contents(bytes: &[u8], from: usize, placeholder_len: usize) -> Result<(usize, usize)> {
    let zeros = placeholder_len * 2;
    // A short unique needle expanded outward is enough and cheap. Searching
    // from `from` skips earlier signatures (their zero padding is a long run too).
    let needle = vec![b'0'; zeros.min(256)];
    let hit = from
        + find_subslice(&bytes[from..], &needle).ok_or_else(|| {
            PdfError::Backend("sign: could not locate the /Contents placeholder".into())
        })?;
    // Never walk back into the previous revision.
    let floor = from;
    // Walk left to the '<' and right to the '>' bounding the full zero run.
    let mut open = hit;
    while open > floor && bytes[open - 1] == b'0' {
        open -= 1;
    }
    let mut close = hit + needle.len();
    while close < bytes.len() && bytes[close] == b'0' {
        close += 1;
    }
    let open_lt = open
        .checked_sub(1)
        .filter(|&i| bytes[i] == b'<')
        .ok_or_else(|| {
            PdfError::Backend("sign: malformed /Contents placeholder (no '<')".into())
        })?;
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
fn patch_byte_range(bytes: &mut [u8], from: usize, range: [usize; 4]) -> Result<()> {
    let key = b"/ByteRange";
    let key_at = from
        + find_subslice(&bytes[from..], key)
            .ok_or_else(|| PdfError::Backend("sign: /ByteRange key not found".into()))?;
    let open = key_at
        + key.len()
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
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod byte_range_tests {
    use super::*;

    #[test]
    fn a_byte_range_must_be_non_negative_ordered_and_in_bounds() {
        assert_eq!(
            checked_byte_range(&[0, 10, 20, 5], 25),
            Some([0, 10, 20, 5])
        );
        assert_eq!(checked_byte_range(&[-1, 10, 20, 5], 25), None, "negative");
        assert_eq!(checked_byte_range(&[0, 10, 20, -5], 25), None, "negative");
        assert_eq!(
            checked_byte_range(&[0, i64::MAX, 20, 5], 25),
            None,
            "overflow"
        );
        assert_eq!(checked_byte_range(&[0, 10, i64::MAX, i64::MAX], 25), None);
        assert_eq!(checked_byte_range(&[0, 10, 20, 6], 25), None, "past EOF");
        assert_eq!(checked_byte_range(&[0, 21, 20, 5], 25), None, "overlap");
        assert_eq!(checked_byte_range(&[0, 10, 20], 25), None, "three entries");
    }

    /// `prefix/Contents <hex>suffix`, and the range that leaves out `<hex>`.
    fn doc(prefix: &str, hex: &str, suffix: &str) -> (Vec<u8>, [usize; 4]) {
        let head = format!("{prefix}/Contents ");
        let gap = format!("<{hex}>");
        let pdf = format!("{head}{gap}{suffix}").into_bytes();
        let s2 = head.len() + gap.len();
        (pdf.clone(), [0, head.len(), s2, pdf.len() - s2])
    }

    #[test]
    fn the_whole_file_but_its_own_contents_is_covered() {
        let (pdf, range) = doc("%PDF-1.7 ", "3003", " %%EOF");
        assert!(covers_whole_file(&pdf, range, &[0x30, 0x03]));
    }

    #[test]
    fn a_gap_of_the_hex_digits_alone_covers_the_file_too() {
        // `prepare_signature` leaves the `<` and `>` inside the signed spans.
        let (pdf, [s1, l1, s2, l2]) = doc("%PDF-1.7 ", "3003", " %%EOF");
        assert!(covers_whole_file(
            &pdf,
            [s1, l1 + 1, s2 - 1, l2 + 1],
            &[0x30, 0x03]
        ));
        // One digit short of the string is not it.
        assert!(!covers_whole_file(
            &pdf,
            [s1, l1 + 2, s2 - 1, l2 + 1],
            &[0x30, 0x03]
        ));
    }

    #[test]
    fn a_range_that_skips_the_head_does_not_cover_the_file() {
        let (pdf, [_, l1, s2, l2]) = doc("%PDF-1.7 ", "3003", " %%EOF");
        assert!(!covers_whole_file(&pdf, [1, l1 - 1, s2, l2], &[0x30, 0x03]));
    }

    #[test]
    fn a_gap_that_is_not_this_contents_does_not_cover_the_file() {
        let (pdf, range) = doc("%PDF-1.7 ", "3003", " %%EOF");
        // Same shape, other value: the gap is not this signature's /Contents.
        assert!(!covers_whole_file(&pdf, range, &[0x30, 0x04]));
        // A wider gap that also swallows unsigned bytes after the hex.
        let [s1, l1, s2, l2] = range;
        assert!(!covers_whole_file(
            &pdf,
            [s1, l1, s2 + 2, l2 - 2],
            &[0x30, 0x03]
        ));
        assert!(!covers_whole_file(
            &pdf,
            [s1, l1 - 2, s2, l2],
            &[0x30, 0x03]
        ));
        // A gap somewhere else than right after the /Contents key.
        let other = b"%PDF-1.7 /Other <3003> %%EOF".to_vec();
        let s2 = other.len() - 6;
        assert!(!covers_whole_file(&other, [0, 16, s2, 6], &[0x30, 0x03]));
    }

    #[test]
    fn a_der_length_wider_than_usize_is_left_whole() {
        let blob = [
            0x30, 0x89, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ];
        assert_eq!(trim_der(&blob), blob.to_vec());
        let blob = [
            0x30, 0x88, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00,
        ];
        assert_eq!(trim_der(&blob), blob.to_vec(), "2 + n + l overflows");
    }
}
