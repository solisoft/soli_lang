//! Integration tests for the raw-PDF-generation features: vector shapes, the
//! payment QR, clickable hyperlinks, and the watermark stamp. Each asserts the
//! feature reaches the laid-out draw model and/or the final PDF bytes.

use std::path::PathBuf;
use std::time::Duration;

use lopdf::Object;
use soli_pdf::data::DataDocument;
use soli_pdf::draw::DrawOp;
use soli_pdf::fonts::FontRegistry;
use soli_pdf::layout::Engine;
use soli_pdf::template::Template;
use soli_pdf::{
    facturx, generate_facturx_from_invoice, render_with_warnings, FacturxMetadata, Invoice,
    Profile, RenderOptions,
};

fn opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
        ..Default::default()
    }
}

/// Lay out a template against empty data and return the first page's draw ops.
fn lay_out(template_json: &[u8]) -> Vec<DrawOp> {
    let t = Template::parse(template_json).expect("template");
    let fonts = FontRegistry::from_font_dirs(&[PathBuf::from("fonts")], &t.fonts).expect("fonts");
    let o = opts();
    let (doc, _w) = Engine::new(&t, &fonts, &o)
        .layout(&t, &DataDocument::empty())
        .expect("layout");
    doc.pages.into_iter().flat_map(|p| p.ops).collect()
}

#[test]
fn shapes_emit_line_and_fillrect_ops() {
    let tmpl = br#"{
        "fonts": ["titillium"],
        "content": [
            { "type": "hr", "color": "cccccc", "thickness": 1 },
            { "type": "rect", "width": 100, "height": 40, "fill": "eeeeee", "border": "000000" },
            { "type": "line", "dx": 50, "dy": 0, "color": "ff0000" }
        ]
    }"#;
    let ops = lay_out(tmpl);
    let lines = ops
        .iter()
        .filter(|o| matches!(o, DrawOp::Line { .. }))
        .count();
    let fills = ops
        .iter()
        .filter(|o| matches!(o, DrawOp::FillRect { .. }))
        .count();
    // hr = 1 line; rect = 1 fill + 4 border lines; line = 1 line  => 6 lines, 1 fill.
    assert_eq!(fills, 1, "rect fill");
    assert_eq!(lines, 6, "hr(1) + rect border(4) + line(1)");
}

#[test]
fn ellipse_and_rounded_rect_emit_polygons() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "ellipse", "rx": 20, "ry": 20, "fill": "16a34a" },
        { "type": "rect", "width": 120, "height": 40, "fill": "f5f5f5", "border": "000000", "radius": 8 }
    ] }"#;
    let ops = lay_out(tmpl);
    let polys = ops
        .iter()
        .filter(|o| matches!(o, DrawOp::Polygon { .. }))
        .count();
    assert_eq!(polys, 2, "ellipse + rounded rect are each a polygon");
    // The rounded rect must NOT also emit a plain FillRect.
    assert_eq!(
        ops.iter()
            .filter(|o| matches!(o, DrawOp::FillRect { .. }))
            .count(),
        0
    );
}

#[test]
fn dashed_line_carries_dash_pattern() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "line", "dx": 200, "dy": 0, "color": "cccccc", "dash": [3, 2] }
    ] }"#;
    let ops = lay_out(tmpl);
    let dash = ops
        .iter()
        .find_map(|o| match o {
            DrawOp::Line { dash, .. } => Some(dash.clone()),
            _ => None,
        })
        .flatten();
    assert_eq!(dash, Some(vec![3, 2]));
}

