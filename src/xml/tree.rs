//! `XmlTree`: an arena of nodes addressed by `NodeId`.
//!
//! Nodes live in one `Vec` and refer to each other by `u32` index, so the tree
//! holds no pointers, has no cycles to break, and is `Send`. `NodeId(0)` is
//! always the document node.
//!
//! **Mutation and tombstones.** A removed node is detached (its `parent`
//! becomes `None`) but stays in the arena: a Soli handle may still point at
//! it, and it stays usable as the root of its own fragment. Detached nodes
//! count toward the node limit until the tree is dropped; [`XmlTree::dup`]
//! copies a subtree into a fresh, compacted tree.
//!
//! **Document order.** Every node carries a pre-order index (`order`). A
//! structural edit only sets `order_dirty`; [`XmlTree::ensure_order`]
//! recomputes the indices lazily, before the next query that needs them.
//!
//! **Namespaces.** An element's namespace URI is resolved once, when it is
//! parsed or created, and stored on the node (`ns_uri`, shared through
//! `Arc<str>`). The serialiser re-declares whatever a moved or detached
//! element needs, so a resolved URI is never lost by an edit. Lookups follow
//! the same rule: a prefix is bound by a declaration **or** by a resolved
//! name using it (the element's own, then its attributes'), nearest first.
//! So a `dup`, an `import` or a removed node can be built on with exactly
//! the prefixes its own `to_xml` declares.
//!
//! **Sibling position** is found by a scan of the parent's children
//! (`next_node`, `previous_node`, `next_sibling`, `previous_sibling`,
//! `insert_before`, `detach`, …): walking all children of one element with
//! `next_node` is quadratic in its child count. Walk `children(id)` instead.
//!
//! Every walk here is iterative: a built tree has no depth limit, so nothing
//! may recurse once per level.

use std::collections::HashSet;
use std::sync::Arc;

use super::error::{XmlError, XmlErrorKind};
use super::escape::{drop_forbidden, is_ncname, is_qname, split_qname};
use super::ns::Bindings;
use super::parse::normalize_eol;
use super::policy::MAX_NAME_LEN;

/// The namespace the `xml` prefix is bound to, everywhere, without a
/// declaration.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// A node's index in its tree. `NodeId(0)` is the document node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The document node of every tree.
    pub const DOCUMENT: NodeId = NodeId(0);

    /// The arena index.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One attribute, as written: its qname, its resolved namespace (`None` for an
/// unprefixed attribute, or for an unbound prefix a lenient policy allowed),
/// and its value with references decoded and whitespace normalised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub qname: Box<str>,
    pub ns_uri: Option<Arc<str>>,
    pub value: Box<str>,
}

impl Attr {
    /// `Some("xlink")` for `xlink:href`.
    pub fn prefix(&self) -> Option<&str> {
        split_qname(&self.qname).0
    }

    /// `"href"` for `xlink:href`.
    pub fn local_name(&self) -> &str {
        split_qname(&self.qname).1
    }
}

/// One namespace declaration made on an element: `xmlns="uri"` has prefix
/// `None`, `xmlns:p="uri"` has `Some("p")`. `xmlns=""` (undeclaring the
/// default namespace) has an empty `uri`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsDecl {
    pub prefix: Option<Box<str>>,
    pub uri: Arc<str>,
}

/// `<?xml version="…" encoding="…" standalone="…"?>`, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlDecl {
    pub version: Box<str>,
    pub encoding: Option<Box<str>>,
    pub standalone: Option<Box<str>>,
}

impl Default for XmlDecl {
    /// `version="1.0" encoding="UTF-8"`: what `Xml.document` writes.
    fn default() -> Self {
        XmlDecl {
            version: "1.0".into(),
            encoding: Some("UTF-8".into()),
            standalone: None,
        }
    }
}

/// What a node is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Document,
    Element {
        /// The name as written: `ram:Name`.
        qname: Box<str>,
        /// Byte offset of the local name in `qname` (0 when unprefixed).
        local_start: u16,
        ns_uri: Option<Arc<str>>,
        /// Regular attributes in source order (`xmlns*` excluded).
        attrs: Vec<Attr>,
        /// Namespace declarations made on this element, in source order.
        ns_decls: Vec<NsDecl>,
        /// Parsed as `<a/>`, or built: written `<a/>` when it has no child.
        self_closing: bool,
    },
    Text(Box<str>),
    CData(Box<str>),
    Comment(Box<str>),
    Pi {
        target: Box<str>,
        data: Box<str>,
    },
}

impl Kind {
    /// `"document"`, `"element"`, `"text"`, `"cdata"`, `"comment"`, `"pi"`:
    /// the names `node.kind` returns.
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Document => "document",
            Kind::Element { .. } => "element",
            Kind::Text(_) => "text",
            Kind::CData(_) => "cdata",
            Kind::Comment(_) => "comment",
            Kind::Pi { .. } => "pi",
        }
    }
}

/// One arena slot.
#[derive(Debug, Clone)]
pub struct NodeData {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    /// Pre-order index; valid only while the tree's `order_dirty` is false.
    pub order: u32,
    pub kind: Kind,
}

/// A parsed or built XML document (or fragment).
#[derive(Debug, Clone)]
pub struct XmlTree {
    nodes: Vec<NodeData>,
    decl: Option<XmlDecl>,
    /// The name of the DOCTYPE skipped while parsing (`doctype: "skip"`);
    /// nothing else of the declaration is kept, and it is never serialised.
    doctype: Option<Box<str>>,
    order_dirty: bool,
    version: u64,
    node_limit: usize,
    /// Some element of this tree carries a namespace declaration or a
    /// namespaced name. While none does, namespace lookups answer without
    /// walking the ancestors (building a deep tree with no namespaces stays
    /// linear).
    has_namespaces: bool,
}

/// A borrowed view of an element's fields, for readers that do not want to
/// match on [`Kind`].
#[derive(Debug, Clone, Copy)]
pub struct ElementRef<'a> {
    pub qname: &'a str,
    pub local_name: &'a str,
    pub prefix: Option<&'a str>,
    pub ns_uri: Option<&'a Arc<str>>,
    pub attrs: &'a [Attr],
    pub ns_decls: &'a [NsDecl],
    pub self_closing: bool,
}

