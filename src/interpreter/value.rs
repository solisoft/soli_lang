//! Runtime values for the Solilang interpreter.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// Set the first time this process creates a `Value::Future`, and never
/// cleared. Until then no value can hold one, so the walks that look for
/// futures to resolve before rendering (`render_json`, `render`) are skipped —
/// 50 hashes walked per JSON response for nothing in an app that never calls
/// anything asynchronous. Every `Value::Future(Arc::new(..))` site calls
/// [`note_future_created`].
pub static FUTURES_CREATED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[inline]
pub fn note_future_created() {
    FUTURES_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
}

use ahash::RandomState as AHasher;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::ser::{SerializeMap, SerializeSeq};

use crate::ast::{Body, Expr, FunctionDecl, MethodDecl, Parameter, TypeAnnotation};
use crate::interpreter::builtins::model::QueryBuilder;
use crate::interpreter::environment::Environment;
use crate::span::Span;
use crate::vm::upvalue::VmClosure;

/// A Decimal value wrapper for financial calculations.
/// Uses rust_decimal for exact decimal arithmetic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalValue(pub Decimal, pub u32); // (value, precision)

impl DecimalValue {
    /// Create a new DecimalValue from a string representation
    pub fn from_str(s: &str, precision: u32) -> Result<Self, String> {
        let decimal: Decimal = s.parse().map_err(|_| format!("Invalid decimal: {}", s))?;
        Ok(Self(decimal, precision))
    }

    /// The value of a Decimal literal from its text, `"19.99"` for `19.99D`.
    /// The precision is the number of digits written after the point, also
    /// past the 28 places the value keeps. `None` when the text does not
    /// parse: an integer part beyond Decimal's range, or a `%n[...]` word
    /// that is not a number. Both engines read literals through this; the
    /// VM does it once, when it compiles one.
    pub fn from_literal(text: &str) -> Option<Self> {
        let decimal: Decimal = text.parse().ok()?;
        let precision = text
            .split('.')
            .nth(1)
            .map_or(0, |digits| digits.len() as u32);
        Some(Self(decimal, precision))
    }

    /// Get the precision (number of decimal places)
    pub fn precision(&self) -> u32 {
        self.1
    }

    /// Get the underlying decimal value
    pub fn value(&self) -> &Decimal {
        &self.0
    }

    /// Convert to f64 (loss of precision)
    pub fn to_f64(&self) -> f64 {
        self.0.to_f64().unwrap_or(0.0)
    }
}

impl std::fmt::Display for DecimalValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Hash for DecimalValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// Soli's immutable string payload. Strings are immutable in the language
/// (every "mutating" method returns a new string), so the buffer can be
/// shared: `EcoString` stores up to 15 bytes inline (constructing a short
/// string never touches the heap — cheaper than `String`) and switches to an
/// atomically-refcounted heap buffer beyond that, making `clone()` O(1) for
/// long strings instead of a byte copy. Thread-safe, so the same payload can
/// flow into `HashKey` and the VM constant pool, which cross threads via the
/// compiled-module cache. (A plain `Arc<str>` was benchmarked first and lost:
/// `Arc<str>::from(String)` re-copies the bytes on every construction, which
/// dominated the clone savings on construction-heavy workloads.)
pub type SoliStr = ecow::EcoString;

/// Whether `current` is still the string `before` was read as, in O(1) for a
/// long one. With `before` still held, a heap buffer at the same address and
/// length cannot have changed (copy-on-write detaches on any edit). A short
/// string may be stored inline, so it has no shared address; compare its
/// bytes. `false` is always safe — the caller then does an ordinary add.
pub fn same_string(current: &str, before: &str) -> bool {
    current.len() == before.len()
        && (std::ptr::eq(current.as_ptr(), before.as_ptr())
            || (current.len() <= 32 && current == before))
}

/// Whether `inst` is a value object — one relaxed load while none is
/// declared.
fn is_value_object_instance(inst: &Rc<RefCell<Instance>>) -> bool {
    crate::interpreter::domain::value_objects_declared() && inst.borrow().class.is_value_object()
}

/// A hashable key type for use in IndexMap.
/// This wraps primitive Value types that can be used as hash keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashKey {
    Int(i64),
    Decimal(DecimalValue), // Hashable Decimal
    String(SoliStr),
    Bool(bool),
    Null,
    Symbol(SoliStr),
    /// A value object (`Amount.new(500)`): equal by value, frozen, so it can
    /// key a hash.
    ValueObject(ValueObjectKey),
}

/// A value object used as a hash key: its class and its fields as
/// canonical JSON — equal by value, and `Send` like every other key (hashes
/// are shared through process-wide caches). Read back, it is rebuilt as the
/// value object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ValueObjectKey {
    pub class: String,
    pub fields: String,
}

impl ValueObjectKey {
    fn of(inst: &Rc<RefCell<Instance>>) -> Option<Self> {
        let value = Value::Instance(inst.clone());
        let fields = serde_json::to_string(&value_to_json(&value).ok()?).ok()?;
        Some(Self {
            class: inst.borrow().class.name.clone(),
            fields,
        })
    }

    /// The value object again — or its fields as a hash where the class is
    /// not loaded on this thread.
    pub fn to_value(&self) -> Value {
        let fields = serde_json::from_str::<serde_json::Value>(&self.fields)
            .map(crate::interpreter::value_json::json_to_value_or_null)
            .unwrap_or(Value::Null);
        crate::interpreter::domain::hydrate(&self.class, &fields).unwrap_or(fields)
    }
}

impl Hash for HashKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            HashKey::Int(n) => {
                0u8.hash(state);
                n.hash(state);
            }
            HashKey::Decimal(d) => {
                1u8.hash(state);
                d.hash(state);
            }
            HashKey::String(s) => {
                2u8.hash(state);
                s.hash(state);
            }
            HashKey::Bool(b) => {
                3u8.hash(state);
                b.hash(state);
            }
            HashKey::Null => {
                4u8.hash(state);
            }
            HashKey::Symbol(s) => {
                5u8.hash(state);
                s.hash(state);
            }
            HashKey::ValueObject(key) => {
                6u8.hash(state);
                key.hash(state);
            }
        }
    }
}

/// Zero-allocation key for looking up string keys in IndexMap<HashKey, Value>.
/// Hashes identically to HashKey::String, avoiding String clone for lookups.
#[repr(transparent)]
pub struct StrKey<'a>(pub &'a str);

impl Hash for StrKey<'_> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        2u8.hash(state); // Must match HashKey::String tag
        self.0.hash(state);
    }
}

impl indexmap::Equivalent<HashKey> for StrKey<'_> {
    #[inline]
    fn equivalent(&self, key: &HashKey) -> bool {
        matches!(key, HashKey::String(s) if &**s == self.0)
    }
}

/// Zero-allocation key for looking up symbol keys in IndexMap<HashKey, Value>.
/// Hashes identically to HashKey::Symbol, avoiding String clone for lookups.
#[repr(transparent)]
pub struct SymKey<'a>(pub &'a str);

impl Hash for SymKey<'_> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        5u8.hash(state); // Must match HashKey::Symbol tag
        self.0.hash(state);
    }
}

impl indexmap::Equivalent<HashKey> for SymKey<'_> {
    #[inline]
    fn equivalent(&self, key: &HashKey) -> bool {
        matches!(key, HashKey::Symbol(s) if &**s == self.0)
    }
}

impl HashKey {
    /// Convert a Value to a HashKey if possible.
    pub fn from_value(value: &Value) -> Option<HashKey> {
        match value {
            Value::Int(n) => Some(HashKey::Int(*n)),
            Value::Decimal(d) => Some(HashKey::Decimal(d.clone())),
            Value::String(s) => Some(HashKey::String(s.clone())),
            Value::Bool(b) => Some(HashKey::Bool(*b)),
            Value::Null => Some(HashKey::Null),
            Value::Symbol(s) => Some(HashKey::Symbol(s.clone())),
            Value::Instance(inst) if is_value_object_instance(inst) => {
                ValueObjectKey::of(inst).map(HashKey::ValueObject)
            }
            // Floats are not hashable due to NaN != NaN issues
            _ => None,
        }
    }

    /// Like `from_value`, but consumes the Value to avoid cloning the String/Decimal.
    /// Used when the source value is going to be discarded immediately.
    pub fn from_value_owned(value: Value) -> Option<HashKey> {
        match value {
            Value::Int(n) => Some(HashKey::Int(n)),
            Value::Decimal(d) => Some(HashKey::Decimal(d)),
            Value::String(s) => Some(HashKey::String(s)),
            Value::Bool(b) => Some(HashKey::Bool(b)),
            Value::Null => Some(HashKey::Null),
            Value::Symbol(s) => Some(HashKey::Symbol(s)),
            Value::Instance(inst) if is_value_object_instance(&inst) => {
                ValueObjectKey::of(&inst).map(HashKey::ValueObject)
            }
            _ => None,
        }
    }

    /// Convert back to a Value.
    pub fn to_value(&self) -> Value {
        match self {
            HashKey::Int(n) => Value::Int(*n),
            HashKey::Decimal(d) => Value::Decimal(d.clone()),
            HashKey::String(s) => Value::String(s.clone()),
            HashKey::Bool(b) => Value::Bool(*b),
            HashKey::Null => Value::Null,
            HashKey::Symbol(s) => Value::Symbol(s.clone()),
            HashKey::ValueObject(key) => key.to_value(),
        }
    }

    #[inline]
    pub fn display_len(&self) -> usize {
        match self {
            HashKey::Int(n) => itoa::Buffer::new().format(*n).len(),
            HashKey::Decimal(d) => d.to_string().len(),
            // Matches `write_key_to_string` (no surrounding quotes on string keys).
            HashKey::String(s) => s.len(),
            HashKey::Bool(b) => {
                if *b {
                    4
                } else {
                    5
                }
            }
            HashKey::Null => 4,
            HashKey::Symbol(s) => s.len() + 1,
            HashKey::ValueObject(key) => key.to_value().to_string().len(),
        }
    }

    #[inline]
    pub fn write_key_to_string(&self, s: &mut String) {
        match self {
            HashKey::Int(n) => s.push_str(itoa::Buffer::new().format(*n)),
            HashKey::Decimal(d) => s.push_str(&d.to_string()),
            HashKey::String(st) => {
                s.push_str(st);
            }
            HashKey::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
            HashKey::Null => s.push_str("null"),
            HashKey::Symbol(sym) => {
                s.push(':');
                s.push_str(sym);
            }
            HashKey::ValueObject(key) => s.push_str(&key.to_value().to_string()),
        }
    }
}

impl std::fmt::Display for HashKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashKey::Int(n) => write!(f, "{}", n),
            HashKey::Decimal(d) => write!(f, "{}", d),
            HashKey::String(s) => write!(f, "{}", s),
            HashKey::Bool(b) => write!(f, "{}", b),
            HashKey::Null => write!(f, "null"),
            HashKey::Symbol(s) => write!(f, ":{}", s),
            HashKey::ValueObject(key) => write!(f, "{}", key.to_value()),
        }
    }
}

/// Helper function to create a hash Value from string key-value pairs.
/// This is a convenience function for creating hashes in builtin functions.
pub fn hash_from_pairs<I, K>(pairs: I) -> Value
where
    I: IntoIterator<Item = (K, Value)>,
    K: Into<SoliStr>,
{
    let map: HashPairs = pairs
        .into_iter()
        .map(|(k, v)| (HashKey::String(k.into()), v))
        .collect();
    Value::Hash(Rc::new(RefCell::new(map)))
}

/// Helper function to create an empty hash Value.
pub fn empty_hash() -> Value {
    Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
}

/// Storage for a Soli hash: insertion-ordered, linear up to eight entries and
/// an `IndexMap` (ahash) beyond — see [`super::small_map`].
pub type HashPairs = super::small_map::SmallMap;
pub use super::small_map::map_hasher;

#[inline]
pub fn hash_get_value<'a>(hash: &'a HashPairs, key: &Value) -> Option<&'a Value> {
    match key {
        Value::String(s) => hash.get(&StrKey(s)),
        Value::Int(n) => hash.get(&HashKey::Int(*n)),
        Value::Decimal(d) => hash.get(&HashKey::Decimal(d.clone())),
        Value::Bool(b) => hash.get(&HashKey::Bool(*b)),
        Value::Null => hash.get(&HashKey::Null),
        Value::Symbol(s) => hash.get(&SymKey(s)),
        Value::Instance(_) => HashKey::from_value(key).and_then(|k| hash.get(&k)),
        _ => None,
    }
}

#[inline]
pub fn hash_contains_value(hash: &HashPairs, key: &Value) -> bool {
    hash_get_value(hash, key).is_some()
}

/// Insert or overwrite `key`. Returns `false` if `key` is not hashable.
///
/// On a hit, the existing slot is overwritten without allocating a new
/// `HashKey` (string/symbol keys used to clone on every `h[k] = v`).
#[inline]
pub fn hash_set_value(hash: &mut HashPairs, key: &Value, value: Value) -> bool {
    match key {
        Value::String(s) => {
            hash.set_str(s, value);
            true
        }
        Value::Symbol(s) => {
            if let Some((_, _, existing)) = hash.get_full_mut(&SymKey(s)) {
                *existing = value;
            } else {
                hash.insert_absent(HashKey::Symbol(s.clone()), value);
            }
            true
        }
        Value::Int(n) => {
            if let Some((_, _, existing)) = hash.get_full_mut(&HashKey::Int(*n)) {
                *existing = value;
            } else {
                hash.insert_absent(HashKey::Int(*n), value);
            }
            true
        }
        Value::Bool(b) => {
            if let Some((_, _, existing)) = hash.get_full_mut(&HashKey::Bool(*b)) {
                *existing = value;
            } else {
                hash.insert_absent(HashKey::Bool(*b), value);
            }
            true
        }
        Value::Null => {
            if let Some((_, _, existing)) = hash.get_full_mut(&HashKey::Null) {
                *existing = value;
            } else {
                hash.insert_absent(HashKey::Null, value);
            }
            true
        }
        Value::Decimal(d) => {
            if let Some((_, _, existing)) = hash.get_full_mut(&HashKey::Decimal(d.clone())) {
                *existing = value;
            } else {
                hash.insert_absent(HashKey::Decimal(d.clone()), value);
            }
            true
        }
        Value::Instance(_) => match HashKey::from_value(key) {
            Some(hash_key) => {
                hash.insert(hash_key, value);
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// A runtime value in Solilang.
#[derive(Debug, Clone)]
pub enum Value {
    /// Integer value
    Int(i64),
    /// Floating point value
    Float(f64),
    /// Decimal value (exact arithmetic for financial calculations)
    Decimal(DecimalValue),
    /// String value (refcounted: clone = Rc bump, not a byte copy)
    String(SoliStr),
    /// Symbol value (:name)
    Symbol(SoliStr),
    /// Boolean value
    Bool(bool),
    /// Null value
    Null,
    /// Array value
    Array(Rc<RefCell<Vec<Value>>>),
    /// Hash/Map value (ordered, O(1) lookup using IndexMap with ahash)
    Hash(Rc<RefCell<HashPairs>>),
    /// Function value (closure)
    Function(Rc<Function>),
    /// Native/builtin function
    NativeFunction(NativeFunction),
    /// Class definition
    Class(Rc<Class>),
    /// Class instance
    Instance(Rc<RefCell<Instance>>),
    /// An instant, as nanoseconds since the Unix epoch. Native rather than
    /// an `Instance` so a DateTime costs no allocation and no hash lookup.
    ///
    /// The `bool` is a **display/component view**: `false` = process-local
    /// zone (default), `true` = UTC. Component accessors (`year`, `hour`,
    /// `format`, …) and string rendering honour it; equality, ordering,
    /// `to_unix`, and `to_iso` always use the instant. Toggle with
    /// `.utc()` / `.local()`.
    DateTime(i64, bool),
    /// Future value (async result that auto-resolves when used)
    Future(Arc<Mutex<FutureState>>),
    /// Method on a value (array/hash) - captures receiver and method name.
    /// Behind an `Rc` so the variant is pointer-sized: `Method` is the largest
    /// inline payload otherwise, and it's a transient block receiver (the hot
    /// `arr.map(fn)` path bypasses it), so the extra refcount is cold.
    Method(Rc<ValueMethod>),
    /// Breakpoint marker - triggers debug mode when encountered
    Breakpoint,
    /// Continue marker - used by next() alias for continue in loops
    Continue,
    /// Query builder for chainable database queries
    QueryBuilder(Rc<RefCell<QueryBuilder>>),
    /// Super reference - used for super.method() calls, carries the superclass
    Super(Rc<Class>),
    /// VM bytecode closure (used by the bytecode VM)
    VmClosure(Rc<VmClosure>),
    /// Image value (holds DynamicImage and metadata)
    Image(Rc<RefCell<crate::interpreter::builtins::image::ImageData>>),
    /// Image plan value (lazy, recorded ops to be executed in parallel)
    ImagePlan(Rc<RefCell<crate::interpreter::builtins::image::ImagePlan>>),
    /// Deferred query result inside a `grouped(fn() { ... })` batch. The query
    /// has been registered but not yet fetched; the cell is filled when the
    /// batch flushes (block end, or the first time the result is read — see
    /// `builtins::model::batch`). Resolved transparently at read points
    /// (`evaluate_variable`, member access, `value_to_json`, `Display`).
    Deferred(Rc<RefCell<DeferredCell>>),
}

impl Value {
    /// Construct a `Value::Method`, boxing the receiver+name into the `Rc`
    /// the variant now holds so the variant stays pointer-sized.
    #[inline]
    pub fn method(vm: ValueMethod) -> Value {
        Value::Method(Rc::new(vm))
    }
}

/// Backing cell for a [`Value::Deferred`]. `resolved` is `None` until the
/// owning batch flushes, then holds the materialised query result.
#[derive(Debug, Default)]
pub struct DeferredCell {
    pub resolved: Option<Value>,
}

/// Equality used by the `==` / `!=` operators in both engines. Enum values are
/// distinct `Instance`s, so payload variants would never compare equal under
/// the default (identity) `Instance` equality. This compares **enum** instances
/// structurally — class name + `__variant` tag + payload fields (recursively) —
/// and defers to the default `PartialEq` for everything else, preserving
/// existing behaviour for ordinary objects.
///
/// A unit variant is a shared singleton, so `status == Status.Active` is
/// usually one instance compared with itself, and identity answers it before
/// any field is looked at. That holds for any instance that is no payload
/// variant: an ordinary object's `PartialEq` is identity first, and a unit
/// variant's only field is its tag. A payload variant compared with itself
/// still goes field by field, so `v == v` stays false when its payload holds
/// a NaN, exactly as for two equal copies.
pub fn enum_aware_equal(a: &Value, b: &Value) -> bool {
    if let (Value::Instance(ia), Value::Instance(ib)) = (a, b) {
        // The left operand decides: an ordinary object never compares
        // structurally, so the right one is probed only behind an enum.
        let ra = ia.borrow();
        if !ra.fields.contains_key("__variant") {
            // `PartialEq`'s own first answer for one object, without its
            // depth guard.
            if Rc::ptr_eq(ia, ib) {
                return true;
            }
        } else {
            // A unit variant (its tag is its only field) compared with
            // itself: identity answers. A payload variant compares field by
            // field even against itself (a NaN payload is unequal to itself
            // on both engines).
            if ra.fields.len() <= 1 && Rc::ptr_eq(ia, ib) {
                return true;
            }
            let rb = ib.borrow();
            if rb.fields.contains_key("__variant") {
                if ra.class.name != rb.class.name || ra.fields.len() != rb.fields.len() {
                    return false;
                }
                return ra.fields.iter().all(|(key, va)| match rb.fields.get(key) {
                    Some(vb) => enum_aware_equal(va, vb),
                    None => false,
                });
            }
        }
    }
    a == b
}

/// If `inst` is an enum value, return its variant tag (the `__variant` field).
/// Drives the DB/JSON serialization shape and `enum_field` reconstruction.
pub(crate) fn enum_variant_tag(inst: &Instance) -> Option<&str> {
    match inst.fields.get("__variant") {
        Some(Value::String(v)) => Some(v.as_str()),
        _ => None,
    }
}

/// Rebuild an enum value from its stored representation: a bare tag string
/// (`"Active"` → a unit variant) or a tagged object (`{"variant": "Pending",
/// "reason": "x"}` → a payload variant). Used by `Enum.from(value)` and by the
/// model `enum_field` read-path reconstruction. A value that's neither a string
/// nor a hash is returned unchanged (defensive).
pub fn build_enum_value(class: &Rc<Class>, stored: &Value) -> Value {
    let mut inst = Instance::new(class.clone());
    match stored {
        Value::String(tag) => {
            inst.set("__variant", Value::String(tag.clone()));
        }
        Value::Hash(hash) => {
            for (key, value) in hash.borrow().iter() {
                if let HashKey::String(name) = key {
                    if name.as_str() == "variant" {
                        inst.set("__variant", value.clone());
                    } else {
                        inst.set(name.to_string(), value.clone());
                    }
                }
            }
        }
        other => return other.clone(),
    }
    Value::Instance(Rc::new(RefCell::new(inst)))
}

/// The type of HTTP future result
#[derive(Clone)]
pub enum HttpFutureKind {
    /// Returns body as string
    String,
    /// Returns parsed JSON
    Json,
    /// Returns parsed JSON after stripping JSONP callback padding
    Jsonp,
    /// Returns full response hash (status, headers, body)
    FullResponse,
    /// Returns SystemResult (for System.run())
    SystemResult,
}

/// State of a Future value
pub enum FutureState {
    /// Waiting for result - holds receiver for raw String data and the kind
    Pending {
        receiver: Receiver<Result<String, String>>,
        kind: HttpFutureKind,
    },
    /// Result is ready
    Resolved(Value),
    /// Error occurred
    Error(String),
}

impl std::fmt::Debug for FutureState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FutureState::Pending { .. } => write!(f, "FutureState::Pending"),
            FutureState::Resolved(v) => write!(f, "FutureState::Resolved({:?})", v),
            FutureState::Error(e) => write!(f, "FutureState::Error({:?})", e),
        }
    }
}

impl std::fmt::Debug for HttpFutureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpFutureKind::String => write!(f, "String"),
            HttpFutureKind::Json => write!(f, "Json"),
            HttpFutureKind::Jsonp => write!(f, "Jsonp"),
            HttpFutureKind::FullResponse => write!(f, "FullResponse"),
            HttpFutureKind::SystemResult => write!(f, "SystemResult"),
        }
    }
}

impl Value {
    /// Build a string Value from anything that converts into the shared
    /// `SoliStr` payload (`&str`, `String`, `Box<str>`, ...). Preferred over
    /// spelling out `Value::String(x.into())` at construction sites.
    #[inline]
    pub fn str(s: impl Into<SoliStr>) -> Value {
        Value::String(s.into())
    }

    /// If this value is a `DateTime`, return the nanosecond timestamp.
    /// Used by equality and ordering so two DateTimes pointing at the same
    /// moment compare equal and order correctly (view flag ignored).
    pub fn datetime_ts(&self) -> Option<i64> {
        if let Value::DateTime(ts, _) = self {
            return Some(*ts);
        }
        if let Value::Instance(inst) = self {
            let inst = inst.borrow();
            if inst.class.name == "DateTime" {
                if let Some(Value::Int(ts)) = inst.fields.get("_ts") {
                    return Some(*ts);
                }
            }
        }
        None
    }

    /// Construct a DateTime in the default (local-component) view.
    #[inline]
    pub fn datetime(nanos: i64) -> Value {
        Value::DateTime(nanos, false)
    }

    /// Whether calling this value with `()` can dispatch somewhere — i.e.
    /// it is a function-like value rather than plain data. Used by both
    /// engines to treat `obj.m()` like `obj.m` when the member access
    /// already evaluated to a plain value (zero-arg builtins on primitives).
    pub fn is_callable(&self) -> bool {
        matches!(
            self,
            Value::Function(_)
                | Value::NativeFunction(_)
                | Value::Class(_)
                | Value::Method(_)
                | Value::VmClosure(_)
                | Value::Super(_)
        )
    }

