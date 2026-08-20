//! Side table so the VM can attach `included do` / `class_methods` metadata
//! when a module is created from bytecode (the AST is gone by then).
//!
//! Two properties this table must have, both learned the hard way:
//!
//! * **Process-global, not thread-local.** `compiled_cache::MODULE_CACHE` is a
//!   process-global, so only the first thread to compile a source file runs the
//!   compiler — and therefore only that thread registers. A thread served by a
//!   cache hit found nothing and silently lost every `included do` /
//!   `class_methods do` on the module.
//! * **Non-destructive reads.** `Op::Module` can execute more than once (a
//!   module declared inside a function, a re-entered scope). A `remove` handed
//!   the hooks to the first execution and nothing to the rest.
//!
//! Records are keyed by module name. `Stmt` is `Send + Sync`, so the map can be
//! shared across threads directly.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::ast::Stmt;

#[derive(Clone, Default)]
pub struct ModuleHookRecord {
    pub included: Vec<Vec<Stmt>>,
    pub extended: Vec<Vec<Stmt>>,
    pub concern_method_names: Vec<String>,
}

static HOOKS: OnceLock<Mutex<HashMap<String, ModuleHookRecord>>> = OnceLock::new();

fn hooks() -> &'static Mutex<HashMap<String, ModuleHookRecord>> {
    HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register(name: String, record: ModuleHookRecord) {
    if let Ok(mut map) = hooks().lock() {
        map.insert(name, record);
    }
}

/// The record for `name`, if one was registered. Cloned rather than removed so
/// a second `Op::Module` for the same module still sees it.
pub fn get(name: &str) -> Option<ModuleHookRecord> {
    hooks().lock().ok()?.get(name).cloned()
}
