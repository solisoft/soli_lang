//! The standalone `pdfa` render option: PDF/A-3b output without any Factur-X
//! payload, plus its incompatibility rules.

use std::time::Duration;

use lopdf::Object;
use soli_pdf::{
    generate_facturx, render_to_bytes, Attachment, EncryptOptions, FacturxMetadata, Profile,
    RenderOptions,
};

const TEMPLATE: &[u8] = include_bytes!("fixtures/template.json");
const DATA: &[u8] = include_bytes!("fixtures/data.json");
const XML: &[u8] = include_bytes!("fixtures/factur-x.xml");

fn offline_opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false, // deterministic; the sample logo is a remote URL
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
        pdfa: true,
        ..Default::default()
    }
}

fn read_metadata(doc: &lopdf::Document) -> Option<Vec<u8>> {
    let catalog = doc.catalog().ok()?;
    let id = catalog.get(b"Metadata").ok()?.as_reference().ok()?;
    if let Ok(Object::Stream(s)) = doc.get_object(id) {
        return Some(s.content.clone());
    }
    None
}

fn embedded_file_count(doc: &lopdf::Document) -> usize {
    doc.objects
        .values()
        .filter(|obj| {
            matches!(obj, Object::Stream(s)
                if matches!(s.dict.get(b"Type"), Ok(Object::Name(t)) if t == b"EmbeddedFile"))
        })
        .count()
}

#[test]
fn pdfa_flag_produces_pdfa_without_facturx() {
    let pdf = render_to_bytes(TEMPLATE, DATA, &offline_opts()).expect("render pdfa");
    let doc = lopdf::Document::load_mem(&pdf).expect("reparse");

    assert_eq!(doc.version, "1.7", "PDF/A-3 must be PDF 1.7");
    let catalog = doc.catalog().expect("catalog");
    assert!(catalog.has(b"OutputIntents"), "catalog has /OutputIntents");
    assert!(catalog.has(b"Metadata"), "catalog has /Metadata");
    assert!(!catalog.has(b"AF"), "no associated files without attachments");

    let xmp = read_metadata(&doc).expect("xmp metadata");
    let xmp = String::from_utf8_lossy(&xmp);
    assert!(xmp.contains("<pdfaid:part>3</pdfaid:part>"));
    assert!(xmp.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
    assert!(!xmp.contains("fx:"), "no Factur-X namespace in plain PDF/A");
    assert!(!xmp.contains("urn:factur-x"));

    assert_eq!(embedded_file_count(&doc), 0, "no embedded factur-x.xml");
}

#[test]
fn pdfa_with_attachments_keeps_embedded_files() {
    let opts = RenderOptions {
        attachments: vec![Attachment {
            name: "data.csv".to_string(),
            mime: "text/csv".to_string(),
            bytes: b"a;b\n1;2\n".to_vec(),
        }],
        ..offline_opts()
    };
    let pdf = render_to_bytes(TEMPLATE, DATA, &opts).expect("render pdfa + attachment");
    let doc = lopdf::Document::load_mem(&pdf).expect("reparse");

    let catalog = doc.catalog().expect("catalog");
    assert!(catalog.has(b"AF"), "attachment /AF entry survives the pdfa pass");
    assert_eq!(embedded_file_count(&doc), 1, "the attachment is embedded");

    let xmp = read_metadata(&doc).expect("xmp metadata");
    let xmp = String::from_utf8_lossy(&xmp);
    assert!(xmp.contains("<pdfaid:part>3</pdfaid:part>"));
    assert!(!xmp.contains("fx:"));
}

#[test]
fn pdfa_rejects_encrypt() {
    let opts = RenderOptions {
        encrypt: Some(EncryptOptions {
            user_password: "secret".to_string(),
            owner_password: String::new(),
            allow: Vec::new(),
        }),
        ..offline_opts()
    };
    let err = render_to_bytes(TEMPLATE, DATA, &opts).expect_err("pdfa + encrypt must fail");
    assert!(err.to_string().contains("encryption is incompatible with PDF/A"));
}

#[test]
fn pdfa_rejects_tagged_template() {
    let template = br#"{
        "options": { "tagged": true },
        "content": [ { "type": "paragraph", "value": "hello" } ]
    }"#;
    let err =
        render_to_bytes(template, b"{}", &offline_opts()).expect_err("pdfa + tagged must fail");
    assert!(err.to_string().contains("tagged"));
}

#[test]
fn facturx_rejects_pdfa_option() {
    let err = generate_facturx(
        TEMPLATE,
        DATA,
        XML,
        Profile::En16931,
        &FacturxMetadata::default(),
        &offline_opts(),
    )
    .expect_err("facturx + pdfa option must fail");
    assert!(err.to_string().contains("implied by Factur-X"));
}
