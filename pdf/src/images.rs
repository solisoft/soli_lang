//! Fetch + decode images referenced by the template.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::draw::{ImageData, PixelFormat};
use crate::error::{PdfError, Result};

/// Process-wide cache of decoded images, keyed by a hash of the source (plus
/// file mtime+len for filesystem paths, so an edited file re-decodes). PNG
/// decode — and SVG → PDF conversion (usvg parse + svg2pdf) — costs
/// milliseconds per image, yet a server renders the same template logo on
/// every request. Only deterministic sources are cached: `data:` URIs (the
/// URI *is* the content) and local files (guarded by mtime+len); http(s)
/// responses are not.
struct ImageCache {
    /// Sum of `pixels.len()` over all entries, bounding memory.
    bytes: usize,
    map: HashMap<u64, Arc<ImageData>>,
}

/// Decoded-image budget. Raster logos are a few MB; SVG Form PDFs are tens of
/// KB. Wholesale clear on overflow keeps it simple; live sources re-fill.
const IMAGE_CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;

fn image_cache() -> &'static Mutex<ImageCache> {
    static CACHE: OnceLock<Mutex<ImageCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(ImageCache {
            bytes: 0,
            map: HashMap::new(),
        })
    })
}

/// Cache key for `src`, or `None` when the source must not be cached.
/// `font_bytes` shape SVG `<text>` embedding, so a fingerprint of them
/// (count + lengths) is folded in — different font sets must not share hits.
fn cache_key(src: &str, font_bytes: &[&[u8]]) -> Option<u64> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    font_bytes.len().hash(&mut h);
    for b in font_bytes {
        b.len().hash(&mut h);
    }
    if src.starts_with("data:") {
        Some(h.finish())
    } else if src.starts_with("http://") || src.starts_with("https://") {
        None
    } else {
        // Local file: bind the key to what's on disk right now.
        let path = src.strip_prefix("file://").unwrap_or(src);
        let meta = std::fs::metadata(path).ok()?;
        meta.len().hash(&mut h);
        if let Ok(mtime) = meta.modified() {
            mtime.hash(&mut h);
        }
        Some(h.finish())
    }
}

/// Load and decode an image from an http(s) URL, a `file://` URL / filesystem
/// path, or a `data:` URI. Network fetches are gated by `fetch`. `font_bytes`
/// supplies fonts for `<text>` in SVG sources (ignored for raster formats) —
/// pass the already-loaded [`crate::fonts::FontRegistry::all_font_bytes`]
/// rather than re-reading `font_dirs` from disk per image. Decodes of
/// deterministic sources are cached process-wide.
pub fn load_image(
    src: &str,
    fetch: bool,
    timeout: Duration,
    font_bytes: &[&[u8]],
) -> Result<Arc<ImageData>> {
    let key = cache_key(src, font_bytes);
    if let Some(k) = key {
        if let Some(hit) = image_cache().lock().unwrap().map.get(&k) {
            return Ok(hit.clone());
        }
    }
    let bytes = fetch_bytes(src, fetch, timeout)?;
    let mut decoded = decode(&bytes, font_bytes)?;
    // Give cacheable images their source identity, so the PDF backend can
    // also reuse the encoded XObject (plane split + flate) across renders.
    decoded.source_key = key;
    let img = Arc::new(decoded);
    if let Some(k) = key {
        let mut cache = image_cache().lock().unwrap();
        let n = img.cache_bytes();
        if cache.bytes + n > IMAGE_CACHE_MAX_BYTES {
            cache.map.clear();
            cache.bytes = 0;
        }
        cache.bytes += n;
        cache.map.insert(k, img.clone());
    }
    Ok(img)
}

/// Decides whether an `http(s)` image source may be fetched.
///
/// Installed by the host application (`soli` installs its SSRF validator).
/// Unset, network fetches are refused outright rather than silently allowed:
/// this crate cannot know what network it is running on, and the safe default
/// for an image reference that may have come from user-supplied markup is not
/// to make the request.
pub type UrlGuard = fn(&str) -> std::result::Result<(), String>;

/// Resolves a local image path, or refuses it.
///
/// Installed by the host application (`soli` installs its filesystem jail).
/// Unset, local reads are refused: an image `src` can reach this from
/// `pdf_from_markdown(user_markdown)` via `![](/etc/passwd)` or from a template
/// field carrying user data, and an unguarded `fs::read` there is an arbitrary
/// file read whose result is embedded in the generated document.
pub type PathGuard = fn(&str) -> std::result::Result<std::path::PathBuf, String>;

