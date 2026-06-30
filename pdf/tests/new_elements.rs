//! Integration tests for the newer template elements: SVG image embedding,
//! 1D barcodes, block lists, and charts. Each asserts the feature reaches the
//! laid-out draw model through the full template → layout pipeline.

use std::path::PathBuf;
use std::time::Duration;

use soli_pdf::data::DataDocument;
use soli_pdf::draw::{DrawOp, LaidOutDoc};
use soli_pdf::fonts::FontRegistry;
use soli_pdf::layout::Engine;
use soli_pdf::template::Template;
use soli_pdf::{RenderOptions, RenderWarning};

fn opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
    }
}

/// Lay out a template against the given data, returning the whole document and
/// the warnings.
fn render(template_json: &[u8], data_json: &[u8]) -> (LaidOutDoc, Vec<RenderWarning>) {
    let t = Template::parse(template_json).expect("template");
    let fonts = FontRegistry::from_font_dirs(&[PathBuf::from("fonts")], &t.fonts).expect("fonts");
    let data = DataDocument::parse(data_json).expect("data");
    let o = opts();
    Engine::new(&t, &fonts, &o)
        .layout(&t, &data)
        .expect("layout")
}

fn all_ops(doc: &LaidOutDoc) -> Vec<DrawOp> {
    doc.pages.iter().flat_map(|p| p.ops.clone()).collect()
}

/// The concatenated text of a plain `Text` op, if it is one.
fn text_of(op: &DrawOp) -> Option<String> {
    match op {
        DrawOp::Text(td) => Some(td.pieces.iter().map(|p| p.text.as_str()).collect()),
        _ => None,
    }
}

// --- SVG ---------------------------------------------------------------------

#[test]
fn svg_image_is_decoded_and_embedded() {
    // An inline SVG data-URI (text-free, so no fonts are needed to rasterise).
    let tmpl = br##"{ "fonts": ["titillium"], "content": [
        { "type": "image", "width": 80,
          "value": "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='40' height='40'><rect width='40' height='40' fill='%23123456'/></svg>" }
    ] }"##;
    let (doc, warnings) = render(tmpl, b"{}");
    let images = ops_of(&doc, |o| matches!(o, DrawOp::Image { .. }));
    assert_eq!(images, 1, "the SVG produced one image op");
    assert_eq!(doc.images.len(), 1, "one decoded image");
    assert!(doc.images[0].has_alpha, "SVG rasterises to RGBA");
    assert!(!doc.images[0].pixels.is_empty(), "non-empty raster");
    assert!(
        warnings.is_empty(),
        "no warnings for a valid SVG: {warnings:?}"
    );
}

#[test]
fn svg_text_renders_with_a_font_from_font_dirs() {
    // Black text on a transparent canvas (no background rect): if the named
    // family resolves from `font_dirs`, the glyphs paint opaque near-black
    // pixels; if it does not, the raster stays fully transparent.
    let tmpl = br##"{ "fonts": ["titillium"], "content": [
        { "type": "image", "width": 160,
          "value": "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='160' height='40'><text x='4' y='30' font-family='Titillium Web' font-size='28' fill='black'>Soli</text></svg>" }
    ] }"##;
    let (doc, _) = render(tmpl, b"{}");
    assert_eq!(doc.images.len(), 1);
    let dark = doc.images[0]
        .pixels
        .chunks_exact(4)
        .filter(|px| px[3] > 200 && px[0] < 60 && px[1] < 60 && px[2] < 60)
        .count();
    assert!(
        dark > 0,
        "SVG <text> rendered glyphs using a font_dirs font"
    );
}

// --- Barcode -----------------------------------------------------------------

#[test]
fn barcode_emits_image_and_caption() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "barcode", "symbology": "code128", "value": "SOLI-42",
          "width": 200, "height": 50, "humanReadable": true }
    ] }"#;
    let (doc, warnings) = render(tmpl, b"{}");
    assert_eq!(
        ops_of(&doc, |o| matches!(o, DrawOp::Image { .. })),
        1,
        "barcode raster image"
    );
    assert_eq!(doc.images.len(), 1);
    // The human-readable caption is a plain text op carrying the value.
    let ops = all_ops(&doc);
    assert!(
        ops.iter().filter_map(text_of).any(|t| t == "SOLI-42"),
        "caption text present"
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn invalid_barcode_is_skipped_with_warning() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "barcode", "symbology": "ean13", "value": "nope" }
    ] }"#;
    let (doc, warnings) = render(tmpl, b"{}");
    assert_eq!(
        ops_of(&doc, |o| matches!(o, DrawOp::Image { .. })),
        0,
        "no image for invalid data"
    );
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, RenderWarning::ElementSkipped { kind, .. } if kind == "barcode")),
        "an ElementSkipped(barcode) warning: {warnings:?}"
    );
}

