//! Class operations for the VM: property access, inheritance, instantiation.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::{Class, HashKey, Instance, NativeFunction, Value, ValueMethod};
use crate::span::Span;

use super::vm::{CallFrame, Vm};

/// A frame whose slot 0 holds `this`: a method dispatched with its class, or
/// one compiled as a method (see `Vm::current_this`).
fn is_method_frame(frame: &CallFrame) -> bool {
    frame.class.is_some() || frame.closure.proto.is_method
}

#[cfg(test)]
thread_local! {
    /// Test-only: the frame scans [`Vm::current_this`] ran on this thread.
    static THIS_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only: how many frame scans [`Vm::current_this`] has run on this
/// thread, for a test to tell a privacy check that scanned from one that did
/// not.
#[cfg(test)]
fn this_scans() -> usize {
    THIS_SCANS.with(std::cell::Cell::get)
}

/// Members the tree-walker implements on a class value itself, rather than
/// looking them up in the class's own method tables.
///
/// Kept beside the `EngineFallback` that uses it so the two are read together;
/// the authoritative implementations are in
/// `interpreter::executor::access::member`'s `Value::Class` arm.
/// `Class.define_method(name, body)` for both engines.
///
/// A tree-walked body is an `Rc<Function>` and goes in `methods`; a compiled one
/// is a `VmClosure` and goes in `vm_methods`. Instance dispatch consults both, so
/// either lands as a real method. Primitive-tagged classes (Int, Float, …) route
/// to the per-type user-method overlay, matching the tree-walker — their dispatch
/// never consults `Class.methods`.
fn vm_define_method(class: Rc<Class>) -> Value {
    Value::NativeFunction(NativeFunction::new(
        "define_method",
        Some(2),
        move |args: &[Value]| -> Result<Value, String> {
            let method_name = match args.first() {
                Some(Value::String(s)) | Some(Value::Symbol(s)) => s.to_string(),
                _ => return Err("define_method expects method name as first argument".to_string()),
            };
            match args.get(1) {
                Some(Value::Function(f)) => {
                    if let Some(prim) = class.primitive {
                        crate::interpreter::executor::calls::user_methods::register_user_method(
                            prim,
                            method_name,
                            f.clone(),
                        );
                    } else {
                        class.methods.borrow_mut().insert(method_name, f.clone());
                    }
                }
                Some(Value::VmClosure(c)) => {
                    if class.primitive.is_some() {
                        return Err("define_method on a primitive class needs an interpreted \
                             function; run this file without --vm"
                            .to_string());
                    }
                    class.vm_methods.borrow_mut().insert(method_name, c.clone());
                }
                _ => return Err("define_method expects a function as second argument".to_string()),
            }
            class.invalidate_method_cache();
            Ok(Value::Null)
        },
    ))
}

/// `Class.alias_method(new_name, existing_name)` for both engines.
fn vm_alias_method(class: Rc<Class>) -> Value {
    Value::NativeFunction(NativeFunction::new(
        "alias_method",
        Some(2),
        move |args: &[Value]| -> Result<Value, String> {
            let name_of = |arg: Option<&Value>, which: &str| -> Result<String, String> {
                match arg {
                    Some(Value::String(s)) | Some(Value::Symbol(s)) => Ok(s.to_string()),
                    _ => Err(format!(
                        "alias_method expects {which} as a string or symbol"
                    )),
                }
            };
            let new_name = name_of(args.first(), "the new name")?;
            let old_name = name_of(args.get(1), "the existing name")?;
            // Resolve into owned values FIRST. A `borrow()` temporary inside an
            // `if let` scrutinee lives to the end of the statement, so borrowing
            // mutably in the body panics.
            let interpreted = class.methods.borrow().get(&old_name).cloned();
            let compiled = class.vm_methods.borrow().get(&old_name).cloned();
            match (interpreted, compiled) {
                (Some(f), _) => {
                    class.methods.borrow_mut().insert(new_name, f);
                }
                (None, Some(c)) => {
                    class.vm_methods.borrow_mut().insert(new_name, c);
                }
                (None, None) => {
                    return Err(format!(
                        "alias_method: {:?} has no method {old_name:?}",
                        class.name
                    ));
                }
            }
            class.invalidate_method_cache();
            Ok(Value::Null)
        },
    ))
}

pub(crate) fn is_class_reflection_member(name: &str) -> bool {
    matches!(
        name,
        "define_method"
            | "alias_method"
            | "class_eval"
            | "methods"
            | "respond_to?"
            | "send"
            | "inspect"
            | "to_s"
            | "to_string"
    )
}

impl Vm {
    /// Resolve a `Future` or `grouped` `Deferred` the way the tree-walker does
    /// on member access, so `` `echo hi`.stdout `` and a deferred query's
    /// fields work under the VM.
    pub(crate) fn force_lazy_receiver(object: &Value, span: Span) -> Result<Value, RuntimeError> {
        match object {
            Value::Future(future) => Value::Future(future.clone())
                .resolve()
                .map_err(|e| RuntimeError::new(e, span)),
            Value::Deferred(cell) => crate::interpreter::builtins::model::batch::force(cell)
                .map_err(|e| RuntimeError::General { message: e, span }),
            other => Ok(other.clone()),
        }
    }

    /// Model-instance member access through the tree-walker's resolver. The
    /// interpreter is built on an empty environment: resolving a member reads
    /// the model registry and the database, not program globals, so the
    /// builtin registration of `Interpreter::new` would be pure cost here.
    pub(crate) fn model_instance_member(
        inst: &Rc<RefCell<Instance>>,
        name: &str,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let mut interp = Interpreter::with_environment(Rc::new(RefCell::new(
            crate::interpreter::environment::Environment::new(),
        )));
        interp.instance_member_access(inst.clone(), name, span)
    }

    /// Whether `name` is a scope declared on the builder's model.
    fn is_query_builder_scope(object: &Value, name: &str) -> bool {
        let Value::QueryBuilder(qb) = object else {
            return false;
        };
        crate::interpreter::symbol::symbol_string(qb.borrow().class_name).is_some_and(
            |class_name| {
                crate::interpreter::builtins::model::scopes::lookup_scope(class_name, name)
                    .is_some()
            },
        )
    }

    /// Whether `name` is a scope declared on a model class (`Measure.t`,
    /// `Post.published`). The tree-walker's `class_member_access` checks the
    /// scope registry before the statics; the VM's class branch did not, so
    /// the read fell through to "Cannot access property" — and, under
    /// `soli serve`, to a 500 once the handler had written.
    pub(crate) fn is_model_class_scope(
        class: &crate::interpreter::value::Class,
        name: &str,
    ) -> bool {
        class.is_model_subclass()
            && crate::interpreter::builtins::model::scopes::lookup_scope(&class.name, name)
                .is_some()
    }

    /// A model scope bound to a fresh builder for the class, resolved by the
    /// tree-walker. A scope closure is user code — a tenant scope calls
    /// `Current` or an app helper — but its body runs bound to the builder in
    /// its own closure's environment (`bind_user_method_to_receiver`), which
    /// is where those names resolve. The interpreter here only looks the scope
    /// up and calls it, so it needs no copy of the VM's globals: copying them
    /// (`for_vm_fragment`) was most of a scope call's cost, ~40 µs of ~42.
    pub(crate) fn model_class_scope(
        &self,
        class_val: &Value,
        name: &str,
        span: Span,
    ) -> Result<(Interpreter, Value), RuntimeError> {
        let mut interp = Interpreter::for_bound_body();
        let scope = interp.evaluate_member_on_value(class_val.clone(), name, span)?;
        Ok((interp, scope))
    }

    /// An interpreter for query-builder work. It reads only the builder and the
    /// database; a scope's own body, which may call application helpers, runs
    /// in its closure's environment (see `model_class_scope`).
    fn query_builder_interpreter(&self) -> Interpreter {
        Interpreter::for_bound_body()
    }

    fn query_builder_property(
        &self,
        object: &Value,
        name: &str,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if matches!(name, "find_each" | "in_batches" | "find_in_batches") {
            // These run a block per batch; see `call_query_builder_method`.
            return Err(RuntimeError::EngineFallback(
                format!("query builder batch iteration '{}'", name),
                span,
            ));
        }
        let mut interp = self.query_builder_interpreter();
        let member = interp.query_builder_member_access(name, span, object.clone())?;
        interp.auto_invoke_member(member, span)
    }

    /// `builder.name(args)` on the VM. Methods that take a block (`each`,
    /// `map`, `filter`, …) materialize the rows and run on the VM's own array
    /// methods, so a compiled closure is called here, with its captured
    /// variables, rather than from a second VM. The rest — chaining,
    /// terminals, aggregates, writes, scopes — take plain data and run in the
    /// tree-walker's query-builder code.
    pub(crate) fn call_query_builder_method(
        &mut self,
        object: Value,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let Value::QueryBuilder(qb) = &object else {
            unreachable!("call_query_builder_method on a non-builder");
        };
        match name {
            // Batch iteration walks a keyset cursor and calls the block per
            // record or per batch from inside the tree-walker; a compiled block
            // cannot be run from there. The handler demotes.
            "find_each" | "in_batches" | "find_in_batches" => {
                return Err(RuntimeError::EngineFallback(
                    format!("query builder batch iteration '{}'", name),
                    span,
                ))
            }
            "each" | "map" | "filter" | "reduce" | "find" | "any?" | "all?" | "sort_by"
                if args.iter().any(Value::is_callable) =>
            {
                let rows = crate::interpreter::builtins::model::raise_if_error_value(
                    crate::interpreter::builtins::model::execute_query_builder(&qb.borrow())
                        .force_deferred(),
                )
                .map_err(|e| RuntimeError::new(e, span))?;
                if let Value::Array(arr) = &rows {
                    return self.vm_call_array_method(arr, name, &args, span);
                }
            }
            _ => {}
        }
        let is_scope = Self::is_query_builder_scope(&object, name);
        let mut interp = self.query_builder_interpreter();
        if is_scope {
            let scope = interp.query_builder_member_access(name, span, object.clone())?;
            return interp.call_value(scope, args, span);
        }
        interp.call_query_builder_method(qb.clone(), name, args, span)
    }

    /// Get a property from a value.
    pub fn op_get_property(
        &self,
        object: &Value,
        name: &str,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // User-defined methods on primitives win over builtins. Gated by a
        // single Relaxed atomic load: zero overhead when no user methods
        // have ever been registered.
        use crate::interpreter::executor::access::member::bind_user_method_to_receiver;
        use crate::interpreter::executor::calls::user_methods::{
            has_user_methods, lookup_user_method, PrimType,
        };
        let prim = match object {
            Value::Int(_) => Some(PrimType::Int),
            Value::Float(_) => Some(PrimType::Float),
            Value::Bool(_) => Some(PrimType::Bool),
            Value::Null => Some(PrimType::Null),
            Value::Decimal(_) => Some(PrimType::Decimal),
            Value::String(_) => Some(PrimType::String),
            Value::Array(_) => Some(PrimType::Array),
            Value::Hash(_) => Some(PrimType::Hash),
            Value::Symbol(_) => Some(PrimType::Symbol),
            _ => None,
        };
        if let Some(t) = prim {
            if has_user_methods(t) {
                if let Some(f) = lookup_user_method(t, name) {
                    return Ok(bind_user_method_to_receiver(object.clone(), f));
                }
            }
        }
        if matches!(object, Value::Future(_) | Value::Deferred(_)) {
            let resolved = Self::force_lazy_receiver(object, span)?;
            return self.op_get_property(&resolved, name, span);
        }
        match object {
            // A DateTime is a native value with no class, so it resolves
            // through the same helper the tree-walker uses — one definition of
            // what `d.year` means, for both engines.
            Value::DateTime(ts, use_utc) => {
                crate::interpreter::executor::Interpreter::datetime_member_access(
                    *ts, *use_utc, name, span,
                )
            }
            Value::Instance(inst)
                if name == "with" && Self::is_value_object_for_with(object).is_some() =>
            {
                // `amount.with(cents: 600)` compiles to this read plus a
                // named call; the marker lets `call_value_named` rebuild it.
                let _ = inst;
                Ok(Value::method(ValueMethod {
                    receiver: Box::new(object.clone()),
                    method_name: "with".to_string(),
                }))
            }
            Value::Instance(inst) => {
                let inst_ref = inst.borrow();
                // A model instance answers relations (`card.project`), preloads,
                // translated fields, uploader and HABTM helpers, state-machine
                // events and `nil` for an unset column. That logic lives once,
                // in the tree-walker's `instance_member_access`; the VM hands it
                // every model read it would not answer the same way itself.
                // Raising here instead was harmless while a VM error re-ran
                // the action on the interpreter — but not once the action has
                // written, when it answered 500 (`rec._errors` after `create`).
                if inst_ref.class.is_model_subclass()
                    && (!inst_ref.fields.contains_key(name)
                        || crate::interpreter::builtins::model::member_needs_model_resolution(
                            &inst_ref.class.name,
                            name,
                        ))
                    && inst_ref.class.find_method(name).is_none()
                {
                    drop(inst_ref);
                    return Self::model_instance_member(inst, name, span);
                }
                // Check instance fields first
                if let Some(val) = inst_ref.fields.get(name) {
                    // A field holding a `grouped {}` deferred query result is
                    // forced on read, so `@posts` / `this.posts` hands back
                    // materialised data. The tree-walker does the same (see
                    // `evaluate_member`'s `was_instance` branch in
                    // executor/access/member.rs) — without it the VM leaks a
                    // `Value::Deferred` into for-loops, indexing and natives.
                    if let Value::Deferred(cell) = val {
                        let cell = cell.clone();
                        drop(inst_ref);
                        return crate::interpreter::builtins::model::batch::force(&cell)
                            .map_err(|e| RuntimeError::General { message: e, span });
                    }
                    return Ok(val.clone());
                }
                // Check class methods
                if let Some(method) = inst_ref.class.find_method(name) {
                    return Ok(Value::Function(method));
                }
                // Check native methods — bind to the instance so the wrapper
                // prepends the receiver (e.g. `DateTime.year` reads `args[0]`
                // as the instance). Same binding the tree-walker performs.
                // Model persist/delete wrappers run on CallMethod (`save()`),
                // not on this bound-value path (`m = record.save`).
                if let Some(native) = inst_ref.class.find_native_method(name) {
                    let class_name = inst_ref.class.name.clone();
                    let native = native.clone();
                    drop(inst_ref);
                    return Ok(
                        crate::interpreter::executor::access::member::bind_native_method_to_instance(
                            inst, &class_name, name, &native,
                        ),
                    );
                }
                // Universal members — mirror the tree-walker's
                // instance_member_access.
                match name {
                    "class" => {
                        return Ok(Value::String(inst_ref.class.name.clone().into()));
                    }
                    "nil?" | "blank?" => return Ok(Value::Bool(false)),
                    "present?" => return Ok(Value::Bool(true)),
                    "is_a?" => {
                        let inst_clone = inst.clone();
                        return Ok(Value::NativeFunction(NativeFunction::new(
                            "is_a?",
                            Some(1),
                            move |args: &[Value]| -> Result<Value, String> {
                                let class_name = match args.first() {
                                    Some(Value::String(s)) => s.clone(),
                                    _ => return Err("is_a? expects a string argument".to_string()),
                                };
                                let inst_ref = inst_clone.borrow();
                                let mut current: Option<&Class> = Some(&inst_ref.class);
                                while let Some(c) = current {
                                    if c.name == class_name.as_ref() {
                                        return Ok(Value::Bool(true));
                                    }
                                    current = c.superclass.as_deref();
                                }
                                Ok(Value::Bool(false))
                            },
                        )));
                    }
                    // Reflection reads the class and the fields, never the
                    // interpreter's state: the tree-walker's definitions
                    // answer for both engines (they see the VM's methods too).
                    "inspect"
                    | "respond_to?"
                    | "methods"
                    | "instance_variables"
                    | "instance_variable_get"
                    | "instance_variable_set" => {
                        drop(inst_ref);
                        return Self::model_instance_member(inst, name, span);
                    }
                    _ => {}
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: inst_ref.class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Class(class) => {
                // A named scope on a model class comes first, as in the
                // tree-walker's `class_member_access`. A bare read runs the
                // scope (`Measure.t.where(...)`), under the same paren-free
                // rule the tree-walker applies to a member.
                if Self::is_model_class_scope(class, name) {
                    let (mut interp, scope) = self.model_class_scope(object, name, span)?;
                    return interp.auto_invoke_member(scope, span);
                }
                // Static field access — a nested class is one on the VM; a
                // `private class` of a module only from code inside it.
                if let Some(val) = class.static_fields.borrow().get(name) {
                    if matches!(val, Value::Class(_)) {
                        let namespace = self
                            .frames
                            .last()
                            .and_then(|f| f.closure.proto.namespace.clone());
                        crate::interpreter::domain::check_nested_access(
                            class,
                            name,
                            namespace.as_deref(),
                        )
                        .map_err(|e| RuntimeError::type_error(e, span))?;
                    }
                    return Ok(val.clone());
                }
                // Static method access (AST-interpreted)
                if let Some(method) = class.find_static_method(name) {
                    return Ok(Value::Function(method));
                }
                // Static method access (VM-compiled) — used when the call site
                // can't use the CallMethod fast path (a static method resolved
                // via GetProperty then Op::Call). Named-argument calls are
                // compiled as a fallback and run in the interpreter, so they
                // never reach this path.
                if let Some(closure) = class.find_vm_static_method(name) {
                    return Ok(Value::VmClosure(closure));
                }
                // Native static method — Model subclass statics expect the
                // class as args[0] (collection resolution); bind it like the
                // tree-walker does. Plain statics (DateTime.now) stay raw.
                if let Some(native) = class.find_native_static_method(name) {
                    if class.is_model_subclass() {
                        return Ok(
                            crate::interpreter::executor::access::member::bind_native_static_to_model_class(
                                object, name, &native,
                            ),
                        );
                    }
                    return Ok(Value::NativeFunction((*native).clone()));
                }
                // Nested class (as the tree-walker stores it)
                if let Some(nested) = class.nested_classes.borrow().get(name) {
                    let namespace = self
                        .frames
                        .last()
                        .and_then(|f| f.closure.proto.namespace.clone());
                    crate::interpreter::domain::check_nested_access(
                        class,
                        name,
                        namespace.as_deref(),
                    )
                    .map_err(|e| RuntimeError::type_error(e, span))?;
                    return Ok(Value::Class(nested.clone()));
                }
                // Class reflection (`define_method`, `class_eval`, `send`, …) is
                // implemented only in the tree-walker's member access, which owns
                // the primitive-overlay and method-cache bookkeeping. The VM used
                // to report "Cannot access property 'define_method'", so a script
                // that ran fine under `--dev` failed under `--vm`. Punt via the
                // same `EngineFallback` route the `method_missing` and
                // state-machine carve-outs above use.
                // `define_method` / `alias_method` are implemented here rather
                // than punted, because a bytecode body is a `VmClosure` and the
                // tree-walker's version only accepts a `Function` — and because
                // `EngineFallback` only re-runs in serve mode, so a `--vm` script
                // would have no fallback at all.
                if name == "define_method" {
                    return Ok(vm_define_method(class.clone()));
                }
                if name == "alias_method" {
                    return Ok(vm_alias_method(class.clone()));
                }
                // The rest of the reflection surface (`class_eval`, `send`,
                // `methods`, …) lives only in the tree-walker's member access,
                // which owns the primitive-overlay bookkeeping. Punt via the same
                // route the `method_missing` and state-machine carve-outs use.
                if is_class_reflection_member(name) {
                    return Err(RuntimeError::EngineFallback(
                        format!("class reflection member '{}'", name),
                        span,
                    ));
                }
                // Dynamic finders resolve BEFORE `method_missing`, matching
                // `class_member_access`. Resolving `method_missing` first sent
                // `User.find_by_email("x")` into the class's `method_missing`
                // on any model that defines one (the `Mailer` prelude shape),
                // which returned a wrong value instead of the record. The
                // finder is `find_by` for the fields in the name, so the
                // action stays on the VM.
                if let Some(finder) =
                    crate::interpreter::executor::access::member::dynamic_finder_value(class, name)
                {
                    return Ok(finder);
                }
                // Class-level `method_missing` is the LAST resort — after
                // statics, reflection and dynamic finders — exactly as the
                // tree-walker orders it.
                if let Some(bound) =
                    crate::interpreter::executor::access::member::bind_class_method_missing(
                        class, object, name,
                    )
                {
                    return Ok(bound);
                }
                // Whatever the VM does not resolve itself is asked of the
                // tree-walker's `class_member_access` — the one definition of
                // class members — before answering "no such property". The
                // case that shipped: `klass.new(attrs)` on a model class
                // reached through a variable (a generic `tenant_add(klass,
                // attrs)` helper) raised "Cannot access property 'new'", which
                // after a write was a 500 in production (2.6.2). The
                // interpreter answers the same NoSuchProperty for a member
                // neither engine knows.
                let mut interp = Interpreter::for_vm_fragment(&self.globals);
                interp.evaluate_member_on_value(object.clone(), name, span)
            }
            // The tree-walker's rule, by the same function
            // (`Interpreter::hash_member_value`): user methods, the universal
            // members (`nil?`, `blank?`, `present?`…), then a builtin method
            // name reads as the method and anything else as the key's value —
            // `null` when the key is absent. The VM used to answer a method for
            // every absent key, so `if rec._errors` was true on a hash without
            // `_errors` (2.6.2); then it skipped the universal members, so
            // `h.present?` was `null` (2.6.4).
            Value::Hash(hash) => Ok(Interpreter::hash_member_value(hash, name, object.clone())),
            Value::Array(_) => {
                // Array methods like .length, .map, .filter, etc.
                Ok(Value::method(ValueMethod {
                    receiver: Box::new(object.clone()),
                    method_name: name.to_string(),
                }))
            }
            Value::String(s) => {
                // String properties
                if name == "length" {
                    Ok(Value::Int(s.len() as i64))
                } else {
                    Ok(Value::method(ValueMethod {
                        receiver: Box::new(object.clone()),
                        method_name: name.to_string(),
                    }))
                }
            }
            // Primitive member access shares the tree-walker's tables:
            // zero-arg methods evaluate directly, with-args methods come
            // back as a ValueMethod (invoked via CallMethod or, when
            // zero-arg-callable like `round`/`to_s`, auto-invoked by
            // op_get_property_member), and unknown names error.
            Value::Int(n) => Interpreter::int_member_access(*n, name, span),
            Value::Float(n) => Interpreter::float_member_access(*n, name, span),
            Value::Bool(b) => Interpreter::bool_member_access(*b, name, span),
            Value::Null => Interpreter::null_member_access(name, span),
            Value::Decimal(d) => Interpreter::decimal_member_access(d, name, span),
            Value::Symbol(s) => match name {
                "to_s" | "to_string" => Ok(Value::String(s.clone())),
                "inspect" => Ok(Value::String(format!(":{}", s).into())),
                "class" => Ok(Value::String("symbol".into())),
                "nil?" => Ok(Value::Bool(false)),
                "blank?" => Ok(Value::Bool(false)),
                "present?" => Ok(Value::Bool(true)),
                _ => Ok(Value::method(ValueMethod {
                    receiver: Box::new(object.clone()),
                    method_name: name.to_string(),
                })),
            },
            Value::Super(superclass) => {
                // super.method() — look up method in superclass
                if let Some(method) = superclass.find_method(name) {
                    return Ok(Value::Function(method));
                }
                if let Some(native) = superclass.find_native_method(name) {
                    return Ok(Value::NativeFunction((*native).clone()));
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: format!("super({})", superclass.name),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Function(_) | Value::NativeFunction(_) => match name {
                "nil?" => Ok(Value::Bool(false)),
                "blank?" => Ok(Value::Bool(false)),
                "present?" => Ok(Value::Bool(true)),
                "class" => Ok(Value::String("Function".into())),
                "inspect" => Ok(Value::String("<function>".into())),
                _ => Err(RuntimeError::NoSuchProperty {
                    value_type: object.type_name().to_string(),
                    property: name.to_string(),
                    span,
                }),
            },
            // A bare member of a query builder (`Post.where(...).all`,
            // `.count`, a scope) resolves through the tree-walker's
            // `query_builder_member_access`, the single definition of what a
            // builder answers, then auto-invokes zero-argument members as the
            // tree-walker does. This used to be an `EngineFallback`: harmless
            // before a write, a 500 after one.
            Value::QueryBuilder(_) => self.query_builder_property(object, name, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: object.type_name().to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    /// Resolve a bare (no-parens) member access. Auto-invokes zero-arg builtin
    /// methods so `arr.empty?`, `s.blank?`, `h.keys`, `a.length` evaluate to
    /// their result — matching the tree-walking interpreter — instead of
    /// yielding an (always-truthy) bound-method value. `obj.method()` with parens
    /// goes through CallMethod and is unaffected.
    /// Is `name` a method of the instance `this` (compiled or not yet)?
    pub(crate) fn has_instance_method(this: &Value, name: &str) -> bool {
        match this {
            Value::Instance(inst) => {
                let class = inst.borrow().class.clone();
                class.find_vm_method_with_class(name).is_some() || class.find_method(name).is_some()
            }
            _ => false,
        }
    }

    /// The `this` of the method running now: the receiver slot of the nearest
    /// method frame — dispatched with its class, or compiled as a method (a
    /// served action enters through `call_method_bound`, with no class) —
    /// looking past the blocks it runs. From a plain function that scans the
    /// whole frame stack, so the privacy check asks only for a restricted name.
    fn current_this(&self) -> Option<&Value> {
        #[cfg(test)]
        THIS_SCANS.with(|scans| scans.set(scans.get() + 1));
        self.frames
            .iter()
            .rev()
            .find(|frame| is_method_frame(frame))
            .and_then(|frame| self.stack.get(frame.stack_base))
    }

    /// Is the running frame the one [`Self::current_this`] answers from? Then
    /// its slot 0 *is* `this`, and a member read on it is a member of self.
    #[inline]
    pub(crate) fn slot_zero_is_this(&self) -> bool {
        self.frames.last().is_some_and(is_method_frame)
    }

    /// Is `inst` the `this` of the running frame, that frame being a method
    /// frame? There [`Self::current_this`] would answer with its slot 0, so
    /// this is its answer without the scan. `false` from a block or a plain
    /// function: only the scan knows their `this`.
    #[inline]
    fn is_running_method_this(&self, inst: &Rc<RefCell<Instance>>) -> bool {
        match self.frames.last() {
            Some(frame) if is_method_frame(frame) => matches!(
                self.stack.get(frame.stack_base),
                Some(Value::Instance(this)) if Rc::ptr_eq(this, inst)
            ),
            _ => false,
        }
    }

    /// Refuse `receiver.name` for a restricted member the running code may not
    /// reach: a private one off `self`, a protected one from outside the
    /// declaring class and its subclasses. One relaxed load until some class
    /// declares one. After that the cheap answers come first, each the one
    /// the full rule gives:
    ///
    /// 1. A member of the running method's `this` (`@x = v`, `@helper()` in
    ///    the method's own body) passes on the running frame's slot 0, since
    ///    no restriction refuses `self` (`MethodAccess::allows`).
    /// 2. A name no class of the receiver's chain restricts passes on the
    ///    chain's memo (`Class::restricted_method`): no walk and no frame
    ///    scan, whatever the call depth.
    /// 3. Only for a restricted name does [`Self::current_this`] scan the
    ///    frames, which finds the `this` of a method past the blocks it runs.
    ///    A member of that `this` passes.
    /// 4. Otherwise the caller's class decides a protected member, and the
    ///    rest is refused.
    pub(crate) fn check_private_value(
        &self,
        receiver: &Value,
        name: &str,
    ) -> Result<(), RuntimeError> {
        if !crate::interpreter::value::restricted_methods_declared() {
            return Ok(());
        }
        let Value::Instance(inst) = receiver else {
            return Ok(());
        };
        if self.is_running_method_this(inst) {
            return Ok(());
        }
        let instance = inst.borrow();
        let Some((access, defining)) = instance.class.restricted_method(name) else {
            return Ok(());
        };
        let this = match self.current_this() {
            Some(Value::Instance(this)) => Some(this),
            _ => None,
        };
        if this.is_some_and(|this| Rc::ptr_eq(this, inst)) {
            return Ok(());
        }
        let caller = this.map(|this| this.borrow().class.clone());
        if access.allows(defining, false, caller.as_deref()) {
            return Ok(());
        }
        Err(RuntimeError::General {
            message: access.refusal(
                name,
                &instance.class.name,
                instance.fields.contains_key(name),
            ),
            span: self.current_span(),
        })
    }

    /// [`Self::check_private_value`] for the receiver sitting at `receiver_idx`.
    pub(crate) fn check_private_receiver(
        &self,
        receiver_idx: usize,
        name: &str,
    ) -> Result<(), RuntimeError> {
        self.check_private_value(&self.stack[receiver_idx], name)
    }

    /// The model persist/delete native a bare `record.<name>` would
    /// auto-invoke, when `name` is one: a model instance with no field of that
    /// name, and a native that runs with no arguments.
    fn bare_model_mutator(
        &self,
        inst: &Rc<RefCell<Instance>>,
        name: &str,
    ) -> Option<Rc<NativeFunction>> {
        if !crate::interpreter::executor::calls::function::is_model_instance_mutator(name) {
            return None;
        }
        let inst_ref = inst.borrow();
        if !inst_ref.class.is_model_subclass() || inst_ref.fields.contains_key(name) {
            return None;
        }
        // `def delete(force)` on the model: `post.delete` reads that method, as
        // the tree-walker does. Taking the native here deleted the row and
        // never ran the override.
        inst_ref
            .class
            .find_native_method_unshadowed(name)
            .filter(|native| native.is_auto_invocable || native.arity == Some(0))
    }

    pub fn op_get_property_member(
        &mut self,
        object: &Value,
        name: &str,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if matches!(object, Value::Future(_) | Value::Deferred(_)) {
            let resolved = Self::force_lazy_receiver(object, span)?;
            return self.op_get_property_member(&resolved, name, span);
        }
        // `title.upcase`, `items.first`: a built-in zero-arg method read bare
        // on a string or an array is called here. `op_get_property` would wrap
        // it in a `Method` — the name copied, the receiver cloned — only for
        // the auto-invoke below to unwrap it and make this same call. A user
        // method on the type outranks the built-in, so it keeps that path;
        // so does a string's `length`, which `op_get_property` answers itself.
        {
            use crate::interpreter::executor::calls::method_registry::is_zero_arg_method;
            use crate::interpreter::executor::calls::user_methods::{has_user_methods, PrimType};
            match object {
                Value::String(s)
                    if name != "length"
                        && !has_user_methods(PrimType::String)
                        && is_zero_arg_method(name, object) =>
                {
                    return self.vm_call_string_method(s, name, &[], span);
                }
                Value::Array(arr)
                    if !has_user_methods(PrimType::Array) && is_zero_arg_method(name, object) =>
                {
                    return self.vm_call_array_method(arr, name, &[], span);
                }
                _ => {}
            }
        }
        // Compiled (VmClosure) instance methods: bare access auto-invokes
        // the zero-arg form with the receiver as `this`, mirroring the
        // tree-walker's auto-invoke of zero-arg class methods. Instance
        // fields shadow methods, matching op_get_property's lookup order.
        if let Value::Instance(inst) = object {
            let inst_ref = inst.borrow();
            let field_hit = inst_ref.fields.contains_key(name);
            if !field_hit {
                let class = inst_ref.class.clone();
                drop(inst_ref);
                // A method that exists only as AST (inherited from a class the
                // VM never compiled) is compiled here too: the tree-walker
                // auto-invokes it (`@kpis = @_kpis`), and returning the unbound
                // `Function` left a function in the field instead.
                let lookup = match class.find_vm_method_with_class(name) {
                    Some(found) => Some(found),
                    None => self.compile_tree_walking_method(&class, name)?,
                };
                let inst_ref = inst.borrow();
                if let Some((closure, defining_class)) = lookup {
                    drop(inst_ref);
                    if closure.proto.arity == 0 {
                        self.push(object.clone());
                        let saved_depth = self.return_depth;
                        let frames_before = self.frames.len();
                        self.return_depth = frames_before;
                        let result = (|| -> Result<Value, RuntimeError> {
                            self.call_closure_in_class(closure, 0, span, Some(defining_class))?;
                            if self.frames.len() == frames_before {
                                Ok(self.pop())
                            } else {
                                self.run()
                            }
                        })();
                        self.return_depth = saved_depth;
                        return result;
                    }
                    if let Some(result) = self.try_sm_dispatch(inst, name, span)? {
                        return Ok(result);
                    }
                } else {
                    drop(inst_ref);
                    if let Some(result) = self.try_sm_dispatch(inst, name, span)? {
                        return Ok(result);
                    }
                }
            }
        }
        // A paren-less `record.save` / `.delete` / … runs the lifecycle
        // callbacks, veto and cascades exactly like `record.save()` does on
        // CallMethod. Auto-invoking the bound native below skipped them all.
        if let Value::Instance(inst) = object {
            if let Some(native) = self.bare_model_mutator(inst, name) {
                let receiver_idx = self.stack.len();
                self.push(object.clone());
                self.call_model_instance_native(inst.clone(), native, receiver_idx, 0, name, span)?;
                return Ok(self.pop());
            }
        }
        let val = self.op_get_property(object, name, span)?;
        // Native methods (DateTime/Duration/Model instance wrappers, static
        // class methods like `DateTime.now`): bare access auto-invokes
        // zero-arg / auto-invocable functions — mirroring the tree-walker's
        // try_auto_invoke Member-context rule. Bound instance wrappers
        // already carry their receiver in the closure.
        if let Value::NativeFunction(func) = &val {
            if func.is_auto_invocable || func.arity == Some(0) {
                return (func.func)(&[]).map_err(|msg| RuntimeError::new(msg, span));
            }
            return Ok(val);
        }
        // A zero-parameter static method read bare (`Assistant.history_messages`)
        // is called, as the tree-walker's member auto-invoke does for methods.
        // The class is its `this`, as in a call with parentheses.
        if let Value::Class(class) = object {
            match &val {
                Value::VmClosure(closure) if closure.proto.param_names.is_empty() => {
                    let closure = closure.clone();
                    self.push(object.clone());
                    return self.run_to_completion(|vm| {
                        vm.call_closure_in_class(closure, 0, span, Some(class.clone()))
                    });
                }
                Value::Function(func) if func.is_method && func.params.is_empty() => {
                    if func.binds_this {
                        return self.call_method_bound_args(func, object.clone(), &[], span);
                    }
                    return self.invoke_callable(val, &[], span);
                }
                _ => {}
            }
        }
        let invoke = match &val {
            Value::Method(m)
                if crate::interpreter::executor::calls::method_registry::is_zero_arg_method(
                    &m.method_name,
                    &m.receiver,
                ) =>
            {
                Some((m.method_name.clone(), (*m.receiver).clone()))
            }
            _ => None,
        };
        match invoke {
            Some((method_name, receiver)) => match &receiver {
                Value::Array(arr) => self.vm_call_array_method(arr, &method_name, &[], span),
                Value::String(s) => self.vm_call_string_method(s, &method_name, &[], span),
                Value::Hash(h) => self.vm_call_hash_method(h, &method_name, &[], span),
                Value::Int(_)
                | Value::Float(_)
                | Value::Bool(_)
                | Value::Null
                | Value::Decimal(_)
                | Value::DateTime(_, _) => {
                    self.vm_call_primitive_method(&receiver, &method_name, &[], span)
                }
                _ => Ok(val),
            },
            None => Ok(val),
        }
    }

    /// Set a property on a value.
    pub fn op_set_property(
        &self,
        object: &Value,
        name: &str,
        value: Value,
        span: Span,
    ) -> Result<(), RuntimeError> {
        match object {
            Value::Instance(inst) => {
                {
                    let inst_ref = inst.borrow();
                    crate::interpreter::domain::check_writable(inst, &inst_ref.class, name)
                        .map_err(|e| RuntimeError::type_error(e, span))?;
                }
                inst.borrow_mut().fields.insert(name.into(), value);
                Ok(())
            }
            Value::Class(class) => {
                class
                    .static_fields
                    .borrow_mut()
                    .insert(name.to_string(), value);
                Ok(())
            }
            Value::Hash(hash) => {
                hash.borrow_mut()
                    .insert(HashKey::String(name.to_string().into()), value);
                Ok(())
            }
            _ => Err(RuntimeError::type_error(
                format!("Cannot set property on {}", object.type_name()),
                span,
            )),
        }
    }

    /// Set up inheritance between subclass and superclass.
    pub fn op_inherit(
        &mut self,
        subclass_val: &Value,
        superclass_val: &Value,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let superclass = match superclass_val {
            Value::Class(c) => c.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    format!(
                        "Superclass must be a class, got {}",
                        superclass_val.type_name()
                    ),
                    span,
                ));
            }
        };

        // We need to reconstruct the subclass with the superclass set.
        // Since Class is in an Rc, we need to create a new one.
        // The class was just created by Op::Class, so we replace the top of stack.
        if let Value::Class(sub) = subclass_val {
            let mut new_class = Class::new(
                sub.name.clone(),
                Some(superclass.clone()),
                sub.methods.borrow().clone(),
                sub.static_methods.clone(),
                sub.native_static_methods.clone(),
                sub.native_methods.clone(),
                sub.static_fields.clone(),
                sub.fields.clone(),
                sub.constructor.clone(),
                sub.nested_classes.clone(),
            );
            // Preserve the shared bytecode-method maps across rebuilds.
            new_class.vm_methods = sub.vm_methods.clone();
            new_class.vm_static_methods = sub.vm_static_methods.clone();
            new_class.is_module = sub.is_module;
            new_class.included_modules = sub.included_modules.clone();
            new_class.mixin_static_methods = sub.mixin_static_methods.clone();
            new_class.declared_fields = sub.declared_fields.clone();
            crate::interpreter::domain::note_class(&new_class);
            // Replace the class on top of the stack
            let new_class = Rc::new(new_class);
            crate::interpreter::domain::register_class(&new_class);
            let top = self.stack.len() - 1;
            self.stack[top] = Value::Class(new_class);
            Ok(())
        } else {
            Err(RuntimeError::type_error(
                format!("Expected class, got {}", subclass_val.type_name()),
                span,
            ))
        }
    }

    /// Add a method to a class on top of the stack.
    pub fn op_add_method(
        &mut self,
        class_val: &Value,
        name: &str,
        method: Value,
        is_static: bool,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if let Value::Class(_class) = class_val {
            // Since Class is behind Rc, we need to reconstruct with the new method.
            // For mutability, we'll use a different approach: store VM methods separately.
            // For now, we use the approach of rebuilding the class.
            // This is fine since class setup only happens once at startup.

            // Get the current class from the stack
            let top = self.stack.len() - 1;
            if let Value::Class(current) = &self.stack[top] {
                let mut static_methods = current.static_methods.clone();

                match method {
                    Value::VmClosure(closure) => {
                        // Bytecode methods from compile_class_decl. The maps
                        // are shared `Rc`s, so no class rebuild is needed —
                        // and the constructor ("init") plus instance methods
                        // become dispatchable via find_vm_method.
                        if is_static {
                            current
                                .vm_static_methods
                                .borrow_mut()
                                .insert(name.to_string(), closure);
                        } else {
                            current
                                .vm_methods
                                .borrow_mut()
                                .insert(name.to_string(), closure.clone());
                            // Modules expose instance methods as module functions.
                            if current.is_module {
                                current
                                    .vm_static_methods
                                    .borrow_mut()
                                    .insert(name.to_string(), closure);
                            }
                        }
                        return Ok(());
                    }
                    Value::Function(func) => {
                        if is_static {
                            static_methods.insert(name.to_string(), func);
                        } else {
                            current.methods.borrow_mut().insert(name.to_string(), func);
                        }
                    }
                    _ => {}
                }

                let mut new_class = Class::new(
                    current.name.clone(),
                    current.superclass.clone(),
                    current.methods.borrow().clone(),
                    static_methods,
                    current.native_static_methods.clone(),
                    current.native_methods.clone(),
                    current.static_fields.clone(),
                    current.fields.clone(),
                    current.constructor.clone(),
                    current.nested_classes.clone(),
                );
                // Preserve the shared bytecode-method maps across rebuilds.
                new_class.vm_methods = current.vm_methods.clone();
                new_class.vm_static_methods = current.vm_static_methods.clone();
                new_class.is_module = current.is_module;
                new_class.included_modules = current.included_modules.clone();
                new_class.mixin_static_methods = current.mixin_static_methods.clone();
                new_class.included_hook_stmts = current.included_hook_stmts.clone();
                new_class.extended_hook_stmts = current.extended_hook_stmts.clone();
                new_class.concern_static_methods = current.concern_static_methods.clone();
                new_class.concern_method_names = current.concern_method_names.clone();
                self.stack[top] = Value::Class(Rc::new(new_class));
            }
            Ok(())
        } else {
            Err(RuntimeError::type_error(
                format!("Expected class, got {}", class_val.type_name()),
                span,
            ))
        }
    }

    pub fn op_include(
        &mut self,
        class_val: &Value,
        module_val: &Value,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let class = match class_val {
            Value::Class(c) => c,
            _ => {
                return Err(RuntimeError::type_error(
                    format!("include expected a class, got {}", class_val.type_name()),
                    span,
                ));
            }
        };
        let module = match module_val {
            Value::Class(c) => c,
            _ => {
                return Err(RuntimeError::type_error(
                    format!("include expected a module, got {}", module_val.type_name()),
                    span,
                ));
            }
        };
        // Hooks fire for every module that actually joined the class — the named
        // one plus its transitive includes, innermost first. See the matching
        // comment in the tree-walker's `apply_mixins`.
        let added = class
            .include_module_collecting(module)
            .map_err(|e| RuntimeError::new(e, span))?;
        let class = class.clone();
        for joined in &added {
            self.fire_mixin_hooks(&class, joined, true, span)?;
        }
        Ok(())
    }

    pub fn op_extend(
        &mut self,
        class_val: &Value,
        module_val: &Value,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let class = match class_val {
            Value::Class(c) => c,
            _ => {
                return Err(RuntimeError::type_error(
                    format!("extend expected a class, got {}", class_val.type_name()),
                    span,
                ));
            }
        };
        let module = match module_val {
            Value::Class(c) => c,
            _ => {
                return Err(RuntimeError::type_error(
                    format!("extend expected a module, got {}", module_val.type_name()),
                    span,
                ));
            }
        };
        class
            .extend_module(module)
            .map_err(|e| RuntimeError::new(e, span))?;
        self.fire_mixin_hooks(class, module, false, span)
    }

    fn fire_mixin_hooks(
        &mut self,
        class: &Rc<Class>,
        module: &Rc<Class>,
        including: bool,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let hooks = if including {
            module.included_hook_stmts.borrow().clone()
        } else {
            module.extended_hook_stmts.borrow().clone()
        };
        if !hooks.is_empty() {
            // Seeded from the VM's globals: a hook body is AST, so it runs in a
            // tree-walking interpreter, and a bare `Interpreter::new()` saw only
            // builtins — a hook calling an application helper died with
            // `Undefined variable` under `--vm` but worked under `--dev`.
            let mut interp = Interpreter::for_vm_fragment(&self.globals);
            for body in hooks {
                interp.execute_class_level_calls(class, &body)?;
            }
        }
        let hook_name = if including { "included" } else { "extended" };
        if let Some(closure) = module.find_vm_static_method(hook_name) {
            self.push(Value::VmClosure(closure));
            self.push(Value::Class(class.clone()));
            self.call_value(1, span)?;
        } else if let Some(func) = module.find_static_method(hook_name) {
            let mut interp = Interpreter::for_vm_fragment(&self.globals);
            interp.call_function(&func, vec![Value::Class(class.clone())])?;
        }
        Ok(())
    }

    /// Instantiate a class with constructor arguments.
    pub fn op_new(&mut self, argc: usize, span: Span) -> Result<(), RuntimeError> {
        let callee_idx = self.stack.len() - 1 - argc;
        let class_val = self.stack[callee_idx].clone();

        match class_val {
            // Same protocol as calling the class value directly — runs the
            // compiled "init" constructor (or JIT-compiles a tree-walking
            // one) with the instance bound as `this`.
            Value::Class(class) => self.call_class(&class, argc, span),
            _ => Err(RuntimeError::NotAClass(
                class_val.type_name().to_string(),
                span,
            )),
        }
    }
}

#[cfg(test)]
mod deferred_force_tests {
    //! A `grouped(fn() { @posts = Post.all })` block leaves a
    //! `Value::Deferred` behind. The tree-walker forces it on every read — bare
    //! variable, instance field, index, `for` subject — so the VM has to as
    //! well, or the documented pattern (`for post in @posts`) raises on the
    //! bytecode path and demotes the handler *after* the coalesced queries
    //! already ran. These use a pre-resolved cell, so no server is involved.
    use crate::interpreter::value::{Class, DeferredCell, Instance, Value};
    use crate::vm::compiler::Compiler;
    use crate::vm::Vm;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn resolved_deferred(items: Vec<Value>) -> Value {
        Value::Deferred(Rc::new(RefCell::new(DeferredCell {
            resolved: Some(Value::Array(Rc::new(RefCell::new(items)))),
        })))
    }

    fn instance_holding(field: &str, value: Value) -> Value {
        let class = Rc::new(Class {
            name: "Rec".to_string(),
            ..Default::default()
        });
        let inst = Rc::new(RefCell::new(Instance::new(class)));
        inst.borrow_mut().fields.insert(field.into(), value);
        Value::Instance(inst)
    }

    fn run(src: &str, binding: (&str, Value)) -> Value {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = Vm::new();
        vm.globals.insert(binding.0.to_string(), binding.1);
        vm.execute(&module.main).expect("vm");
        vm.globals.get("out").cloned().expect("out")
    }

    fn two_posts() -> Value {
        resolved_deferred(vec![Value::String("a".into()), Value::String("b".into())])
    }

    #[test]
    fn for_loop_forces_a_deferred_subject() {
        let out = run(
            "let out = \"\"\nfor p in posts { out = out + p }",
            ("posts", two_posts()),
        );
        assert_eq!(out, Value::String("ab".into()));
    }

    #[test]
    fn index_access_forces_a_deferred() {
        let out = run("let out = posts[1]", ("posts", two_posts()));
        assert_eq!(out, Value::String("b".into()));
    }

    #[test]
    fn instance_field_read_forces_a_deferred() {
        // `@posts` compiles to `this.posts`; a controller field holding a
        // deferred must hand back the materialised array, not the placeholder.
        let out = run(
            "let out = rec.posts.length()",
            ("rec", instance_holding("posts", two_posts())),
        );
        assert_eq!(out, Value::Int(2));
    }

    /// The property fast paths (`GetProperty` / `GetLocalProperty`) push a
    /// field value straight onto the stack without reaching
    /// `op_get_property`, so they need their own force. Putting the field into
    /// a container is the case that has no other force to fall back on —
    /// `render("posts/index", { "posts": @posts })` in real code.
    #[test]
    fn instance_field_deferred_forces_into_a_container() {
        let out = run(
            "let out = [rec.posts]",
            ("rec", instance_holding("posts", two_posts())),
        );
        let Value::Array(outer) = out else {
            panic!("expected an array, got {out:?}");
        };
        let inner = outer.borrow()[0].clone();
        assert!(
            matches!(&inner, Value::Array(a) if a.borrow().len() == 2),
            "the deferred must be materialised, not pushed as a placeholder: {inner:?}"
        );
    }

    #[test]
    fn instance_field_deferred_is_iterable() {
        let out = run(
            "let out = \"\"\nfor p in rec.posts { out = out + p }",
            ("rec", instance_holding("posts", two_posts())),
        );
        assert_eq!(out, Value::String("ab".into()));
    }
}

#[cfg(test)]
mod privacy_check_tests {
    //! Once a class declares a private member, the privacy check answers the
    //! common accesses without the class-chain walk or a frame scan: a member
    //! of the running method's `this` on that frame's slot 0, a name no class
    //! of the receiver's chain restricts on the chain's memo. Only a
    //! restricted name walks the chain and scans the frames for `this`, which
    //! is what every `@x` read, write and call used to pay. Refusals stay.
    use super::this_scans;
    use crate::interpreter::value::{restricted_lookups, Value};
    use crate::vm::compiler::Compiler;
    use crate::vm::Vm;

    /// What a run cost the check: `Class::restricted_method` calls past the
    /// flag, how many of them walked the chain, and `Vm::current_this` frame
    /// scans.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Cost {
        lookups: usize,
        walks: usize,
        scans: usize,
    }

    impl Cost {
        fn so_far() -> Self {
            let (lookups, walks) = restricted_lookups();
            Cost {
                lookups,
                walks,
                scans: this_scans(),
            }
        }

        fn since(self, start: Cost) -> Cost {
            Cost {
                lookups: self.lookups - start.lookups,
                walks: self.walks - start.walks,
                scans: self.scans - start.scans,
            }
        }
    }

    fn cost(lookups: usize, walks: usize, scans: usize) -> Cost {
        Cost {
            lookups,
            walks,
            scans,
        }
    }

    /// Run `src` with each `__NAME__` replaced by its value; return its
    /// `result` and `refused` globals, and what the run cost the check.
    fn run(src: &str, values: &[(&str, i64)]) -> (Value, Option<Value>, Cost) {
        let src = values.iter().fold(src.to_string(), |src, (name, value)| {
            src.replace(&format!("__{name}__"), &value.to_string())
        });
        let tokens = crate::lexer::Scanner::new(&src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = Vm::new();
        let start = Cost::so_far();
        vm.execute(&module.main).expect("vm");
        let spent = Cost::so_far().since(start);
        (
            vm.globals.get("result").cloned().expect("result"),
            vm.globals.get("refused").cloned(),
            spent,
        )
    }

    fn refused() -> Option<Value> {
        Some(Value::String("refused".into()))
    }

    const LEDGER: &str = r#"
class Ledger
  private total: Int

  new()
    @total = 0
  end

  def run(n)
    i = 0
    while i < n
      @total = @total + @_step(i) + _step(1)
      this.total = this.total + 1
      i = i + 1
    end
    @total
  end

  private

  def _step(i)
    i
  end
end

ledger = new Ledger()
result = ledger.run(__N__)
refused = ledger.total rescue "refused"
"#;

    /// `@x`, `@x = v`, `this.x`, `@m()` and a bare call in a method's own
    /// body never reach the class and never scan the frames, however many
    /// times the loop runs. The whole cost is the two accesses from outside:
    /// `ledger.run` (a lookup) and the refused `ledger.total` (a lookup, a
    /// walk and the scan that finds no `this`).
    #[test]
    fn members_of_self_skip_the_lookup() {
        let (short, refused_read, short_cost) = run(LEDGER, &[("N", 3)]);
        let (long, _, long_cost) = run(LEDGER, &[("N", 30)]);
        assert_eq!(short, Value::Int(9));
        assert_eq!(long, Value::Int(495));
        assert_eq!(refused_read, refused());
        assert_eq!(short_cost, cost(2, 1, 1));
        assert_eq!(long_cost, cost(2, 1, 1));
    }

    const TALLY: &str = r#"
class Tally
  private secret: Int
  count: Int

  new()
    @secret = 0
    @count = 0
  end

  def run(n)
    i = 0
    while i < n
      [1].each { |x| @count = @count + x }
      [1].each { |x| @secret = @secret + @_one() }
      i = i + 1
    end
    @count * 100 + @secret
  end

  private

  def _one
    1
  end
end

tally = new Tally()
result = tally.run(__N__)
refused = tally.secret rescue "refused"
"#;

    /// A block's frame is no method frame, so its `@x` asks the class. For
    /// `count`, which no class of the chain restricts, that is the memo
    /// alone: a lookup for the read and one for the write, no walk, no scan.
    /// The restricted `@secret` read and write and the `@_one()` call walk the
    /// chain and scan down to the method's frame, where they find `this` and
    /// pass. That is three lookups, three walks and three scans per
    /// iteration, which is what they cost before.
    #[test]
    fn a_block_scans_for_this_only_for_a_restricted_name() {
        let (short, refused_read, short_cost) = run(TALLY, &[("N", 3)]);
        let (long, _, long_cost) = run(TALLY, &[("N", 30)]);
        assert_eq!(short, Value::Int(303));
        assert_eq!(long, Value::Int(3030));
        assert_eq!(refused_read, refused());
        let per_iteration = cost(2 + 3, 3, 3);
        let outside = cost(2, 1, 1);
        for (n, spent) in [(3, short_cost), (30, long_cost)] {
            assert_eq!(
                spent,
                cost(
                    outside.lookups + n * per_iteration.lookups,
                    outside.walks + n * per_iteration.walks,
                    outside.scans + n * per_iteration.scans,
                ),
                "n = {n}"
            );
        }
    }

    const POSTS: &str = r##"
class Post
  title: String

  new(title: String)
    @title = title
  end

  private

  def _slug
    "slug"
  end
end

def retitle(post, n)
  i = 0
  while i < n
    post.title = "#{post.title}."
    i = i + 1
  end
  post.title
end

post = new Post("a")
result = retitle(post, __N__)
refused = post._slug() rescue "refused"
"##;

    /// A field of a class that has private members, read and written from
    /// outside: each access asks the class, which answers from its memo,
    /// with no walk and no frame scan. Two accesses per iteration and the
    /// final read, plus the refused `_slug` call, which alone walks the chain
    /// and scans.
    #[test]
    fn an_unrestricted_name_skips_the_walk() {
        let (short, refused_call, short_cost) = run(POSTS, &[("N", 3)]);
        let (long, _, long_cost) = run(POSTS, &[("N", 30)]);
        assert_eq!(short, Value::String("a...".into()));
        assert_eq!(long, Value::String(format!("a{}", ".".repeat(30)).into()));
        assert_eq!(refused_call, refused());
        assert_eq!(short_cost, cost(2 * 3 + 2, 1, 1));
        assert_eq!(long_cost, cost(2 * 30 + 2, 1, 1));
    }

    const NODES: &str = r#"
class Node
  v: Int

  new(v: Int)
    @v = v
  end

  private

  def _hidden
    0
  end
end

def reads(node, n)
  total = 0
  i = 0
  while i < n
    total = total + node.v
    [1].each { |x| node.v = node.v * x }
    i = i + 1
  end
  total
end

def at_depth(node, depth, n)
  return reads(node, n) if depth == 0

  at_depth(node, depth - 1, n)
end

node = new Node(1)
result = at_depth(node, __DEPTH__, __N__)
refused = node._hidden() rescue "refused"
"#;

    /// `node.v` read from a plain function at the bottom of a recursion, and
    /// read and written from a block it runs: no frame on the stack is a
    /// method frame, so a scan for `this` would walk every one of them. An
    /// unrestricted name never asks for it, so the scans stay at the one the
    /// refused `_hidden` call makes, whatever the depth and the loop length.
    #[test]
    fn an_unrestricted_name_scans_no_frames_at_any_depth() {
        let (shallow, refused_call, shallow_cost) = run(NODES, &[("DEPTH", 0), ("N", 3)]);
        let (deep, _, deep_cost) = run(NODES, &[("DEPTH", 40), ("N", 30)]);
        assert_eq!(shallow, Value::Int(3));
        assert_eq!(deep, Value::Int(30));
        assert_eq!(refused_call, refused());
        assert_eq!(shallow_cost, cost(3 * 3 + 1, 1, 1));
        assert_eq!(deep_cost, cost(3 * 30 + 1, 1, 1));
    }
}
