//! Method call evaluation for Array, Hash, QueryBuilder, and String.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{ControlFlow, Interpreter, RuntimeResult};
use crate::interpreter::value::{HashKey, Value, ValueMethod};
use crate::span::Span;

impl Interpreter {
    /// Call a method on a Value.
    pub(crate) fn call_method(
        &mut self,
        method: ValueMethod,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match *method.receiver {
            Value::Array(ref arr) => {
                match method.method_name.as_str() {
                    "push" | "pop" | "clear" => {
                        // Mutating methods need the original Rc<RefCell>
                        match method.method_name.as_str() {
                            "push" => {
                                if arguments.len() != 1 {
                                    return Err(RuntimeError::wrong_arity(
                                        1,
                                        arguments.len(),
                                        span,
                                    ));
                                }
                                arr.borrow_mut().push(arguments[0].clone());
                                Ok(Value::Null)
                            }
                            "pop" => {
                                if !arguments.is_empty() {
                                    return Err(RuntimeError::wrong_arity(
                                        0,
                                        arguments.len(),
                                        span,
                                    ));
                                }
                                arr.borrow_mut().pop().ok_or_else(|| {
                                    RuntimeError::type_error("pop on empty array", span)
                                })
                            }
                            "clear" => {
                                if !arguments.is_empty() {
                                    return Err(RuntimeError::wrong_arity(
                                        0,
                                        arguments.len(),
                                        span,
                                    ));
                                }
                                arr.borrow_mut().clear();
                                Ok(Value::Null)
                            }
                            _ => unreachable!(),
                        }
                    }
                    _ => {
                        let items = arr.borrow().clone();
                        self.call_array_method(&items, &method.method_name, arguments, span)
                    }
                }
            }
            Value::Hash(ref hash) => {
                match method.method_name.as_str() {
                    "set" | "delete" | "clear" => {
                        // Mutating methods
                        match method.method_name.as_str() {
                            "set" => {
                                if arguments.len() != 2 {
                                    return Err(RuntimeError::wrong_arity(
                                        2,
                                        arguments.len(),
                                        span,
                                    ));
                                }
                                let key = &arguments[0];
                                let value = arguments[1].clone();
                                let hash_key = key.to_hash_key().ok_or_else(|| {
                                    RuntimeError::type_error(
                                        format!("{} cannot be used as a hash key", key.type_name()),
                                        span,
                                    )
                                })?;
                                hash.borrow_mut().insert(hash_key, value.clone());
                                Ok(value)
                            }
                            "delete" => {
                                if arguments.len() != 1 {
                                    return Err(RuntimeError::wrong_arity(
                                        1,
                                        arguments.len(),
                                        span,
                                    ));
                                }
                                let key = &arguments[0];
                                let hash_key = match key.to_hash_key() {
                                    Some(k) => k,
                                    None => return Ok(Value::Null),
                                };
                                let deleted_value = hash.borrow_mut().shift_remove(&hash_key);
                                Ok(deleted_value.unwrap_or(Value::Null))
                            }
                            "clear" => {
                                if !arguments.is_empty() {
                                    return Err(RuntimeError::wrong_arity(
                                        0,
                                        arguments.len(),
                                        span,
                                    ));
                                }
                                hash.borrow_mut().clear();
                                Ok(Value::Null)
                            }
                            _ => unreachable!(),
                        }
                    }
                    _ => {
                        let entries: Vec<(HashKey, Value)> = hash
                            .borrow()
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        self.call_hash_method(&entries, &method.method_name, arguments, span)
                    }
                }
            }
            Value::QueryBuilder(qb) => {
                self.call_query_builder_method(qb, &method.method_name, arguments, span)
            }
            Value::String(s) => self.call_string_method(&s, &method.method_name, arguments, span),
            _ => Err(RuntimeError::type_error(
                format!("{} does not support methods", method.receiver.type_name()),
                span,
            )),
        }
    }

