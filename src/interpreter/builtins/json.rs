//! JSON class for parsing and stringifying JSON data.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{parse_json, stringify_to_string, Class, NativeFunction, Value};

/// Register the JSON class with static methods.
pub fn register_json_class(env: &mut Environment) {
    let mut json_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // JSON.parse(string) - Parse JSON string to Value
    json_static_methods.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new("JSON.parse", Some(1), |mut args| {
            let json_str = match args.swap_remove(0) {
                Value::String(s) => s,
                other => {
                    return Err(format!(
                        "JSON.parse() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            parse_json(&json_str)
        })),
    );

    // JSON.stringify(value) - Convert Value to JSON string
    json_static_methods.insert(
        "stringify".to_string(),
        Rc::new(NativeFunction::new("JSON.stringify", Some(1), |args| {
            let json_str = stringify_to_string(&args[0])
                .map_err(|e| format!("JSON serialization error: {}", e))?;
            Ok(Value::String(json_str))
        })),
    );

    // Create JSON class
    let json_class = Class {
        name: "JSON".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: json_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("JSON".to_string(), Value::Class(Rc::new(json_class)));

    // Legacy standalone aliases: json_stringify() and json_parse()
    env.define(
        "json_stringify".to_string(),
        Value::NativeFunction(NativeFunction::new("json_stringify", Some(1), |args| {
            let json_str = stringify_to_string(&args[0])
                .map_err(|e| format!("JSON serialization error: {}", e))?;
            Ok(Value::String(json_str))
        })),
    );

    env.define(
        "json_parse".to_string(),
        Value::NativeFunction(NativeFunction::new("json_parse", Some(1), |mut args| {
            let json_str = match args.swap_remove(0) {
                Value::String(s) => s,
                other => {
                    return Err(format!(
                        "json_parse() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            parse_json(&json_str)
        })),
    );
}
