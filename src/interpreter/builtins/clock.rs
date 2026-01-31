//! Clock and timing built-in functions.
//!
//! Provides sleep() function. Use DateTime.microtime() for current timestamp.

use std::thread;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register all clock built-in functions.
pub fn register_clock_builtins(env: &mut Environment) {
    // sleep(seconds) - Pause execution for given seconds
    env.define(
        "sleep".to_string(),
        Value::NativeFunction(NativeFunction::new("sleep", Some(1), |args| {
            match &args[0] {
                Value::Int(n) => {
                    thread::sleep(std::time::Duration::from_secs(*n as u64));
                    Ok(Value::Null)
                }
                Value::Float(f) => {
                    thread::sleep(std::time::Duration::from_secs_f64(*f));
                    Ok(Value::Null)
                }
                _ => Err("sleep() expects number".to_string()),
            }
        })),
    );

    // microtime() has been moved to DateTime.microtime()
}
