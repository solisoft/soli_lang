//! The XPath evaluator. Node sets are `Vec<NodeId>` kept in document order
//! without duplicates after every step; a bitset over the arena deduplicates
//! the steps that could repeat a node (`//`, `..`).
//!
//! A detached fragment (`Xml.element`, `node.remove`, `dup`) has no document
//! node. Absolute paths on it start at a *virtual* root, [`VIRTUAL`], whose
//! only child is the fragment's top node, so `//item` and `/a/b` mean what
//! they would if the fragment were a document. The virtual root is never
//! returned.

use std::collections::HashMap;
use std::sync::Arc;

use super::parse::{Expr, NameTest, NodeTest, Operand, Path, Pred, Start, Step};
use super::{Flavour, Item, Query, XPath, XPathError, XPathErrorKind};
use crate::xml::tree::{Attr, ElementRef, Kind, NodeId, XmlTree, XML_NS};

/// The virtual root of a detached fragment.
const VIRTUAL: NodeId = NodeId(u32::MAX);

/// Bytes of text a string-value copies per visit charged.
pub(super) const TEXT_BYTES_PER_VISIT: usize = 64;

/// A node's children: a slice of the arena, or the virtual root's one
/// child.
#[derive(Clone, Copy)]
enum Kids<'t> {
    Arena(&'t [NodeId]),
    One([NodeId; 1]),
}

impl Kids<'_> {
    fn as_slice(&self) -> &[NodeId] {
        match self {
            Kids::Arena(slice) => slice,
            Kids::One(one) => one,
        }
    }
}

/// A resolved namespace: `None` is "no namespace".
type Ns = Option<Arc<str>>;

struct Eval<'t> {
    tree: &'t XmlTree,
    flavour: Flavour,
    /// The table's `""` key: `None` without one, `Some(ns)` with one.
    default_ns: Option<Ns>,
    prefixes: HashMap<String, Ns>,
    visits: u64,
    budget: u64,
    /// One bit per arena slot, plus one for [`VIRTUAL`].
    seen: Vec<u64>,
    /// The top of the context node's fragment (the document node when it is
    /// attached).
    frag_root: NodeId,
    /// `true` when the fragment is detached, so it hangs from [`VIRTUAL`].
    detached: bool,
    /// Arena size: also [`VIRTUAL`]'s bit.
    arena: usize,
}

fn same_ns(a: Option<&Arc<str>>, b: &Ns) -> bool {
    a.map(|s| &**s) == b.as_deref()
}

fn ns_of(uri: &str) -> Ns {
    if uri.is_empty() {
        None
    } else {
        Some(Arc::from(uri))
    }
}

/// The element whose bindings back the root-declaration fallback: the
/// document's root element, or the fragment's top element.
fn fallback_element(tree: &XmlTree, frag_root: NodeId) -> Option<NodeId> {
    if tree.is_element(frag_root) {
        Some(frag_root)
    } else if frag_root == NodeId::DOCUMENT {
        tree.root_element()
    } else {
        None
    }
}

/// `prefix`'s URI: the caller's table, then the root element's bindings,
/// then `xmlns` as the root's default namespace.
fn resolve(
    tree: &XmlTree,
    root: Option<NodeId>,
    table: &[(String, String)],
    prefix: &str,
) -> Result<Ns, XPathError> {
    if let Some((_, uri)) = table.iter().find(|(k, _)| k == prefix) {
        return Ok(ns_of(uri));
    }
    if prefix == "xml" {
        return Ok(Some(Arc::from(XML_NS)));
    }
    if let Some(root) = root {
        if let Some(uri) = tree.lookup_namespace(root, Some(prefix)) {
            return Ok(ns_of(&uri));
        }
        if prefix == "xmlns" {
            if let Some(uri) = tree.lookup_namespace(root, None) {
                return Ok(ns_of(&uri));
            }
        }
    }
    Err(XPathError {
        kind: XPathErrorKind::UnboundPrefix(prefix.to_string()),
    })
}

