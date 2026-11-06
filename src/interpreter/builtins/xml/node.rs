//! The methods of `XmlNode` and `XmlDocument`, and the `Xml` statics that
//! build trees (`Xml.parse`, `Xml.document`, `Xml.element`, `Xml.build`, …).
//!
//! Every native here borrows its tree only for the length of the call and
//! never calls Soli code: the block of `node.add(…) do |child| … end` and
//! `Xml.build(…) do |root| … end` is run by the engines (see `block.rs`),
//! after the native returned.
//!
//! Queries (`find`, `find_all`, `xpath`, `at_xpath`) are lane L2's: they are
//! registered from [`register_query_methods`].

use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;

use super::value::{add_method, add_static, wrap, Arity, MethodTable, XmlRef};
use crate::interpreter::value::{HashKey, HashPairs, Value};
use crate::xml::{
    escape, parse, parse_bytes, DoctypeMode, NodeId, ParsePolicy, Proj, SerializeOptions, XmlError,
    XmlTree,
};

/// Deepest `to_h` value (hash and array levels) handed to Soli. Deeper
/// projections are refused so that converting, printing and dropping the
/// result never recurses past a bounded depth.
const MAX_TO_H_DEPTH: usize = 1024;

/// `find`, `find_all`, `xpath` and `at_xpath` (lane L2, `query.rs`).
pub(crate) fn register_query_methods(map: &mut MethodTable) {
    super::query::register(map);
}

// --------------------------------------------------------------- arguments

fn node_limit() -> usize {
    usize::try_from(crate::interpreter::limits::max_xml_nodes()).unwrap_or(usize::MAX)
}

fn is_callable(value: &Value) -> bool {
    matches!(
        value,
        Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
    )
}

/// A block reaching a native means an engine call site did not intercept
/// it: refuse loudly rather than ignore the block.
fn refuse_block(who: &str, args: &[Value]) -> Result<(), String> {
    if args.iter().any(is_callable) {
        return Err(format!(
            "{who}: the block could not be run here (an engine call site misses the XML block interception)"
        ));
    }
    Ok(())
}

/// The text a scalar is written as (`String`, `Int`, `Float`, `Bool`,
/// `Decimal`), or `None` for anything else.
fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.to_string()),
        Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Decimal(_) => {
            Some(value.to_string())
        }
        _ => None,
    }
}

fn string_arg(args: &[Value], index: usize, who: &str, what: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(format!(
            "{who}: {what} must be a String, got {}",
            other.type_name()
        )),
        None => Err(format!("{who}: missing {what}")),
    }
}

fn hash_key_name(key: &HashKey) -> Option<String> {
    match key {
        HashKey::String(s) | HashKey::Symbol(s) => Some(s.to_string()),
        _ => None,
    }
}

/// `attrs`: a Hash of name => scalar; a `nil` value omits the attribute.
fn attrs_from(value: Option<&Value>, who: &str) -> Result<Vec<(String, String)>, String> {
    let hash = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Hash(hash)) => hash,
        Some(other) => {
            return Err(format!(
                "{who}: attributes must be a Hash, got {}",
                other.type_name()
            ))
        }
    };
    let hash = hash.borrow();
    let mut out = Vec::with_capacity(hash.len());
    for (key, value) in hash.iter() {
        let Some(name) = hash_key_name(key) else {
            return Err(format!("{who}: attribute names must be Strings"));
        };
        if matches!(value, Value::Null) {
            continue;
        }
        let Some(text) = scalar_text(value) else {
            return Err(format!(
                "{who}: attribute '{name}' must be a String, Int, Float, Bool or Decimal, got {}",
                value.type_name()
            ));
        };
        out.push((name, text));
    }
    Ok(out)
}

fn pairs(attrs: &[(String, String)]) -> Vec<(&str, &str)> {
    attrs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

/// One piece of content: text, or a node to move (same tree) or copy.
enum Piece {
    Text(String),
    Node(XmlRef),
}

fn pieces_from(value: Option<&Value>, who: &str) -> Result<Vec<Piece>, String> {
    let mut out = Vec::new();
    match value {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for item in items.borrow().iter() {
                if matches!(item, Value::Null) {
                    continue;
                }
                out.push(piece(item, who)?);
            }
        }
        Some(other) => out.push(piece(other, who)?),
    }
    Ok(out)
}

