//! Exclusive XML Canonicalization (exc-c14n) built-in class for SoliLang.
//!
//! Implements the W3C *Exclusive XML Canonicalization 1.0* algorithm
//! (<http://www.w3.org/2001/10/xml-exc-c14n#>), the canonical form used by
//! XML-DSig / SAML / WS-Security. Together with [`Crypto.modexp`] and the
//! PKCS#1 v1.5 padding helpers (`crypto.rs`) this is the third primitive
//! needed to build XML digital signatures in pure Soli.
//!
//! Exposed as:
//! - `Xml.c14n_exclusive(xml)` -> String
//! - `Xml.c14n_exclusive(xml, inclusive_prefixes)` -> String
//!
//! `inclusive_prefixes` is the *InclusiveNamespaces PrefixList* — a
//! space-separated string (e.g. `"ds saml"`) or an array of prefix strings.
//! Those prefixes are rendered as if visibly utilized, matching the
//! `<ec:InclusiveNamespaces PrefixList="...">` transform parameter.
//!
//! ## Scope / known limitations
//! - Comments are omitted (the default, no-comments exc-c14n form).
//! - The whole document element subtree of `xml` is canonicalized; node-set
//!   selection (signing one referenced element by Id) is left to the caller.
//! - Full XML attribute-value whitespace normalization (collapsing a literal
//!   tab/newline that is *not* from a character reference to a space) is not
//!   performed; such whitespace is escaped per the C14N character-escaping
//!   rules instead. This matches real-world XML-DSig inputs, which do not put
//!   raw control whitespace in attribute values.
//! - `DOCTYPE` declarations are rejected (XXE / entity-expansion defense,
//!   mirroring the SOAP parser).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
#[cfg(test)]
use std::ops::Deref;
use std::rc::Rc;

#[cfg(test)]
use quick_xml::events::{BytesStart, Event};
#[cfg(test)]
use quick_xml::{Reader, XmlVersion};

/// Decode + entity-unescape a text event.
///
/// quick-xml 0.41 removed `BytesText::unescape()` in favour of `decode()`
/// (bytes -> str) plus the free `escape::unescape()` (`&lt;` -> `<`). This
/// restores the original decode-then-resolve-predefined-entities behaviour
/// used when canonicalizing text content.
#[cfg(test)]
fn decode_text_event(e: &quick_xml::events::BytesText) -> Result<String, String> {
    let decoded = e
        .decode()
        .map_err(|err| format!("XML text error: {}", err))?;
    let unescaped =
        quick_xml::escape::unescape(&decoded).map_err(|err| format!("XML text error: {}", err))?;
    Ok(unescaped.into_owned())
}

/// Resolve a general entity reference (`Event::GeneralRef`) to its replacement
/// text.
///
/// quick-xml 0.41 surfaces even the five predefined entities (`amp`, `lt`,
/// `gt`, `apos`, `quot`) and numeric character references (`&#65;` / `&#x41;`)
/// as their own event rather than inlining them into `Text`. We resolve those
/// here; any other named entity would require a DTD (rejected up front as an
/// XXE / entity-expansion defense), so an unknown name is an error.
#[cfg(test)]
fn resolve_general_ref(e: &quick_xml::events::BytesRef) -> Result<String, String> {
    if e.is_char_ref() {
        return e
            .resolve_char_ref()
            .map_err(|err| format!("XML character reference error: {}", err))?
            .map(|c| c.to_string())
            .ok_or_else(|| "invalid numeric character reference".to_string());
    }
    let name = e
        .decode()
        .map_err(|err| format!("XML entity error: {}", err))?;
    quick_xml::escape::resolve_predefined_entity(&name)
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "general entity reference `&{};` is not allowed (XXE / entity-expansion defense)",
                name
            )
        })
}

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

/// The implicit `xml` prefix namespace, declared everywhere per the XML spec.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// The XML Signature namespace. Used by the enveloped-signature transform to
/// recognise (and drop) the `<ds:Signature>` element being verified.
const XMLDSIG_NS: &str = "http://www.w3.org/2000/09/xmldsig#";

/// Element-nesting cap — a deeply nested document is the classic XML DoS
/// vector. Generous enough for any real signed payload.
#[cfg(test)]
const MAX_DEPTH: usize = 256;

/// A regular (non-`xmlns`) attribute.
pub(crate) struct Attr {
    pub(crate) qname: String,
    pub(crate) prefix: String,
    pub(crate) local: String,
    pub(crate) value: String,
}

/// A parsed element with its namespace declarations, attributes and children.
pub(crate) struct Element {
    pub(crate) qname: String,
    pub(crate) prefix: String,
    pub(crate) local: String,
    pub(crate) ns_decls: Vec<(String, String)>, // (prefix or "" for default, uri)
    pub(crate) attrs: Vec<Attr>,
    pub(crate) children: Vec<Node>,
    /// Value of an `Id`/`ID`/`id` attribute, if any — used to resolve
    /// XML-DSig `Reference URI="#..."` selections.
    pub(crate) id: Option<String>,
}

pub(crate) enum Node {
    Element(Element),
    Text(String),
    Pi(String), // already-formatted "target content"
}

fn split_qname(q: &str) -> (String, String) {
    match q.split_once(':') {
        Some((p, l)) => (p.to_string(), l.to_string()),
        None => (String::new(), q.to_string()),
    }
}

#[cfg(test)]
fn bytes_lossy<T: Deref<Target = [u8]>>(b: &T) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Escape character content (rule: `&`, `<`, `>` and `#xD`).
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape an attribute value (rule: `&`, `<`, `"`, `#x9`, `#xA`, `#xD`).
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
fn build_element(e: &BytesStart) -> Result<Element, String> {
    let qname = bytes_lossy(&e.name().into_inner().to_vec());
    let (prefix, local) = split_qname(&qname);
    let mut ns_decls = Vec::new();
    let mut attrs = Vec::new();
    let mut id = None;

    for attr in e.attributes() {
        let attr = attr.map_err(|err| format!("XML attribute error: {}", err))?;
        let key = bytes_lossy(&attr.key.as_ref().to_vec());
        // quick-xml 0.41: `unescape_value()` is deprecated in favour of
        // `normalized_value()`, which for `Implicit1_0` performs the same
        // decode + predefined-entity resolution (plus XML-spec attribute-value
        // normalization, which a conformant parser applies before C14N anyway).
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| format!("XML attribute value error: {}", err))?
            .into_owned();
        if key == "xmlns" {
            ns_decls.push((String::new(), value));
        } else if let Some(p) = key.strip_prefix("xmlns:") {
            ns_decls.push((p.to_string(), value));
        } else {
            let (aprefix, alocal) = split_qname(&key);
            // No DTD/schema, so match the common ID-attribute spellings
            // (Id / ID / id) by local name — what real XML-DSig libraries do.
            if id.is_none() && alocal.eq_ignore_ascii_case("id") {
                id = Some(value.clone());
            }
            attrs.push(Attr {
                qname: key,
                prefix: aprefix,
                local: alocal,
                value,
            });
        }
    }
    Ok(Element {
        qname,
        prefix,
        local,
        ns_decls,
        attrs,
        children: Vec::new(),
        id,
    })
}

