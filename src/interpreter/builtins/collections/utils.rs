//! Common utilities and wrapper functions for collections.
//! Also includes String class and Base64 class.

use base64::{engine::general_purpose, Engine as _};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashPairs, Instance, NativeFunction, Value};

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
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        primitive: Some(crate::interpreter::value::PrimType::String),
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
                        inst.set("__value".to_string(), Value::String(s));
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
                        inst.set("__value".to_string(), Value::String(s));
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
                        inst.set(
                            "__value".to_string(),
                            Value::String(s.trim().to_string().into()),
                        );
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
                Some(Value::String(s)) => Ok(Value::Bool(s.contains(&*substr))),
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
                Some(Value::String(s)) => Ok(Value::Bool(s.starts_with(&*prefix))),
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
                Some(Value::String(s)) => Ok(Value::Bool(s.ends_with(&*suffix))),
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
                            .split(&*delim)
                            .map(|p| {
                                let mut inst = Instance::new(class_ref.clone());
                                inst.set(
                                    "__value".to_string(),
                                    Value::String(p.to_string().into()),
                                );
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
                    if let Some(idx) = s.find(&*substr) {
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
                                    inst.set(
                                        "__value".to_string(),
                                        Value::String(String::new().into()),
                                    );
                                    return Ok(Value::Instance(Rc::new(RefCell::new(inst))));
                                }
                                let mut inst = Instance::new(class_ref.clone());
                                inst.set(
                                    "__value".to_string(),
                                    Value::String(s[start_usize..end_usize].to_string().into()),
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
                                    Value::String(s.replace(from.as_ref(), to.as_ref())),
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
                                Value::String((pad_char.to_string().repeat(padding) + &s).into()),
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
                                Value::String(
                                    format!("{}{}", s, pad_char.to_string().repeat(padding)).into(),
                                ),
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
                    let parts: Vec<String> = s.split(&*delim).map(|p| p.to_string()).collect();
                    Ok(Value::String(parts.join(&delim).into()))
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
                    Some(Value::Int(i)) => i.to_string().into(),
                    Some(Value::Float(f)) => f.to_string().into(),
                    Some(Value::Bool(b)) => b.to_string().into(),
                    Some(Value::Null) => "null".into(),
                    Some(other) => other.to_string().into(),
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
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: string_static_methods,
        native_methods: string_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        primitive: Some(crate::interpreter::value::PrimType::String),
        ..Default::default()
    };

    env.assign("String", Value::Class(Rc::new(string_class)));
}

/// Coerce a Soli value into bytes for the Base64 encoders: a String contributes
/// its UTF-8 bytes, an Array must hold ints in `0..=255`.
fn base64_input_bytes(value: &Value, method: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Int(n) if (*n >= 0 && *n <= 255) => Ok(*n as u8),
                Value::Int(n) => Err(format!("byte value {} out of range", n)),
                other => Err(format!("expected byte, got {}", other.type_name())),
            })
            .collect(),
        other => Err(format!(
            "{}() expects string or array, got {}",
            method,
            other.type_name()
        )),
    }
}

/// Turn decoded bytes back into a Soli value: a String when the bytes are valid
/// UTF-8, an Array of byte ints otherwise.
fn base64_output_value(bytes: Vec<u8>) -> Value {
    match String::from_utf8(bytes) {
        Ok(string) => Value::String(string.into()),
        Err(e) => Value::Array(Rc::new(RefCell::new(
            e.as_bytes().iter().map(|&b| Value::Int(b as i64)).collect(),
        ))),
    }
}

