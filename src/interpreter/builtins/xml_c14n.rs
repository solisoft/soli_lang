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
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::rc::Rc;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

/// The implicit `xml` prefix namespace, declared everywhere per the XML spec.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// The XML Signature namespace. Used by the enveloped-signature transform to
/// recognise (and drop) the `<ds:Signature>` element being verified.
const XMLDSIG_NS: &str = "http://www.w3.org/2000/09/xmldsig#";

/// Element-nesting cap — a deeply nested document is the classic XML DoS
/// vector. Generous enough for any real signed payload.
const MAX_DEPTH: usize = 256;

/// A regular (non-`xmlns`) attribute.
struct Attr {
    qname: String,
    prefix: String,
    local: String,
    value: String,
}

/// A parsed element with its namespace declarations, attributes and children.
struct Element {
    qname: String,
    prefix: String,
    local: String,
    ns_decls: Vec<(String, String)>, // (prefix or "" for default, uri)
    attrs: Vec<Attr>,
    children: Vec<Node>,
    /// Value of an `Id`/`ID`/`id` attribute, if any — used to resolve
    /// XML-DSig `Reference URI="#..."` selections.
    id: Option<String>,
}

enum Node {
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

fn build_element(e: &BytesStart) -> Result<Element, String> {
    let qname = bytes_lossy(&e.name().into_inner().to_vec());
    let (prefix, local) = split_qname(&qname);
    let mut ns_decls = Vec::new();
    let mut attrs = Vec::new();
    let mut id = None;

    for attr in e.attributes() {
        let attr = attr.map_err(|err| format!("XML attribute error: {}", err))?;
        let key = bytes_lossy(&attr.key.as_ref().to_vec());
        let value = attr
            .unescape_value()
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
fn parse_document(xml: &str) -> Result<Element, String> {
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
                let text = e
                    .unescape()
                    .map_err(|err| format!("XML text error: {}", err))?
                    .into_owned();
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

fn attach(stack: &mut [Element], roots: &mut Vec<Node>, node: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => roots.push(node),
    }
}

/// Namespace URI used to sort an attribute (unprefixed = no namespace; the
/// default namespace deliberately does NOT apply to attributes).
fn attr_ns_uri(a: &Attr, in_scope: &BTreeMap<String, String>) -> String {
    if a.prefix.is_empty() {
        String::new()
    } else if a.prefix == "xml" {
        XML_NS.to_string()
    } else {
        in_scope.get(&a.prefix).cloned().unwrap_or_default()
    }
}

/// Resolve an element's namespace URI, honouring its own declarations first
/// then the inherited in-scope context.
fn element_ns_uri(el: &Element, in_scope: &BTreeMap<String, String>) -> String {
    for (p, uri) in &el.ns_decls {
        if *p == el.prefix {
            return uri.clone();
        }
    }
    if el.prefix == "xml" {
        return XML_NS.to_string();
    }
    in_scope.get(&el.prefix).cloned().unwrap_or_default()
}

/// True if `el` is the `<ds:Signature>` element of an enveloped signature.
fn is_enveloped_signature(el: &Element, in_scope: &BTreeMap<String, String>) -> bool {
    el.local == "Signature" && element_ns_uri(el, in_scope) == XMLDSIG_NS
}

/// Recursively canonicalize `el`, appending to `out`.
///
/// `in_scope` is the input namespace context (prefix -> uri, "" = default;
/// an empty uri value means the default namespace is undeclared). `rendered`
/// is the *output* namespace context already emitted by ancestors — the heart
/// of the "exclusive" rule: a declaration is emitted only when visibly
/// utilized and not already in the output context.
fn canonicalize(
    el: &Element,
    in_scope_parent: &BTreeMap<String, String>,
    rendered_parent: &BTreeMap<String, String>,
    prefix_list: &BTreeSet<String>,
    enveloped: bool,
    out: &mut String,
) {
    let mut in_scope = in_scope_parent.clone();
    for (p, uri) in &el.ns_decls {
        in_scope.insert(p.clone(), uri.clone());
    }

    // Visibly utilized prefixes: the element's own prefix, every *prefixed*
    // attribute's prefix, plus the InclusiveNamespaces prefix list.
    let mut utilized: BTreeSet<String> = BTreeSet::new();
    utilized.insert(el.prefix.clone());
    for a in &el.attrs {
        if !a.prefix.is_empty() {
            utilized.insert(a.prefix.clone());
        }
    }
    for p in prefix_list {
        utilized.insert(p.clone());
    }

    let mut rendered = rendered_parent.clone();
    let mut to_render: Vec<(String, String)> = Vec::new();

    for p in &utilized {
        if p == "xml" {
            // The xml prefix is implicitly declared everywhere; never emitted.
            continue;
        }
        if p.is_empty() {
            let uri = in_scope.get("").cloned().unwrap_or_default();
            if uri.is_empty() {
                // Default namespace is "none": emit xmlns="" only to override
                // a non-empty default still in effect in the output.
                if rendered.get("").map(|u| !u.is_empty()).unwrap_or(false) {
                    to_render.push((String::new(), String::new()));
                    rendered.insert(String::new(), String::new());
                }
            } else if rendered.get("").map(|u| u != &uri).unwrap_or(true) {
                to_render.push((String::new(), uri.clone()));
                rendered.insert(String::new(), uri.clone());
            }
        } else if let Some(uri) = in_scope.get(p) {
            if !uri.is_empty() && rendered.get(p).map(|u| u != uri).unwrap_or(true) {
                to_render.push((p.clone(), uri.clone()));
                rendered.insert(p.clone(), uri.clone());
            }
        }
        // A used prefix that is not in scope cannot be rendered; skip it.
    }

    to_render.sort_by(|a, b| a.0.cmp(&b.0));

    let mut attrs_sorted: Vec<&Attr> = el.attrs.iter().collect();
    attrs_sorted.sort_by(|a, b| {
        attr_ns_uri(a, &in_scope)
            .cmp(&attr_ns_uri(b, &in_scope))
            .then_with(|| a.local.cmp(&b.local))
    });

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
    for a in &attrs_sorted {
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
                if enveloped && is_enveloped_signature(c, &in_scope) {
                    continue;
                }
                canonicalize(c, &in_scope, &rendered, prefix_list, enveloped, out);
            }
        }
    }

    out.push_str("</");
    out.push_str(&el.qname);
    out.push('>');
}

/// Find the element with the given Id, returning it together with the
/// namespace context inherited from its ancestors (so the subtree can be
/// canonicalized exactly as it would be in the full document).
fn find_by_id<'a>(
    el: &'a Element,
    target: &str,
    in_scope_parent: &BTreeMap<String, String>,
) -> Option<(&'a Element, BTreeMap<String, String>)> {
    if el.id.as_deref() == Some(target) {
        return Some((el, in_scope_parent.clone()));
    }
    let mut in_scope = in_scope_parent.clone();
    for (p, uri) in &el.ns_decls {
        in_scope.insert(p.clone(), uri.clone());
    }
    for child in &el.children {
        if let Node::Element(c) = child {
            if let Some(found) = find_by_id(c, target, &in_scope) {
                return Some(found);
            }
        }
    }
    None
}

