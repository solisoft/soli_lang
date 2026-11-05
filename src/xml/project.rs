//! Projections of a tree into plain data: `Proj`.
//!
//! A `Proj` is what `to_h` and `SOAP.parse` return, built without any
//! interpreter `Value`, so `SOAP.call`'s worker thread can build one and send
//! it back. The binding converts it to a `Value`; [`Proj::to_json_string`]
//! writes it as JSON with key order kept.
//!
//! Two shapes:
//! - [`to_h`] (§1.9 of the design): `{qname => value}`; attributes under
//!   `"@name"`, children by qname (repeats become a list), non-blank direct
//!   text joined and trimmed under `"_text"`; a leaf is its trimmed text or
//!   `nil`.
//! - [`soap_shape`]: today's `SOAP.parse` output, with its six bugs fixed
//!   (whitespace around entities kept, CDATA kept, self-closing repeats
//!   appended, a self-closing root gives `{"a" => nil}`, mixed-content runs
//!   joined then trimmed, truncated input refused by the parser) and CRLF
//!   normalised. Attributes are dropped, as today.
//!
//! Both are iterative: a built tree may be deeper than any parse limit.

use std::collections::HashMap;

use indexmap::IndexMap;

use super::error::{XmlError, XmlErrorKind};
use super::tree::{Kind, NodeId, XmlTree};

/// Plain data: `nil`, a String, an ordered Hash or an Array.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Proj {
    #[default]
    Null,
    Str(String),
    Map(IndexMap<String, Proj>),
    List(Vec<Proj>),
}

/// Dropping is iterative: a projection of a deep built tree is as deep as the
/// tree, and the derived drop would recurse once per level. (Because of this
/// impl, take a value out with `std::mem::take` rather than by moving out of a
/// pattern.)
impl Drop for Proj {
    fn drop(&mut self) {
        let mut stack: Vec<Proj> = Vec::new();
        match self {
            Proj::Map(map) => stack.extend(map.drain(..).map(|(_, v)| v)),
            Proj::List(items) => stack.append(items),
            _ => return,
        }
        while let Some(mut value) = stack.pop() {
            match &mut value {
                Proj::Map(map) => stack.extend(map.drain(..).map(|(_, v)| v)),
                Proj::List(items) => stack.append(items),
                _ => {}
            }
            // `value` now holds no children: its own drop does not recurse.
        }
    }
}

