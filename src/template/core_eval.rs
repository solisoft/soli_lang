//! Core language expression evaluation for templates.
//!
//! This module provides expression evaluation using the core language's
//! interpreter, giving templates access to all builtins.
//!
//! Optimizations:
//! - Direct AST translation: template Expr → core ExprKind (no string round-trip)
//! - Shared builtins: thread-local Rc<RefCell<Environment>> avoids cloning builtins
//! - One interpreter per render, drawn from a thread-local pool

use std::cell::{Cell, RefCell};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::Value;

// ---------------------------------------------------------------------------
// Thread-local shared builtins environment (Rc, not cloned)
// ---------------------------------------------------------------------------

thread_local! {
    /// Shared builtins environment. Uses Rc so child scopes can reference it
    /// without cloning the entire HashMap of builtins.
    static BUILTINS_RC: RefCell<Option<Rc<RefCell<Environment>>>> = const { RefCell::new(None) };
}

/// Get the shared builtins environment Rc.
fn get_builtins_rc() -> Rc<RefCell<Environment>> {
    BUILTINS_RC.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            let mut env = Environment::with_builtins_capacity();
            // `false`: no test-only builtins. This used to pass `true`, so
            // `visit`, `click`, `assert_eq`, the factories and the mock-HTTP
            // helpers resolved inside every rendered view in production —
            // names `register_builtins` refuses everywhere else in serve mode
            // precisely because leaking them into a served app is a hazard.
            crate::interpreter::builtins::register_builtins(&mut env, false);
            crate::interpreter::builtins::template::register_static_template_helpers(&mut env);
            crate::interpreter::builtins::template::inject_helpers_into_env(&mut env);
            // Mirror the worker's main env: views need `<name>_path` /
            // `<name>_url` helpers too. Without this, calls like
            // `admin_path()` in a `.html.slv` template fall through to the
            // lenient-undefined path and crash as "not callable".
            crate::interpreter::builtins::named_routes::register_named_route_helpers(&mut env);
            let env_rc = Rc::new(RefCell::new(env));
            // Form builder layer (form_with / csrf_field / button_to) is
            // pure Soli, evaluated once per thread into an environment of its
            // own and defined here — evaluated into env_rc, its functions held
            // env_rc and every hot reload leaked the whole registry.
            if let Err(e) = crate::interpreter::builtins::template::register_form_builder(&env_rc) {
                eprintln!("[WARN] template form builder failed to load: {}", e);
            }
            // Retry is pure Soli too, so it needs the wrapped env like the form
            // builder. It was wired only into the `Interpreter::*` constructors,
            // so every other class from `register_builtins` resolved in a view
            // while `Retry.with_backoff(...)` raised "Undefined variable".
            if let Err(e) = crate::interpreter::builtins::retry::register_retry_class(&env_rc) {
                eprintln!("[WARN] Retry stdlib failed to load: {}", e);
            }
            *opt = Some(env_rc);
        }
        opt.as_ref().unwrap().clone()
    })
}

/// Drop the cached builtins env so the next template render rebuilds it.
/// Called on helper hot reload so updated `app/helpers/*.sl` definitions
/// take effect, and on routes hot reload so updated `<name>_path` /
/// `<name>_url` helpers reflect the new patterns. Active renders keep
/// their existing `Rc<Environment>` and finish cleanly — only the
/// thread-local cache slot is cleared.
pub fn reset_builtins_rc() {
    BUILTINS_RC.with(|cell| *cell.borrow_mut() = None);
    // Pooled interpreters close over this env. Drop them, and refuse one that
    // was already taken out when it comes back.
    INTERPRETER_POOL_GEN.with(|generation| generation.set(generation.get() + 1));
    INTERPRETER_POOL.with(|pool| pool.borrow_mut().clear());
    // The views compiled to the VM hold this env's globals.
    super::vm_template::reset_thread();
}

/// Every name a view resolves past its data — builtins, view helpers, route
/// helpers — as the globals of the VM views are compiled to.
pub(crate) fn template_env_bindings() -> std::collections::HashMap<String, Value> {
    get_builtins_rc().borrow().get_all_bindings()
}

// ---------------------------------------------------------------------------
// Interpreter lifecycle for template rendering
// ---------------------------------------------------------------------------

/// How many interpreters a thread keeps. One request holds the view's, and
/// each nested include holds another; past this, extras are dropped.
const INTERPRETER_POOL_LIMIT: usize = 64;

thread_local! {
    static INTERPRETER_POOL: RefCell<Vec<Interpreter>> = const { RefCell::new(Vec::new()) };
    /// Bumped when the builtins env is rebuilt. An interpreter taken out
    /// before then closes over the old env and is not put back.
    static INTERPRETER_POOL_GEN: Cell<u64> = const { Cell::new(0) };
}

