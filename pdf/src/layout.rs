//! Pass 1: turn a parsed template + data into a [`LaidOutDoc`] of positioned
//! draw operations, handling interpolation, the free cursor + `move`, images,
//! tables (data binding, multi-content cells, borders, header-row repeat on
//! page breaks), pagination, and header/footer bands.

use crate::color::{self, Rgb};
use crate::data::{DataDocument, Resolver};
use crate::draw::{
    DrawOp, ImageData, LaidOutDoc, PageTextDraw, PolyPoint, RenderedPage, StructRole, StyledPiece,
    TextDraw, TextPiece,
};
use crate::error::{RenderWarning, Result};
use crate::fonts::{FaceKey, FontRegistry};
use crate::geometry::{
    named_page_size, Cursor, Margins, Page, A4_HEIGHT_PT, A4_WIDTH_PT, DEFAULT_MARGIN_PT,
};
use crate::images;
use crate::interpolate::{has_page_tokens, interpolate};
use crate::template::{
    Alignment, BarcodeEl, Cell, CellContent, CellStyle, ChartEl, ColumnsEl, Conditional, Element,
    EllipseEl, FontWeight, HrEl, LineEl, ListEl, ListItem, PageFilter, PageSpec, Paragraph, QrEl,
    RectEl, RepeatEl, StyledSpan, Table, TableHeaderStyle, Template, VAlign, Watermark,
};
use crate::text::{align_x, layout_styled_lines, line_height, wrap, StyledSeg, LINE_HEIGHT_FACTOR};
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
    images: Vec<std::sync::Arc<ImageData>>,
    image_cache: std::collections::HashMap<String, Option<usize>>,
    warnings: Vec<RenderWarning>,
    bookmarks: Vec<(String, usize, u32)>,
    anchors: std::collections::HashMap<String, (usize, f32)>,
    /// Active multi-column flow (v1: no nesting, so a single Option). While
    /// set, the layout region is the CURRENT COLUMN instead of the page body,
    /// and overflow hops to the next column before starting a new page.
    columns: Option<ColumnFlow>,
    /// Emit a tagged (accessible) PDF. When set, content ops are wrapped in
    /// [`DrawOp::Tagged`] with a structure role.
    tagged: bool,
    /// The structure role for body text currently being drawn (baseline `P`,
    /// raised to `H1..6` inside a heading paragraph). `None` means "not content"
    /// so it emits unwrapped (→ artifact).
    content_role: Option<StructRole>,
    /// While true (header/footer band), content ops emit unwrapped so
    /// headers/footers become artifacts rather than tagged content, and
    /// pagination is suppressed (a band must never trigger a page break).
    artifact_mode: bool,
    /// The document-level data, for data-bound elements (`table`, `repeat`,
    /// `chart`) in the header band. Set once by [`Engine::layout`].
    data: Option<&'a DataDocument>,
}

/// Multi-column flow state. All coordinates are logical (top-left origin).
struct ColumnFlow {
    /// Left edge of the whole set (page content_left at start).
    left: f32,
    /// Total width of the set (page content_width at start).
    width: f32,
    count: usize,
    gap: f32,
    /// Current column, 0-based.
    index: usize,
    /// Y where the set starts on the *current* page (reset on page restart).
    top: f32,
    /// Deepest y any column reached on the current page (for resume-below).
    max_y: f32,
}

