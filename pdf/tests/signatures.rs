//! Byte-mechanics tests for the signature placeholder pass (`sign.rs`). These
//! exercise the crypto-free half: reserving a signature dictionary, finalizing
//! the `/ByteRange`, and splicing a (here dummy) CMS blob — verifying the two
//! invariants everything downstream relies on: the ByteRange covers the whole
//! file except the `/Contents` hex, and `embed_cms` never changes a byte offset.

use std::time::Duration;

use lopdf::Document;
use soli_pdf::{prepare_signature, render_to_bytes, RenderOptions, SignMeta};

fn opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
        ..Default::default()
    }
}

fn simple_pdf() -> Vec<u8> {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Signed document" }
    ] }"#;
    render_to_bytes(tmpl, b"{}", &opts()).expect("render")
}

fn meta() -> SignMeta {
    SignMeta {
        reason: Some("Invoice issued".into()),
        location: Some("Paris, FR".into()),
        name: Some("ACME SARL".into()),
        signing_time: Some("D:20260703120000+00'00'".into()),
        ..Default::default()
    }
}

/// The catalog's `/AcroForm` dictionary (signing stores it as its own object).
fn acroform(doc: &Document) -> &lopdf::Dictionary {
    let obj = doc.catalog().unwrap().get(b"AcroForm").expect("AcroForm");
    match obj {
        lopdf::Object::Reference(id) => doc.get_object(*id).unwrap().as_dict().unwrap(),
        other => other.as_dict().expect("AcroForm dict"),
    }
}

/// Read the four `/ByteRange` integers back out of the serialized bytes.
fn parse_byte_range(pdf: &[u8]) -> [usize; 4] {
    let key = b"/ByteRange";
    let at = pdf
        .windows(key.len())
        .position(|w| w == key)
        .expect("/ByteRange present");
    let open = at
        + key.len()
        + pdf[at + key.len()..]
            .iter()
            .position(|&b| b == b'[')
            .unwrap();
    let close = open + pdf[open..].iter().position(|&b| b == b']').unwrap();
    let inner = std::str::from_utf8(&pdf[open + 1..close]).unwrap();
    let nums: Vec<usize> = inner
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    [nums[0], nums[1], nums[2], nums[3]]
}

#[test]
fn prepare_reserves_a_signature_field() {
    let pdf = simple_pdf();
    let prepared = prepare_signature(&pdf, &meta(), 4096).expect("prepare");

    let doc = Document::load_mem(&prepared.pdf).expect("reload");
    let acro = acroform(&doc);
    assert_eq!(
        acro.get(b"SigFlags").unwrap().as_i64().unwrap(),
        3,
        "SigFlags must be SignaturesExist|AppendOnly"
    );
    let fields = acro.get(b"Fields").unwrap().as_array().unwrap();
    assert_eq!(fields.len(), 1, "exactly one signature field");
}

#[test]
fn byte_range_covers_everything_but_the_contents() {
    let pdf = simple_pdf();
    let prepared = prepare_signature(&pdf, &meta(), 4096).expect("prepare");
    let bytes = &prepared.pdf;

    // The four integers we report match what got written into the file.
    assert_eq!(parse_byte_range(bytes), prepared.byte_range);

    let [s1, l1, s2, l2] = prepared.byte_range;
    assert_eq!(s1, 0, "first range starts at the file head");
    // The gap between the two ranges is exactly the reserved hex, bracketed by
    // '<' (last byte of range 1) and '>' (first byte of range 2).
    assert_eq!(bytes[l1 - 1], b'<', "range 1 ends on the '<'");
    assert_eq!(bytes[s2], b'>', "range 2 starts on the '>'");
    assert_eq!(s2 - l1, 4096 * 2, "the gap is the reserved hex placeholder");
    assert_eq!(s2 + l2, bytes.len(), "second range runs to EOF");

    // Every byte between the ranges is a zeroed hex digit.
    assert!(bytes[l1..s2].iter().all(|&b| b == b'0'));
}

#[test]
fn embed_cms_is_length_preserving_and_splices_hex() {
    let pdf = simple_pdf();
    let prepared = prepare_signature(&pdf, &meta(), 4096).expect("prepare");
    let before_len = prepared.pdf.len();
    let range_before = prepared.byte_range;
    let hex_offset = prepared.contents_hex_offset;

    // A dummy "CMS" blob — the byte mechanics don't care what it is.
    let fake_cms: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8).collect();
    let signed = soli_pdf::embed_cms(prepared, &fake_cms).expect("embed");

    assert_eq!(
        signed.len(),
        before_len,
        "splice must not change file length"
    );
    // ByteRange is untouched by the splice.
    assert_eq!(parse_byte_range(&signed), range_before);
    // The hex of the blob is present at the reserved offset, trailing zeros kept.
    let expect_hex: String = fake_cms.iter().map(|b| format!("{:02x}", b)).collect();
    assert_eq!(
        &signed[hex_offset..hex_offset + expect_hex.len()],
        expect_hex.as_bytes()
    );
    assert_eq!(signed[hex_offset + expect_hex.len()], b'0', "padding kept");
    // Still a loadable PDF.
    Document::load_mem(&signed).expect("reload signed");
}

