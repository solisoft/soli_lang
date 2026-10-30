//! Variable access expression evaluation.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::ast::{Expr, ExprKind};
use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::{Interpreter, RuntimeResult};

crate::request_local! {
    /// When true, undefined variable lookups return Null instead of raising.
    /// Set by the template renderer while walking template ASTs so that
    /// optional locals (e.g. `flash`) don't crash the whole response.
    static TEMPLATE_LENIENT_VARS: Cell<bool> = const { Cell::new(false) };
}

crate::request_local! {
    /// Holds the current Environment during function call execution.
    /// Used by the `defined()` builtin to check if a variable exists.
    static CURRENT_ENV: RefCell<Option<Rc<RefCell<Environment>>>> = const { RefCell::new(None) };
}

/// Make `env` the current environment (used by `defined()`) until the guard
/// drops, which puts back the one before it. A native that runs Soli code
/// which calls another native nests these: clearing on the way out, as this
/// used to, left the outer native with no environment, and an error return
/// skipped the clear and kept the environment alive past the call.
pub fn set_current_env(env: Rc<RefCell<Environment>>) -> CurrentEnvGuard {
    let previous = CURRENT_ENV.with(|c| c.borrow_mut().replace(env));
    CurrentEnvGuard { previous }
}

/// Restores the environment that was current before [`set_current_env`].
pub struct CurrentEnvGuard {
    previous: Option<Rc<RefCell<Environment>>>,
}

impl Drop for CurrentEnvGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let _ = CURRENT_ENV.try_with(|c| *c.borrow_mut() = previous);
    }
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

/// Look up a value from the current environment chain (returns the Value if
/// defined). Used by builtins that need to resolve a name at runtime, e.g. the
/// jobs callback dispatcher fetching a `XJob` class by string.
pub fn current_env_lookup(name: &str) -> Option<crate::interpreter::Value> {
    CURRENT_ENV.with(|c| {
        c.borrow()
            .as_ref()
            .and_then(|env_ref| env_ref.borrow().get(name))
    })
}

/// Temporarily enter lenient-variable mode. Returns a guard; drop it to
/// restore the prior mode. Used by the template engine to treat optional
/// template locals as Null instead of erroring on missing definitions.
pub fn enter_template_lenient_vars() -> TemplateLenientVarsGuard {
    let prev = TEMPLATE_LENIENT_VARS.with(|c| c.replace(true));
    TemplateLenientVarsGuard { prev }
}

/// Whether template lenient-variable mode is currently active. Used by member
/// access to let view `@ivar` references fall back to a bare local instead of
/// erroring with "'this' outside of class".
pub fn template_lenient_vars_enabled() -> bool {
    TEMPLATE_LENIENT_VARS.with(|c| c.get())
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
    /// The namespace the running code is written in (`Billing::PayInvoice`),
    /// when it is in one.
    pub(crate) fn current_namespace(&self) -> Option<String> {
        match self
            .environment
            .borrow()
            .get(crate::interpreter::domain::NAMESPACE_BINDING)
        {
            Some(Value::String(ns)) => Some(ns.to_string()),
            _ => None,
        }
    }

    /// `name` read unqualified in a class nested in a module: resolved in the
    /// namespace its environment records (`interpreter::domain`).
    pub(crate) fn resolve_in_current_namespace(&self, name: &str) -> Option<Value> {
        let namespace = self.current_namespace()?;
        let env = self.environment.clone();
        crate::interpreter::domain::resolve_in_namespace(&namespace, name, |global| {
            env.borrow().get(global)
        })
    }

    /// Inside an instance method, is `name` — which resolved to no variable or
    /// function — a method of `this`? Then the bare `name` means `@name`.
    /// Only consulted after a lookup failed, so resolved names cost nothing.
    pub(crate) fn implicit_self_method(&self, name: &str) -> bool {
        match self.environment.borrow().get("this") {
            Some(Value::Instance(inst)) => inst.borrow().class.find_method(name).is_some(),
            _ => false,
        }
    }

    /// `@name` for a bare `name` that resolved to nothing (see
    /// [`Self::implicit_self_method`]): the member read, no auto-invoke.
    pub(crate) fn implicit_self_member(&mut self, name: &str, span: Span) -> RuntimeResult<Value> {
        let this = Expr::new(ExprKind::This, span);
        self.evaluate_member(&this, name, span)
    }

    /// Evaluate variable access expressions.
    pub(crate) fn evaluate_variable(&mut self, name: &str, expr: &Expr) -> RuntimeResult<Value> {
        let looked_up = self.environment.borrow().get(name);
        if let Some(v) = looked_up {
            // Reading a `grouped {}` deferred result forces it: flush the batch
            // (auto-flush) and yield the materialised value. Flush/transform
            // errors (e.g. `find`'s RecordNotFound) surface here.
            if let Value::Deferred(cell) = &v {
                return crate::interpreter::builtins::model::batch::force(cell).map_err(|e| {
                    RuntimeError::General {
                        message: e,
                        span: expr.span,
                    }
                });
            }
            return Ok(v);
        }
        if TEMPLATE_LENIENT_VARS.with(|c| c.get()) {
            return Ok(Value::Null);
        }
        Err(RuntimeError::undefined_variable(name, expr.span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::Value;

    fn env_with(name: &str) -> Rc<RefCell<Environment>> {
        let mut env = Environment::new();
        env.define(name.to_string(), Value::Int(1));
        Rc::new(RefCell::new(env))
    }

    #[test]
    fn a_nested_current_env_gives_the_outer_one_back() {
        let outer = set_current_env(env_with("outer"));
        assert!(is_defined("outer"));
        {
            let _inner = set_current_env(env_with("inner"));
            assert!(is_defined("inner"));
            assert!(!is_defined("outer"));
        }
        assert!(
            is_defined("outer"),
            "the outer native still sees its environment"
        );
        drop(outer);
        assert!(
            !is_defined("outer"),
            "nothing kept once the last guard drops"
        );
    }
}
