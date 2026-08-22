//! Toml / Yaml built-in classes for Soli.
//!
//! Parse and generate TOML and YAML from strings, reusing the JSON
//! conversion plumbing (`serde_json::Value` as the bridge): both crates
//! deserialize/serialize into it, so scalars, arrays, and tables map onto
//! the same Value shapes the app already knows.
//!
//! All methods are static:
//!   Toml.parse(str) / Toml.stringify(hash)
//!   Yaml.parse(str)  / Yaml.stringify(value)

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};
use crate::interpreter::value_json::{json_to_value_ref, value_to_json};

pub fn register_toml_yaml_classes(env: &mut Environment) {
    let mut toml_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Toml.parse(str) -> Hash
    toml_methods.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new("Toml.parse", Some(1), |args| {
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Toml.parse() expects a string, got {}",
                        other.type_name()
                    ))
                }
            };
            let parsed: serde_json::Value =
                toml::from_str(&text).map_err(|e| format!("Toml.parse(): {e}"))?;
            match json_to_value_ref(&parsed)? {
                Value::Hash(h) => Ok(Value::Hash(h)),
                _ => Err("Toml.parse(): document is not a table".to_string()),
            }
        })),
    );

    // Toml.stringify(hash) -> String
    toml_methods.insert(
        "stringify".to_string(),
        Rc::new(NativeFunction::new("Toml.stringify", Some(1), |args| {
            let doc = match &args[0] {
                Value::Hash(_) => {
                    value_to_json(&args[0]).map_err(|e| format!("Toml.stringify(): {e}"))?
                }
                other => {
                    return Err(format!(
                        "Toml.stringify() expects a hash, got {}",
                        other.type_name()
                    ))
                }
            };
            toml::to_string_pretty(&doc)
                .map(|s| Value::String(s.into()))
                .map_err(|e| format!("Toml.stringify(): {e}"))
        })),
    );

    env.define(
        "Toml".to_string(),
        Value::Class(Rc::new(Class {
            name: "Toml".to_string(),
            superclass: None,
            methods: Rc::new(RefCell::new(HashMap::new())),
            static_methods: HashMap::new(),
            native_static_methods: toml_methods,
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        })),
    );

    let mut yaml_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Yaml.parse(str) -> Hash or scalar (whatever the document is)
    yaml_methods.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new("Yaml.parse", Some(1), |args| {
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Yaml.parse() expects a string, got {}",
                        other.type_name()
                    ))
                }
            };
            let parsed: serde_json::Value =
                serde_yaml::from_str(&text).map_err(|e| format!("Yaml.parse(): {e}"))?;
            json_to_value_ref(&parsed)
        })),
    );

    // Yaml.stringify(value) -> String — any serializable Value.
    yaml_methods.insert(
        "stringify".to_string(),
        Rc::new(NativeFunction::new("Yaml.stringify", Some(1), |args| {
            let doc = value_to_json(&args[0]).map_err(|e| format!("Yaml.stringify(): {e}"))?;
            serde_yaml::to_string(&doc)
                .map_err(|e| format!("Yaml.stringify(): {e}"))
                .map(|s| Value::String(s.into()))
        })),
    );

    env.define(
        "Yaml".to_string(),
        Value::Class(Rc::new(Class {
            name: "Yaml".to_string(),
            superclass: None,
            methods: Rc::new(RefCell::new(HashMap::new())),
            static_methods: HashMap::new(),
            native_static_methods: yaml_methods,
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trips() {
        let src = "title = \"demo\"\n\n[owner]\nname = \"Ann\"\nports = [1, 2]\n";
        let parsed: serde_json::Value = toml::from_str(src).unwrap();
        let value = json_to_value_ref(&parsed).unwrap();
        let out = value_to_json(&value).unwrap();
        assert_eq!(out["title"], "demo");
        assert_eq!(out["owner"]["name"], "Ann");
    }

    #[test]
    fn toml_rejects_garbage_cleanly() {
        assert!(toml::from_str::<serde_json::Value>("this is [ not toml").is_err());
    }

    #[test]
    fn yaml_round_trips_scalars_and_nesting() {
        let parsed: serde_json::Value =
            serde_yaml::from_str("name: demo\ncounts:\n  - 1\n  - 2\n").unwrap();
        let value = json_to_value_ref(&parsed).unwrap();
        assert!(matches!(value, Value::Hash(_)));
        // YAML 1.1-style booleans parse per serde_yaml's rules; plain keys work.
        let flat: serde_json::Value = serde_yaml::from_str("on: true").unwrap();
        assert_eq!(flat["on"], true);
    }

    #[test]
    fn yaml_rejects_garbage_cleanly() {
        assert!(serde_yaml::from_str::<serde_json::Value>("\t- [\nbad: : :").is_err());
    }
}
