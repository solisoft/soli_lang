//! Function call evaluation.

use crate::ast::expr::Argument;
use crate::ast::{Expr, ExprKind};
use crate::error::RuntimeError;
use crate::interpreter::builtins::server::is_server_listen_marker;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{Instance, Value};
use crate::span::Span;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

impl Interpreter {
    /// Evaluate a function call expression.
    pub(crate) fn evaluate_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Value> {
        // Bypass auto-invoke for callees so that func() gets the raw
        // function reference, not the auto-invoked result.
        let callee_val = match &callee.kind {
            ExprKind::Variable(name) => self.evaluate_variable(name, callee)?,
            ExprKind::Member { object, name } => self.evaluate_member(object, name, callee.span)?,
            ExprKind::SafeMember { object, name } => {
                self.evaluate_safe_member(object, name, callee.span)?
            }
            _ => self.evaluate(callee)?,
        };

        // Safe navigation: if &.method() and object was null, propagate null
        if matches!(callee.kind, ExprKind::SafeMember { .. }) && matches!(callee_val, Value::Null) {
            return Ok(Value::Null);
        }

        let mut arg_values = Vec::new();
        let mut named_args = HashMap::new();

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

    /// Call a value with both positional and named arguments.
    pub(crate) fn call_value_with_named(
        &mut self,
        callee: Value,
        positional_args: Vec<Value>,
        named_args: HashMap<String, Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match callee {
            Value::Function(func) => {
                let required_arity = func.arity();
                let full_arity = func.full_arity();

                let param_names: Vec<String> = func.params.iter().map(|p| p.name.clone()).collect();

                // Check for unknown named arguments
                for name in named_args.keys() {
                    if !param_names.contains(name) {
                        return Err(RuntimeError::undefined_variable(name.clone(), span));
                    }
                }

                // Build final argument list
                let mut final_args = Vec::new();
                let mut used_params = std::collections::HashSet::new();

                // Add positional arguments
                for (i, arg_val) in positional_args.iter().enumerate() {
                    if i < param_names.len() {
                        final_args.push(arg_val.clone());
                        used_params.insert(param_names[i].clone());
                    } else {
                        return Err(RuntimeError::wrong_arity(
                            full_arity,
                            positional_args.len() + named_args.len(),
                            span,
                        ));
                    }
                }

                // Fill in named arguments and defaults
                for (i, param_name) in param_names.iter().enumerate() {
                    if used_params.contains(param_name) {
                        continue;
                    }

                    if let Some(named_val) = named_args.get(param_name) {
                        final_args.push(named_val.clone());
                    } else if let Some(default_expr) = func.param_default_value(i) {
                        let default_value = self.evaluate(default_expr)?;
                        final_args.push(default_value);
                    } else {
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
                    let thread_name = std::thread::current().name().map(|s| s.to_string());
                    let is_main_thread = thread_name
                        .as_ref()
                        .is_some_and(|n| n == "main" || n.starts_with("tokio-runtime"));
                    if is_main_thread {
                        return self.run_http_server(port);
                    }
                }

                Ok(result)
            }

            Value::Class(class) => {
                // Class instantiation
                let instance = Rc::new(RefCell::new(Instance::new(class.clone())));

                if let Some(ref ctor) = class.constructor {
                    let required_arity = ctor.arity();
                    let full_arity = ctor.full_arity();

                    let param_names: Vec<String> =
                        ctor.params.iter().map(|p| p.name.clone()).collect();

                    for name in named_args.keys() {
                        if !param_names.contains(name) {
                            return Err(RuntimeError::undefined_variable(name.clone(), span));
                        }
                    }

                    let mut ctor_args = Vec::new();
                    let mut used_params = std::collections::HashSet::new();

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

                    let ctor_env = Environment::with_enclosing(ctor.closure.clone());
                    let mut ctor_env = ctor_env;
                    ctor_env.define("this".to_string(), Value::Instance(instance.clone()));

                    for (param, value) in ctor.params.iter().zip(ctor_args.iter()) {
                        ctor_env.define(param.name.clone(), value.clone());
                    }

                    let _ = self.execute_block(&ctor.body, ctor_env);
                }

                Ok(Value::Instance(instance))
            }

            Value::Instance(inst) => {
                // Callable instance
                match inst.borrow().get_method("call") {
                    Some(method) => match method {
                        Value::Function(func) => {
                            let required_arity = func.arity();
                            let full_arity = func.full_arity();

                            let param_names: Vec<String> =
                                func.params.iter().map(|p| p.name.clone()).collect();

                            for name in named_args.keys() {
                                if !param_names.contains(name) {
                                    return Err(RuntimeError::undefined_variable(
                                        name.clone(),
                                        span,
                                    ));
                                }
                            }

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
                    },
                    _ => Err(RuntimeError::type_error("instance is not callable", span)),
                }
            }

            Value::Super(superclass) => {
                // Super constructor call
                let this_val =
                    self.environment.borrow().get("this").ok_or_else(|| {
                        RuntimeError::type_error("'super' outside of class", span)
                    })?;

                let instance = match this_val {
                    Value::Instance(inst) => inst,
                    _ => return Err(RuntimeError::type_error("'super' outside of class", span)),
                };

                if let Some(ref ctor) = superclass.constructor {
                    let required_arity = ctor.arity();
                    let full_arity = ctor.full_arity();

                    let param_names: Vec<String> =
                        ctor.params.iter().map(|p| p.name.clone()).collect();

                    for name in named_args.keys() {
                        if !param_names.contains(name) {
                            return Err(RuntimeError::undefined_variable(name.clone(), span));
                        }
                    }

                    let mut ctor_args = Vec::new();
                    let mut used_params = std::collections::HashSet::new();

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

            Value::Method(method) => self.call_method(method, positional_args, span),

            _ => Err(RuntimeError::type_error(
                format!("{} is not callable", callee.type_name()),
                span,
            )),
        }
    }

    /// Call a value with positional arguments only.
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

                while arguments.len() < full_arity {
                    if let Some(default_expr) = func.param_default_value(arguments.len()) {
                        let default_value = self.evaluate(default_expr)?;
                        arguments.push(default_value);
                    } else {
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

                if let Some(port) = is_server_listen_marker(&result) {
                    let thread_name = std::thread::current().name().map(|s| s.to_string());
                    let is_main_thread = thread_name
                        .as_ref()
                        .is_some_and(|n| n == "main" || n.starts_with("tokio-runtime"));
                    if is_main_thread {
                        return self.run_http_server(port);
                    }
                }

                Ok(result)
            }

            Value::Class(class) => {
                let instance = Rc::new(RefCell::new(Instance::new(class.clone())));

                if let Some(ref ctor) = class.constructor {
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

                    let mut final_args = arguments.clone();
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

            Value::Method(method) => self.call_method(method, arguments, span),

            _ => Err(RuntimeError::not_callable(span)),
        }
    }
}
