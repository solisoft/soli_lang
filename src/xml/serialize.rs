//! Writing a tree (or one node of it) back out as XML.
//!
//! - **Lossless by default.** With no options a parsed tree is written as it
//!   was read, modulo what the parser normalises: attribute values in `"`,
//!   whitespace outside the root dropped, and top-level nodes separated by one
//!   newline. An element keeps its self-closing bit (`<a/>` stays `<a/>`,
//!   `<a></a>` stays expanded); built elements are self-closing until they get
//!   a child.
//! - **Standalone fragments.** An element written on its own declares every
//!   namespace it (or a descendant, or an attribute) uses but does not declare
//!   itself. The same fix-up keeps a moved element's namespace: URIs are
//!   resolved on the node, and declarations are re-derived on output. An
//!   attribute whose prefix is already taken on its element by another URI
//!   gets a fresh `nsN` prefix.
//! - **Indent** pretty-prints element-only content (whitespace-only text is
//!   dropped there) and never touches mixed content or anything below it.
//! - CDATA holding `]]>` is split across sections; text and attribute values
//!   go through [`super::escape`], which drops characters XML forbids. A
//!   comment a lenient parse kept with `--` in it gets a space between the
//!   dashes, so the output always parses.
//! - Output is capped at `max_bytes`.
//!
//! Iterative: a built tree has no depth limit.

use std::collections::HashSet;
use std::sync::Arc;

use super::error::{XmlError, XmlErrorKind};
use super::escape::{drop_forbidden, escape_attr_into, escape_text_into, split_qname};
use super::ns::Bindings;
use super::tree::{Kind, NodeId, XmlTree, XML_NS};

/// `to_xml` options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerializeOptions {
    /// `Some(n)`: pretty-print element-only content with `n` spaces per level.
    pub indent: Option<usize>,
    /// Write `<?xml …?>`. `None`: on the document node, when the tree has a
    /// declaration (parsed, or set by `Xml.document`); on any other node, no.
    pub declaration: Option<bool>,
    /// Write empty elements as `<a></a>` instead of `<a/>`.
    pub expand_empty: bool,
    /// Output cap in bytes.
    pub max_bytes: usize,
}

impl SerializeOptions {
    /// No options, capped at `SOLI_MAX_XML_BYTES`.
    pub fn compact() -> Self {
        SerializeOptions {
            indent: None,
            declaration: None,
            expand_empty: false,
            max_bytes: usize::try_from(crate::interpreter::limits::max_xml_bytes())
                .unwrap_or(usize::MAX),
        }
    }
}

impl Default for SerializeOptions {
    fn default() -> Self {
        SerializeOptions::compact()
    }
}

/// `node.to_xml(opts)`. On the document node: the declaration (per
/// `opts.declaration`), then the prolog, root and epilogue, one per line. On
/// an element: a standalone fragment.
pub fn serialize(tree: &XmlTree, id: NodeId, opts: &SerializeOptions) -> Result<String, XmlError> {
    let mut writer = Writer::new(tree, opts);
    let is_document = id == NodeId::DOCUMENT;
    let declaration = opts
        .declaration
        .unwrap_or(is_document && tree.decl().is_some());
    if declaration {
        writer.declaration();
    }
    if is_document {
        for (i, child) in tree.children(NodeId::DOCUMENT).iter().enumerate() {
            if i > 0 || declaration {
                writer.out.push('\n');
            }
            writer.node(*child, 0, opts.indent.is_some())?;
        }
    } else {
        if declaration {
            writer.out.push('\n');
        }
        writer.node(id, 0, opts.indent.is_some())?;
    }
    writer.check()?;
    Ok(writer.out)
}

/// `node.inner_xml`: the children of `id`, written in `id`'s namespace scope
/// (so prefixes declared on `id` or above are not re-declared).
pub fn serialize_children(
    tree: &XmlTree,
    id: NodeId,
    opts: &SerializeOptions,
) -> Result<String, XmlError> {
    let mut writer = Writer::new(tree, opts);
    for decl in tree.in_scope_namespaces(id) {
        writer.scope.push(decl.prefix.as_deref(), decl.uri);
    }
    for child in tree.children(id) {
        writer.node(*child, 0, false)?;
    }
    writer.check()?;
    Ok(writer.out)
}

enum Work {
    Node {
        id: NodeId,
        depth: usize,
        pretty: bool,
    },
    Indent(usize),
    Close {
        id: NodeId,
        mark: usize,
    },
}

struct Writer<'t> {
    tree: &'t XmlTree,
    opts: SerializeOptions,
    out: String,
    /// Output namespace scope, one stack per prefix; an empty URI
    /// undeclares the default namespace.
    scope: Bindings,
}

impl<'t> Writer<'t> {
    fn new(tree: &'t XmlTree, opts: &SerializeOptions) -> Self {
        Writer {
            tree,
            opts: *opts,
            out: String::new(),
            scope: Bindings::default(),
        }
    }

