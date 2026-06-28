//! Type conversion built-in functions.
//!
//! Provides functions for converting between types and inspecting type information.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, Instance, NativeFunction, Value};

/// Register all type conversion built-in functions.
pub fn register_type_builtins(env: &mut Environment) {
    // str(value) - Convert to string (auto-resolves Futures)
    env.define(
        "str".to_string(),
        Value::NativeFunction(NativeFunction::new("str", Some(1), |args| {
            let resolved = args.into_iter().next().unwrap().resolve()?;
            Ok(Value::String(format!("{}", resolved).into()))
        })),
    );

    // __enum_construct(EnumClass, "Variant", {field: value, ...}) — internal
    // helper emitted by enum lowering. Builds a tagged enum-value instance,
    // setting `__variant` (which `.new`'s `_`-prefix skip would otherwise drop).
    env.define(
        "__enum_construct".to_string(),
        Value::NativeFunction(NativeFunction::new("__enum_construct", Some(3), |args| {
            let class_rc = match &args[0] {
                Value::Class(c) => c.clone(),
                other => {
                    return Err(format!(
                        "__enum_construct expected an enum class, got {}",
                        other.type_name()
                    ))
                }
            };
            let variant = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "__enum_construct expected a variant name, got {}",
                        other.type_name()
                    ))
                }
            };
            let mut instance = Instance::new(class_rc);
            instance.set("__variant".to_string(), Value::String(variant));
            if let Value::Hash(pairs) = &args[2] {
                for (key, value) in pairs.borrow().iter() {
                    if let HashKey::String(field) = key {
                        instance.set(field.to_string(), value.clone());
                    }
                }
            }
            Ok(Value::Instance(Rc::new(RefCell::new(instance))))
        })),
    );

    // __enum_from(EnumClass, stored) — internal helper behind `Enum.from(...)`.
    // Rebuilds an enum value from its stored DB/JSON shape (a tag string for a
    // unit variant, or a { "variant": ..., ...payload } object).
    env.define(
        "__enum_from".to_string(),
        Value::NativeFunction(NativeFunction::new("__enum_from", Some(2), |args| {
            let class_rc = match &args[0] {
                Value::Class(c) => c.clone(),
                other => {
                    return Err(format!(
                        "__enum_from expected an enum class, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(crate::interpreter::value::build_enum_value(
                &class_rc, &args[1],
            ))
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
            Ok(Value::String(args[0].type_name().to_string().into()))
        })),
    );
}
