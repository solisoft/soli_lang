//! Native hash method dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::executor::calls::hash_pure::hash_method_borrowed;
use crate::interpreter::value::{hash_set_value, HashKey, HashPairs, Value};
use crate::span::Span;

use super::vm::Vm;

/// Build an empty `HashPairs` pre-sized to `cap`.
#[inline]
fn new_hash(cap: usize) -> HashPairs {
    HashPairs::with_capacity(cap)
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
        // Universal zero-argument methods, guarded in one place. The
        // tree-walking interpreter already rejects `x.nil?("junk")`; the VM
        // accepted the argument and threw it away, so the same call errored
        // under `soli test` and quietly returned a value under `soli serve`.
        if !args.is_empty()
            && matches!(
                name,
                "class" | "nil?" | "blank?" | "present?" | "inspect" | "to_s" | "to_string"
            )
        {
            {
                return Err(RuntimeError::wrong_arity(0, args.len(), span));
            }
        }
        // Everything answerable from a borrow — lookups, `keys`, `slice`,
        // `merge`, `dig`, … — lives in one place shared with the interpreter.
        if let Some(result) = hash_method_borrowed(&hash.borrow(), name, args, span) {
            return result;
        }
        match name {
            // --- Mutating methods ---
            "set" => {
                if args.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, args.len(), span));
                }
                if !hash_set_value(&mut hash.borrow_mut(), &args[0], args[1].clone()) {
                    return Err(RuntimeError::type_error(
                        format!("Cannot use {} as hash key", args[0].type_name()),
                        span,
                    ));
                }
                Ok(Value::Null)
            }
            "delete" => {
                if args.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, args.len(), span));
                }
                let Some(key) = args[0].to_hash_key() else {
                    return Err(RuntimeError::type_error(
                        format!("Cannot use {} as hash key", args[0].type_name()),
                        span,
                    ));
                };
                // `shift_remove`, not `swap_remove`: the interpreter keeps the
                // remaining keys in insertion order, and swapping the last key
                // into the hole reordered the hash under `soli serve` only.
                let removed = hash.borrow_mut().shift_remove(&key);
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
            // Universal methods
            "class" => Ok(Value::String("hash".into())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(hash.borrow().is_empty())),
            "present?" => Ok(Value::Bool(!hash.borrow().is_empty())),
            "inspect" => {
                let rendered = crate::interpreter::executor::Interpreter::inspect_value(
                    &Value::Hash(hash.clone()),
                );
                Ok(Value::String(rendered.into()))
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
                        .shift_remove_index(0)
                        .ok_or_else(|| RuntimeError::General {
                            message: "unexpected error in hash shift".to_string(),
                            span,
                        })?;
                Ok(Value::Array(Rc::new(RefCell::new(vec![
                    key.to_value(),
                    value,
                ]))))
            }
            // --- Closure-taking methods ---
            "map" => {
                let cb = Self::single_callback(args, name, span)?;
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
                let cb = Self::single_callback(args, name, span)?;
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
                let cb = Self::single_callback(args, name, span)?;
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
                let cb = Self::single_callback(args, name, span)?;
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
                let cb = Self::single_callback(args, name, span)?;
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
                let cb = Self::single_callback(args, name, span)?;
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
            // Not a built-in hash method. A hash entry holding a function is a
            // dispatch table — `handlers.on_save(record)` — and the
            // tree-walking interpreter has always called it. The VM did not, so
            // that pattern passed every test and then failed in production,
            // where the VM runs. Look the name up and invoke it when it is
            // callable; anything else still reports an unknown property, which
            // is what catches `h.lenght()`.
            _ => {
                let entry = hash
                    .borrow()
                    .get(&crate::interpreter::value::StrKey(name))
                    .filter(|v| v.is_callable())
                    .cloned();
                match entry {
                    Some(callable) => {
                        let batch = self.enter_callable_batch();
                        let outcome = self.invoke_in_batch(&batch, &callable, args, span);
                        self.exit_callable_batch(batch);
                        outcome
                    }
                    None => Err(RuntimeError::NoSuchProperty {
                        value_type: "Hash".to_string(),
                        property: name.to_string(),
                        span,
                    }),
                }
            }
        }
    }

    /// Extract the single closure argument shared by hash iterator methods.
    #[inline]
    fn single_callback(args: &[Value], method: &str, span: Span) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, args.len(), span));
        }
        super::vm_array_methods::expect_callback(&args[0], method, span)?;
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