fn piece(value: &Value, who: &str) -> Result<Piece, String> {
    if let Some(node) = super::value::value_as_xml(value) {
        return Ok(Piece::Node(node));
    }
    scalar_text(value).map(Piece::Text).ok_or_else(|| {
        format!(
            "{who}: content must be a String, Int, Float, Bool, Decimal, XmlNode or an Array of these, got {}",
            value.type_name()
        )
    })
}

/// An element's name, content pieces and attributes, as written in a call.
type ElementParts = (String, Vec<Piece>, Vec<(String, String)>);

/// `(name, content = nil, attrs = nil)`, where a Hash in the content position
/// is the attributes (`add("item", id: 1)`).
fn name_content_attrs(args: &[Value], who: &str) -> Result<ElementParts, String> {
    if args.len() > 3 {
        return Err(format!(
            "{who} takes (name, content = nil, attrs = nil), got {} arguments",
            args.len()
        ));
    }
    let name = string_arg(args, 0, who, "the element name")?;
    let (content, attrs) = match (args.get(1), args.get(2)) {
        (Some(Value::Hash(_)), None) => (None, args.get(1)),
        (Some(Value::Hash(_)), Some(_)) => {
            return Err(format!(
                "{who}: a Hash in the content position is the attributes; pass content first"
            ))
        }
        (content, attrs) => (content, attrs),
    };
    Ok((name, pieces_from(content, who)?, attrs_from(attrs, who)?))
}

fn xml_err(who: &str) -> impl Fn(XmlError) -> String + '_ {
    move |e| format!("{who}: {e}")
}

/// Make `piece` a node of `tree` (whose handle is `target`): a String becomes
/// a new text node, a node of the same tree is used as is (the caller moves
/// it), a node of another tree is deep-copied. A document stands for its root
/// element.
fn materialize(
    tree: &mut XmlTree,
    target: &XmlRef,
    piece: &Piece,
    who: &str,
) -> Result<NodeId, String> {
    match piece {
        Piece::Text(text) => tree.create_text(text).map_err(xml_err(who)),
        Piece::Node(node) if node.same_tree(target) => {
            if node.id == NodeId::DOCUMENT {
                tree.root_element()
                    .ok_or_else(|| format!("{who}: the document has no root element"))
            } else {
                Ok(node.id)
            }
        }
        Piece::Node(node) => {
            let source = node
                .tree
                .try_borrow()
                .map_err(|_| format!("{who}: the XML tree is being modified"))?;
            if source.get(node.id).is_none() {
                return Err(format!("{who}: the XML node does not belong to its tree"));
            }
            let id = if node.id == NodeId::DOCUMENT {
                source
                    .root_element()
                    .ok_or_else(|| format!("{who}: the document has no root element"))?
            } else {
                node.id
            };
            tree.import(&source, id).map_err(xml_err(who))
        }
    }
}

/// Append every piece under `parent`.
fn append_pieces(
    tree: &mut XmlTree,
    target: &XmlRef,
    parent: NodeId,
    pieces: &[Piece],
    who: &str,
) -> Result<(), String> {
    for piece in pieces {
        match piece {
            Piece::Text(text) => {
                tree.add_text(parent, text).map_err(xml_err(who))?;
            }
            Piece::Node(_) => {
                let id = materialize(tree, target, piece, who)?;
                tree.append_child(parent, id).map_err(xml_err(who))?;
            }
        }
    }
    Ok(())
}

fn opt_node(node: &XmlRef, id: Option<NodeId>) -> Value {
    id.map_or(Value::Null, |id| wrap(node.at(id)))
}

fn node_array(node: &XmlRef, ids: impl IntoIterator<Item = NodeId>) -> Value {
    let items: Vec<Value> = ids.into_iter().map(|id| wrap(node.at(id))).collect();
    Value::Array(Rc::new(RefCell::new(items)))
}

fn opt_string(s: Option<&str>) -> Value {
    s.map_or(Value::Null, |s| Value::String(s.into()))
}

fn string_hash(entries: impl IntoIterator<Item = (String, Value)>) -> Value {
    let mut map = HashPairs::default();
    for (key, value) in entries {
        map.insert(HashKey::String(key.into()), value);
    }
    Value::Hash(Rc::new(RefCell::new(map)))
}

