//! The XML core: a parser, a mutable tree, a serialiser and projections, in
//! plain Rust.
//!
//! Nothing here knows about the interpreter: no `Value`, no `Rc`, no
//! `Environment`. An [`XmlTree`] is `Send`, so `SOAP.call`'s worker thread can
//! parse and project, and the fuzz target calls [`parse`] directly. The Soli
//! binding (`interpreter/builtins/xml/`) wraps a tree in
//! `Rc<RefCell<XmlTree>>` plus a [`NodeId`].
//!
//! # Frozen API (later lanes build on these signatures)
//!
//! **Parsing** — [`parse`]`(&str, &ParsePolicy) -> Result<XmlTree, XmlError>`
//! and [`parse_bytes`]. One loop, [`EventLoop`]`<R: BufRead>`, reads
//! quick-xml's `NsReader` events and feeds a [`Sink`]: [`TreeSink`] builds a
//! tree; a streaming sink (lane L3) gets the same `start_element(StartTag,
//! &NsScope)` / `end_element` / `text` / `cdata` / `comment` / `pi` calls and
//! drives the loop one [`EventLoop::step`] at a time. The loop applies the
//! policy; a sink only builds.
//!
//! **Policies** — [`ParsePolicy::xml`], [`ParsePolicy::soap`],
//! [`ParsePolicy::c14n`]; [`ParsePolicy::lowered`] applies per-call limits
//! (they can only go down) and [`ParsePolicy::strip_blank`] drops blank text.
//! Knobs: `doctype_message`, `doctype` (refuse or skip), `unknown_entity`, `unbound_prefix`,
//! `duplicate_attribute`, `trailing_content`, `no_root`, `max_depth`,
//! `max_nodes`, `max_bytes`, `max_attrs`, `max_text_run`, `max_ns_bindings`,
//! `keep_blank_text`, `strict_syntax`, `raw_pi_data`, `lenient_attributes`, and
//! the legacy knobs that keep `SOAP.parse` and the c14n trio accepting what
//! their old parsers accepted: `max_ns_decls_per_element`,
//! `legacy_char_refs`, `duplicate_by_qname`, `self_closing_past_depth`,
//! `stop_after_root`, `keep_text_before_root`. Limits come from
//! `interpreter::limits::max_xml_*`.
//!
//! **The tree** — [`XmlTree`], [`NodeId`] (`NodeId::DOCUMENT` is 0), [`Kind`]
//! (`Document`, `Element { qname, local_start, ns_uri, attrs, ns_decls,
//! self_closing }`, `Text`, `CData`, `Comment`, `Pi { target, data }`),
//! [`Attr`], [`NsDecl`], [`XmlDecl`], and the borrowed [`ElementRef`] from
//! [`XmlTree::element`]. Reading: `kind`, `parent`, `parent_element`,
//! `children` (all kinds), `element_children`, `root_element`, `name`,
//! `attribute`, `attribute_ns`, `lookup_namespace`, `in_scope_namespaces`,
//! `next_node` / `previous_node`, `next_sibling` / `previous_sibling`
//! (elements), `ancestors`, `descendants`, `descendant_nodes`, `text`
//! (string-value), `own_text`, `path`, `fragment_root`, `is_attached`,
//! `version`. Document order: `ensure_order` then `order(id)`.
//!
//! **Building and mutation** — `XmlTree::new(limit)`,
//! `XmlTree::new_document(limit, Some((root, attrs)))` (declaration on; the
//! tree side of `Xml.document` / `Xml.build`), `XmlTree::new_fragment`
//! (`Xml.element`); `add_element(parent, qname, attrs)` (the tree side of
//! `node.add` with or without a block: it returns the child the block
//! receives), `add_text`, `add_cdata`, `add_comment`, `add_pi`; detached
//! `create_*`; `append_child`, `prepend_child`, `insert_before`,
//! `insert_after`, `replace` (same-tree moves, `would create a cycle` when a
//! node would become its own ancestor), `detach` (`node.remove`; the node
//! stays as a fragment — a tombstone counted by the node limit), `import`
//! (deep copy from another tree), `dup` (fresh compacted tree),
//! `set_attribute` / `remove_attribute`, `set_text`, `rename`. `attrs` are
//! `&[(&str, &str)]`; `xmlns` / `xmlns:p` keys become declarations; names are
//! validated as QNames; characters XML forbids are dropped from text and
//! values.
//!
//! **Serialisation** — [`serialize`]`(tree, id, &SerializeOptions)` and
//! [`serialize_children`] (`inner_xml`). [`SerializeOptions`] has `indent`,
//! `declaration` (`None` = the default rule), `expand_empty`, `max_bytes`.
//!
//! **Projections** — [`Proj`] (`Null`, `Str`, `Map(IndexMap)`, `List`),
//! [`to_h`], [`soap_shape`], [`soap_error_message`].
//!
//! **Escaping** — [`escape::escape5`] (`Xml.escape` = `SOAP.xml_escape`),
//! [`escape::escape_text`], [`escape::escape_attr`], [`escape::is_qname`],
//! [`escape::is_ncname`], [`escape::drop_forbidden`].
//!
//! **Streaming** — [`stream::Pull`] (`Xml.each`, `Xml.reader`) drives the
//! loop one step at a time with a `StreamSink` and hands over each
//! [`stream::Matcher`] match as a tree of its own, a detached element
//! carrying the namespaces in scope where it stood. `max_bytes` caps one
//! subtree and one event (a byte-limiting reader enforces it before quick-xml
//! buffers), `max_nodes` one subtree; the file itself has no cap.
//!
//! **Queries** — [`XPath::compile`] then [`select`]`(tree, context, &xpath, &Query { flavour, namespaces, max_visits })`: the XPath subset of `find` / `find_all` ([`Flavour::Lenient`]) and `xpath` / `at_xpath` ([`Flavour::Strict`]); see [`xpath`].
//!
//! **Errors** — [`XmlError`] `{ kind, offset, position }`; `Display` is
//! `line L, column C: message` once located. The binding prefixes it
//! (`Xml.parse: …`).

pub mod error;
pub mod escape;
mod ns;
pub mod parse;
pub mod policy;
pub mod project;
pub mod serialize;
pub mod stream;
pub mod tree;
pub mod xpath;

#[cfg(test)]
mod tests;

pub use error::{line_col, XmlError, XmlErrorKind};
pub use parse::{parse, parse_bytes, EventLoop, NsScope, Sink, StartTag, Step, TreeSink};
pub use policy::{
    DoctypeMode, OnNoRoot, OnUnknownEntity, ParsePolicy, Strictness, TrailingContent,
    MAX_DEPTH_CEILING, MAX_NAME_LEN, MAX_NS_BINDINGS, QUICK_XML_NS_DECLS_PER_ELEMENT,
};
pub use project::{soap_error_message, soap_shape, to_h, Proj};
pub use serialize::{serialize, serialize_children, SerializeOptions};
pub use stream::{Matcher, Pull};
pub use tree::{Attr, ElementRef, Kind, NodeData, NodeId, NsDecl, XmlDecl, XmlTree, XML_NS};
pub use xpath::{select, Flavour, Item, Query, XPath, XPathError, XPathErrorKind};

#[cfg(test)]
mod send_check {
    fn assert_send<T: Send>() {}

    #[test]
    fn the_core_types_are_send() {
        assert_send::<super::XmlTree>();
        assert_send::<super::XmlError>();
        assert_send::<super::Proj>();
        assert_send::<super::ParsePolicy>();
    }
}
