//! Pass 1: turn a parsed template + data into a [`LaidOutDoc`] of positioned
//! draw operations, handling interpolation, the free cursor + `move`, images,
//! tables (data binding, multi-content cells, borders, header-row repeat on
//! page breaks), pagination, and header/footer bands.

use crate::color::{self, Rgb};
use crate::data::{DataDocument, Resolver};
use crate::draw::{
    DrawOp, ImageData, LaidOutDoc, PageTextDraw, PolyPoint, RenderedPage, StyledPiece, TextDraw,
    TextPiece,
};
use crate::error::{RenderWarning, Result};
use crate::fonts::FontRegistry;
use crate::geometry::{
    named_page_size, Cursor, Margins, Page, A4_HEIGHT_PT, A4_WIDTH_PT, DEFAULT_MARGIN_PT,
};
use crate::images;
use crate::interpolate::{has_page_tokens, interpolate};
use crate::template::{
    Alignment, Cell, CellContent, CellStyle, Element, EllipseEl, FontWeight, HrEl, LineEl,
    PageSpec, Paragraph, QrEl, RectEl, StyledSpan, Table, TableHeaderStyle, Template, Watermark,
};
use crate::text::{align_x, layout_styled_lines, line_height, wrap, StyledSeg};
use crate::RenderOptions;

const DEFAULT_CELL_FONT_SIZE: f32 = 10.0;
const FOOTER_PADDING: f32 = 6.0;

/// A resolved table column.
#[derive(Clone, Copy)]
struct Column {
    x: f32,
    width: f32,
}

/// The layout engine, holding mutable accumulation state for one render.
pub struct Engine<'a> {
    fonts: &'a FontRegistry,
    opts: &'a RenderOptions,
    page: Page,
    cursor: Cursor,
    pages: Vec<RenderedPage>,
    current: Vec<DrawOp>,
    images: Vec<ImageData>,
    image_cache: std::collections::HashMap<String, Option<usize>>,
    warnings: Vec<RenderWarning>,
    bookmarks: Vec<(String, usize)>,
    anchors: std::collections::HashMap<String, (usize, f32)>,
}