    /// How a `DateTime` renders in string position — wall clock in the
    /// selected view (local by default, UTC after `.utc()`), the same format
    /// `to_string()` produces, so `str(d)` and `d.to_string()` agree.
    /// Previously a DateTime was an `Instance` and rendered with the
    /// generic object form, leaking the internal field as
    /// `<DateTime _ts: 1794744000000000000>`.
    pub fn render_datetime(ts: i64, use_utc: bool) -> String {
        Self::datetime_text(ts, use_utc).as_str().to_string()
    }

    /// Append a `DateTime`'s string form to `out` without an intermediate
    /// String. Byte-identical to `format("%Y-%m-%d %H:%M:%S")`.
    #[inline]
    pub fn write_datetime(ts: i64, use_utc: bool, out: &mut String) {
        out.push_str(Self::datetime_text(ts, use_utc).as_str());
    }

    /// `YYYY-MM-DD HH:MM:SS` on the stack. This used to go through chrono's
    /// `format(...)`, which parses the strftime pattern on every call, formats
    /// through `core::fmt` and allocates a String the caller then copied —
    /// about 190-235 ns more per date written into a view than a string of
    /// the same length. Here: the view's UTC offset (0, or the cached zone's
    /// offset at this instant — the same rule `with_timezone` applies), then
    /// days-from-epoch to a civil date and two-digit writes.
    ///
    /// Always 19 bytes: an `i64` of nanoseconds spans 1677-09-21 to
    /// 2262-04-11, and no zone moves that by more than a day, so `%Y` is
    /// always the four-digit, unsigned form chrono prints for 0..=9999.
    pub fn datetime_text(ts: i64, use_utc: bool) -> DateTimeText {
        let offset = if use_utc {
            0
        } else {
            crate::interpreter::builtins::datetime::local_zone::local_offset_secs(ts)
        };
        DateTimeText::new(ts, offset)
    }

    /// How a `DateTime` serialises to JSON: RFC 3339, matching `to_iso()`.
    /// Previously it serialised as `{}` — an `Instance` with only a private
    /// field, so every DateTime in an API response came out an empty object.
    pub fn datetime_to_rfc3339(ts: i64) -> String {
        chrono::DateTime::from_timestamp_nanos(ts).to_rfc3339()
    }

    pub fn type_name(&self) -> String {
        match self {
            Value::DateTime(_, _) => "DateTime".to_string(),
            Value::Int(_) => "int".to_string(),
            Value::Float(_) => "float".to_string(),
            Value::Decimal(_) => "decimal".to_string(),
            Value::String(_) => "string".to_string(),
            Value::Symbol(_) => "symbol".to_string(),
            Value::Bool(_) => "bool".to_string(),
            Value::Null => "null".to_string(),
            Value::Array(_) => "array".to_string(),
            Value::Hash(_) => "hash".to_string(),
            Value::Function(_) => "Function".to_string(),
            Value::NativeFunction(_) => "Function".to_string(),
            Value::Class(_) => "Class".to_string(),
            Value::Instance(i) => i.borrow().class.name.clone(),
            Value::Future(_) => "Future".to_string(),
            Value::Method(_) => "Method".to_string(),
            Value::Breakpoint => "Breakpoint".to_string(),
            Value::Continue => "Continue".to_string(),
            Value::QueryBuilder(_) => "QueryBuilder".to_string(),
            Value::Super(_) => "Super".to_string(),
            Value::VmClosure(_) => "Function".to_string(),
            Value::Image(_) => "Image".to_string(),
            Value::ImagePlan(_) => "ImagePlan".to_string(),
            Value::Deferred(_) => self.force_deferred().type_name(),
        }
    }

    /// Append this value's string representation directly into `out`.
    /// Avoids the intermediate `to_string()` allocation and the `fmt::Display`
    /// machinery for primitive types — used on the string-interpolation hot
    /// path (see `build_interpolated`).
    ///
    /// Only the cheap arms live here so the whole thing inlines into the
    /// build loop; a call per part measurably slowed multi-part strings.
    #[inline(always)]
    pub fn append_to<W: std::fmt::Write>(&self, out: &mut W) {
        // `write_str` on `String`/`EcoString` never fails.
        let _ = match self {
            Value::String(s) => out.write_str(s),
            Value::Int(n) => out.write_str(itoa::Buffer::new().format(*n)),
            Value::Bool(true) => out.write_str("true"),
            Value::Bool(false) => out.write_str("false"),
            Value::Null => out.write_str("null"),
            _ => self.append_uncommon_to(out),
        };
    }

    #[inline(never)]
    fn append_uncommon_to<W: std::fmt::Write>(&self, out: &mut W) -> fmt::Result {
        match self {
            Value::Float(n) => write_float(*n, out),
            Value::Symbol(s) => out.write_char(':').and_then(|()| out.write_str(s)),
            other => write!(out, "{}", other),
        }
    }

    /// Resolve a Future value, blocking until the result is ready.
    /// For non-Future values, returns the value unchanged.
    ///
    /// Every tree-walker index and binary operation calls this on its
    /// operands, and almost none is a Future: the check is inlined there and
    /// the blocking path stays out of line (it was 2% of a template render
    /// as a call, moving each operand in and out).
    #[inline]
    pub fn resolve(self) -> Result<Value, String> {
        if matches!(self, Value::Future(_)) {
            self.resolve_future()
        } else {
            Ok(self)
        }
    }

    #[cold]
    #[inline(never)]
    fn resolve_future(self) -> Result<Value, String> {
        match self {
            Value::Future(state) => {
                let mut guard = state.lock().map_err(|_| "Future lock poisoned")?;
                match std::mem::replace(
                    &mut *guard,
                    FutureState::Error("Future already consumed".into()),
                ) {
                    FutureState::Pending { receiver, kind } => {
                        match receiver.recv() {
                            Ok(Ok(raw_data)) => {
                                // Convert raw string data to Value based on kind
                                let value = convert_future_result(&raw_data, &kind)?;
                                *guard = FutureState::Resolved(value.clone());
                                Ok(value)
                            }
                            Ok(Err(e)) => {
                                *guard = FutureState::Error(e.clone());
                                Err(e)
                            }
                            Err(_) => Err("Future channel closed".into()),
                        }
                    }
                    FutureState::Resolved(value) => {
                        *guard = FutureState::Resolved(value.clone());
                        Ok(value)
                    }
                    FutureState::Error(e) => {
                        *guard = FutureState::Error(e.clone());
                        Err(e)
                    }
                }
            }
            other => Ok(other),
        }
    }

    /// Check if this value is a Future
    pub fn is_future(&self) -> bool {
        matches!(self, Value::Future(_))
    }

    /// Whether this is an unresolved/resolved `grouped {}` batch placeholder.
    #[inline]
    pub fn is_deferred(&self) -> bool {
        matches!(self, Value::Deferred(_))
    }

    /// If this is a `Deferred`, resolve it — flushing the active batch if the
    /// cell is still pending — and return the inner value. Non-deferred values
    /// are returned as a clone. This is the *best-effort* resolver used by the
    /// `Value` formatting/comparison methods (which have no error channel); the
    /// real read sites (`evaluate_variable`, member access, `value_to_json`)
    /// call `builtins::model::batch::force`, which propagates flush errors.
    pub fn force_deferred(&self) -> Value {
        match self {
            Value::Deferred(cell) => {
                if cell.borrow().resolved.is_none() {
                    crate::interpreter::builtins::model::batch::flush_current();
                }
                cell.borrow().resolved.clone().unwrap_or(Value::Null)
            }
            other => other.clone(),
        }
    }

    /// Soli truthiness: `false`, `nil`, `0`, `""`, `[]` and `{}` are falsy,
    /// everything else (`0.0` included) is truthy.
    ///
    /// Inlined for the scalar tags, which is what nearly every condition
    /// tests: the VM's conditional jumps call this once per branch, and as an
    /// out-of-line function the `Bool` case paid a call, a stack frame and a
    /// return around a one-byte read. Collections, strings and `Deferred`
    /// (which may flush a query batch) go to `is_truthy_slow`, kept out of
    /// line so the dispatch loop does not inline a batch flush at each site.
    #[inline]
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::Int(i) => *i != 0,
            _ => self.is_truthy_slow(),
        }
    }

    #[inline(never)]
    fn is_truthy_slow(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::Int(0) => false,
            Value::Decimal(_) => true,
            Value::String(s) if s.is_empty() => false,
            Value::Array(arr) if arr.borrow().is_empty() => false,
            Value::Hash(hash) if hash.borrow().is_empty() => false,
            Value::Future(_) => true,
            Value::VmClosure(_) => true,
            // Truthiness follows the resolved query result (e.g. an empty
            // array is falsy), so resolve before testing.
            Value::Deferred(_) => self.force_deferred().is_truthy(),
            _ => true,
        }
    }

    /// Convert this value to a HashKey if possible.
    pub fn to_hash_key(&self) -> Option<HashKey> {
        HashKey::from_value(self)
    }

    #[inline]
    pub fn display_len(&self) -> usize {
        match self {
            Value::DateTime(_, _) => DateTimeText::LEN,
            // Count digits arithmetically. This used to be `n.to_string().len()`,
            // which heap-allocated a String purely to read its length and then
            // dropped it — and `join` calls this once per element to size its
            // buffer, so a 1000-element join did 1000 pointless allocations.
            Value::Int(n) => {
                let digits = n
                    .unsigned_abs()
                    .checked_ilog10()
                    .map_or(1, |l| l as usize + 1);
                digits + usize::from(*n < 0)
            }
            // Match `write_to_string` / Display (`n.to_string()`). ryu's
            // shortest form can differ (e.g. `1.0` → `"1.0"` vs Display `"1"`),
            // so we deliberately stay on Display for float identity.
            Value::Float(n) => n.to_string().len(),
            Value::Decimal(d) => d.to_string().len(),
            Value::String(s) => s.len(),
            Value::Symbol(s) => s.len() + 1,
            Value::Bool(b) => {
                if *b {
                    4
                } else {
                    5
                }
            }
            Value::Null => 4,
            Value::Array(arr) => {
                let arr = arr.borrow();
                if arr.is_empty() {
                    return 2;
                }
                let mut len = 1;
                for (i, v) in arr.iter().enumerate() {
                    len += v.display_len();
                    if i > 0 {
                        len += 2;
                    }
                }
                len + 1
            }
            Value::Hash(hash) => {
                let hash = hash.borrow();
                if hash.is_empty() {
                    return 2;
                }
                let mut len = 1;
                for (i, (k, v)) in hash.iter().enumerate() {
                    len += k.to_value().display_len();
                    len += 4;
                    len += v.display_len();
                    if i > 0 {
                        len += 2;
                    }
                }
                len + 1
            }
            Value::Function(func) => func.name.len() + 5,
            Value::NativeFunction(func) => func.name.len() + 13,
            Value::Class(class) => class.name.len() + 8,
            Value::Instance(inst) => {
                let inst = inst.borrow();
                inst.class.name.len() + 15
            }
            Value::Future(_) => 7,
            Value::Method(_) => 8,
            Value::Breakpoint => 10,
            Value::Continue => 9,
            Value::QueryBuilder(_) => 13,
            Value::Super(_) => 7,
            Value::VmClosure(func) => func.proto.name.len() + 5,
            Value::Image(_) => 7,
            Value::ImagePlan(_) => 11,
            Value::Deferred(_) => self.force_deferred().display_len(),
        }
    }

    #[inline]
    pub fn write_to_string(&self, s: &mut String) {
        match self {
            Value::DateTime(ts, use_utc) => Self::write_datetime(*ts, *use_utc, s),
            // itoa writes into a stack buffer; `to_string()` allocated one
            // String per element. Integers have a single decimal form, so the
            // output is byte-identical.
            Value::Int(n) => s.push_str(itoa::Buffer::new().format(*n)),
            Value::Float(n) => s.push_str(&n.to_string()),
            Value::Decimal(d) => s.push_str(&d.to_string()),
            Value::String(st) => s.push_str(st),
            Value::Symbol(sym) => {
                s.push(':');
                s.push_str(sym);
            }
            Value::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
            Value::Null => s.push_str("null"),
            Value::Array(arr) => {
                s.push('[');
                let arr = arr.borrow();
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    v.write_to_string(s);
                }
                s.push(']');
            }
            Value::Hash(hash) => {
                s.push('{');
                let hash = hash.borrow();
                for (i, (k, v)) in hash.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    k.to_value().write_to_string(s);
                    s.push_str(" => ");
                    v.write_to_string(s);
                }
                s.push('}');
            }
            Value::Function(func) => {
                s.push_str("<fn ");
                s.push_str(&func.name);
                s.push('>');
            }
            Value::NativeFunction(func) => {
                s.push_str("<native fn ");
                s.push_str(&func.name);
                s.push('>');
            }
            Value::Class(class) => {
                s.push_str("<class ");
                s.push_str(&class.name);
                s.push('>');
            }
            Value::Instance(inst) => {
                let inst = inst.borrow();
                s.push('<');
                s.push_str(&inst.class.name);
                s.push_str(" instance>");
            }
            Value::Future(_) => s.push_str("<Future>"),
            Value::Method(_) => s.push_str("<Method>"),
            Value::Breakpoint => s.push_str("<Breakpoint>"),
            Value::Continue => s.push_str("<Continue>"),
            Value::QueryBuilder(_) => s.push_str("<QueryBuilder>"),
            Value::Super(_) => s.push_str("<Super>"),
            Value::VmClosure(func) => {
                s.push_str("<fn ");
                s.push_str(&func.proto.name);
                s.push('>');
            }
            Value::Image(_) => s.push_str("<Image>"),
            Value::ImagePlan(_) => s.push_str("<ImagePlan>"),
            Value::Deferred(_) => self.force_deferred().write_to_string(s),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        // A `grouped {}` deferred compares as its resolved value.
        if self.is_deferred() || other.is_deferred() {
            return self.force_deferred() == other.force_deferred();
        }
        // Same reason as `Display`: comparing two cyclic structures recursed
        // forever and overflowed the native stack, aborting the process. At the
        // limit we answer "not equal", which is the safe direction — an
        // authorization check that compares deep structures fails closed.
        if matches!(self, Value::Array(_) | Value::Hash(_) | Value::Instance(_)) {
            return with_depth(&EQ_DEPTH, MAX_EQ_DEPTH, false, || self.eq_inner(other));
        }
        self.eq_inner(other)
    }
}

impl Value {
    fn eq_inner(&self, other: &Self) -> bool {
        match (self, other) {
            // By instant, so two DateTimes built from the same moment compare
            // equal. While a DateTime was an `Instance` this fell to the
            // Instance arm below, which compares `datetime_ts`; the native
            // variant needs its own arm or `a == b` is always false.
            (Value::DateTime(a, _), Value::DateTime(b, _)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Decimal(a), Value::Decimal(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Symbol(a), Value::Symbol(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::Array(a), Value::Array(b)) => {
                // Use structural equality for arrays
                let a_ref = a.borrow();
                let b_ref = b.borrow();
                if a_ref.len() != b_ref.len() {
                    return false;
                }
                a_ref.iter().zip(b_ref.iter()).all(|(x, y)| x == y)
            }
            (Value::Hash(a), Value::Hash(b)) => {
                // Use structural equality for hashes (O(n) with IndexMap)
                let a_ref = a.borrow();
                let b_ref = b.borrow();
                if a_ref.len() != b_ref.len() {
                    return false;
                }
                // Check that all key-value pairs in a exist in b with same values
                a_ref.iter().all(|(k, v_a)| b_ref.get(k) == Some(v_a))
            }
            (Value::Instance(a), Value::Instance(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true;
                }
                // Two value objects are equal when their fields are.
                if crate::interpreter::domain::value_objects_declared() {
                    let (a_ref, b_ref) = (a.borrow(), b.borrow());
                    if a_ref.class.is_value_object() {
                        return crate::interpreter::domain::value_objects_equal(&a_ref, &b_ref);
                    }
                }
                // Two DateTime instances are equal when they point at the
                // same moment (compare by internal `_ts` nanosecond field).
                // All other instances fall back to pointer identity.
                if let (Some(ts_a), Some(ts_b)) = (self.datetime_ts(), other.datetime_ts()) {
                    return ts_a == ts_b;
                }
                false
            }
            (Value::Method(a), Value::Method(b)) => {
                *a.receiver == *b.receiver && a.method_name == b.method_name
            }
            // A class is its name: a reopened class, a model rebound per worker
            // or a job given its facade is a fresh `Rc` that still answers to
            // the same constant. With no arm here, `Account == Account` was
            // false.
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b) || a.name == b.name,
            // `record.class` answers the class *name*, a String, so
            // `record.class == Account` compared a string with a class and
            // was always false. Both spellings hold: `== Account` and
            // `== "Account"`.
            (Value::Class(class), Value::String(name))
            | (Value::String(name), Value::Class(class)) => class.name == name.as_str(),
            _ => false,
        }
    }
}

thread_local! {
    /// Nesting depth of in-flight `Display for Value` recursions.
    ///
    /// `str(x)`, string interpolation and `print` all walk a value recursively.
    /// Runtime graphs are routinely cyclic — `parent` ↔ `children`, a relation
    /// cache, `h["me"] = h` — and a cycle made the walk infinite, overflowing
    /// the native stack. That is not a catchable panic: it aborts the process,
    /// so a single log line like `"#{node}"` could take down every worker.
    /// Serialization was already guarded this way; formatting was not.
    static DISPLAY_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };

    /// Same, for structural equality.
    static EQ_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Depth at which `Display` stops descending and prints a marker.
///
/// Deep enough that no legitimate structure reaches it (the JSON parser caps
/// inbound nesting at 512), shallow enough to be safe on any thread stack.
const MAX_DISPLAY_DEPTH: usize = 256;

/// Depth at which structural equality gives up and answers `false`.
const MAX_EQ_DEPTH: usize = 256;

/// Run `body` one level deeper, or return `on_overflow` at the limit.
/// Restores the counter on every path so one guarded call cannot leave the
/// depth raised for the next.
/// Is a value being formatted, compared or serialized on this thread? A
/// request does not suspend in the middle of one (`serve::coroutine`).
pub(crate) fn formatting_in_progress() -> bool {
    DISPLAY_DEPTH.with(|d| d.get() > 0)
        || EQ_DEPTH.with(|d| d.get() > 0)
        || SERIALIZE_DEPTH.with(|d| d.get() > 0)
}

fn with_depth<T>(
    counter: &'static std::thread::LocalKey<std::cell::Cell<usize>>,
    limit: usize,
    on_overflow: T,
    body: impl FnOnce() -> T,
) -> T {
    let depth = counter.with(|d| {
        d.set(d.get() + 1);
        d.get()
    });
    if depth > limit {
        counter.with(|d| d.set(depth - 1));
        return on_overflow;
    }
    let result = body();
    counter.with(|d| d.set(d.get() - 1));
    result
}

/// Concatenate already-evaluated interpolation parts into one `SoliStr`
/// (the VM's `BuildString`).
///
/// The strategy depends on the length, which is known up front for strings
/// and the cheap scalars. Measured under the release profile (fat LTO,
/// mimalloc, where a small malloc/free pair is nearly free):
/// - A result within `SoliStr`'s inline limit (15 bytes) is pushed straight
///   into a `SoliStr`, with no heap at all.
/// - Anything longer, or of unknown length, is built in an exactly-sized
///   `String` and copied once. Pushing into a spilled `SoliStr` pays a
///   uniqueness and capacity check per push and loses by 20–30% there.
///
/// A stack buffer and a thread-local reusable `String` were both tried and
/// measured no faster. Out of line so the VM's dispatch loop only carries a
/// call.
#[inline(never)]
pub fn build_interpolated(parts: &[Value]) -> SoliStr {
    build_display(parts.iter())
}

/// `a` followed by `b`, each as `Display` writes it: the `+` of a string and
/// any other value, on both engines. Built like an interpolation (and through
/// the same code), so `"n=" + 1.5` and `"n=#{1.5}"` are one string, made the
/// same way; `eco_format!` went through `Display`'s formatter and depth guard
/// for every number, 17 ns more per `"str" + int` than the interpolation.
#[inline(never)]
pub fn concat_display(a: &Value, b: &Value) -> SoliStr {
    build_display([a, b].into_iter())
}

/// The body of [`build_interpolated`] and [`concat_display`]: two passes over
/// the parts, one to size the result and one to write it.
#[inline(always)]
fn build_display<'a, I>(parts: I) -> SoliStr
where
    I: Iterator<Item = &'a Value> + Clone,
{
    let mut len = 0;
    for part in parts.clone() {
        // Measured inline rather than through `display_len`, whose recursive
        // container arms keep it from inlining: a call per part cost the
        // multi-part case more than the build saves.
        len += match part {
            Value::String(s) => s.len(),
            Value::Int(n) => {
                let digits = n
                    .unsigned_abs()
                    .checked_ilog10()
                    .map_or(1, |l| l as usize + 1);
                digits + usize::from(*n < 0)
            }
            Value::Bool(true) | Value::Null => 4,
            Value::Bool(false) => 5,
            Value::Symbol(s) => s.len() + 1,
            // Unknown until formatted: count past the inline limit.
            _ => SoliStr::INLINE_LIMIT + 1,
        };
    }
    if len <= SoliStr::INLINE_LIMIT {
        let mut out = SoliStr::new();
        for part in parts {
            part.append_to(&mut out);
        }
        out
    } else {
        let mut out = String::with_capacity(len);
        for part in parts {
            part.append_to(&mut out);
        }
        SoliStr::from(out.as_str())
    }
}

/// Write `n` exactly as `format!("{}", n)` would, but through `ryu` for the
/// common case.
///
/// Both produce the shortest digits that round-trip, and differ in notation:
/// ryu prints `1.0` where Display prints `1`, and switches to exponent form
/// (`1e16`) where Display writes every digit. The first is normalised, the
/// second handed back to Display.
///
/// They can also differ in the digits themselves. When the shortest form
/// needs 16–17 significant digits, two candidates may both round-trip and
/// the two libraries break the tie differently (`1658206780088562.2` vs
/// `…562.3`). With at most 15 there is only one candidate: 15-digit decimals
/// sit at least 1e-15 apart relative to the value, while the round-trip
/// interval is one ulp, at most 2.2e-16. So ryu's answer is only taken at 15
/// digits or fewer, which covers the floats people actually print.
#[inline]
pub(crate) fn write_float<W: std::fmt::Write + ?Sized>(n: f64, out: &mut W) -> fmt::Result {
    const UNIQUE_SHORTEST_DIGITS: usize = 15;
    if n.is_finite() {
        let mut buf = ryu::Buffer::new();
        let formatted = buf.format_finite(n);
        let bytes = formatted.as_bytes();
        // Counting trailing zeros too only makes the check stricter.
        let significant = bytes
            .iter()
            .filter(|b| b.is_ascii_digit())
            .skip_while(|&&b| b == b'0')
            .count();
        if significant <= UNIQUE_SHORTEST_DIGITS && !bytes.contains(&b'e') {
            return out.write_str(formatted.strip_suffix(".0").unwrap_or(formatted));
        }
    }
    write!(out, "{}", n)
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Containers are the only recursive arms, so scalars skip the guard.
        if matches!(
            self,
            Value::Array(_) | Value::Hash(_) | Value::Instance(_) | Value::Deferred(_)
        ) {
            return with_depth(&DISPLAY_DEPTH, MAX_DISPLAY_DEPTH, Ok(()), || {
                self.fmt_inner(f)
            })
            .and_then(|()| {
                if DISPLAY_DEPTH.with(|d| d.get()) >= MAX_DISPLAY_DEPTH {
                    write!(f, "<...>")
                } else {
                    Ok(())
                }
            });
        }
        self.fmt_inner(f)
    }
}

