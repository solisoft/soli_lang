//! Method call evaluation - QueryBuilder methods.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::builtins::model::{
    execute_query_builder, execute_query_builder_count, execute_query_builder_first,
};
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Handle QueryBuilder methods for chaining: where, order, limit, offset, all, first, count
    pub(crate) fn call_query_builder_method(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "where" => self.qb_where(qb, arguments, span),
            "order" => self.qb_order(qb, arguments, span),
            "limit" => self.qb_limit(qb, arguments, span),
            "offset" => self.qb_offset(qb, arguments, span),
            "all" => self.qb_all(qb, arguments, span),
            "first" => self.qb_first(qb, arguments, span),
            "count" => self.qb_count(qb, arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "QueryBuilder".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn qb_where(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let filter = match &arguments[0] {
            Value::String(s) => s.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "where() expects string filter expression",
                    span,
                ))
            }
        };
        let bind_vars = match &arguments[1] {
            Value::Hash(ref hash) => {
                let mut map = std::collections::HashMap::new();
                for (k, v) in hash.borrow().iter() {
                    if let crate::interpreter::value::HashKey::String(key) = k {
                        map.insert(
                            key.clone(),
                            crate::interpreter::builtins::model::value_to_json(v)
                                .map_err(|e| RuntimeError::General { message: e, span })?,
                        );
                    }
                }
                map
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "where() expects hash for bind variables",
                    span,
                ))
            }
        };

        let mut new_qb = qb.borrow().clone();
        if let Some(existing_filter) = &new_qb.filter {
            new_qb.filter = Some(format!("({}) AND ({})", existing_filter, filter));
        } else {
            new_qb.filter = Some(filter);
        }
        for (k, v) in bind_vars {
            new_qb
                .bind_vars
                .insert(crate::interpreter::get_symbol(&k), v);
        }
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_order(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::type_error(
                "order() expects 1 or 2 arguments: field and optional direction",
                span,
            ));
        }
        let field = match &arguments[0] {
            Value::String(s) => s.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "order() expects string field",
                    span,
                ))
            }
        };
        let direction = if arguments.len() == 2 {
            match &arguments[1] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "order() expects string direction",
                        span,
                    ))
                }
            }
        } else {
            "asc".to_string()
        };

        let mut new_qb = qb.borrow().clone();
        new_qb.set_order(field, direction);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_limit(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let limit = match &arguments[0] {
            Value::Int(n) if *n >= 0 => *n as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "limit() expects positive integer",
                    span,
                ))
            }
        };

        let mut new_qb = qb.borrow().clone();
        new_qb.set_limit(limit);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_offset(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let offset = match &arguments[0] {
            Value::Int(n) if *n >= 0 => *n as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "offset() expects positive integer",
                    span,
                ))
            }
        };

        let mut new_qb = qb.borrow().clone();
        new_qb.set_offset(offset);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_all(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(execute_query_builder(&qb.borrow()))
    }

    fn qb_first(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(execute_query_builder_first(&qb.borrow()))
    }

    fn qb_count(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(execute_query_builder_count(&qb.borrow()))
    }
}