pub(super) fn evaluate(
    tree: &XmlTree,
    context: NodeId,
    xpath: &XPath,
    query: &Query<'_>,
) -> Result<Vec<Item>, XPathError> {
    let frag_root = tree.fragment_root(context);
    let root = fallback_element(tree, frag_root);
    let mut prefixes = HashMap::new();
    for prefix in xpath.prefixes() {
        prefixes.insert(
            prefix.clone(),
            resolve(tree, root, query.namespaces, prefix)?,
        );
    }
    let default_ns = query
        .namespaces
        .iter()
        .find(|(k, _)| k.is_empty())
        .map(|(_, uri)| ns_of(uri));
    let arena = tree.node_count() + 1;
    let mut ev = Eval {
        tree,
        flavour: query.flavour,
        default_ns,
        prefixes,
        visits: 0,
        budget: query.max_visits,
        seen: vec![0; arena / 64 + 1],
        frag_root,
        detached: frag_root != NodeId::DOCUMENT,
        arena,
    };
    let mut all = Vec::new();
    for path in &xpath.paths {
        all.extend(ev.path(path, context)?);
    }
    if xpath.paths.len() > 1 {
        all.sort_unstable_by_key(|item| ev.item_key(item));
        all.dedup();
    }
    Ok(all)
}

impl<'t> Eval<'t> {
    fn charge(&mut self, n: usize) -> Result<(), XPathError> {
        self.visits = self.visits.saturating_add(n as u64);
        if self.visits > self.budget {
            return Err(XPathError {
                kind: XPathErrorKind::Budget(self.budget),
            });
        }
        Ok(())
    }

    fn slot(&self, id: NodeId) -> usize {
        if id == VIRTUAL {
            self.arena
        } else {
            id.index()
        }
    }

    fn clear_seen(&mut self) {
        self.seen.iter_mut().for_each(|w| *w = 0);
    }

    /// Mark `id`; `false` if it was already marked.
    fn mark(&mut self, id: NodeId) -> bool {
        let slot = self.slot(id);
        let (word, bit) = (slot / 64, 1u64 << (slot % 64));
        match self.seen.get_mut(word) {
            Some(w) if *w & bit != 0 => false,
            Some(w) => {
                *w |= bit;
                true
            }
            None => true,
        }
    }

    fn is_seen(&self, id: NodeId) -> bool {
        let slot = self.slot(id);
        self.seen
            .get(slot / 64)
            .is_some_and(|w| w & (1u64 << (slot % 64)) != 0)
    }

    /// Document-order key: the virtual root first, then the pre-order index.
    fn key(&self, id: NodeId) -> u64 {
        if id == VIRTUAL {
            0
        } else {
            u64::from(self.tree.order(id)) + 1
        }
    }

    fn item_key(&self, item: &Item) -> (u64, u64, u8) {
        match *item {
            Item::Node(id) => (self.key(id), 0, 0),
            Item::Text(id) => (self.key(id), 0, 1),
            // An element's attributes come after it and before its children.
            Item::Attr { element, index } => (self.key(element), index as u64 + 1, 0),
        }
    }

    fn sort(&self, set: &mut [NodeId]) {
        if !set.windows(2).all(|w| self.key(w[0]) < self.key(w[1])) {
            set.sort_unstable_by_key(|id| self.key(*id));
        }
    }

