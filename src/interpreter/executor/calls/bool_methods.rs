//! Bool method call implementations.

use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Handle bool methods that require arguments.
    pub(crate) fn call_bool_method(
        &mut self,
        _b: bool,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "is_a?" => self.bool_is_a(arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "bool".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn bool_is_a(&self, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
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
        Ok(Value::Bool(class_name == "bool" || class_name == "object"))
    }
}