/// A projection as a Soli value, refusing more than [`MAX_TO_H_DEPTH`]
/// levels.
fn proj_to_value(proj: &Proj, depth: usize) -> Result<Value, String> {
    if depth > MAX_TO_H_DEPTH {
        return Err(format!(
            "XmlNode.to_h: more than {MAX_TO_H_DEPTH} nested levels"
        ));
    }
    Ok(match proj {
        Proj::Null => Value::Null,
        Proj::Str(s) => Value::String(s.as_str().into()),
        Proj::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(proj_to_value(item, depth + 1)?);
            }
            Value::Array(Rc::new(RefCell::new(out)))
        }
        Proj::Map(map) => {
            let mut out = HashPairs::default();
            for (key, value) in map {
                out.insert(
                    HashKey::String(key.as_str().into()),
                    proj_to_value(value, depth + 1)?,
                );
            }
            Value::Hash(Rc::new(RefCell::new(out)))
        }
    })
}

// ------------------------------------------------------------ options

/// The options Hash of a call (`opts = nil`, or named arguments), with each
/// key checked against `known`.
pub(crate) fn options(
    value: Option<&Value>,
    who: &str,
    known: &[&str],
) -> Result<Vec<(String, Value)>, String> {
    let hash = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Hash(hash)) => hash,
        Some(other) => {
            return Err(format!(
                "{who}: options must be a Hash, got {}",
                other.type_name()
            ))
        }
    };
    let mut out = Vec::new();
    for (key, value) in hash.borrow().iter() {
        let name = hash_key_name(key).unwrap_or_default();
        if !known.contains(&name.as_str()) {
            return Err(format!(
                "{who}: unknown option '{name}' (expected one of: {})",
                known.join(", ")
            ));
        }
        out.push((name, value.clone()));
    }
    Ok(out)
}

fn int_option(value: &Value, who: &str, name: &str) -> Result<usize, String> {
    match value {
        Value::Int(n) if *n >= 0 => Ok(usize::try_from(*n).unwrap_or(usize::MAX)),
        _ => Err(format!(
            "{who}: option '{name}' must be a non-negative Int, got {}",
            value.type_name()
        )),
    }
}

fn bool_option(value: &Value, who: &str, name: &str) -> Result<bool, String> {
    match value {
        Value::Bool(b) => Ok(*b),
        Value::Null => Ok(false),
        _ => Err(format!(
            "{who}: option '{name}' must be a Bool, got {}",
            value.type_name()
        )),
    }
}

/// The `doctype:` option: `"refuse"` (the default, also `nil`) or `"skip"`.
/// Shared with `Xml.each` / `Xml.reader`.
pub(crate) fn doctype_option(value: &Value, who: &str) -> Result<DoctypeMode, String> {
    let mode = match value {
        Value::Null => Some(DoctypeMode::Refuse),
        Value::String(s) => DoctypeMode::from_option(s),
        _ => None,
    };
    mode.ok_or_else(|| {
        let got = match value {
            Value::String(s) => format!("\"{s}\""),
            other => other.type_name().to_string(),
        };
        format!("{who}: option 'doctype' must be \"refuse\" or \"skip\", got {got}")
    })
}

/// `Xml.parse` / `Xml.parse_file` options: `strip_blank`, `max_depth`,
/// `max_nodes`, `max_bytes` (limits can only be lowered), `doctype`.
fn parse_policy(value: Option<&Value>, who: &str) -> Result<ParsePolicy, String> {
    let (mut depth, mut nodes, mut bytes, mut strip) = (None, None, None, false);
    let mut doctype = DoctypeMode::Refuse;
    for (name, value) in options(
        value,
        who,
        &[
            "strip_blank",
            "max_depth",
            "max_nodes",
            "max_bytes",
            "doctype",
        ],
    )? {
        match name.as_str() {
            "strip_blank" => strip = bool_option(&value, who, &name)?,
            "doctype" => doctype = doctype_option(&value, who)?,
            "max_depth" => depth = Some(int_option(&value, who, &name)?),
            "max_nodes" => nodes = Some(int_option(&value, who, &name)?),
            _ => bytes = Some(int_option(&value, who, &name)?),
        }
    }
    Ok(ParsePolicy::xml()
        .lowered(depth, nodes, bytes)
        .strip_blank(strip)
        .with_doctype(doctype))
}

