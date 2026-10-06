//! Argument helpers several builtins wrote for themselves.

use crate::interpreter::value::{HashKey, HashPairs, Value};

/// A string argument, or `"<context> expects string argument"`.
pub(crate) fn expect_string(value: &Value, context: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.to_string()),
        _ => Err(format!("{} expects string argument", context)),
    }
}

/// `args[idx]` as a string, naming the function and parameter when it is
/// missing or of another type.
pub(crate) fn string_arg(
    args: &[Value],
    idx: usize,
    fn_name: &str,
    param: &str,
) -> Result<String, String> {
    match args.get(idx) {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(format!(
            "{}() expects string {}, got {}",
            fn_name,
            param,
            other.type_name()
        )),
        None => Err(format!("{}() missing argument: {}", fn_name, param)),
    }
}

/// The string under `key`, if there is one.
pub(crate) fn hash_str(hash: &HashPairs, key: &str) -> Option<String> {
    match hash.get(&HashKey::String(key.into())) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// A string's UTF-8 bytes, or a byte array (`Int`s 0–255) as bytes. `what`
/// prefixes the error.
pub(crate) fn value_to_raw_bytes(value: &Value, what: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                Value::Int(n) => Err(format!("{}: byte value {} out of range 0-255", what, n)),
                other => Err(format!(
                    "{}: expected byte (Int 0-255), got {}",
                    what,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{}: expected string or byte array, got {}",
            what,
            other.type_name()
        )),
    }
}
