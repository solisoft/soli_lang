//! The printpdf backend — the ONLY module that imports `printpdf`. It turns a
//! laid-out document (logical, top-left coordinates) into PDF bytes, flipping y
//! to PDF's bottom-left space and registering fonts/images.
//!
//! Notes on printpdf 0.9.1 (verified against the crate source):
//! * Real font parsing/embedding needs the `text_layout` feature; the
//!   `TextItem::Text` path emits Type0 composite fonts with `/ToUnicode` and
//!   embedded font files (good for PDF/A text extraction + CJK).
//! * Font *subsetting* is disabled upstream (`if false && ...`), so we subset
//!   each face ourselves *before* `ParsedFont::from_bytes`, retaining glyph ids
//!   and the cmap so it stays transparent (see `crate::fonts::subset`). We also
//!   only register the large CJK fallback when the document actually uses it.
//! * `PdfDocument::save` writes a PDF 1.3 header; the facturx step rewrites the
//!   version to 1.7 for PDF/A-3.

use std::collections::{BTreeMap, BTreeSet};

use printpdf::{
    Actions, BorderArray, Color as PpColor, Destination, FontId, ImageCompression,
    ImageOptimizationOptions, Line, LineDashPattern, LinePoint, LinkAnnotation, Mm, Op, PaintMode,
    ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Polygon as PpPolygon,
    PolygonRing, Pt, RawImage, RawImageData, RawImageFormat, Rect as PpRect, Rgb as PpRgb,
    TextItem, TextMatrix, WindingOrder, XObjectId, XObjectTransform,
};

use crate::color::Rgb;
use crate::draw::{DrawOp, ImageData, LaidOutDoc, PolyPoint, StyledPiece, TextDraw, TextPiece};
use crate::error::{PdfError, Result};
use crate::fonts::{FontRegistry, FontSlot};

fn pp_color(c: Rgb) -> PpColor {
    PpColor::Rgb(PpRgb::new(c.r, c.g, c.b, None))
}

/// Set a dash pattern for subsequent strokes (no-op when `None`).
fn push_dash(out: &mut Vec<Op>, dash: &Option<Vec<i64>>) {
    if let Some(d) = dash {
        out.push(Op::SetLineDashPattern {
            dash: LineDashPattern::from_array(d, 0),
        });
    }
}

/// Reset to a solid stroke after a dashed shape, so the dash doesn't leak.
fn clear_dash(out: &mut Vec<Op>, dash: &Option<Vec<i64>>) {
    if dash.is_some() {
        out.push(Op::SetLineDashPattern {
            dash: LineDashPattern {
                offset: 0,
                dash_1: None,
                gap_1: None,
                dash_2: None,
                gap_2: None,
                dash_3: None,
                gap_3: None,
            },
        });
    }
}

