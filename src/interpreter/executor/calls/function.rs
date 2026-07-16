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
            // `grouped(fn() { ... })` runs the block with a request-coalescing
            // batch active so DB reads inside are combined into one round-trip.
            // Like `respond_to`, the block must run with `&mut Interpreter`, so
            // it's dispatched here rather than through the native placeholder.
            if name == "grouped" {
                if let Some(result) = self.try_evaluate_grouped(arguments, span)? {
                    return Ok(result);
                }
            }
            if name == "with_transaction" {
                if let Some(result) = self.try_evaluate_with_transaction(arguments, span)? {
                    return Ok(result);
                }
            }
            // `event :name do … end` inside a `state_machine` block. Scoped to an
            // active builder so a stray `event(...)` elsewhere falls through to
            // the native placeholder (which raises a clear error). The block must
            // run with `&mut Interpreter`, hence the interceptor.
            if name == "event"
                && crate::interpreter::builtins::model::state_machine::builder_active()
            {
                if let Some(result) = self.try_evaluate_sm_event(arguments, span)? {
                    return Ok(result);
                }
            }
        }

        // Factory.create / create_with / create_list / insert need the
        // interpreter to invoke callable factory templates and persist via
        // Model.create — handled here before member dispatch.
        if let ExprKind::Member { object, name } = &callee.kind {
            if let ExprKind::Variable(prefix) = &object.kind {
                if prefix == "Factory"
                    && matches!(
                        name.as_str(),
                        "create" | "create_with" | "create_list" | "insert"
                    )
                {
                    if let Some(result) = self.try_evaluate_factory_call(name, arguments, span)? {
                        return Ok(result);
                    }
                }
            }
        }

        // Member-callee interceptor: `<ModelClass>.transaction(fn() { ... })`
        // (or a trailing `do … end` / `{ … }` block) runs the block inside a DB
        // transaction (begin → run → commit, or rollback + re-raise on throw).
        // A NativeFunction can't reach `&mut Interpreter` to invoke the block,
        // so — like `respond_to` — it's handled here. Keyed on a *block*
        // argument, so the string (AQL) and no-arg (handle) forms, and any
        // unrelated `.transaction` call, fall through to normal dispatch with no
        // double evaluation.
        if let ExprKind::Member { object, name } = &callee.kind {
            if name == "transaction" && arguments.len() == 1 {
                let block_expr = match &arguments[0] {
                    Argument::Block(e) => Some(e),
                    Argument::Positional(e) if matches!(e.kind, ExprKind::Lambda { .. }) => Some(e),
                    _ => None,
                };
                if let Some(block_expr) = block_expr {
                    if let Some(result) =
                        self.try_evaluate_model_transaction(object, block_expr, span)?
                    {
                        return Ok(result);
                    }
                    // Not a model class → fall through to normal dispatch.
                }
            }
        }

        // Controller static-block DSL is declarative. `this.layout(...)`,
        // `this.before_action(...)` and `this.after_action(...)` are parsed
        // out of the source into the controller registry — they are not
        // executed for effect. Inside a `static {}` block `this` is the class,
        // so when one of these is *called* on a `Value::Class` we treat it as
        // a no-op. Without this, `this.layout("x", only: [...])` would try to
        // invoke the string that `this.layout = "..."` assigned to the field
        // ("string is not callable"), and the filtered-hook call form would
        // spuriously invoke whatever function the field happens to hold.
        if let ExprKind::Member { object, name } | ExprKind::SafeMember { object, name } =
            &callee.kind
        {
            if matches!(object.kind, ExprKind::This)
                && matches!(name.as_str(), "layout" | "before_action" | "after_action")
            {
                if let Ok(Value::Class(_)) = self.evaluate(object) {
                    return Ok(Value::Null);
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
                        self.try_run_model_class_delete(&obj_val, name, arguments, span)?
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
                    // State machine event call: `order.pay()`, `order.pay!()`,
                    // `order.can_pay?()`, `order.paid?()`. Only fires for empty-arg
                    // calls on a model instance whose class has a machine and whose
                    // method name maps to an event/predicate (user methods win).
                    if arguments.is_empty() {
                        if let Value::Instance(inst) = &obj_val {
                            if let Some(result) = self.sm_dispatch_on_instance(inst, name, span)? {
                                return Ok(result);
                            }
                        }
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
                            // Prefer native instance methods (same order as
                            // instance_member_access: fields → native → user).
                            // Natives apply to models too (`save`, `update`, …);
                            // user-method direct call stays non-model only so
                            // relation/translated-field access keeps the slow path.
                            let direct_native = {
                                let inst_ref = inst.borrow();
                                if inst_ref.fields.contains_key(name.as_str()) {
                                    None
                                } else {
                                    inst_ref.class.find_native_method(name)
                                }
                            };
                            if let Some(native) = direct_native {
                                let inst = inst.clone();
                                let mut arg_values = Vec::with_capacity(arguments.len());
                                for arg in arguments {
                                    if let Argument::Positional(expr) = arg {
                                        arg_values.push(self.evaluate(expr)?);
                                    }
                                }
                                if let Some(expected) = native.arity {
                                    if arg_values.len() != expected {
                                        return Err(RuntimeError::wrong_arity(
                                            expected,
                                            arg_values.len(),
                                            span,
                                        ));
                                    }
                                }
                                let _native_span =
                                    crate::serve::span_log::maybe_instrument_native(&native.name);
                                let result = crate::interpreter::executor::access::member::call_native_instance_method(
                                    &inst, &native, &arg_values,
                                )
                                .map_err(|msg| RuntimeError::General {
                                    message: msg,
                                    span,
                                })?;
                                drop(_native_span);
                                return Ok(result);
                            }
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
    pub(crate) fn try_run_model_delete_callbacks(
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

        let has_dependents = super::cascade::class_declares_dependents(&class.name);

        if before_names.is_empty()
            && after_names.is_empty()
            && !has_closure_callbacks(&class.name, before_events)
            && !has_closure_callbacks(&class.name, after_events)
            && !has_dependents
        {
            return Ok(None);
        }

        // Cycle guard for `dependent:` cascades: a document already being
        // deleted higher up the chain is treated as handled.
        let mut _cascade_guard = None;
        if has_dependents {
            let (collection, key) = {
                let inst_ref = instance.borrow();
                let collection = crate::interpreter::builtins::model::class_name_to_collection(
                    &inst_ref.class.name,
                );
                let key = match inst_ref.get("_key") {
                    Some(Value::String(s)) => Some(s.to_string()),
                    _ => None,
                };
                (collection, key)
            };
            if let Some(key) = key {
                match super::cascade::enter_cascade(&collection, &key) {
                    Some(guard) => _cascade_guard = Some(guard),
                    None => return Ok(Some(Value::Bool(true))),
                }
            }
        }

        // SEC-086a: a `before_delete` callback returning `false` vetoes
        // the deletion. Skip the native call, the cascades, and the
        // after-callbacks; the instance keeps its DB-side state. Surface
        // `_errors` and return `Bool(false)` so callers can branch.
        if !self.run_model_callbacks(&class, &instance, &before_names, before_events, span)? {
            set_callback_aborted_error(&instance, "before_delete");
            return Ok(Some(Value::Bool(false)));
        }

        // `dependent:` cascades run on hard deletes only, after the veto
        // and before the owner row is removed (Rails ordering). A
        // soft-deleting owner keeps its children.
        if has_dependents && !crate::interpreter::builtins::model::is_soft_delete(&class.name) {
            self.run_dependent_cascades(&instance, span)?;
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

    /// Intercept the class form `Model.delete(id)` when the class declares
    /// `dependent:` relations: load the document and route through the full
    /// instance-delete flow so cascades (and, as a documented side effect,
    /// delete callbacks) run. Classes without dependents keep the plain
    /// native behavior.
    fn try_run_model_class_delete(
        &mut self,
        obj_val: &Value,
        method_name: &str,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::{
            class_name_to_collection, crud, get_model_class,
        };

        if method_name != "delete" || arguments.len() != 1 {
            return Ok(None);
        }
        let class = match obj_val {
            Value::Class(c) if c.is_model_subclass() => c.clone(),
            _ => return Ok(None),
        };
        if !super::cascade::class_declares_dependents(&class.name) {
            return Ok(None);
        }
        // From here on we own the call: arguments are evaluated exactly once.
        let id_val = match &arguments[0] {
            Argument::Positional(expr) => self.evaluate(expr)?,
            _ => return Ok(None),
        };
        let id = match &id_val {
            Value::String(s) => s.to_string(),
            other => {
                return Err(RuntimeError::new(
                    format!(
                        "Model.delete() expects string id, got {}",
                        other.type_name()
                    ),
                    span,
                ))
            }
        };

        let collection = class_name_to_collection(&class.name);
        let doc = match crud::exec_get(&collection, &id) {
            Ok(doc) => doc,
            // Mirror the native's miss behavior: an "Error: ..." string.
            Err(e) => return Ok(Some(Value::String(format!("Error: {}", e).into()))),
        };
        let model_class = get_model_class(&class.name).unwrap_or(class);
        let instance_value = crud::json_doc_to_instance(&model_class, &doc);
        let result = self.delete_model_instance(&instance_value, span)?;
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
    pub(crate) fn try_run_model_persist_callbacks(
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

    // ----- State machine DSL -------------------------------------------------

    /// Define a state machine declared in a model class body. Pushes a builder,
    /// runs the `do … end` block (its `initial`/`event`/`transition`/`guard`/
    /// `before_transition`/`after_transition` calls record onto it), then
    /// finalizes + registers it. Tears the builder down on any error so a failed
    /// declaration can't leak a frame onto the per-worker stack.
    pub(crate) fn define_state_machine(
        &mut self,
        class: Rc<crate::interpreter::value::Class>,
        field_expr: &Expr,
        block_expr: &Expr,
        span: Span,
    ) -> RuntimeResult<()> {
        use crate::interpreter::builtins::model::state_machine as sm;

        let field = match self.evaluate(field_expr)? {
            Value::Symbol(s) => s.to_string(),
            Value::String(s) => s.to_string(),
            other => {
                return Err(RuntimeError::type_error(
                    format!(
                        "state_machine expects a field name symbol, got {}",
                        other.type_name()
                    ),
                    span,
                ))
            }
        };
        let block = self.evaluate(block_expr)?;
        if !matches!(
            block,
            Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
        ) {
            return Err(RuntimeError::type_error(
                "state_machine expects a `do … end` block".to_string(),
                span,
            ));
        }

        sm::push_builder(field);
        if let Err(e) = self.call_value(block, Vec::new(), span) {
            sm::abort_builder();
            return Err(e);
        }
        sm::finalize(&class.name).map_err(|e| RuntimeError::General { message: e, span })
    }

    /// Handle `event :name do … end` inside a `state_machine` block: open an
    /// event frame, run the block (its `transition`/`guard` calls record onto
    /// it), then close it. Returns `Ok(None)` when the argument shape doesn't
    /// match so the call falls through to the native placeholder.
    fn try_evaluate_sm_event(
        &mut self,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::state_machine as sm;

        let block = arguments.iter().find_map(|a| match a {
            Argument::Block(e) => Some(e),
            Argument::Positional(e) if matches!(e.kind, ExprKind::Lambda { .. }) => Some(e),
            _ => None,
        });
        let name_expr = arguments.iter().find_map(|a| match a {
            Argument::Positional(e) if !matches!(e.kind, ExprKind::Lambda { .. }) => Some(e),
            _ => None,
        });
        let (Some(name_expr), Some(block_expr)) = (name_expr, block) else {
            return Ok(None);
        };

        let event_name = match self.evaluate(name_expr)? {
            Value::Symbol(s) => s.to_string(),
            Value::String(s) => s.to_string(),
            other => {
                return Err(RuntimeError::type_error(
                    format!("event expects a name symbol, got {}", other.type_name()),
                    span,
                ))
            }
        };
        let block = self.evaluate(block_expr)?;
        sm::begin_event(event_name).map_err(|e| RuntimeError::General { message: e, span })?;
        // On block error, leave the half-built event; the enclosing
        // define_state_machine aborts the whole builder. Surface the error.
        self.call_value(block, Vec::new(), span)?;
        sm::end_event().map_err(|e| RuntimeError::General { message: e, span })?;
        Ok(Some(Value::Null))
    }

    /// Map a zero-arg method name on a model instance to a state machine
    /// event / predicate and run it. Returns `Ok(None)` when the class has no
    /// machine, a real user method shadows the name, or the name matches no
    /// event/predicate (so normal dispatch proceeds).
    pub(crate) fn sm_dispatch_on_instance(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::state_machine as sm;

        let class = inst.borrow().class.clone();
        if !class.is_model_subclass() {
            return Ok(None);
        }
        let machines = sm::machines_for(&class.name);
        if machines.is_empty() {
            return Ok(None);
        }
        // A real user-defined method of the same name always wins.
        if class.find_method(name).is_some() {
            return Ok(None);
        }

        for machine in &machines {
            if let Some(stem) = name.strip_suffix('?') {
                // can_<event>?
                if let Some(event) = stem.strip_prefix("can_") {
                    if machine.event(event).is_some() {
                        let ok = self.sm_can(inst, machine, event, span)?;
                        return Ok(Some(Value::Bool(ok)));
                    }
                }
                // <state>? predicate
                if let Some(tag) = machine.states.iter().find(|t| sm::snake_case(t) == stem) {
                    let current = self.sm_current_tag(inst, machine);
                    return Ok(Some(Value::Bool(current.as_deref() == Some(tag.as_str()))));
                }
            }
            // Event mutator: `pay` (in-memory) or `pay!` (also persists).
            let (event_name, persist) = match name.strip_suffix('!') {
                Some(stem) => (stem, true),
                None => (name, false),
            };
            if machine.event(event_name).is_some() {
                let result = self.sm_fire_event(inst, machine, event_name, persist, span)?;
                return Ok(Some(result));
            }
        }
        Ok(None)
    }

    /// Current state tag of `inst` for `machine`: the `__variant` of the enum
    /// stored in the field, the bare stored tag string, or the machine's
    /// `initial` when the field is unset.
    fn sm_current_tag(
        &self,
        inst: &Rc<RefCell<Instance>>,
        machine: &crate::interpreter::builtins::model::state_machine::StateMachineDef,
    ) -> Option<String> {
        let b = inst.borrow();
        match b.fields.get(machine.field.as_str()) {
            Some(Value::Instance(e)) => e.borrow().fields.get("__variant").and_then(|v| match v {
                Value::String(s) => Some(s.to_string()),
                _ => None,
            }),
            Some(Value::String(s)) => Some(s.to_string()),
            _ => machine.initial.clone(),
        }
    }

    /// `can_<event>?` — legal from the current state and the guard (if any) passes.
    fn sm_can(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        machine: &crate::interpreter::builtins::model::state_machine::StateMachineDef,
        event: &str,
        span: Span,
    ) -> RuntimeResult<bool> {
        use crate::interpreter::builtins::model::state_machine as sm;
        let Some(current) = self.sm_current_tag(inst, machine) else {
            return Ok(false);
        };
        if machine.target_for(event, &current).is_none() {
            return Ok(false);
        }
        let class_name = inst.borrow().class.name.clone();
        if let Some(guard) = sm::lookup_guard(&class_name, event) {
            let v = self.run_sm_closure(inst, &guard, span)?;
            return Ok(!matches!(v, Value::Bool(false) | Value::Null));
        }
        Ok(true)
    }

    /// Fire `event`: check legality + guard, run before hooks, set the new
    /// state, optionally persist, run after hooks. Raises on illegal transition,
    /// guard failure, or a `before_transition` veto.
    fn sm_fire_event(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        machine: &crate::interpreter::builtins::model::state_machine::StateMachineDef,
        event: &str,
        persist: bool,
        span: Span,
    ) -> RuntimeResult<Value> {
        use crate::interpreter::builtins::model::state_machine as sm;

        let class_name = inst.borrow().class.name.clone();
        let current = self
            .sm_current_tag(inst, machine)
            .ok_or_else(|| RuntimeError::General {
                message: format!(
                    "{}: state field '{}' is unset and the machine has no initial state",
                    class_name, machine.field
                ),
                span,
            })?;
        let to_tag = match machine.target_for(event, &current) {
            Some(t) => t.to_string(),
            None => {
                return Err(RuntimeError::General {
                    message: format!(
                        "{}: cannot '{}' from state '{}'",
                        class_name, event, current
                    ),
                    span,
                })
            }
        };

        if let Some(guard) = sm::lookup_guard(&class_name, event) {
            let v = self.run_sm_closure(inst, &guard, span)?;
            if matches!(v, Value::Bool(false) | Value::Null) {
                return Err(RuntimeError::General {
                    message: format!("{}: guard for '{}' failed", class_name, event),
                    span,
                });
            }
        }

        for hook in sm::lookup_before(&class_name, &to_tag) {
            let r = self.run_sm_closure(inst, &hook, span)?;
            if matches!(r, Value::Bool(false)) {
                return Err(RuntimeError::General {
                    message: format!(
                        "{}: before_transition to '{}' vetoed '{}'",
                        class_name, to_tag, event
                    ),
                    span,
                });
            }
        }

        let new_value =
            sm::build_state_value(&class_name, &machine.field, &to_tag).ok_or_else(|| {
                RuntimeError::General {
                    message: format!(
                        "{}: state_machine field '{}' is not declared with enum_field",
                        class_name, machine.field
                    ),
                    span,
                }
            })?;
        inst.borrow_mut().set(machine.field.clone(), new_value);

        if persist {
            let callee =
                self.evaluate_member_on_value(Value::Instance(inst.clone()), "save", span)?;
            self.call_value(callee, Vec::new(), span)?;
        }

        for hook in sm::lookup_after(&class_name, &to_tag) {
            self.run_sm_closure(inst, &hook, span)?;
        }

        Ok(Value::Bool(true))
    }

    /// Invoke a guard/before/after closure with `this`/`self` bound to the
    /// instance (the same binding `run_model_callbacks` uses).
    fn run_sm_closure(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        closure: &Rc<crate::interpreter::value::Function>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let mut bound_env = Environment::with_enclosing(closure.closure.clone());
        bound_env.define("this".to_string(), Value::Instance(inst.clone()));
        bound_env.define("self".to_string(), Value::Instance(inst.clone()));
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
        self.call_value(Value::Function(Rc::new(bound)), Vec::new(), span)
    }

    /// Run the block form `<ModelClass>.transaction(fn() { ... })`: begin a DB
    /// transaction, execute the block, commit on success, or roll back and
    /// **re-raise the original error** if the block throws. The block's value is
    /// the call's value.
    ///
    /// Nested calls *join* the outer transaction: only the outermost
    /// `transaction` begins/commits/rolls back (SolidB has no savepoints). After
    /// a commit/rollback that itself errors, the thread-local tx state is force-
    /// cleared so a half-finished transaction can't leak into the next request
    /// on a reused worker thread.
    ///
    /// Called from `evaluate_call` only when the sole argument is a block (an
    /// `fn` literal or a trailing `do … end` / `{ … }`). Returns `Ok(None)` when
    /// the receiver isn't a model class so an unrelated `.transaction(block)`
    /// method falls through to normal dispatch; the AQL-string and no-arg handle
    /// forms never reach here.
    fn try_evaluate_model_transaction(
        &mut self,
        object: &Expr,
        block_expr: &Expr,
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::crud::{
            begin_transaction, clear_current_tx, commit_transaction, has_active_tx,
            rollback_transaction,
        };

        // Only a model class gets the transaction-block treatment; anything else
        // is some unrelated `.transaction` method → let normal dispatch handle it.
        let receiver = self.evaluate(object)?;
        match &receiver {
            Value::Class(class) if class.is_model_subclass() => {}
            _ => return Ok(None),
        }

        let block = self.evaluate(block_expr)?;
        if !matches!(
            block,
            Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
        ) {
            return Ok(None);
        }

        // Nested transaction → join the outer one; the outer scope settles it.
        if has_active_tx() {
            return self.call_value(block, Vec::new(), span).map(Some);
        }

        begin_transaction(None).map_err(|e| RuntimeError::General {
            message: format!("transaction: failed to begin: {}", e),
            span,
        })?;

        match self.call_value(block, Vec::new(), span) {
            Ok(value) => match commit_transaction() {
                Ok(()) => Ok(Some(value)),
                Err(e) => {
                    clear_current_tx();
                    Err(RuntimeError::General {
                        message: format!("transaction: commit failed: {}", e),
                        span,
                    })
                }
            },
            Err(err) => {
                // Roll back, then surface the block's original error (not the
                // rollback's). Force-clear in case rollback also failed.
                let _ = rollback_transaction();
                clear_current_tx();
                Err(err)
            }
        }
    }

    /// Implement `grouped(fn() { ... })` — run the block with a request-
    /// coalescing batch active. DB reads inside the block register themselves
    /// instead of firing, and are combined into a single round-trip when the
    /// batch flushes: at block end, or the first time one of the deferred
    /// results is read inside the block (auto-flush).
    ///
    /// Returns `Ok(None)` when the sole argument isn't a callable block, so the
    /// call falls through to the native `grouped` placeholder (which raises a
    /// clear usage error). A nested `grouped` joins the outer batch; only the
    /// outermost call begins/flushes it.
    fn try_evaluate_with_transaction(
        &mut self,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::crud::{
            begin_transaction, clear_current_tx, has_active_tx, rollback_transaction,
        };

        if arguments.len() != 1 {
            return Ok(None);
        }
        let block_expr = match &arguments[0] {
            Argument::Block(e) | Argument::Positional(e) => e,
            _ => return Ok(None),
        };
        let block = self.evaluate(block_expr)?;
        if !matches!(
            block,
            Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
        ) {
            return Ok(None);
        }

        if has_active_tx() {
            return self.call_value(block, Vec::new(), span).map(Some);
        }

        begin_transaction(None).map_err(|e| RuntimeError::General {
            message: format!("with_transaction: failed to begin: {}", e),
            span,
        })?;

        match self.call_value(block, Vec::new(), span) {
            Ok(value) => {
                if let Err(e) = rollback_transaction() {
                    clear_current_tx();
                    return Err(RuntimeError::General {
                        message: format!("with_transaction: rollback failed: {}", e),
                        span,
                    });
                }
                Ok(Some(value))
            }
            Err(err) => {
                let _ = rollback_transaction();
                clear_current_tx();
                Err(err)
            }
        }
    }

    fn try_evaluate_factory_call(
        &mut self,
        method: &str,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::factories;

        let mut arg_values = Vec::new();
        for arg in arguments {
            match arg {
                Argument::Positional(expr) | Argument::Block(expr) => {
                    arg_values.push(self.evaluate(expr)?);
                }
                Argument::Named(_) => return Ok(None),
            }
        }

        let result = match method {
            "create" => {
                if arg_values.len() != 1 {
                    return Ok(None);
                }
                let name = factories::factory_name_from_value(&arg_values[0])?;
                factories::build(self, &name, None, span)
            }
            "create_with" => {
                if arg_values.len() != 2 {
                    return Ok(None);
                }
                let name = factories::factory_name_from_value(&arg_values[0])?;
                factories::build(self, &name, Some(&arg_values[1]), span)
            }
            "create_list" => {
                if arg_values.len() != 2 {
                    return Ok(None);
                }
                let name = factories::factory_name_from_value(&arg_values[0])?;
                let count = match &arg_values[1] {
                    Value::Int(n) if *n >= 0 => *n as usize,
                    other => {
                        return Err(RuntimeError::General {
                            message: format!(
                                "Factory.create_list expects count as non-negative integer, got {}",
                                other.type_name()
                            ),
                            span,
                        });
                    }
                };
                factories::build_list(self, &name, count, span)
            }
            "insert" => {
                let overrides = if arg_values.len() == 2 {
                    Some(&arg_values[1])
                } else if arg_values.len() == 1 {
                    None
                } else {
                    return Ok(None);
                };
                let name = factories::factory_name_from_value(&arg_values[0])?;
                factories::insert(self, &name, overrides, span)
            }
            _ => return Ok(None),
        };

        result.map(Some)
    }

    fn try_evaluate_grouped(
        &mut self,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Option<Value>> {
        use crate::interpreter::builtins::model::batch;

        if arguments.len() != 1 {
            return Ok(None);
        }
        let block_expr = match &arguments[0] {
            Argument::Block(e) | Argument::Positional(e) => e,
            _ => return Ok(None),
        };
        let block = self.evaluate(block_expr)?;
        if !matches!(
            block,
            Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
        ) {
            return Ok(None);
        }

        // Nested grouped → join the outer batch; the outer scope flushes it.
        if batch::is_active() {
            return self.call_value(block, Vec::new(), span).map(Some);
        }

        // In `--dev`, don't coalesce: run the block without a batch so each
        // read fires (and logs) as its own natural statement instead of one
        // combined `LET … RETURN […]` that's hard to read in the dev query
        // log. Coalescing stays on in production for the single-round-trip win.
        if crate::interpreter::builtins::template::is_dev_mode() {
            return self.call_value(block, Vec::new(), span).map(Some);
        }

        batch::begin();
        match self.call_value(block, Vec::new(), span) {
            Ok(value) => {
                // Flush whatever is still queued. Surface a flush error on the
                // success path so a failed combined query isn't swallowed.
                batch::end().map_err(|e| RuntimeError::General {
                    message: format!("grouped: failed to flush queries: {}", e),
                    span,
                })?;
                Ok(Some(value))
            }
            Err(err) => {
                // Block threw: tear the batch down and surface the block's error.
                let _ = batch::end();
                Err(err)
            }
        }
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

                // Ruby-style: trailing named arguments collapse into a single
                // trailing hash positional arg. This lets native DSL helpers
                // accept keyword-style calls (`transition from: X, to: Y`),
                // mirroring the class-body desugaring in `execute_class`. Keys
                // are accessed by name by the receiver, so HashMap order is fine.
                if !named_args.is_empty() {
                    let mut pairs = crate::interpreter::value::HashPairs::default();
                    for (key, value) in &named_args {
                        pairs.insert(HashKey::String(key.clone().into()), value.clone());
                    }
                    all_args.push(Value::Hash(Rc::new(RefCell::new(pairs))));
                }

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

                    self.execute_constructor_body(ctor, ctor_env);
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

                    self.execute_constructor_body(ctor, ctor_env);
                }

                Ok(Value::Null)
            }

            Value::Method(method) => {
                let mut args = positional_args;
                // Ruby-style: trailing named arguments collapse into a single
                // trailing hash positional arg, mirroring the NativeFunction
                // arm above. Built-in method receivers (QueryBuilder chaining,
                // `.time_bucket("1h", avg: "value")`, …) read the keys by name.
                if !named_args.is_empty() {
                    let mut pairs = crate::interpreter::value::HashPairs::default();
                    for (key, value) in &named_args {
                        pairs.insert(HashKey::String(key.clone().into()), value.clone());
                    }
                    args.push(Value::Hash(Rc::new(RefCell::new(pairs))));
                }
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

                    self.execute_constructor_body(ctor, ctor_env);
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
