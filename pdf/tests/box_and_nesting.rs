//! Integration tests for the `box` container element and for scope-aware array
//! binding (a `repeat` or data-bound `table` nested inside another `repeat`).
//!
//! Both exist because the template language previously had no way to express
//! two very ordinary documents: a panel whose background fits its own contents,
//! and line items grouped under sections. The first had to be faked with
//! `rect` + hand-computed `move` offsets; the second silently rendered nothing.

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

fn all_ops(doc: &LaidOutDoc) -> Vec<DrawOp> {
    doc.pages.iter().flat_map(|p| p.ops.clone()).collect()
}

fn texts(doc: &LaidOutDoc) -> Vec<String> {
    all_ops(doc)
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(td) => Some(td.pieces.iter().map(|p| p.text.as_str()).collect()),
            _ => None,
        })
        .collect()
}

/// Every filled rectangle in the document as `(x, y, w, h)`.
fn fill_rects(doc: &LaidOutDoc) -> Vec<(f32, f32, f32, f32)> {
    all_ops(doc)
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillRect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect()
}

// --- nested data binding -----------------------------------------------------

const SECTIONS: &[u8] = br#"{ "data": { "sections": [
    { "title": "Structural", "lines": [ {"name":"Demolition"}, {"name":"Screed"} ] },
    { "title": "Plumbing",   "lines": [ {"name":"Supply"} ] }
] } }"#;

#[test]
fn repeat_nested_in_repeat_binds_to_the_current_item() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "repeat", "data": "sections", "content": [
            { "type": "paragraph", "value": "S:${title}" },
            { "type": "repeat", "data": "lines", "content": [
                { "type": "paragraph", "value": "L:${name}" }
            ] }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, SECTIONS);
    assert_eq!(
        texts(&doc),
        vec![
            "S:Structural",
            "L:Demolition",
            "L:Screed",
            "S:Plumbing",
            "L:Supply"
        ],
        "each section's own lines render under it, in order"
    );
}

#[test]
fn data_bound_table_nested_in_repeat_binds_to_the_current_item() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "repeat", "data": "sections", "content": [
            { "type": "paragraph", "value": "S:${title}" },
            { "type": "table", "data": "lines", "header_columns": [],
              "rows": [ [ { "text": "L:${name}", "width": 200 } ] ] }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, SECTIONS);
    assert_eq!(
        texts(&doc),
        vec![
            "S:Structural",
            "L:Demolition",
            "L:Screed",
            "S:Plumbing",
            "L:Supply"
        ],
    );
}

#[test]
fn root_level_binding_still_resolves_from_the_root() {
    // Scope-first must not break the ordinary case, nor the documented fallback
    // where a path inside a repeat resolves against the root.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "repeat", "data": "sections", "content": [
            { "type": "paragraph", "value": "${title}/${company}" }
        ] }
    ] }"#;
    let data = br#"{ "data": { "company": "ACME",
        "sections": [ { "title": "A" }, { "title": "B" } ] } }"#;
    let (doc, _) = render(tmpl, data);
    assert_eq!(texts(&doc), vec!["A/ACME", "B/ACME"]);
}

#[test]
fn a_missing_nested_array_renders_nothing_without_erroring() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "repeat", "data": "sections", "content": [
            { "type": "paragraph", "value": "S:${title}" },
            { "type": "repeat", "data": "nope", "content": [
                { "type": "paragraph", "value": "never" }
            ] }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, SECTIONS);
    assert_eq!(texts(&doc), vec!["S:Structural", "S:Plumbing"]);
}

// --- box ---------------------------------------------------------------------

#[test]
fn box_height_follows_its_content() {
    // Two boxes, identical but for how much text they hold: the taller content
    // must produce the taller panel. This is the property `rect` cannot express.
    let one = br#"{ "fonts": ["titillium"], "content": [
        { "type": "box", "fill": "EEEEEE", "padding": 10, "content": [
            { "type": "paragraph", "value": "one" } ] } ] }"#;
    let three = br#"{ "fonts": ["titillium"], "content": [
        { "type": "box", "fill": "EEEEEE", "padding": 10, "content": [
            { "type": "paragraph", "value": "one" },
            { "type": "paragraph", "value": "two" },
            { "type": "paragraph", "value": "three" } ] } ] }"#;

    let (d1, _) = render(one, b"{}");
    let (d3, _) = render(three, b"{}");
    let h1 = fill_rects(&d1)[0].3;
    let h3 = fill_rects(&d3)[0].3;
    assert!(h3 > h1, "three lines ({h3}) must be taller than one ({h1})");
}