/// Parse `xml` into a tree, returning the single document element.
///
/// The shared XML core parses (`ParsePolicy::c14n()`: DOCTYPE refused, depth
/// 256, unbound prefixes allowed, trailing roots ignored, comments and text
/// outside the root dropped here) and the tree is copied into this module's
/// `Element`, built exactly as the old quick-xml loop built it. The old loop
/// is kept as `tests::legacy_parse_document`, the oracle the adapter is
/// checked against.
pub(crate) fn parse_document(xml: &str) -> Result<Element, String> {
    use crate::xml::{Kind, NodeId, ParsePolicy, XmlError, XmlErrorKind, XmlTree};

    /// The old loop's error strings, for the errors it could give.
    fn message(err: &XmlError) -> String {
        match &err.kind {
            XmlErrorKind::Doctype(message) | XmlErrorKind::NoRoot(message) => message.to_string(),
            XmlErrorKind::DepthExceeded(limit) => {
                format!("nesting depth exceeded {} levels", limit)
            }
            XmlErrorKind::UnknownEntity(name) => format!(
                "general entity reference `&{};` is not allowed (XXE / entity-expansion defense)",
                name
            ),
            _ => format!("XML parsing error: {}", err.quick_xml_message()),
        }
    }

    fn element(tree: &XmlTree, id: NodeId) -> Option<Element> {
        let el = tree.element(id)?;
        let (prefix, local) = split_qname(el.qname);
        let ns_decls = el
            .ns_decls
            .iter()
            .map(|d| {
                (
                    d.prefix.as_deref().unwrap_or("").to_string(),
                    d.uri.to_string(),
                )
            })
            .collect();
        let mut id_value = None;
        let attrs = el
            .attrs
            .iter()
            .map(|a| {
                let (aprefix, alocal) = split_qname(&a.qname);
                // No DTD/schema, so match the common ID-attribute spellings
                // (Id / ID / id) by local name — what real XML-DSig libraries do.
                if id_value.is_none() && alocal.eq_ignore_ascii_case("id") {
                    id_value = Some(a.value.to_string());
                }
                Attr {
                    qname: a.qname.to_string(),
                    prefix: aprefix,
                    local: alocal,
                    value: a.value.to_string(),
                }
            })
            .collect();
        let mut children = Vec::new();
        for child in tree.children(id) {
            match tree.kind(*child) {
                Kind::Text(text) | Kind::CData(text) => children.push(Node::Text(text.to_string())),
                Kind::Pi { target, data } => children.push(Node::Pi(if data.is_empty() {
                    target.to_string()
                } else {
                    format!("{} {}", target, data)
                })),
                // Bounded by the policy's depth limit (256, plus one
                // self-closing leaf), like the old recursion.
                Kind::Element { .. } => children.extend(element(tree, *child).map(Node::Element)),
                // Comments are omitted (no-comments form).
                _ => {}
            }
        }
        Some(Element {
            qname: el.qname.to_string(),
            prefix,
            local,
            ns_decls,
            attrs,
            children,
            id: id_value,
        })
    }

    // XML line-ending normalization (#xD#xA and lone #xD -> #xA). Done on the
    // raw source so literal CRs become LFs, while `&#xD;` character references
    // (which survive normalization untouched) round-trip back to CR and are
    // re-escaped as `&#xD;` by the output stage.
    let normalized = xml.replace("\r\n", "\n").replace('\r', "\n");
    let tree = crate::xml::parse(&normalized, &ParsePolicy::c14n()).map_err(|e| message(&e))?;
    let root = tree
        .root_element()
        .and_then(|root| element(&tree, root))
        .ok_or_else(|| "no document element found".to_string())?;
    refuse_prefix_undeclarations(&root)?;
    Ok(root)
}

/// Refuse `xmlns:p=""`, which Namespaces in XML 1.0 forbids.
///
/// The canonical form has no way to write it: `canonicalize` renders a
/// prefix only when it is bound to a URI, so `<saml:Attribute xmlns:saml="">`
/// canonicalizes to the very bytes of `<saml:Attribute>` under the outer
/// binding, while a reader resolves it to no namespace at all. A signed
/// element could then be hidden from the code reading the signed document
/// without its digest changing.
fn refuse_prefix_undeclarations(el: &Element) -> Result<(), String> {
    if let Some((prefix, _)) = el
        .ns_decls
        .iter()
        .find(|(prefix, uri)| !prefix.is_empty() && uri.is_empty())
    {
        return Err(format!(
            "namespace prefix '{}' is bound to an empty URI (not allowed in XML 1.0)",
            prefix
        ));
    }
    for node in &el.children {
        if let Node::Element(child) = node {
            refuse_prefix_undeclarations(child)?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn attach(stack: &mut [Element], roots: &mut Vec<Node>, node: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => roots.push(node),
    }
}

/// The namespace bindings in scope at one point of a walk down the tree.
///
/// One binding stack per prefix plus an undo log, the shape the shared XML
/// core uses (`crate::xml::ns::Bindings`): entering an element pushes its
/// own declarations, leaving it truncates back to the mark taken on entry.
/// Nothing is copied per element — a map cloned at every element made a
/// document with thousands of declarations on its root and a few hundred
/// thousand empty elements cost seconds of CPU before any signature was
/// looked at.
#[derive(Default)]
pub(crate) struct NsStack {
    by_prefix: HashMap<String, Vec<String>>,
    log: Vec<String>,
}

impl NsStack {
    /// The bindings of a flat map (prefix -> uri, "" = default).
    pub(crate) fn from_map(map: &BTreeMap<String, String>) -> Self {
        let mut stack = NsStack::default();
        for (prefix, uri) in map {
            stack.push(prefix, uri);
        }
        stack
    }

    /// The mark to pass to [`NsStack::truncate`].
    pub(crate) fn mark(&self) -> usize {
        self.log.len()
    }

    pub(crate) fn push(&mut self, prefix: &str, uri: &str) {
        match self.by_prefix.get_mut(prefix) {
            Some(stack) => stack.push(uri.to_string()),
            None => {
                self.by_prefix
                    .insert(prefix.to_string(), vec![uri.to_string()]);
            }
        }
        self.log.push(prefix.to_string());
    }

    /// Push the declarations `el` makes.
    pub(crate) fn enter(&mut self, el: &Element) -> usize {
        let mark = self.mark();
        for (prefix, uri) in &el.ns_decls {
            self.push(prefix, uri);
        }
        mark
    }

    /// Drop every binding pushed since `mark` was taken.
    pub(crate) fn truncate(&mut self, mark: usize) {
        while self.log.len() > mark {
            let Some(prefix) = self.log.pop() else {
                break;
            };
            if let Some(stack) = self.by_prefix.get_mut(&prefix) {
                stack.pop();
                if stack.is_empty() {
                    self.by_prefix.remove(&prefix);
                }
            }
        }
    }

    /// The innermost binding of `prefix`, an empty undeclaration included.
    pub(crate) fn get(&self, prefix: &str) -> Option<&str> {
        self.by_prefix
            .get(prefix)
            .and_then(|stack| stack.last())
            .map(String::as_str)
    }

    /// Every prefix in scope with its innermost URI. Linear in the distinct
    /// prefixes; taken once per element a caller asked for, not per element
    /// walked.
    pub(crate) fn snapshot(&self) -> BTreeMap<String, String> {
        self.by_prefix
            .iter()
            .filter_map(|(prefix, stack)| stack.last().map(|uri| (prefix.clone(), uri.clone())))
            .collect()
    }
}

/// Something a namespace prefix can be looked up in.
pub(crate) trait NsLookup {
    fn lookup(&self, prefix: &str) -> Option<&str>;
}

impl NsLookup for NsStack {
    fn lookup(&self, prefix: &str) -> Option<&str> {
        self.get(prefix)
    }
}

impl NsLookup for BTreeMap<String, String> {
    fn lookup(&self, prefix: &str) -> Option<&str> {
        self.get(prefix).map(String::as_str)
    }
}

/// Namespace URI used to sort an attribute (unprefixed = no namespace; the
/// default namespace deliberately does NOT apply to attributes).
fn attr_ns_uri<'s>(a: &Attr, in_scope: &'s NsStack) -> &'s str {
    if a.prefix.is_empty() {
        ""
    } else if a.prefix == "xml" {
        XML_NS
    } else {
        in_scope.get(&a.prefix).unwrap_or("")
    }
}

