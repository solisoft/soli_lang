//! Native hash method dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::{hash_contains_value, hash_get_value, HashKey, HashPairs, Value};
use crate::span::Span;

use super::vm::Vm;

/// Build an empty `HashPairs` pre-sized to `cap`.
#[inline]
fn new_hash(cap: usize) -> HashPairs {
    indexmap::IndexMap::with_capacity_and_hasher(cap, ahash::RandomState::default())
}

/// Full parameter count of a callback value — decides whether a hash iterator
/// passes `(key, value)` as two args (arity >= 2) or a single `[key, value]`
/// pair (arity < 2), matching the tree-walking interpreter's semantics.
#[inline]
fn callback_wants_two_args(cb: &Value) -> bool {
    match cb {
        Value::VmClosure(c) => c.proto.arity as usize >= 2,
        Value::Function(f) => f.full_arity() >= 2,
        _ => false,
    }
}

impl Vm {
    /// Dispatch a hash method call.
    pub fn vm_call_hash_method(
        &mut self,
        hash: &Rc<RefCell<HashPairs>>,
        name: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match name {
            // --- Mutating methods ---
            "set" => {
                if args.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, args.len(), span));
                }
                let key = value_to_hash_key(&args[0], span)?;
                hash.borrow_mut().insert(key, args[1].clone());
                Ok(Value::Null)
            }
            "delete" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let key = value_to_hash_key(&args[0], span)?;
                let removed = hash.borrow_mut().swap_remove(&key);
                Ok(removed.unwrap_or(Value::Null))
            }
            "clear" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                hash.borrow_mut().clear();
                Ok(Value::Null)
            }

            // --- Non-mutating methods ---
            "length" | "len" | "size" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                Ok(Value::Int(hash.borrow().len() as i64))
            }
            "empty?" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                Ok(Value::Bool(hash.borrow().is_empty()))
            }
            "keys" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let keys: Vec<Value> = hash.borrow().keys().map(HashKey::to_value).collect();
                Ok(Value::Array(Rc::new(RefCell::new(keys))))
            }
            "values" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let values: Vec<Value> = hash.borrow().values().cloned().collect();
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }
            "entries" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let entries: Vec<Value> = hash
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                    })
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(entries))))
            }
            "has_key" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                Ok(Value::Bool(hash_contains_value(&hash.borrow(), &args[0])))
            }
            "get" | "fetch" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let found = hash_get_value(&hash.borrow(), &args[0]).cloned();
                Ok(match found {
                    Some(v) => v,
                    None => args.get(1).cloned().unwrap_or(Value::Null),
                })
            }
            "merge" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                match &args[0] {
                    Value::Hash(other) => {
                        let mut new_hash = hash.borrow().clone();
                        for (k, v) in other.borrow().iter() {
                            new_hash.insert(k.clone(), v.clone());
                        }
                        Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
                    }
                    _ => Err(RuntimeError::type_error(
                        "merge expects a hash argument",
                        span,
                    )),
                }
            }
            "compact" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                // Clone the whole map (preserves the hash table — no rehashing)
                // then drop null entries via retain. Faster than re-inserting each
                // non-null entry individually.
                let mut new_hash = hash.borrow().clone();
                new_hash.retain(|_, v| !matches!(v, Value::Null));
                Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
            }
            "invert" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let h = hash.borrow();
                let mut new_hash = indexmap::IndexMap::with_capacity_and_hasher(
                    h.len(),
                    ahash::RandomState::default(),
                );
                for (k, v) in h.iter() {
                    let new_key = value_to_hash_key(v, span)?;
                    new_hash.insert(new_key, k.to_value());
                }
                Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
            }
            "to_string" | "to_s" => {
                let h = hash.borrow();
                if h.is_empty() {
                    return Ok(Value::String("{}".into()));
                }
                let mut result = String::with_capacity(2 + h.len() * 12);
                result.push('{');
                for (i, (k, v)) in h.iter().enumerate() {
                    if i > 0 {
                        result.push_str(", ");
                    }
                    k.write_key_to_string(&mut result);
                    result.push_str(" => ");
                    v.write_to_string(&mut result);
                }
                result.push('}');
                Ok(Value::String(result.into()))
            }
            // Universal methods
            "class" => Ok(Value::String("hash".into())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(hash.borrow().is_empty())),
            "present?" => Ok(Value::Bool(!hash.borrow().is_empty())),
            "inspect" => {
                let h = hash.borrow();
                if h.is_empty() {
                    return Ok(Value::String("{}".into()));
                }
                let mut result = String::with_capacity(2 + h.len() * 12);
                result.push('{');
                for (i, (k, v)) in h.iter().enumerate() {
                    if i > 0 {
                        result.push_str(", ");
                    }
                    k.write_key_to_string(&mut result);
                    result.push_str(": ");
                    v.write_to_string(&mut result);
                }
                result.push('}');
                Ok(Value::String(result.into()))
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
                Ok(Value::Bool(class_name == "hash" || class_name == "object"))
            }
            "shift" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let mut hash_ref = hash.borrow_mut();
                if hash_ref.is_empty() {
                    return Ok(Value::Null);
                }
                let (key, value) =
                    hash_ref
                        .swap_remove_index(0)
                        .ok_or_else(|| RuntimeError::General {
                            message: "unexpected error in hash shift".to_string(),
                            span,
                        })?;
                Ok(Value::Array(Rc::new(RefCell::new(vec![
                    key.to_value(),
                    value,
                ]))))
            }
            "flatten" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let pairs: Vec<Value> = hash
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                    })
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(pairs))))
            }
            "values_at" => {
                if args.is_empty() {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let h = hash.borrow();
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    let v = hash_get_value(&h, arg).cloned().unwrap_or(Value::Null);
                    values.push(v);
                }
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }
            "key" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let needle = &args[0];
                for (k, v) in hash.borrow().iter() {
                    if v == needle {
                        return Ok(k.to_value());
                    }
                }
                Ok(Value::Null)
            }
            "has_value?" | "value?" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let needle = &args[0];
                let found = hash.borrow().values().any(|v| v == needle);
                Ok(Value::Bool(found))
            }
            "to_h" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let new_hash = hash.borrow().clone();
                Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
            }
            "update" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                match &args[0] {
                    Value::Hash(other) => {
                        let mut new_hash = hash.borrow().clone();
                        for (k, v) in other.borrow().iter() {
                            new_hash.insert(k.clone(), v.clone());
                        }
                        Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
                    }
                    _ => Err(RuntimeError::type_error(
                        "update expects a hash argument",
                        span,
                    )),
                }
            }
            "assoc" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let value = hash_get_value(&hash.borrow(), &args[0]).cloned();
                match value {
                    Some(v) => Ok(Value::Array(Rc::new(RefCell::new(vec![
                        args[0].clone(),
                        v,
                    ])))),
                    None => Ok(Value::Null),
                }
            }
            "rassoc" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let needle = &args[0];
                for (k, v) in hash.borrow().iter() {
                    if v == needle {
                        return Ok(Value::Array(Rc::new(RefCell::new(vec![
                            k.to_value(),
                            v.clone(),
                        ]))));
                    }
                }
                Ok(Value::Null)
            }
            "fetch_values" => {
                if args.is_empty() {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let h = hash.borrow();
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    match hash_get_value(&h, arg) {
                        Some(v) => values.push(v.clone()),
                        None => {
                            return Err(RuntimeError::type_error(
                                format!("key not found: {:?}", arg),
                                span,
                            ))
                        }
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }
            "to_json" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                match crate::interpreter::value_stringify::stringify_hash_map_to_string(
                    &hash.borrow(),
                ) {
                    Ok(json) => Ok(Value::String(json.into())),
                    Err(e) => Err(RuntimeError::General { message: e, span }),
                }
            }
            "slice" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let Value::Array(keys) = &args[0] else {
                    return Err(RuntimeError::type_error(
                        "slice expects an array of keys",
                        span,
                    ));
                };
                let keys = keys.borrow();
                let src = hash.borrow();
                let mut result = new_hash(keys.len());
                for key in keys.iter() {
                    let Some(hash_key) = key.to_hash_key() else {
                        return Err(RuntimeError::type_error(
                            format!("{} cannot be used as a hash key", key.type_name()),
                            span,
                        ));
                    };
                    if let Some(v) = hash_get_value(&src, key) {
                        result.insert(hash_key, v.clone());
                    }
                }
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "except" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let Value::Array(keys) = &args[0] else {
                    return Err(RuntimeError::type_error(
                        "except expects an array of keys",
                        span,
                    ));
                };
                let exclude: std::collections::HashSet<HashKey> = keys
                    .borrow()
                    .iter()
                    .filter_map(|k| k.to_hash_key())
                    .collect();
                let src = hash.borrow();
                let mut result = new_hash(src.len());
                for (k, v) in src.iter() {
                    if !exclude.contains(k) {
                        result.insert(k.clone(), v.clone());
                    }
                }
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "dig" => {
                if args.is_empty() {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let mut current = hash_get_value(&hash.borrow(), &args[0]).cloned();
                for key in &args[1..] {
                    current = match current.take() {
                        Some(Value::Hash(h)) => hash_get_value(&h.borrow(), key).cloned(),
                        Some(Value::Array(arr)) => {
                            if let Value::Int(idx) = key {
                                let arr_ref = arr.borrow();
                                let idx = if *idx < 0 {
                                    arr_ref.len() as i64 + idx
                                } else {
                                    *idx
                                };
                                usize::try_from(idx)
                                    .ok()
                                    .and_then(|i| arr_ref.get(i).cloned())
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
            // --- Closure-taking methods ---
            "map" => {
                let cb = Self::single_callback(args, span)?;
                let two = callback_wants_two_args(&cb);
                let len = hash.borrow().len();
                let mut result = new_hash(len);
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let Some((k, v)) = clone_entry(hash, i) else {
                            break;
                        };
                        let r = self.hash_invoke_kv(&batch, &cb, two, k.to_value(), v, span)?;
                        if let Value::Array(arr) = r {
                            let arr = arr.borrow();
                            if arr.len() == 2 {
                                let nk = arr[0].to_hash_key().ok_or_else(|| {
                                    RuntimeError::type_error("hash key must be hashable", span)
                                })?;
                                result.insert(nk, arr[1].clone());
                            }
                        }
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "filter" | "select" | "reject" | "keep_if" | "delete_if" => {
                let cb = Self::single_callback(args, span)?;
                let two = callback_wants_two_args(&cb);
                let keep_when_truthy = !matches!(name, "reject" | "delete_if");
                let len = hash.borrow().len();
                let mut result = new_hash(len);
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let Some((k, v)) = clone_entry(hash, i) else {
                            break;
                        };
                        let r =
                            self.hash_invoke_kv(&batch, &cb, two, k.to_value(), v.clone(), span)?;
                        if r.is_truthy() == keep_when_truthy {
                            result.insert(k, v);
                        }
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "transform_values" => {
                let cb = Self::single_callback(args, span)?;
                let len = hash.borrow().len();
                let mut result = new_hash(len);
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let Some((k, v)) = clone_entry(hash, i) else {
                            break;
                        };
                        let nv = self.invoke_in_batch_one(&batch, &cb, v, span)?;
                        result.insert(k, nv);
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "transform_keys" => {
                let cb = Self::single_callback(args, span)?;
                let len = hash.borrow().len();
                let mut result = new_hash(len);
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let Some((k, v)) = clone_entry(hash, i) else {
                            break;
                        };
                        let nk = self.invoke_in_batch_one(&batch, &cb, k.to_value(), span)?;
                        let nk = nk.to_hash_key().ok_or_else(|| {
                            RuntimeError::type_error("transformed key must be hashable", span)
                        })?;
                        result.insert(nk, v);
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "each" | "each_value" | "each_key" => {
                let cb = Self::single_callback(args, span)?;
                let two = name == "each" && callback_wants_two_args(&cb);
                let len = hash.borrow().len();
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let Some((k, v)) = clone_entry(hash, i) else {
                            break;
                        };
                        match name {
                            "each_key" => {
                                self.invoke_in_batch_one(&batch, &cb, k.to_value(), span)?;
                            }
                            "each_value" => {
                                self.invoke_in_batch_one(&batch, &cb, v, span)?;
                            }
                            _ => {
                                self.hash_invoke_kv(&batch, &cb, two, k.to_value(), v, span)?;
                            }
                        }
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Hash(hash.clone()))
            }
            "all?" | "any?" => {
                let cb = Self::single_callback(args, span)?;
                let two = callback_wants_two_args(&cb);
                let want_any = name == "any?";
                let len = hash.borrow().len();
                let mut answer = !want_any; // all? starts true, any? starts false
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..len {
                        let Some((k, v)) = clone_entry(hash, i) else {
                            break;
                        };
                        let r = self.hash_invoke_kv(&batch, &cb, two, k.to_value(), v, span)?;
                        if want_any && r.is_truthy() {
                            answer = true;
                            break;
                        }
                        if !want_any && !r.is_truthy() {
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
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Hash".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    /// Extract the single closure argument shared by hash iterator methods.
    #[inline]
    fn single_callback(args: &[Value], span: Span) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, args.len(), span));
        }
        Ok(args[0].clone())
    }

    /// Invoke a `(key, value)` callback, passing two args when the callback
    /// declares >= 2 params, or a single `[key, value]` pair otherwise.
    #[inline]
    fn hash_invoke_kv(
        &mut self,
        batch: &super::vm_calls::CallableBatch,
        cb: &Value,
        two_args: bool,
        key: Value,
        value: Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if two_args {
            self.invoke_in_batch_two(batch, cb, key, value, span)
        } else {
            let pair = Value::Array(Rc::new(RefCell::new(vec![key, value])));
            self.invoke_in_batch_one(batch, cb, pair, span)
        }
    }
}

/// Clone the `(key, value)` at position `i` from a live hash, re-borrowing each
/// time so user callbacks can mutate the hash between iterations. Returns
/// `None` when `i` is past the (possibly shrunk) end.
#[inline]
fn clone_entry(hash: &Rc<RefCell<HashPairs>>, i: usize) -> Option<(HashKey, Value)> {
    let b = hash.borrow();
    b.get_index(i).map(|(k, v)| (k.clone(), v.clone()))
}

fn value_to_hash_key(val: &Value, span: Span) -> Result<HashKey, RuntimeError> {
    match val {
        Value::String(s) => Ok(HashKey::String(s.clone())),
        Value::Int(n) => Ok(HashKey::Int(*n)),
        Value::Bool(b) => Ok(HashKey::Bool(*b)),
        Value::Null => Ok(HashKey::Null),
        _ => Err(RuntimeError::type_error(
            format!("Cannot use {} as hash key", val.type_name()),
            span,
        )),
    }
}
