//! Null method call implementations.

use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Handle null methods that require arguments.
    pub(crate) fn call_null_method(
        &mut self,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "is_a?" => self.null_is_a(arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "null".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn null_is_a(&self, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let class_name = match &arguments[0] {
            Value::String(s) => s.as_str(),
            _ => {
                return Err(RuntimeError::type_error(
                    "is_a? expects a string argument",
                    span,
                ))
            }
        };
        Ok(Value::Bool(class_name == "null" || class_name == "object"))
    }
}
