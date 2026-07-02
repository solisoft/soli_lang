//! Shared PDF/A-3b baseline transformation: version 1.7, CIDToGIDMap +
//! annotation-flag conformance fixes, sRGB OutputIntent, XMP metadata, and
//! Info dict + /ID sync. Used by the Factur-X embed step (with the Factur-X
//! XMP extension) and by the standalone `pdfa` render option (plain packet).

use lopdf::{Dictionary, Document, Object, Stream, StringFormat};
use md5::{Digest, Md5};
use time::OffsetDateTime;

use super::{xmp, FacturxMetadata};
use crate::error::{PdfError, Result};

/// Bundled redistributable sRGB ICC profile (saucecontrol Compact-ICC, CC0).
static SRGB_ICC: &[u8] = include_bytes!("../../assets/sRGB-v2-micro.icc");

/// Convert rendered PDF bytes to PDF/A-3b bytes without any Factur-X payload.
pub fn to_pdfa(pdf: &[u8], meta: &FacturxMetadata) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Pdfa(format!("could not parse rendered PDF: {e}")))?;
    apply_pdfa_base(&mut doc, xmp::build(None, meta), meta)?;
    let mut buf = Vec::new();
    doc.save_to(&mut buf)
        .map_err(|e| PdfError::Pdfa(format!("save: {e}")))?;
    Ok(buf)
}

/// Apply the PDF/A-3b baseline to a parsed document, with the caller-built XMP
/// packet as the catalog metadata stream.
pub(crate) fn apply_pdfa_base(
    doc: &mut Document,
    xmp_packet: String,
    meta: &FacturxMetadata,
) -> Result<()> {
    // PDF/A-3 is PDF 1.7.
    doc.version = "1.7".to_string();

    // PDF/A requires every embedded Type 2 CIDFont to carry an explicit
    // /CIDToGIDMap (ISO 19005-3 6.2.11.3.2). printpdf relies on the implicit
    // Identity default, so add it explicitly here.
    fix_cid_to_gid_map(doc);

    // PDF/A (ISO 19005-3 6.3.2): every annotation must set the Print flag and
    // clear Hidden/Invisible/NoView. printpdf's link annotations omit /F.
    fix_annotation_flags(doc);

    let output_intent = add_output_intent(doc);
    let metadata = add_metadata_stream(doc, xmp_packet);

    {
        let catalog = doc
            .catalog_mut()
            .map_err(|e| PdfError::Pdfa(format!("no catalog: {e}")))?;
        catalog.set(
            "OutputIntents",
            Object::Array(vec![Object::Reference(output_intent)]),
        );
        catalog.set("Metadata", Object::Reference(metadata));
    }

    sync_info_and_id(doc, meta);
    Ok(())
}

/// Add `/CIDToGIDMap /Identity` to every embedded Type 2 CIDFont that lacks it.
///
/// printpdf stores the CIDFont as a dictionary nested inside the Type0 font's
/// `/DescendantFonts` array (usually inline, occasionally as a reference), so we
/// reach into each Type0 font to patch its descendant(s).
fn fix_cid_to_gid_map(doc: &mut Document) {
    let type0_ids: Vec<lopdf::ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(id, obj)| {
            let d = obj.as_dict().ok()?;
            if d.get(b"Subtype").ok()?.as_name().ok()? == b"Type0" {
                Some(*id)
            } else {
                None
            }
        })
        .collect();

    // Descendant fonts that are indirect references (patched separately).
    let mut referenced: Vec<lopdf::ObjectId> = Vec::new();

    for id in &type0_ids {
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(*id) {
            if let Ok(Object::Array(arr)) = d.get_mut(b"DescendantFonts") {
                for el in arr.iter_mut() {
                    match el {
                        Object::Dictionary(cid) => ensure_identity_cidmap(cid),
                        Object::Reference(r) => referenced.push(*r),
                        _ => {}
                    }
                }
            }
        }
    }

    for id in referenced {
        if let Ok(Object::Dictionary(cid)) = doc.get_object_mut(id) {
            ensure_identity_cidmap(cid);
        }
    }
}

