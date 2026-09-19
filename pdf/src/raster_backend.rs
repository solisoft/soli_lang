//! The raster backend — the ONLY module that imports `tiny_skia` and `resvg`.
//! It paints a laid-out document (logical, top-left coordinates) into RGBA
//! pixmaps and encodes them as PNG, for previews and thumbnails.
//!
//! [`crate::pdf_backend`] is the normative reference: it produces the document
//! that ships. When the two disagree the PDF wins, and this module is the one
//! with the bug. Each arm of [`paint_op`] names the `pdf_backend` arm it
//! mirrors.
//!
//! Coordinates are easier here than in the PDF backend: logical space is
//! top-left with y increasing downward, which *is* device space, so the common
//! ops need only a scale. The y-flip appears exactly twice — in the two glyph
//! paths, because font outlines are y-up — and once more for
//! [`DrawOp::RotatedText`], whose coordinates layout already emitted in PDF
//! space.
//!
//! Known, deliberate divergences from the PDF (documented rather than fixed):
//!
//! * **Stationery** — a letterhead is composited onto emitted PDF bytes by a
//!   lopdf post-pass, which this path never produces. See
//!   [`unsupported_warnings`].
//! * **Colour/bitmap glyphs** — `ttf_parser::outline_glyph` returns nothing for
//!   CBDT/sbix/COLR faces, so they are invisible here while the PDF embeds
//!   them.
//! * **Hairlines** — below one device pixel, tiny-skia draws a coverage-scaled
//!   1 px line rather than a true sub-pixel stroke, so thin table rules read
//!   slightly differently than in a PDF viewer. Visible at low dpi.
//! * **Text advances** come from [`FontRegistry::char_advance`], not from the
//!   face's `hmtx` as a PDF viewer would. That is deliberate: layout measured
//!   with the same function, so the preview reproduces the line the document
//!   was laid out against.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tiny_skia::{
    FillRule, FilterQuality, IntSize, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    StrokeDash, Transform,
};

use crate::color::Rgb;
use crate::draw::{
    DrawOp, ImageData, LaidOutDoc, PixelFormat, PolyPoint, StyledPiece, TextDraw, TextPiece,
};
use crate::error::{PdfError, RenderWarning, Result};
use crate::fonts::{FontRegistry, FontSlot};
use crate::geometry::Page;

/// PDF's default miter limit is 10.0 and `pdf_backend` never overrides it;
/// `tiny_skia::Stroke::default()` uses 4.0, which would bevel sharp chart
/// corners the PDF keeps pointed.
const PDF_MITER_LIMIT: f32 = 10.0;

/// Default device resolution: CSS pixels, the natural size for a web preview.
pub const DEFAULT_DPI: f32 = 96.0;

/// Default ceiling on one page's pixel count (≈160 MB of RGBA). `dpi` is
/// typically a request parameter, so this is load-bearing, not decorative: an
/// A4 at 20 000 dpi would ask for hundreds of gigabytes.
pub const DEFAULT_MAX_PIXELS: u64 = 40_000_000;

/// Which pages to rasterise.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PageSelection {
    #[default]
    All,
    /// 0-based indices, in the given order. Out-of-range indices are an error
    /// — a caller that asks for page 9 of a 3-page document has a bug it
    /// cannot see if we quietly hand back two images.
    Indices(Vec<usize>),
}

/// PNG deflate effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PngCompression {
    /// Preview default: encoding dominates the per-page cost, and ~20% more
    /// bytes over a LAN beats 2-3× the latency.
    #[default]
    Fast,
    /// For committed artifacts.
    Best,
}