    fn check(&self) -> Result<(), XmlError> {
        if self.out.len() > self.opts.max_bytes {
            return Err(XmlError::new(XmlErrorKind::OutputExceeded(
                self.opts.max_bytes,
            )));
        }
        Ok(())
    }

    fn declaration(&mut self) {
        let decl = self.tree.decl();
        self.out.push_str("<?xml version=\"");
        escape_attr_into(&mut self.out, decl.map(|d| &*d.version).unwrap_or("1.0"));
        self.out.push('"');
        // The output is UTF-8 whatever the source declared; a declaration
        // without an encoding stays without one.
        if decl.is_none_or(|d| d.encoding.is_some()) {
            self.out.push_str(" encoding=\"UTF-8\"");
        }
        if let Some(standalone) = decl.and_then(|d| d.standalone.as_deref()) {
            self.out.push_str(" standalone=\"");
            escape_attr_into(&mut self.out, standalone);
            self.out.push('"');
        }
        self.out.push_str("?>");
    }

    fn lookup(&self, prefix: Option<&str>) -> Option<&Arc<str>> {
        if prefix == Some("xml") {
            return None;
        }
        self.scope.get(prefix).filter(|uri| !uri.is_empty())
    }

    fn bound_to(&self, prefix: Option<&str>, uri: Option<&Arc<str>>) -> bool {
        if prefix == Some("xml") {
            return uri.is_some_and(|u| &**u == XML_NS);
        }
        match (self.lookup(prefix), uri) {
            (Some(a), Some(b)) => a == b,
            (None, None) => true,
            _ => false,
        }
    }

    fn write_decl(&mut self, prefix: Option<&str>, uri: Option<&str>) {
        match prefix {
            Some(p) => {
                self.out.push_str(" xmlns:");
                self.out.push_str(p);
            }
            None => self.out.push_str(" xmlns"),
        }
        self.out.push_str("=\"");
        escape_attr_into(&mut self.out, uri.unwrap_or(""));
        self.out.push('"');
    }

    /// Write `id` and everything below it.
    fn node(&mut self, id: NodeId, depth: usize, pretty: bool) -> Result<(), XmlError> {
        let mut stack = vec![Work::Node { id, depth, pretty }];
        while let Some(work) = stack.pop() {
            match work {
                Work::Indent(depth) => {
                    self.out.push('\n');
                    let width = self.opts.indent.unwrap_or(0).saturating_mul(depth);
                    self.out.extend(std::iter::repeat_n(' ', width));
                }
                Work::Close { id, mark } => {
                    if let Some(el) = self.tree.element(id) {
                        self.out.push_str("</");
                        self.out.push_str(el.qname);
                        self.out.push('>');
                    }
                    self.scope.truncate(mark);
                }
                Work::Node { id, depth, pretty } => {
                    self.write_node(id, depth, pretty, &mut stack);
                }
            }
            self.check()?;
        }
        Ok(())
    }

    fn write_node(&mut self, id: NodeId, depth: usize, pretty: bool, stack: &mut Vec<Work>) {
        let tree = self.tree;
        match tree.kind(id) {
            Kind::Document => {
                // Only reachable through a hand-built call: write its children.
                for child in tree.children(id).iter().rev() {
                    stack.push(Work::Node {
                        id: *child,
                        depth,
                        pretty,
                    });
                }
            }
            Kind::Text(text) => escape_text_into(&mut self.out, text),
            Kind::CData(text) => {
                self.out.push_str("<![CDATA[");
                self.out
                    .push_str(&drop_forbidden(text).replace("]]>", "]]]]><![CDATA[>"));
                self.out.push_str("]]>");
            }
            Kind::Comment(text) => {
                self.out.push_str("<!--");
                push_comment_body(&mut self.out, &drop_forbidden(text));
                self.out.push_str("-->");
            }
            Kind::Pi { target, data } => {
                self.out.push_str("<?");
                self.out.push_str(target);
                if !data.is_empty() {
                    self.out.push(' ');
                    self.out.push_str(&drop_forbidden(data));
                }
                self.out.push_str("?>");
            }
            Kind::Element { .. } => self.write_element(id, depth, pretty, stack),
        }
    }

