//! Hash literal evaluation.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{HashKey, Value};

use indexmap::IndexMap;

impl Interpreter {
    /// Evaluate hash literal expression: {key: value, ...}
    pub(crate) fn evaluate_hash(&mut self, pairs: &[(Expr, Expr)]) -> RuntimeResult<Value> {
        let mut entries: IndexMap<HashKey, Value> = IndexMap::new();
        for (key_expr, value_expr) in pairs {
            let key = self.evaluate(key_expr)?;
            let hash_key = key.to_hash_key().ok_or_else(|| {
                RuntimeError::type_error(
                    format!("{} cannot be used as a hash key", key.type_name()),
                    key_expr.span,
                )
            })?;
            let value = self.evaluate(value_expr)?;
            // Insert or update (IndexMap handles this automatically)
            entries.insert(hash_key, value);
        }
        Ok(Value::Hash(Rc::new(RefCell::new(entries))))
    }
}
