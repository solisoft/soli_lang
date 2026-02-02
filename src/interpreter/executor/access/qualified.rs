//! Qualified name access evaluation (Module::name).

use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Evaluate qualified name expression: qualifier::name
    pub(crate) fn evaluate_qualified_name(
        &mut self,
        qualifier: &Expr,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Value> {
        let qualifier_val = self.evaluate(qualifier)?;

        match qualifier_val {
            Value::Class(class) => {
                // Check if this is a nested class
                if let Some(nested_class) = class.nested_classes.borrow().get(name) {
                    return Ok(Value::Class(nested_class.clone()));
                }

                // Also check static fields (nested classes might be stored there)
                if let Some(value) = class.static_fields.borrow().get(name).cloned() {
                    return Ok(value);
                }

                Err(RuntimeError::NoSuchProperty {
                    value_type: class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            _ => Err(RuntimeError::type_error(
                format!("'{}' is not a class", qualifier_val.type_name()),
                span,
            )),
        }
    }
}