fn too_many(limit: usize) -> XmlError {
    XmlError::new(XmlErrorKind::NodesExceeded(limit))
}

fn invalid(message: &str) -> XmlError {
    XmlError::new(XmlErrorKind::InvalidOperation(message.to_string()))
}

/// Validate an element or attribute name given by a caller (not the parser).
fn check_qname(qname: &str) -> Result<(), XmlError> {
    if qname.len() > MAX_NAME_LEN {
        return Err(XmlError::new(XmlErrorKind::NameTooLong(MAX_NAME_LEN)));
    }
    if !is_qname(qname) {
        return Err(XmlError::new(XmlErrorKind::InvalidName(qname.to_string())));
    }
    Ok(())
}

fn local_start_of(qname: &str) -> u16 {
    match qname.find(':') {
        Some(colon) => u16::try_from(colon + 1).unwrap_or(0),
        None => 0,
    }
}

impl XmlTree {
    /// An empty tree: just the document node, no declaration. `node_limit`
    /// caps the nodes it will ever hold (document node excluded).
    pub fn new(node_limit: usize) -> Self {
        XmlTree {
            nodes: vec![NodeData {
                parent: None,
                children: Vec::new(),
                order: 0,
                kind: Kind::Document,
            }],
            decl: None,
            doctype: None,
            order_dirty: false,
            version: 0,
            node_limit,
            has_namespaces: false,
        }
    }

    /// `Xml.document(root_name, attrs)` / `Xml.build(root_name, attrs)`: a
    /// tree with the default declaration and, when `root` is given, a root
    /// element. Returns the tree and the root's id (`None` without one).
    pub fn new_document(
        node_limit: usize,
        root: Option<(&str, &[(&str, &str)])>,
    ) -> Result<(XmlTree, Option<NodeId>), XmlError> {
        let mut tree = XmlTree::new(node_limit);
        tree.decl = Some(XmlDecl::default());
        let root_id = match root {
            Some((qname, attrs)) => Some(tree.add_element(NodeId::DOCUMENT, qname, attrs)?),
            None => None,
        };
        Ok((tree, root_id))
    }

    /// `Xml.element(name, …)`: a tree with no declaration holding one
    /// detached element. Returns the tree and the element's id.
    pub fn new_fragment(
        node_limit: usize,
        qname: &str,
        attrs: &[(&str, &str)],
    ) -> Result<(XmlTree, NodeId), XmlError> {
        let mut tree = XmlTree::new(node_limit);
        let id = tree.create_element(qname, attrs, None)?;
        Ok((tree, id))
    }

    // ----------------------------------------------------------------- reading

    /// The node limit this tree enforces.
    pub fn node_limit(&self) -> usize {
        self.node_limit
    }

    /// Arena size, document node excluded: live, detached and tombstoned
    /// nodes alike. This is what the node limit counts.
    pub fn node_count(&self) -> usize {
        self.nodes.len() - 1
    }

    /// Bumped by every edit (the response cache can key on it).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// `true` when `id` names a slot of this tree.
    pub fn contains(&self, id: NodeId) -> bool {
        id.index() < self.nodes.len()
    }

