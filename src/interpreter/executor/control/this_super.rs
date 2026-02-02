//! this and super keyword evaluation.

use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;

impl Interpreter {
    /// Evaluate 'this' expression.
    pub(crate) fn evaluate_this(&mut self, expr: &Expr) -> RuntimeResult<Value> {
        self.environment
            .borrow()
            .get("this")
            .ok_or_else(|| RuntimeError::type_error("'this' outside of class", expr.span))
    }

    /// Evaluate 'super' expression.
    pub(crate) fn evaluate_super(&mut self, expr: &Expr) -> RuntimeResult<Value> {
        // First check if we're in a super call (stored by call_function)
        if let Some(Value::Class(superclass)) =
            self.environment.borrow().get("__defining_superclass__")
        {
            return Ok(Value::Super(superclass.clone()));
        }

        // Get 'this' to find the current instance
        let this_val = self
            .environment
            .borrow()
            .get("this")
            .ok_or_else(|| RuntimeError::type_error("'super' outside of class", expr.span))?;

        // Get the instance's class
        let inst = match this_val {
            Value::Instance(inst) => inst,
            _ => {
                return Err(RuntimeError::type_error(
                    "'super' outside of class",
                    expr.span,
                ))
            }
        };

        // Get the superclass
        let superclass = match &inst.borrow().class.superclass {
            Some(sc) => sc.clone(),
            None => {
                return Err(RuntimeError::type_error(
                    "class has no superclass",
                    expr.span,
                ))
            }
        };

        Ok(Value::Super(superclass))
    }
}