/// How to rasterise.
#[derive(Debug, Clone)]
pub struct RasterOptions {
    /// Device resolution; `scale = dpi / 72`. Ignored when `width` or `height`
    /// is set.
    pub dpi: f32,
    /// Target width in px. **Wins over `dpi`.**
    pub width: Option<u32>,
    /// Target height in px. **Wins over `dpi`.** Given both, the page is
    /// scaled to *fit inside* the box and the aspect ratio is preserved — a
    /// preview that stretches is a bug, not an option.
    pub height: Option<u32>,
    pub pages: PageSelection,
    /// Paper colour composited under the page; `None` leaves it transparent.
    /// The template's own `background` is already a `FillRect` op
    /// (`layout.rs`) and paints over this.
    pub paper: Option<Rgb>,
    pub compression: PngCompression,
    /// Hard cap on `px_w * px_h` for one page.
    pub max_pixels: u64,
}

impl Default for RasterOptions {
    fn default() -> Self {
        RasterOptions {
            dpi: DEFAULT_DPI,
            width: None,
            height: None,
            pages: PageSelection::All,
            paper: Some(Rgb::WHITE),
            compression: PngCompression::default(),
            max_pixels: DEFAULT_MAX_PIXELS,
        }
    }
}

/// Resolved device geometry for a page: pixel size and the logical→device
/// scale that produced it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceSize {
    pub px_w: u32,
    pub px_h: u32,
    pub scale: f32,
}

/// A usable positive dimension: finite and greater than zero. Spelled out
/// rather than `!(x > 0.0)` so NaN is rejected explicitly rather than by
/// accident.
fn positive(x: f32) -> bool {
    x.is_finite() && x > 0.0
}

/// Resolve `page` + options to a device size. The single place the
/// `width`/`height`/`dpi` precedence rule lives.
pub fn device_size(page: &Page, o: &RasterOptions) -> Result<DeviceSize> {
    if !positive(page.width) || !positive(page.height) {
        return Err(PdfError::Backend(format!(
            "page has a non-positive size ({} x {} pt)",
            page.width, page.height
        )));
    }
    let scale = match (o.width, o.height) {
        (Some(w), Some(h)) => (w as f32 / page.width).min(h as f32 / page.height),
        (Some(w), None) => w as f32 / page.width,
        (None, Some(h)) => h as f32 / page.height,
        (None, None) => {
            if !positive(o.dpi) {
                return Err(PdfError::Backend(format!(
                    "dpi must be positive, got {}",
                    o.dpi
                )));
            }
            o.dpi / 72.0
        }
    };
    if !positive(scale) {
        return Err(PdfError::Backend(
            "the requested size resolves to a non-positive scale".to_string(),
        ));
    }
    let px_w = ((page.width * scale).round() as i64).clamp(1, u32::MAX as i64) as u32;
    let px_h = ((page.height * scale).round() as i64).clamp(1, u32::MAX as i64) as u32;
    let total = px_w as u64 * px_h as u64;
    if total > o.max_pixels {
        return Err(PdfError::Backend(format!(
            "a {px_w}x{px_h} page is {total} pixels, over the {} cap",
            o.max_pixels
        )));
    }
    Ok(DeviceSize { px_w, px_h, scale })
}

/// Warnings for [`crate::RenderOptions`] features the raster path cannot
/// honour. Warn, never fail: a preview that refuses to appear because the
/// document has a letterhead is useless for exactly the documents that most
/// want previewing.
///
/// `sign` is absent deliberately — signing is a post-pass on PDF bytes
/// (`crate::sign`), outside the render entirely, so this path never sees it.
pub fn unsupported_warnings(opts: &crate::RenderOptions) -> Vec<RenderWarning> {
    let mut out = Vec::new();
    let mut warn = |feature: &str| {
        out.push(RenderWarning::RasterUnsupported {
            feature: feature.to_string(),
        })
    };
    if opts.stationery.is_some() {
        warn(
            "the `stationery` letterhead (it is composited onto emitted PDF bytes, \
             so the preview shows the content without it)",
        );
    }
    if !opts.attachments.is_empty() {
        warn("`attachments` (embedded files are not drawn on a page)");
    }
    if opts.encrypt.is_some() {
        warn("`password` protection (a preview image is plaintext by nature)");
    }
    if opts.pdfa {
        warn("`pdfa` (metadata and OutputIntent only — no visual effect)");
    }
    out
}

