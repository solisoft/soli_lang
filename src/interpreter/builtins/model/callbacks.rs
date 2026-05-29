//! Lifecycle callbacks for models.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::{Class, Function, HashKey, HashPairs, Instance, Value};

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

/// Method-name callbacks registered for a single lifecycle `event`.
fn names_for_event(callbacks: &ModelCallbacks, event: &str) -> Vec<String> {
    match event {
        "before_save" => callbacks.before_save.clone(),
        "after_save" => callbacks.after_save.clone(),
        "before_create" => callbacks.before_create.clone(),
        "after_create" => callbacks.after_create.clone(),
        "before_update" => callbacks.before_update.clone(),
        "after_update" => callbacks.after_update.clone(),
        "before_delete" => callbacks.before_delete.clone(),
        "after_delete" => callbacks.after_delete.clone(),
        _ => Vec::new(),
    }
}

/// Invoke one callback (a class method or a registered closure) with `this`
/// and `self` bound to `instance`. Uses the shared `bind_user_method_to_receiver`
/// bridge, which runs the body on a tree-walking interpreter while resolving
/// globals (other model classes, builtins) through the function's own closure
/// chain — so it works identically whether the caller is the VM or the
/// executor. Returns `Ok(true)` when the callback vetoed by returning `false`.
fn invoke_callback(instance: &Rc<RefCell<Instance>>, func: Rc<Function>) -> Result<bool, String> {
    let bound = crate::interpreter::executor::access::member::bind_user_method_to_receiver(
        Value::Instance(instance.clone()),
        func,
    );
    if let Value::NativeFunction(native) = bound {
        let result = (native.func)(vec![])?;
        return Ok(matches!(result, Value::Bool(false)));
    }
    Ok(false)
}

/// Run every lifecycle callback registered for `events` (in order) on
/// `instance`: method-name callbacks first, then closure callbacks. Stops and
/// returns `Ok(false)` as soon as a `before_*` callback vetoes by returning
/// `false` (remaining callbacks are skipped); returns `Ok(true)` otherwise.
///
/// This is the single shared entry point that the native model methods call,
/// so callbacks fire identically under the tree-walking executor and the
/// bytecode VM (both reach the same native). `get_or_create_metadata` returns
/// an owned clone, so the registry lock is released before any callback runs —
/// callbacks (e.g. a `before_delete` cascade) are free to re-enter the registry.
pub fn run_lifecycle_callbacks(
    class: &Rc<Class>,
    instance: &Rc<RefCell<Instance>>,
    events: &[&str],
) -> Result<bool, String> {
    let metadata = super::get_or_create_metadata(&class.name);
    for event in events {
        for name in names_for_event(&metadata.callbacks, event) {
            if let Some(method) = class.find_method(&name) {
                if invoke_callback(instance, method)? {
                    return Ok(false);
                }
            }
        }
        for closure in closure_callbacks_for(&class.name, event) {
            if invoke_callback(instance, closure)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Whether any callback (method-name or closure) is registered for any of
/// `events` on `class_name`. The native `create`/`update` statics use this to
/// decide whether they must re-derive the persisted data from the (possibly
/// callback-mutated) instance — skipping the round-trip when no callbacks exist
/// preserves the raw input verbatim (e.g. a user-supplied `_key`).
pub fn has_lifecycle_callbacks(class_name: &str, events: &[&str]) -> bool {
    let metadata = super::get_or_create_metadata(class_name);
    events.iter().any(|event| {
        !names_for_event(&metadata.callbacks, event).is_empty()
            || !closure_callbacks_for(class_name, event).is_empty()
    })
}

/// Stamp `_errors` onto an instance when a `before_*` callback vetoed
/// persistence by returning `false`. Mirrors the validation-failure /
/// DB-failure shape (`Array<Hash>` of `{message}`) so callers inspecting
/// `instance._errors` see one uniform "persistence aborted" contract.
pub fn set_callback_aborted_error(instance: &Rc<RefCell<Instance>>, callback_kind: &str) {
    let mut entry = HashPairs::default();
    entry.insert(
        HashKey::String("message".to_string()),
        Value::String(format!(
            "{} callback returned false; persistence aborted",
            callback_kind
        )),
    );
    let error_hash = Value::Hash(Rc::new(RefCell::new(entry)));
    instance.borrow_mut().set(
        "_errors".to_string(),
        Value::Array(Rc::new(RefCell::new(vec![error_hash]))),
    );
}
