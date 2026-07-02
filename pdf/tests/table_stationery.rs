//! Integration tests for table polish (zebra striping, per-cell fill, colspan)
//! and the letterhead/stationery underlay.

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

/// All FillRect ops (x, w, color-as-tuple) on page 1.
fn fills(doc: &LaidOutDoc) -> Vec<(f32, f32, (f32, f32, f32))> {
    doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillRect { x, w, color, .. } => Some((*x, *w, (color.r, color.g, color.b))),
            _ => None,
        })
        .collect()
}

// --- zebra striping ---------------------------------------------------------

#[test]
fn stripe_fills_every_second_body_row() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "table", "data": "items",
          "rows": [ [ { "text": "${name}", "width": 200 }, { "text": "${amount}" } ] ],
          "options": { "stripe": "f1f5f9", "padding_y": 4 } }
    ] }"#;
    let data = br#"{ "data": { "items": [
        { "name": "a", "amount": "1" }, { "name": "b", "amount": "2" },
        { "name": "c", "amount": "3" }, { "name": "d", "amount": "4" }
    ] } }"#;
    let (doc, _) = render(tmpl, data);
    // 4 body rows -> rows 2 and 4 striped: exactly two full-width fills.
    let fills = fills(&doc);
    assert_eq!(fills.len(), 2, "two striped rows out of four: {fills:?}");
    // Stripes span the full table width (both columns), not one cell.
    for (_, w, _) in &fills {
        assert!(*w > 400.0, "stripe spans the whole table: width {w}");
    }
}

// --- per-cell fill ------------------------------------------------------------

#[test]
fn cell_fill_paints_one_cell() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "table",
          "rows": [ [ { "text": "plain", "width": 200 },
                      { "text": "highlighted", "fill": "fef3c7" } ] ] }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let fills = fills(&doc);
    assert_eq!(fills.len(), 1, "exactly one filled cell: {fills:?}");
    let (x, w, _) = fills[0];
    // The fill covers the SECOND column (x past the first column's 200pt).
    assert!(x > 200.0, "fill starts at the second column: x {x}");
    assert!(w > 100.0 && w < 400.0, "fill covers one column: w {w}");
}

// --- colspan -------------------------------------------------------------------

#[test]
fn colspan_merges_column_slots() {
    // 3 columns defined by the header; the summary row's first cell spans 2.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "table",
          "header_columns": [ { "text": "A", "width": 100 }, { "text": "B", "width": 100 },
                              { "text": "C", "width": 100 } ],
          "rows": [
            [ { "text": "a" }, { "text": "b" }, { "text": "c" } ],
            [ { "text": "Total", "colspan": 2, "alignment": "right" }, { "text": "42" } ]
          ] }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    // Find the right-aligned "Total" text: with colspan 2 (200pt box) it must
    // end near x=100+100=200+left, i.e. start clearly PAST the first column.
    let texts: Vec<(f32, String)> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(td) => Some((
                td.x,
                td.pieces
                    .iter()
                    .map(|p| p.text.as_str())
                    .collect::<String>(),
            )),
            _ => None,
        })
        .collect();
    let total = texts
        .iter()
        .find(|(_, t)| t == "Total")
        .expect("Total cell");
    let c42 = texts.iter().find(|(_, t)| t == "42").expect("42 cell");
    assert!(
        total.0 > 100.0,
        "right-aligned Total sits in the merged 2-column box, not the first column: x {}",
        total.0
    );
    assert!(
        c42.0 > total.0,
        "the cell after the span lands in the third column"
    );
}

// --- stationery underlay ---------------------------------------------------------

