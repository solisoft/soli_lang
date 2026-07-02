//! Integration tests for multi-column flow and page background images.

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

/// (x, baseline, text) of every plain Text op across all pages.
fn texts(doc: &LaidOutDoc) -> Vec<(usize, f32, f32, String)> {
    let mut out = Vec::new();
    for (pi, page) in doc.pages.iter().enumerate() {
        for op in &page.ops {
            if let DrawOp::Text(td) = op {
                out.push((
                    pi,
                    td.x,
                    td.baseline,
                    td.pieces.iter().map(|p| p.text.as_str()).collect(),
                ));
            }
        }
    }
    out
}

fn long_para(tag: &str, n: usize) -> String {
    let body = format!("{tag} word ").repeat(n);
    format!(r#"{{ "type": "paragraph", "value": "{body}", "options": {{ "fontSize": 10 }} }}"#)
}

// --- sequential fill ---------------------------------------------------------------

#[test]
fn two_columns_fill_sequentially_then_resume_full_width() {
    // Enough text to overflow column 1 into column 2 (may span pages — the
    // test is size-robust and doesn't assert the page count).
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "columns", "count": 2, "gap": 20, "content": [ {} ] }},
            {{ "type": "paragraph", "value": "AFTER", "options": {{ "fontSize": 10 }} }}
        ] }}"#,
        long_para("col", 550)
    );
    let (doc, warnings) = render(tmpl.as_bytes(), b"{}");
    assert!(warnings.is_empty(), "no warnings: {warnings:?}");

    let t = texts(&doc);
    // The set filled into column 2: `col` ops span from the left edge to well
    // past it (the second column).
    let col_lefts: Vec<f32> = t
        .iter()
        .filter(|(_, _, _, s)| s.starts_with("col"))
        .map(|(_, x, _, _)| *x)
        .collect();
    let min_left = col_lefts.iter().cloned().fold(f32::MAX, f32::min);
    let max_x = col_lefts.iter().cloned().fold(0.0f32, f32::max);
    assert!(
        max_x > min_left + 150.0,
        "column-2 content exists (spread {min_left}..{max_x})"
    );

    // "AFTER" resumes at the content left edge (full width), below the columns.
    let after = t
        .iter()
        .find(|(_, _, _, s)| s.contains("AFTER"))
        .expect("AFTER");
    assert!(
        (after.1 - min_left).abs() < 1.0,
        "AFTER is full-width at the content left ({} vs {min_left})",
        after.1
    );
}

#[test]
fn wrap_width_is_the_column_width() {
    // The same text produces MORE lines inside 2 columns than full width.
    let para = long_para("x", 60);
    let full = format!(r#"{{ "fonts": ["titillium"], "content": [ {para} ] }}"#);
    let cols = format!(
        r#"{{ "fonts": ["titillium"], "content": [ {{ "type": "columns", "count": 2, "content": [ {para} ] }} ] }}"#
    );
    let (dfull, _) = render(full.as_bytes(), b"{}");
    let (dcols, _) = render(cols.as_bytes(), b"{}");
    let lines_full = texts(&dfull).len();
    let lines_cols = texts(&dcols).len();
    assert!(
        lines_cols > lines_full,
        "narrower columns wrap into more lines ({lines_cols} vs {lines_full})"
    );
}

#[test]
fn page_break_inside_columns_is_a_column_break() {
    let tmpl = r#"{ "fonts": ["titillium"], "content": [
        { "type": "columns", "count": 2, "content": [
            { "type": "paragraph", "value": "FIRST", "options": { "fontSize": 10 } },
            { "type": "page_break" },
            { "type": "paragraph", "value": "SECOND", "options": { "fontSize": 10 } }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    assert_eq!(doc.pages.len(), 1, "column break stays on the same page");
    let t = texts(&doc);
    let first = t.iter().find(|(_, _, _, s)| s.contains("FIRST")).unwrap();
    let second = t.iter().find(|(_, _, _, s)| s.contains("SECOND")).unwrap();
    assert!(second.1 > first.1 + 100.0, "SECOND is in the next column");
    assert!(
        (first.2 - second.2).abs() < 1.0,
        "both at the same top (column break, not a downward move)"
    );
}

#[test]
fn overflowing_last_column_starts_a_new_page_and_restarts_the_set() {
    // Way more than two columns of content -> spills onto page 2.
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"],
             "options": {{ "header_height": 30 }},
             "header": [ {{ "type": "paragraph", "value": "RUNNING HEADER", "options": {{ "fontSize": 8 }} }} ],
             "content": [ {{ "type": "columns", "count": 2, "content": [ {} ] }} ] }}"#,
        long_para("z", 900)
    );
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    assert!(doc.pages.len() >= 2, "spills to a second page");
    // The header repeats on page 2 (begin_page redrew it with the flow suspended).
    let header_on_p2 = doc.pages[1].ops.iter().any(|op| match op {
        DrawOp::Text(td) => td
            .pieces
            .iter()
            .any(|p| p.text.contains("RUNNING") || p.text.contains("HEADER")),
        _ => false,
    });
    assert!(header_on_p2, "running header redrawn on page 2");
    // Page 2 restarts at column 1 (some op near the left content edge).
    let p2_min_x = doc.pages[1]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(td) => Some(td.x),
            _ => None,
        })
        .fold(f32::MAX, f32::min);
    assert!(
        p2_min_x < 80.0,
        "page 2 body starts at column 1 (x {p2_min_x})"
    );
}

