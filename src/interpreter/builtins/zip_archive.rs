//! `Zip`: list, read and write a zip archive.
//!
//! Uploads are recognized as zip and `Spreadsheet` opens xlsx from the
//! inside. `Zip` is the archive an app can use itself. Inputs and outputs
//! are base64, the same shape as `PDF` and `HTTP`. The whole archive is in
//! memory. Names written by `Zip.create` may not be absolute or contain a
//! `..` segment. `Zip.read` takes any name `Zip.entries` lists, and inflates
//! at most `max_size` bytes of it (32 MiB unless the call says otherwise): a
//! small uploaded archive can declare, or hold, gigabytes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::rc::Rc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::interpreter::builtins::bytes::expect_byte_array;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, HashKey, NativeFunction, Value};

pub fn register_zip_class(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    methods.insert(
        "entries".to_string(),
        Rc::new(NativeFunction::new("Zip.entries", Some(1), |args| {
            let bytes = archive_bytes(args, "Zip.entries")?;
            let mut archive = open(bytes, "Zip.entries")?;
            let mut entries = Vec::with_capacity(archive.len());
            for index in 0..archive.len() {
                let file = archive
                    .by_index(index)
                    .map_err(|error| format!("Zip.entries() could not read an entry: {error}"))?;
                let name = file
                    .name()
                    .map_err(|error| format!("Zip.entries() entry name: {error}"))?;
                entries.push(hash_from_pairs([
                    ("name", Value::String(name.into_owned().into())),
                    ("size", Value::Int(as_int(file.size(), "Zip.entries")?)),
                    (
                        "compressed_size",
                        Value::Int(as_int(file.compressed_size(), "Zip.entries")?),
                    ),
                    ("directory", Value::Bool(file.is_dir())),
                ]));
            }
            Ok(Value::Array(Rc::new(RefCell::new(entries))))
        })),
    );
    methods.insert(
        "read".to_string(),
        Rc::new(NativeFunction::new("Zip.read", None, |args| {
            let bytes = read_entry(args, "Zip.read")?;
            Ok(Value::String(STANDARD.encode(bytes).into()))
        })),
    );
    methods.insert(
        "read_text".to_string(),
        Rc::new(NativeFunction::new("Zip.read_text", None, |args| {
            let bytes = read_entry(args, "Zip.read_text")?;
            match String::from_utf8(bytes) {
                Ok(text) => Ok(Value::String(text.into())),
                Err(_) => Err("Zip.read_text() requires a UTF-8 entry".to_string()),
            }
        })),
    );
    methods.insert(
        "create".to_string(),
        Rc::new(NativeFunction::new("Zip.create", Some(1), |args| {
            let files = match args.first() {
                Some(Value::Hash(hash)) => hash.clone(),
                Some(other) => {
                    return Err(format!(
                        "Zip.create() expects a hash of name to contents, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("Zip.create() requires a hash".to_string()),
            };
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (key, value) in files.borrow().iter() {
                let HashKey::String(name) = key else {
                    return Err("Zip.create() names must be strings".to_string());
                };
                check_name(name, "Zip.create")?;
                let bytes = entry_bytes(value, name)?;
                writer
                    .start_file(name.to_string(), options)
                    .map_err(|error| format!("Zip.create() could not add {name}: {error}"))?;
                writer
                    .write_all(&bytes)
                    .map_err(|error| format!("Zip.create() could not write {name}: {error}"))?;
            }
            let cursor = writer
                .finish()
                .map_err(|error| format!("Zip.create() could not finish the archive: {error}"))?;
            Ok(Value::String(STANDARD.encode(cursor.into_inner()).into()))
        })),
    );
    env.define(
        "Zip".to_string(),
        Value::Class(Rc::new(Class {
            name: "Zip".to_string(),
            native_static_methods: methods,
            ..Default::default()
        })),
    );
}

fn archive_bytes(args: &[Value], method: &str) -> Result<Vec<u8>, String> {
    let encoded = match args.first() {
        Some(Value::String(text)) => text.to_string(),
        Some(other) => {
            return Err(format!(
                "{method}() expects base64 archive, got {}",
                other.type_name()
            ))
        }
        None => return Err(format!("{method}() requires a base64 archive")),
    };
    STANDARD
        .decode(encoded.trim())
        .map_err(|error| format!("{method}() is not standard base64: {error}"))
}

fn open(bytes: Vec<u8>, method: &str) -> Result<zip::ZipArchive<Cursor<Vec<u8>>>, String> {
    zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("{method}() is not a zip archive: {error}"))
}

/// Default cap on one inflated entry: 32 MiB. `{"max_size": n}` moves it.
const DEFAULT_MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

/// `Zip.read(archive, name, options?)`: the entry's bytes, inflated up to
/// `max_size`. The name is looked up as `Zip.entries` lists it, with no
/// check of its shape: nothing here touches the disk.
fn read_entry(args: &[Value], method: &str) -> Result<Vec<u8>, String> {
    if !(2..=3).contains(&args.len()) {
        return Err(format!(
            "{method}() takes an archive, an entry name and optional options, got {} argument(s)",
            args.len()
        ));
    }
    let bytes = archive_bytes(args, method)?;
    let name = match args.get(1) {
        Some(Value::String(text)) => text.to_string(),
        Some(other) => {
            return Err(format!(
                "{method}() expects a string entry name, got {}",
                other.type_name()
            ))
        }
        None => return Err(format!("{method}() requires an entry name")),
    };
    let max = max_entry_bytes(args.get(2), method)?;
    let mut archive = open(bytes, method)?;
    let file = archive.by_name(&name).map_err(|error| match error {
        zip::result::ZipError::FileNotFound => format!("{method}() has no entry \"{name}\""),
        other => format!("{method}() could not open \"{name}\": {other}"),
    })?;
    if file.is_dir() {
        return Err(format!("{method}() entry \"{name}\" is a directory"));
    }
    if file.size() > max {
        return Err(too_large(method, &name, max));
    }
    let mut out = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut out)
        .map_err(|error| format!("{method}() could not read \"{name}\": {error}"))?;
    if out.len() as u64 > max {
        return Err(too_large(method, &name, max));
    }
    Ok(out)
}

fn too_large(method: &str, name: &str, max: u64) -> String {
    format!(
        "{method}() entry \"{name}\" inflates past {max} bytes; \
         pass {{\"max_size\": n}} to read a larger one"
    )
}

fn max_entry_bytes(options: Option<&Value>, method: &str) -> Result<u64, String> {
    let hash = match options {
        None | Some(Value::Null) => return Ok(DEFAULT_MAX_ENTRY_BYTES),
        Some(Value::Hash(hash)) => hash,
        Some(other) => {
            return Err(format!(
                "{method}() options must be a hash, got {}",
                other.type_name()
            ))
        }
    };
    let mut max = DEFAULT_MAX_ENTRY_BYTES;
    for (key, value) in hash.borrow().iter() {
        match key {
            HashKey::String(field) if field.as_ref() == "max_size" => match value {
                Value::Int(size) if *size > 0 => max = *size as u64,
                _ => {
                    return Err(format!(
                        "{method}() max_size must be a positive integer (bytes)"
                    ))
                }
            },
            other => {
                return Err(format!(
                    "{method}() unknown option {} (max_size)",
                    match other {
                        HashKey::String(field) => format!("\"{field}\""),
                        _ => "key".to_string(),
                    }
                ))
            }
        }
    }
    Ok(max)
}

fn entry_bytes(value: &Value, name: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::String(text) => Ok(text.as_bytes().to_vec()),
        Value::Hash(hash) => {
            let encoded = hash.borrow().iter().find_map(|(key, item)| match key {
                HashKey::String(field) if field.as_ref() == "base64" => match item {
                    Value::String(text) => Some(text.to_string()),
                    _ => None,
                },
                _ => None,
            });
            let Some(encoded) = encoded else {
                return Err(format!(
                    "Zip.create() entry \"{name}\" hash needs a \"base64\" string"
                ));
            };
            STANDARD.decode(encoded).map_err(|error| {
                format!("Zip.create() entry \"{name}\" base64 is invalid: {error}")
            })
        }
        Value::Array(_) => expect_byte_array(Some(value), "Zip.create"),
        other => Err(format!(
            "Zip.create() entry \"{name}\" must be a string, a {{\"base64\": ...}} hash, or a byte array, got {}",
            other.type_name()
        )),
    }
}

