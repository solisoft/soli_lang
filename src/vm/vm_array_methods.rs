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
        &self,
        arr: &Rc<RefCell<Vec<Value>>>,
        name: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match name {
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
            "length" | "len" => {
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
                let mut total = 0i64;
                for item in items.iter() {
                    if let Value::Int(n) = item {
                        total += n;
                    }
                }
                Ok(Value::Int(total))
            }
            "min" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let items = arr.borrow();
                let mut min: Option<i64> = None;
                for item in items.iter() {
                    if let Value::Int(n) = item {
                        min = Some(min.map_or(*n, |m: i64| m.min(*n)));
                    }
                }
                Ok(min.map_or(Value::Null, Value::Int))
            }
            "max" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let items = arr.borrow();
                let mut max: Option<i64> = None;
                for item in items.iter() {
                    if let Value::Int(n) = item {
                        max = Some(max.map_or(*n, |m: i64| m.max(*n)));
                    }
                }
                Ok(max.map_or(Value::Null, Value::Int))
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
                let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
                Ok(Value::String(parts.join(sep)))
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
                let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
                Ok(Value::String(format!("[{}]", parts.join(", "))))
            }
            // Universal methods
            "class" => Ok(Value::String("array".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(arr.borrow().is_empty())),
            "present?" => Ok(Value::Bool(!arr.borrow().is_empty())),
            "inspect" => {
                let items = arr.borrow();
                let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
                Ok(Value::String(format!("[{}]", parts.join(", "))))
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
