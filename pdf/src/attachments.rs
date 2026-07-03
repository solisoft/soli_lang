//! Generic file attachments: embed arbitrary files (CSV/XML/JSON sources,
//! terms PDFs, …) into the document's `/EmbeddedFiles` name tree, the same
//! mechanism Factur-X uses for `factur-x.xml`.
//!
//! Applied as a lopdf post-pass (after `stationery`); the Factur-X step runs
//! later and merges with — rather than clobbers — entries added here, so
//! `attachments` + `pdf_facturx` compose. Each filespec carries
//! `AFRelationship /Unspecified` and is appended to the catalog's `/AF` array
//! so PDF/A-3 outputs keep every embedded file associated.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use md5::{Digest, Md5};

use crate::error::{PdfError, Result};

/// One file to embed.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// File name shown in the reader's attachments panel (also the name-tree key).
    pub name: String,
    /// MIME type (written as the EmbeddedFile `/Subtype`).
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Read every embedded file out of `pdf` — the reverse of [`apply_attachments`].
/// Used to process an *incoming* document (e.g. a received Factur-X invoice).
pub fn extract_attachments(pdf: &[u8]) -> Result<Vec<Attachment>> {
    let doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("attachments: could not parse the PDF: {e}")))?;
    let Ok(catalog) = doc.catalog() else {
        return Ok(Vec::new());
    };

    let mut filespec_ids: Vec<ObjectId> = Vec::new();
    // Primary source: the `/Names /EmbeddedFiles` name tree.
    if let Some(ef) = catalog
        .get(b"Names")
        .ok()
        .and_then(|o| deref_dict(&doc, o))
        .and_then(|names| names.get(b"EmbeddedFiles").ok())
        .and_then(|o| deref_dict(&doc, o))
    {
        collect_filespecs(&doc, ef, &mut filespec_ids);
    }
    // Fallback: the `/AF` associated-files array (some producers omit the tree).
    if filespec_ids.is_empty() {
        if let Ok(af) = catalog.get(b"AF").and_then(|o| o.as_array()) {
            filespec_ids.extend(af.iter().filter_map(|o| o.as_reference().ok()));
        }
    }

    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for fid in filespec_ids {
        if seen.insert(fid) {
            if let Some(att) = read_filespec(&doc, fid) {
                out.push(att);
            }
        }
    }
    Ok(out)
}

/// The embedded Factur-X / ZUGFeRD / XRechnung invoice XML, if present, so a
/// received e-invoice can be parsed. Matches the standard attachment names.
pub fn extract_facturx(pdf: &[u8]) -> Result<Option<Vec<u8>>> {
    const NAMES: [&str; 5] = [
        "factur-x.xml",
        "zugferd-invoice.xml",
        "xrechnung.xml",
        "cii.xml",
        "factur-x.cii.xml",
    ];
    for att in extract_attachments(pdf)? {
        let lower = att.name.to_ascii_lowercase();
        if NAMES.contains(&lower.as_str()) || lower.ends_with("factur-x.xml") {
            return Ok(Some(att.bytes));
        }
    }
    Ok(None)
}

fn deref_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_dict().ok()),
        _ => None,
    }
}

/// Recurse an EmbeddedFiles name tree, collecting filespec object ids.
fn collect_filespecs(doc: &Document, node: &Dictionary, out: &mut Vec<ObjectId>) {
    if let Ok(names) = node.get(b"Names").and_then(|o| o.as_array()) {
        for pair in names.chunks(2) {
            if let Some(Ok(id)) = pair.get(1).map(|v| v.as_reference()) {
                out.push(id);
            }
        }
    }
    if let Ok(kids) = node.get(b"Kids").and_then(|o| o.as_array()) {
        for kid in kids {
            if let Some(kd) = deref_dict(doc, kid) {
                collect_filespecs(doc, kd, out);
            }
        }
    }
}

fn read_filespec(doc: &Document, fid: ObjectId) -> Option<Attachment> {
    let fs = doc.get_object(fid).ok()?.as_dict().ok()?;
    let name = fs
        .get(b"UF")
        .or_else(|_| fs.get(b"F"))
        .ok()
        .and_then(|o| o.as_str().ok())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .unwrap_or_else(|| "attachment".to_string());
    let ef = deref_dict(doc, fs.get(b"EF").ok()?)?;
    let stream_ref = ef
        .get(b"F")
        .or_else(|_| ef.get(b"UF"))
        .ok()?
        .as_reference()
        .ok()?;
    let Ok(Object::Stream(stream)) = doc.get_object(stream_ref) else {
        return None;
    };
    let bytes = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    let mime = stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    Some(Attachment { name, mime, bytes })
}

