//! Native string method dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Dispatch a string method call. The string and args are already available.
    /// Returns the result value.
    pub fn vm_call_string_method(
        &self,
        s: &str,
        name: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match name {
            // --- Zero-arg methods ---
            "upcase" | "uppercase" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.to_uppercase()))
            }
            "downcase" | "lowercase" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.to_lowercase()))
            }
            "len" | "length" | "size" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::Int(s.len() as i64))
            }
            "trim" | "strip" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.trim().to_string()))
            }
            "lstrip" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.trim_start().to_string()))
            }
            "rstrip" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.trim_end().to_string()))
            }
            "capitalize" => {
                check_arity(0, args.len(), span)?;
                let mut chars = s.chars();
                let result: String = match chars.next() {
                    None => String::new(),
                    Some(first) => {
                        first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                    }
                };
                Ok(Value::String(result))
            }
            "swapcase" => {
                check_arity(0, args.len(), span)?;
                let mut result = String::with_capacity(s.len());
                for c in s.chars() {
                    if c.is_uppercase() {
                        for lc in c.to_lowercase() {
                            result.push(lc);
                        }
                    } else {
                        for uc in c.to_uppercase() {
                            result.push(uc);
                        }
                    }
                }
                Ok(Value::String(result))
            }
            "chomp" => {
                check_arity(0, args.len(), span)?;
                let result = s
                    .strip_suffix('\n')
                    .or_else(|| s.strip_suffix("\r\n"))
                    .or_else(|| s.strip_suffix('\r'))
                    .unwrap_or(s);
                Ok(Value::String(result.to_string()))
            }
            "squeeze" => {
                if args.len() > 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let chars_to_squeeze: Option<Vec<char>> = args.first().map(|v| match v {
                    Value::String(s) => s.chars().collect(),
                    _ => vec![],
                });
                let mut result = String::with_capacity(s.len());
                let mut last_char: Option<char> = None;
                for c in s.chars() {
                    let should_squeeze = chars_to_squeeze
                        .as_ref()
                        .map(|chars| chars.contains(&c))
                        .unwrap_or(true);
                    if should_squeeze {
                        if last_char != Some(c) {
                            result.push(c);
                        }
                    } else {
                        result.push(c);
                    }
                    last_char = Some(c);
                }
                Ok(Value::String(result))
            }
            "reverse" => {
                check_arity(0, args.len(), span)?;
                let result: String = s.chars().rev().collect();
                Ok(Value::String(result))
            }
            "chars" => {
                check_arity(0, args.len(), span)?;
                let chars: Vec<Value> = s.chars().map(|c| Value::String(c.to_string())).collect();
                Ok(Value::Array(Rc::new(RefCell::new(chars))))
            }
            "bytes" => {
                check_arity(0, args.len(), span)?;
                let bytes: Vec<Value> = s.bytes().map(|b| Value::Int(b as i64)).collect();
                Ok(Value::Array(Rc::new(RefCell::new(bytes))))
            }
            "lines" => {
                check_arity(0, args.len(), span)?;
                let lines: Vec<Value> = s.lines().map(|l| Value::String(l.to_string())).collect();
                Ok(Value::Array(Rc::new(RefCell::new(lines))))
            }
            "bytesize" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::Int(s.len() as i64))
            }
            "empty?" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::Bool(s.is_empty()))
            }
            "hex" => {
                check_arity(0, args.len(), span)?;
                let result = i64::from_str_radix(s, 16)
                    .map_err(|e| RuntimeError::type_error(format!("invalid hex: {}", e), span))?;
                Ok(Value::Int(result))
            }
            "oct" => {
                check_arity(0, args.len(), span)?;
                let result = i64::from_str_radix(s, 8)
                    .map_err(|e| RuntimeError::type_error(format!("invalid octal: {}", e), span))?;
                Ok(Value::Int(result))
            }
            "to_s" | "to_string" => Ok(Value::String(s.to_string())),
            "to_i" | "to_int" => {
                let trimmed = s.trim();
                Ok(Value::Int(
                    trimmed
                        .parse::<i64>()
                        .or_else(|_| trimmed.replace(',', ".").parse::<f64>().map(|f| f as i64))
                        .unwrap_or(0),
                ))
            }
            "to_f" | "to_float" => {
                let trimmed = s.trim();
                Ok(Value::Float(
                    trimmed
                        .parse::<f64>()
                        .or_else(|_| trimmed.replace(',', ".").parse::<f64>())
                        .unwrap_or(0.0),
                ))
            }
            "ord" => {
                check_arity(0, args.len(), span)?;
                if let Some(c) = s.chars().next() {
                    Ok(Value::Int(c as i64))
                } else {
                    Err(RuntimeError::type_error("ord on empty string", span))
                }
            }

            // --- One-arg methods ---
            "contains" => {
                check_arity(1, args.len(), span)?;
                let sub = expect_string(&args[0], "contains", span)?;
                Ok(Value::Bool(s.contains(sub)))
            }
            "starts_with" | "starts_with?" => {
                check_arity(1, args.len(), span)?;
                let prefix = expect_string(&args[0], "starts_with", span)?;
                Ok(Value::Bool(s.starts_with(prefix)))
            }
            "ends_with" | "ends_with?" => {
                check_arity(1, args.len(), span)?;
                let suffix = expect_string(&args[0], "ends_with", span)?;
                Ok(Value::Bool(s.ends_with(suffix)))
            }
            "include?" => {
                check_arity(1, args.len(), span)?;
                let sub = expect_string(&args[0], "include?", span)?;
                Ok(Value::Bool(s.contains(sub)))
            }
            "split" => {
                check_arity(1, args.len(), span)?;
                let delim = expect_string(&args[0], "split", span)?;
                let parts: Vec<Value> = s
                    .split(delim)
                    .map(|p| Value::String(p.to_string()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(parts))))
            }
            "index_of" => {
                check_arity(1, args.len(), span)?;
                let sub = expect_string(&args[0], "index_of", span)?;
                if let Some(idx) = s.find(sub) {
                    Ok(Value::Int(idx as i64))
                } else {
                    Ok(Value::Int(-1))
                }
            }
            "count" => {
                check_arity(1, args.len(), span)?;
                let sub = expect_string(&args[0], "count", span)?;
                Ok(Value::Int(s.matches(sub).count() as i64))
            }
            "delete" => {
                check_arity(1, args.len(), span)?;
                let to_delete = expect_string(&args[0], "delete", span)?;
                Ok(Value::String(s.replace(to_delete, "")))
            }
            "delete_prefix" => {
                check_arity(1, args.len(), span)?;
                let prefix = expect_string(&args[0], "delete_prefix", span)?;
                Ok(Value::String(
                    s.strip_prefix(prefix).unwrap_or(s).to_string(),
                ))
            }
            "delete_suffix" => {
                check_arity(1, args.len(), span)?;
                let suffix = expect_string(&args[0], "delete_suffix", span)?;
                Ok(Value::String(
                    s.strip_suffix(suffix).unwrap_or(s).to_string(),
                ))
            }
            "partition" => {
                check_arity(1, args.len(), span)?;
                let sep = expect_string(&args[0], "partition", span)?;
                if let Some(pos) = s.find(sep) {
                    let result = vec![
                        Value::String(s[..pos].to_string()),
                        Value::String(sep.to_string()),
                        Value::String(s[pos + sep.len()..].to_string()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                } else {
                    let result = vec![
                        Value::String(s.to_string()),
                        Value::String(String::new()),
                        Value::String(String::new()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
            }
            "rpartition" => {
                check_arity(1, args.len(), span)?;
                let sep = expect_string(&args[0], "rpartition", span)?;
                if let Some(pos) = s.rfind(sep) {
                    let result = vec![
                        Value::String(s[..pos].to_string()),
                        Value::String(sep.to_string()),
                        Value::String(s[pos + sep.len()..].to_string()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                } else {
                    let result = vec![
                        Value::String(String::new()),
                        Value::String(String::new()),
                        Value::String(s.to_string()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
            }

            // --- Two-arg methods ---
            "replace" => {
                check_arity(2, args.len(), span)?;
                let from = expect_string(&args[0], "replace from", span)?;
                let to = expect_string(&args[1], "replace to", span)?;
                Ok(Value::String(s.replace(from, to)))
            }
            "gsub" => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(RuntimeError::wrong_arity(2, args.len(), span));
                }
                let pattern = expect_string(&args[0], "gsub pattern", span)?;
                let replacement = expect_string(&args[1], "gsub replacement", span)?;
                // Try simple string replacement first (faster than regex)
                if !has_regex_metacharacters(pattern) {
                    if args.len() == 3 {
                        let limit = match &args[2] {
                            Value::Int(n) if *n >= 0 => *n as usize,
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "gsub limit must be a non-negative integer",
                                    span,
                                ))
                            }
                        };
                        Ok(Value::String(replacen_str(s, pattern, replacement, limit)))
                    } else {
                        Ok(Value::String(s.replace(pattern, replacement)))
                    }
                } else {
                    let re = crate::regex_cache::get_regex(pattern)
                        .map_err(|e| RuntimeError::type_error(e, span))?;
                    if args.len() == 3 {
                        let limit = match &args[2] {
                            Value::Int(n) if *n >= 0 => *n as usize,
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "gsub limit must be a non-negative integer",
                                    span,
                                ))
                            }
                        };
                        Ok(Value::String(
                            re.replacen(s, limit, replacement).to_string(),
                        ))
                    } else {
                        Ok(Value::String(re.replace_all(s, replacement).to_string()))
                    }
                }
            }
            "sub" => {
                check_arity(2, args.len(), span)?;
                let pattern = expect_string(&args[0], "sub pattern", span)?;
                let replacement = expect_string(&args[1], "sub replacement", span)?;
                if !has_regex_metacharacters(pattern) {
                    Ok(Value::String(replacen_str(s, pattern, replacement, 1)))
                } else {
                    let re = crate::regex_cache::get_regex(pattern)
                        .map_err(|e| RuntimeError::type_error(e, span))?;
                    Ok(Value::String(re.replacen(s, 1, replacement).to_string()))
                }
            }
            "tr" => {
                check_arity(2, args.len(), span)?;
                let from_chars = expect_string(&args[0], "tr from", span)?;
                let to_chars = expect_string(&args[1], "tr to", span)?;
                let from_vec: Vec<char> = from_chars.chars().collect();
                let to_vec: Vec<char> = to_chars.chars().collect();
                let mut result = String::with_capacity(s.len());
                for c in s.chars() {
                    if let Some(pos) = from_vec.iter().position(|&fc| fc == c) {
                        if let Some(&replacement) = to_vec.get(pos) {
                            result.push(replacement);
                        }
                    } else {
                        result.push(c);
                    }
                }
                Ok(Value::String(result))
            }
            "substring" => {
                check_arity(2, args.len(), span)?;
                let start = match &args[0] {
                    Value::Int(i) => *i,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "substring expects integer start",
                            span,
                        ))
                    }
                };
                let end = match &args[1] {
                    Value::Int(i) => *i,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "substring expects integer end",
                            span,
                        ))
                    }
                };
                let start_usize = if start < 0 { 0 } else { start as usize };
                let end_usize = if end > s.len() as i64 {
                    s.len()
                } else {
                    end as usize
                };
                if start_usize >= end_usize || start_usize >= s.len() {
                    Ok(Value::String(String::new()))
                } else {
                    Ok(Value::String(s[start_usize..end_usize].to_string()))
                }
            }
            "insert" => {
                check_arity(2, args.len(), span)?;
                let index = match &args[0] {
                    Value::Int(i) if *i >= 0 => *i as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "insert expects a non-negative integer index",
                            span,
                        ))
                    }
                };
                let insert_str = expect_string(&args[1], "insert string", span)?;
                let char_count = s.chars().count();
                if index > char_count {
                    return Err(RuntimeError::type_error("insert index out of bounds", span));
                }
                let mut result = String::new();
                for (i, c) in s.chars().enumerate() {
                    if i == index {
                        result.push_str(insert_str);
                    }
                    result.push(c);
                }
                if index == char_count {
                    result.push_str(insert_str);
                }
                Ok(Value::String(result))
            }

            // --- Variable-arg methods ---
            "center" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let width = expect_positive_int(&args[0], "center", span)?;
                let pad_char = args
                    .get(1)
                    .and_then(|v| match v {
                        Value::String(s) => s.chars().next(),
                        _ => None,
                    })
                    .unwrap_or(' ');
                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let total_pad = width - s.len();
                    let left_pad = total_pad / 2;
                    let right_pad = total_pad - left_pad;
                    let mut result = String::with_capacity(width);
                    for _ in 0..left_pad {
                        result.push(pad_char);
                    }
                    result.push_str(s);
                    for _ in 0..right_pad {
                        result.push(pad_char);
                    }
                    Ok(Value::String(result))
                }
            }
            "ljust" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let width = expect_positive_int(&args[0], "ljust", span)?;
                let pad_char = args
                    .get(1)
                    .and_then(|v| match v {
                        Value::String(s) => s.chars().next(),
                        _ => None,
                    })
                    .unwrap_or(' ');
                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let mut result = String::with_capacity(width);
                    result.push_str(s);
                    for _ in 0..(width - s.len()) {
                        result.push(pad_char);
                    }
                    Ok(Value::String(result))
                }
            }
            "rjust" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let width = expect_positive_int(&args[0], "rjust", span)?;
                let pad_char = args
                    .get(1)
                    .and_then(|v| match v {
                        Value::String(s) => s.chars().next(),
                        _ => None,
                    })
                    .unwrap_or(' ');
                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let mut result = String::with_capacity(width);
                    for _ in 0..(width - s.len()) {
                        result.push(pad_char);
                    }
                    result.push_str(s);
                    Ok(Value::String(result))
                }
            }
            "lpad" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let width = match &args[0] {
                    Value::Int(w) if *w >= 0 => *w as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "lpad expects non-negative integer width",
                            span,
                        ))
                    }
                };
                let pad_char = args
                    .get(1)
                    .and_then(|v| match v {
                        Value::String(s) => s.chars().next(),
                        _ => None,
                    })
                    .unwrap_or(' ');
                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let padding = width - s.len();
                    let mut result = String::with_capacity(width);
                    for _ in 0..padding {
                        result.push(pad_char);
                    }
                    result.push_str(s);
                    Ok(Value::String(result))
                }
            }
            "rpad" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let width = match &args[0] {
                    Value::Int(w) if *w >= 0 => *w as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "rpad expects non-negative integer width",
                            span,
                        ))
                    }
                };
                let pad_char = args
                    .get(1)
                    .and_then(|v| match v {
                        Value::String(s) => s.chars().next(),
                        _ => None,
                    })
                    .unwrap_or(' ');
                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let padding = width - s.len();
                    let mut result = String::with_capacity(width);
                    result.push_str(s);
                    for _ in 0..padding {
                        result.push(pad_char);
                    }
                    Ok(Value::String(result))
                }
            }
            "truncate" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let length = expect_positive_int(&args[0], "truncate", span)?;
                let suffix = args
                    .get(1)
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or("...");
                if s.len() <= length {
                    Ok(Value::String(s.to_string()))
                } else {
                    let result = &s[..length.saturating_sub(suffix.len())];
                    Ok(Value::String(format!("{}{}", result, suffix)))
                }
            }
            "match" => {
                check_arity(1, args.len(), span)?;
                let pattern = expect_string(&args[0], "match", span)?;
                let re = crate::regex_cache::get_regex(pattern)
                    .map_err(|e| RuntimeError::type_error(e, span))?;
                if let Some(captures) = re.captures(s) {
                    let mut result = Vec::new();
                    for i in 0..captures.len() {
                        if let Some(m) = captures.get(i) {
                            result.push(Value::String(m.as_str().to_string()));
                        }
                    }
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                } else {
                    Ok(Value::Null)
                }
            }
            "scan" => {
                check_arity(1, args.len(), span)?;
                let pattern = expect_string(&args[0], "scan", span)?;
                let re = crate::regex_cache::get_regex(pattern)
                    .map_err(|e| RuntimeError::type_error(e, span))?;
                let matches: Vec<Value> = re
                    .find_iter(s)
                    .map(|m| Value::String(m.as_str().to_string()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(matches))))
            }
            "join" => Ok(Value::String(s.to_string())),
            "to_sym" => Ok(Value::Symbol(s.to_string())),
            "parse_json" => match crate::interpreter::value::parse_json(s) {
                Ok(value) => Ok(value),
                Err(_) => Ok(Value::Hash(Rc::new(RefCell::new(
                    indexmap::IndexMap::with_hasher(ahash::RandomState::new()),
                )))),
            },
            "is_a?" => {
                check_arity(1, args.len(), span)?;
                let class_name = match &args[0] {
                    Value::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "is_a? expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(
                    class_name == "string" || class_name == "object",
                ))
            }
            // Universal methods
            "class" => Ok(Value::String("string".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(s.trim().is_empty())),
            "present?" => Ok(Value::Bool(!s.trim().is_empty())),
            "inspect" => Ok(Value::String(format!("\"{}\"", s))),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "String".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }
}

