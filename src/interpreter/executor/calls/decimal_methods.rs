//! Decimal method call implementations.
//!
//! All decimal methods are pure, so the dispatch lives in
//! `call_decimal_method_impl`, shared by the tree-walker and the VM
//! (`vm_primitive_methods.rs`).

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
        call_decimal_method_impl(&d, method_name, &arguments, span)
    }
}

/// Decimal method dispatch shared by the tree-walker and the VM.
pub(crate) fn call_decimal_method_impl(
    d: &DecimalValue,
    method_name: &str,
    arguments: &[Value],
    span: Span,
) -> RuntimeResult<Value> {
    match method_name {
        "round" => decimal_round(d, arguments, span),
        "between?" => decimal_between(d, arguments, span),
        "clamp" => decimal_clamp(d, arguments, span),
        "is_a?" => decimal_is_a(arguments, span),
        _ => Err(RuntimeError::NoSuchProperty {
            value_type: "decimal".to_string(),
            property: method_name.to_string(),
            span,
        }),
    }
}

fn decimal_round(d: &DecimalValue, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
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

fn decimal_between(d: &DecimalValue, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
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

fn decimal_clamp(d: &DecimalValue, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
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

fn decimal_is_a(arguments: &[Value], span: Span) -> RuntimeResult<Value> {
    if arguments.len() != 1 {
        return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
    }
    let class_name = match &arguments[0] {
        Value::String(s) => s.as_ref(),
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
