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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use printpdf::{
    Actions, BorderArray, Color as PpColor, Destination, DictItem, ExternalStream, ExternalXObject,
    FontId, ImageCompression, ImageOptimizationOptions, Line, LineDashPattern, LinePoint,
    LinkAnnotation, Mm, Op, PaintMode, ParsedFont, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Polygon as PpPolygon, PolygonRing, Pt, Px, RawImage, RawImageData,
    RawImageFormat, Rect as PpRect, Rgb as PpRgb, TextItem, TextMatrix, WindingOrder, XObjectId,
    XObjectTransform,
};

use crate::color::Rgb;
use crate::draw::{
    DrawOp, ImageData, LaidOutDoc, PixelFormat, PolyPoint, StyledPiece, TextDraw, TextPiece,
};
use crate::error::{PdfError, Result};
use crate::fonts::{FontRegistry, FontSlot};

fn pp_color(c: Rgb) -> PpColor {
    PpColor::Rgb(PpRgb::new(c.r, c.g, c.b, None))
}

/// Process-wide cache of subset + printpdf-parsed faces.
///
/// Subsetting a face and re-parsing it with `ParsedFont::from_bytes` dominated
/// emit time (~2.6 ms/render with a CJK face — see `benches/render.rs`), yet
/// for template-driven documents the used character set is identical from one
/// render to the next. Keyed by the face's content digest plus the *exact*
/// used-char set (verified on hit), so identical inputs still embed identical
/// subsets — renders stay deterministic, unlike a grow-only superset cache.
struct EmbedEntry {
    chars: BTreeSet<char>,
    parsed: Arc<ParsedFont>,
}

/// Entry count bound; at ~10–100 KB per subset face this caps the cache at a
/// few MB. Wholesale clear on overflow keeps it simple — steady-state servers
/// re-fill with their handful of live (face, char-set) pairs immediately.
const EMBED_CACHE_MAX: usize = 64;

/// Keyed by (face content digest, used-char-set hash).
type EmbedCache = HashMap<([u8; 16], u64), EmbedEntry>;

fn embed_cache() -> &'static Mutex<EmbedCache> {
    static CACHE: OnceLock<Mutex<EmbedCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Subset `slot`'s face to `chars` and parse it for embedding, via the