    /// The node at `id`. Panics only on an id from another tree; the binding
    /// never mixes them, and [`XmlTree::get`] is the checked form.
    fn node(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.index()]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.nodes[id.index()]
    }

    /// The node at `id`, or `None` for an id from elsewhere.
    pub fn get(&self, id: NodeId) -> Option<&NodeData> {
        self.nodes.get(id.index())
    }

    pub fn kind(&self, id: NodeId) -> &Kind {
        &self.node(id).kind
    }

    pub fn decl(&self) -> Option<&XmlDecl> {
        self.decl.as_ref()
    }

    /// The name of the DOCTYPE the parser skipped, if any.
    pub fn doctype(&self) -> Option<&str> {
        self.doctype.as_deref()
    }

    pub(crate) fn set_doctype(&mut self, name: Option<Box<str>>) {
        self.doctype = name;
    }

    pub fn set_decl(&mut self, decl: Option<XmlDecl>) {
        self.decl = decl;
        self.version += 1;
    }

    /// The node's parent of any kind (the document node for the root).
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    /// `node.parent`: the parent **element**, `None` for the root, a detached
    /// node or the document.
    pub fn parent_element(&self, id: NodeId) -> Option<NodeId> {
        self.parent(id).filter(|p| self.is_element(*p))
    }

    /// All children, of every kind, in order (`node.nodes`).
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.node(id).children
    }

    /// Element children only, in order (`node.children`).
    pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children(id)
            .iter()
            .copied()
            .filter(|c| self.is_element(*c))
    }

    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.kind(id), Kind::Element { .. })
    }

    /// The document's root element.
    pub fn root_element(&self) -> Option<NodeId> {
        self.element_children(NodeId::DOCUMENT).next()
    }

    /// The element's fields, or `None` for another kind.
    pub fn element(&self, id: NodeId) -> Option<ElementRef<'_>> {
        match self.kind(id) {
            Kind::Element {
                qname,
                local_start,
                ns_uri,
                attrs,
                ns_decls,
                self_closing,
            } => {
                let start = usize::from(*local_start).min(qname.len());
                let prefix = if start > 0 {
                    qname.get(..start - 1)
                } else {
                    None
                };
                Some(ElementRef {
                    qname,
                    local_name: qname.get(start..).unwrap_or(qname),
                    prefix,
                    ns_uri: ns_uri.as_ref(),
                    attrs,
                    ns_decls,
                    self_closing: *self_closing,
                })
            }
            _ => None,
        }
    }

    /// `node.name`: the element qname or the PI target.
    pub fn name(&self, id: NodeId) -> Option<&str> {
        match self.kind(id) {
            Kind::Element { qname, .. } => Some(qname),
            Kind::Pi { target, .. } => Some(target),
            _ => None,
        }
    }

    /// Attribute value by qname as written (`node["id"]`).
    pub fn attribute(&self, id: NodeId, qname: &str) -> Option<&str> {
        self.element(id)?
            .attrs
            .iter()
            .find(|a| &*a.qname == qname)
            .map(|a| &*a.value)
    }

    /// Attribute value by expanded name (`node.attribute("href", xlink_uri)`);
    /// `ns_uri` `None` matches an attribute in no namespace.
    pub fn attribute_ns(&self, id: NodeId, local: &str, ns_uri: Option<&str>) -> Option<&str> {
        self.element(id)?
            .attrs
            .iter()
            .find(|a| a.local_name() == local && a.ns_uri.as_deref() == ns_uri)
            .map(|a| &*a.value)
    }

    /// The URI `prefix` (`None` = default namespace) is bound to at `id`:
    /// walking from `id` up, the first element that binds it — by a
    /// declaration, by its own name, or by one of its attributes' names —
    /// decides. `xml` is always bound. An undeclared default (`xmlns=""`, or
    /// an unprefixed element in no namespace) gives `None`.
    pub fn lookup_namespace(&self, id: NodeId, prefix: Option<&str>) -> Option<Arc<str>> {
        if prefix == Some("xml") {
            return Some(Arc::from(XML_NS));
        }
        if !self.has_namespaces {
            return None;
        }
        let mut current = Some(id);
        while let Some(node) = current {
            if let Some(el) = self.element(node) {
                if let Some(binding) = binding_at(&el, prefix) {
                    return binding;
                }
            }
            current = self.parent(node);
        }
        None
    }

    /// `node.namespaces`: every prefix in scope at `id` (`None` = default),
    /// nearest binding winning, in the order the prefixes were first bound
    /// walking from the root down. A binding is a declaration or a resolved
    /// name that uses the prefix (the rule [`XmlTree::lookup_namespace`] and
    /// the serialiser follow). `xml` and undeclared defaults are left out.
    /// Linear in the declarations and attributes on the path.
    pub fn in_scope_namespaces(&self, id: NodeId) -> Vec<NsDecl> {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(node) = current {
            chain.push(node);
            current = self.parent(node);
        }
        let mut scope = Bindings::default();
        let mut seen: HashSet<&str> = HashSet::new();
        for node in chain.into_iter().rev() {
            let Some(el) = self.element(node) else {
                continue;
            };
            for decl in el.ns_decls {
                scope.push(decl.prefix.as_deref(), decl.uri.clone());
            }
            let declared = |p: Option<&str>| el.ns_decls.iter().any(|d| d.prefix.as_deref() == p);
            let name_binds =
                el.prefix != Some("xml") && (el.prefix.is_none() || el.ns_uri.is_some());
            if name_binds && !declared(el.prefix) {
                let uri = el.ns_uri.map_or("", |u| &**u);
                if !scope.means(el.prefix, uri) {
                    scope.push(el.prefix, Arc::from(uri));
                }
            }
            seen.clear();
            for attr in el.attrs {
                let (Some(p), Some(uri)) = (attr.prefix(), attr.ns_uri.as_ref()) else {
                    continue;
                };
                if p == "xml" || el.prefix == Some(p) || declared(Some(p)) || !seen.insert(p) {
                    continue;
                }
                if !scope.means(Some(p), uri) {
                    scope.push(Some(p), uri.clone());
                }
            }
        }
        scope.in_scope()
    }

    /// Position of `id` among its parent's children.
    fn index_in_parent(&self, id: NodeId) -> Option<(NodeId, usize)> {
        let parent = self.parent(id)?;
        let pos = self.children(parent).iter().position(|c| *c == id)?;
        Some((parent, pos))
    }

    /// `node.next_node`: the next sibling of any kind.
    pub fn next_node(&self, id: NodeId) -> Option<NodeId> {
        let (parent, pos) = self.index_in_parent(id)?;
        self.children(parent).get(pos + 1).copied()
    }

    /// `node.previous_node`: the previous sibling of any kind.
    pub fn previous_node(&self, id: NodeId) -> Option<NodeId> {
        let (parent, pos) = self.index_in_parent(id)?;
        pos.checked_sub(1)
            .and_then(|p| self.children(parent).get(p).copied())
    }

    /// `node.next_sibling`: the next element sibling.
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let (parent, pos) = self.index_in_parent(id)?;
        self.children(parent)[pos + 1..]
            .iter()
            .copied()
            .find(|c| self.is_element(*c))
    }

    /// `node.previous_sibling`: the previous element sibling.
    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        let (parent, pos) = self.index_in_parent(id)?;
        self.children(parent)[..pos]
            .iter()
            .rev()
            .copied()
            .find(|c| self.is_element(*c))
    }

    /// `node.ancestors`: ancestor elements, nearest first.
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut current = self.parent_element(id);
        while let Some(node) = current {
            out.push(node);
            current = self.parent_element(node);
        }
        out
    }

    /// Every node strictly below `id`, any kind, in document order.
    pub fn descendant_nodes(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.children(id).iter().rev().copied().collect();
        while let Some(node) = stack.pop() {
            out.push(node);
            stack.extend(self.children(node).iter().rev().copied());
        }
        out
    }

    /// `node.descendants`: element descendants in document order.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = self.descendant_nodes(id);
        out.retain(|n| self.is_element(*n));
        out
    }

    /// `true` when `ancestor` is `node` or one of its ancestors.
    pub fn is_ancestor_or_self(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(n) = current {
            if n == ancestor {
                return true;
            }
            current = self.parent(n);
        }
        false
    }

    /// The top of `id`'s fragment: the document node when `id` is attached,
    /// otherwise the detached node it hangs from.
    pub fn fragment_root(&self, id: NodeId) -> NodeId {
        let mut current = id;
        while let Some(parent) = self.parent(current) {
            current = parent;
        }
        current
    }

    /// `true` when `id` is reachable from the document node.
    pub fn is_attached(&self, id: NodeId) -> bool {
        self.fragment_root(id) == NodeId::DOCUMENT
    }

    /// `node.text`: the XPath string-value — every descendant text and CDATA
    /// node, in order, not trimmed. On a text, CDATA or comment node its
    /// content; on a PI its data.
    pub fn text(&self, id: NodeId) -> String {
        match self.kind(id) {
            Kind::Text(s) | Kind::CData(s) | Kind::Comment(s) => s.to_string(),
            Kind::Pi { data, .. } => data.to_string(),
            Kind::Element { .. } | Kind::Document => {
                let mut out = String::new();
                for node in self.descendant_nodes(id) {
                    if let Kind::Text(s) | Kind::CData(s) = self.kind(node) {
                        out.push_str(s);
                    }
                }
                out
            }
        }
    }

    /// `node.own_text`: direct text and CDATA children only.
    pub fn own_text(&self, id: NodeId) -> String {
        match self.kind(id) {
            Kind::Element { .. } | Kind::Document => {
                let mut out = String::new();
                for child in self.children(id) {
                    if let Kind::Text(s) | Kind::CData(s) = self.kind(*child) {
                        out.push_str(s);
                    }
                }
                out
            }
            _ => self.text(id),
        }
    }

    /// `node.path`: `/rss/channel/item[2]`. A `[k]` index is written when the
    /// parent holds more than one sibling of the same name (or kind, for
    /// non-elements: `text()[k]`, `comment()[k]`, …). A detached node's path
    /// starts at its fragment root.
    pub fn path(&self, id: NodeId) -> String {
        if id == NodeId::DOCUMENT {
            return "/".to_string();
        }
        let mut steps = Vec::new();
        let mut current = id;
        loop {
            let Some(parent) = self.parent(current) else {
                // A detached node: its own step, without an index.
                steps.push(self.step_name(current));
                break;
            };
            let same: Vec<NodeId> = self
                .children(parent)
                .iter()
                .copied()
                .filter(|c| self.step_name(*c) == self.step_name(current))
                .collect();
            let mut step = self.step_name(current);
            if same.len() > 1 {
                let k = same.iter().position(|c| *c == current).unwrap_or(0) + 1;
                step.push_str(&format!("[{k}]"));
            }
            steps.push(step);
            if parent == NodeId::DOCUMENT {
                break;
            }
            current = parent;
        }
        steps.reverse();
        let attached = self.is_attached(id);
        let mut out = String::new();
        for (i, step) in steps.iter().enumerate() {
            if i > 0 || attached {
                out.push('/');
            }
            out.push_str(step);
        }
        out
    }

    fn step_name(&self, id: NodeId) -> String {
        match self.kind(id) {
            Kind::Element { qname, .. } => qname.to_string(),
            Kind::Text(_) | Kind::CData(_) => "text()".to_string(),
            Kind::Comment(_) => "comment()".to_string(),
            Kind::Pi { .. } => "processing-instruction()".to_string(),
            Kind::Document => String::new(),
        }
    }

    // ------------------------------------------------------------ document order

    /// `true` after a structural edit, until [`XmlTree::ensure_order`] runs.
    pub fn order_dirty(&self) -> bool {
        self.order_dirty
    }

    /// Recompute every pre-order index if an edit invalidated them: the
    /// document's nodes first, then each detached fragment in arena order.
    pub fn ensure_order(&mut self) {
        if !self.order_dirty {
            return;
        }
        let mut next: u32 = 0;
        let roots: Vec<NodeId> = (0..self.nodes.len())
            .map(|i| NodeId(i as u32))
            .filter(|id| self.nodes[id.index()].parent.is_none())
            .collect();
        for root in roots {
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                self.nodes[node.index()].order = next;
                next = next.saturating_add(1);
                stack.extend(self.nodes[node.index()].children.iter().rev().copied());
            }
        }
        self.order_dirty = false;
    }

    /// The pre-order index of `id`. Call [`XmlTree::ensure_order`] after an
    /// edit first; with a dirty tree this is the stale index.
    pub fn order(&self, id: NodeId) -> u32 {
        self.node(id).order
    }

    // ------------------------------------------------------------- building

    fn push_node(&mut self, kind: Kind) -> Result<NodeId, XmlError> {
        if self.node_count() >= self.node_limit {
            return Err(too_many(self.node_limit));
        }
        let id = NodeId(u32::try_from(self.nodes.len()).map_err(|_| too_many(self.node_limit))?);
        let order = id.0;
        if let Kind::Element {
            ns_decls,
            ns_uri,
            attrs,
            ..
        } = &kind
        {
            self.has_namespaces |= !ns_decls.is_empty()
                || ns_uri.is_some()
                || attrs.iter().any(|a| a.ns_uri.is_some());
        }
        self.nodes.push(NodeData {
            parent: None,
            children: Vec::new(),
            order,
            kind,
        });
        // A fresh node's index is its arena index; it is only out of order
        // once attached, which marks the tree dirty.
        Ok(id)
    }

    /// Append an already-built node to `parent` with no checks: the parser's
    /// fast path (it has already validated everything).
    pub(crate) fn push_child_unchecked(
        &mut self,
        parent: NodeId,
        kind: Kind,
    ) -> Result<NodeId, XmlError> {
        let id = self.push_node(kind)?;
        self.node_mut(id).parent = Some(parent);
        self.node_mut(parent).children.push(id);
        Ok(id)
    }

    /// Add an already-built node with no parent and no checks: the root of
    /// a streamed match (`stream.rs`), a fragment like `Xml.element`'s.
    pub(crate) fn push_detached_unchecked(&mut self, kind: Kind) -> Result<NodeId, XmlError> {
        self.push_node(kind)
    }

    pub(crate) fn set_decl_unchecked(&mut self, decl: XmlDecl) {
        self.decl = Some(decl);
    }

    /// Create a detached element. `attrs` are `(qname, value)` pairs in order:
    /// `xmlns` / `xmlns:p` keys become namespace declarations, the others
    /// attributes (values have forbidden characters dropped). The element's and
    /// attributes' prefixes resolve against those declarations first, then
    /// against `scope`'s in-scope namespaces. Raises on an invalid name, an
    /// unbound prefix or a duplicate attribute.
    pub fn create_element(
        &mut self,
        qname: &str,
        attrs: &[(&str, &str)],
        scope: Option<NodeId>,
    ) -> Result<NodeId, XmlError> {
        check_qname(qname)?;
        if split_qname(qname).0 == Some("xmlns") {
            return Err(XmlError::new(XmlErrorKind::InvalidName(qname.to_string())));
        }
        let mut ns_decls: Vec<NsDecl> = Vec::new();
        let mut plain: Vec<(&str, &str)> = Vec::new();
        let max_attrs = max_attrs();
        for &(key, value) in attrs {
            if key == "xmlns" {
                push_decl(&mut ns_decls, None, value)?;
            } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                if !is_ncname(prefix) || prefix == "xmlns" {
                    return Err(XmlError::new(XmlErrorKind::InvalidName(key.to_string())));
                }
                push_decl(&mut ns_decls, Some(prefix), value)?;
            } else {
                if plain.len() >= max_attrs {
                    return Err(XmlError::new(XmlErrorKind::AttrsExceeded(max_attrs)));
                }
                plain.push((key, value));
            }
        }
        let resolve = |prefix: Option<&str>, tree: &XmlTree| -> Option<Arc<str>> {
            if prefix == Some("xml") {
                return Some(Arc::from(XML_NS));
            }
            if let Some(decl) = ns_decls.iter().find(|d| d.prefix.as_deref() == prefix) {
                return (!decl.uri.is_empty()).then(|| decl.uri.clone());
            }
            scope.and_then(|s| tree.lookup_namespace(s, prefix))
        };
        let (prefix, _) = split_qname(qname);
        let ns_uri = resolve(prefix, self);
        if let (Some(p), None) = (prefix, &ns_uri) {
            return Err(XmlError::new(XmlErrorKind::UnboundPrefix(p.to_string())));
        }
        let mut out_attrs: Vec<Attr> = Vec::with_capacity(plain.len());
        for (key, value) in plain {
            check_qname(key)?;
            let (attr_prefix, _) = split_qname(key);
            let attr_ns = match attr_prefix {
                Some(p) => match resolve(Some(p), self) {
                    Some(uri) => Some(uri),
                    None => return Err(XmlError::new(XmlErrorKind::UnboundPrefix(p.to_string()))),
                },
                None => None,
            };
            out_attrs.push(Attr {
                qname: key.into(),
                ns_uri: attr_ns,
                value: drop_forbidden(value).into(),
            });
        }
        if let Some(dup) = find_duplicate(&out_attrs) {
            return Err(XmlError::new(XmlErrorKind::DuplicateAttribute(
                dup.qname.to_string(),
            )));
        }
        let id = self.push_node(Kind::Element {
            qname: qname.into(),
            local_start: local_start_of(qname),
            ns_uri,
            attrs: out_attrs,
            ns_decls,
            self_closing: true,
        })?;
        self.version += 1;
        self.order_dirty = true;
        Ok(id)
    }

    /// Create a detached text node. Forbidden characters are dropped.
    pub fn create_text(&mut self, text: &str) -> Result<NodeId, XmlError> {
        self.created(Kind::Text(drop_forbidden(text).into()))
    }

    /// Create a detached CDATA node. Forbidden characters are dropped and
    /// line endings normalised to LF (a CR cannot be escaped there, and a
    /// parser would read it back as LF); a `]]>` inside is split across
    /// sections when written.
    pub fn create_cdata(&mut self, text: &str) -> Result<NodeId, XmlError> {
        self.created(Kind::CData(normalize_eol(&drop_forbidden(text)).into()))
    }

    /// Create a detached comment (line endings normalised, as for CDATA).
    /// Raises when it holds `--` or ends with `-`.
    pub fn create_comment(&mut self, text: &str) -> Result<NodeId, XmlError> {
        let text = normalize_eol(&drop_forbidden(text));
        check_comment(&text)?;
        self.created(Kind::Comment(text.into()))
    }

    /// Create a detached processing instruction (line endings normalised, as
    /// for CDATA). Raises on a target that is not an NCName or is `xml` (any
    /// case), or data holding `?>`.
    pub fn create_pi(&mut self, target: &str, data: &str) -> Result<NodeId, XmlError> {
        let data = normalize_eol(&drop_forbidden(data));
        check_pi(target, &data)?;
        self.created(Kind::Pi {
            target: target.into(),
            data: data.into(),
        })
    }

    fn created(&mut self, kind: Kind) -> Result<NodeId, XmlError> {
        let id = self.push_node(kind)?;
        self.version += 1;
        self.order_dirty = true;
        Ok(id)
    }

    /// `parent.add(name, content, attrs)`: create an element resolved in
    /// `parent`'s scope and append it. Returns the new element.
    pub fn add_element(
        &mut self,
        parent: NodeId,
        qname: &str,
        attrs: &[(&str, &str)],
    ) -> Result<NodeId, XmlError> {
        self.check_container(parent)?;
        if parent == NodeId::DOCUMENT && self.root_element().is_some() {
            return Err(invalid("a document has only one root element"));
        }
        let id = self.create_element(qname, attrs, Some(parent))?;
        self.append_child(parent, id)?;
        Ok(id)
    }

    /// `node.add_text(s)`: append a text node. Returns it.
    pub fn add_text(&mut self, parent: NodeId, text: &str) -> Result<NodeId, XmlError> {
        self.check_container(parent)?;
        let id = self.create_text(text)?;
        self.append_child(parent, id)?;
        Ok(id)
    }

    /// `node.add_cdata(s)`.
    pub fn add_cdata(&mut self, parent: NodeId, text: &str) -> Result<NodeId, XmlError> {
        self.check_container(parent)?;
        let id = self.create_cdata(text)?;
        self.append_child(parent, id)?;
        Ok(id)
    }

    /// `node.add_comment(s)`.
    pub fn add_comment(&mut self, parent: NodeId, text: &str) -> Result<NodeId, XmlError> {
        self.check_container(parent)?;
        let id = self.create_comment(text)?;
        self.append_child(parent, id)?;
        Ok(id)
    }

    /// `node.add_pi(target, data)`.
    pub fn add_pi(&mut self, parent: NodeId, target: &str, data: &str) -> Result<NodeId, XmlError> {
        self.check_container(parent)?;
        let id = self.create_pi(target, data)?;
        self.append_child(parent, id)?;
        Ok(id)
    }

    // ------------------------------------------------------------- mutation

    fn check_container(&self, parent: NodeId) -> Result<(), XmlError> {
        match self.kind(parent) {
            Kind::Element { .. } | Kind::Document => Ok(()),
            other => Err(invalid(&format!(
                "a {} node cannot have children",
                other.name()
            ))),
        }
    }

    /// Can `child` go under `parent`? Kind rules, the one-root rule of a
    /// document, and the cycle check.
    fn check_insert(&self, parent: NodeId, child: NodeId) -> Result<(), XmlError> {
        self.check_container(parent)?;
        if child == NodeId::DOCUMENT {
            return Err(invalid("the document node cannot be inserted"));
        }
        if self.is_ancestor_or_self(child, parent) {
            return Err(XmlError::new(XmlErrorKind::Cycle));
        }
        if parent == NodeId::DOCUMENT {
            match self.kind(child) {
                Kind::Element { .. } => {
                    if self.root_element().is_some_and(|root| root != child) {
                        return Err(invalid("a document has only one root element"));
                    }
                }
                Kind::Comment(_) | Kind::Pi { .. } => {}
                _ => {
                    return Err(invalid(
                        "a document holds one root element, comments and processing instructions",
                    ))
                }
            }
        }
        Ok(())
    }

    /// `node.remove`: detach `id` from its parent. The node and its subtree
    /// stay in the arena as a fragment. Detaching a detached node does
    /// nothing; the document node cannot be detached.
    pub fn detach(&mut self, id: NodeId) -> Result<NodeId, XmlError> {
        if id == NodeId::DOCUMENT {
            return Err(invalid("the document node cannot be removed"));
        }
        if let Some((parent, pos)) = self.index_in_parent(id) {
            self.node_mut(parent).children.remove(pos);
            self.node_mut(id).parent = None;
            self.order_dirty = true;
            self.version += 1;
        }
        Ok(id)
    }

    /// Insert `child` (a node of this tree, attached or not) into `parent`'s
    /// children at `index`. A node that is attached elsewhere is moved.
    fn insert_at(
        &mut self,
        parent: NodeId,
        index: usize,
        child: NodeId,
    ) -> Result<NodeId, XmlError> {
        self.check_insert(parent, child)?;
        let mut index = index;
        if let Some((old_parent, pos)) = self.index_in_parent(child) {
            if old_parent == parent && pos < index {
                index -= 1;
            }
        }
        self.detach(child)?;
        let len = self.children(parent).len();
        self.node_mut(parent).children.insert(index.min(len), child);
        self.node_mut(child).parent = Some(parent);
        self.order_dirty = true;
        self.version += 1;
        Ok(child)
    }

    /// `parent.append(child)`: move `child` (same tree) to the end of
    /// `parent`'s children. Raises `would create a cycle` when `child` is
    /// `parent` or one of its ancestors.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<NodeId, XmlError> {
        let len = self.children(parent).len();
        self.insert_at(parent, len, child)
    }

    /// `parent.prepend(child)`.
    pub fn prepend_child(&mut self, parent: NodeId, child: NodeId) -> Result<NodeId, XmlError> {
        self.insert_at(parent, 0, child)
    }

    /// `reference.insert_before(node)`: `reference` must have a parent.
    pub fn insert_before(&mut self, reference: NodeId, node: NodeId) -> Result<NodeId, XmlError> {
        if node == reference {
            return Ok(node);
        }
        let (parent, pos) = self
            .index_in_parent(reference)
            .ok_or_else(|| invalid("insert_before needs a node that has a parent"))?;
        self.insert_at(parent, pos, node)
    }

    /// `reference.insert_after(node)`: `reference` must have a parent.
    pub fn insert_after(&mut self, reference: NodeId, node: NodeId) -> Result<NodeId, XmlError> {
        if node == reference {
            return Ok(node);
        }
        let (parent, pos) = self
            .index_in_parent(reference)
            .ok_or_else(|| invalid("insert_after needs a node that has a parent"))?;
        self.insert_at(parent, pos + 1, node)
    }

    /// `old.replace(new)`: put `new` where `old` is and detach `old`. Returns
    /// `new`.
    pub fn replace(&mut self, old: NodeId, new: NodeId) -> Result<NodeId, XmlError> {
        if old == new {
            return Ok(new);
        }
        let (parent, _) = self
            .index_in_parent(old)
            .ok_or_else(|| invalid("replace needs a node that has a parent"))?;
        if new == NodeId::DOCUMENT {
            return Err(invalid("the document node cannot be inserted"));
        }
        if self.is_ancestor_or_self(new, parent) || self.is_ancestor_or_self(new, old) {
            return Err(XmlError::new(XmlErrorKind::Cycle));
        }
        // Check everything before touching the tree, as if `old` were already
        // gone, so a refused replace changes nothing.
        if parent == NodeId::DOCUMENT {
            match self.kind(new) {
                Kind::Element { .. } => {
                    let root = self.root_element();
                    if root.is_some_and(|r| r != old && r != new) {
                        return Err(invalid("a document has only one root element"));
                    }
                }
                Kind::Comment(_) | Kind::Pi { .. } => {}
                _ => {
                    return Err(invalid(
                        "a document holds one root element, comments and processing instructions",
                    ))
                }
            }
        }
        // `new` may sit before `old` under the same parent: detach it first,
        // then find `old`'s position.
        self.detach(new)?;
        let pos = self
            .index_in_parent(old)
            .map(|(_, p)| p)
            .ok_or_else(|| invalid("replace needs a node that has a parent"))?;
        self.detach(old)?;
        self.insert_at(parent, pos, new)
    }

    /// `node["name"] = value`: set an attribute (forbidden characters dropped),
    /// resolving its prefix in `id`'s scope; `None` removes it. Returns the
    /// previous value. `xmlns` / `xmlns:p` are refused: declarations are made
    /// when an element is created.
    pub fn set_attribute(
        &mut self,
        id: NodeId,
        qname: &str,
        value: Option<&str>,
    ) -> Result<Option<String>, XmlError> {
        if qname == "xmlns" || qname.starts_with("xmlns:") {
            return Err(invalid(
                "namespace declarations are made when an element is created",
            ));
        }
        let Some(value) = value else {
            return self.remove_attribute(id, qname);
        };
        if self.element(id).is_none() {
            return Err(invalid("only an element has attributes"));
        }
        check_qname(qname)?;
        let ns_uri = match split_qname(qname).0 {
            Some(p) => Some(
                self.lookup_namespace(id, Some(p))
                    .ok_or_else(|| XmlError::new(XmlErrorKind::UnboundPrefix(p.to_string())))?,
            ),
            None => None,
        };
        let attr = Attr {
            qname: qname.into(),
            ns_uri,
            value: drop_forbidden(value).into(),
        };
        let Kind::Element { attrs, .. } = &mut self.node_mut(id).kind else {
            return Err(invalid("only an element has attributes"));
        };
        let previous = if let Some(slot) = attrs.iter_mut().find(|a| a.qname == attr.qname) {
            let old = std::mem::replace(&mut slot.value, attr.value);
            slot.ns_uri = attr.ns_uri;
            Some(old.into_string())
        } else {
            if attrs.iter().any(|a| same_expanded_name(a, &attr)) {
                return Err(XmlError::new(XmlErrorKind::DuplicateAttribute(
                    qname.to_string(),
                )));
            }
            let limit = max_attrs();
            if attrs.len() >= limit {
                return Err(XmlError::new(XmlErrorKind::AttrsExceeded(limit)));
            }
            attrs.push(attr);
            None
        };
        self.version += 1;
        Ok(previous)
    }

    /// `node.remove_attribute(name)`: the removed value, or `None`.
    pub fn remove_attribute(
        &mut self,
        id: NodeId,
        qname: &str,
    ) -> Result<Option<String>, XmlError> {
        let Kind::Element { attrs, .. } = &mut self.node_mut(id).kind else {
            return Ok(None);
        };
        let Some(pos) = attrs.iter().position(|a| &*a.qname == qname) else {
            return Ok(None);
        };
        let removed = attrs.remove(pos);
        self.version += 1;
        Ok(Some(removed.value.into_string()))
    }

    /// `node.text = s`. On an element: every child is detached and replaced
    /// by one text node (none for an empty string). On a text, CDATA or
    /// comment node: its content; on a PI: its data.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Result<(), XmlError> {
        let text = drop_forbidden(text);
        match self.kind(id) {
            Kind::Document => Err(invalid("the document node has no text of its own")),
            Kind::Element { .. } => {
                // Create first: a refused node (limit) leaves the children be.
                let node = if text.is_empty() {
                    None
                } else {
                    Some(self.push_node(Kind::Text(text.into()))?)
                };
                let old = std::mem::take(&mut self.node_mut(id).children);
                for child in old {
                    self.node_mut(child).parent = None;
                }
                if let Some(node) = node {
                    self.node_mut(node).parent = Some(id);
                    self.node_mut(id).children.push(node);
                }
                self.order_dirty = true;
                self.version += 1;
                Ok(())
            }
            Kind::Text(_) => self.set_kind(id, Kind::Text(text.into())),
            Kind::CData(_) => self.set_kind(id, Kind::CData(normalize_eol(&text).into())),
            Kind::Comment(_) => {
                let text = normalize_eol(&text);
                check_comment(&text)?;
                self.set_kind(id, Kind::Comment(text.into()))
            }
            Kind::Pi { target, .. } => {
                let target = target.clone();
                let text = normalize_eol(&text);
                check_pi(&target, &text)?;
                self.set_kind(
                    id,
                    Kind::Pi {
                        target,
                        data: text.into(),
                    },
                )
            }
        }
    }

    fn set_kind(&mut self, id: NodeId, kind: Kind) -> Result<(), XmlError> {
        self.node_mut(id).kind = kind;
        self.version += 1;
        Ok(())
    }

    /// `node.name = s`: rename an element (prefix resolved in its scope) or
    /// retarget a PI. The name is validated.
    pub fn rename(&mut self, id: NodeId, qname: &str) -> Result<(), XmlError> {
        match self.kind(id) {
            Kind::Element { .. } => {
                check_qname(qname)?;
                let prefix = split_qname(qname).0;
                if prefix == Some("xmlns") {
                    return Err(XmlError::new(XmlErrorKind::InvalidName(qname.to_string())));
                }
                let uri = self.lookup_namespace(id, prefix);
                if let (Some(p), None) = (prefix, &uri) {
                    return Err(XmlError::new(XmlErrorKind::UnboundPrefix(p.to_string())));
                }
                if let Kind::Element {
                    qname: name,
                    local_start,
                    ns_uri,
                    ..
                } = &mut self.node_mut(id).kind
                {
                    *name = qname.into();
                    *local_start = local_start_of(qname);
                    *ns_uri = uri;
                }
                self.version += 1;
                Ok(())
            }
            Kind::Pi { data, .. } => {
                let data = data.clone();
                check_pi(qname, &data)?;
                self.set_kind(
                    id,
                    Kind::Pi {
                        target: qname.into(),
                        data,
                    },
                )
            }
            other => Err(invalid(&format!("a {} node has no name", other.name()))),
        }
    }

    /// Copy `src`'s subtree at `src_id` into this tree as a detached fragment
    /// (`append` of a node from **another** tree). Returns the copy's id. The
    /// document node cannot be imported.
    pub fn import(&mut self, src: &XmlTree, src_id: NodeId) -> Result<NodeId, XmlError> {
        if src_id == NodeId::DOCUMENT {
            return Err(invalid("a document cannot be inserted into another tree"));
        }
        let copy = self.copy_subtree_from(src, src_id, None)?;
        self.order_dirty = true;
        self.version += 1;
        Ok(copy)
    }

    /// Copy `src_id`'s subtree from `src` into `self`, under `parent` (or
    /// detached). Iterative: the source may be arbitrarily deep.
    fn copy_subtree_from(
        &mut self,
        src: &XmlTree,
        src_id: NodeId,
        parent: Option<NodeId>,
    ) -> Result<NodeId, XmlError> {
        let mut stack: Vec<(NodeId, Option<NodeId>)> = vec![(src_id, parent)];
        let mut top = None;
        while let Some((from, to_parent)) = stack.pop() {
            let id = self.push_node(src.kind(from).clone())?;
            if let Some(p) = to_parent {
                self.node_mut(id).parent = Some(p);
                self.node_mut(p).children.push(id);
            }
            if top.is_none() {
                top = Some(id);
            }
            for child in src.children(from).iter().rev() {
                stack.push((*child, Some(id)));
            }
        }
        top.ok_or_else(|| invalid("nothing to copy"))
    }

    /// `node.dup`: a deep copy into a fresh, compacted tree (no tombstones,
    /// same node limit). For the document node the whole document is copied,
    /// declaration included, and the result is `NodeId::DOCUMENT`; for any
    /// other node the copy is a detached fragment and the result its id.
    pub fn dup(&self, id: NodeId) -> Result<(XmlTree, NodeId), XmlError> {
        let mut tree = XmlTree::new(self.node_limit);
        if id == NodeId::DOCUMENT {
            tree.decl = self.decl.clone();
            tree.doctype = self.doctype.clone();
            for child in self.children(NodeId::DOCUMENT) {
                tree.copy_subtree_from(self, *child, Some(NodeId::DOCUMENT))?;
            }
            tree.order_dirty = true;
            tree.ensure_order();
            return Ok((tree, NodeId::DOCUMENT));
        }
        let copy = tree.copy_subtree_from(self, id, None)?;
        tree.order_dirty = true;
        tree.ensure_order();
        Ok((tree, copy))
    }

    /// Structural equality of two subtrees (kinds, names, namespaces,
    /// attributes, declarations, self-closing bits and text), ignoring node
    /// ids. The round-trip tests and the fuzz target use it.
    pub fn subtree_eq(&self, a: NodeId, other: &XmlTree, b: NodeId) -> bool {
        let mut stack = vec![(a, b)];
        while let Some((x, y)) = stack.pop() {
            let (cx, cy) = (self.children(x), other.children(y));
            if cx.len() != cy.len() || !kinds_eq(self.kind(x), other.kind(y), cx.is_empty()) {
                return false;
            }
            stack.extend(cx.iter().copied().zip(cy.iter().copied()));
        }
        true
    }
}

