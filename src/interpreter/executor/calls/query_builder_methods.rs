//! Method call evaluation - QueryBuilder methods.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::builtins::model::{
    execute_query_builder, execute_query_builder_aggregate, execute_query_builder_count,
    execute_query_builder_delete_all, execute_query_builder_exists, execute_query_builder_first,
    execute_query_builder_group_by, execute_query_builder_update_all, AggregationFunc,
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
        // Traversal mode (instance.traverse(...)) composes with
        // where/order/limit/offset/select/pluck and the terminals, but not
        // with eager-loading, relation joins, aggregation modes, or bulk
        // writes — reject those early with a clear message.
        if qb.borrow().traversal.is_some()
            && matches!(
                method_name,
                "includes"
                    | "includes_count"
                    | "join"
                    | "group_by"
                    | "aggregate"
                    | "having"
                    | "update_all"
                    | "delete_all"
            )
        {
            return Err(RuntimeError::General {
                message: format!("{}() cannot be combined with traverse()", method_name),
                span,
            });
        }

        match method_name {
            "create" => self.qb_create(qb, arguments, span),
            "where" => self.qb_where(qb, arguments, span),
            "order" => self.qb_order(qb, arguments, span),
            "limit" => self.qb_limit(qb, arguments, span),
            "offset" => self.qb_offset(qb, arguments, span),
            "includes" => self.qb_includes(qb, arguments, span),
            "includes_count" => self.qb_includes_count(qb, arguments, span),
            "select" | "fields" => self.qb_select(qb, arguments, span),
            "join" => self.qb_join(qb, arguments, span),
            "all" => self.qb_all(qb, arguments, span),
            "first" => self.qb_first(qb, arguments, span),
            "count" => self.qb_count(qb, arguments, span),
            "paginate" => self.qb_paginate(qb, arguments, span),
            "delete_all" => self.qb_delete_all(qb, arguments, span),
            "update_all" => self.qb_update_all(qb, arguments, span),
            "exists" => self.qb_exists(qb, arguments, span),
            "pluck" => self.qb_pluck(qb, arguments, span),
            "similar" => self.qb_similar(qb, arguments, span),
            "sum" => self.qb_aggregate(qb, arguments, span, AggregationFunc::Sum),
            "avg" => self.qb_aggregate(qb, arguments, span, AggregationFunc::Avg),
            "min" => self.qb_aggregate(qb, arguments, span, AggregationFunc::Min),
            "max" => self.qb_aggregate(qb, arguments, span, AggregationFunc::Max),
            "median" => self.qb_aggregate(qb, arguments, span, AggregationFunc::Median),
            "stddev" => self.qb_aggregate(qb, arguments, span, AggregationFunc::Stddev),
            "variance" => self.qb_aggregate(qb, arguments, span, AggregationFunc::Variance),
            "count_distinct" => {
                self.qb_aggregate(qb, arguments, span, AggregationFunc::CountDistinct)
            }
            "aggregate" => self.qb_aggregate_spec(qb, arguments, span),
            "having" => self.qb_having(qb, arguments, span),
            "group_by" => self.qb_group_by(qb, arguments, span),
            "time_bucket" => self.qb_time_bucket(qb, arguments, span),
            "to_query" => self.qb_to_query(qb, arguments, span),
            // Array passthrough: materialize the QueryBuilder once, then
            // dispatch the method to the resulting array. Lets has_many
            // relations behave Enumerable-style — user.posts.each(...),
            // user.posts.map(...), user.posts.length, etc.
            "length" | "len" | "size" | "each" | "map" | "filter" | "reduce" | "find" | "any?"
            | "all?" | "sort" | "sort_by" | "reverse" | "uniq" | "compact" | "compact_blank"
            | "flatten" | "last" | "empty?" | "includes?" | "contains" | "sample" | "shuffle"
            | "take" | "drop" | "zip" | "to_string" | "to_json" | "is_a?" | "to_a" | "to_array" => {
                let materialized =
                    crate::interpreter::builtins::model::execute_query_builder(&qb.borrow());
                let method = crate::interpreter::value::ValueMethod {
                    receiver: Box::new(materialized),
                    method_name: method_name.to_string(),
                };
                self.call_method(method, arguments, span)
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "QueryBuilder".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    /// `owner.comments.create({...})` — create a child through the relation
    /// accessor: the association seed (FK, plus the polymorphic type pair on
    /// `as:` relations) is stamped over the given attributes, then the child
    /// persists through the regular save path (validations, callbacks,
    /// counter caches, dirty tracking). Returns the instance — carrying
    /// `_errors` on validation failure, like `Model.create`.
    fn qb_create(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let (seed, class, class_name) = {
            let qb_ref = qb.borrow();
            if qb_ref.through.is_some() {
                return Err(RuntimeError::General {
                    message: "create() on a through: relation is not supported — create the \
record and push it (`owner.rel << record`) or create the join record directly"
                        .to_string(),
                    span,
                });
            }
            let Some(seed) = qb_ref.assoc_seed.clone() else {
                return Err(RuntimeError::General {
                    message: "create() is only available on a has_many relation accessor of a \
persisted record (e.g. user.posts.create({...})) — use Model.create for plain inserts"
                        .to_string(),
                    span,
                });
            };
            let Some(class) = qb_ref.class.clone() else {
                return Err(RuntimeError::General {
                    message: "create(): the relation's model class is not loaded".to_string(),
                    span,
                });
            };
            let class_name = crate::interpreter::symbol_string(qb_ref.class_name)
                .unwrap_or("unknown")
                .to_string();
            (seed, class, class_name)
        };

        let attrs = match arguments.first() {
            None => None,
            Some(Value::Hash(h)) => Some(h.clone()),
            Some(other) => {
                return Err(RuntimeError::General {
                    message: format!(
                        "{}.create() expects an attribute hash, got {}",
                        class_name,
                        other.type_name()
                    ),
                    span,
                })
            }
        };

        let mut child = crate::interpreter::value::Instance::new(class);
        if let Some(attrs) = attrs {
            use crate::interpreter::value::HashKey;
            for (k, v) in attrs.borrow().iter() {
                if let HashKey::String(key) = k {
                    if !key.starts_with('_') {
                        child.set(key.to_string(), v.clone());
                    }
                }
            }
        }
        // The association seed wins over caller-supplied values.
        for (field, value) in &seed {
            child.set(field.clone(), Value::String(value.as_str().into()));
        }

        let child_value = Value::Instance(Rc::new(RefCell::new(child)));
        let result = self.save_model_instance(&child_value, span)?;
        // Match Model.create's contract: `_errors` is null on success (the
        // save path stamps an empty array) and an error array on failure.
        if matches!(result, Value::Bool(true)) {
            if let Value::Instance(inst) = &child_value {
                inst.borrow_mut().set("_errors".to_string(), Value::Null);
            }
        }
        Ok(child_value)
    }

    fn qb_where(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        // Two shapes (mirrors `Model.where`):
        //   1. Hash form (safe):     where({field: value, ...})
        //   2. String form (raw):    where("doc.foo == @foo", {foo: ...})
        // Plus the legacy 1-arg string form (no binds), which still works.
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::type_error(
                "where() expects 1 or 2 arguments: a Hash filter, or a string filter (with optional bind-vars hash)",
                span,
            ));
        }
        let (filter, bind_vars): (String, std::collections::HashMap<String, serde_json::Value>) =
            match &arguments[0] {
                Value::Hash(hash) => {
                    if arguments.len() != 1 {
                        return Err(RuntimeError::type_error(
                            "where(Hash) takes a single argument; the bind-vars hash is only valid with the string filter form",
                            span,
                        ));
                    }
                    crate::interpreter::builtins::model::build_safe_filter_from_hash(hash, "where")
                        .map_err(|e| RuntimeError::General { message: e, span })?
                }
                Value::String(s) => {
                    let filter = s.clone();
                    let binds = match arguments.get(1) {
                        Some(Value::Hash(hash)) => {
                            let mut map = std::collections::HashMap::new();
                            for (k, v) in hash.borrow().iter() {
                                if let crate::interpreter::value::HashKey::String(key) = k {
                                    let json_val =
                                        crate::interpreter::builtins::model::ensure_string_form_bind_value(
                                            v, key, "where",
                                        )
                                        .map_err(|e| RuntimeError::General {
                                            message: e,
                                            span,
                                        })?;
                                    map.insert(key.to_string(), json_val);
                                }
                            }
                            map
                        }
                        Some(_) => {
                            return Err(RuntimeError::type_error(
                                "where() expects hash for bind variables",
                                span,
                            ))
                        }
                        None => std::collections::HashMap::new(),
                    };
                    (filter.to_string(), binds)
                }
                _ => {
                    return Err(RuntimeError::type_error(
                        "where() expects a Hash filter or a string filter expression",
                        span,
                    ))
                }
            };

        // `where({})` (or an empty string filter) is a no-op — return the
        // builder unchanged rather than combining in an empty clause, which
        // would produce invalid AQL like `(...) AND ()`.
        if filter.trim().is_empty() {
            return Ok(Value::QueryBuilder(qb));
        }

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
        // SEC-004b: chain form `Model.where(...).order(field, dir)` was the
        // counterpart that escaped SEC-004's static-method gate. Same sink
        // (`SORT doc.{field}`), same validator.
        crate::interpreter::builtins::model::validate_field_name(&field, "order")
            .map_err(|e| RuntimeError::type_error(e, span))?;
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
            "asc".into()
        };
        // Same direction whitelist as `Model.order(...)` (SEC-004a) — the
        // QueryBuilder chain (`User.where(...).order(field, dir)`) lands
        // on the identical SORT-clause builder, so the input must clear
        // the same gate regardless of which entry point is used.
        crate::interpreter::builtins::model::validate_order_direction(&direction, "order")
            .map_err(|e| RuntimeError::type_error(e, span))?;

        let mut new_qb = qb.borrow().clone();
        new_qb.set_order(field.to_string(), direction.to_string());
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
                    crate::interpreter::builtins::model::relations::reject_through_relation(
                        "includes", &rel,
                    )
                    .map_err(|message| RuntimeError::General { message, span })?;
                    crate::interpreter::builtins::model::relations::reject_polymorphic_relation(
                        "includes", &rel,
                    )
                    .map_err(|message| RuntimeError::General { message, span })?;
                    let fields = match v {
                        Value::Array(arr) => {
                            let field_names: Vec<String> = arr
                                .borrow()
                                .iter()
                                .filter_map(|v| {
                                    if let Value::String(s) = v {
                                        Some(s.to_string())
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
                        rel_name.to_string(),
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
            crate::interpreter::builtins::model::relations::reject_through_relation(
                "includes", &rel,
            )
            .map_err(|message| RuntimeError::General { message, span })?;
            crate::interpreter::builtins::model::relations::reject_polymorphic_relation(
                "includes", &rel,
            )
            .map_err(|message| RuntimeError::General { message, span })?;

            let filter = if arguments.len() >= 3 {
                match &arguments[1] {
                    Value::String(s) => Some(s.to_string()),
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
                    if **key == *"fields" {
                        if let Value::Array(arr) = v {
                            let field_names: Vec<String> = arr
                                .borrow()
                                .iter()
                                .filter_map(|v| {
                                    if let Value::String(s) = v {
                                        Some(s.to_string())
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
                            key.to_string(),
                            crate::interpreter::builtins::model::value_to_json(v)
                                .map_err(|e| RuntimeError::General { message: e, span })?,
                        );
                    }
                }
            }

            new_qb.add_include(rel_name.to_string(), rel, filter, bind_vars, fields);
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
                crate::interpreter::builtins::model::relations::reject_through_relation(
                    "includes", &rel,
                )
                .map_err(|message| RuntimeError::General { message, span })?;
                crate::interpreter::builtins::model::relations::reject_polymorphic_relation(
                    "includes", &rel,
                )
                .map_err(|message| RuntimeError::General { message, span })?;
                new_qb.add_include(
                    rel_name.to_string(),
                    rel,
                    None,
                    std::collections::HashMap::new(),
                    None,
                );
            }
        }

        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_includes_count(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Err(RuntimeError::type_error(
                "includes_count() requires at least one relation name",
                span,
            ));
        }

        let mut new_qb = qb.borrow().clone();
        let class_name = crate::interpreter::symbol_string(new_qb.class_name)
            .unwrap_or("unknown")
            .to_string();

        for arg in &arguments {
            let rel_name = match arg {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "includes_count() expects string relation names",
                        span,
                    ))
                }
            };
            let rel = crate::interpreter::builtins::model::get_relation(&class_name, &rel_name)
                .ok_or_else(|| RuntimeError::General {
                    message: format!("No relation '{}' defined on {}", rel_name, class_name),
                    span,
                })?;
            new_qb
                .add_include_count(rel_name.to_string(), rel)
                .map_err(|e| RuntimeError::General { message: e, span })?;
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
                Value::String(s) => {
                    crate::interpreter::builtins::model::validate_field_name(s, "select")
                        .map_err(|e| RuntimeError::type_error(e, span))?;
                    fields.push(s.to_string());
                }
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
        crate::interpreter::builtins::model::relations::reject_through_relation("join", &rel)
            .map_err(|message| RuntimeError::General { message, span })?;
        crate::interpreter::builtins::model::relations::reject_polymorphic_relation("join", &rel)
            .map_err(|message| RuntimeError::General { message, span })?;

        let filter = match arguments.get(1) {
            Some(Value::String(s)) => Some(s.to_string()),
            _ => None,
        };

        let bind_vars = match arguments.get(2) {
            Some(Value::Hash(hash)) => {
                let mut map = std::collections::HashMap::new();
                for (k, v) in hash.borrow().iter() {
                    if let crate::interpreter::value::HashKey::String(key) = k {
                        map.insert(
                            key.to_string(),
                            crate::interpreter::builtins::model::value_to_json(v)
                                .map_err(|e| RuntimeError::General { message: e, span })?,
                        );
                    }
                }
                map
            }
            _ => std::collections::HashMap::new(),
        };

        new_qb.add_join(rel_name.to_string(), rel, filter, bind_vars);
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
        let qb_ref = qb.borrow();
        if qb_ref.exists_mode {
            Ok(execute_query_builder_exists(&qb_ref))
        } else if let Some((ref func, ref field)) = qb_ref.aggregation {
            Ok(execute_query_builder_aggregate(
                &qb_ref,
                func.clone(),
                field,
            ))
        } else if !qb_ref.group_fields.is_empty() || !qb_ref.aggregate_specs.is_empty() {
            crate::interpreter::builtins::model::execute_query_builder_grouped(&qb_ref)
                .map_err(|e| RuntimeError::General { message: e, span })
        } else if let Some((ref gf, ref func, ref af)) = qb_ref.group_by_info {
            Ok(execute_query_builder_group_by(
                &qb_ref,
                gf,
                func.clone(),
                af,
            ))
        } else if qb_ref.time_bucket_info.is_some() {
            Ok(crate::interpreter::builtins::model::execute_query_builder_time_bucket(&qb_ref))
        } else {
            Ok(execute_query_builder(&qb_ref))
        }
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
        let qb_ref = qb.borrow();
        if qb_ref.exists_mode {
            Ok(execute_query_builder_exists(&qb_ref))
        } else if let Some((ref func, ref field)) = qb_ref.aggregation {
            Ok(execute_query_builder_aggregate(
                &qb_ref,
                func.clone(),
                field,
            ))
        } else if !qb_ref.group_fields.is_empty() || !qb_ref.aggregate_specs.is_empty() {
            // Grouped mode: .first returns the first group row (the whole
            // result for ungrouped multi-aggregates, which yield one row).
            let rows = crate::interpreter::builtins::model::execute_query_builder_grouped(&qb_ref)
                .map_err(|e| RuntimeError::General { message: e, span })?;
            if let Value::Array(arr) = &rows {
                let first = arr.borrow().first().cloned();
                return Ok(first.unwrap_or(Value::Null));
            }
            Ok(rows)
        } else if let Some((ref gf, ref func, ref af)) = qb_ref.group_by_info {
            Ok(execute_query_builder_group_by(
                &qb_ref,
                gf,
                func.clone(),
                af,
            ))
        } else if qb_ref.time_bucket_info.is_some() {
            Ok(crate::interpreter::builtins::model::execute_query_builder_time_bucket(&qb_ref))
        } else {
            Ok(execute_query_builder_first(&qb_ref))
        }
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

    fn qb_paginate(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let params = match arguments.first() {
            Some(Value::Hash(h)) => h.borrow().clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "paginate() expects a Hash argument",
                    span,
                ))
            }
        };

        use crate::interpreter::value::HashKey;
        let page = match params.get(&HashKey::String("page".into())) {
            Some(Value::Int(n)) if *n > 0 => *n as usize,
            _ => 1,
        };
        let per = match params.get(&HashKey::String("per".into())) {
            Some(Value::Int(n)) if *n > 0 => *n as usize,
            _ => 25,
        };

        let total_val =
            crate::interpreter::builtins::model::execute_query_builder_count(&qb.borrow());
        let total = match total_val {
            Value::Int(n) => n as usize,
            _ => 0,
        };

        let total_pages = if total == 0 { 1 } else { total.div_ceil(per) };
        let page = if page > total_pages {
            total_pages
        } else {
            page
        };
        let offset = (page - 1) * per;

        let mut new_qb = qb.borrow().clone();
        new_qb.set_offset(offset);
        new_qb.set_limit(per);

        let records = crate::interpreter::builtins::model::execute_query_builder(&new_qb);

        let mut pagination = crate::interpreter::value::HashPairs::default();
        pagination.insert(HashKey::String("page".into()), Value::Int(page as i64));
        pagination.insert(HashKey::String("per".into()), Value::Int(per as i64));
        pagination.insert(HashKey::String("total".into()), Value::Int(total as i64));
        pagination.insert(
            HashKey::String("total_pages".into()),
            Value::Int(total_pages as i64),
        );

        let mut result = crate::interpreter::value::HashPairs::default();
        result.insert(HashKey::String("records".into()), records);
        result.insert(
            HashKey::String("pagination".into()),
            Value::Hash(Rc::new(RefCell::new(pagination))),
        );

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }

    fn qb_exists(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        // Sets exists mode on QB — use .first to execute, .to_query to inspect
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut new_qb = qb.borrow().clone();
        new_qb.exists_mode = true;
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_delete_all(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if qb.borrow().through.is_some() {
            return Err(RuntimeError::General {
                message: "delete_all on a through: relation is not supported — it would remove \
target rows, not join rows; delete the through relation's records instead"
                    .to_string(),
                span,
            });
        }
        Ok(execute_query_builder_delete_all(&qb.borrow()))
    }

    fn qb_update_all(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        if qb.borrow().through.is_some() {
            return Err(RuntimeError::General {
                message: "update_all on a through: relation is not supported — update the \
through relation's records instead"
                    .to_string(),
                span,
            });
        }
        {
            let qb_ref = qb.borrow();
            let class_name = crate::interpreter::symbol_string(qb_ref.class_name)
                .unwrap_or_default()
                .to_string();
            if crate::interpreter::builtins::model::is_timeseries_model(&class_name) {
                return Err(RuntimeError::General {
                    message: crate::interpreter::builtins::model::timeseries_insert_only_error(
                        &class_name,
                        "update_all",
                    ),
                    span,
                });
            }
        }
        let update_data = match &arguments[0] {
            hash @ Value::Hash(_) => crate::interpreter::value::value_to_json(hash)
                .map_err(|e| RuntimeError::type_error(e, span))?,
            other => {
                return Err(RuntimeError::type_error(
                    format!("update_all() expects a hash, got {}", other.type_name()),
                    span,
                ))
            }
        };
        Ok(execute_query_builder_update_all(&qb.borrow(), update_data))
    }

    fn qb_pluck(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        // pluck accepts one or more field names: pluck("name") or pluck("name", "email")
        if arguments.is_empty() {
            return Err(RuntimeError::new(
                "pluck() requires at least one field name",
                span,
            ));
        }
        let mut fields = Vec::new();
        for arg in &arguments {
            match arg {
                Value::String(s) => {
                    crate::interpreter::builtins::model::validate_field_name(s, "pluck")
                        .map_err(|e| RuntimeError::type_error(e, span))?;
                    fields.push(s.to_string());
                }
                _other => {
                    return Err(RuntimeError::type_error(
                        "pluck() expects string field names",
                        span,
                    ))
                }
            }
        }
        let mut new_qb = qb.borrow().clone();
        new_qb.set_pluck(fields);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_aggregate(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
        func: AggregationFunc,
    ) -> RuntimeResult<Value> {
        // sum("field"), avg("field"), min("field"), max("field")
        // Sets aggregation mode on QB — use .first to execute, .to_query to inspect
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let field = match &arguments[0] {
            Value::String(s) => s.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "aggregate function expects string field name",
                    span,
                ))
            }
        };
        crate::interpreter::builtins::model::validate_field_name(&field, "aggregate")
            .map_err(|e| RuntimeError::type_error(e, span))?;
        let mut new_qb = qb.borrow().clone();
        new_qb.aggregation = Some((func, field.to_string()));
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    fn qb_group_by(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        // 1-arg form: group_by("country") or group_by(["country", "plan"]) —
        // sets multi-key grouping; combine with .aggregate({...})/.having().
        // Without aggregates, an implicit per-group count applies.
        if arguments.len() == 1 {
            let fields: Vec<String> = match &arguments[0] {
                Value::String(s) => vec![s.to_string()],
                Value::Array(arr) => {
                    let arr = arr.borrow();
                    let mut out = Vec::with_capacity(arr.len());
                    for v in arr.iter() {
                        match v {
                            Value::String(s) => out.push(s.to_string()),
                            other => {
                                return Err(RuntimeError::type_error(
                                    format!(
                                        "group_by() expects string field names, got {} in array",
                                        other.type_name()
                                    ),
                                    span,
                                ))
                            }
                        }
                    }
                    out
                }
                other => {
                    return Err(RuntimeError::type_error(
                        format!(
                            "group_by() expects a field name or array of field names, got {}",
                            other.type_name()
                        ),
                        span,
                    ))
                }
            };
            if fields.is_empty() {
                return Err(RuntimeError::new(
                    "group_by() requires at least one field",
                    span,
                ));
            }
            for f in &fields {
                crate::interpreter::builtins::model::validate_field_name(f, "group_by")
                    .map_err(|e| RuntimeError::type_error(e, span))?;
            }
            let mut new_qb = qb.borrow().clone();
            if new_qb.group_by_info.is_some() || new_qb.time_bucket_info.is_some() {
                return Err(RuntimeError::new(
                    "group_by() cannot be combined with the legacy 3-arg group_by or \
                     time_bucket",
                    span,
                ));
            }
            new_qb.group_fields = fields;
            return Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))));
        }

        // Legacy 3-arg form: group_by("field", "sum", "amount") — unchanged,
        // returns [{group, result}] rows.
        if arguments.len() < 3 {
            return Err(RuntimeError::type_error(
                "group_by() expects a field (or array of fields), or the legacy \
                 (field, func, agg_field) form",
                span,
            ));
        }
        let group_field = match &arguments[0] {
            Value::String(s) => s.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "group_by() expects string group field name",
                    span,
                ))
            }
        };
        crate::interpreter::builtins::model::validate_field_name(&group_field, "group_by")
            .map_err(|e| RuntimeError::type_error(e, span))?;
        let func_name = match &arguments[1] {
            Value::String(s) => s.clone().to_lowercase(),
            _ => {
                return Err(RuntimeError::type_error(
                    "group_by() expects string function name",
                    span,
                ))
            }
        };
        let agg_field = match &arguments[2] {
            Value::String(s) => s.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "group_by() expects string aggregate field name",
                    span,
                ))
            }
        };
        crate::interpreter::builtins::model::validate_field_name(&agg_field, "group_by")
            .map_err(|e| RuntimeError::type_error(e, span))?;
        let func = match func_name.as_str() {
            "sum" => AggregationFunc::Sum,
            "avg" => AggregationFunc::Avg,
            "min" => AggregationFunc::Min,
            "max" => AggregationFunc::Max,
            _ => {
                return Err(RuntimeError::new(
                    "group_by() function must be one of: sum, avg, min, max",
                    span,
                ))
            }
        };
        let mut new_qb = qb.borrow().clone();
        new_qb.group_by_info = Some((group_field.to_string(), func, agg_field.to_string()));
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    /// aggregate({alias: [func, field], ...}) — multi-aggregate spec for the
    /// grouped mode (with or without group_by). Repeated calls extend the
    /// spec list.
    fn qb_aggregate_spec(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let specs = crate::interpreter::builtins::model::parse_aggregate_spec_hash(&arguments[0])
            .map_err(|e| RuntimeError::General { message: e, span })?;
        let mut new_qb = qb.borrow().clone();
        if new_qb.group_by_info.is_some() || new_qb.time_bucket_info.is_some() {
            return Err(RuntimeError::new(
                "aggregate() cannot be combined with the legacy 3-arg group_by or time_bucket",
                span,
            ));
        }
        new_qb.aggregate_specs.extend(specs);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    /// having("total > @min", {min: ...}) — post-COLLECT filter over bare
    /// group/aggregate aliases. String is developer-trusted like string-form
    /// where(); binds merge into the query's bind vars.
    fn qb_having(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::type_error(
                "having() expects a filter string and an optional bind-vars hash",
                span,
            ));
        }
        let filter = match &arguments[0] {
            Value::String(s) => s.to_string(),
            other => {
                return Err(RuntimeError::type_error(
                    format!(
                        "having() expects a filter string, got {}",
                        other.type_name()
                    ),
                    span,
                ))
            }
        };
        let mut new_qb = qb.borrow().clone();
        if new_qb.group_fields.is_empty() && new_qb.aggregate_specs.is_empty() {
            return Err(RuntimeError::new(
                "having() requires group_by()/aggregate() earlier in the chain",
                span,
            ));
        }
        if let Some(Value::Hash(hash)) = arguments.get(1) {
            for (k, v) in hash.borrow().iter() {
                if let crate::interpreter::value::HashKey::String(key) = k {
                    let json_val =
                        crate::interpreter::builtins::model::ensure_string_form_bind_value(
                            v, key, "having",
                        )
                        .map_err(|e| RuntimeError::General { message: e, span })?;
                    new_qb
                        .bind_vars
                        .insert(crate::interpreter::get_symbol(key), json_val);
                }
            }
        } else if let Some(other) = arguments.get(1) {
            return Err(RuntimeError::type_error(
                format!(
                    "having() bind vars must be a hash, got {}",
                    other.type_name()
                ),
                span,
            ));
        }
        new_qb.having = Some(filter);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }

    /// time_bucket(interval, aggregates) — timeseries bucketed aggregation.
    /// `interval` is "<n><s|m|h|d>"; `aggregates` maps alias → field for
    /// sum/avg/min/max keys, plus `count: true`. Keyword style works too:
    /// `.time_bucket("1h", avg: "value")` (named args collapse to a hash).
    fn qb_time_bucket(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::type_error(
                "time_bucket(interval, {aggregates}) expects an interval string and an \
                 aggregates hash, e.g. time_bucket(\"1h\", { \"avg\": \"value\" })",
                span,
            ));
        }

        {
            let qb_ref = qb.borrow();
            if qb_ref.traversal.is_some()
                || qb_ref.group_by_info.is_some()
                || qb_ref.aggregation.is_some()
            {
                return Err(RuntimeError::General {
                    message: "time_bucket() cannot be combined with traverse()/group_by()/\
                              aggregations"
                        .to_string(),
                    span,
                });
            }
        }

        let class_name = {
            let qb_ref = qb.borrow();
            crate::interpreter::symbol_string(qb_ref.class_name)
                .unwrap_or_default()
                .to_string()
        };
        let spec = crate::interpreter::builtins::model::query::parse_time_bucket_args(
            &arguments[0],
            arguments.get(1),
            &class_name,
        )
        .map_err(|e| RuntimeError::General { message: e, span })?;

        let mut new_qb = qb.borrow().clone();
        new_qb.time_bucket_info = Some(spec);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
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
        let qb_ref = qb.borrow();

        // Handle special modes
        let (query, bind_vars) = if qb_ref.exists_mode {
            qb_ref.build_exists_query()
        } else if let Some((ref func, ref field)) = qb_ref.aggregation {
            crate::interpreter::builtins::model::build_aggregation_query(
                &qb_ref,
                func.clone(),
                field,
            )
        } else if !qb_ref.group_fields.is_empty() || !qb_ref.aggregate_specs.is_empty() {
            qb_ref
                .build_grouped_query()
                .map_err(|e| RuntimeError::General { message: e, span })?
        } else if let Some((ref group_field, ref func, ref agg_field)) = qb_ref.group_by_info {
            qb_ref.build_group_by_query(group_field, func.clone(), agg_field)
        } else if qb_ref.time_bucket_info.is_some() {
            qb_ref.build_time_bucket_query()
        } else {
            qb_ref.build_query()
        };

        if bind_vars.is_empty() {
            Ok(Value::String(query.into()))
        } else {
            Ok(Value::String(
                format!("{} | bind_vars: {:?}", query, bind_vars).into(),
            ))
        }
    }

    /// similar(query, [field], [k], [options]) — vector similarity. `query`
    /// is a text string (embedded client-side) or a numeric vector literal.
    /// Options: `exact: true` (force client-side exact cosine even with a
    /// declared vector index), `ef_search: n` (ANN search width).
    fn qb_similar(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        use crate::interpreter::builtins::model::query::{SimilarInput, SimilarSpec};

        if arguments.is_empty() || arguments.len() > 4 {
            return Err(RuntimeError::wrong_arity(4, arguments.len(), span));
        }
        let input = match &arguments[0] {
            Value::String(s) => SimilarInput::Text(s.to_string()),
            Value::Array(arr) => {
                let vec: Vec<f64> = arr
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => Ok(*n as f64),
                        Value::Float(f) => Ok(*f),
                        other => Err(RuntimeError::type_error(
                            format!(
                                "similar() vector entries must be numbers, got {}",
                                other.type_name()
                            ),
                            span,
                        )),
                    })
                    .collect::<Result<_, _>>()?;
                if vec.is_empty() {
                    return Err(RuntimeError::new("similar() vector is empty", span));
                }
                SimilarInput::Vector(vec)
            }
            other => {
                return Err(RuntimeError::type_error(
                    format!(
                        "similar() expects query text or a numeric vector, got {}",
                        other.type_name()
                    ),
                    span,
                ))
            }
        };
        let field = match arguments.get(1) {
            Some(Value::String(s)) => s.clone(),
            _ => "embedding".into(),
        };
        let top_k = match arguments.get(2) {
            Some(Value::Int(n)) if *n > 0 => *n as usize,
            _ => 10,
        };
        let mut exact = false;
        let mut ef_search: Option<usize> = None;
        if let Some(Value::Hash(opts)) = arguments.get(3) {
            for (k, v) in opts.borrow().iter() {
                let key = match k {
                    crate::interpreter::value::HashKey::String(s) => s.to_string(),
                    _ => continue,
                };
                match (key.as_str(), v) {
                    ("exact", Value::Bool(b)) => exact = *b,
                    ("ef_search", Value::Int(n)) if *n > 0 => ef_search = Some(*n as usize),
                    (other, _) => {
                        return Err(RuntimeError::General {
                            message: format!(
                                "similar() unknown/invalid option '{}': expected exact: Bool \
                                 or ef_search: Int",
                                other
                            ),
                            span,
                        })
                    }
                }
            }
        }

        let mut new_qb = qb.borrow().clone();
        new_qb.similar_query = Some(SimilarSpec {
            input,
            field: field.to_string(),
            top_k,
            exact,
            ef_search,
        });
        // Similar search results always need a limit (top_k)
        if new_qb.limit_val.is_none() || new_qb.limit_val.unwrap() > top_k {
            new_qb.limit_val = Some(top_k);
        }
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
    }
}