#[test]
fn box_paints_behind_its_content() {
    // The panel must be emitted BEFORE the text it sits behind, or it would
    // paint over it. This is what the splice-at-recorded-index does.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "box", "fill": "EEEEEE", "padding": 8, "content": [
            { "type": "paragraph", "value": "inside" } ] } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let ops = all_ops(&doc);
    let rect_at = ops
        .iter()
        .position(|o| matches!(o, DrawOp::FillRect { .. }))
        .expect("a fill");
    let text_at = ops
        .iter()
        .position(|o| matches!(o, DrawOp::Text(_)))
        .expect("the text");
    assert!(rect_at < text_at, "the box must paint under its content");
}

#[test]
fn box_advances_the_cursor_below_itself() {
    // Content after a box must clear it — the failure mode `rect` had was text
    // landing on top of the panel because nothing moved the cursor.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "box", "fill": "EEEEEE", "padding": 10, "gap": 12, "content": [
            { "type": "paragraph", "value": "inside" } ] },
        { "type": "paragraph", "value": "after" } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let (_, ry, _, rh) = fill_rects(&doc)[0];
    let after_y = all_ops(&doc)
        .iter()
        .find_map(|op| match op {
            DrawOp::Text(td) if td.pieces.iter().any(|p| p.text == "after") => Some(td.baseline),
            _ => None,
        })
        .expect("the trailing paragraph");
    assert!(
        after_y >= ry + rh,
        "text after the box (y={after_y}) must sit below it (bottom={})",
        ry + rh
    );
}

#[test]
fn nested_boxes_grow_the_outer_one() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "box", "fill": "DDDDDD", "padding": 12, "content": [
            { "type": "box", "fill": "FFFFFF", "padding": 10, "content": [
                { "type": "paragraph", "value": "inner" } ] } ] } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let rects = fill_rects(&doc);
    assert_eq!(rects.len(), 2, "outer and inner panels");
    let (ox, oy, ow, oh) = rects[0];
    let (ix, iy, iw, ih) = rects[1];
    assert!(ox < ix && oy < iy, "the inner box is inset from the outer");
    assert!(
        ox + ow >= ix + iw && oy + oh >= iy + ih,
        "the outer box contains the inner one"
    );
}

#[test]
fn box_width_defaults_to_the_content_region() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "box", "fill": "EEEEEE", "content": [
            { "type": "paragraph", "value": "x" } ] } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let (_, _, w, _) = fill_rects(&doc)[0];
    // A4 (595pt) less the default 56.693pt margins on each side.
    assert!(
        (w - 481.6).abs() < 1.0,
        "an unqualified box spans the content column, got {w}"
    );
}

#[test]
fn box_respects_an_explicit_width() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "box", "fill": "EEEEEE", "width": 200, "content": [
            { "type": "paragraph", "value": "x" } ] } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let (_, _, w, _) = fill_rects(&doc)[0];
    assert!((w - 200.0).abs() < 0.01, "got {w}");
}

#[test]
fn text_wraps_at_the_boxs_inner_edge() {
    // The same sentence in a narrow box must break into more lines than in a
    // full-width one — proving children see the box's inner width, not the page's.
    let long = "the quick brown fox jumps over the lazy dog and keeps running well past the margin";
    let wide = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "box", "padding": 10, "content": [
                {{ "type": "paragraph", "value": "{long}" }} ] }} ] }}"#
    );
    let narrow = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "box", "padding": 10, "width": 150, "content": [
                {{ "type": "paragraph", "value": "{long}" }} ] }} ] }}"#
    );
    let (dw, _) = render(wide.as_bytes(), b"{}");
    let (dn, _) = render(narrow.as_bytes(), b"{}");
    assert!(
        texts(&dn).len() > texts(&dw).len(),
        "narrow box wrapped into {} lines, wide into {}",
        texts(&dn).len(),
        texts(&dw).len()
    );
}

