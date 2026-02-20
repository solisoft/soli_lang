//! Type coercion for validation.

use crate::interpreter::value::Value;

use super::create_error;
use super::validator::ValidatorType;

/// Coerce a value to the expected type.
/// Returns Err with validation error if coercion fails.
pub fn coerce_value(value: &Value, target_type: &ValidatorType) -> Result<Value, Value> {
    match target_type {
        ValidatorType::String => coerce_to_string(value),
        ValidatorType::Int => coerce_to_int(value),
        ValidatorType::Float => coerce_to_float(value),
        ValidatorType::Bool => coerce_to_bool(value),
        ValidatorType::Array => coerce_to_array(value),
        ValidatorType::Hash => coerce_to_hash(value),
    }
}

/// Coerce a value to a string.
fn coerce_to_string(value: &Value) -> Result<Value, Value> {
    match value {
        Value::String(_) => Ok(value.clone()),
        Value::Int(n) => Ok(Value::String(n.to_string())),
        Value::Float(n) => Ok(Value::String(n.to_string())),
        Value::Bool(b) => Ok(Value::String(b.to_string())),
        Value::Null => Ok(Value::String(String::new())),
        _ => Err(create_error(
            "",
            &format!("cannot convert {} to string", value.type_name()),
            "type_error",
        )),
    }
}

/// Coerce a value to an integer.
fn coerce_to_int(value: &Value) -> Result<Value, Value> {
    match value {
        Value::Int(_) => Ok(value.clone()),
        Value::Float(f) => Ok(Value::Int(*f as i64)),
        Value::String(s) => {
            // Try to parse as integer
            if let Ok(n) = s.trim().parse::<i64>() {
                return Ok(Value::Int(n));
            }
            // Try to parse as float and convert
            if let Ok(f) = s.trim().parse::<f64>() {
                return Ok(Value::Int(f as i64));
            }
            Err(create_error(
                "",
                &format!("cannot convert '{}' to int", s),
                "type_error",
            ))
        }
        Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
        _ => Err(create_error(
            "",
            &format!("cannot convert {} to int", value.type_name()),
            "type_error",
        )),
    }
}

/// Coerce a value to a float.
fn coerce_to_float(value: &Value) -> Result<Value, Value> {
    match value {
        Value::Float(_) => Ok(value.clone()),
        Value::Int(n) => Ok(Value::Float(*n as f64)),
        Value::String(s) => {
            if let Ok(f) = s.trim().parse::<f64>() {
                return Ok(Value::Float(f));
            }
            Err(create_error(
                "",
                &format!("cannot convert '{}' to float", s),
                "type_error",
            ))
        }
        Value::Bool(b) => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
        _ => Err(create_error(
            "",
            &format!("cannot convert {} to float", value.type_name()),
            "type_error",
        )),
    }
}

/// Coerce a value to a boolean.
fn coerce_to_bool(value: &Value) -> Result<Value, Value> {
    match value {
        Value::Bool(_) => Ok(value.clone()),
        Value::Int(n) => Ok(Value::Bool(*n != 0)),
        Value::Float(f) => Ok(Value::Bool(*f != 0.0)),
        Value::String(s) => {
            let s_lower = s.trim().to_lowercase();
            match s_lower.as_str() {
                "true" | "1" | "yes" | "on" => Ok(Value::Bool(true)),
                "false" | "0" | "no" | "off" | "" => Ok(Value::Bool(false)),
                _ => Err(create_error(
                    "",
                    &format!("cannot convert '{}' to bool", s),
                    "type_error",
                )),
            }
        }
        Value::Null => Ok(Value::Bool(false)),
        _ => Err(create_error(
            "",
            &format!("cannot convert {} to bool", value.type_name()),
            "type_error",
        )),
    }
}

/// Coerce a value to an array.
fn coerce_to_array(value: &Value) -> Result<Value, Value> {
    match value {
        Value::Array(_) => Ok(value.clone()),
        _ => Err(create_error(
            "",
            &format!("cannot convert {} to array", value.type_name()),
            "type_error",
        )),
    }
}

/// Coerce a value to a hash.
fn coerce_to_hash(value: &Value) -> Result<Value, Value> {
    match value {
        Value::Hash(_) => Ok(value.clone()),
        _ => Err(create_error(
            "",
            &format!("cannot convert {} to hash", value.type_name()),
            "type_error",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coerce_string_to_int() {
        let result = coerce_to_int(&Value::String("123".to_string()));
        assert!(result.is_ok());
        if let Ok(Value::Int(n)) = result {
            assert_eq!(n, 123);
        } else {
            panic!("Expected Int");
        }
    }

    #[test]
    fn test_coerce_string_to_float() {
        let result = coerce_to_float(&Value::String("3.14".to_string()));
        assert!(result.is_ok());
        if let Ok(Value::Float(f)) = result {
            assert!((f - std::f64::consts::PI).abs() < 0.001);
        } else {
            panic!("Expected Float");
        }
    }

    #[test]
    fn test_coerce_string_to_bool() {
        assert!(matches!(
            coerce_to_bool(&Value::String("true".to_string())),
            Ok(Value::Bool(true))
        ));
        assert!(matches!(
            coerce_to_bool(&Value::String("false".to_string())),
            Ok(Value::Bool(false))
        ));
        assert!(matches!(
            coerce_to_bool(&Value::String("1".to_string())),
            Ok(Value::Bool(true))
        ));
        assert!(matches!(
            coerce_to_bool(&Value::String("0".to_string())),
            Ok(Value::Bool(false))
        ));
    }
}
