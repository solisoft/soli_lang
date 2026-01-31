//! Regex built-in class for Soli.
//!
//! Provides regex functionality with ReDoS protection via regex crate limits.
//! All methods are static: Regex.match(pattern, string), Regex.find(pattern, string), etc.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;
use regex::RegexBuilder;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};

/// Maximum regex complexity (nesting level) to prevent ReDoS.
const REGEX_NEST_LIMIT: u32 = 10;

/// Maximum size of the compiled regex in bytes.
const REGEX_SIZE_LIMIT: usize = 100_000;

/// Create a regex with safety limits to prevent ReDoS attacks.
fn create_safe_regex(pattern: &str) -> Result<regex::Regex, String> {
    RegexBuilder::new(pattern)
        .nest_limit(REGEX_NEST_LIMIT)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("Invalid regex pattern: {}", e))
}

pub fn register_regex_class(env: &mut Environment) {
    // Build static methods for Regex class
    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Regex.matches(pattern, string) - Check if string matches pattern
    static_methods.insert(
        "matches".to_string(),
        Rc::new(NativeFunction::new(
            "Regex.matches",
            Some(2),
            |args| match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    Ok(Value::Bool(re.is_match(s)))
                }
                _ => Err("Regex.matches requires (string, string)".to_string()),
            },
        )),
    );

    // Regex.find(pattern, string) - Find first match, return hash or null
    static_methods.insert(
        "find".to_string(),
        Rc::new(NativeFunction::new("Regex.find", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    if let Some(m) = re.find(s) {
                        let mut matches: IndexMap<HashKey, Value> = IndexMap::new();
                        matches.insert(
                            HashKey::String("match".to_string()),
                            Value::String(m.as_str().to_string()),
                        );
                        matches.insert(
                            HashKey::String("start".to_string()),
                            Value::Int(m.start() as i64),
                        );
                        matches.insert(
                            HashKey::String("end".to_string()),
                            Value::Int(m.end() as i64),
                        );
                        Ok(Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
                            matches,
                        ))))
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Err("Regex.find requires (string, string)".to_string()),
            }
        })),
    );

    // Regex.find_all(pattern, string) - Find all matches, return array
    static_methods.insert(
        "find_all".to_string(),
        Rc::new(NativeFunction::new(
            "Regex.find_all",
            Some(2),
            |args| match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    let matches: Vec<Value> = re
                        .find_iter(s)
                        .map(|m| {
                            let mut match_hash: IndexMap<HashKey, Value> = IndexMap::new();
                            match_hash.insert(
                                HashKey::String("match".to_string()),
                                Value::String(m.as_str().to_string()),
                            );
                            match_hash.insert(
                                HashKey::String("start".to_string()),
                                Value::Int(m.start() as i64),
                            );
                            match_hash.insert(
                                HashKey::String("end".to_string()),
                                Value::Int(m.end() as i64),
                            );
                            Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(match_hash)))
                        })
                        .collect();
                    Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(
                        matches,
                    ))))
                }
                _ => Err("Regex.find_all requires (string, string)".to_string()),
            },
        )),
    );

    // Regex.replace(pattern, string, replacement) - Replace first match
    static_methods.insert(
        "replace".to_string(),
        Rc::new(NativeFunction::new(
            "Regex.replace",
            Some(3),
            |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(pattern), Value::String(s), Value::String(replacement)) => {
                    let re = create_safe_regex(pattern)?;
                    let result = re.replace(s, replacement.as_str());
                    Ok(Value::String(result.to_string()))
                }
                _ => Err("Regex.replace requires (string, string, string)".to_string()),
            },
        )),
    );

    // Regex.replace_all(pattern, string, replacement) - Replace all matches
    static_methods.insert(
        "replace_all".to_string(),
        Rc::new(NativeFunction::new(
            "Regex.replace_all",
            Some(3),
            |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(pattern), Value::String(s), Value::String(replacement)) => {
                    let re = create_safe_regex(pattern)?;
                    let result = re.replace_all(s, replacement.as_str());
                    Ok(Value::String(result.to_string()))
                }
                _ => Err("Regex.replace_all requires (string, string, string)".to_string()),
            },
        )),
    );

    // Regex.split(pattern, string) - Split string by regex pattern
    static_methods.insert(
        "split".to_string(),
        Rc::new(NativeFunction::new("Regex.split", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    let parts: Vec<Value> =
                        re.split(s).map(|p| Value::String(p.to_string())).collect();
                    Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(
                        parts,
                    ))))
                }
                _ => Err("Regex.split requires (string, string)".to_string()),
            }
        })),
    );

    // Regex.capture(pattern, string) - Find first match with captures, return hash with named groups
    static_methods.insert(
        "capture".to_string(),
        Rc::new(NativeFunction::new(
            "Regex.capture",
            Some(2),
            |args| match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    if let Some(caps) = re.captures(s) {
                        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                        result.insert(
                            HashKey::String("match".to_string()),
                            Value::String(
                                caps.get(0)
                                    .map(|m| m.as_str().to_string())
                                    .unwrap_or_default(),
                            ),
                        );
                        result.insert(
                            HashKey::String("start".to_string()),
                            Value::Int(caps.get(0).map(|m| m.start() as i64).unwrap_or(-1)),
                        );
                        result.insert(
                            HashKey::String("end".to_string()),
                            Value::Int(caps.get(0).map(|m| m.end() as i64).unwrap_or(-1)),
                        );
                        for name in re.capture_names().flatten() {
                            if let Some(cap) = caps.name(name) {
                                result.insert(
                                    HashKey::String(name.to_string()),
                                    Value::String(cap.as_str().to_string()),
                                );
                            }
                        }
                        Ok(Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
                            result,
                        ))))
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Err("Regex.capture requires (string, string)".to_string()),
            },
        )),
    );

    // Regex.escape(string) - Escape special regex characters
    static_methods.insert(
        "escape".to_string(),
        Rc::new(NativeFunction::new(
            "Regex.escape",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => {
                    let escaped = regex::escape(s);
                    Ok(Value::String(escaped))
                }
                _ => Err("Regex.escape requires (string)".to_string()),
            },
        )),
    );

    // Create Regex class with only static methods (no instance methods)
    let regex_class = Class {
        name: "Regex".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    };

    env.define("Regex".to_string(), Value::Class(Rc::new(regex_class)));
}
