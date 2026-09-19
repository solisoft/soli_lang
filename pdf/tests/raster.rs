//! Integration tests for the raster (PNG) backend.
//!
//! Golden images are deliberately avoided: tiny-skia's SIMD pipelines can
//! differ in the last bit between architectures, so a byte-comparison would be
//! a CI landmine. Instead each `DrawOp` family gets a *structural probe* — a
//! three-line template whose output has one pixel-level property that can only
//! hold if that op was painted correctly. Determinism is asserted within a
//! build, which is what the caching actually needs to guarantee.

use std::time::Duration;

use soli_pdf::{
    render_png_pages, render_png_with_warnings, PageSelection, RasterOptions, RenderOptions,
    RenderWarning,
};

fn opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
        ..Default::default()
    }
}

/// A raster config that keeps the probes small and fast.
fn raster(width: u32) -> RasterOptions {
    RasterOptions {
        width: Some(width),
        ..Default::default()
    }
}

/// Decode a PNG to `(width, height, rgba)`.
fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .expect("valid PNG")
        .to_rgba8();
    (img.width(), img.height(), img.into_raw())
}

fn px(rgba: &(u32, u32, Vec<u8>), x: u32, y: u32) -> [u8; 4] {
    let i = ((y * rgba.0 + x) * 4) as usize;
    [rgba.2[i], rgba.2[i + 1], rgba.2[i + 2], rgba.2[i + 3]]
}

/// Fraction of pixels that are not paper white.
fn ink(rgba: &(u32, u32, Vec<u8>)) -> f32 {
    let inked = rgba
        .2
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] < 250 || p[1] < 250 || p[2] < 250)
        .count();
    inked as f32 / (rgba.0 * rgba.1) as f32
}

/// Render a synthetic template against empty data.
fn render(template: &[u8], r: &RasterOptions) -> Vec<Vec<u8>> {
    render_png_pages(template, b"{}", &opts(), r).expect("raster render")
}

fn render_one(template: &[u8], r: &RasterOptions) -> (u32, u32, Vec<u8>) {
    let pages = render(template, r);
    assert_eq!(pages.len(), 1, "expected a single page");
    decode(&pages[0])
}

// ---------------------------------------------------------------------------
// Shape, size and stability
// ---------------------------------------------------------------------------

#[test]
fn every_page_comes_back_as_a_png() {
    let template = std::fs::read("tests/fixtures/template.json").unwrap();
    let data = std::fs::read("tests/fixtures/data.json").unwrap();
    let out = render_png_with_warnings(&template, &data, &opts(), &raster(400)).expect("render");

    assert!(
        !out.pages.is_empty(),
        "the fixture lays out at least a page"
    );
    for (i, page) in out.pages.iter().enumerate() {
        assert_eq!(page.index, i);
        assert_eq!(
            &page.png[..8],
            b"\x89PNG\r\n\x1a\n",
            "page {i} is not a PNG"
        );
        // The IHDR must agree with the reported size.
        let w = u32::from_be_bytes(page.png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(page.png[20..24].try_into().unwrap());
        assert_eq!((w, h), (page.width, page.height));
        assert_eq!(w, 400, "width was requested exactly");
    }
}

#[test]
fn a4_at_150_dpi_is_the_documented_size() {
    let template = std::fs::read("tests/fixtures/template.json").unwrap();
    let data = std::fs::read("tests/fixtures/data.json").unwrap();
    let r = RasterOptions {
        dpi: 150.0,
        ..Default::default()
    };
    let pages = render_png_pages(&template, &data, &opts(), &r).expect("render");
    let (w, h, _) = decode(&pages[0]);
    // 595.276 x 841.89 pt at 150/72. poppler's pdftoppm ceils where we round,
    // so its long edge was 1755; ours is 1754.
    assert_eq!((w, h), (1240, 1754));
}

#[test]
fn the_same_input_rasterises_to_the_same_bytes() {
    let template = std::fs::read("tests/fixtures/template.json").unwrap();
    let data = std::fs::read("tests/fixtures/data.json").unwrap();
    let a = render_png_pages(&template, &data, &opts(), &raster(300)).unwrap();
    let b = render_png_pages(&template, &data, &opts(), &raster(300)).unwrap();
    assert_eq!(a, b, "rasterisation must be deterministic within a build");
}

#[test]
fn a_real_document_is_neither_blank_nor_solid() {
    let template = std::fs::read("tests/fixtures/template.json").unwrap();
    let data = std::fs::read("tests/fixtures/data.json").unwrap();
    let pages = render_png_pages(&template, &data, &opts(), &raster(600)).unwrap();
    let coverage = ink(&decode(&pages[0]));
    assert!(
        (0.005..0.40).contains(&coverage),
        "ink coverage {coverage} suggests a blank or flooded page"
    );
}

#[test]
fn width_overrides_dpi_end_to_end() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "hello" } ] }"#;
    let r = RasterOptions {
        dpi: 300.0,
        width: Some(400),
        ..Default::default()
    };
    let (w, _, _) = render_one(tmpl, &r);
    assert_eq!(w, 400);
}

