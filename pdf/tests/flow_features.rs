//! Integration tests for the flow/metadata features: the explicit `page_break`
//! element, `#PAGE#`/`#TOTAL_PAGE#` tokens outside the footer (header + body),
//! `${...}` interpolation in the header band, underline/strikethrough
//! decorations, and document metadata on plain renders.

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

/// The concatenated text of a plain `Text` op, if it is one.
fn text_of(op: &DrawOp) -> Option<String> {
    match op {
        DrawOp::Text(td) => Some(td.pieces.iter().map(|p| p.text.as_str()).collect()),
        _ => None,
    }
}

// --- page_break ----------------------------------------------------------------

#[test]
fn page_break_forces_a_new_page() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "First" },
        { "type": "page_break" },
        { "type": "paragraph", "value": "Second" }
    ] }"#;
    let (doc, warnings) = render(tmpl, b"{}");
    assert_eq!(doc.pages.len(), 2, "page_break yields a second page");
    let page2_text: Vec<String> = doc.pages[1].ops.iter().filter_map(text_of).collect();
    assert!(
        page2_text.iter().any(|t| t.contains("Second")),
        "the element after the break landed on page 2: {page2_text:?}"
    );
    let page1_text: Vec<String> = doc.pages[0].ops.iter().filter_map(text_of).collect();
    assert!(
        !page1_text.iter().any(|t| t.contains("Second")),
        "page 1 does not also hold the second paragraph"
    );
    assert!(warnings.is_empty(), "no warnings: {warnings:?}");
}

// --- page tokens + data in the header band --------------------------------------

#[test]
fn header_interpolates_data_and_defers_page_tokens() {
    let tmpl = br#"{ "fonts": ["titillium"],
        "options": { "header_height": 40 },
        "header": [
            { "type": "paragraph", "value": "Invoice ${number} - page #PAGE#/#TOTAL_PAGE#" }
        ],
        "content": [
            { "type": "paragraph", "value": "Body" },
            { "type": "page_break" },
            { "type": "paragraph", "value": "More" }
        ] }"#;
    let (doc, _) = render(tmpl, br#"{ "data": { "number": "F-42" } }"#);
    assert_eq!(doc.pages.len(), 2);
    for (i, page) in doc.pages.iter().enumerate() {
        let deferred: Vec<&str> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                DrawOp::PageText(pt) => Some(pt.raw.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            deferred.len(),
            1,
            "page {} has exactly one deferred header line: {deferred:?}",
            i + 1
        );
        assert!(
            deferred[0].contains("F-42"),
            "`${{number}}` interpolated in the header (page {}): {deferred:?}",
            i + 1
        );
        assert!(
            deferred[0].contains("#PAGE#") && deferred[0].contains("#TOTAL_PAGE#"),
            "page tokens deferred to pass 2 (page {}): {deferred:?}",
            i + 1
        );
    }
}

#[test]
fn body_paragraph_page_tokens_are_deferred_too() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "This is page #PAGE#" }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let deferred = doc.pages[0]
        .ops
        .iter()
        .filter(|op| matches!(op, DrawOp::PageText(_)))
        .count();
    assert_eq!(deferred, 1, "body page tokens defer to pass 2");
}

// --- lineHeight / spacing ----------------------------------------------------------

#[test]
fn line_height_multiplier_changes_line_advance() {
    // Two 2-line paragraphs, default (1.2) vs lineHeight 2.0.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "one\ntwo", "options": { "fontSize": 10 } },
        { "type": "paragraph", "value": "three\nfour", "options": { "fontSize": 10, "lineHeight": 2.0 } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let baselines: Vec<f32> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(td) => Some(td.baseline),
            _ => None,
        })
        .collect();
    assert_eq!(baselines.len(), 4);
    let default_advance = baselines[1] - baselines[0];
    let wide_advance = baselines[3] - baselines[2];
    assert!(
        (default_advance - 12.0).abs() < 0.01,
        "default advance is 1.2 * 10pt: {default_advance}"
    );
    assert!(
        (wide_advance - 20.0).abs() < 0.01,
        "lineHeight 2.0 advance is 2.0 * 10pt: {wide_advance}"
    );
}