/// Resolve an element's namespace URI, honouring its own declarations first
/// then the inherited in-scope context.
pub(crate) fn element_ns_uri<L: NsLookup + ?Sized>(el: &Element, in_scope: &L) -> String {
    for (p, uri) in el.ns_decls.iter().rev() {
        if *p == el.prefix {
            return uri.clone();
        }
    }
    if el.prefix == "xml" {
        return XML_NS.to_string();
    }
    in_scope.lookup(&el.prefix).unwrap_or_default().to_string()
}

/// True if `el` is the `<ds:Signature>` element of an enveloped signature.
fn is_enveloped_signature(el: &Element, in_scope: &NsStack) -> bool {
    el.local == "Signature" && element_ns_uri(el, in_scope) == XMLDSIG_NS
}

/// Canonicalize `el`, appending to `out`.
///
/// `in_scope_parent` is the input namespace context inherited from the
/// ancestors of `el` (prefix -> uri, "" = default; an empty uri value means
/// the default namespace is undeclared). The output context starts empty.
///
/// `enveloped` drops every descendant `<ds:Signature>`; `skip` drops exactly
/// one element, by identity — the enveloped-signature transform as the spec
/// words it, which a signed assertion inside a signed response needs (the
/// outer digest covers the inner signature).
pub(crate) fn canonicalize(
    el: &Element,
    in_scope_parent: &BTreeMap<String, String>,
    prefix_list: &BTreeSet<String>,
    enveloped: bool,
    skip: Option<*const Element>,
    out: &mut String,
) {
    let mut in_scope = NsStack::from_map(in_scope_parent);
    let mut rendered = NsStack::default();
    canonicalize_in(
        el,
        &mut in_scope,
        &mut rendered,
        prefix_list,
        enveloped,
        skip,
        out,
    );
}

/// The recursion of [`canonicalize`]. `rendered` is the *output* namespace
/// context already emitted by ancestors — the heart of the "exclusive" rule:
/// a declaration is emitted only when visibly utilized and not already in
/// the output context. Both stacks come back as they were given.
fn canonicalize_in(
    el: &Element,
    in_scope: &mut NsStack,
    rendered: &mut NsStack,
    prefix_list: &BTreeSet<String>,
    enveloped: bool,
    skip: Option<*const Element>,
    out: &mut String,
) {
    let scope_mark = in_scope.enter(el);

    // Visibly utilized prefixes: the element's own prefix, every *prefixed*
    // attribute's prefix, plus the InclusiveNamespaces prefix list.
    let mut utilized: BTreeSet<&str> = BTreeSet::new();
    utilized.insert(el.prefix.as_str());
    for a in &el.attrs {
        if !a.prefix.is_empty() {
            utilized.insert(a.prefix.as_str());
        }
    }
    for p in prefix_list {
        utilized.insert(p.as_str());
    }

    // The prefixes are distinct, so deciding them all before recording any
    // in `rendered` decides each exactly as recording them one by one would.
    let mut to_render: Vec<(String, String)> = Vec::new();
    for p in utilized {
        if p == "xml" {
            // The xml prefix is implicitly declared everywhere; never emitted.
            continue;
        }
        if p.is_empty() {
            let uri = in_scope.get("").unwrap_or("");
            if uri.is_empty() {
                // Default namespace is "none": emit xmlns="" only to override
                // a non-empty default still in effect in the output.
                if rendered.get("").map(|u| !u.is_empty()).unwrap_or(false) {
                    to_render.push((String::new(), String::new()));
                }
            } else if rendered.get("") != Some(uri) {
                to_render.push((String::new(), uri.to_string()));
            }
        } else if let Some(uri) = in_scope.get(p) {
            if !uri.is_empty() && rendered.get(p) != Some(uri) {
                to_render.push((p.to_string(), uri.to_string()));
            }
        }
        // A used prefix that is not in scope cannot be rendered; skip it.
    }
    // `utilized` is ordered, so `to_render` already is.
    let render_mark = rendered.mark();
    for (p, uri) in &to_render {
        rendered.push(p, uri);
    }

    let mut attrs_sorted: Vec<(&str, &Attr)> = el
        .attrs
        .iter()
        .map(|a| (attr_ns_uri(a, in_scope), a))
        .collect();
    attrs_sorted.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.local.cmp(&b.1.local)));

    out.push('<');
    out.push_str(&el.qname);
    for (p, uri) in &to_render {
        if p.is_empty() {
            out.push_str(" xmlns=\"");
        } else {
            out.push_str(" xmlns:");
            out.push_str(p);
            out.push_str("=\"");
        }
        out.push_str(&escape_attr(uri));
        out.push('"');
    }
    for (_, a) in &attrs_sorted {
        out.push(' ');
        out.push_str(&a.qname);
        out.push_str("=\"");
        out.push_str(&escape_attr(&a.value));
        out.push('"');
    }
    out.push('>');

    for child in &el.children {
        match child {
            Node::Text(t) => out.push_str(&escape_text(t)),
            Node::Pi(p) => {
                out.push_str("<?");
                out.push_str(p);
                out.push_str("?>");
            }
            Node::Element(c) => {
                // Enveloped-signature transform: drop the <ds:Signature> the
                // signature lives in before canonicalizing the referenced subtree.
                if enveloped && is_enveloped_signature(c, in_scope) {
                    continue;
                }
                if skip == Some(c as *const Element) {
                    continue;
                }
                canonicalize_in(c, in_scope, rendered, prefix_list, enveloped, skip, out);
            }
        }
    }

    out.push_str("</");
    out.push_str(&el.qname);
    out.push('>');

    rendered.truncate(render_mark);
    in_scope.truncate(scope_mark);
}