#[test]
fn oversize_cms_is_rejected() {
    let pdf = simple_pdf();
    let prepared = prepare_signature(&pdf, &meta(), 64).expect("prepare");
    // 64-byte placeholder = 128 hex chars; a 100-byte blob needs 200.
    let too_big: Vec<u8> = vec![0xAB; 100];
    let err = soli_pdf::embed_cms(prepared, &too_big).unwrap_err();
    assert!(err.to_string().contains("reserved"), "got: {err}");
}

#[test]
fn visible_appearance_places_a_block() {
    use soli_pdf::SignAppearance;
    let pdf = simple_pdf();
    let mut m = meta();
    m.appearance = Some(SignAppearance {
        page: 1,
        x: 360.0,
        y: 40.0,
        width: 190.0,
        height: 64.0,
        image: None,
    });
    let prepared = prepare_signature(&pdf, &m, 4096).expect("prepare");
    let doc = Document::load_mem(&prepared.pdf).expect("reload");

    // The AcroForm's single widget is now visible (non-zero Rect) and carries
    // an /AP appearance stream.
    let acro = acroform(&doc);
    let field_ref = acro.get(b"Fields").unwrap().as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let field = doc.get_object(field_ref).unwrap().as_dict().unwrap();
    let rect = field.get(b"Rect").unwrap().as_array().unwrap();
    let x1 = rect[2].as_float().unwrap();
    assert!(x1 > 0.0, "widget has a visible rect");
    assert!(field.get(b"AP").is_ok(), "widget has an appearance stream");
}

#[test]
fn extract_signatures_reads_back_the_signed_bytes() {
    // Sign a rendered PDF, then extract the signature the way pdf_verify does.
    let pdf = simple_pdf();
    let prepared = prepare_signature(&pdf, &meta(), 4096).expect("prepare");
    // A throwaway CMS blob (verification of the crypto lives in the solilang
    // pades tests); here we only exercise the byte extraction.
    let signed = soli_pdf::embed_cms(prepared, &[0x30, 0x03, 0x02, 0x01, 0x2a]).expect("embed");

    let sigs = soli_pdf::extract_signatures(&signed).expect("extract");
    assert_eq!(sigs.len(), 1, "one signature field");
    let s = &sigs[0];
    assert_eq!(s.field, "Signature1");
    assert!(s.covers_document, "ByteRange reaches EOF");
    assert!(s.cms_der.starts_with(&[0x30]), "CMS is a DER SEQUENCE");
    assert!(!s.signed_bytes.is_empty());
}

#[test]
fn extract_signatures_empty_on_unsigned_pdf() {
    let pdf = simple_pdf();
    assert!(soli_pdf::extract_signatures(&pdf).unwrap().is_empty());
}

/// A dummy CMS DER (a SEQUENCE): the byte mechanics don't care what it is.
const FAKE_CMS: [u8; 5] = [0x30, 0x03, 0x02, 0x01, 0x2a];

fn sign_dummy(pdf: &[u8], meta: &SignMeta) -> Vec<u8> {
    let prepared = prepare_signature(pdf, meta, 4096).expect("prepare");
    soli_pdf::embed_cms(prepared, &FAKE_CMS).expect("embed")
}

#[test]
fn a_second_signature_is_an_incremental_update() {
    let once = sign_dummy(&simple_pdf(), &meta());
    let twice = sign_dummy(&once, &meta());

    assert!(
        twice.starts_with(&once),
        "the signed revision is kept verbatim"
    );
    let sigs = soli_pdf::extract_signatures(&twice).expect("extract");
    assert_eq!(sigs.len(), 2);
    let first = sigs
        .iter()
        .find(|s| s.field == "Signature1")
        .expect("first");
    let second = sigs
        .iter()
        .find(|s| s.field == "Signature2")
        .expect("second");
    // The first signature's bytes are exactly the first revision minus its hex.
    let [_, l1, s2, _] = parse_byte_range(&once);
    assert_eq!(first.signed_bytes.len(), l1 + (once.len() - s2));
    assert!(!first.covers_document);
    assert!(second.covers_document);
    Document::load_mem(&twice).expect("reload");
}

