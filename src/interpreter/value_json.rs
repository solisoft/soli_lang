use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use ahash::RandomState as AHasher;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::interpreter::value::{HashKey, HashPairs, Value};

/// Fixed-seed hasher for JSON object maps (avoids entropy cost per object).
#[inline(always)]
pub(crate) fn json_map_hasher() -> AHasher {
    AHasher::with_seeds(
        0x0123_4567_89AB_CDEF,
        0xFEDC_BA98_7654_3210,
        0x0F1E_2D3C_4B5A_6978,
        0x8877_6655_4433_2211,
    )
}

/// Deserialize JSON directly into a Soli [`Value`] via sonic-rs / any serde
/// deserializer — no intermediate `serde_json::Value` tree.
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Int(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        if v <= i64::MAX as u64 {
            Ok(Value::Int(v as i64))
        } else {
            Ok(Value::Float(v as f64))
        }
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Float(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.into()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v.into()))
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(8));
        while let Some(v) = seq.next_element()? {
            items.push(v);
        }
        Ok(Value::Array(Rc::new(RefCell::new(items))))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut pairs =
            HashPairs::with_capacity_and_hasher(map.size_hint().unwrap_or(8), json_map_hasher());
        while let Some((k, v)) = map.next_entry::<String, Value>()? {
            pairs.insert(HashKey::String(k.into()), v);
        }
        Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
    }
}

/// SIMD-accelerated parse via sonic-rs, building Soli Values in one pass
/// (no intermediate serde_json tree).
#[inline]
pub fn parse_json_sonic(s: &str) -> Result<Value, String> {
    sonic_rs::from_str(s).map_err(|e| e.to_string())
}

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
            // Fixed seeds — same rationale as the hand-rolled parser: these maps
            // are per-document and short-lived, not long-lived hash tables.
            let mut map = HashPairs::with_capacity_and_hasher(obj.len(), json_map_hasher());
            for (k, v) in obj {
                map.insert(HashKey::String(k.into()), json_to_value(v)?);
            }
            Ok(Value::Hash(Rc::new(RefCell::new(map))))
        }
    }
}

/// Convert a serde_json::Value reference to a Soli Value (clones strings).
/// Maximum nesting depth when converting between Values and JSON. Runtime
/// code can build arbitrarily deep structures (`loop { a = [a] }`); naive
/// recursion on such a value would overflow the native stack — which aborts
/// the process without unwinding.
const MAX_JSON_DEPTH: usize = 512;

pub fn json_to_value_ref(json: &serde_json::Value) -> Result<Value, String> {
    fn inner(json: &serde_json::Value, depth: usize) -> Result<Value, String> {
        if depth > MAX_JSON_DEPTH {
            return Err("JSON structure nested too deeply".to_string());
        }
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
                    items.push(inner(v, depth + 1)?);
                }
                Ok(Value::Array(Rc::new(RefCell::new(items))))
            }
            serde_json::Value::Object(obj) => {
                let mut map = HashPairs::with_capacity_and_hasher(obj.len(), json_map_hasher());
                for (k, v) in obj {
                    map.insert(HashKey::String(k.clone().into()), inner(v, depth + 1)?);
                }
                Ok(Value::Hash(Rc::new(RefCell::new(map))))
            }
        }
    }
    inner(json, 0)
}

/// Convert a Soli Value to serde_json::Value.
pub fn value_to_json(value: &Value) -> Result<serde_json::Value, String> {
    fn inner(value: &Value, depth: usize) -> Result<serde_json::Value, String> {
        if depth > MAX_JSON_DEPTH {
            return Err("structure nested too deeply to serialize".to_string());
        }
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
                    vec.push(inner(v, depth + 1)?);
                }
                Ok(serde_json::Value::Array(vec))
            }
            Value::Hash(hash) => {
                let borrow = hash.borrow();
                let mut map = serde_json::Map::with_capacity(borrow.len());
                for (k, v) in borrow.iter() {
                    if let HashKey::String(key) = k {
                        map.insert(key.to_string(), inner(v, depth + 1)?);
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
                        map.insert(k.to_string(), inner(v, depth + 1)?);
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
                    map.insert(k.to_string(), inner(v, depth + 1)?);
                }
                Ok(serde_json::Value::Object(map))
            }
            // A `grouped {}` deferred (e.g. an `@ivar` serialised into a JSON
            // response or template locals) resolves to its query result first.
            Value::Deferred(cell) => {
                let resolved = crate::interpreter::builtins::model::batch::force(cell)?;
                inner(&resolved, depth)
            }
            _ => Err(format!("Cannot convert {} to JSON", value.type_name())),
        }
    }
    inner(value, 0)
}
