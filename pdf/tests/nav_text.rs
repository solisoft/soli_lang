//! Integration tests for phase 2: hierarchical bookmarks, keep-together
//! (`minSpaceBelow`), and justified text.

use std::path::PathBuf;
use std::time::Duration;

use soli_pdf::data::DataDocument;
use soli_pdf::draw::{DrawOp, LaidOutDoc};
use soli_pdf::fonts::FontRegistry;
use soli_pdf::layout::Engine;
use soli_pdf::template::Template;
use soli_pdf::{render_to_bytes, RenderOptions, RenderWarning};

fn opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
        ..Default::default()
    }
}

fn render(template_json: &[u8], data_json: &[u8]) -> (LaidOutDoc, Vec<RenderWarning>) {
    let t = Template::parse(template_json).expect("template");
    let fonts = FontRegistry::from_font_dirs(&[PathBuf::from("fonts")], &t.fonts).expect("fonts");
    let data = DataDocument::parse(data_json).expect("data");
    let o = opts();
    Engine::new(&t, &fonts, &o)
        .layout(&t, &data)
        .expect("layout")
}

// --- hierarchical bookmarks --------------------------------------------------------

#[test]
fn bookmark_levels_build_an_outline_tree() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Part I", "options": { "bookmark": "Part I" } },
        { "type": "paragraph", "value": "Chapter 1", "options": { "bookmark": "Chapter 1", "bookmarkLevel": 2 } },
        { "type": "paragraph", "value": "Chapter 2", "options": { "bookmark": "Chapter 2", "bookmarkLevel": 2 } },
        { "type": "page_break" },
        { "type": "paragraph", "value": "Part II", "options": { "bookmark": "Part II" } }
    ] }"#;
    let pdf = render_to_bytes(tmpl, b"{}", &opts()).expect("render");
    let doc = lopdf::Document::load_mem(&pdf).expect("parse");

    // Find the Outlines root via the catalog.
    let catalog_id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let catalog = doc.get_dictionary(catalog_id).unwrap();
    let outlines_id = catalog.get(b"Outlines").unwrap().as_reference().unwrap();
    let outlines = doc.get_dictionary(outlines_id).unwrap();

    let title_of = |id: lopdf::ObjectId| -> String {
        let d = doc.get_dictionary(id).unwrap();
        match d.get(b"Title").unwrap() {
            lopdf::Object::String(b, _) => {
                if b.starts_with(&[0xFE, 0xFF]) {
                    let units: Vec<u16> = b[2..]
                        .chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    String::from_utf16_lossy(&units)
                } else {
                    String::from_utf8_lossy(b).into_owned()
                }
            }
            other => panic!("Title: {other:?}"),
        }
    };

    // Root children: Part I, Part II (in document order).
    let first = outlines.get(b"First").unwrap().as_reference().unwrap();
    assert_eq!(title_of(first), "Part I");
    let part1 = doc.get_dictionary(first).unwrap();
    let part2_id = part1.get(b"Next").unwrap().as_reference().unwrap();
    assert_eq!(title_of(part2_id), "Part II");

    // Part I nests the two chapters.
    let c1_id = part1.get(b"First").unwrap().as_reference().unwrap();
    assert_eq!(title_of(c1_id), "Chapter 1");
    let c1 = doc.get_dictionary(c1_id).unwrap();
    let c2_id = c1.get(b"Next").unwrap().as_reference().unwrap();
    assert_eq!(title_of(c2_id), "Chapter 2");
    assert_eq!(
        part1.get(b"Count").unwrap().as_i64().unwrap(),
        2,
        "Part I counts its two descendants"
    );
    // The chapters' Parent is Part I, not the outline root.
    assert_eq!(
        doc.get_dictionary(c1_id)
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap(),
        first
    );
}

