//! Function call evaluation.

use crate::ast::expr::Argument;
use crate::ast::{Expr, ExprKind};
use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{HashKey, Instance, Value};
use crate::span::Span;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Check whether any closure-based callbacks are registered for any of `events`
/// on `class_name`. Used to decide whether to enter the after-callback block
/// even when no method-name callbacks exist.
fn has_closure_callbacks(class_name: &str, events: &[&str]) -> bool {
    events.iter().any(|ev| {
        !crate::interpreter::builtins::model::callbacks::closure_callbacks_for(class_name, ev)
            .is_empty()
    })
}

/// SEC-086a: stamp `_errors = [{"message": "...callback aborted persistence"}]`
/// onto an instance when a `before_*` callback returns `false`. Mirrors the
/// validation-failure / DB-failure shape (`Array<Hash>`) that
/// `instance.update`, `Model.create`, etc. already use, so callers
/// inspecting `instance._errors` see a single uniform contract.
fn set_callback_aborted_error(instance: &Rc<RefCell<Instance>>, callback_kind: &str) {
    let mut entry = crate::interpreter::value::HashPairs::default();
    entry.insert(
        HashKey::String("message".into()),
        Value::String(
            format!(
                "{} callback returned false; persistence aborted",
                callback_kind
            )
            .into(),
        ),
    );
    let error_hash = Value::Hash(Rc::new(RefCell::new(entry)));
    instance.borrow_mut().set(
        "_errors".to_string(),
        Value::Array(Rc::new(RefCell::new(vec![error_hash]))),
    );
}

/// SEC-086: pick the (before-events, after-events) pair the persist
/// interceptor should fire for `method_name`. `instance.save` branches on
/// whether the instance is already persisted (`has_key == true`) — new
/// records run the create chain, persisted records run the update chain.
/// All other instance mutators (`update`, `restore`, `increment`,
/// `decrement`, `touch`) run the update chain regardless. Returns
/// `None` for method names this interceptor doesn't handle.
fn persist_events_for(
    method_name: &str,
    has_key: bool,
) -> Option<(&'static [&'static str], &'static [&'static str])> {
    match method_name {
        "save" => {
            if has_key {
                Some((
                    &["before_save", "before_update"],
                    &["after_update", "after_save"],
                ))
            } else {
                Some((
                    &["before_save", "before_create"],
                    &["after_create", "after_save"],
                ))
            }
        }
        "update" | "restore" | "increment" | "decrement" | "touch" => Some((
            &["before_save", "before_update"],
            &["after_update", "after_save"],
        )),
        _ => None,
    }
}

/// Member names handled inline by `instance_member_access` before any
/// field/method lookup (the "universal methods" match at its top). The
/// direct instance-method call fast path must never claim these. Keep in
/// sync with that match — a name missing here would shadow the universal
/// behavior with a user method of the same name (which the slow path
/// resolves the other way around).
fn is_universal_instance_member(name: &str) -> bool {
    matches!(
        name,
        "inspect"
            | "class"
            | "is_a?"
            | "nil?"
            | "blank?"
            | "present?"
            | "respond_to?"
            | "send"
            | "instance_variables"
            | "instance_variable_get"
            | "instance_variable_set"
            | "define_method"
            | "alias_method"
            | "instance_eval"
            | "methods"
    )
}

/// Hash method names eligible for the direct `h.method(args)` dispatch in
/// `evaluate_call`. These resolve as METHODS even when the hash stores a
/// value under the same string key (matching the long-standing fast-path
/// precedence); any other name goes through member access, where a stored
/// callable value wins.
fn is_direct_hash_method(name: &str) -> bool {
    matches!(
        name,
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
    )
}

/// String method names eligible for the direct `s.method(args)` dispatch.
/// Names outside this list go through member access so user extensions
/// (`class_eval` on String) and universal members keep their behavior.
fn is_direct_string_method(name: &str) -> bool {
    matches!(
        name,
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
            | "slugify"
    )
}

