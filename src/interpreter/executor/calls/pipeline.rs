//! Pipeline operator evaluation (|>).

use crate::ast::expr::Argument;
use crate::ast::{Expr, ExprKind};
use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Evaluate pipeline expression: left |> right
    /// x |> foo() becomes foo(x)
    /// x |> f becomes f(x)
    pub(crate) fn evaluate_pipeline(
        &mut self,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> RuntimeResult<Value> {
        let left_val = self.evaluate(left)?;

        match &right.kind {
            ExprKind::Call { callee, arguments } => {
                // Check for array methods: map, filter, each
                if let ExprKind::Variable(name) = &callee.kind {
                    if matches!(name.as_str(), "map" | "filter" | "each") {
                        let resolved = left_val
                            .resolve()
                            .map_err(|e| RuntimeError::type_error(e, span))?;
                        if let Value::Array(arr) = resolved {
                            let items: Vec<Value> = arr.borrow().clone();
                            let mut args = Vec::new();
                            for arg in arguments {
                                match arg {
                                    Argument::Positional(expr) => {
                                        args.push(self.evaluate(expr)?);
                                    }
                                    Argument::Named(_) => {
                                        return Err(RuntimeError::type_error(
                                            "pipeline method does not support named arguments",
                                            span,
                                        ));
                                    }
                                }
                            }
                            return self.call_array_method(&items, name, args, span);
                        } else {
                            return Err(RuntimeError::type_error(
                                format!("{}() expects array, got {}", name, resolved.type_name()),
                                span,
                            ));
                        }
                    }
                }

                // Prepend left_val to arguments
                let mut new_args = vec![left_val];
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => {
                            new_args.push(self.evaluate(expr)?);
                        }
                        Argument::Named(_) => {
                            return Err(RuntimeError::type_error(
                                "pipeline method does not support named arguments",
                                span,
                            ));
                        }
                    }
                }

                // Bypass auto-invoke for Member/SafeMember callees in pipelines
                let callee_val = match &callee.kind {
                    ExprKind::Member { object, name } => {
                        self.evaluate_member(object, name, callee.span)?
                    }
                    ExprKind::SafeMember { object, name } => {
                        self.evaluate_safe_member(object, name, callee.span)?
                    }
                    _ => self.evaluate(callee)?,
                };
                self.call_value(callee_val, new_args, span)
            }
            _ => {
                // Try evaluating right as a function value
                let right_val = self.evaluate(right)?;
                match right_val {
                    Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => {
                        self.call_value(right_val, vec![left_val], span)
                    }
                    _ => Err(RuntimeError::type_error(
                        "right side of pipeline must be a function call or a function value",
                        right.span,
                    )),
                }
            }
        }
    }
}