// --- degenerate cases --------------------------------------------------------------

#[test]
fn count_one_is_plain_flow() {
    let tmpl = r#"{ "fonts": ["titillium"], "content": [
        { "type": "columns", "count": 1, "content": [
            { "type": "paragraph", "value": "solo", "options": { "fontSize": 10 } }
        ] }
    ] }"#;
    let (doc, warnings) = render(tmpl.as_bytes(), b"{}");
    let t = texts(&doc);
    assert_eq!(t.len(), 1);
    assert!(t[0].1 < 80.0, "single column flows full width");
    assert!(warnings.is_empty());
}

#[test]
fn count_is_clamped_and_warns() {
    let tmpl = r#"{ "fonts": ["titillium"], "content": [
        { "type": "columns", "count": 12, "content": [
            { "type": "paragraph", "value": "hi", "options": { "fontSize": 8 } }
        ] }
    ] }"#;
    let (_, warnings) = render(tmpl.as_bytes(), b"{}");
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            RenderWarning::ElementSkipped { kind, reason }
                if kind == "columns" && reason.contains("clamped")
        )),
        "count>6 warns: {warnings:?}"
    );
}

#[test]
fn tables_and_charts_inside_columns_are_skipped_with_warnings() {
    let tmpl = r#"{ "fonts": ["titillium"], "content": [
        { "type": "columns", "count": 2, "content": [
            { "type": "table", "rows": [ [ { "text": "x" } ] ] },
            { "type": "chart", "kind": "pie", "points": [ { "label": "a", "value": 1 } ] }
        ] }
    ] }"#;
    let (doc, warnings) = render(tmpl.as_bytes(), b"{}");
    let skipped: Vec<&str> = warnings
        .iter()
        .filter_map(|w| match w {
            RenderWarning::ElementSkipped { kind, reason } if reason.contains("inside columns") => {
                Some(kind.as_str())
            }
            _ => None,
        })
        .collect();
    assert!(
        skipped.contains(&"table") && skipped.contains(&"chart"),
        "{skipped:?}"
    );
    // No table/chart draw ops leaked.
    assert!(texts(&doc).is_empty());
}

#[test]
fn nested_columns_flatten_with_a_warning() {
    let tmpl = r#"{ "fonts": ["titillium"], "content": [
        { "type": "columns", "count": 2, "content": [
            { "type": "columns", "count": 3, "content": [
                { "type": "paragraph", "value": "deep", "options": { "fontSize": 10 } }
            ] }
        ] }
    ] }"#;
    let (doc, warnings) = render(tmpl.as_bytes(), b"{}");
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            RenderWarning::ElementSkipped { kind, reason }
                if kind == "columns" && reason.contains("nested")
        )),
        "nested columns warn: {warnings:?}"
    );
    assert_eq!(texts(&doc).len(), 1, "content survives (flattened)");
}

#[test]
fn empty_columns_is_a_noop() {
    let tmpl = r#"{ "fonts": ["titillium"], "content": [
        { "type": "columns", "count": 2, "content": [] },
        { "type": "paragraph", "value": "next", "options": { "fontSize": 10 } }
    ] }"#;
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    let t = texts(&doc);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].3, "next");
}

// --- background image ---------------------------------------------------------------

#[test]
fn background_image_is_drawn_behind_every_page() {
    let svg = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='40' height='40'><rect width='40' height='40' fill='%23e0f2fe'/></svg>";
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"],
             "options": {{ "backgroundImage": {{ "src": "{svg}" }} }},
             "content": [
                {{ "type": "paragraph", "value": "one" }},
                {{ "type": "page_break" }},
                {{ "type": "paragraph", "value": "two" }}
             ] }}"#
    );
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    assert_eq!(doc.pages.len(), 2);
    for (i, page) in doc.pages.iter().enumerate() {
        // The FIRST op is the page-sized background image.
        match &page.ops[0] {
            DrawOp::Image { w, h, .. } => {
                assert!(
                    (*w - doc.page.width).abs() < 0.5 && (*h - doc.page.height).abs() < 0.5,
                    "page {} background covers the whole page ({w}x{h})",
                    i + 1
                );
            }
            other => panic!(
                "page {} first op is not the background image: {other:?}",
                i + 1
            ),
        }
    }
}

#[test]
fn background_image_first_only_filter() {
    let svg = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10' fill='%23fff'/></svg>";
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"],
             "options": {{ "backgroundImage": {{ "src": "{svg}", "pages": "first" }} }},
             "content": [
                {{ "type": "paragraph", "value": "one" }},
                {{ "type": "page_break" }},
                {{ "type": "paragraph", "value": "two" }}
             ] }}"#
    );
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    let has_bg =
        |p: &soli_pdf::draw::RenderedPage| matches!(p.ops.first(), Some(DrawOp::Image { .. }));
    assert!(has_bg(&doc.pages[0]), "page 1 has the background");
    assert!(!has_bg(&doc.pages[1]), "page 2 does not");
}
