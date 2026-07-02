//! Factur-X / ZUGFeRD embedding: turn a rendered PDF + a caller-provided CII
//! XML into a PDF/A-3b electronic invoice.

mod embed;
mod pdfa;
mod xmp;

pub use pdfa::to_pdfa;

use time::OffsetDateTime;

use crate::error::{PdfError, Result};

/// A Factur-X profile. Determines the `/AFRelationship` and the XMP
/// `fx:ConformanceLevel`, and (for callers generating XML) the BT-24 guideline
/// id. The library embeds caller-provided XML; the profile should match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    Minimum,
    BasicWl,
    Basic,
    #[default]
    En16931,
    Extended,
}

impl Profile {
    /// The CII `GuidelineSpecifiedDocumentContextParameter/ID` (BT-24) value.
    pub fn guideline_id(&self) -> &'static str {
        match self {
            Profile::Minimum => "urn:factur-x.eu:1p0:minimum",
            Profile::BasicWl => "urn:factur-x.eu:1p0:basicwl",
            Profile::Basic => "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic",
            Profile::En16931 => "urn:cen.eu:en16931:2017",
            Profile::Extended => "urn:cen.eu:en16931:2017#conformant#urn:factur-x.eu:1p0:extended",
        }
    }

    /// The XMP `fx:ConformanceLevel` string.
    pub fn xmp_level(&self) -> &'static str {
        match self {
            Profile::Minimum => "MINIMUM",
            Profile::BasicWl => "BASIC WL",
            Profile::Basic => "BASIC",
            Profile::En16931 => "EN 16931",
            Profile::Extended => "EXTENDED",
        }
    }

    /// `/AFRelationship` for the embedded file. MINIMUM/BASIC WL carry only
    /// header data (`Data`); richer profiles are an alternative representation.
    pub fn af_relationship(&self) -> &'static str {
        match self {
            Profile::Minimum | Profile::BasicWl => "Data",
            Profile::Basic | Profile::En16931 | Profile::Extended => "Alternative",
        }
    }

    /// Parse a profile name (case-insensitive, spaces/underscores ignored).
    pub fn parse(s: &str) -> Option<Profile> {
        match s
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '_', '-'], "")
            .as_str()
        {
            "minimum" => Some(Profile::Minimum),
            "basicwl" => Some(Profile::BasicWl),
            "basic" => Some(Profile::Basic),
            "en16931" | "comfort" => Some(Profile::En16931),
            "extended" => Some(Profile::Extended),
            _ => None,
        }
    }
}

/// Document metadata written into both the XMP packet and the Info dictionary.
#[derive(Debug, Clone)]
pub struct FacturxMetadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub producer: String,
    pub creator_tool: String,
    pub created: OffsetDateTime,
}

impl Default for FacturxMetadata {
    fn default() -> Self {
        FacturxMetadata {
            title: "Invoice".to_string(),
            author: String::new(),
            subject: "Factur-X invoice".to_string(),
            producer: concat!("soli-pdf ", env!("CARGO_PKG_VERSION")).to_string(),
            creator_tool: concat!("soli-pdf ", env!("CARGO_PKG_VERSION")).to_string(),
            created: OffsetDateTime::UNIX_EPOCH,
        }
    }
}

/// Embed a Factur-X CII XML into rendered PDF bytes, producing PDF/A-3b bytes.
pub fn embed_facturx(
    pdf: &[u8],
    xml: &[u8],
    profile: Profile,
    meta: &FacturxMetadata,
) -> Result<Vec<u8>> {
    let doc = lopdf::Document::load_mem(pdf)
        .map_err(|e| PdfError::Facturx(format!("could not parse rendered PDF: {e}")))?;
    embed::embed(doc, xml, profile, meta)
}
