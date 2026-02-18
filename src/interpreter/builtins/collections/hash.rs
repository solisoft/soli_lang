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
        ..Default::default()
    };

    env.assign("Hash", Value::Class(Rc::new(hash_class)));
}
