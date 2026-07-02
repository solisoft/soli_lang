//! Integration tests for tagged / accessible output (`options.tagged`).
//!
//! The backend wraps each text run in `/P <</MCID n>> BDC … EMC` and the
//! `accessibility` post-pass turns those marks into a `StructTreeRoot` +
//! `MarkInfo`/`ParentTree`/`Lang`. These tests assert the structure lands in
//! the saved bytes (full PDF/UA conformance can't be checked here — no veraPDF).

use std::time::Duration;

use lopdf::{Document, Object};
use soli_pdf::{render_to_bytes, RenderOptions};

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
fn tagged_is_rejected_for_facturx() {
    use soli_pdf::{FacturxMetadata, Profile};
    let meta = FacturxMetadata::default();
    let xml = b"<xml/>";
    let err = soli_pdf::generate_facturx(TAGGED, b"{}", xml, Profile::En16931, &meta, &opts())
        .expect_err("tagged + facturx must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("incompatible with Factur-X"),
        "unexpected error: {msg}"
    );
}
