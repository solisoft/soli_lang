//! Error and warning types for the crate.

use thiserror::Error;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, PdfError>;

/// Fatal errors that abort a render or embed.
#[derive(Debug, Error)]
pub enum PdfError {
    /// The template or data JSON could not be parsed into the expected shape.
    #[error("failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// An I/O error (reading inputs, writing outputs).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A required font could not be parsed/loaded.
    #[error("font error: {0}")]
    Font(String),

    /// An image could not be fetched or decoded.
    #[error("image error: {0}")]
    Image(String),

    /// The PDF backend failed to produce output.
    #[error("pdf backend error: {0}")]
    Backend(String),

    /// Factur-X embedding / PDF-A3 conformance step failed.
    #[error("facturx error: {0}")]
    Facturx(String),

    /// The typed invoice model was invalid (e.g. a malformed date).
    #[error("invoice error: {0}")]
    Invoice(String),

    /// A low-level PDF (lopdf) operation failed during post-processing.
    #[error("lopdf error: {0}")]
    Lopdf(#[from] lopdf::Error),
}

/// Non-fatal issues collected during a render. Surfaced to the caller so a
/// missing glyph or unresolved placeholder degrades gracefully instead of
/// aborting the whole document.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderWarning {
    /// A `${path}` placeholder did not resolve against the data document.
    MissingPath(String),
    /// One or more characters were not covered by any loaded font.
    MissingGlyph { text: String },
    /// A referenced font family was not found; a fallback was used.
    UnknownFont(String),
    /// An image could not be fetched/decoded and was skipped.
    ImageSkipped { src: String, reason: String },
    /// A QR element could not be built/encoded and was skipped.
    QrSkipped { reason: String },
    /// An element/cell was too tall for a page and was allowed to overflow.
    Overflow(String),
}

impl std::fmt::Display for RenderWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderWarning::MissingPath(p) => write!(f, "unresolved placeholder: ${{{p}}}"),
            RenderWarning::MissingGlyph { text } => {
                write!(f, "no font covers some glyphs in {text:?}")
            }
            RenderWarning::UnknownFont(name) => {
                write!(f, "unknown font family {name:?}, using fallback")
            }
            RenderWarning::ImageSkipped { src, reason } => {
                write!(f, "image {src:?} skipped: {reason}")
            }
            RenderWarning::QrSkipped { reason } => {
                write!(f, "qr code skipped: {reason}")
            }
            RenderWarning::Overflow(what) => write!(f, "content overflowed page: {what}"),
        }
    }
}
