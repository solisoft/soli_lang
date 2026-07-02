//! 1D barcode generation.
//!
//! Supported symbologies: **Code 128**, **EAN-13**, **EAN-8**, **Code 39**.
//! The bar pattern from [`barcoders`] is rasterised here into an [`ImageData`]
//! (Gray8, black bars on white, with a horizontal quiet zone), so it flows
//! through the same image XObject path as a QR code or any other picture — no
//! image/SVG generator features from `barcoders` are pulled in.

use barcoders::sym::code128::Code128;
use barcoders::sym::code39::Code39;
use barcoders::sym::ean13::EAN13;
use barcoders::sym::ean8::EAN8;

use crate::draw::{ImageData, PixelFormat};
use crate::error::{PdfError, Result};

/// Horizontal pixels per narrow module in the rasterised image.
const MODULE_PX: usize = 3;
/// Quiet-zone width in modules on each side (10× is the Code 128 / EAN floor).
const QUIET_MODULES: usize = 10;
/// Rasterised bar height in pixels. The element scales this to the placed
/// `height` in points, so it only sets the source resolution.
const HEIGHT_PX: usize = 120;

/// Encode `value` in `symbology` and rasterise it to a Gray8 [`ImageData`]
/// (black bars on white, with a quiet zone). Returns an error for an unknown
/// symbology or data the symbology rejects (wrong length, bad characters, …).
pub fn encode_barcode(symbology: &str, value: &str) -> Result<ImageData> {
    let pattern = encode_pattern(symbology, value)?;
    rasterize(&pattern)
}

/// Produce the 0/1 module pattern (one entry per module column) for `value`.
fn encode_pattern(symbology: &str, value: &str) -> Result<Vec<u8>> {
    let err = |m: String| PdfError::Image(format!("barcode: {m}"));
    match symbology.trim().to_ascii_lowercase().as_str() {
        "code128" | "code-128" | "c128" => {
            // barcoders requires a leading character-set selector. Code B
            // (`\u{0181}`) covers all printable ASCII (letters, digits, symbols),
            // so we prepend it transparently and let callers pass plain text.
            Code128::new(format!("\u{0181}{value}"))
                .map(|b| b.encode())
                .map_err(|e| err(format!("code128 rejected value: {e:?}")))
        }
        "ean13" | "ean-13" => EAN13::new(value)
            .map(|b| b.encode())
            .map_err(|e| err(format!("ean13 rejected value: {e:?}"))),
        "ean8" | "ean-8" => EAN8::new(value)
            .map(|b| b.encode())
            .map_err(|e| err(format!("ean8 rejected value: {e:?}"))),
        "code39" | "code-39" => Code39::new(value)
            .map(|b| b.encode())
            .map_err(|e| err(format!("code39 rejected value: {e:?}"))),
        other => Err(err(format!("unknown symbology {other:?}"))),
    }
}

/// Rasterise a 0/1 module row into a Gray8 image: each module column is
/// `MODULE_PX` wide and spans the full height, framed by a quiet zone.
/// Grayscale keeps the buffer — and the flate work the backend does on every
/// save — a third of RGB8.
fn rasterize(pattern: &[u8]) -> Result<ImageData> {
    if pattern.is_empty() {
        return Err(PdfError::Image("barcode: empty bar pattern".into()));
    }
    let modules = pattern.len() + 2 * QUIET_MODULES;
    let width_px = modules * MODULE_PX;
    let height_px = HEIGHT_PX;
    let mut pixels = vec![255u8; width_px * height_px]; // white Gray8

    for (i, &bit) in pattern.iter().enumerate() {
        if bit == 0 {
            continue; // space stays white
        }
        let x0 = (QUIET_MODULES + i) * MODULE_PX;
        for y in 0..height_px {
            let row = y * width_px;
            pixels[row + x0..row + x0 + MODULE_PX].fill(0);
        }
    }

    Ok(ImageData {
        width_px,
        height_px,
        format: PixelFormat::Gray8,
        source_key: None,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_code128_text() {
        let img = encode_barcode("code128", "ABC-1234").unwrap();
        assert_eq!(img.format, PixelFormat::Gray8);
        assert_eq!(img.height_px, HEIGHT_PX);
        assert_eq!(img.pixels.len(), img.width_px * img.height_px);
        // At least one black column exists (a bar was drawn).
        assert!(img.pixels.contains(&0));
    }

    #[test]
    fn encodes_ean13_twelve_digits() {
        // 12 digits; the 13th check digit is computed by the symbology.
        let img = encode_barcode("ean13", "750103131130").unwrap();
        assert_eq!(img.height_px, HEIGHT_PX);
        assert!(img.width_px > 0);
    }

    #[test]
    fn rejects_unknown_symbology() {
        assert!(encode_barcode("pdf417", "x").is_err());
    }

    #[test]
    fn rejects_bad_ean13_length() {
        assert!(encode_barcode("ean13", "123").is_err());
    }

    #[test]
    fn code39_uppercase_alnum() {
        let img = encode_barcode("code39", "SOLI 123").unwrap();
        assert!(img.width_px > 0);
    }
}
