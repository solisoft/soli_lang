//! Expectation class for chainable test assertions.
//! Supports: expect(value).to_be(), .to_equal(), .to_be_greater_than(), etc.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;

fn get_actual(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("Missing self argument".to_string());
    }
    let this = &args[0];
    if let Value::Hash(hash) = this {
        let borrowed = hash.borrow();
        if let Some(actual) = borrowed.get(&HashKey::String("actual".to_string())) {
            return Ok(actual.clone());
        }
    }
    Err("expect() must be called first".to_string())
}

pub fn register_expectation_class(env: &mut Environment) {
    let mut expectation_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    expectation_native_methods.insert(
        "to_be".to_string(),
        Rc::new(NativeFunction::new("Expectation.to_be", Some(1), |args| {
            let actual = get_actual(&args)?;
            let expected = &args[1];
            if actual == *expected {
                crate::interpreter::builtins::assertions::increment_assertion_count();
                Ok(Value::Bool(true))
            } else {
                Err(format!("Expected {:?} to be {:?}", actual, expected))
            }
        })),
    );

    expectation_native_methods.insert(
        "to_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_equal",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                if actual == *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to equal {:?}", actual, expected))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_not_be".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_not_be",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                if actual != *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to not be {:?}", actual, expected))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_not_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_not_equal",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                if actual != *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to not equal {:?}", actual, expected))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_null".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_null",
            Some(0),
            |args| {
                let actual = get_actual(&args)?;
                if matches!(actual, Value::Null) {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to be null", actual))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_not_be_null".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_not_be_null",
            Some(0),
            |args| {
                let actual = get_actual(&args)?;
                if !matches!(actual, Value::Null) {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err("Expected value to not be null".to_string())
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_greater_than".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_greater_than",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                let (actual_num, expected_num) = match (actual, expected) {
                    (Value::Int(a), Value::Int(b)) => (a as f64, *b as f64),
                    (Value::Int(a), Value::Float(b)) => (a as f64, *b),
                    (Value::Float(a), Value::Int(b)) => (a, *b as f64),
                    (Value::Float(a), Value::Float(b)) => (a, *b),
                    _ => return Err("to_be_greater_than expects numbers".to_string()),
                };
                if actual_num > expected_num {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!(
                        "Expected {} to be greater than {}",
                        actual_num, expected_num
                    ))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_less_than".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_less_than",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                let (actual_num, expected_num) = match (actual, expected) {
                    (Value::Int(a), Value::Int(b)) => (a as f64, *b as f64),
                    (Value::Int(a), Value::Float(b)) => (a as f64, *b),
                    (Value::Float(a), Value::Int(b)) => (a, *b as f64),
                    (Value::Float(a), Value::Float(b)) => (a, *b),
                    _ => return Err("to_be_less_than expects numbers".to_string()),
                };
                if actual_num < expected_num {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!(
                        "Expected {} to be less than {}",
                        actual_num, expected_num
                    ))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_greater_than_or_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_greater_than_or_equal",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                let (actual_num, expected_num) = match (actual, expected) {
                    (Value::Int(a), Value::Int(b)) => (a as f64, *b as f64),
                    (Value::Int(a), Value::Float(b)) => (a as f64, *b),
                    (Value::Float(a), Value::Int(b)) => (a, *b as f64),
                    (Value::Float(a), Value::Float(b)) => (a, *b),
                    _ => return Err("to_be_greater_than_or_equal expects numbers".to_string()),
                };
                if actual_num >= expected_num {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!(
                        "Expected {} to be greater than or equal to {}",
                        actual_num, expected_num
                    ))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_less_than_or_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_less_than_or_equal",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                let (actual_num, expected_num) = match (actual, expected) {
                    (Value::Int(a), Value::Int(b)) => (a as f64, *b as f64),
                    (Value::Int(a), Value::Float(b)) => (a as f64, *b),
                    (Value::Float(a), Value::Int(b)) => (a, *b as f64),
                    (Value::Float(a), Value::Float(b)) => (a, *b),
                    _ => return Err("to_be_less_than_or_equal expects numbers".to_string()),
                };
                if actual_num <= expected_num {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!(
                        "Expected {} to be less than or equal to {}",
                        actual_num, expected_num
                    ))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_contain".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_contain",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let item = &args[1];
                let contains = match (&actual, item) {
                    (Value::Array(arr), item) => arr.borrow().contains(item),
                    (Value::String(s), Value::String(sub)) => s.contains(sub.as_str()),
                    (Value::String(s), item) => s.contains(&item.to_string()),
                    _ => return Err("to_contain expects array or string".to_string()),
                };
                if contains {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to contain {:?}", actual, item))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_valid_json".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_valid_json",
            Some(0),
            |args| {
                let actual = get_actual(&args)?;
                if let Value::String(s) = actual {
                    match serde_json::from_str::<serde_json::Value>(&s) {
                        Ok(_) => {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        }
                        Err(_) => Err("Expected valid JSON string".to_string()),
                    }
                } else {
                    Err("to_be_valid_json expects string".to_string())
                }
            },
        )),
    );

    let expectation_class = Class {
        name: "Expectation".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: expectation_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        const_fields: HashSet::new(),
        static_const_fields: HashSet::new(),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    };

    env.define(
        "Expectation".to_string(),
        Value::Class(Rc::new(expectation_class)),
    );
}
