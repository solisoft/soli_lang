//! Native array method dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::{hash_get_value, HashKey, Value};
use crate::span::Span;

use super::vm::Vm;

/// Reject a non-function where a callback belongs, naming the method.
///
/// The VM used to discover this only when it tried to invoke the value, and
/// reported "Cannot call non-function value" — which says neither which call was
/// wrong nor what it wanted, and is useless in a chain. The interpreter has
/// always said `all? expects a function argument`; this says the same thing.
#[inline]
pub(crate) fn expect_callback(value: &Value, method: &str, span: Span) -> Result<(), RuntimeError> {
    if value.is_callable() {
        Ok(())
    } else {
        Err(RuntimeError::type_error(
            format!("{method} expects a function argument"),
            span,
        ))
    }
}

impl Vm {
    /// Dispatch an array method call.
    pub fn vm_call_array_method(
        &mut self,
        arr: &Rc<RefCell<Vec<Value>>>,
        name: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // Universal zero-argument methods, guarded in one place. The
        // tree-walking interpreter already rejects `x.nil?("junk")`; the VM
        // accepted the argument and threw it away, so the same call errored
        // under `soli test` and quietly returned a value under `soli serve`.
        if !args.is_empty()
            && matches!(
                name,
                "class" | "nil?" | "blank?" | "present?" | "inspect" | "to_s" | "to_string"
            )
        {
            {
                return Err(RuntimeError::wrong_arity(0, args.len(), span));
            }
        }
        match name {
            // --- Closure-taking methods ---
            // Snapshot the length once and re-borrow per iteration. Avoids the
            // upfront Vec<Value> clone (large for big arrays) at the cost of a
            // RefCell borrow check per element. Iteration uses the live array,
            // matching Ruby's semantics; if the closure shrinks it past `i`,
            // we stop early.
            "map" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let mut result = Vec::with_capacity(len);
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        let v = self.invoke_in_batch_one(&batch, &cb, item, span)?;
                        result.push(v);
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "filter" | "select" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let mut result = Vec::new();
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        let keep = self.invoke_in_batch_one(&batch, &cb, item.clone(), span)?;
                        if keep.is_truthy() {
                            result.push(item);
                        }
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "reduce" | "fold" => {
                // The initial value is optional, as it is in Ruby and as the
                // interpreter has always allowed: without one, the first element
                // seeds the accumulator. Requiring it here meant the idiomatic
                // `xs.reduce(fn(a, b) a + b)` ran under `soli test` and failed
                // under `soli serve`.
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(2, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let seeded = args.len() == 2;
                let mut acc = if seeded {
                    args[1].clone()
                } else if len > 0 {
                    arr.borrow()[0].clone()
                } else {
                    return Err(RuntimeError::type_error(
                        "reduce on empty array requires initial value",
                        span,
                    ));
                };
                let start_idx = if seeded { 0 } else { 1 };
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in start_idx..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        acc = self.invoke_in_batch_two(&batch, &cb, acc.clone(), item, span)?;
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(acc)
            }
            "each" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        self.invoke_in_batch_one(&batch, &cb, item, span)?;
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Array(arr.clone()))
            }
            "each_with_index" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        self.invoke_in_batch_two(&batch, &cb, item, Value::Int(i as i64), span)?;
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Array(arr.clone()))
            }
            "index_of" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let items = arr.borrow();
                let idx = items
                    .iter()
                    .position(|v| v == &args[0])
                    .map(|i| i as i64)
                    .unwrap_or(-1);
                Ok(Value::Int(idx))
            }
            // --- Mutating methods ---
            "push" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                arr.borrow_mut().push(args[0].clone());
                Ok(Value::Null)
            }
            "pop" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                // Matches the VM's own method-table `pop` and the interpreter:
                // an empty array pops to null rather than raising.
                Ok(arr.borrow_mut().pop().unwrap_or(Value::Null))
            }
            "clear" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                arr.borrow_mut().clear();
                Ok(Value::Null)
            }

            // --- Non-mutating methods ---
            "length" | "len" | "size" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                Ok(Value::Int(arr.borrow().len() as i64))
            }
            "empty?" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                Ok(Value::Bool(arr.borrow().is_empty()))
            }
            "first" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                Ok(arr.borrow().first().cloned().unwrap_or(Value::Null))
            }
            "last" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                Ok(arr.borrow().last().cloned().unwrap_or(Value::Null))
            }
            "reverse" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                // Clone-then-reverse, deliberately: it looks like two passes where
                // `iter().rev().cloned().collect()` is one, but it measures 12-15%
                // faster (20k and 200k elements). The clone is a forward vectorized
                // memcpy and the reverse is a vectorized two-pointer swap, while
                // collecting from a reversed iterator is a backwards scalar walk
                // that defeats the prefetcher.
                let mut reversed = arr.borrow().clone();
                reversed.reverse();
                Ok(Value::Array(Rc::new(RefCell::new(reversed))))
            }
            "uniq" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let result =
                    crate::interpreter::executor::calls::array_ops::uniq_values(&arr.borrow());
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "intersection" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let other = extract_array_arg(&args[0], "intersection", span)?;
                let result = crate::interpreter::executor::calls::array_ops::intersection_values(
                    &arr.borrow(),
                    &other,
                );
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "union" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let other = extract_array_arg(&args[0], "union", span)?;
                let result = crate::interpreter::executor::calls::array_ops::union_values(
                    &arr.borrow(),
                    &other,
                );
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "difference" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let other = extract_array_arg(&args[0], "difference", span)?;
                let result = crate::interpreter::executor::calls::array_ops::difference_values(
                    &arr.borrow(),
                    &other,
                );
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "compact" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let result =
                    crate::interpreter::executor::calls::array_ops::compact_values(&arr.borrow());
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "flatten" => {
                // Match the tree-walker: recursive flatten with an optional
                // non-negative depth. (This arm used to flatten only one level
                // and reject any argument.)
                let max_depth = match args.len() {
                    0 => None,
                    1 => match &args[0] {
                        Value::Int(n) if *n >= 0 => Some(*n as usize),
                        _ => {
                            return Err(RuntimeError::type_error(
                                "flatten expects a non-negative integer",
                                span,
                            ))
                        }
                    },
                    _ => return Err(RuntimeError::wrong_arity(1, args.len(), span)),
                };
                let result = crate::interpreter::executor::calls::array_ops::flatten_values(
                    &arr.borrow(),
                    max_depth,
                );
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "concat" => {
                // Validate all args and snapshot their elements before
                // mutating self, so `arr.concat(arr)` and bad-arg failures
                // leave the receiver untouched.
                let mut to_append: Vec<Value> = Vec::new();
                for arg in args.iter() {
                    let other = match arg {
                        Value::Array(other_arr) => other_arr.borrow().clone(),
                        Value::Instance(inst) => {
                            match inst.borrow().fields.get("__value").cloned() {
                                Some(Value::Array(other_arr)) => other_arr.borrow().clone(),
                                _ => {
                                    return Err(RuntimeError::type_error(
                                        "Array.concat() argument must be an Array",
                                        span,
                                    ))
                                }
                            }
                        }
                        _ => {
                            return Err(RuntimeError::type_error(
                                "Array.concat() argument must be an Array",
                                span,
                            ))
                        }
                    };
                    to_append.extend(other);
                }
                arr.borrow_mut().extend(to_append);
                Ok(Value::Array(arr.clone()))
            }
            "contains" | "include?" | "includes?" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let items = arr.borrow();
                Ok(Value::Bool(items.iter().any(|v| v == &args[0])))
            }
            "sum" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let items = arr.borrow();
                let mut has_float = false;
                let mut has_decimal = false;
                let mut total_i = 0i64;
                let mut total_f = 0.0f64;
                for item in items.iter() {
                    match item {
                        Value::Int(n) => total_i += n,
                        Value::Float(n) => {
                            has_float = true;
                            total_f += n;
                        }
                        Value::Decimal(d) => {
                            has_decimal = true;
                            total_f += d.to_f64();
                        }
                        _ => {
                            return Err(RuntimeError::type_error("sum expects numeric array", span))
                        }
                    }
                }
                if has_float || has_decimal {
                    Ok(Value::Float(total_i as f64 + total_f))
                } else {
                    Ok(Value::Int(total_i))
                }
            }
            "min" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let items = arr.borrow();
                if items.is_empty() {
                    return Ok(Value::Null);
                }
                let mut min = &items[0];
                for item in items.iter().skip(1) {
                    match (min, item) {
                        // See `max` above — same missing String arm.
                        (Value::String(a), Value::String(b)) if b < a => min = item,
                        (Value::Int(a), Value::Int(b)) if b < a => min = item,
                        (Value::Float(a), Value::Float(b)) if b < a => min = item,
                        (Value::Int(a), Value::Float(b)) if *b < *a as f64 => min = item,
                        (Value::Float(a), Value::Int(b)) if (*b as f64) < *a => min = item,
                        (Value::Decimal(a), Value::Decimal(b)) if b.to_f64() < a.to_f64() => {
                            min = item
                        }
                        (Value::Int(a), Value::Decimal(b)) if b.to_f64() < *a as f64 => min = item,
                        (Value::Decimal(a), Value::Int(b)) if (*b as f64) < a.to_f64() => {
                            min = item
                        }
                        (Value::Float(a), Value::Decimal(b)) if b.to_f64() < *a => min = item,
                        (Value::Decimal(a), Value::Float(b)) if *b < a.to_f64() => min = item,
                        _ => {}
                    }
                }
                Ok(min.clone())
            }
            "max" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let items = arr.borrow();
                if items.is_empty() {
                    return Ok(Value::Null);
                }
                let mut max = &items[0];
                for item in items.iter().skip(1) {
                    match (max, item) {
                        // Strings had no arm here, so they never displaced the
                        // running candidate: `["a", "b", "c"].max()` answered
                        // "a", the first element. The interpreter compares them
                        // through the shared sort comparator and always has.
                        (Value::String(a), Value::String(b)) if b > a => max = item,
                        (Value::Int(a), Value::Int(b)) if b > a => max = item,
                        (Value::Float(a), Value::Float(b)) if b > a => max = item,
                        (Value::Int(a), Value::Float(b)) if *b > *a as f64 => max = item,
                        (Value::Float(a), Value::Int(b)) if (*b as f64) > *a => max = item,
                        (Value::Decimal(a), Value::Decimal(b)) if b.to_f64() > a.to_f64() => {
                            max = item
                        }
                        (Value::Int(a), Value::Decimal(b)) if b.to_f64() > *a as f64 => max = item,
                        (Value::Decimal(a), Value::Int(b)) if (*b as f64) > a.to_f64() => {
                            max = item
                        }
                        (Value::Float(a), Value::Decimal(b)) if b.to_f64() > *a => max = item,
                        (Value::Decimal(a), Value::Float(b)) if *b > a.to_f64() => max = item,
                        _ => {}
                    }
                }
                Ok(max.clone())
            }
            "sort" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let mut sorted = arr.borrow().clone();
                sorted.sort_by(|a, b| match (a, b) {
                    (Value::Int(x), Value::Int(y)) => x.cmp(y),
                    (Value::Float(x), Value::Float(y)) => {
                        x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Value::String(x), Value::String(y)) => x.cmp(y),
                    _ => std::cmp::Ordering::Equal,
                });
                Ok(Value::Array(Rc::new(RefCell::new(sorted))))
            }
            "join" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let sep = match &args[0] {
                    Value::String(s) => s.as_ref(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "join expects a string separator",
                            span,
                        ))
                    }
                };
                let items = arr.borrow();
                if items.is_empty() {
                    return Ok(Value::String(String::new().into()));
                }
                let mut total_len = sep.len() * (items.len() - 1);
                for v in items.iter() {
                    total_len += v.display_len();
                }
                let mut result = String::with_capacity(total_len);
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        result.push_str(sep);
                    }
                    v.write_to_string(&mut result);
                }
                Ok(Value::String(result.into()))
            }
            "get" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                match &args[0] {
                    Value::Int(idx) => {
                        let items = arr.borrow();
                        let index = if *idx < 0 {
                            (items.len() as i64 + idx) as usize
                        } else {
                            *idx as usize
                        };
                        Ok(items.get(index).cloned().unwrap_or(Value::Null))
                    }
                    _ => Err(RuntimeError::type_error(
                        "get expects an integer index",
                        span,
                    )),
                }
            }
            "take" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let n = match &args[0] {
                    Value::Int(n) if *n >= 0 => *n as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "take expects a non-negative integer",
                            span,
                        ))
                    }
                };
                let items = arr.borrow();
                let result: Vec<Value> = items.iter().take(n).cloned().collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "drop" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let n = match &args[0] {
                    Value::Int(n) if *n >= 0 => *n as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "drop expects a non-negative integer",
                            span,
                        ))
                    }
                };
                let items = arr.borrow();
                let result: Vec<Value> = items.iter().skip(n).cloned().collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "slice" => {
                let start = if !args.is_empty() {
                    match &args[0] {
                        Value::Int(n) => Some(*n),
                        _ => None,
                    }
                } else {
                    None
                };
                let end = if args.len() >= 2 {
                    match &args[1] {
                        Value::Int(n) => Some(*n),
                        _ => None,
                    }
                } else {
                    None
                };
                let items = arr.borrow();
                let len = items.len() as i64;
                let start_idx = match start {
                    Some(s) if s < 0 => (len + s).max(0) as usize,
                    Some(s) => (s as usize).min(len as usize),
                    None => 0,
                };
                let end_idx = match end {
                    Some(e) if e < 0 => (len + e).max(0) as usize,
                    Some(e) => (e as usize).min(len as usize),
                    None => len as usize,
                };
                let result: Vec<Value> = items
                    .iter()
                    .skip(start_idx)
                    .take(end_idx.saturating_sub(start_idx))
                    .cloned()
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "to_json" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                match crate::interpreter::value_stringify::stringify_array_to_string(&arr.borrow())
                {
                    Ok(json) => Ok(Value::String(json.into())),
                    Err(e) => Err(RuntimeError::General { message: e, span }),
                }
            }
            "to_string" | "to_s" => {
                let items = arr.borrow();
                if items.is_empty() {
                    return Ok(Value::String("[]".into()));
                }
                let mut total_len = 2;
                for (i, v) in items.iter().enumerate() {
                    total_len += v.display_len();
                    if i > 0 {
                        total_len += 2;
                    }
                }
                let mut result = String::with_capacity(total_len);
                result.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        result.push_str(", ");
                    }
                    v.write_to_string(&mut result);
                }
                result.push(']');
                Ok(Value::String(result.into()))
            }
            // Universal methods
            "class" => Ok(Value::String("array".into())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(arr.borrow().is_empty())),
            "present?" => Ok(Value::Bool(!arr.borrow().is_empty())),
            "inspect" => {
                let rendered = crate::interpreter::executor::Interpreter::inspect_value(
                    &Value::Array(arr.clone()),
                );
                Ok(Value::String(rendered.into()))
            }
            "is_a?" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let class_name = match &args[0] {
                    Value::String(s) => s.as_ref(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "is_a? expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(class_name == "array" || class_name == "object"))
            }
            "delete" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let val = &args[0];
                let b = arr.borrow();
                if b.contains(val) {
                    let new_arr: Vec<Value> = b.iter().filter(|v| *v != val).cloned().collect();
                    Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
                } else {
                    Ok(Value::Null)
                }
            }
            "delete_at" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let b = arr.borrow();
                let idx = match &args[0] {
                    Value::Int(i) => {
                        if *i >= 0 {
                            *i as usize
                        } else {
                            b.len().saturating_sub((-*i) as usize)
                        }
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "delete_at expects an integer index",
                            span,
                        ))
                    }
                };
                if idx < b.len() {
                    let mut new_arr = b.clone();
                    new_arr.remove(idx);
                    Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
                } else {
                    Ok(Value::Null)
                }
            }
            "shift" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let b = arr.borrow();
                if b.is_empty() {
                    Ok(Value::Null)
                } else {
                    let mut new_arr = b.clone();
                    new_arr.remove(0);
                    Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
                }
            }
            "unshift" => {
                if args.is_empty() {
                    return Ok(Value::Array(Rc::new(RefCell::new(arr.borrow().clone()))));
                }
                let mut new_arr = args.to_vec();
                new_arr.extend(arr.borrow().iter().cloned());
                Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
            }
            "insert" => {
                if args.len() < 2 {
                    return Err(RuntimeError::wrong_arity(2, args.len(), span));
                }
                let b = arr.borrow();
                let idx = match &args[0] {
                    Value::Int(i) => {
                        if *i >= 0 {
                            *i as usize
                        } else {
                            b.len().saturating_sub((-*i) as usize)
                        }
                    }
                    _ => return Err(RuntimeError::type_error("insert expects integer", span)),
                };
                let mut new_arr = b.clone();
                let vals = &args[1..];
                let insert_at = idx.min(new_arr.len());
                let mut tail = new_arr.split_off(insert_at);
                new_arr.extend(vals.iter().cloned());
                new_arr.append(&mut tail);
                Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
            }
            "rotate" => {
                if args.len() > 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let b = arr.borrow();
                let count = match args.first() {
                    Some(Value::Int(n)) => *n,
                    None => 1,
                    _ => return Err(RuntimeError::type_error("rotate expects an integer", span)),
                };
                if b.is_empty() {
                    return Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))));
                }
                let len = b.len() as i64;
                let normalized = ((count % len) + len) % len;
                let split_at = normalized as usize;
                let rotated: Vec<Value> = b[split_at..]
                    .iter()
                    .chain(b[..split_at].iter())
                    .cloned()
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(rotated))))
            }
            "reject" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let mut result = Vec::new();
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        let v = self.invoke_in_batch_one(&batch, &cb, item.clone(), span)?;
                        if !v.is_truthy() {
                            result.push(item);
                        }
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "none?" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let batch = self.enter_callable_batch();
                let outcome: Result<Value, RuntimeError> = (|| {
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        let v = self.invoke_in_batch_one(&batch, &cb, item, span)?;
                        if v.is_truthy() {
                            return Ok(Value::Bool(false));
                        }
                    }
                    Ok(Value::Bool(true))
                })();
                self.exit_callable_batch(batch);
                outcome
            }
            "one?" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let batch = self.enter_callable_batch();
                let outcome: Result<Value, RuntimeError> = (|| {
                    let mut found = false;
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        let v = self.invoke_in_batch_one(&batch, &cb, item, span)?;
                        if v.is_truthy() {
                            if found {
                                return Ok(Value::Bool(false));
                            }
                            found = true;
                        }
                    }
                    Ok(Value::Bool(found))
                })();
                self.exit_callable_batch(batch);
                outcome
            }
            "values_at" => {
                let b = arr.borrow();
                let mut result = Vec::new();
                for arg in args {
                    match arg {
                        Value::Int(i) => {
                            let idx = if *i >= 0 {
                                *i as usize
                            } else {
                                b.len().saturating_sub((-*i) as usize)
                            };
                            result.push(if idx < b.len() {
                                b[idx].clone()
                            } else {
                                Value::Null
                            });
                        }
                        Value::Array(indices) => {
                            for i in indices.borrow().iter() {
                                if let Value::Int(n) = i {
                                    let idx = if *n >= 0 {
                                        *n as usize
                                    } else {
                                        b.len().saturating_sub((-*n) as usize)
                                    };
                                    result.push(if idx < b.len() {
                                        b[idx].clone()
                                    } else {
                                        Value::Null
                                    });
                                }
                            }
                        }
                        _ => result.push(Value::Null),
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "count" => {
                if args.is_empty() {
                    return Ok(Value::Int(arr.borrow().len() as i64));
                }
                if args.len() == 1 {
                    if let Value::Function(_) = &args[0] {
                        expect_callback(&args[0], name, span)?;
                        let cb = args[0].clone();
                        let len = arr.borrow().len();
                        let mut count = 0i64;
                        let batch = self.enter_callable_batch();
                        let outcome: Result<(), RuntimeError> = (|| {
                            for i in 0..len {
                                let b = arr.borrow();
                                if i >= b.len() {
                                    break;
                                }
                                let item = b[i].clone();
                                drop(b);
                                let v = self.invoke_in_batch_one(&batch, &cb, item, span)?;
                                if v.is_truthy() {
                                    count += 1;
                                }
                            }
                            Ok(())
                        })();
                        self.exit_callable_batch(batch);
                        outcome?;
                        return Ok(Value::Int(count));
                    }
                    let c = arr.borrow().iter().filter(|v| *v == &args[0]).count() as i64;
                    return Ok(Value::Int(c));
                }
                Err(RuntimeError::wrong_arity(1, args.len(), span))
            }
            // --- Closure-taking search/predicate methods ---
            "find" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let len = arr.borrow().len();
                let mut found = Value::Null;
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        if self
                            .invoke_in_batch_one(&batch, &cb, item.clone(), span)?
                            .is_truthy()
                        {
                            found = item;
                            break;
                        }
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(found)
            }
            "any?" | "all?" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                expect_callback(&args[0], name, span)?;
                let cb = args[0].clone();
                let want_any = name == "any?";
                let len = arr.borrow().len();
                let mut answer = !want_any; // all? starts true, any? starts false
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let b = arr.borrow();
                        if i >= b.len() {
                            break;
                        }
                        let item = b[i].clone();
                        drop(b);
                        let truthy = self
                            .invoke_in_batch_one(&batch, &cb, item, span)?
                            .is_truthy();
                        if want_any && truthy {
                            answer = true;
                            break;
                        }
                        if !want_any && !truthy {
                            answer = false;
                            break;
                        }
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Bool(answer))
            }
            "sort_by" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                match &args[0] {
                    Value::String(key) => {
                        let hash_key = HashKey::String(key.clone());
                        let mut sorted = arr.borrow().clone();
                        sorted.sort_by(|a, b| {
                            compare_sort_values(
                                &extract_hash_value(a, &hash_key),
                                &extract_hash_value(b, &hash_key),
                            )
                        });
                        Ok(Value::Array(Rc::new(RefCell::new(sorted))))
                    }
                    cb @ (Value::Function(_) | Value::VmClosure(_)) => {
                        let cb = cb.clone();
                        let len = arr.borrow().len();
                        let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(len);
                        let batch = self.enter_callable_batch();
                        let outcome: Result<(), RuntimeError> = (|| {
                            for i in 0..len {
                                let b = arr.borrow();
                                if i >= b.len() {
                                    break;
                                }
                                let item = b[i].clone();
                                drop(b);
                                let key =
                                    self.invoke_in_batch_one(&batch, &cb, item.clone(), span)?;
                                keyed.push((item, key));
                            }
                            Ok(())
                        })();
                        self.exit_callable_batch(batch);
                        outcome?;
                        keyed.sort_by(|a, b| compare_sort_values(&a.1, &b.1));
                        let sorted: Vec<Value> = keyed.into_iter().map(|(item, _)| item).collect();
                        Ok(Value::Array(Rc::new(RefCell::new(sorted))))
                    }
                    _ => Err(RuntimeError::type_error(
                        "sort_by expects a string key or a function argument",
                        span,
                    )),
                }
            }
            // --- Pure transforms / lookups ---
            "compact_blank" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let result: Vec<Value> = arr
                    .borrow()
                    .iter()
                    .filter(|v| !is_blank(v))
                    .cloned()
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "sample" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                use rand::seq::SliceRandom;
                let items = arr.borrow();
                let mut rng = rand::thread_rng();
                Ok(items.choose(&mut rng).cloned().unwrap_or(Value::Null))
            }
            "shuffle" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                use rand::seq::SliceRandom;
                let mut result = arr.borrow().clone();
                let mut rng = rand::thread_rng();
                result.shuffle(&mut rng);
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "zip" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let other = extract_array_arg(&args[0], "zip", span)?;
                let items = arr.borrow();
                let result: Vec<Value> = items
                    .iter()
                    .zip(other.iter())
                    .map(|(a, b)| Value::Array(Rc::new(RefCell::new(vec![a.clone(), b.clone()]))))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "dig" => {
                if args.is_empty() {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let mut current = match &args[0] {
                    Value::Int(idx) => {
                        let items = arr.borrow();
                        let idx = if *idx < 0 {
                            items.len() as i64 + idx
                        } else {
                            *idx
                        };
                        usize::try_from(idx)
                            .ok()
                            .and_then(|i| items.get(i).cloned())
                    }
                    _ => None,
                };
                for key in &args[1..] {
                    current = match current.take() {
                        Some(Value::Hash(h)) => hash_get_value(&h.borrow(), key).cloned(),
                        Some(Value::Array(a)) => {
                            if let Value::Int(idx) = key {
                                let items = a.borrow();
                                let idx = if *idx < 0 {
                                    items.len() as i64 + idx
                                } else {
                                    *idx
                                };
                                usize::try_from(idx)
                                    .ok()
                                    .and_then(|i| items.get(i).cloned())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if current.is_none() {
                        return Ok(Value::Null);
                    }
                }
                Ok(current.unwrap_or(Value::Null))
            }
            "sum_by" | "group_by" | "index_by" | "count_by" | "avg_by" | "uniq_by" | "max_by"
            | "min_by" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                use crate::interpreter::executor::calls::array_ops as ops;
                let items = arr.borrow();
                let f = &args[0];
                ops::check_field_arg(name, f)
                    .map_err(|msg| RuntimeError::type_error(&msg, span))?;
                Ok(match name {
                    "sum_by" => ops::sum_by(&items, f),
                    "group_by" => ops::group_by_field(&items, f),
                    "index_by" => ops::index_by(&items, f),
                    "avg_by" => ops::avg_by(&items, f),
                    "max_by" => ops::max_by(&items, f),
                    "min_by" => ops::min_by(&items, f),
                    "uniq_by" => Value::Array(Rc::new(RefCell::new(ops::uniq_by(&items, f)))),
                    _ => ops::count_by(&items, f),
                })
            }
            "filter_by" | "find_by" => {
                if args.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, args.len(), span));
                }
                use crate::interpreter::executor::calls::array_ops as ops;
                let items = arr.borrow();
                let (f, wanted) = (&args[0], &args[1]);
                ops::check_field_arg(name, f)
                    .map_err(|msg| RuntimeError::type_error(&msg, span))?;
                Ok(if name == "find_by" {
                    ops::find_by(&items, f, wanted)
                } else {
                    Value::Array(Rc::new(RefCell::new(ops::filter_by(&items, f, wanted))))
                })
            }
            "tally" | "avg" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                use crate::interpreter::executor::calls::array_ops as ops;
                let items = arr.borrow();
                Ok(if name == "avg" {
                    ops::avg(&items)
                } else {
                    ops::tally(&items)
                })
            }
            "pluck" => {
                if args.is_empty() {
                    return Err(RuntimeError::new(
                        "pluck() requires at least one field name or index",
                        span,
                    ));
                }
                let items = arr.borrow();
                let mut result = Vec::with_capacity(items.len());
                for item in items.iter() {
                    if args.len() == 1 {
                        result.push(extract_pluck_field(item, &args[0]));
                    } else {
                        let row: Vec<Value> =
                            args.iter().map(|k| extract_pluck_field(item, k)).collect();
                        result.push(Value::Array(Rc::new(RefCell::new(row))));
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "pick" => {
                if args.is_empty() {
                    return Err(RuntimeError::new(
                        "pick() requires at least one field name or index",
                        span,
                    ));
                }
                let items = arr.borrow();
                let Some(first) = items.first() else {
                    return Ok(Value::Null);
                };
                if args.len() == 1 {
                    return Ok(extract_pluck_field(first, &args[0]));
                }
                let row: Vec<Value> = args.iter().map(|k| extract_pluck_field(first, k)).collect();
                Ok(Value::Array(Rc::new(RefCell::new(row))))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Array".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }
}

/// Ruby-style "blank": null, empty string, empty array, empty hash.
fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.borrow().is_empty(),
        Value::Hash(h) => h.borrow().is_empty(),
        _ => false,
    }
}

/// Pull `key` out of a hash element (for `sort_by("field")`).
fn extract_hash_value(value: &Value, key: &HashKey) -> Value {
    match value {
        Value::Hash(hash) => hash.borrow().get(key).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// Total order used by `sort`/`sort_by` (numeric cross-compare, strings, else equal).
/// Delegates to the canonical comparator so `sort_by`, `max_by` and `min_by`
/// order values identically across both engines.
fn compare_sort_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    crate::interpreter::executor::calls::array_ops::compare_sort_values(a, b)
}

/// Field/index extraction shared by `pluck`/`pick`.
/// Delegates to the shared accessor so `pluck`, `pick` and every field-keyed
/// method read a record identically — including instances, which is what rows
/// from the ORM are.
fn extract_pluck_field(value: &Value, key: &Value) -> Value {
    crate::interpreter::executor::calls::array_ops::field_of(value, key)
}

fn extract_array_arg(
    value: &Value,
    method_name: &str,
    span: Span,
) -> Result<Vec<Value>, RuntimeError> {
    match value {
        Value::Array(arr) => Ok(arr.borrow().clone()),
        Value::Instance(inst) => match inst.borrow().fields.get("__value").cloned() {
            Some(Value::Array(arr)) => Ok(arr.borrow().clone()),
            _ => Err(RuntimeError::type_error(
                format!("{method_name} expects an array argument"),
                span,
            )),
        },
        _ => Err(RuntimeError::type_error(
            format!("{method_name} expects an array argument"),
            span,
        )),
    }
}
