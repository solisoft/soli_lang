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

/// One model class's scopes, by scope name.
type ClassScopes = HashMap<String, Rc<Function>, ahash::RandomState>;

/// The registry is keyed class -> (scope -> closure) so a lookup borrows both
/// `&str` keys (`String: Borrow<str>`): probing it allocates nothing. The VM
/// probes it on every model-class member read, model-class method call and
/// query-builder method call, almost always for a name that is no scope.
#[derive(Default)]
struct Registry {
    classes: HashMap<String, ClassScopes, ahash::RandomState>,
}

impl Registry {
    fn get(&self, class_name: &str, scope_name: &str) -> Option<&Rc<Function>> {
        if self.classes.is_empty() {
            return None;
        }
        self.classes.get(class_name)?.get(scope_name)
    }

    fn insert(&mut self, class_name: &str, scope_name: &str, func: Rc<Function>) {
        if let Some(scopes) = self.classes.get_mut(class_name) {
            scopes.insert(scope_name.to_string(), func);
            return;
        }
        let mut scopes = ClassScopes::default();
        scopes.insert(scope_name.to_string(), func);
        self.classes.insert(class_name.to_string(), scopes);
    }
}

thread_local! {
    static SCOPES: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Register a named scope for a model class. Re-registration overwrites.
pub fn register_scope(class_name: &str, scope_name: &str, func: Rc<Function>) {
    SCOPES.with(|s| s.borrow_mut().insert(class_name, scope_name, func));
}

/// Look up a scope closure on `(class_name, scope_name)`. Returns `None` if
/// no scope is registered.
pub fn lookup_scope(class_name: &str, scope_name: &str) -> Option<Rc<Function>> {
    SCOPES.with(|s| s.borrow().get(class_name, scope_name).cloned())
}

/// Whether `scope_name` is a scope of `class_name`. The VM's dispatch probes
/// only need the answer, not a clone of the closure.
pub fn has_scope(class_name: &str, scope_name: &str) -> bool {
    SCOPES.with(|s| s.borrow().get(class_name, scope_name).is_some())
}

/// STI copy-down: register every scope of `parent` under `child` too, so
/// `Admin.recent` resolves when `recent` was declared on `User`. The child's
/// own `scope(...)` calls run afterward and overwrite same-named entries.
pub fn copy_scopes(parent: &str, child: &str) {
    SCOPES.with(|s| {
        let mut registry = s.borrow_mut();
        let Some(inherited) = registry.classes.get(parent).cloned() else {
            return;
        };
        for (scope, func) in inherited {
            registry.insert(child, &scope, func);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn func() -> Rc<Function> {
        Rc::new(Function::default())
    }

    #[test]
    fn lookup_misses_on_an_empty_registry_and_unknown_names() {
        assert!(lookup_scope("ScopeTestEmpty", "recent").is_none());
        assert!(!has_scope("ScopeTestEmpty", "recent"));
        register_scope("ScopeTestPost", "recent", func());
        assert!(!has_scope("ScopeTestPost", "published"));
        assert!(!has_scope("ScopeTestOther", "recent"));
    }

    #[test]
    fn register_then_lookup_returns_the_same_closure_and_overwrites() {
        let first = func();
        register_scope("ScopeTestArticle", "recent", first.clone());
        let found = lookup_scope("ScopeTestArticle", "recent").unwrap();
        assert!(Rc::ptr_eq(&found, &first));
        assert!(has_scope("ScopeTestArticle", "recent"));

        let second = func();
        register_scope("ScopeTestArticle", "recent", second.clone());
        let found = lookup_scope("ScopeTestArticle", "recent").unwrap();
        assert!(Rc::ptr_eq(&found, &second));
    }

    #[test]
    fn copy_scopes_inherits_and_child_overrides_stay_per_class() {
        let recent = func();
        let active = func();
        register_scope("ScopeTestUser", "recent", recent.clone());
        register_scope("ScopeTestUser", "active", active.clone());
        copy_scopes("ScopeTestUser", "ScopeTestAdmin");
        assert!(Rc::ptr_eq(
            &lookup_scope("ScopeTestAdmin", "recent").unwrap(),
            &recent
        ));
        assert!(Rc::ptr_eq(
            &lookup_scope("ScopeTestAdmin", "active").unwrap(),
            &active
        ));

        let admin_recent = func();
        register_scope("ScopeTestAdmin", "recent", admin_recent.clone());
        assert!(Rc::ptr_eq(
            &lookup_scope("ScopeTestAdmin", "recent").unwrap(),
            &admin_recent
        ));
        assert!(Rc::ptr_eq(
            &lookup_scope("ScopeTestUser", "recent").unwrap(),
            &recent
        ));

        copy_scopes("ScopeTestNoScopes", "ScopeTestChild");
        assert!(!has_scope("ScopeTestChild", "recent"));
    }
}
