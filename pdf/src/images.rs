//! Fetch + decode images referenced by the template.

use std::path::PathBuf;
use std::time::Duration;

use crate::draw::ImageData;
use crate::error::{PdfError, Result};

/// Load and decode an image from an http(s) URL, a `file://` URL / filesystem
/// path, or a `data:` URI. Network fetches are gated by `fetch`. `font_dirs`
/// supplies fonts for `<text>` in SVG sources (ignored for raster formats).
pub fn load_image(
    src: &str,
    fetch: bool,
    timeout: Duration,
    font_dirs: &[PathBuf],
) -> Result<ImageData> {
    let bytes = fetch_bytes(src, fetch, timeout)?;
    decode(&bytes, font_dirs)
}

fn fetch_bytes(src: &str, fetch: bool, timeout: Duration) -> Result<Vec<u8>> {
    if let Some(rest) = src.strip_prefix("data:") {
        // data:[<mediatype>][;base64],<data>
        let comma = rest
            .find(',')
            .ok_or_else(|| PdfError::Image("malformed data URI".into()))?;
        let meta = &rest[..comma];
        let payload = &rest[comma + 1..];
        if meta.contains("base64") {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(payload)
                .map_err(|e| PdfError::Image(format!("base64 decode: {e}")))
        } else {
            // Leniently percent-decode: `%23` → `#` (SVG colors copied from a
            // browser), while a bare `%` not followed by two hex digits stays
            // literal so SVG percentages like `width='50%'` survive.
            Ok(percent_decode_lenient(payload))
        }
    } else if src.starts_with("http://") || src.starts_with("https://") {
        if !fetch {
            return Err(PdfError::Image(format!(
                "network fetch disabled, skipping {src}"
            )));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| PdfError::Image(e.to_string()))?;
        let resp = client
            .get(src)
            .send()
            .map_err(|e| PdfError::Image(format!("GET {src}: {e}")))?
            .error_for_status()
            .map_err(|e| PdfError::Image(format!("GET {src}: {e}")))?;
        Ok(resp
            .bytes()
            .map_err(|e| PdfError::Image(e.to_string()))?
            .to_vec())
    } else {
        let path = src.strip_prefix("file://").unwrap_or(src);
        std::fs::read(path).map_err(PdfError::from)
    }
}

/// Leniently percent-decode a non-base64 `data:` payload. Decodes well-formed
/// `%XX` escapes (so `%23` → `#`), but leaves a `%` that is not followed by two
/// hex digits untouched — SVG legitimately uses `%` for percentages (e.g.
/// `width='50%'`), which a strict decoder would corrupt or reject.
fn percent_decode_lenient(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(hi), Some(lo)) = (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode(bytes: &[u8], font_dirs: &[PathBuf]) -> Result<ImageData> {
    if looks_like_svg(bytes) {
        return decode_svg(bytes, font_dirs);
    }
    let img =
        image::load_from_memory(bytes).map_err(|e| PdfError::Image(format!("decode: {e}")))?;
    let has_alpha = img.color().has_alpha();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let pixels = if has_alpha {
        img.to_rgba8().into_raw()
    } else {
        img.to_rgb8().into_raw()
    };
    Ok(ImageData {
        width_px: w,
        height_px: h,
        has_alpha,
        pixels,
    })
}

/// Sniff whether `bytes` are an SVG document. Raster formats (PNG/JPEG/GIF/WebP)
/// open with binary magic bytes, never `<`, so this only matches XML/SVG text:
/// after skipping a UTF-8 BOM and leading whitespace it must start with `<` and
/// carry an `<svg` root tag within the first kilobyte.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let s = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let start = match s.iter().position(|b| !b.is_ascii_whitespace()) {
        Some(i) => &s[i..],
        None => return false,
    };
    if start.first() != Some(&b'<') {
        return false;
    }
    let head = &start[..start.len().min(1024)];
    contains_ci(head, b"<svg")
}

/// Case-insensitive (ASCII) substring search.
fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return needle.is_empty();
    }
    haystack
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle))
}

