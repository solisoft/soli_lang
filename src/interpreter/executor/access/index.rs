//! Index access evaluation (array[index], hash[key]).

use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Evaluate index access expression: object[index]
    pub(crate) fn evaluate_index(
        &mut self,
        object: &Expr,
        index: &Expr,
        span: Span,
    ) -> RuntimeResult<Value> {
        let obj_val = self.evaluate(object)?;
        let idx_val = self.evaluate(index)?;

        // Auto-resolve Futures before indexing
        let obj_val = obj_val.resolve().map_err(|e| RuntimeError::new(e, span))?;

        match (&obj_val, &idx_val) {
            (Value::Array(arr), Value::Int(idx)) => {
                let arr = arr.borrow();
                let original_idx = *idx;
                let idx_usize = if *idx < 0 {
                    (arr.len() as i64 + idx) as usize
                } else {
                    *idx as usize
                };
                arr.get(idx_usize)
                    .cloned()
                    .ok_or_else(|| RuntimeError::IndexOutOfBounds {
                        index: original_idx,
                        length: arr.len(),
                        span,
                    })
            }
            (Value::String(s), Value::Int(idx)) => {
                let original_idx = *idx;
                let idx_usize = if *idx < 0 {
                    (s.len() as i64 + idx) as usize
                } else {
                    *idx as usize
                };
                s.chars()
                    .nth(idx_usize)
                    .map(|c| Value::String(c.to_string()))
                    .ok_or(RuntimeError::IndexOutOfBounds {
                        index: original_idx,
                        length: s.len(),
                        span,
                    })
            }
            (Value::Hash(hash), key) => {
                let hash_key = key.to_hash_key().ok_or_else(|| {
                    RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key.type_name()),
                        index.span,
                    )
                })?;
                let hash = hash.borrow();
                Ok(hash.get(&hash_key).cloned().unwrap_or(Value::Null))
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "cannot index {} with {}",
                    obj_val.type_name(),
                    idx_val.type_name()
                ),
                span,
            )),
        }
    }

    /// Evaluate index assignment: object[index] = value
    pub(crate) fn evaluate_index_assign(
        &mut self,
        object: &Expr,
        index: &Expr,
        new_value: Value,
        span: Span,
    ) -> RuntimeResult<Value> {
        let obj_val = self.evaluate(object)?;
        let idx_val = self.evaluate(index)?;

        match (&obj_val, &idx_val) {
            (Value::Array(arr), Value::Int(idx)) => {
                let mut arr = arr.borrow_mut();
                let original_idx = *idx;
                let idx_usize = if *idx < 0 {
                    (arr.len() as i64 + idx) as usize
                } else {
                    *idx as usize
                };
                if idx_usize >= arr.len() {
                    return Err(RuntimeError::IndexOutOfBounds {
                        index: original_idx,
                        length: arr.len(),
                        span,
                    });
                }
                arr[idx_usize] = new_value.clone();
                Ok(new_value)
            }
            (Value::Hash(hash), key) => {
                let hash_key = key.to_hash_key().ok_or_else(|| {
                    RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key.type_name()),
                        index.span,
                    )
                })?;
                hash.borrow_mut().insert(hash_key, new_value.clone());
                Ok(new_value)
            }
            _ => Err(RuntimeError::type_error("invalid assignment target", span)),
        }
    }
}
