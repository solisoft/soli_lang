//! `Bytes`: one binary value.
//!
//! PDF, images, uploads and HTTP bodies are base64 strings, or an array of
//! integers when a caller asks for bytes. `Bytes` is that value: length,
//! slice, and base64, UTF-8 and byte-array in and out. The payload sits in a
//! hidden base64 field, so this does not add a `Value` variant.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

const FIELD: &str = "__b64";

thread_local! {
    static BYTES_CLASS: RefCell<Option<Rc<Class>>> = const { RefCell::new(None) };
}

fn bytes_class() -> Result<Rc<Class>, String> {
    BYTES_CLASS
        .with(|slot| slot.borrow().clone())
        .ok_or_else(|| "Bytes class is not registered".to_string())
}

pub fn register_bytes_class(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    methods.insert(
        "from_base64".to_string(),
        Rc::new(NativeFunction::new("Bytes.from_base64", Some(1), |args| {
            let encoded = expect_string(args, "Bytes.from_base64")?;
            decode_standard(&encoded)?;
            instance(&encoded)
        })),
    );
    methods.insert(
        "from_string".to_string(),
        Rc::new(NativeFunction::new("Bytes.from_string", Some(1), |args| {
            let text = expect_string(args, "Bytes.from_string")?;
            instance(&STANDARD.encode(text.as_bytes()))
        })),
    );
    methods.insert(
        "from_array".to_string(),
        Rc::new(NativeFunction::new("Bytes.from_array", Some(1), |args| {
            let bytes = expect_byte_array(args.first(), "Bytes.from_array")?;
            instance(&STANDARD.encode(bytes))
        })),
    );

    let mut instance_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    instance_methods.insert(
        "length".to_string(),
        Rc::new(NativeFunction::new("Bytes.length", Some(0), |args| {
            Ok(Value::Int(payload(args, "Bytes.length")?.len() as i64))
        })),
    );
    instance_methods.insert(
        "to_base64".to_string(),
        Rc::new(NativeFunction::new("Bytes.to_base64", Some(0), |args| {
            let _ = payload(args, "Bytes.to_base64")?;
            Ok(Value::String(stored(args, "Bytes.to_base64")?.into()))
        })),
    );
    instance_methods.insert(
        "to_string".to_string(),
        Rc::new(NativeFunction::new("Bytes.to_string", Some(0), |args| {
            let bytes = payload(args, "Bytes.to_string")?;
            match String::from_utf8(bytes) {
                Ok(text) => Ok(Value::String(text.into())),
                Err(_) => Err("Bytes.to_string() requires UTF-8 bytes".to_string()),
            }
        })),
    );
    instance_methods.insert(
        "to_array".to_string(),
        Rc::new(NativeFunction::new("Bytes.to_array", Some(0), |args| {
            let bytes = payload(args, "Bytes.to_array")?;
            Ok(Value::Array(Rc::new(RefCell::new(
                bytes
                    .into_iter()
                    .map(|byte| Value::Int(byte as i64))
                    .collect(),
            ))))
        })),
    );
    instance_methods.insert(
        "slice".to_string(),
        Rc::new(NativeFunction::new("Bytes.slice", None, |args| {
            let bytes = payload(args, "Bytes.slice")?;
            let start = expect_index(args.get(1), "Bytes.slice", "start")?;
            let end = match args.get(2) {
                None => bytes.len(),
                Some(_) => expect_index(args.get(2), "Bytes.slice", "end")?,
            };
            if start > end || end > bytes.len() {
                return Err(format!(
                    "Bytes.slice() range {start}..{end} is outside 0..{}",
                    bytes.len()
                ));
            }
            instance(&STANDARD.encode(&bytes[start..end]))
        })),
    );

    let class = Rc::new(Class {
        name: "Bytes".to_string(),
        native_static_methods: methods,
        native_methods: instance_methods,
        ..Default::default()
    });
    BYTES_CLASS.with(|slot| *slot.borrow_mut() = Some(class.clone()));
    env.define("Bytes".to_string(), Value::Class(class));
}

fn instance(encoded: &str) -> Result<Value, String> {
    let class = bytes_class()?;
    let mut inst = Instance::new(class);
    inst.set(FIELD, Value::String(encoded.into()));
    Ok(Value::Instance(Rc::new(RefCell::new(inst))))
}

