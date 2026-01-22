//! Test DSL built-in functions for Soli.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register test DSL functions in the environment.
pub fn register_test_builtins(env: &mut Environment) {
    env.define(
        "test".to_string(),
        Value::NativeFunction(NativeFunction::new("test", Some(2), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "describe".to_string(),
        Value::NativeFunction(NativeFunction::new("describe", Some(2), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "context".to_string(),
        Value::NativeFunction(NativeFunction::new("context", Some(2), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "before_each".to_string(),
        Value::NativeFunction(NativeFunction::new("before_each", Some(1), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "after_each".to_string(),
        Value::NativeFunction(NativeFunction::new("after_each", Some(1), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "before_all".to_string(),
        Value::NativeFunction(NativeFunction::new("before_all", Some(1), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "after_all".to_string(),
        Value::NativeFunction(NativeFunction::new("after_all", Some(1), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "pending".to_string(),
        Value::NativeFunction(NativeFunction::new("pending", Some(0), |_args| {
            Err("PENDING".to_string())
        })),
    );

    env.define(
        "skip".to_string(),
        Value::NativeFunction(NativeFunction::new("skip", Some(0), |_args| {
            Err("SKIPPED".to_string())
        })),
    );

    env.define(
        "it".to_string(),
        Value::NativeFunction(NativeFunction::new("it", Some(2), |_args| Ok(Value::Null))),
    );

    env.define(
        "specify".to_string(),
        Value::NativeFunction(NativeFunction::new("specify", Some(2), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "expect".to_string(),
        Value::NativeFunction(NativeFunction::new("expect", Some(1), |args| {
            Ok(args[0].clone())
        })),
    );
}