/// Create a template interpreter populated with data.
/// Uses shared builtins (no clone) + data hash reference (no copy).
/// Data variables are looked up directly in the hash via zero-alloc StrKey.
///
/// Also binds `locals` to the passed-in hash (or an empty hash if none)
/// so partials can read reserved-word keys via `locals["class"]` etc.,
/// mirroring Rails' `local_assigns`. Bare-identifier access keeps working
/// for non-reserved keys; `locals` is the escape hatch for the rest.
pub fn create_template_interpreter(data: &Value) -> Interpreter {
    let mut data_env = Environment::with_enclosing(get_builtins_rc());
    bind_template_data(&mut data_env, data);
    Interpreter::with_environment(Rc::new(RefCell::new(data_env)))
}

/// A template interpreter for one render, returned to the pool on drop.
///
/// A view and each partial it renders used to build one. The shell and the
/// data scope are what a page of small includes repeated; the builtins stay
/// shared either way. An interpreter left inside a child scope, or still
/// referenced elsewhere, is dropped instead of reused.
pub(crate) struct PooledInterpreter {
    inner: Option<Interpreter>,
    generation: u64,
}

impl Deref for PooledInterpreter {
    type Target = Interpreter;

    fn deref(&self) -> &Interpreter {
        // `inner` is `Some` from construction until `Drop` takes it back to
        // the pool, and nothing derefs a value being dropped: an internal
        // invariant, out of reach of any template or request.
        match &self.inner {
            Some(interpreter) => interpreter,
            None => unreachable!("pooled interpreter used after drop"),
        }
    }
}

impl DerefMut for PooledInterpreter {
    fn deref_mut(&mut self) -> &mut Interpreter {
        match &mut self.inner {
            Some(interpreter) => interpreter,
            None => unreachable!("pooled interpreter used after drop"),
        }
    }
}

impl Drop for PooledInterpreter {
    fn drop(&mut self) {
        let Some(interp) = self.inner.take() else {
            return;
        };
        if self.generation != INTERPRETER_POOL_GEN.with(Cell::get) || !recyclable(&interp) {
            return;
        }
        INTERPRETER_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < INTERPRETER_POOL_LIMIT {
                pool.push(interp);
            }
        });
    }
}

/// Take a pooled interpreter, or build one, bound to `data`.
pub(crate) fn template_interpreter(data: &Value) -> PooledInterpreter {
    let generation = INTERPRETER_POOL_GEN.with(Cell::get);
    let interp = match INTERPRETER_POOL.with(|pool| pool.borrow_mut().pop()) {
        Some(mut interp) => {
            rebind_interpreter(&mut interp, data);
            interp
        }
        None => create_template_interpreter(data),
    };
    PooledInterpreter {
        inner: Some(interp),
        generation,
    }
}

fn bind_template_data(env: &mut Environment, data: &Value) {
    use crate::interpreter::value::HashPairs;

    let (data_hash, locals) = if let Value::Hash(map) = data {
        (Some(map.clone()), data.clone())
    } else {
        (
            None,
            Value::Hash(Rc::new(RefCell::new(HashPairs::default()))),
        )
    };
    env.reuse_for_template(data_hash, locals);
}

fn rebind_interpreter(interp: &mut Interpreter, data: &Value) {
    interp.call_stack.clear();
    interp.assertion_count = 0;
    interp.current_source_path = None;
    interp.vm_globals = None;
    interp.kernels = None;
    interp.coverage_tracker = None;
    bind_template_data(&mut interp.environment.borrow_mut(), data);
}

/// The data scope, with nothing else still pointing at it. A child scope's
/// enclosing chain is longer, and a closure that captured this env keeps it
/// alive: either one would make clearing the map corrupt another owner.
fn recyclable(interp: &Interpreter) -> bool {
    if !interp.call_stack.is_empty()
        || interp.coverage_tracker.is_some()
        || interp.vm_globals.is_some()
        || interp.kernels.is_some()
    {
        return false;
    }
    if Rc::strong_count(&interp.environment) != 1 {
        return false;
    }
    let Ok(env) = interp.environment.try_borrow() else {
        return false;
    };
    match env.enclosing() {
        Some(parent) => parent.borrow().enclosing().is_none(),
        None => false,
    }
}

#[cfg(test)]
fn pooled_interpreters() -> usize {
    INTERPRETER_POOL.with(|pool| pool.borrow().len())
}

/// Push a new child scope on the interpreter's environment.
/// Used for loop bodies so loop vars don't leak to outer scope.
#[inline]
pub fn push_scope(interpreter: &mut Interpreter) {
    let old_env = interpreter.environment.clone();
    let new_env = Environment::with_enclosing(old_env);
    interpreter.environment = Rc::new(RefCell::new(new_env));
}

