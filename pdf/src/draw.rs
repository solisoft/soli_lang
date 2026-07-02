//! The intermediate, backend-neutral draw model produced by layout (pass 1)
//! and consumed by the PDF backend (pass 2). All coordinates are **logical**
//! (top-left origin, y down, points); the backend flips y to PDF space.

use std::collections::HashMap;

use crate::color::Rgb;
use crate::fonts::FontSlot;
use crate::geometry::Page;
use crate::template::Alignment;

/// A point on a [`DrawOp::Polygon`] path. `bezier` marks a cubic control point.
#[derive(Debug, Clone, Copy)]
pub struct PolyPoint {
    pub x: f32,
    pub y: f32,
    pub bezier: bool,
}

/// A piece of text drawn with one font slot.
#[derive(Debug, Clone)]
pub struct TextPiece {
    pub slot: FontSlot,
    pub text: String,
}

/// A styled inline-text piece carrying its own size and color (for rich text).
#[derive(Debug, Clone)]
pub struct StyledPiece {
    pub slot: FontSlot,
    pub text: String,
    pub size: f32,
    pub color: Rgb,
}

/// Concrete positioned text (already itemized + aligned).
#[derive(Debug, Clone)]
pub struct TextDraw {
    /// Left x of the text (logical).
    pub x: f32,
    /// Baseline y (logical, top-down).
    pub baseline: f32,
    pub size: f32,
    pub color: Rgb,
    pub pieces: Vec<TextPiece>,
}

/// A footer/header text line that still contains `#PAGE#` / `#TOTAL_PAGE#`
/// tokens. Resolved to a [`TextDraw`] in pass 2 once the page count is known
/// (alignment x is recomputed because the substituted width changes).
#[derive(Debug, Clone)]
pub struct PageTextDraw {
    /// Left edge of the region the text is aligned within (logical).
    pub region_left: f32,
    /// Width of that region.
    pub region_width: f32,
    pub baseline: f32,
    pub size: f32,
    pub color: Rgb,
    pub alignment: Alignment,
    pub weight: crate::template::FontWeight,
    /// Raw text after `${...}` interpolation, still holding page tokens.
    pub raw: String,
}

/// One draw operation.
#[derive(Debug, Clone)]
pub enum DrawOp {
    Text(TextDraw),
    PageText(PageTextDraw),
    /// A stroked line segment. `dash` (pt lengths) makes it dashed/dotted.
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width: f32,
        color: Rgb,
        dash: Option<Vec<i64>>,
    },
    /// A filled and/or stroked polygon path in logical coords. `bezier` points
    /// are cubic control points (2 controls + 1 endpoint per curve) — used for
    /// ellipses and rounded rectangles.
    Polygon {
        points: Vec<PolyPoint>,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
        stroke_width: f32,
        dash: Option<Vec<i64>>,
    },
    /// A filled rectangle; `(x, y)` is the top-left (logical).
    FillRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Rgb,
    },
    /// An image placed with its top-left at `(x, y)` (logical), sized `w`×`h` pt.
    Image {
        index: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// A clickable external-URL link annotation over the box whose top-left is
    /// `(x, y)` (logical), sized `w`×`h` pt.
    Link {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        uri: String,
    },
    /// Rotated text (a watermark/stamp). Unlike other ops, `(x, y)` is the text
    /// matrix origin already in **PDF space** (bottom-left), because the rotation
    /// mixes the axes and can't be y-flipped per-point by the backend. `angle` is
    /// in degrees and passed straight to printpdf's `TranslateRotate`.
    RotatedText {
        x: f32,
        y: f32,
        angle: f32,
        size: f32,
        color: Rgb,
        pieces: Vec<TextPiece>,
    },
    /// A clickable internal jump to a named `anchor`, over the box whose top-left
    /// is `(x, y)` (logical). Resolved to a page/position at emit time.
    InternalLink {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        anchor: String,
    },
    /// A wrapped line of inline rich text whose pieces carry their own size and
    /// color (mixed styling within one line). `baseline` is logical (top-down).
    StyledText {
        x: f32,
        baseline: f32,
        pieces: Vec<StyledPiece>,
    },
}

/// Pixel layout of an [`ImageData`] buffer.
///
/// Synthetic black-on-white images (QR, barcode) use `Gray8`: one byte per
/// pixel instead of three, which cuts both the rasterisation work and the
/// flate input the PDF backend compresses on every save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 1 byte/px, DeviceGray.
    Gray8,
    /// 3 bytes/px RGB.
    Rgb8,
    /// 4 bytes/px RGB + straight alpha.
    Rgba8,
}

impl PixelFormat {
    pub fn bytes_per_px(self) -> usize {
        match self {
            PixelFormat::Gray8 => 1,
            PixelFormat::Rgb8 => 3,
            PixelFormat::Rgba8 => 4,
        }
    }
}

/// Decoded raster image ready to embed.
#[derive(Debug, Clone)]
pub struct ImageData {
    pub width_px: usize,
    pub height_px: usize,
    pub format: PixelFormat,
    pub pixels: Vec<u8>,
    /// Identity of the image's *source* (set by the image loader for
    /// deterministic, cacheable sources; `None` for per-render rasters like
    /// QR/barcode). The PDF backend forwards it so the encoded XObject —
    /// plane split + flate, milliseconds for a large logo — is reused across
    /// renders instead of recomputed per save.
    pub source_key: Option<u64>,
}

/// A single laid-out page.
#[derive(Debug, Clone, Default)]
pub struct RenderedPage {
    pub ops: Vec<DrawOp>,
}

/// The full laid-out document (pass 1 output).
#[derive(Debug, Clone)]
pub struct LaidOutDoc {
    pub page: Page,
    pub pages: Vec<RenderedPage>,
    /// `Arc` so a process-wide image-cache hit shares pixels with the document
    /// instead of cloning multi-MB buffers per render.
    pub images: Vec<std::sync::Arc<ImageData>>,
    /// Outline entries: `(label, 0-based page index, level)` — level 1 is top;
    /// deeper levels nest under the last shallower entry (like headings).
    pub bookmarks: Vec<(String, usize, u32)>,
    /// Named jump targets: `anchor → (0-based page index, logical y)`.
    pub anchors: HashMap<String, (usize, f32)>,
    /// Emit a tagged (structured) PDF — see `TemplateOptions::tagged`.
    pub tagged: bool,
    /// Document language for `/Lang` (tagged output).
    pub lang: Option<String>,
}
