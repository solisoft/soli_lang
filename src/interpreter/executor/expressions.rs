//! Expression evaluation.

use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::ast::expr::Argument;
use crate::ast::*;
use crate::error::RuntimeError;
use crate::interpreter::builtins::model::{
    execute_query_builder, execute_query_builder_count, execute_query_builder_first,
};
use crate::interpreter::builtins::server::is_server_listen_marker;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Function, HashKey, Instance, NativeFunction, Value, ValueMethod};
use crate::span::Span;

use super::{ControlFlow, Interpreter, RuntimeResult};

impl Interpreter {
    /// Evaluate an expression.
    pub(crate) fn evaluate(&mut self, expr: &Expr) -> RuntimeResult<Value> {
        self.record_coverage(expr.span.line);
        match &expr.kind {
            ExprKind::IntLiteral(n) => Ok(Value::Int(*n)),
            ExprKind::FloatLiteral(n) => Ok(Value::Float(*n)),
            ExprKind::StringLiteral(s) => Ok(Value::String(s.clone())),
            ExprKind::BoolLiteral(b) => Ok(Value::Bool(*b)),
            ExprKind::Null => Ok(Value::Null),

            ExprKind::Variable(name) => self
                .environment
                .borrow()
                .get(name)
                .ok_or_else(|| RuntimeError::undefined_variable(name, expr.span)),

            ExprKind::Grouping(inner) => self.evaluate(inner),

            ExprKind::Binary {
                left,
                operator,
                right,
            } => self.evaluate_binary(left, *operator, right, expr.span),

            ExprKind::Unary { operator, operand } => {
                self.evaluate_unary(*operator, operand, expr.span)
            }

            ExprKind::LogicalAnd { left, right } => {
                let left_val = self.evaluate(left)?;
                if !left_val.is_truthy() {
                    Ok(left_val)
                } else {
                    self.evaluate(right)
                }
            }

            ExprKind::LogicalOr { left, right } => {
                let left_val = self.evaluate(left)?;
                if left_val.is_truthy() {
                    Ok(left_val)
                } else {
                    self.evaluate(right)
                }
            }

            ExprKind::NullishCoalescing { left, right } => {
                let left_val = self.evaluate(left)?;
                if matches!(left_val, Value::Null) {
                    self.evaluate(right)
                } else {
                    Ok(left_val)
                }
            }

            ExprKind::Call { callee, arguments } => {
                self.evaluate_call(callee, arguments, expr.span)
            }

            ExprKind::Pipeline { left, right } => self.evaluate_pipeline(left, right, expr.span),

            ExprKind::Member { object, name } => self.evaluate_member(object, name, expr.span),

            ExprKind::QualifiedName { qualifier, name } => {
                self.evaluate_qualified_name(qualifier, name, expr.span)
            }

            ExprKind::Index { object, index } => self.evaluate_index(object, index, expr.span),

            ExprKind::This => self
                .environment
                .borrow()
                .get("this")
                .ok_or_else(|| RuntimeError::type_error("'this' outside of class", expr.span)),

            ExprKind::Super => {
                // First check if we're in a super call (stored by call_function)
                if let Some(Value::Class(superclass)) =
                    self.environment.borrow().get("__defining_superclass__")
                {
                    return Ok(Value::Super(superclass.clone()));
                }

                // Get 'this' to find the current instance
                let this_val = self.environment.borrow().get("this").ok_or_else(|| {
                    RuntimeError::type_error("'super' outside of class", expr.span)
                })?;

                // Get the instance's class
                let inst = match this_val {
                    Value::Instance(inst) => inst,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "'super' outside of class",
                            expr.span,
                        ))
                    }
                };

                // Get the superclass
                let superclass = match &inst.borrow().class.superclass {
                    Some(sc) => sc.clone(),
                    None => {
                        return Err(RuntimeError::type_error(
                            "class has no superclass",
                            expr.span,
                        ))
                    }
                };

