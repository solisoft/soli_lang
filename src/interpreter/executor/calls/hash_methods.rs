//! Method call evaluation (continued) - Hash methods.

use std::cell::RefCell;
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
            "transform_values" => self.hash_transform_values(entries, arguments, span),
            "transform_keys" => self.hash_transform_keys(entries, arguments, span),
            "select" => self.hash_select(entries, arguments, span),
            "reject" => self.hash_reject(entries, arguments, span),
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
            ControlFlow::Throw(v) => {
                // Carry the thrown value out of the callback.
                // Replacing it with a generic message destroyed both the
                // payload and the message the author wrote.
                Err(RuntimeError::Thrown { value: v, span })
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
    ) -> RuntimeResult<Value> {
        {
            let mut e = env.borrow_mut();
            let param = func.params.first().map(|p| p.name.as_str()).unwrap_or("it");
            e.define_or_update(param, bound);
        }
        match self.execute_block_in(&func.body, env.clone())? {
            ControlFlow::Return(v) | ControlFlow::Normal(v) => Ok(v),
            ControlFlow::Continue | ControlFlow::Break => Ok(Value::Null),
            ControlFlow::Throw(v) => {
                // Carry the thrown value out of the callback.
                // Replacing it with a generic message destroyed both the
                // payload and the message the author wrote.
                Err(RuntimeError::Thrown { value: v, span })
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
            let v = self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
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
            let result_value =
                self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
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
            self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
        }

        let result: HashPairs = entries.iter().cloned().collect();
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
            let new_value = self.invoke_hash_single(&func, &call_env, value.clone(), span)?;
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
            let new_key = self.invoke_hash_single(&func, &call_env, key.to_value(), span)?;
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
            let result_value =
                self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
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
            let result_value =
                self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
            if !result_value.is_truthy() {
                result.insert(key.clone(), value.clone());
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
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
            self.invoke_hash_single(&func, &call_env, key.to_value(), span)?;
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
            self.invoke_hash_single(&func, &call_env, value.clone(), span)?;
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
            let result_value =
                self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
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
            let result_value =
                self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
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
            let result_value =
                self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
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
            let result_value =
                self.invoke_hash_kv(&func, &call_env, key.to_value(), value.clone(), span)?;
            if result_value.is_truthy() {
                return Ok(Value::Bool(true));
            }
        }

        Ok(Value::Bool(false))
    }
}
