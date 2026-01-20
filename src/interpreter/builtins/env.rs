//! Environment variable built-in functions.

use std::env;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

pub fn register_env_builtins(env: &mut Environment) {
    env.define(
        "getenv".to_string(),
        Value::NativeFunction(NativeFunction::new("getenv", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "getenv() expects string name, got {}",
                        other.type_name()
                    ))
                }
            };

            match env::var(&name) {
                Ok(value) => Ok(Value::String(value)),
                Err(_) => Ok(Value::Null),
            }
        })),
    );

    env.define(
        "setenv".to_string(),
        Value::NativeFunction(NativeFunction::new("setenv", Some(2), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "setenv() expects string name, got {}",
                        other.type_name()
                    ))
                }
            };

            let value = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "setenv() expects string value, got {}",
                        other.type_name()
                    ))
                }
            };

            env::set_var(name, value);
            Ok(Value::Null)
        })),
    );

    env.define(
        "unsetenv".to_string(),
        Value::NativeFunction(NativeFunction::new("unsetenv", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "unsetenv() expects string name, got {}",
                        other.type_name()
                    ))
                }
            };

            env::remove_var(name);
            Ok(Value::Null)
        })),
    );

    env.define(
        "hasenv".to_string(),
        Value::NativeFunction(NativeFunction::new("hasenv", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "hasenv() expects string name, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(Value::Bool(env::var(&name).is_ok()))
        })),
    );
}