// --- List --------------------------------------------------------------------

#[test]
fn ordered_list_numbers_items() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "list", "ordered": true, "items": ["Alpha", "Beta", "Gamma"] }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let texts: Vec<String> = all_ops(&doc).iter().filter_map(text_of).collect();
    for marker in ["1.", "2.", "3."] {
        assert!(texts.iter().any(|t| t == marker), "marker {marker} present");
    }
    for body in ["Alpha", "Beta", "Gamma"] {
        assert!(texts.iter().any(|t| t == body), "body {body} present");
    }
}

#[test]
fn nested_list_indents_deeper() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "list", "items": [
            { "text": "Parent", "list": { "items": ["Child"] } }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    // Collect the x of the "Parent" and "Child" body text ops.
    let mut parent_x = None;
    let mut child_x = None;
    for op in all_ops(&doc) {
        if let DrawOp::Text(td) = &op {
            let t: String = td.pieces.iter().map(|p| p.text.as_str()).collect();
            if t == "Parent" {
                parent_x = Some(td.x);
            } else if t == "Child" {
                child_x = Some(td.x);
            }
        }
    }
    let (p, c) = (parent_x.expect("parent"), child_x.expect("child"));
    assert!(c > p, "nested item ({c}) indented past its parent ({p})");
}

// --- Chart -------------------------------------------------------------------

#[test]
fn data_bound_bar_chart_one_bar_per_point() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "chart", "kind": "bar", "data": "months", "label": "name", "value": "v",
          "width": 300, "height": 150 }
    ] }"#;
    let data = br#"{ "months": [
        { "name": "Jan", "v": 10 }, { "name": "Feb", "v": 25 }, { "name": "Mar", "v": 15 }
    ] }"#;
    let (doc, _) = render(tmpl, data);
    assert_eq!(
        ops_of(&doc, |o| matches!(o, DrawOp::FillRect { .. })),
        3,
        "one bar per data point"
    );
}

#[test]
fn inline_pie_chart_one_slice_per_point() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "chart", "kind": "pie", "legend": false, "width": 300, "height": 150, "points": [
            { "label": "A", "value": 3 }, { "label": "B", "value": 1 }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    assert_eq!(
        ops_of(&doc, |o| matches!(o, DrawOp::Polygon { .. })),
        2,
        "one wedge per slice"
    );
}

#[test]
fn empty_chart_is_skipped_with_warning() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "chart", "kind": "bar", "data": "missing" }
    ] }"#;
    let (_, warnings) = render(tmpl, b"{}");
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, RenderWarning::ElementSkipped { kind, .. } if kind == "chart")),
        "an ElementSkipped(chart) warning: {warnings:?}"
    );
}

// --- Polish bundle: paragraph color + page background ------------------------

#[test]
fn plain_paragraph_honors_options_color() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Teal", "options": { "color": "0f766e" } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let teal = soli_pdf::color::parse_hex("0f766e").unwrap();
    let colored = all_ops(&doc)
        .iter()
        .any(|op| matches!(op, DrawOp::Text(td) if td.color == teal));
    assert!(colored, "plain paragraph text uses options.color");
}