/// Array method names eligible for the direct `arr.method(args)` dispatch —
/// the union of the mutating set, the borrowed tier, and everything
/// `call_array_method` handles. A name missing here is not a bug, just
/// slower: it takes the member-access route (ValueMethod boxing) and still
/// dispatches correctly.
fn is_direct_array_method(name: &str) -> bool {
    matches!(
        name,
        "push"
            | "pop"
            | "clear"
            | "concat"
            | "map"
            | "filter"
            | "select"
            | "each"
            | "each_with_index"
            | "index_of"
            | "reduce"
            | "fold"
            | "find"
            | "any?"
            | "all?"
            | "sort"
            | "sort_by"
            | "reverse"
            | "uniq"
            | "intersection"
            | "union"
            | "difference"
            | "compact"
            | "compact_blank"
            | "flatten"
            | "first"
            | "last"
            | "empty?"
            | "includes?"
            | "include?"
            | "contains"
            | "sample"
            | "shuffle"
            | "take"
            | "drop"
            | "slice"
            | "zip"
            | "sum"
            | "min"
            | "max"
            | "delete"
            | "delete_at"
            | "shift"
            | "unshift"
            | "insert"
            | "rotate"
            | "reject"
            | "none?"
            | "one?"
            | "values_at"
            | "count"
            | "get"
            | "dig"
            | "pluck"
            | "pick"
            | "length"
            | "len"
            | "size"
            | "to_string"
            | "to_json"
            | "join"
            | "all"
            | "includes"
            | "order"
            | "is_a?"
    )
}

impl Interpreter {
    /// Direct invocation of a user-defined instance method: same
    /// arity/default handling as `call_value`'s `Function` arm, then
    /// `call_function_with_this` binds the receiver and executes the
    /// method body in place — no bound-`Function` allocation and no deep
    /// clone of the body AST (the dominant cost of the old route).
    pub(crate) fn invoke_instance_method(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        method: &Rc<crate::interpreter::value::Function>,
        mut arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let required_arity = method.arity();
        let full_arity = method.full_arity();

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
            if let Some(default_expr) = method.param_default_value(arguments.len()) {
                let default_value = self.evaluate(default_expr)?;
                arguments.push(default_value);
            } else {
                return Err(RuntimeError::wrong_arity(full_arity, arguments.len(), span));
            }
        }

