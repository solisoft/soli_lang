//! Method call evaluation (continued) - Hash methods.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::executor::{ControlFlow, Interpreter, RuntimeResult};
use crate::interpreter::value::{Function, HashKey, Value};
use crate::span::Span;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::HashPairs;

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
            "length" | "len" => self.hash_length(entries, arguments, span),
            "to_string" => self.hash_to_string(entries, arguments, span),
            "to_json" => {
                match crate::interpreter::value::stringify_hash_entries_to_string(entries) {
                    Ok(json) => Ok(Value::String(json.into())),
                    Err(e) => Err(RuntimeError::General { message: e, span }),
                }
            }
            "keys" => self.hash_keys(entries, arguments, span),
            "values" => self.hash_values(entries, arguments, span),
            "has_key" => self.hash_has_key(entries, arguments, span),
            "delete" => self.hash_delete(entries, arguments, span),
            "merge" => self.hash_merge(entries, arguments, span),
            "entries" => self.hash_entries(entries, arguments, span),
            "clear" => self.hash_clear(entries, arguments, span),
            "set" => self.hash_set(entries, arguments, span),
            "empty?" => self.hash_empty(entries, arguments, span),
            "is_a?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let class_name = match &arguments[0] {
                    Value::String(s) => s.as_ref(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "is_a? expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(class_name == "hash" || class_name == "object"))
            }
            "each_key" => self.hash_each_key(entries, arguments, span),
            "each_value" => self.hash_each_value(entries, arguments, span),
            "keep_if" => self.hash_keep_if(entries, arguments, span),
            "delete_if" => self.hash_delete_if(entries, arguments, span),
            "all?" => self.hash_all(entries, arguments, span),
            "any?" => self.hash_any(entries, arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Hash".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    /// Invoke a `(key, value)` hash closure, reusing `env` across entries to
    /// avoid allocating a fresh `Environment` per iteration (matching the array
    /// iterators). Binds two params when the closure declares >= 2, else a
    /// single `[key, value]` pair. Maps control flow to a value
    /// (`Continue` -> Null, `Throw` -> error).
    #[inline]
    fn invoke_hash_kv(
        &mut self,
        func: &Rc<Function>,
        env: &Rc<RefCell<Environment>>,
        key_value: Value,
        value: Value,
        span: Span,
        ctx: &'static str,
    ) -> RuntimeResult<Value> {
        {
            let mut e = env.borrow_mut();
            if func.params.len() >= 2 {
                e.define_or_update(&func.params[0].name, key_value);
                e.define_or_update(&func.params[1].name, value);
            } else if func.params.len() == 1 {
                let pair = Value::Array(Rc::new(RefCell::new(vec![key_value, value])));
                e.define_or_update(&func.params[0].name, pair);
            }
        }
        match self.execute_block_in(&func.body, env.clone())? {
            ControlFlow::Return(v) | ControlFlow::Normal(v) => Ok(v),
            ControlFlow::Continue | ControlFlow::Break => Ok(Value::Null),
            ControlFlow::Throw(_) => {
                Err(RuntimeError::new(format!("Exception in hash {ctx}"), span))
            }
        }
    }

    /// Invoke a single-argument hash closure (`transform_values`/`transform_keys`/
    /// `each_key`/`each_value`), reusing `env`. Binds the closure's first param
    /// (or `it`) to `bound`.
    #[inline]
    fn invoke_hash_single(
        &mut self,
        func: &Rc<Function>,
        env: &Rc<RefCell<Environment>>,
        bound: Value,
        span: Span,
        ctx: &'static str,
    ) -> RuntimeResult<Value> {
        {
            let mut e = env.borrow_mut();
            let param = func.params.first().map(|p| p.name.as_str()).unwrap_or("it");
            e.define_or_update(param, bound);
        }
        match self.execute_block_in(&func.body, env.clone())? {
            ControlFlow::Return(v) | ControlFlow::Normal(v) => Ok(v),
            ControlFlow::Continue | ControlFlow::Break => Ok(Value::Null),
            ControlFlow::Throw(_) => {
                Err(RuntimeError::new(format!("Exception in hash {ctx}"), span))
            }
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

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        let mut result: HashPairs = HashPairs::default();
        for (key, value) in entries {
            let v =
                self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span, "map")?;
            if let Value::Array(arr) = v {
                let arr = arr.borrow();
                if arr.len() == 2 {
                    let hash_key = arr[0].to_hash_key().ok_or_else(|| {
                        RuntimeError::type_error("hash key must be hashable", span)
                    })?;
                    result.insert(hash_key, arr[1].clone());
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

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        let mut result: HashPairs = HashPairs::default();
        for (key, value) in entries {
            let result_value = self.invoke_hash_kv(
                &func,
                &call_env,
                key.to_value(),
                value.clone(),
                span,
                "filter",
            )?;
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

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        for (key, value) in entries {
            self.invoke_hash_kv(
                &func,
                &call_env,
                key.to_value(),
                value.clone(),
                span,
                "each",
            )?;
        }

        let result: HashPairs = entries.iter().cloned().collect();
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

        let entries_map: HashPairs = entries.iter().cloned().collect();
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

        let entries_map: HashPairs = entries.iter().cloned().collect();
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
        let mut result: HashPairs = HashPairs::default();
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

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        let mut result: HashPairs = HashPairs::default();
        for (key, value) in entries {
            let new_value =
                self.invoke_hash_single(&func, &call_env, value.clone(), span, "transform_values")?;
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

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        let mut result: HashPairs = HashPairs::default();
        for (key, value) in entries {
            let new_key =
                self.invoke_hash_single(&func, &call_env, key.to_value(), span, "transform_keys")?;
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

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        let mut result: HashPairs = HashPairs::default();
        for (key, value) in entries {
            let result_value = self.invoke_hash_kv(
                &func,
                &call_env,
                key.to_value(),
                value.clone(),
                span,
                "select",
            )?;
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

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        let mut result: HashPairs = HashPairs::default();
        for (key, value) in entries {
            let result_value = self.invoke_hash_kv(
                &func,
                &call_env,
                key.to_value(),
                value.clone(),
                span,
                "reject",
            )?;
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

        let entries_map: HashPairs = entries.iter().cloned().collect();
        let mut result: HashPairs = HashPairs::default();
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

        let mut result: HashPairs = HashPairs::default();
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
        let result: HashPairs = entries
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

        let entries_map: HashPairs = entries.iter().cloned().collect();
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
        Ok(Value::String(format!("[{}]", parts.join(", ")).into()))
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
        let entries_map: HashPairs = entries.iter().cloned().collect();
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
        let mut entries_map: HashPairs = entries.iter().cloned().collect();
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
        let mut result: HashPairs = entries.iter().cloned().collect();
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
        Ok(Value::Hash(Rc::new(RefCell::new(HashPairs::default()))))
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
        let mut result: HashPairs = entries.iter().cloned().collect();
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

    fn hash_each_key(
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
                    "each_key expects a function argument",
                    span,
                ))
            }
        };

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        for (key, _value) in entries {
            self.invoke_hash_single(&func, &call_env, key.to_value(), span, "each_key")?;
        }

        let result: HashPairs = entries.iter().cloned().collect();
        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_each_value(
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
                    "each_value expects a function argument",
                    span,
                ))
            }
        };

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        for (_key, value) in entries {
            self.invoke_hash_single(&func, &call_env, value.clone(), span, "each_value")?;
        }

        let result: HashPairs = entries.iter().cloned().collect();
        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_keep_if(
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
                    "keep_if expects a function argument",
                    span,
                ))
            }
        };

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        let mut result: HashPairs = HashPairs::default();
        for (key, value) in entries {
            let result_value = self.invoke_hash_kv(
                &func,
                &call_env,
                key.to_value(),
                value.clone(),
                span,
                "keep_if",
            )?;
            if result_value.is_truthy() {
                result.insert(key.clone(), value.clone());
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_delete_if(
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
                    "delete_if expects a function argument",
                    span,
                ))
            }
        };

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        let mut result: HashPairs = HashPairs::default();
        for (key, value) in entries {
            let result_value = self.invoke_hash_kv(
                &func,
                &call_env,
                key.to_value(),
                value.clone(),
                span,
                "delete_if",
            )?;
            if !result_value.is_truthy() {
                result.insert(key.clone(), value.clone());
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn hash_all(
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
                    "all? expects a function argument",
                    span,
                ))
            }
        };

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        for (key, value) in entries {
            let result_value = self.invoke_hash_kv(
                &func,
                &call_env,
                key.to_value(),
                value.clone(),
                span,
                "all?",
            )?;
            if !result_value.is_truthy() {
                return Ok(Value::Bool(false));
            }
        }

        Ok(Value::Bool(true))
    }

    fn hash_any(
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
                    "any? expects a function argument",
                    span,
                ))
            }
        };

        let call_env = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        for (key, value) in entries {
            let result_value = self.invoke_hash_kv(
                &func,
                &call_env,
                key.to_value(),
                value.clone(),
                span,
                "any?",
            )?;
            if result_value.is_truthy() {
                return Ok(Value::Bool(true));
            }
        }

        Ok(Value::Bool(false))
    }
}
