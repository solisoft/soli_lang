//! Float method call implementations.
//!
//! All float methods are pure, so the dispatch lives in
//! `call_float_method_impl`, shared by the tree-walker and the VM
//! (`vm_primitive_methods.rs`).

use std::cell::RefCell;
use std::rc::Rc;

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
        call_float_method_impl(n, method_name, &arguments, span)
    }
}

/// Float method dispatch shared by the tree-walker and the VM.
pub(crate) fn call_float_method_impl(
    n: f64,
    method_name: &str,
    arguments: &[Value],
    span: Span,
) -> RuntimeResult<Value> {
    match method_name {
        "round" => float_round(n, arguments, span),
        "between?" => float_between(n, arguments, span),
        "clamp" => float_clamp(n, arguments, span),
        "is_a?" => float_is_a(arguments, span),
        "divmod" => {
            if arguments.len() != 1 {
                return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
            }
            let divisor = match &arguments[0] {
                Value::Int(d) if *d != 0 => *d as f64,
                Value::Float(d) if *d != 0.0 => *d,
                Value::Int(_) | Value::Float(_) => {
                    return Err(RuntimeError::type_error("divmod: division by zero", span))
                }
                other => {
                    return Err(RuntimeError::type_error(
                        format!("divmod expects numeric, got {}", other.type_name()),
                        span,
                    ))
                }
            };
            let quotient = (n / divisor).floor();
            Ok(Value::Array(Rc::new(RefCell::new(vec![
                Value::Float(quotient),
                Value::Float(n - quotient * divisor),
            ]))))
        }
        _ => Err(RuntimeError::NoSuchProperty {
            value_type: "float".to_string(),
            property: method_name.to_string(),
            span,
        }),
    }
}

fn float_round(n: f64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
    use rust_decimal::prelude::ToPrimitive;
    use rust_decimal::{Decimal, RoundingStrategy};
    use std::str::FromStr;

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

    if !n.is_finite() {
        return Ok(Value::Float(n));
    }

    // Round through the shortest round-trip decimal representation so
    // `38.995.round(2) == 39.0` (matching Ruby) instead of `38.99` — a
    // binary-float artifact, since `38.995 * 100 == 3899.4999...` in
    // IEEE 754. `format!("{}", n)` uses Grisu, giving the shortest
    // decimal that round-trips back to `n`.
    let naive_fallback = || {
        let factor = 10_f64.powi(digits as i32);
        Value::Float((n * factor).round() / factor)
    };
    let d = match Decimal::from_str(&format!("{}", n)) {
        Ok(d) => d,
        Err(_) => return Ok(naive_fallback()),
    };

    let rounded = if digits >= 0 {
        let dp = (digits as u32).min(28);
        d.round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero)
    } else {
        let abs = (-digits) as u32;
        if abs > 28 {
            return Ok(Value::Float(0.0));
        }
        let scale = match Decimal::try_from_i128_with_scale(10_i128.pow(abs), 0) {
            Ok(s) => s,
            Err(_) => return Ok(naive_fallback()),
        };
        let scaled = d / scale;
        let rounded_scaled =
            scaled.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
        rounded_scaled * scale
    };

    Ok(Value::Float(rounded.to_f64().unwrap_or_else(|| {
        if let Value::Float(f) = naive_fallback() {
            f
        } else {
            n
        }
    })))
}

fn float_between(n: f64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
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

fn float_clamp(n: f64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
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

fn float_is_a(arguments: &[Value], span: Span) -> RuntimeResult<Value> {
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
        class_name == "float" || class_name == "numeric" || class_name == "object",
    ))
}
