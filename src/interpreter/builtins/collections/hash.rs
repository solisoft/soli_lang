//! Hash/Map class operations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, Instance, NativeFunction, Value};

pub fn register_hash_class(env: &mut Environment) {
    let empty_class = Rc::new(Class {
        name: "Hash".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });

    env.define("Hash".to_string(), Value::Class(empty_class.clone()));

    let mut hash_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    hash_native_methods.insert(
        "to_string".to_string(),
        Rc::new(NativeFunction::new("Hash.to_string", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.to_string() called on non-Hash".to_string()),
            };
            let value_opt = this.borrow().fields.get("__value").cloned();
            match value_opt {
                Some(Value::Hash(hash)) => {
                    let hash = hash.borrow();
                    let parts: Vec<String> = hash
                        .iter()
                        .map(|(k, v)| format!("{} => {}", k, v))
                        .collect();
                    Ok(Value::String(format!("{{{}}}", parts.join(", "))))
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "length".to_string(),
        Rc::new(NativeFunction::new("Hash.length", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.length() called on non-Hash".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Hash(hash)) => Ok(Value::Int(hash.borrow().len() as i64)),
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "clear".to_string(),
        Rc::new(NativeFunction::new("Hash.clear", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst.clone(),
                _ => return Err("Hash.clear() called on non-Hash".to_string()),
            };
            let value_opt = this.borrow().fields.get("__value").cloned();
            match value_opt {
                Some(Value::Hash(hash)) => {
                    hash.borrow_mut().clear();
                    Ok(Value::Null)
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "dig".to_string(),
        Rc::new(NativeFunction::new("Hash.dig", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.dig() called on non-Hash".to_string()),
            };
            let keys = &args[1..];
            if keys.is_empty() {
                return Err("Hash.dig() requires at least one key".to_string());
            }
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Hash(hash)) => {
                    let mut current: Option<Value> = Some(Value::Hash(hash));
                    for key in keys {
                        current = match current.take() {
                            Some(Value::Hash(h)) => {
                                let h = h.borrow();
                                if let Some(hash_key) = HashKey::from_value(key) {
                                    h.get(&hash_key).cloned()
                                } else {
                                    None
                                }
                            }
                            Some(Value::Array(arr)) => {
                                if let Value::Int(idx) = key {
                                    let arr = arr.borrow();
                                    let idx = *idx as usize;
                                    if idx < arr.len() {
                                        Some(arr[idx].clone())
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        if current.is_none() {
                            return Ok(Value::Null);
                        }
                    }
                    Ok(current.unwrap_or(Value::Null))
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    let mut hash_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    hash_static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("Hash.new", Some(0), {
            let class_ref = empty_class.clone();
            move |_args| {
                let mut inst = Instance::new(class_ref.clone());
                inst.set(
                    "__value".to_string(),
                    Value::Hash(Rc::new(RefCell::new(HashPairs::default()))),
                );
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            }
        })),
    );

    hash_static_methods.insert(
        "from_entries".to_string(),
        Rc::new(NativeFunction::new("Hash.from_entries", Some(1), {
            move |args| {
                let arr = match args.first() {
                    Some(Value::Array(a)) => a.borrow().clone(),
                    _ => return Err("Hash.from_entries() expects an Array".to_string()),
                };
                let mut map = HashPairs::default();
                for (i, item) in arr.iter().enumerate() {
                    match item {
                        Value::Array(pair) => {
                            let pair = pair.borrow();
                            if pair.len() != 2 {
                                return Err(format!(
                                    "Hash.from_entries(): entry at index {} must have exactly 2 elements, got {}",
                                    i, pair.len()
                                ));
                            }
                            let key = HashKey::from_value(&pair[0]).ok_or_else(|| {
                                format!("Hash.from_entries(): unhashable key at index {}", i)
                            })?;
                            map.insert(key, pair[1].clone());
                        }
                        _ => return Err(format!(
                            "Hash.from_entries(): entry at index {} must be an Array [key, value]", i
                        )),
                    }
                }
                Ok(Value::Hash(Rc::new(RefCell::new(map))))
            }
        })),
    );

    hash_native_methods.insert(
        "keys".to_string(),
        Rc::new(NativeFunction::new("Hash.keys", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.keys() called on non-Hash".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Hash(hash)) => {
                    let keys: Vec<Value> = hash.borrow().keys().map(|k| k.to_value()).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(keys))))
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "values".to_string(),
        Rc::new(NativeFunction::new("Hash.values", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.values() called on non-Hash".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Hash(hash)) => {
                    let values: Vec<Value> = hash.borrow().values().cloned().collect();
                    Ok(Value::Array(Rc::new(RefCell::new(values))))
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "entries".to_string(),
        Rc::new(NativeFunction::new("Hash.entries", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.entries() called on non-Hash".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Hash(hash)) => {
                    let entries: Vec<Value> = hash
                        .borrow()
                        .iter()
                        .map(|(k, v)| {
                            Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                        })
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(entries))))
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "has_key".to_string(),
        Rc::new(NativeFunction::new("Hash.has_key", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.has_key() called on non-Hash".to_string()),
            };
            let key = match args.get(1) {
                Some(k) => HashKey::from_value(k).ok_or_else(|| {
                    format!(
                        "Hash.has_key(): key must be string, int, or bool, got {}",
                        k.type_name()
                    )
                })?,
                None => return Err("Hash.has_key() requires a key argument".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Hash(hash)) => Ok(Value::Bool(hash.borrow().contains_key(&key))),
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("Hash.delete", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst.clone(),
                _ => return Err("Hash.delete() called on non-Hash".to_string()),
            };
            let key = match args.get(1) {
                Some(k) => HashKey::from_value(k).ok_or_else(|| {
                    format!(
                        "Hash.delete(): key must be string, int, or bool, got {}",
                        k.type_name()
                    )
                })?,
                None => return Err("Hash.delete() requires a key argument".to_string()),
            };
            let hash_val = this.borrow().fields.get("__value").cloned();
            match hash_val {
                Some(Value::Hash(hash)) => {
                    let removed = hash.borrow_mut().shift_remove(&key);
                    Ok(removed.unwrap_or_else(|| Value::Null))
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "merge".to_string(),
        Rc::new(NativeFunction::new("Hash.merge", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.merge() called on non-Hash".to_string()),
            };
            let other = match args.get(1) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.merge() requires a Hash argument".to_string()),
            };
            let this_value = this.borrow().fields.get("__value").cloned();
            let other_value = other.borrow().fields.get("__value").cloned();
            match (this_value, other_value) {
                (Some(Value::Hash(this_hash)), Some(Value::Hash(other_hash))) => {
                    let mut new_hash = this_hash.borrow().clone();
                    for (k, v) in other_hash.borrow().iter() {
                        new_hash.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    let hash_class = Class {
        name: "Hash".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: hash_static_methods,
        native_methods: hash_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.assign("Hash", Value::Class(Rc::new(hash_class)));
}
