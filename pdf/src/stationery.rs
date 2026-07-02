//! Letterhead / stationery underlay: draw rendered content **on top of an
//! existing PDF** (a company letterhead) instead of rebuilding the letterhead
//! from rects and images.
//!
//! Implemented as a lopdf post-pass over the emitted bytes (like the Factur-X
//! embedding step): each used letterhead page is imported into the document as
//! a **Form XObject** — content stream plus a deep copy of its resources — and
//! drawn *beneath* every page's own content by prepending a balanced
//! `q <scale> cm /SoliLHn Do Q` stream to the page's `Contents`.
//!
//! Page mapping follows the word-processor convention: the document's first
//! page uses letterhead page 1; every following page uses letterhead page 2
//! when it exists, else page 1 (a "first page different" letterhead just works,
//! a single-page letterhead repeats).
//!
//! Caveats (documented): the letterhead is scaled to the target page size
//! (stretched if the aspect ratios differ), a template `options.background`
//! fill paints *over* it, and Factur-X/PDF-A conformance is only as good as
//! the letterhead's own content (e.g. transparency groups may violate PDF/A).

use std::collections::HashMap;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::error::{PdfError, Result};

/// Draw `letterhead` (a PDF) beneath every page of `pdf`, returning the new
/// document bytes.
pub fn apply_stationery(pdf: &[u8], letterhead: &[u8]) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("stationery: could not parse the render: {e}")))?;
    let lh = Document::load_mem(letterhead).map_err(|e| {
        PdfError::Backend(format!(
            "stationery: could not parse the letterhead PDF: {e}"
        ))
    })?;

    let lh_pages: Vec<ObjectId> = lh.page_iter().collect();
    if lh_pages.is_empty() {
        return Err(PdfError::Backend(
            "stationery: the letterhead PDF has no pages".to_string(),
        ));
    }

    let target_pages: Vec<ObjectId> = doc.page_iter().collect();
    // Imported Form XObjects, one per used letterhead page index:
    // (xobject id, name, letterhead media box).
    let mut forms: HashMap<usize, (ObjectId, String, [f32; 4])> = HashMap::new();
    let mut map: HashMap<ObjectId, ObjectId> = HashMap::new();

    for (i, page_id) in target_pages.iter().enumerate() {
        // First page → letterhead page 1; later pages → letterhead page 2 when
        // present, else page 1.
        let lh_index = if i == 0 { 0 } else { 1.min(lh_pages.len() - 1) };
        if let std::collections::hash_map::Entry::Vacant(slot) = forms.entry(lh_index) {
            let (form_id, bbox) = import_page_as_form(&lh, &mut doc, lh_pages[lh_index], &mut map)?;
            slot.insert((form_id, format!("SoliLH{lh_index}"), bbox));
        }
        let (form_id, name, lh_box) = forms[&lh_index].clone();

        let target_box = inherited_media_box(&doc, *page_id).unwrap_or([0.0, 0.0, 595.0, 842.0]);
        let sx = (target_box[2] - target_box[0]) / (lh_box[2] - lh_box[0]).max(1.0);
        let sy = (target_box[3] - target_box[1]) / (lh_box[3] - lh_box[1]).max(1.0);

        // The underlay stream: balanced q/Q so the page's own content starts
        // from a clean graphics state.
        let underlay = format!("q {sx:.4} 0 0 {sy:.4} 0 0 cm /{name} Do Q");
        let underlay_id = doc.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            underlay.into_bytes(),
        )));

        prepend_content(&mut doc, *page_id, underlay_id)?;
        register_xobject(&mut doc, *page_id, &name, form_id)?;
    }

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("stationery: could not save: {e}")))?;
    Ok(out)
}

/// Import one letterhead page into `dst` as a Form XObject (content +
/// deep-copied resources). Returns the new object's id and the page's media
/// box (the form's BBox).
fn import_page_as_form(
    src: &Document,
    dst: &mut Document,
    page_id: ObjectId,
    map: &mut HashMap<ObjectId, ObjectId>,
) -> Result<(ObjectId, [f32; 4])> {
    let content = src.get_page_content(page_id).map_err(|e| {
        PdfError::Backend(format!(
            "stationery: could not read letterhead content: {e}"
        ))
    })?;
    let bbox = inherited_media_box(src, page_id).unwrap_or([0.0, 0.0, 595.0, 842.0]);

    // Resources: the page's own dict if inline, else the inherited chain's
    // referenced dicts merged shallowly (later = closer to the page, wins).
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
    let resources = import_object(src, dst, &Object::Dictionary(resources), map);

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Form".to_vec()));
    dict.set("FormType", Object::Integer(1));
    dict.set(
        "BBox",
        Object::Array(bbox.iter().map(|v| Object::Real(*v)).collect()),
    );
    dict.set("Resources", resources);

    let mut stream = Stream::new(dict, content);
    // Keep the underlay small; failure just leaves it uncompressed.
    let _ = stream.compress();
    Ok((dst.add_object(Object::Stream(stream)), bbox))
}

