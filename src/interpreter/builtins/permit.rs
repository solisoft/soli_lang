//! `permit(params, shape)` — strong parameters for documents.
//!
//! SoliDB is schemaless, so unfiltered mass-assignment persists *anything*
//! a client posts — extra keys, nested garbage, `{"is_admin": true}`. With
//! nested params this matters twice over: `permit` whitelists exactly the
//! shape a controller expects and drops everything else.
//!
//! ```soli
//! permitted = permit(params, {
//!   "title": true,                         # scalar
//!   "tags": [],                            # array of scalars
//!   "author": {"name": true, "email": true},   # nested hash
//!   "items": [{"sku": true, "qty": true}]      # array of hashes
//! })
//! Post.create(permitted)
//! ```
//!
//! Rules: `true` keeps a scalar (string/number/bool/null — container values
//! are dropped so structure can't smuggle through a scalar slot); `[]` keeps
//! an array's scalar elements; `{…}` recurses; `[{…}]` filters each hash of
//! an array — and also accepts a numeric-keyed hash (`items[0][sku]` form
//! parsing) which it converts to an array of its values, Rails-style.
//! Unlisted keys are silently dropped; missing keys are simply absent.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};
use ahash::RandomState as AHasher;

fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null
    )
}

/// Apply one shape spec to one value. `None` drops the key.
fn filter_value(value: &Value, spec: &Value) -> Option<Value> {
    match spec {
        // `true` — scalar slot. A container here is an over-posting attempt.
        Value::Bool(true) => is_scalar(value).then(|| value.clone()),
        Value::Array(specs) => {
            let specs = specs.borrow();
            match specs.first() {
                // `[]` — array of scalars.
                None => {
                    let Value::Array(items) = value else {
                        return None;
                    };
                    let kept: Vec<Value> = items
                        .borrow()
                        .iter()
                        .filter(|item| is_scalar(item))
                        .cloned()
                        .collect();
                    Some(Value::Array(Rc::new(RefCell::new(kept))))
                }
                // `[{…}]` — array of hashes, each filtered by the shape.
                // A numeric-keyed hash (items[0][sku] parsing) converts to
                // an array of its values in key order, Rails-style.
                Some(element_spec @ Value::Hash(_)) => {
                    let elements: Vec<Value> = match value {
                        Value::Array(items) => items.borrow().clone(),
                        Value::Hash(map)
                            if map.borrow().keys().all(|k| {
                                matches!(k, HashKey::String(s) if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty())
                            }) =>
                        {
                            map.borrow().values().cloned().collect()
                        }
                        _ => return None,
                    };
                    let kept: Vec<Value> = elements
                        .iter()
                        .filter_map(|item| filter_value(item, element_spec))
                        .collect();
                    Some(Value::Array(Rc::new(RefCell::new(kept))))
                }
                Some(_) => None,
            }
        }
        // `{…}` — nested hash whitelist.
        Value::Hash(shape) => {
            let Value::Hash(source) = value else {
                return None;
            };
            let source = source.borrow();
            let shape = shape.borrow();
            let mut kept = HashPairs::with_capacity_and_hasher(shape.len(), AHasher::default());
            for (key, key_spec) in shape.iter() {
                if let Some(found) = source.get(key) {
                    if let Some(filtered) = filter_value(found, key_spec) {
                        kept.insert(key.clone(), filtered);
                    }
                }
            }
            Some(Value::Hash(Rc::new(RefCell::new(kept))))
        }
        _ => None,
    }
}

