//! String manipulation built-in functions.
//! Most string methods are available via the String class (e.g., "hello".upcase()).
//! These standalone functions are provided for convenience and backward compatibility.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register all string manipulation built-in functions.
pub fn register_string_builtins(env: &mut Environment) {
    // starts_with(string, prefix) -> bool
    env.define(
        "starts_with".to_string(),
        Value::NativeFunction(NativeFunction::new("starts_with", Some(2), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "starts_with() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let prefix = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "starts_with() expects string prefix, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::Bool(s.starts_with(prefix.as_str())))
        })),
    );

    // ends_with(string, suffix) -> bool
    env.define(
        "ends_with".to_string(),
        Value::NativeFunction(NativeFunction::new("ends_with", Some(2), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ends_with() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let suffix = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "ends_with() expects string suffix, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::Bool(s.ends_with(suffix.as_str())))
        })),
    );

    // replace(string, old, new) -> string
    env.define(
        "replace".to_string(),
        Value::NativeFunction(NativeFunction::new("replace", Some(3), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "replace() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let old = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "replace() expects string pattern, got {}",
                        other.type_name()
                    ))
                }
            };
            let new = match &args[2] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "replace() expects string replacement, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(s.replace(old.as_str(), new.as_str())))
        })),
    );

    // contains(string, substring) -> bool
    env.define(
        "contains".to_string(),
        Value::NativeFunction(NativeFunction::new("contains", Some(2), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "contains() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let substr = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "contains() expects string substring, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::Bool(s.contains(substr.as_str())))
        })),
    );
}
