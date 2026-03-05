//! Native hash method dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::{HashKey, HashPairs, Value};
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Dispatch a hash method call.
    pub fn vm_call_hash_method(
        &self,
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
            "length" | "len" => {
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
                let keys: Vec<Value> = hash
                    .borrow()
                    .keys()
                    .map(|k| match k {
                        HashKey::String(s) => Value::String(s.clone()),
                        HashKey::Int(n) => Value::Int(*n),
                        HashKey::Bool(b) => Value::Bool(*b),
                        HashKey::Null => Value::Null,
                        HashKey::Decimal(d) => Value::String(d.to_string()),
                    })
                    .collect();
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
                        let key = match k {
                            HashKey::String(s) => Value::String(s.clone()),
                            HashKey::Int(n) => Value::Int(*n),
                            HashKey::Bool(b) => Value::Bool(*b),
                            HashKey::Null => Value::Null,
                            HashKey::Decimal(d) => Value::String(d.to_string()),
                        };
                        Value::Array(Rc::new(RefCell::new(vec![key, v.clone()])))
                    })
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(entries))))
            }
            "has_key" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let key = value_to_hash_key(&args[0], span)?;
                Ok(Value::Bool(hash.borrow().contains_key(&key)))
            }
            "get" | "fetch" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let key = value_to_hash_key(&args[0], span)?;
                let default = args.get(1).cloned().unwrap_or(Value::Null);
                let result = hash.borrow().get(&key).cloned().unwrap_or(default);
                Ok(result)
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
                let h = hash.borrow();
                let mut new_hash = indexmap::IndexMap::with_capacity_and_hasher(
                    h.len(),
                    ahash::RandomState::new(),
                );
                for (k, v) in h.iter() {
                    if !matches!(v, Value::Null) {
                        new_hash.insert(k.clone(), v.clone());
                    }
                }
                Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
            }
            "invert" => {
                if !args.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, args.len(), span));
                }
                let h = hash.borrow();
                let mut new_hash = indexmap::IndexMap::with_capacity_and_hasher(
                    h.len(),
                    ahash::RandomState::new(),
                );
                for (k, v) in h.iter() {
                    let new_key = value_to_hash_key(v, span)?;
                    let new_value = match k {
                        HashKey::String(s) => Value::String(s.clone()),
                        HashKey::Int(n) => Value::Int(*n),
                        HashKey::Bool(b) => Value::Bool(*b),
                        HashKey::Null => Value::Null,
                        HashKey::Decimal(d) => Value::String(d.to_string()),
                    };
                    new_hash.insert(new_key, new_value);
                }
                Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
            }
            "to_string" | "to_s" => {
                let h = hash.borrow();
                let parts: Vec<String> = h
                    .iter()
                    .map(|(k, v)| {
                        let key_str = match k {
                            HashKey::String(s) => format!("\"{}\"", s),
                            HashKey::Int(n) => n.to_string(),
                            _ => format!("{:?}", k),
                        };
                        format!("{}: {}", key_str, v)
                    })
                    .collect();
                Ok(Value::String(format!("{{{}}}", parts.join(", "))))
            }
            // Universal methods
            "class" => Ok(Value::String("hash".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(hash.borrow().is_empty())),
            "present?" => Ok(Value::Bool(!hash.borrow().is_empty())),
            "inspect" => {
                let h = hash.borrow();
                let parts: Vec<String> = h
                    .iter()
                    .map(|(k, v)| {
                        let key_str = match k {
                            HashKey::String(s) => format!("\"{}\"", s),
                            HashKey::Int(n) => n.to_string(),
                            _ => format!("{:?}", k),
                        };
                        format!("{}: {}", key_str, v)
                    })
                    .collect();
                Ok(Value::String(format!("{{{}}}", parts.join(", "))))
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
                Ok(Value::Bool(class_name == "hash" || class_name == "object"))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Hash".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }
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