impl Proj {
    /// JSON text, keys in order. Iterative.
    pub fn to_json_string(&self) -> String {
        enum Item<'a> {
            Value(&'a Proj),
            Raw(&'static str),
            Key(&'a str),
        }
        let mut out = String::new();
        let mut stack = vec![Item::Value(self)];
        while let Some(item) = stack.pop() {
            match item {
                Item::Raw(text) => out.push_str(text),
                Item::Key(key) => {
                    out.push_str(&serde_json::Value::String(key.to_string()).to_string());
                    out.push(':');
                }
                Item::Value(Proj::Null) => out.push_str("null"),
                Item::Value(Proj::Str(s)) => {
                    out.push_str(&serde_json::Value::String(s.clone()).to_string())
                }
                Item::Value(Proj::List(items)) => {
                    out.push('[');
                    stack.push(Item::Raw("]"));
                    for (i, value) in items.iter().enumerate().rev() {
                        stack.push(Item::Value(value));
                        if i > 0 {
                            stack.push(Item::Raw(","));
                        }
                    }
                }
                Item::Value(Proj::Map(map)) => {
                    out.push('{');
                    stack.push(Item::Raw("}"));
                    for (i, (key, value)) in map.iter().enumerate().rev() {
                        stack.push(Item::Value(value));
                        stack.push(Item::Key(key));
                        if i > 0 {
                            stack.push(Item::Raw(","));
                        }
                    }
                }
            }
        }
        out
    }
}

/// Add `value` under `key`; a repeated key turns into a list.
fn insert_repeat(map: &mut IndexMap<String, Proj>, key: &str, value: Proj) {
    match map.get_mut(key) {
        Some(Proj::List(items)) => items.push(value),
        Some(existing) => {
            let first = std::mem::replace(existing, Proj::Null);
            *existing = Proj::List(vec![first, value]);
        }
        None => {
            map.insert(key.to_string(), value);
        }
    }
}

/// Elements of `id`'s subtree (itself included), children before parents.
fn elements_bottom_up(tree: &XmlTree, id: NodeId) -> Vec<NodeId> {
    let mut order = vec![id];
    order.extend(tree.descendants(id));
    order.reverse();
    order
}

/// Compute a value per element, bottom-up, with `value_of` seeing its
/// children's values already computed (it removes them from the map).
fn project<F>(tree: &XmlTree, id: NodeId, mut value_of: F) -> Proj
where
    F: FnMut(NodeId, &mut HashMap<NodeId, Proj>) -> Proj,
{
    let mut values: HashMap<NodeId, Proj> = HashMap::new();
    for node in elements_bottom_up(tree, id) {
        if tree.is_element(node) {
            let value = value_of(node, &mut values);
            values.insert(node, value);
        }
    }
    values.remove(&id).unwrap_or(Proj::Null)
}

fn trimmed_or_null(text: &str) -> Proj {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Proj::Null
    } else {
        Proj::Str(trimmed.to_string())
    }
}

/// `node.to_h`: `{qname => value}` for an element, the root's for the
/// document node, `nil` for anything else (or a document with no root).
pub fn to_h(tree: &XmlTree, id: NodeId) -> Proj {
    let id = if id == NodeId::DOCUMENT {
        match tree.root_element() {
            Some(root) => root,
            None => return Proj::Null,
        }
    } else {
        id
    };
    let Some(el) = tree.element(id) else {
        return Proj::Null;
    };
    let value = project(tree, id, |node, values| {
        let Some(el) = tree.element(node) else {
            return Proj::Null;
        };
        let has_child_element = tree.element_children(node).next().is_some();
        if el.attrs.is_empty() && !has_child_element {
            return trimmed_or_null(&tree.own_text(node));
        }
        let mut map = IndexMap::new();
        for attr in el.attrs {
            map.insert(
                format!("@{}", attr.qname),
                Proj::Str(attr.value.to_string()),
            );
        }
        for child in tree.element_children(node) {
            let name = tree.name(child).unwrap_or_default();
            let value = values.remove(&child).unwrap_or(Proj::Null);
            insert_repeat(&mut map, name, value);
        }
        let text = tree.own_text(node);
        let text = text.trim();
        if !text.is_empty() {
            map.insert("_text".to_string(), Proj::Str(text.to_string()));
        }
        Proj::Map(map)
    });
    let mut out = IndexMap::new();
    out.insert(el.qname.to_string(), value);
    Proj::Map(out)
}

/// `SOAP.parse`'s shape for a tree parsed with `ParsePolicy::soap()`:
/// `{root_qname => value}`, or `nil` when there is no root.
///
/// Rules kept from the old parser: attributes are dropped; qnames are kept as
/// written; text is trimmed and stays a String; an element with nothing in it
/// is `nil`; a repeated name gives a list; text beside children goes under
/// `"_text"`, at the position where the old parser first flushed it (before
/// the next non-self-closing child, or last); comments and PIs are skipped; a
/// child named `_text` stays ambiguous.
pub fn soap_shape(tree: &XmlTree) -> Proj {
    let Some(root) = tree.root_element() else {
        return Proj::Null;
    };
    let value = project(tree, root, |node, values| {
        let mut map: IndexMap<String, Proj> = IndexMap::new();
        let mut text = String::new();
        let mut placed = false;
        for child in tree.children(node) {
            match tree.kind(*child) {
                Kind::Text(s) | Kind::CData(s) => text.push_str(s),
                Kind::Element {
                    qname,
                    self_closing,
                    ..
                } => {
                    // The old parser flushed pending text when a child
                    // *started* (`Event::Start`); a self-closing child was an
                    // `Event::Empty` and flushed nothing.
                    if !self_closing && !placed && !text.trim().is_empty() {
                        map.insert("_text".to_string(), Proj::Null);
                        placed = true;
                    }
                    let value = values.remove(child).unwrap_or(Proj::Null);
                    insert_repeat(&mut map, qname, value);
                }
                _ => {}
            }
        }
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            map.insert("_text".to_string(), Proj::Str(trimmed.to_string()));
        }
        if map.is_empty() {
            Proj::Null
        } else if map.len() == 1 && map.contains_key("_text") {
            map.swap_remove("_text").unwrap_or(Proj::Null)
        } else {
            Proj::Map(map)
        }
    });
    let mut out = IndexMap::new();
    out.insert(tree.name(root).unwrap_or_default().to_string(), value);
    Proj::Map(out)
}

/// The error string `SOAP.parse` gives for `err`, worded as today.
pub fn soap_error_message(err: &XmlError) -> String {
    match &err.kind {
        XmlErrorKind::Doctype(message) => message.to_string(),
        XmlErrorKind::DepthExceeded(limit) => {
            format!("XML parsing error: nesting depth exceeded {limit} levels (likely DoS payload)")
        }
        XmlErrorKind::TextRunExceeded(limit) => {
            format!("XML parsing error: element text exceeded {limit} bytes (likely DoS payload)")
        }
        _ => format!("XML parsing error: {}", err.quick_xml_message()),
    }
}
