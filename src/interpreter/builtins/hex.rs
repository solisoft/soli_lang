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

/// Lowercase hex of `bytes`. Every hex encoder in the crate goes through here.
pub(crate) fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

/// Why a string is not hex. Callers word the message themselves.
#[derive(Debug, PartialEq)]
pub(crate) enum HexError {
    OddLength,
    /// The offending pair, lossily decoded.
    InvalidPair(String),
}

/// The bytes a hex string spells, either case. It reads the string as bytes,
/// so a multi-byte character is an invalid pair rather than a slice through
/// it (which panics), and `+` is not a digit (`u8::from_str_radix` takes it
/// as a sign, so `"+f+f"` used to decode).
pub(crate) fn decode(hex: &str) -> Result<Vec<u8>, HexError> {
    let raw = hex.as_bytes();
    if !raw.len().is_multiple_of(2) {
        return Err(HexError::OddLength);
    }
    let nibble = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let (pairs, _) = raw.as_chunks::<2>();
    pairs
        .iter()
        .map(|&[a, b]| match (nibble(a), nibble(b)) {
            (Some(hi), Some(lo)) => Ok(hi << 4 | lo),
            _ => Err(HexError::InvalidPair(
                String::from_utf8_lossy(&[a, b]).into_owned(),
            )),
        })
        .collect()
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
            Ok(Value::String(encode(&bytes).into()))
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
            let bytes = decode(hex).map_err(|e| match e {
                HexError::OddLength => "Hex.decode(): odd-length hex string".to_string(),
                HexError::InvalidPair(pair) => {
                    format!("Hex.decode(): invalid hex byte '{}'", pair)
                }
            })?;
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
        (class.native_static_methods.get(name).unwrap().func)(&[arg])
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
    fn decode_rejects_a_multibyte_character_instead_of_panicking() {
        let mut env = Environment::new();
        register_hex_class(&mut env);
        // Four bytes, so the length check passes; `é` straddles the first pair.
        assert!(call(&env, "decode", Value::String("aé1".into())).is_err());
    }

    #[test]
    fn decode_rejects_a_sign() {
        assert!(matches!(decode("+f+f"), Err(HexError::InvalidPair(_))));
        assert!(matches!(decode("0A"), Ok(ref b) if b == &[10]));
    }

    #[test]
    fn encode_is_lowercase_and_zero_padded() {
        assert_eq!(encode(&[0x00, 0x0f, 0xab]), "000fab");
        assert_eq!(encode(&[]), "");
    }

    #[test]
    fn decode_rejects_odd_length() {
        let mut env = Environment::new();
        register_hex_class(&mut env);
        assert!(call(&env, "decode", Value::String("abc".into())).is_err());
    }
}