impl Value {
    fn fmt_inner(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::DateTime(ts, use_utc) => {
                f.write_str(Value::datetime_text(*ts, *use_utc).as_str())
            }
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Decimal(d) => write!(f, "{}", d),
            Value::String(s) => write!(f, "{}", s),
            Value::Symbol(s) => write!(f, ":{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Null => write!(f, "null"),
            Value::Array(arr) => {
                write!(f, "[")?;
                let arr = arr.borrow();
                for (i, val) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", val)?;
                }
                write!(f, "]")
            }
            Value::Hash(hash) => {
                write!(f, "{{")?;
                let hash = hash.borrow();
                for (i, (key, val)) in hash.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} => {}", key.to_value(), val)?;
                }
                write!(f, "}}")
            }
            Value::Function(func) => write!(f, "<fn {}>", func.name),
            Value::NativeFunction(func) => write!(f, "<native fn {}>", func.name),
            Value::Class(class) => write!(f, "<class {}>", class.name),
            Value::Instance(inst) => {
                let inst_ref = inst.borrow();
                if crate::interpreter::domain::value_objects_declared()
                    && inst_ref.class.is_value_object()
                {
                    // A value is its fields: `Amount(cents: 500, currency: "EUR")`.
                    write!(f, "{}(", inst_ref.class.name)?;
                    for (i, (k, v)) in crate::interpreter::domain::value_fields(&inst_ref)
                        .iter()
                        .enumerate()
                    {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        match v {
                            Value::String(s) => write!(f, "{}: \"{}\"", k, s)?,
                            _ => write!(f, "{}: {}", k, v)?,
                        }
                    }
                    write!(f, ")")
                } else if inst_ref.fields.is_empty() {
                    write!(f, "<{} instance>", inst_ref.class.name)
                } else {
                    write!(f, "<{}", inst_ref.class.name)?;
                    let mut first = true;
                    for (k, v) in inst_ref.fields.iter() {
                        // Hide _errors when empty
                        if k == "_errors" {
                            if let Value::Array(arr) = v {
                                if arr.borrow().is_empty() {
                                    continue;
                                }
                            }
                        }
                        if first {
                            write!(f, " ")?;
                            first = false;
                        } else {
                            write!(f, ",\n ")?;
                        }
                        match v {
                            Value::String(s) => write!(f, "{}: \"{}\"", k, s)?,
                            _ => write!(f, "{}: {}", k, v)?,
                        }
                    }
                    write!(f, ">")
                }
            }
            Value::Future(state) => {
                // Auto-resolve the future when displaying
                let guard = state.lock().unwrap();
                match &*guard {
                    FutureState::Pending { .. } => write!(f, "<pending future>"),
                    FutureState::Resolved(val) => write!(f, "{}", val),
                    FutureState::Error(e) => write!(f, "<error: {}>", e),
                }
            }
            Value::Method(method) => write!(
                f,
                "<method {}.{}>",
                method.receiver.type_name(),
                method.method_name
            ),
            Value::Breakpoint => write!(f, "<breakpoint>"),
            Value::Continue => write!(f, "<continue>"),
            Value::QueryBuilder(qb) => {
                let qb = qb.borrow();
                if qb.filter.is_some() {
                    write!(f, "<QueryBuilder for {} with filter>", qb.class_name)
                } else {
                    write!(f, "<QueryBuilder for {}>", qb.class_name)
                }
            }
            Value::Super(class) => write!(f, "<super of {}>", class.name),
            Value::VmClosure(c) => write!(f, "<fn {}>", c.proto.name),
            Value::Image(_) => write!(f, "<Image>"),
            Value::ImagePlan(p) => {
                let p = p.borrow();
                write!(f, "<ImagePlan src=\"{}\" ops={}>", p.src, p.ops.len())
            }
            // Auto-resolve a `grouped {}` deferred when displaying.
            Value::Deferred(_) => write!(f, "{}", self.force_deferred()),
        }
    }
}

/// A user-defined function.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    // `Rc<[..]>` so binding `this` (which rebuilds a `Function` from an existing
    // method on every call) is an O(1) refcount bump instead of a deep clone of
    // the whole parameter list / method-body AST. Built once from the AST via
    // `from_decl`/`from_method`; never mutated after construction.
    pub params: Rc<[Parameter]>,
    // The body is the parsed AST's own `Body`, shared with every worker
    // rather than copied per worker. It sits behind this thread's `Rc` so
    // that rebinding `this` on a call bumps a non-atomic count: an `Arc`
    // bumped on every call by every worker would be one cache line bounced
    // between all their cores.
    pub body: Rc<Body>,
    pub closure: Rc<RefCell<Environment>>,
    pub is_method: bool,
    pub span: Option<Span>,
    pub source_path: Option<String>,
    /// The superclass of the class where this method was defined.
    /// Used for super calls to resolve to the correct parent class.
    pub defining_superclass: Option<Rc<Class>>,
    /// The declared return type annotation, if any.
    /// Used for runtime return type enforcement.
    pub return_type: Option<TypeAnnotation>,
    /// Single-slot cache of the lambda's call environment.
    ///
    /// Taken on entry to `call_function`, cleared, re-populated with the new
    /// arguments, and put back on exit. Saves 2 HashMap + 1 Rc alloc per call.
    /// Recursive calls observe `None` and transparently allocate a fresh env;
    /// the outer call's restore wins, which is fine (re-caching is a hint).
    pub cached_env: RefCell<Option<Rc<RefCell<Environment>>>>,
    /// Cached JIT-compiled FunctionProto — compiled once on first call,
    /// reused on subsequent calls.
    pub jit_cache: RefCell<Option<std::sync::Arc<crate::vm::chunk::FunctionProto>>>,
    /// The VM closure over `jit_cache`'s proto (it has no upvalues), made on
    /// the first VM call and reused by the next ones instead of a fresh
    /// `Rc<VmClosure>` per call. Only trusted while its proto is the one in
    /// `jit_cache` (`crate::vm::vm_calls::cached_function_closure`).
    pub jit_closure: RefCell<Option<Rc<VmClosure>>>,
    /// Machine code for this function, when it is a top-level def typed
    /// Int/Float/Bool end to end (see `crate::native`). Tried first on each
    /// call; a declined call runs the body as usual.
    pub kernel: Option<crate::native::KernelRef>,
    /// A `static def` whose body reads `this`: reading it off a class binds
    /// `this` to that class. The others skip the binding, which costs an
    /// environment and a function per access.
    pub binds_this: bool,
}

impl Default for Function {
    fn default() -> Self {
        Self {
            name: String::new(),
            params: Vec::new().into(),
            body: Rc::new(Body::default()),
            closure: Rc::new(RefCell::new(Environment::new())),
            is_method: false,
            span: None,
            source_path: None,
            defining_superclass: None,
            return_type: None,
            cached_env: RefCell::new(None),
            jit_cache: RefCell::new(None),
            jit_closure: RefCell::new(None),
            binds_this: false,
            kernel: None,
        }
    }
}

impl Function {
    pub fn from_decl(
        decl: &FunctionDecl,
        closure: Rc<RefCell<Environment>>,
        source_path: Option<String>,
    ) -> Self {
        Self {
            name: decl.name.clone(),
            params: decl.params.clone().into(),
            body: Rc::new(decl.body.clone()),
            closure,
            is_method: false,
            span: Some(decl.span),
            source_path,
            defining_superclass: None,
            return_type: decl.return_type.clone(),
            cached_env: RefCell::new(None),
            jit_cache: RefCell::new(None),
            jit_closure: RefCell::new(None),
            binds_this: false,
            kernel: None,
        }
    }

    pub fn from_method(
        decl: &MethodDecl,
        closure: Rc<RefCell<Environment>>,
        source_path: Option<String>,
    ) -> Self {
        Self {
            name: decl.name.clone(),
            params: decl.params.clone().into(),
            body: Rc::new(decl.body.clone()),
            closure,
            is_method: true,
            span: Some(decl.span),
            source_path,
            defining_superclass: None,
            return_type: decl.return_type.clone(),
            cached_env: RefCell::new(None),
            jit_cache: RefCell::new(None),
            jit_closure: RefCell::new(None),
            binds_this: false,
            kernel: None,
        }
    }

    pub fn arity(&self) -> usize {
        // Return the number of required parameters (params without defaults)
        self.params
            .iter()
            .filter(|p| p.default_value.is_none())
            .count()
    }

    /// Full arity including optional parameters
    pub fn full_arity(&self) -> usize {
        self.params.len()
    }

    /// Get the default value expression for a parameter at index
    pub fn param_default_value(&self, index: usize) -> Option<&Expr> {
        self.params
            .get(index)
            .and_then(|p| p.default_value.as_ref())
    }
}

/// The callable behind a `NativeFunction`. Takes a *slice*, not an owned `Vec`:
/// every native call used to copy its arguments into a fresh allocation,
/// including the zero-argument calls that dominate (`dt.year()`, `s.upcase()`).
pub type NativeFn = Rc<dyn Fn(&[Value]) -> Result<Value, String>>;

/// A native/builtin function. Newtype over `Rc<NativeFunctionInner>` so the
/// `Value::NativeFunction` variant is pointer-sized (8B) instead of inlining
/// the ~64B payload into every `Value` (arrays, hashes, model rows, env
/// bindings all pay `size_of::<Value>()`). All field access flows through
/// `Deref`, so existing `.name` / `.arity` / `.func` / `.is_auto_invocable`
/// reads and `Value::NativeFunction(f) => …` patterns compile unchanged.
#[derive(Clone)]
pub struct NativeFunction(Rc<NativeFunctionInner>);

pub struct NativeFunctionInner {
    pub name: String,
    pub arity: Option<usize>, // None means variadic
    /// Takes a *slice*, not an owned `Vec`. Every native call used to copy its
    /// arguments into a fresh `Vec` — including the zero-argument calls that
    /// dominate (`dt.year()`, `s.upcase()`, `d.total_seconds()`), where the
    /// allocation was pure overhead.
    pub func: NativeFn,
    pub is_auto_invocable: bool, // Can be called without parentheses
    /// Which of the few natives the VM intercepts this one is, worked out
    /// once from `name` at construction so a global native call branches on
    /// a byte instead of comparing its name against every special.
    pub vm_special: NativeSpecial,
}

/// The natives the VM handles itself before (or instead of) calling `func`:
/// block-taking builtins that must run a closure on the calling VM, and the
/// ones whose tree-walker behaviour needs VM state. `None` for every other
/// native, which the VM calls straight on its stack slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeSpecial {
    None,
    Grouped,
    WithTransaction,
    AfterCommit,
    WithTenant,
    RenderJson,
    AssertRaises,
    Defined,
    ConstGet,
}

impl NativeSpecial {
    pub fn of(name: &str) -> Self {
        match name {
            "grouped" => Self::Grouped,
            "with_transaction" => Self::WithTransaction,
            "after_commit" => Self::AfterCommit,
            "with_tenant" => Self::WithTenant,
            "render_json" => Self::RenderJson,
            "assert_raises" => Self::AssertRaises,
            "defined" => Self::Defined,
            "const_get" => Self::ConstGet,
            _ => Self::None,
        }
    }
}

impl std::ops::Deref for NativeFunction {
    type Target = NativeFunctionInner;
    #[inline]
    fn deref(&self) -> &NativeFunctionInner {
        &self.0
    }
}

impl NativeFunction {
    pub fn new<F>(name: impl Into<String>, arity: Option<usize>, func: F) -> Self
    where
        F: Fn(&[Value]) -> Result<Value, String> + 'static,
    {
        Self::new_with_auto(name, arity, false, func)
    }

    pub fn new_auto_invocable<F>(name: impl Into<String>, arity: Option<usize>, func: F) -> Self
    where
        F: Fn(&[Value]) -> Result<Value, String> + 'static,
    {
        Self::new_with_auto(name, arity, true, func)
    }

    /// Construct with an explicit `is_auto_invocable` flag — for wrapper
    /// functions that inherit the flag from the method they bind.
    pub fn new_with_auto<F>(
        name: impl Into<String>,
        arity: Option<usize>,
        is_auto_invocable: bool,
        func: F,
    ) -> Self
    where
        F: Fn(&[Value]) -> Result<Value, String> + 'static,
    {
        let name = name.into();
        let vm_special = NativeSpecial::of(&name);
        Self(Rc::new(NativeFunctionInner {
            name,
            arity,
            func: Rc::new(func),
            is_auto_invocable,
            vm_special,
        }))
    }
}

impl fmt::Debug for NativeFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NativeFunction({})", self.name)
    }
}

/// A method on a value (array/hash) that captures the receiver and method name.
/// This allows calling methods like `.map()`, `.filter()`, `.each()` on values.
#[derive(Clone)]
pub struct ValueMethod {
    pub receiver: Box<Value>,
    pub method_name: String,
}

impl fmt::Debug for ValueMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<method {}.{}>",
            self.receiver.type_name(),
            self.method_name
        )
    }
}

/// Tag identifying one of Soli's primitive types. Used on `Class.primitive`
/// so `class_eval` / `define_method` can route writes to the per-type user-method
/// overlay (`executor::calls::user_methods::USER_METHODS`) instead of the
/// `Class.methods` map, which is irrelevant for primitive dispatch.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimType {
    Int = 0,
    Float = 1,
    Bool = 2,
    Null = 3,
    Decimal = 4,
    String = 5,
    Array = 6,
    Hash = 7,
    Symbol = 8,
}

pub const PRIM_TYPE_COUNT: usize = 9;

/// The native method a Rust-defined class runs when it is called like a
/// function, `RateLimiter("api:42", 100, 60)`: it receives the new instance
/// followed by the arguments, and fills the instance's fields. A native class
/// has no Soli constructor to run, so without this hook the arguments were
/// dropped and the instance came back empty.
pub const NATIVE_CONSTRUCTOR: &str = "__construct__";

/// The method that evaluates a class's field defaults that are not literals
/// (`created: Int = clock()`) on a new instance and returns them as a hash,
/// `{"created": …}`. The VM compiles one per class that has such defaults
/// and keeps it in [`FieldInit::computed`], never among the class's methods.
pub const FIELD_DEFAULTS_METHOD: &str = "__field_defaults__";

/// How the VM starts one class's own declared instance fields on a new
/// instance, before any constructor runs: what the tree-walker's
/// `initialize_instance_fields` does for that class (each ancestor has its
/// own). Built once per class: by `Op::Field` and the
/// [`FIELD_DEFAULTS_METHOD`] when the VM compiles the class, from
/// [`Class::fields`] for one the tree-walker defined.
#[derive(Debug, Clone, Default)]
pub struct FieldInit {
    /// Every declared instance field, in declaration order, with the value it
    /// starts with: its literal default (`count: Int = 0`, `tags: Array =
    /// []`), else `nil`. That covers a field without a default, and one whose
    /// default `computed` evaluates.
    pub fields: Vec<(SoliStr, Value)>,
    /// The defaults that are not literals, run on the new instance as the
    /// class: a method returning them by field name.
    pub computed: Option<Rc<VmClosure>>,
}

impl FieldInit {
    /// Set every field to the value it starts with. An array or hash default
    /// is copied, so two instances never share one.
    pub fn start(&self, instance: &mut Instance) {
        for (name, value) in &self.fields {
            instance.fields.insert(name.clone(), fresh_copy(value));
        }
    }
}

/// `value` with each array and hash in it copied: a literal default handed to
/// one more instance.
fn fresh_copy(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(Rc::new(RefCell::new(
            items.borrow().iter().map(fresh_copy).collect(),
        ))),
        Value::Hash(pairs) => {
            let pairs = pairs.borrow();
            let mut copy = HashPairs::with_capacity(pairs.len());
            for (key, item) in pairs.iter() {
                copy.insert(key.clone(), fresh_copy(item));
            }
            Value::Hash(Rc::new(RefCell::new(copy)))
        }
        other => other.clone(),
    }
}

thread_local! {
    /// Bumped whenever a class's instance methods change after it was built
    /// (`define_method`, `alias_method`, `include`, a module body). Each
    /// class's flattened method cache records the epoch it was built at and
    /// is rebuilt on a mismatch. Classes are `Rc` and never cross threads, so
    /// a per-thread counter is enough.
    static METHOD_EPOCH: Cell<u64> = const { Cell::new(1) };
}

/// The current method-definition epoch (see `METHOD_EPOCH`).
#[inline]
pub fn method_epoch() -> u64 {
    METHOD_EPOCH.with(|epoch| epoch.get())
}

/// Mark every flattened method cache stale: a class's methods changed, and
/// its subclasses copied the old set into their own caches.
pub fn bump_method_epoch() {
    METHOD_EPOCH.with(|epoch| epoch.set(epoch.get().wrapping_add(1)));
}

/// A method as `Class::find_instance_method_level` (or
/// `find_static_method_level`) finds it on the class that declares it:
/// already compiled to bytecode, or still the tree-walker's AST.
pub enum InstanceMethodDef {
    Compiled(Rc<VmClosure>),
    Source(Rc<Function>),
}

/// A class definition.
#[derive(Debug, Clone)]
pub struct Class {
    pub name: String,
    pub superclass: Option<Rc<Class>>,
    /// Instance methods - using RefCell for interior mutability to support define_method
    pub methods: Rc<RefCell<HashMap<String, Rc<Function>>>>,
    pub static_methods: HashMap<String, Rc<Function>>,
    pub native_static_methods: HashMap<String, Rc<NativeFunction>>,
    pub native_methods: HashMap<String, Rc<NativeFunction>>,
    pub static_fields: Rc<RefCell<HashMap<String, Value>>>,
    pub fields: HashMap<String, Option<Expr>>,
    pub constructor: Option<Rc<Function>>,
    /// Nested classes defined within this class - using RefCell for interior mutability
    pub nested_classes: Rc<RefCell<HashMap<String, Rc<Class>>>>,
    /// Instance field names declared as `const` (immutable after initialization).
    pub const_fields: HashSet<String>,
    /// Static field names declared as `const` (immutable after initialization).
    pub static_const_fields: HashSet<String>,
    /// Flattened method cache for O(1) lookups including inherited methods.
    /// This is computed lazily on first access and includes all methods from the inheritance chain.
    /// ahash-keyed: `find_method` probes this once per method call.
    /// NOTE: Should not be manually set; use Class::new() constructor instead.
    pub all_methods_cache: RefCell<Option<HashMap<String, Rc<Function>, AHasher>>>,
    /// The [`method_epoch`] `all_methods_cache` was built at. The cache copies
    /// the superclass's flattened map, so a method added to an ancestor later
    /// (`A.define_method`, `alias_method`, `include`) must rebuild it too —
    /// the epoch is global and a mismatch means "rebuild". `pub` only for
    /// struct-literal construction: always `Cell::new(0)` outside this module.
    pub all_methods_cache_epoch: Cell<u64>,
    /// Flattened native method cache for O(1) lookups.
    /// NOTE: Should not be manually set; use Class::new() constructor instead.
    pub all_native_methods_cache: RefCell<Option<HashMap<String, Rc<NativeFunction>, AHasher>>>,
    /// `Some` when this Class represents a Soli primitive type (Int, Float, etc.).
    /// `class_eval` / `define_method` on a primitive-tagged class route writes
    /// to the per-type user-method overlay rather than `methods`, since primitive
    /// dispatch in `member.rs` and the VM does not consult `Class.methods`.
    pub primitive: Option<PrimType>,
    /// Bytecode instance methods, registered by `Op::Method` when a class is
    /// compiled in the VM (`compile_class_decl`). The constructor lands here
    /// under the name `"init"` (it returns `this`). `Rc` so the per-method
    /// class rebuilds in `op_add_method` share one map. ahash-keyed: the VM
    /// probes this on every compiled instance-method call. Write it through
    /// [`Class::insert_vm_method`], or call [`note_vm_methods_changed`] after
    /// writing: the VM's method cache trusts an entry until that epoch moves.
    pub vm_methods: Rc<RefCell<HashMap<String, Rc<VmClosure>, AHasher>>>,
    /// Bytecode static methods, registered by `Op::StaticMethod`. Compiled
    /// as plain functions (no `this` slot).
    pub vm_static_methods: Rc<RefCell<HashMap<String, Rc<VmClosure>, AHasher>>>,
    /// Bytecode for this class's own tree-walker `static def`s that do not
    /// read `this`, compiled by the VM as plain functions on their first call
    /// so later calls skip the name lookups and the closure allocation. Only
    /// entries of `static_methods` land here, and that map never changes once
    /// the class is built; a clone starts empty (see [`PlainStaticCache`]).
    pub vm_plain_static_methods: PlainStaticCache,
    /// Memoized result of [`Class::is_model_subclass`]. The superclass chain
    /// and class names are fixed at construction, so the walk's result can
    /// never change for a given `Class` value. Instance member access checks
    /// model-ness up to four times per access — without this memo each check
    /// re-walks the whole superclass chain with string compares.
    /// `pub` only so `..Default::default()` struct-update construction works
    /// outside this module — always leave it `Cell::new(None)`; reading goes
    /// through [`Class::is_model_subclass`].
    pub model_subclass_memo: Cell<Option<bool>>,
    /// Memoized [`Class::is_value_object`]. Same contract as
    /// `model_subclass_memo`: always `Cell::new(None)` outside this module.
    pub value_object_memo: Cell<Option<bool>>,
    /// How the VM starts this class's own declared instance fields (see
    /// [`FieldInit`]). Set when the VM compiles the class, or the first time
    /// it builds an instance of a class the tree-walker defined. `None` until
    /// then, and for a class that declares no instance field.
    pub field_init: RefCell<Option<Rc<FieldInit>>>,
    /// The VM's compiled form of [`Class::constructor`] (a tree-walker
    /// constructor: every class of a served app), with the function it was
    /// compiled from. Compiled once, in constructor mode, the first time the
    /// VM runs it (`Vm::compiled_constructor`); every construction then
    /// enters it as one frame. A copy of the class given another
    /// constructor compiles its own: the function is compared by pointer.
    pub vm_constructor: RefCell<Option<(Rc<Function>, Rc<VmClosure>)>>,
    /// Memo for [`Class::intercepts_new`]: always `Default::default()`
    /// outside this module.
    pub intercepts_new_memo: InterceptsNewMemo,
    /// The instance fields this class declares (`cents: Int`), in order. A
    /// value object's implicit constructor binds positional arguments to
    /// them, and `to_h` lists them in this order. Inherited ones live on the
    /// superclass (see [`Class::all_declared_fields`]).
    pub declared_fields: Rc<RefCell<Vec<String>>>,
    /// `true` when this Class was defined with `module Name … end`.
    pub is_module: bool,
    /// Modules mixed into this class via `include` (instance methods).
    pub included_modules: Rc<RefCell<Vec<Rc<Class>>>>,
    /// Instance methods copied from `extend Module` as class methods.
    pub mixin_static_methods: Rc<RefCell<HashMap<String, Rc<Function>>>>,
    /// `included do … end` bodies, replayed on each includer.
    pub included_hook_stmts: Rc<RefCell<Vec<Vec<crate::ast::Stmt>>>>,
    /// `extended do … end` bodies, replayed on each extender.
    pub extended_hook_stmts: Rc<RefCell<Vec<Vec<crate::ast::Stmt>>>>,
    /// `class_methods do` methods (copied onto includers as class methods).
    pub concern_static_methods: Rc<RefCell<HashMap<String, Rc<Function>>>>,
    /// Names of concern class methods (VM copies matching `vm_static_methods`).
    pub concern_method_names: Rc<RefCell<Vec<String>>>,
    /// Instance methods and fields declared `private` on this class (not its
    /// ancestors): reachable on `self` only — `@name`, `this.name`, or the
    /// bare `name` for a method. `Rc` so the VM's per-method class rebuilds
    /// share one set.
    pub private_members: Rc<RefCell<HashSet<String>>>,
    /// Instance methods and fields declared `protected` on this class:
    /// reachable from code running in an instance of the declaring class or a
    /// subclass, on that instance or another one. `Rc` as above.
    ///
    /// A name added to either set after the class exists must be followed by
    /// [`note_restricted_methods_declared`] (or
    /// [`note_restricted_members_changed`]), which stales
    /// `restricted_names_memo` on every class.
    pub protected_members: Rc<RefCell<HashSet<String>>>,
    /// Memo for [`Class::restricted_method`]: always `Default::default()`
    /// outside this module.
    pub restricted_names_memo: RefCell<RestrictedNames>,
}

/// [`Class::vm_plain_static_methods`]: plain-function bytecode for a class's
/// own `static def`s, by name.
///
/// Cloning gives an empty cache rather than a copy. A class is rebuilt by
/// cloning it and replacing some of its maps (`op_add_method`, a reopened
/// class), and the clone's `static_methods` may name a different function
/// than the original's did; it compiles its own entries on first call. The
/// map itself is created on the first insert, so a class nobody calls a
/// static on never seeds an `ahash` state.
#[derive(Default)]
pub struct PlainStaticCache(RefCell<Option<HashMap<String, Rc<VmClosure>, AHasher>>>);

impl PlainStaticCache {
    /// The cached bytecode for `name`, if it was compiled.
    #[inline]
    pub fn get(&self, name: &str) -> Option<Rc<VmClosure>> {
        self.0.borrow().as_ref()?.get(name).cloned()
    }

