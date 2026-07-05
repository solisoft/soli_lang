//! Named scopes for models. A scope is a closure that produces (or refines)
//! a QueryBuilder for a given model class. Registered via `Model.add_scope`
//! and accessed as `MyModel.scope_name`.
//!
//! Storage is thread-local because the closure captures interpreter state
//! (`Rc<Function>` is `!Send`) and can't go in the global `MODEL_REGISTRY`.
//! Each worker thread registers scopes independently when it loads model
//! files.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::Function;

thread_local! {
    static SCOPES: RefCell<HashMap<(String, String), Rc<Function>>> =
        RefCell::new(HashMap::new());
}

/// Register a named scope for a model class. Re-registration overwrites.
pub fn register_scope(class_name: &str, scope_name: &str, func: Rc<Function>) {
    SCOPES.with(|s| {
        s.borrow_mut()
            .insert((class_name.to_string(), scope_name.to_string()), func);
    });
}

/// Look up a scope closure on `(class_name, scope_name)`. Returns `None` if
/// no scope is registered.
pub fn lookup_scope(class_name: &str, scope_name: &str) -> Option<Rc<Function>> {
    SCOPES.with(|s| {
        s.borrow()
            .get(&(class_name.to_string(), scope_name.to_string()))
            .cloned()
    })
}

/// STI copy-down: register every scope of `parent` under `child` too, so
/// `Admin.recent` resolves when `recent` was declared on `User`. The child's
/// own `scope(...)` calls run afterward and overwrite same-named entries.
pub fn copy_scopes(parent: &str, child: &str) {
    SCOPES.with(|s| {
        let mut map = s.borrow_mut();
        let inherited: Vec<(String, Rc<Function>)> = map
            .iter()
            .filter(|((class, _), _)| class == parent)
            .map(|((_, scope), func)| (scope.clone(), func.clone()))
            .collect();
        for (scope, func) in inherited {
            map.insert((child.to_string(), scope), func);
        }
    });
}

/// All registered scope names for a class, used by REPL completion and
/// reflection.
pub fn scopes_for(class_name: &str) -> Vec<String> {
    SCOPES.with(|s| {
        s.borrow()
            .keys()
            .filter(|(c, _)| c == class_name)
            .map(|(_, n)| n.clone())
            .collect()
    })
}