/// Find the element with the given Id, returning it together with the
/// namespace context inherited from its ancestors (so the subtree can be
/// canonicalized exactly as it would be in the full document).
fn find_by_id<'a>(
    el: &'a Element,
    target: &str,
    in_scope: &mut NsStack,
) -> Option<(&'a Element, BTreeMap<String, String>)> {
    if el.id.as_deref() == Some(target) {
        return Some((el, in_scope.snapshot()));
    }
    let mark = in_scope.enter(el);
    for child in &el.children {
        if let Node::Element(c) = child {
            if let Some(found) = find_by_id(c, target, in_scope) {
                return Some(found);
            }
        }
    }
    in_scope.truncate(mark);
    None
}

/// The most `Xml.get_elements_by_tag` returns, in bytes. Matches can nest,
/// and each one carries every namespace in scope, so the result of a small
/// document can be far larger than the document.
const MAX_EXTRACTED_BYTES: usize = 32 * 1024 * 1024;

/// Serialize every element with the given local name as a standalone
/// fragment, each carrying its inherited namespace context.
fn collect_by_local(
    el: &Element,
    local: &str,
    in_scope: &mut NsStack,
    out: &mut Vec<String>,
    budget: &mut usize,
) -> Result<(), String> {
    if el.local == local {
        let mut fragment = String::new();
        serialize_faithful(el, &in_scope.snapshot(), &mut fragment);
        *budget = budget.checked_sub(fragment.len()).ok_or_else(|| {
            format!(
                "the matching elements add up to more than {} bytes",
                MAX_EXTRACTED_BYTES
            )
        })?;
        out.push(fragment);
    }
    let mark = in_scope.enter(el);
    for child in &el.children {
        if let Node::Element(c) = child {
            collect_by_local(c, local, in_scope, out, budget)?;
        }
    }
    in_scope.truncate(mark);
    Ok(())
}

pub(crate) fn root_in_scope() -> BTreeMap<String, String> {
    let mut in_scope = BTreeMap::new();
    in_scope.insert("xml".to_string(), XML_NS.to_string());
    in_scope
}

/// Exclusive canonicalization with full options.
///
/// - `id`: when set, canonicalize only the subtree rooted at the element whose
///   Id attribute matches (inheriting the ancestor namespace context). When
///   unset, the whole document element.
/// - `enveloped`: drop any descendant `<ds:Signature>` (the enveloped-signature
///   transform used by XML-DSig / SAML).
fn canonicalize_exclusive_opts(
    xml: &str,
    prefix_list: &BTreeSet<String>,
    id: Option<&str>,
    enveloped: bool,
) -> Result<String, String> {
    let root = parse_document(xml)?;
    let base = root_in_scope();
    let (target, in_scope_parent) = match id {
        Some(want) => find_by_id(&root, want, &mut NsStack::from_map(&base))
            .ok_or_else(|| format!("no element with Id '{}' found", want))?,
        None => (&root, base),
    };
    let mut out = String::new();
    canonicalize(
        target,
        &in_scope_parent,
        prefix_list,
        enveloped,
        None,
        &mut out,
    );
    Ok(out)
}

#[cfg(test)]
fn canonicalize_exclusive(xml: &str, prefix_list: &BTreeSet<String>) -> Result<String, String> {
    canonicalize_exclusive_opts(xml, prefix_list, None, false)
}

/// Faithfully (non-canonically) serialize an element subtree, injecting the
/// inherited in-scope namespace declarations onto its root so the fragment is
/// a valid standalone document. Exact byte layout is not preserved (entities
/// are resolved, attribute quoting normalised), but the *information* is —
/// which is what callers need before handing the fragment to canonicalization.
fn serialize_faithful(el: &Element, inherited: &BTreeMap<String, String>, out: &mut String) {
    serialize_element(el, Some(inherited), out);
}

/// One element of [`serialize_faithful`]. Only the fragment's root carries
/// the inherited declarations; below it, an element writes its own — they
/// are in scope there already, and repeating them on every descendant made
/// the output grow with elements times declarations.
fn serialize_element(el: &Element, inherited: Option<&BTreeMap<String, String>>, out: &mut String) {
    out.push('<');
    out.push_str(&el.qname);

    // Inject inherited namespaces not redeclared here (skip the implicit xml).
    if let Some(inherited) = inherited {
        let local_prefixes: BTreeSet<&str> = el.ns_decls.iter().map(|(p, _)| p.as_str()).collect();
        for (p, uri) in inherited {
            if p == "xml" || uri.is_empty() || local_prefixes.contains(p.as_str()) {
                continue;
            }
            if p.is_empty() {
                out.push_str(" xmlns=\"");
            } else {
                out.push_str(" xmlns:");
                out.push_str(p);
                out.push_str("=\"");
            }
            out.push_str(&escape_attr(uri));
            out.push('"');
        }
    }
    // This element's own declarations, in source order.
    for (p, uri) in &el.ns_decls {
        if p.is_empty() {
            out.push_str(" xmlns=\"");
        } else {
            out.push_str(" xmlns:");
            out.push_str(p);
            out.push_str("=\"");
        }
        out.push_str(&escape_attr(uri));
        out.push('"');
    }
    for a in &el.attrs {
        out.push(' ');
        out.push_str(&a.qname);
        out.push_str("=\"");
        out.push_str(&escape_attr(&a.value));
        out.push('"');
    }
    out.push('>');

    for child in &el.children {
        match child {
            Node::Text(t) => out.push_str(&escape_text(t)),
            Node::Pi(p) => {
                out.push_str("<?");
                out.push_str(p);
                out.push_str("?>");
            }
            Node::Element(c) => serialize_element(c, None, out),
        }
    }

    out.push_str("</");
    out.push_str(&el.qname);
    out.push('>');
}

/// Extract the subtree with the given Id as a standalone XML fragment.
fn extract_by_id(xml: &str, id: &str) -> Result<String, String> {
    let root = parse_document(xml)?;
    let (target, inherited) = find_by_id(&root, id, &mut NsStack::from_map(&root_in_scope()))
        .ok_or_else(|| format!("no element with Id '{}' found", id))?;
    let mut out = String::new();
    serialize_faithful(target, &inherited, &mut out);
    Ok(out)
}

/// Extract every element with the given local name as standalone XML fragments.
fn extract_by_local(xml: &str, local: &str) -> Result<Vec<String>, String> {
    let root = parse_document(xml)?;
    let mut found = Vec::new();
    let mut budget = MAX_EXTRACTED_BYTES;
    collect_by_local(
        &root,
        local,
        &mut NsStack::from_map(&root_in_scope()),
        &mut found,
        &mut budget,
    )?;
    Ok(found)
}

