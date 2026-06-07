//! Int method call implementations.
//!
//! The closure-taking methods (`times`, `upto`, `downto`) need the
//! interpreter to execute blocks and live on `impl Interpreter`. Everything
//! else is pure and lives in `call_int_method_impl`, shared with the VM
//! (`vm_int_methods.rs`).

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{ControlFlow, Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Handle int methods that require arguments.
    pub(crate) fn call_int_method(
        &mut self,
        n: i64,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "times" => self.int_times(n, arguments, span),
            "upto" => self.int_upto(n, arguments, span),
            "downto" => self.int_downto(n, arguments, span),
            _ => call_int_method_impl(n, method_name, &arguments, span),
        }
    }

    fn int_times(&mut self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "times expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        for i in 0..n {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), Value::Int(i));

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) | ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in int.times", span));
                }
            }
        }

        Ok(Value::Int(n))
    }

    fn int_upto(&mut self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let limit = match &arguments[0] {
            Value::Int(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "upto expects an integer limit",
                    span,
                ))
            }
        };
        let func = match &arguments[1] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "upto expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        for i in n..=limit {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), Value::Int(i));

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) | ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in int.upto", span));
                }
            }
        }

        Ok(Value::Int(n))
    }

    fn int_downto(&mut self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let limit = match &arguments[0] {
            Value::Int(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "downto expects an integer limit",
                    span,
                ))
            }
        };
        let func = match &arguments[1] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "downto expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        let mut i = n;
        while i >= limit {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), Value::Int(i));

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) | ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in int.downto", span));
                }
            }
            i -= 1;
        }

        Ok(Value::Int(n))
    }
}

/// Pure int methods (everything except the closure-taking
/// `times`/`upto`/`downto`), shared by the tree-walker and the VM.
pub(crate) fn call_int_method_impl(
    n: i64,
    method_name: &str,
    arguments: &[Value],
    span: Span,
) -> RuntimeResult<Value> {
    match method_name {
        "pow" => int_pow(n, arguments, span),
        "gcd" => int_gcd(n, arguments, span),
        "lcm" => int_lcm(n, arguments, span),
        "between?" => int_between(n, arguments, span),
        "clamp" => int_clamp(n, arguments, span),
        "is_a?" => int_is_a(arguments, span),
        "to_s" | "to_string" => int_to_s(n, arguments, span),
        "succ" | "next" => {
            if !arguments.is_empty() {
                return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
            }
            Ok(Value::Int(n + 1))
        }
        "pred" => {
            if !arguments.is_empty() {
                return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
            }
            Ok(Value::Int(n - 1))
        }
        "divmod" => {
            if arguments.len() != 1 {
                return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
            }
            let divisor = match &arguments[0] {
                Value::Int(d) if *d != 0 => *d,
                Value::Int(_) => {
                    return Err(RuntimeError::type_error("divmod: division by zero", span))
                }
                other => {
                    return Err(RuntimeError::type_error(
                        format!("divmod expects integer, got {}", other.type_name()),
                        span,
                    ))
                }
            };
            Ok(Value::Array(Rc::new(RefCell::new(vec![
                Value::Int(n.div_euclid(divisor)),
                Value::Int(n.rem_euclid(divisor)),
            ]))))
        }
        _ => Err(RuntimeError::NoSuchProperty {
            value_type: "int".to_string(),
            property: method_name.to_string(),
            span,
        }),
    }
}

fn int_pow(n: i64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
    if arguments.len() != 1 {
        return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
    }
    match &arguments[0] {
        Value::Int(exp) => {
            if *exp < 0 {
                Ok(Value::Float((n as f64).powi(*exp as i32)))
            } else {
                Ok(Value::Int(n.pow(*exp as u32)))
            }
        }
        Value::Float(exp) => Ok(Value::Float((n as f64).powf(*exp))),
        _ => Err(RuntimeError::type_error(
            "pow expects a numeric argument",
            span,
        )),
    }
}

fn int_gcd(n: i64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
    if arguments.len() != 1 {
        return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
    }
    let other = match &arguments[0] {
        Value::Int(m) => *m,
        _ => {
            return Err(RuntimeError::type_error(
                "gcd expects an integer argument",
                span,
            ))
        }
    };
    Ok(Value::Int(gcd(n, other)))
}

fn int_lcm(n: i64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
    if arguments.len() != 1 {
        return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
    }
    let other = match &arguments[0] {
        Value::Int(m) => *m,
        _ => {
            return Err(RuntimeError::type_error(
                "lcm expects an integer argument",
                span,
            ))
        }
    };
    if n == 0 && other == 0 {
        Ok(Value::Int(0))
    } else {
        Ok(Value::Int((n / gcd(n, other) * other).abs()))
    }
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

fn int_between(n: i64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
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
    Ok(Value::Bool((n as f64) >= min && (n as f64) <= max))
}

fn int_clamp(n: i64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
    if arguments.len() != 2 {
        return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
    }
    let min = match &arguments[0] {
        Value::Int(m) => *m,
        _ => {
            return Err(RuntimeError::type_error(
                "clamp expects integer arguments",
                span,
            ))
        }
    };
    let max = match &arguments[1] {
        Value::Int(m) => *m,
        _ => {
            return Err(RuntimeError::type_error(
                "clamp expects integer arguments",
                span,
            ))
        }
    };
    Ok(Value::Int(n.max(min).min(max)))
}

fn int_is_a(arguments: &[Value], span: Span) -> RuntimeResult<Value> {
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
        class_name == "int" || class_name == "numeric" || class_name == "object",
    ))
}

/// `n.to_s` — decimal string. `n.to_s(base)` — Ruby-style radix
/// conversion for bases 2..=36 (lowercase digits, leading `-` for
/// negative numbers).
fn int_to_s(n: i64, arguments: &[Value], span: Span) -> RuntimeResult<Value> {
    if arguments.is_empty() {
        return Ok(Value::String(n.to_string().into()));
    }
    if arguments.len() != 1 {
        return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
    }
    let base = match &arguments[0] {
        Value::Int(b) => *b,
        other => {
            return Err(RuntimeError::type_error(
                format!("to_s expects an integer base, got {}", other.type_name()),
                span,
            ))
        }
    };
    if !(2..=36).contains(&base) {
        return Err(RuntimeError::type_error(
            format!("to_s base must be between 2 and 36, got {}", base),
            span,
        ));
    }
    if base == 10 {
        return Ok(Value::String(n.to_string().into()));
    }
    Ok(Value::String(int_to_radix_string(n, base as u64).into()))
}

/// Render `n` in the given radix (2..=36), lowercase digits with a leading
/// `-` for negative numbers — matching Ruby's `Integer#to_s(base)`. Callers
/// validate the base range.
fn int_to_radix_string(n: i64, base: u64) -> String {
    // `unsigned_abs` so i64::MIN doesn't overflow on negation.
    let mut magnitude = n.unsigned_abs();
    let mut digits = Vec::new();
    loop {
        let digit = (magnitude % base) as u32;
        digits.push(char::from_digit(digit, base as u32).unwrap());
        magnitude /= base;
        if magnitude == 0 {
            break;
        }
    }
    if n < 0 {
        digits.push('-');
    }
    digits.iter().rev().collect()
}
