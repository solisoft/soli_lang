//! Type conversion built-in functions.
//!
//! Provides functions for converting between types and inspecting type information.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register all type conversion built-in functions.
pub fn register_type_builtins(env: &mut Environment) {
    // str(value) - Convert to string (auto-resolves Futures)
    env.define(
        "str".to_string(),
        Value::NativeFunction(NativeFunction::new("str", Some(1), |args| {
            let resolved = args.into_iter().next().unwrap().resolve()?;
            Ok(Value::String(format!("{}", resolved)))
        })),
    );

    // int(value) - Convert to int
    env.define(
        "int".to_string(),
        Value::NativeFunction(NativeFunction::new("int", Some(1), |args| match &args[0] {
            Value::Int(n) => Ok(Value::Int(*n)),
            Value::Float(n) => Ok(Value::Int(*n as i64)),
            Value::String(s) => s
                .parse::<i64>()
                .map(Value::Int)
                .map_err(|_| format!("cannot convert '{}' to int", s)),
            Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
            other => Err(format!("cannot convert {} to int", other.type_name())),
        })),
    );

    // float(value) - Convert to float
    env.define(
        "float".to_string(),
        Value::NativeFunction(NativeFunction::new("float", Some(1), |args| {
            match &args[0] {
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                Value::Float(n) => Ok(Value::Float(*n)),
                Value::String(s) => s
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| format!("cannot convert '{}' to float", s)),
                other => Err(format!("cannot convert {} to float", other.type_name())),
            }
        })),
    );

    // type(value) - Get type name as string
    env.define(
        "type".to_string(),
        Value::NativeFunction(NativeFunction::new("type", Some(1), |args| {
            Ok(Value::String(args[0].type_name().to_string()))
        })),
    );
}
