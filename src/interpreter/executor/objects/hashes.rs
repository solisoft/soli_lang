//! Hash literal evaluation.

use std::cell::RefCell;
use std::rc::Rc;

use ahash::RandomState as AHasher;

use crate::ast::{Expr, ExprKind};
use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{HashKey, HashPairs, Value};

impl Interpreter {
    /// Evaluate hash literal expression: {key: value, ...}
    pub(crate) fn evaluate_hash(&mut self, pairs: &[(Expr, Expr)]) -> RuntimeResult<Value> {
        let mut entries: HashPairs =
            HashPairs::with_capacity_and_hasher(pairs.len(), AHasher::default());
        for (key_expr, value_expr) in pairs {
            let hash_key = self.evaluate_hash_key(key_expr)?;
            let value = self.evaluate(value_expr)?;
            // Insert or update (IndexMap handles this automatically)
            entries.insert(hash_key, value);
        }
        Ok(Value::Hash(Rc::new(RefCell::new(entries))))
    }

    fn evaluate_hash_key(&mut self, key_expr: &Expr) -> RuntimeResult<HashKey> {
        match &key_expr.kind {
            ExprKind::IntLiteral(n) => Ok(HashKey::Int(*n)),
            ExprKind::StringLiteral(s) => Ok(HashKey::String(s.clone())),
            ExprKind::BoolLiteral(b) => Ok(HashKey::Bool(*b)),
            ExprKind::Null => Ok(HashKey::Null),
            ExprKind::Symbol(s) => Ok(HashKey::Symbol(s.clone())),
            _ => {
                let key = self.evaluate(key_expr)?;
                key.to_hash_key().ok_or_else(|| {
                    RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key.type_name()),
                        key_expr.span,
                    )
                })
            }
        }
    }
}
