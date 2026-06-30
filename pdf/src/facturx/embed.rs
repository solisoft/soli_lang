//! lopdf operations that turn a rendered PDF into a PDF/A-3b Factur-X file:
//! embed `factur-x.xml`, wire up the `/AF` + `/EmbeddedFiles` name tree, add an
//! sRGB OutputIntent, attach the XMP metadata, and sync the Info dict + /ID.

use lopdf::{Dictionary, Document, Object, Stream, StringFormat};
use md5::{Digest, Md5};
use time::OffsetDateTime;

use super::xmp;
use super::{FacturxMetadata, Profile};
use crate::error::{PdfError, Result};

/// Bundled redistributable sRGB ICC profile (saucecontrol Compact-ICC, CC0).
static SRGB_ICC: &[u8] = include_bytes!("../../assets/sRGB-v2-micro.icc");

const FILENAME: &str = "factur-x.xml";

/// Perform the embedding on a parsed document, returning PDF/A-3b bytes.
pub fn embed(
    mut doc: Document,
    xml: &[u8],
    profile: Profile,
    meta: &FacturxMetadata,
) -> Result<Vec<u8>> {
    // PDF/A-3 is PDF 1.7.
    doc.version = "1.7".to_string();

    // PDF/A requires every embedded Type 2 CIDFont to carry an explicit
    // /CIDToGIDMap (ISO 19005-3 6.2.11.3.2). printpdf relies on the implicit
    // Identity default, so add it explicitly here.
    fix_cid_to_gid_map(&mut doc);

    // PDF/A (ISO 19005-3 6.3.2): every annotation must set the Print flag and
    // clear Hidden/Invisible/NoView. printpdf's link annotations omit /F.
    fix_annotation_flags(&mut doc);

    let embedded_file = add_embedded_file(&mut doc, xml, meta);
    let filespec = add_filespec(&mut doc, embedded_file, profile);
    let output_intent = add_output_intent(&mut doc);
    let metadata = add_metadata(&mut doc, profile, meta);

    // Catalog wiring (done after objects exist so we can reference them).
    {
        let catalog = doc
            .catalog_mut()
            .map_err(|e| PdfError::Facturx(format!("no catalog: {e}")))?;

        // /AF [ filespec ]
        catalog.set("AF", Object::Array(vec![Object::Reference(filespec)]));

        // /Names << /EmbeddedFiles << /Names [ (factur-x.xml) filespec ] >> >>
        let mut embedded_files = Dictionary::new();
        embedded_files.set(
            "Names",
            Object::Array(vec![
                Object::String(FILENAME.as_bytes().to_vec(), StringFormat::Literal),
                Object::Reference(filespec),
            ]),
        );
        let names = match catalog.get(b"Names").ok().and_then(|o| o.as_dict().ok()) {
            Some(existing) => {
                let mut d = existing.clone();
                d.set("EmbeddedFiles", Object::Dictionary(embedded_files));
                d
            }
            None => {
                let mut d = Dictionary::new();
                d.set("EmbeddedFiles", Object::Dictionary(embedded_files));
                d
            }
        };
        catalog.set("Names", Object::Dictionary(names));

        catalog.set(
            "OutputIntents",
            Object::Array(vec![Object::Reference(output_intent)]),
        );
        catalog.set("Metadata", Object::Reference(metadata));
        catalog.set("PageMode", Object::Name(b"UseAttachments".to_vec()));
    }

    sync_info_and_id(&mut doc, meta);

    let mut buf = Vec::new();
    doc.save_to(&mut buf)
        .map_err(|e| PdfError::Facturx(format!("save: {e}")))?;
    Ok(buf)
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

fn add_embedded_file(doc: &mut Document, xml: &[u8], meta: &FacturxMetadata) -> lopdf::ObjectId {
    let mut checksum = Md5::new();
    checksum.update(xml);
    let digest = checksum.finalize();

    let mut params = Dictionary::new();
    params.set(
        "ModDate",
        Object::String(pdf_date(meta.created).into_bytes(), StringFormat::Literal),
    );
    params.set("Size", Object::Integer(xml.len() as i64));
    params.set(
        "CheckSum",
        Object::String(digest.to_vec(), StringFormat::Hexadecimal),
    );

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"EmbeddedFile".to_vec()));
    // lopdf escapes the '/' as #2F on write.
    dict.set("Subtype", Object::Name(b"text/xml".to_vec()));
    dict.set("Params", Object::Dictionary(params));

    // Store uncompressed (small) and prevent lopdf from re-compressing.
    let stream = Stream::new(dict, xml.to_vec()).with_compression(false);
    doc.add_object(Object::Stream(stream))
}

fn add_filespec(
    doc: &mut Document,
    embedded_file: lopdf::ObjectId,
    profile: Profile,
) -> lopdf::ObjectId {
    let mut ef = Dictionary::new();
    ef.set("F", Object::Reference(embedded_file));
    ef.set("UF", Object::Reference(embedded_file));

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Filespec".to_vec()));
    dict.set(
        "F",
        Object::String(FILENAME.as_bytes().to_vec(), StringFormat::Literal),
    );
    dict.set(
        "UF",
        Object::String(FILENAME.as_bytes().to_vec(), StringFormat::Literal),
    );
    dict.set(
        "AFRelationship",
        Object::Name(profile.af_relationship().as_bytes().to_vec()),
    );
    dict.set(
        "Desc",
        Object::String(b"Factur-X invoice".to_vec(), StringFormat::Literal),
    );
    dict.set("EF", Object::Dictionary(ef));
    doc.add_object(Object::Dictionary(dict))
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

fn add_metadata(doc: &mut Document, profile: Profile, meta: &FacturxMetadata) -> lopdf::ObjectId {
    let packet = xmp::build(profile, meta);
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
fn pdf_date(dt: OffsetDateTime) -> String {
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