/// Embed `attachments` into `pdf`, merging with any existing name-tree entries.
pub fn apply_attachments(pdf: &[u8], attachments: &[Attachment]) -> Result<Vec<u8>> {
    if attachments.is_empty() {
        return Ok(pdf.to_vec());
    }
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("attachments: could not parse the render: {e}")))?;

    let mut new_entries: Vec<(String, lopdf::ObjectId)> = Vec::with_capacity(attachments.len());
    for att in attachments {
        let stream_id = add_embedded_file(&mut doc, att);
        let filespec_id = add_filespec(&mut doc, att, stream_id);
        new_entries.push((att.name.clone(), filespec_id));
    }

    let catalog = doc
        .catalog_mut()
        .map_err(|e| PdfError::Backend(format!("attachments: no catalog: {e}")))?;

    // Merge into /Names /EmbeddedFiles /Names [ (name) ref … ] — a PDF name
    // tree must stay sorted by key.
    let mut pairs = existing_embedded_names(catalog);
    for (name, id) in &new_entries {
        pairs.push((name.clone(), *id));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut names_array: Vec<Object> = Vec::with_capacity(pairs.len() * 2);
    for (name, id) in &pairs {
        names_array.push(Object::String(
            name.as_bytes().to_vec(),
            StringFormat::Literal,
        ));
        names_array.push(Object::Reference(*id));
    }
    let mut embedded_files = Dictionary::new();
    embedded_files.set("Names", Object::Array(names_array));
    let mut names = match catalog.get(b"Names").ok().and_then(|o| o.as_dict().ok()) {
        Some(existing) => existing.clone(),
        None => Dictionary::new(),
    };
    names.set("EmbeddedFiles", Object::Dictionary(embedded_files));
    catalog.set("Names", Object::Dictionary(names));

    // Append the filespecs to /AF (associated files), creating it if absent.
    let mut af: Vec<Object> = match catalog.get(b"AF").ok().and_then(|o| o.as_array().ok()) {
        Some(existing) => existing.clone(),
        None => Vec::new(),
    };
    af.extend(new_entries.iter().map(|(_, id)| Object::Reference(*id)));
    catalog.set("AF", Object::Array(af));

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("attachments: could not save: {e}")))?;
    Ok(out)
}

/// Read the existing `(name, filespec)` pairs out of the catalog's
/// EmbeddedFiles name tree (flat `/Names` form only, which is what soli-pdf
/// itself writes).
fn existing_embedded_names(catalog: &Dictionary) -> Vec<(String, lopdf::ObjectId)> {
    let mut out = Vec::new();
    let Some(names) = catalog.get(b"Names").ok().and_then(|o| o.as_dict().ok()) else {
        return out;
    };
    let Some(ef) = names
        .get(b"EmbeddedFiles")
        .ok()
        .and_then(|o| o.as_dict().ok())
    else {
        return out;
    };
    let Some(arr) = ef.get(b"Names").ok().and_then(|o| o.as_array().ok()) else {
        return out;
    };
    for pair in arr.chunks_exact(2) {
        if let (Object::String(name, _), Object::Reference(id)) = (&pair[0], &pair[1]) {
            out.push((String::from_utf8_lossy(name).into_owned(), *id));
        }
    }
    out
}

fn add_embedded_file(doc: &mut Document, att: &Attachment) -> lopdf::ObjectId {
    let mut checksum = Md5::new();
    checksum.update(&att.bytes);
    let digest = checksum.finalize();

    let mut params = Dictionary::new();
    params.set("Size", Object::Integer(att.bytes.len() as i64));
    params.set(
        "CheckSum",
        Object::String(digest.to_vec(), StringFormat::Hexadecimal),
    );

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"EmbeddedFile".to_vec()));
    // lopdf escapes the '/' as #2F on write.
    dict.set("Subtype", Object::Name(att.mime.as_bytes().to_vec()));
    dict.set("Params", Object::Dictionary(params));

    let stream = Stream::new(dict, att.bytes.clone());
    doc.add_object(Object::Stream(stream))
}

fn add_filespec(
    doc: &mut Document,
    att: &Attachment,
    embedded_file: lopdf::ObjectId,
) -> lopdf::ObjectId {
    let mut ef = Dictionary::new();
    ef.set("F", Object::Reference(embedded_file));
    ef.set("UF", Object::Reference(embedded_file));

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Filespec".to_vec()));
    dict.set(
        "F",
        Object::String(att.name.as_bytes().to_vec(), StringFormat::Literal),
    );
    dict.set(
        "UF",
        Object::String(att.name.as_bytes().to_vec(), StringFormat::Literal),
    );
    dict.set("AFRelationship", Object::Name(b"Unspecified".to_vec()));
    dict.set("EF", Object::Dictionary(ef));
    doc.add_object(Object::Dictionary(dict))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    /// A minimal one-page PDF with a catalog, so `apply_attachments` has
    /// something to attach to.
    fn blank_pdf() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Page".to_vec()),
            "Parent" => Object::Reference(pages),
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });
        doc.set_object(
            pages,
            dictionary! {
                "Type" => Object::Name(b"Pages".to_vec()),
                "Kids" => vec![Object::Reference(page)], "Count" => 1,
            },
        );
        let cat = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Catalog".to_vec()), "Pages" => Object::Reference(pages),
        });
        doc.trailer.set("Root", Object::Reference(cat));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn embed_then_extract_round_trips() {
        let pdf = blank_pdf();
        let xml = b"<CrossIndustryInvoice/>".to_vec();
        let with = apply_attachments(
            &pdf,
            &[
                Attachment {
                    name: "factur-x.xml".into(),
                    mime: "text/xml".into(),
                    bytes: xml.clone(),
                },
                Attachment {
                    name: "notes.txt".into(),
                    mime: "text/plain".into(),
                    bytes: b"hello".to_vec(),
                },
            ],
        )
        .expect("embed");

        let all = extract_attachments(&with).expect("extract");
        assert_eq!(all.len(), 2, "both attachments read back");
        assert!(all
            .iter()
            .any(|a| a.name == "notes.txt" && a.bytes == b"hello"));

        let facturx = extract_facturx(&with).expect("extract facturx");
        assert_eq!(
            facturx.as_deref(),
            Some(xml.as_slice()),
            "invoice XML found by name"
        );
    }

    #[test]
    fn extract_facturx_none_without_invoice() {
        let pdf = blank_pdf();
        assert!(extract_facturx(&pdf).unwrap().is_none());
    }
}
