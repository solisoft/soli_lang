//! JSON class for parsing and stringifying JSON data.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

/// Register the JSON class with static methods.
pub fn register_json_class(env: &mut Environment) {
    let mut json_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // JSON.parse(string) - Parse JSON string to Value
    json_static_methods.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new("JSON.parse", Some(1), |args| {
            let json_str = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "JSON.parse() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let json = serde_json::from_str::<serde_json::Value>(&json_str)
                .map_err(|e| format!("Failed to parse JSON: {}", e))?;
            crate::interpreter::value::json_to_value(&json)
        })),
    );

    // JSON.stringify(value) - Convert Value to JSON string
    json_static_methods.insert(
        "stringify".to_string(),
        Rc::new(NativeFunction::new("JSON.stringify", Some(1), |args| {
            let json = crate::interpreter::value::value_to_json(&args[0])
                .map_err(|e| format!("JSON serialization error: {}", e))?;
            let json_str = serde_json::to_string(&json)
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
}
