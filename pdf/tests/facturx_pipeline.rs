//! End-to-end: render the sample invoice and embed the Factur-X XML, then
//! assert the result is a structurally valid PDF/A-3b Factur-X file.

use std::time::Duration;

use lopdf::Object;
use soli_pdf::{facturx, render_with_warnings, FacturxMetadata, Profile, RenderOptions};

const TEMPLATE: &[u8] = include_bytes!("fixtures/template.json");
const DATA: &[u8] = include_bytes!("fixtures/data.json");
const XML: &[u8] = include_bytes!("fixtures/factur-x.xml");

fn offline_opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false, // deterministic; the sample logo is a remote URL
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()], // Titillium only (no CJK here)
        ..Default::default()
    }
}

#[test]
fn full_pipeline_produces_valid_facturx() {
    let rendered = render_with_warnings(TEMPLATE, DATA, &offline_opts()).expect("render");
    // Expected warnings: the skipped remote logo, and the CJK string in the
    // sample title (no CJK font in ./fonts — degrades gracefully).
    assert!(rendered.warnings.iter().all(|w| matches!(
        w,
        soli_pdf::RenderWarning::ImageSkipped { .. } | soli_pdf::RenderWarning::MissingGlyph { .. }
    )));

    let pdf = facturx::embed_facturx(
        &rendered.pdf,
        XML,
        Profile::En16931,
        &FacturxMetadata::default(),
    )
    .expect("embed facturx");

    let doc = lopdf::Document::load_mem(&pdf).expect("reparse");
    assert_eq!(doc.version, "1.7", "PDF/A-3 must be PDF 1.7");

    let catalog = doc.catalog().expect("catalog");
    assert!(catalog.has(b"AF"), "catalog has /AF");
    assert!(catalog.has(b"Names"), "catalog has /Names");
    assert!(catalog.has(b"OutputIntents"), "catalog has /OutputIntents");
    assert!(catalog.has(b"Metadata"), "catalog has /Metadata");

    // The embedded XML round-trips byte-for-byte.
    let embedded = find_embedded_file(&doc).expect("embedded file present");
    assert_eq!(embedded, XML, "embedded factur-x.xml matches input");

    // The XMP metadata declares PDF/A-3b + Factur-X EN 16931.
    let xmp = read_metadata(&doc).expect("xmp metadata");
    let xmp = String::from_utf8_lossy(&xmp);
    assert!(xmp.contains("<pdfaid:part>3</pdfaid:part>"));
    assert!(xmp.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
    assert!(xmp.contains("<fx:ConformanceLevel>EN 16931</fx:ConformanceLevel>"));
}

fn find_embedded_file(doc: &lopdf::Document) -> Option<Vec<u8>> {
    for (_, obj) in doc.objects.iter() {
        if let Object::Stream(s) = obj {
            if let Ok(Object::Name(t)) = s.dict.get(b"Type") {
                if t == b"EmbeddedFile" {
                    return Some(s.content.clone());
                }
            }
        }
    }
    None
}

fn read_metadata(doc: &lopdf::Document) -> Option<Vec<u8>> {
    let catalog = doc.catalog().ok()?;
    let id = catalog.get(b"Metadata").ok()?.as_reference().ok()?;
    if let Ok(Object::Stream(s)) = doc.get_object(id) {
        return Some(s.content.clone());
    }
    None
}

/// Sizes of every embedded `/FontFile2` (TrueType) program in the document.
fn embedded_truetype_sizes(doc: &lopdf::Document) -> Vec<usize> {
    let mut sizes = Vec::new();
    for (_, obj) in doc.objects.iter() {
        let Ok(dict) = obj.as_dict() else { continue };
        let Ok(ff) = dict.get(b"FontFile2") else {
            continue;
        };
        if let Ok(id) = ff.as_reference() {
            if let Ok(Object::Stream(s)) = doc.get_object(id) {
                sizes.push(s.content.len());
            }
        }
    }
    sizes
}

#[test]
fn embedded_fonts_are_subset() {
    // On-disk source faces (Titillium Regular ~62 KB, Bold ~58 KB).
    let regular = std::fs::metadata("fonts/TitilliumWeb-Regular.ttf")
        .map(|m| m.len() as usize)
        .expect("regular font present");
    let bold = std::fs::metadata("fonts/TitilliumWeb-Bold.ttf")
        .map(|m| m.len() as usize)
        .expect("bold font present");
    let smallest_source = regular.min(bold);

    let rendered = render_with_warnings(TEMPLATE, DATA, &offline_opts()).expect("render");
    let doc = lopdf::Document::load_mem(&rendered.pdf).expect("parse rendered pdf");

    let sizes = embedded_truetype_sizes(&doc);
    assert!(
        !sizes.is_empty(),
        "expected at least one embedded TrueType font"
    );
    // The sample only uses a few dozen distinct glyphs, so every embedded face
    // must be far smaller than even the smaller of the two source files.
    for size in &sizes {
        assert!(
            *size < smallest_source / 2,
            "embedded font {size} bytes not < half of source {smallest_source} bytes \
             (subsetting did not take effect)"
        );
    }
}

#[test]
fn profile_metadata_is_consistent() {
    assert_eq!(Profile::En16931.af_relationship(), "Alternative");
    assert_eq!(Profile::En16931.xmp_level(), "EN 16931");
    assert_eq!(Profile::Minimum.af_relationship(), "Data");
    assert_eq!(Profile::parse("EN 16931"), Some(Profile::En16931));
    assert_eq!(Profile::parse("basic_wl"), Some(Profile::BasicWl));
}
