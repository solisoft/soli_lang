//! UUID generation built-ins.
//!
//! Exposes `uuid_v4()` and `uuid_v7()` standalone functions and the matching
//! `UUID.v4()` / `UUID.v7()` static methods.

use std::collections::HashMap;
use std::rc::Rc;

use uuid::Uuid;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

fn make_v4(_args: Vec<Value>) -> Result<Value, String> {
    Ok(Value::String(Uuid::new_v4().to_string()))
}

fn make_v7(_args: Vec<Value>) -> Result<Value, String> {
    Ok(Value::String(Uuid::now_v7().to_string()))
}

pub fn register_uuid_builtins(env: &mut Environment) {
    env.define(
        "uuid_v4".to_string(),
        Value::NativeFunction(NativeFunction::new("uuid_v4", Some(0), make_v4)),
    );
    env.define(
        "uuid_v7".to_string(),
        Value::NativeFunction(NativeFunction::new("uuid_v7", Some(0), make_v7)),
    );

    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    static_methods.insert(
        "v4".to_string(),
        Rc::new(NativeFunction::new("UUID.v4", Some(0), make_v4)),
    );
    static_methods.insert(
        "v7".to_string(),
        Rc::new(NativeFunction::new("UUID.v7", Some(0), make_v7)),
    );

    let uuid_class = Class {
        name: "UUID".to_string(),
        superclass: None,
        methods: Default::default(),
        static_methods: HashMap::new(),
        native_static_methods: static_methods,
        native_methods: HashMap::new(),
        static_fields: Default::default(),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Default::default(),
        ..Default::default()
    };
    env.define("UUID".to_string(), Value::Class(Rc::new(uuid_class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v4_returns_hyphenated_36_char_string() {
        let v = make_v4(vec![]).unwrap();
        match v {
            Value::String(s) => {
                assert_eq!(s.len(), 36);
                let parsed = Uuid::parse_str(&s).expect("valid uuid");
                assert_eq!(parsed.get_version_num(), 4);
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn v7_returns_version_7_uuid() {
        let v = make_v7(vec![]).unwrap();
        match v {
            Value::String(s) => {
                assert_eq!(s.len(), 36);
                let parsed = Uuid::parse_str(&s).expect("valid uuid");
                assert_eq!(parsed.get_version_num(), 7);
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn v4_calls_produce_distinct_values() {
        let a = make_v4(vec![]).unwrap();
        let b = make_v4(vec![]).unwrap();
        assert_ne!(format!("{}", a), format!("{}", b));
    }
}
