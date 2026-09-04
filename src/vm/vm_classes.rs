//! Class operations for the VM: property access, inheritance, instantiation.

use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value, ValueMethod};
use crate::span::Span;

use super::vm::Vm;

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
            Value::Instance(inst) => {
                let inst_ref = inst.borrow();
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
                    _ => {}
                }
                Err(RuntimeError::NoSuchProperty {
                    value_type: inst_ref.class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Class(class) => {
                // Static field access
                if let Some(val) = class.static_fields.borrow().get(name) {
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
                // Nested class
                if let Some(nested) = class.nested_classes.borrow().get(name) {
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
                // which returned a wrong value instead of the record.
                if class.is_model_subclass() && name.starts_with("find_by_") {
                    return Err(RuntimeError::EngineFallback(
                        format!("dynamic finder '{}'", name),
                        span,
                    ));
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
                Err(RuntimeError::NoSuchProperty {
                    value_type: class.name.clone(),
                    property: name.to_string(),
                    span,
                })
            }
            Value::Hash(hash) => {
                let hash = hash.borrow();
                let key = HashKey::String(name.to_string().into());
                if let Some(val) = hash.get(&key) {
                    Ok(val.clone())
                } else {
                    Ok(Value::method(ValueMethod {
                        receiver: Box::new(object.clone()),
                        method_name: name.to_string(),
                    }))
                }
            }
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
            // Query builders are the tree-walker's entirely: `where`, `limit`,
            // `first`, `order`, the aggregates and scope chaining all live in
            // `query_builder_member_access`, and executing them needs the
            // interpreter. The VM had no arm at all, so any `Model.where(...)`
            // chain inside a VM-compiled handler died with
            // "Cannot access property 'limit' on QueryBuilder" — a hard error,
            // not a demotion, because the catch-all below is not an
            // `EngineFallback`. Punt via the same route class reflection and
            // dynamic finders use; the handler demotes once and is then
            // blacklisted, so this costs one re-run rather than one per request.
            Value::QueryBuilder(_) => Err(RuntimeError::EngineFallback(
                format!("query builder member '{}'", name),
                span,
            )),
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
        // Compiled (VmClosure) instance methods: bare access auto-invokes
        // the zero-arg form with the receiver as `this`, mirroring the
        // tree-walker's auto-invoke of zero-arg class methods. Instance
        // fields shadow methods, matching op_get_property's lookup order.
        if let Value::Instance(inst) = object {
            let inst_ref = inst.borrow();
            let field_hit = inst_ref.fields.contains_key(name);
            if !field_hit {
                let lookup = {
                    let class = inst_ref.class.clone();
                    class.find_vm_method_with_class(name)
                };
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
            // Replace the class on top of the stack
            let top = self.stack.len() - 1;
            self.stack[top] = Value::Class(Rc::new(new_class));
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
