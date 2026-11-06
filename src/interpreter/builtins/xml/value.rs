//! How an XML tree reaches Soli code: the handle, its classes, and the hooks
//! the two engines call.
//!
//! **Frozen for lanes L2 (queries) and L3 (streaming)**: [`XmlRef`],
//! [`XmlPayload`], [`wrap`], [`as_xml`], [`node_class`], [`document_class`],
//! the [`MethodTable`] helpers ([`Arity`], [`add_static`], [`add_method`]) and
//! the hook signatures. A lane may add a payload variant (L3's reader) and
//! the arms that go with it; it does not change these signatures.
//!
//! A node handle is a `Value::Instance` of the thread-local `XmlNode` class
//! (or `XmlDocument` for the document node) holding one field, `__xml`, whose
//! value is `Value::Xml(Rc<XmlPayload>)`. `__xml` is a `const` field, so the
//! VM never takes its in-place field-write fast path on these instances: every
//! write reaches `set_instance_field`, where [`field_write`] answers first.
//!
//! [`as_xml`] succeeds only for an instance whose class **is** (pointer-equal
//! to) the thread-local class: a user class named `XmlNode` never passes.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use crate::interpreter::value::{Class, Instance, NativeFunction, Value, NATIVE_CONSTRUCTOR};
use crate::xml::{NodeId, SerializeOptions, XmlTree};

/// The hidden field that holds the payload.
pub const HANDLE_FIELD: &str = "__xml";

/// One node of one tree. Cheap to clone: the tree is shared.
#[derive(Clone)]
pub struct XmlRef {
    pub tree: Rc<RefCell<XmlTree>>,
    pub id: NodeId,
}

impl XmlRef {
    /// A handle on `id` of a tree that has no other handle yet.
    pub fn new(tree: XmlTree, id: NodeId) -> Self {
        XmlRef {
            tree: Rc::new(RefCell::new(tree)),
            id,
        }
    }

    /// Another node of the same tree.
    pub fn at(&self, id: NodeId) -> Self {
        XmlRef {
            tree: self.tree.clone(),
            id,
        }
    }

    pub fn same_tree(&self, other: &XmlRef) -> bool {
        Rc::ptr_eq(&self.tree, &other.tree)
    }

    /// Identity: the same tree and the same node (`==` on handles).
    pub fn same_node(&self, other: &XmlRef) -> bool {
        self.same_tree(other) && self.id == other.id
    }

    /// Read the tree. The id is checked against the tree (an id from another
    /// tree is an error, never a panic), and the borrow ends with `f`.
    pub fn read<R>(&self, f: impl FnOnce(&XmlTree, NodeId) -> R) -> Result<R, String> {
        let tree = self
            .tree
            .try_borrow()
            .map_err(|_| "the XML tree is being modified".to_string())?;
        if tree.get(self.id).is_none() {
            return Err("the XML node does not belong to its tree".to_string());
        }
        Ok(f(&tree, self.id))
    }

    /// Mutate the tree, with the same checks as [`XmlRef::read`].
    pub fn write<R>(&self, f: impl FnOnce(&mut XmlTree, NodeId) -> R) -> Result<R, String> {
        let mut tree = self
            .tree
            .try_borrow_mut()
            .map_err(|_| "the XML tree is already in use".to_string())?;
        if tree.get(self.id).is_none() {
            return Err("the XML node does not belong to its tree".to_string());
        }
        Ok(f(&mut tree, self.id))
    }

    /// The compact `to_xml` of this node (what `print`, `#{}`, `inspect` and
    /// JSON show).
    pub fn compact_xml(&self) -> Result<String, String> {
        self.read(|tree, id| crate::xml::serialize(tree, id, &SerializeOptions::compact()))?
            .map_err(|e| e.to_string())
    }
}

/// What `Value::Xml` holds.
pub enum XmlPayload {
    /// A node (or the document node) of a tree.
    Node(XmlRef),
}

impl fmt::Debug for XmlPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmlPayload::Node(node) => write!(f, "XmlPayload::Node({})", node.id.0),
        }
    }
}

// ------------------------------------------------------------------ classes

/// The native methods of a class, by name.
pub type MethodTable = HashMap<String, Rc<NativeFunction>>;

/// How a native method is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    /// No argument: `Some(0)`, run when read bare (`node.name`).
    Zero,
    /// Exactly `n` arguments, never run bare.
    Exact(usize),
    /// Optional arguments (`to_xml(opts = nil)`): variadic **and** run when
    /// read bare, with no argument.
    Optional,
    /// Variadic, at least one argument: never run bare.
    Variadic,
}

impl Arity {
    fn split(self) -> (Option<usize>, bool) {
        match self {
            Arity::Zero => (Some(0), true),
            Arity::Exact(n) => (Some(n), false),
            Arity::Optional => (None, true),
            Arity::Variadic => (None, false),
        }
    }
}

/// Register a static method `class.name` (`Xml.parse`): `f` gets the call's
/// arguments.
pub fn add_static(
    table: &mut MethodTable,
    class: &str,
    name: &str,
    arity: Arity,
    f: impl Fn(&[Value]) -> Result<Value, String> + 'static,
) {
    let (count, auto) = arity.split();
    table.insert(
        name.to_string(),
        Rc::new(NativeFunction::new_with_auto(
            format!("{class}.{name}"),
            count,
            auto,
            f,
        )),
    );
}