fn serialize_options(value: Option<&Value>, who: &str) -> Result<SerializeOptions, String> {
    let mut opts = SerializeOptions::compact();
    for (name, value) in options(value, who, &["indent", "declaration", "expand_empty"])? {
        match name.as_str() {
            "indent" => {
                opts.indent = match value {
                    Value::Null => None,
                    other => Some(int_option(&other, who, &name)?.min(64)),
                }
            }
            "declaration" => {
                opts.declaration = match value {
                    Value::Null => None,
                    other => Some(bool_option(&other, who, &name)?),
                }
            }
            _ => opts.expand_empty = bool_option(&value, who, &name)?,
        }
    }
    Ok(opts)
}

// ------------------------------------------------------------ parsing

fn parse_string(args: &[Value]) -> Result<Value, String> {
    const WHO: &str = "Xml.parse";
    if args.is_empty() || args.len() > 2 {
        return Err(format!(
            "{WHO}(xml, opts = nil) takes 1 or 2 arguments, got {}",
            args.len()
        ));
    }
    let Value::String(source) = &args[0] else {
        return Err(format!(
            "{WHO}: expects the XML as a String, got {} (use Xml.parse_file for a path)",
            args[0].type_name()
        ));
    };
    let policy = parse_policy(args.get(1), WHO)?;
    let tree = parse(source, &policy).map_err(xml_err(WHO))?;
    Ok(wrap(XmlRef::new(tree, NodeId::DOCUMENT)))
}