/// Encode a painted page.
pub fn encode_png(pixmap: &Pixmap, o: &RasterOptions) -> Result<Vec<u8>> {
    match o.compression {
        // `Pixmap::encode_png` uses the png crate's default (fast) settings.
        PngCompression::Fast => pixmap
            .encode_png()
            .map_err(|e| PdfError::Backend(format!("PNG encode: {e}"))),
        PngCompression::Best => encode_png_best(pixmap),
    }
}

fn encode_png_best(pixmap: &Pixmap) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        pixmap.data(),
        pixmap.width(),
        pixmap.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| PdfError::Backend(format!("PNG encode: {e}")))?;
    Ok(out)
}

/// Rasterise page `index` of an already laid-out document.
///
/// `doc` must have been through `resolve_page_tokens` — `PageText` ops are
/// skipped here exactly as `pdf_backend` skips them.
pub fn render_page(
    doc: &LaidOutDoc,
    fonts: &FontRegistry,
    index: usize,
    o: &RasterOptions,
) -> Result<Pixmap> {
    let page = doc.pages.get(index).ok_or_else(|| {
        PdfError::Backend(format!(
            "page {} is past the end of the document ({} pages)",
            index + 1,
            doc.pages.len()
        ))
    })?;
    let dev = device_size(&doc.page, o)?;
    let mut pixmap = Pixmap::new(dev.px_w, dev.px_h).ok_or_else(|| {
        PdfError::Backend(format!(
            "could not allocate a {}x{} pixmap",
            dev.px_w, dev.px_h
        ))
    })?;
    if let Some(paper) = o.paper {
        pixmap.fill(ts_color(paper, 1.0));
    }
    let mut painter = Painter::new(doc, fonts, dev);
    for op in &page.ops {
        painter.paint_op(op, &mut pixmap);
    }
    Ok(pixmap)
}

/// A painted pixmap as straight-alpha RGBA8.
///
/// tiny-skia stores premultiplied alpha; every image encoder wants straight.
/// A page is opaque whenever `paper` is set, which is the default, so this is
/// usually a copy — but a transparent preview must not come out with its
/// colours darkened towards black.
pub fn to_straight_rgba(pixmap: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixmap.data().len());
    for px in pixmap.pixels() {
        let a = px.alpha();
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            out.extend_from_slice(&[px.red(), px.green(), px.blue(), 255]);
        } else {
            let un = |c: u8| ((c as u16 * 255) / a as u16).min(255) as u8;
            out.extend_from_slice(&[un(px.red()), un(px.green()), un(px.blue()), a]);
        }
    }
    out
}

/// Resolve a page selection against a document, rejecting out-of-range asks.
pub(crate) fn selected_pages(doc: &LaidOutDoc, sel: &PageSelection) -> Result<Vec<usize>> {
    match sel {
        PageSelection::All => Ok((0..doc.pages.len()).collect()),
        PageSelection::Indices(ix) => {
            for &i in ix {
                if i >= doc.pages.len() {
                    return Err(PdfError::Backend(format!(
                        "page {} was requested, but the document has {} page(s)",
                        i + 1,
                        doc.pages.len()
                    )));
                }
            }
            Ok(ix.clone())
        }
    }
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn ts_color(c: Rgb, alpha: f32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(
        c.r.clamp(0.0, 1.0),
        c.g.clamp(0.0, 1.0),
        c.b.clamp(0.0, 1.0),
        alpha.clamp(0.0, 1.0),
    )
    .unwrap_or(tiny_skia::Color::BLACK)
}

fn solid(c: Rgb) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(ts_color(c, 1.0));
    paint.anti_alias = true;
    paint
}

