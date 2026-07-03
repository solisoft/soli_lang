//! Tagged-PDF post-pass (accessibility / PDF/UA groundwork).
//!
//! The backend wrapped every real content op in a `/<role> <</MCID n>> BDC … EMC`
//! sequence (role = `P`/`H1..H6`/`Figure`) and marked decoration as `/Artifact`.
//! It also hands us the resolved [`StructLeaf`] list (page, MCID, role) so this
//! pass doesn't have to re-scan the streams. From it we build:
//!
//! * `/MarkInfo <</Marked true>>` and `/Lang` on the catalog,
//! * a `StructTreeRoot` with a `Document → {H1..6 | P | Figure}` tree whose `/K`
//!   reference the page MCIDs (`/Pg` points at the page; figures carry `/Alt`),
//! * a `/ParentTree` number tree + per-page `/StructParents`,
//! * `/Tabs /S` (logical tab order) on every page,
//! * an XMP metadata stream carrying the `pdfuaid:part=1` identifier,
//! * a PDF version bump to 1.5.
//!
//! Scope: real heading/paragraph/figure semantics. Lists and tables are tagged
//! as paragraphs for now (readable, but not `L`/`Table` structured) — that's the
//! remaining PDF/UA mapping. Everything is derived from the leaf list the backend
//! produced, so emit and this pass can't drift.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::draw::{StructLeaf, StructRole};
use crate::error::{PdfError, Result};

/// Add the tagging structure to an already-marked PDF, driven by the structure
/// leaves the backend emitted.
pub fn apply_tags(pdf: &[u8], lang: Option<&str>, leaves: &[StructLeaf]) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("tagging: could not parse the render: {e}")))?;

    let pages: Vec<ObjectId> = doc.page_iter().collect();

    let struct_root_id = doc.new_object_id();

    // One StructElem per leaf, in the leaves' order (page, then MCID). The
    // element's page is recorded so the ParentTree can be grouped per page.
    let mut elems: Vec<(ObjectId, usize)> = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let page_id = match pages.get(leaf.page) {
            Some(id) => *id,
            None => continue, // defensive: a leaf past the page count
        };
        let elem_id = doc.new_object_id();
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"StructElem".to_vec()));
        d.set("S", Object::Name(leaf.role.tag().into_bytes()));
        d.set("Pg", Object::Reference(page_id));
        d.set("K", Object::Integer(leaf.mcid as i64));
        // A figure's alt text is required for UA; carry it when present.
        if let StructRole::Figure { alt: Some(alt) } = &leaf.role {
            d.set(
                "Alt",
                Object::String(alt.as_bytes().to_vec(), StringFormat::Literal),
            );
        }
        doc.set_object(elem_id, d);
        elems.push((elem_id, leaf.page));
    }

    // The `/Document` element groups every leaf in reading order.
    let document_elem_id = doc.new_object_id();
    let kids: Vec<Object> = elems.iter().map(|(id, _)| Object::Reference(*id)).collect();
    let mut document_elem = Dictionary::new();
    document_elem.set("Type", Object::Name(b"StructElem".to_vec()));
    document_elem.set("S", Object::Name(b"Document".to_vec()));
    document_elem.set("P", Object::Reference(struct_root_id));
    document_elem.set("K", Object::Array(kids));
    doc.set_object(document_elem_id, document_elem);

    // Parent every leaf under `/Document`.
    for (elem_id, _) in &elems {
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(*elem_id) {
            d.set("P", Object::Reference(document_elem_id));
        }
    }

    // ParentTree: map each page's StructParents key (its index) to the array of
    // structure elements whose MCIDs live on that page. Every page gets a key
    // and `/Tabs /S`, even if it holds only artifacts (empty array).
    let mut nums: Vec<Object> = Vec::new();
    for (page_idx, page_id) in pages.iter().enumerate() {
        let page_elems: Vec<Object> = elems
            .iter()
            .filter(|(_, p)| *p == page_idx)
            .map(|(id, _)| Object::Reference(*id))
            .collect();
        let arr_id = doc.add_object(Object::Array(page_elems));
        nums.push(Object::Integer(page_idx as i64));
        nums.push(Object::Reference(arr_id));
        if let Ok(Object::Dictionary(page)) = doc.get_object_mut(*page_id) {
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
    root.set("ParentTreeNextKey", Object::Integer(pages.len() as i64));
    doc.set_object(struct_root_id, root);

    // XMP metadata carrying the PDF/UA identifier (uncompressed — validators read
    // the packet as plaintext).
    let lang = lang.unwrap_or("en-US");
    let metadata_id = doc.add_object(ua_metadata_stream(lang));

    // Catalog: /MarkInfo, /StructTreeRoot, /Lang, /Metadata.
    let catalog = doc
        .catalog_mut()
        .map_err(|e| PdfError::Backend(format!("tagging: no catalog: {e}")))?;
    let mut mark_info = Dictionary::new();
    mark_info.set("Marked", Object::Boolean(true));
    catalog.set("MarkInfo", Object::Dictionary(mark_info));
    catalog.set("StructTreeRoot", Object::Reference(struct_root_id));
    catalog.set(
        "Lang",
        Object::String(lang.as_bytes().to_vec(), StringFormat::Literal),
    );
    catalog.set("Metadata", Object::Reference(metadata_id));
    // PDF/UA (ISO 14289-1 7.1 t10): the reader must show the document title,
    // not the file name.
    let mut view_prefs = catalog
        .get(b"ViewerPreferences")
        .ok()
        .and_then(|o| o.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    view_prefs.set("DisplayDocTitle", Object::Boolean(true));
    catalog.set("ViewerPreferences", Object::Dictionary(view_prefs));

    // Tagged PDF is a 1.4+ feature; printpdf writes a 1.3 header.
    doc.version = "1.5".to_string();

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("tagging: could not save: {e}")))?;
    Ok(out)
}

/// A minimal XMP packet declaring PDF/UA part 1 plus the document language.
fn ua_metadata_stream(lang: &str) -> Stream {
    let xmp = format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\" xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\">\n\
         <pdfuaid:part>1</pdfuaid:part>\n\
         </rdf:Description>\n\
         <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
         <dc:language><rdf:Bag><rdf:li>{lang}</rdf:li></rdf:Bag></dc:language>\n\
         </rdf:Description>\n\
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>"
    );
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Metadata".to_vec()));
    dict.set("Subtype", Object::Name(b"XML".to_vec()));
    let mut stream = Stream::new(dict, xmp.into_bytes());
    // XMP must not be flate-compressed.
    stream.allows_compression = false;
    stream
}