/// Register the Base64 class with encode/decode methods.
pub fn register_base64_class(env: &mut Environment) {
    let mut base64_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Base64.encode(data) - Encode bytes to base64
    base64_static_methods.insert(
        "encode".to_string(),
        Rc::new(NativeFunction::new("Base64.encode", Some(1), |args| {
            let data = base64_input_bytes(&args[0], "Base64.encode")?;

            Ok(Value::String(
                general_purpose::STANDARD.encode(&data).into(),
            ))
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
            match general_purpose::STANDARD.decode(&*data) {
                Ok(bytes) => Ok(base64_output_value(bytes)),
                Err(e) => Err(format!("Base64 decode error: {}", e)),
            }
        })),
    );

    // Base64.urlsafe_encode(data) - RFC 4648 §5 alphabet, never padded.
    //
    // JWS §2, PKCE (RFC 7636), JWK (RFC 7517) and JWK thumbprints (RFC 7638) all
    // mandate the unpadded URL-safe form, so there is deliberately no padding
    // knob: a padded value would be rejected by every consumer in that family.
    base64_static_methods.insert(
        "urlsafe_encode".to_string(),
        Rc::new(NativeFunction::new(
            "Base64.urlsafe_encode",
            Some(1),
            |args| {
                let data = base64_input_bytes(&args[0], "Base64.urlsafe_encode")?;

                Ok(Value::String(
                    general_purpose::URL_SAFE_NO_PAD.encode(&data).into(),
                ))
            },
        )),
    );

    // Base64.urlsafe_decode(data) - accepts padded or unpadded input, since
    // producers in the wild differ on whether they strip `=`.
    base64_static_methods.insert(
        "urlsafe_decode".to_string(),
        Rc::new(NativeFunction::new(
            "Base64.urlsafe_decode",
            Some(1),
            |args| {
                let data = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Base64.urlsafe_decode() expects string, got {}",
                            other.type_name()
                        ))
                    }
                };
                let trimmed = data.trim_end_matches('=');

                match general_purpose::URL_SAFE_NO_PAD.decode(trimmed) {
                    Ok(bytes) => Ok(base64_output_value(bytes)),
                    Err(e) => Err(format!("Base64 urlsafe decode error: {}", e)),
                }
            },
        )),
    );

    let base64_class = Class {
        name: "Base64".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
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
            inst.set("__value".to_string(), Value::String(value.into()));
            Value::Instance(Rc::new(RefCell::new(inst)))
        }
        _ => Value::String(value.into()),
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
pub fn wrap_hash(value: HashPairs, env: &Environment) -> Value {
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

#[cfg(test)]
mod base64_tests {
    use super::*;

    fn base64_fn(env: &Environment, method: &str) -> Rc<NativeFunction> {
        match env.get("Base64") {
            Some(Value::Class(c)) => c
                .native_static_methods
                .get(method)
                .unwrap_or_else(|| panic!("Base64.{method} is not registered"))
                .clone(),
            other => panic!("expected Base64 class, got {other:?}"),
        }
    }

    fn fresh_env() -> Environment {
        let mut env = Environment::new();
        register_base64_class(&mut env);
        env
    }

    fn call(method: &str, arg: Value) -> Value {
        let env = fresh_env();
        (base64_fn(&env, method).func)(vec![arg]).unwrap()
    }

    fn as_string(value: Value) -> String {
        match value {
            Value::String(s) => s.to_string(),
            other => panic!("expected string, got {other:?}"),
        }
    }

    /// JWS §2, PKCE, JWK and RFC 7638 all mandate the unpadded URL-safe
    /// alphabet — any of `+`, `/` or `=` leaking out breaks every consumer.
    #[test]
    fn urlsafe_encode_avoids_standard_alphabet_and_padding() {
        // These bytes encode to "+/" under the standard alphabet.
        let bytes = Value::Array(Rc::new(RefCell::new(vec![
            Value::Int(0xfb),
            Value::Int(0xff),
            Value::Int(0xbf),
        ])));
        let encoded = as_string(call("urlsafe_encode", bytes));
        assert!(
            !encoded.contains(['+', '/', '=']),
            "url-safe output must not contain +, / or =: {encoded}"
        );
        assert_eq!(encoded, "-_-_");
    }

    /// Known vector: base64url(sha256("abc")), the shape PKCE S256 produces.
    #[test]
    fn urlsafe_encode_matches_known_pkce_vector() {
        let digest = Value::Array(Rc::new(RefCell::new(
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
            .iter()
            .map(|&b| Value::Int(b as i64))
            .collect::<Vec<_>>(),
        )));
        assert_eq!(
            as_string(call("urlsafe_encode", digest)),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
    }

    #[test]
    fn urlsafe_round_trips_text() {
        let encoded = call("urlsafe_encode", Value::String("hello/world?+=".into()));
        assert_eq!(as_string(call("urlsafe_decode", encoded)), "hello/world?+=");
    }

    /// Producers disagree on whether to strip `=`, so the decoder accepts both.
    #[test]
    fn urlsafe_decode_tolerates_padding() {
        assert_eq!(
            as_string(call("urlsafe_decode", Value::String("YWJjZA==".into()))),
            "abcd"
        );
        assert_eq!(
            as_string(call("urlsafe_decode", Value::String("YWJjZA".into()))),
            "abcd"
        );
    }

    /// Non-UTF-8 payloads come back as a byte array rather than erroring —
    /// the shape callers get for digests and key material.
    #[test]
    fn urlsafe_decode_returns_bytes_for_non_utf8() {
        let encoded = call(
            "urlsafe_encode",
            Value::Array(Rc::new(RefCell::new(vec![
                Value::Int(0xff),
                Value::Int(0xfe),
            ]))),
        );
        match call("urlsafe_decode", encoded) {
            Value::Array(arr) => {
                assert_eq!(*arr.borrow(), vec![Value::Int(0xff), Value::Int(0xfe)]);
            }
            other => panic!("expected byte array, got {other:?}"),
        }
    }

    #[test]
    fn encode_rejects_out_of_range_bytes() {
        let env = fresh_env();
        let err = (base64_fn(&env, "urlsafe_encode").func)(vec![Value::Array(Rc::new(
            RefCell::new(vec![Value::Int(256)]),
        ))])
        .expect_err("byte values above 255 must error");
        assert!(err.contains("out of range"), "{}", err);
    }
}