#[test]
fn stationery_draws_letterhead_beneath_every_page() {
    // Build the letterhead with the engine itself: a colored band + name.
    let letterhead_tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "rect", "width": 495, "height": 40, "fill": "0f766e" },
        { "type": "move", "y": 50 },
        { "type": "paragraph", "value": "ACME letterhead" }
    ] }"#;
    let letterhead = render_to_bytes(letterhead_tmpl, b"{}", &opts()).expect("letterhead");

    let doc_tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Page one body" },
        { "type": "page_break" },
        { "type": "paragraph", "value": "Page two body" }
    ] }"#;
    let o = RenderOptions {
        stationery: Some(letterhead),
        ..opts()
    };
    let pdf = render_to_bytes(doc_tmpl, b"{}", &o).expect("render with stationery");

    let doc = lopdf::Document::load_mem(&pdf).expect("parse output");
    let pages: Vec<_> = doc.page_iter().collect();
    assert_eq!(pages.len(), 2, "stationery does not change the page count");
    for (i, page_id) in pages.iter().enumerate() {
        let content = doc.get_page_content(*page_id).expect("content");
        let text = String::from_utf8_lossy(&content);
        assert!(
            text.contains("/SoliLH0 Do"),
            "page {} draws the letterhead XObject: {}",
            i + 1,
            &text[..text.len().min(120)]
        );
        // The underlay must be FIRST (beneath the page's own content).
        let do_pos = text.find("/SoliLH0 Do").unwrap();
        assert!(
            do_pos < 40,
            "underlay comes before the page content (pos {do_pos})"
        );
        // And the XObject must be registered in the page resources.
        let (resources, resource_ids) = doc.get_page_resources(*page_id).expect("resources");
        let has_xobject = resources
            .map(|r| r.get(b"XObject").is_ok())
            .unwrap_or(false)
            || resource_ids.iter().any(|rid| {
                doc.get_dictionary(*rid)
                    .map(|d| d.get(b"XObject").is_ok())
                    .unwrap_or(false)
            });
        assert!(has_xobject, "page {} resources carry XObject", i + 1);
    }
}

#[test]
fn stationery_with_invalid_letterhead_errors() {
    let doc_tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Body" }
    ] }"#;
    let o = RenderOptions {
        stationery: Some(b"not a pdf".to_vec()),
        ..opts()
    };
    let err = render_to_bytes(doc_tmpl, b"{}", &o).expect_err("must fail");
    assert!(
        err.to_string().contains("letterhead"),
        "error names the letterhead: {err}"
    );
}

// --- valign -----------------------------------------------------------------------

#[test]
fn valign_positions_cell_text_within_the_row() {
    // A tall row (forced by a 3-line neighbour) with top/middle/bottom cells.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "table",
          "rows": [ [
            { "text": "one\ntwo\nthree", "width": 160 },
            { "text": "TOP", "width": 100, "valign": "top" },
            { "text": "MID", "width": 100 },
            { "text": "BOT", "width": 100, "valign": "bottom" }
          ] ],
          "options": { "padding_y": 4 } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let baseline_of = |needle: &str| {
        doc.pages[0]
            .ops
            .iter()
            .find_map(|op| match op {
                DrawOp::Text(td) if td.pieces.iter().any(|p| p.text.contains(needle)) => {
                    Some(td.baseline)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no op for {needle}"))
    };
    let top = baseline_of("TOP");
    let mid = baseline_of("MID");
    let bot = baseline_of("BOT");
    assert!(top < mid, "top sits above middle ({top} vs {mid})");
    assert!(mid < bot, "middle sits above bottom ({mid} vs {bot})");
}

// --- table footer row ----------------------------------------------------------------

#[test]
fn footer_row_closes_the_table_and_repeats_on_page_breaks() {
    // Enough data rows to span two pages; the footer must appear on BOTH.
    let mut items = String::new();
    for i in 0..60 {
        if i > 0 {
            items.push(',');
        }
        items.push_str(&format!(r#"{{ "name": "row {i}", "amount": "{i}.00" }}"#));
    }
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "table", "data": "items",
          "header_columns": [ { "text": "NAME", "width": 300, "fontWeight": "bold" },
                              { "text": "AMOUNT", "width": 190, "alignment": "right" } ],
          "footer_columns": [ { "text": "carried forward", "width": 300, "fontWeight": "bold" },
                              { "text": "${doc.total}", "width": 190, "alignment": "right" } ],
          "rows": [ [ { "text": "${name}", "width": 300 },
                      { "text": "${amount}", "width": 190, "alignment": "right" } ] ],
          "options": { "padding_y": 4 } }
    ] }"#;
    let data = format!(r#"{{ "data": {{ "doc": {{ "total": "999.00" }}, "items": [{items}] }} }}"#);
    let (doc, _) = render(tmpl, data.as_bytes());
    assert!(
        doc.pages.len() >= 2,
        "the table paginates: {}",
        doc.pages.len()
    );
    for (i, page) in doc.pages.iter().enumerate() {
        let texts: Vec<String> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                DrawOp::Text(td) => Some(td.pieces.iter().map(|p| p.text.as_str()).collect()),
                _ => None,
            })
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("carried forward")),
            "footer row on page {}: {:?}",
            i + 1,
            texts.iter().take(4).collect::<Vec<_>>()
        );
        assert!(
            texts.iter().any(|t| t.contains("999.00")),
            "footer interpolates against the ROOT data on page {}",
            i + 1
        );
    }
}

