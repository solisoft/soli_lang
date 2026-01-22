//! Built-in functions for Soli.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

// Re-export submodules
pub mod assertions;
pub mod controller;
pub mod crypto;
pub mod datetime;
pub mod datetime_class;
pub mod dotenv;
pub mod env;
pub mod factories;
pub mod http;
pub mod i18n;
pub mod jwt;
pub mod model;
pub mod regex;
pub mod router;
pub mod server;
pub mod session;
pub mod solidb;
pub mod template;
pub mod test_dsl;
pub mod validation;

/// Register all built-in functions in the given environment.
pub fn register_builtins(env: &mut Environment) {
    // print(...) - Print values to stdout (auto-resolves Futures)
    env.define(
        "print".to_string(),
        Value::NativeFunction(NativeFunction::new("print", None, |args| {
            for (i, arg) in args.into_iter().enumerate() {
                if i > 0 {
                    print!(" ");
                }
                // Auto-resolve futures before printing
                let resolved = arg.resolve()?;
                print!("{}", resolved);
            }
            println!();
            Ok(Value::Null)
        })),
    );

    // println(...) - Same as print, alias
    env.define(
        "println".to_string(),
        Value::NativeFunction(NativeFunction::new("println", None, |args| {
            for (i, arg) in args.into_iter().enumerate() {
                if i > 0 {
                    print!(" ");
                }
                // Auto-resolve futures before printing
                let resolved = arg.resolve()?;
                print!("{}", resolved);
            }
            println!();
            Ok(Value::Null)
        })),
    );

    // break() - Trigger a breakpoint for debugging (opens dev page with REPL)
    env.define(
        "break".to_string(),
        Value::NativeFunction(NativeFunction::new("break", Some(0), |_args| {
            Ok(Value::Breakpoint)
        })),
    );

    // input(prompt?) - Read a line from stdin
    env.define(
        "input".to_string(),
        Value::NativeFunction(NativeFunction::new("input", None, |args| {
            if let Some(Value::String(prompt)) = args.first() {
                print!("{}", prompt);
                io::stdout().flush().ok();
            }
            let mut line = String::new();
            io::stdin()
                .read_line(&mut line)
                .map_err(|e| e.to_string())?;
            Ok(Value::String(line.trim_end().to_string()))
        })),
    );

    // len(array|string|hash) - Get length (auto-resolves Futures)
    env.define(
        "len".to_string(),
        Value::NativeFunction(NativeFunction::new("len", Some(1), |args| {
            let resolved = args.into_iter().next().unwrap().resolve()?;
            match &resolved {
                Value::Array(arr) => Ok(Value::Int(arr.borrow().len() as i64)),
                Value::String(s) => Ok(Value::Int(s.len() as i64)),
                Value::Hash(hash) => Ok(Value::Int(hash.borrow().len() as i64)),
                other => Err(format!(
                    "len() expects array, string, or hash, got {}",
                    other.type_name()
                )),
            }
        })),
    );

    // push(array, value) - Add element to array
    env.define(
        "push".to_string(),
        Value::NativeFunction(NativeFunction::new("push", Some(2), |args| {
            match &args[0] {
                Value::Array(arr) => {
                    arr.borrow_mut().push(args[1].clone());
                    Ok(Value::Null)
                }
                other => Err(format!("push() expects array, got {}", other.type_name())),
            }
        })),
    );

    // pop(array) - Remove and return last element
    env.define(
        "pop".to_string(),
        Value::NativeFunction(NativeFunction::new("pop", Some(1), |args| match &args[0] {
            Value::Array(arr) => arr
                .borrow_mut()
                .pop()
                .ok_or_else(|| "pop() on empty array".to_string()),
            other => Err(format!("pop() expects array, got {}", other.type_name())),
        })),
    );

    // str(value) - Convert to string (auto-resolves Futures)
    env.define(
        "str".to_string(),
        Value::NativeFunction(NativeFunction::new("str", Some(1), |args| {
            let resolved = args.into_iter().next().unwrap().resolve()?;
            Ok(Value::String(format!("{}", resolved)))
        })),
    );

    // await(future) - Explicitly wait for a Future to resolve
    env.define(
        "await".to_string(),
        Value::NativeFunction(NativeFunction::new("await", Some(1), |args| {
            args.into_iter().next().unwrap().resolve()
        })),
    );

    // int(value) - Convert to int
    env.define(
        "int".to_string(),
        Value::NativeFunction(NativeFunction::new("int", Some(1), |args| match &args[0] {
            Value::Int(n) => Ok(Value::Int(*n)),
            Value::Float(n) => Ok(Value::Int(*n as i64)),
            Value::String(s) => s
                .parse::<i64>()
                .map(Value::Int)
                .map_err(|_| format!("cannot convert '{}' to int", s)),
            Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
            other => Err(format!("cannot convert {} to int", other.type_name())),
        })),
    );

    // float(value) - Convert to float
    env.define(
        "float".to_string(),
        Value::NativeFunction(NativeFunction::new("float", Some(1), |args| {
            match &args[0] {
                Value::Int(n) => Ok(Value::Float(*n as f64)),
                Value::Float(n) => Ok(Value::Float(*n)),
                Value::String(s) => s
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| format!("cannot convert '{}' to float", s)),
                other => Err(format!("cannot convert {} to float", other.type_name())),
            }
        })),
    );

    // type(value) - Get type name as string
    env.define(
        "type".to_string(),
        Value::NativeFunction(NativeFunction::new("type", Some(1), |args| {
            Ok(Value::String(args[0].type_name().to_string()))
        })),
    );

    // clock() - Current time in seconds since epoch
    env.define(
        "clock".to_string(),
        Value::NativeFunction(NativeFunction::new("clock", Some(0), |_| {
            let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            Ok(Value::Float(duration.as_secs_f64()))
        })),
    );

    // range(start, end) - Create array from start to end-1
    env.define(
        "range".to_string(),
        Value::NativeFunction(NativeFunction::new("range", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(start), Value::Int(end)) => {
                    let arr: Vec<Value> = (*start..*end).map(Value::Int).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(arr))))
                }
                _ => Err("range() expects two integers".to_string()),
            }
        })),
    );

    // abs(number) - Absolute value
    env.define(
        "abs".to_string(),
        Value::NativeFunction(NativeFunction::new("abs", Some(1), |args| match &args[0] {
            Value::Int(n) => Ok(Value::Int(n.abs())),
            Value::Float(n) => Ok(Value::Float(n.abs())),
            other => Err(format!("abs() expects number, got {}", other.type_name())),
        })),
    );

    // min(a, b) - Minimum of two values
    env.define(
        "min".to_string(),
        Value::NativeFunction(NativeFunction::new("min", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a.min(b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.min(*b))),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).min(*b))),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.min(*b as f64))),
                _ => Err("min() expects two numbers".to_string()),
            }
        })),
    );

    // max(a, b) - Maximum of two values
    env.define(
        "max".to_string(),
        Value::NativeFunction(NativeFunction::new("max", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a.max(b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.max(*b))),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).max(*b))),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.max(*b as f64))),
                _ => Err("max() expects two numbers".to_string()),
            }
        })),
    );

    // sqrt(number) - Square root
    env.define(
        "sqrt".to_string(),
        Value::NativeFunction(NativeFunction::new("sqrt", Some(1), |args| {
            match &args[0] {
                Value::Int(n) => Ok(Value::Float((*n as f64).sqrt())),
                Value::Float(n) => Ok(Value::Float(n.sqrt())),
                other => Err(format!("sqrt() expects number, got {}", other.type_name())),
            }
        })),
    );

    // pow(base, exp) - Exponentiation
    env.define(
        "pow".to_string(),
        Value::NativeFunction(NativeFunction::new("pow", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(base), Value::Int(exp)) => {
                    if *exp >= 0 {
                        Ok(Value::Int(base.pow(*exp as u32)))
                    } else {
                        Ok(Value::Float((*base as f64).powi(*exp as i32)))
                    }
                }
                (Value::Float(base), Value::Int(exp)) => Ok(Value::Float(base.powi(*exp as i32))),
                (Value::Int(base), Value::Float(exp)) => {
                    Ok(Value::Float((*base as f64).powf(*exp)))
                }
                (Value::Float(base), Value::Float(exp)) => Ok(Value::Float(base.powf(*exp))),
                _ => Err("pow() expects two numbers".to_string()),
            }
        })),
    );

    // ===== Hash functions =====

    // keys(hash) - Get all keys as array
    env.define(
        "keys".to_string(),
        Value::NativeFunction(NativeFunction::new("keys", Some(1), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    let keys: Vec<Value> = hash.borrow().iter().map(|(k, _)| k.clone()).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(keys))))
                }
                other => Err(format!("keys() expects hash, got {}", other.type_name())),
            }
        })),
    );

    // values(hash) - Get all values as array
    env.define(
        "values".to_string(),
        Value::NativeFunction(NativeFunction::new("values", Some(1), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    let values: Vec<Value> = hash.borrow().iter().map(|(_, v)| v.clone()).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(values))))
                }
                other => Err(format!("values() expects hash, got {}", other.type_name())),
            }
        })),
    );

    // has_key(hash, key) - Check if key exists
    env.define(
        "has_key".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "has_key",
            Some(2),
            |args| match &args[0] {
                Value::Hash(hash) => {
                    let key = &args[1];
                    if !key.is_hashable() {
                        return Err(format!("{} cannot be used as a hash key", key.type_name()));
                    }
                    let exists = hash.borrow().iter().any(|(k, _)| key.hash_eq(k));
                    Ok(Value::Bool(exists))
                }
                other => Err(format!(
                    "has_key() expects hash as first argument, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // delete(hash, key) - Remove key and return its value (or null)
    env.define(
        "delete".to_string(),
        Value::NativeFunction(NativeFunction::new("delete", Some(2), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    let key = &args[1];
                    if !key.is_hashable() {
                        return Err(format!("{} cannot be used as a hash key", key.type_name()));
                    }
                    let mut hash = hash.borrow_mut();
                    let mut removed_value = Value::Null;
                    hash.retain(|(k, v)| {
                        if key.hash_eq(k) {
                            removed_value = v.clone();
                            false
                        } else {
                            true
                        }
                    });
                    Ok(removed_value)
                }
                other => Err(format!(
                    "delete() expects hash as first argument, got {}",
                    other.type_name()
                )),
            }
        })),
    );

    // merge(hash1, hash2) - Merge two hashes (returns new hash, hash2 values win)
    env.define(
        "merge".to_string(),
        Value::NativeFunction(NativeFunction::new("merge", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Hash(hash1), Value::Hash(hash2)) => {
                    let mut result: Vec<(Value, Value)> = hash1.borrow().clone();
                    for (k2, v2) in hash2.borrow().iter() {
                        let mut found = false;
                        for (k1, v1) in result.iter_mut() {
                            if k2.hash_eq(k1) {
                                *v1 = v2.clone();
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            result.push((k2.clone(), v2.clone()));
                        }
                    }
                    Ok(Value::Hash(Rc::new(RefCell::new(result))))
                }
                _ => Err("merge() expects two hashes".to_string()),
            }
        })),
    );

    // entries(hash) / to_a(hash) - Get array of [key, value] pairs
    env.define(
        "entries".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "entries",
            Some(1),
            |args| match &args[0] {
                Value::Hash(hash) => {
                    let pairs: Vec<Value> = hash
                        .borrow()
                        .iter()
                        .map(|(k, v)| {
                            Value::Array(Rc::new(RefCell::new(vec![k.clone(), v.clone()])))
                        })
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(pairs))))
                }
                other => Err(format!("entries() expects hash, got {}", other.type_name())),
            },
        )),
    );

    // from_entries(array) - Create hash from array of [key, value] pairs (reverse of entries)
    env.define(
        "from_entries".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "from_entries",
            Some(1),
            |args| match &args[0] {
                Value::Array(arr) => {
                    let mut result: Vec<(Value, Value)> = Vec::new();

                    for entry in arr.borrow().iter() {
                        match entry {
                            Value::Array(pair) => {
                                let borrowed = pair.borrow();
                                if borrowed.len() != 2 {
                                    return Err(format!(
                                        "from_entries expects array of [key, value] pairs, got array with {} elements",
                                        borrowed.len()
                                    ));
                                }
                                let key = &borrowed[0];
                                if !key.is_hashable() {
                                    return Err(format!(
                                        "{} cannot be used as a hash key",
                                        key.type_name()
                                    ));
                                }
                                // Update existing key or add new one
                                let mut found = false;
                                for (k, v) in result.iter_mut() {
                                    if k.hash_eq(key) {
                                        *v = borrowed[1].clone();
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    result.push((key.clone(), borrowed[1].clone()));
                                }
                            }
                            other => {
                                return Err(format!(
                                    "from_entries expects array of [key, value] pairs, got {}",
                                    other.type_name()
                                ));
                            }
                        }
                    }

                    Ok(Value::Hash(Rc::new(RefCell::new(result))))
                }
                other => Err(format!("from_entries() expects array, got {}", other.type_name())),
            },
        )),
    );

    // clear(hash) - Remove all entries from hash (mutates)
    env.define(
        "clear".to_string(),
        Value::NativeFunction(NativeFunction::new("clear", Some(1), |args| {
            match &args[0] {
                Value::Hash(hash) => {
                    hash.borrow_mut().clear();
                    Ok(Value::Null)
                }
                Value::Array(arr) => {
                    arr.borrow_mut().clear();
                    Ok(Value::Null)
                }
                other => Err(format!(
                    "clear() expects hash or array, got {}",
                    other.type_name()
                )),
            }
        })),
    );

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

    // split(string, delimiter) - Split string by delimiter, return array
    env.define(
        "split".to_string(),
        Value::NativeFunction(NativeFunction::new("split", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(delim)) => {
                    let parts: Vec<Value> = s
                        .split(delim.as_str())
                        .map(|p| Value::String(p.to_string()))
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(parts))))
                }
                _ => Err("split requires (string, string)".to_string()),
            }
        })),
    );

    // join(array, delimiter) - Join array elements with delimiter
    env.define(
        "join".to_string(),
        Value::NativeFunction(NativeFunction::new("join", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Array(arr), Value::String(delim)) => {
                    let parts: Vec<String> = arr
                        .borrow()
                        .iter()
                        .map(|v| format!("{}", v))
                        .collect();
                    Ok(Value::String(parts.join(delim.as_str())))
                }
                _ => Err("join requires (array, string)".to_string()),
            }
        })),
    );

    // contains(string, substring) - Check if string contains substring
    env.define(
        "contains".to_string(),
        Value::NativeFunction(NativeFunction::new("contains", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(sub)) => {
                    Ok(Value::Bool(s.contains(sub.as_str())))
                }
                _ => Err("contains requires (string, string)".to_string()),
            }
        })),
    );

    // index_of(string, substring) - Find index of substring (-1 if not found)
    env.define(
        "index_of".to_string(),
        Value::NativeFunction(NativeFunction::new("index_of", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(sub)) => {
                    if let Some(idx) = s.find(sub.as_str()) {
                        Ok(Value::Int(idx as i64))
                    } else {
                        Ok(Value::Int(-1))
                    }
                }
                _ => Err("index_of requires (string, string)".to_string()),
            }
        })),
    );

    // substring(string, start, end) - Get substring from start to end
    env.define(
        "substring".to_string(),
        Value::NativeFunction(NativeFunction::new("substring", Some(3), |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(start), Value::Int(end)) => {
                    let start_usize = if *start < 0 { 0 } else { *start as usize };
                    let end_usize = if *end > s.len() as i64 { s.len() as i64 } else { *end } as usize;
                    if start_usize >= end_usize || start_usize >= s.len() {
                        return Ok(Value::String(String::new()));
                    }
                    Ok(Value::String(s[start_usize..end_usize].to_string()))
                }
                _ => Err("substring requires (string, int, int)".to_string()),
            }
        })),
    );

    // upcase(string) - Convert to uppercase
    env.define(
        "upcase".to_string(),
        Value::NativeFunction(NativeFunction::new("upcase", Some(1), |args| {
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                other => Err(format!("upcase expects string, got {}", other.type_name())),
            }
        })),
    );

    // downcase(string) - Convert to lowercase
    env.define(
        "downcase".to_string(),
        Value::NativeFunction(NativeFunction::new("downcase", Some(1), |args| {
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                other => Err(format!("downcase expects string, got {}", other.type_name())),
            }
        })),
    );

    // trim(string) - Remove whitespace from both ends
    env.define(
        "trim".to_string(),
        Value::NativeFunction(NativeFunction::new("trim", Some(1), |args| {
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                other => Err(format!("trim expects string, got {}", other.type_name())),
            }
        })),
    );

    // html_escape(string) - Escape HTML special characters
    env.define(
        "html_escape".to_string(),
        Value::NativeFunction(NativeFunction::new("html_escape", Some(1), |args| {
            match &args[0] {
                Value::String(s) => {
                    let escaped = s
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                        .replace('"', "&quot;")
                        .replace('\'', "&#39;");
                    Ok(Value::String(escaped))
                }
                other => Err(format!("html_escape expects string, got {}", other.type_name())),
            }
        })),
    );

    // html_unescape(string) - Unescape HTML entities
    env.define(
        "html_unescape".to_string(),
        Value::NativeFunction(NativeFunction::new("html_unescape", Some(1), |args| {
            match &args[0] {
                Value::String(s) => {
                    let mut result = s.clone();
                    let replacements = [
                        ("&amp;", "&"),
                        ("&lt;", "<"),
                        ("&gt;", ">"),
                        ("&quot;", "\""),
                        ("&#39;", "'"),
                        ("&apos;", "'"),
                        ("&nbsp;", " "),
                    ];
                    for (from, to) in replacements {
                        result = result.replace(from, to);
                    }
                    Ok(Value::String(result))
                }
                other => Err(format!("html_unescape expects string, got {}", other.type_name())),
            }
        })),
    );

    // sanitize_html(string) - Remove dangerous HTML tags and attributes
    env.define(
        "sanitize_html".to_string(),
        Value::NativeFunction(NativeFunction::new("sanitize_html", Some(1), |args| {
            match &args[0] {
                Value::String(s) => {
                    let mut result = String::new();
                    let mut in_tag = false;
                    let mut tag_buffer = String::new();

                    for c in s.chars() {
                        if c == '<' {
                            in_tag = true;
                            tag_buffer.clear();
                            tag_buffer.push(c);
                        } else if c == '>' {
                            if in_tag {
                                tag_buffer.push(c);
                                let tag = tag_buffer.trim().to_lowercase();
                                let is_closing = tag.starts_with("</");
                                let is_self_closing = tag.ends_with("/>");
                                let tag_name = if is_closing {
                                    tag.trim_start_matches('<').trim_start_matches('/').trim_end_matches('>').split_whitespace().next().unwrap_or("")
                                } else {
                                    tag.trim_start_matches('<').trim_end_matches('/').trim_end_matches('>').split_whitespace().next().unwrap_or("")
                                };
                                let allowed_tags = ["p", "br", "b", "i", "u", "em", "strong", "a", "ul", "ol", "li", "blockquote", "code", "pre", "h1", "h2", "h3", "h4", "h5", "h6", "span", "div", "img"];
                                let is_allowed = allowed_tags.contains(&tag_name);
                                let is_dangerous_attr = tag.contains("javascript:") || tag.contains("onload=") || tag.contains("onerror=") || tag.contains("onclick=");
                                if is_allowed && !is_dangerous_attr {
                                    let cleaned_tag = if is_closing {
                                        format!("</{}>", tag_name)
                                    } else if is_self_closing {
                                        format!("<{}/>", tag_name)
                                    } else {
                                        let attrs: Vec<&str> = tag
                                            .strip_prefix('<')
                                            .and_then(|t| t.strip_suffix('>').or(Some(t)))
                                            .unwrap_or("")
                                            .split_whitespace()
                                            .skip(1)
                                            .collect();
                                        let safe_attrs = ["href", "src", "title", "alt", "class", "id", "style"];
                                        let safe_attrs_result: Vec<String> = attrs.iter().filter_map(|&attr| {
                                            let parts: Vec<&str> = attr.splitn(2, '=').collect();
                                            if parts.len() == 2 {
                                                let attr_name = parts[0].to_lowercase();
                                                let attr_value = parts[1].trim_matches('"').trim_matches('\'');
                                                if safe_attrs.contains(&attr_name.as_str()) && !attr_value.to_lowercase().contains("javascript:") {
                                                    Some(format!("{}={}", attr_name, parts[1]))
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        }).collect();
                                        if safe_attrs_result.is_empty() {
                                            format!("<{}>", tag_name)
                                        } else {
                                            format!("<{} {}>", tag_name, safe_attrs_result.join(" "))
                                        }
                                    };
                                    result.push_str(&cleaned_tag);
                                }
                                in_tag = false;
                            } else {
                                result.push(c);
                            }
                        } else if in_tag {
                            tag_buffer.push(c);
                        } else {
                            result.push(c);
                        }
                    }
                    if in_tag {
                        result.push_str(&tag_buffer);
                    }
                    Ok(Value::String(result))
                }
                other => Err(format!("sanitize_html expects string, got {}", other.type_name())),
            }
        })),
    );

    // Register HTTP client functions
    http::register_http_builtins(env);

    // Register HTTP server functions
    server::register_server_builtins(env);

    // Register WebSocket server functions
    server::register_websocket_builtins(env);

    // Register cryptographic functions
    crypto::register_crypto_builtins(env);

    // Register SoliDB functions
    solidb::register_solidb_builtins(env);

    // Register Model/ORM functions
    model::register_model_builtins(env);

    // Register dotenv functions
    dotenv::register_dotenv_builtins(env);

    // Register env functions
    env::register_env_builtins(env);

    // Register template functions
    template::register_template_builtins(env);

    // Register regex functions
    regex::register_regex_builtins(env);

    // Register router functions
    router::register_router_builtins(env);

    // Register controller functions
    controller::register_controller_builtins(env);

    // Register datetime functions (helper functions)
    datetime::register_datetime_builtins(env);

    // Register DateTime and Duration classes
    datetime_class::register_datetime_and_duration_classes(env);

    // Register I18n class
    i18n::register_i18n_class(env);

    // Register validation system (V class and validate function)
    validation::register_validation_builtins(env);

    // Register session management builtins
    session::register_session_builtins(env);

    // Register JWT builtins
    jwt::register_jwt_builtins(env);

    // Register factory builtins
    factories::register_factories(env);

    // Register assertion builtins
    assertions::register_assertions(env);

    // Register test DSL builtins
    test_dsl::register_test_builtins(env);

    // Register Error class and error types
    register_error_classes(env);
}

/// Register the Error class and built-in error types.
fn register_error_classes(env: &mut Environment) {
    use crate::interpreter::value::Class;

    // Create the Error base class
    let error_class = Class {
        name: "Error".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
    };
    env.define(
        "Error".to_string(),
        Value::Class(Rc::new(error_class)),
    );

    // ValueError class
    let value_error_class = Class {
        name: "ValueError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            constructor: None,
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
    };
    env.define(
        "ValueError".to_string(),
        Value::Class(Rc::new(value_error_class)),
    );

    // TypeError class
    let type_error_class = Class {
        name: "TypeError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            constructor: None,
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
    };
    env.define(
        "TypeError".to_string(),
        Value::Class(Rc::new(type_error_class)),
    );

    // KeyError class (for hash key not found)
    let key_error_class = Class {
        name: "KeyError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            constructor: None,
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
    };
    env.define(
        "KeyError".to_string(),
        Value::Class(Rc::new(key_error_class)),
    );

    // IndexError class (for array index out of bounds)
    let index_error_class = Class {
        name: "IndexError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            constructor: None,
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
    };
    env.define(
        "IndexError".to_string(),
        Value::Class(Rc::new(index_error_class)),
    );

    // RuntimeError class
    let runtime_error_class = Class {
        name: "RuntimeError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            constructor: None,
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        constructor: None,
    };
    env.define(
        "RuntimeError".to_string(),
        Value::Class(Rc::new(runtime_error_class)),
    );
}