                Ok(Value::Super(superclass))
            }

            ExprKind::New {
                class_expr,
                arguments,
            } => self.evaluate_new(class_expr, arguments, expr.span),

            ExprKind::Array(elements) => {
                let mut values = Vec::new();
                for elem in elements {
                    match &elem.kind {
                        ExprKind::Spread(inner) => {
                            // Evaluate the spread expression and extend with its elements
                            let spread_val = self.evaluate(inner)?;
                            match spread_val {
                                Value::Array(ref arr) => {
                                    let arr = arr.borrow();
                                    values.extend(arr.clone());
                                }
                                _ => {
                                    return Err(RuntimeError::type_error(
                                        "cannot spread non-array value",
                                        elem.span,
                                    ));
                                }
                            }
                        }
                        _ => {
                            values.push(self.evaluate(elem)?);
                        }
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }

            ExprKind::Hash(pairs) => self.evaluate_hash(pairs),

            ExprKind::Block(statements) => {
                let env = Environment::with_enclosing(self.environment.clone());
                match self.execute_block(statements, env)? {
                    ControlFlow::Normal => Ok(Value::Null),
                    ControlFlow::Return(v) => Ok(v),
                    ControlFlow::Throw(e) => Err(RuntimeError::General {
                        message: format!("Unhandled exception: {}", e),
                        span: expr.span,
                    }),
                }
            }

            ExprKind::Assign { target, value } => self.evaluate_assign(target, value),

            ExprKind::Lambda { params, body, .. } => self.evaluate_lambda(params, body, expr.span),

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_value = self.evaluate(condition)?;
                if cond_value.is_truthy() {
                    self.evaluate(then_branch)
                } else {
                    match else_branch {
                        Some(else_expr) => self.evaluate(else_expr),
                        None => Ok(Value::Null),
                    }
                }
            }

            ExprKind::InterpolatedString(parts) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        crate::ast::expr::InterpolatedPart::Literal(s) => {
                            result.push_str(&s);
                        }
                        crate::ast::expr::InterpolatedPart::Expression(expr) => {
                            let value = self.evaluate(expr)?;
                            result.push_str(&value.to_string());
                        }
                    }
                }
                Ok(Value::String(result))
            }
            ExprKind::Match { expression, arms } => {
                let input_value = self.evaluate(expression)?;

                for arm in arms {
                    if let Some(bindings) = self.match_pattern(&input_value, &arm.pattern)? {
                        let env = self.environment.clone();

                        for (name, value) in bindings {
                            env.borrow_mut().define(name, value);
                        }

                        if let Some(guard) = &arm.guard {
                            let guard_value = self.evaluate(guard)?;
                            if !guard_value.is_truthy() {
                                continue;
                            }
                        }

                        return self.evaluate(&arm.body);
                    }
                }

                Err(RuntimeError::type_error(
                    "no pattern matched the value",
                    expr.span,
                ))
            }
            ExprKind::ListComprehension {
                element,
                variable,
                iterable,
                condition,
            } => {
                // Evaluate the iterable
                let iter_value = self.evaluate(iterable)?;
                // Clone items once to avoid holding borrow across loop body
                let items: Vec<Value> = match iter_value {
                    Value::Array(arr) => arr.borrow().iter().cloned().collect(),
                    _ => {
                        return Err(RuntimeError::type_error("expected array", iterable.span));
                    }
                };

                // Create a new environment for the comprehension
                let mut result = Vec::new();
                for item in items {
                    // Create a new scope for each iteration with the loop variable already defined
                    let mut loop_env = Environment::with_enclosing(self.environment.clone());
                    loop_env.define(variable.clone(), item);
                    let prev_env =
                        std::mem::replace(&mut self.environment, Rc::new(RefCell::new(loop_env)));

                    // Check condition if present and evaluate element
                    let pass_condition = if let Some(cond) = condition {
                        let cond_value = self.evaluate(cond)?;
                        if !cond_value.is_truthy() {
                            self.environment = prev_env;
                            continue;
                        }
                        true
                    } else {
                        true
                    };

                    // Evaluate the element expression
                    let elem_value = self.evaluate(element)?;

                    // Restore previous environment
                    self.environment = prev_env;

                    if pass_condition {
                        result.push(elem_value);
                    }
                }

                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            ExprKind::HashComprehension {
                key,
                value,
                variable,
                iterable,
                condition,
            } => {
                // Evaluate the iterable
                let iter_value = self.evaluate(iterable)?;
                // Clone items once to avoid holding borrow across loop body
                let items: Vec<Value> = match iter_value {
                    Value::Array(arr) => arr.borrow().iter().cloned().collect(),
                    _ => {
                        return Err(RuntimeError::type_error("expected array", iterable.span));
                    }
                };

                // Create a new environment for the comprehension
                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for item in items {
                    // Create a new scope for each iteration with the loop variable already defined
                    let mut loop_env = Environment::with_enclosing(self.environment.clone());
                    loop_env.define(variable.clone(), item);
                    let prev_env =
                        std::mem::replace(&mut self.environment, Rc::new(RefCell::new(loop_env)));

                    // Check condition if present and evaluate key/value
                    let should_include = if let Some(cond) = condition {
                        let cond_value = self.evaluate(cond)?;
                        if !cond_value.is_truthy() {
                            self.environment = prev_env;
                            continue;
                        }
                        true
                    } else {
                        true
                    };

                    // Evaluate the key and value expressions
                    let key_value = self.evaluate(key)?;
                    let val_value = self.evaluate(value)?;

                    // Restore previous environment
                    self.environment = prev_env;

                    if should_include {
                        if let Some(hash_key) = key_value.to_hash_key() {
                            result.insert(hash_key, val_value);
                        } else {
                            return Err(RuntimeError::type_error(
                                format!("{} cannot be used as a hash key", key_value.type_name()),
                                key.span,
                            ));
                        }
                    }
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            ExprKind::Await(_) => {
                unimplemented!("Await expressions not yet implemented")
            }
            ExprKind::Spread(_) => {
                unimplemented!("Spread expressions not yet implemented")
            }
            ExprKind::Throw(value) => {
                let error_value = self.evaluate(value)?;
                Err(RuntimeError::General {
                    message: format!("{}", error_value),
                    span: expr.span,
                })
            }
        }
    }

    fn evaluate_pipeline(&mut self, left: &Expr, right: &Expr, span: Span) -> RuntimeResult<Value> {
        // x |> foo() becomes foo(x)
        // x |> f becomes f(x)
        let left_val = self.evaluate(left)?;

        match &right.kind {
            ExprKind::Call { callee, arguments } => {
                // Check for array methods: map, filter, each
                // These need special handling because they take a function argument
                if let ExprKind::Variable(name) = &callee.kind {
                    if matches!(name.as_str(), "map" | "filter" | "each") {
                        // Resolve the array from left_val
                        let resolved = left_val
                            .resolve()
                            .map_err(|e| RuntimeError::type_error(e, span))?;
                        if let Value::Array(arr) = resolved {
                            let items: Vec<Value> = arr.borrow().clone();
                            // Evaluate the function argument
                            let mut args = Vec::new();
                            for arg in arguments {
                                match arg {
                                    Argument::Positional(expr) => {
                                        args.push(self.evaluate(expr)?);
                                    }
                                    Argument::Named(_) => {
                                        return Err(RuntimeError::type_error(
                                            "pipeline method does not support named arguments",
                                            span,
                                        ));
                                    }
                                }
                            }
                            return self.call_array_method(&items, name, args, span);
                        } else {
                            return Err(RuntimeError::type_error(
                                format!("{}() expects array, got {}", name, resolved.type_name()),
                                span,
                            ));
                        }
                    }
                }

                // Prepend left_val to arguments
                let mut new_args = vec![left_val];
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => {
                            new_args.push(self.evaluate(expr)?);
                        }
                        Argument::Named(_) => {
                            return Err(RuntimeError::type_error(
                                "pipeline method does not support named arguments",
                                span,
                            ));
                        }
                    }
                }

                let callee_val = self.evaluate(callee)?;
                self.call_value(callee_val, new_args, span)
            }
            _ => {
                // Try evaluating right as a function value
                let right_val = self.evaluate(right)?;
                match right_val {
                    Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => {
                        self.call_value(right_val, vec![left_val], span)
                    }
                    _ => Err(RuntimeError::type_error(
                        "right side of pipeline must be a function call or a function value",
                        right.span,
                    )),
                }
            }
        }
    }

    fn evaluate_member(&mut self, object: &Expr, name: &str, span: Span) -> RuntimeResult<Value> {
        let obj_val = self.evaluate(object)?;
        match obj_val {
            Value::Instance(inst) => {
                // Single borrow for all member access checks
                let inst_ref = inst.borrow();

                // First check for field
                if let Some(value) = inst_ref.get(name) {
                    return Ok(value);
                }

                // Then check for native method
                if let Some(native_method) = inst_ref.class.find_native_method(name) {
                    // Create a wrapper that will call the native method with 'this'
                    let class_name = inst_ref.class.name.clone();
                    let native_method_clone = native_method.clone();
                    // Drop borrow before capturing inst in closure
                    drop(inst_ref);
                    let instance_clone = inst.clone();
                    return Ok(Value::NativeFunction(NativeFunction::new(
                        &format!("{}.{}", class_name, name),
                        None,
                        move |args| {
                            // Prepend 'this' as first argument (the instance)
                            let mut new_args = vec![Value::Instance(instance_clone.clone())];
                            new_args.extend(args.iter().cloned());
                            (native_method_clone.func)(new_args)
                        },
                    )));
                }

                // Then check for user-defined method
                if let Some(method) = inst_ref.class.find_method(name) {
                    // Get class name for potential error before dropping borrow
                    let _class_name = inst_ref.class.name.clone();
                    // Drop borrow before creating new environment with inst
                    drop(inst_ref);

                    // Bind 'this'
                    let mut bound_env = Environment::with_enclosing(method.closure.clone());
                    bound_env.define("this".to_string(), Value::Instance(inst.clone()));

                    let bound_method = Function {
                        name: method.name.clone(),
                        params: method.params.clone(),
                        body: method.body.clone(),
                        closure: Rc::new(RefCell::new(bound_env)),
                        is_method: true,
                        span: method.span,
                        source_path: method.source_path.clone(),
                        defining_superclass: None,
                    };
                    return Ok(Value::Function(Rc::new(bound_method)));
                }

                // Get class name for error before borrow ends
                let class_name = inst_ref.class.name.clone();
                Err(RuntimeError::NoSuchProperty {
                    value_type: class_name,
                    property: name.to_string(),
                    span,
                })
            }
            Value::Class(ref class) => {
                // Static method access - search up superclass chain
                if let Some(method) = class.find_static_method(name) {
                    return Ok(Value::Function(method));
                }
                if let Some(native_method) = class.find_native_static_method(name) {
                    // Check if this is a Model subclass - if so, create bound function
                    if class.is_model_subclass() {
                        // Create a new native function that captures the class
                        let class_val = obj_val.clone();
                        let method_name = name.to_string();
                        let original_func = native_method.func.clone();
                        let original_arity = native_method.arity;

                        // Adjust arity (user passes N-1 args, we prepend class)
                        let user_arity = original_arity.map(|a| a.saturating_sub(1));

                        let bound_func = NativeFunction::new(
                            Box::leak(format!("bound_{}", method_name).into_boxed_str()),
                            user_arity,
                            move |args| {
                                let mut full_args = vec![class_val.clone()];
                                full_args.extend(args);
                                original_func(full_args)
                            },
                        );
                        return Ok(Value::NativeFunction(bound_func));
                    }
                    // For non-Model classes, return the native function directly
                    return Ok(Value::NativeFunction((*native_method).clone()));
                }
                // Check for static field
                // Check for static field (including inherited static fields)
                fn find_static_field(
                    class: &crate::interpreter::value::Class,
                    name: &str,
                ) -> Option<Value> {
                    if let Some(value) = class.static_fields.borrow().get(name) {
                        return Some(value.clone());
                    }
                    if let Some(ref superclass) = class.superclass {
                        return find_static_field(superclass, name);
                    }
                    None
                }

                if let Some(value) = find_static_field(class, name) {
                    return Ok(value);
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Super(ref superclass) => {
                // Get 'this' instance for field lookup
                let this_val =
                    self.environment.borrow().get("this").ok_or_else(|| {
                        RuntimeError::type_error("'super' outside of class", span)
                    })?;

                let instance = match this_val {
                    Value::Instance(inst) => inst,
                    _ => return Err(RuntimeError::type_error("'super' outside of class", span)),
                };

                // First check for field in the instance (fields are inherited at instance creation)
                // Fields on the instance are accessible via 'this', not 'super'
                if let Some(value) = instance.borrow().get(name) {
                    return Ok(value);
                }

                // super.method() - look up method in superclass
                if let Some(method) = superclass.find_method(name) {
                    // Bind 'this' to the method (defining_superclass is already in method.closure)
                    let bound_env = Environment::with_enclosing(method.closure.clone());
                    let mut bound_env = bound_env;
                    bound_env.define("this".to_string(), Value::Instance(instance.clone()));
                    // Note: __defining_superclass__ is already defined in method.closure

                    let bound_method = Function {
                        name: method.name.clone(),
                        params: method.params.clone(),
                        body: method.body.clone(),
                        closure: Rc::new(RefCell::new(bound_env)),
                        is_method: true,
                        span: method.span,
                        source_path: method.source_path.clone(),
                        defining_superclass: None,
                    };
                    return Ok(Value::Function(Rc::new(bound_method)));
                }
                // Also check for native methods in superclass
                if let Some(native_method) = superclass.find_native_method(name) {
                    let instance_clone = instance.clone();
                    let native_method_clone = native_method.clone();
                    return Ok(Value::NativeFunction(NativeFunction::new(
                        &format!("{}.{}", superclass.name, name),
                        None,
                        move |args| {
                            let mut new_args = vec![Value::Instance(instance_clone.clone())];
                            new_args.extend(args.iter().cloned());
                            (native_method_clone.func)(new_args)
                        },
                    )));
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: superclass.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Array(ref _arr) => {
                // Handle array methods: map, filter, each, reduce, find, any?, all?, sort, reverse, uniq, compact, flatten, first, last, empty?, include?, sample, shuffle, take, drop, zip, sum, min, max
                match name {
                    "length" | "map" | "filter" | "each" | "reduce" | "find" | "any?" | "all?"
                    | "sort" | "reverse" | "uniq" | "compact" | "flatten" | "first" | "last"
                    | "empty?" | "include?" | "sample" | "shuffle" | "take" | "drop" | "zip"
                    | "sum" | "min" | "max" | "push" | "pop" | "clear" | "get" | "to_string"
                    | "join" => Ok(Value::Method(ValueMethod {
                        receiver: Box::new(obj_val),
                        method_name: name.to_string(),
                    })),
                    _ => Err(RuntimeError::NoSuchProperty {
                        value_type: "Array".to_string(),
                        property: name.to_string(),
                        span,
                    }),
                }
            }
            Value::Hash(ref hash) => {
                // First check if it's a known method
                match name {
                    "length" | "map" | "filter" | "each" | "get" | "fetch" | "invert"
                    | "transform_values" | "transform_keys" | "select" | "reject" | "slice"
                    | "except" | "compact" | "dig" | "to_string" | "keys" | "values"
                    | "has_key" | "delete" | "merge" | "entries" | "clear" | "set" | "empty?" => {
                        Ok(Value::Method(ValueMethod {
                            receiver: Box::new(obj_val),
                            method_name: name.to_string(),
                        }))
                    }
                    _ => {
                        // Try to access as a hash key (dot notation for hash access) - O(1)
                        let hash_key = HashKey::String(name.to_string());
                        if let Some(v) = hash.borrow().get(&hash_key) {
                            return Ok(v.clone());
                        }
                        // Key not found, return null (like JavaScript undefined)
                        Ok(Value::Null)
                    }
                }
            }
            Value::QueryBuilder(_) => {
                // Handle QueryBuilder methods for chaining
                match name {
                    "where" | "order" | "limit" | "offset" | "all" | "first" | "count" => {
                        Ok(Value::Method(ValueMethod {
                            receiver: Box::new(obj_val),
                            method_name: name.to_string(),
                        }))
                    }
                    _ => Err(RuntimeError::NoSuchProperty {
                        value_type: "QueryBuilder".to_string(),
                        property: name.to_string(),
                        span,
                    }),
                }
            }
            Value::String(ref _s) => {
                // Handle string methods and properties
                match name {
                    // Core string methods from collection_classes.rs
                    "length" | "to_string" | "upcase" | "uppercase" | "downcase" | "lowercase" | "trim" | "contains" | "starts_with"
                    | "ends_with" | "split" | "index_of" | "substring" | "replace" | "lpad"
                    | "rpad" | "join" | "empty?"
                    // Ruby-style methods
                    | "starts_with?" | "ends_with?" | "include?" | "chomp" | "lstrip" | "rstrip" | "squeeze"
                    | "count" | "gsub" | "sub" | "match" | "scan" | "tr" | "center" | "ljust"
                    | "rjust" | "ord" | "chr" | "bytes" | "chars" | "lines" | "bytesize"
                    | "capitalize" | "swapcase" | "insert" | "delete" | "delete_prefix"
                    | "delete_suffix" | "partition" | "rpartition" | "reverse" | "hex" | "oct"
                    | "truncate" => Ok(Value::Method(ValueMethod {
                        receiver: Box::new(obj_val),
                        method_name: name.to_string(),
                    })),
                    _ => Err(RuntimeError::NoSuchProperty {
                        value_type: "String".to_string(),
                        property: name.to_string(),
                        span,
                    }),
                }
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: obj_val.type_name().to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn evaluate_qualified_name(
        &mut self,
        qualifier: &Expr,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Value> {
        let qualifier_val = self.evaluate(qualifier)?;

        match qualifier_val {
            Value::Class(class) => {
                // Check if this is a nested class
                if let Some(nested_class) = class.nested_classes.borrow().get(name) {
                    return Ok(Value::Class(nested_class.clone()));
                }

                // Also check static fields (nested classes might be stored there)
                if let Some(value) = class.static_fields.borrow().get(name).cloned() {
                    return Ok(value);
                }

                Err(RuntimeError::NoSuchProperty {
                    value_type: class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            _ => Err(RuntimeError::type_error(
                &format!("'{}' is not a class", qualifier_val.type_name()),
                span,
            )),
        }
    }

    fn evaluate_index(&mut self, object: &Expr, index: &Expr, span: Span) -> RuntimeResult<Value> {
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
                    .ok_or_else(|| RuntimeError::IndexOutOfBounds {
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
                // O(1) lookup
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

    fn evaluate_new(
        &mut self,
        class_expr: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Value> {
        use std::collections::{HashMap, HashSet};

        // Evaluate the class expression to get the class
        let class_val = self.evaluate(class_expr)?;

        let class = match class_val {
            Value::Class(c) => c,
            _ => {
                return Err(RuntimeError::type_error(
                    &format!("expected class name, got {}", class_val.type_name()),
                    span,
                ));
            }
        };

        // Create instance
        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));

        // Initialize fields from class field declarations (including inherited fields)
        fn initialize_fields(
            interpreter: &mut Interpreter,
            class: &crate::interpreter::value::Class,
            instance: &Rc<std::cell::RefCell<crate::interpreter::value::Instance>>,
        ) -> RuntimeResult<()> {
            // First initialize fields from superclass
            if let Some(ref superclass) = class.superclass {
                initialize_fields(interpreter, superclass, instance)?;
            }
            // Then initialize fields from this class
            for (field_name, field_initializer) in &class.fields {
                let value = if let Some(init_expr) = field_initializer {
                    interpreter.evaluate(init_expr)?
                } else {
                    Value::Null
                };
                instance.borrow_mut().set(field_name.clone(), value);
            }
            Ok(())
        }
        initialize_fields(self, &class, &instance)?;

        // Call constructor if present (check superclass chain if needed)
        if let Some(ctor) = class.find_constructor() {
            let ctor = &ctor;
            let mut positional_args = Vec::new();
            let mut named_args = HashMap::new();

            for arg in arguments {
                match arg {
                    Argument::Positional(expr) => {
                        positional_args.push(self.evaluate(expr)?);
                    }
                    Argument::Named(named) => {
                        if named_args.contains_key(&named.name) {
                            return Err(RuntimeError::type_error(
                                format!("duplicate named argument '{}'", named.name),
                                named.span,
                            ));
                        }
                        named_args.insert(named.name.clone(), self.evaluate(&named.value)?);
                    }
                }
            }

            let param_names: Vec<String> = ctor.params.iter().map(|p| p.name.clone()).collect();

            // Check for unknown named arguments
            for name in named_args.keys() {
                if !param_names.contains(name) {
                    return Err(RuntimeError::undefined_variable(name.clone(), span));
                }
            }

            // Build constructor arguments
            let mut ctor_args = Vec::new();
            let mut used_params = HashSet::new();

            // Positional arguments first
            for (i, arg_val) in positional_args.iter().enumerate() {
                if i < param_names.len() {
                    ctor_args.push(arg_val.clone());
                    used_params.insert(param_names[i].clone());
                } else {
                    return Err(RuntimeError::wrong_arity(
                        ctor.full_arity(),
                        positional_args.len() + named_args.len(),
                        span,
                    ));
                }
            }

            // Named arguments and defaults
            for (i, param_name) in param_names.iter().enumerate() {
                if used_params.contains(param_name) {
                    continue;
                }
                if let Some(named_val) = named_args.get(param_name) {
                    ctor_args.push(named_val.clone());
                } else if let Some(default_expr) = ctor.param_default_value(i) {
                    let default_value = self.evaluate(default_expr)?;
                    ctor_args.push(default_value);
                } else {
                    return Err(RuntimeError::wrong_arity(
                        ctor.arity(),
                        ctor_args.len(),
                        span,
                    ));
                }
            }

            // Create constructor environment
            let ctor_env = Environment::with_enclosing(ctor.closure.clone());
            let mut ctor_env = ctor_env;
            ctor_env.define("this".to_string(), Value::Instance(instance.clone()));

            for (param, value) in ctor.params.iter().zip(ctor_args.iter()) {
                ctor_env.define(param.name.clone(), value.clone());
            }

            // Execute constructor body
            let _ = self.execute_block(&ctor.body, ctor_env);
        }

        Ok(Value::Instance(instance))
    }

    fn evaluate_hash(&mut self, pairs: &[(Expr, Expr)]) -> RuntimeResult<Value> {
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

    fn evaluate_assign(&mut self, target: &Expr, value: &Expr) -> RuntimeResult<Value> {
        let new_value = self.evaluate(value)?;

        match &target.kind {
            ExprKind::Variable(name) => {
                // Check if the variable is a const
                if self.environment.borrow().is_const(name) {
                    return Err(RuntimeError::type_error(
                        format!("cannot reassign constant '{}'", name),
                        target.span,
                    ));
                }
                if !self
                    .environment
                    .borrow_mut()
                    .assign(name, new_value.clone())
                {
                    return Err(RuntimeError::undefined_variable(name, target.span));
                }
                Ok(new_value)
            }
            ExprKind::Member { object, name } => {
                let obj_val = self.evaluate(object)?;
                match obj_val {
                    Value::Instance(inst) => {
                        inst.borrow_mut().set(name.clone(), new_value.clone());
                        Ok(new_value)
                    }
                    Value::Class(class) => {
                        // Set static field on class
                        class
                            .static_fields
                            .borrow_mut()
                            .insert(name.clone(), new_value.clone());
                        Ok(new_value)
                    }
                    _ => Err(RuntimeError::type_error(
                        format!("cannot set property on {}", obj_val.type_name()),
                        target.span,
                    )),
                }
            }
            ExprKind::Index { object, index } => {
                self.evaluate_index_assign(object, index, new_value, target.span)
            }
            _ => Err(RuntimeError::type_error(
                "invalid assignment target",
                target.span,
            )),
        }
    }

    fn evaluate_index_assign(
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
                // O(1) insert/update
                hash.borrow_mut().insert(hash_key, new_value.clone());
                Ok(new_value)
            }
            _ => Err(RuntimeError::type_error("invalid assignment target", span)),
        }
    }

    fn evaluate_lambda(
        &mut self,
        params: &[Parameter],
        body: &[Stmt],
        span: Span,
    ) -> RuntimeResult<Value> {
        let func = Function {
            name: "<lambda>".to_string(),
            params: params.to_vec(),
            body: body.to_vec(),
            closure: self.environment.clone(),
            is_method: false,
            span: Some(span),
            source_path: self
                .current_source_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            defining_superclass: None,
        };
        Ok(Value::Function(Rc::new(func)))
    }

    fn evaluate_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Value> {
        let callee_val = self.evaluate(callee)?;

        let mut arg_values = Vec::new();
        let mut named_args = std::collections::HashMap::new();

        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    arg_values.push(self.evaluate(expr)?);
                }
                Argument::Named(named) => {
                    if named_args.contains_key(&named.name) {
                        return Err(RuntimeError::type_error(
                            format!("duplicate named argument '{}'", named.name),
                            named.span,
                        ));
                    }
                    named_args.insert(named.name.clone(), self.evaluate(&named.value)?);
                }
            }
        }

        self.call_value_with_named(callee_val, arg_values, named_args, span)
    }

    /// Helper function to call a value with both positional and named arguments.
    fn call_value_with_named(
        &mut self,
        callee: Value,
        positional_args: Vec<Value>,
        named_args: std::collections::HashMap<String, Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match callee {
            Value::Method(method) => {
                // Handle array/hash/QueryBuilder/String methods
                match *method.receiver {
                    Value::Array(ref arr) => {
                        // For mutating methods, we need the original Rc<RefCell>
                        match method.method_name.as_str() {
                            "push" => {
                                if positional_args.len() != 1 {
                                    return Err(RuntimeError::wrong_arity(
                                        1,
                                        positional_args.len(),
                                        span,
                                    ));
                                }
                                arr.borrow_mut().push(positional_args[0].clone());
                                Ok(Value::Null)
                            }
                            "pop" => {
                                if !positional_args.is_empty() {
                                    return Err(RuntimeError::wrong_arity(
                                        0,
                                        positional_args.len(),
                                        span,
                                    ));
                                }
                                arr.borrow_mut().pop().ok_or_else(|| {
                                    RuntimeError::type_error("pop on empty array", span)
                                })
                            }
                            "clear" => {
                                if !positional_args.is_empty() {
                                    return Err(RuntimeError::wrong_arity(
                                        0,
                                        positional_args.len(),
                                        span,
                                    ));
                                }
                                arr.borrow_mut().clear();
                                Ok(Value::Null)
                            }
                            _ => {
                                let items = arr.borrow().clone();
                                self.call_array_method(
                                    &items,
                                    &method.method_name,
                                    positional_args,
                                    span,
                                )
                            }
                        }
                    }
                    Value::Hash(ref hash) => {
                        // For mutating methods, we need the original Rc<RefCell>
                        match method.method_name.as_str() {
                            "set" => {
                                if positional_args.len() != 2 {
                                    return Err(RuntimeError::wrong_arity(
                                        2,
                                        positional_args.len(),
                                        span,
                                    ));
                                }
                                let key = &positional_args[0];
                                let value = positional_args[1].clone();
                                let hash_key = key.to_hash_key().ok_or_else(|| {
                                    RuntimeError::type_error(
                                        format!("{} cannot be used as a hash key", key.type_name()),
                                        span,
                                    )
                                })?;
                                // O(1) insert
                                hash.borrow_mut().insert(hash_key, value.clone());
                                Ok(value)
                            }
                            "delete" => {
                                if positional_args.len() != 1 {
                                    return Err(RuntimeError::wrong_arity(
                                        1,
                                        positional_args.len(),
                                        span,
                                    ));
                                }
                                let key = &positional_args[0];
                                let hash_key = match key.to_hash_key() {
                                    Some(k) => k,
                                    None => return Ok(Value::Null),
                                };
                                // O(1) removal
                                let deleted_value = hash.borrow_mut().shift_remove(&hash_key);
                                Ok(deleted_value.unwrap_or(Value::Null))
                            }
                            "clear" => {
                                if !positional_args.is_empty() {
                                    return Err(RuntimeError::wrong_arity(
                                        0,
                                        positional_args.len(),
                                        span,
                                    ));
                                }
                                hash.borrow_mut().clear();
                                Ok(Value::Null)
                            }
                            _ => {
                                let entries: Vec<(HashKey, Value)> = hash
                                    .borrow()
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone()))
                                    .collect();
                                self.call_hash_method(
                                    &entries,
                                    &method.method_name,
                                    positional_args,
                                    span,
                                )
                            }
                        }
                    }
                    Value::QueryBuilder(qb) => self.call_query_builder_method(
                        qb,
                        &method.method_name,
                        positional_args,
                        span,
                    ),
                    Value::String(s) => {
                        self.call_string_method(&s, &method.method_name, positional_args, span)
                    }
                    _ => Err(RuntimeError::type_error(
                        format!("{} does not support methods", method.receiver.type_name()),
                        span,
                    )),
                }
            }

            Value::Function(func) => {
                let required_arity = func.arity();
                let full_arity = func.full_arity();

                // Get parameter names for named argument matching
                let param_names: Vec<String> = func.params.iter().map(|p| p.name.clone()).collect();

                // Check for unknown named arguments
                for name in named_args.keys() {
                    if !param_names.contains(name) {
                        return Err(RuntimeError::undefined_variable(name.clone(), span));
                    }
                }

                // Build the final argument list in parameter order
                let mut final_args = Vec::new();
                let mut used_params = std::collections::HashSet::new();

                // First, add positional arguments
                for (i, arg_val) in positional_args.iter().enumerate() {
                    if i < param_names.len() {
                        final_args.push(arg_val.clone());
                        used_params.insert(param_names[i].clone());
                    } else {
                        // Too many positional arguments
                        return Err(RuntimeError::wrong_arity(
                            full_arity,
                            positional_args.len() + named_args.len(),
                            span,
                        ));
                    }
                }

                // Then, fill in named arguments and defaults
                for (i, param_name) in param_names.iter().enumerate() {
                    if used_params.contains(param_name) {
                        // Already provided as positional
                        continue;
                    }

                    if let Some(named_val) = named_args.get(param_name) {
                        // Named argument provided
                        final_args.push(named_val.clone());
                    } else if let Some(default_expr) = func.param_default_value(i) {
                        // Use default value
                        let default_value = self.evaluate(default_expr)?;
                        final_args.push(default_value);
                    } else {
                        // Required parameter missing
                        return Err(RuntimeError::wrong_arity(
                            required_arity,
                            final_args.len(),
                            span,
                        ));
                    }
                }

                // Final arity check
                if final_args.len() != full_arity {
                    return Err(RuntimeError::wrong_arity(
                        full_arity,
                        final_args.len(),
                        span,
                    ));
                }

                self.call_function(&func, final_args)
            }

            Value::NativeFunction(native) => {
                // Native functions only support positional arguments for now
                if positional_args.len() + named_args.len()
                    != native.arity.unwrap_or(positional_args.len())
                {
                    return Err(RuntimeError::wrong_arity(
                        native.arity.unwrap_or(0),
                        positional_args.len() + named_args.len(),
                        span,
                    ));
                }
                if !named_args.is_empty() {
                    return Err(RuntimeError::type_error(
                        "native functions do not support named arguments".to_string(),
                        span,
                    ));
                }
                let result = (native.func)(positional_args)
                    .map_err(|msg| RuntimeError::General { message: msg, span })?;

                // Check if this is the http_server_listen marker
                if let Some(port) = is_server_listen_marker(&result) {
                    // Only start server in main thread, not in worker threads
                    let thread_name = std::thread::current().name().map(|s| s.to_string());
                    let is_main_thread = thread_name
                        .as_ref()
                        .is_some_and(|n| n == "main" || n.starts_with("tokio-runtime"));
                    if is_main_thread {
                        // Run the HTTP server (this blocks until server stops)
                        return self.run_http_server(port);
                    }
                }

                Ok(result)
            }

            Value::Class(class) => {
                // Class instantiation with named arguments
                let instance = Rc::new(RefCell::new(Instance::new(class.clone())));

                if let Some(ref ctor) = class.constructor {
                    let required_arity = ctor.arity();
                    let full_arity = ctor.full_arity();

                    // Get parameter names
                    let param_names: Vec<String> =
                        ctor.params.iter().map(|p| p.name.clone()).collect();

                    // Check for unknown named arguments
                    for name in named_args.keys() {
                        if !param_names.contains(name) {
                            return Err(RuntimeError::undefined_variable(name.clone(), span));
                        }
                    }

                    // Build constructor arguments
                    let mut ctor_args = Vec::new();
                    let mut used_params = std::collections::HashSet::new();

                    // Positional arguments first
                    for (i, arg_val) in positional_args.iter().enumerate() {
                        if i < param_names.len() {
                            ctor_args.push(arg_val.clone());
                            used_params.insert(param_names[i].clone());
                        } else {
                            return Err(RuntimeError::wrong_arity(
                                full_arity,
                                positional_args.len() + named_args.len(),
                                span,
                            ));
                        }
                    }

                    // Named arguments and defaults
                    for (i, param_name) in param_names.iter().enumerate() {
                        if used_params.contains(param_name) {
                            continue;
                        }
                        if let Some(named_val) = named_args.get(param_name) {
                            ctor_args.push(named_val.clone());
                        } else if let Some(default_expr) = ctor.param_default_value(i) {
                            let default_value = self.evaluate(default_expr)?;
                            ctor_args.push(default_value);
                        } else {
                            return Err(RuntimeError::wrong_arity(
                                required_arity,
                                ctor_args.len(),
                                span,
                            ));
                        }
                    }

                    // Create constructor environment
                    let ctor_env = Environment::with_enclosing(ctor.closure.clone());
                    let mut ctor_env = ctor_env;
                    ctor_env.define("this".to_string(), Value::Instance(instance.clone()));

                    for (param, value) in ctor.params.iter().zip(ctor_args.iter()) {
                        ctor_env.define(param.name.clone(), value.clone());
                    }

                    // Execute constructor body
                    let _ = self.execute_block(&ctor.body, ctor_env);
                }

                Ok(Value::Instance(instance))
            }

            Value::Instance(inst) => {
                // Call method on instance with named arguments
                if let Some(method) = inst.borrow().get_method("call") {
                    match method {
                        Value::Function(func) => {
                            let required_arity = func.arity();
                            let full_arity = func.full_arity();

                            // Get parameter names
                            let param_names: Vec<String> =
                                func.params.iter().map(|p| p.name.clone()).collect();

                            // Check for unknown named arguments
                            for name in named_args.keys() {
                                if !param_names.contains(name) {
                                    return Err(RuntimeError::undefined_variable(
                                        name.clone(),
                                        span,
                                    ));
                                }
                            }

                            // Build method arguments
                            let mut method_args = vec![Value::Instance(inst.clone())];
                            let mut used_params = std::collections::HashSet::new();

                            for (i, arg_val) in positional_args.iter().enumerate() {
                                if i + 1 < param_names.len() {
                                    method_args.push(arg_val.clone());
                                    used_params.insert(param_names[i + 1].clone());
                                } else {
                                    return Err(RuntimeError::wrong_arity(
                                        full_arity,
                                        positional_args.len() + named_args.len() + 1,
                                        span,
                                    ));
                                }
                            }

                            for (i, param_name) in param_names.iter().enumerate() {
                                if i == 0 {
                                    // Skip 'this' parameter - already added
                                    continue;
                                }
                                if used_params.contains(param_name) {
                                    continue;
                                }
                                if let Some(named_val) = named_args.get(param_name) {
                                    method_args.push(named_val.clone());
                                } else if let Some(default_expr) = func.param_default_value(i) {
                                    let default_value = self.evaluate(default_expr)?;
                                    method_args.push(default_value);
                                } else {
                                    return Err(RuntimeError::wrong_arity(
                                        required_arity,
                                        method_args.len() - 1,
                                        span,
                                    ));
                                }
                            }

                            self.call_function(&func, method_args)
                        }
                        _ => Err(RuntimeError::type_error(
                            "callable object method is not a function",
                            span,
                        )),
                    }
                } else {
                    Err(RuntimeError::type_error("instance is not callable", span))
                }
            }

            Value::Super(superclass) => {
                // super(args) - call the superclass constructor
                // Get 'this' instance
                let this_val =
                    self.environment.borrow().get("this").ok_or_else(|| {
                        RuntimeError::type_error("'super' outside of class", span)
                    })?;

                let instance = match this_val {
                    Value::Instance(inst) => inst,
                    _ => return Err(RuntimeError::type_error("'super' outside of class", span)),
                };

                // Find and call the superclass constructor
                if let Some(ref ctor) = superclass.constructor {
                    let required_arity = ctor.arity();
                    let full_arity = ctor.full_arity();

                    let param_names: Vec<String> =
                        ctor.params.iter().map(|p| p.name.clone()).collect();

                    // Check for unknown named arguments
                    for name in named_args.keys() {
                        if !param_names.contains(name) {
                            return Err(RuntimeError::undefined_variable(name.clone(), span));
                        }
                    }

                    // Build constructor arguments
                    let mut ctor_args = Vec::new();
                    let mut used_params = std::collections::HashSet::new();

                    // Positional arguments first
                    for (i, arg_val) in positional_args.iter().enumerate() {
                        if i < param_names.len() {
                            ctor_args.push(arg_val.clone());
                            used_params.insert(param_names[i].clone());
                        } else {
                            return Err(RuntimeError::wrong_arity(
                                full_arity,
                                positional_args.len() + named_args.len(),
                                span,
                            ));
                        }
                    }

                    // Named arguments and defaults
                    for (i, param_name) in param_names.iter().enumerate() {
                        if used_params.contains(param_name) {
                            continue;
                        }
                        if let Some(named_val) = named_args.get(param_name) {
                            ctor_args.push(named_val.clone());
                        } else if let Some(default_expr) = ctor.param_default_value(i) {
                            let default_value = self.evaluate(default_expr)?;
                            ctor_args.push(default_value);
                        } else {
                            return Err(RuntimeError::wrong_arity(
                                required_arity,
                                ctor_args.len(),
                                span,
                            ));
                        }
                    }

                    // Execute constructor body with 'this' bound to the current instance
                    let ctor_env = Environment::with_enclosing(ctor.closure.clone());
                    let mut ctor_env = ctor_env;
                    ctor_env.define("this".to_string(), Value::Instance(instance.clone()));

                    for (param, value) in ctor.params.iter().zip(ctor_args) {
                        ctor_env.define(param.name.clone(), value);
                    }

                    let _ = self.execute_block(&ctor.body, ctor_env);
                }

                Ok(Value::Null)
            }

            _ => Err(RuntimeError::type_error(
                format!("{} is not callable", callee.type_name()),
                span,
            )),
        }
    }

    pub(crate) fn call_value(
        &mut self,
        callee: Value,
        mut arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match callee {
            Value::Function(func) => {
                let required_arity = func.arity();
                let full_arity = func.full_arity();

                // Check if we have enough arguments
                if arguments.len() < required_arity {
                    return Err(RuntimeError::wrong_arity(
                        required_arity,
                        arguments.len(),
                        span,
                    ));
                }

                // Check if we have too many arguments
                if arguments.len() > full_arity {
                    return Err(RuntimeError::wrong_arity(full_arity, arguments.len(), span));
                }

                // Pad arguments with default values if needed
                while arguments.len() < full_arity {
                    if let Some(default_expr) = func.param_default_value(arguments.len()) {
                        let default_value = self.evaluate(default_expr)?;
                        arguments.push(default_value);
                    } else {
                        // This shouldn't happen if arity checks are correct
                        return Err(RuntimeError::wrong_arity(full_arity, arguments.len(), span));
                    }
                }

                self.call_function(&func, arguments)
            }

            Value::NativeFunction(native) => {
                if let Some(arity) = native.arity {
                    if arguments.len() != arity {
                        return Err(RuntimeError::wrong_arity(arity, arguments.len(), span));
                    }
                }
                let result = (native.func)(arguments)
                    .map_err(|msg| RuntimeError::General { message: msg, span })?;

                // Check if this is the http_server_listen marker
                if let Some(port) = is_server_listen_marker(&result) {
                    // Only start server in main thread, not in worker threads
                    let thread_name = std::thread::current().name().map(|s| s.to_string());
                    let is_main_thread = thread_name
                        .as_ref()
                        .is_some_and(|n| n == "main" || n.starts_with("tokio-runtime"));
                    if is_main_thread {
                        // Run the HTTP server (this blocks until server stops)
                        return self.run_http_server(port);
                    }
                }

                Ok(result)
            }

            Value::Class(class) => {
                // Class as callable creates instance
                let instance = Rc::new(RefCell::new(Instance::new(class.clone())));

                if let Some(ref ctor) = class.constructor {
                    let required_arity = ctor.arity();
                    let full_arity = ctor.full_arity();

                    // Check if we have enough arguments
                    if arguments.len() < required_arity {
                        return Err(RuntimeError::wrong_arity(
                            required_arity,
                            arguments.len(),
                            span,
                        ));
                    }

                    // Check if we have too many arguments
                    if arguments.len() > full_arity {
                        return Err(RuntimeError::wrong_arity(full_arity, arguments.len(), span));
                    }

                    let mut final_args = arguments.clone();
                    // Pad arguments with default values if needed
                    while final_args.len() < full_arity {
                        if let Some(default_expr) = ctor.param_default_value(final_args.len()) {
                            let default_value = self.evaluate(default_expr)?;
                            final_args.push(default_value);
                        } else {
                            return Err(RuntimeError::wrong_arity(
                                full_arity,
                                final_args.len(),
                                span,
                            ));
                        }
                    }

                    let ctor_env = Environment::with_enclosing(ctor.closure.clone());
                    let mut ctor_env = ctor_env;
                    ctor_env.define("this".to_string(), Value::Instance(instance.clone()));

                    for (param, value) in ctor.params.iter().zip(final_args) {
                        ctor_env.define(param.name.clone(), value);
                    }

                    let _ = self.execute_block(&ctor.body, ctor_env);
                }

                Ok(Value::Instance(instance))
            }

            Value::Method(method) => {
                // Handle array/hash/QueryBuilder/String methods
                match *method.receiver {
                    Value::Array(ref arr) => {
                        let items = arr.borrow().clone();
                        self.call_array_method(&items, &method.method_name, arguments, span)
                    }
                    Value::Hash(ref hash) => {
                        let entries: Vec<(HashKey, Value)> = hash
                            .borrow()
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        self.call_hash_method(&entries, &method.method_name, arguments, span)
                    }
                    Value::QueryBuilder(qb) => {
                        self.call_query_builder_method(qb, &method.method_name, arguments, span)
                    }
                    Value::String(s) => {
                        self.call_string_method(&s, &method.method_name, arguments, span)
                    }
                    _ => Err(RuntimeError::type_error(
                        format!("{} does not support methods", method.receiver.type_name()),
                        span,
                    )),
                }
            }

            Value::Super(superclass) => {
                // super(args) - call the superclass constructor
                // Get 'this' instance
                let this_val =
                    self.environment.borrow().get("this").ok_or_else(|| {
                        RuntimeError::type_error("'super' outside of class", span)
                    })?;

                let instance = match this_val {
                    Value::Instance(inst) => inst,
                    _ => return Err(RuntimeError::type_error("'super' outside of class", span)),
                };

                // Find and call the superclass constructor
                if let Some(ref ctor) = superclass.constructor {
                    let required_arity = ctor.arity();
                    let full_arity = ctor.full_arity();

                    if arguments.len() < required_arity {
                        return Err(RuntimeError::wrong_arity(
                            required_arity,
                            arguments.len(),
                            span,
                        ));
                    }

                    if arguments.len() > full_arity {
                        return Err(RuntimeError::wrong_arity(full_arity, arguments.len(), span));
                    }

                    let mut final_args = arguments;
                    while final_args.len() < full_arity {
                        if let Some(default_expr) = ctor.param_default_value(final_args.len()) {
                            let default_value = self.evaluate(default_expr)?;
                            final_args.push(default_value);
                        } else {
                            return Err(RuntimeError::wrong_arity(
                                full_arity,
                                final_args.len(),
                                span,
                            ));
                        }
                    }

                    // Execute constructor body with 'this' bound to the current instance
                    let ctor_env = Environment::with_enclosing(ctor.closure.clone());
                    let mut ctor_env = ctor_env;
                    ctor_env.define("this".to_string(), Value::Instance(instance.clone()));

                    for (param, value) in ctor.params.iter().zip(final_args) {
                        ctor_env.define(param.name.clone(), value);
                    }

                    let _ = self.execute_block(&ctor.body, ctor_env);
                }

                Ok(Value::Null)
            }

            _ => Err(RuntimeError::not_callable(span)),
        }
    }

    /// Handle array methods: map, filter, each
    fn call_array_method(
        &mut self,
        items: &[Value],
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "map" => {
                // map expects a function that transforms each element
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let func = match &arguments[0] {
                    Value::Function(f) => f.clone(),
                    Value::NativeFunction(_) => {
                        return Err(RuntimeError::type_error(
                            "map expects a user-defined function",
                            span,
                        ))
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "map expects a function argument",
                            span,
                        ))
                    }
                };

                // Pre-compute param name once outside the loop
                let param_name = func
                    .params
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "it".to_string());

                let mut result = Vec::new();
                for item in items {
                    // Create new environment with the closure and define param in one step
                    let mut call_env = Environment::with_enclosing(func.closure.clone());
                    call_env.define(param_name.clone(), item.clone());

                    // Call the function
                    match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => result.push(v),
                        ControlFlow::Normal => result.push(Value::Null),
                        ControlFlow::Throw(_) => {
                            // Propagate the exception
                            return Err(RuntimeError::new("Exception in array method", span));
                        }
                    }
                }

                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "filter" => {
                // filter expects a function that returns a boolean
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let func = match &arguments[0] {
                    Value::Function(f) => f.clone(),
                    Value::NativeFunction(_) => {
                        return Err(RuntimeError::type_error(
                            "filter expects a user-defined function",
                            span,
                        ))
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "filter expects a function argument",
                            span,
                        ))
                    }
                };

                // Pre-compute param name once outside the loop
                let param_name = func
                    .params
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "it".to_string());

                let mut result = Vec::new();
                for item in items {
                    // Create new environment with the closure and define param in one step
                    let mut call_env = Environment::with_enclosing(func.closure.clone());
                    call_env.define(param_name.clone(), item.clone());

                    // Call the function and check if result is truthy
                    let result_value = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in array filter", span));
                        }
                    };

                    if result_value.is_truthy() {
                        result.push(item.clone());
                    }
                }

                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "each" => {
                // each expects a function and executes it for side effects
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let func = match &arguments[0] {
                    Value::Function(f) => f.clone(),
                    Value::NativeFunction(_) => {
                        return Err(RuntimeError::type_error(
                            "each expects a user-defined function",
                            span,
                        ))
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "each expects a function argument",
                            span,
                        ))
                    }
                };

                // Pre-compute param name once outside the loop
                let param_name = func
                    .params
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "it".to_string());

                for item in items {
                    // Create new environment with the closure and define param in one step
                    let mut call_env = Environment::with_enclosing(func.closure.clone());
                    call_env.define(param_name.clone(), item.clone());

                    // Call the function (discard return value)
                    match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(_) | ControlFlow::Normal => {}
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in array each", span));
                        }
                    }
                }

                // Return the original array for chaining
                Ok(Value::Array(Rc::new(RefCell::new(items.to_vec()))))
            }
            "reduce" => {
                // reduce expects a function and optional initial value
                if arguments.is_empty() || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let func = match &arguments[0] {
                    Value::Function(f) => f.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "reduce expects a function argument",
                            span,
                        ))
                    }
                };

                let mut acc = if arguments.len() == 2 {
                    arguments[1].clone()
                } else if !items.is_empty() {
                    items[0].clone()
                } else {
                    return Err(RuntimeError::type_error(
                        "reduce on empty array requires initial value",
                        span,
                    ));
                };

                let start_idx = if arguments.len() == 2 { 0 } else { 1 };

                for item in items.iter().skip(start_idx) {
                    // Create new environment with the closure and define params in one step
                    let mut call_env = Environment::with_enclosing(func.closure.clone());

                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), acc.clone());
                        call_env.define(func.params[1].name.clone(), item.clone());
                    } else if func.params.len() == 1 {
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![acc.clone(), item.clone()])));
                        call_env.define(func.params[0].name.clone(), pair);
                    }

                    acc = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in array reduce", span));
                        }
                    };
                }

                Ok(acc)
            }
            "find" => {
                // find expects a function that returns boolean
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let func = match &arguments[0] {
                    Value::Function(f) => f.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "find expects a function argument",
                            span,
                        ))
                    }
                };

                for item in items {
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    let param_name = func
                        .params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "it".to_string());
                    call_env.define(param_name, item.clone());

                    let result_value = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in array find", span));
                        }
                    };

                    if result_value.is_truthy() {
                        return Ok(item.clone());
                    }
                }

                Ok(Value::Null)
            }
            "any?" => {
                // any? expects a function that returns boolean
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

                for item in items {
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    let param_name = func
                        .params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "it".to_string());
                    call_env.define(param_name, item.clone());

                    let result_value = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in array any?", span));
                        }
                    };

                    if result_value.is_truthy() {
                        return Ok(Value::Bool(true));
                    }
                }

                Ok(Value::Bool(false))
            }
            "all?" => {
                // all? expects a function that returns boolean
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

                for item in items {
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    let param_name = func
                        .params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "it".to_string());
                    call_env.define(param_name, item.clone());

                    let result_value = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in array all?", span));
                        }
                    };

                    if !result_value.is_truthy() {
                        return Ok(Value::Bool(false));
                    }
                }

                Ok(Value::Bool(true))
            }
            "sort" => {
                // sort with optional comparator function
                let mut result = items.to_vec();
                if arguments.len() > 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }

                if let Some(func_val) = arguments.first() {
                    // Custom comparator
                    let func = match func_val {
                        Value::Function(f) => f.clone(),
                        _ => {
                            return Err(RuntimeError::type_error(
                                "sort expects a function argument",
                                span,
                            ))
                        }
                    };

                    result.sort_by(|a, b| {
                        let call_env = Environment::with_enclosing(func.closure.clone());
                        let mut call_env = call_env;

                        if func.params.len() >= 2 {
                            call_env.define(func.params[0].name.clone(), a.clone());
                            call_env.define(func.params[1].name.clone(), b.clone());
                        }

                        match self.execute_block(&func.body, call_env) {
                            Ok(ControlFlow::Return(Value::Int(n))) => n.cmp(&0),
                            Ok(ControlFlow::Return(Value::Float(n))) => {
                                if n < 0.0 {
                                    std::cmp::Ordering::Less
                                } else if n > 0.0 {
                                    std::cmp::Ordering::Greater
                                } else {
                                    std::cmp::Ordering::Equal
                                }
                            }
                            _ => std::cmp::Ordering::Equal,
                        }
                    });
                } else {
                    // Default sort
                    result.sort_by(|a, b| match (a, b) {
                        (Value::Int(a), Value::Int(b)) => a.cmp(b),
                        (Value::Float(a), Value::Float(b)) => {
                            a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                        }
                        (Value::String(a), Value::String(b)) => a.cmp(b),
                        (Value::Int(a), Value::Float(b)) => (*a as f64)
                            .partial_cmp(b)
                            .unwrap_or(std::cmp::Ordering::Equal),
                        (Value::Float(a), Value::Int(b)) => a
                            .partial_cmp(&(*b as f64))
                            .unwrap_or(std::cmp::Ordering::Equal),
                        _ => std::cmp::Ordering::Equal,
                    });
                }

                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "reverse" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let mut result = items.to_vec();
                result.reverse();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "uniq" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let mut result = Vec::new();
                for item in items {
                    if !result.contains(item) {
                        result.push(item.clone());
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "compact" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let result: Vec<Value> = items
                    .iter()
                    .filter(|v| !matches!(v, Value::Null))
                    .cloned()
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "flatten" => {
                let depth = match arguments.len() {
                    0 => None, // Flatten all levels
                    1 => match &arguments[0] {
                        Value::Int(n) if *n >= 0 => Some(*n as usize),
                        _ => {
                            return Err(RuntimeError::type_error(
                                "flatten expects a non-negative integer",
                                span,
                            ))
                        }
                    },
                    _ => return Err(RuntimeError::wrong_arity(1, arguments.len(), span)),
                };

                fn flatten_recursive(
                    arr: &[Value],
                    current_depth: usize,
                    max_depth: Option<usize>,
                ) -> Vec<Value> {
                    if let Some(max) = max_depth {
                        if current_depth >= max {
                            return arr.to_vec();
                        }
                    }

                    let mut result = Vec::new();
                    for item in arr {
                        if let Value::Array(inner) = item {
                            result.extend(flatten_recursive(
                                &inner.borrow(),
                                current_depth + 1,
                                max_depth,
                            ));
                        } else {
                            result.push(item.clone());
                        }
                    }
                    result
                }

                let result = flatten_recursive(items, 0, depth);
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "first" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(items.first().cloned().unwrap_or(Value::Null))
            }
            "last" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(items.last().cloned().unwrap_or(Value::Null))
            }
            "empty?" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Bool(items.is_empty()))
            }
            "include?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                Ok(Value::Bool(items.contains(&arguments[0])))
            }
            "sample" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                if items.is_empty() {
                    return Ok(Value::Null);
                }
                use rand::seq::SliceRandom;
                use rand::thread_rng;
                let mut rng = thread_rng();
                Ok(items.choose(&mut rng).cloned().unwrap_or(Value::Null))
            }
            "shuffle" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                use rand::seq::SliceRandom;
                use rand::thread_rng;
                let mut result = items.to_vec();
                let mut rng = thread_rng();
                result.shuffle(&mut rng);
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "take" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let n = match &arguments[0] {
                    Value::Int(n) if *n >= 0 => *n as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "take expects a non-negative integer",
                            span,
                        ))
                    }
                };
                let result: Vec<Value> = items.iter().take(n).cloned().collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "drop" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let n = match &arguments[0] {
                    Value::Int(n) if *n >= 0 => *n as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "drop expects a non-negative integer",
                            span,
                        ))
                    }
                };
                let result: Vec<Value> = items.iter().skip(n).cloned().collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "zip" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let other = match &arguments[0] {
                    Value::Array(arr) => arr.borrow().clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "zip expects an array argument",
                            span,
                        ))
                    }
                };

                let result: Vec<Value> = items
                    .iter()
                    .zip(other.iter())
                    .map(|(a, b)| Value::Array(Rc::new(RefCell::new(vec![a.clone(), b.clone()]))))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            "sum" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let mut total = 0.0;
                for item in items {
                    match item {
                        Value::Int(n) => total += *n as f64,
                        Value::Float(n) => total += *n,
                        _ => {
                            return Err(RuntimeError::type_error("sum expects numeric array", span))
                        }
                    }
                }
                Ok(Value::Float(total))
            }
            "min" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                if items.is_empty() {
                    return Ok(Value::Null);
                }
                let mut min = &items[0];
                for item in items.iter().skip(1) {
                    match (min, item) {
                        (Value::Int(a), Value::Int(b)) if b < a => min = item,
                        (Value::Float(a), Value::Float(b)) if b < a => min = item,
                        (Value::String(a), Value::String(b)) if b < a => min = item,
                        (Value::Int(a), Value::Float(b)) if *b < *a as f64 => min = item,
                        (Value::Float(a), Value::Int(b)) if (*b as f64) < *a => min = item,
                        _ => {}
                    }
                }
                Ok(min.clone())
            }
            "max" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                if items.is_empty() {
                    return Ok(Value::Null);
                }
                let mut max = &items[0];
                for item in items.iter().skip(1) {
                    match (max, item) {
                        (Value::Int(a), Value::Int(b)) if b > a => max = item,
                        (Value::Float(a), Value::Float(b)) if b > a => max = item,
                        (Value::String(a), Value::String(b)) if b > a => max = item,
                        (Value::Int(a), Value::Float(b)) if *b > *a as f64 => max = item,
                        (Value::Float(a), Value::Int(b)) if (*b as f64) > *a => max = item,
                        _ => {}
                    }
                }
                Ok(max.clone())
            }
            "push" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                // push returns a new array with the element added (functional style)
                let mut new_arr = items.to_vec();
                new_arr.push(arguments[0].clone());
                Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
            }
            "pop" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                // pop returns a new array with the last element removed (functional style)
                let mut new_arr = items.to_vec();
                new_arr.pop();
                Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
            }
            "clear" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                // clear returns an empty array (functional style)
                Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
            }
            "get" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let idx = match &arguments[0] {
                    Value::Int(n) => *n,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "get expects an integer index",
                            span,
                        ))
                    }
                };
                let idx_usize = if idx < 0 {
                    (items.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                items
                    .get(idx_usize)
                    .cloned()
                    .ok_or_else(|| RuntimeError::IndexOutOfBounds {
                        index: idx,
                        length: items.len(),
                        span,
                    })
            }
            "length" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Int(items.len() as i64))
            }
            "to_string" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
                Ok(Value::String(format!("[{}]", parts.join(", "))))
            }
            "join" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let delim = match &arguments[0] {
                    Value::String(d) => d.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "join expects a string delimiter",
                            span,
                        ))
                    }
                };
                let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
                Ok(Value::String(parts.join(&delim)))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Array".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    /// Handle hash methods: map, filter, each
    fn call_hash_method(
        &mut self,
        entries: &[(HashKey, Value)],
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "map" => {
                // map on hash expects a function that takes (key, value) and returns [key, new_value]
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let func = match &arguments[0] {
                    Value::Function(f) => f.clone(),
                    Value::NativeFunction(_) => {
                        return Err(RuntimeError::type_error(
                            "map expects a user-defined function",
                            span,
                        ))
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "map expects a function argument",
                            span,
                        ))
                    }
                };

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for (key, value) in entries {
                    // Create new environment with the closure
                    let mut call_env = Environment::with_enclosing(func.closure.clone());

                    // Define parameters: key and value
                    let key_value = key.to_value();
                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), key_value.clone());
                        call_env.define(func.params[1].name.clone(), value.clone());
                    } else if func.params.len() == 1 {
                        // If only one param, pass [key, value] array
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                        call_env.define(func.params[0].name.clone(), pair);
                    }

                    // Call the function and expect [key, value] pair back
                    match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => {
                            // Expect an array of [key, value]
                            if let Value::Array(arr) = v {
                                let arr = arr.borrow();
                                if arr.len() == 2 {
                                    let new_key = arr[0].clone();
                                    let new_val = arr[1].clone();
                                    let hash_key = new_key.to_hash_key().ok_or_else(|| {
                                        RuntimeError::type_error("hash key must be hashable", span)
                                    })?;
                                    result.insert(hash_key, new_val);
                                }
                            }
                        }
                        ControlFlow::Normal => {}
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in hash map", span));
                        }
                    }
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "filter" => {
                // filter on hash expects a function that takes (key, value) and returns boolean
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let func = match &arguments[0] {
                    Value::Function(f) => f.clone(),
                    Value::NativeFunction(_) => {
                        return Err(RuntimeError::type_error(
                            "filter expects a user-defined function",
                            span,
                        ))
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "filter expects a function argument",
                            span,
                        ))
                    }
                };

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for (key, value) in entries {
                    // Create new environment with the closure
                    let mut call_env = Environment::with_enclosing(func.closure.clone());

                    // Define parameters: key and value
                    let key_value = key.to_value();
                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), key_value.clone());
                        call_env.define(func.params[1].name.clone(), value.clone());
                    } else if func.params.len() == 1 {
                        // If only one param, pass [key, value] array
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                        call_env.define(func.params[0].name.clone(), pair);
                    }

                    // Call the function and check if result is truthy
                    let result_value = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in hash filter", span));
                        }
                    };

                    if result_value.is_truthy() {
                        result.insert(key.clone(), value.clone());
                    }
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "each" => {
                // each on hash expects a function and executes it for side effects
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let func = match &arguments[0] {
                    Value::Function(f) => f.clone(),
                    Value::NativeFunction(_) => {
                        return Err(RuntimeError::type_error(
                            "each expects a user-defined function",
                            span,
                        ))
                    }
                    _ => {
                        return Err(RuntimeError::type_error(
                            "each expects a function argument",
                            span,
                        ))
                    }
                };

                for (key, value) in entries {
                    // Create new environment with the closure
                    let mut call_env = Environment::with_enclosing(func.closure.clone());

                    // Define parameters: key and value
                    let key_value = key.to_value();
                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), key_value.clone());
                        call_env.define(func.params[1].name.clone(), value.clone());
                    } else if func.params.len() == 1 {
                        // If only one param, pass [key, value] array
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                        call_env.define(func.params[0].name.clone(), pair);
                    }

                    // Call the function (discard return value)
                    match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(_) | ControlFlow::Normal => {}
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in hash each", span));
                        }
                    }
                }

                // Return the original hash for chaining
                let result: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "get" => {
                // get(key) or get(key, default) - O(1) lookup
                if arguments.is_empty() || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let key = &arguments[0];
                let hash_key = key.to_hash_key().ok_or_else(|| {
                    RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key.type_name()),
                        span,
                    )
                })?;
                let default = arguments.get(1).cloned().unwrap_or(Value::Null);

                // O(1) lookup using IndexMap
                let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                Ok(entries_map.get(&hash_key).cloned().unwrap_or(default))
            }
            "fetch" => {
                // fetch(key) or fetch(key, default) - raises error if key not found and no default
                if arguments.is_empty() || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let key = &arguments[0];
                let hash_key = key.to_hash_key().ok_or_else(|| {
                    RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key.type_name()),
                        span,
                    )
                })?;

                // O(1) lookup using IndexMap
                let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                if let Some(v) = entries_map.get(&hash_key) {
                    Ok(v.clone())
                } else if let Some(default) = arguments.get(1) {
                    Ok(default.clone())
                } else {
                    Err(RuntimeError::type_error(
                        format!("key not found: {:?}", key),
                        span,
                    ))
                }
            }
            "invert" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for (k, v) in entries {
                    let new_key = v.to_hash_key().ok_or_else(|| {
                        RuntimeError::type_error(
                            format!("{} cannot be used as a hash key", v.type_name()),
                            span,
                        )
                    })?;
                    result.insert(new_key, k.to_value());
                }
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "transform_values" => {
                // transform_values(fn(value)) - transform all values
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

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for (key, value) in entries {
                    let mut call_env = Environment::with_enclosing(func.closure.clone());

                    let param_name = func
                        .params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "it".to_string());
                    call_env.define(param_name, value.clone());

                    let new_value = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new(
                                "Exception in hash transform_values",
                                span,
                            ));
                        }
                    };
                    result.insert(key.clone(), new_value);
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "transform_keys" => {
                // transform_keys(fn(key)) - transform all keys
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

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for (key, value) in entries {
                    let mut call_env = Environment::with_enclosing(func.closure.clone());

                    let param_name = func
                        .params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "it".to_string());
                    call_env.define(param_name, key.to_value());

                    let new_key = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new(
                                "Exception in hash transform_keys",
                                span,
                            ));
                        }
                    };

                    let new_hash_key = new_key.to_hash_key().ok_or_else(|| {
                        RuntimeError::type_error("transformed key must be hashable", span)
                    })?;
                    result.insert(new_hash_key, value.clone());
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "select" => {
                // select(fn(key, value)) - keep entries where function returns true
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

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for (key, value) in entries {
                    let mut call_env = Environment::with_enclosing(func.closure.clone());

                    let key_value = key.to_value();
                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), key_value.clone());
                        call_env.define(func.params[1].name.clone(), value.clone());
                    } else if func.params.len() == 1 {
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                        call_env.define(func.params[0].name.clone(), pair);
                    }

                    let result_value = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in hash select", span));
                        }
                    };

                    if result_value.is_truthy() {
                        result.insert(key.clone(), value.clone());
                    }
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "reject" => {
                // reject(fn(key, value)) - remove entries where function returns true
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

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for (key, value) in entries {
                    let mut call_env = Environment::with_enclosing(func.closure.clone());

                    let key_value = key.to_value();
                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), key_value.clone());
                        call_env.define(func.params[1].name.clone(), value.clone());
                    } else if func.params.len() == 1 {
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![key_value, value.clone()])));
                        call_env.define(func.params[0].name.clone(), pair);
                    }

                    let result_value = match self.execute_block(&func.body, call_env)? {
                        ControlFlow::Return(v) => v,
                        ControlFlow::Normal => Value::Null,
                        ControlFlow::Throw(_) => {
                            return Err(RuntimeError::new("Exception in hash reject", span));
                        }
                    };

                    if !result_value.is_truthy() {
                        result.insert(key.clone(), value.clone());
                    }
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "slice" => {
                // slice([key1, key2, ...]) - get subset with specified keys - O(n) with O(1) lookups
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let keys_arr = match &arguments[0] {
                    Value::Array(arr) => arr.borrow().clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "slice expects an array of keys",
                            span,
                        ))
                    }
                };

                // Build an IndexMap from entries for O(1) lookups
                let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for key in keys_arr {
                    let hash_key = key.to_hash_key().ok_or_else(|| {
                        RuntimeError::type_error(
                            format!("{} cannot be used as a hash key", key.type_name()),
                            span,
                        )
                    })?;
                    if let Some(v) = entries_map.get(&hash_key) {
                        result.insert(hash_key, v.clone());
                    }
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "except" => {
                // except([key1, key2, ...]) - get hash without specified keys - O(n) with O(1) lookups
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let keys_arr = match &arguments[0] {
                    Value::Array(arr) => arr.borrow().clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "except expects an array of keys",
                            span,
                        ))
                    }
                };

                // Build set of keys to exclude for O(1) lookups
                let exclude_keys: std::collections::HashSet<HashKey> =
                    keys_arr.iter().filter_map(|k| k.to_hash_key()).collect();

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                for (k, v) in entries {
                    if !exclude_keys.contains(k) {
                        result.insert(k.clone(), v.clone());
                    }
                }

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "compact" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let result: IndexMap<HashKey, Value> = entries
                    .iter()
                    .filter(|(_, v)| !matches!(v, Value::Null))
                    .cloned()
                    .collect();
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "dig" => {
                // dig(key, key2, ...) - navigate nested hashes with O(1) lookups
                if arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }

                let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                let mut current = Value::Hash(Rc::new(RefCell::new(entries_map)));
                for key in arguments {
                    match current {
                        Value::Hash(ref hash) => {
                            let hash_key = match key.to_hash_key() {
                                Some(k) => k,
                                None => return Ok(Value::Null),
                            };
                            let hash_ref = hash.borrow();
                            match hash_ref.get(&hash_key).cloned() {
                                Some(v) => {
                                    drop(hash_ref);
                                    current = v;
                                }
                                None => return Ok(Value::Null),
                            }
                        }
                        _ => return Ok(Value::Null),
                    }
                }
                Ok(current)
            }
            "length" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Int(entries.len() as i64))
            }
            "to_string" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let parts: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{} => {}", k.to_value(), v))
                    .collect();
                Ok(Value::String(format!("{{{}}}", parts.join(", "))))
            }
            "keys" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let keys: Vec<Value> = entries.iter().map(|(k, _)| k.to_value()).collect();
                Ok(Value::Array(Rc::new(RefCell::new(keys))))
            }
            "values" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let values: Vec<Value> = entries.iter().map(|(_, v)| v.clone()).collect();
                Ok(Value::Array(Rc::new(RefCell::new(values))))
            }
            "has_key" => {
                // O(1) lookup
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let key = &arguments[0];
                let hash_key = match key.to_hash_key() {
                    Some(k) => k,
                    None => return Ok(Value::Bool(false)),
                };
                let entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                Ok(Value::Bool(entries_map.contains_key(&hash_key)))
            }
            "delete" => {
                // O(1) lookup and removal
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let key = &arguments[0];
                let hash_key = match key.to_hash_key() {
                    Some(k) => k,
                    None => return Ok(Value::Null),
                };
                let mut entries_map: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                Ok(entries_map.shift_remove(&hash_key).unwrap_or(Value::Null))
            }
            "merge" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let other = match &arguments[0] {
                    Value::Hash(h) => h.borrow().clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "merge expects a hash argument",
                            span,
                        ))
                    }
                };
                let mut result: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                for (k, v) in other {
                    result.insert(k, v);
                }
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "entries" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let pairs: Vec<Value> = entries
                    .iter()
                    .map(|(k, v)| {
                        Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                    })
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(pairs))))
            }
            "clear" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Hash(Rc::new(RefCell::new(IndexMap::new()))))
            }
            "set" => {
                // O(1) insert
                if arguments.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let key = &arguments[0];
                let value = arguments[1].clone();
                let hash_key = key.to_hash_key().ok_or_else(|| {
                    RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key.type_name()),
                        span,
                    )
                })?;
                let mut result: IndexMap<HashKey, Value> = entries.iter().cloned().collect();
                result.insert(hash_key, value);
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            }
            "empty?" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Bool(entries.is_empty()))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Hash".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    /// Handle QueryBuilder methods for chaining: where, order, limit, offset, all, first, count
    fn call_query_builder_method(
        &mut self,
        qb: Rc<RefCell<crate::interpreter::builtins::model::QueryBuilder>>,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "where" => {
                // .where(filter, bind_vars) - add another filter condition (ANDed with existing)
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
                            if let HashKey::String(key) = k {
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

                // Clone the QueryBuilder and add/merge the filter
                let mut new_qb = qb.borrow().clone();
                if let Some(existing_filter) = &new_qb.filter {
                    // AND the new filter with existing
                    new_qb.filter = Some(format!("({}) AND ({})", existing_filter, filter));
                } else {
                    new_qb.filter = Some(filter);
                }
                // Merge bind vars
                for (k, v) in bind_vars {
                    new_qb
                        .bind_vars
                        .insert(crate::interpreter::get_symbol(&k), v);
                }
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
            }
            "order" => {
                // .order(field, direction) - set ordering
                if arguments.len() < 1 || arguments.len() > 2 {
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
            "limit" => {
                // .limit(n) - set limit
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
            "offset" => {
                // .offset(n) - set offset
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
            "all" => {
                // .all() - execute query and return all results
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(execute_query_builder(&qb.borrow()))
            }
            "first" => {
                // .first() - execute query and return first result
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(execute_query_builder_first(&qb.borrow()))
            }
            "count" => {
                // .count() - execute count query
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(execute_query_builder_count(&qb.borrow()))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "QueryBuilder".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    /// Handle string methods
    fn call_string_method(
        &mut self,
        s: &str,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        use regex::Regex;

        match method_name {
            "starts_with?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let prefix = match &arguments[0] {
                    Value::String(p) => p,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "starts_with? expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(s.starts_with(prefix)))
            }
            "ends_with?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let suffix = match &arguments[0] {
                    Value::String(suf) => suf,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "ends_with? expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(s.ends_with(suffix)))
            }
            "chomp" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let result = s
                    .strip_suffix('\n')
                    .or_else(|| s.strip_suffix("\r\n"))
                    .or_else(|| s.strip_suffix('\r'))
                    .unwrap_or(s);
                Ok(Value::String(result.to_string()))
            }
            "lstrip" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::String(s.trim_start().to_string()))
            }
            "rstrip" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::String(s.trim_end().to_string()))
            }
            "squeeze" => {
                if arguments.len() > 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let chars_to_squeeze: Option<Vec<char>> = arguments.first().map(|v| match v {
                    Value::String(s) => s.chars().collect(),
                    _ => vec![],
                });

                let mut result = String::new();
                let mut last_char: Option<char> = None;

                for c in s.chars() {
                    let should_squeeze = chars_to_squeeze
                        .as_ref()
                        .map(|chars| chars.contains(&c))
                        .unwrap_or(true);

                    if should_squeeze {
                        if last_char != Some(c) {
                            result.push(c);
                        }
                    } else {
                        result.push(c);
                    }
                    last_char = Some(c);
                }
                Ok(Value::String(result))
            }
            "count" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let substr = match &arguments[0] {
                    Value::String(sub) => sub,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "count expects a string argument",
                            span,
                        ))
                    }
                };
                let count = s.matches(substr).count() as i64;
                Ok(Value::Int(count))
            }
            "gsub" => {
                if arguments.len() < 2 || arguments.len() > 3 {
                    return Err(RuntimeError::wrong_arity(3, arguments.len(), span));
                }
                let pattern = match &arguments[0] {
                    Value::String(p) => p,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "gsub expects a string pattern",
                            span,
                        ))
                    }
                };
                let replacement = match &arguments[1] {
                    Value::String(r) => r.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "gsub expects a string replacement",
                            span,
                        ))
                    }
                };

                let result = if arguments.len() == 3 {
                    // With limit
                    let limit = match &arguments[2] {
                        Value::Int(n) if *n >= 0 => *n as usize,
                        _ => {
                            return Err(RuntimeError::type_error(
                                "gsub limit must be a non-negative integer",
                                span,
                            ))
                        }
                    };
                    let re = Regex::new(pattern).map_err(|e| {
                        RuntimeError::type_error(format!("invalid regex: {}", e), span)
                    })?;
                    re.replacen(s, limit, &replacement).to_string()
                } else {
                    let re = Regex::new(pattern).map_err(|e| {
                        RuntimeError::type_error(format!("invalid regex: {}", e), span)
                    })?;
                    re.replace_all(s, &replacement).to_string()
                };
                Ok(Value::String(result))
            }
            "sub" => {
                if arguments.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let pattern = match &arguments[0] {
                    Value::String(p) => p,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "sub expects a string pattern",
                            span,
                        ))
                    }
                };
                let replacement = match &arguments[1] {
                    Value::String(r) => r.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "sub expects a string replacement",
                            span,
                        ))
                    }
                };

                let re = Regex::new(pattern)
                    .map_err(|e| RuntimeError::type_error(format!("invalid regex: {}", e), span))?;
                let result = re.replacen(s, 1, &replacement).to_string();
                Ok(Value::String(result))
            }
            "match" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let pattern = match &arguments[0] {
                    Value::String(p) => p,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "match expects a string pattern",
                            span,
                        ))
                    }
                };

                let re = Regex::new(pattern)
                    .map_err(|e| RuntimeError::type_error(format!("invalid regex: {}", e), span))?;
                if let Some(captures) = re.captures(s) {
                    let mut result = Vec::new();
                    for i in 0..captures.len() {
                        if let Some(m) = captures.get(i) {
                            result.push(Value::String(m.as_str().to_string()));
                        }
                    }
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                } else {
                    Ok(Value::Null)
                }
            }
            "scan" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let pattern = match &arguments[0] {
                    Value::String(p) => p,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "scan expects a string pattern",
                            span,
                        ))
                    }
                };

                let re = Regex::new(pattern)
                    .map_err(|e| RuntimeError::type_error(format!("invalid regex: {}", e), span))?;
                let matches: Vec<Value> = re
                    .find_iter(s)
                    .map(|m| Value::String(m.as_str().to_string()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(matches))))
            }
            "tr" => {
                if arguments.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let from_chars = match &arguments[0] {
                    Value::String(f) => f,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "tr expects a string from pattern",
                            span,
                        ))
                    }
                };
                let to_chars = match &arguments[1] {
                    Value::String(t) => t,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "tr expects a string to pattern",
                            span,
                        ))
                    }
                };

                let mut result = String::new();
                for c in s.chars() {
                    if let Some(pos) = from_chars.find(c) {
                        if let Some(replacement) = to_chars.chars().nth(pos) {
                            result.push(replacement);
                        }
                    } else {
                        result.push(c);
                    }
                }
                Ok(Value::String(result))
            }
            "center" => {
                if arguments.len() < 1 || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let width = match &arguments[0] {
                    Value::Int(w) if *w > 0 => *w as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "center expects a positive integer width",
                            span,
                        ))
                    }
                };
                let pad_char = arguments
                    .get(1)
                    .map(|v| match v {
                        Value::String(s) => s.chars().next().unwrap_or(' '),
                        _ => ' ',
                    })
                    .unwrap_or(' ');

                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let total_pad = width - s.len();
                    let left_pad = total_pad / 2;
                    let right_pad = total_pad - left_pad;
                    let result = pad_char.to_string().repeat(left_pad)
                        + s
                        + &pad_char.to_string().repeat(right_pad);
                    Ok(Value::String(result))
                }
            }
            "ljust" => {
                if arguments.len() < 1 || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let width = match &arguments[0] {
                    Value::Int(w) if *w > 0 => *w as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "ljust expects a positive integer width",
                            span,
                        ))
                    }
                };
                let pad_char = arguments
                    .get(1)
                    .map(|v| match v {
                        Value::String(s) => s.chars().next().unwrap_or(' '),
                        _ => ' ',
                    })
                    .unwrap_or(' ');

                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let result = s.to_string() + &pad_char.to_string().repeat(width - s.len());
                    Ok(Value::String(result))
                }
            }
            "rjust" => {
                if arguments.len() < 1 || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let width = match &arguments[0] {
                    Value::Int(w) if *w > 0 => *w as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "rjust expects a positive integer width",
                            span,
                        ))
                    }
                };
                let pad_char = arguments
                    .get(1)
                    .map(|v| match v {
                        Value::String(s) => s.chars().next().unwrap_or(' '),
                        _ => ' ',
                    })
                    .unwrap_or(' ');

                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let result = pad_char.to_string().repeat(width - s.len()) + s;
                    Ok(Value::String(result))
                }
            }
            "ord" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                if let Some(c) = s.chars().next() {
                    Ok(Value::Int(c as i64))
                } else {
                    Err(RuntimeError::type_error("ord on empty string", span))
                }
            }
            "chr" => {
                // This is a class method, not instance method - should not be called on a string instance
                Err(RuntimeError::type_error(
                    "chr is not a string instance method",
                    span,
                ))
            }
            "bytes" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let bytes: Vec<Value> = s.bytes().map(|b| Value::Int(b as i64)).collect();
                Ok(Value::Array(Rc::new(RefCell::new(bytes))))
            }
            "chars" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let chars: Vec<Value> = s.chars().map(|c| Value::String(c.to_string())).collect();
                Ok(Value::Array(Rc::new(RefCell::new(chars))))
            }
            "lines" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let lines: Vec<Value> = s.lines().map(|l| Value::String(l.to_string())).collect();
                Ok(Value::Array(Rc::new(RefCell::new(lines))))
            }
            "bytesize" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Int(s.len() as i64))
            }
            "capitalize" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let mut chars = s.chars();
                let result: String = match chars.next() {
                    None => String::new(),
                    Some(first) => {
                        first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                    }
                };
                Ok(Value::String(result))
            }
            "swapcase" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let result: String = s
                    .chars()
                    .map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().to_string()
                        } else {
                            c.to_uppercase().to_string()
                        }
                    })
                    .collect();
                Ok(Value::String(result))
            }
            "insert" => {
                if arguments.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let index = match &arguments[0] {
                    Value::Int(i) if *i >= 0 => *i as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "insert expects a non-negative integer index",
                            span,
                        ))
                    }
                };
                let insert_str = match &arguments[1] {
                    Value::String(str) => str,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "insert expects a string to insert",
                            span,
                        ))
                    }
                };

                let char_count = s.chars().count();
                if index > char_count {
                    return Err(RuntimeError::type_error("insert index out of bounds", span));
                }

                let mut result = String::new();
                for (i, c) in s.chars().enumerate() {
                    if i == index {
                        result.push_str(insert_str);
                    }
                    result.push(c);
                }
                if index == char_count {
                    result.push_str(insert_str);
                }
                Ok(Value::String(result))
            }
            "delete" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let to_delete = match &arguments[0] {
                    Value::String(d) => d,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "delete expects a string argument",
                            span,
                        ))
                    }
                };
                let result = s.replace(to_delete, "");
                Ok(Value::String(result))
            }
            "delete_prefix" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let prefix = match &arguments[0] {
                    Value::String(p) => p,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "delete_prefix expects a string argument",
                            span,
                        ))
                    }
                };
                let result = s.strip_prefix(prefix).unwrap_or(s);
                Ok(Value::String(result.to_string()))
            }
            "delete_suffix" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let suffix = match &arguments[0] {
                    Value::String(suf) => suf,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "delete_suffix expects a string argument",
                            span,
                        ))
                    }
                };
                let result = s.strip_suffix(suffix).unwrap_or(s);
                Ok(Value::String(result.to_string()))
            }
            "partition" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let sep = match &arguments[0] {
                    Value::String(s) => s,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "partition expects a string separator",
                            span,
                        ))
                    }
                };

                if let Some(pos) = s.find(sep) {
                    let before = &s[..pos];
                    let after = &s[pos + sep.len()..];
                    let result = vec![
                        Value::String(before.to_string()),
                        Value::String(sep.to_string()),
                        Value::String(after.to_string()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                } else {
                    let result = vec![
                        Value::String(s.to_string()),
                        Value::String("".to_string()),
                        Value::String("".to_string()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
            }
            "rpartition" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let sep = match &arguments[0] {
                    Value::String(s) => s,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "rpartition expects a string separator",
                            span,
                        ))
                    }
                };

                if let Some(pos) = s.rfind(sep) {
                    let before = &s[..pos];
                    let after = &s[pos + sep.len()..];
                    let result = vec![
                        Value::String(before.to_string()),
                        Value::String(sep.to_string()),
                        Value::String(after.to_string()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                } else {
                    let result = vec![
                        Value::String("".to_string()),
                        Value::String("".to_string()),
                        Value::String(s.to_string()),
                    ];
                    Ok(Value::Array(Rc::new(RefCell::new(result))))
                }
            }
            "reverse" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let result: String = s.chars().rev().collect();
                Ok(Value::String(result))
            }
            "hex" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let result = i64::from_str_radix(s, 16)
                    .map_err(|e| RuntimeError::type_error(format!("invalid hex: {}", e), span))?;
                Ok(Value::Int(result))
            }
            "oct" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                let result = i64::from_str_radix(s, 8)
                    .map_err(|e| RuntimeError::type_error(format!("invalid octal: {}", e), span))?;
                Ok(Value::Int(result))
            }
            "truncate" => {
                if arguments.len() < 1 || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let length = match &arguments[0] {
                    Value::Int(l) if *l > 0 => *l as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "truncate expects a positive integer length",
                            span,
                        ))
                    }
                };
                let suffix = arguments
                    .get(1)
                    .map(|v| match v {
                        Value::String(s) => s.as_str(),
                        _ => "...",
                    })
                    .unwrap_or("...");

                if s.len() <= length {
                    Ok(Value::String(s.to_string()))
                } else {
                    let result = &s[..length.saturating_sub(suffix.len())];
                    Ok(Value::String(result.to_string() + suffix))
                }
            }
            // Methods from collection_classes.rs String class
            "length" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Int(s.len() as i64))
            }
            "to_string" => Ok(Value::String(s.to_string())),
            "upcase" | "uppercase" => Ok(Value::String(s.to_uppercase())),
            "downcase" | "lowercase" => Ok(Value::String(s.to_lowercase())),
            "trim" => Ok(Value::String(s.trim().to_string())),
            "contains" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let substr = match &arguments[0] {
                    Value::String(sub) => sub,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "contains expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(s.contains(substr)))
            }
            "starts_with" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let prefix = match &arguments[0] {
                    Value::String(p) => p,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "starts_with expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(s.starts_with(prefix)))
            }
            "ends_with" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let suffix = match &arguments[0] {
                    Value::String(suf) => suf,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "ends_with expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(s.ends_with(suffix)))
            }
            "split" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let delim = match &arguments[0] {
                    Value::String(d) => d,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "split expects a string delimiter",
                            span,
                        ))
                    }
                };
                let parts: Vec<Value> = s
                    .split(delim)
                    .map(|p| Value::String(p.to_string()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(parts))))
            }
            "index_of" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let substr = match &arguments[0] {
                    Value::String(sub) => sub,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "index_of expects a string argument",
                            span,
                        ))
                    }
                };
                if let Some(idx) = s.find(substr) {
                    Ok(Value::Int(idx as i64))
                } else {
                    Ok(Value::Int(-1))
                }
            }
            "substring" => {
                if arguments.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let start = match &arguments[0] {
                    Value::Int(i) => *i,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "substring expects integer start",
                            span,
                        ))
                    }
                };
                let end = match &arguments[1] {
                    Value::Int(i) => *i,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "substring expects integer end",
                            span,
                        ))
                    }
                };
                let start_usize = if start < 0 { 0 } else { start as usize };
                let end_usize = if end > s.len() as i64 {
                    s.len()
                } else {
                    end as usize
                };
                if start_usize >= end_usize || start_usize >= s.len() {
                    Ok(Value::String(String::new()))
                } else {
                    Ok(Value::String(s[start_usize..end_usize].to_string()))
                }
            }
            "replace" => {
                if arguments.len() != 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let from = match &arguments[0] {
                    Value::String(f) => f,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "replace expects string from",
                            span,
                        ))
                    }
                };
                let to = match &arguments[1] {
                    Value::String(t) => t,
                    _ => return Err(RuntimeError::type_error("replace expects string to", span)),
                };
                Ok(Value::String(s.replace(from, to)))
            }
            "lpad" => {
                if arguments.is_empty() || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let width = match &arguments[0] {
                    Value::Int(w) if *w >= 0 => *w as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "lpad expects non-negative integer width",
                            span,
                        ))
                    }
                };
                let pad_char = arguments
                    .get(1)
                    .map(|v| match v {
                        Value::String(ps) => ps.chars().next().unwrap_or(' '),
                        _ => ' ',
                    })
                    .unwrap_or(' ');
                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let padding = width - s.len();
                    Ok(Value::String(pad_char.to_string().repeat(padding) + s))
                }
            }
            "rpad" => {
                if arguments.is_empty() || arguments.len() > 2 {
                    return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                }
                let width = match &arguments[0] {
                    Value::Int(w) if *w >= 0 => *w as usize,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "rpad expects non-negative integer width",
                            span,
                        ))
                    }
                };
                let pad_char = arguments
                    .get(1)
                    .map(|v| match v {
                        Value::String(ps) => ps.chars().next().unwrap_or(' '),
                        _ => ' ',
                    })
                    .unwrap_or(' ');
                if s.len() >= width {
                    Ok(Value::String(s.to_string()))
                } else {
                    let padding = width - s.len();
                    Ok(Value::String(
                        s.to_string() + &pad_char.to_string().repeat(padding),
                    ))
                }
            }
            "join" => {
                // For string, join doesn't make much sense, but we can return the string itself
                Ok(Value::String(s.to_string()))
            }
            "empty?" => {
                if !arguments.is_empty() {
                    return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                }
                Ok(Value::Bool(s.is_empty()))
            }
            "include?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let substr = match &arguments[0] {
                    Value::String(sub) => sub,
                    _ => {
                        return Err(RuntimeError::type_error(
                            "include? expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(s.contains(substr)))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "String".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn match_pattern(
        &mut self,
        value: &Value,
        pattern: &MatchPattern,
    ) -> RuntimeResult<Option<Vec<(String, Value)>>> {
        match pattern {
            MatchPattern::Wildcard => Ok(Some(Vec::new())),

            MatchPattern::Variable(name) => Ok(Some(vec![(name.clone(), value.clone())])),

            MatchPattern::Typed { name, type_name } => {
                let matches = match type_name.as_str() {
                    "Int" => matches!(value, Value::Int(_)),
                    "Float" => matches!(value, Value::Float(_)),
                    "Bool" => matches!(value, Value::Bool(_)),
                    "String" => matches!(value, Value::String(_)),
                    "Void" => matches!(value, Value::Null),
                    _ => {
                        if let Value::Instance(inst) = value {
                            inst.borrow().class.name == *type_name
                        } else {
                            false
                        }
                    }
                };

                if matches {
                    Ok(Some(vec![(name.clone(), value.clone())]))
                } else {
                    Ok(None)
                }
            }

            MatchPattern::Literal(literal) => {
                let literal_value = self.evaluate_literal(literal)?;
                if self.values_equal(&literal_value, value) {
                    Ok(Some(Vec::new()))
                } else {
                    Ok(None)
                }
            }

            MatchPattern::Array { elements, rest } => {
                let arr = match value {
                    Value::Array(arr) => arr.borrow().clone(),
                    _ => return Ok(None),
                };

                // Without rest: pattern must match array length exactly
                // With rest: pattern elements must be <= array length
                if rest.is_none() {
                    if elements.len() != arr.len() {
                        return Ok(None);
                    }
                } else if elements.len() > arr.len() {
                    return Ok(None);
                }

                let mut bindings = Vec::new();

                for (i, elem_pattern) in elements.iter().enumerate() {
                    if i >= arr.len() {
                        return Ok(None);
                    }
                    if let Some(elem_bindings) = self.match_pattern(&arr[i], elem_pattern)? {
                        bindings.extend(elem_bindings);
                    } else {
                        return Ok(None);
                    }
                }

                if let Some(rest_name) = rest {
                    let rest_values =
                        Value::Array(Rc::new(RefCell::new(arr[elements.len()..].to_vec())));
                    bindings.push((rest_name.clone(), rest_values));
                }

                Ok(Some(bindings))
            }

            MatchPattern::Hash { fields, rest } => {
                let hash = match value {
                    Value::Hash(hash) => hash.borrow().clone(),
                    _ => return Ok(None),
                };

                let mut bindings = Vec::new();

                for (field_name, field_pattern) in fields {
                    let hash_key = HashKey::String(field_name.clone());
                    if let Some(val) = hash.get(&hash_key) {
                        if let Some(field_bindings) = self.match_pattern(val, field_pattern)? {
                            bindings.extend(field_bindings);
                        } else {
                            return Ok(None);
                        }
                    } else {
                        return Ok(None);
                    }
                }

                if let Some(rest_name) = rest {
                    let matched_keys: std::collections::HashSet<HashKey> = fields
                        .iter()
                        .map(|(f, _)| HashKey::String(f.clone()))
                        .collect();
                    let rest_map: IndexMap<HashKey, Value> = hash
                        .into_iter()
                        .filter(|(k, _)| !matched_keys.contains(k))
                        .collect();
                    let rest_values = Value::Hash(Rc::new(RefCell::new(rest_map)));
                    bindings.push((rest_name.clone(), rest_values));
                }

                Ok(Some(bindings))
            }

            MatchPattern::Destructuring { type_name, fields } => {
                let instance = match value {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Ok(None),
                };

                if instance.borrow().class.name != *type_name {
                    return Ok(None);
                }

                let mut bindings = Vec::new();

                for (field_name, field_pattern) in fields {
                    if let Some(field_value) = instance.borrow().fields.get(field_name) {
                        if let Some(field_bindings) =
                            self.match_pattern(field_value, field_pattern)?
                        {
                            bindings.extend(field_bindings);
                        } else {
                            return Ok(None);
                        }
                    } else {
                        return Ok(None);
                    }
                }

                Ok(Some(bindings))
            }

            MatchPattern::And(patterns) => {
                let mut all_bindings = Vec::new();
                for p in patterns {
                    match self.match_pattern(value, p)? {
                        Some(bindings) => all_bindings.extend(bindings),
                        None => return Ok(None),
                    }
                }
                Ok(Some(all_bindings))
            }

            MatchPattern::Or(patterns) => {
                for p in patterns {
                    if let Some(_) = self.match_pattern(value, p)? {
                        return self.match_pattern(value, p);
                    }
                }
                Ok(None)
            }
        }
    }

    fn evaluate_literal(&self, literal: &ExprKind) -> RuntimeResult<Value> {
        match literal {
            ExprKind::IntLiteral(n) => Ok(Value::Int(*n)),
            ExprKind::FloatLiteral(n) => Ok(Value::Float(*n)),
            ExprKind::StringLiteral(s) => Ok(Value::String(s.clone())),
            ExprKind::BoolLiteral(b) => Ok(Value::Bool(*b)),
            ExprKind::Null => Ok(Value::Null),
            _ => Err(RuntimeError::type_error(
                "expected literal expression",
                Span::default(),
            )),
        }
    }

    fn values_equal(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            _ => false,
        }
    }
}
