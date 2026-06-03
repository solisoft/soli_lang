//! Hex encoding built-in class for SoliLang.
//!
//! Bridges the hex world (`Crypto.modexp`, `Crypto.sha256`, `Crypto.pkcs1_*`
//! all speak hex) and the byte/base64 world (`Base64`, XML-DSig's
//! base64-encoded `DigestValue` / `SignatureValue`):
//!
//! ```text
//! # hex digest -> base64 DigestValue:
//! digest_value = Base64.encode(Hex.decode(Crypto.sha256(canonical_xml)))
//! # base64 value -> hex (to compare against a Crypto.* result):
//! Hex.encode(Base64.decode(incoming_b64))
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn register_hex_class(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Hex.encode(data) -> String — bytes/string to lowercase hex.
    methods.insert(
        "encode".to_string(),
        Rc::new(NativeFunction::new("Hex.encode", Some(1), |args| {
            let bytes: Vec<u8> = match &args[0] {
                Value::String(s) => s.as_bytes().to_vec(),
                Value::Array(arr) => arr
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                        Value::Int(n) => {
                            Err(format!("Hex.encode(): byte value {} out of range", n))
                        }
                        other => Err(format!(
                            "Hex.encode(): expected byte (Int 0-255), got {}",
                            other.type_name()
                        )),
                    })
                    .collect::<Result<_, _>>()?,
                other => {
                    return Err(format!(
                        "Hex.encode() expects string or byte array, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(bytes_to_hex(&bytes).into()))
        })),
    );

    // Hex.decode(hex) -> Array<Int> — hex string (optional 0x prefix) to bytes.
    methods.insert(
        "decode".to_string(),
        Rc::new(NativeFunction::new("Hex.decode", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Hex.decode() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let hex = s
                .strip_prefix("0x")
                .or_else(|| s.strip_prefix("0X"))
                .unwrap_or(&s);
            if !hex.len().is_multiple_of(2) {
                return Err("Hex.decode(): odd-length hex string".to_string());
            }
            let mut bytes = Vec::with_capacity(hex.len() / 2);
            for i in (0..hex.len()).step_by(2) {
                let byte = u8::from_str_radix(&hex[i..i + 2], 16)
                    .map_err(|_| format!("Hex.decode(): invalid hex byte '{}'", &hex[i..i + 2]))?;
                bytes.push(byte);
            }
            let values: Vec<Value> = bytes.into_iter().map(|b| Value::Int(b as i64)).collect();
            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    let class = Class {
        name: "Hex".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("Hex".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(env: &Environment, name: &str, arg: Value) -> Result<Value, String> {
        let class = match env.get("Hex").unwrap() {
            Value::Class(c) => c,
            _ => panic!("Hex not a class"),
        };
        (class.native_static_methods.get(name).unwrap().func)(vec![arg])
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut env = Environment::new();
        register_hex_class(&mut env);
        let bytes = Value::Array(Rc::new(RefCell::new(vec![
            Value::Int(0),
            Value::Int(255),
            Value::Int(16),
        ])));
        let hex = call(&env, "encode", bytes).unwrap();
        assert_eq!(hex, Value::String("00ff10".into()));
        let back = call(&env, "decode", Value::String("0x00ff10".into())).unwrap();
        match back {
            Value::Array(a) => {
                let v: Vec<i64> = a
                    .borrow()
                    .iter()
                    .map(|x| match x {
                        Value::Int(n) => *n,
                        _ => panic!(),
                    })
                    .collect();
                assert_eq!(v, vec![0, 255, 16]);
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn decode_rejects_odd_length() {
        let mut env = Environment::new();
        register_hex_class(&mut env);
        assert!(call(&env, "decode", Value::String("abc".into())).is_err());
    }
}
