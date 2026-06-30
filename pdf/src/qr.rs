//! QR-code generation for "scan-to-pay" invoices.
//!
//! Two payloads are supported:
//! * **EPC069-12 "GiroCode"** — a SEPA Credit Transfer the buyer scans in their
//!   banking app to pre-fill a payment. EUR only, error-correction level **M**.
//! * **Text** — an arbitrary string, encoded verbatim.
//!
//! The QR module matrix is rasterised here into an [`ImageData`] (RGB8, black on
//! white, with a quiet zone), so it flows through the same image XObject path as
//! any other picture — no network fetch, no PNG round-trip.

use qrcode::{EcLevel, QrCode};

use crate::draw::ImageData;
use crate::error::{PdfError, Result};

/// Pixels per QR module in the rasterised image.
const MODULE_PX: usize = 4;
/// Quiet-zone width in modules (EPC/ISO recommend at least 4).
const QUIET_MODULES: usize = 4;

/// Parse an error-correction level (`"L"`, `"M"`, `"Q"`, `"H"`), defaulting to
/// `M` (which EPC mandates).
pub fn parse_ec_level(s: Option<&str>) -> EcLevel {
    match s.map(|s| s.trim().to_ascii_uppercase()).as_deref() {
        Some("L") => EcLevel::L,
        Some("Q") => EcLevel::Q,
        Some("H") => EcLevel::H,
        _ => EcLevel::M,
    }
}

/// Build an EPC069-12 (GiroCode) SEPA Credit Transfer payload.
///
/// `amount` is an optional decimal string; when present it must be EUR and is
/// re-formatted to two decimals. Returns an error if a required field is missing
/// or a field exceeds its EPC length limit.
pub fn epc_payload(
    name: &str,
    iban: &str,
    bic: &str,
    amount: Option<&str>,
    currency: &str,
    remittance: &str,
    purpose: &str,
) -> Result<String> {
    let err = |m: &str| PdfError::Invoice(format!("EPC QR: {m}"));

    let name = name.trim();
    let iban = iban.trim().replace(' ', "");
    if name.is_empty() {
        return Err(err("beneficiary name is required"));
    }
    if iban.is_empty() {
        return Err(err("IBAN is required"));
    }
    if !currency.trim().eq_ignore_ascii_case("EUR") {
        return Err(err("only EUR is supported (EPC SEPA Credit Transfer)"));
    }
    if name.chars().count() > 70 {
        return Err(err("beneficiary name exceeds 70 characters"));
    }
    if iban.len() > 34 {
        return Err(err("IBAN exceeds 34 characters"));
    }
    if remittance.chars().count() > 140 {
        return Err(err("remittance exceeds 140 characters"));
    }

    // Amount line: "EUR<amount>" with two decimals, or empty (payer fills it in).
    let amount_line = match amount.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => {
            let v: f64 = a
                .replace(',', ".")
                .parse()
                .map_err(|_| err("amount is not a number"))?;
            if !(0.01..=999_999_999.99).contains(&v) {
                return Err(err("amount out of EPC range (0.01..=999999999.99)"));
            }
            format!("EUR{v:.2}")
        }
        None => String::new(),
    };

    // EPC069-12 v002 lines. Line 10 (structured creditor reference) is left empty
    // because we carry the invoice number as unstructured remittance (line 11).
    let mut lines = vec![
        "BCD".to_string(),
        "002".to_string(),
        "1".to_string(), // character set 1 = UTF-8
        "SCT".to_string(),
        bic.trim().to_string(),
        name.to_string(),
        iban,
        amount_line,
        purpose.trim().to_string(),
        String::new(),
        remittance.trim().to_string(),
    ];
    // Trailing empty fields may be omitted.
    while matches!(lines.last(), Some(l) if l.is_empty()) {
        lines.pop();
    }
    let payload = lines.join("\n");
    if payload.len() > 331 {
        return Err(err("payload exceeds 331 bytes"));
    }
    Ok(payload)
}

/// Encode `payload` as a QR code and rasterise it to an RGB8 [`ImageData`]
/// (black modules on white, with a quiet zone).
pub fn encode_qr(payload: &str, ec: EcLevel) -> Result<ImageData> {
    let code = QrCode::with_error_correction_level(payload.as_bytes(), ec)
        .map_err(|e| PdfError::Image(format!("QR encode failed: {e}")))?;
    let modules = code.width(); // module count per side (no quiet zone)
    let colors = code.to_colors();

    let side_modules = modules + 2 * QUIET_MODULES;
    let side_px = side_modules * MODULE_PX;
    let mut pixels = vec![255u8; side_px * side_px * 3]; // white RGB8

    for py in 0..side_px {
        let my = py / MODULE_PX;
        if my < QUIET_MODULES || my >= QUIET_MODULES + modules {
            continue; // quiet zone row stays white
        }
        let row = my - QUIET_MODULES;
        for px in 0..side_px {
            let mx = px / MODULE_PX;
            if mx < QUIET_MODULES || mx >= QUIET_MODULES + modules {
                continue;
            }
            let col = mx - QUIET_MODULES;
            if colors[row * modules + col] == qrcode::Color::Dark {
                let off = (py * side_px + px) * 3;
                pixels[off] = 0;
                pixels[off + 1] = 0;
                pixels[off + 2] = 0;
            }
        }
    }

    Ok(ImageData {
        width_px: side_px,
        height_px: side_px,
        has_alpha: false,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epc_payload_for_sample_invoice() {
        let p = epc_payload(
            "PDFx",
            "FR7630006000011234567890189",
            "",
            Some("600.00"),
            "EUR",
            "#12345",
            "",
        )
        .unwrap();
        let lines: Vec<&str> = p.split('\n').collect();
        assert_eq!(lines[0], "BCD");
        assert_eq!(lines[1], "002");
        assert_eq!(lines[2], "1");
        assert_eq!(lines[3], "SCT");
        assert_eq!(lines[4], ""); // empty BIC
        assert_eq!(lines[5], "PDFx");
        assert_eq!(lines[6], "FR7630006000011234567890189");
        assert_eq!(lines[7], "EUR600.00");
        assert_eq!(lines[8], ""); // purpose
        assert_eq!(lines[9], ""); // structured ref
        assert_eq!(lines[10], "#12345");
    }

    #[test]
    fn epc_rejects_non_eur_and_missing_fields() {
        assert!(epc_payload("PDFx", "FR76...", "", Some("10"), "USD", "x", "").is_err());
        assert!(epc_payload("", "FR76...", "", Some("10"), "EUR", "x", "").is_err());
        assert!(epc_payload("PDFx", "", "", Some("10"), "EUR", "x", "").is_err());
    }

    #[test]
    fn amount_is_normalised_to_two_decimals() {
        let p = epc_payload("N", "FR76", "", Some("12,5"), "EUR", "", "").unwrap();
        assert!(p.contains("EUR12.50"));
    }

    #[test]
    fn encode_produces_square_rgb_with_quiet_zone() {
        let img = encode_qr("hello", EcLevel::M).unwrap();
        assert!(!img.has_alpha);
        assert_eq!(img.width_px, img.height_px);
        assert_eq!(img.pixels.len(), img.width_px * img.height_px * 3);
        // Side is a whole number of modules including the 4-module quiet zone.
        assert_eq!(img.width_px % MODULE_PX, 0);
        let side_modules = img.width_px / MODULE_PX;
        assert!(side_modules >= 2 * QUIET_MODULES + 21); // smallest QR is 21 modules
    }
}