#[test]
fn qr_renders_as_image_and_stays_pdfa() {
    let template = std::fs::read("tests/fixtures/template.json").unwrap();
    let invoice = Invoice::parse(&std::fs::read("tests/fixtures/invoice.json").unwrap()).unwrap();

    // The template embeds an EPC "scan-to-pay" QR bound to ${payment.*}.
    let pdf = generate_facturx_from_invoice(
        &template,
        &invoice,
        Profile::En16931,
        &FacturxMetadata::default(),
        &opts(),
    )
    .expect("generate");

    let doc = lopdf::Document::load_mem(&pdf).expect("reparse");

    // Exactly one image XObject (the QR; the remote logo is skipped offline).
    let images = doc
        .objects
        .values()
        .filter(|o| {
            matches!(o, Object::Stream(s)
                if s.dict.get(b"Subtype").ok().and_then(|v| v.as_name().ok()) == Some(b"Image".as_ref()))
        })
        .count();
    assert_eq!(images, 1, "the QR should be the one embedded image");

    // Still a Factur-X PDF/A-3b: embedded XML + the AF/OutputIntent scaffolding.
    let catalog = doc.catalog().unwrap();
    assert!(catalog.has(b"AF") && catalog.has(b"OutputIntents"));
    let has_xml = doc.objects.values().any(|o| matches!(o, Object::Stream(s)
        if s.dict.get(b"Type").ok().and_then(|v| v.as_name().ok()) == Some(b"EmbeddedFile".as_ref())));
    assert!(has_xml, "factur-x.xml still embedded alongside the QR");
}

#[test]
fn qr_skipped_gracefully_on_bad_payment_data() {
    // EPC requires EUR + an IBAN; a non-EUR currency must warn, not abort.
    let tmpl = br#"{
        "fonts": ["titillium"],
        "content": [
            { "type": "qr", "kind": "epc", "name": "X", "iban": "FR76", "amount": "10", "currency": "USD" }
        ]
    }"#;
    let t = Template::parse(tmpl).unwrap();
    let fonts = FontRegistry::from_font_dirs(&[PathBuf::from("fonts")], &t.fonts).unwrap();
    let o = opts();
    let (doc, warnings) = Engine::new(&t, &fonts, &o)
        .layout(&t, &DataDocument::empty())
        .unwrap();
    assert!(doc
        .pages
        .iter()
        .all(|p| !p.ops.iter().any(|op| matches!(op, DrawOp::Image { .. }))));
    assert!(warnings
        .iter()
        .any(|w| matches!(w, soli_pdf::RenderWarning::QrSkipped { .. })));
}

#[test]
fn paragraph_link_emits_link_op() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Pay online", "options": { "link": "https://pay.example.com/x", "fontSize": 12 } }
    ] }"#;
    let ops = lay_out(tmpl);
    let link = ops.iter().find_map(|o| match o {
        DrawOp::Link { uri, w, h, .. } => Some((uri.clone(), *w, *h)),
        _ => None,
    });
    let (uri, w, h) = link.expect("a link op should be emitted");
    assert_eq!(uri, "https://pay.example.com/x");
    assert!(w > 0.0 && h > 0.0, "link box should cover the text");
}

#[test]
fn link_annotation_present_and_pdfa_flagged() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Pay online", "options": { "link": "https://pay.example.com/x" } }
    ] }"#;
    let rendered = render_with_warnings(tmpl, b"{}", &opts()).unwrap();
    let xml = std::fs::read("tests/fixtures/factur-x.xml").unwrap();
    let pdf = facturx::embed_facturx(
        &rendered.pdf,
        &xml,
        Profile::En16931,
        &FacturxMetadata::default(),
    )
    .unwrap();
    let contains = |needle: &[u8]| pdf.windows(needle.len()).any(|w| w == needle);
    assert!(contains(b"/Subtype/Link"), "Link annotation present");
    assert!(contains(b"/URI(https://pay.example.com/x)"), "URI action");
    // PDF/A (6.3.2): the Print flag must be set on the annotation.
    assert!(contains(b"/F 4"), "annotation carries Print flag /F 4");
}