/// Recursively copy an object graph from `src` into `dst`, renumbering
/// references. `map` carries already-imported ids (reserved before recursion,
/// so reference cycles terminate).
fn import_object(
    src: &Document,
    dst: &mut Document,
    obj: &Object,
    map: &mut HashMap<ObjectId, ObjectId>,
) -> Object {
    match obj {
        Object::Reference(id) => {
            if let Some(mapped) = map.get(id) {
                return Object::Reference(*mapped);
            }
            dst.max_id += 1;
            let new_id = (dst.max_id, 0);
            map.insert(*id, new_id);
            let resolved = match src.get_object(*id) {
                Ok(o) => o.clone(),
                Err(_) => Object::Null,
            };
            let copied = import_object(src, dst, &resolved, map);
            dst.objects.insert(new_id, copied);
            Object::Reference(new_id)
        }
        Object::Dictionary(d) => {
            let mut nd = Dictionary::new();
            for (k, v) in d.iter() {
                nd.set(k.clone(), import_object(src, dst, v, map));
            }
            Object::Dictionary(nd)
        }
        Object::Array(a) => {
            Object::Array(a.iter().map(|v| import_object(src, dst, v, map)).collect())
        }
        Object::Stream(s) => {
            // Raw content is copied verbatim; the imported dict keeps the
            // original Filter/Length so the bytes stay valid as-is.
            let dict = match import_object(src, dst, &Object::Dictionary(s.dict.clone()), map) {
                Object::Dictionary(d) => d,
                _ => unreachable!("dictionary import returns a dictionary"),
            };
            let mut ns = Stream::new(Dictionary::new(), s.content.clone());
            ns.dict = dict;
            Object::Stream(ns)
        }
        other => other.clone(),
    }
}

/// Prepend `stream_id` to the page's `Contents` (normalizing a single
/// reference into an array).
fn prepend_content(doc: &mut Document, page_id: ObjectId, stream_id: ObjectId) -> Result<()> {
    let page = doc
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| PdfError::Backend(format!("stationery: bad page object: {e}")))?;
    let mut list = match page.get(b"Contents") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => vec![Object::Reference(*r)],
        _ => Vec::new(),
    };
    list.insert(0, Object::Reference(stream_id));
    page.set("Contents", Object::Array(list));
    Ok(())
}

/// Register `form_id` under `name` in the page's `Resources → XObject` dict,
/// following references and creating missing dicts inline.
fn register_xobject(
    doc: &mut Document,
    page_id: ObjectId,
    name: &str,
    form_id: ObjectId,
) -> Result<()> {
    // Locate where the Resources dict lives (inline on the page vs referenced).
    let resources_ref = {
        let page = doc
            .get_object(page_id)
            .and_then(Object::as_dict)
            .map_err(|e| PdfError::Backend(format!("stationery: bad page object: {e}")))?;
        match page.get(b"Resources") {
            Ok(Object::Reference(r)) => Some(*r),
            _ => None,
        }
    };

    let resources: &mut Dictionary = match resources_ref {
        Some(rid) => doc
            .get_object_mut(rid)
            .and_then(Object::as_dict_mut)
            .map_err(|e| PdfError::Backend(format!("stationery: bad Resources object: {e}")))?,
        None => {
            let page = doc
                .get_object_mut(page_id)
                .and_then(Object::as_dict_mut)
                .map_err(|e| PdfError::Backend(format!("stationery: bad page object: {e}")))?;
            if !matches!(page.get(b"Resources"), Ok(Object::Dictionary(_))) {
                page.set("Resources", Object::Dictionary(Dictionary::new()));
            }
            match page.get_mut(b"Resources") {
                Ok(Object::Dictionary(d)) => d,
                _ => unreachable!("Resources was just normalized to a dictionary"),
            }
        }
    };

    // XObject sub-dict: inline it if missing; follow a reference if present.
    match resources.get(b"XObject") {
        Ok(Object::Reference(r)) => {
            let r = *r;
            let xd = doc
                .get_object_mut(r)
                .and_then(Object::as_dict_mut)
                .map_err(|e| PdfError::Backend(format!("stationery: bad XObject dict: {e}")))?;
            xd.set(name, Object::Reference(form_id));
        }
        Ok(Object::Dictionary(_)) => {
            if let Ok(Object::Dictionary(xd)) = resources.get_mut(b"XObject") {
                xd.set(name, Object::Reference(form_id));
            }
        }
        _ => {
            let mut xd = Dictionary::new();
            xd.set(name, Object::Reference(form_id));
            resources.set("XObject", Object::Dictionary(xd));
        }
    }
    Ok(())
}

/// A page's media box, walking the `Parent` chain for the inherited value.
fn inherited_media_box(doc: &Document, page_id: ObjectId) -> Option<[f32; 4]> {
    let mut dict_id = page_id;
    let mut hops = 0;
    loop {
        let dict = doc.get_dictionary(dict_id).ok()?;
        if let Ok(mb) = dict.get(b"MediaBox") {
            let mb = match mb {
                Object::Reference(r) => doc.get_object(*r).ok()?,
                other => other,
            };
            if let Object::Array(a) = mb {
                if a.len() == 4 {
                    let num = |o: &Object| -> Option<f32> {
                        match o {
                            Object::Integer(i) => Some(*i as f32),
                            Object::Real(r) => Some(*r),
                            _ => None,
                        }
                    };
                    return Some([num(&a[0])?, num(&a[1])?, num(&a[2])?, num(&a[3])?]);
                }
            }
        }
        dict_id = dict.get(b"Parent").and_then(Object::as_reference).ok()?;
        hops += 1;
        if hops > 64 {
            return None; // defensive: malformed Parent cycle
        }
    }
}