/// Parse the optional InclusiveNamespaces argument: a space-separated string
/// or an array of prefix strings. `#default` selects the default namespace
/// (represented as the empty prefix), matching the exc-c14n convention.
fn parse_prefix_list(value: &Value) -> Result<BTreeSet<String>, String> {
    let mut set = BTreeSet::new();
    let mut add = |token: &str| {
        let token = token.trim();
        if token.is_empty() {
            return;
        }
        if token == "#default" {
            set.insert(String::new());
        } else {
            set.insert(token.to_string());
        }
    };
    match value {
        Value::String(s) => {
            for token in s.split_whitespace() {
                add(token);
            }
        }
        Value::Array(arr) => {
            for v in arr.borrow().iter() {
                match v {
                    Value::String(s) => add(s),
                    other => {
                        return Err(format!(
                            "inclusive prefix list entries must be strings, got {}",
                            other.type_name()
                        ))
                    }
                }
            }
        }
        Value::Null => {}
        other => {
            return Err(format!(
                "inclusive prefix list must be a string or array, got {}",
                other.type_name()
            ))
        }
    }
    Ok(set)
}

/// Parsed options for `Xml.c14n_exclusive`'s optional second argument.
struct C14nOpts {
    prefix_list: BTreeSet<String>,
    id: Option<String>,
    enveloped: bool,
}

/// The optional 2nd arg is either the InclusiveNamespaces list directly
/// (String / Array — backward compatible) or an options Hash with
/// `inclusive_prefixes`, `id`, and `enveloped_signature` keys.
fn parse_c14n_opts(value: &Value) -> Result<C14nOpts, String> {
    use crate::interpreter::value::HashKey;
    match value {
        Value::Hash(h) => {
            let h = h.borrow();
            let get = |k: &str| h.get(&HashKey::String(k.to_string().into())).cloned();
            let prefix_list = match get("inclusive_prefixes") {
                Some(v) => parse_prefix_list(&v)?,
                None => BTreeSet::new(),
            };
            let id = match get("id") {
                Some(Value::String(s)) => Some(s),
                Some(Value::Null) | None => None,
                Some(other) => {
                    return Err(format!("id must be a string, got {}", other.type_name()))
                }
            };
            let enveloped = matches!(get("enveloped_signature"), Some(Value::Bool(true)));
            Ok(C14nOpts {
                prefix_list,
                id: id.map(|s| s.to_string()),
                enveloped,
            })
        }
        other => Ok(C14nOpts {
            prefix_list: parse_prefix_list(other)?,
            id: None,
            enveloped: false,
        }),
    }
}

