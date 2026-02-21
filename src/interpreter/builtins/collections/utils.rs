//! Common utilities and wrapper functions for collections.
//! Also includes String class and Base64 class.

use base64::{engine::general_purpose, Engine as _};
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, Instance, NativeFunction, Value};

use super::array::register_array_class;
use super::hash::register_hash_class;
use super::range::register_range_class;
use super::set::register_set_class;

/// Register all collection classes including String, Array, Hash, Set, Range, and Base64.
pub fn register_collection_classes(env: &mut Environment) {
    register_string_class(env);
    register_array_class(env);
    register_hash_class(env);
    register_base64_class(env);
    register_set_class(env);
    register_range_class(env);
}

/// Register the String class with methods.
fn register_string_class(env: &mut Environment) {
    let empty_class = Rc::new(Class {
        name: "String".to_string(),
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

    env.define("String".to_string(), Value::Class(empty_class.clone()));

    let mut string_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    string_native_methods.insert(
        "to_string".to_string(),
        Rc::new(NativeFunction::new("String.to_string", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("String.to_string() called on non-String".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::String(s)) => Ok(Value::String(s)),
                _ => Err("String missing internal value".to_string()),
            }
        })),
    );

    string_native_methods.insert(
        "length".to_string(),
        Rc::new(NativeFunction::new("String.length", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("String.length() called on non-String".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::String(s)) => Ok(Value::Int(s.len() as i64)),
                _ => Err("String missing internal value".to_string()),
            }
        })),
    );

    string_native_methods.insert(
        "len".to_string(),
        Rc::new(NativeFunction::new("String.len", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("String.len() called on non-String".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::String(s)) => Ok(Value::Int(s.len() as i64)),
                _ => Err("String missing internal value".to_string()),
            }
        })),
    );

    string_native_methods.insert(
        "upcase".to_string(),
        Rc::new(NativeFunction::new("String.upcase", Some(0), {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("String.upcase() called on non-String".to_string()),
                };
                match this.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => {
                        let mut inst = Instance::new(class_ref.clone());
                        inst.set("__value".to_string(), Value::String(s.to_uppercase()));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("String missing internal value".to_string()),
                }
            }
        })),
    );

    string_native_methods.insert(
        "downcase".to_string(),
        Rc::new(NativeFunction::new("String.downcase", Some(0), {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("String.downcase() called on non-String".to_string()),
                };
                match this.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => {
                        let mut inst = Instance::new(class_ref.clone());
                        inst.set("__value".to_string(), Value::String(s.to_lowercase()));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("String missing internal value".to_string()),
                }
            }
        })),
    );

    string_native_methods.insert(
        "trim".to_string(),
        Rc::new(NativeFunction::new("String.trim", Some(0), {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("String.trim() called on non-String".to_string()),
                };
                match this.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => {
                        let mut inst = Instance::new(class_ref.clone());
                        inst.set("__value".to_string(), Value::String(s.trim().to_string()));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("String missing internal value".to_string()),
                }
            }
        })),
    );

    string_native_methods.insert(
        "contains".to_string(),
        Rc::new(NativeFunction::new("String.contains", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("String.contains() called on non-String".to_string()),
            };
            let substr = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => s,
                    _ => return Err("String.contains() requires string argument".to_string()),
                },
                _ => return Err("String.contains() requires string argument".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::String(s)) => Ok(Value::Bool(s.contains(&substr))),
                _ => Err("String missing internal value".to_string()),
            }
        })),
    );

    string_native_methods.insert(
        "starts_with".to_string(),
        Rc::new(NativeFunction::new("String.starts_with", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("String.starts_with() called on non-String".to_string()),
            };
            let prefix = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => s,
                    _ => return Err("String.starts_with() requires string argument".to_string()),
                },
                _ => return Err("String.starts_with() requires string argument".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::String(s)) => Ok(Value::Bool(s.starts_with(&prefix))),
                _ => Err("String missing internal value".to_string()),
            }
        })),
    );

    string_native_methods.insert(
        "ends_with".to_string(),
        Rc::new(NativeFunction::new("String.ends_with", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("String.ends_with() called on non-String".to_string()),
            };
            let suffix = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => s,
                    _ => return Err("String.ends_with() requires string argument".to_string()),
                },
                _ => return Err("String.ends_with() requires string argument".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::String(s)) => Ok(Value::Bool(s.ends_with(&suffix))),
                _ => Err("String missing internal value".to_string()),
            }
        })),
    );

    string_native_methods.insert(
        "split".to_string(),
        Rc::new(NativeFunction::new("String.split", Some(1), {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("String.split() called on non-String".to_string()),
                };
                let delim = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Instance(inst)) => {
                        match inst.borrow().fields.get("__value").cloned() {
                            Some(Value::String(s)) => s,
                            _ => return Err("String.split() requires string delimiter".to_string()),
                        }
                    }
                    _ => return Err("String.split() requires string delimiter".to_string()),
                };
                match this.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => {
                        let parts: Vec<Value> = s
                            .split(&delim)
                            .map(|p| {
                                let mut inst = Instance::new(class_ref.clone());
                                inst.set("__value".to_string(), Value::String(p.to_string()));
                                Value::Instance(Rc::new(RefCell::new(inst)))
                            })
                            .collect();
                        Ok(Value::Array(Rc::new(RefCell::new(parts))))
                    }
                    _ => Err("String missing internal value".to_string()),
                }
            }
        })),
    );

    string_native_methods.insert(
        "index_of".to_string(),
        Rc::new(NativeFunction::new("String.index_of", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("String.index_of() called on non-String".to_string()),
            };
            let substr = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => s,
                    _ => return Err("String.index_of() requires string argument".to_string()),
                },
                _ => return Err("String.index_of() requires string argument".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::String(s)) => {
                    if let Some(idx) = s.find(&substr) {
                        Ok(Value::Int(idx as i64))
                    } else {
                        Ok(Value::Int(-1))
                    }
                }
                _ => Err("String missing internal value".to_string()),
            }
        })),
    );

    string_native_methods.insert(
        "substring".to_string(),
        Rc::new(NativeFunction::new("String.substring", Some(2), {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("String.substring() called on non-String".to_string()),
                };
                match (&args[1], &args[2]) {
                    (Value::Int(start), Value::Int(end)) => {
                        match this.borrow().fields.get("__value").cloned() {
                            Some(Value::String(s)) => {
                                let start_usize = if *start < 0 { 0 } else { *start as usize };
                                let end_usize = if *end > s.len() as i64 {
                                    s.len() as i64
                                } else {
                                    *end
                                } as usize;
                                if start_usize >= end_usize || start_usize >= s.len() {
                                    let mut inst = Instance::new(class_ref.clone());
                                    inst.set("__value".to_string(), Value::String(String::new()));
                                    return Ok(Value::Instance(Rc::new(RefCell::new(inst))));
                                }
                                let mut inst = Instance::new(class_ref.clone());
                                inst.set(
                                    "__value".to_string(),
                                    Value::String(s[start_usize..end_usize].to_string()),
                                );
                                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                            }
                            _ => Err("String missing internal value".to_string()),
                        }
                    }
                    _ => Err("String.substring() requires (int, int)".to_string()),
                }
            }
        })),
    );

    string_native_methods.insert(
        "replace".to_string(),
        Rc::new(NativeFunction::new("String.replace", Some(2), {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("String.replace() called on non-String".to_string()),
                };
                match (&args[1], &args[2]) {
                    (Value::String(from), Value::String(to)) => {
                        match this.borrow().fields.get("__value").cloned() {
                            Some(Value::String(s)) => {
                                let mut inst = Instance::new(class_ref.clone());
                                inst.set(
                                    "__value".to_string(),
                                    Value::String(s.replace(from.as_str(), to.as_str())),
                                );
                                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                            }
                            _ => Err("String missing internal value".to_string()),
                        }
                    }
                    _ => Err("String.replace() requires (string, string)".to_string()),
                }
            }
        })),
    );

    string_native_methods.insert(
        "lpad".to_string(),
        Rc::new(NativeFunction::new("String.lpad", None, {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("String.lpad() called on non-String".to_string()),
                };
                let (width, pad_char) = match args.len() {
                    2 => match &args[1] {
                        Value::Int(width) => (*width as usize, ' '),
                        _ => return Err("String.lpad() requires int width".to_string()),
                    },
                    3 => match (&args[1], &args[2]) {
                        (Value::Int(width), Value::String(pad_str)) => {
                            (*width as usize, pad_str.chars().next().unwrap_or(' '))
                        }
                        _ => {
                            return Err("String.lpad() requires (int) or (int, string)".to_string())
                        }
                    },
                    _ => return Err("String.lpad() requires (int) or (int, string)".to_string()),
                };
                match this.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => {
                        let mut inst = Instance::new(class_ref.clone());
                        if s.len() >= width {
                            inst.set("__value".to_string(), Value::String(s));
                        } else {
                            let padding = width - s.len();
                            inst.set(
                                "__value".to_string(),
                                Value::String(pad_char.to_string().repeat(padding) + &s),
                            );
                        }
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("String missing internal value".to_string()),
                }
            }
        })),
    );

    string_native_methods.insert(
        "rpad".to_string(),
        Rc::new(NativeFunction::new("String.rpad", None, {
            let class_ref = empty_class.clone();
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("String.rpad() called on non-String".to_string()),
                };
                let (width, pad_char) = match args.len() {
                    2 => match &args[1] {
                        Value::Int(width) => (*width as usize, ' '),
                        _ => return Err("String.rpad() requires int width".to_string()),
                    },
                    3 => match (&args[1], &args[2]) {
                        (Value::Int(width), Value::String(pad_str)) => {
                            (*width as usize, pad_str.chars().next().unwrap_or(' '))
                        }
                        _ => {
                            return Err("String.rpad() requires (int) or (int, string)".to_string())
                        }
                    },
                    _ => return Err("String.rpad() requires (int) or (int, string)".to_string()),
                };
                match this.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => {
                        let mut inst = Instance::new(class_ref.clone());
                        if s.len() >= width {
                            inst.set("__value".to_string(), Value::String(s));
                        } else {
                            let padding = width - s.len();
                            inst.set(
                                "__value".to_string(),
                                Value::String(s + &pad_char.to_string().repeat(padding)),
                            );
                        }
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("String missing internal value".to_string()),
                }
            }
        })),
    );

    string_native_methods.insert(
        "join".to_string(),
        Rc::new(NativeFunction::new("String.join", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("String.join() called on non-String".to_string()),
            };
            let delim = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => s,
                    _ => return Err("String.join() requires string argument".to_string()),
                },
                _ => return Err("String.join() requires string argument".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::String(s)) => {
                    let parts: Vec<String> = s.split(&delim).map(|p| p.to_string()).collect();
                    Ok(Value::String(parts.join(&delim)))
                }
                _ => Err("String missing internal value".to_string()),
            }
        })),
    );

    let mut string_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    string_static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("String.new", Some(1), {
            let class_ref = empty_class.clone();
            move |args| {
                let value = match args.first() {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Int(i)) => i.to_string(),
                    Some(Value::Float(f)) => f.to_string(),
                    Some(Value::Bool(b)) => b.to_string(),
                    Some(Value::Null) => "null".to_string(),
                    Some(other) => other.to_string(),
                    None => return Err("String.new() requires an argument".to_string()),
                };
                let mut inst = Instance::new(class_ref.clone());
                inst.set("__value".to_string(), Value::String(value));
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            }
        })),
    );

    let string_class = Class {
        name: "String".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: string_static_methods,
        native_methods: string_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.assign("String", Value::Class(Rc::new(string_class)));
}

