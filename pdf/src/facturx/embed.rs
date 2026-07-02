//! lopdf operations that turn a rendered PDF into a PDF/A-3b Factur-X file:
//! apply the shared PDF/A baseline (see [`super::pdfa`]), embed `factur-x.xml`,
//! and wire up the `/AF` + `/EmbeddedFiles` name tree.

use lopdf::{Dictionary, Document, Object, Stream, StringFormat};
use md5::{Digest, Md5};

use super::pdfa::{apply_pdfa_base, pdf_date};
use super::xmp;
use super::{FacturxMetadata, Profile};
use crate::error::{PdfError, Result};

const FILENAME: &str = "factur-x.xml";

/// Perform the embedding on a parsed document, returning PDF/A-3b bytes.
pub fn embed(
    mut doc: Document,
    xml: &[u8],
    profile: Profile,
    meta: &FacturxMetadata,
) -> Result<Vec<u8>> {
    apply_pdfa_base(&mut doc, xmp::build(Some(profile), meta), meta)?;

    let embedded_file = add_embedded_file(&mut doc, xml, meta);
    let filespec = add_filespec(&mut doc, embedded_file, profile);

    // Catalog wiring (done after objects exist so we can reference them).
    {
        let catalog = doc
            .catalog_mut()
            .map_err(|e| PdfError::Facturx(format!("no catalog: {e}")))?;

        // /AF [ … filespec ] — the Factur-X spec entry comes FIRST, appended
        // ahead of any generic attachments already associated (the render's
        // `attachments` post-pass runs before this step and must survive it).
        let mut af: Vec<Object> = vec![Object::Reference(filespec)];
        if let Some(existing) = catalog.get(b"AF").ok().and_then(|o| o.as_array().ok()) {
            af.extend(existing.iter().cloned());
        }
        catalog.set("AF", Object::Array(af));

        // /Names << /EmbeddedFiles << /Names [ … (factur-x.xml) filespec … ] >> >>
        // Merged (and re-sorted — name trees are ordered) with any entries the
        // generic-attachments pass wrote, instead of clobbering them.
        let mut pairs: Vec<(Vec<u8>, Object)> = Vec::new();
        if let Some(arr) = catalog
            .get(b"Names")
            .ok()
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"EmbeddedFiles").ok())
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"Names").ok())
            .and_then(|o| o.as_array().ok())
        {
            for pair in arr.chunks_exact(2) {
                if let Object::String(name, _) = &pair[0] {
                    pairs.push((name.clone(), pair[1].clone()));
                }
            }
        }
        pairs.push((FILENAME.as_bytes().to_vec(), Object::Reference(filespec)));
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let mut names_array: Vec<Object> = Vec::with_capacity(pairs.len() * 2);
        for (name, spec) in pairs {
            names_array.push(Object::String(name, StringFormat::Literal));
            names_array.push(spec);
        }
        let mut embedded_files = Dictionary::new();
        embedded_files.set("Names", Object::Array(names_array));
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

        catalog.set("PageMode", Object::Name(b"UseAttachments".to_vec()));
    }

    let mut buf = Vec::new();
    doc.save_to(&mut buf)
        .map_err(|e| PdfError::Facturx(format!("save: {e}")))?;
    Ok(buf)
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