/// What `el` binds `prefix` to, if anything: `Some(Some(uri))` bound,
/// `Some(None)` unbound here (an undeclaration, or an unprefixed element in
/// no namespace), `None` when `el` says nothing about `prefix`. A
/// declaration wins, then the element's own name, then its first attribute
/// using the prefix — the order the serialiser settles prefixes in.
fn binding_at(el: &ElementRef<'_>, prefix: Option<&str>) -> Option<Option<Arc<str>>> {
    if let Some(decl) = el.ns_decls.iter().find(|d| d.prefix.as_deref() == prefix) {
        return Some((!decl.uri.is_empty()).then(|| decl.uri.clone()));
    }
    if el.prefix == prefix {
        return match (prefix, el.ns_uri) {
            // A prefix a lenient parse left unbound binds nothing.
            (Some(_), None) => None,
            (_, uri) => Some(uri.cloned()),
        };
    }
    prefix?;
    el.attrs
        .iter()
        .find(|a| a.prefix() == prefix && a.ns_uri.is_some())
        .map(|a| a.ns_uri.clone())
}

/// `SOLI_MAX_XML_ATTRS`, for built elements as for parsed ones.
fn max_attrs() -> usize {
    usize::try_from(crate::interpreter::limits::max_xml_attrs()).unwrap_or(usize::MAX)
}

