//! Clock and timing built-in functions.
//!
//! Provides sleep() and microtime() functions.

use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

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

    // microtime() - Returns current time in microseconds as float
    env.define(
        "microtime".to_string(),
        Value::NativeFunction(NativeFunction::new("microtime", Some(0), |_args| {
            let duration = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?;
            let micros = duration.as_secs() as f64 * 1_000_000.0 + duration.subsec_micros() as f64;
            Ok(Value::Float(micros))
        })),
    );
}