#[inline]
fn check_arity(expected: usize, got: usize, span: Span) -> Result<(), RuntimeError> {
    if got != expected {
        Err(RuntimeError::wrong_arity(expected, got, span))
    } else {
        Ok(())
    }
}

#[inline]
fn expect_string<'a>(val: &'a Value, method: &str, span: Span) -> Result<&'a str, RuntimeError> {
    match val {
        Value::String(s) => Ok(s.as_str()),
        _ => Err(RuntimeError::type_error(
            format!("{} expects a string argument", method),
            span,
        )),
    }
}

#[inline]
fn expect_positive_int(val: &Value, method: &str, span: Span) -> Result<usize, RuntimeError> {
    match val {
        Value::Int(w) if *w > 0 => Ok(*w as usize),
        _ => Err(RuntimeError::type_error(
            format!("{} expects a positive integer", method),
            span,
        )),
    }
}

/// Check if a pattern contains regex metacharacters.
#[inline]
fn has_regex_metacharacters(pattern: &str) -> bool {
    pattern.bytes().any(|b| {
        matches!(
            b,
            b'.' | b'*'
                | b'+'
                | b'?'
                | b'('
                | b')'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b'\\'
                | b'^'
                | b'$'
                | b'|'
        )
    })
}

/// String replacen without regex.
fn replacen_str(s: &str, pattern: &str, replacement: &str, limit: usize) -> String {
    let mut result = String::with_capacity(s.len());
    let mut remaining = s;
    let mut count = 0;
    while count < limit {
        if let Some(pos) = remaining.find(pattern) {
            result.push_str(&remaining[..pos]);
            result.push_str(replacement);
            remaining = &remaining[pos + pattern.len()..];
            count += 1;
        } else {
            break;
        }
    }
    result.push_str(remaining);
    result
}
