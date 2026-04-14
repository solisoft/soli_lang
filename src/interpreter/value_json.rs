use std::cell::RefCell;
use std::rc::Rc;

use ahash::RandomState as AHasher;
use rust_decimal::Decimal;

use crate::interpreter::value::{DecimalValue, HashKey, HashPairs, Value};

/// Convert a serde_json::Value to a Soli Value (consuming — moves strings instead of cloning).
pub fn json_to_value(json: serde_json::Value) -> Result<Value, String> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Err("Invalid JSON number".to_string())
            }
        }
        serde_json::Value::String(s) => {
            if let Ok(d) = s.parse::<Decimal>() {
                let precision = s.split('.').nth(1).map(|p| p.len() as u32).unwrap_or(0);
                Ok(Value::Decimal(DecimalValue(d, precision)))
            } else {
                Ok(Value::String(s))
            }
        }
        serde_json::Value::Array(arr) => {
            let mut items = Vec::with_capacity(arr.len());
            for v in arr {
                items.push(json_to_value(v)?);
            }
            Ok(Value::Array(Rc::new(RefCell::new(items))))
        }
        serde_json::Value::Object(obj) => {
            let mut map = HashPairs::with_capacity_and_hasher(obj.len(), AHasher::default());
            for (k, v) in obj {
                map.insert(HashKey::String(k), json_to_value(v)?);
            }
            Ok(Value::Hash(Rc::new(RefCell::new(map))))
        }
    }
}

/// Convert a serde_json::Value reference to a Soli Value (clones strings).
pub fn json_to_value_ref(json: &serde_json::Value) -> Result<Value, String> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Err("Invalid JSON number".to_string())
            }
        }
        serde_json::Value::String(s) => {
            if let Ok(d) = s.parse::<Decimal>() {
                let precision = s.split('.').nth(1).map(|p| p.len() as u32).unwrap_or(0);
                Ok(Value::Decimal(DecimalValue(d, precision)))
            } else {
                Ok(Value::String(s.clone()))
            }
        }
        serde_json::Value::Array(arr) => {
            let mut items = Vec::with_capacity(arr.len());
            for v in arr {
                items.push(json_to_value_ref(v)?);
            }
            Ok(Value::Array(Rc::new(RefCell::new(items))))
        }
        serde_json::Value::Object(obj) => {
            let mut map = HashPairs::with_capacity_and_hasher(obj.len(), AHasher::default());
            for (k, v) in obj {
                map.insert(HashKey::String(k.clone()), json_to_value_ref(v)?);
            }
            Ok(Value::Hash(Rc::new(RefCell::new(map))))
        }
    }
}

/// Convert a Soli Value to serde_json::Value.
pub fn value_to_json(value: &Value) -> Result<serde_json::Value, String> {
    match value {
        Value::Int(n) => Ok(serde_json::Value::Number(serde_json::Number::from(*n))),
        Value::Float(f) => Ok(serde_json::Value::Number(
            serde_json::Number::from_f64(*f).ok_or_else(|| "Invalid float".to_string())?,
        )),
        Value::Decimal(d) => Ok(serde_json::Value::String(d.to_string())),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Null => Ok(serde_json::Value::Null),
        Value::Array(arr) => {
            let borrow = arr.borrow();
            let mut vec = Vec::with_capacity(borrow.len());
            for v in borrow.iter() {
                vec.push(value_to_json(v)?);
            }
            Ok(serde_json::Value::Array(vec))
        }
        Value::Hash(hash) => {
            let borrow = hash.borrow();
            let mut map = serde_json::Map::with_capacity(borrow.len());
            for (k, v) in borrow.iter() {
                if let HashKey::String(key) = k {
                    map.insert(key.clone(), value_to_json(v)?);
                }
            }
            Ok(serde_json::Value::Object(map))
        }
        Value::Instance(inst) => {
            let borrow = inst.borrow();
            let mut map = serde_json::Map::with_capacity(borrow.fields.len());
            for (k, v) in borrow.fields.iter() {
                map.insert(k.clone(), value_to_json(v)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        _ => Err(format!("Cannot convert {} to JSON", value.type_name())),
    }
}