fn check_name(name: &str, method: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!(
            "{method}() refuses entry name \"{name}\" (relative, no empty, \".\" or \"..\" segments)"
        ));
    }
    Ok(())
}

fn as_int(value: u64, method: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{method}() size does not fit an integer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(env: &Environment, name: &str) -> NativeFunction {
        let Value::Class(class) = env.get("Zip").expect("Zip") else {
            panic!("Zip is not a class");
        };
        class
            .native_static_methods
            .get(name)
            .unwrap_or_else(|| panic!("Zip.{name}"))
            .as_ref()
            .clone()
    }

    #[test]
    fn framework_class_zip_round_trip() {
        let mut env = Environment::new();
        register_zip_class(&mut env);
        let files = hash_from_pairs([
            ("hello.txt", Value::String("hi".into())),
            (
                "bin.dat",
                hash_from_pairs([("base64", Value::String(STANDARD.encode(b"abc").into()))]),
            ),
        ]);
        let archive = (method(&env, "create").func)(&[files]).unwrap();
        let text =
            (method(&env, "read_text").func)(&[archive.clone(), Value::String("hello.txt".into())])
                .unwrap();
        assert_eq!(text, Value::String("hi".into()));
        let raw = (method(&env, "read").func)(&[archive.clone(), Value::String("bin.dat".into())])
            .unwrap();
        assert_eq!(raw, Value::String(STANDARD.encode(b"abc").into()));
        let entries = (method(&env, "entries").func)(&[archive]).unwrap();
        let Value::Array(items) = entries else {
            panic!("entries");
        };
        assert_eq!(items.borrow().len(), 2);
        let err = (method(&env, "create").func)(&[hash_from_pairs([(
            "../secret",
            Value::String("no".into()),
        )])])
        .unwrap_err();
        assert!(err.contains("Zip.create()"), "{err}");
    }

    fn raw_archive(entries: &[(&str, &[u8])], directories: &[&str]) -> Value {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            writer.start_file(name.to_string(), options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        for name in directories {
            writer.add_directory(name.to_string(), options).unwrap();
        }
        Value::String(
            STANDARD
                .encode(writer.finish().unwrap().into_inner())
                .into(),
        )
    }

    #[test]
    fn framework_class_zip_reads_every_listed_name() {
        let mut env = Environment::new();
        register_zip_class(&mut env);
        let archive = raw_archive(
            &[
                ("./readme.txt", b"one"),
                ("docs\\guide.txt", b"two"),
                ("a//b.txt", b"three"),
            ],
            &["dir/"],
        );
        for (name, text) in [
            ("./readme.txt", "one"),
            ("docs\\guide.txt", "two"),
            ("a//b.txt", "three"),
        ] {
            let read =
                (method(&env, "read_text").func)(&[archive.clone(), Value::String(name.into())])
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(read, Value::String(text.into()));
        }
        let dir = (method(&env, "read").func)(&[archive.clone(), Value::String("dir/".into())])
            .unwrap_err();
        assert!(dir.contains("is a directory"), "{dir}");
        let missing =
            (method(&env, "read").func)(&[archive, Value::String("nope".into())]).unwrap_err();
        assert!(missing.contains("has no entry \"nope\""), "{missing}");
    }

    #[test]
    fn framework_class_zip_caps_inflated_size() {
        let mut env = Environment::new();
        register_zip_class(&mut env);
        let big = vec![b'a'; 4096];
        let archive = raw_archive(&[("big.txt", &big)], &[]);
        let name = Value::String("big.txt".into());
        let small = hash_from_pairs([("max_size", Value::Int(1024))]);
        let err = (method(&env, "read").func)(&[archive.clone(), name.clone(), small]).unwrap_err();
        assert!(err.contains("inflates past 1024 bytes"), "{err}");
        let exact = hash_from_pairs([("max_size", Value::Int(4096))]);
        let read =
            (method(&env, "read_text").func)(&[archive.clone(), name.clone(), exact]).unwrap();
        assert_eq!(read, Value::String("a".repeat(4096).into()));
        let read = (method(&env, "read_text").func)(&[archive.clone(), name.clone()]).unwrap();
        assert_eq!(read, Value::String("a".repeat(4096).into()));
        let bad = hash_from_pairs([("max", Value::Int(1))]);
        let err = (method(&env, "read").func)(&[archive, name, bad]).unwrap_err();
        assert!(err.contains("unknown option \"max\""), "{err}");
    }
}