#[test]
fn inline_rich_text_mixes_size_color_and_links() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "options": { "fontSize": 12 }, "spans": [
            { "text": "Total: " },
            { "text": "600", "fontWeight": "bold", "color": "0F766E", "fontSize": 18 },
            { "text": " pay " },
            { "text": "now", "link": "https://pay/42" }
        ] }
    ] }"#;
    let ops = lay_out(tmpl);
    let pieces: Vec<_> = ops
        .iter()
        .filter_map(|o| match o {
            DrawOp::StyledText { pieces, .. } => Some(pieces.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(!pieces.is_empty(), "styled text emitted");
    // Mixed sizes (12 and 18) present.
    let sizes: std::collections::BTreeSet<i32> = pieces.iter().map(|p| p.size as i32).collect();
    assert!(
        sizes.contains(&12) && sizes.contains(&18),
        "sizes: {sizes:?}"
    );
    // The bold "600" piece is non-black (teal).
    assert!(pieces
        .iter()
        .any(|p| p.text.contains("600") && p.color != soli_pdf::color::Rgb::BLACK));
    // The "now" span is an inline external link.
    assert!(ops
        .iter()
        .any(|o| matches!(o, DrawOp::Link { uri, .. } if uri == "https://pay/42")));
    // A plain value paragraph would emit DrawOp::Text; this one must not.
    assert!(!ops.iter().any(|o| matches!(o, DrawOp::Text(_))));
}

/// A two-page template: a bookmarked/anchored "Summary" on page 1, a big spacer,
/// then a bookmarked/anchored "Details" on page 2, with internal jumps both ways.
const NAV_TMPL: &[u8] = br#"{ "fonts": ["titillium"], "content": [
  { "type": "paragraph", "value": "Summary", "options": { "fontSize": 18, "fontWeight": "bold", "bookmark": "Summary", "anchor": "summary" } },
  { "type": "paragraph", "value": "Go to details", "options": { "link_to": "details" } },
  { "type": "move", "y": 700 },
  { "type": "paragraph", "value": "Details", "options": { "fontSize": 18, "fontWeight": "bold", "bookmark": "Details", "anchor": "details" } },
  { "type": "paragraph", "value": "Back to summary", "options": { "link_to": "summary" } }
] }"#;

#[test]
fn bookmarks_anchors_and_internal_links_collected() {
    let t = Template::parse(NAV_TMPL).unwrap();
    let fonts = FontRegistry::from_font_dirs(&[PathBuf::from("fonts")], &t.fonts).unwrap();
    let (doc, _w) = Engine::new(&t, &fonts, &opts())
        .layout(&t, &DataDocument::empty())
        .unwrap();
    assert_eq!(doc.pages.len(), 2, "content spans two pages");
    let labels: Vec<&str> = doc.bookmarks.iter().map(|(l, _, _)| l.as_str()).collect();
    assert_eq!(labels, vec!["Summary", "Details"]);
    assert_eq!(doc.bookmarks[0].1, 0, "Summary bookmark on page 0");
    assert_eq!(doc.bookmarks[1].1, 1, "Details bookmark on page 1");
    assert!(doc.anchors.contains_key("summary") && doc.anchors.contains_key("details"));
    let internal = doc
        .pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter(|o| matches!(o, DrawOp::InternalLink { .. }))
        .count();
    assert_eq!(internal, 2, "two internal jump links");
}

#[test]
fn outline_and_internal_dest_present_after_facturx() {
    let rendered = render_with_warnings(NAV_TMPL, b"{}", &opts()).unwrap();
    let xml = std::fs::read("tests/fixtures/factur-x.xml").unwrap();
    let pdf = facturx::embed_facturx(
        &rendered.pdf,
        &xml,
        Profile::En16931,
        &FacturxMetadata::default(),
    )
    .unwrap();
    let count = |needle: &[u8]| pdf.windows(needle.len()).filter(|w| *w == needle).count();
    assert!(count(b"/Outlines") >= 1, "document outline present");
    // printpdf serialises an internal go-to as a /Dest on the Link annotation.
    assert!(count(b"/Dest") >= 2, "internal destination links present");
}

