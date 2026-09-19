//! Convert an SVG document to a standalone PDF (page 1 is later imported as a
//! Form XObject). Fonts come from the caller — no system-font scan.
//!
//! Parsing is separate from conversion because the raster backend needs the
//! same tree: it hands it to `resvg::render` instead. The type is always
//! spelled `svg2pdf::usvg::Tree`, never `resvg::usvg::Tree`, so that if the
//! two crates ever resolve to different `usvg` versions the result is a
//! compile error rather than a silently duplicated dependency.

use svg2pdf::{ConversionOptions, PageOptions};

use crate::error::{PdfError, Result};

/// Parse `bytes` as SVG. `font_bytes` are loaded into usvg's font database so
/// `<text>` glyphs resolve the same way body text does.
pub fn parse_svg(bytes: &[u8], font_bytes: &[&[u8]]) -> Result<svg2pdf::usvg::Tree> {
    use svg2pdf::usvg;

    let mut opt = usvg::Options::default();
    {
        let db = opt.fontdb_mut();
        for &face in font_bytes {
            db.load_font_data(face.to_vec());
        }
    }
    usvg::Tree::from_data(bytes, &opt).map_err(|e| PdfError::Image(format!("SVG parse: {e}")))
}

/// Convert an already-parsed tree to a standalone PDF, returning it with the
/// tree's intrinsic size (SVG user units, treated as pt at 72 dpi).
pub fn tree_to_pdf(tree: &svg2pdf::usvg::Tree) -> Result<(Vec<u8>, f32, f32)> {
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if w <= 0.0 || h <= 0.0 {
        return Err(PdfError::Image("SVG has zero size".into()));
    }

    let conversion = ConversionOptions {
        compress: true,
        embed_text: true,
        ..ConversionOptions::default()
    };

    let pdf = svg2pdf::to_pdf(tree, conversion, PageOptions { dpi: 72.0 })
        .map_err(|e| PdfError::Image(format!("SVG to PDF: {e}")))?;
    Ok((pdf, w, h))
}

/// Parse `bytes` as SVG and convert to PDF.
pub fn svg_to_pdf(bytes: &[u8], font_bytes: &[&[u8]]) -> Result<(Vec<u8>, f32, f32)> {
    let tree = parse_svg(bytes, font_bytes)?;
    tree_to_pdf(&tree)
}