/// Pop back to the enclosing scope.
#[inline]
pub fn pop_scope(interpreter: &mut Interpreter) {
    let enclosing = interpreter
        .environment
        .borrow()
        .enclosing()
        .expect("pop_scope called without enclosing scope");
    interpreter.environment = enclosing;
}

/// Define a variable in the interpreter's current scope.
/// Uses define_or_update to avoid String key allocation when variable already exists
/// (common in for-loops where the same variable is redefined every iteration).
#[inline]
pub fn define_var(interpreter: &mut Interpreter, name: &str, value: Value) {
    interpreter
        .environment
        .borrow_mut()
        .define_or_update(name, value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Evaluate `source` the way `<%= … %>` does (`TemplateNode::CoreOutput`):
    /// through the core parser and the interpreter, an undefined name reading
    /// as `nil`.
    fn eval(source: &str, interp: &mut Interpreter) -> Result<Value, String> {
        use crate::ast::stmt::StmtKind;
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .map_err(|e| e.to_string())?;
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| e.to_string())?;
        let Some(StmtKind::Expression(expr)) =
            program.statements.into_iter().next().map(|s| s.kind)
        else {
            return Err(format!("{source:?} is not an expression"));
        };
        match interp.evaluate(&expr) {
            Ok(value) => Ok(value),
            Err(e) if e.to_string().contains("Undefined variable") => Ok(Value::Null),
            Err(e) => Err(e.to_string()),
        }
    }

    fn make_hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut map = HashPairs::default();
        for (k, v) in pairs {
            map.insert(HashKey::String(k.to_string().into()), v);
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    #[test]
    fn test_evaluate_with_context() {
        let data = make_hash(vec![("name", Value::String("World".into()))]);
        let mut interp = create_template_interpreter(&data);
        let result = eval("\"Hello \" + name", &mut interp);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Value::String("Hello World".into()));
    }

    #[test]
    fn test_evaluate_nested_hash_access() {
        let user = make_hash(vec![("name", Value::String("Alice".into()))]);
        let data = make_hash(vec![("user", user)]);
        let mut interp = create_template_interpreter(&data);
        let result = eval("user.name", &mut interp);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Value::String("Alice".into()));
    }

    #[test]
    fn test_reuse_interpreter() {
        let data = make_hash(vec![("x", Value::Int(10)), ("y", Value::Int(20))]);
        let mut interp = create_template_interpreter(&data);

        let r1 = eval("x", &mut interp).unwrap();
        assert_eq!(r1, Value::Int(10));

        let r2 = eval("y", &mut interp).unwrap();
        assert_eq!(r2, Value::Int(20));
    }

    #[test]
    fn test_scope_push_pop() {
        let data = make_hash(vec![("x", Value::Int(1))]);
        let mut interp = create_template_interpreter(&data);

        push_scope(&mut interp);
        define_var(&mut interp, "x", Value::Int(99));
        let r = eval("x", &mut interp).unwrap();
        assert_eq!(r, Value::Int(99));
        pop_scope(&mut interp);

        let r = eval("x", &mut interp).unwrap();
        assert_eq!(r, Value::Int(1));
    }

    #[test]
    fn a_pooled_interpreter_does_not_keep_the_previous_render(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let before = pooled_interpreters();
        let ada = make_hash(vec![("name", Value::String("Ada".into()))]);
        let bea = make_hash(vec![("other", Value::String("Bea".into()))]);

        let mut first = template_interpreter(&ada);
        define_var(&mut first, "extra", Value::Int(1));
        drop(first);
        assert_eq!(pooled_interpreters(), before + 1);

        let mut second = template_interpreter(&bea);
        assert_eq!(pooled_interpreters(), before);
        assert_eq!(eval("other", &mut second)?, Value::String("Bea".into()));
        assert_eq!(eval("name", &mut second)?, Value::Null);
        assert_eq!(eval("extra", &mut second)?, Value::Null);
        drop(second);
        assert_eq!(pooled_interpreters(), before + 1);

        // A scope left open is not recycled: clearing it would wipe the child
        // and leave the previous render's data hash one frame up.
        let mut poisoned = template_interpreter(&ada);
        assert_eq!(pooled_interpreters(), before);
        push_scope(&mut poisoned);
        drop(poisoned);
        assert_eq!(pooled_interpreters(), before);

        let mut third = template_interpreter(&bea);
        assert_eq!(eval("name", &mut third)?, Value::Null);
        assert_eq!(eval("other", &mut third)?, Value::String("Bea".into()));
        Ok(())
    }

    #[test]
    fn hot_reload_drops_pooled_interpreters() {
        let data = make_hash(vec![("name", Value::String("Ada".into()))]);
        let held = template_interpreter(&data);
        drop(template_interpreter(&data));
        assert!(pooled_interpreters() >= 1);

        reset_builtins_rc();
        assert_eq!(pooled_interpreters(), 0);
        // Taken out before the reload: it still closes over the old builtins.
        drop(held);
        assert_eq!(pooled_interpreters(), 0);
    }

    #[test]
    fn test_view_helpers_resolve_via_enclosing_scope() {
        use crate::interpreter::builtins::template::{clear_view_helpers, register_view_helper};
        use crate::interpreter::value::NativeFunction;

        // Isolate this thread's template state.
        clear_view_helpers();
        reset_builtins_rc();

        register_view_helper(
            "__spec_helper_uppercase".to_string(),
            Value::NativeFunction(NativeFunction::new(
                "__spec_helper_uppercase",
                None,
                |args| match args.first() {
                    Some(Value::String(s)) => Ok(Value::String(s.to_uppercase())),
                    _ => Ok(Value::Null),
                },
            )),
        );

        // Data hash has no helper key — the helper must resolve via the
        // enclosing BUILTINS_RC env seeded by inject_helpers_into_env.
        let data = make_hash(vec![("other", Value::Int(1))]);
        let mut interp = create_template_interpreter(&data);
        let v = eval("__spec_helper_uppercase", &mut interp).unwrap();
        assert!(matches!(v, Value::NativeFunction(_)));

        clear_view_helpers();
        reset_builtins_rc();
    }

    #[test]
    fn test_named_route_helpers_resolve_in_template_env() {
        use crate::interpreter::builtins::named_routes::rebuild_named_routes;
        use crate::interpreter::builtins::server::{clear_routes, restore_routes, Route};
        use crate::interpreter::builtins::template::clear_view_helpers;

        // Isolate this thread's route + template state.
        clear_view_helpers();
        clear_routes();
        reset_builtins_rc();

        // Install one named route (mirrors `get("/admin", "admin#index", name: "admin")`)
        // and rebuild the named-route lookup the helpers read from.
        let routes = vec![Route {
            method: "GET".to_string(),
            path_pattern: "/admin".to_string(),
            handler_name: "admin#index".to_string(),
            name: Some("admin".to_string()),
            middleware: vec![],
            middleware_names: vec![],
            openapi: None,
        }];
        rebuild_named_routes(&routes);
        restore_routes(routes);

        // Bare-name lookup must resolve to the registered NativeFunction
        // (proves the helper is bound in the template's enclosing env).
        let data = make_hash(vec![]);
        let mut interp = create_template_interpreter(&data);
        let resolved = eval("admin_path", &mut interp).unwrap();
        assert!(matches!(resolved, Value::NativeFunction(_)));

        // And actually invoking `admin_path()` must produce the registered
        // path string — this is the user-visible behavior the fix restores.
        let called = eval("admin_path()", &mut interp).unwrap();
        assert_eq!(called, Value::String("/admin".into()));

        clear_routes();
        reset_builtins_rc();
    }

    /// A hot reload drops the cached env; nothing evaluated into it may hold
    /// it, or every reload leaks a whole builtins registry.
    #[test]
    fn test_reset_builtins_rc_frees_the_old_env() {
        reset_builtins_rc();
        let old = Rc::downgrade(&get_builtins_rc());
        reset_builtins_rc();
        assert!(
            old.upgrade().is_none(),
            "the template builtins env outlived its reset"
        );

        // The form builder still resolves in the rebuilt env.
        let data = make_hash(vec![]);
        let mut interp = create_template_interpreter(&data);
        let form_with = eval("form_with", &mut interp);
        assert!(
            form_with.is_ok_and(|value| !matches!(value, Value::Null)),
            "form_with no longer resolves after a reset"
        );
        reset_builtins_rc();
    }

    #[test]
    fn test_reset_builtins_rc_picks_up_new_helpers() {
        use crate::interpreter::builtins::template::{clear_view_helpers, register_view_helper};
        use crate::interpreter::value::NativeFunction;

        clear_view_helpers();
        reset_builtins_rc();

        // Force BUILTINS_RC initialization with no helpers registered.
        let data = make_hash(vec![]);
        let mut interp_before = create_template_interpreter(&data);
        let before = eval("__spec_reset_helper", &mut interp_before).unwrap();
        assert!(matches!(before, Value::Null));

        // Register a helper and reset — the next interpreter must see it.
        register_view_helper(
            "__spec_reset_helper".to_string(),
            Value::NativeFunction(NativeFunction::new("__spec_reset_helper", None, |_| {
                Ok(Value::Int(42))
            })),
        );
        reset_builtins_rc();

        let mut interp_after = create_template_interpreter(&data);
        let after = eval("__spec_reset_helper", &mut interp_after).unwrap();
        assert!(matches!(after, Value::NativeFunction(_)));

        clear_view_helpers();
        reset_builtins_rc();
    }
}
