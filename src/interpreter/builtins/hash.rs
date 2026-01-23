//! Hash operation built-in functions.
//!
//! Provides functions for manipulating hash maps (dictionaries).

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register all hash operation built-in functions.
pub fn register_hash_builtins(env: &mut Environment) {
    // keys(hash) - Get all keys as array
    env.define(
        "keys".to_string(),
        Value::NativeFunction(NativeFunction::new("keys", Some(1), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    let keys: Vec<Value> = hash.borrow().iter().map(|(k, _)| k.clone()).collect();
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
                    let values: Vec<Value> = hash.borrow().iter().map(|(_, v)| v.clone()).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(values))))
                }
                other => Err(format!("values() expects hash, got {}", other.type_name())),
            }
        })),
    );

    // has_key(hash, key) - Check if key exists
    env.define(
        "has_key".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "has_key",
            Some(2),
            |args| match &args[0] {
                Value::Hash(hash) => {
                    let key = &args[1];
                    if !key.is_hashable() {
                        return Err(format!("{} cannot be used as a hash key", key.type_name()));
                    }
                    let exists = hash.borrow().iter().any(|(k, _)| key.hash_eq(k));
                    Ok(Value::Bool(exists))
                }
                other => Err(format!(
                    "has_key() expects hash as first argument, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // delete(hash, key) - Remove key and return its value (or null)
    env.define(
        "delete".to_string(),
        Value::NativeFunction(NativeFunction::new("delete", Some(2), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    let key = &args[1];
                    if !key.is_hashable() {
                        return Err(format!("{} cannot be used as a hash key", key.type_name()));
                    }
                    let mut hash = hash.borrow_mut();
                    let mut removed_value = Value::Null;
                    hash.retain(|(k, v)| {
                        if key.hash_eq(k) {
                            removed_value = v.clone();
                            false
                        } else {
                            true
                        }
                    });
                    Ok(removed_value)
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
                    let mut result: Vec<(Value, Value)> = hash1.borrow().clone();
                    for (k2, v2) in hash2.borrow().iter() {
                        let mut found = false;
                        for (k1, v1) in result.iter_mut() {
                            if k2.hash_eq(k1) {
                                *v1 = v2.clone();
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            result.push((k2.clone(), v2.clone()));
                        }
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
                            Value::Array(Rc::new(RefCell::new(vec![k.clone(), v.clone()])))
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
                    let mut result: Vec<(Value, Value)> = Vec::new();

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
                                if !key.is_hashable() {
                                    return Err(format!(
                                        "{} cannot be used as a hash key",
                                        key.type_name()
                                    ));
                                }
                                // Update existing key or add new one
                                let mut found = false;
                                for (k, v) in result.iter_mut() {
                                    if k.hash_eq(key) {
                                        *v = borrowed[1].clone();
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    result.push((key.clone(), borrowed[1].clone()));
                                }
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