#[test]
fn bookmark_dest_targets_the_right_page() {
    // Regression for the 0/1-based off-by-one: a bookmark on page 2 must
    // reference page 2's object, not page 1's.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "first" },
        { "type": "page_break" },
        { "type": "paragraph", "value": "second", "options": { "bookmark": "On page two" } }
    ] }"#;
    let pdf = render_to_bytes(tmpl, b"{}", &opts()).expect("render");
    let doc = lopdf::Document::load_mem(&pdf).expect("parse");
    let pages: Vec<_> = doc.page_iter().collect();
    assert_eq!(pages.len(), 2);

    let catalog_id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let catalog = doc.get_dictionary(catalog_id).unwrap();
    let outlines_id = catalog.get(b"Outlines").unwrap().as_reference().unwrap();
    let first = doc
        .get_dictionary(outlines_id)
        .unwrap()
        .get(b"First")
        .unwrap()
        .as_reference()
        .unwrap();
    let dest = doc
        .get_dictionary(first)
        .unwrap()
        .get(b"Dest")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let target = dest[0].as_reference().unwrap();
    assert_eq!(target, pages[1], "bookmark jumps to page 2");
}

// --- keep-together -------------------------------------------------------------------

#[test]
fn min_space_below_moves_an_orphan_heading_to_the_next_page() {
    // Fill most of the page, then a heading that demands 200pt below itself.
    let filler = "line\\n".repeat(52); // JSON-escaped \n: ~52 * 12pt of a ~718pt page
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "paragraph", "value": "{filler}", "options": {{ "fontSize": 10 }} }},
            {{ "type": "paragraph", "value": "Heading", "options": {{ "fontSize": 14, "minSpaceBelow": 200 }} }}
        ] }}"#
    );
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    assert_eq!(doc.pages.len(), 2, "the heading forced a page break");
    let page2_text: Vec<String> = doc.pages[1]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(td) => Some(td.pieces.iter().map(|p| p.text.as_str()).collect()),
            _ => None,
        })
        .collect();
    assert!(
        page2_text.iter().any(|t| t.contains("Heading")),
        "heading landed on page 2: {page2_text:?}"
    );
}

// --- justify ---------------------------------------------------------------------------

#[test]
fn justify_stretches_lines_to_the_region_and_leaves_the_last_line() {
    // A long text wrapping to several lines within the ~495pt content width.
    let words = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua ".repeat(2);
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "paragraph", "value": "{words}", "options": {{ "fontSize": 11, "alignment": "justify" }} }}
        ] }}"#
    );
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    // Group word-ops by baseline (one group per line).
    let mut lines: std::collections::BTreeMap<i64, Vec<(f32, f32)>> = Default::default();
    for op in &doc.pages[0].ops {
        if let DrawOp::Text(td) = op {
            let w: f32 = td.pieces.iter().map(|p| p.text.len() as f32).sum(); // proxy
            lines
                .entry((td.baseline * 100.0) as i64)
                .or_default()
                .push((td.x, w));
        }
    }
    assert!(lines.len() >= 3, "several wrapped lines: {}", lines.len());
    let line_vec: Vec<&Vec<(f32, f32)>> = lines.values().collect();
    // Every non-last line was emitted as MULTIPLE word ops (justified),
    // and its rightmost op starts far past the midpoint of the region.
    for (i, ops) in line_vec.iter().enumerate() {
        if i + 1 == line_vec.len() {
            continue; // last line: single op, left-aligned
        }
        assert!(ops.len() > 1, "justified line {i} split into word ops");
        let max_x = ops.iter().map(|(x, _)| *x).fold(0.0f32, f32::max);
        assert!(
            max_x > 300.0,
            "line {i}: last word pushed toward the right edge (x {max_x})"
        );
    }
    // The very last line is one single op at the left edge.
    let last = line_vec.last().unwrap();
    assert_eq!(last.len(), 1, "last line is not justified");
}

