//! Method call evaluation for Array, Hash, QueryBuilder, and String.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{ControlFlow, Interpreter, RuntimeResult};
use crate::interpreter::value::{
    hash_contains_value, hash_get_value, HashKey, HashPairs, Value, ValueMethod,
};
use crate::span::Span;

impl Interpreter {
    pub(crate) fn call_hash_method_on_rc(
        &mut self,
        hash: &Rc<RefCell<HashPairs>>,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "set" | "delete" | "clear" => match method_name {
                "set" => {
                    if arguments.len() != 2 {
                        return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                    }
                    let key = &arguments[0];
                    let value = arguments[1].clone();
                    match key {
                        Value::String(s) => {
                            let mut hash_ref = hash.borrow_mut();
                            if let Some((_, _, existing)) =
                                hash_ref.get_full_mut(&crate::interpreter::value::StrKey(s))
                            {
                                *existing = value.clone();
                            } else {
                                hash_ref.insert(HashKey::String(s.clone()), value.clone());
                            }
                        }
                        _ => {
                            let hash_key = key.to_hash_key().ok_or_else(|| {
                                RuntimeError::type_error(
                                    format!("{} cannot be used as a hash key", key.type_name()),
                                    span,
                                )
                            })?;
                            hash.borrow_mut().insert(hash_key, value.clone());
                        }
                    }
                    Ok(value)
                }
                "delete" => {
                    if arguments.len() != 1 {
                        return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                    }
                    let key = &arguments[0];
                    let deleted_value = match key {
                        Value::String(s) => hash
                            .borrow_mut()
                            .shift_remove(&crate::interpreter::value::StrKey(s)),
                        _ => {
                            let hash_key = match key.to_hash_key() {
                                Some(k) => k,
                                None => return Ok(Value::Null),
                            };
                            hash.borrow_mut().shift_remove(&hash_key)
                        }
                    };
                    Ok(deleted_value.unwrap_or(Value::Null))
                }
                "clear" => {
                    if !arguments.is_empty() {
                        return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                    }
                    hash.borrow_mut().clear();
                    Ok(Value::Null)
                }
                _ => unreachable!(),
            },
            _ => {
                {
                    let entries = hash.borrow();
                    if let Some(result) =
                        self.call_hash_method_borrowed(&entries, method_name, &arguments, span)
                    {
                        return result;
                    }
                }
                let entries: Vec<(HashKey, Value)> = hash
                    .borrow()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                self.call_hash_method(&entries, method_name, arguments, span)
            }
        }
    }

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
                        {
                            let items = arr.borrow();
                            if let Some(result) = self.call_array_method_borrowed(
                                &items,
                                &method.method_name,
                                &arguments,
                                span,
                            ) {
                                return result;
                            }
                        }
                        let items = arr.borrow().clone();
                        self.call_array_method(&items, &method.method_name, arguments, span)
                    }
                }
            }
            Value::Hash(ref hash) => {
                self.call_hash_method_on_rc(hash, &method.method_name, arguments, span)
            }
            Value::QueryBuilder(qb) => {
                self.call_query_builder_method(qb, &method.method_name, arguments, span)
            }
            Value::String(s) => {
                if let Some(result) =
                    self.call_string_method_borrowed(&s, &method.method_name, &arguments, span)
                {
                    result
                } else {
                    self.call_string_method(&s, &method.method_name, arguments, span)
                }
            }
            Value::Int(n) => self.call_int_method(n, &method.method_name, arguments, span),
            Value::Float(n) => self.call_float_method(n, &method.method_name, arguments, span),
            Value::Bool(b) => self.call_bool_method(b, &method.method_name, arguments, span),
            Value::Null => self.call_null_method(&method.method_name, arguments, span),
            Value::Decimal(d) => self.call_decimal_method(d, &method.method_name, arguments, span),
            Value::Class(ref class) => match (class.name.as_str(), method.method_name.as_str()) {
                ("Cache", "fetch") => self.cache_fetch(arguments, span),
                _ => Err(RuntimeError::type_error(
                    format!("{}.{}() is not supported", class.name, method.method_name),
                    span,
                )),
            },
            _ => Err(RuntimeError::type_error(
                format!("{} does not support methods", method.receiver.type_name()),
                span,
            )),
        }
    }

    fn call_array_method_borrowed(
        &self,
        items: &[Value],
        method_name: &str,
        arguments: &[Value],
        span: Span,
    ) -> Option<RuntimeResult<Value>> {
        match method_name {
            "reverse" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut result = items.to_vec();
                result.reverse();
                Some(Ok(Value::Array(Rc::new(RefCell::new(result)))))
            }
            "uniq" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut result = Vec::with_capacity(items.len());
                for item in items {
                    if !result.contains(item) {
                        result.push(item.clone());
                    }
                }
                Some(Ok(Value::Array(Rc::new(RefCell::new(result)))))
            }
            "compact" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let result: Vec<Value> = items
                    .iter()
                    .filter(|v| !matches!(v, Value::Null))
                    .cloned()
                    .collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(result)))))
            }
            "first" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(items.first().cloned().unwrap_or(Value::Null)))
            }
            "last" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(items.last().cloned().unwrap_or(Value::Null)))
            }
            "empty?" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(items.is_empty())))
            }
            "includes?" | "contains" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(items.contains(&arguments[0]))))
            }
            "get" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let idx = match &arguments[0] {
                    Value::Int(n) => *n,
                    _ => {
                        return Some(Err(RuntimeError::type_error(
                            "get expects an integer index",
                            span,
                        )))
                    }
                };
                let idx_usize = if idx < 0 {
                    (items.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                Some(
                    items
                        .get(idx_usize)
                        .cloned()
                        .ok_or(RuntimeError::IndexOutOfBounds {
                            index: idx,
                            length: items.len(),
                            span,
                        }),
                )
            }
            "length" | "len" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Int(items.len() as i64)))
            }
            "to_string" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut total_len = 2;
                for (i, value) in items.iter().enumerate() {
                    total_len += value.display_len();
                    if i > 0 {
                        total_len += 2;
                    }
                }
                let mut result = String::with_capacity(total_len);
                result.push('[');
                for (i, value) in items.iter().enumerate() {
                    if i > 0 {
                        result.push_str(", ");
                    }
                    value.write_to_string(&mut result);
                }
                result.push(']');
                Some(Ok(Value::String(result)))
            }
            "join" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let delim = match &arguments[0] {
                    Value::String(d) => d.as_str(),
                    _ => {
                        return Some(Err(RuntimeError::type_error(
                            "join expects a string delimiter",
                            span,
                        )))
                    }
                };
                let mut total_len = delim.len().saturating_mul(items.len().saturating_sub(1));
                for value in items {
                    total_len += value.display_len();
                }
                let mut result = String::with_capacity(total_len);
                for (i, value) in items.iter().enumerate() {
                    if i > 0 {
                        result.push_str(delim);
                    }
                    value.write_to_string(&mut result);
                }
                Some(Ok(Value::String(result)))
            }
            _ => None,
        }
    }

    fn call_hash_method_borrowed(
        &self,
        entries: &HashPairs,
        method_name: &str,
        arguments: &[Value],
        span: Span,
    ) -> Option<RuntimeResult<Value>> {
        match method_name {
            "get" => {
                if arguments.is_empty() || arguments.len() > 2 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let found = hash_get_value(entries, &arguments[0]).cloned();
                Some(Ok(match found {
                    Some(v) => v,
                    None => arguments.get(1).cloned().unwrap_or(Value::Null),
                }))
            }
            "fetch" => {
                if arguments.is_empty() || arguments.len() > 2 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                if let Some(value) = hash_get_value(entries, &arguments[0]) {
                    Some(Ok(value.clone()))
                } else if let Some(default) = arguments.get(1) {
                    Some(Ok(default.clone()))
                } else {
                    Some(Err(RuntimeError::type_error(
                        format!("key not found: {:?}", arguments[0]),
                        span,
                    )))
                }
            }
            "length" | "len" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Int(entries.len() as i64)))
            }
            "keys" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let keys: Vec<Value> = entries.keys().map(HashKey::to_value).collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(keys)))))
            }
            "values" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let values: Vec<Value> = entries.values().cloned().collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(values)))))
            }
            "entries" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let pairs: Vec<Value> = entries
                    .iter()
                    .map(|(k, v)| {
                        Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                    })
                    .collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(pairs)))))
            }
            "merge" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                match &arguments[0] {
                    Value::Hash(other) => {
                        let mut merged = entries.clone();
                        for (k, v) in other.borrow().iter() {
                            merged.insert(k.clone(), v.clone());
                        }
                        Some(Ok(Value::Hash(Rc::new(RefCell::new(merged)))))
                    }
                    _ => Some(Err(RuntimeError::type_error(
                        "merge expects a hash argument",
                        span,
                    ))),
                }
            }
            "compact" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut compacted = HashPairs::with_capacity_and_hasher(
                    entries.len(),
                    ahash::RandomState::default(),
                );
                for (k, v) in entries.iter() {
                    if !matches!(v, Value::Null) {
                        compacted.insert(k.clone(), v.clone());
                    }
                }
                Some(Ok(Value::Hash(Rc::new(RefCell::new(compacted)))))
            }
            "invert" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut inverted = HashPairs::with_capacity_and_hasher(
                    entries.len(),
                    ahash::RandomState::default(),
                );
                for (k, v) in entries.iter() {
                    let new_key = match v.to_hash_key() {
                        Some(key) => key,
                        None => {
                            return Some(Err(RuntimeError::type_error(
                                format!("{} cannot be used as a hash key", v.type_name()),
                                span,
                            )))
                        }
                    };
                    inverted.insert(new_key, k.to_value());
                }
                Some(Ok(Value::Hash(Rc::new(RefCell::new(inverted)))))
            }
            "has_key" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(hash_contains_value(entries, &arguments[0]))))
            }
            "empty?" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(entries.is_empty())))
            }
            "to_string" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut total_len = 2;
                for (i, (k, v)) in entries.iter().enumerate() {
                    total_len += k.display_len();
                    total_len += 4 + v.display_len();
                    if i > 0 {
                        total_len += 2;
                    }
                }
                let mut result = String::with_capacity(total_len);
                result.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        result.push_str(", ");
                    }
                    k.write_key_to_string(&mut result);
                    result.push_str(" => ");
                    v.write_to_string(&mut result);
                }
                result.push('}');
                Some(Ok(Value::String(result)))
            }
            _ => None,
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
            "sort_by" => self.array_sort_by(items, arguments, span),
            "reverse" => self.array_reverse(items, arguments, span),
            "uniq" => self.array_uniq(items, arguments, span),
            "compact" => self.array_compact(items, arguments, span),
            "flatten" => self.array_flatten(items, arguments, span),
            "first" => self.array_first(items, arguments, span),
            "last" => self.array_last(items, arguments, span),
            "empty?" => self.array_empty(items, arguments, span),
            "includes?" | "contains" => self.array_include(items, arguments, span),
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
            "length" | "len" => self.array_length(items, arguments, span),
            "to_string" => self.array_to_string(items, arguments, span),
            "to_json" => match crate::interpreter::value::stringify_array_to_string(items) {
                Ok(json) => Ok(Value::String(json)),
                Err(e) => Err(RuntimeError::General { message: e, span }),
            },
            "join" => self.array_join(items, arguments, span),
            "is_a?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let class_name = match &arguments[0] {
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

    fn array_sort_by(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }

        let mut result = items.to_vec();

        match &arguments[0] {
            Value::String(key) => {
                let hash_key = HashKey::String(key.clone());
                result.sort_by(|a, b| {
                    let val_a = Self::extract_hash_value(a, &hash_key);
                    let val_b = Self::extract_hash_value(b, &hash_key);
                    Self::compare_sort_values(&val_a, &val_b)
                });
            }
            Value::Function(func) => {
                let func = func.clone();
                let param_name = func
                    .params
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "it".to_string());

                // Extract key values for each item using the function
                let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(result.len());
                for item in &result {
                    let mut call_env = Environment::with_enclosing(func.closure.clone());
                    call_env.define(param_name.clone(), item.clone());

                    let key_val = match self.execute_block(&func.body, call_env) {
                        Ok(ControlFlow::Return(v)) | Ok(ControlFlow::Normal(v)) => v,
                        _ => Value::Null,
                    };
                    keyed.push((item.clone(), key_val));
                }

                keyed.sort_by(|a, b| Self::compare_sort_values(&a.1, &b.1));
                result = keyed.into_iter().map(|(item, _)| item).collect();
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "sort_by expects a string key or a function argument",
                    span,
                ))
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn extract_hash_value(value: &Value, key: &HashKey) -> Value {
        match value {
            Value::Hash(hash) => hash.borrow().get(key).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    fn compare_sort_values(a: &Value, b: &Value) -> std::cmp::Ordering {
        match (a, b) {
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
        }
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
                Value::Decimal(d) => total += d.to_f64(),
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
                (Value::Decimal(a), Value::Decimal(b)) if b.to_f64() < a.to_f64() => min = item,
                (Value::Int(a), Value::Decimal(b)) if b.to_f64() < *a as f64 => min = item,
                (Value::Decimal(a), Value::Int(b)) if (*b as f64) < a.to_f64() => min = item,
                (Value::Float(a), Value::Decimal(b)) if b.to_f64() < *a => min = item,
                (Value::Decimal(a), Value::Float(b)) if *b < a.to_f64() => min = item,
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
                (Value::Decimal(a), Value::Decimal(b)) if b.to_f64() > a.to_f64() => max = item,
                (Value::Int(a), Value::Decimal(b)) if b.to_f64() > *a as f64 => max = item,
                (Value::Decimal(a), Value::Int(b)) if (*b as f64) > a.to_f64() => max = item,
                (Value::Float(a), Value::Decimal(b)) if b.to_f64() > *a => max = item,
                (Value::Decimal(a), Value::Float(b)) if *b > a.to_f64() => max = item,
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

    fn cache_fetch(&mut self, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        use crate::interpreter::builtins::cache::{cache_get_impl, cache_set_impl};

        if arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(1, 0, span));
        }
        let key = match &arguments[0] {
            Value::String(s) => s.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "Cache.fetch() expects string key",
                    span,
                ))
            }
        };

        // Parse optional TTL and block from remaining args
        let mut ttl: Option<u64> = None;
        let mut block = None;
        for arg in arguments.iter().skip(1) {
            match arg {
                Value::Int(i) => ttl = Some(*i as u64),
                Value::Function(f) => block = Some(f.clone()),
                _ => {}
            }
        }

        // Check cache
        let cached =
            cache_get_impl(&key).map_err(|e| RuntimeError::General { message: e, span })?;
        if !matches!(cached, Value::Null) {
            return Ok(cached);
        }

        // Cache miss — no block means return null
        let func = match block {
            Some(f) => f,
            None => return Ok(Value::Null),
        };

        // Execute block
        let call_env = Environment::with_enclosing(func.closure.clone());
        let result = match self.execute_block(&func.body, call_env)? {
            ControlFlow::Return(v) | ControlFlow::Normal(v) => v,
            ControlFlow::Throw(_) => {
                return Err(RuntimeError::new("Exception in Cache.fetch block", span))
            }
        };

        // Store in cache
        cache_set_impl(&key, &result, ttl)
            .map_err(|e| RuntimeError::General { message: e, span })?;

        Ok(result)
    }
}
