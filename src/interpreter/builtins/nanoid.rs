//! NanoID generation built-in.
//!
//! Exposes `nanoid()`, `nanoid(size)`, and `nanoid(size, alphabet)` standalone
//! function forms plus the matching `NanoID.generate(...)` / `NanoID.new(...)`
//! static methods.
//!
//! Defaults: size 21, URL-safe 64-char alphabet (`A-Z a-z 0-9 _ -`).
//! Custom alphabet must be 1-255 characters.

use std::collections::HashMap;
use std::rc::Rc;

use ::nanoid::{alphabet, format, rngs};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

const DEFAULT_SIZE: usize = 21;
const MAX_ALPHABET_LEN: usize = u8::MAX as usize;

fn parse_size(value: &Value) -> Result<usize, String> {
    match value {
        Value::Int(n) if *n > 0 && *n <= 1024 => Ok(*n as usize),
        Value::Int(n) => Err(format!("nanoid size must be between 1 and 1024, got {}", n)),
        other => Err(format!(
            "nanoid size must be an integer, got {}",
            other.type_name()
        )),
    }
}

fn parse_alphabet(value: &Value) -> Result<Vec<char>, String> {
    match value {
        Value::String(s) => {
            let chars: Vec<char> = s.chars().collect();
            if chars.is_empty() {
                return Err("nanoid alphabet cannot be empty".to_string());
            }
            if chars.len() > MAX_ALPHABET_LEN {
                return Err(format!(
                    "nanoid alphabet must have at most {} characters, got {}",
                    MAX_ALPHABET_LEN,
                    chars.len()
                ));
            }
            Ok(chars)
        }
        other => Err(format!(
            "nanoid alphabet must be a string, got {}",
            other.type_name()
        )),
    }
}

fn make_nanoid(args: Vec<Value>) -> Result<Value, String> {
    let (size, alphabet) = match args.as_slice() {
        [] => (DEFAULT_SIZE, None),
        [size] => (parse_size(size)?, None),
        [size, alphabet] => (parse_size(size)?, Some(parse_alphabet(alphabet)?)),
        _ => return Err(format!("nanoid() takes 0-2 args, got {}", args.len())),
    };

    let id = match alphabet {
        Some(custom) => format(rngs::default, &custom, size),
        None => format(rngs::default, &alphabet::SAFE, size),
    };
    Ok(Value::String(id))
}

pub fn register_nanoid_builtins(env: &mut Environment) {
    env.define(
        "nanoid".to_string(),
        Value::NativeFunction(NativeFunction::new("nanoid", None, make_nanoid)),
    );

    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    static_methods.insert(
        "generate".to_string(),
        Rc::new(NativeFunction::new("NanoID.generate", None, make_nanoid)),
    );
    static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("NanoID.new", None, make_nanoid)),
    );

    let nanoid_class = Class {
        name: "NanoID".to_string(),
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
    env.define("NanoID".to_string(), Value::Class(Rc::new(nanoid_class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_21_chars_url_safe() {
        let v = make_nanoid(vec![]).unwrap();
        match v {
            Value::String(s) => {
                assert_eq!(s.len(), DEFAULT_SIZE);
                for c in s.chars() {
                    assert!(
                        c.is_ascii_alphanumeric() || c == '-' || c == '_',
                        "unexpected char in default nanoid: {}",
                        c
                    );
                }
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn custom_size_respected() {
        let v = make_nanoid(vec![Value::Int(10)]).unwrap();
        match v {
            Value::String(s) => assert_eq!(s.len(), 10),
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn custom_alphabet_respected() {
        let v = make_nanoid(vec![Value::Int(16), Value::String("ABC".into())]).unwrap();
        match v {
            Value::String(s) => {
                assert_eq!(s.len(), 16);
                for c in s.chars() {
                    assert!(matches!(c, 'A' | 'B' | 'C'), "unexpected char: {}", c);
                }
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn rejects_zero_or_negative_size() {
        assert!(make_nanoid(vec![Value::Int(0)]).is_err());
        assert!(make_nanoid(vec![Value::Int(-1)]).is_err());
    }

    #[test]
    fn rejects_empty_alphabet() {
        assert!(make_nanoid(vec![Value::Int(8), Value::String(String::new())]).is_err());
    }

    #[test]
    fn rejects_too_many_args() {
        assert!(make_nanoid(vec![
            Value::Int(8),
            Value::String("abc".into()),
            Value::Int(1),
        ])
        .is_err());
    }
}
