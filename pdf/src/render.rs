//! Orchestrates a render: parse → layout (pass 1) → resolve page tokens
//! (pass 2) → emit PDF bytes.

use crate::data::DataDocument;
use crate::draw::{DrawOp, LaidOutDoc, TextDraw, TextPiece};
use crate::error::{PdfError, RenderWarning, Result};
use crate::facturx::FacturxMetadata;
use crate::fonts::FontRegistry;
use crate::interpolate::{substitute_anchor_tokens, substitute_page_tokens};
use crate::layout::Engine;
use crate::pdf_backend;
use crate::template::Template;
use crate::text::align_x;
use crate::RenderOptions;

/// The result of a render: the PDF bytes plus any non-fatal warnings.
#[derive(Debug, Clone)]
pub struct RenderOutput {
    pub pdf: Vec<u8>,
    pub warnings: Vec<RenderWarning>,
}

/// Render a template + data document to PDF bytes, collecting warnings.
/// Lay out a template and report **where every element landed**, without
/// producing a PDF.
///
/// A visual editor cannot compute this itself: a flowing element's position
/// depends on everything before it, which only the layout engine knows. This is
/// what lets an editor hit-test the rendered page.
pub fn layout_boxes(
    template_json: &[u8],
    data_json: &[u8],
    opts: &RenderOptions,
) -> Result<Vec<crate::draw::ElementBox>> {
    let template = Template::parse(template_json)?;
    let data = DataDocument::parse(data_json)?;
    let fonts = FontRegistry::cached(&opts.font_dirs, &template.fonts)?;
    let engine = Engine::new(&template, &fonts, opts);
    let (doc, _warnings) = engine.layout(&template, &data)?;
    Ok(doc.element_boxes)
}

pub fn render_with_warnings(
    template_json: &[u8],
    data_json: &[u8],
    opts: &RenderOptions,
) -> Result<RenderOutput> {
    let template = Template::parse(template_json)?;
    if opts.pdfa && opts.encrypt.is_some() {
        return Err(PdfError::Pdfa(
            "encryption is incompatible with PDF/A; drop `password` or `pdfa`".to_string(),
        ));
    }
    let data = DataDocument::parse(data_json)?;
    let fonts = FontRegistry::cached(&opts.font_dirs, &template.fonts)?;

    let engine = Engine::new(&template, &fonts, opts);
    let (mut doc, mut warnings) = engine.layout(&template, &data)?;

    resolve_page_tokens(&mut doc, &fonts, &mut warnings);

    let mut pdf = pdf_backend::emit(&doc, &fonts, opts)?;
    // Tagged output: build the structure tree from the MCIDs the backend
    // emitted. Before stationery/attachments/encryption so those post-passes
    // (which don't touch structure) run on top.
    if doc.tagged {
        let leaves = pdf_backend::struct_leaves(&doc);
        pdf = crate::accessibility::apply_tags(&pdf, doc.lang.as_deref(), &leaves)?;
    }
    if let Some(letterhead) = &opts.stationery {
        pdf = crate::stationery::apply_stationery(&pdf, letterhead)?;
    }
    if !opts.attachments.is_empty() {
        pdf = crate::attachments::apply_attachments(&pdf, &opts.attachments)?;
    }
    // PDF/A conversion runs after stationery/attachments so imported letterhead
    // fonts and annotations get the conformance fixes too, and the attachments'
    // /AF entries exist (PDF/A-3 associated-files requirement).
    if opts.pdfa {
        pdf = crate::facturx::to_pdfa(&pdf, &pdfa_metadata(opts))?;
    }
    // Encryption must be the LAST pass — it must see every object added above.
    if let Some(enc) = &opts.encrypt {
        pdf = crate::encrypt::apply_encryption(&pdf, enc)?;
    }
    Ok(RenderOutput { pdf, warnings })
}

/// Document metadata for the standalone PDF/A pass, mirroring the Info-dict
/// values the backend writes (`title` defaults to the historical `"invoice"`).
/// `created` comes from the system clock via `SystemTime` (the `time` dep has
/// no `clock` feature).
fn pdfa_metadata(opts: &RenderOptions) -> FacturxMetadata {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    FacturxMetadata {
        title: opts.title.clone().unwrap_or_else(|| "invoice".to_string()),
        author: opts.author.clone().unwrap_or_default(),
        subject: opts.subject.clone().unwrap_or_default(),
        created: time::OffsetDateTime::from_unix_timestamp(secs)
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
        ..Default::default()
    }
}

/// Render a template + data document to PDF bytes.
pub fn render_to_bytes(
    template_json: &[u8],
    data_json: &[u8],
    opts: &RenderOptions,
) -> Result<Vec<u8>> {
    render_with_warnings(template_json, data_json, opts).map(|o| o.pdf)
}

/// Pass 2: replace deferred `PageText` ops (footer page numbers) with concrete
/// positioned text, now that the total page count is known. Alignment x is
/// recomputed because substituting the page count changes the measured width.
fn resolve_page_tokens(
    doc: &mut LaidOutDoc,
    fonts: &FontRegistry,
    warnings: &mut Vec<RenderWarning>,
) {
    let total = doc.pages.len();
    // Anchor targets are read while pages are mutated below — snapshot them.
    let anchors = doc.anchors.clone();
    for (i, page) in doc.pages.iter_mut().enumerate() {
        let page_no = i + 1;
        for op in &mut page.ops {
            if let DrawOp::PageText(pt) = op {
                let text = substitute_page_tokens(&pt.raw, page_no, total);
                let text = substitute_anchor_tokens(&text, &anchors, warnings);
                let runs = fonts.itemize(&text, pt.weight, warnings);
                let width: f32 = runs.iter().map(|r| fonts.measure_run(r, pt.size)).sum();
                let x = align_x(pt.region_left, pt.region_width, width, pt.alignment);
                let pieces = runs
                    .into_iter()
                    .map(|r| TextPiece {
                        slot: r.slot,
                        text: r.text,
                    })
                    .collect();
                *op = DrawOp::Text(TextDraw {
                    x,
                    baseline: pt.baseline,
                    size: pt.size,
                    color: pt.color,
                    pieces,
                });
            }
        }
    }
}