static URL_GUARD: std::sync::OnceLock<UrlGuard> = std::sync::OnceLock::new();
static PATH_GUARD: std::sync::OnceLock<PathGuard> = std::sync::OnceLock::new();

/// Install the host application's image-source policy. Idempotent; the first
/// call wins, so a library user cannot be silently overridden later.
pub fn set_image_source_guards(url: UrlGuard, path: PathGuard) {
    let _ = URL_GUARD.set(url);
    let _ = PATH_GUARD.set(path);
}

fn check_url(src: &str) -> Result<()> {
    match URL_GUARD.get() {
        Some(guard) => guard(src).map_err(|e| PdfError::Image(format!("{src}: {e}"))),
        None => Err(PdfError::Image(format!(
            "refusing to fetch {src}: no image-source policy installed"
        ))),
    }
}

fn resolve_path(src: &str) -> Result<std::path::PathBuf> {
    match PATH_GUARD.get() {
        Some(guard) => guard(src).map_err(|e| PdfError::Image(format!("{src}: {e}"))),
        None => Err(PdfError::Image(format!(
            "refusing to read {src}: no image-source policy installed"
        ))),
    }
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
        // The host's policy decides. Without it there is nothing sensible to
        // check against, and an image URL is often the most user-influenced
        // field in a document template.
        check_url(src)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            // Each hop would need re-validating against the policy above, and
            // this client has no hook for that, so refuse to follow any.
            .redirect(reqwest::redirect::Policy::none())
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
        let resolved = resolve_path(path)?;
        std::fs::read(resolved).map_err(PdfError::from)
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

fn decode(bytes: &[u8], font_bytes: &[&[u8]]) -> Result<ImageData> {
    if looks_like_svg(bytes) {
        return decode_svg(bytes, font_bytes);
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
    Ok(ImageData::raster(
        w,
        h,
        if has_alpha {
            PixelFormat::Rgba8
        } else {
            PixelFormat::Rgb8
        },
        pixels,
    ))
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

/// Convert an SVG into a vector [`ImageData`]: a standalone PDF whose first
/// page the backend imports as a Form XObject. Intrinsic size is the SVG's
/// user-unit size (no quality-upscale pixmap). `<text>` fonts come from
/// `font_bytes`.
fn decode_svg(bytes: &[u8], font_bytes: &[&[u8]]) -> Result<ImageData> {
    let (pdf, w, h) = crate::svg::svg_to_pdf(bytes, font_bytes)?;
    Ok(ImageData {
        width_px: w.round().max(1.0) as usize,
        height_px: h.round().max(1.0) as usize,
        format: PixelFormat::Rgba8,
        pixels: Vec::new(),
        source_key: None,
        vector: Some(pdf),
        opacity: 1.0,
    })
}

/// A copy of `img` with its alpha multiplied by `opacity` (clamped to 0.0–1.0),
/// promoting Gray/RGB sources to RGBA so the backend composites them
/// semi-transparently over whatever is behind. Used to fade a `backgroundImage`
/// into a soft wash. `source_key` is dropped so the faded copy never shares the
/// original's cached XObject.
pub fn faded(img: &ImageData, opacity: f32) -> ImageData {
    let opacity = opacity.clamp(0.0, 1.0);
    if let Some(pdf) = &img.vector {
        return ImageData {
            width_px: img.width_px,
            height_px: img.height_px,
            format: img.format,
            pixels: Vec::new(),
            source_key: None,
            vector: Some(pdf.clone()),
            opacity: img.opacity * opacity,
        };
    }
    let alpha = (opacity * 255.0).round() as u16;
    let scaled = |orig: u16| ((orig * alpha) / 255) as u8;
    let n = img.width_px * img.height_px;
    let src = &img.pixels;
    let mut pixels = Vec::with_capacity(n * 4);
    match img.format {
        PixelFormat::Gray8 => {
            for &g in src.iter().take(n) {
                pixels.extend_from_slice(&[g, g, g, scaled(255)]);
            }
        }
        PixelFormat::Rgb8 => {
            for px in src.chunks_exact(3).take(n) {
                pixels.extend_from_slice(&[px[0], px[1], px[2], scaled(255)]);
            }
        }
        PixelFormat::Rgba8 => {
            for px in src.chunks_exact(4).take(n) {
                pixels.extend_from_slice(&[px[0], px[1], px[2], scaled(px[3] as u16)]);
            }
        }
    }
    ImageData {
        width_px: img.width_px,
        height_px: img.height_px,
        format: PixelFormat::Rgba8,
        pixels,
        source_key: None,
        vector: None,
        opacity: 1.0,
    }
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
    fn faded_scales_alpha_and_promotes_to_rgba() {
        // RGB source: colours kept, a fresh alpha = round(255 * opacity).
        let mut rgb = ImageData::raster(1, 1, PixelFormat::Rgb8, vec![10, 20, 30]);
        rgb.source_key = Some(7);
        let f = faded(&rgb, 0.5);
        assert_eq!(f.format, PixelFormat::Rgba8);
        assert_eq!(f.pixels, vec![10, 20, 30, 128]);
        assert_eq!(f.source_key, None, "faded copy drops the cache key");

        // RGBA source: existing alpha is multiplied by opacity.
        let rgba = ImageData::raster(1, 1, PixelFormat::Rgba8, vec![10, 20, 30, 200]);
        assert_eq!(faded(&rgba, 0.5).pixels, vec![10, 20, 30, 100]);

        // Opacity is clamped to [0, 1].
        assert_eq!(faded(&rgb, 2.0).pixels[3], 255);
        assert_eq!(faded(&rgb, -1.0).pixels[3], 0);
    }

    #[test]
    fn detects_and_converts_svg() {
        // A 100x60 SVG with a filled rect — text-free, so no fonts needed.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60"><rect width="100" height="60" fill="#0a7"/></svg>"##;
        assert!(looks_like_svg(svg));
        let img = decode(svg, &[]).unwrap();
        assert!(img.is_vector(), "SVG becomes a PDF Form, not a pixmap");
        assert_eq!(img.width_px, 100);
        assert_eq!(img.height_px, 60);
        let pdf = img.vector.as_ref().unwrap();
        assert!(pdf.starts_with(b"%PDF"), "svg2pdf emitted a PDF");
    }

    #[test]
    fn svg_data_uri_is_detected() {
        let uri = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><circle cx='5' cy='5' r='5'/></svg>";
        let img = load_image(uri, false, Duration::from_secs(1), &[]).unwrap();
        assert!(img.is_vector());
        assert_eq!((img.width_px, img.height_px), (10, 10));
    }

    #[test]
    fn raster_magic_bytes_are_not_svg() {
        // PNG magic must never be mistaken for SVG.
        assert!(!looks_like_svg(&[
            0x89, b'P', b'N', b'G', b'<', b's', b'v', b'g'
        ]));
    }
}

#[cfg(test)]
mod source_guard_tests {
    use super::*;

    /// With no policy installed, a network image source must be refused rather
    /// than fetched. This crate cannot know what network it is on, and an image
    /// `src` is often the most user-influenced field in a document template —
    /// `pdf_from_markdown` turns `![](url)` straight into one.
    #[test]
    fn network_sources_are_refused_without_a_policy() {
        let err = fetch_bytes(
            "http://169.254.169.254/latest/meta-data/",
            true,
            Duration::from_secs(1),
        )
        .expect_err("an unguarded fetch must not happen");
        let message = err.to_string();
        assert!(
            message.contains("refusing to fetch") || message.contains("policy"),
            "{message}"
        );
    }

    /// Likewise a local path: an unguarded `fs::read` here is an arbitrary file
    /// read whose result is embedded in the generated document.
    #[test]
    fn local_paths_are_refused_without_a_policy() {
        let err = fetch_bytes("/etc/passwd", false, Duration::from_secs(1))
            .expect_err("an unguarded read must not happen");
        let message = err.to_string();
        assert!(
            message.contains("refusing to read") || message.contains("policy"),
            "{message}"
        );
    }

    /// `data:` URIs carry their own bytes and need no policy — they are the one
    /// source that references nothing outside the document.
    #[test]
    fn data_uris_still_work() {
        // "hi" base64-encoded.
        let bytes = fetch_bytes("data:text/plain;base64,aGk=", false, Duration::from_secs(1))
            .expect("data URIs are self-contained");
        assert_eq!(bytes, b"hi");
    }
}