#[test]
fn spacing_adds_a_gap_below_the_block() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "a", "options": { "fontSize": 10, "spacing": 30 } },
        { "type": "paragraph", "value": "b", "options": { "fontSize": 10 } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let baselines: Vec<f32> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(td) => Some(td.baseline),
            _ => None,
        })
        .collect();
    let gap = baselines[1] - baselines[0];
    // One line-height (12) + spacing (30).
    assert!((gap - 42.0).abs() < 0.01, "advance includes spacing: {gap}");
}

// --- #PAGE_OF:anchor# ---------------------------------------------------------------

#[test]
fn page_of_tokens_resolve_against_anchors() {
    use soli_pdf::interpolate::substitute_anchor_tokens;
    let mut anchors = std::collections::HashMap::new();
    anchors.insert("sec".to_string(), (3usize, 100.0f32));
    let mut warnings = Vec::new();
    assert_eq!(
        substitute_anchor_tokens("Charts .... p. #PAGE_OF:sec#", &anchors, &mut warnings),
        "Charts .... p. 4",
        "anchor page index is 0-based; token renders 1-based"
    );
    assert!(warnings.is_empty());
    assert_eq!(
        substitute_anchor_tokens("#PAGE_OF:missing#!", &anchors, &mut warnings),
        "!",
    );
    assert_eq!(warnings.len(), 1, "unknown anchor warns");
}

#[test]
fn page_of_lines_defer_to_pass_two() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Chapter ... p. #PAGE_OF:ch1#" },
        { "type": "page_break" },
        { "type": "paragraph", "value": "Chapter one", "options": { "anchor": "ch1" } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let deferred = doc.pages[0]
        .ops
        .iter()
        .any(|op| matches!(op, DrawOp::PageText(_)));
    assert!(deferred, "a #PAGE_OF: line defers like #PAGE# lines");
    assert_eq!(doc.anchors.get("ch1").map(|a| a.0), Some(1));
}

// --- image height / contain fit ------------------------------------------------------

#[test]
fn image_height_and_contain_fit() {
    // The SVG rasterises at 40x40 px (square).
    let svg = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='40' height='40'><rect width='40' height='40' fill='%23123456'/></svg>";
    let tmpl = format!(
        r#"{{ "fonts": ["titillium"], "content": [
            {{ "type": "image", "height": 50, "value": "{svg}" }},
            {{ "type": "image", "width": 100, "height": 30, "value": "{svg}" }}
        ] }}"#
    );
    let (doc, _) = render(tmpl.as_bytes(), b"{}");
    let dims: Vec<(f32, f32)> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Image { w, h, .. } => Some((*w, *h)),
            _ => None,
        })
        .collect();
    assert_eq!(dims.len(), 2);
    // height-only: square aspect -> width == height == 50.
    assert!((dims[0].0 - 50.0).abs() < 0.01 && (dims[0].1 - 50.0).abs() < 0.01);
    // contain in 100x30: square scales to 30x30 (never stretched).
    assert!(
        (dims[1].0 - 30.0).abs() < 0.01 && (dims[1].1 - 30.0).abs() < 0.01,
        "contain fit keeps aspect: {:?}",
        dims[1]
    );
}

// --- paragraph-level italic / mono ------------------------------------------------

