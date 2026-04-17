//! Runtime environment for variable scopes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ahash::AHashMap;

use crate::interpreter::value::{HashPairs, StrKey, Value};

/// Result of an `Environment::assign` call.
///
/// Distinguishes "hit a const" from "not defined anywhere" so the caller can
/// either surface a reassignment error or fall through to `define` — without
/// paying for a separate chain walk via `is_const` beforehand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignResult {
    Assigned,
    IsConst,
    NotFound,
}

/// A runtime environment containing variable bindings.
///
/// Internal storage uses `ahash::AHashMap` rather than `std::HashMap` (SipHash)
/// — variable lookups are on the hot path of every expression evaluation, and
/// ahash is ~3× faster for short string keys like identifiers.
#[derive(Debug, Clone)]
pub struct Environment {
    values: AHashMap<String, Value>,
    consts: AHashMap<String, Value>,
    enclosing: Option<Rc<RefCell<Environment>>>,
    /// Optional data hash for template rendering.
    /// Checked during get() before walking the enclosing chain.
    /// Avoids copying all data fields into the HashMap.
    data_hash: Option<Rc<RefCell<HashPairs>>>,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            values: AHashMap::new(),
            consts: AHashMap::new(),
            enclosing: None,
            data_hash: None,
        }
    }

    /// Create an environment pre-sized for builtin registration (~300 entries).
    /// Avoids repeated rehashing during `register_builtins()`.
    pub fn with_builtins_capacity() -> Self {
        Self {
            values: AHashMap::with_capacity(300),
            consts: AHashMap::new(),
            enclosing: None,
            data_hash: None,
        }
    }

    pub fn with_enclosing(enclosing: Rc<RefCell<Environment>>) -> Self {
        Self {
            values: AHashMap::new(),
            consts: AHashMap::new(),
            enclosing: Some(enclosing),
            data_hash: None,
        }
    }

    /// Create an environment with a data hash for template rendering.
    /// Variables are looked up in the data hash before walking the enclosing chain,
    /// avoiding the need to copy all data fields into the values HashMap.
    pub fn with_enclosing_and_data(
        enclosing: Rc<RefCell<Environment>>,
        data_hash: Rc<RefCell<HashPairs>>,
    ) -> Self {
        Self {
            values: AHashMap::new(),
            consts: AHashMap::new(),
            enclosing: Some(enclosing),
            data_hash: Some(data_hash),
        }
    }

    /// Reset this environment for reuse in template rendering.
    /// Clears local variables (keeps HashMap capacity) and updates the data hash.
    #[inline]
    pub fn reset_for_reuse(&mut self, data_hash: Option<Rc<RefCell<HashPairs>>>) {
        self.values.clear();
        self.data_hash = data_hash;
    }

    /// Reset this environment for reuse as a function call frame.
    /// Clears locals and constants (keeping HashMap capacity) while preserving
    /// the enclosing-chain pointer, so the cached env can serve another call
    /// of the same function.
    #[inline]
    pub fn reset_for_call(&mut self) {
        self.values.clear();
        self.consts.clear();
    }

    /// Define a new variable in the current scope.
    pub fn define(&mut self, name: String, value: Value) {
        self.values.insert(name, value);
    }

    /// Update an existing variable or define it if not found.
    /// Avoids String allocation for the key when the variable already exists.
    /// Ideal for loop variables that are redefined every iteration.
    #[inline]
    pub fn define_or_update(&mut self, name: &str, value: Value) {
        if let Some(existing) = self.values.get_mut(name) {
            *existing = value;
        } else {
            self.values.insert(name.to_string(), value);
        }
    }

    /// Define a constant in the current scope.
    pub fn define_const(&mut self, name: String, value: Value) {
        self.consts.insert(name, value);
    }

    /// Check if a name is a constant.
    pub fn is_const(&self, name: &str) -> bool {
        if self.consts.contains_key(name) {
            return true;
        }
        if let Some(ref enclosing) = self.enclosing {
            return enclosing.borrow().is_const(name);
        }
        false
    }

    /// Get a variable's value, searching up the scope chain.
    /// Consts are checked first so they shadow let variables with the same name.
    /// Data hash (if set) is checked before walking the enclosing chain.
    pub fn get(&self, name: &str) -> Option<Value> {
        // Fast path: most function-call envs have no consts declared locally,
        // so skip the hash lookup entirely when the map is empty. Saves one
        // string hash on every variable access in a hot loop.
        if !self.consts.is_empty() {
            if let Some(value) = self.consts.get(name) {
                return Some(value.clone());
            }
        }
        if let Some(value) = self.values.get(name) {
            return Some(value.clone());
        }
        // Check data hash (template data, avoids copying into values HashMap)
        if let Some(ref hash) = self.data_hash {
            if let Some(value) = hash.borrow().get(&StrKey(name)) {
                return Some(value.clone());
            }
        }
        if let Some(ref enclosing) = self.enclosing {
            return enclosing.borrow().get(name);
        }
        None
    }

    /// Get a constant's value, searching up the scope chain.
    pub fn get_const(&self, name: &str) -> Option<Value> {
        if let Some(value) = self.consts.get(name) {
            return Some(value.clone());
        }
        if let Some(ref enclosing) = self.enclosing {
            return enclosing.borrow().get_const(name);
        }
        None
    }

    /// Assign to an existing variable, searching up the scope chain.
    /// Returns false if the variable is const.
    pub fn assign(&mut self, name: &str, value: Value) -> AssignResult {
        if !self.consts.is_empty() && self.consts.contains_key(name) {
            return AssignResult::IsConst;
        }
        if let Some(slot) = self.values.get_mut(name) {
            *slot = value;
            return AssignResult::Assigned;
        }
        if let Some(ref enclosing) = self.enclosing {
            return enclosing.borrow_mut().assign(name, value);
        }
        AssignResult::NotFound
    }

    /// Check if a variable exists in the current scope only (values or consts).
    pub fn contains_local(&self, name: &str) -> bool {
        self.values.contains_key(name) || self.consts.contains_key(name)
    }

    /// Get all variable names in the current scope (for REPL introspection).
    pub fn get_var_names(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }

    /// Get a variable from local scope only (no parent chain traversal).
    /// Useful when you know the variable should be in the current scope.
    #[inline]
    pub fn get_local(&self, name: &str) -> Option<Value> {
        self.consts
            .get(name)
            .or_else(|| self.values.get(name))
            .cloned()
    }

    /// Assign a value to a variable, or define it if not found.
    /// This is useful for loop variables that need to be reassigned each iteration.
    /// Returns false only if the variable is a constant.
    pub fn assign_or_define(&mut self, name: &str, value: Value) -> bool {
        if self.consts.contains_key(name) {
            return false;
        }
        // Always define in local scope - this is for loop variables
        self.values.insert(name.to_string(), value);
        true
    }

    /// Get the enclosing environment.
    pub fn enclosing(&self) -> Option<Rc<RefCell<Environment>>> {
        self.enclosing.clone()
    }

    /// Get all bindings (variables + constants) from this scope and all enclosing scopes.
    /// Used by VM integration to copy interpreter globals into VM globals.
    pub fn get_all_bindings(&self) -> HashMap<String, Value> {
        let mut all = HashMap::new();
        if let Some(ref enclosing) = self.enclosing {
            all.extend(enclosing.borrow().get_all_bindings());
        }
        all.extend(self.values.clone());
        all.extend(self.consts.clone());
        all
    }

    /// Get all variables from this scope and all enclosing scopes.
    /// Used for debugging (breakpoints).
    pub fn get_all_variables(&self) -> HashMap<String, Value> {
        let mut all_vars = HashMap::new();

        // First get variables from enclosing scopes (so local ones can override)
        if let Some(ref enclosing) = self.enclosing {
            all_vars.extend(enclosing.borrow().get_all_variables());
        }

        // Then add/override with variables from current scope
        all_vars.extend(self.values.clone());

        all_vars
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}
