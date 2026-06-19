//! Method call evaluation - String methods.

use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

use std::cell::RefCell;
use std::rc::Rc;

/// Convert `snake_case` / `kebab-case` input to camel case. With `upper=false`
/// the first emitted char is lowercased (`fooBar`); with `upper=true` it is
/// uppercased (`FooBar`). Leading and consecutive separators are collapsed,
/// internal capitals are preserved (so already-camelized input is idempotent).
pub(crate) fn camelize_string(s: &str, upper: bool) -> String {
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

/// URL-safe slug: lowercase, ASCII-fold common Latin accents, collapse any
/// run of non-`[a-z0-9]` chars to a single `-`, trim leading/trailing `-`.
pub(crate) fn slugify_string(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut folded = String::with_capacity(lower.len());
    for ch in lower.chars() {
        match ch {
            'a'..='z' | '0'..='9' => folded.push(ch),
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ą' => folded.push('a'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => folded.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' => folded.push('i'),
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ø' | 'ō' => folded.push('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' => folded.push('u'),
            'ý' | 'ÿ' => folded.push('y'),
            'ç' | 'ć' | 'č' => folded.push('c'),
            'ñ' | 'ń' => folded.push('n'),
            'š' => folded.push('s'),
            'ž' | 'ź' | 'ż' => folded.push('z'),
            'ł' => folded.push('l'),
            'œ' => folded.push_str("oe"),
            'æ' => folded.push_str("ae"),
            'ß' => folded.push_str("ss"),
            _ => folded.push('-'),
        }
    }
    let mut out = String::with_capacity(folded.len());
    let mut prev_hyphen = true; // suppress leading hyphens
    for ch in folded.chars() {
        if ch == '-' {
            if !prev_hyphen {
                out.push('-');
                prev_hyphen = true;
            }
        } else {
            out.push(ch);
            prev_hyphen = false;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    out
}

impl Interpreter {
    pub(crate) fn call_string_method_borrowed(
        &self,
        s: &str,
        method_name: &str,
        arguments: &[Value],
        span: Span,
    ) -> Option<RuntimeResult<Value>> {
        match method_name {
            "length" | "len" | "size" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Int(s.len() as i64)))
            }
            "to_s" | "to_string" | "join" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::String(s.to_string().into())))
            }
            "upcase" | "uppercase" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::String(s.to_uppercase().into())))
            }
            "downcase" | "lowercase" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::String(s.to_lowercase().into())))
            }
            "trim" | "strip" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::String(s.trim().to_string().into())))
            }
            "lstrip" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::String(s.trim_start().to_string().into())))
            }
            "rstrip" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::String(s.trim_end().to_string().into())))
            }
            "reverse" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::String(
                    s.chars().rev().collect::<String>().into(),
                )))
            }
            "slugify" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::String(slugify_string(s).into())))
            }
            "camelize" => {
                if arguments.len() > 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let upper = match arguments.first() {
                    None => false,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => {
                        return Some(Err(RuntimeError::type_error(
                            "camelize expects a boolean argument (true for PascalCase)",
                            span,
                        )))
                    }
                };
                Some(Ok(Value::String(camelize_string(s, upper).into())))
            }
            "empty?" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(s.is_empty())))
            }
            "contains" | "includes?" | "include?" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                match &arguments[0] {
                    Value::String(sub) => Some(Ok(Value::Bool(s.contains(&**(sub))))),
                    _ => Some(Err(RuntimeError::type_error(
                        format!("{} expects a string argument", method_name),
                        span,
                    ))),
                }
            }
            "starts_with" | "starts_with?" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                match &arguments[0] {
                    Value::String(prefix) => Some(Ok(Value::Bool(s.starts_with(&**(prefix))))),
                    _ => Some(Err(RuntimeError::type_error(
                        "starts_with? expects a string argument",
                        span,
                    ))),
                }
            }
            "ends_with" | "ends_with?" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                match &arguments[0] {
                    Value::String(suffix) => Some(Ok(Value::Bool(s.ends_with(&**(suffix))))),
                    _ => Some(Err(RuntimeError::type_error(
                        "ends_with? expects a string argument",
                        span,
                    ))),
                }
            }
            "split" => {
                if arguments.len() > 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let delim = if arguments.is_empty() {
                    " "
                } else {
                    match &arguments[0] {
                        Value::String(delim) => delim.as_ref(),
                        _ => {
                            return Some(Err(RuntimeError::type_error(
                                "split expects a string delimiter",
                                span,
                            )))
                        }
                    }
                };
                let mut parts = Vec::with_capacity(if delim.is_empty() {
                    s.len() + 1
                } else {
                    s.matches(delim).count() + 1
                });
                for part in s.split(delim) {
                    parts.push(Value::String(part.to_string().into()));
                }
                Some(Ok(Value::Array(Rc::new(RefCell::new(parts)))))
            }
            "replace" => {
                if arguments.len() != 2 {
                    return Some(Err(RuntimeError::wrong_arity(2, arguments.len(), span)));
                }
                let from = match &arguments[0] {
                    Value::String(from) => from,
                    _ => {
                        return Some(Err(RuntimeError::type_error(
                            "replace expects a string pattern",
                            span,
                        )))
                    }
                };
                let to = match &arguments[1] {
                    Value::String(to) => to,
                    _ => {
                        return Some(Err(RuntimeError::type_error(
                            "replace expects a string replacement",
                            span,
                        )))
                    }
                };
                Some(Ok(Value::String(s.replace(&**(from), to).into())))
            }
            _ => None,
        }
    }

    /// Handle string methods.
    pub(crate) fn call_string_method(
        &mut self,
        s: &str,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if let Some(result) = self.call_string_method_borrowed(s, method_name, &arguments, span) {
            return result;
        }

        match method_name {
            "starts_with?" => self.string_starts_with(s, arguments, span),
            "ends_with?" => self.string_ends_with(s, arguments, span),
            "chomp" => self.string_chomp(s, arguments, span),
            "lstrip" => self.string_lstrip(s, arguments, span),
            "rstrip" => self.string_rstrip(s, arguments, span),
            "squeeze" => self.string_squeeze(s, arguments, span),
            "count" => self.string_count(s, arguments, span),
            "gsub" | "replace_all" => self.string_gsub(s, arguments, span),
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
            "length" | "len" | "size" => self.string_length(s, arguments, span),
            "to_s" | "to_string" => Ok(Value::String(s.to_string().into())),
            "to_i" | "to_int" => {
                let trimmed = s.trim();
                // Try integer first, then float-truncate (e.g. "4.88".to_i => 4)
                Ok(Value::Int(
                    trimmed
                        .parse::<i64>()
                        .or_else(|_| trimmed.replace(',', ".").parse::<f64>().map(|f| f as i64))
                        .unwrap_or(0),
                ))
            }
            "to_f" | "to_float" => {
                let trimmed = s.trim();
                // Support comma as decimal separator (e.g. "4,88".to_f => 4.88)
                Ok(Value::Float(
                    trimmed
                        .parse::<f64>()
                        .or_else(|_| trimmed.replace(',', ".").parse::<f64>())
                        .unwrap_or(0.0),
                ))
            }
            "upcase" | "uppercase" => Ok(Value::String(s.to_uppercase().into())),
            "downcase" | "lowercase" => Ok(Value::String(s.to_lowercase().into())),
            "html_entities" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::String(
                    crate::interpreter::builtins::html::html_numeric_entities(s).into(),
                ))
            }
            "trim" | "strip" => Ok(Value::String(s.trim().to_string().into())),
            "contains" => self.string_contains(s, arguments, span),
            "starts_with" => self.string_starts_with(s, arguments, span),
            "ends_with" => self.string_ends_with(s, arguments, span),
            "split" => self.string_split(s, arguments, span),
            "index_of" => self.string_index_of(s, arguments, span),
            "substring" => self.string_substring(s, arguments, span),
            "replace" => self.string_replace(s, arguments, span),
            "lpad" => self.string_lpad(s, arguments, span),
            "rpad" => self.string_rpad(s, arguments, span),
            "join" => Ok(Value::String(s.to_string().into())),
            "empty?" => self.string_empty(s, arguments, span),
            "includes?" | "include?" => self.string_include(s, arguments, span),
            "to_sym" => Ok(Value::Symbol(s.to_string().into())),
            "parse_json" => match crate::interpreter::value::parse_json(s) {
                Ok(value) => Ok(value),
                Err(_) => Ok(Value::Hash(Rc::new(RefCell::new(
                    indexmap::IndexMap::with_hasher(ahash::RandomState::new()),
                )))),
            },
            // Parse JSON and only return a Hash; null when the input isn't
            // valid JSON or parses to a non-object (array, scalar, ...).
            "to_h" => match crate::interpreter::value::parse_json(s) {
                Ok(Value::Hash(h)) => Ok(Value::Hash(h)),
                _ => Ok(Value::Null),
            },
            "is_a?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let class_name = match &arguments[0] {
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
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let other = match &arguments[0] {
                    Value::String(o) => o,
                    _ => return Err(RuntimeError::type_error("casecmp expects a string", span)),
                };
                use std::cmp::Ordering;
                Ok(Value::Int(
                    match s.to_lowercase().as_str().cmp(other.to_lowercase().as_ref()) {
                        Ordering::Less => -1,
                        Ordering::Equal => 0,
                        Ordering::Greater => 1,
                    },
                ))
            }
            "casecmp?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let other = match &arguments[0] {
                    Value::String(o) => o,
                    _ => return Err(RuntimeError::type_error("casecmp? expects a string", span)),
                };
                Ok(Value::Bool(s.to_lowercase() == other.to_lowercase()))
            }
            "prepend" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let other = match &arguments[0] {
                    Value::String(o) => o,
                    _ => return Err(RuntimeError::type_error("prepend expects a string", span)),
                };
                let mut result = other.to_string();
                result.push_str(s);
                Ok(Value::String(result.into()))
            }
            "chop" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let mut chars: Vec<char> = s.chars().collect();
                chars.pop();
                Ok(Value::String(chars.into_iter().collect::<String>().into()))
            }
            "ascii_only?" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Bool(s.is_ascii()))
            }
            "succ" | "next" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::String(string_succ(s).into()))
            }
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
        Ok(Value::Bool(s.starts_with(&**(prefix))))
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
        Ok(Value::Bool(s.ends_with(&**(suffix))))
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
        Ok(Value::String(result.to_string().into()))
    }

    fn string_lstrip(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::String(s.trim_start().to_string().into()))
    }

    fn string_rstrip(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::String(s.trim_end().to_string().into()))
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
        Ok(Value::String(result.into()))
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
        let count = s.matches(&**(substr)).count() as i64;
        Ok(Value::Int(count))
    }

    fn string_gsub(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
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
            let re = crate::regex_cache::get_regex(pattern)
                .map_err(|e| RuntimeError::type_error(e, span))?;
            re.replacen(s, limit, &*replacement).to_string()
        } else {
            let re = crate::regex_cache::get_regex(pattern)
                .map_err(|e| RuntimeError::type_error(e, span))?;
            re.replace_all(s, &*replacement).to_string()
        };
        Ok(Value::String(result.into()))
    }

    fn string_sub(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
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

        let re = crate::regex_cache::get_regex(pattern)
            .map_err(|e| RuntimeError::type_error(e, span))?;
        let result = re.replacen(s, 1, &*replacement).to_string();
        Ok(Value::String(result.into()))
    }

    fn string_match(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
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

    fn string_scan(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
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

        let re = crate::regex_cache::get_regex(pattern)
            .map_err(|e| RuntimeError::type_error(e, span))?;
        let matches: Vec<Value> = re
            .find_iter(s)
            .map(|m| Value::String(m.as_str().to_string().into()))
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
        Ok(Value::String(result.into()))
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
            Ok(Value::String(s.to_string().into()))
        } else {
            let total_pad = width - s.len();
            let left_pad = total_pad / 2;
            let right_pad = total_pad - left_pad;
            let result =
                pad_char.to_string().repeat(left_pad) + s + &pad_char.to_string().repeat(right_pad);
            Ok(Value::String(result.into()))
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
            Ok(Value::String(s.to_string().into()))
        } else {
            let result = s.to_string() + &pad_char.to_string().repeat(width - s.len());
            Ok(Value::String(result.into()))
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
            Ok(Value::String(s.to_string().into()))
        } else {
            let result = pad_char.to_string().repeat(width - s.len()) + s;
            Ok(Value::String(result.into()))
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
        let mut chars = Vec::with_capacity(s.len());
        for c in s.chars() {
            chars.push(Value::String(c.to_string().into()));
        }
        Ok(Value::Array(Rc::new(RefCell::new(chars))))
    }

    fn string_lines(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut lines = Vec::with_capacity(s.bytes().filter(|b| *b == b'\n').count() + 1);
        for line in s.lines() {
            lines.push(Value::String(line.to_string().into()));
        }
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
        Ok(Value::String(result.into()))
    }

    fn string_swapcase(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            if c.is_uppercase() {
                for lower in c.to_lowercase() {
                    result.push(lower);
                }
            } else {
                for upper in c.to_uppercase() {
                    result.push(upper);
                }
            }
        }
        Ok(Value::String(result.into()))
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

        let mut result = String::with_capacity(s.len() + insert_str.len());
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
        let result = s.replace(&**(to_delete), "");
        Ok(Value::String(result.into()))
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
        let result = s.strip_prefix(&**(prefix)).unwrap_or(s);
        Ok(Value::String(result.to_string().into()))
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
        let result = s.strip_suffix(&**(suffix)).unwrap_or(s);
        Ok(Value::String(result.to_string().into()))
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

        if let Some(pos) = s.find(&**(sep)) {
            let before = &s[..pos];
            let after = &s[pos + sep.len()..];
            let result = vec![
                Value::String(before.to_string().into()),
                Value::String(sep.to_string().into()),
                Value::String(after.to_string().into()),
            ];
            Ok(Value::Array(Rc::new(RefCell::new(result))))
        } else {
            let result = vec![
                Value::String(s.to_string().into()),
                Value::String("".into()),
                Value::String("".into()),
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

        if let Some(pos) = s.rfind(&**(sep)) {
            let before = &s[..pos];
            let after = &s[pos + sep.len()..];
            let result = vec![
                Value::String(before.to_string().into()),
                Value::String(sep.to_string().into()),
                Value::String(after.to_string().into()),
            ];
            Ok(Value::Array(Rc::new(RefCell::new(result))))
        } else {
            let result = vec![
                Value::String("".into()),
                Value::String("".into()),
                Value::String(s.to_string().into()),
            ];
            Ok(Value::Array(Rc::new(RefCell::new(result))))
        }
    }

    fn string_reverse(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result: String = s.chars().rev().collect();
        Ok(Value::String(result.into()))
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
                Value::String(s) => s.as_ref(),
                _ => "...",
            })
            .unwrap_or("...");

        if s.len() <= length {
            Ok(Value::String(s.to_string().into()))
        } else {
            let result = &s[..length.saturating_sub(suffix.len())];
            Ok(Value::String((result.to_string() + suffix).into()))
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
        Ok(Value::Bool(s.contains(&**(substr))))
    }

    fn string_split(&self, s: &str, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() > 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let delim: &str = if arguments.is_empty() {
            " "
        } else {
            match &arguments[0] {
                Value::String(d) => d.as_ref(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "split expects a string delimiter",
                        span,
                    ))
                }
            }
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
        if let Some(idx) = s.find(&**(substr)) {
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
        let chars: Vec<char> = s.chars().collect();
        let start_usize = if start < 0 { 0 } else { start as usize };
        let end_usize = if end < 0 {
            0
        } else {
            (end as usize).min(chars.len())
        };
        if start_usize >= end_usize || start_usize >= chars.len() {
            Ok(Value::String(String::new().into()))
        } else {
            Ok(Value::String(
                chars[start_usize..end_usize]
                    .iter()
                    .collect::<String>()
                    .into(),
            ))
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
        Ok(Value::String(s.replace(&**(from), to).into()))
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
            Ok(Value::String(s.to_string().into()))
        } else {
            let padding = width - s.len();
            Ok(Value::String(
                (pad_char.to_string().repeat(padding) + s).into(),
            ))
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
            Ok(Value::String(s.to_string().into()))
        } else {
            let padding = width - s.len();
            Ok(Value::String(
                (s.to_string() + &pad_char.to_string().repeat(padding)).into(),
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
        Ok(Value::Bool(s.contains(&**(substr))))
    }
}

/// Increment a string like Ruby's `String#succ`.
/// Finds the last alphanumeric run and increments it with carry.
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

#[cfg(test)]
mod tests {
    use super::{camelize_string, slugify_string, string_succ};

    #[test]
    fn camelize_snake_case_lower() {
        assert_eq!(camelize_string("foo_bar", false), "fooBar");
        assert_eq!(camelize_string("foo_bar_baz", false), "fooBarBaz");
    }

    #[test]
    fn camelize_snake_case_upper() {
        assert_eq!(camelize_string("foo_bar", true), "FooBar");
        assert_eq!(camelize_string("foo_bar_baz", true), "FooBarBaz");
    }

    #[test]
    fn camelize_kebab_case() {
        assert_eq!(camelize_string("foo-bar", false), "fooBar");
        assert_eq!(camelize_string("foo-bar", true), "FooBar");
        assert_eq!(camelize_string("a-b-c-d", false), "aBCD");
    }

    #[test]
    fn camelize_mixed_separators() {
        assert_eq!(camelize_string("foo_bar-baz", false), "fooBarBaz");
    }

    #[test]
    fn camelize_idempotent_on_camelcase() {
        assert_eq!(camelize_string("fooBar", false), "fooBar");
        assert_eq!(camelize_string("FooBar", true), "FooBar");
    }

    #[test]
    fn camelize_lowercases_first_char_in_lower_mode() {
        assert_eq!(camelize_string("FooBar", false), "fooBar");
    }

    #[test]
    fn camelize_uppercases_first_char_in_upper_mode() {
        assert_eq!(camelize_string("fooBar", true), "FooBar");
    }

    #[test]
    fn camelize_empty_and_single_word() {
        assert_eq!(camelize_string("", false), "");
        assert_eq!(camelize_string("", true), "");
        assert_eq!(camelize_string("foo", false), "foo");
        assert_eq!(camelize_string("foo", true), "Foo");
    }

    #[test]
    fn camelize_handles_leading_trailing_consecutive_separators() {
        assert_eq!(camelize_string("_foo_bar", false), "fooBar");
        assert_eq!(camelize_string("foo_bar_", false), "fooBar");
        assert_eq!(camelize_string("foo__bar", false), "fooBar");
        assert_eq!(camelize_string("--foo--bar--", true), "FooBar");
        assert_eq!(camelize_string("___", false), "");
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify_string("Hello World"), "hello-world");
    }

    #[test]
    fn slugify_strips_french_accents() {
        assert_eq!(slugify_string("Café & Croissant"), "cafe-croissant");
        assert_eq!(slugify_string("Crème brûlée"), "creme-brulee");
        assert_eq!(slugify_string("Œuf au plat"), "oeuf-au-plat");
    }

    #[test]
    fn slugify_collapses_and_trims_hyphens() {
        assert_eq!(slugify_string("  ---hello---  "), "hello");
        assert_eq!(slugify_string("a___b   c"), "a-b-c");
    }

    #[test]
    fn slugify_empty_and_punctuation_only() {
        assert_eq!(slugify_string(""), "");
        assert_eq!(slugify_string("!!!"), "");
    }

    #[test]
    fn slugify_keeps_digits() {
        assert_eq!(slugify_string("Pizza 4 You"), "pizza-4-you");
    }

    #[test]
    fn succ_basic_lowercase() {
        assert_eq!(string_succ("a"), "b");
        assert_eq!(string_succ("z"), "aa");
        assert_eq!(string_succ("aa"), "ab");
        assert_eq!(string_succ("az"), "ba");
        assert_eq!(string_succ("zz"), "aaa");
    }

    #[test]
    fn succ_basic_uppercase() {
        assert_eq!(string_succ("A"), "B");
        assert_eq!(string_succ("Z"), "AA");
        assert_eq!(string_succ("ZZ"), "AAA");
    }

    #[test]
    fn succ_basic_digits() {
        assert_eq!(string_succ("0"), "1");
        assert_eq!(string_succ("9"), "10");
        assert_eq!(string_succ("99"), "100");
    }

    #[test]
    fn succ_mixed() {
        assert_eq!(string_succ("a9"), "b0");
    }

    #[test]
    fn succ_no_alnum() {
        assert_eq!(string_succ("!!!"), "!!!");
        assert_eq!(string_succ(""), "");
    }
}