#[test]
fn paragraph_options_italic_and_mono_select_their_faces() {
    use soli_pdf::fonts::{FaceKey, FontSlot};
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "plain" },
        { "type": "paragraph", "value": "slanted", "options": { "italic": true } },
        { "type": "paragraph", "value": "typewriter", "options": { "mono": true } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let slot_of = |needle: &str| {
        doc.pages[0]
            .ops
            .iter()
            .find_map(|op| match op {
                DrawOp::Text(td) if td.pieces.iter().any(|p| p.text.contains(needle)) => {
                    Some(td.pieces[0].slot)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no text op containing {needle}"))
    };
    assert_eq!(slot_of("plain"), FontSlot::Styled(FaceKey::default()));
    assert_eq!(
        slot_of("slanted"),
        FontSlot::Styled(FaceKey {
            italic: true,
            ..Default::default()
        }),
        "paragraph-level italic selects the italic face"
    );
    assert_eq!(
        slot_of("typewriter"),
        FontSlot::Styled(FaceKey {
            mono: true,
            ..Default::default()
        }),
        "paragraph-level mono selects the monospace face"
    );
}

// --- underline / strikethrough ---------------------------------------------------

/// Horizontal `Line` ops on a page (decorations; the template has no hr/line
/// elements, so any horizontal stroke comes from a decoration).
fn horizontal_lines(doc: &LaidOutDoc) -> Vec<(f32, f32, f32)> {
    doc.pages
        .iter()
        .flat_map(|p| &p.ops)
        .filter_map(|op| match op {
            DrawOp::Line { x1, y1, x2, y2, .. } if y1 == y2 => Some((*x1, *x2, *y1)),
            _ => None,
        })
        .collect()
}

#[test]
fn underline_draws_a_stroke_below_the_baseline() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Hello", "options": { "underline": true } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let baseline = doc.pages[0]
        .ops
        .iter()
        .find_map(|op| match op {
            DrawOp::Text(td) => Some(td.baseline),
            _ => None,
        })
        .expect("text op");
    let lines = horizontal_lines(&doc);
    assert_eq!(lines.len(), 1, "one underline stroke: {lines:?}");
    let (x1, x2, y) = lines[0];
    assert!(x2 > x1, "non-empty stroke");
    assert!(
        y > baseline && y - baseline < 3.0,
        "underline sits just below the baseline (baseline {baseline}, stroke {y})"
    );
}

#[test]
fn strike_draws_a_stroke_above_the_baseline() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Hello", "options": { "strike": true } }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let baseline = doc.pages[0]
        .ops
        .iter()
        .find_map(|op| match op {
            DrawOp::Text(td) => Some(td.baseline),
            _ => None,
        })
        .expect("text op");
    let lines = horizontal_lines(&doc);
    assert_eq!(lines.len(), 1, "one strike stroke: {lines:?}");
    let (_, _, y) = lines[0];
    assert!(
        y < baseline && baseline - y < 5.0,
        "strike sits above the baseline (baseline {baseline}, stroke {y})"
    );
}

#[test]
fn span_underline_covers_only_that_span() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "spans": [
            { "text": "plain " },
            { "text": "marked", "underline": true },
            { "text": " tail" }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let lines = horizontal_lines(&doc);
    assert_eq!(lines.len(), 1, "exactly one decorated run: {lines:?}");
    let (x1, x2, _) = lines[0];
    // The run starts after "plain " — not at the paragraph's left edge.
    let text_x = doc.pages[0]
        .ops
        .iter()
        .find_map(|op| match op {
            DrawOp::StyledText { x, .. } => Some(*x),
            _ => None,
        })
        .expect("styled text op");
    assert!(
        x1 > text_x + 1.0,
        "underline starts after the undecorated prefix (line x1 {x1}, text x {text_x})"
    );
    assert!(x2 > x1);
}

// --- document metadata on plain renders ------------------------------------------

/// Read a text entry from the PDF's Info dictionary, decoding printpdf's
/// UTF-16BE (BOM-prefixed) strings.
fn info_entry(pdf: &[u8], key: &[u8]) -> String {
    let doc = lopdf::Document::load_mem(pdf).expect("parse pdf");
    let info_ref = doc.trailer.get(b"Info").expect("trailer Info");
    let info_id = info_ref.as_reference().expect("Info is a reference");
    let dict = doc
        .get_object(info_id)
        .and_then(|o| o.as_dict())
        .expect("Info dict");
    let bytes = match dict.get(key) {
        Ok(lopdf::Object::String(b, _)) => b.clone(),
        other => panic!(
            "Info /{}: unexpected {other:?}",
            String::from_utf8_lossy(key)
        ),
    };
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[test]
fn plain_render_carries_metadata() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Body" }
    ] }"#;
    let o = RenderOptions {
        title: Some("Quarterly Report".to_string()),
        author: Some("ACME Corp".to_string()),
        subject: Some("Q3 numbers".to_string()),
        ..opts()
    };
    let pdf = render_to_bytes(tmpl, b"{}", &o).expect("render");
    assert!(pdf.starts_with(b"%PDF"), "valid PDF header");
    assert_eq!(info_entry(&pdf, b"Title"), "Quarterly Report");
    assert_eq!(info_entry(&pdf, b"Author"), "ACME Corp");
    assert_eq!(info_entry(&pdf, b"Subject"), "Q3 numbers");
}

