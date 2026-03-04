//! Float method call implementations.

use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Handle float methods that require arguments.
    pub(crate) fn call_float_method(
        &mut self,
        n: f64,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "round" => self.float_round(n, arguments, span),
            "between?" => self.float_between(n, arguments, span),
            "clamp" => self.float_clamp(n, arguments, span),
            "is_a?" => self.float_is_a(arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "float".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn float_round(&self, n: f64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Ok(Value::Int(n.round() as i64));
        }
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let digits = match &arguments[0] {
            Value::Int(d) => *d,
            _ => {
                return Err(RuntimeError::type_error(
                    "round expects an integer argument",
                    span,
                ))
            }
        };
        let factor = 10_f64.powi(digits as i32);
        Ok(Value::Float((n * factor).round() / factor))
    }

    fn float_between(&self, n: f64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let min = match &arguments[0] {
            Value::Int(m) => *m as f64,
            Value::Float(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "between? expects numeric arguments",
                    span,
                ))
            }
        };
        let max = match &arguments[1] {
            Value::Int(m) => *m as f64,
            Value::Float(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "between? expects numeric arguments",
                    span,
                ))
            }
        };
        Ok(Value::Bool(n >= min && n <= max))
    }

    fn float_clamp(&self, n: f64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let min = match &arguments[0] {
            Value::Int(m) => *m as f64,
            Value::Float(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "clamp expects numeric arguments",
                    span,
                ))
            }
        };
        let max = match &arguments[1] {
            Value::Int(m) => *m as f64,
            Value::Float(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "clamp expects numeric arguments",
                    span,
                ))
            }
        };
        Ok(Value::Float(n.max(min).min(max)))
    }

    fn float_is_a(&self, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
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
        Ok(Value::Bool(
            class_name == "float" || class_name == "numeric" || class_name == "object",
        ))
    }
}
