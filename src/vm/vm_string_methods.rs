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
    #[inline]
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
                Ok(Value::String(s.to_uppercase().into()))
            }
            "downcase" | "lowercase" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.to_lowercase().into()))
            }
            "len" | "length" | "size" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::Int(s.len() as i64))
            }
            "trim" | "strip" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.trim().to_string().into()))
            }
            "lstrip" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.trim_start().to_string().into()))
            }
            "rstrip" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(s.trim_end().to_string().into()))
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
                Ok(Value::String(result.into()))
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
                Ok(Value::String(result.into()))
            }
            "chomp" => {
                check_arity(0, args.len(), span)?;
                let result = s
                    .strip_suffix('\n')
                    .or_else(|| s.strip_suffix("\r\n"))
                    .or_else(|| s.strip_suffix('\r'))
                    .unwrap_or(s);
                Ok(Value::String(result.to_string().into()))
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
                Ok(Value::String(result.into()))
            }
            "reverse" => {
                check_arity(0, args.len(), span)?;
                let result: String = s.chars().rev().collect();
                Ok(Value::String(result.into()))
            }
            "chars" => {
                check_arity(0, args.len(), span)?;
                let mut chars = Vec::with_capacity(s.len());
                for c in s.chars() {
                    chars.push(Value::String(c.to_string().into()));
                }
                Ok(Value::Array(Rc::new(RefCell::new(chars))))
            }
            "bytes" => {
                check_arity(0, args.len(), span)?;
                let bytes: Vec<Value> = s.bytes().map(|b| Value::Int(b as i64)).collect();
                Ok(Value::Array(Rc::new(RefCell::new(bytes))))
            }
            "lines" => {
                check_arity(0, args.len(), span)?;
                let mut lines = Vec::with_capacity(s.bytes().filter(|b| *b == b'\n').count() + 1);
                for line in s.lines() {
                    lines.push(Value::String(line.to_string().into()));
                }
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
            "to_s" | "to_string" => Ok(Value::String(s.to_string().into())),
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
            "camelize" => {
                if args.len() > 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let upper = match args.first() {
                    None => false,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => {
                        return Err(RuntimeError::type_error(
                            "camelize expects a boolean argument (true for PascalCase)",
                            span,
                        ))
                    }
                };
                Ok(Value::String(camelize_string(s, upper).into()))
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
            "include?" | "includes?" => {
                check_arity(1, args.len(), span)?;
                let sub = expect_string(&args[0], "include?", span)?;
                Ok(Value::Bool(s.contains(sub)))
            }
            "slugify" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(
                    crate::interpreter::executor::calls::string_methods::slugify_string(s).into(),
                ))
            }
            "split" => {
                if args.len() > 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let delim: &str = if args.is_empty() {
                    " "
                } else {
                    expect_string(&args[0], "split", span)?
                };
                let capacity = if delim.is_empty() {
                    s.len() + 1
                } else {
                    s.matches(delim).count() + 1
                };
                let mut parts = Vec::with_capacity(capacity);
                for part in s.split(delim) {
                    parts.push(Value::String(part.to_string().into()));
                }
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
                Ok(Value::String(s.replace(to_delete, "").into()))
            }
            "delete_prefix" => {
                check_arity(1, args.len(), span)?;
                let prefix = expect_string(&args[0], "delete_prefix", span)?;
                Ok(Value::String(
                    s.strip_prefix(prefix).unwrap_or(s).to_string().into(),
                ))
            }
            "delete_suffix" => {
                check_arity(1, args.len(), span)?;
                let suffix = expect_string(&args[0], "delete_suffix", span)?;
                Ok(Value::String(
                    s.strip_suffix(suffix).unwrap_or(s).to_string().into(),
                ))
            }
            "partition" => {
                check_arity(1, args.len(), span)?;
                let sep = expect_string(&args[0], "partition", span)?;
                if let Some(pos) = s.find(sep) {
                    let result = vec![
                        Value::String(s[..pos].to_string().into()),
                        Value::String(sep.to_string().into()),
                        Value::String(s[pos + sep.len()..].to_string().into()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                } else {
                    let result = vec![
                        Value::String(s.to_string().into()),
                        Value::String(String::new().into()),
                        Value::String(String::new().into()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
            }
            "rpartition" => {
                check_arity(1, args.len(), span)?;
                let sep = expect_string(&args[0], "rpartition", span)?;
                if let Some(pos) = s.rfind(sep) {
                    let result = vec![
                        Value::String(s[..pos].to_string().into()),
                        Value::String(sep.to_string().into()),
                        Value::String(s[pos + sep.len()..].to_string().into()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                } else {
                    let result = vec![
                        Value::String(String::new().into()),
                        Value::String(String::new().into()),
                        Value::String(s.to_string().into()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
            }

            // --- Two-arg methods ---
            "replace" => {
                check_arity(2, args.len(), span)?;
                let from = expect_string(&args[0], "replace from", span)?;
                let to = expect_string(&args[1], "replace to", span)?;
                Ok(Value::String(s.replace(from, to).into()))
            }
            "gsub" | "replace_all" => {
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
                        Ok(Value::String(
                            replacen_str(s, pattern, replacement, limit).into(),
                        ))
                    } else {
                        Ok(Value::String(s.replace(pattern, replacement).into()))
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
                            re.replacen(s, limit, replacement).to_string().into(),
                        ))
                    } else {
                        Ok(Value::String(
                            re.replace_all(s, replacement).to_string().into(),
                        ))
                    }
                }
            }
            "sub" => {
                check_arity(2, args.len(), span)?;
                let pattern = expect_string(&args[0], "sub pattern", span)?;
                let replacement = expect_string(&args[1], "sub replacement", span)?;
                if !has_regex_metacharacters(pattern) {
                    Ok(Value::String(
                        replacen_str(s, pattern, replacement, 1).into(),
                    ))
                } else {
                    let re = crate::regex_cache::get_regex(pattern)
                        .map_err(|e| RuntimeError::type_error(e, span))?;
                    Ok(Value::String(
                        re.replacen(s, 1, replacement).to_string().into(),
                    ))
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
                Ok(Value::String(result.into()))
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
                    Ok(Value::String(String::new().into()))
                } else {
                    Ok(Value::String(s[start_usize..end_usize].to_string().into()))
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
                Ok(Value::String(result.into()))
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
                    Ok(Value::String(s.to_string().into()))
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
                    Ok(Value::String(result.into()))
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
                    Ok(Value::String(s.to_string().into()))
                } else {
                    let mut result = String::with_capacity(width);
                    result.push_str(s);
                    for _ in 0..(width - s.len()) {
                        result.push(pad_char);
                    }
                    Ok(Value::String(result.into()))
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
                    Ok(Value::String(s.to_string().into()))
                } else {
                    let mut result = String::with_capacity(width);
                    for _ in 0..(width - s.len()) {
                        result.push(pad_char);
                    }
                    result.push_str(s);
                    Ok(Value::String(result.into()))
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
                    Ok(Value::String(s.to_string().into()))
                } else {
                    let padding = width - s.len();
                    let mut result = String::with_capacity(width);
                    for _ in 0..padding {
                        result.push(pad_char);
                    }
                    result.push_str(s);
                    Ok(Value::String(result.into()))
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
                    Ok(Value::String(s.to_string().into()))
                } else {
                    let padding = width - s.len();
                    let mut result = String::with_capacity(width);
                    result.push_str(s);
                    for _ in 0..padding {
                        result.push(pad_char);
                    }
                    Ok(Value::String(result.into()))
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
                        Value::String(s) => Some(s.as_ref()),
                        _ => None,
                    })
                    .unwrap_or("...");
                if s.len() <= length {
                    Ok(Value::String(s.to_string().into()))
                } else {
                    let result = &s[..length.saturating_sub(suffix.len())];
                    Ok(Value::String(format!("{}{}", result, suffix).into()))
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
                            result.push(Value::String(m.as_str().to_string().into()));
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
                    .map(|m| Value::String(m.as_str().to_string().into()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(matches))))
            }
            "join" => Ok(Value::String(s.to_string().into())),
            "to_sym" => Ok(Value::Symbol(s.to_string().into())),
            "parse_json" => match crate::interpreter::value::parse_json(s) {
                Ok(value) => Ok(value),
                Err(_) => Ok(Value::Hash(Rc::new(RefCell::new(
                    indexmap::IndexMap::with_hasher(ahash::RandomState::new()),
                )))),
            },
            // Parse JSON and only return a Hash; null when the input isn't
            // valid JSON or parses to a non-object (array, scalar, ...).
            "to_h" => {
                check_arity(0, args.len(), span)?;
                match crate::interpreter::value::parse_json(s) {
                    Ok(Value::Hash(h)) => Ok(Value::Hash(h)),
                    _ => Ok(Value::Null),
                }
            }
            "is_a?" => {
                check_arity(1, args.len(), span)?;
                let class_name = match &args[0] {
                    Value::String(s) => s.as_ref(),
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
            "casecmp" => {
                check_arity(1, args.len(), span)?;
                let other = expect_string(&args[0], "casecmp", span)?;
                use std::cmp::Ordering;
                Ok(Value::Int(
                    match s.to_lowercase().cmp(&other.to_lowercase()) {
                        Ordering::Less => -1,
                        Ordering::Equal => 0,
                        Ordering::Greater => 1,
                    },
                ))
            }
            "casecmp?" => {
                check_arity(1, args.len(), span)?;
                let other = expect_string(&args[0], "casecmp?", span)?;
                Ok(Value::Bool(s.to_lowercase() == other.to_lowercase()))
            }
            "prepend" => {
                check_arity(1, args.len(), span)?;
                let other = expect_string(&args[0], "prepend", span)?;
                let mut result = other.to_string();
                result.push_str(s);
                Ok(Value::String(result.into()))
            }
            "chop" => {
                check_arity(0, args.len(), span)?;
                let mut chars: Vec<char> = s.chars().collect();
                chars.pop();
                let out: String = chars.into_iter().collect();
                Ok(Value::String(out.into()))
            }
            "ascii_only?" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::Bool(s.is_ascii()))
            }
            "succ" | "next" => {
                check_arity(0, args.len(), span)?;
                Ok(Value::String(string_succ(s).into()))
            }
            // Universal methods
            "class" => Ok(Value::String("string".into())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(s.trim().is_empty())),
            "present?" => Ok(Value::Bool(!s.trim().is_empty())),
            "inspect" => Ok(Value::String(format!("\"{}\"", s).into())),
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
        Value::String(s) => Ok(s.as_ref()),
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

/// Increment a string like Ruby's `String#succ`.
fn string_succ(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return s.to_string();
    }

    let mut end = chars.len();
    while end > 0 {
        end -= 1;
        if chars[end].is_alphanumeric() {
            break;
        }
    }
    if !chars[end].is_alphanumeric() {
        return s.to_string();
    }

    let mut start = end;
    while start > 0 && chars[start - 1].is_alphanumeric() {
        start -= 1;
    }

    let mut result: Vec<char> = chars.clone();
    let mut carry = true;
    let mut j = end;
    loop {
        if !carry || j < start {
            break;
        }
        let c = result[j];
        if c.is_ascii_digit() {
            if c == '9' {
                result[j] = '0';
            } else {
                result[j] = (c as u8 + 1) as char;
                carry = false;
            }
        } else if c.is_ascii_lowercase() {
            if c == 'z' {
                result[j] = 'a';
            } else {
                result[j] = (c as u8 + 1) as char;
                carry = false;
            }
        } else if c.is_ascii_uppercase() {
            if c == 'Z' {
                result[j] = 'A';
            } else {
                result[j] = (c as u8 + 1) as char;
                carry = false;
            }
        }
        if j > start {
            j -= 1;
        } else {
            break;
        }
    }

    if carry {
        let first = chars[start];
        let new = if first.is_ascii_digit() {
            '1'
        } else if first.is_ascii_lowercase() {
            'a'
        } else {
            'A'
        };
        result.insert(start, new);
    }

    result.into_iter().collect()
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

/// Convert `snake_case` / `kebab-case` input to camel case. With `upper=false`
/// the first emitted char is lowercased (`fooBar`); with `upper=true` it is
/// uppercased (`FooBar`). Leading and consecutive separators are collapsed,
/// internal capitals are preserved (so already-camelized input is idempotent).
fn camelize_string(s: &str, upper: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut emitted_first = false;
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '_' || ch == '-' {
            capitalize_next = true;
            continue;
        }
        if !emitted_first {
            if upper {
                for u in ch.to_uppercase() {
                    out.push(u);
                }
            } else {
                for l in ch.to_lowercase() {
                    out.push(l);
                }
            }
            emitted_first = true;
            capitalize_next = false;
        } else if capitalize_next {
            for u in ch.to_uppercase() {
                out.push(u);
            }
            capitalize_next = false;
        } else {
            out.push(ch);
        }
    }
    out
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
