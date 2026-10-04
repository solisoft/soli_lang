//! QR codes as inline SVG, for the install links on `/__soli/mobile`.
//!
//! Rendered on the server so the page needs no JavaScript. One `<path>` of
//! unit squares on a white background with the four-module quiet zone the
//! spec asks for; `shape-rendering="crispEdges"` keeps the modules sharp at
//! any size.

use qrcode::{Color, EcLevel, QrCode};

const QUIET_ZONE: usize = 4;

/// An SVG QR code for `text`, `size_px` wide and high.
pub fn svg(text: &str, size_px: u32) -> Result<String, String> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|e| format!("cannot encode QR code: {e}"))?;
    let width = code.width();
    let colors = code.to_colors();
    let mut path = String::new();
    for (i, color) in colors.iter().enumerate() {
        if *color == Color::Dark {
            let (x, y) = (i % width + QUIET_ZONE, i / width + QUIET_ZONE);
            path.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    let full = width + 2 * QUIET_ZONE;
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {full} {full}\" \
width=\"{size_px}\" height=\"{size_px}\" shape-rendering=\"crispEdges\" role=\"img\" \
aria-label=\"QR code\"><rect width=\"{full}\" height=\"{full}\" fill=\"#fff\"/>\
<path fill=\"#000\" d=\"{path}\"/></svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_an_svg_with_a_quiet_zone() {
        let out = svg("https://staging.example.com/__soli/mobile/i/abc", 200).unwrap();
        assert!(out.starts_with("<svg"));
        // Version-4 code for this length: 33 modules + 8 of quiet zone.
        assert!(out.contains("viewBox=\"0 0 41 41\""), "{}", &out[..120]);
        assert!(out.contains("M4 4h1v1h-1z"), "the finder pattern's corner");
    }
}
