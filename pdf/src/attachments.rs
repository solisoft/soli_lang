//! Generic file attachments: embed arbitrary files (CSV/XML/JSON sources,
//! terms PDFs, …) into the document's `/EmbeddedFiles` name tree, the same
//! mechanism Factur-X uses for `factur-x.xml`.
//!
//! Applied as a lopdf post-pass (after `stationery`); the Factur-X step runs
//! later and merges with — rather than clobbers — entries added here, so
//! `attachments` + `pdf_facturx` compose. Each filespec carries
//! `AFRelationship /Unspecified` and is appended to the catalog's `/AF` array
//! so PDF/A-3 outputs keep every embedded file associated.

use lopdf::{Dictionary, Document, Object, Stream, StringFormat};
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
