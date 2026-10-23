use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::error::{PdfError, Result};

/// Emit an open outline tree in document order. Page indices are zero-based;
/// levels select the outline depth.
pub(super) fn append(doc: &mut Document, bookmarks: &[(String, usize, u32)]) -> Result<()> {
    if bookmarks.is_empty() {
        return Ok(());
    }
    let pages: Vec<_> = doc.page_iter().collect();
    let root = doc.new_object_id();
    let mut nodes = Vec::new();
    let mut stack = Vec::new();
    let mut children: BTreeMap<ObjectId, Vec<usize>> = BTreeMap::new();
    let mut counts: BTreeMap<ObjectId, i64> = BTreeMap::new();
    for (label, page, level) in bookmarks {
        let Some(&page_id) = pages.get(*page) else {
            continue;
        };
        stack.truncate(level.saturating_sub(1) as usize);
        let parent = stack.last().copied().unwrap_or(root);
        let id = doc.new_object_id();
        children.entry(parent).or_default().push(nodes.len());
        for ancestor in &stack {
            *counts.entry(*ancestor).or_default() += 1;
        }
        nodes.push((id, label, page_id));
        stack.push(id);
    }
    for (&parent, siblings) in &children {
        for (pos, &index) in siblings.iter().enumerate() {
            let (id, label, page_id) = nodes[index];
            let mut title = vec![0xfe, 0xff];
            title.extend(label.encode_utf16().flat_map(u16::to_be_bytes));
            let mut entry = Dictionary::new();
            entry.set("Parent", parent);
            entry.set("Title", Object::string_literal(title));
            entry.set(
                "Dest",
                vec![
                    Object::Reference(page_id),
                    Object::Name(b"XYZ".to_vec()),
                    Object::Null,
                    Object::Null,
                    Object::Null,
                ],
            );
            if let Some(&prev) = pos.checked_sub(1).and_then(|p| siblings.get(p)) {
                entry.set("Prev", nodes[prev].0);
            }
            if let Some(&next) = siblings.get(pos + 1) {
                entry.set("Next", nodes[next].0);
            }
            if let Some(kids) = children.get(&id) {
                entry.set("First", nodes[kids[0]].0);
                entry.set("Last", nodes[kids[kids.len() - 1]].0);
                entry.set("Count", counts[&id]);
            }
            doc.set_object(id, entry);
        }
    }
    let mut outline = Dictionary::new();
    outline.set("Type", Object::Name(b"Outlines".to_vec()));
    outline.set("Count", nodes.len() as i64);
    if let Some(roots) = children.get(&root) {
        outline.set("First", nodes[roots[0]].0);
        outline.set("Last", nodes[roots[roots.len() - 1]].0);
    }
    doc.set_object(root, outline);
    let catalog = doc
        .catalog_mut()
        .map_err(|e| PdfError::Backend(format!("PDF outline catalog: {e}")))?;
    catalog.set("Outlines", root);
    catalog.set("PageMode", Object::Name(b"UseOutlines".to_vec()));
    Ok(())
}
