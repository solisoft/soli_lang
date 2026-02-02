//! Array literal evaluation.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{Expr, ExprKind};
use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;

impl Interpreter {
    /// Evaluate array literal expression: [elements]
    pub(crate) fn evaluate_array(&mut self, elements: &Vec<Expr>) -> RuntimeResult<Value> {
        let mut values = Vec::new();
        for elem in elements {
            match &elem.kind {
                ExprKind::Spread(inner) => {
                    // Evaluate the spread expression and extend with its elements
                    let spread_val = self.evaluate(inner)?;
                    match spread_val {
                        Value::Array(ref arr) => {
                            let arr = arr.borrow();
                            values.extend(arr.clone());
                        }
                        _ => {
                            return Err(RuntimeError::type_error(
                                "cannot spread non-array value",
                                elem.span,
                            ));
                        }
                    }
                }
                _ => {
                    values.push(self.evaluate(elem)?);
                }
            }
        }
        Ok(Value::Array(Rc::new(RefCell::new(values))))
    }
}