#[test]
fn incremental_flag_appends_even_on_an_unsigned_pdf() {
    let pdf = simple_pdf();
    let mut m = meta();
    m.incremental = true;
    let signed = sign_dummy(&pdf, &m);
    assert!(signed.starts_with(&pdf), "original bytes kept");
    assert_eq!(soli_pdf::extract_signatures(&signed).unwrap().len(), 1);
}

/// Rewrite `pdf` with an xref *stream* and object streams, the way many
/// producers (and `qpdf --object-streams=generate`) emit PDF 1.5+.
fn modern(pdf: &[u8]) -> Vec<u8> {
    let mut doc = Document::load_mem(pdf).expect("load");
    let mut out = Vec::new();
    doc.save_modern(&mut out).expect("save modern");
    out
}

#[test]
fn incremental_update_over_an_xref_stream_file() {
    let base = modern(&simple_pdf());
    let once = sign_dummy(&base, &meta());
    let twice = sign_dummy(&once, &meta());
    assert!(twice.starts_with(&once));
    let doc = Document::load_mem(&twice).expect("reload");
    assert!(doc.get_pages().len() == 1, "page tree still resolves");
    assert_eq!(soli_pdf::extract_signatures(&twice).unwrap().len(), 2);
}

/// A PDF with one empty signature field named `Client` placed on page 1.
fn with_placed_field() -> Vec<u8> {
    let mut doc = Document::load_mem(&simple_pdf()).expect("load");
    let page = *doc.get_pages().values().next().unwrap();
    let mut field = lopdf::Dictionary::new();
    field.set("Type", lopdf::Object::Name(b"Annot".to_vec()));
    field.set("Subtype", lopdf::Object::Name(b"Widget".to_vec()));
    field.set("FT", lopdf::Object::Name(b"Sig".to_vec()));
    field.set("T", lopdf::Object::string_literal("Client"));
    field.set(
        "Rect",
        lopdf::Object::Array(vec![40.into(), 40.into(), 240.into(), 100.into()]),
    );
    field.set("P", lopdf::Object::Reference(page));
    let fid = doc.add_object(lopdf::Object::Dictionary(field));
    let mut acro = lopdf::Dictionary::new();
    acro.set(
        "Fields",
        lopdf::Object::Array(vec![lopdf::Object::Reference(fid)]),
    );
    doc.catalog_mut()
        .unwrap()
        .set("AcroForm", lopdf::Object::Dictionary(acro));
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Annots",
            lopdf::Object::Array(vec![lopdf::Object::Reference(fid)]),
        );
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn signing_fills_a_preplaced_field_once() {
    let mut m = meta();
    m.field = Some("Client".into());
    let signed = sign_dummy(&with_placed_field(), &m);

    let sigs = soli_pdf::extract_signatures(&signed).expect("extract");
    assert_eq!(sigs.len(), 1, "the placed field was filled, not duplicated");
    assert_eq!(sigs[0].field, "Client");

    // The pre-placed widget got a visible appearance in its own rectangle.
    let doc = Document::load_mem(&signed).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let annots = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(annots.len(), 1);
    let widget = doc
        .get_object(annots[0].as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    assert!(widget.get(b"AP").is_ok(), "appearance drawn into the field");

    let err = prepare_signature(&signed, &m, 4096).err().expect("refused");
    assert!(err.to_string().contains("already signed"), "got: {err}");
}

#[test]
fn appearance_image_is_embedded_with_its_transparency() {
    use soli_pdf::SignAppearance;
    // A 4x2 PNG with a transparent column.
    let mut img = image::RgbaImage::new(4, 2);
    for (x, _, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba([10, 20, 200, if x == 0 { 0 } else { 255 }]);
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let mut m = meta();
    m.appearance = Some(SignAppearance {
        page: 1,
        x: 40.0,
        y: 40.0,
        width: 160.0,
        height: 60.0,
        image: Some(png),
    });
    let signed = sign_dummy(&simple_pdf(), &m);
    let doc = Document::load_mem(&signed).unwrap();
    let images: Vec<_> = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.get(b"Subtype").and_then(|n| n.as_name()).ok() == Some(b"Image"))
        .collect();
    assert_eq!(images.len(), 2, "the RGB image and its soft mask");
    assert!(
        images.iter().any(|s| s.dict.get(b"SMask").is_ok()),
        "transparency kept as /SMask"
    );
}

#[test]
fn a_garbage_appearance_image_is_an_error() {
    use soli_pdf::SignAppearance;
    let mut m = meta();
    m.appearance = Some(SignAppearance {
        page: 1,
        width: 100.0,
        height: 40.0,
        image: Some(b"not an image".to_vec()),
        ..Default::default()
    });
    let err = prepare_signature(&simple_pdf(), &m, 4096)
        .err()
        .expect("refused");
    assert!(err.to_string().contains("appearance image"), "got: {err}");
}