// ---------------------------------------------------------------------------
// Structural probes, one per DrawOp family
// ---------------------------------------------------------------------------

#[test]
fn fill_rect_paints_its_colour_and_nothing_else() {
    // A 100x40 red rect at the content origin, on an A4 page at 1 px/pt.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "rect", "width": 100, "height": 40, "fill": "ff0000" } ] }"#;
    let img = render_one(tmpl, &raster(596));

    // Inside the rect: pure red. The margin is 20mm ~ 56.7pt, so (80, 75) is
    // comfortably within a 100x40 box starting near (57, 57).
    assert_eq!(px(&img, 80, 75), [255, 0, 0, 255], "inside the rect");
    // Outside it: paper.
    assert_eq!(px(&img, 400, 600), [255, 255, 255, 255], "outside the rect");
}

#[test]
fn a_rule_paints_a_dark_run_with_clear_paper_above_and_below() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "hr", "color": "000000", "thickness": 2 } ] }"#;
    let img = render_one(tmpl, &raster(596));

    // Find the darkest scanline; it must be the rule, and it must be dark.
    let (w, h, _) = (img.0, img.1, ());
    let darkest = (0..h)
        .min_by_key(|&y| (0..w).map(|x| px(&img, x, y)[0] as u32).sum::<u32>())
        .unwrap();
    assert!(px(&img, w / 2, darkest)[0] < 128, "the rule is dark");
    // Well clear of the rule, the page is paper.
    assert_eq!(px(&img, w / 2, darkest + 20), [255, 255, 255, 255]);
}

#[test]
fn an_ellipse_fills_its_centre_but_not_its_bounding_corner() {
    // This is the bezier walk and the winding rule, together.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "ellipse", "rx": 40, "ry": 40, "fill": "16a34a" } ] }"#;
    let img = render_one(tmpl, &raster(596));

    // Locate the green blob rather than hard-coding layout's origin.
    let (w, h) = (img.0, img.1);
    let green: Vec<(u32, u32)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = px(&img, x, y);
            p[1] > 100 && p[0] < 100 && p[2] < 100
        })
        .collect();
    assert!(!green.is_empty(), "the ellipse painted nothing");

    let (min_x, max_x) = (
        green.iter().map(|p| p.0).min().unwrap(),
        green.iter().map(|p| p.0).max().unwrap(),
    );
    let (min_y, max_y) = (
        green.iter().map(|p| p.1).min().unwrap(),
        green.iter().map(|p| p.1).max().unwrap(),
    );
    // Centre filled…
    let c = px(&img, (min_x + max_x) / 2, (min_y + max_y) / 2);
    assert!(c[1] > 100 && c[0] < 100, "ellipse centre is filled");
    // …bounding-box corner not. A rectangle would fail this.
    let corner = px(&img, min_x + 1, min_y + 1);
    assert!(
        corner[0] > 200 && corner[2] > 200,
        "the bounding-box corner must stay paper, got {corner:?}"
    );
}

#[test]
fn a_dashed_line_leaves_gaps() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "line", "dx": 400, "dy": 0, "color": "000000", "width": 3,
          "dash": [6, 6] } ] }"#;
    let img = render_one(tmpl, &raster(596));

    let (w, h) = (img.0, img.1);
    let row = (0..h)
        .min_by_key(|&y| (0..w).map(|x| px(&img, x, y)[0] as u32).sum::<u32>())
        .unwrap();
    let dark = (0..w).filter(|&x| px(&img, x, row)[0] < 128).count();
    let light = (60..400).filter(|&x| px(&img, x, row)[0] > 200).count();
    assert!(dark > 20, "a dashed line still has ink, got {dark}");
    assert!(light > 20, "a dashed line still has gaps, got {light}");
}

#[test]
fn text_ink_sits_below_the_baseline_ascent_not_above_it() {
    // The single most likely bug is the glyph y-flip. A flipped run would put
    // its ink above the text band instead of in it.
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "HHHHHHHH", "options": { "fontSize": 48 } } ] }"#;
    let img = render_one(tmpl, &raster(596));

    let (w, h) = (img.0, img.1);
    let rows: Vec<u32> = (0..h)
        .filter(|&y| (0..w).any(|x| px(&img, x, y)[0] < 128))
        .collect();
    assert!(!rows.is_empty(), "the paragraph painted nothing");
    let top = *rows.first().unwrap();
    let bottom = *rows.last().unwrap();

    // The top margin is 20mm = 56.7pt, so at 1 px/pt no ink may appear above
    // it — which is exactly what an upside-down glyph transform would do.
    assert!(top >= 50, "text ink starts at row {top}, above the margin");
    assert!(
        bottom < 200,
        "48pt text should not reach row {bottom} — the run looks mirrored"
    );
}