/// Kind equality for [`XmlTree::subtree_eq`]: the self-closing bit only
/// matters on an element with no children (it is how an empty one is written).
fn kinds_eq(a: &Kind, b: &Kind, empty: bool) -> bool {
    match (a, b) {
        (
            Kind::Element {
                qname: qa,
                ns_uri: na,
                attrs: aa,
                ns_decls: da,
                self_closing: sa,
                ..
            },
            Kind::Element {
                qname: qb,
                ns_uri: nb,
                attrs: ab,
                ns_decls: db,
                self_closing: sb,
                ..
            },
        ) => qa == qb && na == nb && aa == ab && da == db && (!empty || sa == sb),
        _ => a == b,
    }
}

/// Namespace declarations one built element may make: quick-xml's per-element
/// cap, so whatever is built can be parsed back.
const MAX_BUILT_DECLS: usize = 256;

fn push_decl(decls: &mut Vec<NsDecl>, prefix: Option<&str>, uri: &str) -> Result<(), XmlError> {
    if decls.len() >= MAX_BUILT_DECLS {
        return Err(invalid(&format!(
            "more than {MAX_BUILT_DECLS} namespace declarations on one element"
        )));
    }
    if decls.iter().any(|d| d.prefix.as_deref() == prefix) {
        let name = match prefix {
            Some(p) => format!("xmlns:{p}"),
            None => "xmlns".to_string(),
        };
        return Err(XmlError::new(XmlErrorKind::DuplicateAttribute(name)));
    }
    if prefix.is_some() && uri.is_empty() {
        return Err(invalid(
            "a namespace prefix cannot be bound to an empty URI",
        ));
    }
    if prefix == Some("xml") && uri != XML_NS {
        return Err(invalid(
            "the 'xml' prefix is bound to the XML namespace only",
        ));
    }
    decls.push(NsDecl {
        prefix: prefix.map(Box::from),
        uri: Arc::from(drop_forbidden(uri).as_ref()),
    });
    Ok(())
}

