//! Tagged-PDF post-pass (accessibility baseline / PDF/UA groundwork).
//!
//! The backend already wrapped each text run in `/P <</MCID n>> BDC … EMC` and
//! marked decorations as `/Artifact`. This pass reads those MCIDs back out of
//! each page's content stream and builds the document structure:
//!
//! * `/MarkInfo <</Marked true>>` and `/Lang` on the catalog,
//! * a `StructTreeRoot` with a flat `Document → P…` tree whose `/K` reference
//!   the page MCIDs (`/Pg` points at the page),
//! * a `/ParentTree` number tree + per-page `/StructParents`,
//! * `/Tabs /S` (logical tab order) on every page,
//! * a PDF version bump to 1.5.
//!
//! Scope: this is the *structural* baseline. Full PDF/UA also needs real
//! structure types (H1/L/Table/Figure with alt text) and a UA XMP identifier —
//! see the roadmap. Everything here is derived from the stream, so emit and
//! this pass can't drift.

use lopdf::{Dictionary, Document, Object, ObjectId, StringFormat};

use crate::error::{PdfError, Result};

/// Add the tagging structure to an already-marked PDF.
pub fn apply_tags(pdf: &[u8], lang: Option<&str>) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("tagging: could not parse the render: {e}")))?;

    let pages: Vec<ObjectId> = doc.page_iter().collect();

    // How many MCIDs each page carries (0..n), read from its content stream.
    let mcid_counts: Vec<usize> = pages
        .iter()
        .map(|pid| count_mcids(&doc, *pid))
        .collect::<Vec<_>>();

    let struct_root_id = doc.new_object_id();

    // One structure element (`/P`) per MCID, in page then MCID order. The
    // element's index in this flat list is its ParentTree key.
    let mut struct_elems: Vec<(ObjectId, ObjectId, i64)> = Vec::new(); // (elem, page, mcid)
    for (page_idx, &count) in mcid_counts.iter().enumerate() {
        for mcid in 0..count {
            struct_elems.push((doc.new_object_id(), pages[page_idx], mcid as i64));
        }
    }

    // Emit each `/P` element.
    let kids: Vec<Object> = struct_elems
        .iter()
        .map(|(elem_id, page_id, mcid)| {
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"StructElem".to_vec()));
            d.set("S", Object::Name(b"P".to_vec()));
            d.set("P", Object::Reference(struct_root_id));
            d.set("Pg", Object::Reference(*page_id));
            d.set("K", Object::Integer(*mcid));
            doc.set_object(*elem_id, d);
            Object::Reference(*elem_id)
        })
        .collect();

    // The `/Document` element groups all the paragraphs.
    let document_elem_id = doc.new_object_id();
    let mut document_elem = Dictionary::new();
    document_elem.set("Type", Object::Name(b"StructElem".to_vec()));
    document_elem.set("S", Object::Name(b"Document".to_vec()));
    document_elem.set("P", Object::Reference(struct_root_id));
    document_elem.set("K", Object::Array(kids));
    doc.set_object(document_elem_id, document_elem);

    // Re-parent each `/P` under `/Document`.
    for (elem_id, _, _) in &struct_elems {
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(*elem_id) {
            d.set("P", Object::Reference(document_elem_id));
        }
    }

    // ParentTree: a number tree mapping each page's StructParents index to the
    // array of structure elements whose MCIDs live on that page.
    let mut nums: Vec<Object> = Vec::new();
    let mut cursor = 0usize;
    for (page_idx, &count) in mcid_counts.iter().enumerate() {
        let page_elems: Vec<Object> = struct_elems[cursor..cursor + count]
            .iter()
            .map(|(id, _, _)| Object::Reference(*id))
            .collect();
        cursor += count;
        let arr_id = doc.add_object(Object::Array(page_elems));
        nums.push(Object::Integer(page_idx as i64));
        nums.push(Object::Reference(arr_id));

        // Wire the page: /StructParents (its ParentTree key), /Tabs /S.
        if let Ok(Object::Dictionary(page)) = doc.get_object_mut(pages[page_idx]) {
            page.set("StructParents", Object::Integer(page_idx as i64));
            page.set("Tabs", Object::Name(b"S".to_vec()));
        }
    }
    let mut parent_tree = Dictionary::new();
    parent_tree.set("Nums", Object::Array(nums));
    let parent_tree_id = doc.add_object(Object::Dictionary(parent_tree));

    // StructTreeRoot.
    let mut root = Dictionary::new();
    root.set("Type", Object::Name(b"StructTreeRoot".to_vec()));
    root.set("K", Object::Reference(document_elem_id));
    root.set("ParentTree", Object::Reference(parent_tree_id));
    root.set(
        "ParentTreeNextKey",
        Object::Integer(mcid_counts.len() as i64),
    );
    doc.set_object(struct_root_id, root);

    // Catalog: /MarkInfo, /StructTreeRoot, /Lang.
    let catalog = doc
        .catalog_mut()
        .map_err(|e| PdfError::Backend(format!("tagging: no catalog: {e}")))?;
    let mut mark_info = Dictionary::new();
    mark_info.set("Marked", Object::Boolean(true));
    catalog.set("MarkInfo", Object::Dictionary(mark_info));
    catalog.set("StructTreeRoot", Object::Reference(struct_root_id));
    catalog.set(
        "Lang",
        Object::String(
            lang.unwrap_or("en-US").as_bytes().to_vec(),
            StringFormat::Literal,
        ),
    );

    // Tagged PDF is a 1.4+ feature; printpdf writes a 1.3 header.
    doc.version = "1.5".to_string();

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("tagging: could not save: {e}")))?;
    Ok(out)
}

/// Count `/MCID n` occurrences in a page's (decompressed) content stream.
/// MCIDs are emitted 0..n per page by the backend, so the count is the number
/// of tagged text runs on the page.
fn count_mcids(doc: &Document, page_id: ObjectId) -> usize {
    let content = match doc.get_page_content(page_id) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let text = String::from_utf8_lossy(&content);
    text.matches("/MCID").count()
}
