//! Method call evaluation - String methods.

use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

use std::cell::RefCell;
use std::rc::Rc;

impl Interpreter {
    /// Handle string methods.
    pub(crate) fn call_string_method(
        &mut self,
        s: &str,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "starts_with?" => self.string_starts_with(s, arguments, span),
            "ends_with?" => self.string_ends_with(s, arguments, span),
            "chomp" => self.string_chomp(s, arguments, span),
            "lstrip" => self.string_lstrip(s, arguments, span),
            "rstrip" => self.string_rstrip(s, arguments, span),
            "squeeze" => self.string_squeeze(s, arguments, span),
            "count" => self.string_count(s, arguments, span),
            "gsub" => self.string_gsub(s, arguments, span),
            "sub" => self.string_sub(s, arguments, span),
            "match" => self.string_match(s, arguments, span),
            "scan" => self.string_scan(s, arguments, span),
            "tr" => self.string_tr(s, arguments, span),
            "center" => self.string_center(s, arguments, span),
            "ljust" => self.string_ljust(s, arguments, span),
            "rjust" => self.string_rjust(s, arguments, span),
            "ord" => self.string_ord(s, arguments, span),
            "bytes" => self.string_bytes(s, arguments, span),
            "chars" => self.string_chars(s, arguments, span),
            "lines" => self.string_lines(s, arguments, span),
            "bytesize" => self.string_bytesize(s, arguments, span),
            "capitalize" => self.string_capitalize(s, arguments, span),
            "swapcase" => self.string_swapcase(s, arguments, span),
            "insert" => self.string_insert(s, arguments, span),
            "delete" => self.string_delete(s, arguments, span),
            "delete_prefix" => self.string_delete_prefix(s, arguments, span),
            "delete_suffix" => self.string_delete_suffix(s, arguments, span),
            "partition" => self.string_partition(s, arguments, span),
            "rpartition" => self.string_rpartition(s, arguments, span),
            "reverse" => self.string_reverse(s, arguments, span),
            "hex" => self.string_hex(s, arguments, span),
            "oct" => self.string_oct(s, arguments, span),
            "truncate" => self.string_truncate(s, arguments, span),
            "length" => self.string_length(s, arguments, span),
            "to_string" => Ok(Value::String(s.to_string())),
            "upcase" | "uppercase" => Ok(Value::String(s.to_uppercase())),
            "downcase" | "lowercase" => Ok(Value::String(s.to_lowercase())),
            "trim" => Ok(Value::String(s.trim().to_string())),
            "contains" => self.string_contains(s, arguments, span),
            "starts_with" => self.string_starts_with(s, arguments, span),
            "ends_with" => self.string_ends_with(s, arguments, span),
            "split" => self.string_split(s, arguments, span),
            "index_of" => self.string_index_of(s, arguments, span),
            "substring" => self.string_substring(s, arguments, span),
            "replace" => self.string_replace(s, arguments, span),
            "lpad" => self.string_lpad(s, arguments, span),
            "rpad" => self.string_rpad(s, arguments, span),
            "join" => Ok(Value::String(s.to_string())),
            "empty?" => self.string_empty(s, arguments, span),
            "include?" => self.string_include(s, arguments, span),
            "chr" => Err(RuntimeError::type_error(
                "chr is not a string instance method",
                span,
            )),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "String".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn string_starts_with(
        &self,
        s: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let prefix = match &arguments[0] {
            Value::String(p) => p,
            _ => {
                return Err(RuntimeError::type_error(
                    "starts_with? expects a string argument",
                    span,
                ))
            }
        };
        Ok(Value::Bool(s.starts_with(prefix)))
    }

    fn string_ends_with(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let suffix = match &arguments[0] {
            Value::String(suf) => suf,
            _ => {
                return Err(RuntimeError::type_error(
                    "ends_with? expects a string argument",
                    span,
                ))
            }
        };
        Ok(Value::Bool(s.ends_with(suffix)))
    }

    fn string_chomp(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result = s
            .strip_suffix('\n')
            .or_else(|| s.strip_suffix("\r\n"))
            .or_else(|| s.strip_suffix('\r'))
            .unwrap_or(s);
        Ok(Value::String(result.to_string()))
    }

    fn string_lstrip(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::String(s.trim_start().to_string()))
    }

