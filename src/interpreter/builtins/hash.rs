//! Hash operation built-in functions.
//! Core hash functions are kept for convenience. Most operations are available via the Hash class.

use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register hash operation built-in functions.
pub fn register_hash_builtins(env: &mut Environment) {
    // hash() - Create an empty hash (constructor kept for convenience)
    env.define(
        "hash".to_string(),
        Value::NativeFunction(NativeFunction::new("hash", Some(0), |_args| {
            Ok(Value::Hash(Rc::new(RefCell::new(IndexMap::new()))))
        })),
    );

    // has_key(hash, key) - Check if a hash contains a key
    env.define(
        "has_key".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "has_key",
            Some(2),
            |args| match &args[0] {
                Value::Hash(hash) => {
                    let key = crate::interpreter::value::HashKey::from_value(&args[1]).ok_or_else(
                        || {
                            format!(
                                "has_key() key must be string, int, or bool, got {}",
                                args[1].type_name()
                            )
                        },
                    )?;
                    Ok(Value::Bool(hash.borrow().contains_key(&key)))
                }
                _ => Err(format!(
                    "has_key() expects hash as first argument, got {}",
                    args[0].type_name()
                )),
            },
        )),
    );
}