        self.call_function_with_this(method, Some(Value::Instance(inst.clone())), arguments)
    }

    /// Evaluate a function call expression.
    pub(crate) fn evaluate_call(
        &mut self,
        callee: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Value> {
        // Variable-callee interceptor: `respond_to(req, block)` runs through
        // a custom dispatcher that can invoke user lambdas (a NativeFunction
        // can't reach `&mut Interpreter`). Intentionally checked before
        // `evaluate_callee` so a local `respond_to` can't shadow the magic.
        if let ExprKind::Variable(name) = &callee.kind {
            if name == "respond_to" {
                if let Some(result) = self.try_evaluate_respond_to(arguments, span)? {
                    return Ok(result);
                }
            }
            // SEC-013b: `render_json(instance)` auto-dispatches through a
            // user-defined `def as_json` on the instance's class. Without
            // this interceptor, the native `render_json` closure has no
            // interpreter handle and can't call user methods, so the
            // override only worked for `render_json(user.as_json())`.
            if name == "render_json" {
                if let Some(result) = self.try_evaluate_render_json_with_as_json(arguments, span)? {
                    return Ok(result);
                }
            }
        }

        // Unified Member/SafeMember call dispatch.
        //
        // The receiver expression is evaluated EXACTLY ONCE here, then every
        // interceptor and fast path works on the evaluated value. The old
        // design (separate model / hash / string interceptors that each
        // evaluated the object and returned None on a type mismatch) ran a
        // side-effectful receiver expression twice — `make().map(f)` called
        // `make()` two times.
        //
        // Direct dispatch by receiver type:
        // - Hash / String / Array with a known builtin method name → the
        //   `call_*_method_on_rc` / string method dispatchers, skipping the
        //   member-access route (which boxes a ValueMethod per call).
        // - Non-model Instance with a user-defined method → direct
        //   `invoke_instance_method` (binds `this` in the call env; no bound
        //   `Function`, no method-body AST clone).
        // - Model receivers first get the callback interceptors, then the
        //   regular member route.
        // Anything else falls through to member resolution on the
        // already-evaluated value, then the generic call path below.
        let callee_val = match &callee.kind {
            ExprKind::Member { object, name } | ExprKind::SafeMember { object, name }
                // Mirror `evaluate_member`'s template-lenient `@ivar`
                // special case: that path must stay on the slow route.
                if !(matches!(object.kind, ExprKind::This)
                    && crate::interpreter::executor::template_lenient_vars_enabled()
                    && self.environment.borrow().get("this").is_none()) =>
            {
                let safe_navigation = matches!(callee.kind, ExprKind::SafeMember { .. });
                let obj_val = self.evaluate(object)?;
                if safe_navigation && matches!(obj_val, Value::Null) {
                    return Ok(Value::Null);
                }

                // Model callback interceptors (Class.create/update, instance
                // save/update/delete chains). Cheap name filters up front;
                // they consume the evaluated receiver, never re-evaluate it.
                if !safe_navigation {
                    if let Some(result) =
                        self.try_run_model_before_save(&obj_val, name, arguments, span)?
                    {
                        return Ok(result);
                    }
                    if let Some(result) =
                        self.try_run_model_delete_callbacks(&obj_val, name, arguments, span)?
                    {
                        return Ok(result);
                    }
                    if let Some(result) =
                        self.try_run_model_persist_callbacks(&obj_val, name, arguments, span)?
                    {
                        return Ok(result);
                    }
                }

                let all_positional = arguments
                    .iter()
                    .all(|a| matches!(a, Argument::Positional(_)));

                if all_positional {
                    match &obj_val {
                        Value::Hash(hash) if is_direct_hash_method(name) => {
                            let hash = hash.clone();
                            let mut arg_values = Vec::with_capacity(arguments.len());
                            for arg in arguments {
                                if let Argument::Positional(expr) = arg {
                                    arg_values.push(self.evaluate(expr)?);
                                }
                            }
                            return self.call_hash_method_on_rc(&hash, name, arg_values, span);
                        }
                        Value::Array(arr) if is_direct_array_method(name) => {
                            let arr = arr.clone();
                            let mut arg_values = Vec::with_capacity(arguments.len());
                            for arg in arguments {
                                if let Argument::Positional(expr) = arg {
                                    arg_values.push(self.evaluate(expr)?);
                                }
                            }
                            return self.call_array_method_on_rc(&arr, name, arg_values, span);
                        }
                        Value::String(s) if is_direct_string_method(name) => {
                            let s = s.clone();
                            let mut arg_values = Vec::with_capacity(arguments.len());
                            for arg in arguments {
                                if let Argument::Positional(expr) = arg {
                                    arg_values.push(self.evaluate(expr)?);
                                }
                            }
                            if let Some(result) =
                                self.call_string_method_borrowed(&s, name, &arg_values, span)
                            {
                                return result;
                            }
                            return self.call_string_method(&s, name, arg_values, span);
                        }
                        Value::Instance(inst) if !is_universal_instance_member(name) => {
                            let direct_method = {
                                let inst_ref = inst.borrow();
                                if !inst_ref.class.is_model_subclass()
                                    && !inst_ref.fields.contains_key(name.as_str())
                                {
                                    inst_ref.class.find_method(name)
                                } else {
                                    None
                                }
                            };
                            if let Some(method) = direct_method {
                                let inst = inst.clone();
                                let mut arg_values = Vec::with_capacity(arguments.len());
                                for arg in arguments {
                                    if let Argument::Positional(expr) = arg {
                                        arg_values.push(self.evaluate(expr)?);
                                    }
                                }
                                return self
                                    .invoke_instance_method(&inst, &method, arg_values, span);
                            }
                        }
                        _ => {}
                    }
                }
                self.evaluate_member_on_value(obj_val, name, callee.span)?
            }
            _ => self.evaluate_callee(callee)?,
        };

        // Safe navigation: if &.method() and object was null, propagate null
        if matches!(callee.kind, ExprKind::SafeMember { .. }) && matches!(callee_val, Value::Null) {
            return Ok(Value::Null);
        }

        // `n.abs()` / `x.to_f()` — explicit empty parens on a member whose
        // access already evaluated to a plain value (primitive zero-arg
        // builtins return their result directly). Treat the parens form
        // like the bare form, Ruby-style, instead of "cannot call
        // non-function value". Only member callees: `5()` stays an error.
        if arguments.is_empty()
            && matches!(
                callee.kind,
                ExprKind::Member { .. } | ExprKind::SafeMember { .. }
            )
            && !callee_val.is_callable()
        {
            return Ok(callee_val);
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
        obj_val: &Value,
        method_name: &str,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::get_or_create_metadata;

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

        let class = match obj_val {
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
        let before_events: &[&str] = if method_name == "create" {
            &["before_save", "before_create"]
        } else {
            &["before_save", "before_update"]
        };
        // If neither method-name callbacks nor closure callbacks are
        // registered, fall through to normal dispatch (no-op interception).
        if callback_names.is_empty() && !has_closure_callbacks(&class.name, before_events) {
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
                instance.set(name.clone().to_string(), v.clone());
            }
        }
        let inst_rc = Rc::new(RefCell::new(instance));

        // SEC-086a: a `before_*` callback that returns `false` vetoes the
        // operation. Subsequent callbacks don't run, the native isn't
        // dispatched, and we hand back a Model.create-shaped instance with
        // `_errors` populated so callers see a uniform "persistence
        // failed" contract (same shape as a validation failure).
        if !self.run_model_callbacks(&class, &inst_rc, &callback_names, before_events, span)? {
            let kind = if method_name == "create" {
                "before_create / before_save"
            } else {
                "before_update / before_save"
            };
            set_callback_aborted_error(&inst_rc, kind);
            return Ok(Some(Value::Instance(inst_rc)));
        }

        // Copy the instance's fields back into a new hash — preserving any
        // extra fields callbacks added, and picking up the mutations.
        let inst_ref = inst_rc.borrow();
        let mut new_pairs = crate::interpreter::value::HashPairs::default();
        for (k, v) in &inst_ref.fields {
            new_pairs.insert(HashKey::String(k.clone().into()), v.clone());
        }
        drop(inst_ref);
        arg_values[data_index] = Value::Hash(Rc::new(RefCell::new(new_pairs)));

        // Dispatch to the class's native static method (Model.create / Model.update)
        // with the transformed data. Resolve the member on the already-evaluated
        // class so the native fn closure gets the class as `args[0]` like the
        // normal path — without re-evaluating the receiver expression.
        let callee_val =
            self.evaluate_member_on_value(Value::Class(class.clone()), method_name, span)?;
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
        let after_events: &[&str] = if method_name == "create" {
            &["after_create", "after_save"]
        } else {
            &["after_update", "after_save"]
        };
        if !after_names.is_empty() || has_closure_callbacks(&class.name, after_events) {
            if let Value::Hash(result_hash) = &result {
                let valid = result_hash
                    .borrow()
                    .get(&HashKey::String("valid".into()))
                    .cloned();
                let record = result_hash
                    .borrow()
                    .get(&HashKey::String("record".into()))
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
                        // Closure-based after callbacks.
                        for ev in after_events {
                            for closure in crate::interpreter::builtins::model::callbacks::closure_callbacks_for(&class.name, ev) {
                                let mut bound_env =
                                    Environment::with_enclosing(closure.closure.clone());
                                bound_env.define(
                                    "this".to_string(),
                                    Value::Instance(inst.clone()),
                                );
                                bound_env.define(
                                    "self".to_string(),
                                    Value::Instance(inst.clone()),
                                );
                                let bound = crate::interpreter::value::Function {
                                    name: closure.name.clone(),
                                    params: closure.params.clone(),
                                    body: closure.body.clone(),
                                    closure: Rc::new(RefCell::new(bound_env)),
                                    is_method: true,
                                    span: closure.span,
                                    source_path: closure.source_path.clone(),
                                    defining_superclass: None,
                                    return_type: closure.return_type.clone(),
                                    cached_env: RefCell::new(None),
                                    jit_cache: RefCell::new(None),
                                };
                                self.call_value(
                                    Value::Function(Rc::new(bound)),
                                    Vec::new(),
                                    span,
                                )?;
                            }
                        }
                    }
                }
            }
        }

        Ok(Some(result))
    }

    /// Intercept `record.delete()` for model instances so before_delete /
    /// after_delete callbacks can execute with access to the current
    /// interpreter. Native methods cannot invoke user-defined Soli methods on
    /// their own because they only receive evaluated values.
    fn try_run_model_delete_callbacks(
        &mut self,
        obj_val: &Value,
        method_name: &str,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::get_or_create_metadata;

        if method_name != "delete" {
            return Ok(None);
        }
        if !arguments.is_empty() {
            return Ok(None);
        }

        let instance = match obj_val {
            Value::Instance(inst) if inst.borrow().class.is_model_subclass() => inst.clone(),
            _ => return Ok(None),
        };
        let class = instance.borrow().class.clone();
        let metadata = get_or_create_metadata(&class.name);
        let before_names = metadata.callbacks.before_delete.clone();
        let after_names = metadata.callbacks.after_delete.clone();
        let before_events: &[&str] = &["before_delete"];
        let after_events: &[&str] = &["after_delete"];

        if before_names.is_empty()
            && after_names.is_empty()
            && !has_closure_callbacks(&class.name, before_events)
            && !has_closure_callbacks(&class.name, after_events)
        {
            return Ok(None);
        }

        // SEC-086a: a `before_delete` callback returning `false` vetoes
        // the deletion. Skip the native call and the after-callbacks; the
        // instance keeps its DB-side state. Surface `_errors` and return
        // `Bool(false)` so callers can branch on the result.
        if !self.run_model_callbacks(&class, &instance, &before_names, before_events, span)? {
            set_callback_aborted_error(&instance, "before_delete");
            return Ok(Some(Value::Bool(false)));
        }

        let callee_val =
            self.evaluate_member_on_value(Value::Instance(instance.clone()), method_name, span)?;
        let result = self.call_value(callee_val, Vec::new(), span)?;

        let failed = matches!(&result, Value::String(s) if s.starts_with("Error:"))
            || matches!(&result, Value::Bool(false));
        if !failed {
            self.run_model_callbacks(&class, &instance, &after_names, after_events, span)?;
        }

        Ok(Some(result))
    }

    /// SEC-086: intercept persistence-side instance mutators on Model
    /// instances so user-defined `before_*` / `after_*` callbacks fire
    /// the same way they do for `Model.create` / `Model.update` at the
    /// class level. Covers `instance.update`, `instance.save`,
    /// `instance.restore`, `instance.increment`, `instance.decrement`,
    /// and `instance.touch`. `instance.delete` keeps its dedicated
    /// `try_run_model_delete_callbacks` interceptor (above).
    ///
    /// Per-method callback set (Rails-style: `_save` callbacks fire on
    /// every persistence path, plus the matching specific event):
    ///
    /// - `update(attrs)` / `restore()` / `increment(...)` / `decrement(...)`
    ///   / `touch()` → before_save → before_update → DB write →
    ///   after_update → after_save.
    /// - `save([attrs])` branches on whether the instance has a `_key`
    ///   field: persisted instances run the update chain; brand-new
    ///   instances run the create chain (before_save → before_create →
    ///   DB write → after_create → after_save).
    ///
    /// Falls through to default dispatch (returns Ok(None)) when:
    /// - The callee shape doesn't match (not a Member access).
    /// - The method name isn't one of the six mutators above.
    /// - The receiver isn't a Model-subclass instance.
    /// - No callbacks are registered for any of the matched events
    ///   (cheap path — keeps the existing native dispatch fast).
    /// - Any argument is non-positional (named args / blocks fall
    ///   through to the regular dispatcher).
    fn try_run_model_persist_callbacks(
        &mut self,
        obj_val: &Value,
        method_name: &str,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::get_or_create_metadata;

        if !matches!(
            method_name,
            "update" | "save" | "restore" | "increment" | "decrement" | "touch"
        ) {
            return Ok(None);
        }
        // Named args / blocks: defer to default dispatch. We don't want to
        // half-run before-callbacks and then bail.
        if arguments
            .iter()
            .any(|a| !matches!(a, Argument::Positional(_)))
        {
            return Ok(None);
        }

        let instance = match obj_val {
            Value::Instance(inst) if inst.borrow().class.is_model_subclass() => inst.clone(),
            _ => return Ok(None),
        };
        let class = instance.borrow().class.clone();
        let metadata = get_or_create_metadata(&class.name);

        // For `save()`, the create-vs-update branch is decided at runtime
        // by the presence of `_key` on the instance — match the native
        // method's logic. Other mutators always run the update chain.
        let has_key = matches!(instance.borrow().get("_key"), Some(Value::String(_)));
        let (before_events, after_events) = match persist_events_for(method_name, has_key) {
            Some(pair) => pair,
            None => return Ok(None),
        };

        let collect_names = |evs: &[&str]| -> Vec<String> {
            evs.iter()
                .flat_map(|ev| match *ev {
                    "before_save" => metadata.callbacks.before_save.clone(),
                    "before_create" => metadata.callbacks.before_create.clone(),
                    "before_update" => metadata.callbacks.before_update.clone(),
                    "after_save" => metadata.callbacks.after_save.clone(),
                    "after_create" => metadata.callbacks.after_create.clone(),
                    "after_update" => metadata.callbacks.after_update.clone(),
                    _ => Vec::new(),
                })
                .collect()
        };
        let before_names = collect_names(before_events);
        let after_names = collect_names(after_events);

        // No callbacks registered for any matched event → fall through so
        // the native method runs at the usual cost.
        if before_names.is_empty()
            && after_names.is_empty()
            && !has_closure_callbacks(&class.name, before_events)
            && !has_closure_callbacks(&class.name, after_events)
        {
            return Ok(None);
        }

        // SEC-086a: a `before_*` callback returning `false` vetoes the
        // persistence. The native (which is what triggers `exec_update` /
        // `exec_insert`) does NOT run; the instance keeps its DB-side
        // state, picks up `_errors`, and we surface `Bool(false)` —
        // matching the existing failure signal used by
        // `instance.save` / `update` / `restore`.
        if !self.run_model_callbacks(&class, &instance, &before_names, before_events, span)? {
            let kind = if before_events.contains(&"before_create") {
                "before_create / before_save"
            } else {
                "before_update / before_save"
            };
            set_callback_aborted_error(&instance, kind);
            return Ok(Some(Value::Bool(false)));
        }

        // Evaluate the original arguments and dispatch the native method on
        // the already-evaluated receiver.
        let callee_val =
            self.evaluate_member_on_value(Value::Instance(instance.clone()), method_name, span)?;
        let mut arg_values = Vec::with_capacity(arguments.len());
        for arg in arguments {
            if let Argument::Positional(expr) = arg {
                arg_values.push(self.evaluate(expr)?);
            }
        }
        let result = self.call_value(callee_val, arg_values, span)?;

        // Native methods that report failure return `Bool(false)` (update,
        // save, restore on validation/DB error). `increment`/`decrement`/
        // `touch` either return the instance on success or propagate Err.
        // After-callbacks run only on success — Bool(false) suppresses them.
        let failed = matches!(&result, Value::Bool(false));
        if !failed {
            self.run_model_callbacks(&class, &instance, &after_names, after_events, span)?;
        }

        Ok(Some(result))
    }

    /// Run a list of model callbacks (method-name + closure form) with
    /// `this` bound to `instance`. SEC-086a: returns `Ok(false)` as soon
    /// as any callback returns `Value::Bool(false)`, signalling an abort.
    /// Subsequent callbacks in the chain are NOT run on abort — the first
    /// `false` is the veto. Returns `Ok(true)` if every callback either
    /// ran cleanly or returned a non-`Bool(false)` value (the historical
    /// "discard return value" behaviour for non-veto cases).
    fn run_model_callbacks(
        &mut self,
        class: &Rc<crate::interpreter::value::Class>,
        instance: &Rc<RefCell<Instance>>,
        callback_names: &[String],
        events: &[&str],
        span: Span,
    ) -> RuntimeResult<bool> {
        for cb_name in callback_names {
            let Some(method) = class.find_method(cb_name) else {
                continue;
            };
            let mut bound_env = Environment::with_enclosing(method.closure.clone());
            bound_env.define("this".to_string(), Value::Instance(instance.clone()));
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
            let result =
                self.call_value(Value::Function(Rc::new(bound_method)), Vec::new(), span)?;
            if matches!(result, Value::Bool(false)) {
                return Ok(false);
            }
        }

        for ev in events {
            for closure in crate::interpreter::builtins::model::callbacks::closure_callbacks_for(
                &class.name,
                ev,
            ) {
                let mut bound_env = Environment::with_enclosing(closure.closure.clone());
                bound_env.define("this".to_string(), Value::Instance(instance.clone()));
                bound_env.define("self".to_string(), Value::Instance(instance.clone()));
                let bound = crate::interpreter::value::Function {
                    name: closure.name.clone(),
                    params: closure.params.clone(),
                    body: closure.body.clone(),
                    closure: Rc::new(RefCell::new(bound_env)),
                    is_method: true,
                    span: closure.span,
                    source_path: closure.source_path.clone(),
                    defining_superclass: None,
                    return_type: closure.return_type.clone(),
                    cached_env: RefCell::new(None),
                    jit_cache: RefCell::new(None),
                };
                let result = self.call_value(Value::Function(Rc::new(bound)), Vec::new(), span)?;
                if matches!(result, Value::Bool(false)) {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    /// Implement `respond_to(req, block_or_hash)` — Ruby-style content negotiation.
    ///
    /// Two argument forms:
    /// - DSL:  `respond_to(req, fn(format) { format.html(...); ... })`
    /// - Hash: `respond_to(req, {"html": fn() ..., "json": fn() ...})`
    ///
    /// Returns `Ok(Some(response))` on success, `Ok(None)` if signature doesn't
    /// match (so the caller can fall through to normal dispatch).
    fn try_evaluate_respond_to(
        &mut self,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::respond_to::{
            detect_request_format, make_format_hash, not_acceptable_response, pick_handler,
            RESPOND_TO_BUILDER,
        };
        use crate::interpreter::value::HashKey;

        // Need exactly 2 positional args. Anything else: not our concern.
        if arguments.len() != 2 {
            return Ok(None);
        }
        if !arguments
            .iter()
            .all(|a| matches!(a, Argument::Positional(_)))
        {
            return Ok(None);
        }

        let mut arg_values: Vec<Value> = Vec::with_capacity(2);
        for arg in arguments {
            if let Argument::Positional(expr) = arg {
                arg_values.push(self.evaluate(expr)?);
            }
        }
        let req = arg_values.remove(0);
        let second = arg_values.remove(0);

        // Push a fresh registration vec; popped before we leave (even on error).
        RESPOND_TO_BUILDER.with(|stack| stack.borrow_mut().push(Vec::new()));

        let registrations_result: RuntimeResult<Vec<(String, Value)>> = match &second {
            Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_) => {
                // DSL form: invoke block with the format builder.
                let format_obj = make_format_hash();
                let call_result = self.call_value(second.clone(), vec![format_obj], span);
                let regs =
                    RESPOND_TO_BUILDER.with(|stack| stack.borrow_mut().pop().unwrap_or_default());
                call_result.map(|_| regs)
            }
            Value::Hash(h) => {
                // Hash form: read entries directly. Insertion order preserved (IndexMap).
                let mut regs = Vec::new();
                for (k, v) in h.borrow().iter() {
                    if let HashKey::String(name) = k {
                        if !matches!(
                            v,
                            Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
                        ) {
                            // Pop the builder we pushed before erroring.
                            RESPOND_TO_BUILDER.with(|stack| {
                                stack.borrow_mut().pop();
                            });
                            return Err(crate::error::RuntimeError::type_error(
                                format!(
                                    "respond_to hash entry '{}' must be a function (got {})",
                                    name,
                                    v.type_name()
                                ),
                                span,
                            ));
                        }
                        regs.push((name.to_string(), v.clone()));
                    }
                }
                RESPOND_TO_BUILDER.with(|stack| {
                    stack.borrow_mut().pop();
                });
                Ok(regs)
            }
            _ => {
                RESPOND_TO_BUILDER.with(|stack| {
                    stack.borrow_mut().pop();
                });
                return Err(crate::error::RuntimeError::type_error(
                    format!(
                        "respond_to expects a function or hash as its second argument (got {})",
                        second.type_name()
                    ),
                    span,
                ));
            }
        };

        let registrations = registrations_result?;
        let detected = detect_request_format(&req);
        match pick_handler(&detected, &registrations) {
            Some(handler) => {
                let result = self.call_value(handler, Vec::new(), span)?;
                Ok(Some(result))
            }
            None => Ok(Some(not_acceptable_response())),
        }
    }

    /// SEC-013b: when `render_json` is called with an `Instance` whose class
    /// declares its own `def as_json`, dispatch the user method first and
    /// forward the resulting Hash to `render_json` instead of the raw
    /// instance. This is the auto-dispatch the SEC-013a documented
    /// convention couldn't deliver — the native `render_json` closure has
    /// no `&mut Interpreter` handle, so this interception lives at the
    /// call-evaluation layer where one is in scope.
    ///
    /// Returns `Ok(None)` to fall through to the default builtin in three
    /// cases: no arguments, first arg isn't a Soli `Value::Instance`, or
    /// the instance's class doesn't define `as_json`. Existing
    /// `render_json(hash_or_array)` callers and `render_json(user)`
    /// callers on classes without an override are untouched.
    fn try_evaluate_render_json_with_as_json(
        &mut self,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        if arguments.is_empty() {
            return Ok(None);
        }
        // Only positional first arg — named/spread args fall through.
        let first_expr = match &arguments[0] {
            Argument::Positional(e) => e,
            _ => return Ok(None),
        };
        let data = self.evaluate(first_expr)?;
        let instance = match &data {
            Value::Instance(inst) => inst.clone(),
            _ => return Ok(None),
        };
        let inst_ref = instance.borrow();
        let as_json_method = match inst_ref.class.find_method("as_json") {
            Some(m) => m,
            None => return Ok(None),
        };
        drop(inst_ref);

        // Bind `this` to the instance and execute the user method.
        let mut bound_env = Environment::with_enclosing(as_json_method.closure.clone());
        bound_env.define("this".to_string(), Value::Instance(instance.clone()));
        let bound = crate::interpreter::value::Function {
            name: as_json_method.name.clone(),
            params: as_json_method.params.clone(),
            body: as_json_method.body.clone(),
            closure: Rc::new(RefCell::new(bound_env)),
            is_method: true,
            span: as_json_method.span,
            ..Default::default()
        };
        let serialised = self.call_value(Value::Function(Rc::new(bound)), Vec::new(), span)?;

        // Forward to the original `render_json` builtin with the override
        // result substituted for the first argument. Any further arguments
        // (status code, options) are evaluated and passed through unchanged.
        let render_json_val = self
            .environment
            .borrow()
            .get("render_json")
            .ok_or_else(|| RuntimeError::General {
                message: "render_json builtin missing from environment".to_string(),
                span,
            })?;
        let mut new_args = Vec::with_capacity(arguments.len());
        new_args.push(serialised);
        for arg in arguments.iter().skip(1) {
            if let Argument::Positional(e) = arg {
                new_args.push(self.evaluate(e)?);
            }
        }
        let result = self.call_value(render_json_val, new_args, span)?;
        Ok(Some(result))
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

                // Set current env so `defined()` can inspect the scope chain
                crate::interpreter::executor::variables::set_current_env(self.environment.clone());
                // Wrap the call in a flamegraph `Fn` span when the native
                // is on the request-path whitelist (render, redirect, …).
                // Otherwise no-op so cheap builtins (`len`, `str`, …) don't
                // flood the chart from inside iteration loops.
                let _native_span = crate::serve::span_log::maybe_instrument_native(&native.name);
                let result = (native.func)(all_args)
                    .map_err(|msg| RuntimeError::General { message: msg, span })?;
                drop(_native_span);
                crate::interpreter::executor::variables::clear_current_env();

                Ok(result)
            }

            Value::Class(class) => {
                // Class instantiation
                let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
                self.initialize_instance_fields(&class, &instance)?;

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
                crate::interpreter::executor::variables::set_current_env(self.environment.clone());
                // See call_value_with_named above — same whitelist gating.
                let _native_span = crate::serve::span_log::maybe_instrument_native(&native.name);
                let result = (native.func)(arguments)
                    .map_err(|msg| RuntimeError::General { message: msg, span })?;
                drop(_native_span);
                crate::interpreter::executor::variables::clear_current_env();

                Ok(result)
            }

            Value::Class(class) => {
                let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
                self.initialize_instance_fields(&class, &instance)?;

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

#[cfg(test)]
mod tests {
    use super::persist_events_for;

    // SEC-086 — `persist_events_for` regression coverage.

    #[test]
    fn persist_events_save_with_key_runs_update_chain() {
        // A persisted instance (has _key) running save() goes through the
        // update path: before_save → before_update → DB → after_update → after_save.
        let (before, after) = persist_events_for("save", true).unwrap();
        assert_eq!(before, &["before_save", "before_update"]);
        assert_eq!(after, &["after_update", "after_save"]);
    }

    #[test]
    fn persist_events_save_without_key_runs_create_chain() {
        // A brand-new instance (no _key) running save() goes through the
        // create path: before_save → before_create → DB → after_create → after_save.
        let (before, after) = persist_events_for("save", false).unwrap();
        assert_eq!(before, &["before_save", "before_create"]);
        assert_eq!(after, &["after_create", "after_save"]);
    }

    #[test]
    fn persist_events_update_runs_update_chain() {
        let (before, after) = persist_events_for("update", true).unwrap();
        assert_eq!(before, &["before_save", "before_update"]);
        assert_eq!(after, &["after_update", "after_save"]);
        // `update` ignores `has_key` — it's defined as an update-only path.
        let (before2, after2) = persist_events_for("update", false).unwrap();
        assert_eq!(before, before2);
        assert_eq!(after, after2);
    }

    #[test]
    fn persist_events_restore_increment_decrement_touch_run_update_chain() {
        // SEC-086: all four mutate an existing record's fields; they fire
        // the update + save callbacks (Rails-style, not bespoke
        // `after_increment` / `after_touch` events).
        for method in ["restore", "increment", "decrement", "touch"] {
            let (before, after) = persist_events_for(method, true).unwrap();
            assert_eq!(
                before,
                &["before_save", "before_update"],
                "{method}: before-events"
            );
            assert_eq!(
                after,
                &["after_update", "after_save"],
                "{method}: after-events"
            );
        }
    }

    #[test]
    fn persist_events_returns_none_for_non_persist_methods() {
        // The persist interceptor must not claim methods that have their
        // own dedicated handler (`delete`) or that aren't persist-related
        // at all (`find`, `where`, etc.). Returning `None` here lets the
        // call fall through to the default dispatcher.
        for method in [
            "delete", "find", "where", "create", "all", "reload", "errors",
        ] {
            assert!(
                persist_events_for(method, true).is_none(),
                "expected None for {method}"
            );
            assert!(
                persist_events_for(method, false).is_none(),
                "expected None for {method}"
            );
        }
    }
}