/// A PDF dash array (pt) as a tiny-skia dash. Two impedance mismatches:
/// printpdf truncates the array to six entries, and tiny-skia rejects odd or
/// shorter-than-two arrays while PDF happily cycles them — so an odd array is
/// concatenated with itself, which *is* PDF's cycling semantics.
fn dash_array(dash: &Option<Vec<i64>>) -> Option<Vec<f32>> {
    let d = dash.as_ref()?;
    let mut v: Vec<f32> = d.iter().take(6).map(|&n| n.max(0) as f32).collect();
    if v.is_empty() || v.iter().all(|&n| n == 0.0) {
        return None;
    }
    if v.len() % 2 == 1 {
        let dup = v.clone();
        v.extend(dup);
    }
    Some(v)
}

fn stroke_dash(dash: &Option<Vec<i64>>) -> Option<StrokeDash> {
    StrokeDash::new(dash_array(dash)?, 0.0)
}

/// Adapts `ttf_parser`'s outline callbacks to a tiny-skia path.
struct Outline(PathBuilder);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// Process-wide, bounded cache of parsed SVG trees, mirroring the embed cache
/// in `pdf_backend`. Keyed by a hash of the source bytes, so the faded copy of
/// a background image (which drops `source_key`) still hits.
type TreeCache = HashMap<u64, Arc<svg2pdf::usvg::Tree>>;

const TREE_CACHE_MAX: usize = 32;

fn tree_cache() -> &'static Mutex<TreeCache> {
    static CACHE: OnceLock<Mutex<TreeCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

struct Painter<'a> {
    doc: &'a LaidOutDoc,
    fonts: &'a FontRegistry,
    dev: DeviceSize,
    /// logical pt → device px.
    to_device: Transform,
    /// PDF pt (bottom-left, y-up) → device px. Same component order as a PDF
    /// `[a b c d e f]` matrix, which is what makes `RotatedText` a one-liner.
    pdf_to_device: Transform,
    /// One parsed face per *distinct* face, keyed by content digest because
    /// several slots degrade to the same file. Cannot be process-wide: a
    /// `Face<'a>` borrows the registry's bytes.
    faces: HashMap<[u8; 16], ttf_parser::Face<'a>>,
    /// (face digest, glyph id) → outline in font units, untransformed, so one
    /// entry serves every size and position on the page.
    glyphs: HashMap<([u8; 16], u16), Option<tiny_skia::Path>>,
    /// Image index → premultiplied pixmap, built once per page.
    rasters: HashMap<usize, Option<Pixmap>>,
}

impl<'a> Painter<'a> {
    fn new(doc: &'a LaidOutDoc, fonts: &'a FontRegistry, dev: DeviceSize) -> Self {
        let s = dev.scale;
        Painter {
            doc,
            fonts,
            dev,
            to_device: Transform::from_scale(s, s),
            pdf_to_device: Transform::from_row(s, 0.0, 0.0, -s, 0.0, s * doc.page.height),
            faces: HashMap::new(),
            glyphs: HashMap::new(),
            rasters: HashMap::new(),
        }
    }

    fn face(&mut self, slot: FontSlot) -> Option<([u8; 16], f32)> {
        let digest = self.fonts.face_digest(slot);
        if !self.faces.contains_key(&digest) {
            let bytes = self.fonts.bytes(slot);
            let face = ttf_parser::Face::parse(bytes, 0).ok()?;
            self.faces.insert(digest, face);
        }
        let upem = self.faces.get(&digest)?.units_per_em() as f32;
        Some((digest, upem))
    }

    /// The outline of `ch` in font units, or `None` when the face has no
    /// drawable outline for it (a space, or a colour/bitmap glyph).
    fn glyph(&mut self, digest: [u8; 16], ch: char) -> Option<tiny_skia::Path> {
        let face = self.faces.get(&digest)?;
        let gid = face.glyph_index(ch)?;
        let key = (digest, gid.0);
        if let Some(cached) = self.glyphs.get(&key) {
            return cached.clone();
        }
        let mut outline = Outline(PathBuilder::new());
        let path = self
            .faces
            .get(&digest)
            .and_then(|f| f.outline_glyph(gid, &mut outline))
            .and_then(|_| outline.0.finish());
        self.glyphs.insert(key, path.clone());
        path
    }