#[test]
fn page_background_is_leading_full_page_fill() {
    let tmpl = br#"{ "fonts": ["titillium"], "options": { "background": "f0fdfa" },
        "content": [ { "type": "paragraph", "value": "x" } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let bg = soli_pdf::color::parse_hex("f0fdfa").unwrap();
    match &doc.pages[0].ops[0] {
        DrawOp::FillRect { x, y, w, h, color } => {
            assert_eq!((*x, *y), (0.0, 0.0), "background fill starts at the origin");
            assert!(*w > 0.0 && *h > 0.0, "background fill covers the page");
            assert_eq!(*color, bg, "background fill uses options.background");
        }
        other => panic!("first op should be the page-background fill, got {other:?}"),
    }
}

#[test]
fn behind_watermark_sits_above_the_page_background() {
    // With a background, a behind-content watermark must be op[1] (over the bg),
    // not op[0] (under it).
    let tmpl = br#"{ "fonts": ["titillium"],
        "options": { "background": "f0fdfa", "watermark": { "text": "DRAFT" } },
        "content": [ { "type": "paragraph", "value": "x" } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    assert!(
        matches!(doc.pages[0].ops[0], DrawOp::FillRect { .. }),
        "bg first"
    );
    assert!(
        matches!(doc.pages[0].ops[1], DrawOp::RotatedText { .. }),
        "watermark sits just above the background"
    );
}

// --- Control flow: repeat + if/unless ----------------------------------------

#[test]
fn repeat_renders_content_per_item_scoped() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "repeat", "data": "rows", "content": [
            { "type": "paragraph", "value": "${n}" }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, br#"{ "rows": [ {"n":"A"}, {"n":"B"}, {"n":"C"} ] }"#);
    let texts: Vec<String> = all_ops(&doc).iter().filter_map(text_of).collect();
    for t in ["A", "B", "C"] {
        assert!(
            texts.iter().any(|x| x == t),
            "repeat scopes ${{n}} per item: {t}"
        );
    }
}

#[test]
fn repeat_missing_array_renders_nothing() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "repeat", "data": "nope", "content": [ { "type": "paragraph", "value": "x" } ] }
    ] }"#;
    let (doc, warnings) = render(tmpl, b"{}");
    assert!(
        all_ops(&doc).iter().filter_map(text_of).next().is_none(),
        "a missing array renders nothing"
    );
    assert!(warnings.is_empty());
}

#[test]
fn if_and_unless_select_branches() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "if", "when": "flag", "equals": "on",
          "content": [ { "type": "paragraph", "value": "SHOWN" } ],
          "else": [ { "type": "paragraph", "value": "ELSE" } ] },
        { "type": "unless", "when": "flag", "equals": "on",
          "content": [ { "type": "paragraph", "value": "UNLESS" } ] }
    ] }"#;
    let (doc, _) = render(tmpl, br#"{ "flag": "on" }"#);
    let texts: Vec<String> = all_ops(&doc).iter().filter_map(text_of).collect();
    assert!(
        texts.iter().any(|t| t == "SHOWN"),
        "if-true renders content"
    );
    assert!(!texts.iter().any(|t| t == "ELSE"), "if-true skips else");
    assert!(
        !texts.iter().any(|t| t == "UNLESS"),
        "unless-true skips content"
    );
}

#[test]
fn if_truthiness_treats_false_zero_empty_as_falsy() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "if", "when": "v",
          "content": [ { "type": "paragraph", "value": "YES" } ],
          "else": [ { "type": "paragraph", "value": "NO" } ] }
    ] }"#;
    for (data, expect) in [
        (&br#"{ "v": true }"#[..], "YES"),
        (&br#"{ "v": false }"#[..], "NO"),
        (&br#"{ "v": 0 }"#[..], "NO"),
        (&br#"{ "v": 5 }"#[..], "YES"),
        (&br#"{}"#[..], "NO"),
    ] {
        let (doc, _) = render(tmpl, data);
        let texts: Vec<String> = all_ops(&doc).iter().filter_map(text_of).collect();
        assert!(
            texts.iter().any(|t| t == expect),
            "data {data:?} should pick {expect}"
        );
    }
}

// --- Multi-series charts -----------------------------------------------------

#[test]
fn multi_series_data_bound_grouped_bars() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "chart", "kind": "bar", "data": "q", "label": "name", "legend": false,
          "values": [ { "field": "a" }, { "field": "b" } ], "width": 300, "height": 150 }
    ] }"#;
    let data = br#"{ "q": [ {"name":"Q1","a":1,"b":2}, {"name":"Q2","a":3,"b":4} ] }"#;
    let (doc, _) = render(tmpl, data);
    assert_eq!(
        ops_of(&doc, |o| matches!(o, DrawOp::FillRect { .. })),
        4,
        "2 series × 2 categories = 4 bars"
    );
}

/// Count draw ops matching a predicate.
fn ops_of(doc: &LaidOutDoc, pred: impl Fn(&DrawOp) -> bool) -> usize {
    all_ops(doc).iter().filter(|o| pred(o)).count()
}
