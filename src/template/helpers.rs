//! Shared array and string helper functions for template rendering.
//! These are extracted from the interpreter to avoid code duplication.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::value::Value;

pub fn call_array_method(
    items: &[Value],
    method_name: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    match method_name {
        "length" | "len" | "size" => Ok(Value::Int(items.len() as i64)),
        "first" => Ok(items.first().cloned().unwrap_or(Value::Null)),
        "last" => Ok(items.last().cloned().unwrap_or(Value::Null)),
        "reverse" => {
            let mut new_items = items.to_vec();
            new_items.reverse();
            Ok(Value::Array(Rc::new(RefCell::new(new_items))))
        }
        "join" => {
            let delim = if let Some(Value::String(s)) = args.first() {
                s.clone()
            } else {
                String::new()
            };
            let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
            Ok(Value::String(parts.join(&delim)))
        }
        "empty" | "is_empty" => Ok(Value::Bool(items.is_empty())),
        "sum" => {
            let mut total = 0.0;
            for v in items {
                match v {
                    Value::Int(n) => total += *n as f64,
                    Value::Float(n) => total += *n,
                    _ => {}
                }
            }
            Ok(Value::Float(total))
        }
        "min" => {
            let mut min_val: Option<i64> = None;
            for v in items {
                if let Value::Int(n) = v {
                    match min_val {
                        Some(m) if *n >= m => {}
                        _ => min_val = Some(*n),
                    }
                }
            }
            Ok(min_val.map(Value::Int).unwrap_or(Value::Null))
        }
        "max" => {
            let mut max_val: Option<i64> = None;
            for v in items {
                if let Value::Int(n) = v {
                    match max_val {
                        Some(m) if *n <= m => {}
                        _ => max_val = Some(*n),
                    }
                }
            }
            Ok(max_val.map(Value::Int).unwrap_or(Value::Null))
        }
        "map" | "filter" | "each" | "reduce" | "find" | "any?" | "all?" | "sort"
        | "sort_by" | "uniq" | "compact" | "flatten" | "include?" | "sample" 
        | "shuffle" | "take" | "drop" | "zip" => {
            Err(format!(
                "Method '{}' requires a function argument - not supported in templates. Use the full language in <% %> tags instead.",
                method_name
            ))
        }
        _ => Err(format!("Unknown array method: {}", method_name)),
    }
}

pub fn call_string_method(s: &str, method_name: &str, args: Vec<Value>) -> Result<Value, String> {
    match method_name {
        "length" | "len" | "size" => Ok(Value::Int(s.len() as i64)),
        "empty" | "is_empty" => Ok(Value::Bool(s.is_empty())),
        "reverse" => {
            let mut chars: Vec<char> = s.chars().collect();
            chars.reverse();
            Ok(Value::String(chars.into_iter().collect()))
        }
        "uppercase" | "upcase" => Ok(Value::String(s.to_uppercase())),
        "lowercase" | "downcase" => Ok(Value::String(s.to_lowercase())),
        "trim" => Ok(Value::String(s.trim().to_string())),
        "capitalize" => {
            let mut chars = s.chars();
            match chars.next() {
                None => Ok(Value::String(String::new())),
                Some(c) => Ok(Value::String(
                    c.to_uppercase().collect::<String>() + chars.as_str(),
                )),
            }
        }
        "replace" => {
            if args.len() >= 2 {
                if let (Value::String(from), Value::String(to)) = (&args[0], &args[1]) {
                    return Ok(Value::String(s.replace(from, to)));
                }
            }
            Err("replace requires two string arguments".to_string())
        }
        "split" => {
            if let Some(Value::String(delim)) = args.first() {
                let parts: Vec<Value> = s
                    .split(delim)
                    .map(|p| Value::String(p.to_string()))
                    .collect();
                return Ok(Value::Array(Rc::new(RefCell::new(parts))));
            }
            Err("split requires a string delimiter".to_string())
        }
        "includes" | "contains" => {
            if let Some(Value::String(sub)) = args.first() {
                return Ok(Value::Bool(s.contains(sub)));
            }
            Err("includes requires a string argument".to_string())
        }
        "starts_with" => {
            if let Some(Value::String(prefix)) = args.first() {
                return Ok(Value::Bool(s.starts_with(prefix)));
            }
            Err("starts_with requires a string argument".to_string())
        }
        "ends_with" => {
            if let Some(Value::String(suffix)) = args.first() {
                return Ok(Value::Bool(s.ends_with(suffix)));
            }
            Err("ends_with requires a string argument".to_string())
        }
        _ => Err(format!("Unknown string method: {}", method_name)),
    }
}
