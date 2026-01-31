//! Runtime environment for variable scopes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::Value;

/// A runtime environment containing variable bindings.
#[derive(Debug, Clone)]
pub struct Environment {
    values: HashMap<String, Value>,
    consts: HashMap<String, Value>,
    enclosing: Option<Rc<RefCell<Environment>>>,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            consts: HashMap::new(),
            enclosing: None,
        }
    }

    pub fn with_enclosing(enclosing: Rc<RefCell<Environment>>) -> Self {
        Self {
            values: HashMap::new(),
            consts: HashMap::new(),
            enclosing: Some(enclosing),
        }
    }

    /// Define a new variable in the current scope.
    pub fn define(&mut self, name: String, value: Value) {
        self.values.insert(name, value);
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
    pub fn get(&self, name: &str) -> Option<Value> {
        // Check consts first (const shadows let)
        if let Some(value) = self.consts.get(name) {
            return Some(value.clone());
        }
        if let Some(value) = self.values.get(name) {
            return Some(value.clone());
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
    pub fn assign(&mut self, name: &str, value: Value) -> bool {
        if self.consts.contains_key(name) {
            return false;
        }
        if self.values.contains_key(name) {
            self.values.insert(name.to_string(), value);
            return true;
        }
        if let Some(ref enclosing) = self.enclosing {
            return enclosing.borrow_mut().assign(name, value);
        }
        false
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
