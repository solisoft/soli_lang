//! Decimal method call implementations.

use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{DecimalValue, Value};
use crate::span::Span;

impl Interpreter {
    /// Handle decimal methods that require arguments.
    pub(crate) fn call_decimal_method(
        &mut self,
        d: DecimalValue,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "round" => self.decimal_round(d, arguments, span),
            "between?" => self.decimal_between(d, arguments, span),
            "clamp" => self.decimal_clamp(d, arguments, span),
            "is_a?" => self.decimal_is_a(arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "decimal".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn decimal_round(
        &self,
        d: DecimalValue,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        use rust_decimal::prelude::*;
        if arguments.is_empty() {
            return Ok(Value::Int(d.0.round().to_i64().unwrap_or(0)));
        }
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let digits = match &arguments[0] {
            Value::Int(n) => *n as u32,
            _ => {
                return Err(RuntimeError::type_error(
                    "round expects an integer argument",
                    span,
                ))
            }
        };
        let rounded = d.0.round_dp(digits);
        Ok(Value::Decimal(DecimalValue(rounded, digits)))
    }

    fn decimal_between(
        &self,
        d: DecimalValue,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let val = d.to_f64();
        let min = match &arguments[0] {
            Value::Int(m) => *m as f64,
            Value::Float(m) => *m,
            Value::Decimal(m) => m.to_f64(),
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
            Value::Decimal(m) => m.to_f64(),
            _ => {
                return Err(RuntimeError::type_error(
                    "between? expects numeric arguments",
                    span,
                ))
            }
        };
        Ok(Value::Bool(val >= min && val <= max))
    }

    fn decimal_clamp(
        &self,
        d: DecimalValue,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let min = match &arguments[0] {
            Value::Decimal(m) => m.0,
            Value::Int(m) => rust_decimal::Decimal::from(*m),
            _ => {
                return Err(RuntimeError::type_error(
                    "clamp expects decimal or integer arguments",
                    span,
                ))
            }
        };
        let max = match &arguments[1] {
            Value::Decimal(m) => m.0,
            Value::Int(m) => rust_decimal::Decimal::from(*m),
            _ => {
                return Err(RuntimeError::type_error(
                    "clamp expects decimal or integer arguments",
                    span,
                ))
            }
        };
        let clamped = d.0.max(min).min(max);
        Ok(Value::Decimal(DecimalValue(clamped, d.1)))
    }

    fn decimal_is_a(&self, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
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
            class_name == "decimal" || class_name == "numeric" || class_name == "object",
        ))
    }
}
