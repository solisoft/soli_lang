//! Array class operations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

pub fn register_array_class(env: &mut Environment) {
    let empty_class = Rc::new(Class {
        name: "Array".to_string(),
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

    array_native_methods.insert(
        "first".to_string(),
        Rc::new(NativeFunction::new("Array.first", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.first() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => Ok(arr.borrow().first().cloned().unwrap_or(Value::Null)),
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "last".to_string(),
        Rc::new(NativeFunction::new("Array.last", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.last() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => Ok(arr.borrow().last().cloned().unwrap_or(Value::Null)),
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "reverse".to_string(),
        Rc::new(NativeFunction::new("Array.reverse", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.reverse() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let mut result = arr.borrow().clone();
                    result.reverse();
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "uniq".to_string(),
        Rc::new(NativeFunction::new("Array.uniq", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.uniq() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let mut result = Vec::new();
                    for item in arr.borrow().iter() {
                        if !result.contains(item) {
                            result.push(item.clone());
                        }
                    }
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "compact".to_string(),
        Rc::new(NativeFunction::new("Array.compact", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.compact() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let result: Vec<Value> = arr
                        .borrow()
                        .iter()
                        .filter(|v| !matches!(v, Value::Null))
                        .cloned()
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "flatten".to_string(),
        Rc::new(NativeFunction::new("Array.flatten", None, |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.flatten() called on non-Array".to_string()),
            };
            let depth: Option<usize> = match args.get(1) {
                Some(Value::Int(n)) if *n >= 0 => Some(*n as usize),
                Some(Value::Int(_)) => {
                    return Err("flatten expects a non-negative integer".to_string())
                }
                None => None,
                _ => return Err("flatten expects an optional integer argument".to_string()),
            };

            fn flatten_recursive(
                arr: &[Value],
                current_depth: usize,
                max_depth: Option<usize>,
            ) -> Vec<Value> {
                if let Some(max) = max_depth {
                    if current_depth >= max {
                        return arr.to_vec();
                    }
                }
                let mut result = Vec::new();
                for item in arr {
                    if let Value::Array(inner) = item {
                        result.extend(flatten_recursive(
                            &inner.borrow(),
                            current_depth + 1,
                            max_depth,
                        ));
                    } else {
                        result.push(item.clone());
                    }
                }
                result
            }

            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let result = flatten_recursive(&arr.borrow(), 0, depth);
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "take".to_string(),
        Rc::new(NativeFunction::new("Array.take", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.take() called on non-Array".to_string()),
            };
            let n = match args.get(1) {
                Some(Value::Int(n)) if *n >= 0 => *n as usize,
                _ => return Err("take expects a non-negative integer".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let result: Vec<Value> = arr.borrow().iter().take(n).cloned().collect();
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "drop".to_string(),
        Rc::new(NativeFunction::new("Array.drop", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.drop() called on non-Array".to_string()),
            };
            let n = match args.get(1) {
                Some(Value::Int(n)) if *n >= 0 => *n as usize,
                _ => return Err("drop expects a non-negative integer".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let result: Vec<Value> = arr.borrow().iter().skip(n).cloned().collect();
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "zip".to_string(),
        Rc::new(NativeFunction::new("Array.zip", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.zip() called on non-Array".to_string()),
            };
            let other = match args.get(1) {
                Some(Value::Array(arr)) => arr.borrow().clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::Array(arr)) => arr.borrow().clone(),
                    _ => return Err("zip expects an array argument".to_string()),
                },
                _ => return Err("zip expects an array argument".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let result: Vec<Value> = arr
                        .borrow()
                        .iter()
                        .zip(other.iter())
                        .map(|(a, b)| {
                            Value::Array(Rc::new(RefCell::new(vec![a.clone(), b.clone()])))
                        })
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "sum".to_string(),
        Rc::new(NativeFunction::new("Array.sum", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.sum() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let mut total = 0.0;
                    for item in arr.borrow().iter() {
                        match item {
                            Value::Int(n) => total += *n as f64,
                            Value::Float(n) => total += *n,
                            _ => return Err("sum expects numeric array".to_string()),
                        }
                    }
                    Ok(Value::Float(total))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "min".to_string(),
        Rc::new(NativeFunction::new("Array.min", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.min() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let items = arr.borrow();
                    if items.is_empty() {
                        return Ok(Value::Null);
                    }
                    let mut min = &items[0];
                    for item in items.iter().skip(1) {
                        match (min, item) {
                            (Value::Int(a), Value::Int(b)) if b < a => min = item,
                            (Value::Float(a), Value::Float(b)) if b < a => min = item,
                            (Value::String(a), Value::String(b)) if b < a => min = item,
                            (Value::Int(a), Value::Float(b)) if *b < *a as f64 => min = item,
                            (Value::Float(a), Value::Int(b)) if (*b as f64) < *a => min = item,
                            _ => {}
                        }
                    }
                    Ok(min.clone())
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "max".to_string(),
        Rc::new(NativeFunction::new("Array.max", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.max() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let items = arr.borrow();
                    if items.is_empty() {
                        return Ok(Value::Null);
                    }
                    let mut max = &items[0];
                    for item in items.iter().skip(1) {
                        match (max, item) {
                            (Value::Int(a), Value::Int(b)) if b > a => max = item,
                            (Value::Float(a), Value::Float(b)) if b > a => max = item,
                            (Value::String(a), Value::String(b)) if b > a => max = item,
                            (Value::Int(a), Value::Float(b)) if *b > *a as f64 => max = item,
                            (Value::Float(a), Value::Int(b)) if (*b as f64) > *a => max = item,
                            _ => {}
                        }
                    }
                    Ok(max.clone())
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "sample".to_string(),
        Rc::new(NativeFunction::new("Array.sample", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.sample() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let items = arr.borrow();
                    if items.is_empty() {
                        return Ok(Value::Null);
                    }
                    use rand::seq::SliceRandom;
                    use rand::thread_rng;
                    let mut rng = thread_rng();
                    Ok(items.choose(&mut rng).cloned().unwrap_or(Value::Null))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "shuffle".to_string(),
        Rc::new(NativeFunction::new("Array.shuffle", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.shuffle() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    use rand::seq::SliceRandom;
                    use rand::thread_rng;
                    let mut result = arr.borrow().clone();
                    let mut rng = thread_rng();
                    result.shuffle(&mut rng);
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "empty?".to_string(),
        Rc::new(NativeFunction::new("Array.empty?", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.empty?() called on non-Array".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => Ok(Value::Bool(arr.borrow().is_empty())),
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "include?".to_string(),
        Rc::new(NativeFunction::new("Array.include?", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.include?() called on non-Array".to_string()),
            };
            let item = args.get(1).cloned().unwrap_or(Value::Null);
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => Ok(Value::Bool(arr.borrow().contains(&item))),
                _ => Err("Array missing internal value".to_string()),
            }
        })),
    );

    array_native_methods.insert(
        "join".to_string(),
        Rc::new(NativeFunction::new("Array.join", Some(1), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Array.join() called on non-Array".to_string()),
            };
            let delim = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Instance(inst)) => match inst.borrow().fields.get("__value").cloned() {
                    Some(Value::String(s)) => s,
                    _ => return Err("Array.join() requires string delimiter".to_string()),
                },
                _ => return Err("Array.join() requires string delimiter".to_string()),
            };
            match this.borrow().fields.get("__value").cloned() {
                Some(Value::Array(arr)) => {
                    let parts: Vec<String> =
                        arr.borrow().iter().map(|v| format!("{}", v)).collect();
                    Ok(Value::String(parts.join(&delim)))
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
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    };

    env.assign("Array", Value::Class(Rc::new(array_class)));
}