#[test]
fn a_rotated_watermark_paints_off_the_horizontal() {
    let upright = br#"{ "fonts": ["titillium"],
        "options": { "watermark": { "text": "DRAFT", "angle": 0, "fontSize": 60 } },
        "content": [ { "type": "paragraph", "value": "x" } ] }"#;
    let turned = br#"{ "fonts": ["titillium"],
        "options": { "watermark": { "text": "DRAFT", "angle": 45, "fontSize": 60 } },
        "content": [ { "type": "paragraph", "value": "x" } ] }"#;

    let a = render_one(upright, &raster(400));
    let b = render_one(turned, &raster(400));
    assert!(ink(&a) > 0.001, "the upright watermark painted nothing");
    assert!(ink(&b) > 0.001, "the rotated watermark painted nothing");

    // Height of the inked band: a 45-degree watermark is much taller than a
    // horizontal one. If the rotation were dropped the two would match.
    let band = |img: &(u32, u32, Vec<u8>)| {
        let rows: Vec<u32> = (0..img.1)
            .filter(|&y| (0..img.0).any(|x| px(img, x, y)[0] < 240))
            .collect();
        rows.last().unwrap() - rows.first().unwrap()
    };
    assert!(
        band(&b) > band(&a) + 20,
        "rotated band {} vs upright {} — the rotation was not applied",
        band(&b),
        band(&a)
    );
}

#[test]
fn an_svg_logo_is_drawn_from_its_retained_source() {
    // End-to-end proof of ImageData::svg_source -> usvg -> resvg. The PDF
    // backend draws this through a Form XObject, which a rasteriser cannot
    // read, so nothing but the retained source can make this pass.
    let tmpl = br##"{ "fonts": ["titillium"], "content": [
        { "type": "image",
          "value": "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='100' height='60'><rect width='100' height='60' fill='%230000ff'/></svg>",
          "width": 100 } ] }"##;
    let img = render_one(tmpl, &raster(596));

    let blue = img
        .2
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[2] > 200 && p[0] < 80 && p[1] < 80)
        .count();
    assert!(blue > 1000, "the SVG rectangle did not paint (blue={blue})");
}

#[test]
fn a_qr_paints_both_black_and_white_modules() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "qr", "kind": "text", "value": "https://example.test/pay",
          "width": 120 } ] }"#;
    let img = render_one(tmpl, &raster(596));

    let black = img
        .2
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] < 60)
        .count();
    assert!(black > 500, "the QR painted no dark modules");
}

// ---------------------------------------------------------------------------
// Page selection and warnings
// ---------------------------------------------------------------------------

#[test]
fn page_selection_picks_exactly_the_pages_asked_for() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "one" },
        { "type": "page_break" },
        { "type": "paragraph", "value": "two" },
        { "type": "page_break" },
        { "type": "paragraph", "value": "three" } ] }"#;

    let all = render(tmpl, &raster(200));
    assert_eq!(all.len(), 3, "three pages");

    let r = RasterOptions {
        pages: PageSelection::Indices(vec![1]),
        ..raster(200)
    };
    let one = render(tmpl, &r);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0], all[1], "the selected page is page 2, byte for byte");
}

#[test]
fn asking_for_a_page_past_the_end_is_an_error_not_a_short_array() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "only one page" } ] }"#;
    let r = RasterOptions {
        pages: PageSelection::Indices(vec![8]),
        ..raster(200)
    };
    let err = render_png_pages(tmpl, b"{}", &opts(), &r).unwrap_err();
    assert!(
        err.to_string().contains("page 9"),
        "the error should name the page, got {err}"
    );
}

#[test]
fn a_stationery_document_still_previews_and_says_what_is_missing() {
    // A letterhead is composited onto PDF bytes, so the preview cannot show
    // it — but it must still produce the page.
    let letterhead = {
        let tmpl = br#"{ "fonts": ["titillium"], "content": [
            { "type": "paragraph", "value": "LETTERHEAD" } ] }"#;
        soli_pdf::render_to_bytes(tmpl, b"{}", &opts()).expect("letterhead")
    };
    let o = RenderOptions {
        stationery: Some(letterhead),
        ..opts()
    };
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "body" } ] }"#;
    let out = render_png_with_warnings(tmpl, b"{}", &o, &raster(200)).expect("preview succeeds");

    assert_eq!(out.pages.len(), 1);
    assert!(
        out.warnings.iter().any(|w| matches!(
            w,
            RenderWarning::RasterUnsupported { feature } if feature.contains("stationery")
        )),
        "the caller must be told the letterhead is missing, got {:?}",
        out.warnings
    );
}

#[test]
fn a_hostile_dpi_is_refused_rather_than_allocated() {
    let tmpl = br#"{ "fonts": ["titillium"], "content": [
        { "type": "paragraph", "value": "x" } ] }"#;
    let r = RasterOptions {
        dpi: 20_000.0,
        ..Default::default()
    };
    assert!(render_png_pages(tmpl, b"{}", &opts(), &r).is_err());
}
