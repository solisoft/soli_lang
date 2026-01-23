//! Math operation built-in functions.
//!
//! Provides functions for numeric operations like range, absolute value, min/max, etc.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register all math built-in functions.
pub fn register_math_builtins(env: &mut Environment) {
    // range(start, end) - Create array from start to end-1
    env.define(
        "range".to_string(),
        Value::NativeFunction(NativeFunction::new("range", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(start), Value::Int(end)) => {
                    let arr: Vec<Value> = (*start..*end).map(Value::Int).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(arr))))
                }
                _ => Err("range() expects two integers".to_string()),
            }
        })),
    );

    // abs(number) - Absolute value
    env.define(
        "abs".to_string(),
        Value::NativeFunction(NativeFunction::new("abs", Some(1), |args| match &args[0] {
            Value::Int(n) => Ok(Value::Int(n.abs())),
            Value::Float(n) => Ok(Value::Float(n.abs())),
            other => Err(format!("abs() expects number, got {}", other.type_name())),
        })),
    );

    // min(a, b) - Minimum of two values
    env.define(
        "min".to_string(),
        Value::NativeFunction(NativeFunction::new("min", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a.min(b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.min(*b))),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).min(*b))),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.min(*b as f64))),
                _ => Err("min() expects two numbers".to_string()),
            }
        })),
    );

    // max(a, b) - Maximum of two values
    env.define(
        "max".to_string(),
        Value::NativeFunction(NativeFunction::new("max", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a.max(b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.max(*b))),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).max(*b))),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.max(*b as f64))),
                _ => Err("max() expects two numbers".to_string()),
            }
        })),
    );

    // sqrt(number) - Square root
    env.define(
        "sqrt".to_string(),
        Value::NativeFunction(NativeFunction::new("sqrt", Some(1), |args| {
            match &args[0] {
                Value::Int(n) => Ok(Value::Float((*n as f64).sqrt())),
                Value::Float(n) => Ok(Value::Float(n.sqrt())),
                other => Err(format!("sqrt() expects number, got {}", other.type_name())),
            }
        })),
    );

    // pow(base, exp) - Exponentiation
    env.define(
        "pow".to_string(),
        Value::NativeFunction(NativeFunction::new("pow", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(base), Value::Int(exp)) => {
                    if *exp >= 0 {
                        Ok(Value::Int(base.pow(*exp as u32)))
                    } else {
                        Ok(Value::Float((*base as f64).powi(*exp as i32)))
                    }
                }
                (Value::Float(base), Value::Int(exp)) => Ok(Value::Float(base.powi(*exp as i32))),
                (Value::Int(base), Value::Float(exp)) => {
                    Ok(Value::Float((*base as f64).powf(*exp)))
                }
                (Value::Float(base), Value::Float(exp)) => Ok(Value::Float(base.powf(*exp))),
                _ => Err("pow() expects two numbers".to_string()),
            }
        })),
    );
}
