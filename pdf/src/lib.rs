//! soli-pdf — render a PDF from a JSON layout template + data, then embed a
//! Factur-X (EN 16931) CII XML to produce a PDF/A-3b electronic invoice.
//!
//! Two halves joined by PDF bytes:
//! 1. a render engine: template + data → a normal PDF;
//! 2. a Factur-X step: that PDF + a caller-provided CII XML → PDF/A-3b.

use std::path::PathBuf;
use std::time::Duration;

pub mod color;
pub mod data;
pub mod draw;
pub mod error;
pub mod facturx;
pub mod fonts;
pub mod geometry;
pub mod images;
pub mod interpolate;
pub mod invoice;
pub mod layout;
pub mod pdf_backend;
pub mod qr;
pub mod render;
pub mod template;
pub mod text;

pub use error::{PdfError, RenderWarning, Result};
pub use facturx::{FacturxMetadata, Profile};
pub use invoice::{Amount, Invoice, Line, Party};
pub use render::{render_to_bytes, render_with_warnings, RenderOutput};

/// Options controlling a render.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Whether to fetch http(s) images. When false, remote images are skipped
    /// (with a warning) instead of fetched — useful for offline/deterministic
    /// runs and tests.
    pub fetch_images: bool,
    /// Timeout for each image fetch.
    pub http_timeout: Duration,
    /// Directories scanned for fallback fonts (`.ttf`/`.otf`/`.ttc`), used for
    /// scripts the bundled Latin font can't cover (e.g. CJK). Loaded in order;
    /// missing directories are ignored.
    pub font_dirs: Vec<PathBuf>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            fetch_images: true,
            http_timeout: Duration::from_secs(15),
            font_dirs: Vec::new(),
        }
    }
}

/// End-to-end convenience: render the template+data to a PDF and embed the
/// provided Factur-X CII XML, returning PDF/A-3b bytes.
pub fn generate_facturx(
    template_json: &[u8],
    data_json: &[u8],
    facturx_xml: &[u8],
    profile: Profile,
    meta: &FacturxMetadata,
    opts: &RenderOptions,
) -> Result<Vec<u8>> {
    let pdf = render_to_bytes(template_json, data_json, opts)?;
    facturx::embed_facturx(&pdf, facturx_xml, profile, meta)
}

/// End-to-end from a single source of truth: an [`Invoice`] drives both the
/// visual PDF (its data maps onto the template's `${...}` paths) and the
/// embedded EN 16931 CII XML (totals and the VAT breakdown computed from the
/// lines), so the two representations are guaranteed consistent.
pub fn generate_facturx_from_invoice(
    template_json: &[u8],
    invoice: &Invoice,
    profile: Profile,
    meta: &FacturxMetadata,
    opts: &RenderOptions,
) -> Result<Vec<u8>> {
    let data = serde_json::to_vec(&invoice.to_render_data()).map_err(PdfError::from)?;
    let pdf = render_to_bytes(template_json, &data, opts)?;
    let xml = invoice.to_cii_xml(profile)?;
    facturx::embed_facturx(&pdf, xml.as_bytes(), profile, meta)
}