/// Register a node method `XmlNode.name`: `f` gets the receiver and the
/// arguments after it. A receiver that is not an XML node is refused.
pub fn add_method(
    table: &mut MethodTable,
    name: &str,
    arity: Arity,
    f: impl Fn(&XmlRef, &[Value]) -> Result<Value, String> + 'static,
) {
    let (count, auto) = arity.split();
    let qualified = format!("XmlNode.{name}");
    let label = qualified.clone();
    table.insert(
        name.to_string(),
        Rc::new(NativeFunction::new_with_auto(
            qualified,
            count,
            auto,
            move |args| {
                let receiver = match args.first() {
                    Some(Value::Instance(inst)) => as_xml(&inst.borrow()),
                    _ => None,
                };
                let Some(receiver) = receiver else {
                    return Err(format!("{label} must be called on an XML node"));
                };
                f(&receiver, &args[1..])
            },
        )),
    );
}

fn build_class(name: &str, superclass: Option<Rc<Class>>, mut methods: MethodTable) -> Rc<Class> {
    let refusal = format!("{name} has no constructor: use Xml.parse, Xml.element or node.add");
    methods.insert(
        NATIVE_CONSTRUCTOR.to_string(),
        Rc::new(NativeFunction::new(name.to_string(), None, move |_| {
            Err(refusal.clone())
        })),
    );
    let mut const_fields = HashSet::new();
    const_fields.insert(HANDLE_FIELD.to_string());
    Rc::new(Class {
        name: name.to_string(),
        superclass,
        native_methods: methods,
        const_fields,
        ..Default::default()
    })
}

thread_local! {
    static NODE_CLASS: Rc<Class> =
        build_class("XmlNode", None, super::node::node_methods());
    static DOCUMENT_CLASS: Rc<Class> = build_class(
        "XmlDocument",
        Some(node_class()),
        super::node::document_methods(),
    );
}

/// The `XmlNode` class of this thread.
pub fn node_class() -> Rc<Class> {
    NODE_CLASS.with(Rc::clone)
}

/// The `XmlDocument` class of this thread (`< XmlNode`).
pub fn document_class() -> Rc<Class> {
    DOCUMENT_CLASS.with(Rc::clone)
}

/// Is `class` this thread's `XmlNode` or `XmlDocument`? One name test, then
/// a pointer comparison: cheap enough for the engines' Instance arms.
#[inline]
pub fn is_node_class(class: &Rc<Class>) -> bool {
    class.name.starts_with("Xml")
        && (NODE_CLASS.with(|c| Rc::ptr_eq(c, class))
            || DOCUMENT_CLASS.with(|c| Rc::ptr_eq(c, class)))
}

/// The Soli handle of a node: an `XmlDocument` for the document node, an
/// `XmlNode` otherwise.
pub fn wrap(node: XmlRef) -> Value {
    let class = if node.id == NodeId::DOCUMENT {
        document_class()
    } else {
        node_class()
    };
    let mut inst = Instance::new(class);
    inst.set(HANDLE_FIELD, Value::Xml(Rc::new(XmlPayload::Node(node))));
    Value::Instance(Rc::new(RefCell::new(inst)))
}

/// The node behind a handle, or `None` for any other instance.
pub fn as_xml(inst: &Instance) -> Option<XmlRef> {
    if !is_node_class(&inst.class) {
        return None;
    }
    match inst.fields.get(HANDLE_FIELD) {
        Some(Value::Xml(payload)) => match &**payload {
            XmlPayload::Node(node) => Some(node.clone()),
        },
        _ => None,
    }
}

/// [`as_xml`] on any value.
pub fn value_as_xml(value: &Value) -> Option<XmlRef> {
    match value {
        Value::Instance(inst) => as_xml(&inst.borrow()),
        _ => None,
    }
}

// -------------------------------------------------------------------- hooks
//
// Each returns `None` for an instance that is not an XML handle, so a caller
// falls through to its usual behaviour.

/// `node["k"]` and `node.get("k")`: the attribute by qname, or `nil` (also on
/// a non-element, and for `node["__xml"]`: the handle is never reachable).
pub fn index_get(inst: &Instance, key: &str) -> Option<Value> {
    let node = as_xml(inst)?;
    Some(
        node.read(|tree, id| tree.attribute(id, key).map(|v| Value::String(v.into())))
            .ok()
            .flatten()
            .unwrap_or(Value::Null),
    )
}

/// `node["k"] = v`: set (or, with `nil`, remove) an attribute.
pub fn index_set(inst: &Instance, key: &str, value: &Value) -> Option<Result<(), String>> {
    let node = as_xml(inst)?;
    Some(super::node::set_attribute(&node, key, value))
}

/// `node.text = s`, `node.name = s`; any other dot write is refused. No field
/// is ever created on an XML handle.
pub fn field_write(inst: &Instance, name: &str, value: &Value) -> Option<Result<(), String>> {
    let node = as_xml(inst)?;
    Some(super::node::write_property(
        &node,
        &inst.class.name,
        name,
        value,
    ))
}

/// `print(node)`, `"#{node}"`, `inspect`, JSON: the compact `to_xml`.
pub fn display(inst: &Instance) -> Option<String> {
    let node = as_xml(inst)?;
    Some(
        node.compact_xml()
            .unwrap_or_else(|e| format!("<{} error: {e}>", inst.class.name)),
    )
}

/// `==`: identity (same tree, same node), never structure.
pub fn eq(a: &Instance, b: &Instance) -> Option<bool> {
    let left = as_xml(a)?;
    Some(as_xml(b).is_some_and(|right| left.same_node(&right)))
}

/// What the response cache hashes for a handle: its compact `to_xml`.
pub fn fingerprint(inst: &Instance) -> Option<String> {
    display(inst)
}
