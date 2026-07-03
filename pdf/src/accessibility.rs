//! Tagged-PDF post-pass (accessibility / PDF/UA groundwork).
//!
//! The backend wrapped every real content op in a `/<role> <</MCID n>> BDC … EMC`
//! sequence (role = `P`/`H1..H6`/`Figure`) and marked decoration as `/Artifact`.
//! It also hands us the resolved [`StructLeaf`] list (page, MCID, role) so this
//! pass doesn't have to re-scan the streams. From it we build:
//!
//! * `/MarkInfo <</Marked true>>` and `/Lang` on the catalog,
//! * a `StructTreeRoot` whose leaves (`H1..6 | P | Figure`) reference the page
//!   MCIDs (`/Pg` points at the page; figures carry `/Alt`), grouped into real
//!   `L › LI › LBody` and `Table › TR › TD/TH` subtrees when the leaf carries a
//!   [`StructGroup`] (header cells get a `/Scope` for PDF/UA 7.5),
//! * a `/ParentTree` number tree + per-page `/StructParents`,
//! * `/Tabs /S` (logical tab order) on every page,
//! * `/ViewerPreferences /DisplayDocTitle true` (PDF/UA 7.1),
//! * an XMP metadata stream carrying the `pdfuaid:part=1` identifier,
//! * a PDF version bump to 1.5.
//!
//! Everything is derived from the leaf list the backend produced, so emit and
//! this pass can't drift. A tagged + `pdfa` document validates as both PDF/A-3b
//! and PDF/UA-1 (see the `verapdf` CI job).

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::draw::{StructGroup, StructLeaf, StructRole};
use crate::error::{PdfError, Result};

/// An in-memory structure node, assembled from the leaf stream before it is
/// materialized into PDF `StructElem` objects.
enum Node {
    /// A content leaf owning an MCID (`P`/`H1..6`/`Figure`).
    Leaf {
        page: usize,
        mcid: u32,
        role: StructRole,
    },
    /// A grouping element (`L`/`LI`/`LBody`/`Table`/`TR`/`TD`/`TH`).
    Elem { tag: &'static str, kids: Vec<Node> },
}

fn leaf_node(leaf: &StructLeaf) -> Node {
    Node::Leaf {
        page: leaf.page,
        mcid: leaf.mcid,
        role: leaf.role.clone(),
    }
}

/// Fold the flat leaf stream into a forest: contiguous list/table runs (same
/// `seq`) become nested subtrees, everything else stays a direct leaf.
fn build_forest(leaves: &[StructLeaf]) -> Vec<Node> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < leaves.len() {
        match &leaves[i].group {
            None => {
                out.push(leaf_node(&leaves[i]));
                i += 1;
            }
            Some(StructGroup::ListItem { seq, .. }) => {
                let seq = *seq;
                let start = i;
                while i < leaves.len()
                    && matches!(&leaves[i].group, Some(StructGroup::ListItem { seq: s, .. }) if *s == seq)
                {
                    i += 1;
                }
                out.push(build_list(&leaves[start..i]));
            }
            Some(StructGroup::TableCell { seq, .. }) => {
                let seq = *seq;
                let start = i;
                while i < leaves.len()
                    && matches!(&leaves[i].group, Some(StructGroup::TableCell { seq: s, .. }) if *s == seq)
                {
                    i += 1;
                }
                out.push(build_table(&leaves[start..i]));
            }
        }
    }
    out
}

fn build_list(run: &[StructLeaf]) -> Node {
    let item_of = |l: &StructLeaf| match &l.group {
        Some(StructGroup::ListItem { item, .. }) => *item,
        _ => 0,
    };
    let mut items = Vec::new();
    let mut j = 0;
    while j < run.len() {
        let item = item_of(&run[j]);
        let start = j;
        while j < run.len() && item_of(&run[j]) == item {
            j += 1;
        }
        let lbody = Node::Elem {
            tag: "LBody",
            kids: run[start..j].iter().map(leaf_node).collect(),
        };
        items.push(Node::Elem {
            tag: "LI",
            kids: vec![lbody],
        });
    }
    Node::Elem {
        tag: "L",
        kids: items,
    }
}

fn build_table(run: &[StructLeaf]) -> Node {
    let row_of = |l: &StructLeaf| match &l.group {
        Some(StructGroup::TableCell { row, .. }) => *row,
        _ => 0,
    };
    let col_of = |l: &StructLeaf| match &l.group {
        Some(StructGroup::TableCell { col, .. }) => *col,
        _ => 0,
    };
    let is_header =
        |l: &StructLeaf| matches!(&l.group, Some(StructGroup::TableCell { header: true, .. }));
    let mut rows = Vec::new();
    let mut j = 0;
    while j < run.len() {
        let row = row_of(&run[j]);
        let row_start = j;
        while j < run.len() && row_of(&run[j]) == row {
            j += 1;
        }
        let mut cells = Vec::new();
        let mut k = row_start;
        while k < j {
            let col = col_of(&run[k]);
            let header = is_header(&run[k]);
            let cell_start = k;
            while k < j && col_of(&run[k]) == col {
                k += 1;
            }
            cells.push(Node::Elem {
                tag: if header { "TH" } else { "TD" },
                kids: run[cell_start..k].iter().map(leaf_node).collect(),
            });
        }
        rows.push(Node::Elem {
            tag: "TR",
            kids: cells,
        });
    }
    Node::Elem {
        tag: "Table",
        kids: rows,
    }
}