/// process-wide cache.
fn subset_and_parse(
    fonts: &FontRegistry,
    slot: FontSlot,
    chars: &BTreeSet<char>,
    font_warnings: &mut Vec<printpdf::PdfFontParseWarning>,
) -> Result<Arc<ParsedFont>> {
    let digest = fonts.face_digest(slot);
    let chars_hash = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        chars.hash(&mut h);
        h.finish()
    };
    let key = (digest, chars_hash);
    if let Some(entry) = embed_cache().lock().unwrap().get(&key) {
        // Guard against a (astronomically unlikely) 64-bit hash collision: the
        // char set must match exactly, else fall through and rebuild.
        if entry.chars == *chars {
            return Ok(entry.parsed.clone());
        }
    }
    let bytes = fonts.subset_bytes(slot, chars);
    let parsed = Arc::new(
        ParsedFont::from_bytes(&bytes, 0, font_warnings)
            .ok_or_else(|| PdfError::Font(format!("printpdf could not parse {slot:?} font")))?,
    );
    let mut cache = embed_cache().lock().unwrap();
    if cache.len() >= EMBED_CACHE_MAX {
        cache.clear();
    }
    cache.insert(
        key,
        EmbedEntry {
            chars: chars.clone(),
            parsed: parsed.clone(),
        },
    );
    Ok(parsed)
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
pub fn emit(
    doc: &LaidOutDoc,
    fonts: &FontRegistry,
    opts: &crate::RenderOptions,
) -> Result<Vec<u8>> {
    // `"invoice"` is the historical default title, kept for byte-stable
    // output when no metadata is supplied.
    let mut pdf = PdfDocument::new(opts.title.as_deref().unwrap_or("invoice"));
    if let Some(author) = &opts.author {
        pdf.metadata.info.author = author.clone();
    }
    if let Some(subject) = &opts.subject {
        pdf.metadata.info.subject = subject.clone();
    }
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
            collect_used_chars(op, &mut used);
        }
    }

    let mut font_ids: BTreeMap<FontSlot, FontId> = BTreeMap::new();
    for (slot, chars) in &used {
        let parsed = subset_and_parse(fonts, *slot, chars, &mut font_warnings)?;
        font_ids.insert(*slot, pdf.add_font(&parsed));
    }

    // Register images once each. Rasters become Image XObjects; SVGs become
    // Form XObjects (vectors), built from the svg2pdf page.
    let mut image_ids: Vec<XObjectId> = Vec::with_capacity(doc.images.len());
    for img in &doc.images {
        if let Some(svg_pdf) = &img.vector {
            image_ids.push(pdf.add_xobject(&svg_pdf_to_xobject(svg_pdf, img.opacity)?));
        } else {
            image_ids.push(pdf.add_image_owned(raw_image(img)));
        }
    }

    for page in &doc.pages {
        let mut ops: Vec<Op> = Vec::new();
        if doc.tagged {
            // Tagged output: layout wrapped every real content op in a
            // `DrawOp::Tagged` with its structure role. Emit each such op inside
            // a `/<role> <</MCID n>> BDC … EMC` sequence (the structure tree
            // references these MCIDs), and mark other drawing ops as `/Artifact`
            // (decoration/pagination, ignored by assistive tech). Link/InternalLink
            // are annotations, not content-stream marks, so they emit unwrapped.
            // MCIDs count Tagged ops only, so watermark/background ops inserted
            // around them can't shift the numbering.
            let mut mcid = 0i64;
            for op in &page.ops {
                match op {
                    DrawOp::Tagged { role, inner, .. } => {
                        let mut props = std::collections::BTreeMap::new();
                        props.insert("MCID".to_string(), printpdf::DictItem::Int(mcid));
                        ops.push(Op::BeginMarkedContentWithProperties {
                            tag: role.tag(),
                            properties: printpdf::DictItem::Dict { map: props },
                        });
                        emit_op(inner, doc, &font_ids, &image_ids, &mut ops);
                        ops.push(Op::EndMarkedContent);
                        mcid += 1;
                    }
                    DrawOp::Link { .. } | DrawOp::InternalLink { .. } => {
                        emit_op(op, doc, &font_ids, &image_ids, &mut ops);
                    }
                    _ => {
                        ops.push(Op::BeginMarkedContent {
                            tag: "Artifact".to_string(),
                        });
                        emit_op(op, doc, &font_ids, &image_ids, &mut ops);
                        ops.push(Op::EndMarkedContent);
                    }
                }
            }
        } else {
            for op in &page.ops {
                emit_op(op, doc, &font_ids, &image_ids, &mut ops);
            }
        }
        pdf.pages.push(PdfPage::new(
            Mm(px_mm(doc.page.width)),
            Mm(px_mm(doc.page.height)),
            ops,
        ));
    }

    // Document outline. Bookmark page indices are 0-based in the IR; printpdf
    // wants them 1-based (its serializer does `saturating_sub(1)`) — passing
    // them raw sent every bookmark past page 1 one page short. `bookmarkLevel`
    // nests entries under the last shallower one, like heading levels.
    let mut outline_stack: Vec<printpdf::PageAnnotId> = Vec::new();
    for (label, page_idx, level) in &doc.bookmarks {
        let level = (*level).max(1) as usize;
        outline_stack.truncate(level - 1);
        let parent = outline_stack.last().cloned();
        let id = pdf.add_bookmark_child(label, *page_idx + 1, parent);
        outline_stack.push(id);
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

/// Import page 1 of an svg2pdf document as a self-contained Form XObject.
/// Resources are inlined (no dangling refs — printpdf only adds the stream
/// itself). The Form's matrix maps the page to a 1×1 square so `UseXobject`
/// scaling matches the raster image path.
fn svg_pdf_to_xobject(pdf_bytes: &[u8], opacity: f32) -> Result<ExternalXObject> {
    use lopdf::{Dictionary, Document, Object};

    let src = Document::load_mem(pdf_bytes)
        .map_err(|e| PdfError::Backend(format!("SVG Form: parse: {e}")))?;
    let page_id = src
        .page_iter()
        .next()
        .ok_or_else(|| PdfError::Backend("SVG Form: no page".into()))?;

    let mut content = src
        .get_page_content(page_id)
        .map_err(|e| PdfError::Backend(format!("SVG Form: could not read content: {e}")))?;
    let bbox = page_media_box(&src, page_id).unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let width = (bbox[2] - bbox[0]).abs().max(1.0);
    let height = (bbox[3] - bbox[1]).abs().max(1.0);

    let mut resources = Dictionary::new();
    if let Ok((inline, referenced)) = src.get_page_resources(page_id) {
        for rid in referenced.iter().rev() {
            if let Ok(d) = src.get_dictionary(*rid) {
                for (k, v) in d.iter() {
                    resources.set(k.clone(), v.clone());
                }
            }
        }
        if let Some(d) = inline {
            for (k, v) in d.iter() {
                resources.set(k.clone(), v.clone());
            }
        }
    }
    let resources = inline_object(&src, &Object::Dictionary(resources), 0);

    let opacity = opacity.clamp(0.0, 1.0);
    let resources = if opacity < 0.999 {
        let mut res = match resources {
            Object::Dictionary(d) => d,
            _ => Dictionary::new(),
        };
        let mut gs = Dictionary::new();
        gs.set("ca", Object::Real(opacity));
        gs.set("CA", Object::Real(opacity));
        let mut ext = Dictionary::new();
        ext.set("SoliCa", Object::Dictionary(gs));
        res.set("ExtGState", Object::Dictionary(ext));
        content.splice(0..0, b"q /SoliCa gs ".iter().copied());
        content.extend_from_slice(b" Q");
        Object::Dictionary(res)
    } else {
        resources
    };

    let sx = 1.0 / width;
    let sy = 1.0 / height;
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Form".to_vec()));
    dict.set("FormType", Object::Integer(1));
    dict.set(
        "BBox",
        Object::Array(bbox.iter().copied().map(Object::Real).collect()),
    );
    dict.set(
        "Matrix",
        Object::Array(vec![
            Object::Real(sx),
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(sy),
            Object::Real(0.0),
            Object::Real(0.0),
        ]),
    );
    dict.set("Resources", resources);
    if let Ok(page) = src.get_dictionary(page_id) {
        if let Ok(group) = page.get(b"Group") {
            dict.set("Group", inline_object(&src, group, 0));
        }
    }

    let dict_item = DictItem::from_lopdf(&Object::Dictionary(dict));
    let map = match dict_item {
        DictItem::Dict { map } => map,
        _ => std::collections::BTreeMap::new(),
    };

    Ok(ExternalXObject {
        stream: ExternalStream {
            dict: map,
            content,
            compress: true,
        },
        width: Some(Px(width.round().max(1.0) as usize)),
        height: Some(Px(height.round().max(1.0) as usize)),
        dpi: Some(72.0),
    })
}

fn inline_object(doc: &lopdf::Document, obj: &lopdf::Object, depth: usize) -> lopdf::Object {
    use lopdf::{Dictionary, Object, Stream};
    if depth > 64 {
        return obj.clone();
    }
    match obj {
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(resolved) => inline_object(doc, resolved, depth + 1),
            Err(_) => Object::Null,
        },
        Object::Dictionary(d) => {
            let mut nd = Dictionary::new();
            for (k, v) in d.iter() {
                nd.set(k.clone(), inline_object(doc, v, depth + 1));
            }
            Object::Dictionary(nd)
        }
        Object::Array(a) => {
            Object::Array(a.iter().map(|v| inline_object(doc, v, depth + 1)).collect())
        }
        Object::Stream(s) => {
            let dict = match inline_object(doc, &Object::Dictionary(s.dict.clone()), depth + 1) {
                Object::Dictionary(d) => d,
                _ => s.dict.clone(),
            };
            let mut ns = Stream::new(dict, s.content.clone());
            ns.allows_compression = s.allows_compression;
            Object::Stream(ns)
        }
        other => other.clone(),
    }
}