impl ColumnFlow {
    fn col_width(&self) -> f32 {
        ((self.width - self.gap * (self.count as f32 - 1.0)) / self.count as f32).max(1.0)
    }
    fn col_left(&self, i: usize) -> f32 {
        self.left + i as f32 * (self.col_width() + self.gap)
    }
    fn current_left(&self) -> f32 {
        self.col_left(self.index)
    }
    fn current_right(&self) -> f32 {
        self.col_left(self.index) + self.col_width()
    }
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
            columns: None,
            tagged: template.options.tagged,
            content_role: None,
            artifact_mode: false,
            data: None,
        }
    }

    // --- tagged-content helpers ---

    /// Push a content op, wrapping it in [`DrawOp::Tagged`] with the current
    /// `content_role` when tagging is on and we're not in an artifact band.
    fn push_text(&mut self, op: DrawOp) {
        match (
            self.tagged && !self.artifact_mode,
            self.content_role.clone(),
        ) {
            (true, Some(role)) => self.current.push(DrawOp::Tagged {
                role,
                inner: Box::new(op),
            }),
            _ => self.current.push(op),
        }
    }

    /// Push a content op with an explicit structure role (used for figures,
    /// which aren't governed by the paragraph `content_role`).
    fn push_tagged(&mut self, op: DrawOp, role: StructRole) {
        if self.tagged && !self.artifact_mode {
            self.current.push(DrawOp::Tagged {
                role,
                inner: Box::new(op),
            });
        } else {
            self.current.push(op);
        }
    }

    // --- layout region (page body, or the current column) ---

    /// Left edge of the active layout region.
    fn region_left(&self) -> f32 {
        self.columns
            .as_ref()
            .map_or(self.page.content_left(), |c| c.current_left())
    }

    /// Right edge of the active layout region.
    fn region_right(&self) -> f32 {
        self.columns
            .as_ref()
            .map_or(self.page.content_right(), |c| c.current_right())
    }

    /// Bottom of the active layout region (columns share the page bottom).
    fn region_bottom(&self) -> f32 {
        self.page.content_bottom()
    }

    /// Overflow action: hop to the next column, else start a new page — and
    /// restart an active column set at the top of the fresh page. Also the
    /// implementation of `page_break` (a column break inside a columns block).
    fn advance_region(&mut self, template: &Template, root: &Resolver) {
        // Header/footer bands never paginate: their content just overflows the
        // band (softly), instead of recursing begin_page → band → begin_page.
        if self.artifact_mode {
            return;
        }
        if let Some(flow) = &mut self.columns {
            flow.max_y = flow.max_y.max(self.cursor.y);
            if flow.index + 1 < flow.count {
                flow.index += 1;
                self.cursor = Cursor::new(flow.current_left(), flow.top);
                return;
            }
        }
        self.finish_page(template, root);
        self.begin_page(template, root);
        if let Some(flow) = &mut self.columns {
            flow.index = 0;
            flow.top = self.cursor.y;
            flow.max_y = flow.top;
            self.cursor.x = flow.current_left();
        }
    }

    /// Run pass 1 over the template, returning the laid-out document and the
    /// accumulated warnings.
    pub fn layout(
        mut self,
        template: &Template,
        data: &'a DataDocument,
    ) -> Result<(LaidOutDoc, Vec<RenderWarning>)> {
        self.data = Some(data);
        let root = data.resolver();
        // Intern a page background image up front so its decoded pixels are in
        // `self.images` before layout; the op itself is inserted per page in a
        // post-pass (the `pages` filter needs the final page count).
        let bg_image = template.options.background_image.as_ref().and_then(|bg| {
            self.intern_image(&bg.src)
                .map(|idx| (idx, bg.pages.clone()))
        });

        self.begin_page(template, &root);
        // Body text is tagged as `P` by default; headings raise this per block.
        self.content_role = Some(StructRole::Paragraph);
        for el in &template.content {
            self.element(el, data, &root, template)?;
        }
        self.finish_page(template, &root);

        // Page background image: behind everything but the background fill.
        if let Some((idx, pages)) = bg_image {
            self.apply_background_image(idx, &pages, template.options.background.is_some());
        }

        // Apply the document watermark now that every page exists, so `pages`
        // (incl. "last") and `front` (on-top vs behind) can be honored.
        if let Some(wm) = &template.options.watermark {
            self.apply_document_watermark(wm, template.options.background.is_some());
        }

        let doc = LaidOutDoc {
            page: self.page,
            pages: self.pages,
            images: self.images,
            bookmarks: self.bookmarks,
            anchors: self.anchors,
            tagged: template.options.tagged,
            lang: template.options.lang.clone(),
        };
        Ok((doc, self.warnings))
    }

    // --- page management ---

    fn begin_page(&mut self, template: &Template, root: &Resolver) {
        self.current = Vec::new();
        self.cursor = Cursor::new(self.page.content_left(), self.page.content_top());
        // Page background fill (the very first op, behind everything). The
        // document watermark's behind-content pass inserts itself *after* this
        // (see `apply_document_watermark`) so the stamp stays visible over it.
        if let Some(bg) = template
            .options
            .background
            .as_deref()
            .and_then(color::parse_hex)
        {
            self.current.push(DrawOp::FillRect {
                x: 0.0,
                y: 0.0,
                w: self.page.width,
                h: self.page.height,
                color: bg,
            });
        }
        // The document watermark is applied in a post-layout pass
        // (`apply_document_watermark`) so it can honor `front`/`pages` once the
        // total page count is known.
        // Draw the header band (its own local cursor at the top). Suspend any
        // active column flow so the header isn't squeezed into a column and
        // its overflow can't hop columns.
        if !template.header.is_empty() {
            let saved = self.cursor;
            let saved_flow = self.columns.take();
            // The header band is a running artifact (pagination furniture), not
            // tagged content — assistive tech should skip it.
            let saved_artifact = self.artifact_mode;
            self.artifact_mode = true;
            self.cursor = Cursor::new(self.page.content_left(), self.page.header_top());
            // The real document data, so data-bound elements (table, repeat,
            // chart) work in the header. Pagination is suppressed while
            // `artifact_mode` is on, so header overflow can't recurse.
            let empty = DataDocument::empty();
            let data = self.data.unwrap_or(&empty);
            for el in &template.header {
                // Header elements should not paginate; ignore errors softly.
                let _ = self.element(el, data, root, template);
            }
            self.artifact_mode = saved_artifact;
            self.cursor = saved;
            self.columns = saved_flow;
        }
    }

    fn finish_page(&mut self, template: &Template, root: &Resolver) {
        // Footer band, stacked from footer_top with its own (fx, fy) cursor.
        // Paragraphs, hrs and images advance fy; rect/line/ellipse draw at the
        // cursor without advancing (position them with `move`), as in the body.
        let saved_cursor = self.cursor;
        let saved_artifact = self.artifact_mode;
        self.artifact_mode = true;
        let left = self.page.content_left();
        let right = left + self.page.content_width();
        let mut fx = left;
        let mut fy = self.page.footer_top() + FOOTER_PADDING / 2.0;
        for el in &template.footer {
            match el {
                Element::Paragraph(p) => {
                    fy = self.footer_paragraph(p, root, fy);
                    fx = left;
                }
                Element::Move(m) => {
                    fx += m.x;
                    fy += m.y;
                }
                // Not hr(): that one paginates via ensure_space and spans the
                // *column* region — the footer band spans the page width.
                Element::Hr(h) => {
                    let thickness = h.thickness.max(0.0);
                    let x2 = match h.width {
                        Some(w) if w > 0.0 => fx + w,
                        _ => right,
                    };
                    let y = fy + thickness / 2.0;
                    self.current.push(DrawOp::Line {
                        x1: fx,
                        y1: y,
                        x2,
                        y2: y,
                        width: thickness,
                        color: color::parse_hex_or(h.color.as_deref(), Rgb::LIGHT_GREY),
                        dash: dash_px(&h.dash),
                    });
                    fy += thickness + 2.0;
                }
                Element::Rect(r) => {
                    self.cursor = Cursor::new(fx, fy);
                    self.rect(r);
                }
                Element::Line(l) => {
                    self.cursor = Cursor::new(fx, fy);
                    self.line(l);
                }
                Element::Ellipse(e) => {
                    self.cursor = Cursor::new(fx, fy);
                    self.ellipse(e);
                }
                Element::Image(img) => {
                    if let Some(idx) = self.intern_image(&img.value) {
                        let src = &self.images[idx];
                        let (w, h) = fit_image(
                            src.width_px,
                            src.height_px,
                            img.width,
                            img.height,
                            src.width_px as f32,
                        );
                        self.push_tagged(
                            DrawOp::Image {
                                index: idx,
                                x: fx,
                                y: fy,
                                w,
                                h,
                            },
                            StructRole::Figure {
                                alt: img.alt.clone(),
                            },
                        );
                        fy += h;
                    }
                }
                other => self.warnings.push(RenderWarning::ElementSkipped {
                    kind: element_kind(other).to_string(),
                    reason: "not supported in the footer band".to_string(),
                }),
            }
        }
        self.cursor = saved_cursor;
        self.artifact_mode = saved_artifact;
        let page = std::mem::take(&mut self.current);
        self.pages.push(RenderedPage { ops: page });
    }

    /// Ensure `needed` pt of vertical space remains in the active region;
    /// otherwise advance (next column, or a new page).
    fn ensure_space(&mut self, needed: f32, template: &Template, root: &Resolver) {
        if self.cursor.y + needed > self.region_bottom() {
            self.advance_region(template, root);
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
                if let Some(idx) = self.intern_image(&img.value) {
                    let src = &self.images[idx];
                    let (mut w, mut h) = fit_image(
                        src.width_px,
                        src.height_px,
                        img.width,
                        img.height,
                        src.width_px as f32,
                    );
                    // A body image is a `Figure` in tagged output; its `alt`
                    // becomes `/Alt` (missing alt is a UA gap — warn).
                    let fig = StructRole::Figure {
                        alt: img.alt.clone(),
                    };
                    if self.tagged && !self.artifact_mode && img.alt.is_none() {
                        self.warnings.push(RenderWarning::MissingAlt {
                            src: img.value.clone(),
                        });
                    }
                    if self.columns.is_some() {
                        // Inside columns an image is a FLOW element: cap it to the
                        // remaining column width and advance the cursor below it.
                        let avail = (self.region_right() - self.cursor.x).max(1.0);
                        if w > avail {
                            h *= avail / w;
                            w = avail;
                        }
                        self.ensure_space(h, template, root);
                        self.push_tagged(
                            DrawOp::Image {
                                index: idx,
                                x: self.cursor.x,
                                y: self.cursor.y,
                                w,
                                h,
                            },
                            fig,
                        );
                        self.cursor.y += h;
                    } else {
                        self.push_tagged(
                            DrawOp::Image {
                                index: idx,
                                x: self.cursor.x,
                                y: self.cursor.y,
                                w,
                                h,
                            },
                            fig,
                        );
                        // Image placement is explicit; the cursor is not advanced.
                    }
                }
            }
            Element::Table(t) => {
                if self.columns.is_some() {
                    self.skip_in_columns("table");
                } else {
                    self.table(t, data, root, template)?;
                }
            }
            Element::Hr(h) => self.hr(h, template, root),
            Element::Rect(r) => self.rect(r),
            Element::Line(l) => self.line(l),
            Element::Qr(q) => self.qr(q, root),
            Element::Barcode(b) => self.barcode(b, root),
            Element::Ellipse(e) => self.ellipse(e),
            Element::List(l) => self.list(l, root, template),
            Element::Chart(c) => {
                if self.columns.is_some() {
                    self.skip_in_columns("chart");
                } else {
                    self.chart(c, data, root, template);
                }
            }
            Element::Repeat(r) => self.repeat(r, data, root, template)?,
            Element::If(c) => self.conditional(c, false, data, root, template)?,
            Element::Unless(c) => self.conditional(c, true, data, root, template)?,
            Element::PageBreak => self.advance_region(template, root),
            Element::Columns(c) => self.columns_el(c, data, root, template)?,
        }
        Ok(())
    }

    fn skip_in_columns(&mut self, kind: &str) {
        self.warnings.push(RenderWarning::ElementSkipped {
            kind: kind.to_string(),
            reason: "not supported inside columns (v1)".to_string(),
        });
    }

    /// Lay out a `columns` block: install a column flow, run `content` (which
    /// fills column 1 to the bottom, then column 2, …), then resume full-width
    /// flow below the deepest column.
    fn columns_el(
        &mut self,
        c: &ColumnsEl,
        data: &DataDocument,
        root: &Resolver,
        template: &Template,
    ) -> Result<()> {
        if c.content.is_empty() {
            return Ok(());
        }
        let count = c.count.clamp(1, 6) as usize;
        if count as u32 != c.count {
            self.warnings.push(RenderWarning::ElementSkipped {
                kind: "columns".to_string(),
                reason: format!("count {} clamped to {count}", c.count),
            });
        }
        // Nested columns or a single column: flatten into the enclosing flow
        // (never drop content).
        if self.columns.is_some() || count == 1 {
            if self.columns.is_some() {
                self.warnings.push(RenderWarning::ElementSkipped {
                    kind: "columns".to_string(),
                    reason: "nested columns are not supported; content flattened".to_string(),
                });
            }
            for el in &c.content {
                self.element(el, data, root, template)?;
            }
            return Ok(());
        }
        // Don't start a set in a sliver at the page bottom.
        self.ensure_space(line_height(12.0), template, root);
        let top = self.cursor.y;
        self.columns = Some(ColumnFlow {
            left: self.page.content_left(),
            width: self.page.content_width(),
            count,
            gap: c.gap.max(0.0),
            index: 0,
            top,
            max_y: top,
        });
        self.cursor = Cursor::new(self.region_left(), top);
        for el in &c.content {
            self.element(el, data, root, template)?;
        }
        let flow = self.columns.take().expect("column flow active");
        let resume_y = flow.max_y.max(self.cursor.y);
        self.cursor = Cursor::new(self.page.content_left(), resume_y);
        Ok(())
    }

    /// Lay out a `repeat` block's `content` once per item of its `data` array,
    /// scoping `${field}` to each item (falling back to the root) — the
    /// block-level analogue of data-bound table rows. A missing/empty array
    /// renders nothing.
    fn repeat(
        &mut self,
        r: &RepeatEl,
        data: &DataDocument,
        root: &Resolver,
        template: &Template,
    ) -> Result<()> {
        let Some(items) = data.array(&r.data) else {
            return Ok(());
        };
        for item in items {
            let scoped = root.with_scope(item);
            for el in &r.content {
                self.element(el, data, &scoped, template)?;
            }
        }
        Ok(())
    }

    /// Render an `if`/`unless` block's matching branch. The test reads `${when}`:
    /// with `equals` it's string equality, otherwise truthiness (falsy = missing,
    /// empty, `false`, `0`, or `null`). `negate` (the `unless` element) flips it.
    fn conditional(
        &mut self,
        c: &Conditional,
        negate: bool,
        data: &DataDocument,
        root: &Resolver,
        template: &Template,
    ) -> Result<()> {
        let value = root.lookup(&c.when);
        let pass = match &c.equals {
            Some(eq) => value.as_deref() == Some(eq.as_str()),
            None => value.as_deref().map(is_truthy).unwrap_or(false),
        };
        let branch = if pass ^ negate {
            &c.content
        } else {
            &c.else_content
        };
        for el in branch {
            self.element(el, data, root, template)?;
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
            _ => self.region_right(),
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
        self.push_tagged(
            DrawOp::Image {
                index: idx,
                x: self.cursor.x,
                y: self.cursor.y,
                w: side,
                h: side,
            },
            StructRole::Figure {
                alt: Some("QR code".to_string()),
            },
        );
    }

    /// A 1D barcode rasterised through the image path and placed at the cursor,
    /// sized `width × height` pt (the bars are scaled to fit, which is expected
    /// for barcodes). Degrades to a warning on any error. Does not advance the
    /// cursor — position it with `move`.
    fn barcode(&mut self, b: &BarcodeEl, root: &Resolver) {
        let value = match &b.value {
            Some(s) => interpolate(s, root, &mut self.warnings),
            None => String::new(),
        };
        if value.is_empty() {
            self.warnings.push(RenderWarning::ElementSkipped {
                kind: "barcode".to_string(),
                reason: "empty value".to_string(),
            });
            return;
        }

        let img = match crate::barcode::encode_barcode(&b.symbology, &value) {
            Ok(img) => img,
            Err(e) => {
                self.warnings.push(RenderWarning::ElementSkipped {
                    kind: "barcode".to_string(),
                    reason: e.to_string(),
                });
                return;
            }
        };
        let idx = self.intern_generated_image(img);
        let w = if b.width > 0.0 { b.width } else { 160.0 };
        let h = if b.height > 0.0 { b.height } else { 50.0 };
        self.push_tagged(
            DrawOp::Image {
                index: idx,
                x: self.cursor.x,
                y: self.cursor.y,
                w,
                h,
            },
            StructRole::Figure {
                alt: Some(format!("Barcode: {value}")),
            },
        );

        if b.human_readable {
            // A small caption centered under the bars, in the document font.
            let size = 9.0_f32;
            let runs = self
                .fonts
                .itemize(&value, FontWeight::Normal, &mut self.warnings);
            let text_w: f32 = runs.iter().map(|r| self.fonts.measure_run(r, size)).sum();
            let ascent = self.fonts.ascent(FontWeight::Normal) * size;
            let pieces: Vec<TextPiece> = runs
                .into_iter()
                .map(|r| TextPiece {
                    slot: r.slot,
                    text: r.text,
                })
                .collect();
            self.push_text(DrawOp::Text(TextDraw {
                x: self.cursor.x + (w - text_w).max(0.0) / 2.0,
                baseline: self.cursor.y + h + 2.0 + ascent,
                size,
                color: Rgb::BLACK,
                pieces,
            }));
        }
    }

    /// A bulleted/numbered list. Flows like stacked paragraphs and advances the
    /// cursor below the last item.
    fn list(&mut self, list: &ListEl, root: &Resolver, template: &Template) {
        let left_base = self.cursor.x;
        // Region-relative indent, so restoring the left edge after a column hop
        // lands in the correct column rather than the original one.
        let left_off = left_base - self.region_left();
        self.render_list(list, root, template, left_base);
        // Each item body advanced cursor.y; restore the left edge for siblings.
        self.cursor.x = self.region_left() + left_off;
        // Block spacing below the whole list (items use `ListEl::spacing`).
        self.cursor.y += list.options.spacing.unwrap_or(0.0);
    }

    /// Render one list level, indented from `left_base`. Each item draws a marker
    /// in a left gutter and its body in the column to the right (reusing the
    /// paragraph machinery via a temporary cursor shift); a nested sublist
    /// recurses one level deeper.
    fn render_list(&mut self, list: &ListEl, root: &Resolver, template: &Template, left_base: f32) {
        if list.items.is_empty() {
            return;
        }
        let size = list.options.font_size;
        let weight = list.options.font_weight;
        let lh = size * list.options.line_height.unwrap_or(LINE_HEIGHT_FACTOR);
        let indent = if list.indent > 0.0 { list.indent } else { 18.0 };
        let gutter = self.list_marker_gutter(list, size, weight);
        // Marker/body x are region-relative so a column hop mid-list keeps the
        // markers with their column (`base_off` = how far this level is indented
        // from the region left).
        let base_off = left_base - self.region_left();
        // Markers follow the list's text color (default black), matching body.
        let color = color::parse_hex_or(list.options.color.as_deref(), Rgb::BLACK);
        let mut counter = list.start;

        for item in &list.items {
            let (text, spans, nested) = match item {
                ListItem::Text(t) => (Some(t.clone()), None, None),
                ListItem::Node(n) => (n.text.clone(), n.spans.clone(), n.list.as_deref()),
            };

            if text.is_some() || spans.is_some() {
                // Reserve a line so the marker and the first body line never split
                // across a page break.
                self.ensure_space(lh, template, root);
                let marker_x = self.region_left() + base_off + indent;
                let text_x = marker_x + gutter;
                let marker_top = self.cursor.y;
                let marker = if list.ordered {
                    format!("{counter}.")
                } else {
                    list.marker.clone().unwrap_or_else(|| "•".to_string())
                };
                self.draw_text_line(
                    &marker,
                    marker_x,
                    gutter,
                    marker_top,
                    size,
                    weight,
                    Alignment::Left,
                    color,
                    None,
                    None,
                    list.options.italic,
                    list.options.mono,
                    false,
                    false,
                    false,
                );

                // Item bodies reuse the list's text options, but block `spacing`
                // belongs to the whole list (applied once in `list()`), not to
                // every item — items are spaced by `ListEl::spacing`.
                let mut item_options = list.options.clone();
                item_options.spacing = None;
                let para = Paragraph {
                    value: text.unwrap_or_default(),
                    spans,
                    options: item_options,
                };
                let saved_x = self.cursor.x;
                self.cursor.x = text_x;
                self.paragraph(&para, root, template);
                self.cursor.x = saved_x;
            }

            if list.ordered {
                counter += 1;
            }
            if let Some(nested) = nested {
                // Nested list indents from this level's body column, region-
                // relative so it survives a column hop just like the markers.
                let nested_left = self.region_left() + base_off + indent + gutter;
                self.render_list(nested, root, template, nested_left);
            }
            if list.spacing > 0.0 {
                self.cursor.y += list.spacing;
            }
        }
    }

    /// A bar/line/pie chart drawn from data-bound or inline points. Flows like a
    /// block: a title (optional) above the plot, then the chart, then a small gap.
    fn chart(&mut self, c: &ChartEl, data: &DataDocument, root: &Resolver, template: &Template) {
        let (categories, series) = self.chart_series(c, data, root);
        if categories.is_empty() || series.iter().all(|s| s.values.is_empty()) {
            self.warnings.push(RenderWarning::ElementSkipped {
                kind: "chart".to_string(),
                reason: "no data points".to_string(),
            });
            return;
        }

        let width = if c.width > 0.0 {
            c.width
        } else {
            (self.region_right() - self.cursor.x).max(1.0)
        };
        let plot_h = c.height.max(1.0);
        let has_title = c.title.as_deref().is_some_and(|s| !s.trim().is_empty());
        let title_size = 12.0_f32;
        // Breathing room between the title's baseline band and the plot (the
        // legend row starts right at the chart area's top, so without this the
        // title's descender nearly touches the swatches).
        const CHART_TITLE_GAP: f32 = 6.0;
        let title_h = if has_title {
            line_height(title_size) + CHART_TITLE_GAP
        } else {
            0.0
        };
        let bottom_pad = 6.0;

        self.ensure_space(title_h + plot_h + bottom_pad, template, root);

        let x0 = self.cursor.x;
        let mut y = self.cursor.y;
        if has_title {
            let title = interpolate(c.title.as_deref().unwrap_or(""), root, &mut self.warnings);
            self.draw_text_line(
                &title,
                x0,
                width,
                y,
                title_size,
                FontWeight::Bold,
                Alignment::Left,
                Rgb::BLACK,
                None,
                None,
                false,
                false,
                false,
                false,
                false,
            );
            y += title_h;
        }

        let area = crate::chart::Area {
            x: x0,
            y,
            w: width,
            h: plot_h,
        };
        let ops = crate::chart::render_chart(
            c,
            &categories,
            &series,
            area,
            self.fonts,
            &mut self.warnings,
        );
        self.current.extend(ops);
        self.cursor.y = y + plot_h + bottom_pad;
    }

    /// Resolve a chart's categories + series. Multi-series when `values` is set
    /// (one bound array, several value fields); otherwise a single series from
    /// `data` + `value`, or from the inline `points`.
    fn chart_series(
        &mut self,
        c: &ChartEl,
        data: &DataDocument,
        root: &Resolver,
    ) -> (Vec<String>, Vec<crate::chart::Series>) {
        let label_field = c.label.as_deref().unwrap_or("label");
        let value_field = c.value.as_deref().unwrap_or("value");

        // Multi-series: one bound array, a value field per series.
        if let Some(defs) = &c.values {
            let Some(items) = c.data.as_deref().and_then(|k| data.array(k)) else {
                return (Vec::new(), Vec::new());
            };
            let categories = items
                .iter()
                .map(|item| {
                    root.with_scope(item)
                        .lookup(label_field)
                        .unwrap_or_default()
                })
                .collect();
            let series = defs
                .iter()
                .map(|d| crate::chart::Series {
                    name: d.name.clone(),
                    color: d.color.clone(),
                    values: items
                        .iter()
                        .map(|item| {
                            root.with_scope(item)
                                .lookup(&d.field)
                                .as_deref()
                                .map(parse_num)
                                .unwrap_or(0.0)
                        })
                        .collect(),
                })
                .collect();
            return (categories, series);
        }

        // Single series: bound array, or inline points.
        if let Some(key) = &c.data {
            let Some(items) = data.array(key) else {
                return (Vec::new(), Vec::new());
            };
            let mut categories = Vec::with_capacity(items.len());
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                let scoped = root.with_scope(item);
                categories.push(scoped.lookup(label_field).unwrap_or_default());
                values.push(
                    scoped
                        .lookup(value_field)
                        .as_deref()
                        .map(parse_num)
                        .unwrap_or(0.0),
                );
            }
            return (categories, vec![single_series(values)]);
        }

        let mut categories = Vec::new();
        let mut values = Vec::new();
        for p in c.points.iter().flatten() {
            categories.push(p.label.clone().unwrap_or_default());
            values.push(p.value);
        }
        (categories, vec![single_series(values)])
    }

    /// Width reserved for a list's marker column: the widest marker it will draw
    /// (the last ordinal, or the bullet glyph) plus a half-em gap.
    fn list_marker_gutter(&mut self, list: &ListEl, size: f32, weight: FontWeight) -> f32 {
        let sample = if list.ordered {
            let last = list.start + (list.items.len() as i64).max(1) - 1;
            format!("{last}.")
        } else {
            list.marker.clone().unwrap_or_else(|| "•".to_string())
        };
        let runs = self.fonts.itemize(&sample, weight, &mut self.warnings);
        let w: f32 = runs.iter().map(|r| self.fonts.measure_run(r, size)).sum();
        w + size * 0.5
    }

    /// Push a programmatically generated image (e.g. a QR raster) and return its
    /// index, mirroring [`Self::intern_image`] for in-memory bitmaps.
    fn intern_generated_image(&mut self, img: ImageData) -> usize {
        self.images.push(std::sync::Arc::new(img));
        self.images.len() - 1
    }

    /// Build a rotated-text op for `wm`, centered on the **logical** point
    /// `center` (top-left origin, y down). The text-matrix origin is computed so
    /// the text's cap-height band is centered on that point, matching printpdf's
    /// `TranslateRotate` matrix `[cosθ,-sinθ,sinθ,cosθ,x,y]` with `θ = (360 - angle)°`.
    /// Returns `None` for empty/unmeasurable text.
    fn watermark_op(&mut self, wm: &Watermark, center: (f32, f32)) -> Option<DrawOp> {
        if wm.text.trim().is_empty() {
            return None;
        }
        let size = wm.font_size;
        let weight = wm.font_weight;
        let runs = self.fonts.itemize(&wm.text, weight, &mut self.warnings);
        let width: f32 = runs.iter().map(|r| self.fonts.measure_run(r, size)).sum();
        if width <= 0.0 {
            return None;
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
        // RotatedText origin is PDF space (bottom-left), so flip the logical y.
        let (cx, cy) = (center.0, self.page.height - center.1);
        // Map text-space cap-band center (width/2, cap/2) onto the target point.
        let x = cx - c * (width / 2.0) - s * (cap / 2.0);
        let y = cy + s * (width / 2.0) - c * (cap / 2.0);

        let color = color::parse_hex_or(wm.color.as_deref(), Rgb::LIGHT_GREY);
        Some(DrawOp::RotatedText {
            x,
            y,
            angle: wm.angle,
            size,
            color,
            pieces,
        })
    }

    /// Logical center point for a document watermark: explicit `x`/`y` win,
    /// then the `anchor` vertical hint, else the page center.
    fn watermark_center(&self, wm: &Watermark) -> (f32, f32) {
        let cx = wm.x.unwrap_or(self.page.width / 2.0);
        let cy = wm.y.unwrap_or_else(|| match wm.anchor.as_deref() {
            Some(a) if a.eq_ignore_ascii_case("top") => self.page.height * 0.28,
            Some(a) if a.eq_ignore_ascii_case("bottom") => self.page.height * 0.72,
            _ => self.page.height / 2.0,
        });
        (cx, cy)
    }

    /// Stamp the document watermark onto every page selected by `wm.pages`,
    /// behind the content (default) or on top of it when `wm.front` is set.
    /// Stamp a full-page background image onto every page selected by `pages`,
    /// stretched to the page and sitting just above the background fill (so a
    /// `background` color shows only where the image has transparency) and
    /// below all content.
    fn apply_background_image(&mut self, index: usize, pages: &PageFilter, bg_present: bool) {
        let w = self.page.width;
        let h = self.page.height;
        let insert_at = if bg_present { 1 } else { 0 };
        let total = self.pages.len();
        for (i, page) in self.pages.iter_mut().enumerate() {
            if !pages.matches(i + 1, total) {
                continue;
            }
            page.ops.insert(
                insert_at.min(page.ops.len()),
                DrawOp::Image {
                    index,
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                },
            );
        }
    }

    fn apply_document_watermark(&mut self, wm: &Watermark, bg_present: bool) {
        let center = self.watermark_center(wm);
        let op = match self.watermark_op(wm, center) {
            Some(op) => op,
            None => return,
        };
        // A page-background fill is always the first op (see `begin_page`); a
        // behind-content watermark must sit just above it, not below.
        let behind_at = if bg_present { 1 } else { 0 };
        let total = self.pages.len();
        for (i, page) in self.pages.iter_mut().enumerate() {
            if !wm.pages.matches(i + 1, total) {
                continue;
            }
            if wm.front {
                page.ops.push(op.clone());
            } else {
                page.ops.insert(behind_at.min(page.ops.len()), op.clone());
            }
        }
    }

    fn paragraph(&mut self, p: &Paragraph, root: &Resolver, template: &Template) {
        // A bookmarked paragraph is a section heading: tag it `H<level>` (level
        // from `bookmarkLevel`, default 1). Plain paragraphs keep the `P`
        // baseline. Save/restore so following blocks (tables, lists) don't
        // inherit the heading role.
        let prev_role = self.content_role.clone();
        if self.tagged && !self.artifact_mode {
            let role = if p.options.bookmark.is_some() || p.options.bookmark_level.is_some() {
                let level = p.options.bookmark_level.unwrap_or(1).clamp(1, 6) as u8;
                StructRole::Heading(level)
            } else {
                StructRole::Paragraph
            };
            self.content_role = Some(role);
        }
        if let Some(spans) = &p.spans {
            self.paragraph_styled(p, spans, root, template);
            self.content_role = prev_role;
            return;
        }
        let text = interpolate(&p.value, root, &mut self.warnings);
        let size = p.options.font_size;
        let weight = p.options.font_weight;
        let lh = size * p.options.line_height.unwrap_or(LINE_HEIGHT_FACTOR);
        // Indent-relative left, so wrapping and drawing follow the cursor
        // across a mid-paragraph column hop (all columns share a width).
        let indent = self.cursor.x - self.region_left();
        let region_width = (self.region_right() - self.cursor.x).max(1.0);
        // Wrap with the full face key so italic/mono metrics match the draw.
        let face = FaceKey {
            mono: p.options.mono,
            bold: matches!(weight, FontWeight::Bold),
            italic: p.options.italic,
        };
        let lines = wrap(self.fonts, &text, face, size, region_width);
        // Keep-together: the first line also reserves `minSpaceBelow`, so a
        // heading never strands at the page bottom away from its content.
        let keep = p.options.min_space_below.unwrap_or(0.0);
        let justify = matches!(p.options.alignment, Alignment::Justify);
        let line_count = lines.len();
        let mut first = true;
        for (i, line) in lines.iter().enumerate() {
            self.ensure_space(if first { lh + keep } else { lh }, template, root);
            // Record outline/jump targets at the paragraph's first drawn line, so
            // the page index reflects any page break the line just triggered.
            if first {
                let page_idx = self.pages.len();
                if let Some(label) = &p.options.bookmark {
                    self.bookmarks.push((
                        label.clone(),
                        page_idx,
                        p.options.bookmark_level.unwrap_or(1).max(1),
                    ));
                }
                if let Some(id) = &p.options.anchor {
                    self.anchors.insert(id.clone(), (page_idx, self.cursor.y));
                }
                first = false;
            }
            let region_left = self.region_left() + indent;
            self.draw_text_line(
                line,
                region_left,
                region_width,
                self.cursor.y,
                size,
                weight,
                p.options.alignment,
                color::parse_hex_or(p.options.color.as_deref(), Rgb::BLACK),
                p.options.link.as_deref(),
                p.options.link_to.as_deref(),
                p.options.italic,
                p.options.mono,
                // The paragraph's LAST line stays left-aligned, as usual.
                justify && i + 1 < line_count,
                p.options.underline,
                p.options.strike,
            );
            self.cursor.y += lh;
        }
        // Block spacing: the declarative alternative to a trailing `move`.
        self.cursor.y += p.options.spacing.unwrap_or(0.0);
        self.content_role = prev_role;
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
        let base_italic = p.options.italic;
        let base_mono = p.options.mono;

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
                italic: span.italic.unwrap_or(base_italic),
                mono: span.mono.unwrap_or(base_mono),
                color: span
                    .color
                    .as_deref()
                    .and_then(color::parse_hex)
                    .unwrap_or(Rgb::BLACK),
                link,
                underline: span.underline.unwrap_or(p.options.underline),
                strike: span.strike.unwrap_or(p.options.strike),
            });
        }

        let indent = self.cursor.x - self.region_left();
        let region_width = (self.region_right() - self.cursor.x).max(1.0);
        let lines = layout_styled_lines(self.fonts, &segs, region_width, &mut self.warnings);
        let ascent_ratio = self.fonts.ascent(FontWeight::Normal);

        let lh_factor = p.options.line_height.unwrap_or(LINE_HEIGHT_FACTOR);
        let keep = p.options.min_space_below.unwrap_or(0.0);
        let line_count = lines.len();
        let mut first = true;
        for (line_idx, line) in lines.into_iter().enumerate() {
            let max_size = line.iter().map(|c| c.size).fold(base_size, f32::max);
            let lh = max_size * lh_factor;
            self.ensure_space(if first { lh + keep } else { lh }, template, root);
            if first {
                let page_idx = self.pages.len();
                if let Some(label) = &p.options.bookmark {
                    self.bookmarks.push((
                        label.clone(),
                        page_idx,
                        p.options.bookmark_level.unwrap_or(1).max(1),
                    ));
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
            let region_left = self.region_left() + indent;
            let x0 = align_x(region_left, region_width, width, p.options.alignment);
            let baseline = self.cursor.y + max_size * ascent_ratio;
            // Justified spans: distribute the leftover width across the line's
            // spaces, splitting the drawn text into segments repositioned at
            // the stretched x. The paragraph's LAST line stays left-aligned.
            let n_spaces = line.iter().filter(|c| c.ch == ' ').count();
            let justify_this = matches!(p.options.alignment, Alignment::Justify)
                && line_idx + 1 < line_count
                && n_spaces > 0
                && width < region_width;
            let extra = if justify_this {
                (region_width - width) / n_spaces as f32
            } else {
                0.0
            };

            // Merge consecutive same-style chars into pieces; track x-ranges of
            // each linked span for the link annotations, and of each decorated
            // (underline/strike) stretch for the decoration strokes. A
            // decoration run breaks when the flag toggles or the color changes;
            // its stroke uses the largest char size seen in the run.
            let mut segments: Vec<(f32, Vec<StyledPiece>)> = Vec::new();
            let mut seg_x = x0;
            let mut pieces: Vec<StyledPiece> = Vec::new();
            let mut link_runs: Vec<(f32, f32, usize)> = Vec::new();
            let mut cur_link: Option<(usize, f32)> = None;
            // (start_x, color, max_size) of the open run, per decoration kind.
            let mut deco_runs: [Vec<(f32, f32, Rgb, f32)>; 2] = [Vec::new(), Vec::new()];
            let mut cur_deco: [Option<(f32, Rgb, f32)>; 2] = [None, None];
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
                for (kind, on) in [(0, c.underline), (1, c.strike)] {
                    match (cur_deco[kind], on) {
                        (None, true) => cur_deco[kind] = Some((cx, c.color, c.size)),
                        (Some((start, col, ms)), true) if col == c.color => {
                            cur_deco[kind] = Some((start, col, ms.max(c.size)))
                        }
                        (Some((start, col, ms)), _) => {
                            deco_runs[kind].push((start, cx, col, ms));
                            cur_deco[kind] = on.then_some((cx, c.color, c.size));
                        }
                        (None, false) => {}
                    }
                }
                cx += cw;
                if justify_this && c.ch == ' ' {
                    // Stretch the gap and start a new segment at the new x, so
                    // the following glyphs draw at the justified position.
                    cx += extra;
                    segments.push((seg_x, std::mem::take(&mut pieces)));
                    seg_x = cx;
                }
            }
            if let Some((li, start)) = cur_link {
                link_runs.push((start, cx, li));
            }
            for kind in 0..2 {
                if let Some((start, col, ms)) = cur_deco[kind] {
                    deco_runs[kind].push((start, cx, col, ms));
                }
            }

            segments.push((seg_x, pieces));
            for (sx, seg_pieces) in segments {
                if !seg_pieces.is_empty() {
                    self.push_text(DrawOp::StyledText {
                        x: sx,
                        baseline,
                        pieces: seg_pieces,
                    });
                }
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
            for (kind, runs) in deco_runs.into_iter().enumerate() {
                for (sx, ex, col, ms) in runs {
                    self.decoration_lines(sx, ex, baseline, ms, col, kind == 0, kind == 1);
                }
            }
            self.cursor.y += lh;
        }
        self.cursor.y += p.options.spacing.unwrap_or(0.0);
    }

    /// Draw a single (already-wrapped) line of body text. `link` emits an external
    /// link annotation over the line's box; `link_to` an internal jump to an anchor.
    ///
    /// A line still holding `#PAGE#` / `#TOTAL_PAGE#` / `#PAGE_OF:` tokens is
    /// deferred to pass 2 as a `PageText` op (the totals aren't known yet),
    /// which makes page tokens work in header and body paragraphs, not just
    /// the footer. Deferred lines emit link annotations over the full region
    /// box (their text width is only known in pass 2), skip underline/strike,
    /// and re-itemize by weight only, so italic/mono don't survive on them.
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
        italic: bool,
        mono: bool,
        justify: bool,
        underline: bool,
        strike: bool,
    ) {
        let ascent = self.fonts.ascent(weight) * size;
        if has_page_tokens(line) {
            self.current.push(DrawOp::PageText(PageTextDraw {
                region_left,
                region_width,
                baseline: top + ascent,
                size,
                color,
                alignment,
                weight,
                raw: line.to_string(),
            }));
            // The substituted text width is only known in pass 2, so link
            // annotations cover the whole region box instead — a TOC line can
            // carry both `linkTo` and `#PAGE_OF:` and stay clickable.
            if let Some(uri) = link.filter(|u| !u.is_empty()) {
                self.current.push(DrawOp::Link {
                    x: region_left,
                    y: top,
                    w: region_width,
                    h: line_height(size),
                    uri: uri.to_string(),
                });
            }
            if let Some(anchor) = link_to.filter(|a| !a.is_empty()) {
                self.current.push(DrawOp::InternalLink {
                    x: region_left,
                    y: top,
                    w: region_width,
                    h: line_height(size),
                    anchor: anchor.to_string(),
                });
            }
            return;
        }
        let face = FaceKey {
            mono,
            bold: matches!(weight, FontWeight::Bold),
            italic,
        };
        // Justified line: distribute the leftover width across word gaps and
        // emit one Text op per word. Single-word lines fall through to the
        // normal path (nothing to distribute).
        if justify {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.len() > 1 {
                let mut word_pieces: Vec<(Vec<TextPiece>, f32)> = Vec::with_capacity(words.len());
                let mut words_w = 0.0f32;
                for w in &words {
                    let runs = self.fonts.itemize(w, face, &mut self.warnings);
                    let ww: f32 = runs.iter().map(|r| self.fonts.measure_run(r, size)).sum();
                    let pieces: Vec<TextPiece> = runs
                        .into_iter()
                        .map(|r| TextPiece {
                            slot: r.slot,
                            text: r.text,
                        })
                        .collect();
                    words_w += ww;
                    word_pieces.push((pieces, ww));
                }
                let extra = ((region_width - words_w) / (words.len() - 1) as f32).max(0.0);
                let baseline = top + ascent;
                let mut x = region_left;
                let mut end_x = region_left;
                for (pieces, ww) in word_pieces {
                    if !pieces.is_empty() {
                        self.push_text(DrawOp::Text(TextDraw {
                            x,
                            baseline,
                            size,
                            color,
                            pieces,
                        }));
                    }
                    end_x = x + ww;
                    x += ww + extra;
                }
                if let Some(uri) = link.filter(|u| !u.is_empty()) {
                    self.current.push(DrawOp::Link {
                        x: region_left,
                        y: top,
                        w: (end_x - region_left).max(0.0),
                        h: line_height(size),
                        uri: uri.to_string(),
                    });
                }
                if let Some(anchor) = link_to.filter(|a| !a.is_empty()) {
                    self.current.push(DrawOp::InternalLink {
                        x: region_left,
                        y: top,
                        w: (end_x - region_left).max(0.0),
                        h: line_height(size),
                        anchor: anchor.to_string(),
                    });
                }
                self.decoration_lines(region_left, end_x, baseline, size, color, underline, strike);
                return;
            }
        }
        let runs = self.fonts.itemize(line, face, &mut self.warnings);
        let width: f32 = runs.iter().map(|r| self.fonts.measure_run(r, size)).sum();
        let x = align_x(region_left, region_width, width, alignment);
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
        let baseline = top + ascent;
        self.push_text(DrawOp::Text(TextDraw {
            x,
            baseline,
            size,
            color,
            pieces,
        }));
        self.decoration_lines(x, x + width, baseline, size, color, underline, strike);
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

    /// Emit underline and/or strikethrough strokes for a text run from `x1` to
    /// `x2`. Offsets follow common convention: the underline sits ~0.10 em
    /// below the baseline, the strike ~0.27 em above it (mid x-height); both
    /// are ~0.06 em thick and drawn in the text color.
    #[allow(clippy::too_many_arguments)]
    fn decoration_lines(
        &mut self,
        x1: f32,
        x2: f32,
        baseline: f32,
        size: f32,
        color: Rgb,
        underline: bool,
        strike: bool,
    ) {
        if x2 <= x1 {
            return;
        }
        let thickness = (size * 0.06).max(0.4);
        if underline {
            let y = baseline + size * 0.10;
            self.current.push(DrawOp::Line {
                x1,
                y1: y,
                x2,
                y2: y,
                width: thickness,
                color,
                dash: None,
            });
        }
        if strike {
            let y = baseline - size * 0.27;
            self.current.push(DrawOp::Line {
                x1,
                y1: y,
                x2,
                y2: y,
                width: thickness,
                color,
                dash: None,
            });
        }
    }

    fn footer_paragraph(&mut self, p: &Paragraph, root: &Resolver, top: f32) -> f32 {
        let interpolated = interpolate(&p.value, root, &mut self.warnings);
        let size = p.options.font_size;
        let weight = p.options.font_weight;
        let lh = size * p.options.line_height.unwrap_or(LINE_HEIGHT_FACTOR);
        let region_left = self.page.content_left();
        let region_width = self.page.content_width();
        let ascent = self.fonts.ascent(weight) * size;
        let color = color::parse_hex_or(p.options.color.as_deref(), Rgb::BLACK);
        if has_page_tokens(&interpolated) {
            // Defer to pass 2: page number + alignment recompute.
            self.current.push(DrawOp::PageText(PageTextDraw {
                region_left,
                region_width,
                baseline: top + ascent,
                size,
                color,
                alignment: p.options.alignment,
                weight,
                raw: interpolated,
            }));
        } else {
            let face = FaceKey {
                mono: p.options.mono,
                bold: matches!(weight, FontWeight::Bold),
                italic: p.options.italic,
            };
            let runs = self.fonts.itemize(&interpolated, face, &mut self.warnings);
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
                color,
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
        let result = images::load_image(
            src,
            self.opts.fetch_images,
            self.opts.http_timeout,
            &self.fonts.all_font_bytes(),
        );
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
        let available = (self.region_right() - table_left).max(1.0);
        let columns = compute_columns(t, available, table_left);
        let pad_x = t.options.padding_x;
        let pad_y = t.options.padding_y;
        // Remember where the table starts so a per-table watermark can be
        // centered over its box once the rows are laid out.
        let wm_y_start = self.cursor.y;
        let wm_pages_before = self.pages.len();

        // Body rows: either the literal rows or the data-bound expansion.
        let bound_items: Vec<&serde_json::Value> = match &t.data {
            Some(key) => data
                .array(key)
                .map(|a| a.iter().collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };

        // Footer height, reserved by every row's fit check so the "carried
        // forward" band always fits above an intra-table page break.
        let footer_h = if t.footer_columns.is_empty() {
            0.0
        } else {
            self.row_height(&t.footer_columns, &columns, pad_x, pad_y, root)
        };

        // Draw the header row (if any) now, and remember to repeat it per page.
        self.draw_header_row(t, &columns, pad_x, pad_y, template, root);

        if let Some(_key) = &t.data {
            // One template row repeated per item.
            let template_row = t.rows.first();
            if let Some(template_row) = template_row {
                for (row_index, item) in bound_items.iter().enumerate() {
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
                        row_index,
                        footer_h,
                    )?;
                }
            }
        } else {
            for (row_index, row) in t.rows.iter().enumerate() {
                self.draw_body_row(
                    row, &columns, pad_x, pad_y, root, t, template, root, row_index, footer_h,
                )?;
            }
        }

        // Closing footer row at the table's end.
        self.draw_footer_row(t, &columns, pad_x, pad_y, footer_h, template, root);

        // Per-table watermark: stamp it centered over the table's box, on top.
        if let Some(wm) = &t.watermark {
            let table_width: f32 = columns.iter().map(|col| col.width).sum();
            let cx = table_left + table_width / 2.0;
            if self.pages.len() == wm_pages_before {
                // Table fit on one page: center between its top and the cursor.
                let cy = (wm_y_start + self.cursor.y) / 2.0;
                if let Some(op) = self.watermark_op(wm, (cx, cy)) {
                    self.current.push(op);
                }
            } else {
                // Table paginated: stamp the portion on the page it started on.
                let cy = (wm_y_start + self.page.content_bottom()) / 2.0;
                if let Some(op) = self.watermark_op(wm, (cx, cy)) {
                    self.pages[wm_pages_before].ops.push(op);
                }
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

    /// Draw the table's footer row (no-op when the table has none). Fits by
    /// construction on the intra-break path — every body row reserves
    /// `footer_h` in its own space check.
    #[allow(clippy::too_many_arguments)]
    fn draw_footer_row(
        &mut self,
        t: &Table,
        columns: &[Column],
        pad_x: f32,
        pad_y: f32,
        footer_h: f32,
        template: &Template,
        root: &Resolver,
    ) {
        if t.footer_columns.is_empty() {
            return;
        }
        self.ensure_space(footer_h, template, root);
        self.draw_row_cells(
            &t.footer_columns,
            columns,
            pad_x,
            pad_y,
            self.cursor.y,
            footer_h,
            false,
            &t.options.header,
            root,
        );
        self.cursor.y += footer_h;
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
        row_index: usize,
        footer_h: f32,
    ) -> Result<()> {
        let height = self.row_height(row, columns, pad_x, pad_y, scoped);
        // Page break: if the row (plus a pending footer band) doesn't fit,
        // stamp the footer as "carried forward", move to a new page, and
        // repeat the table header there. Never from inside a header/footer
        // band (`artifact_mode`) — a band table overflows instead of recursing.
        if !self.artifact_mode && self.cursor.y + height + footer_h > self.region_bottom() {
            self.draw_footer_row(t, columns, pad_x, pad_y, footer_h, template, root);
            self.finish_page(template, root);
            self.begin_page(template, root);
            self.draw_header_row(t, columns, pad_x, pad_y, template, root);
        }
        // Zebra stripe: fill every second body row (2nd, 4th, …) across the
        // table's full width, beneath any per-cell fill, borders, and text.
        if row_index % 2 == 1 {
            if let Some(stripe) = t.options.stripe.as_deref().and_then(color::parse_hex) {
                if let Some(first) = columns.first() {
                    let table_width: f32 = columns.iter().map(|c| c.width).sum();
                    self.current.push(DrawOp::FillRect {
                        x: first.x,
                        y: self.cursor.y,
                        w: table_width,
                        h: height,
                        color: stripe,
                    });
                }
            }
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
        for (cell, col) in zip_columns(row, columns) {
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
                                let (_, ih) = fit_image(
                                    src.width_px,
                                    src.height_px,
                                    img.width,
                                    img.height,
                                    inner_w,
                                );
                                h += ih;
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
        for (cell, col) in zip_columns(row, columns) {
            self.draw_one_cell(
                cell,
                col,
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

        // Per-cell fill — painted after the header band / zebra stripe so an
        // explicit cell background always wins.
        if let Some(fill) = style.fill.as_deref().and_then(color::parse_hex) {
            self.current.push(DrawOp::FillRect {
                x: col.x,
                y: row_top,
                w: col.width,
                h: row_height,
                color: fill,
            });
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
                // `valign` picks where the cap-height block sits in the row;
                // middle (the default) keeps the optical centering above.
                let offset = match tc.style.valign.unwrap_or_default() {
                    VAlign::Top => pad_y,
                    VAlign::Middle => (row_height - block) / 2.0,
                    VAlign::Bottom => (row_height - block - pad_y).max(pad_y),
                };
                let mut y = row_top + offset - ascent + cap;
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
                        false,
                        false,
                        false,
                        false,
                        false,
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
                let mut y = match rc.style.valign.unwrap_or_default() {
                    VAlign::Top => row_top + pad_y,
                    VAlign::Middle => row_top + ((row_height - block_h) / 2.0).max(pad_y),
                    VAlign::Bottom => row_top + (row_height - block_h - pad_y).max(pad_y),
                };
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
                                    None, None, false, false, false, false, false,
                                );
                                y += line_height(size);
                            }
                        }
                        CellContent::Image(img) => {
                            if let Some(idx) = self.intern_image(&img.value) {
                                let src = &self.images[idx];
                                let (w, h) = fit_image(
                                    src.width_px,
                                    src.height_px,
                                    img.width,
                                    img.height,
                                    inner_w,
                                );
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

/// Parse a chart value: trims, accepts a comma decimal separator, defaults to 0.
fn parse_num(s: &str) -> f64 {
    s.trim().replace(',', ".").parse::<f64>().unwrap_or(0.0)
}

/// A nameless, palette-colored single chart series.
fn single_series(values: Vec<f64>) -> crate::chart::Series {
    crate::chart::Series {
        name: None,
        color: None,
        values,
    }
}

/// Template truthiness for `if`/`unless`: a rendered scalar is falsy when it is
/// empty, `false`, `0`, or `null` (case-insensitive); anything else is truthy.
fn is_truthy(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && !t.eq_ignore_ascii_case("false") && t != "0" && !t.eq_ignore_ascii_case("null")
}

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

/// Vertical space the footer band reserves: the sum of its advancing elements
/// (paragraph lines, hr rules, images, `move` y — which may be negative).
/// Non-advancing elements (rect/line/ellipse) reserve space via `move`.
fn footer_band_height(template: &Template, _fonts: &FontRegistry) -> f32 {
    let mut h: f32 = 0.0;
    for el in &template.footer {
        match el {
            Element::Paragraph(p) => h += line_height(p.options.font_size),
            Element::Hr(hr) => h += hr.thickness.max(0.0) + 2.0,
            Element::Image(img) => h += img.height.max(0.0),
            Element::Move(m) => h += m.y,
            _ => {}
        }
    }
    if h > 0.0 {
        h + FOOTER_PADDING
    } else {
        0.0
    }
}

/// The JSON `type` name of an element, for warnings.
fn element_kind(el: &Element) -> &'static str {
    match el {
        Element::Paragraph(_) => "paragraph",
        Element::Move(_) => "move",
        Element::Image(_) => "image",
        Element::Table(_) => "table",
        Element::Hr(_) => "hr",
        Element::Rect(_) => "rect",
        Element::Line(_) => "line",
        Element::Qr(_) => "qr",
        Element::Barcode(_) => "barcode",
        Element::Ellipse(_) => "ellipse",
        Element::List(_) => "list",
        Element::Chart(_) => "chart",
        Element::Repeat(_) => "repeat",
        Element::If(_) => "if",
        Element::Unless(_) => "unless",
        Element::PageBreak => "page_break",
        Element::Columns(_) => "columns",
    }
}

/// Resolve column widths from the header row or the first body row, scaling to
/// fit `available` and distributing leftover space to width-less columns.
/// Resolve an image's drawn size (pt) from its pixel dimensions and the
/// optional target box: only `width` → height derives from the aspect ratio;
/// only `height` → width derives; both → scale to fit inside the box
/// ("contain", aspect preserved, never stretched); neither → `default_w` wide.
fn fit_image(
    src_w_px: usize,
    src_h_px: usize,
    width: f32,
    height: f32,
    default_w: f32,
) -> (f32, f32) {
    let sw = src_w_px.max(1) as f32;
    let sh = src_h_px.max(1) as f32;
    let aspect = sh / sw;
    match (width > 0.0, height > 0.0) {
        (true, true) => {
            let scale = (width / sw).min(height / sh);
            (sw * scale, sh * scale)
        }
        (true, false) => (width, width * aspect),
        (false, true) => (height / aspect, height),
        (false, false) => (default_w, default_w * aspect),
    }
}

/// Pair each cell of a row with its (possibly merged) column box. A cell with
/// `colspan: n` consumes `n` column slots: its box starts at the first
/// consumed column's left edge and spans the summed widths; following cells
/// continue from the next free slot. Cells beyond the column count are
/// dropped, matching the plain-zip semantics colspan generalizes.
fn zip_columns<'r>(row: &'r [Cell], columns: &[Column]) -> Vec<(&'r Cell, Column)> {
    let mut out = Vec::with_capacity(row.len());
    let mut idx = 0usize;
    for cell in row {
        if idx >= columns.len() {
            break;
        }
        let span = (cell.style().colspan.unwrap_or(1).max(1) as usize).min(columns.len() - idx);
        let width: f32 = columns[idx..idx + span].iter().map(|c| c.width).sum();
        out.push((
            cell,
            Column {
                x: columns[idx].x,
                width,
            },
        ));
        idx += span;
    }
    out
}

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
