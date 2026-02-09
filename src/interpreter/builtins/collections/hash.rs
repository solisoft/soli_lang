//! Hash/Map class operations.

use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, Instance, NativeFunction, Value};

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
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
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
        "get".to_string(),
        Rc::new(NativeFunction::new("Hash.get", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Hash.get() called on non-Hash".to_string()),
            };
            let key = match args.get(1) {
                Some(k) => k.clone(),
                None => return Err("Hash.get() requires a key".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Hash(hash)) => {
                    let hash = hash.borrow();
                    if let Some(hash_key) = HashKey::from_value(&key) {
                        if let Some(v) = hash.get(&hash_key) {
                            return Ok(v.clone());
                        }
                    }
                    Ok(Value::Null)
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "set".to_string(),
        Rc::new(NativeFunction::new("Hash.set", Some(2), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst.clone(),
                _ => return Err("Hash.set() called on non-Hash".to_string()),
            };
            let key = match args.get(1) {
                Some(k) => k.clone(),
                None => return Err("Hash.set() requires a key".to_string()),
            };
            let value = args.get(2).cloned().unwrap_or(Value::Null);
            let value_opt = this.borrow().fields.get("__value").cloned();
            match value_opt {
                Some(Value::Hash(hash)) => {
                    let mut hash = hash.borrow_mut();
                    if let Some(hash_key) = HashKey::from_value(&key) {
                        hash.insert(hash_key, value.clone());
                    } else {
                        return Err(
                            "Hash key must be a hashable value (int, string, bool, or null)"
                                .to_string(),
                        );
                    }
                    Ok(value)
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
                Some(k) => k.clone(),
                None => return Err("Hash.has_key() requires a key".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Hash(hash)) => {
                    let hash = hash.borrow();
                    if let Some(hash_key) = HashKey::from_value(&key) {
                        Ok(Value::Bool(hash.contains_key(&hash_key)))
                    } else {
                        Ok(Value::Bool(false))
                    }
                }
                _ => Err("Hash missing internal value".to_string()),
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
                    let hash = hash.borrow();
                    let keys: Vec<Value> = hash.keys().map(|k| k.to_value()).collect();
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
                    let hash = hash.borrow();
                    let values: Vec<Value> = hash.iter().map(|(_, v)| v.clone()).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(values))))
                }
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
                Some(k) => k.clone(),
                None => return Err("Hash.delete() requires a key".to_string()),
            };
            let value_opt = this.borrow().fields.get("__value").cloned();
            match value_opt {
                Some(Value::Hash(hash)) => {
                    let mut hash = hash.borrow_mut();
                    if let Some(hash_key) = HashKey::from_value(&key) {
                        let removed_value = hash.shift_remove(&hash_key).unwrap_or(Value::Null);
                        Ok(removed_value)
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Err("Hash missing internal value".to_string()),
            }
        })),
    );

    hash_native_methods.insert(
        "merge".to_string(),
        Rc::new(NativeFunction::new("Hash.merge", Some(1), {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Hash.merge() called on non-Hash".to_string()),
                };
                let other = match args.get(1) {
                    Some(Value::Hash(h)) => h.clone(),
                    Some(Value::Instance(inst)) => {
                        match inst.borrow().fields.get("__value").cloned() {
                            Some(Value::Hash(h)) => h,
                            _ => return Err("Hash.merge() requires hash argument".to_string()),
                        }
                    }
                    _ => return Err("Hash.merge() requires hash argument".to_string()),
                };
                match this.borrow().fields.get("__value").cloned() {
                    Some(Value::Hash(hash1)) => {
                        let mut result: IndexMap<HashKey, Value> = hash1.borrow().clone();
                        for (k2, v2) in other.borrow().iter() {
                            result.insert(k2.clone(), v2.clone());
                        }
                        let mut inst = Instance::new(class_ref.clone());
                        inst.set(
                            "__value".to_string(),
                            Value::Hash(Rc::new(RefCell::new(result))),
                        );
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("Hash missing internal value".to_string()),
                }
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
                    let hash = hash.borrow();
                    let pairs: Vec<Value> = hash
                        .iter()
                        .map(|(k, v)| {
                            Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                        })
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(pairs))))
                }
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

    let mut hash_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    hash_static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("Hash.new", Some(0), {
            let class_ref = empty_class.clone();
            move |_args| {
                let mut inst = Instance::new(class_ref.clone());
                inst.set(
                    "__value".to_string(),
                    Value::Hash(Rc::new(RefCell::new(IndexMap::new()))),
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
                let mut map = IndexMap::new();
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
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    };

    env.assign("Hash", Value::Class(Rc::new(hash_class)));
}
