//! Regex built-in functions for Soli.
//!
//! Provides regex functions with ReDoS protection via regex crate limits.

use regex::RegexBuilder;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::Value;

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

pub fn register_regex_builtins(env: &mut Environment) {
    // regex_match(pattern, string) - Check if string matches pattern
    env.define(
        "regex_match".to_string(),
        Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "regex_match",
            Some(2),
            |args| match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    Ok(Value::Bool(re.is_match(s)))
                }
                _ => Err("regex_match requires (string, string)".to_string()),
            },
        )),
    );

    // regex_find(pattern, string) - Find first match, return hash or null
    env.define(
        "regex_find".to_string(),
        Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "regex_find",
            Some(2),
            |args| match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    if let Some(m) = re.find(s) {
                        let matches: Vec<(Value, Value)> = vec![
                            (
                                Value::String("match".to_string()),
                                Value::String(m.as_str().to_string()),
                            ),
                            (
                                Value::String("start".to_string()),
                                Value::Int(m.start() as i64),
                            ),
                            (Value::String("end".to_string()), Value::Int(m.end() as i64)),
                        ];
                        Ok(Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
                            matches,
                        ))))
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Err("regex_find requires (string, string)".to_string()),
            },
        )),
    );

    // regex_find_all(pattern, string) - Find all matches, return array
    env.define(
        "regex_find_all".to_string(),
        Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "regex_find_all",
            Some(2),
            |args| match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    let matches: Vec<Value> = re
                        .find_iter(s)
                        .map(|m| {
                            let match_hash: Vec<(Value, Value)> = vec![
                                (
                                    Value::String("match".to_string()),
                                    Value::String(m.as_str().to_string()),
                                ),
                                (
                                    Value::String("start".to_string()),
                                    Value::Int(m.start() as i64),
                                ),
                                (Value::String("end".to_string()), Value::Int(m.end() as i64)),
                            ];
                            Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(match_hash)))
                        })
                        .collect();
                    Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(
                        matches,
                    ))))
                }
                _ => Err("regex_find_all requires (string, string)".to_string()),
            },
        )),
    );

    // regex_replace(pattern, string, replacement) - Replace first match
    env.define(
        "regex_replace".to_string(),
        Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "regex_replace",
            Some(3),
            |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(pattern), Value::String(s), Value::String(replacement)) => {
                    let re = create_safe_regex(pattern)?;
                    let result = re.replace(s, replacement.as_str());
                    Ok(Value::String(result.to_string()))
                }
                _ => Err("regex_replace requires (string, string, string)".to_string()),
            },
        )),
    );

    // regex_replace_all(pattern, string, replacement) - Replace all matches
    env.define(
        "regex_replace_all".to_string(),
        Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "regex_replace_all",
            Some(3),
            |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(pattern), Value::String(s), Value::String(replacement)) => {
                    let re = create_safe_regex(pattern)?;
                    let result = re.replace_all(s, replacement.as_str());
                    Ok(Value::String(result.to_string()))
                }
                _ => Err("regex_replace_all requires (string, string, string)".to_string()),
            },
        )),
    );

    // regex_split(pattern, string) - Split string by regex pattern
    env.define(
        "regex_split".to_string(),
        Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "regex_split",
            Some(2),
            |args| match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    let parts: Vec<Value> =
                        re.split(s).map(|p| Value::String(p.to_string())).collect();
                    Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(
                        parts,
                    ))))
                }
                _ => Err("regex_split requires (string, string)".to_string()),
            },
        )),
    );

    // regex_capture(pattern, string) - Find first match with captures, return hash with named groups
    env.define(
        "regex_capture".to_string(),
        Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "regex_capture",
            Some(2),
            |args| match (&args[0], &args[1]) {
                (Value::String(pattern), Value::String(s)) => {
                    let re = create_safe_regex(pattern)?;
                    if let Some(caps) = re.captures(s) {
                        let mut result: Vec<(Value, Value)> = vec![
                            (
                                Value::String("match".to_string()),
                                Value::String(
                                    caps.get(0)
                                        .map(|m| m.as_str().to_string())
                                        .unwrap_or_default(),
                                ),
                            ),
                            (
                                Value::String("start".to_string()),
                                Value::Int(caps.get(0).map(|m| m.start() as i64).unwrap_or(-1)),
                            ),
                            (
                                Value::String("end".to_string()),
                                Value::Int(caps.get(0).map(|m| m.end() as i64).unwrap_or(-1)),
                            ),
                        ];
                        for name in re.capture_names().flatten() {
                            if let Some(cap) = caps.name(name) {
                                result.push((
                                    Value::String(name.to_string()),
                                    Value::String(cap.as_str().to_string()),
                                ));
                            }
                        }
                        Ok(Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
                            result,
                        ))))
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Err("regex_capture requires (string, string)".to_string()),
            },
        )),
    );

    // regex_escape(string) - Escape special regex characters
    env.define(
        "regex_escape".to_string(),
        Value::NativeFunction(crate::interpreter::value::NativeFunction::new(
            "regex_escape",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => {
                    let escaped = regex::escape(s);
                    Ok(Value::String(escaped))
                }
                _ => Err("regex_escape requires (string)".to_string()),
            },
        )),
    );
}