    /// Remember the bytecode compiled for `name`.
    pub fn insert(&self, name: &str, closure: Rc<VmClosure>) {
        self.0
            .borrow_mut()
            .get_or_insert_with(HashMap::default)
            .insert(name.to_string(), closure);
    }
}

impl Clone for PlainStaticCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl fmt::Debug for PlainStaticCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self
            .0
            .borrow()
            .as_ref()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_tuple("PlainStaticCache").field(&names).finish()
    }
}

/// The names the classes of one chain declare private or protected, as
/// [`Class::restricted_method`] last collected them.
#[derive(Debug, Default)]
pub struct RestrictedNames {
    /// The `RESTRICTED_MEMBERS_EPOCH` they were collected in; 0 before the
    /// first collection, which no epoch equals.
    epoch: u64,
    /// `None` when no class of the chain declares one.
    names: Option<HashSet<String, AHasher>>,
}

/// A copy starts uncollected. A cloned [`Class`] can be given another
/// superclass or other private sets before its first lookup, and must not
/// answer from the original's chain. The cost is one collection per clone.
impl Clone for RestrictedNames {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// How far a restricted instance method reaches (see [`Class::restricted_method`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodAccess {
    /// Only on `self`.
    Private,
    /// From code running in an instance of the declaring class or a subclass.
    Protected,
}

impl MethodAccess {
    pub fn label(self) -> &'static str {
        match self {
            MethodAccess::Private => "private",
            MethodAccess::Protected => "protected",
        }
    }

    /// May a call reach it? `on_self`: the receiver is the running method's
    /// own `this`. `caller`: the class of that `this`, when there is one.
    pub fn allows(self, defining_class: &str, on_self: bool, caller: Option<&Class>) -> bool {
        on_self
            || (self == MethodAccess::Protected
                && caller.is_some_and(|class| class.is_or_inherits(defining_class)))
    }

    /// The error for a call (or, when `is_field`, a field access) it refuses.
    pub fn refusal(self, name: &str, class_name: &str, is_field: bool) -> String {
        let (kind, verb) = if is_field {
            ("field", "accessed")
        } else {
            ("method", "called")
        };
        format!(
            "{} {} '{}' {} for an instance of {}",
            self.label(),
            kind,
            name,
            verb,
            class_name
        )
    }
}

/// [`Class::intercepts_new`]'s last answer for one class, with the
/// `STATIC_METHODS_EPOCH` it was computed in (0: never, which no epoch
/// equals).
#[derive(Debug, Default)]
pub struct InterceptsNewMemo(Cell<(u64, bool)>);