    /// Draw one run of characters, advancing the pen with the registry's
    /// metrics (see the module doc). `place` maps (pen offset, glyph units) to
    /// device space.
    #[allow(clippy::too_many_arguments)]
    fn draw_run(
        &mut self,
        pixmap: &mut Pixmap,
        slot: FontSlot,
        text: &str,
        size: f32,
        color: Rgb,
        pen: &mut f32,
        place: &dyn Fn(f32, f32) -> Transform,
    ) {
        let Some((digest, upem)) = self.face(slot) else {
            // No parseable face: still advance so the rest of the line lands
            // where layout put it.
            for ch in text.chars() {
                *pen += self.fonts.char_advance(slot, ch, size);
            }
            return;
        };
        let k = size / upem;
        let paint = solid(color);
        for ch in text.chars() {
            if let Some(path) = self.glyph(digest, ch) {
                pixmap.fill_path(&path, &paint, FillRule::Winding, place(*pen, k), None);
            }
            *pen += self.fonts.char_advance(slot, ch, size);
        }
    }

    /// Mirrors `pdf_backend::emit_text`.
    fn paint_text(&mut self, pixmap: &mut Pixmap, t: &TextDraw) {
        let mut pen = t.x;
        let to_device = self.to_device;
        let baseline = t.baseline;
        for TextPiece { slot, text } in &t.pieces {
            let place = move |pen_x: f32, k: f32| {
                // Glyph outlines are y-up; logical space is y-down, hence -k.
                to_device.pre_translate(pen_x, baseline).pre_scale(k, -k)
            };
            self.draw_run(pixmap, *slot, text, t.size, t.color, &mut pen, &place);
        }
    }

    /// Mirrors `pdf_backend::emit_styled_text` — one pen across pieces, each
    /// carrying its own size and colour.
    fn paint_styled_text(
        &mut self,
        pixmap: &mut Pixmap,
        x: f32,
        baseline: f32,
        pieces: &[StyledPiece],
    ) {
        let mut pen = x;
        let to_device = self.to_device;
        for p in pieces {
            let place =
                move |pen_x: f32, k: f32| to_device.pre_translate(pen_x, baseline).pre_scale(k, -k);
            self.draw_run(pixmap, p.slot, &p.text, p.size, p.color, &mut pen, &place);
        }
    }

    /// Mirrors `pdf_backend`'s `RotatedText` arm. `(x, y)` is already PDF
    /// space, and printpdf builds `Tm = [cos, -sin, sin, cos, x, y]` with
    /// `θ = (360 - angle)°` — which `Transform::from_row` takes in the very
    /// same order. The pen advances in text space, before the rotation, just
    /// as `Op::ShowText` does inside a `Tm`.
    #[allow(clippy::too_many_arguments)]
    fn paint_rotated_text(
        &mut self,
        pixmap: &mut Pixmap,
        x: f32,
        y: f32,
        angle: f32,
        size: f32,
        color: Rgb,
        pieces: &[TextPiece],
    ) {
        let rad = (360.0 - angle).to_radians();
        let (s, c) = (rad.sin(), rad.cos());
        let base = Transform::from_row(c, -s, s, c, x, y).post_concat(self.pdf_to_device);
        let mut pen = 0.0;
        for TextPiece { slot, text } in pieces {
            // Already y-up in PDF space, so k is positive here.
            let place = move |pen_x: f32, k: f32| base.pre_translate(pen_x, 0.0).pre_scale(k, k);
            self.draw_run(pixmap, *slot, text, size, color, &mut pen, &place);
        }
    }

    /// Build (once) the premultiplied pixmap for a raster image.
    fn raster_image(&mut self, index: usize) -> Option<&Pixmap> {
        if !self.rasters.contains_key(&index) {
            let built = self
                .doc
                .images
                .get(index)
                .and_then(|img| pixmap_from_image(img));
            self.rasters.insert(index, built);
        }
        self.rasters.get(&index).and_then(|o| o.as_ref())
    }

