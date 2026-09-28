//! Hash methods that answer from a borrow of the map: no callback, no
//! mutation of the receiver. Both engines dispatch here first — the
//! tree-walker before snapshotting the entries for its closure-taking
//! methods, the VM straight off its operand stack without copying the
//! arguments into a `Vec` — so the two cannot drift apart on what `slice`,
//! `except` or `dig` mean.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::{
    hash_contains_value, hash_get_value, HashKey, HashPairs, StrKey, SymKey, Value,
};
use crate::span::Span;

/// Dispatch `method_name` against a borrowed map. `None` means the method is
/// not one of the borrowed-tier methods and the caller should fall through to
/// its own dispatcher.
#[inline]
pub fn hash_method_borrowed(
    entries: &HashPairs,
    method_name: &str,
    arguments: &[Value],
    span: Span,
) -> Option<Result<Value, RuntimeError>> {
    match method_name {
        "get" => {
            if arguments.is_empty() || arguments.len() > 2 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let found = hash_get_value(entries, &arguments[0]).cloned();
            Some(Ok(match found {
                Some(v) => v,
                None => arguments.get(1).cloned().unwrap_or(Value::Null),
            }))
        }
        "fetch" => {
            if arguments.is_empty() || arguments.len() > 2 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            if let Some(value) = hash_get_value(entries, &arguments[0]) {
                Some(Ok(value.clone()))
            } else if let Some(default) = arguments.get(1) {
                Some(Ok(default.clone()))
            } else {
                Some(Err(RuntimeError::type_error(
                    format!("key not found: {}", arguments[0]),
                    span,
                )))
            }
        }
        // `size` matches the VM and the slow path — without it here,
        // `h.size()` paid a full entry-snapshot clone then fell through.
        "length" | "len" | "size" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            Some(Ok(Value::Int(entries.len() as i64)))
        }
        "keys" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            let mut keys = Vec::with_capacity(entries.len());
            for k in entries.keys() {
                keys.push(k.to_value());
            }
            Some(Ok(Value::Array(Rc::new(RefCell::new(keys)))))
        }
        "values" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            let mut values = Vec::with_capacity(entries.len());
            values.extend(entries.values().cloned());
            Some(Ok(Value::Array(Rc::new(RefCell::new(values)))))
        }
        "entries" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            let mut pairs = Vec::with_capacity(entries.len());
            for (k, v) in entries.iter() {
                pairs.push(Value::Array(Rc::new(RefCell::new(vec![
                    k.to_value(),
                    v.clone(),
                ]))));
            }
            Some(Ok(Value::Array(Rc::new(RefCell::new(pairs)))))
        }
        "merge" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            match &arguments[0] {
                Value::Hash(other) => {
                    let other = other.borrow();
                    let mut merged = entries.clone();
                    merged.reserve(other.len());
                    for (k, v) in other.iter() {
                        merged.insert(k.clone(), v.clone());
                    }
                    Some(Ok(Value::Hash(Rc::new(RefCell::new(merged)))))
                }
                _ => Some(Err(RuntimeError::type_error(
                    "merge expects a hash argument",
                    span,
                ))),
            }
        }
        "compact" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            // Clone + retain keeps the hash table; re-inserting non-nulls
            // rehashes every survivor (same approach as the VM).
            let mut compacted = entries.clone();
            compacted.retain(|_, v| !matches!(v, Value::Null));
            Some(Ok(Value::Hash(Rc::new(RefCell::new(compacted)))))
        }
        "invert" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            let mut inverted =
                HashPairs::with_capacity_and_hasher(entries.len(), ahash::RandomState::default());
            for (k, v) in entries.iter() {
                let new_key = match v.to_hash_key() {
                    Some(key) => key,
                    None => {
                        return Some(Err(RuntimeError::type_error(
                            format!("Cannot use {} as hash key", v.type_name()),
                            span,
                        )))
                    }
                };
                inverted.insert(new_key, k.to_value());
            }
            Some(Ok(Value::Hash(Rc::new(RefCell::new(inverted)))))
        }
        "has_key" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            Some(Ok(Value::Bool(hash_contains_value(entries, &arguments[0]))))
        }
        "empty?" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            Some(Ok(Value::Bool(entries.is_empty())))
        }
        "to_string" | "to_s" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            Some(Ok(Value::String(
                super::array_ops::hash_pairs_to_string(entries.iter(), entries.len()).into(),
            )))
        }
        "flatten" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            let pairs: Vec<Value> = entries
                .iter()
                .map(|(k, v)| Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()]))))
                .collect();
            Some(Ok(Value::Array(Rc::new(RefCell::new(pairs)))))
        }
        "values_at" => {
            if arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let mut values = Vec::with_capacity(arguments.len());
            for arg in arguments {
                let v = hash_get_value(entries, arg).cloned().unwrap_or(Value::Null);
                values.push(v);
            }
            Some(Ok(Value::Array(Rc::new(RefCell::new(values)))))
        }
        "key" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let needle = &arguments[0];
            for (k, v) in entries.iter() {
                if v == needle {
                    return Some(Ok(k.to_value()));
                }
            }
            Some(Ok(Value::Null))
        }
        "has_value?" | "value?" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let needle = &arguments[0];
            let found = entries.values().any(|v| v == needle);
            Some(Ok(Value::Bool(found)))
        }
        "to_h" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            // A clone copies the index table as-is; re-inserting rehashed
            // every key.
            Some(Ok(Value::Hash(Rc::new(RefCell::new(entries.clone())))))
        }
        "update" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            match &arguments[0] {
                Value::Hash(other) => {
                    let mut merged = entries.clone();
                    for (k, v) in other.borrow().iter() {
                        merged.insert(k.clone(), v.clone());
                    }
                    Some(Ok(Value::Hash(Rc::new(RefCell::new(merged)))))
                }
                _ => Some(Err(RuntimeError::type_error(
                    "update expects a hash argument",
                    span,
                ))),
            }
        }
        "assoc" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let value = hash_get_value(entries, &arguments[0]).cloned();
            match value {
                Some(v) => Some(Ok(Value::Array(Rc::new(RefCell::new(vec![
                    arguments[0].clone(),
                    v,
                ]))))),
                None => Some(Ok(Value::Null)),
            }
        }
        "rassoc" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let needle = &arguments[0];
            for (k, v) in entries.iter() {
                if v == needle {
                    return Some(Ok(Value::Array(Rc::new(RefCell::new(vec![
                        k.to_value(),
                        v.clone(),
                    ])))));
                }
            }
            Some(Ok(Value::Null))
        }
        "fetch_values" => {
            if arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let mut values = Vec::with_capacity(arguments.len());
            for arg in arguments {
                match hash_get_value(entries, arg) {
                    Some(v) => values.push(v.clone()),
                    None => {
                        return Some(Err(RuntimeError::type_error(
                            format!("key not found: {:?}", arg),
                            span,
                        )));
                    }
                }
            }
            Some(Ok(Value::Array(Rc::new(RefCell::new(values)))))
        }
        "to_json" => {
            if !arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
            }
            Some(
                match crate::interpreter::value_stringify::stringify_hash_map_to_string(entries) {
                    Ok(json) => Ok(Value::String(json.into())),
                    Err(e) => Err(RuntimeError::General { message: e, span }),
                },
            )
        }
        "is_a?" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let class_name = match &arguments[0] {
                Value::String(s) => s.as_ref(),
                _ => {
                    return Some(Err(RuntimeError::type_error(
                        "is_a? expects a string argument",
                        span,
                    )))
                }
            };
            Some(Ok(Value::Bool(
                class_name == "hash" || class_name == "object",
            )))
        }
        "slice" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let Value::Array(keys) = &arguments[0] else {
                return Some(Err(RuntimeError::type_error(
                    "slice expects an array of keys",
                    span,
                )));
            };
            let keys = keys.borrow();
            let mut result =
                HashPairs::with_capacity_and_hasher(keys.len(), ahash::RandomState::default());
            for key in keys.iter() {
                let Some(hash_key) = key.to_hash_key() else {
                    return Some(Err(RuntimeError::type_error(
                        format!("Cannot use {} as hash key", key.type_name()),
                        span,
                    )));
                };
                if let Some(v) = hash_get_value(entries, key) {
                    result.insert(hash_key, v.clone());
                }
            }
            Some(Ok(Value::Hash(Rc::new(RefCell::new(result)))))
        }
        "except" => {
            if arguments.len() != 1 {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            let Value::Array(keys) = &arguments[0] else {
                return Some(Err(RuntimeError::type_error(
                    "except expects an array of keys",
                    span,
                )));
            };
            // Probe the receiver once per excluded key for its position, then
            // clone the map (index table copied, nothing rehashed) and drop
            // those positions. Building a set of the excluded keys and
            // re-inserting every survivor hashed each key twice.
            let mut drop_at: Vec<usize> = keys
                .borrow()
                .iter()
                .filter_map(|k| hash_index_of(entries, k))
                .collect();
            let mut result = entries.clone();
            if !drop_at.is_empty() {
                drop_at.sort_unstable();
                let mut position = 0;
                result.retain(|_, _| {
                    let keep = drop_at.binary_search(&position).is_err();
                    position += 1;
                    keep
                });
            }
            Some(Ok(Value::Hash(Rc::new(RefCell::new(result)))))
        }
        "dig" => {
            if arguments.is_empty() {
                return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
            }
            // First level: look up directly in the borrowed map (no clone of
            // the whole hash). Subsequent levels descend into nested Hash/Array
            // values, which are independent Rc<RefCell<_>> and borrowed lazily.
            let mut current = hash_get_value(entries, &arguments[0]).cloned();
            for key in &arguments[1..] {
                current = match current.take() {
                    Some(Value::Hash(hash)) => hash_get_value(&hash.borrow(), key).cloned(),
                    Some(Value::Array(arr)) => {
                        if let Value::Int(idx) = key {
                            let arr_ref = arr.borrow();
                            let idx = if *idx < 0 {
                                arr_ref.len() as i64 + idx
                            } else {
                                *idx
                            };
                            usize::try_from(idx)
                                .ok()
                                .and_then(|i| arr_ref.get(i).cloned())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if current.is_none() {
                    return Some(Ok(Value::Null));
                }
            }
            Some(Ok(current.unwrap_or(Value::Null)))
        }
        _ => None,
    }
}

/// Position of `key` in `entries`, probing with a borrowed key so a string or
/// symbol lookup allocates nothing. `None` for a miss or an unhashable key.
#[inline]
fn hash_index_of(entries: &HashPairs, key: &Value) -> Option<usize> {
    match key {
        Value::String(s) => entries.get_index_of(&StrKey(s)),
        Value::Symbol(s) => entries.get_index_of(&SymKey(s)),
        Value::Int(n) => entries.get_index_of(&HashKey::Int(*n)),
        Value::Bool(b) => entries.get_index_of(&HashKey::Bool(*b)),
        Value::Null => entries.get_index_of(&HashKey::Null),
        Value::Decimal(d) => entries.get_index_of(&HashKey::Decimal(d.clone())),
        _ => None,
    }
}
