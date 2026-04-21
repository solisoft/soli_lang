//! Runtime values for the Solilang interpreter.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use ahash::RandomState as AHasher;
use indexmap::IndexMap;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::ser::{SerializeMap, SerializeSeq};

use crate::ast::{Expr, FunctionDecl, MethodDecl, Parameter, Stmt, TypeAnnotation};
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

/// A hashable key type for use in IndexMap.
/// This wraps primitive Value types that can be used as hash keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashKey {
    Int(i64),
    Decimal(DecimalValue), // Hashable Decimal
    String(String),
    Bool(bool),
    Null,
    Symbol(String),
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
        matches!(key, HashKey::String(s) if s.as_str() == self.0)
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
        matches!(key, HashKey::Symbol(s) if s.as_str() == self.0)
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
        }
    }

    #[inline]
    pub fn display_len(&self) -> usize {
        match self {
            HashKey::Int(n) => itoa::Buffer::new().format(*n).len(),
            HashKey::Decimal(d) => d.to_string().len(),
            HashKey::String(s) => s.len() + 2,
            HashKey::Bool(b) => {
                if *b {
                    4
                } else {
                    5
                }
            }
            HashKey::Null => 4,
            HashKey::Symbol(s) => s.len() + 1,
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
        }
    }
}

/// Helper function to create a hash Value from string key-value pairs.
/// This is a convenience function for creating hashes in builtin functions.
pub fn hash_from_pairs<I>(pairs: I) -> Value
where
    I: IntoIterator<Item = (String, Value)>,
{
    let map: HashPairs = pairs
        .into_iter()
        .map(|(k, v)| (HashKey::String(k), v))
        .collect();
    Value::Hash(Rc::new(RefCell::new(map)))
}

/// Helper function to create an empty hash Value.
pub fn empty_hash() -> Value {
    Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
}

/// Type alias for hash map storage — uses ahash for 3-5x faster hashing than SipHash.
pub type HashPairs = IndexMap<HashKey, Value, AHasher>;

#[inline]
pub fn hash_get_value<'a>(hash: &'a HashPairs, key: &Value) -> Option<&'a Value> {
    match key {
        Value::String(s) => hash.get(&StrKey(s)),
        Value::Int(n) => hash.get(&HashKey::Int(*n)),
        Value::Decimal(d) => hash.get(&HashKey::Decimal(d.clone())),
        Value::Bool(b) => hash.get(&HashKey::Bool(*b)),
        Value::Null => hash.get(&HashKey::Null),
        Value::Symbol(s) => hash.get(&SymKey(s)),
        _ => None,
    }
}

#[inline]
pub fn hash_contains_value(hash: &HashPairs, key: &Value) -> bool {
    hash_get_value(hash, key).is_some()
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
    /// String value
    String(String),
    /// Symbol value (:name)
    Symbol(String),
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
    /// Future value (async result that auto-resolves when used)
    Future(Arc<Mutex<FutureState>>),
    /// Method on a value (array/hash) - captures receiver and method name
    Method(ValueMethod),
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
}

/// The type of HTTP future result
#[derive(Clone)]
pub enum HttpFutureKind {
    /// Returns body as string
    String,
    /// Returns parsed JSON
    Json,
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
            HttpFutureKind::FullResponse => write!(f, "FullResponse"),
            HttpFutureKind::SystemResult => write!(f, "SystemResult"),
        }
    }
}

impl Value {
    pub fn type_name(&self) -> String {
        match self {
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
        }
    }

    /// Append this value's string representation directly into `out`.
    /// Avoids the intermediate `to_string()` allocation and the `fmt::Display`
    /// machinery for primitive types — used on the string-interpolation hot path.
    #[inline]
    pub fn append_to_string(&self, out: &mut String) {
        use std::fmt::Write;
        match self {
            Value::String(s) => out.push_str(s),
            Value::Int(n) => out.push_str(itoa::Buffer::new().format(*n)),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Null => out.push_str("null"),
            Value::Symbol(s) => {
                out.push(':');
                out.push_str(s);
            }
            other => {
                let _ = write!(out, "{}", other);
            }
        }
    }

    /// Resolve a Future value, blocking until the result is ready.
    /// For non-Future values, returns the value unchanged.
    pub fn resolve(self) -> Result<Value, String> {
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

    pub fn is_truthy(&self) -> bool {
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
            _ => true,
        }
    }

