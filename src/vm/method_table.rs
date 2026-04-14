//! Static method dispatch table — maps method IDs to fast dispatch.
//!
//! At compile time, method names are resolved to numeric IDs via `resolve_method_id`.
//! At runtime, the VM uses the ID for a fast integer match instead of string comparison.

/// Method ID type — stored in CallMethodById opcode.
pub type MethodId = u16;

/// Sentinel value meaning "unknown method, fall back to string dispatch".
pub const METHOD_UNKNOWN: MethodId = u16::MAX;

/// Resolve a method name to a MethodId at compile time.
/// Returns METHOD_UNKNOWN for unrecognized names (e.g., user-defined methods on classes).
pub fn resolve_method_id(name: &str) -> MethodId {
    match name {
        "len" | "size" => 0,
        "length" => 1,
        "empty?" => 2,
        "bytesize" => 3,
        "upcase" => 4,
        "uppercase" => 5,
        "downcase" => 6,
        "lowercase" => 7,
        "trim" | "strip" => 8,
        "lstrip" => 9,
        "rstrip" => 10,
        "capitalize" => 11,
        "swapcase" => 12,
        "chomp" => 13,
        "reverse" => 14,
        "chars" => 15,
        "bytes" => 16,
        "lines" => 17,
        "hex" => 18,
        "oct" => 19,
        "contains" => 20,
        "starts_with" => 21,
        "ends_with" => 22,
        "include?" => 23,
        "split" => 24,
        "index_of" => 25,
        "count" => 26,
        "delete" => 27,
        "replace" => 28,
        "gsub" => 29,
        "sub" => 30,
        "tr" => 31,
        "center" => 32,
        "ljust" => 33,
        "rjust" => 34,
        "lpad" => 35,
        "rpad" => 36,
        "join" => 37,
        "to_s" => 38,
        "to_i" => 39,
        "to_f" => 40,
        "class" => 41,
        "nil?" => 42,
        "blank?" => 43,
        "present?" => 44,
        "inspect" => 45,
        "squeeze" => 46,
        "is_a?" => 47,
        "substring" => 48,
        "insert" => 49,
        "truncate" => 50,
        "delete_prefix" => 51,
        "delete_suffix" => 52,
        "partition" => 53,
        "rpartition" => 54,
        "match" => 55,
        "scan" => 56,
        "ord" => 57,
        "parse_json" => 58,
        "to_string" => 59,
        "to_int" => 60,
        "to_float" => 61,
        "starts_with?" => 62,
        "ends_with?" => 63,
        "push" => 64,
        "pop" => 65,
        "clear" => 66,
        "first" => 67,
        "last" => 68,
        "uniq" => 69,
        "compact" => 70,
        "flatten" => 71,
        "sum" => 72,
        "min" => 73,
        "max" => 74,
        "sort" => 75,
        "get" => 76,
        "take" => 77,
        "drop" => 78,
        "set" => 79,
        "fetch" => 80,
        "merge" => 81,
        "invert" => 82,
        "has_key" => 83,
        "keys" => 84,
        "values" => 85,
        "entries" => 86,
        _ => METHOD_UNKNOWN,
    }
}

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::{hash_get_value, HashKey, HashPairs, StrKey, Value};
use crate::span::Span;

/// Dispatch a zero-arg string method by integer ID. Returns None if not handled.
#[inline(always)]
pub fn string_method_zero_arg(s: &str, mid: MethodId) -> Option<Value> {
    match mid {
        0 | 1 => Some(Value::Int(s.len() as i64)), // len, length
        2 => Some(Value::Bool(s.is_empty())),      // empty?
        3 => Some(Value::Int(s.len() as i64)),     // bytesize
        4 | 5 => Some(Value::String(s.to_uppercase())), // upcase, uppercase
        6 | 7 => Some(Value::String(s.to_lowercase())), // downcase, lowercase
        8 => Some(Value::String(s.trim().to_string())), // trim
        9 => Some(Value::String(s.trim_start().to_string())), // lstrip
        10 => Some(Value::String(s.trim_end().to_string())), // rstrip
        11 => {
            // capitalize
            let mut chars = s.chars();
            Some(Value::String(match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }))
        }
        12 => {
            // swapcase
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
            Some(Value::String(result))
        }
        13 => {
            // chomp
            let r = s
                .strip_suffix('\n')
                .or_else(|| s.strip_suffix("\r\n"))
                .or_else(|| s.strip_suffix('\r'))
                .unwrap_or(s);
            Some(Value::String(r.to_string()))
        }
        14 => Some(Value::String(s.chars().rev().collect())), // reverse
        15 => {
            // chars
            let v: Vec<Value> = s.chars().map(|c| Value::String(c.to_string())).collect();
            Some(Value::Array(Rc::new(RefCell::new(v))))
        }
        16 => {
            // bytes
            let v: Vec<Value> = s.bytes().map(|b| Value::Int(b as i64)).collect();
            Some(Value::Array(Rc::new(RefCell::new(v))))
        }
        17 => {
            // lines
            let v: Vec<Value> = s.lines().map(|l| Value::String(l.to_string())).collect();
            Some(Value::Array(Rc::new(RefCell::new(v))))
        }
        18 => i64::from_str_radix(s, 16).ok().map(Value::Int), // hex
        19 => i64::from_str_radix(s, 8).ok().map(Value::Int),  // oct
        38 | 59 => Some(Value::String(s.to_string())),         // to_s, to_string
        41 => Some(Value::String("string".to_string())),       // class
        42 => Some(Value::Bool(false)),                        // nil?
        43 => Some(Value::Bool(s.trim().is_empty())),          // blank?
        44 => Some(Value::Bool(!s.trim().is_empty())),         // present?
        45 => Some(Value::String(format!("\"{}\"", s))),       // inspect
        46 => {
            // squeeze (0-arg form)
            let mut result = String::with_capacity(s.len());
            let mut last: Option<char> = None;
            for c in s.chars() {
                if last != Some(c) {
                    result.push(c);
                }
                last = Some(c);
            }
            Some(Value::String(result))
        }
        57 => s.chars().next().map(|c| Value::Int(c as i64)), // ord
        37 => Some(Value::String(s.to_string())),             // join (string.join = itself)
        _ => None,
    }
}