// --- attachments ------------------------------------------------------------------------

#[test]
fn attachments_land_in_the_embedded_files_tree_and_af() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Body" }
    ] }"#;
    let o = RenderOptions {
        attachments: vec![
            soli_pdf::Attachment {
                name: "data.csv".to_string(),
                mime: "text/csv".to_string(),
                bytes: b"a,b\n1,2\n".to_vec(),
            },
            soli_pdf::Attachment {
                name: "audit.json".to_string(),
                mime: "application/json".to_string(),
                bytes: b"{}".to_vec(),
            },
        ],
        ..opts()
    };
    let pdf = render_to_bytes(tmpl, b"{}", &o).expect("render");
    let doc = lopdf::Document::load_mem(&pdf).expect("parse");
    let catalog_id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let catalog = doc.get_dictionary(catalog_id).unwrap();

    let names = catalog
        .get(b"Names")
        .and_then(|o| o.as_dict())
        .expect("Names");
    let ef = names
        .get(b"EmbeddedFiles")
        .and_then(|o| o.as_dict())
        .expect("EmbeddedFiles");
    let arr = ef.get(b"Names").and_then(|o| o.as_array()).expect("array");
    let keys: Vec<String> = arr
        .chunks_exact(2)
        .filter_map(|p| match &p[0] {
            lopdf::Object::String(b, _) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        })
        .collect();
    // Name trees must be sorted.
    assert_eq!(keys, vec!["audit.json".to_string(), "data.csv".to_string()]);

    let af = catalog.get(b"AF").and_then(|o| o.as_array()).expect("AF");
    assert_eq!(af.len(), 2, "both filespecs associated via /AF");

    // The embedded stream content survives round-trip.
    let spec_id = arr[3].as_reference().unwrap(); // data.csv filespec
    let spec = doc.get_dictionary(spec_id).unwrap();
    let ef_ref = spec
        .get(b"EF")
        .and_then(|o| o.as_dict())
        .unwrap()
        .get(b"F")
        .unwrap()
        .as_reference()
        .unwrap();
    let stream = doc.get_object(ef_ref).unwrap().as_stream().unwrap();
    let content = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    assert_eq!(content, b"a,b\n1,2\n");
}

// --- encryption ------------------------------------------------------------------------

#[test]
fn encryption_protects_and_round_trips_with_the_password() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Secret" }
    ] }"#;
    let o = RenderOptions {
        encrypt: Some(soli_pdf::EncryptOptions {
            user_password: "open-sesame".to_string(),
            owner_password: "master".to_string(),
            allow: vec!["print".to_string()],
        }),
        ..opts()
    };
    let pdf = render_to_bytes(tmpl, b"{}", &o).expect("render");

    // The document is encrypted: the trailer carries /Encrypt.
    let doc = lopdf::Document::load_mem(&pdf).expect("parse");
    assert!(doc.trailer.get(b"Encrypt").is_ok(), "trailer has /Encrypt");
    // Plain text isn't sitting in the clear (streams are encrypted).
    assert!(
        !pdf.windows(6).any(|w| w == b"Secret"),
        "content is not stored in cleartext"
    );

    // It opens with the user password.
    let mut enc = lopdf::Document::load_mem(&pdf).expect("parse2");
    assert!(
        enc.decrypt("open-sesame").is_ok(),
        "opens with the user password"
    );
}
