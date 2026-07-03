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
    let catalog = doc.catalog().expect("catalog");
    let acro = catalog
        .get(b"AcroForm")
        .expect("AcroForm")
        .as_dict()
        .expect("AcroForm dict");
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
