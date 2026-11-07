//! Runtime environment for variable scopes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ahash::AHashMap;

use crate::interpreter::value::{map_hasher, Function, HashPairs, Instance, StrKey, Value};

/// The value a view include reads for a bare `name` (or `@name`) past its own
/// data: the controller's field of that name. `req`, `params`, `session` and
/// `headers` are not inherited; the view gets those another way.
pub(crate) fn inherited_field(controller: &Instance, name: &str) -> Option<Value> {
    let value = controller.fields.get(name)?;
    if !is_inherited_field(name) {
        return None;
    }
    Some(value.clone())
}

/// Whether a controller field of this name reaches the views it renders.
pub(crate) fn is_inherited_field(name: &str) -> bool {
    !matches!(name, "req" | "params" | "session" | "headers")
}

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

/// Result of [`Environment::append_string`].
///
/// `Done` carries the string after the append. The other variants mean the
/// caller should use the ordinary `+` path: the binding is missing, const,
/// or not a string.
pub enum AppendString {
    Done(Value),
    NotString,
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
    /// A view include's controller, whose `@ivars` ([`inherited_field`]) it
    /// reads after its data hash. Set only together with `data_hash`, so a
    /// scope without one pays nothing for it.
    inherited: Option<Rc<RefCell<Instance>>>,
    /// Shared by every render or helper call on a worker thread (the
    /// template builtins, the view helpers' env): user code never writes it.
    /// See [`Environment::freeze`].
    frozen: bool,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            values: AHashMap::with_hasher(map_hasher()),
            consts: AHashMap::with_hasher(map_hasher()),
            enclosing: None,
            data_hash: None,
            inherited: None,
            frozen: false,
        }
    }

    /// Create an environment pre-sized for builtin registration (~300 entries).
    /// Avoids repeated rehashing during `register_builtins()`.
    pub fn with_builtins_capacity() -> Self {
        Self {
            values: AHashMap::with_capacity_and_hasher(300, map_hasher()),
            consts: AHashMap::with_hasher(map_hasher()),
            enclosing: None,
            data_hash: None,
            inherited: None,
            frozen: false,
        }
    }

    pub fn with_enclosing(enclosing: Rc<RefCell<Environment>>) -> Self {
        Self {
            values: AHashMap::with_hasher(map_hasher()),
            consts: AHashMap::with_hasher(map_hasher()),
            enclosing: Some(enclosing),
            data_hash: None,
            inherited: None,
            frozen: false,
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
            values: AHashMap::with_hasher(map_hasher()),
            consts: AHashMap::with_hasher(map_hasher()),
            enclosing: Some(enclosing),
            data_hash: Some(data_hash),
            inherited: None,
            frozen: false,
        }
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

    /// Clear this template scope for another render. The enclosing builtins
    /// stay; the map keeps its capacity. `locals` is bound to the new data.
    /// `inherited` is the controller an include reads `@ivars` from past its
    /// data; it is dropped when there is no data hash.
    pub(crate) fn reuse_for_template(
        &mut self,
        data_hash: Option<Rc<RefCell<HashPairs>>>,
        inherited: Option<Rc<RefCell<Instance>>>,
        locals: Value,
    ) {
        self.values.clear();
        self.consts.clear();
        self.inherited = if data_hash.is_some() { inherited } else { None };
        self.data_hash = data_hash;
        self.values.insert("locals".to_string(), locals);
    }

    /// Let go of the controller a pooled template scope inherited from: a
    /// scope waiting in the pool must not keep a finished request alive.
    pub(crate) fn release_inherited(&mut self) {
        self.inherited = None;
    }

    /// Stop user assignments from writing this scope's bindings.
    ///
    /// For an env every render (or every helper call) on a worker thread
    /// encloses. A bare `name = v` assigns up the scope chain, so a template
    /// `<% type = ... %>` or a helper's `h = s.index_of("#")` overwrote the
    /// builtin for every later render on the worker: the next visitor read
    /// the previous one's value, and every later `h(x)` failed until restart.
    /// Once frozen, `assign` and `append_string` report the name as not found
    /// here (a const still reports `IsConst`), so the assignment defines a
    /// local in the innermost scope instead. `define` / `define_or_update` —
    /// the framework's own writes, such as the helpers' request context —
    /// are unaffected.
    pub fn freeze(&mut self) {
        self.frozen = true;
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
            // An include's inherited `@ivars`, after its own data and before
            // the builtins and helpers, where a copy of them used to sit.
            if let Some(ref controller) = self.inherited {
                if let Some(value) = inherited_field(&controller.borrow(), name) {
                    return Some(value);
                }
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

    /// Append `extra` onto the string bound to `name`, growing that binding's
    /// buffer in place when it is the only owner.
    ///
    /// `SoliStr` (`EcoString`) copies on write: `push_str` mutates the buffer
    /// when the refcount is 1 and otherwise detaches first. A loop of
    /// `buf = buf + "x"` stays linear for the unique case and still leaves a
    /// second variable that aliased `buf` unchanged.
    pub fn append_string(&mut self, name: &str, extra: &str) -> AppendString {
        if !self.consts.is_empty() && self.consts.contains_key(name) {
            return AppendString::IsConst;
        }
        if self.frozen {
            return AppendString::NotFound;
        }
        if let Some(slot) = self.values.get_mut(name) {
            if let Value::String(text) = slot {
                text.push_str(extra);
            } else {
                return AppendString::NotString;
            }
            return AppendString::Done(self.values.get(name).unwrap().clone());
        }
        // `get` answers from the view's data hash, and then from an include's
        // inherited fields, before the enclosing scope; appending further up
        // would edit a different binding.
        if let Some(ref hash) = self.data_hash {
            if hash.borrow().get(&StrKey(name)).is_some() {
                return AppendString::NotString;
            }
            if let Some(ref controller) = self.inherited {
                if inherited_field(&controller.borrow(), name).is_some() {
                    return AppendString::NotString;
                }
            }
        }
        if let Some(ref enclosing) = self.enclosing {
            return enclosing.borrow_mut().append_string(name, extra);
        }
        AppendString::NotFound
    }

    /// Assign to an existing variable, searching up the scope chain.
    /// Returns false if the variable is const.
    pub fn assign(&mut self, name: &str, value: Value) -> AssignResult {
        if !self.consts.is_empty() && self.consts.contains_key(name) {
            return AssignResult::IsConst;
        }
        if self.frozen {
            return AssignResult::NotFound;
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

    /// Every name bound in this scope and the scopes around it, template
    /// locals included. Only the "did you mean" search of an
    /// undefined-variable error reads it.
    pub fn collect_names(&self, out: &mut Vec<String>) {
        out.extend(self.values.keys().cloned());
        out.extend(self.consts.keys().cloned());
        if let Some(Ok(data)) = self.data_hash.as_ref().map(|d| d.try_borrow()) {
            for key in data.keys() {
                if let crate::interpreter::value::HashKey::String(s) = key {
                    out.push(s.to_string());
                }
            }
        }
        if let Some(Ok(enclosing)) = self.enclosing.as_ref().map(|e| e.try_borrow()) {
            enclosing.collect_names(out);
        }
    }

    /// Get all variable names in the current scope (for REPL introspection).
    pub fn get_var_names(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
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

/// Strong references `func` holds on `target`: its `closure` field, plus its
/// parked call env (`cached_env`, whose `enclosing` is that same closure)
/// when nothing but the slot owns the parked env.
fn refs_into(func: &Function, target: &Rc<RefCell<Environment>>) -> usize {
    if !Rc::ptr_eq(&func.closure, target) {
        return 0;
    }
    let mut refs = 1;
    if let Ok(slot) = func.cached_env.try_borrow() {
        if let Some(parked) = slot.as_ref() {
            let parked_points_here = Rc::strong_count(parked) == 1
                && parked.try_borrow().is_ok_and(|parked| {
                    parked
                        .enclosing
                        .as_ref()
                        .is_some_and(|up| Rc::ptr_eq(up, target))
                });
            if parked_points_here {
                refs += 1;
            }
        }
    }
    refs
}

/// Break the `env -> binding -> Function -> closure -> env` cycle of a scope
/// that is being abandoned, when — and only when — nothing else can reach it.
///
/// `held_by_caller` is how many strong references the caller itself holds on
/// `env` (the frame that is about to let go of it). Returns `true` when the
/// bindings were dropped.
///
/// This is trial deletion restricted to the one cycle shape the tree-walker
/// creates on its own: a `def` or `fn` evaluated inside a scope captures that
/// scope (`Function.closure` is a strong `Rc`) and is stored back into it.
/// Every strong reference to `env` is accounted for, or nothing happens:
///
/// * A `Function` bound in `env` whose `Rc` count equals the number of times it
///   is bound here is *internal*: no array, hash, instance field, global,
///   return value, thrown value, other scope or Rust frame holds it, since each
///   of those would own one more `Rc`.
/// * An internal function contributes its `closure` (when that is `env`) and
///   its parked call env (only when the slot is that env's sole owner).
/// * One level of block nesting: an internal function whose closure is a
///   scope `inner` directly inside `env` contributes `inner`'s `enclosing`
///   pointer — but only when every strong reference to `inner` is itself one
///   of those internal functions' closures or parked envs.
///
/// If the accounted references plus `held_by_caller` equal `env`'s strong
/// count, every path to `env` runs through `env` itself: the scope is garbage
/// that `Rc` alone cannot free, and clearing it is unobservable. Anything
/// uncounted — a closure that escaped by any route, a function nested inside a
/// collection, a live recursive frame whose scope chain passes through `env`,
/// a borrowed `RefCell` — leaves the count short and the scope untouched.
/// Over-counting is the only way this could be wrong, and every counted
/// reference is a distinct `Rc` verified by pointer identity. No `Weak` to an
/// environment or a `Function` exists in the interpreter, so a strong count of
/// zero outside really means unreachable.
pub fn break_self_cycle(env: &Rc<RefCell<Environment>>, held_by_caller: usize) -> bool {
    let total = Rc::strong_count(env);
    if total <= held_by_caller {
        return false;
    }
    {
        let Ok(scope) = env.try_borrow() else {
            return false;
        };
        // Distinct functions bound here, with how many bindings hold each.
        let mut bound: Vec<(&Rc<Function>, usize)> = Vec::new();
        for value in scope.values.values().chain(scope.consts.values()) {
            if let Value::Function(func) = value {
                match bound.iter_mut().find(|(seen, _)| Rc::ptr_eq(*seen, func)) {
                    Some(entry) => entry.1 += 1,
                    None => bound.push((func, 1)),
                }
            }
        }
        // Borrows only — cloning an `Rc` here would itself skew the counts.
        let internal: Vec<&Rc<Function>> = bound
            .into_iter()
            .filter(|&(func, bindings)| Rc::strong_count(func) == bindings)
            .map(|(func, _)| func)
            .collect();
        if internal.is_empty() {
            return false;
        }

        let mut accounted: usize = internal.iter().map(|&func| refs_into(func, env)).sum();

        let mut inner_scopes: Vec<&Rc<RefCell<Environment>>> = Vec::new();
        for &func in &internal {
            let inner = &func.closure;
            if Rc::ptr_eq(inner, env) || inner_scopes.iter().any(|&seen| Rc::ptr_eq(seen, inner)) {
                continue;
            }
            let directly_inside = inner.try_borrow().is_ok_and(|inner| {
                inner
                    .enclosing
                    .as_ref()
                    .is_some_and(|up| Rc::ptr_eq(up, env))
            });
            if directly_inside {
                inner_scopes.push(inner);
            }
        }
        for inner in inner_scopes {
            let held_internally: usize = internal.iter().map(|&func| refs_into(func, inner)).sum();
            if Rc::strong_count(inner) == held_internally {
                accounted += 1;
            }
        }

        if accounted + held_by_caller != total {
            return false;
        }
    }

    // Move the bindings out and drop them only after the borrow is released:
    // dropping them drops the internal functions, whose closures and parked
    // envs decrement `env`'s count.
    let released = match env.try_borrow_mut() {
        Ok(mut scope) => (
            std::mem::take(&mut scope.values),
            std::mem::take(&mut scope.consts),
        ),
        Err(_) => return false,
    };
    drop(released);
    true
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::{Class, HashKey};

    fn controller(fields: &[(&str, Value)]) -> Rc<RefCell<Instance>> {
        let class = Rc::new(Class {
            name: "PostsController".to_string(),
            ..Default::default()
        });
        let mut instance = Instance::new(class);
        for (name, value) in fields {
            instance.fields.insert((*name).into(), value.clone());
        }
        Rc::new(RefCell::new(instance))
    }

    fn data(pairs: &[(&str, Value)]) -> Rc<RefCell<HashPairs>> {
        let mut map = HashPairs::default();
        for (key, value) in pairs {
            map.insert(HashKey::String((*key).into()), value.clone());
        }
        Rc::new(RefCell::new(map))
    }

    /// An include reads its data, then its controller's fields, then the
    /// scope it encloses. The request plumbing is not inherited.
    #[test]
    fn an_include_reads_its_data_then_the_controller_fields() {
        let mut builtins = Environment::new();
        builtins.define("helper".to_string(), Value::Int(1));
        builtins.define("title".to_string(), Value::Int(2));
        builtins.define("params".to_string(), Value::Int(3));
        let mut scope = Environment::with_enclosing(Rc::new(RefCell::new(builtins)));
        let fields = controller(&[
            ("title", Value::String("Posts".into())),
            ("post", Value::String("from @post".into())),
            ("params", Value::String("the request's".into())),
            ("_internal", Value::Int(7)),
        ]);
        let locals = data(&[("post", Value::String("passed".into()))]);
        scope.reuse_for_template(Some(locals), Some(fields), Value::Null);

        assert_eq!(scope.get("post"), Some(Value::String("passed".into())));
        assert_eq!(scope.get("title"), Some(Value::String("Posts".into())));
        assert_eq!(scope.get("_internal"), Some(Value::Int(7)));
        assert_eq!(scope.get("helper"), Some(Value::Int(1)));
        assert_eq!(scope.get("params"), Some(Value::Int(3)));
        assert_eq!(scope.get("absent"), None);

        // `title += "!"` reads the field and binds a local: it never appends
        // into the builtins' `title`, nor into the controller's.
        assert!(matches!(
            scope.append_string("title", "!"),
            AppendString::NotString
        ));

        scope.release_inherited();
        assert_eq!(scope.get("title"), Some(Value::Int(2)));
    }

    /// Without a data hash (a partial given a non-hash context) nothing is
    /// inherited, as no `@ivar` was ever copied into such a render.
    #[test]
    fn a_scope_without_data_inherits_nothing() {
        let mut scope = Environment::new();
        let fields = controller(&[("title", Value::String("Posts".into()))]);
        scope.reuse_for_template(None, Some(fields), Value::Null);
        assert_eq!(scope.get("title"), None);
    }
}