    /// Check if this value can be used as a hash key (must be comparable).
    /// Note: Floats are excluded because NaN != NaN breaks hash map invariants.
    pub fn is_hashable(&self) -> bool {
        matches!(
            self,
            Value::Int(_)
                | Value::Decimal(_)
                | Value::String(_)
                | Value::Symbol(_)
                | Value::Bool(_)
                | Value::Null
        )
    }

    /// Convert this value to a HashKey if possible.
    pub fn to_hash_key(&self) -> Option<HashKey> {
        HashKey::from_value(self)
    }

    /// Value equality for hash key comparison (legacy method, kept for compatibility).
    pub fn hash_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Decimal(a), Value::Decimal(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            _ => false,
        }
    }

    #[inline]
    pub fn display_len(&self) -> usize {
        match self {
            Value::Int(n) => n.to_string().len(),
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
        }
    }

    #[inline]
    pub fn write_to_string(&self, s: &mut String) {
        match self {
            Value::Int(n) => s.push_str(&n.to_string()),
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
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
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
            (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
            (Value::Method(a), Value::Method(b)) => {
                *a.receiver == *b.receiver && a.method_name == b.method_name
            }
            _ => false,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
                if inst_ref.fields.is_empty() {
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
        }
    }
}

/// A user-defined function.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Parameter>,
    pub body: Vec<Stmt>,
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
}

impl Default for Function {
    fn default() -> Self {
        Self {
            name: String::new(),
            params: Vec::new(),
            body: Vec::new(),
            closure: Rc::new(RefCell::new(Environment::new())),
            is_method: false,
            span: None,
            source_path: None,
            defining_superclass: None,
            return_type: None,
            cached_env: RefCell::new(None),
            jit_cache: RefCell::new(None),
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
            params: decl.params.clone(),
            body: decl.body.clone(),
            closure,
            is_method: false,
            span: Some(decl.span),
            source_path,
            defining_superclass: None,
            return_type: decl.return_type.clone(),
            cached_env: RefCell::new(None),
            jit_cache: RefCell::new(None),
        }
    }

