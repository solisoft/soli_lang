//! Class operations for the VM: property access, inheritance, instantiation.

use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value, ValueMethod};
use crate::span::Span;

use super::vm::Vm;

impl Vm {
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
        match object {
            Value::Instance(inst) => {
                let inst_ref = inst.borrow();
                // Check instance fields first
                if let Some(val) = inst_ref.fields.get(name) {
                    return Ok(val.clone());
                }
                // Check class methods
                if let Some(method) = inst_ref.class.find_method(name) {
                    return Ok(Value::Function(method));
                }
                // Check native methods — bind to the instance so the wrapper
                // prepends the receiver (e.g. `DateTime.year` reads `args[0]`
                // as the instance). Same binding the tree-walker performs.
                //
                // EXCEPT Model subclasses: lifecycle callbacks (`before_save`
                // etc.) only fire through the tree-walker's interceptors in
                // `executor/calls/function.rs`. If the VM ran `record.save()`
                // natively it would silently skip them — so leave model
                // instance natives unbound: the call errors and serve mode
                // falls back to the tree-walker, which fires the callbacks.
                // Remove this carve-out once callbacks run inside the natives
                // (Bug B in the vm-model-callback-gaps plan).
                if let Some(native) = inst_ref.class.find_native_method(name) {
                    if inst_ref.class.is_model_subclass() {
                        // Deliberate VM punt (see the comment above): an
                        // EngineFallback error bypasses try/rescue routing so
                        // a user-level catch can't swallow it — serve mode
                        // re-runs the handler on the tree-walker, where the
                        // lifecycle callbacks fire.
                        return Err(RuntimeError::EngineFallback(
                            format!("model instance method '{}'", name),
                            span,
                        ));
                    }
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
                            move |args: Vec<Value>| -> Result<Value, String> {
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
                // State machine events / predicates (`order.pay`, `order.paid?`,
                // `order.can_pay?`). The VM can't run guard/transition closures
                // (`op_get_property` is `&self`), so hand the request back to the
                // tree-walker, which owns the full machinery — same EngineFallback
                // route the model-instance-method carve-out above uses.
                if inst_ref.class.is_model_subclass()
                    && crate::interpreter::builtins::model::state_machine::is_sm_member(
                        &inst_ref.class.name,
                        name,
                    )
                {
                    return Err(RuntimeError::EngineFallback(
                        format!("state machine member '{}'", name),
                        span,
                    ));
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
                // Class-level `method_missing` is dispatched only by the
                // tree-walker (see executor/access/member.rs). When the class
                // (or a superclass) defines one, punt to the interpreter via
                // EngineFallback — serve mode re-runs the request there. This
                // is what makes `UserMailer.welcome(user)` work in production,
                // mirroring the model instance-method punt above.
                if class.find_static_method("method_missing").is_some() {
                    return Err(RuntimeError::EngineFallback(
                        format!("class method_missing for '{}'", name),
                        span,
                    ));
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
                    Ok(Value::Method(ValueMethod {
                        receiver: Box::new(object.clone()),
                        method_name: name.to_string(),
                    }))
                }
            }
            Value::Array(_) => {
                // Array methods like .length, .map, .filter, etc.
                Ok(Value::Method(ValueMethod {
                    receiver: Box::new(object.clone()),
                    method_name: name.to_string(),
                }))
            }
            Value::String(s) => {
                // String properties
                if name == "length" {
                    Ok(Value::Int(s.len() as i64))
                } else {
                    Ok(Value::Method(ValueMethod {
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
                _ => Ok(Value::Method(ValueMethod {
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
        // Compiled (VmClosure) instance methods: bare access auto-invokes
        // the zero-arg form with the receiver as `this`, mirroring the
        // tree-walker's auto-invoke of zero-arg class methods. Instance
        // fields shadow methods, matching op_get_property's lookup order.
        if let Value::Instance(inst) = object {
            let inst_ref = inst.borrow();
            if !inst_ref.fields.contains_key(name) {
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
                return (func.func)(Vec::new()).map_err(|msg| RuntimeError::new(msg, span));
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
                | Value::Decimal(_) => {
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
                inst.borrow_mut().fields.insert(name.to_string(), value);
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
                                .insert(name.to_string(), closure);
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
