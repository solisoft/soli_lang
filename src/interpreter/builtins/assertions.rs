//! Test assertions for the Soli test DSL.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};
use regex::Regex;

pub fn register_assertions(env: &mut Environment) {
    env.define(
        "assert".to_string(),
        Value::NativeFunction(NativeFunction::new("assert", Some(1), |args| {
            match &args[0] {
                Value::Bool(true) => Ok(Value::Null),
                Value::Bool(false) => Err("assertion failed".to_string()),
                _ => Err("assert expects boolean".to_string()),
            }
        })),
    );

    env.define(
        "assert_not".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_not",
            Some(1),
            |args| match &args[0] {
                Value::Bool(false) => Ok(Value::Null),
                Value::Bool(true) => Err("assertion failed".to_string()),
                _ => Err("assert_not expects boolean".to_string()),
            },
        )),
    );

    env.define(
        "assert_eq".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_eq", Some(2), |args| {
            if args[0] == args[1] {
                Ok(Value::Null)
            } else {
                Err(format!("values not equal"))
            }
        })),
    );

    env.define(
        "assert_ne".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_ne", Some(2), |args| {
            if args[0] != args[1] {
                Ok(Value::Null)
            } else {
                Err(format!("values should not be equal"))
            }
        })),
    );

    env.define(
        "assert_null".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_null",
            Some(1),
            |args| match &args[0] {
                Value::Null => Ok(Value::Null),
                _ => Err("expected null".to_string()),
            },
        )),
    );

    env.define(
        "assert_not_null".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_not_null",
            Some(1),
            |args| match &args[0] {
                Value::Null => Err("expected non-null".to_string()),
                _ => Ok(Value::Null),
            },
        )),
    );

    env.define(
        "assert_gt".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_gt", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) if a > b => Ok(Value::Null),
                (Value::Float(a), Value::Float(b)) if a > b => Ok(Value::Null),
                _ => Err("assert_gt failed".to_string()),
            }
        })),
    );

    env.define(
        "assert_lt".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_lt", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) if a < b => Ok(Value::Null),
                (Value::Float(a), Value::Float(b)) if a < b => Ok(Value::Null),
                _ => Err("assert_lt failed".to_string()),
            }
        })),
    );

    env.define(
        "assert_match".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_match", Some(2), |args| {
            if let (Value::String(s), Value::String(pattern)) = (&args[0], &args[1]) {
                match Regex::new(pattern) {
                    Ok(re) if re.is_match(s) => Ok(Value::Null),
                    _ => Err("assert_match failed".to_string()),
                }
            } else {
                Err("assert_match expects strings".to_string())
            }
        })),
    );

    env.define(
        "assert_contains".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_contains",
            Some(2),
            |args| match &args[0] {
                Value::Array(arr) if arr.borrow().contains(&args[1]) => Ok(Value::Null),
                Value::String(s) => {
                    if let Value::String(sub) = &args[1] {
                        if s.contains(sub) {
                            Ok(Value::Null)
                        } else {
                            Err("assert_contains failed".to_string())
                        }
                    } else {
                        Err("assert_contains expects string as second argument".to_string())
                    }
                }
                _ => Err("assert_contains failed".to_string()),
            },
        )),
    );

    env.define(
        "assert_hash_has_key".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_hash_has_key",
            Some(2),
            |args| {
                if let Value::Hash(h) = &args[0] {
                    let key = &args[1];
                    let found = h.borrow().iter().any(|(k, _)| k.hash_eq(key));
                    if found {
                        Ok(Value::Null)
                    } else {
                        Err("hash does not contain key".to_string())
                    }
                } else {
                    Err("assert_hash_has_key expects hash".to_string())
                }
            },
        )),
    );

    env.define(
        "assert_json".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_json", Some(1), |args| {
            if let Value::String(s) = &args[0] {
                match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(_) => Ok(Value::Null),
                    Err(_) => Err("invalid JSON".to_string()),
                }
            } else {
                Err("assert_json expects string".to_string())
            }
        })),
    );
}
