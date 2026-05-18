//! Native array method dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Dispatch an array method call.
    pub fn vm_call_array_method(
        &mut self,
        arr: &Rc<RefCell<Vec<Value>>>,
        name: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
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
                if args.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, args.len(), span));
                }
                let cb = args[0].clone();
                let mut acc = args[1].clone();
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
                arr.borrow_mut()
                    .pop()
                    .ok_or_else(|| RuntimeError::type_error("pop on empty array", span))
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
                let mut reversed = arr.borrow().clone();
                reversed.reverse();
                Ok(Value::Array(Rc::new(RefCell::new(reversed))))
            }
            "uniq" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let items = arr.borrow();
                let mut seen = Vec::new();
                let mut result = Vec::new();
                for item in items.iter() {
                    if !seen.contains(item) {
                        seen.push(item.clone());
                        result.push(item.clone());
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "compact" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let items = arr.borrow();
                let result: Vec<Value> = items
                    .iter()
                    .filter(|v| !matches!(v, Value::Null))
                    .cloned()
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "flatten" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let items = arr.borrow();
                let mut result = Vec::new();
                for item in items.iter() {
                    match item {
                        Value::Array(inner) => {
                            result.extend(inner.borrow().iter().cloned());
                        }
                        _ => result.push(item.clone()),
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "contains" | "include?" => {
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
                    Value::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "join expects a string separator",
                            span,
                        ))
                    }
                };
                let items = arr.borrow();
                if items.is_empty() {
                    return Ok(Value::String(String::new()));
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
                Ok(Value::String(result))
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
            "to_string" | "to_s" => {
                let items = arr.borrow();
                if items.is_empty() {
                    return Ok(Value::String("[]".to_string()));
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
                Ok(Value::String(result))
            }
            // Universal methods
            "class" => Ok(Value::String("array".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(arr.borrow().is_empty())),
            "present?" => Ok(Value::Bool(!arr.borrow().is_empty())),
            "inspect" => {
                let items = arr.borrow();
                if items.is_empty() {
                    return Ok(Value::String("[]".to_string()));
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
                Ok(Value::String(result))
            }
            "is_a?" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let class_name = match &args[0] {
                    Value::String(s) => s.as_str(),
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
                    _ => return Err(RuntimeError::type_error("delete_at expects integer", span)),
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
                    _ => return Err(RuntimeError::type_error("rotate expects integer", span)),
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
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Array".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }
}