impl<'a> Engine<'a> {
    pub fn new(
        template: &Template,
        fonts: &'a FontRegistry,
        opts: &'a RenderOptions,
    ) -> Engine<'a> {
        let margins = match &template.options.margins {
            Some(spec) => {
                let (top, right, bottom, left) = spec.resolve(DEFAULT_MARGIN_PT);
                Margins {
                    top,
                    right,
                    bottom,
                    left,
                }
            }
            None => Margins::default(),
        };
        let (mut pw, mut ph) = match &template.options.page {
            Some(PageSpec::Named(name)) => named_page_size(name),
            Some(PageSpec::Custom { width, height }) => (width.max(1.0), height.max(1.0)),
            None => (A4_WIDTH_PT, A4_HEIGHT_PT),
        };
        if template
            .options
            .orientation
            .as_deref()
            .is_some_and(|o| o.eq_ignore_ascii_case("landscape"))
        {
            std::mem::swap(&mut pw, &mut ph);
        }
        let mut page = Page {
            width: pw,
            height: ph,
            margins,
            header_height: template.options.header_height,
            footer_height: 0.0,
        };
        page.footer_height = footer_band_height(template, fonts);
        Engine {
            fonts,
            opts,
            page,
            cursor: Cursor::new(page.content_left(), page.content_top()),
            pages: Vec::new(),
            current: Vec::new(),
            images: Vec::new(),
            image_cache: std::collections::HashMap::new(),
            warnings: Vec::new(),
            bookmarks: Vec::new(),
            anchors: std::collections::HashMap::new(),
        }
    }

    /// Run pass 1 over the template, returning the laid-out document and the
    /// accumulated warnings.
    pub fn layout(
        mut self,
        template: &Template,
        data: &DataDocument,
    ) -> Result<(LaidOutDoc, Vec<RenderWarning>)> {
        let root = data.resolver();
        self.begin_page(template, &root);
        for el in &template.content {
            self.element(el, data, &root, template)?;
        }
        self.finish_page(template, &root);

        let doc = LaidOutDoc {
            page: self.page,
            pages: self.pages,
            images: self.images,
            bookmarks: self.bookmarks,
            anchors: self.anchors,
        };
        Ok((doc, self.warnings))
    }

    // --- page management ---

    fn begin_page(&mut self, template: &Template, root: &Resolver) {
        self.current = Vec::new();
        self.cursor = Cursor::new(self.page.content_left(), self.page.content_top());
        // Watermark first, so it sits behind everything else on the page.
        if let Some(wm) = &template.options.watermark {
            self.draw_watermark(wm);
        }
        // Draw the header band (its own local cursor at the top).
        if !template.header.is_empty() {
            let saved = self.cursor;
            self.cursor = Cursor::new(self.page.content_left(), self.page.header_top());
            for el in &template.header {
                // Header elements should not paginate; ignore errors softly.
                let _ = self.element(el, &DataDocument::empty(), root, template);
            }
            self.cursor = saved;
        }
    }

    fn finish_page(&mut self, template: &Template, root: &Resolver) {
        // Footer band, stacked from footer_top.
        let mut fy = self.page.footer_top() + FOOTER_PADDING / 2.0;
        for el in &template.footer {
            if let Element::Paragraph(p) = el {
                fy = self.footer_paragraph(p, root, fy);
            }
        }
        let page = std::mem::take(&mut self.current);
        self.pages.push(RenderedPage { ops: page });
    }

    /// Ensure `needed` pt of vertical space remains; otherwise start a new page.
    fn ensure_space(&mut self, needed: f32, template: &Template, root: &Resolver) {
        if self.cursor.y + needed > self.page.content_bottom() {
            self.finish_page(template, root);
            self.begin_page(template, root);
        }
    }

    // --- elements ---

    fn element(
        &mut self,
        el: &Element,
        data: &DataDocument,
        root: &Resolver,
        template: &Template,
    ) -> Result<()> {
        match el {
            Element::Paragraph(p) => self.paragraph(p, root, template),
            Element::Move(m) => self.cursor.move_by(m.x, m.y),
            Element::Image(img) => {
                let text = &img.value;
                if let Some(idx) = self.intern_image(text) {
                    let src = &self.images[idx];
                    let target_w = if img.width > 0.0 {
                        img.width
                    } else {
                        src.width_px as f32
                    };
                    let h = target_w * src.height_px as f32 / src.width_px.max(1) as f32;
                    self.current.push(DrawOp::Image {
                        index: idx,
                        x: self.cursor.x,
                        y: self.cursor.y,
                        w: target_w,
                        h,
                    });
                    // Image placement is explicit; the cursor is not advanced.
                }
            }
            Element::Table(t) => self.table(t, data, root, template)?,
            Element::Hr(h) => self.hr(h, template, root),
            Element::Rect(r) => self.rect(r),
            Element::Line(l) => self.line(l),
            Element::Qr(q) => self.qr(q, root),
            Element::Ellipse(e) => self.ellipse(e),
        }
        Ok(())
    }

    /// A horizontal rule across the content width (or `width` pt); advances the
    /// cursor below it.
    fn hr(&mut self, h: &HrEl, template: &Template, root: &Resolver) {
        let thickness = h.thickness.max(0.0);
        self.ensure_space(thickness, template, root);
        let x1 = self.cursor.x;
        let x2 = match h.width {
            Some(w) if w > 0.0 => self.cursor.x + w,
            _ => self.page.content_right(),
        };
        let y = self.cursor.y + thickness / 2.0;
        let color = color::parse_hex_or(h.color.as_deref(), Rgb::LIGHT_GREY);
        self.current.push(DrawOp::Line {
            x1,
            y1: y,
            x2,
            y2: y,
            width: thickness,
            color,
            dash: dash_px(&h.dash),
        });
        self.cursor.y += thickness + 2.0;
    }

    /// A filled and/or stroked rectangle placed at the cursor (top-left). The
    /// cursor is not advanced — position it with `move`.
    fn rect(&mut self, r: &RectEl) {
        if r.width <= 0.0 || r.height <= 0.0 {
            return;
        }
        let (x, y) = (self.cursor.x, self.cursor.y);
        let fill = r.fill.as_deref().and_then(color::parse_hex);
        let stroke = r.border.as_deref().and_then(color::parse_hex);

        // Rounded corners (or any dashed border) are drawn as a single polygon so
        // the corner curves and dash phase are continuous.
        if let Some(radius) = r.radius.filter(|&r| r > 0.0) {
            let radius = radius.min(r.width / 2.0).min(r.height / 2.0);
            self.current.push(DrawOp::Polygon {
                points: rounded_rect_poly(x, y, r.width, r.height, radius),
                fill,
                stroke,
                stroke_width: r.border_width.max(0.0),
                dash: dash_px(&r.dash),
            });
            return;
        }

        if let Some(fill) = fill {
            self.current.push(DrawOp::FillRect {
                x,
                y,
                w: r.width,
                h: r.height,
                color: fill,
            });
        }
        if let Some(border) = stroke {
            let bw = r.border_width.max(0.0);
            let dash = dash_px(&r.dash);
            let (r2, b2) = (x + r.width, y + r.height);
            for (x1, y1, x2, y2) in [
                (x, y, r2, y),   // top
                (x, b2, r2, b2), // bottom
                (x, y, x, b2),   // left
                (r2, y, r2, b2), // right
            ] {
                self.current.push(DrawOp::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    width: bw,
                    color: border,
                    dash: dash.clone(),
                });
            }
        }
    }

    /// A stroked segment from the cursor to `cursor + (dx, dy)`. No advance.
    fn line(&mut self, l: &LineEl) {
        let color = color::parse_hex_or(l.color.as_deref(), Rgb::BLACK);
        self.current.push(DrawOp::Line {
            x1: self.cursor.x,
            y1: self.cursor.y,
            x2: self.cursor.x + l.dx,
            y2: self.cursor.y + l.dy,
            width: l.width.max(0.0),
            color,
            dash: dash_px(&l.dash),
        });
    }

    /// A filled and/or stroked ellipse whose bounding box's top-left is the
    /// cursor. No advance.
    fn ellipse(&mut self, e: &EllipseEl) {
        if e.rx <= 0.0 || e.ry <= 0.0 {
            return;
        }
        let (cx, cy) = (self.cursor.x + e.rx, self.cursor.y + e.ry);
        self.current.push(DrawOp::Polygon {
            points: ellipse_poly(cx, cy, e.rx, e.ry),
            fill: e.fill.as_deref().and_then(color::parse_hex),
            stroke: e.border.as_deref().and_then(color::parse_hex),
            stroke_width: e.border_width.max(0.0),
            dash: dash_px(&e.dash),
        });
    }

    /// A QR code (EPC "scan-to-pay" or arbitrary text) rasterised through the
    /// image path and placed at the cursor. Degrades to a warning on any error.
    fn qr(&mut self, q: &QrEl, root: &Resolver) {
        fn field(engine: &mut Engine<'_>, root: &Resolver, v: &Option<String>) -> String {
            match v {
                Some(s) => interpolate(s, root, &mut engine.warnings),
                None => String::new(),
            }
        }

        let payload = if q.kind.eq_ignore_ascii_case("text") {
            field(self, root, &q.value)
        } else {
            let name = field(self, root, &q.name);
            let iban = field(self, root, &q.iban);
            let bic = field(self, root, &q.bic);
            let amount = field(self, root, &q.amount);
            let currency = {
                let c = field(self, root, &q.currency);
                if c.is_empty() {
                    "EUR".to_string()
                } else {
                    c
                }
            };
            let remittance = field(self, root, &q.remittance);
            let purpose = field(self, root, &q.purpose);
            match crate::qr::epc_payload(
                &name,
                &iban,
                &bic,
                Some(&amount),
                &currency,
                &remittance,
                &purpose,
            ) {
                Ok(p) => p,
                Err(e) => {
                    self.warnings.push(RenderWarning::QrSkipped {
                        reason: e.to_string(),
                    });
                    return;
                }
            }
        };

        if payload.is_empty() {
            self.warnings.push(RenderWarning::QrSkipped {
                reason: "empty QR payload".to_string(),
            });
            return;
        }

        let img = match crate::qr::encode_qr(&payload, crate::qr::parse_ec_level(None)) {
            Ok(img) => img,
            Err(e) => {
                self.warnings.push(RenderWarning::QrSkipped {
                    reason: e.to_string(),
                });
                return;
            }
        };
        let idx = self.intern_generated_image(img);
        let side = if q.width > 0.0 { q.width } else { 120.0 };
        self.current.push(DrawOp::Image {
            index: idx,
            x: self.cursor.x,
            y: self.cursor.y,
            w: side,
            h: side,
        });
    }

    /// Push a programmatically generated image (e.g. a QR raster) and return its
    /// index, mirroring [`Self::intern_image`] for in-memory bitmaps.
    fn intern_generated_image(&mut self, img: ImageData) -> usize {
        self.images.push(img);
        self.images.len() - 1
    }

    /// Push a centered, rotated watermark for the current page. The text-matrix
    /// origin is computed so the text's cap-height band is centered on the page,
    /// matching printpdf's `TranslateRotate` matrix `[cosθ,-sinθ,sinθ,cosθ,x,y]`
    /// with `θ = (360 - angle)°`.
    fn draw_watermark(&mut self, wm: &Watermark) {
        if wm.text.trim().is_empty() {
            return;
        }
        let size = wm.font_size;
        let weight = wm.font_weight;
        let runs = self.fonts.itemize(&wm.text, weight, &mut self.warnings);
        let width: f32 = runs.iter().map(|r| self.fonts.measure_run(r, size)).sum();
        if width <= 0.0 {
            return;
        }
        let cap = self.fonts.cap_height(weight) * size;
        let pieces: Vec<TextPiece> = runs
            .into_iter()
            .map(|r| TextPiece {
                slot: r.slot,
                text: r.text,
            })
            .collect();

        let theta = (360.0 - wm.angle).to_radians();
        let (s, c) = (theta.sin(), theta.cos());
        let (cx, cy) = (self.page.width / 2.0, self.page.height / 2.0);
        // Map text-space cap-band center (width/2, cap/2) onto the page center.
        let x = cx - c * (width / 2.0) - s * (cap / 2.0);
        let y = cy + s * (width / 2.0) - c * (cap / 2.0);

        let color = color::parse_hex_or(wm.color.as_deref(), Rgb::LIGHT_GREY);
        self.current.push(DrawOp::RotatedText {
            x,
            y,
            angle: wm.angle,
            size,
            color,
            pieces,
        });
    }

    fn paragraph(&mut self, p: &Paragraph, root: &Resolver, template: &Template) {
        if let Some(spans) = &p.spans {
            self.paragraph_styled(p, spans, root, template);
            return;
        }
        let text = interpolate(&p.value, root, &mut self.warnings);
        let size = p.options.font_size;
        let weight = p.options.font_weight;
        let lh = line_height(size);
        let region_left = self.cursor.x;
        let region_width = (self.page.content_right() - self.cursor.x).max(1.0);
        let lines = wrap(self.fonts, &text, weight, size, region_width);
        let mut first = true;
        for line in lines {
            self.ensure_space(lh, template, root);
            // Record outline/jump targets at the paragraph's first drawn line, so
            // the page index reflects any page break the line just triggered.
            if first {
                let page_idx = self.pages.len();
                if let Some(label) = &p.options.bookmark {
                    self.bookmarks.push((label.clone(), page_idx));
                }
                if let Some(id) = &p.options.anchor {
                    self.anchors.insert(id.clone(), (page_idx, self.cursor.y));
                }
                first = false;
            }
            self.draw_text_line(
                &line,
                region_left,
                region_width,
                self.cursor.y,
                size,
                weight,
                p.options.alignment,
                Rgb::BLACK,
                p.options.link.as_deref(),
                p.options.link_to.as_deref(),
            );
            self.cursor.y += lh;
        }
    }

    /// Inline rich text: a paragraph defined as styled `spans`. Wraps the spans
    /// together, then emits one `StyledText` op per line (pieces carry their own
    /// size/color) plus a `Link` op over any linked span's box.
    fn paragraph_styled(
        &mut self,
        p: &Paragraph,
        spans: &[StyledSpan],
        root: &Resolver,
        template: &Template,
    ) {
        let base_size = p.options.font_size;
        let base_weight = p.options.font_weight;

        // Resolve spans into measured segments + a link table.
        let mut links: Vec<String> = Vec::new();
        let mut segs: Vec<StyledSeg> = Vec::new();
        for span in spans {
            let text = interpolate(&span.text, root, &mut self.warnings);
            let link = span.link.as_deref().filter(|s| !s.is_empty()).map(|s| {
                links.push(s.to_string());
                links.len() - 1
            });
            segs.push(StyledSeg {
                text,
                size: span.font_size.unwrap_or(base_size),
                weight: span.font_weight.unwrap_or(base_weight),
                color: span
                    .color
                    .as_deref()
                    .and_then(color::parse_hex)
                    .unwrap_or(Rgb::BLACK),
                link,
            });
        }

        let region_left = self.cursor.x;
        let region_width = (self.page.content_right() - self.cursor.x).max(1.0);
        let lines = layout_styled_lines(self.fonts, &segs, region_width, &mut self.warnings);
        let ascent_ratio = self.fonts.ascent(FontWeight::Normal);

        let mut first = true;
        for line in lines {
            let max_size = line.iter().map(|c| c.size).fold(base_size, f32::max);
            let lh = line_height(max_size);
            self.ensure_space(lh, template, root);
            if first {
                let page_idx = self.pages.len();
                if let Some(label) = &p.options.bookmark {
                    self.bookmarks.push((label.clone(), page_idx));
                }
                if let Some(id) = &p.options.anchor {
                    self.anchors.insert(id.clone(), (page_idx, self.cursor.y));
                }
                first = false;
            }

            let width: f32 = line
                .iter()
                .map(|c| self.fonts.char_advance(c.slot, c.ch, c.size))
                .sum();
            let x0 = align_x(region_left, region_width, width, p.options.alignment);
            let baseline = self.cursor.y + max_size * ascent_ratio;

            // Merge consecutive same-style chars into pieces; track x-ranges of
            // each linked span for the link annotations.
            let mut pieces: Vec<StyledPiece> = Vec::new();
            let mut link_runs: Vec<(f32, f32, usize)> = Vec::new();
            let mut cur_link: Option<(usize, f32)> = None;
            let mut cx = x0;
            for c in &line {
                let cw = self.fonts.char_advance(c.slot, c.ch, c.size);
                match pieces.last_mut() {
                    Some(pp) if pp.slot == c.slot && pp.size == c.size && pp.color == c.color => {
                        pp.text.push(c.ch)
                    }
                    _ => pieces.push(StyledPiece {
                        slot: c.slot,
                        text: c.ch.to_string(),
                        size: c.size,
                        color: c.color,
                    }),
                }
                match (cur_link, c.link) {
                    (None, Some(li)) => cur_link = Some((li, cx)),
                    (Some((li, _)), Some(cli)) if cli == li => {}
                    (Some((li, start)), other) => {
                        link_runs.push((start, cx, li));
                        cur_link = other.map(|n| (n, cx));
                    }
                    (None, None) => {}
                }
                cx += cw;
            }
            if let Some((li, start)) = cur_link {
                link_runs.push((start, cx, li));
            }

            if !pieces.is_empty() {
                self.current.push(DrawOp::StyledText {
                    x: x0,
                    baseline,
                    pieces,
                });
            }
            let line_top = self.cursor.y;
            for (sx, ex, li) in link_runs {
                if let Some(uri) = links.get(li) {
                    self.current.push(DrawOp::Link {
                        x: sx,
                        y: line_top,
                        w: (ex - sx).max(0.0),
                        h: lh,
                        uri: uri.clone(),
                    });
                }
            }
            self.cursor.y += lh;
        }
    }

    /// Draw a single (already-wrapped) line of body text. `link` emits an external
    /// link annotation over the line's box; `link_to` an internal jump to an anchor.
    #[allow(clippy::too_many_arguments)]
    fn draw_text_line(
        &mut self,
        line: &str,
        region_left: f32,
        region_width: f32,
        top: f32,
        size: f32,
        weight: FontWeight,
        alignment: Alignment,
        color: Rgb,
        link: Option<&str>,
        link_to: Option<&str>,
    ) {
        let runs = self.fonts.itemize(line, weight, &mut self.warnings);
        let width: f32 = runs.iter().map(|r| self.fonts.measure_run(r, size)).sum();
        let x = align_x(region_left, region_width, width, alignment);
        let ascent = self.fonts.ascent(weight) * size;
        let pieces = runs
            .into_iter()
            .map(|r| TextPiece {
                slot: r.slot,
                text: r.text,
            })
            .collect::<Vec<_>>();
        if pieces.is_empty() {
            return;
        }
        self.current.push(DrawOp::Text(TextDraw {
            x,
            baseline: top + ascent,
            size,
            color,
            pieces,
        }));
        if let Some(uri) = link.filter(|u| !u.is_empty()) {
            self.current.push(DrawOp::Link {
                x,
                y: top,
                w: width,
                h: line_height(size),
                uri: uri.to_string(),
            });
        }
        if let Some(anchor) = link_to.filter(|a| !a.is_empty()) {
            self.current.push(DrawOp::InternalLink {
                x,
                y: top,
                w: width,
                h: line_height(size),
                anchor: anchor.to_string(),
            });
        }
    }

    fn footer_paragraph(&mut self, p: &Paragraph, root: &Resolver, top: f32) -> f32 {
        let interpolated = interpolate(&p.value, root, &mut self.warnings);
        let size = p.options.font_size;
        let weight = p.options.font_weight;
        let lh = line_height(size);
        let region_left = self.page.content_left();
        let region_width = self.page.content_width();
        let ascent = self.fonts.ascent(weight) * size;
        if has_page_tokens(&interpolated) {
            // Defer to pass 2: page number + alignment recompute.
            self.current.push(DrawOp::PageText(PageTextDraw {
                region_left,
                region_width,
                baseline: top + ascent,
                size,
                color: Rgb::BLACK,
                alignment: p.options.alignment,
                weight,
                raw: interpolated,
            }));
        } else {
            let runs = self
                .fonts
                .itemize(&interpolated, weight, &mut self.warnings);
            let width: f32 = runs.iter().map(|r| self.fonts.measure_run(r, size)).sum();
            let x = align_x(region_left, region_width, width, p.options.alignment);
            let pieces = runs
                .into_iter()
                .map(|r| TextPiece {
                    slot: r.slot,
                    text: r.text,
                })
                .collect();
            self.current.push(DrawOp::Text(TextDraw {
                x,
                baseline: top + ascent,
                size,
                color: Rgb::BLACK,
                pieces,
            }));
        }
        top + lh
    }

    // --- images ---

    /// Decode an image (cached by src). Returns its index, or `None` on failure
    /// (a warning is recorded and the image is skipped).
    fn intern_image(&mut self, src: &str) -> Option<usize> {
        if let Some(cached) = self.image_cache.get(src) {
            return *cached;
        }
        let result = images::load_image(src, self.opts.fetch_images, self.opts.http_timeout);
        let idx = match result {
            Ok(img) => {
                self.images.push(img);
                Some(self.images.len() - 1)
            }
            Err(e) => {
                self.warnings.push(RenderWarning::ImageSkipped {
                    src: src.to_string(),
                    reason: e.to_string(),
                });
                None
            }
        };
        self.image_cache.insert(src.to_string(), idx);
        idx
    }

    // --- tables ---

    fn table(
        &mut self,
        t: &Table,
        data: &DataDocument,
        root: &Resolver,
        template: &Template,
    ) -> Result<()> {
        let table_left = self.cursor.x;
        let available = (self.page.content_right() - table_left).max(1.0);
        let columns = compute_columns(t, available, table_left);
        let pad_x = t.options.padding_x;
        let pad_y = t.options.padding_y;

        // Body rows: either the literal rows or the data-bound expansion.
        let bound_items: Vec<&serde_json::Value> = match &t.data {
            Some(key) => data
                .array(key)
                .map(|a| a.iter().collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };

        // Draw the header row (if any) now, and remember to repeat it per page.
        self.draw_header_row(t, &columns, pad_x, pad_y, template, root);

        if let Some(_key) = &t.data {
            // One template row repeated per item.
            let template_row = t.rows.first();
            if let Some(template_row) = template_row {
                for item in &bound_items {
                    let scoped = root.with_scope(item);
                    self.draw_body_row(
                        template_row,
                        &columns,
                        pad_x,
                        pad_y,
                        &scoped,
                        t,
                        template,
                        root,
                    )?;
                }
            }
        } else {
            for row in &t.rows {
                self.draw_body_row(row, &columns, pad_x, pad_y, root, t, template, root)?;
            }
        }
        Ok(())
    }

    fn draw_header_row(
        &mut self,
        t: &Table,
        columns: &[Column],
        pad_x: f32,
        pad_y: f32,
        template: &Template,
        root: &Resolver,
    ) {
        if t.header_columns.is_empty() {
            return;
        }
        let height = self.row_height(&t.header_columns, columns, pad_x, pad_y, root);
        self.ensure_space(height, template, root);
        self.draw_row_cells(
            &t.header_columns,
            columns,
            pad_x,
            pad_y,
            self.cursor.y,
            height,
            true,
            &t.options.header,
            root,
        );
        self.cursor.y += height;
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_body_row(
        &mut self,
        row: &[Cell],
        columns: &[Column],
        pad_x: f32,
        pad_y: f32,
        scoped: &Resolver,
        t: &Table,
        template: &Template,
        root: &Resolver,
    ) -> Result<()> {
        let height = self.row_height(row, columns, pad_x, pad_y, scoped);
        // Page break: if the row doesn't fit, move to a new page and repeat the
        // table header there.
        if self.cursor.y + height > self.page.content_bottom() {
            self.finish_page(template, root);
            self.begin_page(template, root);
            self.draw_header_row(t, columns, pad_x, pad_y, template, root);
        }
        self.draw_row_cells(
            row,
            columns,
            pad_x,
            pad_y,
            self.cursor.y,
            height,
            false,
            &t.options.header,
            scoped,
        );
        self.cursor.y += height;
        Ok(())
    }

    /// Compute the height a row needs (max over its cells).
    fn row_height(
        &mut self,
        row: &[Cell],
        columns: &[Column],
        pad_x: f32,
        pad_y: f32,
        resolver: &Resolver,
    ) -> f32 {
        let mut max_h = 0.0_f32;
        for (cell, col) in row.iter().zip(columns.iter()) {
            let inner_w = (col.width - 2.0 * pad_x).max(1.0);
            let h = self.cell_content_height(cell, inner_w, resolver);
            max_h = max_h.max(h);
        }
        max_h + 2.0 * pad_y
    }

    fn cell_content_height(&mut self, cell: &Cell, inner_w: f32, resolver: &Resolver) -> f32 {
        match cell {
            Cell::Text(tc) => {
                let size = tc.style.font_size.unwrap_or(DEFAULT_CELL_FONT_SIZE);
                let weight = tc.style.font_weight.unwrap_or(FontWeight::Normal);
                let text = interpolate(&tc.text, resolver, &mut self.warnings);
                let lines = wrap(self.fonts, &text, weight, size, inner_w);
                lines.len() as f32 * line_height(size)
            }
            Cell::Rich(rc) => {
                let mut h = 0.0;
                for item in &rc.content {
                    match item {
                        CellContent::Text(rt) => {
                            let size = rt.font_size.unwrap_or(DEFAULT_CELL_FONT_SIZE);
                            let weight = rt.font_weight.unwrap_or(FontWeight::Normal);
                            let text = interpolate(&rt.value, resolver, &mut self.warnings);
                            let lines = wrap(self.fonts, &text, weight, size, inner_w);
                            h += lines.len() as f32 * line_height(size);
                        }
                        CellContent::Image(img) => {
                            if let Some(idx) = self.intern_image(&img.value) {
                                let src = &self.images[idx];
                                let w = if img.width > 0.0 { img.width } else { inner_w };
                                h += w * src.height_px as f32 / src.width_px.max(1) as f32;
                            }
                        }
                    }
                }
                h
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row_cells(
        &mut self,
        row: &[Cell],
        columns: &[Column],
        pad_x: f32,
        pad_y: f32,
        row_top: f32,
        row_height: f32,
        is_header: bool,
        header_style: &TableHeaderStyle,
        resolver: &Resolver,
    ) {
        for (cell, col) in row.iter().zip(columns.iter()) {
            self.draw_one_cell(
                cell,
                *col,
                pad_x,
                pad_y,
                row_top,
                row_height,
                is_header,
                header_style,
                resolver,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_one_cell(
        &mut self,
        cell: &Cell,
        col: Column,
        pad_x: f32,
        pad_y: f32,
        row_top: f32,
        row_height: f32,
        is_header: bool,
        header_style: &TableHeaderStyle,
        resolver: &Resolver,
    ) {
        let style = cell.style();

        // Header fill.
        if is_header {
            if let Some(fill) = header_style
                .fill_color
                .as_deref()
                .and_then(color::parse_hex)
            {
                self.current.push(DrawOp::FillRect {
                    x: col.x,
                    y: row_top,
                    w: col.width,
                    h: row_height,
                    color: fill,
                });
            }
        }

        // Borders.
        self.draw_cell_borders(style, header_style, is_header, col, row_top, row_height);

        // Content.
        let inner_left = col.x + pad_x;
        let inner_w = (col.width - 2.0 * pad_x).max(1.0);
        let text_color = if is_header {
            header_style
                .text_color
                .as_deref()
                .and_then(color::parse_hex)
                .unwrap_or(Rgb::BLACK)
        } else {
            Rgb::BLACK
        };

        match cell {
            Cell::Text(tc) => {
                let size = tc.style.font_size.unwrap_or(DEFAULT_CELL_FONT_SIZE);
                let weight = tc.style.font_weight.unwrap_or(FontWeight::Normal);
                let align = tc.style.alignment.unwrap_or(Alignment::Left);
                let text = interpolate(&tc.text, resolver, &mut self.warnings);
                let lines = wrap(self.fonts, &text, weight, size, inner_w);
                // Optically center the cap-height band (uppercase/digits) within
                // the row — this is what reads as "centered" for tabular data.
                // Centering the full ascent box instead would ride high, because
                // fonts like Titillium have an ascender far taller than their
                // caps; descenders hang naturally below, as in normal typesetting.
                let lh = line_height(size);
                let ascent = self.fonts.ascent(weight) * size;
                let cap = self.fonts.cap_height(weight) * size;
                let block = (lines.len() as f32 - 1.0) * lh + cap;
                let mut y = row_top + (row_height - block) / 2.0 - ascent + cap;
                for line in lines {
                    self.draw_text_line(
                        &line,
                        inner_left,
                        inner_w,
                        y,
                        size,
                        weight,
                        align,
                        text_color,
                        tc.style.link.as_deref(),
                        None,
                    );
                    y += lh;
                }
            }
            Cell::Rich(rc) => {
                let align = rc.style.alignment.unwrap_or(Alignment::Left);
                // Center the stacked content block vertically within the row.
                // Measure with a throwaway warning sink so we don't double-record
                // warnings already collected during the height pass.
                let block_h = {
                    let saved = std::mem::take(&mut self.warnings);
                    let h = self.cell_content_height(cell, inner_w, resolver);
                    self.warnings = saved;
                    h
                };
                let mut y = row_top + ((row_height - block_h) / 2.0).max(pad_y);
                for item in &rc.content {
                    match item {
                        CellContent::Text(rt) => {
                            let size = rt.font_size.unwrap_or(DEFAULT_CELL_FONT_SIZE);
                            let weight = rt.font_weight.unwrap_or(FontWeight::Normal);
                            let text = interpolate(&rt.value, resolver, &mut self.warnings);
                            let lines = wrap(self.fonts, &text, weight, size, inner_w);
                            for line in lines {
                                self.draw_text_line(
                                    &line, inner_left, inner_w, y, size, weight, align, text_color,
                                    None, None,
                                );
                                y += line_height(size);
                            }
                        }
                        CellContent::Image(img) => {
                            if let Some(idx) = self.intern_image(&img.value) {
                                let src = &self.images[idx];
                                let w = if img.width > 0.0 { img.width } else { inner_w };
                                let h = w * src.height_px as f32 / src.width_px.max(1) as f32;
                                self.current.push(DrawOp::Image {
                                    index: idx,
                                    x: inner_left,
                                    y,
                                    w,
                                    h,
                                });
                                y += h;
                            }
                        }
                    }
                }
            }
        }
    }

    fn draw_cell_borders(
        &mut self,
        style: &CellStyle,
        header_style: &TableHeaderStyle,
        is_header: bool,
        col: Column,
        top: f32,
        height: f32,
    ) {
        let sides = match style.border_sides {
            Some(s) => s,
            None => return, // no borderSides key -> no borders
        };
        if !sides.any() {
            return;
        }
        let color = style
            .border_color
            .as_deref()
            .and_then(color::parse_hex)
            .or_else(|| {
                if is_header {
                    header_style
                        .border_color
                        .as_deref()
                        .and_then(color::parse_hex)
                } else {
                    None
                }
            })
            .unwrap_or(Rgb::LIGHT_GREY);
        let (l, r, tp, b) = (col.x, col.x + col.width, top, top + height);
        let mut line = |x1, y1, x2, y2| {
            self.current.push(DrawOp::Line {
                x1,
                y1,
                x2,
                y2,
                width: 0.5,
                color,
                dash: None,
            });
        };
        if sides.top {
            line(l, tp, r, tp);
        }
        if sides.bottom {
            line(l, b, r, b);
        }
        if sides.left {
            line(l, tp, l, b);
        }
        if sides.right {
            line(r, tp, r, b);
        }
    }
}

/// Footer band height = stacked footer paragraph line-heights + padding.
/// Bezier circle/quadrant constant: control-point offset = radius × KAPPA.
const KAPPA: f32 = 0.552_285;

/// Round/clamp a template dash pattern (pt floats) to printpdf's integer
/// lengths, dropping it if empty or all-zero.
fn dash_px(dash: &Option<Vec<f32>>) -> Option<Vec<i64>> {
    let d = dash.as_ref()?;
    let v: Vec<i64> = d.iter().map(|x| x.round().max(0.0) as i64).collect();
    if v.is_empty() || v.iter().all(|&n| n == 0) {
        None
    } else {
        Some(v)
    }
}

/// Build a rounded-rectangle path (top-left `(x, y)`, size `w`×`h`, corner
/// radius `r`) as polygon points with cubic-bezier corners. Logical coords.
fn rounded_rect_poly(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<PolyPoint> {
    let k = r * KAPPA;
    let (r2, b2) = (x + w, y + h);
    let p = |x: f32, y: f32, bezier: bool| PolyPoint { x, y, bezier };
    vec![
        p(x + r, y, false), // top edge start
        p(r2 - r, y, false),
        p(r2 - r + k, y, true), // top-right corner
        p(r2, y + r - k, true),
        p(r2, y + r, false),
        p(r2, b2 - r, false),    // right edge
        p(r2, b2 - r + k, true), // bottom-right corner
        p(r2 - r + k, b2, true),
        p(r2 - r, b2, false),
        p(x + r, b2, false),    // bottom edge
        p(x + r - k, b2, true), // bottom-left corner
        p(x, b2 - r + k, true),
        p(x, b2 - r, false),
        p(x, y + r, false),    // left edge
        p(x, y + r - k, true), // top-left corner
        p(x + r - k, y, true),
        p(x + r, y, false),
    ]
}

/// Build an ellipse path centered at `(cx, cy)` with radii `(rx, ry)` as four
/// cubic-bezier quadrants. Logical coords.
fn ellipse_poly(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<PolyPoint> {
    let (ox, oy) = (rx * KAPPA, ry * KAPPA);
    let p = |x: f32, y: f32, bezier: bool| PolyPoint { x, y, bezier };
    vec![
        p(cx + rx, cy, false),     // rightmost
        p(cx + rx, cy + oy, true), // → bottom
        p(cx + ox, cy + ry, true),
        p(cx, cy + ry, false),
        p(cx - ox, cy + ry, true), // → left
        p(cx - rx, cy + oy, true),
        p(cx - rx, cy, false),
        p(cx - rx, cy - oy, true), // → top
        p(cx - ox, cy - ry, true),
        p(cx, cy - ry, false),
        p(cx + ox, cy - ry, true), // → right
        p(cx + rx, cy - oy, true),
        p(cx + rx, cy, false),
    ]
}

fn footer_band_height(template: &Template, _fonts: &FontRegistry) -> f32 {
    let mut h = 0.0;
    for el in &template.footer {
        if let Element::Paragraph(p) = el {
            h += line_height(p.options.font_size);
        }
    }
    if h > 0.0 {
        h + FOOTER_PADDING
    } else {
        0.0
    }
}

/// Resolve column widths from the header row or the first body row, scaling to
/// fit `available` and distributing leftover space to width-less columns.
fn compute_columns(t: &Table, available: f32, left: f32) -> Vec<Column> {
    let source: &[Cell] = if !t.header_columns.is_empty() {
        &t.header_columns
    } else {
        t.rows.first().map(|r| r.as_slice()).unwrap_or(&[])
    };
    if source.is_empty() {
        return Vec::new();
    }
    let specified: Vec<Option<f32>> = source.iter().map(|c| c.style().width).collect();
    let sum_known: f32 = specified.iter().flatten().sum();
    let unknown = specified.iter().filter(|w| w.is_none()).count();

    let mut widths: Vec<f32> = if unknown > 0 {
        let each = ((available - sum_known) / unknown as f32).max(1.0);
        specified.iter().map(|w| w.unwrap_or(each)).collect()
    } else if sum_known > available {
        let scale = available / sum_known;
        specified.iter().map(|w| w.unwrap_or(0.0) * scale).collect()
    } else {
        specified.iter().map(|w| w.unwrap_or(0.0)).collect()
    };
    // Guard against zero widths.
    for w in &mut widths {
        if *w <= 0.0 {
            *w = 1.0;
        }
    }
    let mut x = left;
    widths
        .into_iter()
        .map(|width| {
            let col = Column { x, width };
            x += width;
            col
        })
        .collect()
}
