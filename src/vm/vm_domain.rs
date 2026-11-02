//! Value objects on the VM: construction, then freeze, then invariants — the
//! same protocol the tree-walker follows (`Interpreter::instantiate_class`),
//! built on `interpreter::domain`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::domain;
use crate::interpreter::value::{Class, Instance, SoliStr, Value};
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Run whatever `start` pushes (a frame, usually) until it returns, and
    /// give back its value. The caller has already put the callee slot on
    /// the stack; it is consumed.
    pub(crate) fn run_to_completion(
        &mut self,
        start: impl FnOnce(&mut Self) -> Result<(), RuntimeError>,
    ) -> Result<Value, RuntimeError> {
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.return_depth = frames_before;
        let result = match start(self) {
            Ok(()) if self.frames.len() == frames_before => Ok(self.pop()),
            Ok(()) => self.run(),
            Err(e) => Err(e),
        };
        self.return_depth = saved_depth;
        result
    }

    /// `call_class` for a value object: `[class, args…]` on the stack, the
    /// instance pushed in their place.
    pub(crate) fn construct_value_object(
        &mut self,
        class: &Rc<Class>,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let callee_idx = self.stack.len() - 1 - argc;
        let positional: Vec<Value> = self.stack.drain(callee_idx + 1..).collect();
        self.stack.truncate(callee_idx);
        self.build_value_object(class, positional, Vec::new(), span)
    }

    /// Build a value object of `class` from the arguments and push it: its
    /// fields started with their defaults, then the class's constructor — or
    /// the arguments bound to the declared fields when it has none — while
    /// the instance is writable, then the invariants once it is frozen.
    pub(crate) fn build_value_object(
        &mut self,
        class: &Rc<Class>,
        positional: Vec<Value>,
        named: Vec<(SoliStr, Value)>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
        let instance_val = Value::Instance(instance.clone());
        {
            let _constructing = domain::begin_construction(&instance);
            // A field left out takes its default (`Amount.new(500)` is in
            // EUR), and the constructor and invariants see it.
            self.init_fields(class, &instance, span)?;
            match class.find_vm_method_with_class("init") {
                Some((ctor, defining)) => {
                    let (slots, supplied) = super::vm_calls::bind_named_arguments(
                        &ctor.proto,
                        positional,
                        named,
                        span,
                    )?;
                    self.push(instance_val.clone());
                    self.run_to_completion(|vm| {
                        vm.call_closure_with_slots(ctor, slots, supplied, Some(defining))
                    })?;
                }
                None => match class.find_constructor_with_class() {
                    // A constructor the tree-walker defined (a class loaded
                    // from app files): compiled on the fly, and run as the
                    // class that declares it, for the `super(...)` in it.
                    Some((ctor, defining)) => {
                        let positional = if named.is_empty() {
                            positional
                        } else {
                            named_to_positional(&ctor, positional, named, span)?
                        };
                        let argc = positional.len();
                        self.push(instance_val.clone());
                        for value in positional {
                            self.push(value);
                        }
                        self.run_jit_method_to_completion(&ctor, argc, span, defining)?;
                    }
                    None => bind_to_fields(&instance, positional, named, span)?,
                },
            }
            domain::fill_declared_fields(&instance);
        }
        self.run_invariants(class, &instance, span)?;
        self.push(instance_val);
        Ok(())
    }

    /// `Klass.new(args…)` from `call_method_slow_path`, `[class, args…]` on
    /// the stack from `receiver_idx`: the rules of `Interpreter::class_new`.
    pub(crate) fn class_new(
        &mut self,
        class: &Rc<Class>,
        receiver_idx: usize,
    ) -> Result<(), RuntimeError> {
        use crate::interpreter::executor::access::member::{
            assign_attributes, check_attribute_args, has_constructor, is_attribute_new,
        };
        let span = self.current_span();
        let args: Vec<Value> = self.stack.drain(receiver_idx + 1..).collect();
        self.stack.truncate(receiver_idx);
        if !is_attribute_new(&args, false) {
            let argc = args.len();
            self.push(Value::Class(class.clone()));
            for value in args {
                self.push(value);
            }
            return self.call_class(class, argc, span);
        }
        if domain::value_objects_declared() && class.is_value_object() {
            let named = match args.first() {
                Some(hash @ Value::Hash(_)) => domain::changes_from(Some(hash), "new")
                    .map_err(|e| RuntimeError::new(e, span))?
                    .into_iter()
                    .map(|(k, v)| (SoliStr::from(k), v))
                    .collect(),
                _ => Vec::new(),
            };
            return self.build_value_object(class, Vec::new(), named, span);
        }
        // `Klass.new()`, `Klass.new(name: x)`, `Klass.new({...})` on a class
        // with a constructor: the constructor, exactly as `new Klass(...)`.
        if has_constructor(class) {
            let argc = args.len();
            self.push(Value::Class(class.clone()));
            for value in args {
                self.push(value);
            }
            return self.call_class(class, argc, span);
        }
        // No constructor: the fields start as `new Klass()` starts them —
        // every default, `nil` without one — then take the attributes.
        check_attribute_args(class, &args).map_err(|e| RuntimeError::new(e, span))?;
        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
        self.init_fields(class, &instance, span)?;
        assign_attributes(&instance, &args);
        self.push(Value::Instance(instance));
        Ok(())
    }

    /// `amount.with(cents: 600)`: the value object rebuilt through its
    /// constructor with those changes (see `domain::with_arguments`), pushed.
    pub(crate) fn value_object_with(
        &mut self,
        instance: &Rc<RefCell<Instance>>,
        changes: HashMap<String, Value>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let class = instance.borrow().class.clone();
        let params: Option<Vec<String>> = match class.find_vm_method_with_class("init") {
            Some((ctor, _)) => Some(ctor.proto.param_names.clone()),
            None => class
                .find_constructor()
                .map(|ctor| ctor.params.iter().map(|p| p.name.clone()).collect()),
        };
        let named = domain::with_arguments(&instance.borrow(), params.as_deref(), &changes)
            .map_err(|e| RuntimeError::new(e, span))?;
        let named = named
            .into_iter()
            .map(|(k, v)| (SoliStr::from(k), v))
            .collect();
        self.build_value_object(&class, Vec::new(), named, span)
    }

    /// Whether `value` is a value object without a `with` method of its own.
    pub(crate) fn is_value_object_for_with(value: &Value) -> Option<Rc<RefCell<Instance>>> {
        if !domain::value_objects_declared() {
            return None;
        }
        let Value::Instance(inst) = value else {
            return None;
        };
        let class = inst.borrow().class.clone();
        (class.is_value_object()
            && class.find_vm_method_with_class("with").is_none()
            && class.find_method("with").is_none())
        .then(|| inst.clone())
    }

    /// Run `instance`'s invariants, its ancestors' first: the first falsy one
    /// throws an `InvariantViolation` naming it.
    pub(crate) fn run_invariants(
        &mut self,
        class: &Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        for invariant in domain::invariants_of(class) {
            let held = if let Some((method, defining)) =
                class.find_vm_method_with_class(&invariant.method)
            {
                self.push(Value::Instance(instance.clone()));
                self.run_to_completion(|vm| {
                    vm.call_closure_in_class(method, 0, span, Some(defining))
                })?
            } else if let Some((method, defining)) = class.find_method_with_class(&invariant.method)
            {
                // An invariant is a private method of the class that declares
                // it; it runs as that class's.
                self.push(Value::Instance(instance.clone()));
                self.run_jit_method_to_completion(&method, 0, span, defining)?
            } else {
                continue;
            };
            if !held.is_truthy() {
                return Err(RuntimeError::Thrown {
                    value: domain::violation_for(class, &invariant),
                    span,
                });
            }
        }
        Ok(())
    }
}

/// Named arguments to a tree-walker constructor, in its parameter order —
/// the compiled form takes positional ones. A parameter left out before one
/// given cannot be expressed positionally: the handler runs on the
/// interpreter instead.
fn named_to_positional(
    ctor: &crate::interpreter::value::Function,
    mut positional: Vec<Value>,
    named: Vec<(SoliStr, Value)>,
    span: Span,
) -> Result<Vec<Value>, RuntimeError> {
    let mut named: HashMap<String, Value> =
        named.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    for param in ctor.params.iter().skip(positional.len()) {
        match named.remove(&param.name) {
            Some(value) => positional.push(value),
            None => break,
        }
    }
    if let Some(name) = named.keys().next() {
        return Err(RuntimeError::EngineFallback(
            format!("named argument `{name}` out of order for a compiled constructor"),
            span,
        ));
    }
    Ok(positional)
}

fn bind_to_fields(
    instance: &Rc<RefCell<Instance>>,
    positional: Vec<Value>,
    named: Vec<(SoliStr, Value)>,
    span: Span,
) -> Result<(), RuntimeError> {
    let named: HashMap<String, Value> =
        named.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    domain::bind_fields(instance, positional, &named).map_err(|e| RuntimeError::new(e, span))
}
