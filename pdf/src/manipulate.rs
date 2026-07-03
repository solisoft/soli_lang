//! Operate on *existing* PDFs: merge several into one, keep a subset of pages,
//! and stamp text (labels / watermarks) onto pages. Companions to [`crate::forms`]
//! — together they make the engine a PDF *toolkit*, not just a generator.
//!
//! All three are lopdf passes over loaded documents.

use std::collections::BTreeSet;

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId};

use crate::error::{PdfError, Result};

/// Attributes a page may inherit from an ancestor `/Pages` node. We inline them
/// before merging so a merged page keeps its size/resources once its original
/// page tree is dropped.
const INHERITABLE: [&[u8]; 4] = [b"MediaBox", b"Resources", b"CropBox", b"Rotate"];

fn is_type(obj: &Object, name: &[u8]) -> bool {
    obj.as_dict()
        .ok()
        .and_then(|d| d.get(b"Type").ok())
        .and_then(|o| o.as_name().ok())
        == Some(name)
}

/// Copy inheritable attributes from each page's ancestor `/Pages` chain onto the
/// page itself, so the page is self-contained after its tree is discarded.
fn inline_inherited(doc: &mut Document) {
    let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    for pid in page_ids {
        let mut to_set: Vec<(Vec<u8>, Object)> = Vec::new();
        for key in INHERITABLE {
            let page = doc.get_object(pid).ok().and_then(|o| o.as_dict().ok());
            let Some(page) = page else { continue };
            if page.get(key).is_ok() {
                continue;
            }
            let mut cur = page.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
            while let Some(parent) = cur {
                let Some(pd) = doc.get_object(parent).ok().and_then(|o| o.as_dict().ok()) else {
                    break;
                };
                if let Ok(v) = pd.get(key) {
                    to_set.push((key.to_vec(), v.clone()));
                    break;
                }
                cur = pd.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
            }
        }
        if let Ok(d) = doc.get_object_mut(pid).and_then(Object::as_dict_mut) {
            for (k, v) in to_set {
                d.set(k, v);
            }
        }
    }
}

/// Concatenate `pdfs` into a single document, in order.
pub fn merge(pdfs: &[Vec<u8>]) -> Result<Vec<u8>> {
    if pdfs.is_empty() {
        return Err(PdfError::Backend("merge: no PDFs given".into()));
    }
    let mut max_id = 1u32;
    let mut page_ids: Vec<ObjectId> = Vec::new();
    let mut merged = Document::with_version("1.7");

    for bytes in pdfs {
        let mut d = Document::load_mem(bytes)
            .map_err(|e| PdfError::Backend(format!("merge: could not parse a PDF: {e}")))?;
        d.renumber_objects_with(max_id);
        max_id = d.max_id + 1;
        inline_inherited(&mut d);
        for pid in d.get_pages().into_values() {
            page_ids.push(pid);
        }
        for (id, obj) in std::mem::take(&mut d.objects) {
            // The old catalog + page-tree nodes are rebuilt fresh below.
            if is_type(&obj, b"Catalog") || is_type(&obj, b"Pages") {
                continue;
            }
            merged.objects.insert(id, obj);
        }
    }

    // Reserve ids above every merged object so the new nodes don't collide.
    merged.max_id = max_id;
    let pages_id = merged.new_object_id();
    let mut kids = Vec::with_capacity(page_ids.len());
    for pid in &page_ids {
        if let Some(d) = merged
            .objects
            .get_mut(pid)
            .and_then(|o| o.as_dict_mut().ok())
        {
            d.set("Parent", Object::Reference(pages_id));
        }
        kids.push(Object::Reference(*pid));
    }
    let count = kids.len() as i64;
    merged.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => Object::Name(b"Pages".to_vec()),
            "Kids" => Object::Array(kids),
            "Count" => Object::Integer(count),
        }),
    );
    let catalog = merged.add_object(dictionary! {
        "Type" => Object::Name(b"Catalog".to_vec()),
        "Pages" => Object::Reference(pages_id),
    });
    merged.trailer.set("Root", Object::Reference(catalog));
    merged.prune_objects();
    merged.renumber_objects();

    let mut out = Vec::new();
    merged
        .save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("merge: could not save: {e}")))?;
    Ok(out)
}

/// Keep only the pages in `keep` (1-based), dropping the rest. Out-of-range and
/// duplicate page numbers are ignored; kept pages stay in their original order.
pub fn select_pages(pdf: &[u8], keep: &[u32]) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("pages: could not parse the PDF: {e}")))?;
    let total = doc.get_pages().len() as u32;
    let keepset: BTreeSet<u32> = keep
        .iter()
        .copied()
        .filter(|&n| n >= 1 && n <= total)
        .collect();
    if keepset.is_empty() {
        return Err(PdfError::Backend(
            "pages: no valid page numbers selected".into(),
        ));
    }
    let to_delete: Vec<u32> = (1..=total).filter(|n| !keepset.contains(n)).collect();
    doc.delete_pages(&to_delete);
    doc.prune_objects();

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("pages: could not save: {e}")))?;
    Ok(out)
}