fn page_media_box(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Option<[f32; 4]> {
    let dict = doc.get_dictionary(page_id).ok()?;
    let mb = match dict.get(b"MediaBox").ok()? {
        lopdf::Object::Reference(r) => doc.get_object(*r).ok()?,
        other => other,
    };
    let a = match mb {
        lopdf::Object::Array(a) if a.len() == 4 => a,
        _ => return None,
    };
    let num = |o: &lopdf::Object| -> Option<f32> {
        match o {
            lopdf::Object::Integer(i) => Some(*i as f32),
            lopdf::Object::Real(r) => Some(*r),
            _ => None,
        }
    };
    Some([num(&a[0])?, num(&a[1])?, num(&a[2])?, num(&a[3])?])
}

fn raw_image(img: &ImageData) -> RawImage {
    RawImage {
        pixels: RawImageData::U8(img.pixels.clone()),
        width: img.width_px,
        height: img.height_px,
        data_format: match img.format {
            PixelFormat::Gray8 => RawImageFormat::R8,
            PixelFormat::Rgb8 => RawImageFormat::RGB8,
            PixelFormat::Rgba8 => RawImageFormat::RGBA8,
        },
        // The source identity keys printpdf's encoded-XObject cache (a
        // vendored patch); empty = re-encode on every save.
        tag: img
            .source_key
            .map(|k| k.to_le_bytes().to_vec())
            .unwrap_or_default(),
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
                        // Anchor page indices are 0-based in the IR; printpdf's
                        // Destination page is 1-based (it does page - 1 to index),
                        // so add 1 — otherwise every internal jump past page 1
                        // lands one page short (the displayed #PAGE_OF# number is
                        // right, but the click went one page early).
                        Actions::go_to(Destination::Xyz {
                            page: target_page + 1,
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
        // The tagged emit loop unwraps `Tagged` before calling emit_op; this arm
        // is only reached defensively (e.g. a Tagged op on a non-tagged page).
        DrawOp::Tagged { inner, .. } => emit_op(inner, doc, font_ids, image_ids, out),
    }
}

/// Accumulate the characters each font slot needs, recursing through `Tagged`
/// wrappers so tagged text still gets its glyphs subset and embedded.
fn collect_used_chars(op: &DrawOp, used: &mut BTreeMap<FontSlot, BTreeSet<char>>) {
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
        DrawOp::Tagged { inner, .. } => collect_used_chars(inner, used),
        _ => {}
    }
}

/// Reproduce the tagged emit loop's MCID assignment as a flat list of structure
/// leaves for the `accessibility` pass. Both walk `doc.pages` counting only
/// `Tagged` ops in stream order, so the MCIDs are guaranteed to match.
pub fn struct_leaves(doc: &LaidOutDoc) -> Vec<crate::draw::StructLeaf> {
    let mut leaves = Vec::new();
    for (page_idx, page) in doc.pages.iter().enumerate() {
        let mut mcid = 0u32;
        for op in &page.ops {
            if let DrawOp::Tagged { role, group, .. } = op {
                leaves.push(crate::draw::StructLeaf {
                    page: page_idx,
                    mcid,
                    role: role.clone(),
                    group: group.clone(),
                });
                mcid += 1;
            }
        }
    }
    leaves
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
