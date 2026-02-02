//! Runtime values for the Solilang interpreter.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;

use crate::ast::{Expr, FunctionDecl, MethodDecl, Parameter, Stmt};
use crate::interpreter::builtins::model::QueryBuilder;
use crate::interpreter::environment::Environment;
use crate::span::Span;

/// A hashable key type for use in IndexMap.
/// This wraps primitive Value types that can be used as hash keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashKey {
    Int(i64),
    String(String),
    Bool(bool),
    Null,
}

impl Hash for HashKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            HashKey::Int(n) => n.hash(state),
            HashKey::String(s) => s.hash(state),
            HashKey::Bool(b) => b.hash(state),
            HashKey::Null => {}
        }
    }
}

impl HashKey {
    /// Convert a Value to a HashKey if possible.
    pub fn from_value(value: &Value) -> Option<HashKey> {
        match value {
            Value::Int(n) => Some(HashKey::Int(*n)),
            Value::String(s) => Some(HashKey::String(s.clone())),
            Value::Bool(b) => Some(HashKey::Bool(*b)),
            Value::Null => Some(HashKey::Null),
            // Floats are not hashable due to NaN != NaN issues
            _ => None,
        }
    }

    /// Convert back to a Value.
    pub fn to_value(&self) -> Value {
        match self {
            HashKey::Int(n) => Value::Int(*n),
            HashKey::String(s) => Value::String(s.clone()),
            HashKey::Bool(b) => Value::Bool(*b),
            HashKey::Null => Value::Null,
        }
    }
}

impl std::fmt::Display for HashKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashKey::Int(n) => write!(f, "{}", n),
            HashKey::String(s) => write!(f, "{}", s),
            HashKey::Bool(b) => write!(f, "{}", b),
            HashKey::Null => write!(f, "null"),
        }
    }
}

/// Helper function to create a hash Value from string key-value pairs.
/// This is a convenience function for creating hashes in builtin functions.
pub fn hash_from_pairs<I>(pairs: I) -> Value
where
    I: IntoIterator<Item = (String, Value)>,
{
    let map: IndexMap<HashKey, Value> = pairs
        .into_iter()
        .map(|(k, v)| (HashKey::String(k), v))
        .collect();
    Value::Hash(Rc::new(RefCell::new(map)))
}

/// Helper function to create an empty hash Value.
pub fn empty_hash() -> Value {
    Value::Hash(Rc::new(RefCell::new(IndexMap::new())))
}

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
    /// Hash/Map value (ordered, O(1) lookup using IndexMap)
    Hash(Rc<RefCell<IndexMap<HashKey, Value>>>),
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
    /// Query builder for chainable database queries
    QueryBuilder(Rc<RefCell<QueryBuilder>>),
    /// Super reference - used for super.method() calls, carries the superclass
    Super(Rc<Class>),
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
    pub fn type_name(&self) -> String {
        match self {
            Value::Int(_) => "int".to_string(),
            Value::Float(_) => "float".to_string(),
            Value::String(_) => "string".to_string(),
            Value::Bool(_) => "bool".to_string(),
            Value::Null => "null".to_string(),
            Value::Array(_) => "array".to_string(),
            Value::Hash(_) => "hash".to_string(),
            Value::Function(_) => "Function".to_string(),
            Value::NativeFunction(_) => "Function".to_string(),
            Value::Class(_) => "Class".to_string(),
            Value::Instance(inst) => inst.borrow().class.name.clone(),
            Value::Future(_) => "Future".to_string(),
            Value::Method(_) => "Method".to_string(),
            Value::Breakpoint => "Breakpoint".to_string(),
            Value::QueryBuilder(_) => "QueryBuilder".to_string(),
            Value::Super(_) => "Super".to_string(),
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
    /// Note: Floats are excluded because NaN != NaN breaks hash map invariants.
    pub fn is_hashable(&self) -> bool {
        matches!(
            self,
            Value::Int(_) | Value::String(_) | Value::Bool(_) | Value::Null
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
                    write!(f, "{} => {}", key.to_value(), val)?;
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
            Value::Method(method) => write!(
                f,
                "<method {}.{}>",
                method.receiver.type_name(),
                method.method_name
            ),
            Value::Breakpoint => write!(f, "<breakpoint>"),
            Value::QueryBuilder(qb) => {
                let qb = qb.borrow();
                if qb.filter.is_some() {
                    write!(f, "<QueryBuilder for {} with filter>", qb.class_name)
                } else {
                    write!(f, "<QueryBuilder for {}>", qb.class_name)
                }
            }
            Value::Super(class) => write!(f, "<super of {}>", class.name),
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
    pub methods: HashMap<String, Rc<Function>>,
    pub static_methods: HashMap<String, Rc<Function>>,
    pub native_static_methods: HashMap<String, Rc<NativeFunction>>,
    pub native_methods: HashMap<String, Rc<NativeFunction>>,
    pub static_fields: Rc<RefCell<HashMap<String, Value>>>,
    pub fields: HashMap<String, Option<Expr>>,
    pub constructor: Option<Rc<Function>>,
    /// Nested classes defined within this class - using RefCell for interior mutability
    pub nested_classes: Rc<RefCell<HashMap<String, Rc<Class>>>>,
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
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
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
            methods,
            static_methods,
            native_static_methods,
            native_methods,
            static_fields,
            fields,
            constructor,
            nested_classes,
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
        all_methods.extend(self.methods.clone());

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
        if let Some(func) = self.class.methods.get(name) {
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
pub fn json_to_value(json: &serde_json::Value) -> Result<Value, String> {
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
            let mut map = IndexMap::new();
            for (k, v) in obj {
                map.insert(HashKey::String(k.clone()), json_to_value(v)?);
            }
            Ok(Value::Hash(Rc::new(RefCell::new(map))))
        }
    }
}

/// Convert a Soli Value to serde_json::Value.
pub fn value_to_json(value: &Value) -> Result<serde_json::Value, String> {
    match value {
        Value::Int(n) => Ok(serde_json::Value::Number(serde_json::Number::from(*n))),
        Value::Float(f) => Ok(serde_json::Value::Number(
            serde_json::Number::from_f64(*f).ok_or_else(|| "Invalid float".to_string())?,
        )),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Null => Ok(serde_json::Value::Null),
        Value::Array(arr) => {
            let borrow = arr.borrow();
            let vec: Result<Vec<serde_json::Value>, String> =
                borrow.iter().map(value_to_json).collect();
            vec.map(serde_json::Value::Array)
        }
        Value::Hash(hash) => {
            let borrow = hash.borrow();
            let mut map = serde_json::Map::new();
            for (k, v) in borrow.iter() {
                if let HashKey::String(key) = k {
                    map.insert(key.clone(), value_to_json(v)?);
                }
            }
            Ok(serde_json::Value::Object(map))
        }
        Value::Instance(inst) => {
            let borrow = inst.borrow();
            let mut map = serde_json::Map::new();
            for (k, v) in borrow.fields.iter() {
                map.insert(k.clone(), value_to_json(v)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        _ => Err(format!("Cannot convert {} to JSON", value.type_name())),
    }
}

/// Unwrap a value, extracting the underlying value from class instances.
/// For String/Array/Hash class instances, returns the __value field.
/// For other values, returns the value as-is.
pub fn unwrap_value(value: &Value) -> Value {
    match value {
        Value::Instance(inst) => {
            if let Some(inner) = inst.borrow().fields.get("__value").cloned() {
                unwrap_value(&inner)
            } else {
                value.clone()
            }
        }
        _ => value.clone(),
    }
}
