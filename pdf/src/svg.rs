//! Convert an SVG document to a standalone PDF (page 1 is later imported as a
//! Form XObject). Fonts come from the caller — no system-font scan.

use svg2pdf::{ConversionOptions, PageOptions};

use crate::error::{PdfError, Result};

/// Parse `bytes` as SVG and convert to PDF. `font_bytes` are loaded into usvg's
/// font database so `<text>` glyphs resolve the same way body text does.
pub fn svg_to_pdf(bytes: &[u8], font_bytes: &[&[u8]]) -> Result<(Vec<u8>, f32, f32)> {
    use svg2pdf::usvg;

    let mut opt = usvg::Options::default();
    {
        let db = opt.fontdb_mut();
        for &face in font_bytes {
            db.load_font_data(face.to_vec());
        }
    }
    let tree = usvg::Tree::from_data(bytes, &opt)
        .map_err(|e| PdfError::Image(format!("SVG parse: {e}")))?;

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

    let pdf = svg2pdf::to_pdf(&tree, conversion, PageOptions { dpi: 72.0 })
        .map_err(|e| PdfError::Image(format!("SVG to PDF: {e}")))?;
    Ok((pdf, w, h))
}