/// A text stamp / watermark drawn onto an existing PDF's pages.
#[derive(Debug, Clone)]
pub struct StampOptions {
    pub text: String,
    /// 1-based pages to stamp; `None` stamps every page.
    pub pages: Option<Vec<u32>>,
    /// Position in points from the page's bottom-left; `None` centers on the page.
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub size: f32,
    /// Fill color as `(r, g, b)` in 0..1.
    pub color: (f32, f32, f32),
    /// Counter-clockwise rotation in degrees (45 = a classic diagonal watermark).
    pub rotation: f32,
    /// Fill opacity 0..1 (1 = opaque).
    pub opacity: f32,
}

impl Default for StampOptions {
    fn default() -> Self {
        StampOptions {
            text: String::new(),
            pages: None,
            x: None,
            y: None,
            size: 48.0,
            color: (0.6, 0.6, 0.6),
            rotation: 45.0,
            opacity: 0.25,
        }
    }
}

fn escape_pdf(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '(' => "\\(".chars().collect::<Vec<_>>(),
            ')' => "\\)".chars().collect(),
            '\\' => "\\\\".chars().collect(),
            c => vec![c],
        })
        .collect()
}

/// Stamp `opts.text` onto the selected pages.
pub fn stamp(pdf: &[u8], opts: &StampOptions) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("stamp: could not parse the PDF: {e}")))?;

    let font_id = doc.add_object(dictionary! {
        "Type" => Object::Name(b"Font".to_vec()),
        "Subtype" => Object::Name(b"Type1".to_vec()),
        "BaseFont" => Object::Name(b"Helvetica".to_vec()),
    });
    let gs_id = if opts.opacity < 1.0 {
        Some(doc.add_object(dictionary! {
            "Type" => Object::Name(b"ExtGState".to_vec()),
            "ca" => Object::Real(opts.opacity.clamp(0.0, 1.0)),
            "CA" => Object::Real(opts.opacity.clamp(0.0, 1.0)),
        }))
    } else {
        None
    };

    let pages = doc.get_pages();
    for (n, pid) in pages {
        if let Some(sel) = &opts.pages {
            if !sel.contains(&n) {
                continue;
            }
        }
        let (pw, ph) = page_size(&doc, pid);
        add_page_resource(&mut doc, pid, b"Font", b"StampF", font_id)?;
        if let Some(gs) = gs_id {
            add_page_resource(&mut doc, pid, b"ExtGState", b"GSStamp", gs)?;
        }

        // Approximate Helvetica text width to center the stamp on its anchor.
        let text_w = opts.size * 0.5 * opts.text.chars().count() as f32;
        let cx = opts.x.unwrap_or(pw / 2.0);
        let cy = opts.y.unwrap_or(ph / 2.0);
        let theta = opts.rotation.to_radians();
        let (cos, sin) = (theta.cos(), theta.sin());
        let (r, g, b) = opts.color;

        let mut content = String::new();
        content.push_str("q\n");
        if gs_id.is_some() {
            content.push_str("/GSStamp gs\n");
        }
        // Rotate around (cx, cy), then draw the text shifted left by half its width.
        content.push_str(&format!(
            "{cos:.5} {sin:.5} {:.5} {cos:.5} {cx:.2} {cy:.2} cm\n",
            -sin
        ));
        content.push_str("BT\n");
        content.push_str(&format!("/StampF {:.2} Tf\n", opts.size));
        content.push_str(&format!("{r:.3} {g:.3} {b:.3} rg\n"));
        content.push_str(&format!("{:.2} 0 Td\n", -text_w / 2.0));
        content.push_str(&format!("({}) Tj\n", escape_pdf(&opts.text)));
        content.push_str("ET\nQ");

        doc.add_page_contents(pid, content.into_bytes())
            .map_err(|e| PdfError::Backend(format!("stamp: page content: {e}")))?;
    }

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("stamp: could not save: {e}")))?;
    Ok(out)
}

/// The page's `/MediaBox` size in points, defaulting to A4.
fn page_size(doc: &Document, pid: ObjectId) -> (f32, f32) {
    doc.get_object(pid)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"MediaBox").ok())
        .and_then(|o| o.as_array().ok())
        .map(|a| {
            let n: Vec<f32> = a
                .iter()
                .filter_map(|v| {
                    v.as_float()
                        .ok()
                        .or_else(|| v.as_i64().ok().map(|i| i as f32))
                })
                .collect();
            if n.len() == 4 {
                ((n[2] - n[0]).abs(), (n[3] - n[1]).abs())
            } else {
                (595.0, 842.0)
            }
        })
        .unwrap_or((595.0, 842.0))
}

