//! Domain modelling support: value objects, invariants and domain errors.
//!
//! Shared by both engines, so the tree-walker and the VM build, compare,
//! freeze and serialize value objects the same way.
//!
//! A value object is an instance of a class descending from the built-in
//! `ValueObject`:
//!
//! ```soli
//! class Amount < ValueObject
//!   cents: Int
//!   currency: String
//!   invariant("cents must not be negative") { @cents >= 0 }
//! end
//! ```
//!
//! - **Construction** runs the class's constructor, or — without one — binds
//!   the arguments to the declared fields (positionally, then by name). Every
//!   declared field ends up set (`nil` when not given), then the instance is
//!   frozen and its invariants run: a falsy one throws `InvariantViolation`.
//! - **Frozen**: a field write after construction raises. The instance is
//!   writable only while it is on the construction stack ([`Constructing`]),
//!   so no per-instance flag is needed.
//! - **Equality** is by value: same class, same fields ([`value_objects_equal`]).
//!
//! Every check is behind [`value_objects_declared`], a process-wide flag set
//! when the first value-object class is defined: a program without one pays
//! one relaxed atomic load on the paths involved.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::value::{Class, HashKey, HashPairs, Instance, Value};

thread_local! {
    /// The built-in `DomainError` and `InvariantViolation` classes, so Rust
    /// code can raise them. Set when the builtins are registered.
    static ERROR_CLASSES: RefCell<Option<(Rc<Class>, Rc<Class>)>> = const { RefCell::new(None) };

    /// Value-object classes by name, for `value_field` reads.
    static CLASSES: RefCell<HashMap<String, Rc<Class>>> = RefCell::new(HashMap::new());

    /// `private class` declarations by qualified name (`Billing::Ledger`), so
    /// a relation can be refused without resolving the module it sits in.
    static PRIVATE_CLASSES: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
}