/// Serialize a laid-out document to PDF bytes.
pub fn emit(doc: &LaidOutDoc, fonts: &FontRegistry) -> Result<Vec<u8>> {
    let mut pdf = PdfDocument::new("invoice");
    let mut font_warnings = Vec::new();

    // Register fonts. Regular is always embedded (it's the fallback target for
    // any slot that ends up unregistered); every other styled/fallback slot is
    // embedded only when the document actually references it (bold/italic/mono
    // and the large CJK fallback). Each face is subset to just the characters
    // used in its slot before embedding (retaining glyph ids, so printpdf is
    // none the wiser).
    let mut used: BTreeMap<FontSlot, BTreeSet<char>> = BTreeMap::new();
    used.entry(FontSlot::REGULAR).or_default();
    for page in &doc.pages {
        for op in &page.ops {
            match op {
                DrawOp::Text(t) => {
                    for piece in &t.pieces {
                        used.entry(piece.slot)
                            .or_default()
                            .extend(piece.text.chars());
                    }
                }
                DrawOp::RotatedText { pieces, .. } => {
                    for piece in pieces {
                        used.entry(piece.slot)
                            .or_default()
                            .extend(piece.text.chars());
                    }
                }
                DrawOp::StyledText { pieces, .. } => {
                    for piece in pieces {
                        used.entry(piece.slot)
                            .or_default()
                            .extend(piece.text.chars());
                    }
                }
                _ => {}
            }
        }
    }

    let mut font_ids: BTreeMap<FontSlot, FontId> = BTreeMap::new();
    for (slot, chars) in &used {
        let bytes = fonts.subset_bytes(*slot, chars);
        let parsed = ParsedFont::from_bytes(&bytes, 0, &mut font_warnings)
            .ok_or_else(|| PdfError::Font(format!("printpdf could not parse {slot:?} font")))?;
        font_ids.insert(*slot, pdf.add_font(&parsed));
    }

    // Register images once each.
    let mut image_ids: Vec<XObjectId> = Vec::with_capacity(doc.images.len());
    for img in &doc.images {
        image_ids.push(pdf.add_image(&raw_image(img)));
    }

    for page in &doc.pages {
        let mut ops: Vec<Op> = Vec::new();
        for op in &page.ops {
            emit_op(op, doc, &font_ids, &image_ids, &mut ops);
        }
        pdf.pages.push(PdfPage::new(
            Mm(px_mm(doc.page.width)),
            Mm(px_mm(doc.page.height)),
            ops,
        ));
    }

    // Document outline (flat). Bookmark page indices are 0-based.
    for (label, page_idx) in &doc.bookmarks {
        pdf.add_bookmark(label, *page_idx);
    }

    let mut warnings = Vec::new();
    // Flate-compress the embedded image XObjects (logos, QR, barcode). Without
    // this, printpdf stores the raw RGB(A) samples uncompressed — a single
    // 512x512 logo is ~768 KB, so an image-bearing document balloons to MBs.
    // We force lossless `Flate` (never the default `Auto`, which can pick lossy
    // JPEG and would smudge a scannable QR/barcode) and keep `auto_optimize`
    // off so the transparent-logo alpha channel is preserved verbatim.
    let save_opts = PdfSaveOptions {
        image_optimization: Some(ImageOptimizationOptions {
            quality: None,
            max_image_size: None,
            dither_greyscale: None,
            convert_to_greyscale: Some(false),
            auto_optimize: Some(false),
            format: Some(ImageCompression::Flate),
        }),
        ..PdfSaveOptions::default()
    };
    let bytes = pdf.save(&save_opts, &mut warnings);
    Ok(bytes)
}

/// Convert points to millimetres (printpdf page size is in mm).
fn px_mm(pt: f32) -> f32 {
    pt * 0.352_778
}

fn raw_image(img: &ImageData) -> RawImage {
    RawImage {
        pixels: RawImageData::U8(img.pixels.clone()),
        width: img.width_px,
        height: img.height_px,
        data_format: if img.has_alpha {
            RawImageFormat::RGBA8
        } else {
            RawImageFormat::RGB8
        },
        tag: Vec::new(),
    }
}

