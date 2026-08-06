use std::cell::RefCell;
use std::rc::Rc;

use ahash::RandomState as AHasher;

use crate::interpreter::value::{HashKey, HashPairs, Value};

/// Convert a serde_json::Value to a Soli Value (consuming — moves strings instead of cloning).
///
/// Strings stay strings. Numeric-looking text is **not** promoted to `Decimal`
/// (that used to run `parse::<Decimal>()` on every field and disagreed with
/// the hand-rolled `parse_json` path used by `JSON.parse`). Use an explicit
/// Decimal constructor or a model field type when money values are needed.
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
        serde_json::Value::String(s) => Ok(Value::String(s.into())),
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
                map.insert(HashKey::String(k.into()), json_to_value(v)?);
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
        serde_json::Value::String(s) => Ok(Value::String(s.clone().into())),
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
                map.insert(HashKey::String(k.clone().into()), json_to_value_ref(v)?);
            }
            Ok(Value::Hash(Rc::new(RefCell::new(map))))
        }
    }
}

/// Convert a Soli Value to serde_json::Value.
pub fn value_to_json(value: &Value) -> Result<serde_json::Value, String> {
    match value {
        // RFC 3339, matching `to_iso()`. This is the path `.to_json()` and
        // model persistence take, so a DateTime field round-trips as a
        // standard timestamp string rather than the `{}` it used to produce.
        Value::DateTime(ts) => Ok(serde_json::Value::String(Value::datetime_to_rfc3339(*ts))),
        Value::Int(n) => Ok(serde_json::Value::Number(serde_json::Number::from(*n))),
        Value::Float(f) => Ok(serde_json::Value::Number(
            serde_json::Number::from_f64(*f).ok_or_else(|| "Invalid float".to_string())?,
        )),
        Value::Decimal(d) => Ok(serde_json::Value::String(d.to_string())),
        // EcoString → owned String once (no clone-then-to_string).
        Value::String(s) => Ok(serde_json::Value::String(s.to_string())),
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
                    map.insert(key.to_string(), value_to_json(v)?);
                }
            }
            Ok(serde_json::Value::Object(map))
        }
        Value::Instance(inst) => {
            let borrow = inst.borrow();
            // Enum value → tag string (unit) or { "variant": tag, ...payload }
            // (payload). This is what gets stored in the DB; the model
            // `enum_field` DSL reconstructs it on read.
            if let Some(tag) = crate::interpreter::value::enum_variant_tag(&borrow) {
                let payload: Vec<(&crate::interpreter::value::SoliStr, &Value)> = borrow
                    .fields
                    .iter()
                    .filter(|(k, _)| k.as_str() != "__variant")
                    .collect();
                if payload.is_empty() {
                    return Ok(serde_json::Value::String(tag.to_string()));
                }
                let mut map = serde_json::Map::with_capacity(payload.len() + 1);
                map.insert(
                    "variant".to_string(),
                    serde_json::Value::String(tag.to_string()),
                );
                for (k, v) in payload {
                    map.insert(k.to_string(), value_to_json(v)?);
                }
                return Ok(serde_json::Value::Object(map));
            }
            // SEC-013: same filter as the `serde::Serialize` path —
            // `value_to_json(user)` must not leak `password_hash` /
            // `*_token` / framework-internal fields.
            let mut map = serde_json::Map::with_capacity(borrow.fields.len());
            for (k, v) in borrow.fields.iter() {
                if !crate::interpreter::value::is_safe_serialised_field(k) {
                    continue;
                }
                map.insert(k.to_string(), value_to_json(v)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        // A `grouped {}` deferred (e.g. an `@ivar` serialised into a JSON
        // response or template locals) resolves to its query result first.
        Value::Deferred(cell) => {
            let resolved = crate::interpreter::builtins::model::batch::force(cell)?;
            value_to_json(&resolved)
        }
        _ => Err(format!("Cannot convert {} to JSON", value.type_name())),
    }
}
