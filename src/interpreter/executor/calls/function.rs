//! Function call evaluation.

use crate::ast::expr::Argument;
use crate::ast::{Expr, ExprKind};
use crate::error::RuntimeError;
use crate::interpreter::builtins::server::is_server_listen_marker;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{HashKey, Instance, StrKey, Value};
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
        // All three fast paths below require the callee to be a Member/SafeMember
        // expression. For ordinary function calls like `print(x)` or `block()`
        // (callee is a Variable) there's no point calling them at all — skip
        // the three pattern-matches up front.
        if matches!(
            callee.kind,
            ExprKind::Member { .. } | ExprKind::SafeMember { .. }
        ) {
            if let Some(result) = self.try_run_model_before_save(callee, arguments, span)? {
                return Ok(result);
            }
            if let Some(result) = self.try_evaluate_hash_string_key_call(callee, arguments, span)? {
                return Ok(result);
            }
            if let Some(result) =
                self.try_evaluate_direct_hash_method_call(callee, arguments, span)?
            {
                return Ok(result);
            }
            if let Some(result) =
                self.try_evaluate_direct_string_method_call(callee, arguments, span)?
            {
                return Ok(result);
            }
        }

        // Bypass auto-invoke for callees so that func() gets the raw
        // function reference, not the auto-invoked result.
        let callee_val = self.evaluate_callee(callee)?;

        // Safe navigation: if &.method() and object was null, propagate null
        if matches!(callee.kind, ExprKind::SafeMember { .. }) && matches!(callee_val, Value::Null) {
            return Ok(Value::Null);
        }

        // Common fast path: all-positional arguments (no named, no block),
        // and the callee is a value `call_value` can dispatch (Function,
        // NativeFunction, Class, Method). Value::Super is specifically
        // handled only in `call_value_with_named`, so exclude it here.
        let all_positional = arguments
            .iter()
            .all(|a| matches!(a, Argument::Positional(_)));
        let fast_path_callable = !matches!(callee_val, Value::Super(_));
        if all_positional && fast_path_callable {
            let mut arg_values = Vec::with_capacity(arguments.len());
            for arg in arguments {
                if let Argument::Positional(expr) = arg {
                    arg_values.push(self.evaluate(expr)?);
                }
            }
            return self.call_value(callee_val, arg_values, span);
        }

        let mut arg_values = Vec::new();
        let mut named_args = HashMap::new();
        let mut block_arg: Option<Value> = None;

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
                Argument::Block(expr) => {
                    block_arg = Some(self.evaluate(expr)?);
                }
            }
        }

        self.call_value_with_named(callee_val, arg_values, named_args, block_arg, span)
    }

    /// Intercept `SomeModel.create(data)` / `SomeModel.update(id, data)` when
    /// the model has before_save / before_create / before_update callbacks
    /// registered via `before_save("normalize_email")` etc. Builds a temp
    /// instance from the data hash, invokes each callback with `this` bound
    /// to it, extracts the modified fields back to the hash, then delegates
    /// to the underlying native function with the transformed data.
    ///
    /// Returns Ok(None) if this call is not a model create/update needing
    /// callbacks — the caller falls through to the normal dispatch.
    fn try_run_model_before_save(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::get_or_create_metadata;

        let (object, method_name) = match &callee.kind {
            ExprKind::Member { object, name } => (object.as_ref(), name.as_str()),
            _ => return Ok(None),
        };
        if !matches!(method_name, "create" | "update") {
            return Ok(None);
        }

        // All-positional args only (the common case).
        let all_positional = arguments
            .iter()
            .all(|a| matches!(a, Argument::Positional(_)));
        if !all_positional {
            return Ok(None);
        }

        let obj_val = self.evaluate(object)?;
        let class = match &obj_val {
            Value::Class(c) if c.is_model_subclass() => c.clone(),
            _ => return Ok(None),
        };

        let metadata = get_or_create_metadata(&class.name);
        let callback_names: Vec<String> = if method_name == "create" {
            metadata
                .callbacks
                .before_save
                .iter()
                .chain(metadata.callbacks.before_create.iter())
                .cloned()
                .collect()
        } else {
            metadata
                .callbacks
                .before_save
                .iter()
                .chain(metadata.callbacks.before_update.iter())
                .cloned()
                .collect()
        };
        if callback_names.is_empty() {
            return Ok(None);
        }

        // Evaluate all arguments upfront.
        let mut arg_values: Vec<Value> = Vec::with_capacity(arguments.len());
        for arg in arguments {
            if let Argument::Positional(expr) = arg {
                arg_values.push(self.evaluate(expr)?);
            }
        }

        // For `create(data)` the data is args[0]; for `update(id, data)` it's args[1].
        let data_index = if method_name == "create" { 0 } else { 1 };
        let Some(original_data) = arg_values.get(data_index).cloned() else {
            return Ok(None);
        };
        let data_hash = match &original_data {
            Value::Hash(h) => h.clone(),
            _ => return Ok(None),
        };

        // Build a temp Instance populated with the data-hash's fields so the
        // callback methods can read/write via `this.email = ...`.
        let mut instance = Instance::new(class.clone());
        for (k, v) in data_hash.borrow().iter() {
            if let HashKey::String(name) = k {
                instance.set(name.clone(), v.clone());
            }
        }
        let inst_rc = Rc::new(RefCell::new(instance));

        // Run each callback method with `this` bound to the temp instance.
        for cb_name in &callback_names {
            let Some(method) = class.find_method(cb_name) else {
                continue;
            };
            let mut bound_env = Environment::with_enclosing(method.closure.clone());
            bound_env.define("this".to_string(), Value::Instance(inst_rc.clone()));

            let bound_method = crate::interpreter::value::Function {
                name: method.name.clone(),
                params: method.params.clone(),
                body: method.body.clone(),
                closure: Rc::new(RefCell::new(bound_env)),
                is_method: true,
                span: method.span,
                source_path: method.source_path.clone(),
                defining_superclass: None,
                return_type: method.return_type.clone(),
                cached_env: RefCell::new(None),
                jit_cache: RefCell::new(None),
            };
            self.call_value(Value::Function(Rc::new(bound_method)), Vec::new(), span)?;
        }

        // Copy the instance's fields back into a new hash — preserving any
        // extra fields callbacks added, and picking up the mutations.
        let inst_ref = inst_rc.borrow();
        let mut new_pairs = crate::interpreter::value::HashPairs::default();
        for (k, v) in &inst_ref.fields {
            new_pairs.insert(HashKey::String(k.clone()), v.clone());
        }
        drop(inst_ref);
        arg_values[data_index] = Value::Hash(Rc::new(RefCell::new(new_pairs)));

        // Dispatch to the class's native static method (Model.create / Model.update)
        // with the transformed data. We evaluate the callee fresh so the native
        // fn closure gets the class as `args[0]` like the normal path.
        let callee_val = self.evaluate_callee(callee)?;
        let result = self.call_value(callee_val, arg_values, span)?;

        // Run after_save / after_create / after_update with `this` bound to
        // the persisted record — convention: Model.create returns
        // `{ "valid": true, "record": <Instance> }` on success.
        let after_names: Vec<String> = if method_name == "create" {
            metadata
                .callbacks
                .after_create
                .iter()
                .chain(metadata.callbacks.after_save.iter())
                .cloned()
                .collect()
        } else {
            metadata
                .callbacks
                .after_update
                .iter()
                .chain(metadata.callbacks.after_save.iter())
                .cloned()
                .collect()
        };
        if !after_names.is_empty() {
            if let Value::Hash(result_hash) = &result {
                let valid = result_hash
                    .borrow()
                    .get(&HashKey::String("valid".to_string()))
                    .cloned();
                let record = result_hash
                    .borrow()
                    .get(&HashKey::String("record".to_string()))
                    .cloned();
                if matches!(valid, Some(Value::Bool(true))) {
                    if let Some(Value::Instance(inst)) = record {
                        for cb_name in &after_names {
                            let Some(method) = class.find_method(cb_name) else {
                                continue;
                            };
                            let mut bound_env = Environment::with_enclosing(method.closure.clone());
                            bound_env.define("this".to_string(), Value::Instance(inst.clone()));
                            let bound_method = crate::interpreter::value::Function {
                                name: method.name.clone(),
                                params: method.params.clone(),
                                body: method.body.clone(),
                                closure: Rc::new(RefCell::new(bound_env)),
                                is_method: true,
                                span: method.span,
                                source_path: method.source_path.clone(),
                                defining_superclass: None,
                                return_type: method.return_type.clone(),
                                cached_env: RefCell::new(None),
                                jit_cache: RefCell::new(None),
                            };
                            self.call_value(
                                Value::Function(Rc::new(bound_method)),
                                Vec::new(),
                                span,
                            )?;
                        }
                    }
                }
            }
        }

        Ok(Some(result))
    }

    fn try_evaluate_hash_string_key_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        let (object, method_name, safe_navigation) = match &callee.kind {
            ExprKind::Member { object, name } => (object.as_ref(), name.as_str(), false),
            ExprKind::SafeMember { object, name } => (object.as_ref(), name.as_str(), true),
            _ => return Ok(None),
        };

        if !matches!(method_name, "get" | "fetch" | "has_key" | "delete" | "set") {
            return Ok(None);
        }

        if arguments
            .iter()
            .any(|arg| !matches!(arg, Argument::Positional(_)))
        {
            return Ok(None);
        }

        let hash_value = self.evaluate(object)?;
        if safe_navigation && matches!(hash_value, Value::Null) {
            return Ok(Some(Value::Null));
        }

        let hash = match hash_value {
            Value::Hash(hash) => hash,
            _ => return Ok(None),
        };

        match (method_name, arguments) {
            ("get", [Argument::Positional(key)]) => {
                let ExprKind::StringLiteral(key) = &key.kind else {
                    return Ok(None);
                };
                let value = hash
                    .borrow()
                    .get(&StrKey(key))
                    .cloned()
                    .unwrap_or(Value::Null);
                Ok(Some(value))
            }
            ("get", [Argument::Positional(key), Argument::Positional(default_expr)]) => {
                let ExprKind::StringLiteral(key) = &key.kind else {
                    return Ok(None);
                };
                let default_value = self.evaluate(default_expr)?;
                let value = hash
                    .borrow()
                    .get(&StrKey(key))
                    .cloned()
                    .unwrap_or(default_value);
                Ok(Some(value))
            }
            ("fetch", [Argument::Positional(key)]) => {
                let ExprKind::StringLiteral(key) = &key.kind else {
                    return Ok(None);
                };
                let value = hash.borrow().get(&StrKey(key)).cloned();
                Ok(Some(match value {
                    Some(value) => value,
                    None => {
                        return Err(RuntimeError::type_error(
                            format!("key not found: {:?}", Value::String(key.clone())),
                            span,
                        ))
                    }
                }))
            }
            ("fetch", [Argument::Positional(key), Argument::Positional(default_expr)]) => {
                let ExprKind::StringLiteral(key) = &key.kind else {
                    return Ok(None);
                };
                let default_value = self.evaluate(default_expr)?;
                let value = hash
                    .borrow()
                    .get(&StrKey(key))
                    .cloned()
                    .unwrap_or(default_value);
                Ok(Some(value))
            }
            ("has_key", [Argument::Positional(key)]) => {
                let ExprKind::StringLiteral(key) = &key.kind else {
                    return Ok(None);
                };
                Ok(Some(Value::Bool(hash.borrow().contains_key(&StrKey(key)))))
            }
            ("delete", [Argument::Positional(key)]) => {
                let ExprKind::StringLiteral(key) = &key.kind else {
                    return Ok(None);
                };
                Ok(Some(
                    hash.borrow_mut()
                        .shift_remove(&StrKey(key))
                        .unwrap_or(Value::Null),
                ))
            }
            ("set", [Argument::Positional(key), Argument::Positional(value_expr)]) => {
                let ExprKind::StringLiteral(key) = &key.kind else {
                    return Ok(None);
                };
                let value = self.evaluate(value_expr)?;
                let mut hash_ref = hash.borrow_mut();
                if let Some((_, _, existing)) = hash_ref.get_full_mut(&StrKey(key)) {
                    *existing = value.clone();
                } else {
                    hash_ref.insert(HashKey::String(key.clone()), value.clone());
                }
                Ok(Some(value))
            }
            ("get", _) | ("fetch", _) => Err(RuntimeError::wrong_arity(1, arguments.len(), span)),
            ("has_key", _) | ("delete", _) => {
                Err(RuntimeError::wrong_arity(1, arguments.len(), span))
            }
            ("set", _) => Err(RuntimeError::wrong_arity(2, arguments.len(), span)),
            _ => Ok(None),
        }
    }

    fn try_evaluate_direct_hash_method_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        let (object, method_name, safe_navigation) = match &callee.kind {
            ExprKind::Member { object, name } => (object.as_ref(), name.as_str(), false),
            ExprKind::SafeMember { object, name } => (object.as_ref(), name.as_str(), true),
            _ => return Ok(None),
        };

        if !matches!(
            method_name,
            "length"
                | "len"
                | "size"
                | "map"
                | "filter"
                | "each"
                | "get"
                | "fetch"
                | "invert"
                | "transform_values"
                | "transform_keys"
                | "select"
                | "reject"
                | "slice"
                | "except"
                | "compact"
                | "dig"
                | "to_string"
                | "to_json"
                | "keys"
                | "values"
                | "has_key"
                | "delete"
                | "merge"
                | "entries"
                | "clear"
                | "set"
                | "empty?"
                | "is_a?"
        ) {
            return Ok(None);
        }

        if arguments
            .iter()
            .any(|arg| !matches!(arg, Argument::Positional(_)))
        {
            return Ok(None);
        }

        let hash_value = self.evaluate(object)?;
        if safe_navigation && matches!(hash_value, Value::Null) {
            return Ok(Some(Value::Null));
        }

        let hash = match hash_value {
            Value::Hash(hash) => hash,
            _ => return Ok(None),
        };

        let mut arg_values = Vec::with_capacity(arguments.len());
        for arg in arguments {
            let Argument::Positional(expr) = arg else {
                unreachable!();
            };
            arg_values.push(self.evaluate(expr)?);
        }

        Ok(Some(self.call_hash_method_on_rc(
            &hash,
            method_name,
            arg_values,
            span,
        )?))
    }

    fn try_evaluate_direct_string_method_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        let (object, method_name, safe_navigation) = match &callee.kind {
            ExprKind::Member { object, name } => (object.as_ref(), name.as_str(), false),
            ExprKind::SafeMember { object, name } => (object.as_ref(), name.as_str(), true),
            _ => return Ok(None),
        };

        if !matches!(
            method_name,
            "length"
                | "len"
                | "size"
                | "to_s"
                | "to_string"
                | "upcase"
                | "uppercase"
                | "downcase"
                | "lowercase"
                | "trim"
                | "strip"
                | "lstrip"
                | "rstrip"
                | "reverse"
                | "empty?"
                | "contains"
                | "includes?"
                | "starts_with"
                | "starts_with?"
                | "ends_with"
                | "ends_with?"
                | "split"
                | "replace"
                | "join"
        ) {
            return Ok(None);
        }

        if arguments
            .iter()
            .any(|arg| !matches!(arg, Argument::Positional(_)))
        {
            return Ok(None);
        }

        let string_value = self.evaluate(object)?;
        if safe_navigation && matches!(string_value, Value::Null) {
            return Ok(Some(Value::Null));
        }

        let s = match string_value {
            Value::String(s) => s,
            _ => return Ok(None),
        };

        match (method_name, arguments) {
            (
                "length" | "len" | "size" | "to_s" | "to_string" | "upcase" | "uppercase"
                | "downcase" | "lowercase" | "trim" | "strip" | "lstrip" | "rstrip" | "reverse"
                | "empty?" | "join",
                [],
            ) => {
                if let Some(result) = self.call_string_method_borrowed(&s, method_name, &[], span) {
                    return Ok(Some(result?));
                }
            }
            (
                "contains" | "includes?" | "starts_with" | "starts_with?" | "ends_with"
                | "ends_with?" | "split",
                [Argument::Positional(arg)],
            ) => {
                let ExprKind::StringLiteral(arg) = &arg.kind else {
                    let arg_values = vec![self.evaluate(arg)?];
                    if let Some(result) =
                        self.call_string_method_borrowed(&s, method_name, &arg_values, span)
                    {
                        return Ok(Some(result?));
                    }
                    return Ok(Some(self.call_string_method(
                        &s,
                        method_name,
                        arg_values,
                        span,
                    )?));
                };
                let args = [Value::String(arg.clone())];
                if let Some(result) = self.call_string_method_borrowed(&s, method_name, &args, span)
                {
                    return Ok(Some(result?));
                }
            }
            ("replace", [Argument::Positional(from), Argument::Positional(to)]) => {
                let (ExprKind::StringLiteral(from), ExprKind::StringLiteral(to)) =
                    (&from.kind, &to.kind)
                else {
                    let mut arg_values = Vec::with_capacity(2);
                    let Argument::Positional(from) = &arguments[0] else {
                        unreachable!()
                    };
                    let Argument::Positional(to) = &arguments[1] else {
                        unreachable!()
                    };
                    arg_values.push(self.evaluate(from)?);
                    arg_values.push(self.evaluate(to)?);
                    if let Some(result) =
                        self.call_string_method_borrowed(&s, method_name, &arg_values, span)
                    {
                        return Ok(Some(result?));
                    }
                    return Ok(Some(self.call_string_method(
                        &s,
                        method_name,
                        arg_values,
                        span,
                    )?));
                };
                let args = [Value::String(from.clone()), Value::String(to.clone())];
                if let Some(result) = self.call_string_method_borrowed(&s, method_name, &args, span)
                {
                    return Ok(Some(result?));
                }
            }
            _ => {}
        }

        let mut arg_values = Vec::with_capacity(arguments.len());
        for arg in arguments {
            let Argument::Positional(expr) = arg else {
                unreachable!();
            };
            arg_values.push(self.evaluate(expr)?);
        }

        if let Some(result) = self.call_string_method_borrowed(&s, method_name, &arg_values, span) {
            return Ok(Some(result?));
        }

        Ok(Some(self.call_string_method(
            &s,
            method_name,
            arg_values,
            span,
        )?))
    }

    /// Call a value with both positional and named arguments.
    pub(crate) fn call_value_with_named(
        &mut self,
        callee: Value,
        positional_args: Vec<Value>,
        named_args: HashMap<String, Value>,
        block_arg: Option<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match callee {
            Value::Function(func) => {
                // Filter out block parameters from the regular params for arity calculation
                let non_block_params: Vec<_> =
                    func.params.iter().filter(|p| !p.is_block_param).collect();
                let required_arity = non_block_params
                    .iter()
                    .filter(|p| p.default_value.is_none())
                    .count();
                let full_arity = non_block_params.len();

                let param_names: Vec<String> =
                    non_block_params.iter().map(|p| p.name.clone()).collect();

                // Find block parameter if any
                let block_param_index = func.params.iter().position(|p| p.is_block_param);
                let block_param_name = block_param_index.map(|i| func.params[i].name.clone());

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

                // Handle block argument - bind to block parameter if exists
                if let Some(ref block_param) = block_param_name {
                    if let Some(block_val) = block_arg {
                        // Add block as a named argument for the block parameter
                        let block_idx = func
                            .params
                            .iter()
                            .position(|n| n.name == *block_param)
                            .unwrap();
                        if block_idx < final_args.len() {
                            final_args[block_idx] = block_val;
                        } else {
                            final_args.push(block_val);
                        }
                    } else if !named_args.contains_key(block_param) {
                        // Block param exists but no block was passed
                        // This is OK - block will be nil/null
                        final_args.push(Value::Null);
                    }
                }

                self.call_function(&func, final_args)
            }

            Value::NativeFunction(native) => {
                let mut all_args = positional_args.clone();

                // Add block argument as last positional arg for native functions
                if let Some(block_val) = block_arg {
                    all_args.push(block_val);
                }

                if all_args.len() != native.arity.unwrap_or(all_args.len()) {
                    return Err(RuntimeError::wrong_arity(
                        native.arity.unwrap_or(0),
                        all_args.len(),
                        span,
                    ));
                }
                if !named_args.is_empty() {
                    return Err(RuntimeError::type_error(
                        "native functions do not support named arguments".to_string(),
                        span,
                    ));
                }

                let result = (native.func)(all_args)
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

            Value::Method(method) => {
                let mut args = positional_args;
                // Forward block argument as last positional arg for method calls
                if let Some(block_val) = block_arg {
                    args.push(block_val);
                }
                self.call_method(method, args, span)
            }

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