pub fn register_xml_builtins(env: &mut Environment) {
    let mut xml_static_methods: std::collections::HashMap<String, Rc<NativeFunction>> =
        std::collections::HashMap::new();

    // Xml.c14n_exclusive(xml, inclusive_prefixes_or_options?) -> String
    xml_static_methods.insert(
        "c14n_exclusive".to_string(),
        Rc::new(NativeFunction::new("Xml.c14n_exclusive", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "Xml.c14n_exclusive() expects 1-2 arguments (xml, inclusive_prefixes_or_options?), got {}",
                    args.len()
                ));
            }
            let xml = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Xml.c14n_exclusive() expects string XML, got {}",
                        other.type_name()
                    ))
                }
            };
            let opts = if args.len() == 2 {
                parse_c14n_opts(&args[1]).map_err(|e| format!("Xml.c14n_exclusive(): {}", e))?
            } else {
                C14nOpts {
                    prefix_list: BTreeSet::new(),
                    id: None,
                    enveloped: false,
                }
            };
            let canonical =
                canonicalize_exclusive_opts(&xml, &opts.prefix_list, opts.id.as_deref(), opts.enveloped)
                    .map_err(|e| format!("Xml.c14n_exclusive(): {}", e))?;
            Ok(Value::String(canonical.into()))
        })),
    );

    // Xml.get_element_by_id(xml, id) -> String (standalone fragment)
    xml_static_methods.insert(
        "get_element_by_id".to_string(),
        Rc::new(NativeFunction::new(
            "Xml.get_element_by_id",
            Some(2),
            |args| {
                let xml = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Xml.get_element_by_id() expects string XML, got {}",
                            other.type_name()
                        ))
                    }
                };
                let id = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Xml.get_element_by_id() expects string id, got {}",
                            other.type_name()
                        ))
                    }
                };
                let fragment = extract_by_id(&xml, &id)
                    .map_err(|e| format!("Xml.get_element_by_id(): {}", e))?;
                Ok(Value::String(fragment.into()))
            },
        )),
    );

    // Xml.get_elements_by_tag(xml, local_name) -> Array<String>
    xml_static_methods.insert(
        "get_elements_by_tag".to_string(),
        Rc::new(NativeFunction::new(
            "Xml.get_elements_by_tag",
            Some(2),
            |args| {
                let xml = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Xml.get_elements_by_tag() expects string XML, got {}",
                            other.type_name()
                        ))
                    }
                };
                let local = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Xml.get_elements_by_tag() expects string local name, got {}",
                            other.type_name()
                        ))
                    }
                };
                let fragments = extract_by_local(&xml, &local)
                    .map_err(|e| format!("Xml.get_elements_by_tag(): {}", e))?;
                let values: Vec<Value> = fragments
                    .into_iter()
                    .map(|s| Value::String(s.into()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            },
        )),
    );

    let xml_class = Class {
        name: "Xml".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(std::collections::HashMap::new())),
        static_methods: std::collections::HashMap::new(),
        native_static_methods: xml_static_methods,
        native_methods: std::collections::HashMap::new(),
        static_fields: Rc::new(RefCell::new(std::collections::HashMap::new())),
        fields: std::collections::HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(std::collections::HashMap::new())),
        ..Default::default()
    };
    env.define("Xml".to_string(), Value::Class(Rc::new(xml_class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser `parse_document` had before it moved onto the shared XML
    /// core, kept as the oracle the adapter is compared with.
    fn legacy_parse_document(xml: &str) -> Result<Element, String> {
        // XML line-ending normalization (#xD#xA and lone #xD -> #xA). Done on the
        // raw source so literal CRs become LFs, while `&#xD;` character references
        // (which survive normalization untouched) round-trip back to CR and are
        // re-escaped as `&#xD;` by the output stage.
        let normalized = xml.replace("\r\n", "\n").replace('\r', "\n");

        let mut reader = Reader::from_str(&normalized);
        let mut buf = Vec::new();
        let mut stack: Vec<Element> = Vec::new();
        let mut roots: Vec<Node> = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::DocType(_)) => {
                    return Err(
                        "DOCTYPE declarations are not allowed (XXE / entity-expansion defense)"
                            .to_string(),
                    );
                }
                Ok(Event::Decl(_)) | Ok(Event::Comment(_)) => {
                    // XML declaration removed; comments omitted (no-comments form).
                }
                Ok(Event::Start(e)) => {
                    if stack.len() >= MAX_DEPTH {
                        return Err(format!("nesting depth exceeded {} levels", MAX_DEPTH));
                    }
                    stack.push(build_element(&e)?);
                }
                Ok(Event::Empty(e)) => {
                    let el = build_element(&e)?;
                    attach(&mut stack, &mut roots, Node::Element(el));
                }
                Ok(Event::End(_)) => {
                    if let Some(el) = stack.pop() {
                        attach(&mut stack, &mut roots, Node::Element(el));
                    }
                }
                Ok(Event::Text(e)) => {
                    let text = decode_text_event(&e)?;
                    // Text outside the document element is discarded.
                    if !stack.is_empty() {
                        attach(&mut stack, &mut roots, Node::Text(text));
                    }
                }
                Ok(Event::CData(e)) => {
                    // CDATA content is literal character data; escaped on output.
                    if !stack.is_empty() {
                        attach(
                            &mut stack,
                            &mut roots,
                            Node::Text(bytes_lossy(&e.into_inner())),
                        );
                    }
                }
                Ok(Event::PI(e)) => {
                    // Processing instructions inside the document element are kept.
                    // (Top-level PIs, which the spec wraps in newlines, are out of
                    // scope for subtree canonicalization and are dropped.)
                    if !stack.is_empty() {
                        let target = bytes_lossy(&e.target().to_vec());
                        let content = bytes_lossy(&e.content().to_vec());
                        let formatted = if content.is_empty() {
                            target
                        } else {
                            format!("{} {}", target, content)
                        };
                        attach(&mut stack, &mut roots, Node::Pi(formatted));
                    }
                }
                Ok(Event::GeneralRef(e)) => {
                    // quick-xml 0.41 surfaces entity references — including the five
                    // predefined ones (`&amp;` etc.) and numeric character refs
                    // (`&#65;`) — as their own event instead of inlining them into
                    // `Text`. Resolve those to their replacement text (re-escaped by
                    // the output stage). Any *named* entity that isn't predefined
                    // could only be defined by a DTD, which we reject above; refuse
                    // it rather than silently dropping signed material.
                    let text = resolve_general_ref(&e)?;
                    if !stack.is_empty() {
                        attach(&mut stack, &mut roots, Node::Text(text));
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("XML parsing error: {}", e)),
            }
            buf.clear();
        }

        roots
            .into_iter()
            .find_map(|n| match n {
                Node::Element(e) => Some(e),
                _ => None,
            })
            .ok_or_else(|| "no document element found".to_string())
    }

    /// The `Element` tree as text, adjacent text nodes merged (the old loop
    /// made one per quick-xml event; every consumer concatenates them).
    fn dump(el: &Element, out: &mut String) {
        out.push_str(&format!(
            "<{}|{}|{}|{:?}|{:?}|{:?}>",
            el.qname,
            el.prefix,
            el.local,
            el.ns_decls,
            el.attrs
                .iter()
                .map(|a| (&a.qname, &a.prefix, &a.local, &a.value))
                .collect::<Vec<_>>(),
            el.id
        ));
        let mut text = String::new();
        for child in &el.children {
            match child {
                Node::Text(t) => text.push_str(t),
                other => {
                    if !text.is_empty() {
                        out.push_str(&format!("T{:?}", std::mem::take(&mut text)));
                    }
                    match other {
                        Node::Pi(p) => out.push_str(&format!("P{:?}", p)),
                        Node::Element(c) => dump(c, out),
                        Node::Text(_) => {}
                    }
                }
            }
        }
        if !text.is_empty() {
            out.push_str(&format!("T{:?}", text));
        }
        out.push_str("</>");
    }

    fn render(result: Result<Element, String>) -> Result<String, String> {
        let el = result?;
        let mut out = String::new();
        dump(&el, &mut out);
        Ok(out)
    }

    /// Documents both parsers must read into the same `Element`, and the
    /// error strings both must give.
    const EQUIVALENCE_CORPUS: &[&str] = &[
        "<doc/>",
        "<doc></doc>",
        "<doc b='2' a='1' c='3'></doc>",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><doc>x</doc>",
        "<doc><!-- hi -->text</doc>",
        "<doc>a &amp; b &lt; c &gt; d</doc>",
        "<doc x=\"a&#9;b&#10;c&gt;d\"></doc>",
        "<doc>  a  b  </doc>",
        r#"<n0:root xmlns:n0="http://a" xmlns:n2="http://c"><n1:elem xmlns:n1="http://b">text</n1:elem></n0:root>"#,
        r#"<a:x xmlns:a="http://a"><a:y><a:z>q</a:z></a:y></a:x>"#,
        r#"<root xmlns="http://a"><child xmlns="">t</child></root>"#,
        r#"<a:x xmlns:a="http://a"><a:y xmlns:a="http://a">t</a:y></a:x>"#,
        "<doc><![CDATA[a < b & c]]></doc>",
        r#"<doc xmlns:b="http://b" b:z="1" a="2"></doc>"#,
        r#"<root xmlns:a="http://a"><a:obj ID="x"><a:v>hi</a:v></a:obj></root>"#,
        r#"<Document ID="_o"><Data>x</Data><ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:Sig/></ds:Signature></Document>"#,
        r#"<r><Item>1<Item>2</Item></Item><g><Item>3</Item></g></r>"#,
        // Line endings, references, PIs, attribute whitespace.
        "<a>l1\r\nl2\rl3</a>",
        "<a x=\"1\r\n2\" y=\"1\t2\n3\"/>",
        "<a>x&#13;y&#xD;&#9;&#x41;&#66;</a>",
        "<a x=\"&#9;&#10;&#13;&lt;&amp;&quot;&apos;\"/>",
        "<a>x&#1;y&#xFFFE;</a>",
        "<a x=\"&#1;\"/>",
        "<a><?pi  data  here?><?empty?>t<?x y?></a>",
        "<?pi out?><!--c-->  <a>x<!-- y -->z</a><?pi after?><!--d-->",
        "<a>x<![CDATA[<y>&]]>z<![CDATA[]]></a>",
        "&amp; <a>x</a> tail",
        // Roots: trailing ones are ignored, the first one is kept.
        "<a>1</a><b>2</b>",
        "<a>1</a><b>",
        "<a/><b>2</b>",
        // Namespaces: undeclared defaults, unbound prefixes, Id spellings.
        r#"<r xmlns="u"><c xmlns=""><d/></c></r>"#,
        "<p:a><p:b/></p:a>",
        r#"<a x="1" p:x="2"/>"#,
        r#"<a xmlns:p="u" xmlns:q="u" p:x="1" q:x="2"/>"#,
        r#"<a ID="1"><b Id="2"/><c wsu:Id="3" xmlns:wsu="w"/><d id="a" ID="b"/></a>"#,
        r#"<a xml:lang="en" b="1" xmlns:x="a:b:c" x:y:z="2"/>"#,
        r#"<r xmlns:a="u" xmlns:b="v"><s xmlns:b="w"><a:x b:y="1"/></s></r>"#,
        "<a b='\"x\"' c=\"'y'\" d=\"<\"/>",
        "\u{FEFF}<a>x</a>",
        "<a>x\u{1}y</a>",
        "<a><!-- a -- b -->x</a>",
        "<?xml version=\"1.0\" encoding?><a/>",
        "<a/><?xml version=\"1.0\"?>",
        "<a>\n  <b> x </b>\n</a>",
        "<a>caf\u{e9} &#233; \u{1F600}</a>",
        // Errors.
        "",
        "hello",
        "<a><b>1</b>",
        "<a><b></a>",
        "</a>",
        "<a",
        "<!DOCTYPE a><a/>",
        "<a>&foo;</a>",
        "<a>1</a><b>&foo;</b>",
    ];

    #[test]
    fn the_adapter_builds_what_the_legacy_parser_built() -> Result<(), String> {
        let mut nested = String::new();
        for _ in 0..256 {
            nested.push_str("<n>");
        }
        nested.push_str("<leaf/>");
        for _ in 0..256 {
            nested.push_str("</n>");
        }
        let decls: Vec<String> = (0..300).map(|i| format!("xmlns:p{i}=\"u{i}\"")).collect();
        let many_decls = format!("<a {}/>", decls.join(" "));
        let mut inputs: Vec<&str> = EQUIVALENCE_CORPUS.to_vec();
        inputs.push(&nested);
        inputs.push(&many_decls);
        for input in inputs {
            let legacy = render(legacy_parse_document(input));
            let adapter = render(parse_document(input));
            if legacy != adapter {
                return Err(format!(
                    "{input:?}\n  legacy:  {legacy:?}\n  adapter: {adapter:?}"
                ));
            }
        }
        Ok(())
    }

    /// Where the two parsers differ on purpose: refusals of quick-xml's
    /// `NsReader`, which the shared core reads with, and the error wording
    /// of inputs the old loop refused too.
    #[test]
    fn the_adapter_differs_from_the_legacy_parser_only_as_listed() -> Result<(), String> {
        for input in [
            r#"<a xmlns:xml="other"/>"#,
            r#"<a xmlns:xmlns="u"/>"#,
            r#"<a xmlns:p="http://www.w3.org/XML/1998/namespace"/>"#,
            r#"<a xmlns:p="http://www.w3.org/2000/xmlns/"/>"#,
        ] {
            render(legacy_parse_document(input))?;
            let err = match parse_document(input) {
                Ok(_) => return Err(format!("{input}: the adapter accepted it")),
                Err(err) => err,
            };
            if !err.starts_with("XML parsing error: ") {
                return Err(format!("{input}: {err}"));
            }
        }
        for input in [
            r#"<a x="1" x="2"/>"#,
            r#"<a x="&foo;"/>"#,
            r#"<a x="AT&T"/>"#,
            "<a b/>",
            "<a>&#0;</a>",
        ] {
            if legacy_parse_document(input).is_ok() || parse_document(input).is_ok() {
                return Err(format!("{input}: expected both parsers to refuse it"));
            }
        }
        Ok(())
    }

    fn c14n(xml: &str) -> String {
        canonicalize_exclusive(xml, &BTreeSet::new()).unwrap()
    }

    fn c14n_with(xml: &str, prefixes: &[&str]) -> String {
        let set: BTreeSet<String> = prefixes.iter().map(|s| s.to_string()).collect();
        canonicalize_exclusive(xml, &set).unwrap()
    }

    #[test]
    fn empty_element_expands_to_start_end_pair() {
        assert_eq!(c14n("<doc/>"), "<doc></doc>");
        assert_eq!(c14n("<doc></doc>"), "<doc></doc>");
    }

    #[test]
    fn attributes_sorted_and_double_quoted() {
        // Attributes sorted by (namespace uri, local name); single quotes
        // become double quotes.
        assert_eq!(
            c14n("<doc b='2' a='1' c='3'></doc>"),
            "<doc a=\"1\" b=\"2\" c=\"3\"></doc>"
        );
    }

    #[test]
    fn xml_declaration_removed() {
        assert_eq!(
            c14n("<?xml version=\"1.0\" encoding=\"UTF-8\"?><doc>x</doc>"),
            "<doc>x</doc>"
        );
    }

    #[test]
    fn comments_omitted() {
        assert_eq!(c14n("<doc><!-- hi -->text</doc>"), "<doc>text</doc>");
    }

    #[test]
    fn text_special_chars_escaped() {
        assert_eq!(
            c14n("<doc>a &amp; b &lt; c &gt; d</doc>"),
            "<doc>a &amp; b &lt; c &gt; d</doc>"
        );
    }

    #[test]
    fn attr_special_chars_escaped() {
        // Tab/newline char refs in an attribute become &#x9;/&#xA;; ">" stays.
        assert_eq!(
            c14n("<doc x=\"a&#9;b&#10;c&gt;d\"></doc>"),
            "<doc x=\"a&#x9;b&#xA;c>d\"></doc>"
        );
    }

    #[test]
    fn whitespace_in_content_preserved() {
        assert_eq!(c14n("<doc>  a  b  </doc>"), "<doc>  a  b  </doc>");
    }

    #[test]
    fn exclusive_drops_unused_ancestor_namespace() {
        // n2 is declared on the root but only used by <unused>; exclusive
        // c14n must NOT carry n2 onto <n1:elem> where it is not utilized.
        let xml = r#"<n0:root xmlns:n0="http://a" xmlns:n2="http://c"><n1:elem xmlns:n1="http://b">text</n1:elem></n0:root>"#;
        let got = c14n(xml);
        assert_eq!(
            got,
            r#"<n0:root xmlns:n0="http://a"><n1:elem xmlns:n1="http://b">text</n1:elem></n0:root>"#
        );
        assert!(!got.contains("http://c"), "unused n2 leaked: {}", got);
    }

    #[test]
    fn exclusive_renders_namespace_where_used() {
        // A prefix used by a descendant is rendered at the descendant, not
        // duplicated once already in the output context.
        let xml = r#"<a:x xmlns:a="http://a"><a:y><a:z>q</a:z></a:y></a:x>"#;
        assert_eq!(
            c14n(xml),
            r#"<a:x xmlns:a="http://a"><a:y><a:z>q</a:z></a:y></a:x>"#
        );
    }

    #[test]
    fn inclusive_prefix_list_forces_render() {
        // n2 is declared on the root but never visibly utilized. Plain
        // exclusive c14n drops it; with n2 in the InclusiveNamespaces list it
        // is rendered at the topmost in-scope node (the root) and not repeated.
        let xml =
            r#"<n0:root xmlns:n0="http://a" xmlns:n2="http://c"><n0:child>t</n0:child></n0:root>"#;

        let without = c14n(xml);
        assert!(
            !without.contains("http://c"),
            "unused n2 should be dropped without a prefix list: {}",
            without
        );

        let with = c14n_with(xml, &["n2"]);
        assert_eq!(
            with,
            r#"<n0:root xmlns:n0="http://a" xmlns:n2="http://c"><n0:child>t</n0:child></n0:root>"#
        );
    }

    #[test]
    fn default_namespace_undeclared_when_child_has_none() {
        // Child explicitly clears the default namespace -> xmlns="".
        let xml = r#"<root xmlns="http://a"><child xmlns="">t</child></root>"#;
        assert_eq!(
            c14n(xml),
            r#"<root xmlns="http://a"><child xmlns="">t</child></root>"#
        );
    }

    #[test]
    fn redundant_namespace_declaration_collapsed() {
        // Re-declaring the same prefix->uri on a child is redundant in the
        // output and must be dropped.
        let xml = r#"<a:x xmlns:a="http://a"><a:y xmlns:a="http://a">t</a:y></a:x>"#;
        assert_eq!(c14n(xml), r#"<a:x xmlns:a="http://a"><a:y>t</a:y></a:x>"#);
    }

    #[test]
    fn cdata_becomes_escaped_text() {
        assert_eq!(
            c14n("<doc><![CDATA[a < b & c]]></doc>"),
            "<doc>a &lt; b &amp; c</doc>"
        );
    }

    #[test]
    fn rejects_doctype() {
        let err = canonicalize_exclusive(
            "<!DOCTYPE doc [<!ENTITY x \"y\">]><doc>&x;</doc>",
            &BTreeSet::new(),
        )
        .unwrap_err();
        assert!(err.contains("DOCTYPE"), "{}", err);
    }

    #[test]
    fn unprefixed_attribute_not_in_default_namespace() {
        // The default namespace does not apply to attributes, so an unprefixed
        // attribute sorts ahead of any namespaced one (empty namespace URI).
        let xml = r#"<doc xmlns:b="http://b" b:z="1" a="2"></doc>"#;
        assert_eq!(c14n(xml), r#"<doc xmlns:b="http://b" a="2" b:z="1"></doc>"#);
    }

    // ---- by-Id selection, enveloped-signature transform, extraction ----

    fn c14n_opts(xml: &str, id: Option<&str>, enveloped: bool) -> String {
        canonicalize_exclusive_opts(xml, &BTreeSet::new(), id, enveloped).unwrap()
    }

    #[test]
    fn canonicalize_subtree_by_id_inherits_ancestor_namespaces() {
        // The target subtree must carry the ancestor's `a` namespace it uses,
        // even though that decl lived on the root.
        let xml = r#"<root xmlns:a="http://a"><a:obj ID="x"><a:v>hi</a:v></a:obj></root>"#;
        assert_eq!(
            c14n_opts(xml, Some("x"), false),
            r#"<a:obj xmlns:a="http://a" ID="x"><a:v>hi</a:v></a:obj>"#
        );
    }

    #[test]
    fn missing_id_errors() {
        let err = canonicalize_exclusive_opts(
            "<root><child ID=\"a\"/></root>",
            &BTreeSet::new(),
            Some("nope"),
            false,
        )
        .unwrap_err();
        assert!(err.contains("nope"), "{}", err);
    }

    #[test]
    fn enveloped_transform_drops_ds_signature() {
        let xml = concat!(
            r#"<Document ID="_o"><Data>x</Data>"#,
            r#"<ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:Sig/></ds:Signature>"#,
            r#"</Document>"#
        );
        // Without the transform the Signature stays; with it, the Signature is
        // removed before canonicalizing.
        assert!(c14n_opts(xml, Some("_o"), false).contains("Signature"));
        assert_eq!(
            c14n_opts(xml, Some("_o"), true),
            r#"<Document ID="_o"><Data>x</Data></Document>"#
        );
    }

    #[test]
    fn enveloped_transform_keeps_non_dsig_signature() {
        // An element named Signature in a different namespace is NOT dropped.
        let xml = concat!(
            r#"<Document ID="_o"><other:Signature xmlns:other="urn:x">keep</other:Signature>"#,
            r#"</Document>"#
        );
        assert!(c14n_opts(xml, Some("_o"), true).contains("keep"));
    }

    #[test]
    fn extract_by_id_injects_inherited_namespaces() {
        let xml = r#"<root xmlns:a="http://a" xmlns:unused="http://u"><a:obj ID="x"><a:v>hi</a:v></a:obj></root>"#;
        let frag = extract_by_id(xml, "x").unwrap();
        // The fragment is standalone-parseable and re-canonicalizes the same
        // as the in-context subtree (exclusive c14n drops the unused ns).
        let direct = c14n_opts(xml, Some("x"), false);
        assert_eq!(
            canonicalize_exclusive(&frag, &BTreeSet::new()).unwrap(),
            direct
        );
    }

    /// Thousands of declarations on the root over tens of thousands of empty
    /// elements: every walk used to copy the whole prefix map at every
    /// element (seconds to minutes of CPU for 1 MiB). Each walk is now linear.
    #[test]
    fn many_declarations_over_many_elements_stay_linear() {
        let mut xml = String::from("<r");
        for i in 0..4000 {
            xml.push_str(&format!(" xmlns:p{i}=\"urn:{i}\""));
        }
        xml.push('>');
        for _ in 0..50_000 {
            xml.push_str("<a/>");
        }
        xml.push_str("<last ID=\"x\"/></r>");

        let started = std::time::Instant::now();
        let canonical = c14n(&xml);
        assert!(canonical.starts_with("<r><a></a>"), "{}", &canonical[..40]);
        assert_eq!(c14n_opts(&xml, Some("x"), true), "<last ID=\"x\"></last>");
        assert!(extract_by_id(&xml, "x").unwrap().contains("xmlns:p3999"));
        assert!(extract_by_id(&xml, "missing").is_err());
        // Each match carries the 4000 declarations: the budget stops it.
        let error = extract_by_local(&xml, "a").unwrap_err();
        assert!(error.contains("add up to more than"), "{error}");
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "took {elapsed:?}"
        );
    }

    #[test]
    fn a_fragment_declares_inherited_namespaces_on_its_root_only() {
        let xml = r#"<r xmlns:a="http://a"><a:obj ID="x"><a:v><a:w/></a:v></a:obj></r>"#;
        assert_eq!(
            extract_by_id(xml, "x").unwrap(),
            r#"<a:obj xmlns:a="http://a" ID="x"><a:v><a:w></a:w></a:v></a:obj>"#
        );
    }

    #[test]
    fn extract_by_tag_finds_all_matches() {
        let xml = r#"<r><Item>1</Item><g><Item>2</Item></g></r>"#;
        let items = extract_by_local(xml, "Item").unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].contains(">1<"));
        assert!(items[1].contains(">2<"));
    }
}