fn emit_op(
    op: &DrawOp,
    doc: &LaidOutDoc,
    font_ids: &BTreeMap<FontSlot, FontId>,
    image_ids: &[XObjectId],
    out: &mut Vec<Op>,
) {
    let page = &doc.page;
    match op {
        DrawOp::Text(t) => emit_text(t, page, font_ids, out),
        DrawOp::StyledText {
            x,
            baseline,
            pieces,
        } => emit_styled_text(*x, *baseline, pieces, page, font_ids, out),
        DrawOp::PageText(_) => {
            // Page-token text is resolved to a concrete TextDraw in pass 2
            // before reaching the backend; nothing to do here.
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
            out.push(Op::SetOutlineColor {
                col: pp_color(*color),
            });
            out.push(Op::SetOutlineThickness { pt: Pt(*width) });
            push_dash(out, dash);
            out.push(Op::DrawLine {
                line: Line {
                    points: vec![
                        LinePoint {
                            p: Point {
                                x: Pt(*x1),
                                y: Pt(page.to_pdf_y(*y1)),
                            },
                            bezier: false,
                        },
                        LinePoint {
                            p: Point {
                                x: Pt(*x2),
                                y: Pt(page.to_pdf_y(*y2)),
                            },
                            bezier: false,
                        },
                    ],
                    is_closed: false,
                },
            });
            clear_dash(out, dash);
        }
        DrawOp::Polygon {
            points,
            fill,
            stroke,
            stroke_width,
            dash,
        } => {
            let mode = match (fill.is_some(), stroke.is_some()) {
                (true, true) => PaintMode::FillStroke,
                (true, false) => PaintMode::Fill,
                (false, true) => PaintMode::Stroke,
                (false, false) => return,
            };
            if let Some(f) = fill {
                out.push(Op::SetFillColor { col: pp_color(*f) });
            }
            if let Some(s) = stroke {
                out.push(Op::SetOutlineColor { col: pp_color(*s) });
                out.push(Op::SetOutlineThickness {
                    pt: Pt(*stroke_width),
                });
            }
            push_dash(out, dash);
            let ring: Vec<LinePoint> = points
                .iter()
                .map(|PolyPoint { x, y, bezier }| LinePoint {
                    p: Point {
                        x: Pt(*x),
                        y: Pt(page.to_pdf_y(*y)),
                    },
                    bezier: *bezier,
                })
                .collect();
            out.push(Op::DrawPolygon {
                polygon: PpPolygon {
                    rings: vec![PolygonRing { points: ring }],
                    mode,
                    winding_order: WindingOrder::NonZero,
                },
            });
            clear_dash(out, dash);
        }
        DrawOp::FillRect { x, y, w, h, color } => {
            // (x, y) is the top-left; PDF rect origin is the lower-left. NOTE:
            // printpdf's Op::DrawRectangle ends the path with `n` (no paint), so
            // it never fills — we emit a filled Polygon instead.
            let bottom = page.to_pdf_y(*y + *h);
            out.push(Op::SetFillColor {
                col: pp_color(*color),
            });
            let rect = PpRect {
                x: Pt(*x),
                y: Pt(bottom),
                width: Pt(*w),
                height: Pt(*h),
                mode: Some(PaintMode::Fill),
                winding_order: Some(WindingOrder::NonZero),
            };
            out.push(Op::DrawPolygon {
                polygon: rect.to_polygon(),
            });
        }
        DrawOp::RotatedText {
            x,
            y,
            angle,
            size,
            color,
            pieces,
        } => {
            if pieces.is_empty() {
                return;
            }
            // (x, y) is already PDF-space; TranslateRotate sets the text origin
            // and rotates around it, so no SetTextCursor is needed.
            out.push(Op::StartTextSection);
            out.push(Op::SetFillColor {
                col: pp_color(*color),
            });
            out.push(Op::SetTextMatrix {
                matrix: TextMatrix::TranslateRotate(Pt(*x), Pt(*y), *angle),
            });
            for TextPiece { slot, text } in pieces {
                let fid = font_ids
                    .get(slot)
                    .or_else(|| font_ids.get(&FontSlot::REGULAR))
                    .expect("at least Regular font registered");
                out.push(Op::SetFont {
                    font: PdfFontHandle::External(fid.clone()),
                    size: Pt(*size),
                });
                out.push(Op::ShowText {
                    items: vec![TextItem::Text(text.clone())],
                });
            }
            out.push(Op::EndTextSection);
        }
        DrawOp::Link { x, y, w, h, uri } => {
            // Logical top-left (x, y) -> PDF lower-left rectangle.
            let bottom = page.to_pdf_y(*y + *h);
            out.push(Op::LinkAnnotation {
                link: LinkAnnotation::new(
                    PpRect::from_xywh(Pt(*x), Pt(bottom), Pt(*w), Pt(*h)),
                    Actions::uri(uri.clone()),
                    // Zero-width border: the link is clickable but draws no box.
                    Some(BorderArray::Solid([0.0, 0.0, 0.0])),
                    None,
                    None,
                ),
            });
        }
        DrawOp::InternalLink { x, y, w, h, anchor } => {
            // Resolve the anchor to a page + position; skip if it doesn't exist.
            if let Some(&(target_page, anchor_y)) = doc.anchors.get(anchor) {
                let bottom = page.to_pdf_y(*y + *h);
                out.push(Op::LinkAnnotation {
                    link: LinkAnnotation::new(
                        PpRect::from_xywh(Pt(*x), Pt(bottom), Pt(*w), Pt(*h)),
                        Actions::go_to(Destination::Xyz {
                            page: target_page,
                            left: Some(0.0),
                            top: Some(page.to_pdf_y(anchor_y)),
                            zoom: None,
                        }),
                        Some(BorderArray::Solid([0.0, 0.0, 0.0])),
                        None,
                        None,
                    ),
                });
            }
        }
        DrawOp::Image { index, x, y, w, h } => {
            if let (Some(id), Some(src)) = (image_ids.get(*index), doc.images.get(*index)) {
                let bottom = page.to_pdf_y(*y + *h);
                let scale_x = *w / src.width_px.max(1) as f32;
                let scale_y = *h / src.height_px.max(1) as f32;
                out.push(Op::UseXobject {
                    id: id.clone(),
                    transform: XObjectTransform {
                        translate_x: Some(Pt(*x)),
                        translate_y: Some(Pt(bottom)),
                        rotate: None,
                        scale_x: Some(scale_x),
                        scale_y: Some(scale_y),
                        dpi: Some(72.0),
                    },
                });
            }
        }
    }
}