fn parse_file(args: &[Value]) -> Result<Value, String> {
    const WHO: &str = "Xml.parse_file";
    if args.is_empty() || args.len() > 2 {
        return Err(format!(
            "{WHO}(path, opts = nil) takes 1 or 2 arguments, got {}",
            args.len()
        ));
    }
    let path = string_arg(args, 0, WHO, "the path")?;
    let policy = parse_policy(args.get(1), WHO)?;
    let resolved = crate::interpreter::builtins::file::resolve_readable_path(&path, WHO)?;
    // Refuse a FIFO or a device before opening it: opening a FIFO for
    // reading blocks until a writer comes (as `Xml.each` / `Xml.reader` do).
    // The check after the open is the one that counts.
    if let Ok(meta) = std::fs::symlink_metadata(&resolved) {
        if !meta.is_file() && !meta.file_type().is_symlink() {
            return Err(format!("{WHO}: cannot read {path}: not a file"));
        }
    }
    let file = crate::interpreter::builtins::file::open_for_read(&resolved, false)
        .map_err(|e| format!("{WHO}: cannot open {path}: {e}"))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("{WHO}: cannot read {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{WHO}: cannot read {path}: not a file"));
    }
    let size = meta.len();
    let limit = policy.max_bytes;
    if size > limit as u64 {
        return Err(format!(
            "{WHO}: {}",
            XmlError::new(crate::xml::XmlErrorKind::BytesExceeded(
                usize::try_from(size).ok(),
                limit
            ))
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{WHO}: cannot read {path}: {e}"))?;
    let tree = parse_bytes(&bytes, &policy).map_err(xml_err(WHO))?;
    Ok(wrap(XmlRef::new(tree, NodeId::DOCUMENT)))
}

// ------------------------------------------------------------ building

/// `Xml.document(root_name = nil, attrs = nil)` and `Xml.build(root_name,
/// attrs = nil)` (whose block the engines run with the root).
fn new_document(args: &[Value], who: &str, root_required: bool) -> Result<Value, String> {
    refuse_block(who, args)?;
    if args.len() > 2 {
        return Err(format!(
            "{who}(root_name, attrs = nil) takes at most 2 arguments, got {}",
            args.len()
        ));
    }
    let root = match args.first() {
        None | Some(Value::Null) if !root_required => None,
        _ => Some(string_arg(args, 0, who, "the root element name")?),
    };
    let attrs = attrs_from(args.get(1), who)?;
    let attr_pairs = pairs(&attrs);
    let (tree, _) = XmlTree::new_document(
        node_limit(),
        root.as_deref().map(|name| (name, attr_pairs.as_slice())),
    )
    .map_err(xml_err(who))?;
    Ok(wrap(XmlRef::new(tree, NodeId::DOCUMENT)))
}

fn new_element(args: &[Value]) -> Result<Value, String> {
    const WHO: &str = "Xml.element";
    refuse_block(WHO, args)?;
    let (name, content, attrs) = name_content_attrs(args, WHO)?;
    let (tree, id) =
        XmlTree::new_fragment(node_limit(), &name, &pairs(&attrs)).map_err(xml_err(WHO))?;
    let node = XmlRef::new(tree, id);
    node.write(|tree, id| append_pieces(tree, &node, id, &content, WHO))??;
    Ok(wrap(node))
}

/// `node.add(name, content = nil, attrs = nil)`: the new child.
fn add(node: &XmlRef, args: &[Value]) -> Result<Value, String> {
    const WHO: &str = "XmlNode.add";
    refuse_block(WHO, args)?;
    let (name, content, attrs) = name_content_attrs(args, WHO)?;
    let child = node.write(|tree, id| {
        let child = tree
            .add_element(id, &name, &pairs(&attrs))
            .map_err(xml_err(WHO))?;
        // All or nothing: content that fails (a cycle, a bad piece) takes
        // the new element back out rather than leaving it half-built.
        if let Err(err) = append_pieces(tree, node, child, &content, WHO) {
            let _ = tree.detach(child);
            return Err(err);
        }
        Ok::<_, String>(child)
    })??;
    Ok(wrap(node.at(child)))
}

/// `append`, `prepend`, `insert_before`, `insert_after`, `replace`: put `x`
/// (an XmlNode or a String) in place and return what was inserted (the copy,
/// for a node of another tree).
fn insert(node: &XmlRef, args: &[Value], method: &str) -> Result<Value, String> {
    let who = format!("XmlNode.{method}");
    let Some(x) = args.first() else {
        return Err(format!("{who}: missing the node to insert"));
    };
    let piece = piece(x, &who)?;
    let inserted = node.write(|tree, id| {
        let new = materialize(tree, node, &piece, &who)?;
        let err = xml_err(&who);
        match method {
            "append" => tree.append_child(id, new).map_err(err),
            "prepend" => tree.prepend_child(id, new).map_err(err),
            "insert_before" => tree.insert_before(id, new).map_err(err),
            "insert_after" => tree.insert_after(id, new).map_err(err),
            _ => tree.replace(id, new).map_err(err),
        }
    })??;
    Ok(wrap(node.at(inserted)))
}

/// `node[key] = value` (an attribute; `nil` removes it).
pub(crate) fn set_attribute(node: &XmlRef, key: &str, value: &Value) -> Result<(), String> {
    const WHO: &str = "XmlNode[]=";
    let text = match value {
        Value::Null => None,
        other => Some(scalar_text(other).ok_or_else(|| {
            format!(
                "{WHO}: attribute '{key}' must be a String, Int, Float, Bool, Decimal or nil, got {}",
                other.type_name()
            )
        })?),
    };
    node.write(|tree, id| tree.set_attribute(id, key, text.as_deref()))?
        .map(|_| ())
        .map_err(xml_err(WHO))
}

/// `node.text = s`, `node.name = s`; every other dot write is refused.
pub(crate) fn write_property(
    node: &XmlRef,
    class_name: &str,
    name: &str,
    value: &Value,
) -> Result<(), String> {
    match name {
        "text" => {
            let text = match value {
                Value::Null => String::new(),
                other => scalar_text(other).ok_or_else(|| {
                    format!("XmlNode.text=: expects a String, got {}", other.type_name())
                })?,
            };
            node.write(|tree, id| tree.set_text(id, &text))?
                .map_err(xml_err("XmlNode.text="))
        }
        "name" => {
            let Value::String(new_name) = value else {
                return Err(format!(
                    "XmlNode.name=: expects a String, got {}",
                    value.type_name()
                ));
            };
            node.write(|tree, id| tree.rename(id, new_name))?
                .map_err(xml_err("XmlNode.name="))
        }
        _ => Err(format!("{class_name} has no writable property '{name}'")),
    }
}

// ------------------------------------------------------------ the tables

/// The `Xml` statics this lane adds (the c14n trio is registered beside
/// them by `mod.rs`).
pub(crate) fn xml_statics(table: &mut MethodTable) {
    add_static(table, "Xml", "parse", Arity::Variadic, parse_string);
    add_static(table, "Xml", "parse_file", Arity::Variadic, parse_file);
    add_static(table, "Xml", "well_formed?", Arity::Exact(1), |args| {
        Ok(Value::Bool(match args.first() {
            Some(Value::String(s)) => parse(s, &ParsePolicy::xml()).is_ok(),
            _ => false,
        }))
    });
    add_static(table, "Xml", "escape", Arity::Exact(1), |args| {
        let value = args.first().unwrap_or(&Value::Null);
        match scalar_text(value) {
            Some(text) => Ok(Value::String(escape::escape5(&text).into())),
            None if matches!(value, Value::Null) => Ok(Value::String("".into())),
            None => Err(format!(
                "Xml.escape: expects a String, got {}",
                value.type_name()
            )),
        }
    });
    add_static(table, "Xml", "document", Arity::Optional, |args| {
        new_document(args, "Xml.document", false)
    });
    add_static(table, "Xml", "build", Arity::Variadic, |args| {
        new_document(args, "Xml.build", true)
    });
    add_static(table, "Xml", "element", Arity::Variadic, new_element);
}

/// `XmlNode`'s methods (inherited by `XmlDocument`).
pub(crate) fn node_methods() -> MethodTable {
    let mut t = MethodTable::new();

    // Identity and naming (§1.3).
    add_method(&mut t, "kind", Arity::Zero, |n, _| {
        n.read(|tree, id| Value::String(tree.kind(id).name().into()))
    });
    for (method, kind) in [
        ("element?", "element"),
        ("text?", "text"),
        ("cdata?", "cdata"),
        ("comment?", "comment"),
        ("pi?", "pi"),
        ("document?", "document"),
    ] {
        add_method(&mut t, method, Arity::Zero, move |n, _| {
            n.read(|tree, id| Value::Bool(tree.kind(id).name() == kind))
        });
    }
    add_method(&mut t, "name", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_string(tree.name(id)))
    });
    add_method(&mut t, "local_name", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_string(tree.element(id).map(|el| el.local_name)))
    });
    add_method(&mut t, "prefix", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_string(tree.element(id).and_then(|el| el.prefix)))
    });
    add_method(&mut t, "namespace", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_string(tree.element(id).and_then(|el| el.ns_uri.map(|u| &**u))))
    });
    add_method(&mut t, "namespaces", Arity::Zero, |n, _| {
        n.read(|tree, id| {
            if tree.element(id).is_none() {
                return string_hash(Vec::new());
            }
            string_hash(tree.in_scope_namespaces(id).into_iter().map(|decl| {
                (
                    decl.prefix.as_deref().unwrap_or("").to_string(),
                    Value::String((&*decl.uri).into()),
                )
            }))
        })
    });
    add_method(&mut t, "namespace_declarations", Arity::Zero, |n, _| {
        n.read(|tree, id| {
            let decls = tree.element(id).map(|el| el.ns_decls).unwrap_or(&[]);
            string_hash(decls.iter().map(|decl| {
                (
                    decl.prefix.as_deref().unwrap_or("").to_string(),
                    Value::String((&*decl.uri).into()),
                )
            }))
        })
    });
    add_method(&mut t, "path", Arity::Zero, |n, _| {
        n.read(|tree, id| Value::String(tree.path(id).into()))
    });

    // Navigation (§1.4).
    add_method(&mut t, "children", Arity::Zero, |n, _| {
        n.read(|tree, id| node_array(n, tree.element_children(id).collect::<Vec<_>>()))
    });
    add_method(&mut t, "nodes", Arity::Zero, |n, _| {
        n.read(|tree, id| node_array(n, tree.children(id).to_vec()))
    });
    add_method(&mut t, "parent", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_node(n, tree.parent_element(id)))
    });
    add_method(&mut t, "document", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_node(n, tree.is_attached(id).then_some(NodeId::DOCUMENT)))
    });
    add_method(&mut t, "next_sibling", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_node(n, tree.next_sibling(id)))
    });
    add_method(&mut t, "previous_sibling", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_node(n, tree.previous_sibling(id)))
    });
    add_method(&mut t, "next_node", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_node(n, tree.next_node(id)))
    });
    add_method(&mut t, "previous_node", Arity::Zero, |n, _| {
        n.read(|tree, id| opt_node(n, tree.previous_node(id)))
    });
    add_method(&mut t, "ancestors", Arity::Zero, |n, _| {
        n.read(|tree, id| node_array(n, tree.ancestors(id)))
    });
    add_method(&mut t, "descendants", Arity::Zero, |n, _| {
        n.read(|tree, id| node_array(n, tree.descendants(id)))
    });

    // Text and attributes (§1.5).
    add_method(&mut t, "text", Arity::Zero, |n, _| {
        n.read(|tree, id| Value::String(tree.text(id).into()))
    });
    add_method(&mut t, "own_text", Arity::Zero, |n, _| {
        n.read(|tree, id| Value::String(tree.own_text(id).into()))
    });
    add_method(&mut t, "inner_xml", Arity::Zero, |n, _| {
        n.read(|tree, id| {
            crate::xml::serialize_children(tree, id, &SerializeOptions::compact())
                .map(|s| Value::String(s.into()))
                .map_err(xml_err("XmlNode.inner_xml"))
        })?
    });
    add_method(&mut t, "get", Arity::Exact(1), |n, args| {
        let name = string_arg(args, 0, "XmlNode.get", "the attribute name")?;
        n.read(|tree, id| opt_string(tree.attribute(id, &name)))
    });
    // `set` is registered because the VM compiles `node["k"] = v` and
    // `node.set("k", v)` to the same instruction: both set the attribute on
    // both engines, and return the value.
    add_method(&mut t, "set", Arity::Exact(2), |n, args| {
        let name = string_arg(args, 0, "XmlNode.set", "the attribute name")?;
        let value = args.get(1).cloned().unwrap_or(Value::Null);
        set_attribute(n, &name, &value)?;
        Ok(value)
    });
    add_method(&mut t, "attribute", Arity::Variadic, |n, args| {
        const WHO: &str = "XmlNode.attribute";
        if args.is_empty() || args.len() > 2 {
            return Err(format!(
                "{WHO}(local_name, namespace = nil) takes 1 or 2 arguments, got {}",
                args.len()
            ));
        }
        let local = string_arg(args, 0, WHO, "the local name")?;
        let ns = match args.get(1) {
            None | Some(Value::Null) => None,
            Some(_) => Some(string_arg(args, 1, WHO, "the namespace")?),
        };
        n.read(|tree, id| opt_string(tree.attribute_ns(id, &local, ns.as_deref())))
    });
    add_method(&mut t, "attributes", Arity::Zero, |n, _| {
        n.read(|tree, id| {
            let attrs = tree.element(id).map(|el| el.attrs).unwrap_or(&[]);
            string_hash(
                attrs
                    .iter()
                    .map(|a| (a.qname.to_string(), Value::String((&*a.value).into()))),
            )
        })
    });
    add_method(&mut t, "has_attribute?", Arity::Exact(1), |n, args| {
        let name = string_arg(args, 0, "XmlNode.has_attribute?", "the attribute name")?;
        n.read(|tree, id| Value::Bool(tree.attribute(id, &name).is_some()))
    });

    // Building and mutating (§1.7).
    add_method(&mut t, "add", Arity::Optional, add);
    add_method(&mut t, "add_text", Arity::Exact(1), |n, args| {
        add_leaf(n, args, "add_text")
    });
    add_method(&mut t, "add_cdata", Arity::Exact(1), |n, args| {
        add_leaf(n, args, "add_cdata")
    });
    add_method(&mut t, "add_comment", Arity::Exact(1), |n, args| {
        add_leaf(n, args, "add_comment")
    });
    add_method(&mut t, "add_pi", Arity::Variadic, |n, args| {
        const WHO: &str = "XmlNode.add_pi";
        if args.is_empty() || args.len() > 2 {
            return Err(format!(
                "{WHO}(target, data = \"\") takes 1 or 2 arguments, got {}",
                args.len()
            ));
        }
        let target = string_arg(args, 0, WHO, "the target")?;
        let data = match args.get(1) {
            None | Some(Value::Null) => String::new(),
            Some(value) => {
                scalar_text(value).ok_or_else(|| format!("{WHO}: data must be a String"))?
            }
        };
        let pi = n.write(|tree, id| tree.add_pi(id, &target, &data).map_err(xml_err(WHO)))??;
        Ok(wrap(n.at(pi)))
    });
    for method in [
        "append",
        "prepend",
        "insert_before",
        "insert_after",
        "replace",
    ] {
        add_method(&mut t, method, Arity::Exact(1), move |n, args| {
            insert(n, args, method)
        });
    }
    add_method(&mut t, "remove", Arity::Zero, |n, _| {
        let id = n.write(|tree, id| tree.detach(id).map_err(xml_err("XmlNode.remove")))??;
        Ok(wrap(n.at(id)))
    });
    add_method(&mut t, "remove_attribute", Arity::Exact(1), |n, args| {
        const WHO: &str = "XmlNode.remove_attribute";
        let name = string_arg(args, 0, WHO, "the attribute name")?;
        let old = n.write(|tree, id| tree.remove_attribute(id, &name).map_err(xml_err(WHO)))??;
        Ok(old.map_or(Value::Null, |s| Value::String(s.into())))
    });
    add_method(&mut t, "dup", Arity::Zero, |n, _| {
        let (tree, id) = n.read(|tree, id| tree.dup(id).map_err(xml_err("XmlNode.dup")))??;
        Ok(wrap(XmlRef::new(tree, id)))
    });

    // Serialisation (§1.8) and projection (§1.9).
    for method in ["to_xml", "to_s"] {
        add_method(&mut t, method, Arity::Optional, move |n, args| {
            let who = format!("XmlNode.{method}");
            if args.len() > 1 {
                return Err(format!(
                    "{who}(opts = nil) takes at most 1 argument, got {}",
                    args.len()
                ));
            }
            let opts = serialize_options(args.first(), &who)?;
            n.read(|tree, id| {
                crate::xml::serialize(tree, id, &opts)
                    .map(|s| Value::String(s.into()))
                    .map_err(xml_err(&who))
            })?
        });
    }
    add_method(&mut t, "to_h", Arity::Zero, |n, _| {
        let proj = n.read(crate::xml::to_h)?;
        proj_to_value(&proj, 0)
    });

    register_query_methods(&mut t);
    t
}