#[test]
fn default_title_stays_invoice() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Body" }
    ] }"#;
    let pdf = render_to_bytes(tmpl, b"{}", &opts()).expect("render");
    assert_eq!(
        info_entry(&pdf, b"Title"),
        "invoice",
        "unset metadata keeps the historical default title"
    );
}

// --- nested list inherits the parent's text styling -----------------------------------

#[test]
fn nested_list_inherits_parent_font_size() {
    // A sublist with no options of its own must render at the parent list's
    // size, not snap back to the 12 pt default (which made sub-items look
    // bigger than their parents).
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "list", "options": { "fontSize": 10 }, "items": [
            "Alpha",
            { "text": "Beta", "list": { "items": ["Gamma"] } }
        ] }
    ] }"#;
    let (doc, _) = render(tmpl, b"{}");
    let sized: Vec<(String, f32)> = doc.pages[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::Text(td) => {
                let text: String = td.pieces.iter().map(|p| p.text.as_str()).collect();
                Some((text, td.size))
            }
            _ => None,
        })
        .collect();
    let size_of = |needle: &str| {
        sized
            .iter()
            .find(|(t, _)| t.contains(needle))
            .map(|(_, s)| *s)
    };
    let parent = size_of("Alpha").expect("parent item line");
    let child = size_of("Gamma").expect("nested item line");
    assert!(
        (parent - 10.0).abs() < 0.01,
        "parent item at 10 pt, got {parent}"
    );
    assert!(
        (child - parent).abs() < 0.01,
        "nested item inherits the parent's 10 pt (not the 12 pt default), got {child}"
    );
}

// --- linkTo jumps to the anchor's page (printpdf Destination is 1-based) ---------------

/// The bytes between the first `start` and the next `end` marker.
fn slice_between<'a>(hay: &'a [u8], start: &[u8], end: &[u8]) -> Option<&'a [u8]> {
    let s = hay.windows(start.len()).position(|w| w == start)? + start.len();
    let rest = &hay[s..];
    let e = rest.windows(end.len()).position(|w| w == end)?;
    Some(&rest[..e])
}

/// The object ids `N` in each `N 0 R` indirect reference inside `bytes`.
fn obj_refs(bytes: &[u8]) -> Vec<u32> {
    let s = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    for (i, _) in s.match_indices(" 0 R") {
        let head = &s[..i];
        let start = head
            .rfind(|c: char| !c.is_ascii_digit())
            .map(|p| p + 1)
            .unwrap_or(0);
        if let Ok(n) = head[start..].parse() {
            out.push(n);
        }
    }
    out
}

#[test]
fn linkto_jumps_to_the_anchor_page() {
    // A linkTo on page 1 points at an anchor on page 3. printpdf's Destination
    // page is 1-based (it does page - 1 to index the page list), so passing the
    // raw 0-based index landed the jump one page short — the #PAGE_OF# number
    // said 3 but the click went to page 2. Regression guard.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "Go", "options": { "linkTo": "c3" } },
        { "type": "page_break" },
        { "type": "paragraph", "value": "two" },
        { "type": "page_break" },
        { "type": "paragraph", "value": "three", "options": { "anchor": "c3" } }
    ] }"#;
    let pdf = render_to_bytes(tmpl, b"{}", &opts()).expect("render");

    // Page objects in reading order, from the Pages tree /Kids array.
    let kids = slice_between(&pdf, b"/Kids[", b"]").expect("kids array");
    let pages = obj_refs(kids);
    assert_eq!(pages.len(), 3, "three page objects");

    // The GoTo link's destination object id.
    let dest = slice_between(&pdf, b"/D[", b"/XYZ").expect("GoTo destination");
    let target = obj_refs(dest).first().copied().expect("dest object id");

    assert_eq!(
        target, pages[2],
        "linkTo lands on the 3rd page (the anchor's), not one page short"
    );
}
