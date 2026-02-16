//! File I/O built-in functions.
//!
//! Provides functions for reading and writing files.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value, Class, NativeFunction, Value};

/// Register all file I/O built-in functions.
pub fn register_file_builtins(env: &mut Environment) {
    // barf(path, content) - Write file (auto-detects text vs binary)
    env.define(
        "barf".to_string(),
        Value::NativeFunction(NativeFunction::new("barf", None, |args| match &args[..] {
            [Value::String(path), Value::String(content)] => {
                fs::write(path, content)
                    .map_err(|e| format!("barf failed to write {}: {}", path, e))?;
                Ok(Value::Null)
            }
            [Value::String(path), Value::Array(bytes)] => {
                let byte_vec: Result<Vec<u8>, String> = bytes
                    .borrow()
                    .iter()
                    .map(|b| match b {
                        Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                        Value::Int(n) => Err(format!("byte value {} out of range", n)),
                        other => Err(format!("expected byte, got {}", other.type_name())),
                    })
                    .collect();
                fs::write(path, byte_vec?)
                    .map_err(|e| format!("barf failed to write {}: {}", path, e))?;
                Ok(Value::Null)
            }
            _ => Err("barf expects (string, string) or (string, array<int>)".to_string()),
        })),
    );

    // slurp(path) or slurp(path, mode) - Read file (text or binary)
    env.define(
        "slurp".to_string(),
        Value::NativeFunction(NativeFunction::new("slurp", None, |args| match &args[..] {
            [Value::String(path)] => fs::read_to_string(path)
                .map(Value::String)
                .map_err(|e| format!("slurp failed to read {}: {}", path, e)),
            [Value::String(path), Value::String(mode)] => {
                if mode == "binary" {
                    let bytes = fs::read(path)
                        .map_err(|e| format!("slurp failed to read {}: {}", path, e))?;
                    let value_bytes: Vec<Value> =
                        bytes.iter().map(|&b| Value::Int(b as i64)).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(value_bytes))))
                } else {
                    fs::read_to_string(path)
                        .map(Value::String)
                        .map_err(|e| format!("slurp failed to read {}: {}", path, e))
                }
            }
            _ => Err("slurp expects path or (path, mode)".to_string()),
        })),
    );

    // slurp_json(path) - Read and parse JSON file
    env.define(
        "slurp_json".to_string(),
        Value::NativeFunction(NativeFunction::new("slurp_json", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("slurp_json expects a string path".to_string()),
            };
            let content = fs::read_to_string(&path)
                .map_err(|e| format!("slurp_json failed to read {}: {}", path, e))?;
            let json: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| format!("slurp_json failed to parse {}: {}", path, e))?;
            json_to_value(&json)
        })),
    );

    // Register the File class with static methods
    register_file_class(env);
}

/// Register the File class with static methods for file operations.
fn register_file_class(env: &mut Environment) {
    let mut file_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // File.read(path) - Read file contents as string
    file_static_methods.insert(
        "read".to_string(),
        Rc::new(NativeFunction::new("File.read", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.read() expects string path".to_string()),
            };
            fs::read_to_string(&path)
                .map(Value::String)
                .map_err(|e| format!("File.read() failed: {}", e))
        })),
    );

    // File.write(path, content) - Write content to file
    file_static_methods.insert(
        "write".to_string(),
        Rc::new(NativeFunction::new("File.write", Some(2), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.write() expects string path".to_string()),
            };
            let content = match &args[1] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            fs::write(&path, &content)
                .map(|_| Value::Bool(true))
                .map_err(|e| format!("File.write() failed: {}", e))
        })),
    );

    // File.exists(path) - Check if file exists
    file_static_methods.insert(
        "exists".to_string(),
        Rc::new(NativeFunction::new("File.exists", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.exists() expects string path".to_string()),
            };
            Ok(Value::Bool(Path::new(&path).exists()))
        })),
    );

    // File.delete(path) - Delete a file
    file_static_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("File.delete", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.delete() expects string path".to_string()),
            };
            fs::remove_file(&path)
                .map(|_| Value::Bool(true))
                .map_err(|e| format!("File.delete() failed: {}", e))
        })),
    );

    // File.is_file(path) - Check if path is a file
    file_static_methods.insert(
        "is_file".to_string(),
        Rc::new(NativeFunction::new("File.is_file", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.is_file() expects string path".to_string()),
            };
            Ok(Value::Bool(Path::new(&path).is_file()))
        })),
    );

    // File.is_dir(path) - Check if path is a directory
    file_static_methods.insert(
        "is_dir".to_string(),
        Rc::new(NativeFunction::new("File.is_dir", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.is_dir() expects string path".to_string()),
            };
            Ok(Value::Bool(Path::new(&path).is_dir()))
        })),
    );

    // File.size(path) - Get file size in bytes
    file_static_methods.insert(
        "size".to_string(),
        Rc::new(NativeFunction::new("File.size", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.size() expects string path".to_string()),
            };
            fs::metadata(&path)
                .map(|m| Value::Int(m.len() as i64))
                .map_err(|e| format!("File.size() failed: {}", e))
        })),
    );

    // File.append(path, content) - Append content to file
    file_static_methods.insert(
        "append".to_string(),
        Rc::new(NativeFunction::new("File.append", Some(2), |args| {
            use std::fs::OpenOptions;
            use std::io::Write;

            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.append() expects string path".to_string()),
            };
            let content = match &args[1] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| format!("File.append() failed to open: {}", e))?;
            file.write_all(content.as_bytes())
                .map(|_| Value::Bool(true))
                .map_err(|e| format!("File.append() failed to write: {}", e))
        })),
    );

    // File.lines(path) - Read file as array of lines
    file_static_methods.insert(
        "lines".to_string(),
        Rc::new(NativeFunction::new("File.lines", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.lines() expects string path".to_string()),
            };
            let content =
                fs::read_to_string(&path).map_err(|e| format!("File.lines() failed: {}", e))?;
            let lines: Vec<Value> = content
                .lines()
                .map(|l| Value::String(l.to_string()))
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(lines))))
        })),
    );

    // File.copy(src, dest) - Copy a file
    file_static_methods.insert(
        "copy".to_string(),
        Rc::new(NativeFunction::new("File.copy", Some(2), |args| {
            let src = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.copy() expects string source path".to_string()),
            };
            let dest = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err("File.copy() expects string destination path".to_string()),
            };
            fs::copy(&src, &dest)
                .map(|_| Value::Bool(true))
                .map_err(|e| format!("File.copy() failed: {}", e))
        })),
    );

    // File.rename(old_path, new_path) - Rename/move a file
    file_static_methods.insert(
        "rename".to_string(),
        Rc::new(NativeFunction::new("File.rename", Some(2), |args| {
            let old_path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("File.rename() expects string old path".to_string()),
            };
            let new_path = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err("File.rename() expects string new path".to_string()),
            };
            fs::rename(&old_path, &new_path)
                .map(|_| Value::Bool(true))
                .map_err(|e| format!("File.rename() failed: {}", e))
        })),
    );

    let file_class = Class {
        name: "File".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: file_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("File".to_string(), Value::Class(Rc::new(file_class)));
}
