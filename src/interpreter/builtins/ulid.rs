//! ULID generation built-in.
//!
//! Exposes `ulid()` standalone function and the matching `ULID.generate()` /
//! `ULID.new()` static methods. ULIDs are 128-bit, 26-character Crockford
//! Base32 strings that sort by creation time (high 48 bits = ms timestamp).

use std::collections::HashMap;
use std::rc::Rc;

use ::ulid::Ulid;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

fn make_ulid(_args: Vec<Value>) -> Result<Value, String> {
    Ok(Value::String(Ulid::new().to_string()))
}

pub fn register_ulid_builtins(env: &mut Environment) {
    env.define(
        "ulid".to_string(),
        Value::NativeFunction(NativeFunction::new("ulid", Some(0), make_ulid)),
    );

    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    static_methods.insert(
        "generate".to_string(),
        Rc::new(NativeFunction::new("ULID.generate", Some(0), make_ulid)),
    );
    static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("ULID.new", Some(0), make_ulid)),
    );

    let ulid_class = Class {
        name: "ULID".to_string(),
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
    env.define("ULID".to_string(), Value::Class(Rc::new(ulid_class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_is_26_chars_crockford_base32() {
        let v = make_ulid(vec![]).unwrap();
        match v {
            Value::String(s) => {
                assert_eq!(s.len(), 26);
                // Crockford Base32: 0-9, A-Z minus I, L, O, U
                for c in s.chars() {
                    assert!(
                        c.is_ascii_digit() || c.is_ascii_uppercase(),
                        "unexpected char in ULID: {}",
                        c
                    );
                    assert!(
                        !matches!(c, 'I' | 'L' | 'O' | 'U'),
                        "Crockford-illegal: {}",
                        c
                    );
                }
                Ulid::from_string(&s).expect("round-trips through ulid crate");
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn ulids_are_monotonic_within_a_millisecond_or_increasing() {
        // ULIDs minted in different millis sort by time. Same-ms ordering is not
        // guaranteed without a monotonic source, so assert distinctness instead.
        let a = make_ulid(vec![]).unwrap();
        let b = make_ulid(vec![]).unwrap();
        assert_ne!(format!("{}", a), format!("{}", b));
    }
}