fn emit_text(
    t: &TextDraw,
    page: &crate::geometry::Page,
    font_ids: &BTreeMap<FontSlot, FontId>,
    out: &mut Vec<Op>,
) {
    if t.pieces.is_empty() {
        return;
    }
    let baseline = page.to_pdf_y(t.baseline);
    out.push(Op::StartTextSection);
    out.push(Op::SetFillColor {
        col: pp_color(t.color),
    });
    out.push(Op::SetTextCursor {
        pos: Point {
            x: Pt(t.x),
            y: Pt(baseline),
        },
    });
    for TextPiece { slot, text } in &t.pieces {
        // Fall back to Regular if a slot wasn't registered (shouldn't happen).
        let fid = font_ids
            .get(slot)
            .or_else(|| font_ids.get(&FontSlot::REGULAR))
            .expect("at least Regular font registered");
        out.push(Op::SetFont {
            font: PdfFontHandle::External(fid.clone()),
            size: Pt(t.size),
        });
        out.push(Op::ShowText {
            items: vec![TextItem::Text(text.clone())],
        });
    }
    out.push(Op::EndTextSection);
}

/// Emit a line of inline rich text: one text section, the cursor set once, then
/// per-piece fill color + font size (the cursor advances across pieces).
fn emit_styled_text(
    x: f32,
    baseline: f32,
    pieces: &[StyledPiece],
    page: &crate::geometry::Page,
    font_ids: &BTreeMap<FontSlot, FontId>,
    out: &mut Vec<Op>,
) {
    if pieces.is_empty() {
        return;
    }
    out.push(Op::StartTextSection);
    out.push(Op::SetTextCursor {
        pos: Point {
            x: Pt(x),
            y: Pt(page.to_pdf_y(baseline)),
        },
    });
    for StyledPiece {
        slot,
        text,
        size,
        color,
    } in pieces
    {
        let fid = font_ids
            .get(slot)
            .or_else(|| font_ids.get(&FontSlot::REGULAR))
            .expect("at least Regular font registered");
        out.push(Op::SetFillColor {
            col: pp_color(*color),
        });
        out.push(Op::SetFont {
            font: PdfFontHandle::External(fid.clone()),
            size: Pt(*size),
        });
        out.push(Op::ShowText {
            items: vec![TextItem::Text(text.clone())],
        });
    }
    out.push(Op::EndTextSection);
}
