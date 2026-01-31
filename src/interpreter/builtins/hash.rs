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
}