    /// Mirrors `pdf_backend`'s `Image` arm, including its vector branch: an
    /// SVG is drawn from the retained source with resvg, because the Form
    /// XObject the PDF backend imports means nothing here.
    fn paint_image(&mut self, pixmap: &mut Pixmap, index: usize, x: f32, y: f32, w: f32, h: f32) {
        let Some(img) = self.doc.images.get(index).cloned() else {
            return;
        };
        if img.is_vector() {
            self.paint_svg(pixmap, &img, x, y, w, h);
            return;
        }
        let to_device = self.to_device;
        let Some(src) = self.raster_image(index) else {
            return;
        };
        let (sw, sh) = (src.width() as f32, src.height() as f32);
        if sw <= 0.0 || sh <= 0.0 {
            return;
        }
        let transform = to_device.pre_translate(x, y).pre_scale(w / sw, h / sh);
        // `opacity` is already baked into the pixels for rasters
        // (`images::faded`), exactly as the PDF backend ignores it here.
        pixmap.draw_pixmap(
            0,
            0,
            src.as_ref(),
            &PixmapPaint {
                opacity: 1.0,
                quality: FilterQuality::Bilinear,
                ..Default::default()
            },
            transform,
            None,
        );
    }

    fn paint_svg(&mut self, pixmap: &mut Pixmap, img: &ImageData, x: f32, y: f32, w: f32, h: f32) {
        let Some(source) = img.svg_source.as_ref() else {
            return;
        };
        let key = hash_bytes(source);
        let tree = {
            let cached = tree_cache().lock().ok().and_then(|c| c.get(&key).cloned());
            match cached {
                Some(t) => t,
                None => {
                    let fonts = self.fonts.all_font_bytes();
                    let Ok(parsed) = crate::svg::parse_svg(source, &fonts) else {
                        return;
                    };
                    let parsed = Arc::new(parsed);
                    if let Ok(mut cache) = tree_cache().lock() {
                        if cache.len() >= TREE_CACHE_MAX {
                            cache.clear();
                        }
                        cache.insert(key, parsed.clone());
                    }
                    parsed
                }
            }
        };
        let size = tree.size();
        if size.width() <= 0.0 || size.height() <= 0.0 {
            return;
        }
        let fit = Transform::from_scale(w / size.width(), h / size.height());
        let place = self.to_device.pre_translate(x, y).pre_concat(fit);

        if img.opacity >= 0.999 {
            resvg::render(&tree, place, &mut pixmap.as_mut());
            return;
        }
        // Semi-transparent vectors: render to a scratch pixmap and composite,
        // the analogue of the ExtGState the PDF backend writes.
        let pw = (w * self.dev.scale).ceil().max(1.0) as u32;
        let ph = (h * self.dev.scale).ceil().max(1.0) as u32;
        let Some(mut scratch) = Pixmap::new(pw, ph) else {
            return;
        };
        let local = Transform::from_scale(pw as f32 / size.width(), ph as f32 / size.height());
        resvg::render(&tree, local, &mut scratch.as_mut());
        pixmap.draw_pixmap(
            0,
            0,
            scratch.as_ref(),
            &PixmapPaint {
                opacity: img.opacity.clamp(0.0, 1.0),
                quality: FilterQuality::Bilinear,
                ..Default::default()
            },
            self.to_device
                .pre_translate(x, y)
                .pre_scale(w / pw as f32, h / ph as f32),
            None,
        );
    }

