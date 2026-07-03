//! Integration tests for tagged / accessible output (`options.tagged`).
//!
//! The backend wraps each text run in `/P <</MCID n>> BDC … EMC` and the
//! `accessibility` post-pass turns those marks into a `StructTreeRoot` +
//! `MarkInfo`/`ParentTree`/`Lang`. These tests assert the structure lands in
//! the saved bytes (full PDF/UA conformance can't be checked here — no veraPDF).

use std::time::Duration;

use lopdf::{Document, Object};
use soli_pdf::{render_to_bytes, render_with_warnings, RenderOptions};

fn opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
        ..Default::default()
    }
}

const TAGGED: &[u8] = br#"{
  "fonts": [],
  "options": { "tagged": true, "lang": "fr-FR" },
  "content": [
    { "type": "paragraph", "value": "Heading", "options": { "fontSize": 16, "fontWeight": "bold" } },
    { "type": "paragraph", "value": "Body one." },
    { "type": "hr", "color": "cccccc" },
    { "type": "paragraph", "value": "Body two." }
  ]
}"#;

fn catalog(doc: &Document) -> &lopdf::Dictionary {
    let root_ref = doc.trailer.get(b"Root").unwrap();
    let root_id = match root_ref {
        Object::Reference(id) => *id,
        _ => panic!("Root is not a reference"),
    };
    doc.get_object(root_id).unwrap().as_dict().unwrap()
}

#[test]
fn tagged_output_marks_the_catalog() {
    let pdf = render_to_bytes(TAGGED, b"{}", &opts()).expect("render");
    let doc = Document::load_mem(&pdf).expect("load");
    let cat = catalog(&doc);

    // /MarkInfo <</Marked true>>
    let mark_info = cat.get(b"MarkInfo").unwrap().as_dict().unwrap();
    assert!(mark_info.get(b"Marked").unwrap().as_bool().unwrap());

    // /StructTreeRoot present and typed.
    let str_ref = cat.get(b"StructTreeRoot").unwrap();
    let str_id = match str_ref {
        Object::Reference(id) => *id,
        _ => panic!("StructTreeRoot not a reference"),
    };
    let root = doc.get_object(str_id).unwrap().as_dict().unwrap();
    assert_eq!(
        root.get(b"Type").unwrap().as_name().unwrap(),
        b"StructTreeRoot"
    );
    assert!(root.get(b"ParentTree").is_ok());

    // /Lang carried through from the template option.
    let lang = cat.get(b"Lang").unwrap().as_str().unwrap();
    assert_eq!(lang, b"fr-FR");
}

#[test]
fn tagged_output_builds_one_struct_elem_per_text_run() {
    let pdf = render_to_bytes(TAGGED, b"{}", &opts()).expect("render");
    let doc = Document::load_mem(&pdf).expect("load");

    // Three paragraphs = three `/P` StructElems (the hr is an /Artifact, not
    // tagged). Count StructElems whose /S is /P.
    let p_elems = doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| {
            d.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"StructElem")
                && d.get(b"S").ok().and_then(|s| s.as_name().ok()) == Some(b"P")
        })
        .count();
    assert_eq!(p_elems, 3, "expected one /P element per paragraph");
}

#[test]
fn tagged_pages_get_structparents_and_tabs() {
    let pdf = render_to_bytes(TAGGED, b"{}", &opts()).expect("render");
    let doc = Document::load_mem(&pdf).expect("load");
    let page_id = doc.page_iter().next().expect("a page");
    let page = doc.get_object(page_id).unwrap().as_dict().unwrap();
    assert!(
        page.get(b"StructParents").is_ok(),
        "page needs /StructParents"
    );
    assert_eq!(page.get(b"Tabs").unwrap().as_name().unwrap(), b"S");
}

#[test]
fn untagged_output_has_no_struct_tree() {
    let untagged = br#"{ "fonts": [], "content": [ { "type": "paragraph", "value": "plain" } ] }"#;
    let pdf = render_to_bytes(untagged, b"{}", &opts()).expect("render");
    let doc = Document::load_mem(&pdf).expect("load");
    let cat = catalog(&doc);
    assert!(
        cat.get(b"MarkInfo").is_err(),
        "untagged render must not mark"
    );
    assert!(cat.get(b"StructTreeRoot").is_err());
}