pub fn register_permit_builtins(env: &mut Environment) {
    // permit(params, shape) -> Hash — whitelist-filter a params hash.
    env.define(
        "permit".to_string(),
        Value::NativeFunction(NativeFunction::new("permit", Some(2), |args| {
            let source = match &args[0] {
                Value::Hash(_) => args[0].clone(),
                // A missing sub-hash (params["post"] on a bad request)
                // filters to an empty hash rather than erroring.
                Value::Null => return Ok(Value::Hash(Rc::new(RefCell::new(HashPairs::default())))),
                other => {
                    return Err(format!(
                        "permit() expects a params hash, got {}",
                        other.type_name()
                    ))
                }
            };
            let Value::Hash(_) = &args[1] else {
                return Err(format!(
                    "permit() expects a shape hash, got {}",
                    args[1].type_name()
                ));
            };
            Ok(filter_value(&source, &args[1])
                .unwrap_or_else(|| Value::Hash(Rc::new(RefCell::new(HashPairs::default())))))
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(pairs: Vec<(&str, Value)>) -> Value {
        let map: HashPairs = pairs
            .into_iter()
            .map(|(k, v)| (HashKey::String(k.into()), v))
            .collect();
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    fn array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    fn s(v: &str) -> Value {
        Value::String(v.into())
    }

    fn get(value: &Value, key: &str) -> Value {
        let Value::Hash(h) = value else {
            panic!("expected hash")
        };
        h.borrow()
            .get(&crate::interpreter::value::StrKey(key))
            .cloned()
            .unwrap_or(Value::Null)
    }

    #[test]
    fn scalars_kept_containers_dropped_unlisted_dropped() {
        let source = hash(vec![
            ("title", s("Hi")),
            ("is_admin", Value::Bool(true)),
            // Over-posting structure into a scalar slot is dropped.
            ("body", hash(vec![("sneaky", s("x"))])),
        ]);
        let shape = hash(vec![
            ("title", Value::Bool(true)),
            ("body", Value::Bool(true)),
        ]);
        let out = filter_value(&source, &shape).unwrap();
        assert_eq!(get(&out, "title"), s("Hi"));
        assert_eq!(get(&out, "is_admin"), Value::Null);
        assert_eq!(get(&out, "body"), Value::Null);
    }

    #[test]
    fn nested_shapes_and_scalar_arrays() {
        let source = hash(vec![
            (
                "author",
                hash(vec![("name", s("Ada")), ("role", s("admin"))]),
            ),
            (
                "tags",
                array(vec![s("a"), hash(vec![("evil", s("x"))]), s("b")]),
            ),
        ]);
        let shape = hash(vec![
            ("author", hash(vec![("name", Value::Bool(true))])),
            ("tags", array(vec![])),
        ]);
        let out = filter_value(&source, &shape).unwrap();
        assert_eq!(get(&get(&out, "author"), "name"), s("Ada"));
        assert_eq!(get(&get(&out, "author"), "role"), Value::Null);
        let Value::Array(tags) = get(&out, "tags") else {
            panic!("tags")
        };
        assert_eq!(tags.borrow().len(), 2);
    }

    #[test]
    fn array_of_hashes_and_numeric_keyed_conversion() {
        let shape = hash(vec![(
            "items",
            array(vec![hash(vec![("sku", Value::Bool(true))])]),
        )]);
        // Plain array of hashes.
        let source = hash(vec![(
            "items",
            array(vec![hash(vec![("sku", s("a")), ("price", s("1"))])]),
        )]);
        let out = filter_value(&source, &shape).unwrap();
        let Value::Array(items) = get(&out, "items") else {
            panic!("items")
        };
        assert_eq!(get(&items.borrow()[0], "sku"), s("a"));
        assert_eq!(get(&items.borrow()[0], "price"), Value::Null);

        // Numeric-keyed hash (items[0][sku] parsing) converts to an array.
        let source = hash(vec![(
            "items",
            hash(vec![
                ("0", hash(vec![("sku", s("a"))])),
                ("1", hash(vec![("sku", s("b"))])),
            ]),
        )]);
        let out = filter_value(&source, &shape).unwrap();
        let Value::Array(items) = get(&out, "items") else {
            panic!("items from numeric hash")
        };
        assert_eq!(items.borrow().len(), 2);
        assert_eq!(get(&items.borrow()[1], "sku"), s("b"));
    }
}
