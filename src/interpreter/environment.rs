//! Runtime environment for variable scopes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::Value;

/// A runtime environment containing variable bindings.
#[derive(Debug, Clone)]
pub struct Environment {
    values: HashMap<String, Value>,
    enclosing: Option<Rc<RefCell<Environment>>>,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            enclosing: None,
        }
    }

    pub fn with_enclosing(enclosing: Rc<RefCell<Environment>>) -> Self {
        Self {
            values: HashMap::new(),
            enclosing: Some(enclosing),
        }
    }

    /// Define a new variable in the current scope.
    pub fn define(&mut self, name: String, value: Value) {
        self.values.insert(name, value);
    }

    /// Get a variable's value, searching up the scope chain.
    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(value) = self.values.get(name) {
            return Some(value.clone());
        }
        if let Some(ref enclosing) = self.enclosing {
            return enclosing.borrow().get(name);
        }
        None
    }

    /// Assign to an existing variable, searching up the scope chain.
    pub fn assign(&mut self, name: &str, value: Value) -> bool {
        if self.values.contains_key(name) {
            self.values.insert(name.to_string(), value);
            return true;
        }
        if let Some(ref enclosing) = self.enclosing {
            return enclosing.borrow_mut().assign(name, value);
        }
        false
    }

    /// Check if a variable exists in the current scope only.
    pub fn contains_local(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// Get the enclosing environment.
    pub fn enclosing(&self) -> Option<Rc<RefCell<Environment>>> {
        self.enclosing.clone()
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}
