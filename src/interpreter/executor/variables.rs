//! Variable access expression evaluation.

use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::value::Value;

use super::{Interpreter, RuntimeResult};

impl Interpreter {
    /// Evaluate variable access expressions.
    pub(crate) fn evaluate_variable(&mut self, name: &str, expr: &Expr) -> RuntimeResult<Value> {
        self.environment
            .borrow()
            .get(name)
            .ok_or_else(|| RuntimeError::undefined_variable(name, expr.span))
    }
}