/// Dispatch a one-arg string method by integer ID. Returns None if not handled.
#[inline(always)]
pub fn string_method_one_arg(
    s: &str,
    mid: MethodId,
    arg: &Value,
    span: Span,
) -> Option<Result<Value, RuntimeError>> {
    match mid {
        20 | 23 => {
            let Value::String(sub) = arg else {
                return Some(Err(RuntimeError::type_error(
                    "expected string argument",
                    span,
                )));
            };
            Some(Ok(Value::Bool(s.contains(sub))))
        }
        21 | 62 => {
            let Value::String(prefix) = arg else {
                return Some(Err(RuntimeError::type_error(
                    "expected string argument",
                    span,
                )));
            };
            Some(Ok(Value::Bool(s.starts_with(prefix))))
        }
        22 | 63 => {
            let Value::String(suffix) = arg else {
                return Some(Err(RuntimeError::type_error(
                    "expected string argument",
                    span,
                )));
            };
            Some(Ok(Value::Bool(s.ends_with(suffix))))
        }
        24 => {
            let Value::String(delim) = arg else {
                return Some(Err(RuntimeError::type_error(
                    "split expects a string delimiter",
                    span,
                )));
            };
            let mut parts = Vec::with_capacity(if delim.is_empty() {
                s.len() + 1
            } else {
                s.matches(delim.as_str()).count() + 1
            });
            for part in s.split(delim.as_str()) {
                parts.push(Value::String(part.to_string()));
            }
            Some(Ok(Value::Array(Rc::new(RefCell::new(parts)))))
        }
        _ => None,
    }
}

/// Dispatch a two-arg string method by integer ID. Returns None if not handled.
#[inline(always)]
pub fn string_method_two_arg(
    s: &str,
    mid: MethodId,
    arg0: &Value,
    arg1: &Value,
    span: Span,
) -> Option<Result<Value, RuntimeError>> {
    match mid {
        28 => {
            let Value::String(from) = arg0 else {
                return Some(Err(RuntimeError::type_error(
                    "replace expects a string pattern",
                    span,
                )));
            };
            let Value::String(to) = arg1 else {
                return Some(Err(RuntimeError::type_error(
                    "replace expects a string replacement",
                    span,
                )));
            };
            Some(Ok(Value::String(s.replace(from, to))))
        }
        _ => None,
    }
}

/// Dispatch a zero-arg array method by integer ID. Returns None if not handled.
#[inline(always)]
pub fn array_method_zero_arg(arr: &Rc<RefCell<Vec<Value>>>, mid: MethodId) -> Option<Value> {
    match mid {
        0 | 1 => Some(Value::Int(arr.borrow().len() as i64)), // len, length
        2 => Some(Value::Bool(arr.borrow().is_empty())),      // empty?
        67 => Some(arr.borrow().first().cloned().unwrap_or(Value::Null)), // first
        68 => Some(arr.borrow().last().cloned().unwrap_or(Value::Null)), // last
        41 => Some(Value::String("array".to_string())),       // class
        42 => Some(Value::Bool(false)),                       // nil?
        43 => Some(Value::Bool(arr.borrow().is_empty())),     // blank?
        44 => Some(Value::Bool(!arr.borrow().is_empty())),    // present?
        72 => {
            // sum
            let items = arr.borrow();
            let mut total = 0i64;
            for item in items.iter() {
                if let Value::Int(n) = item {
                    total += n;
                }
            }
            Some(Value::Int(total))
        }
        73 => {
            // min
            let items = arr.borrow();
            let mut min: Option<i64> = None;
            for item in items.iter() {
                if let Value::Int(n) = item {
                    min = Some(min.map_or(*n, |m: i64| m.min(*n)));
                }
            }
            Some(min.map_or(Value::Null, Value::Int))
        }
        74 => {
            // max
            let items = arr.borrow();
            let mut max: Option<i64> = None;
            for item in items.iter() {
                if let Value::Int(n) = item {
                    max = Some(max.map_or(*n, |m: i64| m.max(*n)));
                }
            }
            Some(max.map_or(Value::Null, Value::Int))
        }
        14 => {
            // reverse
            let mut reversed = arr.borrow().clone();
            reversed.reverse();
            Some(Value::Array(Rc::new(RefCell::new(reversed))))
        }
        65 => {
            // pop
            arr.borrow_mut().pop().or(Some(Value::Null))
        }
        66 => {
            // clear
            arr.borrow_mut().clear();
            Some(Value::Null)
        }
        _ => None,
    }
}

