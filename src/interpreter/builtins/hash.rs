//! Hash operation built-in functions.
//!
//! Provides functions for manipulating hash maps (dictionaries).

use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};

/// Register all hash operation built-in functions.
pub fn register_hash_builtins(env: &mut Environment) {
    // hash() - Create an empty hash
    env.define(
        "hash".to_string(),
        Value::NativeFunction(NativeFunction::new("hash", Some(0), |_args| {
            Ok(Value::Hash(Rc::new(RefCell::new(IndexMap::new()))))
        })),
    );

    // keys(hash) - Get all keys as array
    env.define(
        "keys".to_string(),
        Value::NativeFunction(NativeFunction::new("keys", Some(1), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    let keys: Vec<Value> =
                        hash.borrow().keys().map(|k| k.to_value()).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(keys))))
                }
                other => Err(format!("keys() expects hash, got {}", other.type_name())),
            }
        })),
    );

    // values(hash) - Get all values as array
    env.define(
        "values".to_string(),
        Value::NativeFunction(NativeFunction::new("values", Some(1), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    let values: Vec<Value> = hash.borrow().values().cloned().collect();
                    Ok(Value::Array(Rc::new(RefCell::new(values))))
                }
                other => Err(format!("values() expects hash, got {}", other.type_name())),
            }
        })),
    );

    // has_key(hash, key) - Check if key exists (O(1) with IndexMap)
    env.define(
        "has_key".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "has_key",
            Some(2),
            |args| match &args[0] {
                Value::Hash(hash) => {
                    let key = &args[1];
                    let hash_key = key.to_hash_key().ok_or_else(|| {
                        format!("{} cannot be used as a hash key", key.type_name())
                    })?;
                    let exists = hash.borrow().contains_key(&hash_key);
                    Ok(Value::Bool(exists))
                }
                other => Err(format!(
                    "has_key() expects hash as first argument, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // delete(hash, key) - Remove key and return its value (or null) - O(1)
    env.define(
        "delete".to_string(),
        Value::NativeFunction(NativeFunction::new("delete", Some(2), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    let key = &args[1];
                    let hash_key = key.to_hash_key().ok_or_else(|| {
                        format!("{} cannot be used as a hash key", key.type_name())
                    })?;
                    let removed_value = hash.borrow_mut().shift_remove(&hash_key);
                    Ok(removed_value.unwrap_or(Value::Null))
                }
                other => Err(format!(
                    "delete() expects hash as first argument, got {}",
                    other.type_name()
                )),
            }
        })),
    );

    // merge(hash1, hash2) - Merge two hashes (returns new hash, hash2 values win)
    env.define(
        "merge".to_string(),
        Value::NativeFunction(NativeFunction::new("merge", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Hash(hash1), Value::Hash(hash2)) => {
                    let mut result = hash1.borrow().clone();
                    for (k, v) in hash2.borrow().iter() {
                        result.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Hash(Rc::new(RefCell::new(result))))
                }
                _ => Err("merge() expects two hashes".to_string()),
            }
        })),
    );

    // entries(hash) / to_a(hash) - Get array of [key, value] pairs
    env.define(
        "entries".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "entries",
            Some(1),
            |args| match &args[0] {
                Value::Hash(hash) => {
                    let pairs: Vec<Value> = hash
                        .borrow()
                        .iter()
                        .map(|(k, v)| {
                            Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                        })
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(pairs))))
                }
                other => Err(format!("entries() expects hash, got {}", other.type_name())),
            },
        )),
    );

    // from_entries(array) - Create hash from array of [key, value] pairs (reverse of entries)
    env.define(
        "from_entries".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "from_entries",
            Some(1),
            |args| match &args[0] {
                Value::Array(arr) => {
                    let mut result: IndexMap<HashKey, Value> = IndexMap::new();

                    for entry in arr.borrow().iter() {
                        match entry {
                            Value::Array(pair) => {
                                let borrowed = pair.borrow();
                                if borrowed.len() != 2 {
                                    return Err(format!(
                                        "from_entries expects array of [key, value] pairs, got array with {} elements",
                                        borrowed.len()
                                    ));
                                }
                                let key = &borrowed[0];
                                let hash_key = key.to_hash_key().ok_or_else(|| {
                                    format!("{} cannot be used as a hash key", key.type_name())
                                })?;
                                result.insert(hash_key, borrowed[1].clone());
                            }
                            other => {
                                return Err(format!(
                                    "from_entries expects array of [key, value] pairs, got {}",
                                    other.type_name()
                                ));
                            }
                        }
                    }

                    Ok(Value::Hash(Rc::new(RefCell::new(result))))
                }
                other => Err(format!("from_entries() expects array, got {}", other.type_name())),
            },
        )),
    );

    // clear(hash) - Remove all entries from hash (mutates)
    env.define(
        "clear".to_string(),
        Value::NativeFunction(NativeFunction::new("clear", Some(1), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    hash.borrow_mut().clear();
                    Ok(Value::Null)
                }
                Value::Array(arr) => {
                    arr.borrow_mut().clear();
                    Ok(Value::Null)
                }
                other => Err(format!(
                    "clear() expects hash or array, got {}",
                    other.type_name()
                )),
            }
        })),
    );
}