    fn paint_op(&mut self, op: &DrawOp, pixmap: &mut Pixmap) {
        match op {
            DrawOp::Text(t) => self.paint_text(pixmap, t),
            DrawOp::StyledText {
                x,
                baseline,
                pieces,
            } => self.paint_styled_text(pixmap, *x, *baseline, pieces),
            DrawOp::PageText(_) => {
                // Resolved to a concrete TextDraw in pass 2, as in pdf_backend.
            }
            DrawOp::Line {
                x1,
                y1,
                x2,
                y2,
                width,
                color,
                dash,
            } => {
                let mut pb = PathBuilder::new();
                pb.move_to(*x1, *y1);
                pb.line_to(*x2, *y2);
                if let Some(path) = pb.finish() {
                    let stroke = Stroke {
                        width: *width,
                        miter_limit: PDF_MITER_LIMIT,
                        dash: stroke_dash(dash),
                        ..Default::default()
                    };
                    pixmap.stroke_path(&path, &solid(*color), &stroke, self.to_device, None);
                }
            }
            DrawOp::Polygon {
                points,
                fill,
                stroke,
                stroke_width,
                dash,
            } => {
                if fill.is_none() && stroke.is_none() {
                    return;
                }
                let Some(path) = polygon_path(points) else {
                    return;
                };
                if let Some(f) = fill {
                    // NonZero in the PDF backend.
                    pixmap.fill_path(&path, &solid(*f), FillRule::Winding, self.to_device, None);
                }
                if let Some(s) = stroke {
                    let st = Stroke {
                        width: *stroke_width,
                        miter_limit: PDF_MITER_LIMIT,
                        dash: stroke_dash(dash),
                        ..Default::default()
                    };
                    pixmap.stroke_path(&path, &solid(*s), &st, self.to_device, None);
                }
            }
            DrawOp::FillRect { x, y, w, h, color } => {
                // No y-flip: logical is already top-left, y-down.
                if let Some(rect) = Rect::from_xywh(*x, *y, *w, *h) {
                    pixmap.fill_rect(rect, &solid(*color), self.to_device, None);
                }
            }
            DrawOp::RotatedText {
                x,
                y,
                angle,
                size,
                color,
                pieces,
            } => self.paint_rotated_text(pixmap, *x, *y, *angle, *size, *color, pieces),
            DrawOp::Image { index, x, y, w, h } => self.paint_image(pixmap, *index, *x, *y, *w, *h),
            // Annotations are not page content.
            DrawOp::Link { .. } | DrawOp::InternalLink { .. } => {}
            DrawOp::Tagged { inner, .. } => self.paint_op(inner, pixmap),
        }
    }
}

/// Walk a polygon's points reproducing printpdf's rule exactly: a `bezier`
/// point followed by another `bezier` point and an endpoint is a cubic;
/// anything else degrades to a line (see `vendor/printpdf/src/serialize.rs`).
fn polygon_path(points: &[PolyPoint]) -> Option<tiny_skia::Path> {
    if points.len() < 2 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(points[0].x, points[0].y);
    let mut i = 1;
    while i < points.len() {
        let p = &points[i];
        if p.bezier && i + 2 < points.len() && points[i + 1].bezier {
            let (c1, c2, end) = (&points[i], &points[i + 1], &points[i + 2]);
            pb.cubic_to(c1.x, c1.y, c2.x, c2.y, end.x, end.y);
            i += 3;
        } else {
            pb.line_to(p.x, p.y);
            i += 1;
        }
    }
    pb.close();
    pb.finish()
}