/// Dispatch a zero-arg hash method by integer ID. Returns None if not handled.
#[inline(always)]
pub fn hash_method_zero_arg(hash: &Rc<RefCell<HashPairs>>, mid: MethodId) -> Option<Value> {
    match mid {
        0 | 1 => Some(Value::Int(hash.borrow().len() as i64)), // len, length
        2 => Some(Value::Bool(hash.borrow().is_empty())),      // empty?
        41 => Some(Value::String("hash".to_string())),         // class
        42 => Some(Value::Bool(false)),                        // nil?
        43 => Some(Value::Bool(hash.borrow().is_empty())),     // blank?
        44 => Some(Value::Bool(!hash.borrow().is_empty())),    // present?
        84 => {
            // keys
            let keys: Vec<Value> = hash
                .borrow()
                .keys()
                .map(|k| match k {
                    HashKey::String(s) => Value::String(s.clone()),
                    HashKey::Symbol(s) => Value::Symbol(s.clone()),
                    HashKey::Int(n) => Value::Int(*n),
                    HashKey::Bool(b) => Value::Bool(*b),
                    HashKey::Null => Value::Null,
                    HashKey::Decimal(d) => Value::String(d.to_string()),
                })
                .collect();
            Some(Value::Array(Rc::new(RefCell::new(keys))))
        }
        85 => {
            // values
            let values: Vec<Value> = hash.borrow().values().cloned().collect();
            Some(Value::Array(Rc::new(RefCell::new(values))))
        }
        _ => None,
    }
}

/// Dispatch a one-arg hash method by integer ID. Returns None if not handled.
#[inline(always)]
pub fn hash_method_one_arg(
    hash: &Rc<RefCell<HashPairs>>,
    mid: MethodId,
    arg: &Value,
    span: Span,
) -> Option<Result<Value, RuntimeError>> {
    match mid {
        76 | 80 => {
            let value = hash_get_value(&hash.borrow(), arg)
                .cloned()
                .unwrap_or(Value::Null);
            Some(Ok(value))
        }
        83 => {
            let found = match arg {
                Value::String(s) => hash.borrow().contains_key(&StrKey(s)),
                _ => hash_get_value(&hash.borrow(), arg).is_some(),
            };
            Some(Ok(Value::Bool(found)))
        }
        27 => {
            let removed = match arg {
                Value::String(s) => hash.borrow_mut().swap_remove(&StrKey(s)),
                Value::Int(n) => hash.borrow_mut().swap_remove(&HashKey::Int(*n)),
                Value::Bool(b) => hash.borrow_mut().swap_remove(&HashKey::Bool(*b)),
                Value::Null => hash.borrow_mut().swap_remove(&HashKey::Null),
                _ => {
                    return Some(Err(RuntimeError::type_error(
                        format!("Cannot use {} as hash key", arg.type_name()),
                        span,
                    )))
                }
            };
            Some(Ok(removed.unwrap_or(Value::Null)))
        }
        _ => None,
    }
}

/// Dispatch a two-arg hash method by integer ID. Returns None if not handled.
#[inline(always)]
pub fn hash_method_two_arg(
    hash: &Rc<RefCell<HashPairs>>,
    mid: MethodId,
    arg0: &Value,
    arg1: &Value,
    span: Span,
) -> Option<Result<Value, RuntimeError>> {
    match mid {
        76 | 80 => {
            let value = hash_get_value(&hash.borrow(), arg0)
                .cloned()
                .unwrap_or_else(|| arg1.clone());
            Some(Ok(value))
        }
        79 => {
            match arg0 {
                Value::String(s) => {
                    let mut hash_ref = hash.borrow_mut();
                    if let Some((_, _, existing)) = hash_ref.get_full_mut(&StrKey(s)) {
                        *existing = arg1.clone();
                    } else {
                        hash_ref.insert(HashKey::String(s.clone()), arg1.clone());
                    }
                }
                Value::Int(n) => {
                    hash.borrow_mut().insert(HashKey::Int(*n), arg1.clone());
                }
                Value::Bool(b) => {
                    hash.borrow_mut().insert(HashKey::Bool(*b), arg1.clone());
                }
                Value::Null => {
                    hash.borrow_mut().insert(HashKey::Null, arg1.clone());
                }
                _ => {
                    return Some(Err(RuntimeError::type_error(
                        format!("Cannot use {} as hash key", arg0.type_name()),
                        span,
                    )))
                }
            }
            Some(Ok(Value::Null))
        }
        _ => None,
    }
}