    /// Handle array methods.
    pub(crate) fn call_array_method(
        &mut self,
        items: &[Value],
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "map" => self.array_map(items, arguments, span),
            "filter" => self.array_filter(items, arguments, span),
            "each" => self.array_each(items, arguments, span),
            "reduce" => self.array_reduce(items, arguments, span),
            "find" => self.array_find(items, arguments, span),
            "any?" => self.array_any(items, arguments, span),
            "all?" => self.array_all(items, arguments, span),
            "sort" => self.array_sort(items, arguments, span),
            "reverse" => self.array_reverse(items, arguments, span),
            "uniq" => self.array_uniq(items, arguments, span),
            "compact" => self.array_compact(items, arguments, span),
            "flatten" => self.array_flatten(items, arguments, span),
            "first" => self.array_first(items, arguments, span),
            "last" => self.array_last(items, arguments, span),
            "empty?" => self.array_empty(items, arguments, span),
            "include?" => self.array_include(items, arguments, span),
            "sample" => self.array_sample(items, arguments, span),
            "shuffle" => self.array_shuffle(items, arguments, span),
            "take" => self.array_take(items, arguments, span),
            "drop" => self.array_drop(items, arguments, span),
            "zip" => self.array_zip(items, arguments, span),
            "sum" => self.array_sum(items, arguments, span),
            "min" => self.array_min(items, arguments, span),
            "max" => self.array_max(items, arguments, span),
            "push" => self.array_push(items, arguments, span),
            "pop" => self.array_pop(items, arguments, span),
            "clear" => self.array_clear(items, arguments, span),
            "get" => self.array_get(items, arguments, span),
            "length" => self.array_length(items, arguments, span),
            "to_string" => self.array_to_string(items, arguments, span),
            "join" => self.array_join(items, arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Array".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn array_map(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "map expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        let mut result = Vec::new();
        for item in items {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), item.clone());

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => result.push(v),
                ControlFlow::Normal(v) => result.push(v),
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array method", span));
                }
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_filter(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "filter expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        let mut result = Vec::new();
        for item in items {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), item.clone());