    fn children(&self, id: NodeId) -> Kids<'t> {
        if id == VIRTUAL {
            Kids::One([self.frag_root])
        } else {
            Kids::Arena(self.tree.children(id))
        }
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        if id == VIRTUAL {
            None
        } else if self.detached && id == self.frag_root {
            Some(VIRTUAL)
        } else {
            self.tree.parent(id)
        }
    }

    fn element(&self, id: NodeId) -> Option<ElementRef<'t>> {
        if id == VIRTUAL {
            None
        } else {
            self.tree.element(id)
        }
    }

    fn ns(&self, prefix: &str) -> Option<&Ns> {
        self.prefixes.get(prefix)
    }

    fn element_matches(&self, test: &NameTest, el: &ElementRef<'_>) -> bool {
        match test {
            NameTest::Any => true,
            NameTest::Name {
                prefix: None,
                local,
            } => {
                el.local_name == local
                    && match &self.default_ns {
                        Some(ns) => same_ns(el.ns_uri, ns),
                        None => self.flavour == Flavour::Lenient || el.ns_uri.is_none(),
                    }
            }
            NameTest::Name {
                prefix: Some(p),
                local,
            } => el.local_name == local && self.ns(p).is_some_and(|ns| same_ns(el.ns_uri, ns)),
            NameTest::Prefix(p) => self.ns(p).is_some_and(|ns| same_ns(el.ns_uri, ns)),
            NameTest::Local(local) => el.local_name == local,
        }
    }

    fn attr_matches(&self, test: &NameTest, attr: &Attr) -> bool {
        match test {
            NameTest::Any => true,
            NameTest::Name {
                prefix: None,
                local,
            } => attr.prefix().is_none() && attr.local_name() == local,
            NameTest::Name {
                prefix: Some(p),
                local,
            } => {
                attr.local_name() == local
                    && self
                        .ns(p)
                        .is_some_and(|ns| same_ns(attr.ns_uri.as_ref(), ns))
            }
            NameTest::Prefix(p) => {
                attr.prefix().is_some()
                    && self
                        .ns(p)
                        .is_some_and(|ns| same_ns(attr.ns_uri.as_ref(), ns))
            }
            NameTest::Local(local) => attr.local_name() == local,
        }
    }

    fn node_matches(&self, id: NodeId, test: &NodeTest) -> bool {
        if id == VIRTUAL {
            return false;
        }
        match (test, self.tree.kind(id)) {
            (NodeTest::Name(name), Kind::Element { .. }) => self
                .tree
                .element(id)
                .is_some_and(|el| self.element_matches(name, &el)),
            (NodeTest::Text, Kind::Text(_) | Kind::CData(_)) => true,
            (NodeTest::Node, kind) => !matches!(kind, Kind::Document),
            (NodeTest::Comment, Kind::Comment(_)) => true,
            (NodeTest::Pi(None), Kind::Pi { .. }) => true,
            (NodeTest::Pi(Some(want)), Kind::Pi { target, .. }) => **target == **want,
            _ => false,
        }
    }

    fn path(&mut self, path: &Path, context: NodeId) -> Result<Vec<Item>, XPathError> {
        let mut set: Vec<NodeId> = match path.start {
            Start::Root if self.detached => vec![VIRTUAL],
            Start::Root => vec![NodeId::DOCUMENT],
            Start::Relative if self.flavour == Flavour::Lenient && context == NodeId::DOCUMENT => {
                self.tree.root_element().into_iter().collect()
            }
            Start::Relative => vec![context],
        };
        let count = path.steps.len();
        for (i, step) in path.steps.iter().enumerate() {
            if set.is_empty() {
                return Ok(Vec::new());
            }
            let last = i + 1 == count;
            match step {
                Step::Attr(test) => return self.attributes(&set, test),
                Step::Child(NodeTest::Text, preds) if last => {
                    let texts = self.child(&set, &NodeTest::Text, preds)?;
                    return Ok(texts.into_iter().map(Item::Text).collect());
                }
                Step::SelfNode => {}
                Step::Parent => set = self.parents(&set)?,
                Step::DescOrSelf => set = self.desc_or_self(&set)?,
                Step::Child(test, preds) => set = self.child(&set, test, preds)?,
            }
        }
        Ok(set
            .into_iter()
            .filter(|id| *id != VIRTUAL)
            .map(Item::Node)
            .collect())
    }

    fn attributes(&mut self, set: &[NodeId], test: &NameTest) -> Result<Vec<Item>, XPathError> {
        let mut out = Vec::new();
        for &id in set {
            let Some(el) = self.element(id) else {
                continue;
            };
            self.charge(el.attrs.len())?;
            for (index, attr) in el.attrs.iter().enumerate() {
                if self.attr_matches(test, attr) {
                    out.push(Item::Attr { element: id, index });
                }
            }
        }
        Ok(out)
    }

    fn parents(&mut self, set: &[NodeId]) -> Result<Vec<NodeId>, XPathError> {
        self.charge(set.len())?;
        self.clear_seen();
        let mut out = Vec::new();
        for &id in set {
            if let Some(parent) = self.parent(id) {
                if self.mark(parent) {
                    out.push(parent);
                }
            }
        }
        self.sort(&mut out);
        Ok(out)
    }

    /// Every node of every context's subtree, the context included. The set
    /// is in document order, so a context already reached as a descendant
    /// of an earlier one is skipped with its whole subtree: each node is
    /// visited once.
    fn desc_or_self(&mut self, set: &[NodeId]) -> Result<Vec<NodeId>, XPathError> {
        self.clear_seen();
        let mut out = Vec::new();
        let mut stack = Vec::new();
        for &context in set {
            if self.is_seen(context) {
                continue;
            }
            stack.push(context);
            while let Some(node) = stack.pop() {
                self.charge(1)?;
                self.mark(node);
                out.push(node);
                stack.extend(self.children(node).as_slice().iter().rev().copied());
            }
        }
        Ok(out)
    }

    fn child(
        &mut self,
        set: &[NodeId],
        test: &NodeTest,
        preds: &[Pred],
    ) -> Result<Vec<NodeId>, XPathError> {
        let mut out = Vec::new();
        for &context in set {
            let kids = self.children(context);
            let children = kids.as_slice();
            self.charge(children.len())?;
            let mut candidates: Vec<NodeId> = children
                .iter()
                .copied()
                .filter(|c| self.node_matches(*c, test))
                .collect();
            for pred in preds {
                if candidates.is_empty() {
                    break;
                }
                candidates = self.apply(candidates, pred)?;
            }
            out.extend(candidates);
        }
        // Each node has one parent and the contexts are distinct, so there
        // are no duplicates; only the order may need fixing.
        self.sort(&mut out);
        Ok(out)
    }

    fn apply(&mut self, candidates: Vec<NodeId>, pred: &Pred) -> Result<Vec<NodeId>, XPathError> {
        match pred {
            Pred::Index(n) => Ok(usize::try_from(*n)
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| candidates.get(i).copied())
                .into_iter()
                .collect()),
            Pred::Last => Ok(candidates.last().copied().into_iter().collect()),
            Pred::Expr(expr) => {
                let size = candidates.len();
                let mut out = Vec::new();
                for (i, id) in candidates.iter().enumerate() {
                    if self.test(expr, *id, i as u64 + 1, size)? {
                        out.push(*id);
                    }
                }
                Ok(out)
            }
        }
    }

    fn test(
        &mut self,
        expr: &Expr,
        node: NodeId,
        position: u64,
        size: usize,
    ) -> Result<bool, XPathError> {
        Ok(match expr {
            Expr::Or(items) => {
                for item in items {
                    if self.test(item, node, position, size)? {
                        return Ok(true);
                    }
                }
                false
            }
            Expr::And(items) => {
                for item in items {
                    if !self.test(item, node, position, size)? {
                        return Ok(false);
                    }
                }
                true
            }
            Expr::Not(inner) => !self.test(inner, node, position, size)?,
            Expr::Position(op, n) => op.holds(position, *n),
            Expr::Exists(operand) => self.any_value(operand, node, None)?,
            Expr::Cmp {
                operand,
                eq,
                literal,
            } => {
                let want = *eq;
                self.any_value(
                    operand,
                    node,
                    Some(&|v: &str| (v == literal.as_str()) == want),
                )?
            }
            Expr::Contains(operand, literal) => {
                self.first_value(operand, node)?.contains(literal.as_str())
            }
            Expr::StartsWith(operand, literal) => self
                .first_value(operand, node)?
                .starts_with(literal.as_str()),
        })
    }

    /// Does `operand` select a value from `node` that `pred` accepts? With
    /// no `pred`, does it select anything (no string is computed)?
    fn any_value(
        &mut self,
        operand: &Operand,
        node: NodeId,
        pred: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<bool, XPathError> {
        match operand {
            Operand::Attr(test) => {
                let Some(el) = self.element(node) else {
                    return Ok(false);
                };
                self.charge(el.attrs.len())?;
                Ok(el
                    .attrs
                    .iter()
                    .any(|a| self.attr_matches(test, a) && pred.is_none_or(|p| p(&a.value))))
            }
            Operand::Text => {
                let kids = self.children(node);
                let children = kids.as_slice();
                self.charge(children.len())?;
                Ok(children.iter().any(|c| match self.tree.kind(*c) {
                    Kind::Text(s) | Kind::CData(s) => pred.is_none_or(|p| p(s)),
                    _ => false,
                }))
            }
            Operand::Dot => match pred {
                None => Ok(true),
                Some(p) => Ok(p(&self.string_value(node)?)),
            },
            Operand::Child(test) => {
                let kids = self.children(node);
                let children = kids.as_slice();
                self.charge(children.len())?;
                for &c in children {
                    let matches = self
                        .tree
                        .element(c)
                        .is_some_and(|el| self.element_matches(test, &el));
                    if !matches {
                        continue;
                    }
                    match pred {
                        None => return Ok(true),
                        Some(p) => {
                            if p(&self.string_value(c)?) {
                                return Ok(true);
                            }
                        }
                    }
                }
                Ok(false)
            }
        }
    }

    /// The first value `operand` selects from `node`, or `""`.
    fn first_value(&mut self, operand: &Operand, node: NodeId) -> Result<String, XPathError> {
        match operand {
            Operand::Attr(test) => {
                let Some(el) = self.element(node) else {
                    return Ok(String::new());
                };
                self.charge(el.attrs.len())?;
                Ok(el
                    .attrs
                    .iter()
                    .find(|a| self.attr_matches(test, a))
                    .map(|a| a.value.to_string())
                    .unwrap_or_default())
            }
            Operand::Text => {
                let kids = self.children(node);
                let children = kids.as_slice();
                self.charge(children.len())?;
                Ok(children
                    .iter()
                    .find_map(|c| match self.tree.kind(*c) {
                        Kind::Text(s) | Kind::CData(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .unwrap_or_default())
            }
            Operand::Dot => self.string_value(node),
            Operand::Child(test) => {
                let kids = self.children(node);
                let children = kids.as_slice();
                self.charge(children.len())?;
                let first = children.iter().copied().find(|c| {
                    self.tree
                        .element(*c)
                        .is_some_and(|el| self.element_matches(test, &el))
                });
                match first {
                    Some(c) => self.string_value(c),
                    None => Ok(String::new()),
                }
            }
        }
    }

    /// XPath string-value, each node read counted as a visit.
    fn string_value(&mut self, id: NodeId) -> Result<String, XPathError> {
        if id == VIRTUAL {
            return self.string_value(self.frag_root);
        }
        match self.tree.kind(id) {
            Kind::Text(s) | Kind::CData(s) | Kind::Comment(s) => Ok(s.to_string()),
            Kind::Pi { data, .. } => Ok(data.to_string()),
            Kind::Element { .. } | Kind::Document => {
                let mut out = String::new();
                let mut stack: Vec<NodeId> = self.tree.children(id).iter().rev().copied().collect();
                while let Some(node) = stack.pop() {
                    self.charge(1)?;
                    match self.tree.kind(node) {
                        Kind::Text(s) | Kind::CData(s) => {
                            // Copying text costs too: a long run read once per
                            // ancestor (`//*[.='x']`) must reach the budget.
                            self.charge(s.len() / TEXT_BYTES_PER_VISIT)?;
                            out.push_str(s);
                        }
                        _ => stack.extend(self.tree.children(node).iter().rev().copied()),
                    }
                }
                Ok(out)
            }
        }
    }
}
