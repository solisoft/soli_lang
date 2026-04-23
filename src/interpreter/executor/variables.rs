//! Variable access expression evaluation.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::Value;

use super::{Interpreter, RuntimeResult};

thread_local! {
    /// When true, undefined variable lookups return Null instead of raising.
    /// Set by the template renderer while walking template ASTs so that
    /// optional locals (e.g. `flash`) don't crash the whole response.
    static TEMPLATE_LENIENT_VARS: Cell<bool> = const { Cell::new(false) };
}

thread_local! {
    /// Holds the current Environment during function call execution.
    /// Used by the `defined()` builtin to check if a variable exists.
    static CURRENT_ENV: RefCell<Option<Rc<RefCell<Environment>>>> = const { RefCell::new(None) };
}

/// Set the current environment (used by `defined()`).
pub fn set_current_env(env: Rc<RefCell<Environment>>) {
    CURRENT_ENV.with(|c: &RefCell<Option<Rc<RefCell<Environment>>>>| *c.borrow_mut() = Some(env));
}

/// Clear the current environment.
pub fn clear_current_env() {
    CURRENT_ENV.with(|c: &RefCell<Option<Rc<RefCell<Environment>>>>| *c.borrow_mut() = None);
}

/// Check if a variable is defined in the current environment chain.
pub fn is_defined(name: &str) -> bool {
    CURRENT_ENV.with(|c: &RefCell<Option<Rc<RefCell<Environment>>>>| {
        if let Some(env_ref) = c.borrow().as_ref() {
            env_ref.borrow().get(name).is_some()
        } else {
            false
        }
    })
}

/// Temporarily enter lenient-variable mode. Returns a guard; drop it to
/// restore the prior mode. Used by the template engine to treat optional
/// template locals as Null instead of erroring on missing definitions.
pub fn enter_template_lenient_vars() -> TemplateLenientVarsGuard {
    let prev = TEMPLATE_LENIENT_VARS.with(|c| c.replace(true));
    TemplateLenientVarsGuard { prev }
}

pub struct TemplateLenientVarsGuard {
    prev: bool,
}

impl Drop for TemplateLenientVarsGuard {
    fn drop(&mut self) {
        TEMPLATE_LENIENT_VARS.with(|c| c.set(self.prev));
    }
}

impl Interpreter {
    /// Evaluate variable access expressions.
    pub(crate) fn evaluate_variable(&mut self, name: &str, expr: &Expr) -> RuntimeResult<Value> {
        if let Some(v) = self.environment.borrow().get(name) {
            return Ok(v);
        }
        if TEMPLATE_LENIENT_VARS.with(|c| c.get()) {
            return Ok(Value::Null);
        }
        Err(RuntimeError::undefined_variable(name, expr.span))
    }
}