#[test]
fn a_box_spanning_a_page_break_warns_instead_of_misplacing_its_panel() {
    // 120 lines cannot fit one page; the decoration is dropped rather than
    // spliced onto an already-flushed page.
    let mut content = String::new();
    for i in 0..120 {
        content.push_str(&format!(
            r#"{{ "type": "paragraph", "value": "line {i}" }},"#
        ));
    }
    content.pop();
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "box", "fill": "EEEEEE", "padding": 10, "content": [{content}] }} ] }}"#
    );
    let (doc, warnings) = render(tmpl.as_bytes(), b"{}");
    assert!(doc.pages.len() > 1, "the content paginated");
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            RenderWarning::ElementSkipped { kind, .. } if kind == "box"
        )),
        "a page-spanning box warns: {warnings:?}"
    );
    assert!(
        fill_rects(&doc).is_empty(),
        "no panel is painted for a page-spanning box"
    );
}

// --- at (absolute placement) -------------------------------------------------

#[test]
fn at_places_content_at_page_coordinates() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "at", "x": 40, "y": 120, "content": [
            { "type": "box", "fill": "EEEEEE", "content": [
                { "type": "paragraph", "value": "placed" } ] } ] } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let (x, y, _, _) = fill_rects(&doc)[0];
    assert!((x - 40.0).abs() < 0.01, "x was {x}");
    assert!((y - 120.0).abs() < 0.01, "y was {y}");
}

#[test]
fn at_restores_the_flow_cursor() {
    // The two flow paragraphs must end up adjacent: an absolutely placed block
    // between them cannot be allowed to push the second one down.
    let with_at = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "one" },
        { "type": "at", "x": 300, "y": 400, "content": [
            { "type": "paragraph", "value": "elsewhere" } ] },
        { "type": "paragraph", "value": "two" } ] }"#;
    let without = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "one" },
        { "type": "paragraph", "value": "two" } ] }"#;

    fn baseline_of(doc: &LaidOutDoc, want: &str) -> f32 {
        all_ops(doc)
            .iter()
            .find_map(|op| match op {
                DrawOp::Text(td) if td.pieces.iter().any(|p| p.text == want) => Some(td.baseline),
                _ => None,
            })
            .expect("the paragraph")
    }

    let (a, _) = render(with_at, b"{}");
    let (b, _) = render(without, b"{}");
    assert!(
        (baseline_of(&a, "two") - baseline_of(&b, "two")).abs() < 0.01,
        "an `at` block must not move the flow around it"
    );
}

#[test]
fn at_blocks_are_independent_of_each_other() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "at", "x": 40,  "y": 100, "content": [
            { "type": "box", "fill": "EEEEEE", "content": [ { "type": "paragraph", "value": "a" } ] } ] },
        { "type": "at", "x": 300, "y": 100, "content": [
            { "type": "box", "fill": "DDDDDD", "content": [ { "type": "paragraph", "value": "b" } ] } ] } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let rects = fill_rects(&doc);
    assert_eq!(rects.len(), 2);
    assert!(
        (rects[0].1 - rects[1].1).abs() < 0.01,
        "both sit at the same y"
    );
    assert!((rects[0].0 - 40.0).abs() < 0.01 && (rects[1].0 - 300.0).abs() < 0.01);
}

#[test]
fn at_width_constrains_wrapping() {
    let long = "the quick brown fox jumps over the lazy dog and keeps on running past the margin";
    let narrow = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "at", "x": 40, "y": 100, "width": 120, "content": [
                {{ "type": "paragraph", "value": "{long}" }} ] }} ] }}"#
    );
    let wide = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "at", "x": 40, "y": 100, "content": [
                {{ "type": "paragraph", "value": "{long}" }} ] }} ] }}"#
    );
    let (dn, _) = render(narrow.as_bytes(), b"{}");
    let (dw, _) = render(wide.as_bytes(), b"{}");
    assert!(
        texts(&dn).len() > texts(&dw).len(),
        "narrow `at` wraps more"
    );
}

#[test]
fn at_coordinates_are_clamped_to_the_page() {
    // A canvas can hand us a stale or dragged-off coordinate; it must not throw
    // content into negative space where it would vanish silently.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "at", "x": -50, "y": -80, "content": [
            { "type": "box", "fill": "EEEEEE", "content": [
                { "type": "paragraph", "value": "clamped" } ] } ] } ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let (x, y, _, _) = fill_rects(&doc)[0];
    assert!(x >= 0.0 && y >= 0.0, "clamped to ({x},{y})");
}
