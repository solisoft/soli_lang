//! Expression evaluation.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::*;
use crate::error::RuntimeError;
use crate::interpreter::builtins::model::{
    execute_query_builder, execute_query_builder_count, execute_query_builder_first,
};
use crate::interpreter::builtins::server::is_server_listen_marker;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Function, Instance, NativeFunction, Value, ValueMethod};
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

            ExprKind::Call { callee, arguments } => {
                self.evaluate_call(callee, arguments, expr.span)
            }

            ExprKind::Pipeline { left, right } => self.evaluate_pipeline(left, right, expr.span),

            ExprKind::Member { object, name } => self.evaluate_member(object, name, expr.span),

            ExprKind::Index { object, index } => self.evaluate_index(object, index, expr.span),

            ExprKind::This => self
                .environment
                .borrow()
                .get("this")
                .ok_or_else(|| RuntimeError::type_error("'this' outside of class", expr.span)),

            ExprKind::Super => Err(RuntimeError::type_error(
                "'super' can only be used for method calls",
                expr.span,
            )),

            ExprKind::New {
                class_name,
                arguments,
            } => self.evaluate_new(class_name, arguments, expr.span),

            ExprKind::Array(elements) => {
                let mut values = Vec::new();
                for elem in elements {
                    match &elem.kind {
                        ExprKind::Spread(inner) => {
                            // Evaluate the spread expression and extend with its elements
                            let spread_val = self.evaluate(inner)?;
                            match spread_val {
                                Value::Array(arr) => {
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
                let array = match iter_value {
                    Value::Array(arr) => arr.borrow().clone(),
                    _ => {
                        return Err(RuntimeError::type_error("expected array", iterable.span));
                    }
                };

                // Create a new environment for the comprehension
                let mut result = Vec::new();
                for item in array {
                    // Create a new scope for each iteration (like for loops)
                    let loop_env = Environment::with_enclosing(self.environment.clone());
                    let prev_env =
                        std::mem::replace(&mut self.environment, Rc::new(RefCell::new(loop_env)));

                    // Define the loop variable
                    self.environment.borrow_mut().define(variable.clone(), item);

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
                let array = match iter_value {
                    Value::Array(arr) => arr.borrow().clone(),
                    _ => {
                        return Err(RuntimeError::type_error("expected array", iterable.span));
                    }
                };

                // Create a new environment for the comprehension
                let mut result = Vec::new();
                for item in array {
                    // Create a new scope for each iteration (like for loops)
                    let loop_env = Environment::with_enclosing(self.environment.clone());
                    let prev_env =
                        std::mem::replace(&mut self.environment, Rc::new(RefCell::new(loop_env)));

                    // Define the loop variable
                    self.environment.borrow_mut().define(variable.clone(), item);

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
                        result.push((key_value, val_value));
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
                        let resolved = left_val.resolve().map_err(|e| RuntimeError::type_error(e, span))?;
                        if let Value::Array(arr) = resolved {
                            let items: Vec<Value> = arr.borrow().clone();
                            // Evaluate the function argument
                            let mut args = Vec::new();
                            for arg in arguments {
                                args.push(self.evaluate(arg)?);
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
                    new_args.push(self.evaluate(arg)?);
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
                // First check for field
                if let Some(value) = inst.borrow().get(name) {
                    return Ok(value);
                }
                // Then check for native method
                if let Some(native_method) = inst.borrow().class.find_native_method(name) {
                    // Create a wrapper that will call the native method with 'this'
                    let instance_clone = inst.clone();
                    let native_method_clone = native_method.clone();
                    return Ok(Value::NativeFunction(NativeFunction::new(
                        &format!("{}.{}", inst.borrow().class.name, name),
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
                if let Some(method) = inst.borrow().class.find_method(name) {
                    // Bind 'this'
                    let bound_env = Environment::with_enclosing(method.closure.clone());
                    let mut bound_env = bound_env;
                    bound_env.define("this".to_string(), Value::Instance(inst.clone()));

                    let bound_method = Function {
                        name: method.name.clone(),
                        params: method.params.clone(),
                        body: method.body.clone(),
                        closure: Rc::new(RefCell::new(bound_env)),
                        is_method: true,
                        span: method.span,
                        source_path: method.source_path.clone(),
                    };
                    return Ok(Value::Function(Rc::new(bound_method)));
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: inst.borrow().class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Class(class) => {
                // Static method access - search up superclass chain
                if let Some(method) = class.find_static_method(name) {
                    return Ok(Value::Function(method));
                }
                if let Some(native_method) = class.find_native_static_method(name) {
                    // Native static methods don't need the class prepended
                    // Just return the native function directly
                    return Ok(Value::NativeFunction((*native_method).clone()));
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Array(_) => {
                // Handle array methods: map, filter, each
                match name {
                    "map" | "filter" | "each" => Ok(Value::Method(ValueMethod {
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
            Value::Hash(_) => {
                // Handle hash methods
                match name {
                    "map" | "filter" | "each" => Ok(Value::Method(ValueMethod {
                        receiver: Box::new(obj_val),
                        method_name: name.to_string(),
                    })),
                    _ => Err(RuntimeError::NoSuchProperty {
                        value_type: "Hash".to_string(),
                        property: name.to_string(),
                        span,
                    }),
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
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: obj_val.type_name().to_string(),
                property: name.to_string(),
                span,
            }),
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
                if !key.is_hashable() {
                    return Err(RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key.type_name()),
                        index.span,
                    ));
                }
                let hash = hash.borrow();
                for (k, v) in hash.iter() {
                    if key.hash_eq(k) {
                        return Ok(v.clone());
                    }
                }
                // Return null for missing keys (like Ruby)
                Ok(Value::Null)
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
        class_name: &str,
        arguments: &[Expr],
        span: Span,
    ) -> RuntimeResult<Value> {
        let class = match self.environment.borrow().get(class_name) {
            Some(Value::Class(c)) => c,
            Some(_) => return Err(RuntimeError::NotAClass(class_name.to_string(), span)),
            None => return Err(RuntimeError::undefined_variable(class_name, span)),
        };

        // Create instance
        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));

        // Call constructor if present
        if let Some(ref ctor) = class.constructor {
            let mut arg_values = Vec::new();
            for arg in arguments {
                arg_values.push(self.evaluate(arg)?);
            }

            // Check arity
            if arg_values.len() != ctor.arity() {
                return Err(RuntimeError::wrong_arity(
                    ctor.arity(),
                    arg_values.len(),
                    span,
                ));
            }

            // Create constructor environment
            let ctor_env = Environment::with_enclosing(ctor.closure.clone());
            let mut ctor_env = ctor_env;
            ctor_env.define("this".to_string(), Value::Instance(instance.clone()));

            for (param, value) in ctor.params.iter().zip(arg_values) {
                ctor_env.define(param.name.clone(), value);
            }

            // Execute constructor body
            let _ = self.execute_block(&ctor.body, ctor_env);
        }

        Ok(Value::Instance(instance))
    }

    fn evaluate_hash(&mut self, pairs: &[(Expr, Expr)]) -> RuntimeResult<Value> {
        let mut entries = Vec::new();
        for (key_expr, value_expr) in pairs {
            let key = self.evaluate(key_expr)?;
            if !key.is_hashable() {
                return Err(RuntimeError::type_error(
                    format!("{} cannot be used as a hash key", key.type_name()),
                    key_expr.span,
                ));
            }
            let value = self.evaluate(value_expr)?;
            // Check if key already exists and update, otherwise add
            let mut found = false;
            for (k, v) in &mut entries {
                if key.hash_eq(k) {
                    *v = value.clone();
                    found = true;
                    break;
                }
            }
            if !found {
                entries.push((key, value));
            }
        }
        Ok(Value::Hash(Rc::new(RefCell::new(entries))))
    }

    fn evaluate_assign(&mut self, target: &Expr, value: &Expr) -> RuntimeResult<Value> {
        let new_value = self.evaluate(value)?;

        match &target.kind {
            ExprKind::Variable(name) => {
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
                if !key.is_hashable() {
                    return Err(RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key.type_name()),
                        index.span,
                    ));
                }
                let mut hash = hash.borrow_mut();
                // Find existing key or add new entry
                for (k, v) in hash.iter_mut() {
                    if key.hash_eq(k) {
                        *v = new_value.clone();
                        return Ok(new_value);
                    }
                }
                // Key not found, add new entry
                hash.push((idx_val.clone(), new_value.clone()));
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
        };
        Ok(Value::Function(Rc::new(func)))
    }

    fn evaluate_call(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
        span: Span,
    ) -> RuntimeResult<Value> {
        let callee_val = self.evaluate(callee)?;

        let mut arg_values = Vec::new();
        for arg in arguments {
            arg_values.push(self.evaluate(arg)?);
        }

        self.call_value(callee_val, arg_values, span)
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
                        .map_or(false, |n| n == "main" || n.starts_with("tokio-runtime"));
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
                // Handle array/hash/QueryBuilder methods
                match *method.receiver {
                    Value::Array(arr) => {
                        let items = arr.borrow().clone();
                        self.call_array_method(&items, &method.method_name, arguments, span)
                    }
                    Value::Hash(hash) => {
                        let entries = hash.borrow().clone();
                        self.call_hash_method(&entries, &method.method_name, arguments, span)
                    }
                    Value::QueryBuilder(qb) => {
                        self.call_query_builder_method(qb, &method.method_name, arguments, span)
                    }
                    _ => Err(RuntimeError::type_error(
                        format!("{} does not support methods", method.receiver.type_name()),
                        span,
                    )),
                }
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

                let mut result = Vec::new();
                for item in items {
                    // Create new environment with the closure
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    // Define the parameter (use first param name or default)
                    let param_name = func
                        .params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "it".to_string());
                    call_env.define(param_name, item.clone());

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

                let mut result = Vec::new();
                for item in items {
                    // Create new environment with the closure
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    // Define the parameter
                    let param_name = func
                        .params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "it".to_string());
                    call_env.define(param_name, item.clone());

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

                for item in items {
                    // Create new environment with the closure
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    // Define the parameter
                    let param_name = func
                        .params
                        .first()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "it".to_string());
                    call_env.define(param_name, item.clone());

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
        entries: &[(Value, Value)],
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

                let mut result = Vec::new();
                for (key, value) in entries {
                    // Create new environment with the closure
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    // Define parameters: key and value
                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), key.clone());
                        call_env.define(func.params[1].name.clone(), value.clone());
                    } else if func.params.len() == 1 {
                        // If only one param, pass [key, value] array
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![key.clone(), value.clone()])));
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
                                    if !new_key.is_hashable() {
                                        return Err(RuntimeError::type_error(
                                            "hash key must be hashable",
                                            span,
                                        ));
                                    }
                                    result.push((new_key, new_val));
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

                let mut result = Vec::new();
                for (key, value) in entries {
                    // Create new environment with the closure
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    // Define parameters: key and value
                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), key.clone());
                        call_env.define(func.params[1].name.clone(), value.clone());
                    } else if func.params.len() == 1 {
                        // If only one param, pass [key, value] array
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![key.clone(), value.clone()])));
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
                        result.push((key.clone(), value.clone()));
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
                    let call_env = Environment::with_enclosing(func.closure.clone());
                    let mut call_env = call_env;

                    // Define parameters: key and value
                    if func.params.len() >= 2 {
                        call_env.define(func.params[0].name.clone(), key.clone());
                        call_env.define(func.params[1].name.clone(), value.clone());
                    } else if func.params.len() == 1 {
                        // If only one param, pass [key, value] array
                        let pair =
                            Value::Array(Rc::new(RefCell::new(vec![key.clone(), value.clone()])));
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
                Ok(Value::Hash(Rc::new(RefCell::new(entries.to_vec()))))
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
                    _ => return Err(RuntimeError::type_error("where() expects string filter expression", span)),
                };
                let bind_vars = match &arguments[1] {
                    Value::Hash(hash) => {
                        let mut map = std::collections::HashMap::new();
                        for (k, v) in hash.borrow().iter() {
                            if let Value::String(key) = k {
                                map.insert(key.clone(), crate::interpreter::builtins::model::value_to_json(v)
                                    .map_err(|e| RuntimeError::General { message: e, span })?);
                            }
                        }
                        map
                    }
                    _ => return Err(RuntimeError::type_error("where() expects hash for bind variables", span)),
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
                new_qb.bind_vars.extend(bind_vars);
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
                    _ => return Err(RuntimeError::type_error("order() expects string field", span)),
                };
                let direction = if arguments.len() == 2 {
                    match &arguments[1] {
                        Value::String(s) => s.clone(),
                        _ => return Err(RuntimeError::type_error("order() expects string direction", span)),
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
                    _ => return Err(RuntimeError::type_error("limit() expects positive integer", span)),
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
                    _ => return Err(RuntimeError::type_error("offset() expects positive integer", span)),
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

                if elements.len() > arr.len() && rest.is_none() {
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

                let hash_vec: Vec<(Value, Value)> = hash;

                let mut bindings = Vec::new();

                for (field_name, field_pattern) in fields {
                    let mut found = false;
                    for (key, val) in &hash_vec {
                        if let Value::String(s) = key {
                            if s == field_name {
                                found = true;
                                if let Some(field_bindings) =
                                    self.match_pattern(val, field_pattern)?
                                {
                                    bindings.extend(field_bindings);
                                } else {
                                    return Ok(None);
                                }
                                break;
                            }
                        }
                    }
                    if !found {
                        return Ok(None);
                    }
                }

                if let Some(rest_name) = rest {
                    let mut rest_vec = Vec::new();
                    for (key, val) in hash_vec {
                        let is_matched = fields.iter().any(|(f, _)| {
                            if let Value::String(s) = &key {
                                s == f
                            } else {
                                false
                            }
                        });
                        if !is_matched {
                            rest_vec.push((key, val));
                        }
                    }
                    let rest_values = Value::Hash(Rc::new(RefCell::new(rest_vec)));
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
