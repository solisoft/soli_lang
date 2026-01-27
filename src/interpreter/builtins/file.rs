//! File I/O built-in functions.
//!
//! Provides functions for reading and writing files.

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value, NativeFunction, Value};

/// Register all file I/O built-in functions.
pub fn register_file_builtins(env: &mut Environment) {
    // barf(path, content) - Write file (auto-detects text vs binary)
    env.define(
        "barf".to_string(),
        Value::NativeFunction(NativeFunction::new("barf", None, |args| match &args[..] {
            [Value::String(path), Value::String(content)] => {
                fs::write(path, content)
                    .map_err(|e| format!("barf failed to write {}: {}", path, e))?;
                Ok(Value::Null)
            }
            [Value::String(path), Value::Array(bytes)] => {
                let byte_vec: Result<Vec<u8>, String> = bytes
                    .borrow()
                    .iter()
                    .map(|b| match b {
                        Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                        Value::Int(n) => Err(format!("byte value {} out of range", n)),
                        other => Err(format!("expected byte, got {}", other.type_name())),
                    })
                    .collect();
                fs::write(path, byte_vec?)
                    .map_err(|e| format!("barf failed to write {}: {}", path, e))?;
                Ok(Value::Null)
            }
            _ => Err("barf expects (string, string) or (string, array<int>)".to_string()),
        })),
    );

    // slurp(path) or slurp(path, mode) - Read file (text or binary)
    env.define(
        "slurp".to_string(),
        Value::NativeFunction(NativeFunction::new("slurp", None, |args| match &args[..] {
            [Value::String(path)] => fs::read_to_string(path)
                .map(Value::String)
                .map_err(|e| format!("slurp failed to read {}: {}", path, e)),
            [Value::String(path), Value::String(mode)] => {
                if mode == "binary" {
                    let bytes = fs::read(path)
                        .map_err(|e| format!("slurp failed to read {}: {}", path, e))?;
                    let value_bytes: Vec<Value> =
                        bytes.iter().map(|&b| Value::Int(b as i64)).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(value_bytes))))
                } else {
                    fs::read_to_string(path)
                        .map(Value::String)
                        .map_err(|e| format!("slurp failed to read {}: {}", path, e))
                }
            }
            _ => Err("slurp expects path or (path, mode)".to_string()),
        })),
    );

    // slurp_json(path) - Read and parse JSON file
    env.define(
        "slurp_json".to_string(),
        Value::NativeFunction(NativeFunction::new("slurp_json", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("slurp_json expects a string path".to_string()),
            };
            let content = fs::read_to_string(&path)
                .map_err(|e| format!("slurp_json failed to read {}: {}", path, e))?;
            let json: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| format!("slurp_json failed to parse {}: {}", path, e))?;
            json_to_value(&json)
        })),
    );
}