/// Recursively write a [`Node`] to a `StructElem` object, returning its id.
/// Leaves are recorded in `leaf_elems` (page, mcid, id) for the ParentTree.
fn materialize(
    node: &Node,
    parent: ObjectId,
    pages: &[ObjectId],
    doc: &mut Document,
    leaf_elems: &mut Vec<(usize, u32, ObjectId)>,
) -> Option<ObjectId> {
    match node {
        Node::Leaf { page, mcid, role } => {
            let page_id = *pages.get(*page)?; // defensive: a leaf past the page count
            let id = doc.new_object_id();
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"StructElem".to_vec()));
            d.set("S", Object::Name(role.tag().into_bytes()));
            d.set("Pg", Object::Reference(page_id));
            d.set("K", Object::Integer(*mcid as i64));
            d.set("P", Object::Reference(parent));
            if let StructRole::Figure { alt: Some(alt) } = role {
                d.set(
                    "Alt",
                    Object::String(alt.as_bytes().to_vec(), StringFormat::Literal),
                );
            }
            doc.set_object(id, d);
            leaf_elems.push((*page, *mcid, id));
            Some(id)
        }
        Node::Elem { tag, kids } => {
            let id = doc.new_object_id();
            let kid_ids: Vec<Object> = kids
                .iter()
                .filter_map(|k| materialize(k, id, pages, doc, leaf_elems))
                .map(Object::Reference)
                .collect();
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"StructElem".to_vec()));
            d.set("S", Object::Name(tag.as_bytes().to_vec()));
            d.set("P", Object::Reference(parent));
            d.set("K", Object::Array(kid_ids));
            // PDF/UA (ISO 14289-1 7.5): a header cell needs a Scope so assistive
            // tech can associate it with its column (all our headers are the
            // top row → column headers).
            if *tag == "TH" {
                let mut attr = Dictionary::new();
                attr.set("O", Object::Name(b"Table".to_vec()));
                attr.set("Scope", Object::Name(b"Column".to_vec()));
                d.set("A", Object::Dictionary(attr));
            }
            doc.set_object(id, d);
            Some(id)
        }
    }
}

/// Add the tagging structure to an already-marked PDF, driven by the structure
/// leaves the backend emitted.
pub fn apply_tags(pdf: &[u8], lang: Option<&str>, leaves: &[StructLeaf]) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("tagging: could not parse the render: {e}")))?;

    let pages: Vec<ObjectId> = doc.page_iter().collect();

    let struct_root_id = doc.new_object_id();
    let document_elem_id = doc.new_object_id();

    // Group the flat leaf stream into a nested forest: ungrouped leaves are
    // direct children of /Document; runs of list/table leaves become real
    // `L › LI › LBody` and `Table › TR › TD/TH` subtrees.
    let forest = build_forest(leaves);

    // Materialize the forest into StructElem objects under /Document, recording
    // each MCID-owning leaf per page for the ParentTree.
    let mut leaf_elems: Vec<(usize, u32, ObjectId)> = Vec::new();
    let mut doc_kids: Vec<Object> = Vec::new();
    for node in &forest {
        if let Some(id) = materialize(node, document_elem_id, &pages, &mut doc, &mut leaf_elems) {
            doc_kids.push(Object::Reference(id));
        }
    }

    // The `/Document` element groups the forest roots in reading order.
    let mut document_elem = Dictionary::new();
    document_elem.set("Type", Object::Name(b"StructElem".to_vec()));
    document_elem.set("S", Object::Name(b"Document".to_vec()));
    document_elem.set("P", Object::Reference(struct_root_id));
    document_elem.set("K", Object::Array(doc_kids));
    doc.set_object(document_elem_id, document_elem);

    // ParentTree: per page, an array indexed by MCID → the leaf StructElem that
    // owns it. Every page gets a key and `/Tabs /S`.
    let mut nums: Vec<Object> = Vec::new();
    for (page_idx, page_id) in pages.iter().enumerate() {
        let mut per_page: Vec<(u32, ObjectId)> = leaf_elems
            .iter()
            .filter(|(p, _, _)| *p == page_idx)
            .map(|(_, mcid, id)| (*mcid, *id))
            .collect();
        per_page.sort_by_key(|(mcid, _)| *mcid);
        let arr: Vec<Object> = per_page
            .into_iter()
            .map(|(_, id)| Object::Reference(id))
            .collect();
        let arr_id = doc.add_object(Object::Array(arr));
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
