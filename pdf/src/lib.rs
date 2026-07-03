//! soli-pdf — render a PDF from a JSON layout template + data, then embed a
//! Factur-X (EN 16931) CII XML to produce a PDF/A-3b electronic invoice.
//!
//! Two halves joined by PDF bytes:
//! 1. a render engine: template + data → a normal PDF;
//! 2. a Factur-X step: that PDF + a caller-provided CII XML → PDF/A-3b.

use std::path::PathBuf;
use std::time::Duration;

pub mod accessibility;
pub mod attachments;
pub mod barcode;
pub mod chart;
pub mod color;
pub mod data;
pub mod draw;
pub mod encrypt;
pub mod error;
pub mod facturx;
pub mod fonts;
pub mod forms;
pub mod geometry;
pub mod images;
pub mod interpolate;
pub mod invoice;
pub mod layout;
pub mod manipulate;
pub mod pdf_backend;
pub mod qr;
pub mod render;
pub mod sign;
pub mod stationery;
pub mod template;
pub mod text;

pub use attachments::{extract_attachments, extract_facturx, Attachment};
pub use encrypt::EncryptOptions;
pub use error::{PdfError, RenderWarning, Result};
pub use facturx::{FacturxMetadata, Profile};
pub use forms::fill_form;
pub use invoice::{AllowanceCharge, Amount, Invoice, Line, Party};
pub use manipulate::{merge, select_pages, stamp, StampOptions};
pub use render::{render_to_bytes, render_with_warnings, RenderOutput};
pub use sign::{
    embed_cms, prepare_signature, PreparedSignature, SignAppearance, SignMeta,
    DEFAULT_PLACEHOLDER_LEN,
};

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
    /// Document title written to the PDF Info dictionary. Defaults to
    /// `"invoice"` (the historical hardcoded value) when unset. The Factur-X
    /// path overrides this later via its own `FacturxMetadata`.
    pub title: Option<String>,
    /// Document author for the Info dictionary.
    pub author: Option<String>,
    /// Document subject for the Info dictionary.
    pub subject: Option<String>,
    /// Letterhead/stationery PDF bytes drawn *beneath* every page's content.
    /// Page 1 uses the letterhead's first page; later pages use its second
    /// page when present, else the first (see [`stationery`]).
    pub stationery: Option<Vec<u8>>,
    /// Files embedded into the document's attachments panel (see
    /// [`attachments`]). Composes with the Factur-X step.
    pub attachments: Vec<Attachment>,
    /// Password-protect the output (AES-128). Applied last; incompatible with
    /// Factur-X/PDF-A (callers must not combine them).
    pub encrypt: Option<EncryptOptions>,
    /// Produce PDF/A-3b output (archival conformance) without any Factur-X
    /// payload. Incompatible with `encrypt` (PDF/A forbids encryption). When the
    /// template is tagged (`options.tagged`), the output additionally declares
    /// PDF/UA-1 — one file that is both accessible and archival. The Factur-X
    /// entry points imply PDF/A and reject this flag.
    pub pdfa: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            fetch_images: true,
            http_timeout: Duration::from_secs(15),
            font_dirs: Vec::new(),
            title: None,
            author: None,
            subject: None,
            stationery: None,
            attachments: Vec::new(),
            encrypt: None,
            pdfa: false,
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
    reject_pdfa_option_for_facturx(opts)?;
    let pdf = render_to_bytes(template_json, data_json, opts)?;
    facturx::embed_facturx(&pdf, facturx_xml, profile, meta)
}

/// Factur-X output already IS PDF/A-3b; letting the standalone `pdfa` pass run
/// too would write the metadata/OutputIntent twice with ambiguous results.
fn reject_pdfa_option_for_facturx(opts: &RenderOptions) -> Result<()> {
    if opts.pdfa {
        return Err(PdfError::Facturx(
            "PDF/A is implied by Factur-X; drop the `pdfa` option".to_string(),
        ));
    }
    Ok(())
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
    reject_pdfa_option_for_facturx(opts)?;
    let data = serde_json::to_vec(&invoice.to_render_data()).map_err(PdfError::from)?;
    let pdf = render_to_bytes(template_json, &data, opts)?;
    let xml = invoice.to_cii_xml(profile)?;
    facturx::embed_facturx(&pdf, xml.as_bytes(), profile, meta)
}