/// Register the Base64 class with encode/decode methods.
pub fn register_base64_class(env: &mut Environment) {
    let mut base64_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Base64.encode(data) - Encode bytes to base64
    base64_static_methods.insert(
        "encode".to_string(),
        Rc::new(NativeFunction::new("Base64.encode", Some(1), |args| {
            let data = match &args[0] {
                Value::String(s) => s.as_bytes().to_vec(),
                Value::Array(arr) => {
                    let bytes: Result<Vec<u8>, String> = arr
                        .borrow()
                        .iter()
                        .map(|v| match v {
                            Value::Int(n) if (*n >= 0 && *n <= 255) => Ok(*n as u8),
                            Value::Int(n) => Err(format!("byte value {} out of range", n)),
                            other => Err(format!("expected byte, got {}", other.type_name())),
                        })
                        .collect();
                    bytes?
                }
                other => {
                    return Err(format!(
                        "Base64.encode() expects string or array, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(general_purpose::STANDARD.encode(&data)))
        })),
    );

    // Base64.decode(data) - Decode base64 to bytes
    base64_static_methods.insert(
        "decode".to_string(),
        Rc::new(NativeFunction::new("Base64.decode", Some(1), |args| {
            let data = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Base64.decode() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            match general_purpose::STANDARD.decode(&data) {
                Ok(bytes) => {
                    match String::from_utf8(bytes.clone()) {
                        Ok(string) => Ok(Value::String(string)),
                        Err(_) => {
                            // If not valid UTF-8, return as byte array
                            let values: Vec<Value> =
                                bytes.iter().map(|&b| Value::Int(b as i64)).collect();
                            Ok(Value::Array(Rc::new(RefCell::new(values))))
                        }
                    }
                }
                Err(e) => Err(format!("Base64 decode error: {}", e)),
            }
        })),
    );

    let base64_class = Class {
        name: "Base64".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: base64_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("Base64".to_string(), Value::Class(Rc::new(base64_class)));
}

/// Wrap a string value in a String class instance.
pub fn wrap_string(value: String, env: &Environment) -> Value {
    match env.get("String") {
        Some(Value::Class(class)) => {
            let mut inst = Instance::new(class.clone());
            inst.set("__value".to_string(), Value::String(value));
            Value::Instance(Rc::new(RefCell::new(inst)))
        }
        _ => Value::String(value),
    }
}

/// Wrap an array value in an Array class instance.
pub fn wrap_array(value: Vec<Value>, env: &Environment) -> Value {
    match env.get("Array") {
        Some(Value::Class(class)) => {
            let mut inst = Instance::new(class.clone());
            inst.set(
                "__value".to_string(),
                Value::Array(Rc::new(RefCell::new(value))),
            );
            Value::Instance(Rc::new(RefCell::new(inst)))
        }
        _ => Value::Array(Rc::new(RefCell::new(value))),
    }
}

/// Wrap a hash value in a Hash class instance.
pub fn wrap_hash(value: IndexMap<HashKey, Value>, env: &Environment) -> Value {
    match env.get("Hash") {
        Some(Value::Class(class)) => {
            let mut inst = Instance::new(class.clone());
            inst.set(
                "__value".to_string(),
                Value::Hash(Rc::new(RefCell::new(value))),
            );
            Value::Instance(Rc::new(RefCell::new(inst)))
        }
        _ => Value::Hash(Rc::new(RefCell::new(value))),
    }
}
