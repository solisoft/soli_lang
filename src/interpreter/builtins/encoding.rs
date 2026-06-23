//! Character-encoding (charset) built-in class for SoliLang.
//!
//! Soli strings are UTF-8. This class bridges the UTF-8 world and legacy
//! byte encodings (Latin-1 / ISO-8859-1 / Windows-1252, etc.) so that files
//! and HTTP bodies in a non-UTF-8 charset can be imported without turning
//! accented characters into replacement chars:
//!
//! ```text
//! # import a Latin-1 file -> proper UTF-8:
//! text = Encoding.decode(slurp(path, "binary"), "latin1")
//! # or directly: slurp(path, "latin1") / File.read(path, "latin1")
//!
//! # export UTF-8 back to Latin-1 bytes:
//! barf(path, Encoding.encode(text, "latin1"))
//! ```
//!
//! Labels are resolved by `encoding_rs` per the WHATWG Encoding Standard, so
//! `latin1`, `iso-8859-1`, `windows-1252`, `utf-8`, … all work (and
//! `latin1`/`iso-8859-1` alias to `windows-1252`, which is what people
//! actually want for "Latin-1" text).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

/// Coerce a Soli value into raw bytes: a String contributes its UTF-8 bytes,
/// a byte array (`Array<Int 0-255>`) contributes its elements.
fn value_to_bytes(value: &Value, who: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                Value::Int(n) => Err(format!("{}: byte value {} out of range", who, n)),
                other => Err(format!(
                    "{}: expected byte (Int 0-255), got {}",
                    who,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{} expects string or byte array, got {}",
            who,
            other.type_name()
        )),
    }
}

/// Decode `bytes` from the named encoding into a UTF-8 string. For
/// single-byte encodings (latin1/windows-1252) every byte maps, so there is
/// no data loss; malformed sequences in multi-byte encodings are replaced
/// with U+FFFD by `encoding_rs`.
pub fn decode_bytes(bytes: &[u8], label: &str) -> Result<String, String> {
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
        .ok_or_else(|| format!("unknown encoding: {}", label))?;
    let (decoded, _, _had_errors) = encoding.decode(bytes);
    Ok(decoded.into_owned())
}

/// Encode a UTF-8 string into bytes in the named encoding. Characters not
/// representable in the target encoding are replaced by `encoding_rs` with
/// HTML numeric character references (e.g. an emoji becomes `&#128512;`).
pub fn encode_string(s: &str, label: &str) -> Result<Vec<u8>, String> {
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
        .ok_or_else(|| format!("unknown encoding: {}", label))?;
    let (encoded, _, _had_unmappable) = encoding.encode(s);
    Ok(encoded.into_owned())
}

pub fn register_encoding_class(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Encoding.decode(input, label) -> String — bytes/string in `label`
    // encoding decoded to a UTF-8 Soli string.
    methods.insert(
        "decode".to_string(),
        Rc::new(NativeFunction::new("Encoding.decode", Some(2), |args| {
            let bytes = value_to_bytes(&args[0], "Encoding.decode()")?;
            let label = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Encoding.decode() expects encoding label string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(decode_bytes(&bytes, &label)?.into()))
        })),
    );

    // Encoding.encode(string, label) -> Array<Int> — UTF-8 string encoded to
    // bytes in `label` encoding.
    methods.insert(
        "encode".to_string(),
        Rc::new(NativeFunction::new("Encoding.encode", Some(2), |args| {
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Encoding.encode() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let label = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Encoding.encode() expects encoding label string, got {}",
                        other.type_name()
                    ))
                }
            };
            let bytes = encode_string(&text, &label)?;
            let values: Vec<Value> = bytes.into_iter().map(|b| Value::Int(b as i64)).collect();
            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    let class = Class {
        name: "Encoding".to_string(),
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
    env.define("Encoding".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_latin1_accent() {
        // 0xE9 is 'é' in Latin-1 / Windows-1252.
        assert_eq!(
            decode_bytes(&[0x63, 0x61, 0x66, 0xE9], "latin1").unwrap(),
            "café"
        );
    }

    #[test]
    fn iso_8859_1_label_is_supported() {
        assert_eq!(decode_bytes(&[0xE9], "iso-8859-1").unwrap(), "é");
    }

    #[test]
    fn encodes_utf8_to_latin1() {
        assert_eq!(
            encode_string("café", "latin1").unwrap(),
            vec![0x63, 0x61, 0x66, 0xE9]
        );
    }

    #[test]
    fn round_trips() {
        let original = "Curaçao — déjà vu";
        let bytes = encode_string(original, "windows-1252").unwrap();
        assert_eq!(decode_bytes(&bytes, "windows-1252").unwrap(), original);
    }

    #[test]
    fn unknown_label_errors() {
        assert!(decode_bytes(&[0x41], "no-such-encoding").is_err());
        assert!(encode_string("a", "no-such-encoding").is_err());
    }
}
