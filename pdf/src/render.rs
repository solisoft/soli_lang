//! Orchestrates a render: parse → layout (pass 1) → resolve page tokens
//! (pass 2) → emit PDF bytes, or PNG page images.
//!
//! Both backends share passes 1 and 2 through `lay_out_resolved`, so a preview
//! is laid out by exactly the code that laid out the document.

use std::sync::Arc;

use crate::data::DataDocument;
use crate::draw::{DrawOp, LaidOutDoc, TextDraw, TextPiece};
use crate::error::{PdfError, RenderWarning, Result};
use crate::facturx::FacturxMetadata;
use crate::fonts::FontRegistry;
use crate::interpolate::{substitute_anchor_tokens, substitute_page_tokens};
use crate::layout::Engine;
use crate::pdf_backend;
use crate::raster_backend::{self, RasterOptions};
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

/// Pass 1 + pass 2, shared by every backend: parse, lay out, then resolve the
/// deferred page tokens. The registry comes back as an `Arc` so a caller can
/// keep the faces alive for as long as it paints from them.
fn lay_out_resolved(
    template_json: &[u8],
    data_json: &[u8],
    opts: &RenderOptions,
) -> Result<(LaidOutDoc, Arc<FontRegistry>, Vec<RenderWarning>)> {
    let template = Template::parse(template_json)?;
    let data = DataDocument::parse(data_json)?;
    let fonts = FontRegistry::cached(&opts.font_dirs, &template.fonts)?;

    let engine = Engine::new(&template, &fonts, opts);
    let (mut doc, mut warnings) = engine.layout(&template, &data)?;

    resolve_page_tokens(&mut doc, &fonts, &mut warnings);
    Ok((doc, fonts, warnings))
}

pub fn render_with_warnings(
    template_json: &[u8],
    data_json: &[u8],
    opts: &RenderOptions,
) -> Result<RenderOutput> {
    if opts.pdfa && opts.encrypt.is_some() {
        return Err(PdfError::Pdfa(
            "encryption is incompatible with PDF/A; drop `password` or `pdfa`".to_string(),
        ));
    }
    let (doc, fonts, warnings) = lay_out_resolved(template_json, data_json, opts)?;

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

/// One rasterised page.
#[derive(Debug, Clone)]
pub struct RasterPage {
    /// 0-based index in the source document — not in the returned `Vec`, which
    /// may hold a subset.
    pub index: usize,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

/// One rasterised page as raw premultiplied-free RGBA8, for a caller that
/// wants to encode it itself.
///
/// The host runtime uses this to emit WebP through libwebp — a document page
/// is 3-5x smaller as lossy WebP than as PNG — without paying for a PNG
/// encode and decode on the way.
#[derive(Debug, Clone)]
pub struct RasterPixels {
    pub index: usize,
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, straight (un-premultiplied) alpha.
    pub rgba: Vec<u8>,
}

/// The result of a raster render: the PNG pages plus any non-fatal warnings.
#[derive(Debug, Clone)]
pub struct RasterOutput {
    pub pages: Vec<RasterPage>,
    pub warnings: Vec<RenderWarning>,
}

/// Lay out once, rasterise on demand.
///
/// This is what an interactive preview should hold: `open` costs a normal
/// render's pass 1, and each `page_png` paints only the page asked for. A page
/// pixmap is several megabytes, so nothing here ever holds them all at once.
///
/// It pins the laid-out document — including every decoded image — for its
/// lifetime, so a server keeping one per user needs its own eviction.
pub struct RasterSession {
    doc: LaidOutDoc,
    fonts: Arc<FontRegistry>,
    warnings: Vec<RenderWarning>,
}

impl RasterSession {
    pub fn open(
        template_json: &[u8],
        data_json: &[u8],
        opts: &RenderOptions,
    ) -> Result<RasterSession> {
        let (doc, fonts, mut warnings) = lay_out_resolved(template_json, data_json, opts)?;
        // Tell the caller once, at open, what the preview cannot show.
        warnings.extend(raster_backend::unsupported_warnings(opts));
        Ok(RasterSession {
            doc,
            fonts,
            warnings,
        })
    }

    pub fn page_count(&self) -> usize {
        self.doc.pages.len()
    }

    pub fn page(&self) -> &crate::geometry::Page {
        &self.doc.page
    }

    pub fn warnings(&self) -> &[RenderWarning] {
        &self.warnings
    }

    /// Paint and encode one 0-based page.
    pub fn page_png(&self, index: usize, raster: &RasterOptions) -> Result<RasterPage> {
        let pixmap = raster_backend::render_page(&self.doc, &self.fonts, index, raster)?;
        let (width, height) = (pixmap.width(), pixmap.height());
        let png = raster_backend::encode_png(&pixmap, raster)?;
        Ok(RasterPage {
            index,
            width,
            height,
            png,
        })
    }

    /// Paint one 0-based page and hand back its raw pixels.
    pub fn page_pixels(&self, index: usize, raster: &RasterOptions) -> Result<RasterPixels> {
        let pixmap = raster_backend::render_page(&self.doc, &self.fonts, index, raster)?;
        let (width, height) = (pixmap.width(), pixmap.height());
        Ok(RasterPixels {
            index,
            width,
            height,
            rgba: raster_backend::to_straight_rgba(&pixmap),
        })
    }

    /// The pages a selection resolves to, 0-based.
    pub fn selected(&self, raster: &RasterOptions) -> Result<Vec<usize>> {
        raster_backend::selected_pages(&self.doc, &raster.pages)
    }

    /// Every selected page. Each pixmap is painted, encoded and dropped before
    /// the next is allocated.
    pub fn pages_png(&self, raster: &RasterOptions) -> Result<Vec<RasterPage>> {
        raster_backend::selected_pages(&self.doc, &raster.pages)?
            .into_iter()
            .map(|i| self.page_png(i, raster))
            .collect()
    }
}

/// Rasterise a template + data document to PNG pages, collecting warnings.
pub fn render_png_with_warnings(
    template_json: &[u8],
    data_json: &[u8],
    opts: &RenderOptions,
    raster: &RasterOptions,
) -> Result<RasterOutput> {
    let session = RasterSession::open(template_json, data_json, opts)?;
    let pages = session.pages_png(raster)?;
    Ok(RasterOutput {
        pages,
        warnings: session.warnings,
    })
}

/// Rasterise a template + data document to one PNG per selected page.
pub fn render_png_pages(
    template_json: &[u8],
    data_json: &[u8],
    opts: &RenderOptions,
    raster: &RasterOptions,
) -> Result<Vec<Vec<u8>>> {
    render_png_with_warnings(template_json, data_json, opts, raster)
        .map(|o| o.pages.into_iter().map(|p| p.png).collect())
}

/// How many pages a template + data pair lays out to, without painting
/// anything — for a caller that needs to page through a preview.
pub fn page_count(template_json: &[u8], data_json: &[u8], opts: &RenderOptions) -> Result<usize> {
    lay_out_resolved(template_json, data_json, opts).map(|(doc, _, _)| doc.pages.len())
}
