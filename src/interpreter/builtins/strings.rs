//! String manipulation built-in functions.
//!
//! Provides functions for splitting, joining, searching, and transforming strings.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register all string manipulation built-in functions.
pub fn register_string_builtins(env: &mut Environment) {
    // split(string, delimiter) - Split string by delimiter, return array
    env.define(
        "split".to_string(),
        Value::NativeFunction(NativeFunction::new("split", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(delim)) => {
                    let parts: Vec<Value> = s
                        .split(delim.as_str())
                        .map(|p| Value::String(p.to_string()))
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(parts))))
                }
                _ => Err("split requires (string, string)".to_string()),
            }
        })),
    );

    // join(array, delimiter) - Join array elements with delimiter
    env.define(
        "join".to_string(),
        Value::NativeFunction(NativeFunction::new("join", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Array(arr), Value::String(delim)) => {
                    let parts: Vec<String> =
                        arr.borrow().iter().map(|v| format!("{}", v)).collect();
                    Ok(Value::String(parts.join(delim.as_str())))
                }
                _ => Err("join requires (array, string)".to_string()),
            }
        })),
    );

    // contains(string, substring) - Check if string contains substring
    env.define(
        "contains".to_string(),
        Value::NativeFunction(NativeFunction::new("contains", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(sub)) => Ok(Value::Bool(s.contains(sub.as_str()))),
                _ => Err("contains requires (string, string)".to_string()),
            }
        })),
    );

    // index_of(string, substring) - Find index of substring (-1 if not found)
    env.define(
        "index_of".to_string(),
        Value::NativeFunction(NativeFunction::new("index_of", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(sub)) => {
                    if let Some(idx) = s.find(sub.as_str()) {
                        Ok(Value::Int(idx as i64))
                    } else {
                        Ok(Value::Int(-1))
                    }
                }
                _ => Err("index_of requires (string, string)".to_string()),
            }
        })),
    );

    // substring(string, start, end) - Get substring from start to end
    env.define(
        "substring".to_string(),
        Value::NativeFunction(NativeFunction::new("substring", Some(3), |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(start), Value::Int(end)) => {
                    let start_usize = if *start < 0 { 0 } else { *start as usize };
                    let end_usize = if *end > s.len() as i64 {
                        s.len() as i64
                    } else {
                        *end
                    } as usize;
                    if start_usize >= end_usize || start_usize >= s.len() {
                        return Ok(Value::String(String::new()));
                    }
                    Ok(Value::String(s[start_usize..end_usize].to_string()))
                }
                _ => Err("substring requires (string, int, int)".to_string()),
            }
        })),
    );

    // upcase(string) - Convert to uppercase
    env.define(
        "upcase".to_string(),
        Value::NativeFunction(NativeFunction::new("upcase", Some(1), |args| {
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                other => Err(format!("upcase expects string, got {}", other.type_name())),
            }
        })),
    );

    // downcase(string) - Convert to lowercase
    env.define(
        "downcase".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "downcase",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                other => Err(format!(
                    "downcase expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // trim(string) - Remove whitespace from both ends
    env.define(
        "trim".to_string(),
        Value::NativeFunction(NativeFunction::new("trim", Some(1), |args| {
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                other => Err(format!("trim expects string, got {}", other.type_name())),
            }
        })),
    );

    // replace(string, from, to) - Replace all occurrences of 'from' with 'to'
    env.define(
        "replace".to_string(),
        Value::NativeFunction(NativeFunction::new("replace", Some(3), |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(from), Value::String(to)) => {
                    Ok(Value::String(s.replace(from.as_str(), to.as_str())))
                }
                _ => Err("replace requires (string, string, string)".to_string()),
            }
        })),
    );
}
