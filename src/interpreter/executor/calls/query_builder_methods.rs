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
            "includes" => self.qb_includes(qb, arguments, span),
            "select" | "fields" => self.qb_select(qb, arguments, span),
            "join" => self.qb_join(qb, arguments, span),
            "all" => self.qb_all(qb, arguments, span),
            "first" => self.qb_first(qb, arguments, span),
            "count" => self.qb_count(qb, arguments, span),
            "to_query" => self.qb_to_query(qb, arguments, span),
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
            Value::Hash(hash) => {
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

    fn qb_includes(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Err(RuntimeError::type_error(
                "includes() requires at least one relation name",
                span,
            ));
        }

        let mut new_qb = qb.borrow().clone();
        let class_name = crate::interpreter::symbol_string(new_qb.class_name)
            .unwrap_or("unknown")
            .to_string();

        if arguments.len() == 1 && matches!(&arguments[0], Value::Hash(_)) {
            // Pattern B: hash arg → { "posts": ["title", "body"] }
            if let Value::Hash(hash) = &arguments[0] {
                for (k, v) in hash.borrow().iter() {
                    let rel_name = match k {
                        crate::interpreter::value::HashKey::String(s) => s.clone(),
                        _ => continue,
                    };
                    let rel =
                        crate::interpreter::builtins::model::get_relation(&class_name, &rel_name)
                            .ok_or_else(|| RuntimeError::General {
                            message: format!(
                                "No relation '{}' defined on {}",
                                rel_name, class_name
                            ),
                            span,
                        })?;
                    let fields = match v {
                        Value::Array(arr) => {
                            let field_names: Vec<String> = arr
                                .borrow()
                                .iter()
                                .filter_map(|v| {
                                    if let Value::String(s) = v {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if field_names.is_empty() {
                                None
                            } else {
                                Some(field_names)
                            }
                        }
                        _ => None,
                    };
                    new_qb.add_include(
                        rel_name,
                        rel,
                        None,
                        std::collections::HashMap::new(),
                        fields,
                    );
                }
            }
        } else if arguments.len() >= 2 && matches!(arguments.last(), Some(Value::Hash(_))) {
            // Pattern C: positional filtered include
            let rel_name = match &arguments[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "includes() expects string relation name as first argument",
                        span,
                    ))
                }
            };
            let rel = crate::interpreter::builtins::model::get_relation(&class_name, &rel_name)
                .ok_or_else(|| RuntimeError::General {
                    message: format!("No relation '{}' defined on {}", rel_name, class_name),
                    span,
                })?;

            let filter = if arguments.len() >= 3 {
                match &arguments[1] {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                }
            } else {
                None
            };

            let options_hash = match arguments.last() {
                Some(Value::Hash(h)) => h.borrow(),
                _ => unreachable!(),
            };

            let mut bind_vars = std::collections::HashMap::new();
            let mut fields: Option<Vec<String>> = None;

            for (k, v) in options_hash.iter() {
                if let crate::interpreter::value::HashKey::String(key) = k {
                    if key == "fields" {
                        if let Value::Array(arr) = v {
                            let field_names: Vec<String> = arr
                                .borrow()
                                .iter()
                                .filter_map(|v| {
                                    if let Value::String(s) = v {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if !field_names.is_empty() {
                                fields = Some(field_names);
                            }
                        }
                    } else {
                        bind_vars.insert(
                            key.clone(),
                            crate::interpreter::builtins::model::value_to_json(v)
                                .map_err(|e| RuntimeError::General { message: e, span })?,
                        );
                    }
                }
            }

            new_qb.add_include(rel_name, rel, filter, bind_vars, fields);
        } else {
            // Pattern A: all strings → multi-relation unfiltered
            for arg in &arguments {
                let rel_name = match arg {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "includes() expects string relation names",
                            span,
                        ))
                    }
                };
                let rel = crate::interpreter::builtins::model::get_relation(&class_name, &rel_name)
                    .ok_or_else(|| RuntimeError::General {
                        message: format!("No relation '{}' defined on {}", rel_name, class_name),
                        span,
                    })?;
                new_qb.add_include(rel_name, rel, None, std::collections::HashMap::new(), None);
            }
        }

        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_select(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Err(RuntimeError::type_error(
                "select() requires at least one field name",
                span,
            ));
        }

        let mut fields = Vec::new();
        for arg in &arguments {
            match arg {
                Value::String(s) => fields.push(s.clone()),
                _ => {
                    return Err(RuntimeError::type_error(
                        "select() expects string field names",
                        span,
                    ))
                }
            }
        }

        let mut new_qb = qb.borrow().clone();
        new_qb.set_select(fields);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_join(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 3 {
            return Err(RuntimeError::type_error(
                "join() expects 1-3 arguments: relation name, optional filter, optional bind vars",
                span,
            ));
        }

        let rel_name = match &arguments[0] {
            Value::String(s) => s.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "join() expects string relation name",
                    span,
                ))
            }
        };

        let mut new_qb = qb.borrow().clone();
        let class_name = crate::interpreter::symbol_string(new_qb.class_name)
            .unwrap_or("unknown")
            .to_string();

        let rel = crate::interpreter::builtins::model::get_relation(&class_name, &rel_name)
            .ok_or_else(|| RuntimeError::General {
                message: format!("No relation '{}' defined on {}", rel_name, class_name),
                span,
            })?;

        let filter = match arguments.get(1) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        };

        let bind_vars = match arguments.get(2) {
            Some(Value::Hash(hash)) => {
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
            _ => std::collections::HashMap::new(),
        };

        new_qb.add_join(rel_name, rel, filter, bind_vars);
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

    fn qb_to_query(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let (query, bind_vars) = qb.borrow().build_query();
        if bind_vars.is_empty() {
            Ok(Value::String(query))
        } else {
            Ok(Value::String(format!(
                "{} | bind_vars: {:?}",
                query, bind_vars
            )))
        }
    }
}
