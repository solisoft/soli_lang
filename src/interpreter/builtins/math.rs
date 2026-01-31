//! Math operation built-in functions.
//!
//! Provides functions for numeric operations like range, absolute value, min/max, etc.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

/// Register all math built-in functions.
pub fn register_math_builtins(env: &mut Environment) {
    // range(start, end) - Create array from start to end-1 (exclusive)
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

    // Register the Math class with static methods and constants
    register_math_class(env);
}

/// Register the Math class with static methods (floor, ceil, round, random, log, log10)
/// and constants (pi, e).
fn register_math_class(env: &mut Environment) {
    let mut math_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Math.floor(n) - Round down to nearest integer
    math_static_methods.insert(
        "floor".to_string(),
        Rc::new(NativeFunction::new(
            "Math.floor",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Float(n) => Ok(Value::Int(n.floor() as i64)),
                other => Err(format!(
                    "Math.floor() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Math.ceil(n) - Round up to nearest integer
    math_static_methods.insert(
        "ceil".to_string(),
        Rc::new(NativeFunction::new(
            "Math.ceil",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Float(n) => Ok(Value::Int(n.ceil() as i64)),
                other => Err(format!(
                    "Math.ceil() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Math.round(n) - Round to nearest integer
    math_static_methods.insert(
        "round".to_string(),
        Rc::new(NativeFunction::new(
            "Math.round",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Float(n) => Ok(Value::Int(n.round() as i64)),
                other => Err(format!(
                    "Math.round() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Math.random() - Random float between 0 and 1
    math_static_methods.insert(
        "random".to_string(),
        Rc::new(NativeFunction::new("Math.random", Some(0), |_args| {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            Ok(Value::Float(rng.gen::<f64>()))
        })),
    );

    // Math.log(n) - Natural logarithm
    math_static_methods.insert(
        "log".to_string(),
        Rc::new(NativeFunction::new(
            "Math.log",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Float((*n as f64).ln())),
                Value::Float(n) => Ok(Value::Float(n.ln())),
                other => Err(format!(
                    "Math.log() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Math.log10(n) - Base-10 logarithm
    math_static_methods.insert(
        "log10".to_string(),
        Rc::new(NativeFunction::new(
            "Math.log10",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Float((*n as f64).log10())),
                Value::Float(n) => Ok(Value::Float(n.log10())),
                other => Err(format!(
                    "Math.log10() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Math.sin(n) - Sine
    math_static_methods.insert(
        "sin".to_string(),
        Rc::new(NativeFunction::new(
            "Math.sin",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Float((*n as f64).sin())),
                Value::Float(n) => Ok(Value::Float(n.sin())),
                other => Err(format!(
                    "Math.sin() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Math.cos(n) - Cosine
    math_static_methods.insert(
        "cos".to_string(),
        Rc::new(NativeFunction::new(
            "Math.cos",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Float((*n as f64).cos())),
                Value::Float(n) => Ok(Value::Float(n.cos())),
                other => Err(format!(
                    "Math.cos() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Math.tan(n) - Tangent
    math_static_methods.insert(
        "tan".to_string(),
        Rc::new(NativeFunction::new(
            "Math.tan",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Float((*n as f64).tan())),
                Value::Float(n) => Ok(Value::Float(n.tan())),
                other => Err(format!(
                    "Math.tan() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Math.exp(n) - e^n
    math_static_methods.insert(
        "exp".to_string(),
        Rc::new(NativeFunction::new(
            "Math.exp",
            Some(1),
            |args| match &args[0] {
                Value::Int(n) => Ok(Value::Float((*n as f64).exp())),
                Value::Float(n) => Ok(Value::Float(n.exp())),
                other => Err(format!(
                    "Math.exp() expects number, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Create static fields for constants
    let static_fields: Rc<RefCell<HashMap<String, Value>>> = Rc::new(RefCell::new(HashMap::new()));
    static_fields
        .borrow_mut()
        .insert("pi".to_string(), Value::Float(std::f64::consts::PI));
    static_fields
        .borrow_mut()
        .insert("e".to_string(), Value::Float(std::f64::consts::E));

    let math_class = Class {
        name: "Math".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: math_static_methods,
        native_methods: HashMap::new(),
        static_fields,
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    };

    env.define("Math".to_string(), Value::Class(Rc::new(math_class)));
}