/// Force `/F 4` (Print set; Hidden/Invisible/NoView clear) on every page
/// annotation, as PDF/A requires. Annotations are usually inline dictionaries in
/// the page's `/Annots` array; occasionally indirect references.
fn fix_annotation_flags(doc: &mut Document) {
    let page_ids: Vec<lopdf::ObjectId> = doc.get_pages().values().copied().collect();
    let mut referenced: Vec<lopdf::ObjectId> = Vec::new();

    for pid in page_ids {
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(pid) {
            if let Ok(Object::Array(annots)) = d.get_mut(b"Annots") {
                for el in annots.iter_mut() {
                    match el {
                        Object::Dictionary(a) => a.set("F", Object::Integer(4)),
                        Object::Reference(r) => referenced.push(*r),
                        _ => {}
                    }
                }
            }
        }
    }

    for id in referenced {
        if let Ok(Object::Dictionary(a)) = doc.get_object_mut(id) {
            a.set("F", Object::Integer(4));
        }
    }
}

fn ensure_identity_cidmap(cid: &mut Dictionary) {
    let is_cid2 = cid
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .map(|n| n == b"CIDFontType2")
        .unwrap_or(false);
    if is_cid2 && cid.get(b"CIDToGIDMap").is_err() {
        cid.set("CIDToGIDMap", Object::Name(b"Identity".to_vec()));
    }
}

fn add_output_intent(doc: &mut Document) -> lopdf::ObjectId {
    // ICC profile stream with /N 3 (RGB).
    let mut icc_dict = Dictionary::new();
    icc_dict.set("N", Object::Integer(3));
    let icc_stream = Stream::new(icc_dict, SRGB_ICC.to_vec()).with_compression(false);
    let icc_id = doc.add_object(Object::Stream(icc_stream));

    let mut oi = Dictionary::new();
    oi.set("Type", Object::Name(b"OutputIntent".to_vec()));
    oi.set("S", Object::Name(b"GTS_PDFA1".to_vec()));
    oi.set(
        "OutputConditionIdentifier",
        Object::String(b"sRGB".to_vec(), StringFormat::Literal),
    );
    oi.set(
        "Info",
        Object::String(b"sRGB IEC61966-2.1".to_vec(), StringFormat::Literal),
    );
    oi.set("DestOutputProfile", Object::Reference(icc_id));
    doc.add_object(Object::Dictionary(oi))
}

fn add_metadata_stream(doc: &mut Document, packet: String) -> lopdf::ObjectId {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Metadata".to_vec()));
    dict.set("Subtype", Object::Name(b"XML".to_vec()));
    // XMP must stay plaintext for PDF/A; never compress.
    let stream = Stream::new(dict, packet.into_bytes()).with_compression(false);
    doc.add_object(Object::Stream(stream))
}

/// Mirror key metadata into the Info dictionary and ensure a trailer `/ID`.
fn sync_info_and_id(doc: &mut Document, meta: &FacturxMetadata) {
    let date = Object::String(pdf_date(meta.created).into_bytes(), StringFormat::Literal);

    let mut info = Dictionary::new();
    info.set(
        "Title",
        Object::String(meta.title.clone().into_bytes(), StringFormat::Literal),
    );
    info.set(
        "Author",
        Object::String(meta.author.clone().into_bytes(), StringFormat::Literal),
    );
    info.set(
        "Producer",
        Object::String(meta.producer.clone().into_bytes(), StringFormat::Literal),
    );
    info.set(
        "Creator",
        Object::String(
            meta.creator_tool.clone().into_bytes(),
            StringFormat::Literal,
        ),
    );
    info.set("CreationDate", date.clone());
    info.set("ModDate", date);
    let info_id = doc.add_object(Object::Dictionary(info));
    doc.trailer.set("Info", Object::Reference(info_id));

    if doc.trailer.get(b"ID").is_err() {
        // A deterministic-ish id from the title; two equal entries are allowed.
        let mut h = Md5::new();
        h.update(meta.title.as_bytes());
        h.update(meta.producer.as_bytes());
        let id = h.finalize().to_vec();
        let entry = Object::String(id, StringFormat::Hexadecimal);
        doc.trailer
            .set("ID", Object::Array(vec![entry.clone(), entry]));
    }
}

/// Format a PDF date string: `D:YYYYMMDDHHmmSS+HH'mm'`.
pub(crate) fn pdf_date(dt: OffsetDateTime) -> String {
    let o = dt.offset();
    let (oh, om, _) = o.as_hms();
    let sign = if oh < 0 || om < 0 { '-' } else { '+' };
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}{}{:02}'{:02}'",
        dt.year(),
        dt.month() as u8,
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        sign,
        oh.unsigned_abs(),
        om.unsigned_abs(),
    )
}