    fn write_element(&mut self, id: NodeId, depth: usize, pretty: bool, stack: &mut Vec<Work>) {
        let tree = self.tree;
        let Some(el) = tree.element(id) else {
            return;
        };
        let mark = self.scope.len();
        self.out.push('<');
        self.out.push_str(el.qname);

        // Prefixes whose meaning on this element is settled: declared here,
        // the element's own (declared here or inherited), and those of
        // attributes already written. A later attribute may not rebind one,
        // or an earlier name would silently change namespace: it gets a
        // fresh `nsN` prefix instead. Sets, so a tag with a thousand
        // prefixed attributes stays linear.
        let mut pinned: HashSet<Option<&str>> = HashSet::new();
        let mut fresh_names: HashSet<String> = HashSet::new();
        let has_prefixed_attr = el.attrs.iter().any(|a| a.prefix().is_some());

        // The element's own declarations, as written.
        for decl in el.ns_decls {
            self.write_decl(decl.prefix.as_deref(), Some(&*decl.uri));
            self.scope.push(decl.prefix.as_deref(), decl.uri.clone());
            pinned.insert(decl.prefix.as_deref());
        }

        // Fix-up for the element's own name.
        let unbound_prefix = el.prefix.is_some() && el.ns_uri.is_none();
        if !unbound_prefix && !self.bound_to(el.prefix, el.ns_uri) && !pinned.contains(&el.prefix) {
            let uri = el.ns_uri.cloned();
            self.write_decl(el.prefix, uri.as_deref());
            self.scope
                .push(el.prefix, uri.unwrap_or_else(|| Arc::from("")));
        }
        if has_prefixed_attr {
            pinned.insert(el.prefix);
        }

        // Fix-ups for attribute prefixes, then the attributes.
        let mut names: Vec<std::borrow::Cow<'_, str>> = Vec::with_capacity(el.attrs.len());
        let mut fresh = 0usize;
        for attr in el.attrs {
            let (prefix, local) = split_qname(&attr.qname);
            let Some(p) = prefix else {
                names.push(std::borrow::Cow::Borrowed(&attr.qname));
                continue;
            };
            let Some(uri) = attr.ns_uri.as_ref() else {
                // An unbound prefix a lenient policy let through: as written.
                names.push(std::borrow::Cow::Borrowed(&attr.qname));
                continue;
            };
            if self.bound_to(Some(p), Some(uri)) {
                names.push(std::borrow::Cow::Borrowed(&attr.qname));
                pinned.insert(Some(p));
                continue;
            }
            if !pinned.contains(&Some(p)) && !fresh_names.contains(p) {
                self.write_decl(Some(p), Some(&**uri));
                self.scope.push(Some(p), uri.clone());
                pinned.insert(Some(p));
                names.push(std::borrow::Cow::Borrowed(&attr.qname));
                continue;
            }
            // `p` is settled on this element with another URI: a fresh prefix
            // (never one already used here or bound above; earlier fresh
            // names are distinct by construction).
            let candidate = loop {
                fresh += 1;
                let candidate = format!("ns{fresh}");
                if !pinned.contains(&Some(candidate.as_str()))
                    && self.lookup(Some(&candidate)).is_none()
                {
                    break candidate;
                }
            };
            self.write_decl(Some(&candidate), Some(&**uri));
            self.scope.push(Some(&candidate), uri.clone());
            names.push(std::borrow::Cow::Owned(format!("{candidate}:{local}")));
            fresh_names.insert(candidate);
        }
        for (attr, name) in el.attrs.iter().zip(names) {
            self.out.push(' ');
            self.out.push_str(&name);
            self.out.push_str("=\"");
            escape_attr_into(&mut self.out, &attr.value);
            self.out.push('"');
        }

        let pretty_children = pretty && self.element_only(id);
        let kids: Vec<NodeId> = tree
            .children(id)
            .iter()
            .copied()
            .filter(|c| !(pretty_children && matches!(tree.kind(*c), Kind::Text(_))))
            .collect();
        if kids.is_empty() {
            if el.self_closing && !self.opts.expand_empty {
                self.out.push_str("/>");
            } else {
                self.out.push_str("></");
                self.out.push_str(el.qname);
                self.out.push('>');
            }
            self.scope.truncate(mark);
            return;
        }
        self.out.push('>');
        stack.push(Work::Close { id, mark });
        if pretty_children {
            stack.push(Work::Indent(depth));
        }
        for kid in kids.into_iter().rev() {
            stack.push(Work::Node {
                id: kid,
                depth: depth + 1,
                pretty: pretty_children,
            });
            if pretty_children {
                stack.push(Work::Indent(depth + 1));
            }
        }
    }

    /// No CDATA and no text but whitespace among the children: indenting
    /// cannot change the content.
    fn element_only(&self, id: NodeId) -> bool {
        self.tree
            .children(id)
            .iter()
            .all(|c| match self.tree.kind(*c) {
                Kind::Text(text) => text
                    .chars()
                    .all(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r')),
                Kind::CData(_) => false,
                _ => true,
            })
    }
}

/// A comment body that is always well-formed. A lenient parse (SOAP, c14n)
/// can keep `--` or a trailing `-`, which no comment may hold: a space goes
/// between two dashes and after a final one. Built comments never need it.
fn push_comment_body(out: &mut String, text: &str) {
    let mut previous_dash = false;
    for c in text.chars() {
        if c == '-' && previous_dash {
            out.push(' ');
        }
        out.push(c);
        previous_dash = c == '-';
    }
    if previous_dash {
        out.push(' ');
    }
}