/// A copy starts empty: a cloned [`Class`] can be given another superclass
/// or other static methods before it is first asked.
impl Clone for InterceptsNewMemo {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Bumped whenever a static method joins a class that already exists — an
/// `include` or `extend`, a `static def` the VM registers on the class it is
/// compiling, a module's `class_methods` — so every
/// [`Class::intercepts_new`] memo is computed again: a subclass's answer
/// depends on its ancestors' tables too. Starts at 1, so a fresh memo is
/// never current. The VM's caches of tree-walker statics
/// (`compile_tree_walking_static`) do not bump it: they only hold statics
/// `find_static_method` already finds, so they cannot change an answer.
static STATIC_METHODS_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A static method joined an existing class: stale every
/// [`Class::intercepts_new`] memo.
pub fn note_static_methods_changed() {
    STATIC_METHODS_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Set once any class declares a private or protected instance method (see
/// [`Class::restricted_method`]).
static RESTRICTED_METHODS_DECLARED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Bumped whenever a name joins some class's `private_members` or
/// `protected_members`: a [`RestrictedNames`] memo collected in an earlier
/// epoch is collected again. Starts at 1, so a fresh memo is never current.
static RESTRICTED_MEMBERS_EPOCH: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

pub fn note_restricted_methods_declared() {
    RESTRICTED_METHODS_DECLARED.store(true, std::sync::atomic::Ordering::Relaxed);
    note_restricted_members_changed();
}

/// A name joined a class's private or protected set where the flag stays as
/// it is (a `private class` nested in a class): stale every
/// [`RestrictedNames`] memo. [`note_restricted_methods_declared`] does it too.
pub fn note_restricted_members_changed() {
    RESTRICTED_MEMBERS_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn restricted_methods_declared() -> bool {
    RESTRICTED_METHODS_DECLARED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bumped whenever some class's `vm_methods` map changes (see
/// [`Class::insert_vm_method`]): the VM's method cache (`vm::method_cache`)
/// answers a call only with what it found in the current epoch. Starts at 1,
/// so an empty entry (epoch 0) is never current.
static VM_METHODS_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Some class's `vm_methods` changed: stale every method-cache entry. A
/// global, not per class: a class's answer also depends on its ancestors'
/// maps, and changes happen at boot and warm-up, not per request.
pub fn note_vm_methods_changed() {
    VM_METHODS_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// The current `VM_METHODS_EPOCH`.
#[inline]
pub fn vm_methods_epoch() -> u64 {
    VM_METHODS_EPOCH.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
thread_local! {
    /// Test-only: the [`Class::restricted_method`] calls this thread made past
    /// the flag, and how many of them walked the chain.
    static RESTRICTED_LOOKUPS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

#[cfg(test)]
fn tally_restricted_lookup(walked: bool) {
    RESTRICTED_LOOKUPS.with(|tally| {
        let (lookups, walks) = tally.get();
        tally.set(if walked {
            (lookups, walks + 1)
        } else {
            (lookups + 1, walks)
        });
    });
}

/// Test-only: `(lookups past the flag, chain walks)` made on this thread so
/// far, for a test to tell a skipped privacy check from one that ran.
#[cfg(test)]
pub(crate) fn restricted_lookups() -> (usize, usize) {
    RESTRICTED_LOOKUPS.with(Cell::get)
}

impl Default for Class {
    fn default() -> Self {
        Self {
            name: String::new(),
            superclass: None,
            methods: Rc::new(RefCell::new(HashMap::new())),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            const_fields: HashSet::new(),
            static_const_fields: HashSet::new(),
            all_methods_cache: RefCell::new(None),
            all_methods_cache_epoch: Cell::new(0),
            all_native_methods_cache: RefCell::new(None),
            primitive: None,
            vm_methods: Rc::new(RefCell::new(HashMap::default())),
            vm_static_methods: Rc::new(RefCell::new(HashMap::default())),
            vm_plain_static_methods: PlainStaticCache::default(),
            model_subclass_memo: Cell::new(None),
            value_object_memo: Cell::new(None),
            field_init: RefCell::new(None),
            vm_constructor: RefCell::new(None),
            intercepts_new_memo: InterceptsNewMemo::default(),
            declared_fields: Rc::new(RefCell::new(Vec::new())),
            is_module: false,
            included_modules: Rc::new(RefCell::new(Vec::new())),
            mixin_static_methods: Rc::new(RefCell::new(HashMap::new())),
            included_hook_stmts: Rc::new(RefCell::new(Vec::new())),
            extended_hook_stmts: Rc::new(RefCell::new(Vec::new())),
            concern_static_methods: Rc::new(RefCell::new(HashMap::new())),
            concern_method_names: Rc::new(RefCell::new(Vec::new())),
            private_members: Rc::new(RefCell::new(HashSet::new())),
            protected_members: Rc::new(RefCell::new(HashSet::new())),
            restricted_names_memo: RefCell::default(),
        }
    }
}

impl Class {
    /// Create a new class with all fields initialized, including caches.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        superclass: Option<Rc<Class>>,
        methods: HashMap<String, Rc<Function>>,
        static_methods: HashMap<String, Rc<Function>>,
        native_static_methods: HashMap<String, Rc<NativeFunction>>,
        native_methods: HashMap<String, Rc<NativeFunction>>,
        static_fields: Rc<RefCell<HashMap<String, Value>>>,
        fields: HashMap<String, Option<Expr>>,
        constructor: Option<Rc<Function>>,
        nested_classes: Rc<RefCell<HashMap<String, Rc<Class>>>>,
    ) -> Self {
        Self {
            name,
            superclass,
            methods: Rc::new(RefCell::new(methods)),
            static_methods,
            native_static_methods,
            native_methods,
            static_fields,
            fields,
            constructor,
            nested_classes,
            const_fields: HashSet::new(),
            static_const_fields: HashSet::new(),
            all_methods_cache: RefCell::new(None),
            all_methods_cache_epoch: Cell::new(0),
            all_native_methods_cache: RefCell::new(None),
            primitive: None,
            vm_methods: Rc::new(RefCell::new(HashMap::default())),
            vm_static_methods: Rc::new(RefCell::new(HashMap::default())),
            vm_plain_static_methods: PlainStaticCache::default(),
            model_subclass_memo: Cell::new(None),
            value_object_memo: Cell::new(None),
            field_init: RefCell::new(None),
            vm_constructor: RefCell::new(None),
            intercepts_new_memo: InterceptsNewMemo::default(),
            declared_fields: Rc::new(RefCell::new(Vec::new())),
            is_module: false,
            included_modules: Rc::new(RefCell::new(Vec::new())),
            mixin_static_methods: Rc::new(RefCell::new(HashMap::new())),
            included_hook_stmts: Rc::new(RefCell::new(Vec::new())),
            extended_hook_stmts: Rc::new(RefCell::new(Vec::new())),
            concern_static_methods: Rc::new(RefCell::new(HashMap::new())),
            concern_method_names: Rc::new(RefCell::new(Vec::new())),
            private_members: Rc::new(RefCell::new(HashSet::new())),
            protected_members: Rc::new(RefCell::new(HashSet::new())),
            restricted_names_memo: RefCell::default(),
        }
    }

    /// Find a bytecode instance method in this class or its superclass chain.
    pub fn find_vm_method(&self, name: &str) -> Option<Rc<VmClosure>> {
        if let Some(closure) = self.vm_methods.borrow().get(name) {
            return Some(closure.clone());
        }
        self.superclass
            .as_ref()
            .and_then(|superclass| superclass.find_vm_method(name))
    }

    /// The restriction on instance member `name`, and the name of the class
    /// that defines it: `None` for a public one. The nearest class in the
    /// chain that defines the member decides, so a subclass that redefines it
    /// without a modifier makes it public again. Answers `None` without a
    /// lookup until some class declares a private or protected member, so an
    /// application with none pays one relaxed load; after that, a name no
    /// class of the chain restricts — a field read, or any member of a class
    /// with no restricted ancestor — costs one probe of the chain's memo.
    pub fn restricted_method(&self, name: &str) -> Option<(MethodAccess, &str)> {
        if !restricted_methods_declared() {
            return None;
        }
        #[cfg(test)]
        tally_restricted_lookup(false);
        if !self.chain_restricts(name) {
            return None;
        }
        #[cfg(test)]
        tally_restricted_lookup(true);
        let mut cursor: Option<&Class> = Some(self);
        while let Some(class) = cursor {
            if class.private_members.borrow().contains(name) {
                return Some((MethodAccess::Private, class.name.as_str()));
            }
            if class.protected_members.borrow().contains(name) {
                return Some((MethodAccess::Protected, class.name.as_str()));
            }
            if class.methods.borrow().contains_key(name)
                || class.vm_methods.borrow().contains_key(name)
                || class.fields.contains_key(name)
            {
                return None;
            }
            cursor = class.superclass.as_deref();
        }
        None
    }

    /// Does some class of this chain declare `name` private or protected?
    /// Only then can [`Self::restricted_method`]'s walk find a restriction,
    /// so every other name skips it. Which members a class *defines* — what
    /// `include` and `define_method` change — is left to the walk: the memo
    /// holds only the declared names, collected once per
    /// `RESTRICTED_MEMBERS_EPOCH`. The chain itself is fixed per `Class`.
    fn chain_restricts(&self, name: &str) -> bool {
        let epoch = RESTRICTED_MEMBERS_EPOCH.load(std::sync::atomic::Ordering::Relaxed);
        {
            let memo = self.restricted_names_memo.borrow();
            if memo.epoch == epoch {
                return memo
                    .names
                    .as_ref()
                    .is_some_and(|names| names.contains(name));
            }
        }
        let mut names: Option<HashSet<String, AHasher>> = None;
        let mut cursor: Option<&Class> = Some(self);
        while let Some(class) = cursor {
            for declared in [&class.private_members, &class.protected_members] {
                let declared = declared.borrow();
                if !declared.is_empty() {
                    names
                        .get_or_insert_with(HashSet::default)
                        .extend(declared.iter().cloned());
                }
            }
            cursor = class.superclass.as_deref();
        }
        let restricts = names.as_ref().is_some_and(|names| names.contains(name));
        *self.restricted_names_memo.borrow_mut() = RestrictedNames { epoch, names };
        restricts
    }

    /// Is this class `name`, or does it inherit from it?
    pub fn is_or_inherits(&self, name: &str) -> bool {
        let mut cursor: Option<&Class> = Some(self);
        while let Some(class) = cursor {
            if class.name == name {
                return true;
            }
            cursor = class.superclass.as_deref();
        }
        false
    }

    /// Like `find_vm_method`, but also returns the class that defines the
    /// method — the VM stores it on the call frame so `super` inside the
    /// method resolves against the *defining* class's superclass (not the
    /// instance's class, which would loop on multi-level hierarchies).
    pub fn find_vm_method_with_class(
        self: &Rc<Self>,
        name: &str,
    ) -> Option<(Rc<VmClosure>, Rc<Class>)> {
        let mut current = self.clone();
        loop {
            let found = current.vm_methods.borrow().get(name).cloned();
            if let Some(closure) = found {
                return Some((closure, current));
            }
            let next = current.superclass.clone()?;
            current = next;
        }
    }

    /// Register bytecode instance method `name`, replacing any of that name,
    /// and stale the VM's method cache ([`note_vm_methods_changed`]).
    pub fn insert_vm_method(&self, name: String, method: Rc<VmClosure>) {
        self.vm_methods.borrow_mut().insert(name, method);
        note_vm_methods_changed();
    }

    /// The nearest definition of instance method `name`, and the class that
    /// declares it: each class from this one up is asked for its compiled
    /// method, then for its tree-walker one, before its superclass is.
    ///
    /// `find_vm_method_with_class` alone looks at compiled methods only, so
    /// once an ancestor's method had been compiled (the VM caches a compiled
    /// tree-walker method on the class that declares it) it beat a subclass
    /// override that had not been compiled yet: `AdminPolicy#show?` was
    /// skipped for `ArticlePolicy#show?` on a worker that had served the
    /// parent first. Dispatch must resolve through this instead.
    pub fn find_instance_method_level(
        self: &Rc<Self>,
        name: &str,
    ) -> Option<(InstanceMethodDef, Rc<Class>)> {
        let mut current = self.clone();
        loop {
            let compiled = current.vm_methods.borrow().get(name).cloned();
            if let Some(closure) = compiled {
                return Some((InstanceMethodDef::Compiled(closure), current));
            }
            let source = current.methods.borrow().get(name).cloned();
            if let Some(method) = source {
                return Some((InstanceMethodDef::Source(method), current));
            }
            let next = current.superclass.clone()?;
            current = next;
        }
    }

    /// The nearest definition of static method `name`, and the class that
    /// declares it: each class from this one up is asked for its compiled
    /// static, then for its tree-walker one (its own, then a mixin's), before
    /// its superclass is.
    ///
    /// `find_vm_static_method` alone looks at compiled statics only, and the
    /// VM caches a compiled tree-walker static on the class that declares it,
    /// so once `Parent.m(...)` had run, `Child.m(...)` ran Parent's body over
    /// Child's override. Dispatch must resolve through this instead.
    pub fn find_static_method_level(
        self: &Rc<Self>,
        name: &str,
    ) -> Option<(InstanceMethodDef, Rc<Class>)> {
        let mut current = self.clone();
        loop {
            let compiled = current.vm_static_methods.borrow().get(name).cloned();
            if let Some(closure) = compiled {
                return Some((InstanceMethodDef::Compiled(closure), current));
            }
            let source = current
                .static_methods
                .get(name)
                .cloned()
                .or_else(|| current.mixin_static_methods.borrow().get(name).cloned());
            if let Some(method) = source {
                return Some((InstanceMethodDef::Source(method), current));
            }
            let next = current.superclass.clone()?;
            current = next;
        }
    }

    /// Find a bytecode static method in this class or its superclass chain.
    /// It answers whether one exists; to pick the one a call runs, use
    /// `find_static_method_level`, which a nearer tree-walker override beats.
    pub fn find_vm_static_method(&self, name: &str) -> Option<Rc<VmClosure>> {
        if let Some(closure) = self.vm_static_methods.borrow().get(name) {
            return Some(closure.clone());
        }
        self.superclass
            .as_ref()
            .and_then(|superclass| superclass.find_vm_static_method(name))
    }

    /// Find a constructor in this class or its superclass chain.
    pub fn find_constructor(&self) -> Option<Rc<Function>> {
        if let Some(ref ctor) = self.constructor {
            return Some(ctor.clone());
        }
        if let Some(ref superclass) = self.superclass {
            return superclass.find_constructor();
        }
        None
    }

    /// [`Self::find_constructor`], with the class that declares the
    /// constructor. The VM runs a tree-walker constructor with that class on
    /// its frame, so the `super(...)` inside it reaches *that* class's
    /// superclass — the one the tree-walker binds as `__defining_superclass__`
    /// where the class is declared — and not the instance's, or the
    /// subclass's the search started from.
    pub fn find_constructor_with_class(self: &Rc<Self>) -> Option<(Rc<Function>, Rc<Class>)> {
        let mut current = self.clone();
        loop {
            if let Some(ctor) = current.constructor.clone() {
                return Some((ctor, current));
            }
            let next = current.superclass.clone()?;
            current = next;
        }
    }

    /// The nearest tree-walker method `name` up the chain, with the class that
    /// declares it: [`Self::find_vm_method_with_class`] for a method the VM
    /// has not compiled. The VM records that class on the method's frame, so
    /// a `super` inside the method resolves against its superclass.
    pub fn find_method_with_class(
        self: &Rc<Self>,
        name: &str,
    ) -> Option<(Rc<Function>, Rc<Class>)> {
        let mut current = self.clone();
        loop {
            let found = current.methods.borrow().get(name).cloned();
            if let Some(method) = found {
                return Some((method, current));
            }
            let next = current.superclass.clone()?;
            current = next;
        }
    }

    /// The class in this one's chain that declares `method` — the same
    /// function, not one of the same name — or `None` when none of them does,
    /// as for a controller hook's lambda.
    pub fn declaring_class_of(self: &Rc<Self>, method: &Function) -> Option<Rc<Class>> {
        let mut current = self.clone();
        loop {
            let declares = current
                .methods
                .borrow()
                .get(method.name.as_str())
                .is_some_and(|own| std::ptr::eq(Rc::as_ptr(own), method));
            if declares {
                return Some(current);
            }
            let next = current.superclass.clone()?;
            current = next;
        }
    }

    /// Build the flattened method cache if not already built, or rebuild it
    /// when a method was defined on any class since it was built (see
    /// [`bump_method_epoch`]): the map holds the ancestors' methods too, so a
    /// change to a parent leaves every subclass's copy stale.
    fn ensure_methods_cached(&self) {
        let epoch = method_epoch();
        // Fast path: check if already cached without borrowing mutably
        if self.all_methods_cache_epoch.get() == epoch && self.all_methods_cache.borrow().is_some()
        {
            return;
        }

        // Build flattened method map
        let mut all_methods = HashMap::default();

        // First, get methods from superclass (if any)
        if let Some(ref superclass) = self.superclass {
            superclass.ensure_methods_cached();
            if let Some(ref parent_cache) = *superclass.all_methods_cache.borrow() {
                all_methods.extend(parent_cache.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }

        // Then, override with methods from this class
        for (k, v) in self.methods.borrow().iter() {
            all_methods.insert(k.clone(), v.clone());
        }

        // Store in cache
        *self.all_methods_cache.borrow_mut() = Some(all_methods);
        self.all_methods_cache_epoch.set(epoch);
    }

    /// Build the flattened native method cache if not already built.
    fn ensure_native_methods_cached(&self) {
        // Fast path: check if already cached without borrowing mutably
        if self.all_native_methods_cache.borrow().is_some() {
            return;
        }

        // Build flattened native method map
        let mut all_native_methods = HashMap::default();

        // First, get native methods from superclass (if any)
        if let Some(ref superclass) = self.superclass {
            superclass.ensure_native_methods_cached();
            if let Some(ref parent_cache) = *superclass.all_native_methods_cache.borrow() {
                all_native_methods.extend(parent_cache.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }

        // Then, override with native methods from this class
        all_native_methods.extend(self.native_methods.clone());

        // Store in cache
        *self.all_native_methods_cache.borrow_mut() = Some(all_native_methods);
    }

    pub fn find_method(&self, name: &str) -> Option<Rc<Function>> {
        // Ensure cache is built, then do O(1) lookup
        self.ensure_methods_cached();
        self.all_methods_cache
            .borrow()
            .as_ref()
            .and_then(|cache| cache.get(name).cloned())
    }

    pub fn find_native_method(&self, name: &str) -> Option<Rc<NativeFunction>> {
        // Ensure cache is built, then do O(1) lookup
        self.ensure_native_methods_cached();
        self.all_native_methods_cache
            .borrow()
            .as_ref()
            .and_then(|cache| cache.get(name).cloned())
    }

    /// [`Self::find_native_method`], unless a method written in Soli overrides
    /// it: `def delete(force)` in `class Post < Model` is what `post.delete`
    /// means, not the `Model#delete` native it inherits. The nearer
    /// declaration wins, as for any inherited method.
    pub fn find_native_method_unshadowed(&self, name: &str) -> Option<Rc<NativeFunction>> {
        let native = self.find_native_method(name)?;
        if self.user_method_overrides_native(name) {
            return None;
        }
        Some(native)
    }

    /// Does a method written in Soli (tree-walker or bytecode) come before the
    /// native `name` on the way up from this class? The walk only runs when
    /// both exist, so a class with no such override pays two cached lookups.
    pub fn user_method_overrides_native(&self, name: &str) -> bool {
        if self.find_method(name).is_none() && self.find_vm_method(name).is_none() {
            return false;
        }
        let mut class = Some(self);
        while let Some(current) = class {
            if current.methods.borrow().contains_key(name)
                || current.vm_methods.borrow().contains_key(name)
            {
                return true;
            }
            if current.native_methods.contains_key(name) {
                return false;
            }
            class = current.superclass.as_deref();
        }
        false
    }

    /// Find a static method in this class or its superclass chain.
    pub fn find_static_method(&self, name: &str) -> Option<Rc<Function>> {
        if let Some(method) = self.static_methods.get(name) {
            return Some(method.clone());
        }
        if let Some(method) = self.mixin_static_methods.borrow().get(name) {
            return Some(method.clone());
        }
        if let Some(ref superclass) = self.superclass {
            return superclass.find_static_method(name);
        }
        None
    }

    /// Call after changing `methods` (or `vm_methods`) of a class that may
    /// already be in use. Drops this class's flattened caches and bumps the
    /// global [`method_epoch`], so every subclass rebuilds its own copy on its
    /// next lookup instead of keeping the parent's old method set.
    ///
    /// The VM's per-site method cache answers the nearest definition, which
    /// a new tree-walker method can change too: it is staled as well
    /// ([`note_vm_methods_changed`]).
    pub fn invalidate_method_cache(&self) {
        *self.all_methods_cache.borrow_mut() = None;
        *self.all_native_methods_cache.borrow_mut() = None;
        bump_method_epoch();
        note_vm_methods_changed();
    }

    /// Mix `module`'s instance methods into this class (Ruby `include`).
    /// Existing methods on `self` win. Transitive includes are applied first.
    /// Returns `true` when the module was newly mixed in (`false` if already included).
    pub fn include_module(&self, module: &Rc<Class>) -> Result<bool, String> {
        if !module.is_module {
            return Err(format!(
                "can only include a module (got class {})",
                module.name
            ));
        }
        if std::ptr::eq(self as *const Class, Rc::as_ptr(module)) {
            return Err("a module cannot include itself".to_string());
        }
        {
            let already = self.included_modules.borrow();
            if already
                .iter()
                .any(|m| std::ptr::eq(Rc::as_ptr(m), Rc::as_ptr(module)))
            {
                return Ok(false);
            }
        }
        // The module's OWN methods are copied first, then its transitive
        // includes. Every copy is `or_insert`, so whoever lands first wins —
        // recursing before copying gave a nested module precedence over the
        // module that includes it (`module Named { include Base; def label(){…} }`
        // resolved `label` to Base's).
        {
            let mut mine = self.methods.borrow_mut();
            for (name, method) in module.methods.borrow().iter() {
                mine.entry(name.clone()).or_insert_with(|| method.clone());
            }
        }
        {
            let mut mine = self.vm_methods.borrow_mut();
            for (name, method) in module.vm_methods.borrow().iter() {
                mine.entry(name.clone()).or_insert_with(|| method.clone());
            }
        }
        note_vm_methods_changed();
        {
            let mut mine = self.mixin_static_methods.borrow_mut();
            for (name, method) in module.concern_static_methods.borrow().iter() {
                mine.entry(name.clone()).or_insert_with(|| method.clone());
            }
        }
        {
            let names = module.concern_method_names.borrow();
            if !names.is_empty() {
                let mut mine = self.vm_static_methods.borrow_mut();
                let src = module.vm_static_methods.borrow();
                for name in names.iter() {
                    if let Some(method) = src.get(name) {
                        mine.entry(name.clone()).or_insert_with(|| method.clone());
                    }
                }
            }
        }
        // Transitive includes, after the module's own methods (see above).
        for inner in module.included_modules.borrow().iter() {
            self.include_module(inner)?;
        }
        self.included_modules.borrow_mut().push(module.clone());
        self.invalidate_method_cache();
        // Its `class_methods`, if any, joined this class's statics.
        note_static_methods_changed();
        Ok(true)
    }

    /// Modules newly added to `self` by including `module` — the module itself
    /// plus any of its transitive includes that were not already mixed in,
    /// innermost first.
    ///
    /// Callers need this to fire concern hooks: `included do` has to run against
    /// the host class for every module that joins it, not just the one named in
    /// the `include`. Firing only for the named module left a nested concern's
    /// hook registered against the intermediate *module*, so it never ran.
    pub fn include_module_collecting(&self, module: &Rc<Class>) -> Result<Vec<Rc<Class>>, String> {
        let before: Vec<*const Class> = self
            .included_modules
            .borrow()
            .iter()
            .map(Rc::as_ptr)
            .collect();
        if !self.include_module(module)? {
            return Ok(Vec::new());
        }
        // `include_module` pushes a module's transitive includes before the
        // module itself, so this order is already innermost-first.
        Ok(self
            .included_modules
            .borrow()
            .iter()
            .filter(|m| !before.contains(&Rc::as_ptr(m)))
            .cloned()
            .collect())
    }

    /// Mix `module`'s instance methods in as class methods (Ruby `extend`).
    pub fn extend_module(&self, module: &Rc<Class>) -> Result<(), String> {
        if !module.is_module {
            return Err(format!(
                "can only extend a module (got class {})",
                module.name
            ));
        }
        {
            let mut mine = self.mixin_static_methods.borrow_mut();
            for (name, method) in module.methods.borrow().iter() {
                mine.entry(name.clone()).or_insert_with(|| method.clone());
            }
        }
        {
            let mut mine = self.vm_static_methods.borrow_mut();
            for (name, method) in module.vm_methods.borrow().iter() {
                mine.entry(name.clone()).or_insert_with(|| method.clone());
            }
        }
        note_static_methods_changed();
        Ok(())
    }

    /// Find a native static method in this class or its superclass chain.
    pub fn find_native_static_method(&self, name: &str) -> Option<Rc<NativeFunction>> {
        if let Some(method) = self.native_static_methods.get(name) {
            return Some(method.clone());
        }
        if let Some(ref superclass) = self.superclass {
            return superclass.find_native_static_method(name);
        }
        None
    }

    /// Check if this class is a subclass of Model (directly or indirectly).
    /// Memoized — the superclass chain is fixed at construction, and this is
    /// called several times per instance member access.
    pub fn is_model_subclass(&self) -> bool {
        if let Some(cached) = self.model_subclass_memo.get() {
            return cached;
        }
        let result = if self.name == "Model" {
            true
        } else {
            self.superclass
                .as_ref()
                .is_some_and(|superclass| superclass.is_model_subclass())
        };
        self.model_subclass_memo.set(Some(result));
        result
    }

    /// Whether this class is `ValueObject` or descends from it: compared by
    /// value, frozen once constructed (see `interpreter::domain`).
    pub fn is_value_object(&self) -> bool {
        if let Some(cached) = self.value_object_memo.get() {
            return cached;
        }
        let result = if self.superclass.is_none() {
            self.name == "ValueObject"
        } else {
            self.superclass
                .as_ref()
                .is_some_and(|superclass| superclass.is_value_object())
        };
        self.value_object_memo.set(Some(result));
        result
    }

    /// Whether `Klass.new(...)` on this class is answered by the engines'
    /// call interceptors: not a model (its `new` is the ORM's), not a module,
    /// and no class of the chain has a static `new` of its own — written in
    /// Soli, compiled by the VM, or native. Memoized per class until a static
    /// method joins any class ([`note_static_methods_changed`]): it was three
    /// walks of the chain on every `Klass.new`, two SipHash probes a level.
    pub fn intercepts_new(&self) -> bool {
        let epoch = STATIC_METHODS_EPOCH.load(std::sync::atomic::Ordering::Relaxed);
        let (computed_in, answer) = self.intercepts_new_memo.0.get();
        if computed_in == epoch {
            return answer;
        }
        let answer = !self.is_model_subclass()
            && !self.is_module
            && self.find_static_method("new").is_none()
            && self.find_vm_static_method("new").is_none()
            && !self.has_native_static_new();
        self.intercepts_new_memo.0.set((epoch, answer));
        answer
    }

    fn has_native_static_new(&self) -> bool {
        self.native_static_methods.contains_key("new")
            || self
                .superclass
                .as_ref()
                .is_some_and(|superclass| superclass.has_native_static_new())
    }

    /// How many instance fields a new instance starts with, at least: the
    /// ones this class and its ancestors declare, which both engines set
    /// before any constructor runs. A class the tree-walker defined lists its
    /// own in `fields` (and `declared_fields`), one the VM compiled in
    /// `declared_fields` only. [`Instance::new`] sizes its field map with it,
    /// so starting them never regrows the map.
    pub fn declared_field_count(&self) -> usize {
        let own = self.fields.len().max(self.declared_fields.borrow().len());
        own + self
            .superclass
            .as_deref()
            .map_or(0, Class::declared_field_count)
    }

    /// Every declared instance field, inherited ones first.
    pub fn all_declared_fields(&self) -> Vec<String> {
        let mut out = match &self.superclass {
            Some(superclass) => superclass.all_declared_fields(),
            None => Vec::new(),
        };
        for name in self.declared_fields.borrow().iter() {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }
}

/// A class instance.
///
/// `fields` uses ahash (like globals and hash literals) — instance field
/// probes happen on every member access, and the std default SipHash was
/// measurably slower on that hot path.
#[derive(Debug, Clone)]
pub struct Instance {
    pub class: Rc<Class>,
    pub fields: HashMap<SoliStr, Value, AHasher>,
    /// Dirty-tracking baseline: the non-`_` fields as last loaded from or
    /// persisted to the database. `None` = new (never-loaded) record.
    pub original_fields: Option<Box<HashMap<SoliStr, Value, AHasher>>>,
    /// Changes applied by the last successful create/save/update, as
    /// `(name, old, new)` sorted by name. `None` = never persisted.
    pub previous_changes: Option<Box<Vec<(SoliStr, Value, Value)>>>,
}

impl Instance {
    pub fn new(class: Rc<Class>) -> Self {
        Self::with_capacity(class, 0)
    }

    /// An instance whose field map has room for `fields` entries, and at
    /// least for every field its class and their ancestors declare
    /// ([`Class::declared_field_count`]) — all of which a construction sets
    /// before any constructor runs — so filling them in does not rehash
    /// partway through. A model row passes its column count. A field a
    /// constructor adds without declaring it can still grow the map, the rare
    /// case. Sized from the class's own tree-walker `fields` alone, a class
    /// the VM compiled started at 0 and regrew at its 4th and 8th field, and
    /// inherited fields were never counted.
    pub fn with_capacity(class: Rc<Class>, fields: usize) -> Self {
        let capacity = fields.max(class.declared_field_count());
        Self {
            class,
            fields: HashMap::with_capacity_and_hasher(capacity, map_hasher()),
            original_fields: None,
            previous_changes: None,
        }
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.fields.get(name).cloned()
    }

    pub fn set(&mut self, name: impl Into<SoliStr>, value: Value) {
        self.fields.insert(name.into(), value);
    }

    pub fn get_method(&self, name: &str) -> Option<Value> {
        // Check instance fields first
        if let Some(value) = self.fields.get(name) {
            return Some(value.clone());
        }
        // Then check class native methods, unless Soli overrides them
        if let Some(native) = self.class.find_native_method_unshadowed(name) {
            return Some(Value::NativeFunction((*native).clone()));
        }
        // Then check class methods - convert Rc<Function> to Value::Function
        if let Some(func) = self.class.methods.borrow().get(name) {
            return Some(Value::Function(func.clone()));
        }
        None
    }
}

/// Convert raw HTTP response data to a Value based on the future kind.
/// For FullResponse, the raw_data is a JSON-encoded response object.
fn convert_future_result(raw_data: &str, kind: &HttpFutureKind) -> Result<Value, String> {
    match kind {
        HttpFutureKind::String => Ok(Value::String(raw_data.to_string().into())),
        HttpFutureKind::Json => {
            // One-pass parse into Value — no intermediate serde_json tree.
            parse_json(raw_data).map_err(|e| format!("Failed to parse JSON: {}", e))
        }
        HttpFutureKind::Jsonp => {
            // Strip the callback padding, then parse the inner JSON.
            let inner = crate::interpreter::jsonp::strip_jsonp_padding(raw_data)?;
            parse_json(inner).map_err(|e| format!("Failed to parse JSONP: {}", e))
        }
        HttpFutureKind::FullResponse => {
            // Parse the JSON-encoded full response
            parse_json(raw_data).map_err(|e| format!("Failed to parse response: {}", e))
        }
        HttpFutureKind::SystemResult => {
            // Parse JSON: {"stdout": "...", "stderr": "...", "exit_code": N}
            #[derive(serde::Deserialize)]
            struct SystemResultJson {
                stdout: String,
                stderr: String,
                exit_code: i32,
            }
            match serde_json::from_str::<SystemResultJson>(raw_data) {
                Ok(data) => {
                    // Create a simple hash with the result data using IndexMap
                    let mut hash: HashPairs = HashPairs::default();
                    hash.insert(
                        HashKey::String("stdout".into()),
                        Value::String(data.stdout.into()),
                    );
                    hash.insert(
                        HashKey::String("stderr".into()),
                        Value::String(data.stderr.into()),
                    );
                    hash.insert(
                        HashKey::String("exit_code".into()),
                        Value::Int(data.exit_code as i64),
                    );
                    Ok(Value::Hash(Rc::new(RefCell::new(hash))))
                }
                Err(e) => Err(format!("Failed to parse SystemResult: {}", e)),
            }
        }
    }
}

pub use crate::interpreter::value_json::{json_to_value, json_to_value_ref, value_to_json};

/// SEC-013: which fields of a `Value::Instance` are safe to include
/// when the runtime serialises an instance via `render_json` /
/// `to_json`. Anything matching a common-sensitive name pattern is
/// skipped; framework-internal `_`-prefixed fields are skipped too,
/// except for the small set of Model metadata fields that
/// applications routinely expose (`_key`, `_id`, `_rev`,
/// `_created_at`, `_updated_at`).
///
/// This is the default; SEC-013a will add an explicit `to_json`
/// override hook so apps can customise their serialisation shape
/// without working around the filter.
///
/// Runs once per field of every serialised instance, so it allocates nothing:
/// the `_` test comes first (one byte), and only `_` names are compared with
/// the metadata list.
pub(crate) fn is_safe_serialised_field(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix('_') {
        // Always-exposed Model metadata; every other `_`-prefixed field is a
        // framework internal (`_errors`, `_text`, `_pending_translations`, …).
        return matches!(rest, "key" | "id" | "rev" | "created_at" | "updated_at");
    }
    !is_sensitive_field_name(name)
}

/// The fields of a plain instance that its JSON carries (see
/// [`is_safe_serialised_field`]), sorted by name.
///
/// `Instance::fields` is a hash map with a random seed per instance, so its
/// iteration order differs from one object to the next and from one request
/// to the next. Sorting gives every writer one stable order: the one
/// `value_to_json` already produced through `serde_json::Map` (a `BTreeMap`,
/// byte order), so `render_json`, `JSON.stringify`, serde and `value_to_json`
/// agree byte for byte, and identical data gives identical bytes (ETags,
/// cache keys, snapshot tests).
pub(crate) fn serialised_instance_fields(instance: &Instance) -> Vec<(&SoliStr, &Value)> {
    let mut fields: Vec<(&SoliStr, &Value)> = instance
        .fields
        .iter()
        .filter(|(name, _)| is_safe_serialised_field(name))
        .collect();
    fields.sort_unstable_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    fields
}

/// Does this *model field* name look like it holds a secret?
///
/// Deliberately narrower than `crate::redaction::looks_sensitive`, and NOT a
/// drifted copy of it — the two answer different questions and should stay
/// apart:
///
/// * This one decides what a model omits from its serialised output, which is
///   API shape. Over-hiding here silently removes a field an app depends on,
///   so it matches precise prefixes and suffixes: `password…`, `…_token`,
///   `…_digest`, `…_secret`, `…_hash`.
/// * `redaction::looks_sensitive` decides what never reaches a log, and matches
///   substrings. Over-redacting there costs a little debugging context, which
///   is the cheaper mistake.
///
/// So a field called `author` is serialised normally and still redacted in an
/// error log. That asymmetry is the point.
///
/// ASCII-case-insensitive on the prefix/suffix, so `PasswordHash` is caught
/// as well as `password_hash`. The bytes are compared in place with
/// `eq_ignore_ascii_case` rather than through a lowercased copy: this runs for
/// every field of every serialised instance, and the copy was a heap
/// allocation per field. Non-ASCII bytes compare unchanged either way.
pub(crate) fn is_sensitive_field_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let starts_with = |prefix: &[u8]| {
        bytes
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    let ends_with = |suffix: &[u8]| {
        bytes.len() >= suffix.len()
            && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    };
    // Catches `password`, `password_digest`, `password_hash`,
    // `password_reset_token`, `passwordHash`, etc.
    starts_with(b"password")
        || ends_with(b"_token")
        || ends_with(b"_digest")
        || ends_with(b"_secret")
        || ends_with(b"_hash")
}

thread_local! {
    /// Nesting depth of in-flight `Serialize for Value` recursions. Runtime
    /// code can build arbitrarily deep structures (`loop { a = [a] }`); the
    /// recursive serde walk would otherwise overflow the native stack — which
    /// aborts without unwinding.
    static SERIALIZE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Maximum Value nesting depth during serialization.
const MAX_SERIALIZE_DEPTH: usize = 512;

impl serde::Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Leaves (ints, strings, bools) cannot recurse. The depth counter is a
        // thread-local read and write, and a JSON array of small objects pays
        // it once per field. Only containers can nest.
        if !matches!(
            self,
            Value::Array(_) | Value::Hash(_) | Value::Instance(_) | Value::Deferred(_)
        ) {
            return self.serialize_inner(serializer);
        }
        let depth = SERIALIZE_DEPTH.with(|d| {
            d.set(d.get() + 1);
            d.get()
        });
        if depth > MAX_SERIALIZE_DEPTH {
            SERIALIZE_DEPTH.with(|d| d.set(depth - 1));
            return Err(serde::ser::Error::custom(
                "structure nested too deeply to serialize",
            ));
        }
        let result = self.serialize_inner(serializer);
        SERIALIZE_DEPTH.with(|d| d.set(d.get() - 1));
        result
    }
}

impl Value {
    fn serialize_inner<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            // RFC 3339, matching `to_iso()`. As an `Instance` its only field was
            // `_ts`, which the `_`-prefix filter below strips — so a DateTime
            // serialised as `{}` and every timestamp in an API response was an
            // empty object.
            Value::DateTime(ts, _) => serializer.serialize_str(&Value::datetime_to_rfc3339(*ts)),
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Int(n) => serializer.serialize_i64(*n),
            Value::Float(f) => serializer.serialize_f64(*f),
            Value::Decimal(d) => serializer.serialize_str(&d.to_string()),
            Value::String(s) => serializer.serialize_str(s),
            Value::Symbol(s) => serializer.serialize_str(s),
            Value::Array(arr) => {
                let borrow = arr.borrow();
                let mut seq = serializer.serialize_seq(Some(borrow.len()))?;
                for v in borrow.iter() {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            Value::Hash(hash) => {
                let borrow = hash.borrow();
                let mut map = serializer.serialize_map(Some(borrow.len()))?;
                for (k, v) in borrow.iter() {
                    match k {
                        HashKey::String(key) | HashKey::Symbol(key) => {
                            map.serialize_entry(key, v)?;
                        }
                        _ => {}
                    }
                }
                map.end()
            }
            Value::Instance(inst) => {
                let borrow = inst.borrow();
                // Enum value → a DB/JSON-friendly shape: a bare tag string for
                // a unit variant, or { "variant": tag, ...payload }. Round-trips
                // via the model `enum_field` DSL and `Enum.from(value)`.
                if let Some(tag) = enum_variant_tag(&borrow) {
                    use serde::ser::SerializeMap;
                    let payload = borrow
                        .fields
                        .iter()
                        .filter(|(k, _)| k.as_str() != "__variant");
                    if borrow.fields.len() == 1 {
                        return serializer.serialize_str(tag);
                    }
                    let mut map = serializer.serialize_map(None)?;
                    map.serialize_entry("variant", tag)?;
                    for (k, v) in payload {
                        map.serialize_entry(k, v)?;
                    }
                    return map.end();
                }
                // A value object: its fields, in order, all of them (see
                // `value_to_json`).
                if crate::interpreter::domain::value_objects_declared()
                    && borrow.class.is_value_object()
                {
                    use serde::ser::SerializeMap;
                    let fields = crate::interpreter::domain::value_fields(&borrow);
                    let mut map = serializer.serialize_map(Some(fields.len()))?;
                    for (k, v) in &fields {
                        map.serialize_entry(k, v)?;
                    }
                    return map.end();
                }
                // SEC-013: a bare `render_json(user)` used to walk every
                // field of the class, leaking `password_hash`,
                // `reset_token`, etc. Default to skipping fields whose
                // names match common-sensitive patterns and most
                // `_`-prefixed framework internals; apps that need the
                // raw shape can serialise explicitly via a Hash literal.
                let fields = serialised_instance_fields(&borrow);
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (k, v) in fields {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            // Resolve a `grouped {}` deferred before writing so JSON.stringify
            // of response assigns that still hold deferred reads does not fail.
            Value::Deferred(cell) => {
                let resolved = crate::interpreter::builtins::model::batch::force(cell)
                    .map_err(serde::ser::Error::custom)?;
                serde::Serialize::serialize(&resolved, serializer)
            }
            _ => Err(serde::ser::Error::custom(format!(
                "Cannot convert {} to JSON",
                self.type_name()
            ))),
        }
    }
}

/// Serialize a Value to a JSON string (see `value_stringify`).
#[inline]
pub fn stringify_to_string(value: &Value) -> Result<String, String> {
    crate::interpreter::value_stringify::stringify_to_string(value)
}

/// Serialize an array slice to JSON without cloning into a Value.
#[inline]
pub fn stringify_array_to_string(items: &[Value]) -> Result<String, String> {
    crate::interpreter::value_stringify::stringify_array_to_string(items)
}

/// Serialize hash entries to JSON without cloning into a Value.
#[inline]
pub fn stringify_hash_entries_to_string(entries: &[(HashKey, Value)]) -> Result<String, String> {
    crate::interpreter::value_stringify::stringify_hash_entries_to_string(entries)
}

/// Fixed-seed hasher for JSON object maps.
///
/// `RandomState::default()` draws entropy (or a thread-local random counter)
/// on every construction. A JSON payload with N objects used to pay that cost
/// N times. Fixed seeds are fine here: the keys are untrusted data, not a
/// HashDoS target with a long-lived global map — these maps are per-document
/// and short-lived.
#[inline(always)]
fn json_hasher() -> AHasher {
    AHasher::with_seeds(
        0x0123_4567_89AB_CDEF,
        0xFEDC_BA98_7654_3210,
        0x0F1E_2D3C_4B5A_6978,
        0x8877_6655_4433_2211,
    )
}

/// Fast i64 parse of a digit slice (optional leading `-` already stripped from
/// `digits`). `neg` applies the sign. Digit counts ≤ 18 always fit in i64
/// (10^18 − 1 < 2^63), so we skip checked arithmetic on the common path.
#[inline(always)]
fn fast_parse_i64_digits(digits: &[u8], neg: bool) -> Value {
    if digits.len() <= 18 {
        let mut n: i64 = 0;
        for &d in digits {
            n = n * 10 + (d - b'0') as i64;
        }
        return Value::Int(if neg { -n } else { n });
    }
    // 19+ digits: checked path (and overflow → f64).
    let mut n: i64 = 0;
    for &d in digits {
        let d = (d - b'0') as i64;
        match n.checked_mul(10).and_then(|n| n.checked_add(d)) {
            Some(v) => n = v,
            None => {
                let s = unsafe { std::str::from_utf8_unchecked(digits) };
                let signed = if neg {
                    format!("-{}", s)
                } else {
                    s.to_string()
                };
                return Value::Float(signed.parse::<f64>().unwrap_or(0.0));
            }
        }
    }
    // i64::MIN is the only negative that needs the positive magnitude to
    // overflow; handle via checked_neg.
    if neg {
        match n.checked_neg() {
            Some(v) => Value::Int(v),
            None => Value::Float(-(n as f64)),
        }
    } else {
        Value::Int(n)
    }
}

/// Parse a JSON string into a Soli [`Value`].
///
/// One-pass hand-rolled parser (no intermediate serde tree). sonic-rs direct
/// `Deserialize` was measured and did not beat this path once Values are
/// materialised (IndexMap + Rc/RefCell dominate either way).
#[inline]
pub fn parse_json(s: &str) -> Result<Value, String> {
    parse_json_bytes(s.as_bytes())
}

/// Parse JSON from bytes.
pub fn parse_json_bytes(bytes: &[u8]) -> Result<Value, String> {
    let mut pos = 0;
    let value = parse_value(bytes, &mut pos, 0)?;
    skip_ws(bytes, &mut pos);
    if pos < bytes.len() {
        return Err(format!("Trailing content at position {}", pos));
    }
    Ok(value)
}

#[inline(always)]
fn skip_ws(b: &[u8], pos: &mut usize) {
    // Unrolled first-byte check: compact JSON (no whitespace) is the hot case.
    let p = *pos;
    if p >= b.len() {
        return;
    }
    match b[p] {
        b' ' | b'\t' | b'\n' | b'\r' => {
            *pos = p + 1;
            while *pos < b.len() {
                match b[*pos] {
                    b' ' | b'\t' | b'\n' | b'\r' => *pos += 1,
                    _ => break,
                }
            }
        }
        _ => {}
    }
}

/// Skip whitespace and return the next byte, or an unexpected-EOF error.
#[inline(always)]
fn peek(b: &[u8], pos: &mut usize) -> Result<u8, String> {
    skip_ws(b, pos);
    if *pos < b.len() {
        Ok(b[*pos])
    } else {
        Err("Unexpected end of JSON".to_string())
    }
}

/// Nesting cap for the hand-rolled parser.
///
/// `[[[[…]]]]` recurses one frame per level, and a native stack overflow
/// **aborts** the process without unwinding — so the per-request
/// `catch_unwind` never sees it and one hostile request takes down the worker.
/// A web payload has no legitimate reason to nest hundreds deep, so past this
/// the parse returns an ordinary catchable error.
///
/// The depth guards added for JSON earlier covered only the
/// `serde_json`↔`Value` conversion helpers in `value_json.rs`; `json_parse`
/// uses this parser, which had no guard at all. Kept equal to
/// `value_json::MAX_JSON_DEPTH` so parsing and serializing agree on what is
/// representable.
pub(crate) const MAX_JSON_PARSE_DEPTH: usize = 512;

#[inline(always)]
fn parse_value(b: &[u8], pos: &mut usize, depth: usize) -> Result<Value, String> {
    if depth > MAX_JSON_PARSE_DEPTH {
        return Err(format!(
            "JSON nested too deeply (over {MAX_JSON_PARSE_DEPTH} levels) at position {pos}",
            pos = *pos
        ));
    }
    match peek(b, pos)? {
        b'"' => parse_string(b, pos).map(Value::String),
        b'{' => parse_object(b, pos, depth),
        b'[' => parse_array(b, pos, depth),
        b't' => parse_literal(b, pos, b"true", Value::Bool(true)),
        b'f' => parse_literal(b, pos, b"false", Value::Bool(false)),
        b'n' => parse_literal(b, pos, b"null", Value::Null),
        b'-' | b'0'..=b'9' => parse_number(b, pos),
        c => Err(format!(
            "Unexpected character '{}' at position {}",
            c as char, *pos
        )),
    }
}

#[inline]
fn parse_literal(
    b: &[u8],
    pos: &mut usize,
    expected: &[u8],
    value: Value,
) -> Result<Value, String> {
    let end = *pos + expected.len();
    if end <= b.len() && &b[*pos..end] == expected {
        *pos = end;
        Ok(value)
    } else {
        Err(format!("Invalid literal at position {}", *pos))
    }
}

fn parse_number(b: &[u8], pos: &mut usize) -> Result<Value, String> {
    let start = *pos;
    let neg = *pos < b.len() && b[*pos] == b'-';
    if neg {
        *pos += 1;
    }
    if *pos >= b.len() || !b[*pos].is_ascii_digit() {
        return Err(format!("Invalid number at position {}", start));
    }

    // Single-pass integer accumulation for the common non-float path.
    // 10^18-1 fits in i64, so we accumulate unchecked until we know the
    // digit count, then only fall back if the number was huge.
    let mut n: u64 = 0;
    let mut digits: u32 = 0;
    if b[*pos] == b'0' {
        *pos += 1;
        digits = 1;
        // Leading zero may only stand alone (or start a float/exponent).
        if *pos < b.len() && b[*pos].is_ascii_digit() {
            return Err(format!("Invalid number at position {}", start));
        }
    } else {
        while *pos < b.len() {
            let c = b[*pos];
            if c.is_ascii_digit() {
                n = n.wrapping_mul(10).wrapping_add((c - b'0') as u64);
                digits += 1;
                *pos += 1;
            } else {
                break;
            }
        }
    }

    let is_float = *pos < b.len() && matches!(b[*pos], b'.' | b'e' | b'E');
    if !is_float {
        // Pure integer.
        if digits <= 18 {
            let signed = n as i64;
            return Ok(Value::Int(if neg { -signed } else { signed }));
        }
        // 19+ digits: re-parse carefully.
        return Ok(fast_parse_i64_digits(
            &b[start + usize::from(neg)..*pos],
            neg,
        ));
    }

    // Float / scientific — extend the span then use the std parser.
    if *pos < b.len() && b[*pos] == b'.' {
        *pos += 1;
        if *pos >= b.len() || !b[*pos].is_ascii_digit() {
            return Err(format!("Invalid number at position {}", start));
        }
        while *pos < b.len() && b[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }
    if *pos < b.len() && (b[*pos] == b'e' || b[*pos] == b'E') {
        *pos += 1;
        if *pos < b.len() && (b[*pos] == b'+' || b[*pos] == b'-') {
            *pos += 1;
        }
        if *pos >= b.len() || !b[*pos].is_ascii_digit() {
            return Err(format!("Invalid number at position {}", start));
        }
        while *pos < b.len() && b[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }

    let num_str = unsafe { std::str::from_utf8_unchecked(&b[start..*pos]) };
    num_str
        .parse::<f64>()
        .map(Value::Float)
        .map_err(|e| format!("Invalid float: {}", e))
}

#[inline(always)]
fn parse_string(b: &[u8], pos: &mut usize) -> Result<SoliStr, String> {
    *pos += 1; // skip opening '"'
    let start = *pos;
    let rest = &b[start..];

    // SIMD-accelerated scan for the first `"` or `\`.
    match memchr::memchr2(b'"', b'\\', rest) {
        Some(rel) if rest[rel] == b'"' => {
            // No escapes — SoliStr from the slice (short keys stay inline).
            let s = SoliStr::from(unsafe { std::str::from_utf8_unchecked(&rest[..rel]) });
            *pos = start + rel + 1;
            return Ok(s);
        }
        Some(rel) => {
            // Escape at `rel`; fall into the slow path from there.
            *pos = start + rel;
        }
        None => return Err("Unterminated string".to_string()),
    }

    // Slow path: build string with escape handling
    let mut result = String::from(unsafe { std::str::from_utf8_unchecked(&b[start..*pos]) });
    while *pos < b.len() {
        match b[*pos] {
            b'"' => {
                *pos += 1;
                return Ok(result.into());
            }
            b'\\' => {
                *pos += 1;
                if *pos >= b.len() {
                    return Err("Unterminated string escape".to_string());
                }
                match b[*pos] {
                    b'"' => result.push('"'),
                    b'\\' => result.push('\\'),
                    b'/' => result.push('/'),
                    b'n' => result.push('\n'),
                    b'r' => result.push('\r'),
                    b't' => result.push('\t'),
                    b'b' => result.push('\u{08}'),
                    b'f' => result.push('\u{0C}'),
                    b'u' => {
                        *pos += 1;
                        let cp = parse_hex4(b, pos)?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            // High surrogate — expect \uXXXX low surrogate
                            if *pos + 1 < b.len() && b[*pos] == b'\\' && b[*pos + 1] == b'u' {
                                *pos += 2;
                                let low = parse_hex4(b, pos)?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err("Invalid surrogate pair".to_string());
                                }
                                let cp =
                                    0x10000 + ((cp as u32 - 0xD800) << 10) + (low as u32 - 0xDC00);
                                result.push(char::from_u32(cp).ok_or("Invalid Unicode")?);
                            } else {
                                return Err("Missing low surrogate".to_string());
                            }
                        } else {
                            result.push(
                                char::from_u32(cp as u32).ok_or("Invalid Unicode codepoint")?,
                            );
                        }
                        continue; // parse_hex4 already advanced pos
                    }
                    c => return Err(format!("Invalid escape \\{}", c as char)),
                }
                *pos += 1;
            }
            _ => {
                // SIMD scan for the next quote or escape; append as UTF-8.
                let run_start = *pos;
                let run = &b[run_start..];
                let rel = memchr::memchr2(b'"', b'\\', run).unwrap_or(run.len());
                *pos = run_start + rel;
                result.push_str(unsafe { std::str::from_utf8_unchecked(&b[run_start..*pos]) });
            }
        }
    }
    Err("Unterminated string".to_string())
}

#[inline]
fn parse_hex4(b: &[u8], pos: &mut usize) -> Result<u16, String> {
    if *pos + 4 > b.len() {
        return Err("Invalid \\u escape".to_string());
    }
    // Hand-rolled hex nibble decode — faster than `from_str_radix` for 4 chars.
    let mut val: u16 = 0;
    for i in 0..4 {
        let c = b[*pos + i];
        let nibble = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return Err("Invalid hex in \\u escape".to_string()),
        };
        val = (val << 4) | nibble as u16;
    }
    *pos += 4;
    Ok(val)
}

fn parse_array(b: &[u8], pos: &mut usize, depth: usize) -> Result<Value, String> {
    *pos += 1; // skip '['
    if peek(b, pos)? == b']' {
        *pos += 1;
        return Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))));
    }
    // 32 covers typical API page sizes better than 16 with one fewer realloc.
    let mut items = Vec::with_capacity(32);
    loop {
        items.push(parse_value(b, pos, depth + 1)?);
        match peek(b, pos)? {
            b',' => *pos += 1,
            b']' => {
                *pos += 1;
                return Ok(Value::Array(Rc::new(RefCell::new(items))));
            }
            _ => return Err(format!("Expected ',' or ']' at position {}", *pos)),
        }
    }
}

fn parse_object(b: &[u8], pos: &mut usize, depth: usize) -> Result<Value, String> {
    *pos += 1; // skip '{'
    if peek(b, pos)? == b'}' {
        *pos += 1;
        return Ok(Value::Hash(Rc::new(RefCell::new(HashPairs::with_hasher(
            json_hasher(),
        )))));
    }
    // Pre-allocate for typical API objects (id + a handful of fields).
    let mut pairs = HashPairs::with_capacity_and_hasher(8, json_hasher());
    loop {
        if peek(b, pos)? != b'"' {
            return Err(format!("Expected string key at position {}", *pos));
        }
        let key = parse_string(b, pos)?;
        if peek(b, pos)? != b':' {
            return Err(format!("Expected ':' at position {}", *pos));
        }
        *pos += 1;
        let value = parse_value(b, pos, depth + 1)?;
        pairs.insert(HashKey::String(key), value);
        match peek(b, pos)? {
            b',' => *pos += 1,
            b'}' => {
                *pos += 1;
                return Ok(Value::Hash(Rc::new(RefCell::new(pairs))));
            }
            _ => return Err(format!("Expected ',' or '}}' at position {}", *pos)),
        }
    }
}

/// Unwrap a value, extracting the underlying value from class instances.
/// For String/Array/Hash class instances, returns the __value field.
/// For other values, returns the value as-is.
pub fn unwrap_value(value: &Value) -> Value {
    match value {
        Value::Instance(inst) => match inst.borrow().fields.get("__value").cloned() {
            Some(inner) => unwrap_value(&inner),
            _ => value.clone(),
        },
        _ => value.clone(),
    }
}

use crate::ast::TypeKind;

/// Check if a runtime value matches an expected type annotation.
/// Used for runtime return type enforcement.
fn class_answers_to(class: &Class, name: &str) -> bool {
    let named = class.name == name
        || class
            .name
            .rsplit_once("::")
            .is_some_and(|(_, short)| short == name);
    named
        || class
            .superclass
            .as_ref()
            .is_some_and(|superclass| class_answers_to(superclass, name))
}

pub fn value_matches_type(value: &Value, expected: &TypeAnnotation) -> bool {
    match &expected.kind {
        TypeKind::Named(name) => match TypeCheck::builtin(name) {
            Some(check) => check.admits(value),
            None => instance_answers_to(value, name),
        },
        TypeKind::Void => matches!(value, Value::Null),
        TypeKind::Nullable(inner) => {
            matches!(value, Value::Null) || value_matches_type(value, inner)
        }
        TypeKind::Array(_) => matches!(value, Value::Array(_)),
        TypeKind::Hash { .. } => matches!(value, Value::Hash(_)),
        TypeKind::Function { .. } => is_function_value(value),
    }
}

/// A function on either engine: the tree-walker's, a builtin, or a VM
/// closure (all of which `type_name` reports as `Function`).
#[inline]
fn is_function_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
    )
}

/// An instance of the class `name` names, or of a subclass of it: the
/// class named in full or, written inside its context, by its short name
/// (`-> Invoice` for a `Billing::Invoice`).
#[inline]
fn instance_answers_to(value: &Value, name: &str) -> bool {
    match value {
        Value::Instance(inst) => class_answers_to(&inst.borrow().class, name),
        _ => false,
    }
}

/// A type annotation lowered once, so a check against it reads no name.
/// It decides exactly what [`value_matches_type`] decides for the same
/// annotation; the VM lowers a function's return type when it compiles
/// the function and checks each return against this.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeCheck {
    Any,
    Int,
    Float,
    Decimal,
    String,
    Bool,
    Array,
    Hash,
    Function,
    /// `Void`, or the name `Null`.
    Null,
    /// A class name, matched case-sensitively against the class and its
    /// ancestors, in full or by short name.
    Class(Box<str>),
    /// `T?`: null, or what the inner check admits.
    Nullable(Box<TypeCheck>),
}

impl TypeCheck {
    /// The check for an annotation.
    pub fn lower(expected: &TypeAnnotation) -> Self {
        match &expected.kind {
            TypeKind::Named(name) => {
                Self::builtin(name).unwrap_or_else(|| Self::Class(name.as_str().into()))
            }
            TypeKind::Void => Self::Null,
            TypeKind::Nullable(inner) => Self::Nullable(Box::new(Self::lower(inner))),
            TypeKind::Array(_) => Self::Array,
            TypeKind::Hash { .. } => Self::Hash,
            TypeKind::Function { .. } => Self::Function,
        }
    }

    /// The built-in type a name spells, in any ASCII case (`int`, `Int`,
    /// `INT`), or `None` for a class name. Dispatches on the length first,
    /// so a name is compared against at most three candidates.
    fn builtin(name: &str) -> Option<Self> {
        let is = |candidate: &str| name.eq_ignore_ascii_case(candidate);
        let check = match name.len() {
            3 if is("int") => Self::Int,
            3 if is("any") => Self::Any,
            4 if is("hash") => Self::Hash,
            4 if is("bool") => Self::Bool,
            4 if is("void") || is("null") => Self::Null,
            5 if is("array") => Self::Array,
            5 if is("float") => Self::Float,
            6 if is("string") => Self::String,
            7 if is("decimal") => Self::Decimal,
            8 if is("function") => Self::Function,
            _ => return None,
        };
        Some(check)
    }

    /// Whether `value` is of this type.
    #[inline]
    pub fn admits(&self, value: &Value) -> bool {
        match self {
            Self::Any => true,
            Self::Int => matches!(value, Value::Int(_)),
            Self::Float => matches!(value, Value::Float(_)),
            Self::Decimal => matches!(value, Value::Decimal(_)),
            Self::String => matches!(value, Value::String(_)),
            Self::Bool => matches!(value, Value::Bool(_)),
            Self::Array => matches!(value, Value::Array(_)),
            Self::Hash => matches!(value, Value::Hash(_)),
            Self::Function => is_function_value(value),
            Self::Null => matches!(value, Value::Null),
            Self::Class(name) => instance_answers_to(value, name),
            Self::Nullable(inner) => matches!(value, Value::Null) || inner.admits(value),
        }
    }
}

/// A `DateTime`'s `YYYY-MM-DD HH:MM:SS` form, formatted on the stack. See
/// [`Value::datetime_text`].
pub struct DateTimeText([u8; DateTimeText::LEN]);

impl DateTimeText {
    pub const LEN: usize = 19;

    /// Format the instant `nanos` (UTC) shifted by `offset_secs` east of UTC.
    pub fn new(nanos: i64, offset_secs: i32) -> Self {
        // Floor to whole seconds, as `from_timestamp_nanos` does: the
        // sub-second part is always non-negative, so -1ns is 23:59:59.
        let secs = nanos.div_euclid(1_000_000_000) + offset_secs as i64;
        let days = secs.div_euclid(86_400);
        let second_of_day = secs.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);
        let year = year as u32;
        debug_assert!(
            year <= 9999,
            "DateTime year {year} out of the 4-digit range"
        );
        let hour = second_of_day / 3600;
        let minute = second_of_day / 60 % 60;
        let second = second_of_day % 60;
        let mut buf = *b"0000-00-00 00:00:00";
        let two = |buf: &mut [u8; 19], at: usize, n: u32| {
            buf[at] = b'0' + (n / 10 % 10) as u8;
            buf[at + 1] = b'0' + (n % 10) as u8;
        };
        two(&mut buf, 0, year / 100);
        two(&mut buf, 2, year % 100);
        two(&mut buf, 5, month);
        two(&mut buf, 8, day);
        two(&mut buf, 11, hour);
        two(&mut buf, 14, minute);
        two(&mut buf, 17, second);
        DateTimeText(buf)
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        // Only ASCII digits, '-', ' ' and ':' are ever written.
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day) —
/// Howard Hinnant's `civil_from_days`, exact for negative days too.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod decimal_tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[test]
    fn test_decimal_value_creation() {
        let decimal = Decimal::from_str("19.99").unwrap();
        let decimal_value = DecimalValue(decimal, 2);

        assert_eq!(decimal_value.precision(), 2);
        assert_eq!(decimal_value.to_string(), "19.99");
    }

    #[test]
    fn test_decimal_value_from_str() {
        let result = DecimalValue::from_str("19.99", 2);
        assert!(result.is_ok());
        let decimal_value = result.unwrap();
        assert_eq!(decimal_value.precision(), 2);
        assert_eq!(decimal_value.to_string(), "19.99");
    }

    #[test]
    fn test_decimal_value_from_str_invalid() {
        let result = DecimalValue::from_str("not_a_decimal", 2);
        assert!(result.is_err());
    }

    /// A literal's precision is the digits written after its point, also
    /// past the 28 places the value keeps: the two literals below print
    /// alike and compare unequal, on both engines.
    #[test]
    fn a_decimal_literal_keeps_the_precision_it_was_written_with() {
        let price = DecimalValue::from_literal("19.99").expect("19.99");
        assert_eq!(price, DecimalValue(Decimal::from_str("19.99").unwrap(), 2));
        assert_eq!(DecimalValue::from_literal("1.00").unwrap().precision(), 2);
        assert_eq!(DecimalValue::from_literal("-2.5").unwrap().precision(), 1);

        let long = DecimalValue::from_literal("1.12345678901234567890123456789012").unwrap();
        assert_eq!(long.precision(), 32);
        assert_eq!(long.to_string(), "1.1234567890123456789012345679");
        let rounded = DecimalValue::from_literal("1.1234567890123456789012345679").unwrap();
        assert_eq!(long.value(), rounded.value());
        assert_ne!(long, rounded);
    }

    #[test]
    fn text_that_is_no_decimal_is_no_literal() {
        // `%n[abcD]` reads as the literal `abc.00`.
        assert_eq!(DecimalValue::from_literal("abc.00"), None);
        // An integer part beyond Decimal's 96-bit range.
        assert_eq!(
            DecimalValue::from_literal("99999999999999999999999999999999.5"),
            None
        );
    }

    #[test]
    fn test_decimal_value_to_f64() {
        let decimal = Decimal::from_str("19.99").unwrap();
        let decimal_value = DecimalValue(decimal, 2);

        let f64_val = decimal_value.to_f64();
        assert!((f64_val - 19.99).abs() < 0.001);
    }

    #[test]
    fn test_decimal_value_display() {
        let decimal = Decimal::from_str("123.45").unwrap();
        let decimal_value = DecimalValue(decimal, 2);

        let display = format!("{}", decimal_value);
        assert_eq!(display, "123.45");
    }

    #[test]
    fn test_decimal_value_clone() {
        let decimal = Decimal::from_str("99.99").unwrap();
        let original = DecimalValue(decimal, 2);
        let cloned = original.clone();

        assert_eq!(original.to_string(), cloned.to_string());
        assert_eq!(original.precision(), cloned.precision());
    }

    #[test]
    fn test_decimal_value_hash() {
        let decimal1 = Decimal::from_str("10.00").unwrap();
        let decimal2 = Decimal::from_str("10.00").unwrap();
        let decimal3 = Decimal::from_str("20.00").unwrap();

        let dv1 = DecimalValue(decimal1, 2);
        let dv2 = DecimalValue(decimal2, 2);
        let dv3 = DecimalValue(decimal3, 2);

        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hash;

        let mut hasher1 = DefaultHasher::new();
        dv1.hash(&mut hasher1);
        let hash1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        dv2.hash(&mut hasher2);
        let hash2 = hasher2.finish();

        let mut hasher3 = DefaultHasher::new();
        dv3.hash(&mut hasher3);
        let hash3 = hasher3.finish();

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_hash_key_decimal() {
        let decimal = Decimal::from_str("19.99").unwrap();
        let decimal_value = DecimalValue(decimal, 2);

        let hash_key = HashKey::Decimal(decimal_value.clone());
        let back_to_value = hash_key.to_value();

        match back_to_value {
            Value::Decimal(dv) => {
                assert_eq!(dv.to_string(), decimal_value.to_string());
            }
            _ => panic!("Expected Decimal value"),
        }
    }

    #[test]
    fn test_json_to_value_numeric_looking_string_stays_string() {
        // Must agree with `parse_json` / `JSON.parse`: no silent Decimal promote.
        for s in ["19.99", "0.0675", "100"] {
            let json = serde_json::Value::String(s.to_string());
            let value = json_to_value(json).unwrap();
            match value {
                Value::String(got) => assert_eq!(&*got, s),
                other => panic!("expected String for {:?}, got {}", s, other.type_name()),
            }
            let direct = parse_json(&format!("\"{}\"", s)).unwrap();
            assert_eq!(direct, Value::String(s.into()));
        }
    }

    #[test]
    fn test_value_to_json_decimal() {
        let decimal = Decimal::from_str("19.99").unwrap();
        let decimal_value = DecimalValue(decimal, 2);
        let value = Value::Decimal(decimal_value);

        let result = value_to_json(&value);

        assert!(result.is_ok());
        let json = result.unwrap();

        match json {
            serde_json::Value::String(s) => {
                assert_eq!(s, "19.99");
            }
            _ => panic!("Expected JSON string"),
        }
    }

    #[test]
    fn test_value_decimal_type_name() {
        let decimal = Decimal::from_str("19.99").unwrap();
        let decimal_value = DecimalValue(decimal, 2);
        let value = Value::Decimal(decimal_value);

        assert_eq!(value.type_name(), "decimal");
    }

    #[test]
    fn test_value_decimal_is_truthy() {
        let decimal = Decimal::from_str("0.00").unwrap();
        let decimal_value = DecimalValue(decimal, 2);
        let value = Value::Decimal(decimal_value);

        assert!(value.is_truthy());
    }

    #[test]
    fn test_value_decimal_equality() {
        let decimal1 = Decimal::from_str("19.99").unwrap();
        let decimal2 = Decimal::from_str("19.99").unwrap();
        let decimal3 = Decimal::from_str("20.00").unwrap();

        let value1 = Value::Decimal(DecimalValue(decimal1, 2));
        let value2 = Value::Decimal(DecimalValue(decimal2, 2));
        let value3 = Value::Decimal(DecimalValue(decimal3, 2));

        assert_eq!(value1, value2);
        assert_ne!(value1, value3);
    }

    #[test]
    fn test_value_decimal_partial_eq() {
        let decimal = Decimal::from_str("10.00").unwrap();
        let value = Value::Decimal(DecimalValue(decimal, 2));

        assert!(value == Value::Decimal(DecimalValue(Decimal::from_str("10.00").unwrap(), 2)));
        assert!(value != Value::Decimal(DecimalValue(Decimal::from_str("20.00").unwrap(), 2)));
    }

    #[test]
    fn test_decimal_precision_variations() {
        // Precision is tracked by the explicit Decimal constructor, not by
        // json_to_value (which no longer silently promotes numeric-looking strings).
        let test_cases = vec![
            ("0.1", 1),
            ("0.01", 2),
            ("0.001", 3),
            ("0.0001", 4),
            ("123.45", 2),
            ("1000", 0),
        ];

        for (input, expected_precision) in test_cases {
            let dv = DecimalValue::from_str(input, expected_precision)
                .unwrap_or_else(|e| panic!("Failed for input {}: {}", input, e));
            assert_eq!(
                dv.precision(),
                expected_precision,
                "Precision mismatch for input: {}",
                input
            );
            assert_eq!(dv.to_string(), input);
        }
    }

    #[test]
    fn test_decimal_zero_values() {
        let zero_values = vec![("0", 0), ("0.0", 1), ("0.00", 2), ("0.000", 3)];

        for (input, precision) in zero_values {
            let dv = DecimalValue::from_str(input, precision)
                .unwrap_or_else(|e| panic!("Failed for zero input {}: {}", input, e));
            let dv_str = dv.to_string();
            assert!(
                dv_str == "0" || dv_str == "0.0" || dv_str == "0.00" || dv_str == "0.000",
                "Unexpected zero format for input {}: got {}",
                input,
                dv_str
            );
        }
    }

    #[test]
    fn test_decimal_negative_values() {
        let dv = DecimalValue::from_str("-19.99", 2).expect("parse -19.99");
        assert_eq!(dv.to_string(), "-19.99");
    }

    #[test]
    fn test_decimal_large_values() {
        let dv = DecimalValue::from_str("9999999999.99", 2).expect("parse large");
        assert_eq!(dv.to_string(), "9999999999.99");
    }

    #[test]
    fn test_decimal_in_array_json() {
        // Numeric-looking strings stay strings (no silent Decimal promote).
        let json = serde_json::Value::Array(vec![
            serde_json::Value::String("10.00".to_string()),
            serde_json::Value::String("20.50".to_string()),
            serde_json::Value::String("30.75".to_string()),
        ]);

        let result = json_to_value(json);

        assert!(result.is_ok());
        let value = result.unwrap();

        match value {
            Value::Array(arr_ref) => {
                let arr = arr_ref.borrow();
                assert_eq!(arr.len(), 3);

                match &arr[0] {
                    Value::String(s) => assert_eq!(&**s, "10.00"),
                    other => panic!("Expected String in array, got {}", other.type_name()),
                }
            }
            _ => panic!("Expected Array value"),
        }
    }

    #[test]
    fn test_decimal_in_hash_json() {
        // Numeric-looking strings stay strings (no silent Decimal promote).
        let mut map = serde_json::Map::new();
        map.insert(
            "price".to_string(),
            serde_json::Value::String("19.99".to_string()),
        );
        map.insert(
            "name".to_string(),
            serde_json::Value::String("Widget".to_string()),
        );
        let json = serde_json::Value::Object(map);

        let result = json_to_value(json);

        assert!(result.is_ok());
        let value = result.unwrap();

        match value {
            Value::Hash(hash_ref) => {
                let hash = hash_ref.borrow();
                let price_key = HashKey::String("price".into());

                if let Some(price_value) = hash.get(&price_key) {
                    match price_value {
                        Value::String(s) => assert_eq!(&**s, "19.99"),
                        other => panic!("Expected String price, got {}", other.type_name()),
                    }
                } else {
                    panic!("Price key not found in hash");
                }
            }
            _ => panic!("Expected Hash value"),
        }
    }
}

#[cfg(test)]
mod return_type_tests {
    use super::*;
    use crate::ast::{TypeAnnotation, TypeKind};
    use crate::span::Span;

    fn make_type(kind: TypeKind) -> TypeAnnotation {
        TypeAnnotation::new(kind, Span::default())
    }

    #[test]
    fn test_int_matches_int() {
        let value = Value::Int(42);
        let ty = make_type(TypeKind::Named("Int".to_string()));
        assert!(value_matches_type(&value, &ty));
    }

    #[test]
    fn test_string_matches_string() {
        let value = Value::String("hello".into());
        let ty = make_type(TypeKind::Named("String".to_string()));
        assert!(value_matches_type(&value, &ty));
    }

    #[test]
    fn test_int_does_not_match_string() {
        let value = Value::Int(42);
        let ty = make_type(TypeKind::Named("String".to_string()));
        assert!(!value_matches_type(&value, &ty));
    }

    #[test]
    fn test_any_matches_everything() {
        let ty = make_type(TypeKind::Named("Any".to_string()));
        assert!(value_matches_type(&Value::Int(1), &ty));
        assert!(value_matches_type(&Value::String("x".into()), &ty));
        assert!(value_matches_type(&Value::Null, &ty));
        assert!(value_matches_type(&Value::Bool(true), &ty));
    }

    #[test]
    fn test_nullable_accepts_null() {
        let inner = make_type(TypeKind::Named("Int".to_string()));
        let ty = make_type(TypeKind::Nullable(Box::new(inner)));
        assert!(value_matches_type(&Value::Null, &ty));
        assert!(value_matches_type(&Value::Int(42), &ty));
        assert!(!value_matches_type(&Value::String("x".into()), &ty));
    }

    #[test]
    fn test_void_matches_null() {
        let ty = make_type(TypeKind::Void);
        assert!(value_matches_type(&Value::Null, &ty));
        assert!(!value_matches_type(&Value::Int(1), &ty));
    }

    #[test]
    fn test_bool_matches_bool() {
        let ty = make_type(TypeKind::Named("Bool".to_string()));
        assert!(value_matches_type(&Value::Bool(true), &ty));
        assert!(value_matches_type(&Value::Bool(false), &ty));
        assert!(!value_matches_type(&Value::Int(1), &ty));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_float_matches_float() {
        let ty = make_type(TypeKind::Named("Float".to_string()));
        assert!(value_matches_type(&Value::Float(3.14), &ty));
        assert!(!value_matches_type(&Value::Int(3), &ty));
    }

    #[test]
    fn test_array_type_matches_array() {
        let inner = make_type(TypeKind::Named("Int".to_string()));
        let ty = make_type(TypeKind::Array(Box::new(inner)));
        let arr = Value::Array(Rc::new(RefCell::new(vec![Value::Int(1)])));
        assert!(value_matches_type(&arr, &ty));
        assert!(!value_matches_type(&Value::Int(1), &ty));
    }

    #[test]
    fn test_hash_type_matches_hash() {
        let key_ty = make_type(TypeKind::Named("String".to_string()));
        let val_ty = make_type(TypeKind::Named("Int".to_string()));
        let ty = make_type(TypeKind::Hash {
            key_type: Box::new(key_ty),
            value_type: Box::new(val_ty),
        });
        let hash = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));
        assert!(value_matches_type(&hash, &ty));
        assert!(!value_matches_type(&Value::Int(1), &ty));
    }

    #[test]
    fn test_case_insensitive_named_types() {
        // TypeKind uses "Int" but we should match case-insensitively
        let ty_lower = make_type(TypeKind::Named("int".to_string()));
        let ty_upper = make_type(TypeKind::Named("INT".to_string()));
        let ty_mixed = make_type(TypeKind::Named("Int".to_string()));
        let value = Value::Int(42);
        assert!(value_matches_type(&value, &ty_lower));
        assert!(value_matches_type(&value, &ty_upper));
        assert!(value_matches_type(&value, &ty_mixed));
    }

    fn annotations() -> Vec<TypeAnnotation> {
        let named = |n: &str| make_type(TypeKind::Named(n.to_string()));
        let mut types: Vec<TypeAnnotation> = [
            "Int", "int", "INT", "Float", "Decimal", "decimal", "String", "Bool", "Array", "Hash",
            "hash", "Function", "Void", "Null", "Any", "Pt", "pt", "Int2", "Strin",
        ]
        .iter()
        .map(|n| named(n))
        .collect();
        types.push(make_type(TypeKind::Void));
        types.push(make_type(TypeKind::Nullable(Box::new(named("Int")))));
        types.push(make_type(TypeKind::Nullable(Box::new(named("Pt")))));
        types.push(make_type(TypeKind::Array(Box::new(named("Int")))));
        types.push(make_type(TypeKind::Hash {
            key_type: Box::new(named("String")),
            value_type: Box::new(named("Int")),
        }));
        types.push(make_type(TypeKind::Function {
            params: vec![named("Int")],
            return_type: Box::new(named("Int")),
        }));
        types
    }

    fn values() -> Vec<Value> {
        vec![
            Value::Int(1),
            Value::Float(1.5),
            Value::Decimal(DecimalValue(rust_decimal::Decimal::new(15, 1), 1)),
            Value::String("s".into()),
            Value::Bool(true),
            Value::Null,
            Value::Array(Rc::new(RefCell::new(vec![]))),
            Value::Hash(Rc::new(RefCell::new(HashPairs::default()))),
        ]
    }

    #[test]
    fn a_lowered_check_decides_what_value_matches_type_decides() {
        for ty in annotations() {
            let check = TypeCheck::lower(&ty);
            for value in values() {
                assert_eq!(
                    check.admits(&value),
                    value_matches_type(&value, &ty),
                    "{ty} against {}",
                    value.type_name()
                );
            }
        }
    }

    #[test]
    fn names_lower_case_insensitively_and_others_are_classes() {
        let named = |n: &str| TypeCheck::lower(&make_type(TypeKind::Named(n.to_string())));
        assert_eq!(named("HASH"), TypeCheck::Hash);
        assert_eq!(named("void"), TypeCheck::Null);
        assert_eq!(named("Null"), TypeCheck::Null);
        assert_eq!(named("function"), TypeCheck::Function);
        assert_eq!(named("Strings"), TypeCheck::Class("Strings".into()));
        assert_eq!(
            named("Billing::Invoice"),
            TypeCheck::Class("Billing::Invoice".into())
        );
    }
}

#[cfg(test)]
mod value_misc_tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// `display_len` sizes the buffer that `write_to_string` then fills, so the
    /// two must agree exactly — and both were rewritten to stop allocating a
    /// throwaway String per value. A disagreement would only cost a realloc,
    /// but a wrong *rendering* would corrupt every `join` and interpolation.
    #[test]
    fn integer_rendering_matches_to_string_and_its_reported_length() {
        for n in [
            0i64,
            1,
            -1,
            9,
            10,
            -10,
            99,
            100,
            -100,
            999,
            1000,
            12345,
            -12345,
            i64::MAX,
            i64::MIN,
            i64::MAX - 1,
            i64::MIN + 1,
        ] {
            let v = Value::Int(n);
            let mut rendered = String::new();
            v.write_to_string(&mut rendered);
            assert_eq!(rendered, n.to_string(), "rendering changed for {n}");
            assert_eq!(v.display_len(), rendered.len(), "length disagrees for {n}");
        }
    }

    #[test]
    fn integer_display_len_is_exact_at_every_digit_boundary() {
        let mut n: i64 = 1;
        loop {
            for probe in [n - 1, n, n + 1] {
                for signed in [probe, -probe] {
                    assert_eq!(
                        Value::Int(signed).display_len(),
                        signed.to_string().len(),
                        "at {signed}"
                    );
                }
            }
            match n.checked_mul(10) {
                Some(next) => n = next,
                None => break,
            }
        }
    }

    /// Pin the zero-alloc lookup invariant after the SoliStr (Arc<str>)
    /// migration: a `StrKey`/`SymKey` borrowed key must hash byte-identically
    /// to `HashKey::String`/`HashKey::Symbol`, or every string-keyed hash
    /// lookup silently misses (map corruption, not a compile error).
    #[test]
    fn strkey_symkey_hash_equivalent_to_hashkey() {
        let mut map = HashPairs::default();
        map.insert(HashKey::String("name".into()), Value::Int(1));
        map.insert(HashKey::Symbol("status".into()), Value::Int(2));

        assert_eq!(map.get(&StrKey("name")), Some(&Value::Int(1)));
        assert_eq!(map.get(&SymKey("status")), Some(&Value::Int(2)));
        // Cross-kind lookups must NOT match (distinct hash tags).
        assert_eq!(map.get(&StrKey("status")), None);
        assert_eq!(map.get(&SymKey("name")), None);
        // Insert-then-update through the borrowed key path.
        assert!(map.contains_key(&StrKey("name")));
        assert!(!map.contains_key(&StrKey("missing")));
    }

    #[test]
    fn hash_set_overwrites_without_growing() {
        let mut map = HashPairs::default();
        let key = Value::String("name".into());
        assert!(hash_set_value(&mut map, &key, Value::Int(1)));
        assert!(hash_set_value(&mut map, &key, Value::Int(2)));
        assert_eq!(map.len(), 1);
        assert_eq!(hash_get_value(&map, &key), Some(&Value::Int(2)));
    }

    #[test]
    fn hash_get_set_small_and_large() {
        let mut map = HashPairs::default();
        for i in 0..20 {
            let key = Value::String(format!("k{i}").into());
            assert!(hash_set_value(&mut map, &key, Value::Int(i)));
        }
        assert_eq!(
            hash_get_value(&map, &Value::String("k0".into())),
            Some(&Value::Int(0))
        );
        assert_eq!(
            hash_get_value(&map, &Value::String("k19".into())),
            Some(&Value::Int(19))
        );
        assert_eq!(hash_get_value(&map, &Value::String("missing".into())), None);
        assert_eq!(hash_get_value(&map, &Value::Int(0)), None);
        assert!(!hash_set_value(&mut map, &Value::Float(1.0), Value::Int(1)));
    }

    #[test]
    fn type_name_covers_all_variants() {
        assert_eq!(Value::Int(0).type_name(), "int");
        assert_eq!(Value::Float(0.0).type_name(), "float");
        assert_eq!(Value::String(String::new().into()).type_name(), "string");
        assert_eq!(Value::Bool(true).type_name(), "bool");
        assert_eq!(Value::Null.type_name(), "null");
        assert_eq!(
            Value::Array(Rc::new(RefCell::new(Vec::new()))).type_name(),
            "array"
        );
        let h = HashPairs::default();
        assert_eq!(Value::Hash(Rc::new(RefCell::new(h))).type_name(), "hash");
        assert_eq!(Value::Symbol("x".into()).type_name(), "symbol");
    }

    #[test]
    fn is_truthy_distinguishes_zero_empty_collections() {
        // Truthy
        assert!(Value::Int(1).is_truthy());
        assert!(Value::Float(0.0).is_truthy());
        assert!(Value::String("hi".into()).is_truthy());
        assert!(Value::Bool(true).is_truthy());
        // Falsy: Bool(false), Null, Int(0), empty string, empty array, empty hash
        assert!(!Value::Bool(false).is_truthy());
        assert!(!Value::Null.is_truthy());
        assert!(!Value::Int(0).is_truthy());
        assert!(!Value::String(String::new().into()).is_truthy());
        assert!(!Value::Array(Rc::new(RefCell::new(Vec::new()))).is_truthy());
        let h = HashPairs::default();
        assert!(!Value::Hash(Rc::new(RefCell::new(h))).is_truthy());
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.14 is test data for float formatting, not π
    fn display_basic_values() {
        assert_eq!(format!("{}", Value::Int(42)), "42");
        assert_eq!(format!("{}", Value::Float(3.14)), "3.14");
        assert_eq!(format!("{}", Value::String("hi".into())), "hi");
        assert_eq!(format!("{}", Value::Bool(true)), "true");
        assert_eq!(format!("{}", Value::Bool(false)), "false");
        assert_eq!(format!("{}", Value::Null), "null");
        assert_eq!(format!("{}", Value::Symbol("foo".into())), ":foo");
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.14 is test data for JSON float parsing, not π
    fn parse_json_primitives() {
        assert_eq!(parse_json("42").unwrap(), Value::Int(42));
        assert_eq!(parse_json("3.14").unwrap(), Value::Float(3.14));
        assert_eq!(parse_json("true").unwrap(), Value::Bool(true));
        assert_eq!(parse_json("false").unwrap(), Value::Bool(false));
        assert_eq!(parse_json("null").unwrap(), Value::Null);
        assert_eq!(
            parse_json(r#""hello""#).unwrap(),
            Value::String("hello".into())
        );
    }

    #[test]
    fn parse_json_string_escape_then_utf8_stays_utf8() {
        // Regression: the escape slow path pushed raw bytes as Latin-1
        // (`b[i] as char`), mojibake-ing any multi-byte UTF-8 that followed
        // an escape ("café" → "cafÃ©"). Fast path (no escape) was fine.
        assert_eq!(
            parse_json(r#""ligne\ncafé déjà — cœur""#).unwrap(),
            Value::String("ligne\ncafé déjà — cœur".into())
        );
        // UTF-8 before the first escape (correct segment) + after (was broken).
        assert_eq!(
            parse_json(r#""été \"chaud\" à Lyon""#).unwrap(),
            Value::String("été \"chaud\" à Lyon".into())
        );
        // Emoji (4-byte sequences) after an escape.
        assert_eq!(
            parse_json(r#""ok\t👍""#).unwrap(),
            Value::String("ok\t👍".into())
        );
    }

    #[test]
    fn parse_json_array() {
        let parsed = parse_json("[1, 2, 3]").unwrap();
        match parsed {
            Value::Array(arr) => {
                let arr = arr.borrow();
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], Value::Int(1));
                assert_eq!(arr[2], Value::Int(3));
            }
            _ => panic!("expected Array, got {:?}", parsed.type_name()),
        }
    }

    #[test]
    fn parse_json_object() {
        let parsed = parse_json(r#"{"a": 1, "b": "x"}"#).unwrap();
        match parsed {
            Value::Hash(h) => {
                let h = h.borrow();
                assert_eq!(h.len(), 2);
                assert_eq!(h.get(&HashKey::String("a".into())), Some(&Value::Int(1)));
                assert_eq!(
                    h.get(&HashKey::String("b".into())),
                    Some(&Value::String("x".into()))
                );
            }
            _ => panic!("expected Hash"),
        }
    }

    #[test]
    fn parse_json_nested() {
        let r = parse_json(r#"{"items": [1, {"k": "v"}], "n": null}"#);
        assert!(r.is_ok(), "nested parse failed: {:?}", r.err());
    }

    /// A hostile request body must not abort the worker.
    ///
    /// `[` × 100_000 recursed one frame per level in the hand-rolled parser,
    /// and a native stack overflow aborts without unwinding — so the
    /// per-request `catch_unwind` never saw it and one request killed the
    /// process. The depth guards shipped for JSON earlier covered only the
    /// `serde_json` conversion helpers, not this parser.
    #[test]
    fn parse_json_deeply_nested_is_an_error_not_an_abort() {
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        let err = parse_json(&deep).expect_err("deeply nested JSON must be rejected");
        assert!(
            err.contains("nested too deeply"),
            "expected a depth error, got: {err}"
        );

        // The request path swallows the error into `None` rather than crashing.
        assert!(crate::interpreter::builtins::server::parse_json_body(&deep).is_none());

        // Ordinary nesting still parses.
        assert!(parse_json("{\"a\":[1,{\"b\":[2,3]}]}").is_ok());
        // And so does a document just inside the cap.
        let ok_depth = format!(
            "{}1{}",
            "[".repeat(MAX_JSON_PARSE_DEPTH - 1),
            "]".repeat(MAX_JSON_PARSE_DEPTH - 1)
        );
        assert!(
            parse_json(&ok_depth).is_ok(),
            "depth just under the cap must parse"
        );
    }

    #[test]
    fn parse_json_invalid_returns_err() {
        assert!(parse_json("not json").is_err());
        assert!(parse_json(r#"{"unclosed""#).is_err());
        assert!(parse_json(r#"[1, 2,"#).is_err());
        // Leading zeros are invalid JSON integers.
        assert!(parse_json("01").is_err());
        assert!(parse_json("-01").is_err());
    }

    #[test]
    fn parse_json_integers_including_boundaries() {
        assert_eq!(parse_json("0").unwrap(), Value::Int(0));
        assert_eq!(parse_json("-0").unwrap(), Value::Int(0));
        assert_eq!(parse_json("42").unwrap(), Value::Int(42));
        assert_eq!(parse_json("-42").unwrap(), Value::Int(-42));
        assert_eq!(
            parse_json("9223372036854775807").unwrap(),
            Value::Int(i64::MAX)
        );
        // One past i64::MAX becomes a float (hand-rolled overflow path).
        match parse_json("9223372036854775808").unwrap() {
            Value::Float(f) => assert!(f > 9.0e18),
            other => panic!("expected Float, got {}", other.type_name()),
        }
    }

    #[test]
    fn parse_json_bytes_matches_str() {
        let s = r#"{"a": 1}"#;
        let from_str = parse_json(s).unwrap();
        let from_bytes = parse_json_bytes(s.as_bytes()).unwrap();
        assert_eq!(from_str, from_bytes);
    }

    #[test]
    fn hashkey_string_int_equality() {
        let k1 = HashKey::String("hello".into());
        let k2 = HashKey::String("hello".into());
        assert_eq!(k1, k2);

        let i1 = HashKey::Int(42);
        let i2 = HashKey::Int(42);
        assert_eq!(i1, i2);

        assert_ne!(HashKey::String("a".into()), HashKey::Int(1));
    }

    #[test]
    fn value_partial_eq_collections() {
        let a = Value::Array(Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(2)])));
        let b = Value::Array(Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(2)])));
        assert_eq!(a, b);
        let c = Value::Array(Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(3)])));
        assert_ne!(a, c);
    }

    #[test]
    fn value_int_float_cross_eq() {
        // Int and Float compare by value when integral.
        assert_eq!(Value::Int(3), Value::Float(3.0));
        assert_eq!(Value::Float(3.0), Value::Int(3));
        assert_ne!(Value::Int(3), Value::Float(3.5));
    }

    // SEC-013 — `is_safe_serialised_field` regression coverage.

    #[test]
    fn safe_serialised_field_allows_normal_attributes() {
        assert!(super::is_safe_serialised_field("name"));
        assert!(super::is_safe_serialised_field("email"));
        assert!(super::is_safe_serialised_field("title"));
        assert!(super::is_safe_serialised_field("created_count"));
    }

    #[test]
    fn safe_serialised_field_allows_public_model_metadata() {
        for f in &["_key", "_id", "_rev", "_created_at", "_updated_at"] {
            assert!(
                super::is_safe_serialised_field(f),
                "{} should remain serialised",
                f
            );
        }
    }

    #[test]
    fn safe_serialised_field_blocks_other_underscore_fields() {
        assert!(!super::is_safe_serialised_field("_errors"));
        assert!(!super::is_safe_serialised_field("_text"));
        assert!(!super::is_safe_serialised_field("_pending_translations"));
        assert!(!super::is_safe_serialised_field("_anything_else"));
    }

    #[test]
    fn safe_serialised_field_blocks_password_variants() {
        assert!(!super::is_safe_serialised_field("password"));
        assert!(!super::is_safe_serialised_field("Password"));
        assert!(!super::is_safe_serialised_field("password_digest"));
        assert!(!super::is_safe_serialised_field("password_hash"));
        assert!(!super::is_safe_serialised_field("PasswordHash"));
        assert!(!super::is_safe_serialised_field("password_reset_token"));
    }

    #[test]
    fn safe_serialised_field_blocks_token_digest_secret_hash_suffixes() {
        assert!(!super::is_safe_serialised_field("reset_token"));
        assert!(!super::is_safe_serialised_field("api_secret"));
        assert!(!super::is_safe_serialised_field("auth_digest"));
        assert!(!super::is_safe_serialised_field("session_hash"));
        assert!(!super::is_safe_serialised_field("Reset_Token")); // case-insensitive
    }

    #[test]
    fn safe_serialised_field_matches_metadata_names_exactly() {
        // Only the exact metadata names pass: no prefix match, no case folding.
        for f in &[
            "_", "_keys", "_Key", "_ID", "__key", "_id_x", "_created", "_rev_",
        ] {
            assert!(
                !super::is_safe_serialised_field(f),
                "{} should be filtered",
                f
            );
        }
        assert!(super::is_safe_serialised_field(""));
    }

    #[test]
    fn safe_serialised_field_keeps_innocent_lookalikes() {
        // `token_count` doesn't end in `_token`; `digest_format` doesn't end in `_digest`.
        // The suffix match is anchored, so legitimate fields aren't false-positives.
        assert!(super::is_safe_serialised_field("token_count"));
        assert!(super::is_safe_serialised_field("digest_format"));
        assert!(super::is_safe_serialised_field("hash_algorithm"));
    }

    // ---------------------------------------------------------------------
    // Memory-size regression guards.
    //
    // `size_of::<Value>()` is paid by every array cell, hash slot, model-row
    // field, env binding, and VM stack slot; the AST node sizes are paid by
    // every parsed statement/expression. These bounds
    // lock in the memory-footprint reductions so a fat inline variant can't
    // silently creep back. Run `cargo test size_guards -- --nocapture` to see
    // the live numbers. Tighten the bounds when a reduction lands.
    // ---------------------------------------------------------------------
    #[test]
    fn size_guards() {
        use crate::ast::expr::{Expr, ExprKind};
        use crate::ast::stmt::{Stmt, StmtKind};
        use crate::ast::types::TypeAnnotation;
        use crate::span::Span;
        use std::mem::size_of;

        eprintln!(
            "size_of::<Value>()          = {}",
            size_of::<super::Value>()
        );
        eprintln!(
            "size_of::<NativeFunction>() = {}",
            size_of::<super::NativeFunction>()
        );
        eprintln!(
            "size_of::<ValueMethod>()    = {}",
            size_of::<super::ValueMethod>()
        );
        eprintln!("size_of::<Span>()           = {}", size_of::<Span>());
        eprintln!("size_of::<Expr>()           = {}", size_of::<Expr>());
        eprintln!("size_of::<ExprKind>()       = {}", size_of::<ExprKind>());
        eprintln!("size_of::<Stmt>()           = {}", size_of::<Stmt>());
        eprintln!("size_of::<StmtKind>()       = {}", size_of::<StmtKind>());
        eprintln!(
            "size_of::<TypeAnnotation>() = {}",
            size_of::<TypeAnnotation>()
        );
    }

    /// Functions built from one declaration share its parsed body — the
    /// workers of a server each build their own `Function`s from the one
    /// cached AST, and a copy per worker was most of what a worker cost.
    #[test]
    fn functions_share_the_parsed_body() {
        use crate::ast::{Body, StmtKind};
        use crate::interpreter::environment::Environment;
        use std::cell::RefCell;
        use std::rc::Rc;

        let program = crate::parser::Parser::new(
            crate::lexer::Scanner::new("def greet(name)\n  \"hi \" + name\nend\n")
                .scan_tokens()
                .expect("lexer error"),
        )
        .parse()
        .expect("parser error");
        let StmtKind::Function(decl) = &program.statements[0].kind else {
            panic!("expected a function declaration");
        };
        let env = || Rc::new(RefCell::new(Environment::new()));
        let one = super::Function::from_decl(decl, env(), None);
        let two = super::Function::from_decl(decl, env(), None);
        assert!(Body::ptr_eq(&one.body, &decl.body));
        assert!(Body::ptr_eq(&one.body, &two.body));

        // A rewrite copies a shared body rather than changing it under the
        // functions that run it.
        let mut edited = decl.body.clone();
        edited.make_mut();
        assert!(!Body::ptr_eq(&edited, &decl.body));
        assert_eq!(edited, decl.body);
        assert!(Body::ptr_eq(&one.body, &decl.body));
    }
}

/// Compile-time memory-size guards — see the `size_guards` test for context.
/// These fail the build if a `Value` variant grows past its budget. Keep them
/// as tight as the current layout allows; loosen only with a deliberate reason.
/// Every array cell, hash slot, model-row field, env binding and VM stack slot
/// is one `Value`, so this is a broad multiplier.
const _: () = assert!(std::mem::size_of::<Value>() <= 24);

#[cfg(test)]
mod sensitive_name_tests {
    use super::is_sensitive_field_name;

    /// One definition of "sensitive", used by Model serialisation and by the
    /// error-path environment dump. The second consumer is why this matters
    /// beyond cosmetics: that dump reaches the `env:` line of the production
    /// error log, so a name this misses is a secret written to disk.
    #[test]
    fn sensitive_names_are_recognised() {
        for name in [
            "password",
            "password_digest",
            "password_hash",
            "PasswordHash",
            "password_reset_token",
            "api_token",
            "session_secret",
            "csrf_token",
            "content_hash",
            "SESSION_SECRET",
        ] {
            assert!(
                is_sensitive_field_name(name),
                "{name} should be treated as sensitive"
            );
        }
    }

    /// The allocation-free matcher must answer exactly what the old
    /// lowercase-then-compare one did, ASCII case folding and non-ASCII names
    /// included.
    #[test]
    fn matches_the_lowercasing_reference() {
        fn reference(name: &str) -> bool {
            let lower = name.to_ascii_lowercase();
            lower.starts_with("password")
                || lower.ends_with("_token")
                || lower.ends_with("_digest")
                || lower.ends_with("_secret")
                || lower.ends_with("_hash")
        }
        for name in [
            "",
            "p",
            "passwor",
            "password",
            "PASSWORD",
            "pAsSwOrD_x",
            "passwordHash",
            "my_password",
            "_token",
            "token",
            "x_TOKEN",
            "x_Digest",
            "a_secreT",
            "a_hasH",
            "_hash",
            "hash",
            "content_hashes",
            "tokens",
            "mot_de_passe",
            "pässword",
            "clé_token",
            "ünïcode_HASH",
            "token_ß",
            "\u{0}_token",
        ] {
            assert_eq!(
                is_sensitive_field_name(name),
                reference(name),
                "disagreement on {name:?}"
            );
        }
    }

    /// It must not swallow ordinary names — over-redacting an error log makes
    /// it useless in the other direction.
    #[test]
    fn ordinary_names_are_left_alone() {
        for name in [
            "user_name",
            "email",
            "id",
            "tokens",
            "hashes",
            "secretary",
            "count",
        ] {
            assert!(
                !is_sensitive_field_name(name),
                "{name} should NOT be treated as sensitive"
            );
        }
    }
}

#[cfg(test)]
mod interpolation_format_tests {
    use super::{build_interpolated, write_float, Value};

    fn via_ryu(n: f64) -> String {
        let mut out = String::new();
        write_float(n, &mut out).unwrap();
        out
    }

    /// Interpolation must print a float byte-for-byte as `str()`/Display does;
    /// the ryu fast path is only allowed to be faster, never different.
    #[test]
    fn ryu_float_path_matches_display() {
        let edges = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.1 + 0.2,
            2.5,
            12.375,
            1e15,
            1e16,
            1e17,
            123456789012345680.0,
            1e-4,
            1e-5,
            1e-7,
            5e-324,
            2.2250738585072014e-308,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
            f64::EPSILON,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for n in edges {
            assert_eq!(via_ryu(n), format!("{}", n), "bits {:#x}", n.to_bits());
        }
        // xorshift over raw bit patterns: every exponent, sign and mantissa shape.
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..200_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let n = f64::from_bits(state);
            assert_eq!(via_ryu(n), format!("{}", n), "bits {:#x}", state);
        }
        // Short decimals — the fast path — at every magnitude: prices,
        // ratios, measurements, integral floats.
        for _ in 0..500_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mantissa = (state % 1_000_000_000_000_000) as i64 >> (state % 50);
            let scale = 10f64.powi((state >> 56) as i32 % 40 - 20);
            let n = mantissa as f64 * scale;
            assert_eq!(via_ryu(n), format!("{}", n), "bits {:#x}", n.to_bits());
            assert_eq!(via_ryu(-n), format!("{}", -n));
        }
        for i in -100_000i64..100_000 {
            for divisor in [1.0, 8.0, 100.0, 1000.0] {
                let n = i as f64 / divisor;
                assert_eq!(via_ryu(n), format!("{}", n));
            }
        }
    }

    #[test]
    fn build_interpolated_matches_display_on_both_paths() {
        // Known length within the inline limit: built straight into `SoliStr`.
        let short = [
            Value::String("v=".into()),
            Value::Int(-42),
            Value::Symbol("ok".into()),
            Value::Null,
        ];
        assert_eq!(build_interpolated(&short).as_str(), "v=-42:oknull");
        // One byte over, and exactly at, the limit.
        let edge = [Value::String("x".repeat(15).into()), Value::Bool(true)];
        assert_eq!(
            build_interpolated(&edge).as_str(),
            format!("{}true", "x".repeat(15))
        );
        let exact = [Value::String("x".repeat(13).into()), Value::Int(10)];
        assert_eq!(
            build_interpolated(&exact).as_str(),
            format!("{}10", "x".repeat(13))
        );

        let parts = [
            Value::String("a".into()),
            Value::Int(-42),
            Value::Float(1.5),
            Value::Bool(true),
            Value::Null,
        ];
        let built = build_interpolated(&parts);
        assert_eq!(built.as_str(), "a-421.5truenull");

        let long = [Value::String("x".repeat(40).into()), Value::Float(1e20)];
        // Crossing the stack buffer mid-part, and multi-byte UTF-8 on both sides.
        let wide = [
            Value::String("é".repeat(60).into()),
            Value::Int(7),
            Value::String("ü".repeat(100).into()),
        ];
        assert_eq!(
            build_interpolated(&wide).as_str(),
            format!("{}7{}", "é".repeat(60), "ü".repeat(100))
        );
        assert_eq!(
            build_interpolated(&long).as_str(),
            format!("{}{}", "x".repeat(40), 1e20)
        );
    }
}

#[cfg(test)]
mod restricted_member_tests {
    //! `Class::restricted_method` answers a name no class of the chain
    //! declares private or protected from a memo, without its walk — and every
    //! answer it gives is still the walk's.
    use super::*;

    fn class(name: &str, superclass: Option<&Rc<Class>>) -> Rc<Class> {
        Rc::new(Class {
            name: name.to_string(),
            superclass: superclass.cloned(),
            ..Default::default()
        })
    }

    fn declare_private(class: &Class, name: &str) {
        class.private_members.borrow_mut().insert(name.to_string());
        note_restricted_methods_declared();
    }

    fn walks() -> usize {
        restricted_lookups().1
    }

    #[test]
    fn a_name_no_class_of_the_chain_restricts_skips_the_walk() {
        let base = class("Base", None);
        declare_private(&base, "token");
        let child = class("Child", Some(&base));
        let plain = class("Plain", None);

        let (lookups, walked) = restricted_lookups();
        for _ in 0..3 {
            assert_eq!(child.restricted_method("title"), None);
            assert_eq!(plain.restricted_method("title"), None);
        }
        assert_eq!(restricted_lookups(), (lookups + 6, walked));

        assert_eq!(
            child.restricted_method("token"),
            Some((MethodAccess::Private, "Base"))
        );
        assert_eq!(walks(), walked + 1);
    }

    #[test]
    fn the_nearest_definition_still_decides() {
        let base = class("Base", None);
        declare_private(&base, "reveal");
        base.protected_members
            .borrow_mut()
            .insert("rank".to_string());
        note_restricted_methods_declared();
        let open = class("Open", Some(&base));
        open.methods
            .borrow_mut()
            .insert("reveal".to_string(), Rc::new(Function::default()));
        let below = class("Below", Some(&open));

        assert_eq!(
            base.restricted_method("reveal"),
            Some((MethodAccess::Private, "Base"))
        );
        assert_eq!(open.restricted_method("reveal"), None);
        assert_eq!(below.restricted_method("reveal"), None);
        assert_eq!(
            below.restricted_method("rank"),
            Some((MethodAccess::Protected, "Base"))
        );
    }

    /// A member restricted after a subclass's memo was collected — a
    /// `private` reaching a class again, a `private class` nested in it — is
    /// seen: each addition moves the epoch.
    #[test]
    fn a_member_restricted_after_the_memo_is_seen() {
        let base = class("Base", None);
        let child = class("Child", Some(&base));
        declare_private(&base, "first");
        assert_eq!(child.restricted_method("late"), None);
        assert_eq!(child.restricted_method("Ledger"), None);

        declare_private(&base, "late");
        assert_eq!(
            child.restricted_method("late"),
            Some((MethodAccess::Private, "Base"))
        );

        base.private_members
            .borrow_mut()
            .insert("Ledger".to_string());
        note_restricted_members_changed();
        assert_eq!(
            child.restricted_method("Ledger"),
            Some((MethodAccess::Private, "Base"))
        );
    }

    /// A clone of a class whose memo is collected, given a superclass that
    /// restricts a name: the clone answers for its own chain. A copied memo
    /// would still say that no class of the chain restricts anything.
    #[test]
    fn a_cloned_class_collects_its_own_memo() {
        let guarded = class("Guarded", None);
        declare_private(&guarded, "token");
        let plain = class("Plain", None);
        assert_eq!(plain.restricted_method("token"), None);

        let mut reparented = (*plain).clone();
        reparented.superclass = Some(guarded);
        assert_eq!(
            reparented.restricted_method("token"),
            Some((MethodAccess::Private, "Guarded"))
        );
        assert_eq!(plain.restricted_method("token"), None);
    }
}

#[cfg(test)]
mod construction_tests {
    //! What a construction asks of its class on every `new`, answered once.
    use super::*;

    fn class(name: &str, superclass: Option<&Rc<Class>>) -> Rc<Class> {
        Rc::new(Class {
            name: name.to_string(),
            superclass: superclass.cloned(),
            ..Default::default()
        })
    }

    /// `Klass.new` asks `intercepts_new` on every call. The answer is kept
    /// per class until a static method joins some class, and is then
    /// computed again: a static `new` an ancestor gains (`extend`) is the
    /// subclass's too, so the subclass's answer flips with it.
    #[test]
    fn intercepts_new_is_memoized_until_a_static_joins_the_chain() {
        let base = class("Base", None);
        let child = class("Child", Some(&base));
        let other = class("Other", None);
        assert!(child.intercepts_new());
        assert!(child.intercepts_new(), "the memo gives the same answer");
        assert!(
            child.intercepts_new_memo.0.get().1,
            "the answer is kept on the class"
        );

        let factory = Rc::new(Class {
            name: "Factory".to_string(),
            is_module: true,
            ..Default::default()
        });
        factory.methods.borrow_mut().insert(
            "new".to_string(),
            Rc::new(Function {
                name: "new".to_string(),
                ..Function::default()
            }),
        );
        base.extend_module(&factory)
            .expect("a module extends a class");
        assert!(!base.intercepts_new(), "Base.new is Factory's now");
        assert!(
            !child.intercepts_new(),
            "an ancestor's static `new` is the subclass's too"
        );
        assert!(
            other.intercepts_new(),
            "an unrelated class keeps its answer"
        );

        let model = class("Model", None);
        let post = class("Post", Some(&model));
        assert!(!post.intercepts_new(), "a model's `new` is the ORM's");
    }

    /// A new instance has room for every field its class and their
    /// ancestors declare: a construction sets all of them before any
    /// constructor runs. A class the VM compiled lists its own in
    /// `declared_fields` only; a model row passes its column count.
    #[test]
    fn a_new_instance_has_room_for_every_declared_field() {
        let parent = Rc::new(Class {
            name: "Parent".to_string(),
            fields: ["a", "b", "c"]
                .iter()
                .map(|name| (name.to_string(), None))
                .collect(),
            ..Default::default()
        });
        *parent.declared_fields.borrow_mut() = vec!["a".into(), "b".into(), "c".into()];
        let kid = class("Kid", Some(&parent));
        *kid.declared_fields.borrow_mut() = (1..=6).map(|i| format!("k{i}")).collect();

        assert_eq!(kid.declared_field_count(), 9);
        assert!(
            Instance::new(kid.clone()).fields.capacity() >= 9,
            "the six own fields of a VM class and the three inherited ones"
        );
        assert!(Instance::with_capacity(kid.clone(), 20).fields.capacity() >= 20);
        assert_eq!(
            Instance::new(class("Plain", None)).fields.capacity(),
            0,
            "no declared field, no allocation"
        );
    }
}

#[cfg(test)]
mod datetime_text_tests {
    use super::{DateTimeText, Value};
    use crate::interpreter::builtins::datetime::local_zone::{local_from_nanos, offset_secs_in};
    use chrono::{DateTime, TimeZone, Utc};
    use chrono_tz::Tz;

    const FMT: &str = "%Y-%m-%d %H:%M:%S";
    const SEC: i64 = 1_000_000_000;

    fn via_chrono_utc(ts: i64) -> String {
        DateTime::from_timestamp_nanos(ts).format(FMT).to_string()
    }

    /// Edges chrono and a hand-rolled civil-date routine most often disagree
    /// on: the epoch, a nanosecond either side of it, pre-1970 instants, the
    /// i64 bounds, leap days and century years.
    fn edge_instants() -> Vec<i64> {
        let mut out = vec![0, -1, 1, -SEC, -SEC + 1, SEC - 1, i64::MIN, i64::MAX];
        for (y, m, d) in [
            (1677, 9, 22),
            (1700, 2, 28),
            (1700, 3, 1),
            (1800, 12, 31),
            (1900, 2, 28),
            (1900, 3, 1),
            (1969, 12, 31),
            (2000, 2, 29),
            (2000, 3, 1),
            (2024, 2, 29),
            (2100, 2, 28),
            (2100, 3, 1),
            (2262, 4, 10),
        ] {
            let midnight = Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap().timestamp() * SEC;
            for delta in [-SEC, -1, 0, 1, SEC, 86_399 * SEC + 999_999_999] {
                out.push(midnight + delta);
            }
        }
        out
    }

    #[test]
    fn utc_view_matches_chrono_strftime() {
        for ts in edge_instants() {
            let text = DateTimeText::new(ts, 0);
            assert_eq!(text.as_str(), via_chrono_utc(ts), "nanos {ts}");
        }
        // A sweep across the whole i64 range (~57k instants).
        let step: i64 = 0x0001_2345_6789_abcd;
        let mut ts = i64::MIN;
        while ts <= i64::MAX - step {
            assert_eq!(
                DateTimeText::new(ts, 0).as_str(),
                via_chrono_utc(ts),
                "nanos {ts}"
            );
            ts += step;
        }
    }

    /// The local view: the offset rule must match `with_timezone` at every
    /// instant, DST transitions, half-hour and 45-minute zones included.
    #[test]
    fn zone_offsets_match_chrono_tz_across_transitions() {
        let zones = [
            "America/New_York",
            "Europe/London",
            "Australia/Lord_Howe", // 30-minute DST shift
            "Asia/Kolkata",        // +05:30, no DST
            "Pacific/Chatham",     // +12:45 / +13:45
            "America/St_Johns",    // -03:30 / -02:30
            "Pacific/Apia",        // skipped 2011-12-30 entirely
            "America/Havana",      // DST at midnight
        ];
        let from = Utc
            .with_ymd_and_hms(2011, 1, 1, 0, 0, 0)
            .unwrap()
            .timestamp();
        let to = Utc
            .with_ymd_and_hms(2013, 1, 1, 0, 0, 0)
            .unwrap()
            .timestamp();
        for name in zones {
            let tz: Tz = name.parse().unwrap();
            // Every 15 minutes for two years: each transition is hit exactly.
            let mut secs = from;
            while secs < to {
                for ts in [secs * SEC, secs * SEC - 1] {
                    let utc = DateTime::from_timestamp_nanos(ts);
                    let expected = utc.with_timezone(&tz).format(FMT).to_string();
                    let offset = offset_secs_in(&tz, &utc.naive_utc());
                    assert_eq!(
                        DateTimeText::new(ts, offset).as_str(),
                        expected,
                        "{name} at {utc}"
                    );
                }
                secs += 900;
            }
        }
        // Pre-1970 local mean time offsets have seconds in them (e.g.
        // Europe/Amsterdam +00:19:32): the second digits must follow too.
        let tz: Tz = "Europe/Amsterdam".parse().unwrap();
        for ts in edge_instants() {
            let utc = DateTime::from_timestamp_nanos(ts);
            let offset = offset_secs_in(&tz, &utc.naive_utc());
            assert_eq!(
                DateTimeText::new(ts, offset).as_str(),
                utc.with_timezone(&tz).format(FMT).to_string(),
                "Amsterdam at {utc}"
            );
        }
    }

    /// Every caller — `str()`/Display, interpolation, `.to_string` — writes
    /// the same 19 bytes, and `display_len` agrees with them, in both views.
    #[test]
    fn all_string_paths_agree_in_both_views() {
        for ts in edge_instants() {
            for use_utc in [true, false] {
                let expected = if use_utc {
                    via_chrono_utc(ts)
                } else {
                    local_from_nanos(ts).format(FMT).to_string()
                };
                let value = Value::DateTime(ts, use_utc);
                let mut written = String::from(">");
                value.write_to_string(&mut written);
                assert_eq!(written, format!(">{expected}"));
                assert_eq!(value.to_string(), expected);
                assert_eq!(Value::render_datetime(ts, use_utc), expected);
                assert_eq!(value.display_len(), expected.len());
            }
        }
    }
}

#[cfg(test)]
mod method_cache_tests {
    use super::{Class, Function};
    use std::rc::Rc;

    fn method(name: &str) -> Rc<Function> {
        Rc::new(Function {
            name: name.to_string(),
            ..Function::default()
        })
    }

    /// A subclass's flattened cache copies its parent's methods. A method
    /// added to the parent after that copy was made must still be found
    /// through the subclass (and the grandchild), not only through the parent.
    #[test]
    fn method_added_to_a_parent_reaches_cached_subclasses() {
        let parent = Rc::new(Class {
            name: "A".to_string(),
            ..Default::default()
        });
        parent
            .methods
            .borrow_mut()
            .insert("hello".to_string(), method("hello"));
        let child = Rc::new(Class {
            name: "B".to_string(),
            superclass: Some(parent.clone()),
            ..Default::default()
        });
        let grandchild = Class {
            name: "C".to_string(),
            superclass: Some(child.clone()),
            ..Default::default()
        };

        // Build every cache along the chain.
        assert!(grandchild.find_method("hello").is_some());
        assert!(grandchild.find_method("late").is_none());
        assert!(child.find_method("late").is_none());

        parent
            .methods
            .borrow_mut()
            .insert("late".to_string(), method("late"));
        parent.invalidate_method_cache();

        assert_eq!(child.find_method("late").unwrap().name, "late");
        assert_eq!(grandchild.find_method("late").unwrap().name, "late");
        assert!(grandchild.find_method("hello").is_some());
    }

    /// With no definition in between, the cache is reused, not rebuilt.
    #[test]
    fn cache_is_kept_while_no_method_changes() {
        let class = Class {
            name: "Kept".to_string(),
            ..Default::default()
        };
        class
            .methods
            .borrow_mut()
            .insert("a".to_string(), method("a"));
        let first = class.find_method("a").unwrap();
        let epoch = class.all_methods_cache_epoch.get();
        let second = class.find_method("a").unwrap();
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(class.all_methods_cache_epoch.get(), epoch);
    }
}

#[cfg(test)]
mod concat_and_equality_tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// `"str" + x` and `x + "str"` render `x` exactly as `Display` does (what
    /// `eco_format!("{}{}")` produced before), on the inline and the heap
    /// path, for every kind of value a string can be added to.
    #[test]
    fn concat_display_matches_display_for_every_operand_kind() {
        let array = Value::Array(Rc::new(RefCell::new(vec![
            Value::Int(1),
            Value::Float(2.5),
            Value::String("s".into()),
        ])));
        let decimal = Value::Decimal(DecimalValue(rust_decimal::Decimal::new(150, 2), 2));
        let others = [
            Value::Int(0),
            Value::Int(-42),
            Value::Int(i64::MIN),
            Value::Int(i64::MAX),
            Value::Float(1.0),
            Value::Float(-0.0),
            Value::Float(0.1 + 0.2),
            Value::Float(1e16),
            Value::Float(1e-7),
            Value::Float(1658206780088562.2),
            Value::Float(f64::NAN),
            Value::Float(f64::INFINITY),
            Value::Bool(true),
            Value::Bool(false),
            Value::Null,
            Value::Symbol("sym".into()),
            decimal,
            array,
        ];
        for prefix in ["", "n=", "a prefix longer than the inline limit: "] {
            let text = Value::String(prefix.into());
            for other in &others {
                assert_eq!(
                    concat_display(&text, other).as_str(),
                    format!("{}{}", prefix, other),
                    "{prefix:?} + {other:?}"
                );
                assert_eq!(
                    concat_display(other, &text).as_str(),
                    format!("{}{}", other, prefix),
                    "{other:?} + {prefix:?}"
                );
            }
        }
    }

    fn instance(class: &Rc<Class>, fields: &[(&str, Value)]) -> Value {
        let mut inst = Instance::new(class.clone());
        for (name, value) in fields {
            inst.set(*name, value.clone());
        }
        Value::Instance(Rc::new(RefCell::new(inst)))
    }

    fn class(name: &str) -> Rc<Class> {
        Rc::new(Class {
            name: name.to_string(),
            ..Class::default()
        })
    }

    /// The identity shortcut answers for ordinary objects only: an enum value
    /// still compares field by field, even against itself.
    #[test]
    fn enum_aware_equal_identity_shortcut_keeps_enum_semantics() {
        let point = class("Point");
        let p1 = instance(&point, &[("x", Value::Int(1))]);
        let p2 = instance(&point, &[("x", Value::Int(1))]);
        assert!(enum_aware_equal(&p1, &p1.clone()));
        assert!(!enum_aware_equal(&p1, &p2));

        let maybe = class("Maybe");
        let some = |v: Value| {
            instance(
                &maybe,
                &[("__variant", Value::String("Some".into())), ("v", v)],
            )
        };
        assert!(enum_aware_equal(&some(Value::Int(7)), &some(Value::Int(7))));
        assert!(!enum_aware_equal(
            &some(Value::Int(7)),
            &some(Value::Int(8))
        ));
        let nan = some(Value::Float(f64::NAN));
        assert!(!enum_aware_equal(&nan, &nan.clone()));
        let same = some(Value::Float(1.5));
        assert!(enum_aware_equal(&same, &same.clone()));

        // An enum against an ordinary object, either way round, is identity.
        let unit = instance(&maybe, &[("__variant", Value::String("None".into()))]);
        assert!(!enum_aware_equal(&unit, &p1));
        assert!(!enum_aware_equal(&p1, &unit));
    }
}