crate::request_local! {

    /// Value objects being constructed on this thread — writable until their
    /// constructor returns. Nested constructions (a value object built inside
    /// another's constructor) stack.
    static CONSTRUCTING: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// `name`, nested in `parent`, was declared `private class` (both engines).
pub fn note_private_class(parent: &Class, name: &str) {
    PRIVATE_CLASSES.with(|p| p.borrow_mut().insert(format!("{}::{}", parent.name, name)));
}

/// A relation of `owner` (`Shipping::Parcel`) toward `target`
/// (`Billing::Ledger`): refused when the target is a private class of a
/// context the owner is not in — the rule `Billing::Ledger` read by name
/// follows, which a `class_name:` would otherwise walk around.
pub fn check_relation_target(owner: &str, target: &str) -> Result<(), String> {
    if !PRIVATE_CLASSES.with(|p| p.borrow().contains(target)) {
        return Ok(());
    }
    let Some((context, _)) = target.rsplit_once("::") else {
        return Ok(());
    };
    let inside = owner
        .rsplit_once("::")
        .is_some_and(|(ns, _)| ns == context || ns.starts_with(&format!("{context}::")));
    if inside {
        return Ok(());
    }
    Err(format!(
        "{owner} cannot reach {target} through a relation: {target} is private to {context} \
         — go through {context}'s public classes, or declare it without `private`"
    ))
}

/// Remember a defined class if it is a value object, so a `value_field` read
/// can find it by name.
pub fn register_class(class: &Rc<Class>) {
    if class.superclass.is_some() && class.is_value_object() {
        CLASSES.with(|c| c.borrow_mut().insert(class.name.clone(), class.clone()));
    }
}

/// A stored hash as the value object `class_name` names: its fields set, then
/// frozen — no constructor, no invariants, as a record read from the database
/// runs no callbacks (a row written before an invariant existed must still
/// load). `None` when no such value object is defined.
/// How a `value_field` spec reads: `Amount`, or `[]Line` for a list of them
/// (`value_field :lines, [Line]`). Returns (is a list, class name).
pub fn value_field_spec(spec: &str) -> (bool, &str) {
    match spec.strip_prefix("[]") {
        Some(class) => (true, class),
        None => (false, spec),
    }
}

/// A stored `value_field` value back as its value object — or, for a list
/// field, its array of value objects. `None` when it is not in stored form.
pub fn hydrate_field(spec: &str, stored: &Value) -> Option<Value> {
    match value_field_spec(spec) {
        (false, class) => hydrate(class, stored),
        (true, class) => {
            let Value::Array(items) = stored else {
                return None;
            };
            let values = items
                .borrow()
                .iter()
                .map(|item| match item {
                    Value::Hash(_) => hydrate(class, item).unwrap_or_else(|| item.clone()),
                    other => other.clone(),
                })
                .collect();
            Some(Value::Array(Rc::new(RefCell::new(values))))
        }
    }
}

/// A value-object class this thread has seen declared, by qualified name.
pub fn registered_class(class_name: &str) -> Option<Rc<Class>> {
    CLASSES.with(|c| c.borrow().get(class_name).cloned())
}

pub fn hydrate(class_name: &str, stored: &Value) -> Option<Value> {
    let class = CLASSES.with(|c| c.borrow().get(class_name).cloned())?;
    let Value::Hash(hash) = stored else {
        return None;
    };
    let instance = Rc::new(RefCell::new(Instance::new(class)));
    {
        let mut inst = instance.borrow_mut();
        for (key, value) in hash.borrow().iter() {
            if let HashKey::String(name) = key {
                if !name.starts_with('_') {
                    inst.set(name.clone(), value.clone());
                }
            }
        }
    }
    fill_declared_fields(&instance);
    Some(Value::Instance(instance))
}

/// Set once any value-object class exists in the process.
static VALUE_OBJECTS_DECLARED: AtomicBool = AtomicBool::new(false);

/// Whether any value-object class has been defined. Every value-object check
/// starts here, so ordinary programs skip them for one atomic load.
#[inline]
pub fn value_objects_declared() -> bool {
    VALUE_OBJECTS_DECLARED.load(Ordering::Relaxed)
}

/// Note a newly defined class: a value object turns the checks on.
pub fn note_class(class: &Class) {
    if class.superclass.is_some() && class.is_value_object() {
        VALUE_OBJECTS_DECLARED.store(true, Ordering::Relaxed);
    }
}

/// Remember the built-in domain error classes (see [`ERROR_CLASSES`]).
pub fn remember_error_classes(domain_error: Rc<Class>, invariant_violation: Rc<Class>) {
    ERROR_CLASSES.with(|c| *c.borrow_mut() = Some((domain_error, invariant_violation)));
}

/// A new `InvariantViolation` carrying `message` and `details`, ready to throw.
/// Falls back to the message alone (a string) before the builtins exist.
pub fn invariant_violation(message: &str, details: Value) -> Value {
    ERROR_CLASSES.with(|c| match c.borrow().as_ref() {
        Some((_, class)) => {
            let mut instance = Instance::new(class.clone());
            instance.set("message", Value::String(message.to_string().into()));
            instance.set("details", details);
            Value::Instance(Rc::new(RefCell::new(instance)))
        }
        None => Value::String(message.to_string().into()),
    })
}

// ---------------------------------------------------------------------------
// Construction and freezing
// ---------------------------------------------------------------------------

fn key(instance: &Rc<RefCell<Instance>>) -> usize {
    Rc::as_ptr(instance) as usize
}

/// A value object under construction: writable until this is dropped (also
/// when the constructor throws).
pub struct Constructing(usize);

impl Drop for Constructing {
    fn drop(&mut self) {
        CONSTRUCTING.with(|c| {
            let mut stack = c.borrow_mut();
            if let Some(i) = stack.iter().rposition(|k| *k == self.0) {
                stack.remove(i);
            }
        });
    }
}

/// Mark `instance` as under construction.
pub fn begin_construction(instance: &Rc<RefCell<Instance>>) -> Constructing {
    let k = key(instance);
    CONSTRUCTING.with(|c| c.borrow_mut().push(k));
    Constructing(k)
}

/// Whether `instance` (of `class`) is a constructed value object, whose
/// fields can no longer change. `class` is passed in so a caller already
/// holding a borrow of the instance can ask.
pub fn is_frozen(instance: &Rc<RefCell<Instance>>, class: &Class) -> bool {
    if !value_objects_declared() || !class.is_value_object() {
        return false;
    }
    let k = key(instance);
    !CONSTRUCTING.with(|c| c.borrow().contains(&k))
}

/// The error for a write to a frozen value object; it names the way out.
pub fn frozen_error(class: &Class, field: &str) -> String {
    format!(
        "cannot set {}.{}: value objects are immutable — build a changed copy with with({}: …)",
        class.name, field, field
    )
}

/// `Ok` unless `instance` is a constructed value object.
pub fn check_writable(
    instance: &Rc<RefCell<Instance>>,
    class: &Class,
    field: &str,
) -> Result<(), String> {
    if is_frozen(instance, class) {
        return Err(frozen_error(class, field));
    }
    Ok(())
}

/// Without a constructor: bind `positional` to the declared fields in order,
/// then `named` by name. Unknown names, repeats and surplus arguments are
/// errors, so a typo cannot pass silently.
pub fn bind_fields(
    instance: &Rc<RefCell<Instance>>,
    positional: Vec<Value>,
    named: &HashMap<String, Value>,
) -> Result<(), String> {
    let class = instance.borrow().class.clone();
    let fields = class.all_declared_fields();
    if positional.len() > fields.len() {
        return Err(format!(
            "{} takes {} field{} ({}), got {} arguments",
            class.name,
            fields.len(),
            if fields.len() == 1 { "" } else { "s" },
            fields.join(", "),
            positional.len()
        ));
    }
    let given = positional.len();
    let mut inst = instance.borrow_mut();
    for (name, value) in fields.iter().zip(positional) {
        inst.set(name.clone(), value);
    }
    for (name, value) in named {
        match fields.iter().position(|f| f == name) {
            None => {
                return Err(format!(
                    "{} has no field `{}` (fields: {})",
                    class.name,
                    name,
                    fields.join(", ")
                ))
            }
            Some(i) if i < given => {
                return Err(format!(
                    "{}: `{}` given both by position and by name",
                    class.name, name
                ))
            }
            Some(_) => inst.set(name.clone(), value.clone()),
        }
    }
    Ok(())
}

/// Set every declared field the constructor left unset to `nil`, so both
/// engines end with the same fields (the VM only sets fields that have a
/// default).
pub fn fill_declared_fields(instance: &Rc<RefCell<Instance>>) {
    let class = instance.borrow().class.clone();
    let mut inst = instance.borrow_mut();
    for name in class.all_declared_fields() {
        if !inst.fields.contains_key(name.as_str()) {
            inst.set(name, Value::Null);
        }
    }
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

/// The static field a class keeps its invariants' messages in: an array of
/// `[message, field_or_nil]`, the i-th one checked by the method
/// [`invariant_method_name`]`(class, i)`.
pub const INVARIANTS_FIELD: &str = "__invariants__";

/// The private method an invariant's block compiles to. Named after its class
/// so a subclass's invariants do not replace its parent's.
pub fn invariant_method_name(class_name: &str, index: usize) -> String {
    format!("__invariant_{}_{}", class_name, index)
}

/// One invariant to check: the method to call, the message, the field it is
/// about (for a model's `_errors`).
#[derive(Clone, Debug)]
pub struct Invariant {
    pub method: String,
    pub message: String,
    pub field: Option<String>,
}

/// The metadata value [`INVARIANTS_FIELD`] holds, from `(message, field)`s.
pub fn invariants_value(specs: &[(String, Option<String>)]) -> Value {
    let rows = specs
        .iter()
        .map(|(message, field)| {
            Value::Array(Rc::new(RefCell::new(vec![
                Value::String(message.clone().into()),
                field
                    .clone()
                    .map(|f| Value::String(f.into()))
                    .unwrap_or(Value::Null),
            ])))
        })
        .collect();
    Value::Array(Rc::new(RefCell::new(rows)))
}

/// Every invariant of `class`, its ancestors' first.
pub fn invariants_of(class: &Class) -> Vec<Invariant> {
    let mut out = match &class.superclass {
        Some(superclass) => invariants_of(superclass),
        None => Vec::new(),
    };
    if let Some(Value::Array(rows)) = class.static_fields.borrow().get(INVARIANTS_FIELD) {
        for (i, row) in rows.borrow().iter().enumerate() {
            let Value::Array(cells) = row else { continue };
            let cells = cells.borrow();
            let message = match cells.first() {
                Some(Value::String(s)) => s.to_string(),
                _ => continue,
            };
            let field = match cells.get(1) {
                Some(Value::String(s)) => Some(s.to_string()),
                _ => None,
            };
            out.push(Invariant {
                method: invariant_method_name(&class.name, i),
                message,
                field,
            });
        }
    }
    out
}

/// The `InvariantViolation` for a failed `invariant` of `class`.
pub fn violation_for(class: &Class, invariant: &Invariant) -> Value {
    let mut details = HashPairs::default();
    details.insert(
        HashKey::String("class".into()),
        Value::String(class.name.clone().into()),
    );
    details.insert(
        HashKey::String("invariant".into()),
        Value::String(invariant.message.clone().into()),
    );
    if let Some(field) = &invariant.field {
        details.insert(
            HashKey::String("field".into()),
            Value::String(field.clone().into()),
        );
    }
    invariant_violation(
        &format!("{}: {}", class.name, invariant.message),
        Value::Hash(Rc::new(RefCell::new(details))),
    )
}

// ---------------------------------------------------------------------------
// Equality, to_h, with
// ---------------------------------------------------------------------------

/// The fields that make a value object's value, in order: declared ones
/// first, then any other a constructor set, by name. `_`-prefixed fields are
/// bookkeeping and stay out.
pub fn value_fields(instance: &Instance) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    for name in instance.class.all_declared_fields() {
        if let Some(value) = instance.fields.get(name.as_str()) {
            out.push((name, value.clone()));
        }
    }
    let mut rest: Vec<(String, Value)> = instance
        .fields
        .iter()
        .filter(|(name, _)| !name.starts_with('_') && !out.iter().any(|(n, _)| n == name.as_ref()))
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect();
    rest.sort_by(|a, b| a.0.cmp(&b.0));
    out.extend(rest);
    out
}

/// Two value objects are equal when they are of the same class and their
/// fields are equal.
pub fn value_objects_equal(a: &Instance, b: &Instance) -> bool {
    if a.class.name != b.class.name {
        return false;
    }
    let fields = |i: &Instance| {
        i.fields
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .count()
    };
    if fields(a) != fields(b) {
        return false;
    }
    a.fields.iter().all(|(name, value)| {
        name.starts_with('_')
            || b.fields
                .get(name)
                .is_some_and(|other| super::value::enum_aware_equal(value, other))
    })
}

/// A value object as a Hash of its fields, in order.
pub fn to_hash(instance: &Instance) -> Value {
    let mut pairs = HashPairs::default();
    for (name, value) in value_fields(instance) {
        pairs.insert(HashKey::String(name.into()), value);
    }
    Value::Hash(Rc::new(RefCell::new(pairs)))
}

/// The named arguments that rebuild `instance` with `changes` applied: for a
/// constructor, one per parameter (a change, else the field of that name;
/// a parameter with neither is left to its default); without one, every
/// field. A change naming nothing the constructor or the fields know is an
/// error.
pub fn with_arguments(
    instance: &Instance,
    constructor_params: Option<&[String]>,
    changes: &HashMap<String, Value>,
) -> Result<HashMap<String, Value>, String> {
    let class = &instance.class;
    let current: HashMap<String, Value> = value_fields(instance).into_iter().collect();
    let names: Vec<String> = match constructor_params {
        Some(params) => params.to_vec(),
        None => current.keys().cloned().collect(),
    };
    for name in changes.keys() {
        if !names.contains(name) {
            return Err(format!(
                "{}.with: no field or constructor parameter `{}`",
                class.name, name
            ));
        }
    }
    let mut out = HashMap::new();
    for name in names {
        if let Some(value) = changes.get(&name) {
            out.insert(name, value.clone());
        } else if let Some(value) = current.get(&name) {
            out.insert(name, value.clone());
        }
    }
    Ok(out)
}

/// `changes` as a name → value map: a Hash of string keys, or named
/// arguments collapsed into one.
pub fn changes_from(value: Option<&Value>, what: &str) -> Result<HashMap<String, Value>, String> {
    match value {
        None => Ok(HashMap::new()),
        Some(Value::Hash(hash)) => {
            let mut out = HashMap::new();
            for (k, v) in hash.borrow().iter() {
                match k {
                    HashKey::String(s) | HashKey::Symbol(s) => {
                        out.insert(s.to_string(), v.clone());
                    }
                    other => return Err(format!("{what}: field names are strings, got {other}")),
                }
            }
            Ok(out)
        }
        Some(other) => Err(format!(
            "{what} expects a hash of field values, got {}",
            other.type_name()
        )),
    }
}

/// The binding a nested class's environment holds: the qualified name of the
/// class its methods are written in, for names read unqualified there.
pub const NAMESPACE_BINDING: &str = "__namespace__";

/// `name` read inside the namespace `namespace` (`Billing::PayInvoice`): the
/// nearest `Billing::PayInvoice::name`, then `Billing::name`, the way Ruby
/// resolves a constant lexically. `global` looks up a top-level name.
pub fn resolve_in_namespace(
    namespace: &str,
    name: &str,
    global: impl Fn(&str) -> Option<Value>,
) -> Option<Value> {
    if !name.starts_with(char::is_uppercase) {
        return None;
    }
    let segments: Vec<&str> = namespace.split("::").collect();
    for depth in (1..=segments.len()).rev() {
        let Some(Value::Class(mut class)) = global(segments[0]) else {
            continue;
        };
        let mut found = true;
        for segment in &segments[1..depth] {
            match nested_class(&class, segment) {
                Some(nested) => class = nested,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if !found {
            continue;
        }
        if let Some(target) = nested_class(&class, name) {
            return Some(Value::Class(target));
        }
    }
    None
}

/// The class `name` nested in `class`: the tree-walker keeps it in
/// `nested_classes`, the VM in a static field of that name.
fn nested_class(class: &Class, name: &str) -> Option<Rc<Class>> {
    if let Some(nested) = class.nested_classes.borrow().get(name) {
        return Some(nested.clone());
    }
    match class.static_fields.borrow().get(name) {
        Some(Value::Class(nested)) => Some(nested.clone()),
        _ => None,
    }
}

/// `name`, a class nested in `parent`, read from code in `namespace`: refused
/// when it is a `private class` of `parent` and the code is not inside
/// `parent` — a bounded context's private classes stay inside it.
pub fn check_nested_access(
    parent: &Class,
    name: &str,
    namespace: Option<&str>,
) -> Result<(), String> {
    if !parent.private_members.borrow().contains(name) {
        return Ok(());
    }
    let inside = namespace.is_some_and(|ns| {
        ns == parent.name
            || ns
                .strip_prefix(parent.name.as_str())
                .is_some_and(|rest| rest.starts_with("::"))
    });
    if inside {
        return Ok(());
    }
    Err(format!(
        "{}::{name} is private to {} — use it from code in {}, or declare it without `private`",
        parent.name, parent.name, parent.name
    ))
}
