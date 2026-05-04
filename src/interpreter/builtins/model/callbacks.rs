//! Lifecycle callbacks for models.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::Function;

use super::core::MODEL_REGISTRY;

// Closure-based callbacks. Stored thread-local because Rc<Function> is !Send
// and the global MODEL_REGISTRY (a RwLock) requires Send+Sync contents. Each
// worker populates this independently. Keyed by (class_name, event_name).
type ClosureCallbackMap = HashMap<(String, String), Vec<Rc<Function>>>;
thread_local! {
    static CALLBACK_CLOSURES: RefCell<ClosureCallbackMap> = RefCell::new(HashMap::new());
}

pub fn register_callback_fn(class_name: &str, event: &str, func: Rc<Function>) {
    CALLBACK_CLOSURES.with(|c| {
        c.borrow_mut()
            .entry((class_name.to_string(), event.to_string()))
            .or_default()
            .push(func);
    });
}

/// Get all closure-based callbacks for a (class, event) pair. Empty if none.
pub fn closure_callbacks_for(class_name: &str, event: &str) -> Vec<Rc<Function>> {
    CALLBACK_CLOSURES.with(|c| {
        c.borrow()
            .get(&(class_name.to_string(), event.to_string()))
            .cloned()
            .unwrap_or_default()
    })
}

/// Lifecycle callbacks for a model.
#[derive(Debug, Clone, Default)]
pub struct ModelCallbacks {
    pub before_save: Vec<String>,
    pub after_save: Vec<String>,
    pub before_create: Vec<String>,
    pub after_create: Vec<String>,
    pub before_update: Vec<String>,
    pub after_update: Vec<String>,
    pub before_delete: Vec<String>,
    pub after_delete: Vec<String>,
}

/// Register a callback for a model class.
pub fn register_callback(class_name: &str, callback_type: &str, method_name: &str) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    match callback_type {
        "before_save" => metadata.callbacks.before_save.push(method_name.to_string()),
        "after_save" => metadata.callbacks.after_save.push(method_name.to_string()),
        "before_create" => metadata
            .callbacks
            .before_create
            .push(method_name.to_string()),
        "after_create" => metadata
            .callbacks
            .after_create
            .push(method_name.to_string()),
        "before_update" => metadata
            .callbacks
            .before_update
            .push(method_name.to_string()),
        "after_update" => metadata
            .callbacks
            .after_update
            .push(method_name.to_string()),
        "before_delete" => metadata
            .callbacks
            .before_delete
            .push(method_name.to_string()),
        "after_delete" => metadata
            .callbacks
            .after_delete
            .push(method_name.to_string()),
        _ => {}
    }
}