/// Add `/<category> /<key> <ref>` to a page's `/Resources`, resolving whether
/// they are inline or referenced (or absent).
fn add_page_resource(
    doc: &mut Document,
    pid: ObjectId,
    category: &[u8],
    key: &[u8],
    value: ObjectId,
) -> Result<()> {
    // Resolve the /Resources dictionary's object id, inlining a fresh one if
    // the page has none.
    let res = doc
        .get_object(pid)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Resources").ok())
        .cloned();
    let res_id = match res {
        Some(Object::Reference(id)) => id,
        Some(Object::Dictionary(d)) => {
            let id = doc.add_object(Object::Dictionary(d));
            set_page_resources_ref(doc, pid, id);
            id
        }
        _ => {
            let id = doc.add_object(Object::Dictionary(Dictionary::new()));
            set_page_resources_ref(doc, pid, id);
            id
        }
    };
    // The category (e.g. /Font) may itself be an indirect reference shared
    // across pages — mutate the referenced dict in place rather than replacing
    // it (which would drop every other font/xobject on the page).
    let cat_obj = doc
        .get_object(res_id)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(category).ok())
        .cloned();
    match cat_obj {
        Some(Object::Reference(cat_id)) => {
            if let Ok(cd) = doc.get_object_mut(cat_id).and_then(Object::as_dict_mut) {
                cd.set(key.to_vec(), Object::Reference(value));
            }
        }
        other => {
            let mut cat = other
                .and_then(|o| o.as_dict().ok().cloned())
                .unwrap_or_default();
            cat.set(key.to_vec(), Object::Reference(value));
            if let Ok(rd) = doc.get_object_mut(res_id).and_then(Object::as_dict_mut) {
                rd.set(category.to_vec(), Object::Dictionary(cat));
            }
        }
    }
    Ok(())
}

fn set_page_resources_ref(doc: &mut Document, pid: ObjectId, res_id: ObjectId) {
    if let Ok(page) = doc.get_object_mut(pid).and_then(Object::as_dict_mut) {
        page.set("Resources", Object::Reference(res_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::Stream;

    fn one_page(label: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content = format!("BT /F1 24 Tf 72 700 Td ({label}) Tj ET");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let font = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Font".to_vec()),
            "Subtype" => Object::Name(b"Type1".to_vec()),
            "BaseFont" => Object::Name(b"Helvetica".to_vec()),
        });
        let page = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Page".to_vec()),
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => Object::Reference(content_id),
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font) } },
        });
        doc.set_object(
            pages_id,
            dictionary! {
                "Type" => Object::Name(b"Pages".to_vec()),
                "Kids" => vec![Object::Reference(page)], "Count" => 1,
            },
        );
        let cat = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Catalog".to_vec()), "Pages" => Object::Reference(pages_id),
        });
        doc.trailer.set("Root", Object::Reference(cat));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn merge_concatenates_pages() {
        let merged = merge(&[one_page("A"), one_page("B"), one_page("C")]).expect("merge");
        let doc = Document::load_mem(&merged).unwrap();
        assert_eq!(doc.get_pages().len(), 3, "all pages present");
        // Each page's /Parent points at the single new Pages node.
        let parents: BTreeSet<ObjectId> = doc
            .get_pages()
            .into_values()
            .filter_map(|pid| {
                doc.get_object(pid)
                    .ok()?
                    .as_dict()
                    .ok()?
                    .get(b"Parent")
                    .ok()?
                    .as_reference()
                    .ok()
            })
            .collect();
        assert_eq!(parents.len(), 1, "one shared page tree");
    }

    #[test]
    fn select_pages_keeps_a_subset() {
        let three = merge(&[one_page("A"), one_page("B"), one_page("C")]).unwrap();
        let picked = select_pages(&three, &[1, 3]).expect("select");
        let doc = Document::load_mem(&picked).unwrap();
        assert_eq!(doc.get_pages().len(), 2, "kept two of three pages");
    }

    #[test]
    fn select_pages_rejects_empty_selection() {
        let one = one_page("A");
        assert!(select_pages(&one, &[9, 10]).is_err());
    }

    #[test]
    fn stamp_appends_content_and_font() {
        let one = one_page("A");
        let stamped = stamp(
            &one,
            &StampOptions {
                text: "DRAFT".into(),
                ..Default::default()
            },
        )
        .expect("stamp");
        let raw = String::from_utf8_lossy(&stamped);
        assert!(raw.contains("(DRAFT) Tj"), "stamp text drawn");
        assert!(raw.contains("/GSStamp gs"), "opacity ExtGState used");
        // Still a loadable, single-page PDF.
        let doc = Document::load_mem(&stamped).unwrap();
        assert_eq!(doc.get_pages().len(), 1);
    }
}
