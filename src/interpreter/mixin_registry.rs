//! Side table so the VM can attach `included do` / `class_methods` metadata
//! when a module is created from bytecode (the AST is gone by then).

use std::cell::RefCell;
use std::collections::HashMap;

use crate::ast::Stmt;

#[derive(Clone, Default)]
pub struct ModuleHookRecord {
    pub included: Vec<Vec<Stmt>>,
    pub extended: Vec<Vec<Stmt>>,
    pub concern_method_names: Vec<String>,
}

thread_local! {
    static HOOKS: RefCell<HashMap<String, ModuleHookRecord>> = RefCell::new(HashMap::new());
}

pub fn register(name: String, record: ModuleHookRecord) {
    HOOKS.with(|h| {
        h.borrow_mut().insert(name, record);
    });
}

pub fn take(name: &str) -> Option<ModuleHookRecord> {
    HOOKS.with(|h| h.borrow_mut().remove(name))
}