#[test]
fn header_binds_document_data() {
    // Data-bound elements (repeat here) must see the document data in the
    // header band, not an empty document.
    let tmpl = br#"{ "fonts": ["titillium"],
        "options": { "headerHeight": 60 },
        "header": [
            { "type": "repeat", "data": "items", "content": [
                { "type": "paragraph", "value": "${name}" }
            ] }
        ],
        "content": [ { "type": "paragraph", "value": "body" } ] }"#;
    let data = br#"{ "data": { "items": [ { "name": "AlphaRow" }, { "name": "BetaRow" } ] } }"#;
    let (doc, _) = render(tmpl, data);
    let mut all_text = String::new();
    for op in &doc.pages[0].ops {
        if let DrawOp::Text(td) = op {
            for p in &td.pieces {
                all_text.push_str(&p.text);
            }
        }
    }
    assert!(
        all_text.contains("AlphaRow"),
        "header repeat bound: {all_text:?}"
    );
    assert!(all_text.contains("BetaRow"));
}

#[test]
fn footer_supports_static_elements() {
    // hr advances the band cursor; rect draws at the (move-positioned) cursor.
    let tmpl = br#"{ "fonts": ["titillium"],
        "footer": [
            { "type": "hr", "thickness": 1.0, "color": "cccccc" },
            { "type": "rect", "width": 100, "height": 8, "fill": "eeeeee" },
            { "type": "move", "y": 10 },
            { "type": "paragraph", "value": "footer text", "options": { "fontSize": 8 } }
        ],
        "content": [ { "type": "paragraph", "value": "body" } ] }"#;
    let (doc, warnings) = render(tmpl, b"{}");
    assert!(
        !warnings
            .iter()
            .any(|w| matches!(w, RenderWarning::ElementSkipped { reason, .. } if reason.contains("footer"))),
        "static footer elements are supported: {warnings:?}"
    );
    let ops = &doc.pages[0].ops;
    assert!(
        ops.iter().any(|op| matches!(op, DrawOp::Line { .. })),
        "footer hr drawn"
    );
    assert!(
        ops.iter().any(|op| matches!(op, DrawOp::FillRect { .. })),
        "footer rect drawn"
    );
    let footer_text = ops.iter().any(
        |op| matches!(op, DrawOp::Text(td) if td.pieces.iter().any(|p| p.text.contains("footer"))),
    );
    assert!(footer_text, "footer paragraph drawn after move");
}

#[test]
fn justify_on_spans_stretches_lines() {
    // Long rich text wraps into several lines; every non-last line must be
    // split into multiple StyledText segments whose rightmost one is pushed
    // toward the right edge (the justified gap distribution).
    let words = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua ".repeat(2);
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "paragraph", "options": {{ "fontSize": 11, "alignment": "justify" }},
              "spans": [ {{ "text": "{words}" }}, {{ "text": "fin", "fontWeight": "bold" }} ] }}
        ] }}"#
    );
    let (doc, warnings) = render(tmpl.as_bytes(), b"{}");
    assert!(
        !warnings
            .iter()
            .any(|w| matches!(w, RenderWarning::ElementSkipped { kind, .. } if kind == "justify")),
        "justify on spans no longer warns: {warnings:?}"
    );
    // Group segment ops by baseline (one group per line).
    let mut lines: std::collections::BTreeMap<i64, Vec<f32>> = Default::default();
    for op in &doc.pages[0].ops {
        if let DrawOp::StyledText { x, baseline, .. } = op {
            lines.entry((baseline * 100.0) as i64).or_default().push(*x);
        }
    }
    assert!(lines.len() >= 3, "several wrapped lines: {}", lines.len());
    let line_vec: Vec<&Vec<f32>> = lines.values().collect();
    for (i, xs) in line_vec.iter().enumerate() {
        if i + 1 == line_vec.len() {
            continue; // last line: single segment, left-aligned
        }
        assert!(xs.len() > 1, "justified line {i} split into segments");
        let max_x = xs.iter().copied().fold(0.0f32, f32::max);
        assert!(
            max_x > 300.0,
            "line {i}: last segment pushed toward the right edge (x {max_x})"
        );
    }
    let last = line_vec.last().unwrap();
    assert_eq!(last.len(), 1, "last line is not justified");
}