    fn string_rstrip(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::String(s.trim_end().to_string()))
    }

    fn string_squeeze(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() > 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let chars_to_squeeze: Option<Vec<char>> = arguments.first().map(|v| match v {
            Value::String(s) => s.chars().collect(),
            _ => vec![],
        });

        let mut result = String::new();
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

    fn string_count(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let substr = match &arguments[0] {
            Value::String(sub) => sub,
            _ => {
                return Err(RuntimeError::type_error(
                    "count expects a string argument",
                    span,
                ))
            }
        };
        let count = s.matches(substr).count() as i64;
        Ok(Value::Int(count))
    }

    fn string_gsub(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        use regex::Regex;

        if arguments.len() < 2 || arguments.len() > 3 {
            return Err(RuntimeError::wrong_arity(3, arguments.len(), span));
        }
        let pattern = match &arguments[0] {
            Value::String(p) => p,
            _ => {
                return Err(RuntimeError::type_error(
                    "gsub expects a string pattern",
                    span,
                ))
            }
        };
        let replacement = match &arguments[1] {
            Value::String(r) => r.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "gsub expects a string replacement",
                    span,
                ))
            }
        };

        let result = if arguments.len() == 3 {
            let limit = match &arguments[2] {
                Value::Int(n) if *n >= 0 => *n as usize,
                _ => {
                    return Err(RuntimeError::type_error(
                        "gsub limit must be a non-negative integer",
                        span,
                    ))
                }
            };
            let re = Regex::new(pattern)
                .map_err(|e| RuntimeError::type_error(format!("invalid regex: {}", e), span))?;
            re.replacen(s, limit, &replacement).to_string()
        } else {
            let re = Regex::new(pattern)
                .map_err(|e| RuntimeError::type_error(format!("invalid regex: {}", e), span))?;
            re.replace_all(s, &replacement).to_string()
        };
        Ok(Value::String(result))
    }

    fn string_sub(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        use regex::Regex;

        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let pattern = match &arguments[0] {
            Value::String(p) => p,
            _ => {
                return Err(RuntimeError::type_error(
                    "sub expects a string pattern",
                    span,
                ))
            }
        };
        let replacement = match &arguments[1] {
            Value::String(r) => r.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "sub expects a string replacement",
                    span,
                ))
            }
        };

        let re = Regex::new(pattern)
            .map_err(|e| RuntimeError::type_error(format!("invalid regex: {}", e), span))?;
        let result = re.replacen(s, 1, &replacement).to_string();
        Ok(Value::String(result))
    }

    fn string_match(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        use regex::Regex;

        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let pattern = match &arguments[0] {
            Value::String(p) => p,
            _ => {
                return Err(RuntimeError::type_error(
                    "match expects a string pattern",
                    span,
                ))
            }
        };

        let re = Regex::new(pattern)
            .map_err(|e| RuntimeError::type_error(format!("invalid regex: {}", e), span))?;
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

    fn string_scan(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        use regex::Regex;

        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let pattern = match &arguments[0] {
            Value::String(p) => p,
            _ => {
                return Err(RuntimeError::type_error(
                    "scan expects a string pattern",
                    span,
                ))
            }
        };

        let re = Regex::new(pattern)
            .map_err(|e| RuntimeError::type_error(format!("invalid regex: {}", e), span))?;
        let matches: Vec<Value> = re
            .find_iter(s)
            .map(|m| Value::String(m.as_str().to_string()))
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(matches))))
    }

    fn string_tr(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let from_chars = match &arguments[0] {
            Value::String(f) => f,
            _ => {
                return Err(RuntimeError::type_error(
                    "tr expects a string from pattern",
                    span,
                ))
            }
        };
        let to_chars = match &arguments[1] {
            Value::String(t) => t,
            _ => {
                return Err(RuntimeError::type_error(
                    "tr expects a string to pattern",
                    span,
                ))
            }
        };

        let mut result = String::new();
        for c in s.chars() {
            if let Some(pos) = from_chars.find(c) {
                if let Some(replacement) = to_chars.chars().nth(pos) {
                    result.push(replacement);
                }
            } else {
                result.push(c);
            }
        }
        Ok(Value::String(result))
    }

    fn string_center(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let width = match &arguments[0] {
            Value::Int(w) if *w > 0 => *w as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "center expects a positive integer width",
                    span,
                ))
            }
        };
        let pad_char = arguments
            .get(1)
            .map(|v| match v {
                Value::String(s) => s.chars().next().unwrap_or(' '),
                _ => ' ',
            })
            .unwrap_or(' ');

        if s.len() >= width {
            Ok(Value::String(s.to_string()))
        } else {
            let total_pad = width - s.len();
            let left_pad = total_pad / 2;
            let right_pad = total_pad - left_pad;
            let result =
                pad_char.to_string().repeat(left_pad) + s + &pad_char.to_string().repeat(right_pad);
            Ok(Value::String(result))
        }
    }

    fn string_ljust(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let width = match &arguments[0] {
            Value::Int(w) if *w > 0 => *w as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "ljust expects a positive integer width",
                    span,
                ))
            }
        };
        let pad_char = arguments
            .get(1)
            .map(|v| match v {
                Value::String(s) => s.chars().next().unwrap_or(' '),
                _ => ' ',
            })
            .unwrap_or(' ');

        if s.len() >= width {
            Ok(Value::String(s.to_string()))
        } else {
            let result = s.to_string() + &pad_char.to_string().repeat(width - s.len());
            Ok(Value::String(result))
        }
    }

    fn string_rjust(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let width = match &arguments[0] {
            Value::Int(w) if *w > 0 => *w as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "rjust expects a positive integer width",
                    span,
                ))
            }
        };
        let pad_char = arguments
            .get(1)
            .map(|v| match v {
                Value::String(s) => s.chars().next().unwrap_or(' '),
                _ => ' ',
            })
            .unwrap_or(' ');

        if s.len() >= width {
            Ok(Value::String(s.to_string()))
        } else {
            let result = pad_char.to_string().repeat(width - s.len()) + s;
            Ok(Value::String(result))
        }
    }

    fn string_ord(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if let Some(c) = s.chars().next() {
            Ok(Value::Int(c as i64))
        } else {
            Err(RuntimeError::type_error("ord on empty string", span))
        }
    }

    fn string_bytes(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let bytes: Vec<Value> = s.bytes().map(|b| Value::Int(b as i64)).collect();
        Ok(Value::Array(Rc::new(RefCell::new(bytes))))
    }

    fn string_chars(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let chars: Vec<Value> = s.chars().map(|c| Value::String(c.to_string())).collect();
        Ok(Value::Array(Rc::new(RefCell::new(chars))))
    }

    fn string_lines(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let lines: Vec<Value> = s.lines().map(|l| Value::String(l.to_string())).collect();
        Ok(Value::Array(Rc::new(RefCell::new(lines))))
    }

    fn string_bytesize(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Int(s.len() as i64))
    }

    fn string_capitalize(
        &self,
        s: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut chars = s.chars();
        let result: String = match chars.next() {
            None => String::new(),
            Some(first) => {
                first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
            }
        };
        Ok(Value::String(result))
    }

    fn string_swapcase(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result: String = s
            .chars()
            .map(|c| {
                if c.is_uppercase() {
                    c.to_lowercase().to_string()
                } else {
                    c.to_uppercase().to_string()
                }
            })
            .collect();
        Ok(Value::String(result))
    }

    fn string_insert(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let index = match &arguments[0] {
            Value::Int(i) if *i >= 0 => *i as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "insert expects a non-negative integer index",
                    span,
                ))
            }
        };
        let insert_str = match &arguments[1] {
            Value::String(str) => str,
            _ => {
                return Err(RuntimeError::type_error(
                    "insert expects a string to insert",
                    span,
                ))
            }
        };

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

    fn string_delete(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let to_delete = match &arguments[0] {
            Value::String(d) => d,
            _ => {
                return Err(RuntimeError::type_error(
                    "delete expects a string argument",
                    span,
                ))
            }
        };
        let result = s.replace(to_delete, "");
        Ok(Value::String(result))
    }

    fn string_delete_prefix(
        &self,
        s: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let prefix = match &arguments[0] {
            Value::String(p) => p,
            _ => {
                return Err(RuntimeError::type_error(
                    "delete_prefix expects a string argument",
                    span,
                ))
            }
        };
        let result = s.strip_prefix(prefix).unwrap_or(s);
        Ok(Value::String(result.to_string()))
    }

    fn string_delete_suffix(
        &self,
        s: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let suffix = match &arguments[0] {
            Value::String(suf) => suf,
            _ => {
                return Err(RuntimeError::type_error(
                    "delete_suffix expects a string argument",
                    span,
                ))
            }
        };
        let result = s.strip_suffix(suffix).unwrap_or(s);
        Ok(Value::String(result.to_string()))
    }

    fn string_partition(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let sep = match &arguments[0] {
            Value::String(s) => s,
            _ => {
                return Err(RuntimeError::type_error(
                    "partition expects a string separator",
                    span,
                ))
            }
        };

        if let Some(pos) = s.find(sep) {
            let before = &s[..pos];
            let after = &s[pos + sep.len()..];
            let result = vec![
                Value::String(before.to_string()),
                Value::String(sep.to_string()),
                Value::String(after.to_string()),
            ];
            Ok(Value::Array(Rc::new(RefCell::new(result))))
        } else {
            let result = vec![
                Value::String(s.to_string()),
                Value::String("".to_string()),
                Value::String("".to_string()),
            ];
            Ok(Value::Array(Rc::new(RefCell::new(result))))
        }
    }

    fn string_rpartition(
        &self,
        s: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let sep = match &arguments[0] {
            Value::String(s) => s,
            _ => {
                return Err(RuntimeError::type_error(
                    "rpartition expects a string separator",
                    span,
                ))
            }
        };

        if let Some(pos) = s.rfind(sep) {
            let before = &s[..pos];
            let after = &s[pos + sep.len()..];
            let result = vec![
                Value::String(before.to_string()),
                Value::String(sep.to_string()),
                Value::String(after.to_string()),
            ];
            Ok(Value::Array(Rc::new(RefCell::new(result))))
        } else {
            let result = vec![
                Value::String("".to_string()),
                Value::String("".to_string()),
                Value::String(s.to_string()),
            ];
            Ok(Value::Array(Rc::new(RefCell::new(result))))
        }
    }

    fn string_reverse(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result: String = s.chars().rev().collect();
        Ok(Value::String(result))
    }

    fn string_hex(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result = i64::from_str_radix(s, 16)
            .map_err(|e| RuntimeError::type_error(format!("invalid hex: {}", e), span))?;
        Ok(Value::Int(result))
    }

    fn string_oct(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result = i64::from_str_radix(s, 8)
            .map_err(|e| RuntimeError::type_error(format!("invalid octal: {}", e), span))?;
        Ok(Value::Int(result))
    }

    fn string_truncate(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let length = match &arguments[0] {
            Value::Int(l) if *l > 0 => *l as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "truncate expects a positive integer length",
                    span,
                ))
            }
        };
        let suffix = arguments
            .get(1)
            .map(|v| match v {
                Value::String(s) => s.as_str(),
                _ => "...",
            })
            .unwrap_or("...");

        if s.len() <= length {
            Ok(Value::String(s.to_string()))
        } else {
            let result = &s[..length.saturating_sub(suffix.len())];
            Ok(Value::String(result.to_string() + suffix))
        }
    }

    fn string_length(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Int(s.len() as i64))
    }

    fn string_contains(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let substr = match &arguments[0] {
            Value::String(sub) => sub,
            _ => {
                return Err(RuntimeError::type_error(
                    "contains expects a string argument",
                    span,
                ))
            }
        };
        Ok(Value::Bool(s.contains(substr)))
    }

    fn string_split(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let delim = match &arguments[0] {
            Value::String(d) => d,
            _ => {
                return Err(RuntimeError::type_error(
                    "split expects a string delimiter",
                    span,
                ))
            }
        };
        let parts: Vec<Value> = s
            .split(delim)
            .map(|p| Value::String(p.to_string()))
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(parts))))
    }

    fn string_index_of(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let substr = match &arguments[0] {
            Value::String(sub) => sub,
            _ => {
                return Err(RuntimeError::type_error(
                    "index_of expects a string argument",
                    span,
                ))
            }
        };
        if let Some(idx) = s.find(substr) {
            Ok(Value::Int(idx as i64))
        } else {
            Ok(Value::Int(-1))
        }
    }

    fn string_substring(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let start = match &arguments[0] {
            Value::Int(i) => *i,
            _ => {
                return Err(RuntimeError::type_error(
                    "substring expects integer start",
                    span,
                ))
            }
        };
        let end = match &arguments[1] {
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

    fn string_replace(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let from = match &arguments[0] {
            Value::String(f) => f,
            _ => {
                return Err(RuntimeError::type_error(
                    "replace expects string from",
                    span,
                ))
            }
        };
        let to = match &arguments[1] {
            Value::String(t) => t,
            _ => return Err(RuntimeError::type_error("replace expects string to", span)),
        };
        Ok(Value::String(s.replace(from, to)))
    }

    fn string_lpad(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let width = match &arguments[0] {
            Value::Int(w) if *w >= 0 => *w as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "lpad expects non-negative integer width",
                    span,
                ))
            }
        };
        let pad_char = arguments
            .get(1)
            .map(|v| match v {
                Value::String(ps) => ps.chars().next().unwrap_or(' '),
                _ => ' ',
            })
            .unwrap_or(' ');
        if s.len() >= width {
            Ok(Value::String(s.to_string()))
        } else {
            let padding = width - s.len();
            Ok(Value::String(pad_char.to_string().repeat(padding) + s))
        }
    }

    fn string_rpad(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let width = match &arguments[0] {
            Value::Int(w) if *w >= 0 => *w as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "rpad expects non-negative integer width",
                    span,
                ))
            }
        };
        let pad_char = arguments
            .get(1)
            .map(|v| match v {
                Value::String(ps) => ps.chars().next().unwrap_or(' '),
                _ => ' ',
            })
            .unwrap_or(' ');
        if s.len() >= width {
            Ok(Value::String(s.to_string()))
        } else {
            let padding = width - s.len();
            Ok(Value::String(
                s.to_string() + &pad_char.to_string().repeat(padding),
            ))
        }
    }

    fn string_empty(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Bool(s.is_empty()))
    }

    fn string_include(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let substr = match &arguments[0] {
            Value::String(sub) => sub,
            _ => {
                return Err(RuntimeError::type_error(
                    "include? expects a string argument",
                    span,
                ))
            }
        };
        Ok(Value::Bool(s.contains(substr)))
    }
}