            let result_value = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array filter", span));
                }
            };

            if result_value.is_truthy() {
                result.push(item.clone());
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_each(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "each expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        for item in items {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), item.clone());

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array each", span));
                }
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(items.to_vec()))))
    }

    fn array_reduce(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "reduce expects a function argument",
                    span,
                ))
            }
        };

        let mut acc = if arguments.len() == 2 {
            arguments[1].clone()
        } else if !items.is_empty() {
            items[0].clone()
        } else {
            return Err(RuntimeError::type_error(
                "reduce on empty array requires initial value",
                span,
            ));
        };

        let start_idx = if arguments.len() == 2 { 0 } else { 1 };

        for item in items.iter().skip(start_idx) {
            let mut call_env = Environment::with_enclosing(func.closure.clone());

            if func.params.len() >= 2 {
                call_env.define(func.params[0].name.clone(), acc.clone());
                call_env.define(func.params[1].name.clone(), item.clone());
            } else if func.params.len() == 1 {
                let pair = Value::Array(Rc::new(RefCell::new(vec![acc.clone(), item.clone()])));
                call_env.define(func.params[0].name.clone(), pair);
            }

            acc = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array reduce", span));
                }
            };
        }

        Ok(acc)
    }

    fn array_find(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "find expects a function argument",
                    span,
                ))
            }
        };

        for item in items {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            let param_name = func
                .params
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "it".to_string());
            call_env.define(param_name, item.clone());

            let result_value = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array find", span));
                }
            };

            if result_value.is_truthy() {
                return Ok(item.clone());
            }
        }

        Ok(Value::Null)
    }

    fn array_any(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "any? expects a function argument",
                    span,
                ))
            }
        };

        for item in items {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            let param_name = func
                .params
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "it".to_string());
            call_env.define(param_name, item.clone());

            let result_value = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array any?", span));
                }
            };

            if result_value.is_truthy() {
                return Ok(Value::Bool(true));
            }
        }

        Ok(Value::Bool(false))
    }

    fn array_all(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "all? expects a function argument",
                    span,
                ))
            }
        };

        for item in items {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            let param_name = func
                .params
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "it".to_string());
            call_env.define(param_name, item.clone());

            let result_value = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array all?", span));
                }
            };

            if !result_value.is_truthy() {
                return Ok(Value::Bool(false));
            }
        }

        Ok(Value::Bool(true))
    }

    fn array_sort(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let mut result = items.to_vec();
        if arguments.len() > 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }

        if let Some(func_val) = arguments.first() {
            let func = match func_val {
                Value::Function(f) => f.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "sort expects a function argument",
                        span,
                    ))
                }
            };

            result.sort_by(|a, b| {
                let mut call_env = Environment::with_enclosing(func.closure.clone());

                if func.params.len() >= 2 {
                    call_env.define(func.params[0].name.clone(), a.clone());
                    call_env.define(func.params[1].name.clone(), b.clone());
                }

                match self.execute_block(&func.body, call_env) {
                    Ok(ControlFlow::Return(Value::Int(n)))
                    | Ok(ControlFlow::Normal(Value::Int(n))) => n.cmp(&0),
                    Ok(ControlFlow::Return(Value::Float(n)))
                    | Ok(ControlFlow::Normal(Value::Float(n))) => {
                        if n < 0.0 {
                            std::cmp::Ordering::Less
                        } else if n > 0.0 {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    }
                    _ => std::cmp::Ordering::Equal,
                }
            });
        } else {
            result.sort_by(|a, b| match (a, b) {
                (Value::Int(a), Value::Int(b)) => a.cmp(b),
                (Value::Float(a), Value::Float(b)) => {
                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                }
                (Value::String(a), Value::String(b)) => a.cmp(b),
                (Value::Int(a), Value::Float(b)) => (*a as f64)
                    .partial_cmp(b)
                    .unwrap_or(std::cmp::Ordering::Equal),
                (Value::Float(a), Value::Int(b)) => a
                    .partial_cmp(&(*b as f64))
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => std::cmp::Ordering::Equal,
            });
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_reverse(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut result = items.to_vec();
        result.reverse();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_uniq(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut result = Vec::new();
        for item in items {
            if !result.contains(item) {
                result.push(item.clone());
            }
        }
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_compact(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result: Vec<Value> = items
            .iter()
            .filter(|v| !matches!(v, Value::Null))
            .cloned()
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_flatten(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let depth = match arguments.len() {
            0 => None,
            1 => match &arguments[0] {
                Value::Int(n) if *n >= 0 => Some(*n as usize),
                _ => {
                    return Err(RuntimeError::type_error(
                        "flatten expects a non-negative integer",
                        span,
                    ))
                }
            },
            _ => return Err(RuntimeError::wrong_arity(1, arguments.len(), span)),
        };

        fn flatten_recursive(
            arr: &[Value],
            current_depth: usize,
            max_depth: Option<usize>,
        ) -> Vec<Value> {
            if let Some(max) = max_depth {
                if current_depth >= max {
                    return arr.to_vec();
                }
            }

            let mut result = Vec::new();
            for item in arr {
                if let Value::Array(inner) = item {
                    result.extend(flatten_recursive(
                        &inner.borrow(),
                        current_depth + 1,
                        max_depth,
                    ));
                } else {
                    result.push(item.clone());
                }
            }
            result
        }

        let result = flatten_recursive(items, 0, depth);
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_first(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(items.first().cloned().unwrap_or(Value::Null))
    }

    fn array_last(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(items.last().cloned().unwrap_or(Value::Null))
    }

    fn array_empty(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Bool(items.is_empty()))
    }

    fn array_include(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        Ok(Value::Bool(items.contains(&arguments[0])))
    }

    fn array_sample(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if items.is_empty() {
            return Ok(Value::Null);
        }
        use rand::seq::SliceRandom;
        use rand::thread_rng;
        let mut rng = thread_rng();
        Ok(items.choose(&mut rng).cloned().unwrap_or(Value::Null))
    }

    fn array_shuffle(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        use rand::seq::SliceRandom;
        use rand::thread_rng;
        let mut result = items.to_vec();
        let mut rng = thread_rng();
        result.shuffle(&mut rng);
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_take(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let n = match &arguments[0] {
            Value::Int(n) if *n >= 0 => *n as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "take expects a non-negative integer",
                    span,
                ))
            }
        };
        let result: Vec<Value> = items.iter().take(n).cloned().collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_drop(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let n = match &arguments[0] {
            Value::Int(n) if *n >= 0 => *n as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "drop expects a non-negative integer",
                    span,
                ))
            }
        };
        let result: Vec<Value> = items.iter().skip(n).cloned().collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_zip(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let other = match &arguments[0] {
            Value::Array(arr) => arr.borrow().clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "zip expects an array argument",
                    span,
                ))
            }
        };

        let result: Vec<Value> = items
            .iter()
            .zip(other.iter())
            .map(|(a, b)| Value::Array(Rc::new(RefCell::new(vec![a.clone(), b.clone()]))))
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_sum(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut total = 0.0;
        for item in items {
            match item {
                Value::Int(n) => total += *n as f64,
                Value::Float(n) => total += *n,
                _ => return Err(RuntimeError::type_error("sum expects numeric array", span)),
            }
        }
        Ok(Value::Float(total))
    }

    fn array_min(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if items.is_empty() {
            return Ok(Value::Null);
        }
        let mut min = &items[0];
        for item in items.iter().skip(1) {
            match (min, item) {
                (Value::Int(a), Value::Int(b)) if b < a => min = item,
                (Value::Float(a), Value::Float(b)) if b < a => min = item,
                (Value::String(a), Value::String(b)) if b < a => min = item,
                (Value::Int(a), Value::Float(b)) if *b < *a as f64 => min = item,
                (Value::Float(a), Value::Int(b)) if (*b as f64) < *a => min = item,
                _ => {}
            }
        }
        Ok(min.clone())
    }

    fn array_max(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if items.is_empty() {
            return Ok(Value::Null);
        }
        let mut max = &items[0];
        for item in items.iter().skip(1) {
            match (max, item) {
                (Value::Int(a), Value::Int(b)) if b > a => max = item,
                (Value::Float(a), Value::Float(b)) if b > a => max = item,
                (Value::String(a), Value::String(b)) if b > a => max = item,
                (Value::Int(a), Value::Float(b)) if *b > *a as f64 => max = item,
                (Value::Float(a), Value::Int(b)) if (*b as f64) > *a => max = item,
                _ => {}
            }
        }
        Ok(max.clone())
    }

    fn array_push(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let mut new_arr = items.to_vec();
        new_arr.push(arguments[0].clone());
        Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
    }

    fn array_pop(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut new_arr = items.to_vec();
        new_arr.pop();
        Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
    }

    fn array_clear(
        &mut self,
        _items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
    }

    fn array_get(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let idx = match &arguments[0] {
            Value::Int(n) => *n,
            _ => {
                return Err(RuntimeError::type_error(
                    "get expects an integer index",
                    span,
                ))
            }
        };
        let idx_usize = if idx < 0 {
            (items.len() as i64 + idx) as usize
        } else {
            idx as usize
        };
        items
            .get(idx_usize)
            .cloned()
            .ok_or(RuntimeError::IndexOutOfBounds {
                index: idx,
                length: items.len(),
                span,
            })
    }

    fn array_length(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Int(items.len() as i64))
    }

    fn array_to_string(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
        Ok(Value::String(format!("[{}]", parts.join(", "))))
    }

    fn array_join(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let delim = match &arguments[0] {
            Value::String(d) => d.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "join expects a string delimiter",
                    span,
                ))
            }
        };
        let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
        Ok(Value::String(parts.join(&delim)))
    }
}
