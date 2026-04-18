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
            "map" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let cb = args[0].clone();
                let items: Vec<Value> = arr.borrow().clone();
                let mut result = Vec::with_capacity(items.len());
                for item in items {
                    let v = self.invoke_callable(cb.clone(), vec![item], span)?;
                    result.push(v);
                }
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "filter" | "select" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let cb = args[0].clone();
                let items: Vec<Value> = arr.borrow().clone();
                let mut result = Vec::new();
                for item in items {
                    let keep = self.invoke_callable(cb.clone(), vec![item.clone()], span)?;
                    if keep.is_truthy() {
                        result.push(item);
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "reduce" | "fold" => {
                if args.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, args.len(), span));
                }
                let cb = args[0].clone();
                let mut acc = args[1].clone();
                let items: Vec<Value> = arr.borrow().clone();
                for item in items {
                    acc = self.invoke_callable(cb.clone(), vec![acc, item], span)?;
                }
                Ok(acc)
            }
            "each" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let cb = args[0].clone();
                let items: Vec<Value> = arr.borrow().clone();
                for item in items.iter() {
                    self.invoke_callable(cb.clone(), vec![item.clone()], span)?;
                }
                Ok(Value::Array(arr.clone()))
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
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Array".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }
}
