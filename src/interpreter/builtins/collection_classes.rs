//! String, Array, and Hash built-in classes for SoliLang.
//!
//! These classes wrap the primitive Value types (String, Array, Hash)
//! and provide methods on them. When a literal like "hello", [1, 2, 3],
//! or {"a": 1} is created, the interpreter automatically wraps it in
//! the appropriate class instance.

use base64::{engine::general_purpose, Engine as _};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

/// Register the String, Array, and Hash classes.
pub fn register_collection_classes(env: &mut Environment) {
    register_string_class(env);
    register_array_class(env);
    register_hash_class(env);
    register_base64_class(env);
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
        constructor: None,
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
        constructor: None,
    };

    env.assign("String", Value::Class(Rc::new(string_class)));
}

fn register_array_class(env: &mut Environment) {
    let empty_class = Rc::new(Class {
        name: "Array".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
    });

    env.define("Array".to_string(), Value::Class(empty_class.clone()));

    let mut array_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    array_native_methods.insert(
        "to_string".to_string(),
        Rc::new(NativeFunction::new("Array.to_string", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.to_string() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let arr = arr.borrow();
                    let parts: Vec<String> = arr.iter().map(|v| format!("{}", v)).collect();
                    Ok(Value::String(format!("[{}]", parts.join(", "))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "length".to_string(),
        Rc::new(NativeFunction::new("Array.length", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.length() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => Ok(Value::Int(arr.borrow().len() as i64)),
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "push".to_string(),
        Rc::new(NativeFunction::new("Array.push", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst.clone(),
                _ => return Err("Array.push() called on non-Array".to_string()),
            };
            let value = args.get(1).cloned().unwrap_or(Value::Null);
            let value_opt = this.borrow().fields.get("__value").cloned();
            match value_opt {
                Some(Value::Array(arr)) => {
                    arr.borrow_mut().push(value);
                    Ok(Value::Null)
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "pop".to_string(),
        Rc::new(NativeFunction::new("Array.pop", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst.clone(),
                _ => return Err("Array.pop() called on non-Array".to_string()),
            };
            let value_opt = this.borrow().fields.get("__value").cloned();
            match value_opt {
                Some(Value::Array(arr)) => arr
                    .borrow_mut()
                    .pop()
                    .ok_or_else(|| "pop() on empty array".to_string()),
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "get".to_string(),
        Rc::new(NativeFunction::new("Array.get", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.get() called on non-Array".to_string()),
            };
            let idx = match args.get(1) {
                Some(Value::Int(i)) => *i,
                _ => return Err("Array.get() requires integer index".to_string()),
            };
            let value_opt = this.borrow().fields.get("__value").cloned();
            match value_opt {
                Some(Value::Array(arr)) => {
                    let arr = arr.borrow();
                    let idx_usize = if idx < 0 {
                        (arr.len() as i64 + idx) as usize
                    } else {
                        idx as usize
                    };
                    arr.get(idx_usize).cloned().ok_or_else(|| {
                        format!("Index {} out of bounds (length: {})", idx, arr.len())
                    })
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "clear".to_string(),
        Rc::new(NativeFunction::new("Array.clear", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst.clone(),
                _ => return Err("Array.clear() called on non-Array".to_string()),
            };
            let value_opt = this.borrow().fields.get("__value").cloned();
            match value_opt {
                Some(Value::Array(arr)) => {
                    arr.borrow_mut().clear();
                    Ok(Value::Null)
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    let mut array_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    array_static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("Array.new", Some(0), {
            let class_ref = empty_class.clone();
            move |_args| {
                let mut inst = Instance::new(class_ref.clone());
                inst.set(
                    "__value".to_string(),
                    Value::Array(Rc::new(RefCell::new(Vec::new()))),
                );
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            }
        })),
    );

    let array_class = Class {
        name: "Array".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: array_static_methods,
        native_methods: array_native_methods,
        constructor: None,
    };

    env.assign("Array", Value::Class(Rc::new(array_class)));
}

fn register_hash_class(env: &mut Environment) {
    let empty_class = Rc::new(Class {
        name: "Hash".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
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
                    for (k, v) in hash.iter() {
                        if key.hash_eq(k) {
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
                    let mut found = false;
                    for (k, v) in hash.iter_mut() {
                        if key.hash_eq(k) {
                            *v = value.clone();
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        hash.push((key, value.clone()));
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
                    for (k, _) in hash.iter() {
                        if key.hash_eq(k) {
                            return Ok(Value::Bool(true));
                        }
                    }
                    Ok(Value::Bool(false))
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
                    let keys: Vec<Value> = hash.iter().map(|(k, _)| k.clone()).collect();
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
                    let mut removed_value = Value::Null;
                    let mut hash = hash.borrow_mut();
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
                        let mut result: Vec<(Value, Value)> = hash1.borrow().clone();
                        for (k2, v2) in other.borrow().iter() {
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
                            Value::Array(Rc::new(RefCell::new(vec![k.clone(), v.clone()])))
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
                    Value::Hash(Rc::new(RefCell::new(Vec::new()))),
                );
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
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
        constructor: None,
    };

    env.assign("Hash", Value::Class(Rc::new(hash_class)));
}

pub fn wrap_string(value: String, env: &Environment) -> Value {
    if let Some(Value::Class(class)) = env.get("String") {
        let mut inst = Instance::new(class.clone());
        inst.set("__value".to_string(), Value::String(value));
        Value::Instance(Rc::new(RefCell::new(inst)))
    } else {
        Value::String(value)
    }
}

pub fn wrap_array(value: Vec<Value>, env: &Environment) -> Value {
    if let Some(Value::Class(class)) = env.get("Array") {
        let mut inst = Instance::new(class.clone());
        inst.set(
            "__value".to_string(),
            Value::Array(Rc::new(RefCell::new(value))),
        );
        Value::Instance(Rc::new(RefCell::new(inst)))
    } else {
        Value::Array(Rc::new(RefCell::new(value)))
    }
}

pub fn wrap_hash(value: Vec<(Value, Value)>, env: &Environment) -> Value {
    if let Some(Value::Class(class)) = env.get("Hash") {
        let mut inst = Instance::new(class.clone());
        inst.set(
            "__value".to_string(),
            Value::Hash(Rc::new(RefCell::new(value))),
        );
        Value::Instance(Rc::new(RefCell::new(inst)))
    } else {
        Value::Hash(Rc::new(RefCell::new(value)))
    }
}

fn register_base64_class(env: &mut Environment) {
    let base64_class = Rc::new(Class {
        name: "Base64".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
    });

    env.define("Base64".to_string(), Value::Class(base64_class.clone()));

    let mut base64_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    base64_static_methods.insert(
        "encode".to_string(),
        Rc::new(NativeFunction::new("Base64.encode", Some(1), |args| {
            let input = match args.first() {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => s,
                    _ => return Err("Base64.encode() requires string argument".to_string()),
                },
                _ => return Err("Base64.encode() requires string argument".to_string()),
            };
            let encoded = general_purpose::STANDARD.encode(input.as_bytes());
            Ok(Value::String(encoded))
        })),
    );

    base64_static_methods.insert(
        "decode".to_string(),
        Rc::new(NativeFunction::new("Base64.decode", Some(1), |args| {
            let input = match args.first() {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => s,
                    _ => return Err("Base64.decode() requires string argument".to_string()),
                },
                _ => return Err("Base64.decode() requires string argument".to_string()),
            };
            let decoded = general_purpose::STANDARD
                .decode(&input)
                .map_err(|e| format!("Invalid Base64: {}", e))?;
            let decoded_str = String::from_utf8(decoded)
                .map_err(|e| format!("Decoded bytes are not valid UTF-8: {}", e))?;
            Ok(Value::String(decoded_str))
        })),
    );

    let base64_class_final = Class {
        name: "Base64".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: base64_static_methods,
        native_methods: HashMap::new(),
        constructor: None,
    };

    env.assign("Base64", Value::Class(Rc::new(base64_class_final)));
}
