//! Method call evaluation (continued) - Hash methods.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::executor::{ControlFlow, Interpreter, RuntimeResult};
use crate::interpreter::value::{HashKey, Value};
use crate::span::Span;

use crate::interpreter::environment::Environment;
use indexmap::IndexMap;

impl Interpreter {
    /// Handle hash methods.
    pub(crate) fn call_hash_method(
        &mut self,
        entries: &[(HashKey, Value)],
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "map" => self.hash_map(entries, arguments, span),
            "filter" => self.hash_filter(entries, arguments, span),
            "each" => self.hash_each(entries, arguments, span),
            "get" => self.hash_get(entries, arguments, span),
            "fetch" => self.hash_fetch(entries, arguments, span),
            "invert" => self.hash_invert(entries, arguments, span),
            "transform_values" => self.hash_transform_values(entries, arguments, span),
            "transform_keys" => self.hash_transform_keys(entries, arguments, span),
            "select" => self.hash_select(entries, arguments, span),
            "reject" => self.hash_reject(entries, arguments, span),
            "slice" => self.hash_slice(entries, arguments, span),
            "except" => self.hash_except(entries, arguments, span),
            "compact" => self.hash_compact(entries, arguments, span),
            "dig" => self.hash_dig(entries, arguments, span),
            "length" => self.hash_length(entries, arguments, span),
            "to_string" => self.hash_to_string(entries, arguments, span),
            "keys" => self.hash_keys(entries, arguments, span),
            "values" => self.hash_values(entries, arguments, span),
            "has_key" => self.hash_has_key(entries, arguments, span),
            "delete" => self.hash_delete(entries, arguments, span),
            "merge" => self.hash_merge(entries, arguments, span),
            "entries" => self.hash_entries(entries, arguments, span),
            "clear" => self.hash_clear(entries, arguments, span),
            "set" => self.hash_set(entries, arguments, span),
            "empty?" => self.hash_empty(entries, arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Hash".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn hash_map(
        &mut self,
        entries: &[(HashKey, Value)],
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

        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for (key, value) in entries {
            let mut call_env = Environment::with_enclosing(func.closure.clone());

            let key_value = key.to_value();
            if func.params.len() >= 2 {
                call_env.define(func.params[0].name.clone(), key_value.clone());
                call_env.define(func.params[1].name.clone(), value.clone());
            } else if func.params.len() == 1 {
                let pair = Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                call_env.define(func.params[0].name.clone(), pair);
            }

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) | ControlFlow::Normal(v) => {
                    if let Value::Array(arr) = v {
                        let arr = arr.borrow();
                        if arr.len() == 2 {
                            let new_key = arr[0].clone();
                            let new_val = arr[1].clone();
                            let hash_key = new_key.to_hash_key().ok_or_else(|| {
                                RuntimeError::type_error("hash key must be hashable", span)
                            })?;
                            result.insert(hash_key, new_val);
                        }
                    }
                }
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in hash map", span));
                }
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_filter(
        &mut self,
        entries: &[(HashKey, Value)],
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

        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for (key, value) in entries {
            let mut call_env = Environment::with_enclosing(func.closure.clone());

            let key_value = key.to_value();
            if func.params.len() >= 2 {
                call_env.define(func.params[0].name.clone(), key_value.clone());
                call_env.define(func.params[1].name.clone(), value.clone());
            } else if func.params.len() == 1 {
                let pair = Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                call_env.define(func.params[0].name.clone(), pair);
            }

            let result_value = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in hash filter", span));
                }
            };

            if result_value.is_truthy() {
                result.insert(key.clone(), value.clone());
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_each(
        &mut self,
        entries: &[(HashKey, Value)],
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

        for (key, value) in entries {
            let mut call_env = Environment::with_enclosing(func.closure.clone());

            let key_value = key.to_value();
            if func.params.len() >= 2 {
                call_env.define(func.params[0].name.clone(), key_value.clone());
                call_env.define(func.params[1].name.clone(), value.clone());
            } else if func.params.len() == 1 {
                let pair = Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                call_env.define(func.params[0].name.clone(), pair);
            }

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in hash each", span));
                }
            }
        }

        let result: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_get(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let key = &arguments[0];
        let hash_key = key.to_hash_key().ok_or_else(|| {
            RuntimeError::type_error(
                format!("{} cannot be used as a hash key", key.type_name()),
                span,
            )
        })?;
        let default = arguments.get(1).cloned().unwrap_or(Value::Null);

        let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        Ok(entries_map.get(&hash_key).cloned().unwrap_or(default))
    }

    fn hash_fetch(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let key = &arguments[0];
        let hash_key = key.to_hash_key().ok_or_else(|| {
            RuntimeError::type_error(
                format!("{} cannot be used as a hash key", key.type_name()),
                span,
            )
        })?;

        let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        if let Some(v) = entries_map.get(&hash_key) {
            Ok(v.clone())
        } else if let Some(default) = arguments.get(1) {
            Ok(default.clone())
        } else {
            Err(RuntimeError::type_error(
                format!("key not found: {:?}", key),
                span,
            ))
        }
    }

    fn hash_invert(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for (k, v) in entries {
            let new_key = v.to_hash_key().ok_or_else(|| {
                RuntimeError::type_error(
                    format!("{} cannot be used as a hash key", v.type_name()),
                    span,
                )
            })?;
            result.insert(new_key, k.to_value());
        }
        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_transform_values(
        &mut self,
        entries: &[(HashKey, Value)],
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
                    "transform_values expects a function argument",
                    span,
                ))
            }
        };

        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for (key, value) in entries {
            let mut call_env = Environment::with_enclosing(func.closure.clone());

            let param_name = func
                .params
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "it".to_string());
            call_env.define(param_name, value.clone());

            let new_value = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new(
                        "Exception in hash transform_values",
                        span,
                    ));
                }
            };
            result.insert(key.clone(), new_value);
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_transform_keys(
        &mut self,
        entries: &[(HashKey, Value)],
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
                    "transform_keys expects a function argument",
                    span,
                ))
            }
        };

        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for (key, value) in entries {
            let mut call_env = Environment::with_enclosing(func.closure.clone());

            let param_name = func
                .params
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "it".to_string());
            call_env.define(param_name, key.to_value());

            let new_key = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in hash transform_keys", span));
                }
            };

            let new_hash_key = new_key.to_hash_key().ok_or_else(|| {
                RuntimeError::type_error("transformed key must be hashable", span)
            })?;
            result.insert(new_hash_key, value.clone());
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_select(
        &mut self,
        entries: &[(HashKey, Value)],
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
                    "select expects a function argument",
                    span,
                ))
            }
        };

        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for (key, value) in entries {
            let mut call_env = Environment::with_enclosing(func.closure.clone());

            let key_value = key.to_value();
            if func.params.len() >= 2 {
                call_env.define(func.params[0].name.clone(), key_value.clone());
                call_env.define(func.params[1].name.clone(), value.clone());
            } else if func.params.len() == 1 {
                let pair = Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                call_env.define(func.params[0].name.clone(), pair);
            }

            let result_value = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in hash select", span));
                }
            };

            if result_value.is_truthy() {
                result.insert(key.clone(), value.clone());
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_reject(
        &mut self,
        entries: &[(HashKey, Value)],
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
                    "reject expects a function argument",
                    span,
                ))
            }
        };

        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for (key, value) in entries {
            let mut call_env = Environment::with_enclosing(func.closure.clone());

            let key_value = key.to_value();
            if func.params.len() >= 2 {
                call_env.define(func.params[0].name.clone(), key_value.clone());
                call_env.define(func.params[1].name.clone(), value.clone());
            } else if func.params.len() == 1 {
                let pair = Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                call_env.define(func.params[0].name.clone(), pair);
            }

            let result_value = match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in hash reject", span));
                }
            };

            if !result_value.is_truthy() {
                result.insert(key.clone(), value.clone());
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_slice(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let keys_arr = match &arguments[0] {
            Value::Array(arr) => arr.borrow().clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "slice expects an array of keys",
                    span,
                ))
            }
        };

        let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for key in keys_arr {
            let hash_key = key.to_hash_key().ok_or_else(|| {
                RuntimeError::type_error(
                    format!("{} cannot be used as a hash key", key.type_name()),
                    span,
                )
            })?;
            if let Some(v) = entries_map.get(&hash_key) {
                result.insert(hash_key, v.clone());
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_except(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let keys_arr = match &arguments[0] {
            Value::Array(arr) => arr.borrow().clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "except expects an array of keys",
                    span,
                ))
            }
        };

        let exclude_keys: HashSet<HashKey> =
            keys_arr.iter().filter_map(|k| k.to_hash_key()).collect();

        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        for (k, v) in entries {
            if !exclude_keys.contains(k) {
                result.insert(k.clone(), v.clone());
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_compact(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result: IndexMap<HashKey, Value> = entries
            .iter()
            .filter(|(_, v)| !matches!(v, Value::Null))
            .cloned()
            .collect();
        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_dig(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }

        let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        let mut current: Option<Value> = Some(Value::Hash(Rc::new(RefCell::new(entries_map))));
        for key in arguments {
            current = match current.take() {
                Some(Value::Hash(hash)) => {
                    let hash_key = key.to_hash_key();
                    if let Some(hash_key) = hash_key {
                        let hash_ref = hash.borrow();
                        hash_ref.get(&hash_key).cloned()
                    } else {
                        None
                    }
                }
                Some(Value::Array(arr)) => {
                    if let Value::Int(idx) = key {
                        let arr_ref = arr.borrow();
                        let idx = if idx < 0 {
                            arr_ref.len() as i64 + idx
                        } else {
                            idx
                        };
                        let idx = idx as usize;
                        if idx < arr_ref.len() {
                            Some(arr_ref[idx].clone())
                        } else {
                            None
                        }
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

    fn hash_length(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Int(entries.len() as i64))
    }

    fn hash_to_string(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let parts: Vec<String> = entries
            .iter()
            .map(|(k, v)| format!("{} => {}", k.to_value(), v))
            .collect();
        Ok(Value::String(format!("[{}]", parts.join(", "))))
    }

    fn hash_keys(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let keys: Vec<Value> = entries.iter().map(|(k, _)| k.to_value()).collect();
        Ok(Value::Array(Rc::new(RefCell::new(keys))))
    }

    fn hash_values(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let values: Vec<Value> = entries.iter().map(|(_, v)| v.clone()).collect();
        Ok(Value::Array(Rc::new(RefCell::new(values))))
    }

    fn hash_has_key(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let key = &arguments[0];
        let hash_key = match key.to_hash_key() {
            Some(k) => k,
            None => return Ok(Value::Bool(false)),
        };
        let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        Ok(Value::Bool(entries_map.contains_key(&hash_key)))
    }

    fn hash_delete(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let key = &arguments[0];
        let hash_key = match key.to_hash_key() {
            Some(k) => k,
            None => return Ok(Value::Null),
        };
        let mut entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        Ok(entries_map.shift_remove(&hash_key).unwrap_or(Value::Null))
    }

    fn hash_merge(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let other = match &arguments[0] {
            Value::Hash(h) => h.borrow().clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "merge expects a hash argument",
                    span,
                ))
            }
        };
        let mut result: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        for (k, v) in other {
            result.insert(k, v);
        }
        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_entries(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let pairs: Vec<Value> = entries
            .iter()
            .map(|(k, v)| Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()]))))
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(pairs))))
    }

    fn hash_clear(
        &mut self,
        _entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Hash(Rc::new(RefCell::new(IndexMap::new()))))
    }

    fn hash_set(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let key = &arguments[0];
        let value = arguments[1].clone();
        let hash_key = key.to_hash_key().ok_or_else(|| {
            RuntimeError::type_error(
                format!("{} cannot be used as a hash key", key.type_name()),
                span,
            )
        })?;
        let mut result: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
        result.insert(hash_key, value);
        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_empty(
        &mut self,
        entries: &[(HashKey, Value)],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Bool(entries.is_empty()))
    }
}
