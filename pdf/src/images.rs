//! Fetch + decode images referenced by the template.

use std::time::Duration;

use crate::draw::ImageData;
use crate::error::{PdfError, Result};

/// Load and decode an image from an http(s) URL, a `file://` URL / filesystem
/// path, or a `data:` URI. Network fetches are gated by `fetch`.
pub fn load_image(src: &str, fetch: bool, timeout: Duration) -> Result<ImageData> {
    let bytes = fetch_bytes(src, fetch, timeout)?;
    decode(&bytes)
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
            Ok(payload.as_bytes().to_vec())
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

fn decode(bytes: &[u8]) -> Result<ImageData> {
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

#[cfg(test)]
mod tests {
    use super::*;

    // A valid 1x1 red PNG, base64.
    const PNG_1X1: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

    #[test]
    fn decodes_data_uri() {
        let img = load_image(PNG_1X1, false, Duration::from_secs(1)).unwrap();
        assert_eq!((img.width_px, img.height_px), (1, 1));
    }

    #[test]
    fn network_disabled_errors() {
        let e = load_image("https://example.com/x.png", false, Duration::from_secs(1));
        assert!(e.is_err());
    }
}