fn stored(args: &[Value], method: &str) -> Result<String, String> {
    let Value::Instance(inst) = args
        .first()
        .ok_or_else(|| format!("{method}() expects a Bytes"))?
    else {
        return Err(format!("{method}() expects a Bytes"));
    };
    match inst.borrow().get(FIELD) {
        Some(Value::String(text)) => Ok(text.to_string()),
        _ => Err(format!("{method}() expects a Bytes")),
    }
}

fn payload(args: &[Value], method: &str) -> Result<Vec<u8>, String> {
    decode_standard(&stored(args, method)?)
}

fn decode_standard(encoded: &str) -> Result<Vec<u8>, String> {
    STANDARD
        .decode(encoded)
        .map_err(|error| format!("Bytes.from_base64() is not standard base64: {error}"))
}

fn expect_string(args: &[Value], method: &str) -> Result<String, String> {
    match args.first() {
        Some(Value::String(text)) => Ok(text.to_string()),
        Some(other) => Err(format!(
            "{method}() expects a string, got {}",
            other.type_name()
        )),
        None => Err(format!("{method}() requires a string")),
    }
}

fn expect_index(value: Option<&Value>, method: &str, what: &str) -> Result<usize, String> {
    match value {
        Some(Value::Int(index)) if *index >= 0 => Ok(*index as usize),
        Some(Value::Int(index)) => Err(format!("{method}() {what} must be >= 0, got {index}")),
        Some(other) => Err(format!(
            "{method}() {what} must be an integer, got {}",
            other.type_name()
        )),
        None => Err(format!("{method}() requires a {what}")),
    }
}

pub(crate) fn expect_byte_array(value: Option<&Value>, method: &str) -> Result<Vec<u8>, String> {
    let Some(Value::Array(items)) = value else {
        return Err(format!(
            "{method}() expects an array of bytes, got {}",
            value
                .map(|item| item.type_name())
                .unwrap_or_else(|| "nothing".to_string())
        ));
    };
    items
        .borrow()
        .iter()
        .map(|item| match item {
            Value::Int(byte) if (0..=255).contains(byte) => Ok(*byte as u8),
            Value::Int(byte) => Err(format!("{method}() byte {byte} is outside 0..255")),
            other => Err(format!(
                "{method}() expects a byte, got {}",
                other.type_name()
            )),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(env: &Environment, name: &str, instance: bool) -> NativeFunction {
        let Value::Class(class) = env.get("Bytes").expect("Bytes") else {
            panic!("Bytes is not a class");
        };
        let table = if instance {
            &class.native_methods
        } else {
            &class.native_static_methods
        };
        table
            .get(name)
            .unwrap_or_else(|| panic!("Bytes.{name}"))
            .as_ref()
            .clone()
    }

    #[test]
    fn framework_class_bytes_round_trip() {
        let mut env = Environment::new();
        register_bytes_class(&mut env);
        let made =
            (method(&env, "from_string", false).func)(&[Value::String("hello".into())]).unwrap();
        let encoded = (method(&env, "to_base64", true).func)(std::slice::from_ref(&made)).unwrap();
        assert_eq!(encoded, Value::String("aGVsbG8=".into()));
        let length = (method(&env, "length", true).func)(std::slice::from_ref(&made)).unwrap();
        assert_eq!(length, Value::Int(5));
        let sliced =
            (method(&env, "slice", true).func)(&[made.clone(), Value::Int(1), Value::Int(4)])
                .unwrap();
        let text = (method(&env, "to_string", true).func)(&[sliced]).unwrap();
        assert_eq!(text, Value::String("ell".into()));

        let bytes =
            (method(&env, "from_array", false).func)(&[Value::Array(Rc::new(RefCell::new(vec![
                Value::Int(104),
                Value::Int(105),
            ])))])
            .unwrap();
        let back = (method(&env, "to_string", true).func)(&[bytes]).unwrap();
        assert_eq!(back, Value::String("hi".into()));

        let err =
            (method(&env, "from_base64", false).func)(&[Value::String("@@@".into())]).unwrap_err();
        assert!(err.contains("Bytes.from_base64()"), "{err}");
    }
}
