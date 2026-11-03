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
        // `contains?` and `includes?` are the same builtins under the
        // spellings the style guide asks for; every dispatcher already
        // treats them as synonyms.
        "contains" | "contains?" => 20,
        "starts_with" => 21,
        "ends_with" => 22,
        "include?" | "includes?" => 23,
        "split" => 24,
        "index_of" => 25,
        "count" => 26,
        "delete" => 27,
        "replace" => 28,
        "gsub" | "replace_all" => 29,
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
use crate::interpreter::value::{hash_get_value, HashKey, HashPairs, StrKey, SymKey, Value};
use crate::span::Span;

/// Dispatch a zero-arg string method by integer ID. Returns None if not handled.
#[inline(always)]
pub fn string_method_zero_arg(s: &str, mid: MethodId) -> Option<Value> {
    match mid {
        0 | 1 => Some(Value::Int(s.len() as i64)), // len, length
        2 => Some(Value::Bool(s.is_empty())),      // empty?
        3 => Some(Value::Int(s.len() as i64)),     // bytesize
        4 | 5 => Some(Value::String(s.to_uppercase().into())), // upcase, uppercase
        6 | 7 => Some(Value::String(s.to_lowercase().into())), // downcase, lowercase
        8 => Some(Value::String(s.trim().to_string().into())), // trim
        9 => Some(Value::String(s.trim_start().to_string().into())), // lstrip
        10 => Some(Value::String(s.trim_end().to_string().into())), // rstrip
        11 => {
            // capitalize
            let mut chars = s.chars();
            Some(Value::String(match chars.next() {
                None => String::new().into(),
                Some(first) => (first.to_uppercase().collect::<String>()
                    + &chars.as_str().to_lowercase())
                    .into(),
            }))
        }
        12 => {
            // swapcase
            Some(Value::String(
                crate::interpreter::executor::calls::string_methods::swapcase_string(s).into(),
            ))
        }
        13 => {
            // chomp
            let r = s
                .strip_suffix('\n')
                .or_else(|| s.strip_suffix("\r\n"))
                .or_else(|| s.strip_suffix('\r'))
                .unwrap_or(s);
            Some(Value::String(r.to_string().into()))
        }
        14 => {
            // reverse
            let out: String =
                crate::interpreter::executor::calls::string_methods::reverse_string(s);
            Some(Value::String(out.into()))
        }
        15 => {
            // chars
            Some(Value::Array(Rc::new(RefCell::new(
                crate::interpreter::executor::calls::string_methods::chars_to_values(s),
            ))))
        }
        16 => {
            // bytes
            let v: Vec<Value> = s.bytes().map(|b| Value::Int(b as i64)).collect();
            Some(Value::Array(Rc::new(RefCell::new(v))))
        }
        17 => {
            // lines
            let v: Vec<Value> = s
                .lines()
                .map(|l| Value::String(l.to_string().into()))
                .collect();
            Some(Value::Array(Rc::new(RefCell::new(v))))
        }
        18 => i64::from_str_radix(s, 16).ok().map(Value::Int), // hex
        19 => i64::from_str_radix(s, 8).ok().map(Value::Int),  // oct
        38 | 59 => Some(Value::String(s.to_string().into())),  // to_s, to_string
        41 => Some(Value::String("string".into())),            // class
        42 => Some(Value::Bool(false)),                        // nil?
        43 => Some(Value::Bool(s.trim().is_empty())),          // blank?
        44 => Some(Value::Bool(!s.trim().is_empty())),         // present?
        45 => Some(Value::String(format!("\"{}\"", s).into())), // inspect
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
            Some(Value::String(result.into()))
        }
        57 => s.chars().next().map(|c| Value::Int(c as i64)), // ord
        37 => Some(Value::String(s.to_string().into())),      // join (string.join = itself)
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
        // contains, contains?, include?, includes?. A non-string argument
        // goes to the by-name dispatcher, whose message names the spelling
        // the caller wrote, as the interpreter's does.
        20 | 23 => {
            let Value::String(sub) = arg else {
                return None;
            };
            Some(Ok(Value::Bool(s.contains(&**(sub)))))
        }
        21 | 62 => {
            let Value::String(prefix) = arg else {
                return Some(Err(RuntimeError::type_error(
                    "starts_with? expects a string argument",
                    span,
                )));
            };
            Some(Ok(Value::Bool(s.starts_with(&**(prefix)))))
        }
        22 | 63 => {
            let Value::String(suffix) = arg else {
                return Some(Err(RuntimeError::type_error(
                    "ends_with? expects a string argument",
                    span,
                )));
            };
            Some(Ok(Value::Bool(s.ends_with(&**(suffix)))))
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
                s.matches(delim.as_ref()).count() + 1
            });
            for part in s.split(delim.as_ref()) {
                parts.push(Value::String(part.to_string().into()));
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
            Some(Ok(Value::String(s.replace(&**(from), to).into())))
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
        41 => Some(Value::String("array".into())),            // class
        42 => Some(Value::Bool(false)),                       // nil?
        43 => Some(Value::Bool(arr.borrow().is_empty())),     // blank?
        44 => Some(Value::Bool(!arr.borrow().is_empty())),    // present?
        72 => {
            // sum — integers only. Anything else returns None so dispatch falls
            // through to the full implementation, which handles floats and
            // decimals and rejects non-numerics.
            //
            // This used to skip whatever it did not recognise and return the
            // partial total, which meant `[1.5, 2.5].sum()` was 0 and
            // `[1, "a"].sum()` was 1 — silently, in the engine that serves
            // production, while the interpreter that runs the tests answered
            // correctly or raised.
            let items = arr.borrow();
            let mut total = 0i64;
            for item in items.iter() {
                match item {
                    Value::Int(n) => total += n,
                    _ => return None,
                }
            }
            Some(Value::Int(total))
        }
        73 => {
            // min — integers only; see `sum` above. Skipping non-integers made
            // `[1.5].min()` answer null instead of 1.5.
            let items = arr.borrow();
            let mut min: Option<i64> = None;
            for item in items.iter() {
                match item {
                    Value::Int(n) => min = Some(min.map_or(*n, |m: i64| m.min(*n))),
                    _ => return None,
                }
            }
            Some(min.map_or(Value::Null, Value::Int))
        }
        74 => {
            // max — integers only; see `sum` above. Skipping non-integers made
            // `[1.5].max()` answer null instead of 1.5.
            let items = arr.borrow();
            let mut max: Option<i64> = None;
            for item in items.iter() {
                match item {
                    Value::Int(n) => max = Some(max.map_or(*n, |m: i64| m.max(*n))),
                    _ => return None,
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

/// Dispatch a one-arg array method by integer ID, reading the argument where
/// it lies on the stack: no receiver clone, no argument `Vec`. Returns None
/// for an id it has no entry for and for any argument it does not answer
/// itself (a non-integer index, a negative count, a non-string separator), so
/// the by-name dispatcher raises the error it always has. Every answer must be
/// `vm_call_array_method`'s; `array_method_one_arg_agrees_with_the_general_path`
/// holds the two together.
#[inline(always)]
pub fn array_method_one_arg(
    arr: &Rc<RefCell<Vec<Value>>>,
    mid: MethodId,
    arg: &Value,
) -> Option<Value> {
    match mid {
        // contains, contains?, include?, includes?
        20 | 23 => Some(Value::Bool(arr.borrow().iter().any(|v| v == arg))),
        // index_of
        25 => Some(Value::Int(
            arr.borrow()
                .iter()
                .position(|v| v == arg)
                .map_or(-1, |i| i as i64),
        )),
        // join
        37 => {
            let Value::String(sep) = arg else {
                return None;
            };
            Some(Value::String(
                crate::interpreter::executor::calls::array_ops::join_values(&arr.borrow(), sep)
                    .into(),
            ))
        }
        // get: a negative index counts from the end; out of range is null.
        76 => {
            let Value::Int(idx) = arg else {
                return None;
            };
            let items = arr.borrow();
            let index = if *idx < 0 {
                (items.len() as i64 + idx) as usize
            } else {
                *idx as usize
            };
            Some(items.get(index).cloned().unwrap_or(Value::Null))
        }
        // take, drop
        77 | 78 => {
            let Value::Int(n) = arg else {
                return None;
            };
            if *n < 0 {
                return None;
            }
            let n = *n as usize;
            let items = arr.borrow();
            let result: Vec<Value> = if mid == 77 {
                items.iter().take(n).cloned().collect()
            } else {
                items.iter().skip(n).cloned().collect()
            };
            Some(Value::Array(Rc::new(RefCell::new(result))))
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
        41 => Some(Value::String("hash".into())),              // class
        42 => Some(Value::Bool(false)),                        // nil?
        43 => Some(Value::Bool(hash.borrow().is_empty())),     // blank?
        44 => Some(Value::Bool(!hash.borrow().is_empty())),    // present?
        84 => {
            // keys
            let keys: Vec<Value> = hash.borrow().keys().map(HashKey::to_value).collect();
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

/// Dispatch a zero-arg Int, Float, Decimal, Bool or Null method by integer ID:
/// the conversions (`to_s`, `to_i`, `to_f` and their long spellings) and the
/// universal members. Returns None if not handled.
///
/// The definition is what a paren-less read answers: the `*_member_access`
/// tables in `access/member.rs`, and `call_int_method_impl` for `Int#to_s`,
/// which its table hands back as a method value.
/// `bare_zero_arg_agrees_with_the_general_path` holds the two together.
#[inline]
pub fn primitive_method_zero_arg(receiver: &Value, mid: MethodId) -> Option<Value> {
    use crate::interpreter::executor::calls::int_methods::int_to_s_value;
    use rust_decimal::prelude::ToPrimitive;
    match receiver {
        Value::Int(n) => match mid {
            38 | 59 | 45 => Some(int_to_s_value(*n)), // to_s, to_string, inspect
            39 | 60 => Some(Value::Int(*n)),          // to_i, to_int
            40 | 61 => Some(Value::Float(*n as f64)), // to_f, to_float
            41 => Some(Value::String("int".into())),
            42 | 43 => Some(Value::Bool(false)), // nil?, blank?
            44 => Some(Value::Bool(true)),       // present?
            _ => None,
        },
        Value::Float(n) => match mid {
            38 | 59 | 45 => Some(Value::String(format!("{}", n).into())),
            39 | 60 => Some(Value::Int(*n as i64)),
            40 | 61 => Some(Value::Float(*n)),
            41 => Some(Value::String("float".into())),
            42 | 43 => Some(Value::Bool(false)),
            44 => Some(Value::Bool(true)),
            _ => None,
        },
        Value::Decimal(d) => match mid {
            38 | 59 => Some(Value::String(d.0.to_string().into())),
            39 | 60 => Some(Value::Int(d.0.to_i64().unwrap_or(0))),
            40 | 61 => Some(Value::Float(d.to_f64())),
            41 => Some(Value::String("decimal".into())),
            42 | 43 => Some(Value::Bool(false)),
            44 => Some(Value::Bool(true)),
            45 => Some(Value::String(format!("Decimal({})", d.0).into())),
            _ => None,
        },
        // No `to_f`: a bool has none.
        Value::Bool(b) => match mid {
            38 | 59 | 45 => {
                let text = if *b { "true" } else { "false" };
                Some(Value::String(text.into()))
            }
            39 | 60 => Some(Value::Int(i64::from(*b))),
            41 => Some(Value::String("bool".into())),
            42 => Some(Value::Bool(false)),
            43 => Some(Value::Bool(!*b)),
            44 => Some(Value::Bool(*b)),
            _ => None,
        },
        Value::Null => match mid {
            38 | 59 => Some(Value::String("".into())),
            39 | 60 => Some(Value::Int(0)),
            40 | 61 => Some(Value::Float(0.0)),
            41 => Some(Value::String("null".into())),
            42 | 43 => Some(Value::Bool(true)),
            44 => Some(Value::Bool(false)),
            45 => Some(Value::String("null".into())),
            _ => None,
        },
        _ => None,
    }
}

/// What a paren-less `receiver.<name>` reads when `name`, whose method id is
/// `mid`, is a zero-argument builtin of the receiver's type: the method's
/// result, from the same id tables `CallMethodById` uses for `name()`. `mask`
/// holds the receiver types `name` is a zero-argument builtin of
/// (`method_registry::zero_arg_receiver_mask`), so `h.len`, which a hash
/// reads as the method, is not called, while `a.len` is.
///
/// None when the read is someone else's to answer: a type the mask leaves
/// out, a user method on the type (it outranks the builtin, and may take
/// arguments), an id the table has no entry for, or a value the table
/// declines (`"zz".hex`, which raises). A hash's key comes first; the caller
/// probes it before asking. The fast tier's `GetProperty` and
/// `GetLocalProperty` call this and hand the op to the general arm on None,
/// which answers by name as it always has.
#[inline]
pub fn bare_zero_arg(receiver: &Value, mid: MethodId, mask: u16) -> Option<Value> {
    use crate::interpreter::executor::calls::method_registry::{
        ZERO_ARG_ARRAY, ZERO_ARG_BOOL, ZERO_ARG_DECIMAL, ZERO_ARG_FLOAT, ZERO_ARG_HASH,
        ZERO_ARG_INT, ZERO_ARG_NULL, ZERO_ARG_STRING,
    };
    use crate::interpreter::executor::calls::user_methods::{has_user_methods, PrimType};
    let (bit, prim) = match receiver {
        Value::String(_) => (ZERO_ARG_STRING, PrimType::String),
        Value::Array(_) => (ZERO_ARG_ARRAY, PrimType::Array),
        Value::Hash(_) => (ZERO_ARG_HASH, PrimType::Hash),
        Value::Int(_) => (ZERO_ARG_INT, PrimType::Int),
        Value::Float(_) => (ZERO_ARG_FLOAT, PrimType::Float),
        Value::Decimal(_) => (ZERO_ARG_DECIMAL, PrimType::Decimal),
        Value::Bool(_) => (ZERO_ARG_BOOL, PrimType::Bool),
        Value::Null => (ZERO_ARG_NULL, PrimType::Null),
        _ => return None,
    };
    if mask & bit == 0 || has_user_methods(prim) {
        return None;
    }
    match receiver {
        Value::String(s) => string_method_zero_arg(s, mid),
        Value::Array(arr) => array_method_zero_arg(arr, mid),
        Value::Hash(hash) => hash_method_zero_arg(hash, mid),
        _ => primitive_method_zero_arg(receiver, mid),
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
        // 76 = get, 80 = fetch. They agree on a hit and differ on a miss —
        // `get` answers null, `fetch` raises — so only the hit can be answered
        // here. A miss returns None and falls through to the full dispatcher,
        // which knows the difference. Sharing one arm made `fetch` answer null
        // for a missing key in the VM while the interpreter raised.
        76 | 80 => match hash_get_value(&hash.borrow(), arg) {
            Some(value) => Some(Ok(value.clone())),
            None if mid == 76 => Some(Ok(Value::Null)),
            None => None,
        },
        83 => {
            let found = match arg {
                Value::String(s) => hash.borrow().contains_key(&StrKey(s)),
                _ => hash_get_value(&hash.borrow(), arg).is_some(),
            };
            Some(Ok(Value::Bool(found)))
        }
        27 => {
            let removed = match arg {
                // `shift_remove` keeps the remaining keys in insertion
                // order, as the interpreter does.
                Value::String(s) => hash.borrow_mut().shift_remove(&StrKey(s)),
                Value::Symbol(s) => hash.borrow_mut().shift_remove(&SymKey(s)),
                Value::Int(n) => hash.borrow_mut().shift_remove(&HashKey::Int(*n)),
                Value::Bool(b) => hash.borrow_mut().shift_remove(&HashKey::Bool(*b)),
                Value::Null => hash.borrow_mut().shift_remove(&HashKey::Null),
                Value::Decimal(d) => hash.borrow_mut().shift_remove(&HashKey::Decimal(d.clone())),
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
            if !crate::interpreter::value::hash_set_value(
                &mut hash.borrow_mut(),
                arg0,
                arg1.clone(),
            ) {
                return Some(Err(RuntimeError::type_error(
                    format!("Cannot use {} as hash key", arg0.type_name()),
                    span,
                )));
            }
            Some(Ok(Value::Null))
        }
        _ => None,
    }
}

#[cfg(test)]
mod bare_zero_arg_tests {
    //! `bare_zero_arg` is the fast tier's answer to a paren-less `x.name`;
    //! `Vm::op_get_property_member` is the general arm's. Wherever the first
    //! answers, the two must agree, value for value, on every name the
    //! registry calls a zero-argument builtin.
    use super::*;
    use crate::interpreter::executor::calls::method_registry::{
        known_methods, zero_arg_receiver_mask,
    };
    use crate::interpreter::value::{hash_from_pairs, DecimalValue};
    use crate::vm::Vm;

    fn array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    /// Receivers of every type `bare_zero_arg` answers for, built afresh on
    /// each call: `a.pop` and `a.clear` mutate theirs.
    fn receivers() -> Vec<Value> {
        let decimal = |text: &str, scale: u32| {
            Value::Decimal(DecimalValue::from_str(text, scale).expect("decimal"))
        };
        vec![
            Value::Int(0),
            Value::Int(7),
            Value::Int(-12),
            Value::Int(i64::MIN),
            Value::Int(1_234_567_890_123_456_789),
            Value::Float(2.5),
            Value::Float(2.0),
            Value::Float(-0.0),
            Value::Float(1e21),
            Value::Float(f64::INFINITY),
            decimal("1.50", 2),
            decimal("-3", 0),
            decimal("12345.678", 3),
            Value::Bool(true),
            Value::Bool(false),
            Value::Null,
            Value::String("".into()),
            Value::String("  ".into()),
            Value::String("abc".into()),
            Value::String("Hello World\n".into()),
            Value::String("ff".into()),
            Value::String("17".into()),
            Value::String("a\"b\\c".into()),
            Value::String("ünïcødé ß".into()),
            Value::String("x".repeat(100).into()),
            array(vec![]),
            array(vec![Value::Int(3), Value::Int(1), Value::Int(2)]),
            array(vec![Value::Float(1.5), Value::Int(2)]),
            array(vec![Value::String("a".into()), Value::Null]),
            hash_from_pairs(Vec::<(&str, Value)>::new()),
            hash_from_pairs([("a", Value::Int(1)), ("b", Value::String("x".into()))]),
        ]
    }

    /// Every registered zero-argument name with a method id.
    fn names() -> Vec<&'static str> {
        let mut names: Vec<&'static str> = [
            "int", "float", "decimal", "bool", "null", "string", "array", "hash",
        ]
        .iter()
        .flat_map(|t| known_methods(t).iter())
        .filter(|def| def.zero_arg && resolve_method_id(def.name) != METHOD_UNKNOWN)
        .map(|def| def.name)
        .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Debug output, so `Int(2)` and `Float(2.0)` do not compare equal.
    fn shown(value: &Value) -> String {
        format!("{:?}", value)
    }

    #[test]
    fn bare_zero_arg_agrees_with_the_general_path() {
        let mut vm = Vm::new();
        let span = Span::default();
        let mut answered = 0;
        for name in names() {
            let mid = resolve_method_id(name);
            let mask = zero_arg_receiver_mask(name);
            for (i, receiver) in receivers().into_iter().enumerate() {
                let Some(fast) = bare_zero_arg(&receiver, mid, mask) else {
                    continue;
                };
                answered += 1;
                let general_receiver = receivers().swap_remove(i);
                let general = vm
                    .op_get_property_member(&general_receiver, name, span)
                    .unwrap_or_else(|e| {
                        panic!(
                            "{:?}.{} answered {:?} by id, raised {}",
                            receiver, name, fast, e
                        )
                    });
                assert_eq!(
                    shown(&fast),
                    shown(&general),
                    "{:?}.{}: by id vs by name",
                    receivers()[i],
                    name
                );
                // A mutating builtin (`a.pop`) leaves both receivers alike.
                assert_eq!(
                    shown(&receiver),
                    shown(&general_receiver),
                    "{}.{} left the receivers apart",
                    receiver.type_name(),
                    name
                );
            }
        }
        // The tables do answer: this is no vacuous pass.
        assert!(answered > 200, "only {answered} reads answered by id");
    }

    /// The reads the fast tier must leave to the general arm.
    #[test]
    fn bare_zero_arg_declines_what_is_not_its_to_answer() {
        let hash = hash_from_pairs([("a", Value::Int(1))]);
        // `len` is no zero-argument builtin of a hash: `h.len` reads as the
        // method, while `a.len` and `s.len` are called.
        let len = resolve_method_id("len");
        let len_mask = zero_arg_receiver_mask("len");
        assert_eq!(bare_zero_arg(&hash, len, len_mask), None);
        assert_eq!(
            bare_zero_arg(&array(vec![Value::Int(1)]), len, len_mask),
            Some(Value::Int(1))
        );
        assert_eq!(
            bare_zero_arg(&Value::String("ab".into()), len, len_mask),
            Some(Value::Int(2))
        );
        // A name with no mask (a field, a user method name) is never answered.
        assert_eq!(bare_zero_arg(&hash, 1, 0), None);
        // A value the table declines raises on the general path.
        let hex = resolve_method_id("hex");
        assert_eq!(
            bare_zero_arg(
                &Value::String("zz".into()),
                hex,
                zero_arg_receiver_mask("hex")
            ),
            None
        );
        // Types it does not handle: a symbol, a function.
        let to_s = resolve_method_id("to_s");
        let to_s_mask = zero_arg_receiver_mask("to_s");
        assert_eq!(
            bare_zero_arg(&Value::Symbol("s".into()), to_s, to_s_mask),
            None
        );
        // `Float#round` has no id, so a paren-less `f.round` keeps the general
        // path (which now calls it without building a method value).
        assert_eq!(resolve_method_id("round"), METHOD_UNKNOWN);
    }

    /// A user method on the type outranks the builtin, so the fast tier must
    /// hand the read back. The flag is process-wide and the method lives in
    /// this test's thread only: other tests see the flag, miss the lookup and
    /// take the general path, which answers the builtin as before.
    #[test]
    fn a_user_method_on_the_type_outranks_the_id_table() {
        use crate::interpreter::executor::calls::user_methods::{register_user_method, PrimType};
        use crate::interpreter::value::Function;
        let to_s = resolve_method_id("to_s");
        let mask = zero_arg_receiver_mask("to_s");
        let body = Function {
            name: "to_s".to_string(),
            ..Function::default()
        };
        register_user_method(PrimType::Bool, "to_s".to_string(), Rc::new(body));
        assert_eq!(bare_zero_arg(&Value::Bool(true), to_s, mask), None);
    }
}

#[cfg(test)]
mod one_arg_tests {
    //! `array_method_one_arg` answers `xs.get(1)`, `xs.includes?(v)`,
    //! `tags.join(", ")` … by id, with the argument read off the stack;
    //! `Vm::vm_call_array_method` is the by-name dispatcher it stands in front
    //! of. Wherever the table answers, the two must agree, and it may decline
    //! only what the dispatcher raises on.
    use super::*;
    use crate::interpreter::value::hash_from_pairs;
    use crate::vm::compiler::Compiler;
    use crate::vm::opcode::Op;
    use crate::vm::Vm;

    fn array(items: Vec<Value>) -> Rc<RefCell<Vec<Value>>> {
        Rc::new(RefCell::new(items))
    }

    fn receivers() -> Vec<Rc<RefCell<Vec<Value>>>> {
        vec![
            array(vec![]),
            array(vec![Value::Int(3), Value::Int(1), Value::Int(2)]),
            array(vec![
                Value::Int(1),
                Value::Float(1.0),
                Value::String("a".into()),
                Value::Null,
                Value::Bool(true),
                array_value(vec![Value::Int(1)]),
                hash_from_pairs([("k", Value::Int(1))]),
            ]),
        ]
    }

    fn array_value(items: Vec<Value>) -> Value {
        Value::Array(array(items))
    }

    fn arguments() -> Vec<Value> {
        vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(2),
            Value::Int(7),
            Value::Int(-1),
            Value::Int(-3),
            Value::Int(-99),
            Value::Float(1.0),
            Value::String("".into()),
            Value::String(", ".into()),
            Value::String("a".into()),
            Value::Null,
            Value::Bool(true),
            array_value(vec![Value::Int(1)]),
            hash_from_pairs([("k", Value::Int(1))]),
        ]
    }

    const NAMES: &[&str] = &[
        "include?",
        "includes?",
        "contains",
        "contains?",
        "index_of",
        "join",
        "get",
        "take",
        "drop",
    ];

    fn shown(value: &Value) -> String {
        format!("{:?}", value)
    }

    #[test]
    fn array_method_one_arg_agrees_with_the_general_path() {
        let mut vm = Vm::new();
        let span = Span::default();
        let mut answered = 0;
        for name in NAMES {
            let mid = resolve_method_id(name);
            assert_ne!(mid, METHOD_UNKNOWN, "{name} has no id");
            for arr in receivers() {
                for arg in arguments() {
                    let general =
                        vm.vm_call_array_method(&arr, name, std::slice::from_ref(&arg), span);
                    match array_method_one_arg(&arr, mid, &arg) {
                        Some(fast) => {
                            answered += 1;
                            let general = general.unwrap_or_else(|e| {
                                panic!("{:?}.{name}({:?}): {:?} by id, raised {e}", arr, arg, fast)
                            });
                            assert_eq!(
                                shown(&fast),
                                shown(&general),
                                "{:?}.{name}({:?}): by id vs by name",
                                arr.borrow(),
                                arg
                            );
                        }
                        None => assert!(
                            general.is_err(),
                            "{:?}.{name}({:?}) declined by id but answers {:?} by name",
                            arr.borrow(),
                            arg,
                            general
                        ),
                    }
                }
            }
        }
        assert!(answered > 200, "only {answered} calls answered by id");
    }

    /// `"s".includes?(x)` and `"s".contains?(x)` take the ids of their
    /// synonyms and agree with the by-name dispatcher, error included: a
    /// non-string argument is declined so the message names the spelling used.
    #[test]
    fn string_membership_spellings_share_an_id() {
        assert_eq!(
            resolve_method_id("includes?"),
            resolve_method_id("include?")
        );
        assert_eq!(
            resolve_method_id("contains?"),
            resolve_method_id("contains")
        );
        let vm = Vm::new();
        let span = Span::default();
        let text: crate::interpreter::value::SoliStr = "hello world".into();
        for name in ["include?", "includes?", "contains", "contains?"] {
            let mid = resolve_method_id(name);
            for arg in [
                Value::String("wor".into()),
                Value::String("xyz".into()),
                Value::String("".into()),
            ] {
                let fast = string_method_one_arg(&text, mid, &arg, span)
                    .expect("a string argument is answered by id")
                    .expect("no error");
                let general = vm
                    .vm_call_string_method(&text, name, std::slice::from_ref(&arg), span)
                    .expect("no error");
                assert_eq!(shown(&fast), shown(&general), "{name}({:?})", arg);
            }
            assert!(string_method_one_arg(&text, mid, &Value::Int(1), span).is_none());
            let err = vm
                .vm_call_string_method(&text, name, &[Value::Int(1)], span)
                .expect_err("a non-string argument raises");
            assert!(
                err.to_string()
                    .contains(&format!("{name} expects a string argument")),
                "{name}: {err}"
            );
        }
    }

    /// The compiler emits `CallMethodById` for the style guide's spellings,
    /// which used to compile to the by-name `CallMethod`.
    #[test]
    fn includes_and_contains_compile_to_call_method_by_id() {
        for name in ["includes?", "contains?"] {
            let src = format!("xs = [1, 2]\nprint(xs.{name}(1))\n");
            let tokens = crate::lexer::Scanner::new(&src).scan_tokens().expect("lex");
            let program = crate::parser::Parser::new(tokens).parse().expect("parse");
            let module = Compiler::compile(&program).expect("compile");
            let code = &module.main.chunk.code;
            assert!(
                code.iter()
                    .any(|op| matches!(op, Op::CallMethodById(_, 1, _, _))),
                "{name}: {:?}",
                code
            );
            assert!(
                !code.iter().any(|op| matches!(op, Op::CallMethod(_, _, _))),
                "{name} still compiles to CallMethod: {:?}",
                code
            );
        }
    }
}