fn add_leaf(node: &XmlRef, args: &[Value], method: &str) -> Result<Value, String> {
    let who = format!("XmlNode.{method}");
    let text = match args.first().unwrap_or(&Value::Null) {
        Value::Null => String::new(),
        other => scalar_text(other)
            .ok_or_else(|| format!("{who}: expects a String, got {}", other.type_name()))?,
    };
    let id = node.write(|tree, id| {
        let err = xml_err(&who);
        match method {
            "add_text" => tree.add_text(id, &text).map_err(err),
            "add_cdata" => tree.add_cdata(id, &text).map_err(err),
            _ => tree.add_comment(id, &text).map_err(err),
        }
    })??;
    Ok(wrap(node.at(id)))
}

/// What `XmlDocument` adds to `XmlNode`.
pub(crate) fn document_methods() -> MethodTable {
    let mut t = MethodTable::new();
    add_method(&mut t, "root", Arity::Zero, |n, _| {
        n.read(|tree, _| opt_node(n, tree.root_element()))
    });
    add_method(&mut t, "declaration", Arity::Zero, |n, _| {
        n.read(|tree, _| match tree.decl() {
            None => Value::Null,
            Some(decl) => string_hash([
                (
                    "version".to_string(),
                    Value::String((&*decl.version).into()),
                ),
                ("encoding".to_string(), opt_string(decl.encoding.as_deref())),
                (
                    "standalone".to_string(),
                    opt_string(decl.standalone.as_deref()),
                ),
            ]),
        })
    });
    // The name of the DOCTYPE `doctype: "skip"` read past, or nil.
    add_method(&mut t, "doctype", Arity::Zero, |n, _| {
        n.read(|tree, _| opt_string(tree.doctype()))
    });
    t
}
