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

use rand::RngCore;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

const DEFAULT_SIZE: usize = 21;
const MAX_ALPHABET_LEN: usize = u8::MAX as usize;

/// The URL-safe alphabet, in the reference implementation's order.
const SAFE: &[char; 64] = &[
    '_', '-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g',
    'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
];

/// `size` characters drawn uniformly from `alphabet` (1-255 entries): each
/// random byte is masked to the next power of two and rejected when it falls
/// past the alphabet. Bytes come from the thread's CSPRNG — the `nanoid` crate
/// seeded a fresh `StdRng` from the OS for every id.
fn generate(alphabet: &[char], size: usize) -> String {
    let mask = alphabet.len().next_power_of_two() - 1;
    let mut rng = rand::thread_rng();
    let mut id = String::with_capacity(size);
    let mut count = 0;
    let mut bytes = [0u8; 64];
    loop {
        rng.fill_bytes(&mut bytes);
        for &byte in &bytes {
            if let Some(&c) = alphabet.get(byte as usize & mask) {
                id.push(c);
                count += 1;
                if count == size {
                    return id;
                }
            }
        }
    }
}

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

fn make_nanoid(args: &[Value]) -> Result<Value, String> {
    let (size, alphabet) = match args {
        [] => (DEFAULT_SIZE, None),
        [size] => (parse_size(size)?, None),
        [size, alphabet] => (parse_size(size)?, Some(parse_alphabet(alphabet)?)),
        _ => return Err(format!("nanoid() takes 0-2 args, got {}", args.len())),
    };

    let id = match alphabet {
        Some(custom) => generate(&custom, size),
        None => generate(SAFE, size),
    };
    Ok(Value::String(id.into()))
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
        let v = make_nanoid(&[]).unwrap();
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
        let v = make_nanoid(&[Value::Int(10)]).unwrap();
        match v {
            Value::String(s) => assert_eq!(s.len(), 10),
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn custom_alphabet_respected() {
        let v = make_nanoid(&[Value::Int(16), Value::String("ABC".into())]).unwrap();
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
    fn a_multibyte_alphabet_gives_the_requested_number_of_characters() {
        // The `nanoid` crate compared the byte length, so this came back short.
        let v = make_nanoid(&[Value::Int(12), Value::String("é✓".into())]).unwrap();
        match v {
            Value::String(s) => {
                assert_eq!(s.chars().count(), 12);
                assert!(s.chars().all(|c| c == 'é' || c == '✓'));
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn every_character_of_the_alphabet_is_reachable() {
        let id = generate(&['a', 'b', 'c'], 3000);
        for c in ['a', 'b', 'c'] {
            let n = id.chars().filter(|&x| x == c).count();
            assert!((800..1200).contains(&n), "{c}: {n} of 3000");
        }
    }

    #[test]
    fn rejects_zero_or_negative_size() {
        assert!(make_nanoid(&[Value::Int(0)]).is_err());
        assert!(make_nanoid(&[Value::Int(-1)]).is_err());
    }

    #[test]
    fn rejects_empty_alphabet() {
        assert!(make_nanoid(&[Value::Int(8), Value::String(String::new().into())]).is_err());
    }

    #[test]
    fn rejects_too_many_args() {
        assert!(
            make_nanoid(&[Value::Int(8), Value::String("abc".into()), Value::Int(1),]).is_err()
        );
    }
}