    pub fn from_method(
        decl: &MethodDecl,
        closure: Rc<RefCell<Environment>>,
        source_path: Option<String>,
    ) -> Self {
        Self {
            name: decl.name.clone(),
            params: decl.params.clone(),
            body: decl.body.clone(),
            closure,
            is_method: true,
            span: Some(decl.span),
            source_path,
            defining_superclass: None,
            return_type: decl.return_type.clone(),
            cached_env: RefCell::new(None),
            jit_cache: RefCell::new(None),
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

    /// Check if a parameter at index has a default value
    pub fn param_has_default(&self, index: usize) -> bool {
        self.params
            .get(index)
            .map(|p| p.default_value.is_some())
            .unwrap_or(false)
    }

    /// Get the default value expression for a parameter at index
    pub fn param_default_value(&self, index: usize) -> Option<&Expr> {
        self.params
            .get(index)
            .and_then(|p| p.default_value.as_ref())
    }
}

/// A native/builtin function.
#[derive(Clone)]
pub struct NativeFunction {
    pub name: String,
    pub arity: Option<usize>, // None means variadic
    pub func: Rc<dyn Fn(Vec<Value>) -> Result<Value, String>>,
}

impl NativeFunction {
    pub fn new<F>(name: impl Into<String>, arity: Option<usize>, func: F) -> Self
    where
        F: Fn(Vec<Value>) -> Result<Value, String> + 'static,
    {
        Self {
            name: name.into(),
            arity,
            func: Rc::new(func),
        }
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

/// Kinds of value methods for arrays
#[derive(Clone, Copy, Debug)]
pub enum ArrayMethodKind {
    Map,
    Filter,
    Each,
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
    /// NOTE: Should not be manually set; use Class::new() constructor instead.
    pub all_methods_cache: RefCell<Option<HashMap<String, Rc<Function>>>>,
    /// Flattened native method cache for O(1) lookups.
    /// NOTE: Should not be manually set; use Class::new() constructor instead.
    pub all_native_methods_cache: RefCell<Option<HashMap<String, Rc<NativeFunction>>>>,
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
            all_native_methods_cache: RefCell::new(None),
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
            all_native_methods_cache: RefCell::new(None),
        }
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

    /// Build the flattened method cache if not already built.
    fn ensure_methods_cached(&self) {
        // Fast path: check if already cached without borrowing mutably
        if self.all_methods_cache.borrow().is_some() {
            return;
        }

        // Build flattened method map
        let mut all_methods = HashMap::new();

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
    }

    /// Build the flattened native method cache if not already built.
    fn ensure_native_methods_cached(&self) {
        // Fast path: check if already cached without borrowing mutably
        if self.all_native_methods_cache.borrow().is_some() {
            return;
        }

        // Build flattened native method map
        let mut all_native_methods = HashMap::new();

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

    /// Find a static method in this class or its superclass chain.
    pub fn find_static_method(&self, name: &str) -> Option<Rc<Function>> {
        if let Some(method) = self.static_methods.get(name) {
            return Some(method.clone());
        }
        if let Some(ref superclass) = self.superclass {
            return superclass.find_static_method(name);
        }
        None
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
    pub fn is_model_subclass(&self) -> bool {
        if self.name == "Model" {
            return true;
        }
        if let Some(ref superclass) = self.superclass {
            return superclass.is_model_subclass();
        }
        false
    }
}

/// A class instance.
#[derive(Debug, Clone)]
pub struct Instance {
    pub class: Rc<Class>,
    pub fields: HashMap<String, Value>,
}

impl Instance {
    pub fn new(class: Rc<Class>) -> Self {
        Self {
            class,
            fields: HashMap::new(),
        }
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.fields.get(name).cloned()
    }

    pub fn set(&mut self, name: String, value: Value) {
        self.fields.insert(name, value);
    }

    pub fn get_method(&self, name: &str) -> Option<Value> {
        // Check instance fields first
        if let Some(value) = self.fields.get(name) {
            return Some(value.clone());
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
        HttpFutureKind::String => Ok(Value::String(raw_data.to_string())),
        HttpFutureKind::Json => {
            // Parse JSON string into Value
            match serde_json::from_str::<serde_json::Value>(raw_data) {
                Ok(json) => json_to_value(json),
                Err(e) => Err(format!("Failed to parse JSON: {}", e)),
            }
        }
        HttpFutureKind::FullResponse => {
            // Parse the JSON-encoded full response
            match serde_json::from_str::<serde_json::Value>(raw_data) {
                Ok(json) => json_to_value(json),
                Err(e) => Err(format!("Failed to parse response: {}", e)),
            }
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
                        HashKey::String("stdout".to_string()),
                        Value::String(data.stdout),
                    );
                    hash.insert(
                        HashKey::String("stderr".to_string()),
                        Value::String(data.stderr),
                    );
                    hash.insert(
                        HashKey::String("exit_code".to_string()),
                        Value::Int(data.exit_code as i64),
                    );
                    Ok(Value::Hash(Rc::new(RefCell::new(hash))))
                }
                Err(e) => Err(format!("Failed to parse SystemResult: {}", e)),
            }
        }
    }
}

/// Convert a serde_json::Value to a Soli Value (consuming — moves strings instead of cloning).
pub fn json_to_value(json: serde_json::Value) -> Result<Value, String> {
    crate::interpreter::value_json::json_to_value(json)
}

/// Convert a serde_json::Value reference to a Soli Value (clones strings).
pub fn json_to_value_ref(json: &serde_json::Value) -> Result<Value, String> {
    crate::interpreter::value_json::json_to_value_ref(json)
}

/// Convert a Soli Value to serde_json::Value.
pub fn value_to_json(value: &Value) -> Result<serde_json::Value, String> {
    crate::interpreter::value_json::value_to_json(value)
}

/// Implement serde::Serialize for Value to leverage serde_json's optimized writer.
impl serde::Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
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
                let mut map = serializer.serialize_map(Some(borrow.fields.len()))?;
                for (k, v) in borrow.fields.iter() {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            _ => Err(serde::ser::Error::custom(format!(
                "Cannot convert {} to JSON",
                self.type_name()
            ))),
        }
    }
}

/// Serialize a Value to a JSON string using sonic-rs SIMD-accelerated writer.
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

/// Fast i64 parsing — avoids the overhead of str::parse for the common case.
#[inline(always)]
fn fast_parse_i64(b: &[u8]) -> Value {
    let (neg, start) = if b[0] == b'-' { (true, 1) } else { (false, 0) };
    let mut n: i64 = 0;
    let mut i = start;
    while i < b.len() {
        let d = (b[i] - b'0') as i64;
        match n.checked_mul(10).and_then(|n| n.checked_add(d)) {
            Some(v) => n = v,
            None => {
                // Overflow — fall back to f64
                let s = unsafe { std::str::from_utf8_unchecked(b) };
                return Value::Float(s.parse::<f64>().unwrap_or(0.0));
            }
        }
        i += 1;
    }
    Value::Int(if neg { -n } else { n })
}

/// Hand-rolled JSON parser — builds Value directly in one pass.
/// No serde, no intermediate tree, no trait dispatch overhead.
pub fn parse_json(s: &str) -> Result<Value, String> {
    let bytes = s.as_bytes();
    let mut pos = 0;
    let value = parse_value(bytes, &mut pos)?;
    skip_ws(bytes, &mut pos);
    if pos < bytes.len() {
        return Err(format!("Trailing content at position {}", pos));
    }
    Ok(value)
}

/// Parse JSON from bytes.
pub fn parse_json_bytes(bytes: &[u8]) -> Result<Value, String> {
    let mut pos = 0;
    let value = parse_value(bytes, &mut pos)?;
    skip_ws(bytes, &mut pos);
    if pos < bytes.len() {
        return Err(format!("Trailing content at position {}", pos));
    }
    Ok(value)
}

#[inline(always)]
fn skip_ws(b: &[u8], pos: &mut usize) {
    while *pos < b.len() {
        match b[*pos] {
            b' ' | b'\t' | b'\n' | b'\r' => *pos += 1,
            _ => break,
        }
    }
}

#[inline(always)]
fn peek(b: &[u8], pos: &mut usize) -> Result<u8, String> {
    skip_ws(b, pos);
    if *pos < b.len() {
        Ok(b[*pos])
    } else {
        Err("Unexpected end of JSON".to_string())
    }
}

fn parse_value(b: &[u8], pos: &mut usize) -> Result<Value, String> {
    match peek(b, pos)? {
        b'"' => parse_string(b, pos).map(Value::String),
        b'{' => parse_object(b, pos),
        b'[' => parse_array(b, pos),
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
    if b[*pos..].starts_with(expected) {
        *pos += expected.len();
        Ok(value)
    } else {
        Err(format!("Invalid literal at position {}", *pos))
    }
}

fn parse_number(b: &[u8], pos: &mut usize) -> Result<Value, String> {
    let start = *pos;
    let mut is_float = false;

    if *pos < b.len() && b[*pos] == b'-' {
        *pos += 1;
    }
    if *pos >= b.len() || !b[*pos].is_ascii_digit() {
        return Err(format!("Invalid number at position {}", start));
    }
    if b[*pos] == b'0' {
        *pos += 1;
    } else {
        while *pos < b.len() && b[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }
    if *pos < b.len() && b[*pos] == b'.' {
        is_float = true;
        *pos += 1;
        if *pos >= b.len() || !b[*pos].is_ascii_digit() {
            return Err(format!("Invalid number at position {}", start));
        }
        while *pos < b.len() && b[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }
    if *pos < b.len() && (b[*pos] == b'e' || b[*pos] == b'E') {
        is_float = true;
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

    // SAFETY: We only advanced past ASCII digits, '.', 'e', 'E', '+', '-'
    let num_str = unsafe { std::str::from_utf8_unchecked(&b[start..*pos]) };

    if is_float {
        num_str
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|e| format!("Invalid float: {}", e))
    } else {
        // Fast path: hand-rolled i64 parse for common case
        Ok(fast_parse_i64(num_str.as_bytes()))
    }
}

fn parse_string(b: &[u8], pos: &mut usize) -> Result<String, String> {
    *pos += 1; // skip opening '"'
    let start = *pos;

    // Fast path: scan for end quote with no escapes using memchr-style scan
    while *pos < b.len() {
        let c = b[*pos];
        if c == b'"' {
            // No escapes found — direct allocation from slice
            let s = unsafe { String::from_utf8_unchecked(b[start..*pos].to_vec()) };
            *pos += 1;
            return Ok(s);
        }
        if c == b'\\' {
            break; // has escapes, use slow path
        }
        *pos += 1;
    }

    // Slow path: build string with escape handling
    let mut result = String::from(unsafe { std::str::from_utf8_unchecked(&b[start..*pos]) });
    while *pos < b.len() {
        match b[*pos] {
            b'"' => {
                *pos += 1;
                return Ok(result);
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
                result.push(b[*pos] as char);
                *pos += 1;
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
    let hex = unsafe { std::str::from_utf8_unchecked(&b[*pos..*pos + 4]) };
    let val = u16::from_str_radix(hex, 16).map_err(|_| "Invalid hex in \\u escape".to_string())?;
    *pos += 4;
    Ok(val)
}

fn parse_array(b: &[u8], pos: &mut usize) -> Result<Value, String> {
    *pos += 1; // skip '['
    if peek(b, pos)? == b']' {
        *pos += 1;
        return Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))));
    }
    let mut items = Vec::with_capacity(8);
    loop {
        items.push(parse_value(b, pos)?);
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

fn parse_object(b: &[u8], pos: &mut usize) -> Result<Value, String> {
    *pos += 1; // skip '{'
    if peek(b, pos)? == b'}' {
        *pos += 1;
        return Ok(Value::Hash(Rc::new(RefCell::new(HashPairs::with_hasher(
            AHasher::default(),
        )))));
    }
    // Pre-allocate for typical JSON objects (6 fields)
    let mut pairs = HashPairs::with_capacity_and_hasher(6, AHasher::default());
    loop {
        if peek(b, pos)? != b'"' {
            return Err(format!("Expected string key at position {}", *pos));
        }
        let key = parse_string(b, pos)?;
        if peek(b, pos)? != b':' {
            return Err(format!("Expected ':' at position {}", *pos));
        }
        *pos += 1;
        let value = parse_value(b, pos)?;
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
pub fn value_matches_type(value: &Value, expected: &TypeAnnotation) -> bool {
    match &expected.kind {
        TypeKind::Named(name) => {
            let name_lower = name.to_lowercase();
            match name_lower.as_str() {
                "any" => true,
                "int" => matches!(value, Value::Int(_)),
                "float" => matches!(value, Value::Float(_)),
                "decimal" => matches!(value, Value::Decimal(_)),
                "string" => matches!(value, Value::String(_)),
                "bool" => matches!(value, Value::Bool(_)),
                "array" => matches!(value, Value::Array(_)),
                "hash" => matches!(value, Value::Hash(_)),
                "function" => matches!(value, Value::Function(_) | Value::NativeFunction(_)),
                "void" | "null" => matches!(value, Value::Null),
                // Class instance check
                _ => match value {
                    Value::Instance(inst) => inst.borrow().class.name == *name,
                    _ => false,
                },
            }
        }
        TypeKind::Void => matches!(value, Value::Null),
        TypeKind::Nullable(inner) => {
            matches!(value, Value::Null) || value_matches_type(value, inner)
        }
        TypeKind::Array(_) => matches!(value, Value::Array(_)),
        TypeKind::Hash { .. } => matches!(value, Value::Hash(_)),
        TypeKind::Function { .. } => {
            matches!(value, Value::Function(_) | Value::NativeFunction(_))
        }
    }
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
    fn test_json_to_value_decimal_string() {
        let json = serde_json::Value::String("19.99".to_string());
        let result = json_to_value(json);

        assert!(result.is_ok());
        let value = result.unwrap();

        match value {
            Value::Decimal(dv) => {
                assert_eq!(dv.to_string(), "19.99");
            }
            _ => panic!("Expected Decimal value, got {:?}", value.type_name()),
        }
    }

    #[test]
    fn test_json_to_value_decimal_string_precision() {
        let json = serde_json::Value::String("0.0675".to_string());
        let result = json_to_value(json);

        assert!(result.is_ok());
        let value = result.unwrap();

        match value {
            Value::Decimal(dv) => {
                assert_eq!(dv.precision(), 4);
                assert_eq!(dv.to_string(), "0.0675");
            }
            _ => panic!("Expected Decimal value"),
        }
    }

    #[test]
    fn test_json_to_value_decimal_integer_string() {
        let json = serde_json::Value::String("100".to_string());
        let result = json_to_value(json);

        assert!(result.is_ok());
        let value = result.unwrap();

        match value {
            Value::Decimal(dv) => {
                assert_eq!(dv.precision(), 0);
                assert_eq!(dv.to_string(), "100");
            }
            _ => panic!("Expected Decimal value"),
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
    fn test_value_decimal_is_hashable() {
        let decimal = Decimal::from_str("19.99").unwrap();
        let decimal_value = DecimalValue(decimal, 2);
        let value = Value::Decimal(decimal_value);

        assert!(value.is_hashable());
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
        let test_cases = vec![
            ("0.1", 1),
            ("0.01", 2),
            ("0.001", 3),
            ("0.0001", 4),
            ("123.45", 2),
            ("1000", 0),
        ];

        for (input, expected_precision) in test_cases {
            let json = serde_json::Value::String(input.to_string());
            let result = json_to_value(json);

            assert!(result.is_ok(), "Failed for input: {}", input);
            let value = result.unwrap();

            match value {
                Value::Decimal(dv) => {
                    assert_eq!(
                        dv.precision(),
                        expected_precision,
                        "Precision mismatch for input: {}",
                        input
                    );
                }
                _ => panic!("Expected Decimal value for input: {}", input),
            }
        }
    }

    #[test]
    fn test_decimal_zero_values() {
        let zero_values = vec!["0", "0.0", "0.00", "0.000"];

        for input in zero_values {
            let json = serde_json::Value::String(input.to_string());
            let result = json_to_value(json);

            assert!(result.is_ok(), "Failed for zero input: {}", input);
            let value = result.unwrap();

            match value {
                Value::Decimal(dv) => {
                    let dv_str = dv.to_string();
                    assert!(
                        dv_str == "0" || dv_str == "0.0" || dv_str == "0.00" || dv_str == "0.000",
                        "Unexpected zero format for input {}: got {}",
                        input,
                        dv_str
                    );
                }
                _ => panic!("Expected Decimal value for zero input: {}", input),
            }
        }
    }

    #[test]
    fn test_decimal_negative_values() {
        let json = serde_json::Value::String("-19.99".to_string());
        let result = json_to_value(json);

        assert!(result.is_ok());
        let value = result.unwrap();

        match value {
            Value::Decimal(dv) => {
                assert_eq!(dv.to_string(), "-19.99");
            }
            _ => panic!("Expected Decimal value for negative input"),
        }
    }

    #[test]
    fn test_decimal_large_values() {
        let json = serde_json::Value::String("9999999999.99".to_string());
        let result = json_to_value(json);

        assert!(result.is_ok());
        let value = result.unwrap();

        match value {
            Value::Decimal(dv) => {
                assert_eq!(dv.to_string(), "9999999999.99");
            }
            _ => panic!("Expected Decimal value for large input"),
        }
    }

    #[test]
    fn test_decimal_in_array_json() {
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
                    Value::Decimal(dv) => assert_eq!(dv.to_string(), "10.00"),
                    _ => panic!("Expected Decimal in array"),
                }
            }
            _ => panic!("Expected Array value"),
        }
    }

    #[test]
    fn test_decimal_in_hash_json() {
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
                let price_key = HashKey::String("price".to_string());

                if let Some(price_value) = hash.get(&price_key) {
                    match price_value {
                        Value::Decimal(dv) => assert_eq!(dv.to_string(), "19.99"),
                        _ => panic!("Expected Decimal value for price"),
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
        let value = Value::String("hello".to_string());
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
        assert!(value_matches_type(&Value::String("x".to_string()), &ty));
        assert!(value_matches_type(&Value::Null, &ty));
        assert!(value_matches_type(&Value::Bool(true), &ty));
    }

    #[test]
    fn test_nullable_accepts_null() {
        let inner = make_type(TypeKind::Named("Int".to_string()));
        let ty = make_type(TypeKind::Nullable(Box::new(inner)));
        assert!(value_matches_type(&Value::Null, &ty));
        assert!(value_matches_type(&Value::Int(42), &ty));
        assert!(!value_matches_type(&Value::String("x".to_string()), &ty));
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
}