/// Same expanded name: same namespace and local name when either is
/// namespaced, otherwise the same qname.
pub(crate) fn same_expanded_name(a: &Attr, b: &Attr) -> bool {
    match (&a.ns_uri, &b.ns_uri) {
        (Some(x), Some(y)) => x == y && a.local_name() == b.local_name(),
        (None, None) => a.qname == b.qname,
        _ => false,
    }
}

/// Check a set of attributes for duplicate expanded names, in O(n) beyond a
/// small count (the parser calls this with up to `max_attrs` attributes).
pub(crate) fn find_duplicate(attrs: &[Attr]) -> Option<&Attr> {
    if attrs.len() <= 16 {
        for (i, a) in attrs.iter().enumerate() {
            if attrs[..i].iter().any(|b| same_expanded_name(a, b)) {
                return Some(a);
            }
        }
        return None;
    }
    let mut seen: HashSet<(Option<&str>, &str)> = HashSet::with_capacity(attrs.len());
    for a in attrs {
        let key = match &a.ns_uri {
            Some(uri) => (Some(&**uri), a.local_name()),
            None => (None, &*a.qname),
        };
        if !seen.insert(key) {
            return Some(a);
        }
    }
    None
}

fn check_comment(text: &str) -> Result<(), XmlError> {
    if text.contains("--") || text.ends_with('-') {
        return Err(XmlError::new(XmlErrorKind::InvalidComment));
    }
    Ok(())
}

fn check_pi(target: &str, data: &str) -> Result<(), XmlError> {
    if target.len() > MAX_NAME_LEN
        || !is_ncname(target)
        || target.eq_ignore_ascii_case("xml")
        || data.contains("?>")
    {
        return Err(XmlError::new(XmlErrorKind::InvalidPi(target.to_string())));
    }
    Ok(())
}