/// Decoded pixels → a premultiplied tiny-skia pixmap.
fn pixmap_from_image(img: &ImageData) -> Option<Pixmap> {
    let (w, h) = (img.width_px, img.height_px);
    let n = w.checked_mul(h)?;
    if n == 0 {
        return None;
    }
    let mut rgba = Vec::with_capacity(n * 4);
    match img.format {
        PixelFormat::Gray8 => {
            for &g in img.pixels.iter().take(n) {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
        }
        PixelFormat::Rgb8 => {
            for px in img.pixels.as_chunks::<3>().0.iter().take(n) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        PixelFormat::Rgba8 => {
            // The IR carries straight alpha; tiny-skia stores premultiplied.
            for px in img.pixels.as_chunks::<4>().0.iter().take(n) {
                let a = px[3] as u16;
                let m = |c: u8| ((c as u16 * a) / 255) as u8;
                rgba.extend_from_slice(&[m(px[0]), m(px[1]), m(px[2]), px[3]]);
            }
        }
    }
    if rgba.len() < n * 4 {
        rgba.resize(n * 4, 0);
    }
    Pixmap::from_vec(rgba, IntSize::from_wh(w as u32, h as u32)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Page;

    fn a4() -> Page {
        Page::default()
    }

    #[test]
    fn default_dpi_gives_css_pixels() {
        let d = device_size(&a4(), &RasterOptions::default()).unwrap();
        assert_eq!((d.px_w, d.px_h), (794, 1123));
    }

    #[test]
    fn width_overrides_dpi() {
        let o = RasterOptions {
            dpi: 600.0,
            width: Some(400),
            ..Default::default()
        };
        let d = device_size(&a4(), &o).unwrap();
        assert_eq!(d.px_w, 400);
        // Aspect preserved.
        assert_eq!(d.px_h, (400.0 * a4().height / a4().width).round() as u32);
    }

    #[test]
    fn height_alone_drives_the_scale() {
        let o = RasterOptions {
            height: Some(500),
            ..Default::default()
        };
        let d = device_size(&a4(), &o).unwrap();
        assert_eq!(d.px_h, 500);
    }

    #[test]
    fn width_and_height_fit_inside_the_box_without_distorting() {
        let o = RasterOptions {
            width: Some(1000),
            height: Some(500),
            ..Default::default()
        };
        let d = device_size(&a4(), &o).unwrap();
        // The page is taller than wide, so height binds.
        assert_eq!(d.px_h, 500);
        assert!(d.px_w < 1000);
    }

    #[test]
    fn a_hostile_dpi_is_refused_before_allocating() {
        let o = RasterOptions {
            dpi: 20_000.0,
            ..Default::default()
        };
        assert!(device_size(&a4(), &o).is_err());
    }

    #[test]
    fn dash_arrays_are_normalised_for_tiny_skia() {
        // Odd arrays cycle in PDF; tiny-skia needs an even one.
        assert_eq!(dash_array(&Some(vec![3])).unwrap(), vec![3.0, 3.0]);
        assert_eq!(
            dash_array(&Some(vec![3, 2, 1])).unwrap(),
            vec![3.0, 2.0, 1.0, 3.0, 2.0, 1.0]
        );
        // printpdf truncates to six entries, so we must too.
        assert_eq!(
            dash_array(&Some(vec![1, 2, 3, 4, 5, 6, 7, 8]))
                .unwrap()
                .len(),
            6
        );
        assert!(dash_array(&Some(vec![0, 0])).is_none());
        assert!(dash_array(&None).is_none());
        // And the tiny-skia object really does build from the normalised form.
        assert!(stroke_dash(&Some(vec![3])).is_some());
    }

    #[test]
    fn rotated_text_origin_lands_where_the_pdf_puts_it() {
        let page = a4();
        let dev = DeviceSize {
            px_w: 794,
            px_h: 1123,
            scale: 96.0 / 72.0,
        };
        let s = dev.scale;
        let pdf_to_device = Transform::from_row(s, 0.0, 0.0, -s, 0.0, s * page.height);
        for angle in [0.0f32, 45.0, 90.0, 315.0] {
            let rad = (360.0 - angle).to_radians();
            let (sn, cs) = (rad.sin(), rad.cos());
            let base =
                Transform::from_row(cs, -sn, sn, cs, 100.0, 200.0).post_concat(pdf_to_device);
            let mut p = [tiny_skia::Point::from_xy(0.0, 0.0)];
            base.map_points(&mut p);
            // Whatever the angle, the text origin is the PDF point, y-flipped.
            assert!((p[0].x - 100.0 * s).abs() < 0.01, "angle {angle}");
            assert!(
                (p[0].y - (page.height - 200.0) * s).abs() < 0.01,
                "angle {angle}"
            );
        }
    }
}
