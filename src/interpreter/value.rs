//! Runtime values for the Solilang interpreter.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use crate::ast::{Expr, FunctionDecl, MethodDecl, Parameter, Stmt};
use crate::interpreter::environment::Environment;

/// A runtime value in Solilang.
#[derive(Debug, Clone)]
pub enum Value {
    /// Integer value
    Int(i64),
    /// Floating point value
    Float(f64),
    /// String value
    String(String),
    /// Boolean value
    Bool(bool),
    /// Null value
    Null,
    /// Array value
    Array(Rc<RefCell<Vec<Value>>>),
    /// Hash/Map value (ordered, like Ruby)
    Hash(Rc<RefCell<Vec<(Value, Value)>>>),
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
        }
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::String(_) => "String",
            Value::Bool(_) => "Bool",
            Value::Null => "Null",
            Value::Array(_) => "Array",
            Value::Hash(_) => "Hash",
            Value::Function(_) => "Function",
            Value::NativeFunction(_) => "Function",
            Value::Class(_) => "Class",
            Value::Instance(_) => "Instance",
            Value::Future(_) => "Future",
            Value::Method(_) => "Method",
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
            Value::String(s) if s.is_empty() => false,
            Value::Array(arr) if arr.borrow().is_empty() => false,
            Value::Hash(hash) if hash.borrow().is_empty() => false,
            Value::Future(_) => true, // Futures are truthy (they represent pending work)
            _ => true,
        }
    }

    /// Check if this value can be used as a hash key (must be comparable).
    pub fn is_hashable(&self) -> bool {
        matches!(
            self,
            Value::Int(_) | Value::Float(_) | Value::String(_) | Value::Bool(_) | Value::Null
        )
    }

    /// Value equality for hash key comparison.
    pub fn hash_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            _ => false,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Hash(a), Value::Hash(b)) => Rc::ptr_eq(a, b),
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
            Value::String(s) => write!(f, "{}", s),
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
                    write!(f, "{} => {}", key, val)?;
                }
                write!(f, "}}")
            }
            Value::Function(func) => write!(f, "<fn {}>", func.name),
            Value::NativeFunction(func) => write!(f, "<native fn {}>", func.name),
            Value::Class(class) => write!(f, "<class {}>", class.name),
            Value::Instance(inst) => write!(f, "<{} instance>", inst.borrow().class.name),
            Value::Future(state) => {
                // Auto-resolve the future when displaying
                let guard = state.lock().unwrap();
                match &*guard {
                    FutureState::Pending { .. } => write!(f, "<pending future>"),
                    FutureState::Resolved(val) => write!(f, "{}", val),
                    FutureState::Error(e) => write!(f, "<error: {}>", e),
                }
            }
            Value::Method(method) => write!(f, "<method {}.{}>", method.receiver.type_name(), method.method_name),
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
}

impl Function {
    pub fn from_decl(decl: &FunctionDecl, closure: Rc<RefCell<Environment>>) -> Self {
        Self {
            name: decl.name.clone(),
            params: decl.params.clone(),
            body: decl.body.clone(),
            closure,
            is_method: false,
        }
    }

    pub fn from_method(decl: &MethodDecl, closure: Rc<RefCell<Environment>>) -> Self {
        Self {
            name: decl.name.clone(),
            params: decl.params.clone(),
            body: decl.body.clone(),
            closure,
            is_method: true,
        }
    }

    pub fn arity(&self) -> usize {
        // Return the number of required parameters (params without defaults)
        self.params.iter().filter(|p| p.default_value.is_none()).count()
    }

    /// Full arity including optional parameters
    pub fn full_arity(&self) -> usize {
        self.params.len()
    }

    /// Check if a parameter at index has a default value
    pub fn param_has_default(&self, index: usize) -> bool {
        self.params.get(index).map(|p| p.default_value.is_some()).unwrap_or(false)
    }

    /// Get the default value expression for a parameter at index
    pub fn param_default_value(&self, index: usize) -> Option<&Expr> {
        self.params.get(index).and_then(|p| p.default_value.as_ref())
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
        write!(f, "<method {}.{}>", self.receiver.type_name(), self.method_name)
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
    pub methods: HashMap<String, Rc<Function>>,
    pub static_methods: HashMap<String, Rc<Function>>,
    pub constructor: Option<Rc<Function>>,
}

impl Class {
    pub fn find_method(&self, name: &str) -> Option<Rc<Function>> {
        if let Some(method) = self.methods.get(name) {
            return Some(method.clone());
        }
        if let Some(ref superclass) = self.superclass {
            return superclass.find_method(name);
        }
        None
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
}

/// Convert raw HTTP response data to a Value based on the future kind.
/// For FullResponse, the raw_data is a JSON-encoded response object.
fn convert_future_result(raw_data: &str, kind: &HttpFutureKind) -> Result<Value, String> {
    match kind {
        HttpFutureKind::String => Ok(Value::String(raw_data.to_string())),
        HttpFutureKind::Json => {
            // Parse JSON string into Value
            match serde_json::from_str::<serde_json::Value>(raw_data) {
                Ok(json) => json_to_value(&json),
                Err(e) => Err(format!("Failed to parse JSON: {}", e)),
            }
        }
        HttpFutureKind::FullResponse => {
            // Parse the JSON-encoded full response
            match serde_json::from_str::<serde_json::Value>(raw_data) {
                Ok(json) => json_to_value(&json),
                Err(e) => Err(format!("Failed to parse response: {}", e)),
            }
        }
    }
}

/// Convert a serde_json::Value to a Soli Value.
fn json_to_value(json: &serde_json::Value) -> Result<Value, String> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Err("Invalid JSON number".to_string())
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<Value>, String> = arr.iter().map(json_to_value).collect();
            Ok(Value::Array(Rc::new(RefCell::new(items?))))
        }
        serde_json::Value::Object(obj) => {
            let pairs: Result<Vec<(Value, Value)>, String> = obj
                .iter()
                .map(|(k, v)| Ok((Value::String(k.clone()), json_to_value(v)?)))
                .collect();
            Ok(Value::Hash(Rc::new(RefCell::new(pairs?))))
        }
    }
}
