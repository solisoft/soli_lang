//! Integration tests for table cell spanning: `colspan` (pre-existing) and
//! `rowspan` (new). A rowspan cell is drawn once at its own row, tall enough to
//! cover every row it claims, and the rows beneath it supply one fewer cell
//! because its column slots are already taken.

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

/// Every filled rect as `(x, y, w, h)`.
fn fills(doc: &LaidOutDoc) -> Vec<(f32, f32, f32, f32)> {
    all_ops(doc)
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillRect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect()
}

/// A 3-row table whose first cell spans all three rows.
const SPAN3: &[u8] = br#"{ "fonts": ["titillium"], "content": [
  { "type": "table", "header_columns": [],
    "rows": [
      [ { "text": "SPAN", "width": 100, "rowspan": 3, "fill": "EEEEEE" },
        { "text": "a1", "width": 140, "fill": "DDDDDD" } ],
      [ { "text": "b1", "width": 140, "fill": "DDDDDD" } ],
      [ { "text": "c1", "width": 140, "fill": "DDDDDD" } ]
    ],
    "options": { "padding_x": 4, "padding_y": 4 } }
] }"#;

/// Height of the first fill whose width matches `w` (cells are identified by
/// width here; sharing one made an earlier version of these tests compare a
/// rectangle against itself).
fn fill_h_of_width(doc: &LaidOutDoc, w: f32) -> f32 {
    fills(doc)
        .into_iter()
        .find(|f| (f.2 - w).abs() < 1.0)
        .map(|f| f.3)
        .unwrap_or_else(|| panic!("no fill {w} wide in {:?}", fills(doc)))
}

#[test]
fn a_rowspan_cell_is_drawn_once() {
    let (doc, _) = render(SPAN3, b"{}");
    let spans = texts(&doc).iter().filter(|t| *t == "SPAN").count();
    assert_eq!(spans, 1, "the spanning cell renders a single time");
}

#[test]
fn a_rowspan_cell_covers_the_rows_it_claims() {
    let (doc, _) = render(SPAN3, b"{}");
    let span = fill_h_of_width(&doc, 100.0);
    let one_row = fill_h_of_width(&doc, 140.0);
    assert!(
        span > one_row * 2.0,
        "spanning fill {span} should cover ~3 rows of {one_row}"
    );
}

#[test]
fn rows_under_a_span_skip_the_claimed_column() {
    // Rows 2 and 3 supply one cell each, and it must land in the SECOND column
    // (the first is taken), not slide left into the spanning cell's slot.
    let (doc, _) = render(SPAN3, b"{}");
    let xs: Vec<f32> = all_ops(&doc)
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(td) if td.pieces.iter().any(|p| p.text == "b1" || p.text == "c1") => {
                Some(td.x)
            }
            _ => None,
        })
        .collect();
    assert_eq!(xs.len(), 2, "both following rows drew their cell");
    let span_x = all_ops(&doc)
        .iter()
        .find_map(|op| match op {
            DrawOp::Text(td) if td.pieces.iter().any(|p| p.text == "SPAN") => Some(td.x),
            _ => None,
        })
        .expect("the spanning cell");
    for x in xs {
        assert!(x > span_x + 50.0, "cell at {x} must sit right of the span");
    }
}

#[test]
fn rowspan_and_colspan_compose() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
      { "type": "table", "header_columns": [],
        "rows": [
          [ { "text": "R", "width": 80, "rowspan": 2, "fill": "DDDDDD" },
            { "text": "x", "width": 80 }, { "text": "y", "width": 80 } ],
          [ { "text": "wide", "width": 160, "colspan": 2, "fill": "EEEEEE" } ]
        ],
        "options": { "padding_x": 4, "padding_y": 4 } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let t = texts(&doc);
    assert!(t.contains(&"R".to_string()) && t.contains(&"wide".to_string()));
    // The colspan cell starts right of the rowspan column and covers two slots.
    let wide = fills(&doc)
        .into_iter()
        .find(|f| (f.2 - 160.0).abs() < 1.0)
        .expect("the colspan fill spans two 80pt columns");
    let span = fills(&doc)
        .into_iter()
        .find(|f| (f.2 - 80.0).abs() < 1.0)
        .expect("the rowspan fill");
    assert!(
        wide.0 > span.0,
        "the wide cell sits right of the spanning one"
    );
}

#[test]
fn rowspan_beyond_the_last_row_is_clamped() {
    // A span of 9 in a 2-row table must not panic or read past the end.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
      { "type": "table", "header_columns": [],
        "rows": [
          [ { "text": "big", "width": 100, "rowspan": 9, "fill": "DDDDDD" },
            { "text": "a", "width": 100 } ],
          [ { "text": "b", "width": 100 } ]
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let t = texts(&doc);
    assert!(t.contains(&"big".to_string()) && t.contains(&"b".to_string()));
}

#[test]
fn a_rowspan_of_one_behaves_like_no_span() {
    let with = br#"{ "fonts": ["titillium"], "content": [
      { "type": "table", "header_columns": [],
        "rows": [ [ { "text": "a", "width": 100, "rowspan": 1 }, { "text": "b", "width": 100 } ],
                  [ { "text": "c", "width": 100 }, { "text": "d", "width": 100 } ] ] } ] }"#;
    let without = br#"{ "fonts": ["titillium"], "content": [
      { "type": "table", "header_columns": [],
        "rows": [ [ { "text": "a", "width": 100 }, { "text": "b", "width": 100 } ],
                  [ { "text": "c", "width": 100 }, { "text": "d", "width": 100 } ] ] } ] }"#;
    let (a, _) = render(with, b"{}");
    let (b, _) = render(without, b"{}");
    assert_eq!(texts(&a), texts(&b));
}

#[test]
fn data_bound_tables_are_unaffected() {
    // One template row repeated per item: there is nothing to span, and the
    // planner must not be reached or disturb the expansion.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
      { "type": "table", "data": "items", "header_columns": [],
        "rows": [ [ { "text": "${name}", "width": 200 } ] ] } ] }"#;
    let data = br#"{ "data": { "items": [ {"name":"one"}, {"name":"two"}, {"name":"three"} ] } }"#;
    let (doc, _) = render(tmpl, data);
    assert_eq!(texts(&doc), vec!["one", "two", "three"]);
}

#[test]
fn a_spanning_cell_does_not_inflate_its_first_row() {
    // The spanning cell holds far more text than its neighbours. Because it has
    // several rows to occupy, the first row must not stretch to its full height.
    let tall = "word ".repeat(60);
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"], "content": [
          {{ "type": "table", "header_columns": [],
            "rows": [
              [ {{ "text": "{tall}", "width": 100, "rowspan": 4, "fill": "DDDDDD" }},
                {{ "text": "a", "width": 140, "fill": "EEEEEE" }} ],
              [ {{ "text": "b", "width": 140, "fill": "EEEEEE" }} ],
              [ {{ "text": "c", "width": 140, "fill": "EEEEEE" }} ],
              [ {{ "text": "d", "width": 140, "fill": "EEEEEE" }} ]
            ] }} ] }}"#
    );
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    let first_row = fill_h_of_width(&doc, 140.0);
    let spanning = fill_h_of_width(&doc, 100.0);
    assert!(
        first_row < spanning,
        "row 1 ({first_row}) must be shorter than the span ({spanning})"
    );
}
