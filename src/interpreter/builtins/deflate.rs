//! Raw DEFLATE (RFC 1951) compression built-in class for SoliLang.
//!
//! The SAML 2.0 HTTP-Redirect binding transports the `SAMLRequest` /
//! `SAMLResponse` as base64(raw-DEFLATE(xml)) — *raw* DEFLATE with no zlib or
//! gzip wrapper. This class exposes exactly that, designed to pipe through the
//! existing `Base64` class:
//!
//! ```text
//! # inbound (decode an AuthnRequest from a redirect):
//! xml = Deflate.inflate(Base64.decode(params["SAMLRequest"]))
//! # outbound:
//! param = Base64.encode(Deflate.deflate(xml))
//! ```
//!
//! Byte conventions mirror `Base64`: a `String` input is treated as its UTF-8
//! bytes, an `Array` as raw bytes; `deflate` returns a byte `Array`, `inflate`
//! returns a `String` when the result is valid UTF-8 (else a byte `Array`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::rc::Rc;

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

/// Convert a Soli value to raw bytes: a `String` yields its UTF-8 bytes, an
/// `Array` its byte values. Matches `Base64.encode`'s input handling.
fn value_to_raw_bytes(value: &Value, what: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                Value::Int(n) => Err(format!("{}: byte value {} out of range 0-255", what, n)),
                other => Err(format!(
                    "{}: expected byte (Int 0-255), got {}",
                    what,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{}: expected string or byte array, got {}",
            what,
            other.type_name()
        )),
    }
}

/// Return decompressed bytes as a `String` if valid UTF-8, else a byte `Array`
/// — the same shape `Base64.decode` uses.
fn bytes_to_text_value(bytes: Vec<u8>) -> Value {
    match String::from_utf8(bytes) {
        Ok(s) => Value::String(s.into()),
        Err(e) => {
            let values: Vec<Value> = e
                .into_bytes()
                .into_iter()
                .map(|b| Value::Int(b as i64))
                .collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
    }
}

fn do_deflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| format!("deflate error: {}", e))?;
    encoder
        .finish()
        .map_err(|e| format!("deflate error: {}", e))
}

/// Default cap on `inflate` output. A raw-DEFLATE stream is unauthenticated
/// network input (SAML `SAMLRequest`/`SAMLResponse` from the HTTP-Redirect
/// binding), and a few-KB highly-repetitive payload can inflate to many GB — a
/// decompression bomb. Cap the output and fail closed; raise via
/// `SOLI_DEFLATE_MAX_BYTES` for legitimately large payloads.
const DEFAULT_INFLATE_MAX_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB

fn inflate_max_bytes() -> u64 {
    std::env::var("SOLI_DEFLATE_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_INFLATE_MAX_BYTES)
}

fn do_inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let max = inflate_max_bytes();
    // Read at most `max + 1` bytes so we can distinguish "exactly at the limit"
    // from "the stream wanted to keep going" and reject the latter.
    let mut decoder = DeflateDecoder::new(data).take(max + 1);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("inflate error: {}", e))?;
    if out.len() as u64 > max {
        return Err(format!(
            "inflate error: decompressed output exceeds the {max}-byte limit \
             (raise SOLI_DEFLATE_MAX_BYTES if this is expected)"
        ));
    }
    Ok(out)
}

pub fn register_deflate_builtins(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Deflate.deflate(data) -> Array<Int> (raw DEFLATE bytes)
    methods.insert(
        "deflate".to_string(),
        Rc::new(NativeFunction::new("Deflate.deflate", Some(1), |args| {
            let data = value_to_raw_bytes(&args[0], "Deflate.deflate()")?;
            let compressed = do_deflate(&data)?;
            let values: Vec<Value> = compressed
                .into_iter()
                .map(|b| Value::Int(b as i64))
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    // Deflate.inflate(data) -> String | Array<Int>
    methods.insert(
        "inflate".to_string(),
        Rc::new(NativeFunction::new("Deflate.inflate", Some(1), |args| {
            let data = value_to_raw_bytes(&args[0], "Deflate.inflate()")?;
            let decompressed = do_inflate(&data)?;
            Ok(bytes_to_text_value(decompressed))
        })),
    );

    let class = Class {
        name: "Deflate".to_string(),
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
    env.define("Deflate".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_text() {
        let original = b"<samlp:AuthnRequest xmlns:samlp=\"urn:oasis\"/>";
        let compressed = do_deflate(original).unwrap();
        let back = do_inflate(&compressed).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn deflate_actually_shrinks_repetitive_input() {
        let data = "A".repeat(1000);
        let compressed = do_deflate(data.as_bytes()).unwrap();
        assert!(
            compressed.len() < data.len(),
            "expected compression, got {} -> {}",
            data.len(),
            compressed.len()
        );
        assert_eq!(do_inflate(&compressed).unwrap(), data.as_bytes());
    }

    #[test]
    fn inflate_rejects_garbage() {
        // Random bytes are very unlikely to be a valid DEFLATE stream.
        assert!(do_inflate(&[0xff, 0xfe, 0xfd, 0xfc, 0x00, 0x01]).is_err());
    }

    #[test]
    fn inflate_non_utf8_returns_byte_array() {
        // Compress raw non-UTF-8 bytes, then confirm inflate hands them back
        // as a byte array rather than erroring.
        let raw = vec![0x00u8, 0xff, 0x80, 0xc0];
        let compressed = do_deflate(&raw).unwrap();
        let v = bytes_to_text_value(do_inflate(&compressed).unwrap());
        match v {
            Value::Array(arr) => {
                let bytes: Vec<u8> = arr
                    .borrow()
                    .iter()
                    .map(|x| match x {
                        Value::Int(n) => *n as u8,
                        _ => panic!("expected Int"),
                    })
                    .collect();
                assert_eq!(bytes, raw);
            }
            other => panic!("expected byte array, got {:?}", other),
        }
    }
}