/// Collect every element with the given local name, each paired with its
/// inherited namespace context. Used to locate `<ds:SignedInfo>` (which has no
/// Id) for verification.
fn collect_by_local<'a>(
    el: &'a Element,
    local: &str,
    in_scope_parent: &BTreeMap<String, String>,
    out: &mut Vec<(&'a Element, BTreeMap<String, String>)>,
) {
    if el.local == local {
        out.push((el, in_scope_parent.clone()));
    }
    let mut in_scope = in_scope_parent.clone();
    for (p, uri) in &el.ns_decls {
        in_scope.insert(p.clone(), uri.clone());
    }
    for child in &el.children {
        if let Node::Element(c) = child {
            collect_by_local(c, local, &in_scope, out);
        }
    }
}

fn root_in_scope() -> BTreeMap<String, String> {
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
        Some(want) => find_by_id(&root, want, &base)
            .ok_or_else(|| format!("no element with Id '{}' found", want))?,
        None => (&root, base),
    };
    let rendered = BTreeMap::new();
    let mut out = String::new();
    canonicalize(
        target,
        &in_scope_parent,
        &rendered,
        prefix_list,
        enveloped,
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
    // Namespaces declared locally on this element.
    let local_prefixes: BTreeSet<&str> = el.ns_decls.iter().map(|(p, _)| p.as_str()).collect();

    out.push('<');
    out.push_str(&el.qname);

    // Inject inherited namespaces not redeclared here (skip the implicit xml).
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

    // Child namespace context for recursion.
    let mut child_scope = inherited.clone();
    for (p, uri) in &el.ns_decls {
        child_scope.insert(p.clone(), uri.clone());
    }
    for child in &el.children {
        match child {
            Node::Text(t) => out.push_str(&escape_text(t)),
            Node::Pi(p) => {
                out.push_str("<?");
                out.push_str(p);
                out.push_str("?>");
            }
            Node::Element(c) => serialize_faithful(c, &child_scope, out),
        }
    }

    out.push_str("</");
    out.push_str(&el.qname);
    out.push('>');
}

/// Extract the subtree with the given Id as a standalone XML fragment.
fn extract_by_id(xml: &str, id: &str) -> Result<String, String> {
    let root = parse_document(xml)?;
    let base = root_in_scope();
    let (target, inherited) =
        find_by_id(&root, id, &base).ok_or_else(|| format!("no element with Id '{}' found", id))?;
    let mut out = String::new();
    serialize_faithful(target, &inherited, &mut out);
    Ok(out)
}

/// Extract every element with the given local name as standalone XML fragments.
fn extract_by_local(xml: &str, local: &str) -> Result<Vec<String>, String> {
    let root = parse_document(xml)?;
    let base = root_in_scope();
    let mut found = Vec::new();
    collect_by_local(&root, local, &base, &mut found);
    Ok(found
        .into_iter()
        .map(|(el, inherited)| {
            let mut out = String::new();
            serialize_faithful(el, &inherited, &mut out);
            out
        })
        .collect())
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

    #[test]
    fn extract_by_tag_finds_all_matches() {
        let xml = r#"<r><Item>1</Item><g><Item>2</Item></g></r>"#;
        let items = extract_by_local(xml, "Item").unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].contains(">1<"));
        assert!(items[1].contains(">2<"));
    }
}
