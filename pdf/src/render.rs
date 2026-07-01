//! Orchestrates a render: parse → layout (pass 1) → resolve page tokens
//! (pass 2) → emit PDF bytes.

use crate::data::DataDocument;
use crate::draw::{DrawOp, LaidOutDoc, TextDraw, TextPiece};
use crate::error::{RenderWarning, Result};
use crate::fonts::FontRegistry;
use crate::interpolate::substitute_page_tokens;
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
pub fn render_with_warnings(
    template_json: &[u8],
    data_json: &[u8],
    opts: &RenderOptions,
) -> Result<RenderOutput> {
    let template = Template::parse(template_json)?;
    let data = DataDocument::parse(data_json)?;
    let fonts = FontRegistry::cached(&opts.font_dirs, &template.fonts)?;

    let engine = Engine::new(&template, &fonts, opts);
    let (mut doc, mut warnings) = engine.layout(&template, &data)?;

    resolve_page_tokens(&mut doc, &fonts, &mut warnings);

    let pdf = pdf_backend::emit(&doc, &fonts)?;
    Ok(RenderOutput { pdf, warnings })
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
    for (i, page) in doc.pages.iter_mut().enumerate() {
        let page_no = i + 1;
        for op in &mut page.ops {
            if let DrawOp::PageText(pt) = op {
                let text = substitute_page_tokens(&pt.raw, page_no, total);
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