/// Rasterise an SVG into an RGBA8 [`ImageData`] via usvg + resvg + tiny-skia, so
/// it flows through the same image XObject path as any raster picture. The SVG
/// is drawn at a quality multiple of its intrinsic size, with the longest edge
/// clamped to `[MIN_EDGE, MAX_EDGE]` px so tiny icons stay crisp when placed large
/// and huge viewBoxes don't blow up memory. `<text>` fonts come from `font_dirs`.
fn decode_svg(bytes: &[u8], font_dirs: &[PathBuf]) -> Result<ImageData> {
    use resvg::{tiny_skia, usvg};

    const QUALITY: f32 = 3.0;
    const MIN_EDGE: f32 = 512.0;
    const MAX_EDGE: f32 = 2048.0;

    let mut opt = usvg::Options::default();
    {
        // Feed fonts to usvg by bytes (`load_font_data`) rather than
        // `load_fonts_dir`, which needs fontdb's `fs` feature — kept off so we
        // don't pull system-font discovery. Scans each dir's top level for faces.
        let db = opt.fontdb_mut();
        for dir in font_dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_ascii_lowercase);
                if matches!(ext.as_deref(), Some("ttf" | "otf" | "ttc")) {
                    if let Ok(bytes) = std::fs::read(&path) {
                        db.load_font_data(bytes);
                    }
                }
            }
        }
    }
    let tree = usvg::Tree::from_data(bytes, &opt)
        .map_err(|e| PdfError::Image(format!("SVG parse: {e}")))?;

    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    // usvg guarantees a positive, finite size, so a plain `<= 0` check is safe.
    if w <= 0.0 || h <= 0.0 {
        return Err(PdfError::Image("SVG has zero size".into()));
    }
    let longest = w.max(h);
    // Vectors re-rasterise cleanly when upscaled, so a small intrinsic size is
    // still pushed up to MIN_EDGE for crispness when placed large.
    let target = (longest * QUALITY).clamp(MIN_EDGE, MAX_EDGE);
    let scale = target / longest;

    let pw = (w * scale).round().max(1.0) as u32;
    let ph = (h * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph)
        .ok_or_else(|| PdfError::Image(format!("SVG pixmap alloc failed ({pw}x{ph})")))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // tiny-skia stores premultiplied RGBA; the backend expects straight alpha
    // (as the `image` crate yields), so demultiply each pixel.
    let mut pixels = Vec::with_capacity((pw as usize) * (ph as usize) * 4);
    for px in pixmap.pixels() {
        let c = px.demultiply();
        pixels.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(ImageData {
        width_px: pw as usize,
        height_px: ph as usize,
        has_alpha: true,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A valid 1x1 red PNG, base64.
    const PNG_1X1: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

    #[test]
    fn decodes_data_uri() {
        let img = load_image(PNG_1X1, false, Duration::from_secs(1), &[]).unwrap();
        assert_eq!((img.width_px, img.height_px), (1, 1));
    }

    #[test]
    fn data_uri_percent_decodes_leniently() {
        // %23 -> '#' (an SVG color copied from a browser).
        assert_eq!(
            percent_decode_lenient("fill='%230f766e'"),
            b"fill='#0f766e'".to_vec()
        );
        assert_eq!(percent_decode_lenient("a%20b%2Fc"), b"a b/c".to_vec());
        // A bare '%' (SVG percentage) and a dangling '%' stay literal.
        assert_eq!(
            percent_decode_lenient("width='50%'"),
            b"width='50%'".to_vec()
        );
        assert_eq!(percent_decode_lenient("x%2"), b"x%2".to_vec());
        // The recommended literal '#' is unchanged.
        assert_eq!(
            percent_decode_lenient("fill='#abc'"),
            b"fill='#abc'".to_vec()
        );
    }

    #[test]
    fn network_disabled_errors() {
        let e = load_image(
            "https://example.com/x.png",
            false,
            Duration::from_secs(1),
            &[],
        );
        assert!(e.is_err());
    }

    #[test]
    fn detects_and_rasterises_svg() {
        // A 100x60 SVG with a filled rect — text-free, so no fonts needed.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60"><rect width="100" height="60" fill="#0a7"/></svg>"##;
        assert!(looks_like_svg(svg));
        let img = decode(svg, &[]).unwrap();
        // Intrinsic 100x60 is upscaled so the longest edge reaches MIN_EDGE (512).
        assert!(img.has_alpha);
        assert_eq!(img.width_px, 512);
        assert_eq!(img.height_px, 307); // 60 * (512/100), rounded
        assert_eq!(img.pixels.len(), img.width_px * img.height_px * 4);
        // The fill is opaque somewhere in the middle.
        let mid = (img.height_px / 2 * img.width_px + img.width_px / 2) * 4;
        assert_eq!(img.pixels[mid + 3], 255);
    }

    #[test]
    fn svg_data_uri_is_detected() {
        let uri = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><circle cx='5' cy='5' r='5'/></svg>";
        let img = load_image(uri, false, Duration::from_secs(1), &[]).unwrap();
        assert!(img.has_alpha);
        assert!(img.width_px >= 512);
    }

    #[test]
    fn raster_magic_bytes_are_not_svg() {
        // PNG magic must never be mistaken for SVG.
        assert!(!looks_like_svg(&[
            0x89, b'P', b'N', b'G', b'<', b's', b'v', b'g'
        ]));
    }
}