/// Lay out a template and return its resolved page geometry.
fn lay_out_page(template_json: &[u8]) -> soli_pdf::geometry::Page {
    let t = Template::parse(template_json).expect("template");
    let fonts = FontRegistry::from_font_dirs(&[PathBuf::from("fonts")], &t.fonts).expect("fonts");
    let (doc, _w) = Engine::new(&t, &fonts, &opts())
        .layout(&t, &DataDocument::empty())
        .expect("layout");
    doc.page
}

#[test]
fn page_preset_letter() {
    let tmpl = br#"{ "fonts": ["titillium"], "options": { "page": "letter" }, "content": [] }"#;
    let page = lay_out_page(tmpl);
    assert_eq!((page.width, page.height), (612.0, 792.0));
}

#[test]
fn page_landscape_swaps_dimensions() {
    let tmpl = br#"{ "fonts": ["titillium"],
        "options": { "page": "a4", "orientation": "landscape" }, "content": [] }"#;
    let page = lay_out_page(tmpl);
    assert!(page.width > page.height, "landscape A4 is wider than tall");
    assert!((page.width - 841.890).abs() < 0.01);
    assert!((page.height - 595.276).abs() < 0.01);
}

#[test]
fn page_custom_dimensions() {
    let tmpl = br#"{ "fonts": ["titillium"],
        "options": { "page": { "width": 300, "height": 500 } }, "content": [] }"#;
    let page = lay_out_page(tmpl);
    assert_eq!((page.width, page.height), (300.0, 500.0));
}

#[test]
fn margins_per_side_with_defaults() {
    let tmpl = br#"{ "fonts": ["titillium"],
        "options": { "margins": { "left": 100, "top": 80 } }, "content": [] }"#;
    let page = lay_out_page(tmpl);
    assert_eq!(page.margins.left, 100.0);
    assert_eq!(page.margins.top, 80.0);
    // Unspecified sides keep the ~20 mm default.
    assert!((page.margins.right - 56.693).abs() < 0.01);
    assert!((page.margins.bottom - 56.693).abs() < 0.01);
    // Content box reflects the margins.
    assert_eq!(page.content_left(), 100.0);
    assert_eq!(page.content_top(), 80.0); // header_height is 0 here
}

#[test]
fn margin_scalar_applies_to_all_sides() {
    let tmpl = br#"{ "fonts": ["titillium"], "options": { "margins": 40 }, "content": [] }"#;
    let m = lay_out_page(tmpl).margins;
    assert_eq!((m.top, m.right, m.bottom, m.left), (40.0, 40.0, 40.0, 40.0));
}

#[test]
fn header_height_offsets_content_below_top_margin() {
    let tmpl = br#"{ "fonts": ["titillium"],
        "options": { "margins": { "top": 50 }, "header_height": 30 }, "content": [] }"#;
    let page = lay_out_page(tmpl);
    // Content starts below the top margin AND the header band.
    assert_eq!(page.content_top(), 80.0);
}

#[test]
fn watermark_emits_rotated_text_behind_content() {
    let tmpl = br#"{ "fonts": ["titillium"],
        "options": { "watermark": { "text": "DRAFT", "angle": 45, "color": "eebbbb" } },
        "content": [ { "type": "paragraph", "value": "Body", "options": { "fontSize": 12 } } ] }"#;
    let ops = lay_out(tmpl);
    // The watermark must be the first op so it renders beneath the content.
    let first = ops.first().expect("page has ops");
    let DrawOp::RotatedText { pieces, angle, .. } = first else {
        panic!("first op should be the rotated watermark, got {first:?}");
    };
    assert_eq!(*angle, 45.0);
    let text: String = pieces.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(text, "DRAFT");
}