#[test]
fn tagged_facturx_is_accessible_and_archival() {
    // A tagged Factur-X invoice is simultaneously accessible (PDF/UA-1),
    // archival (PDF/A-3b) and machine-readable (embedded EN 16931 XML) — the
    // whole point of unifying the tagging and PDF/A passes.
    use soli_pdf::{FacturxMetadata, Profile};
    let meta = FacturxMetadata::default();
    let xml = b"<xml/>";
    let pdf = soli_pdf::generate_facturx(TAGGED, b"{}", xml, Profile::En16931, &meta, &opts())
        .expect("tagged + facturx compose");
    let doc = Document::load_mem(&pdf).expect("load");
    assert!(
        catalog(&doc).get(b"StructTreeRoot").is_ok(),
        "structure tree preserved through the Factur-X pass"
    );
    let raw = String::from_utf8_lossy(&pdf);
    assert!(raw.contains("<pdfaid:part>3</pdfaid:part>"), "PDF/A-3b");
    assert!(raw.contains("<pdfuaid:part>1</pdfuaid:part>"), "PDF/UA-1");
    assert!(raw.contains("urn:factur-x"), "Factur-X extension schema");
}

/// Count StructElems whose `/S` is the given structure type.
fn count_role(doc: &Document, tag: &[u8]) -> usize {
    doc.objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| {
            d.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"StructElem")
                && d.get(b"S").ok().and_then(|s| s.as_name().ok()) == Some(tag)
        })
        .count()
}

#[test]
fn bookmarked_paragraphs_become_headings() {
    let tpl = br#"{ "fonts": [], "options": { "tagged": true },
      "content": [
        { "type": "paragraph", "value": "Title", "options": { "bookmark": "T", "bookmarkLevel": 1 } },
        { "type": "paragraph", "value": "Section", "options": { "bookmarkLevel": 2 } },
        { "type": "paragraph", "value": "Body copy." }
      ] }"#;
    let pdf = render_to_bytes(tpl, b"{}", &opts()).expect("render");
    let doc = Document::load_mem(&pdf).expect("load");
    assert_eq!(count_role(&doc, b"H1"), 1, "one H1");
    assert_eq!(count_role(&doc, b"H2"), 1, "one H2");
    assert_eq!(count_role(&doc, b"P"), 1, "one plain P");
}

// A 2x2 red square SVG — rasterises offline (no fetch), so it interns as a figure.
const IMG: &str = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><rect width='8' height='8' fill='red'/></svg>";

#[test]
fn image_with_alt_becomes_a_figure() {
    let tpl = format!(
        r#"{{ "fonts": [], "options": {{ "tagged": true }},
          "content": [ {{ "type": "image", "value": "{IMG}", "width": 20, "alt": "A red square" }} ] }}"#
    );
    let out = render_with_warnings(tpl.as_bytes(), b"{}", &opts()).expect("render");
    let doc = Document::load_mem(&out.pdf).expect("load");
    assert_eq!(count_role(&doc, b"Figure"), 1, "one Figure element");
    // The Figure carries the alt text as /Alt.
    let fig_alt = doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .find(|d| d.get(b"S").ok().and_then(|s| s.as_name().ok()) == Some(b"Figure"))
        .and_then(|d| d.get(b"Alt").ok())
        .and_then(|a| a.as_str().ok())
        .map(|b| b.to_vec());
    assert_eq!(fig_alt.as_deref(), Some(&b"A red square"[..]));
    // Providing alt means no missing-alt warning.
    assert!(!out.warnings.iter().any(|w| format!("{w}").contains("alt")));
}

#[test]
fn tagged_image_without_alt_warns() {
    let tpl = format!(
        r#"{{ "fonts": [], "options": {{ "tagged": true }},
          "content": [ {{ "type": "image", "value": "{IMG}", "width": 20 }} ] }}"#
    );
    let out = render_with_warnings(tpl.as_bytes(), b"{}", &opts()).expect("render");
    assert!(
        out.warnings.iter().any(|w| format!("{w}").contains("alt")),
        "a tagged image with no alt should warn: {:?}",
        out.warnings
    );
    // It's still emitted as a Figure (just without /Alt).
    let doc = Document::load_mem(&out.pdf).expect("load");
    assert_eq!(count_role(&doc, b"Figure"), 1);
}

#[test]
fn header_and_footer_text_are_artifacts_not_content() {
    let tpl = br#"{ "fonts": [], "options": { "tagged": true, "header_height": 20 },
      "header": [ { "type": "paragraph", "value": "Running header" } ],
      "footer": [ { "type": "paragraph", "value": "Running footer" } ],
      "content": [ { "type": "paragraph", "value": "The only body paragraph." } ] }"#;
    let pdf = render_to_bytes(tpl, b"{}", &opts()).expect("render");
    let doc = Document::load_mem(&pdf).expect("load");
    // Header/footer are pagination artifacts — only the body paragraph is a /P.
    assert_eq!(
        count_role(&doc, b"P"),
        1,
        "header/footer must not be tagged as P"
    );
}

#[test]
fn tagged_output_carries_pdfua_xmp() {
    let pdf = render_to_bytes(TAGGED, b"{}", &opts()).expect("render");
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        text.contains("pdfuaid"),
        "XMP should declare the PDF/UA namespace"
    );
    assert!(
        text.contains("part>1"),
        "XMP should declare pdfuaid:part = 1"
    );
}
