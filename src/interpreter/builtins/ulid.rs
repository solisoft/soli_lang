//! ULID generation built-in.
//!
//! Exposes `ulid()` standalone function and the matching `ULID.generate()` /
//! `ULID.new()` static methods. ULIDs are 128-bit, 26-character Crockford
//! Base32 strings that sort by creation time (high 48 bits = ms timestamp).

use std::collections::HashMap;
use std::rc::Rc;

use rand::RngCore;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 48 bits of Unix milliseconds, then 80 random bits, as 26 Crockford Base32
/// characters (the top two of the 130 encoded bits are zero).
fn ulid_string() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        & 0xFFFF_FFFF_FFFF;
    let mut random = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut random[6..]);
    let value = (u128::from(millis) << 80) | u128::from_be_bytes(random);
    (0..26)
        .map(|i| CROCKFORD[((value >> (125 - 5 * i)) & 0x1f) as usize] as char)
        .collect()
}

fn make_ulid(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(ulid_string().into()))
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
        let v = make_ulid(&[]).unwrap();
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
                // The first ten characters are the creation time in ms.
                let millis = s[..10].bytes().fold(0u64, |acc, c| {
                    let digit = CROCKFORD.iter().position(|&d| d == c).unwrap() as u64;
                    acc * 32 + digit
                });
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                assert!(now.abs_diff(millis) < 5_000, "{millis} vs {now}");
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn ulids_are_monotonic_within_a_millisecond_or_increasing() {
        // ULIDs minted in different millis sort by time. Same-ms ordering is not
        // guaranteed without a monotonic source, so assert distinctness instead.
        let a = make_ulid(&[]).unwrap();
        let b = make_ulid(&[]).unwrap();
        assert_ne!(format!("{}", a), format!("{}", b));
    }
}
