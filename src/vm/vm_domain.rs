//! Value objects on the VM: construction, then freeze, then invariants — the
//! same protocol the tree-walker follows (`Interpreter::instantiate_class`),
//! built on `interpreter::domain`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::domain;
use crate::interpreter::value::{Class, HashKey, Instance, InstanceMethodDef, SoliStr, Value};
use crate::span::Span;

use super::vm::Vm;

#[cfg(test)]
thread_local! {
    /// Test-only: the nested runs (`run_to_completion`) started on this
    /// thread.
    static NESTED_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only: how many nested runs this thread has started, for a test to
/// tell a construction that runs in the current activation from one that
/// starts a run of its own.
#[cfg(test)]
pub(crate) fn nested_runs() -> usize {
    NESTED_RUNS.with(std::cell::Cell::get)
}

impl Vm {
    /// Run whatever `start` pushes (a frame, usually) until it returns, and
    /// give back its value. The caller has already put the callee slot on
    /// the stack; it is consumed.
    pub(crate) fn run_to_completion(
        &mut self,
        start: impl FnOnce(&mut Self) -> Result<(), RuntimeError>,
    ) -> Result<Value, RuntimeError> {
        #[cfg(test)]
        NESTED_RUNS.with(|runs| runs.set(runs.get() + 1));
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
            match class.find_vm_init_with_class() {
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
                    // from app files): compiled once for the class, and run
                    // as the class that declares it, for the `super(...)` in
                    // it.
                    Some((ctor, defining)) => {
                        let positional = if named.is_empty() {
                            positional
                        } else {
                            named_to_positional(&ctor, positional, named, span)?
                        };
                        let argc = positional.len();
                        let closure = self.compiled_constructor(&ctor, &defining, span)?;
                        self.push(instance_val.clone());
                        for value in positional {
                            self.push(value);
                        }
                        self.run_to_completion(|vm| {
                            vm.call_closure_in_class(closure, argc, span, Some(defining))
                        })?;
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
        let argc = self.stack.len() - 1 - receiver_idx;
        // `Klass.new(x, y)` is the constructor, exactly as `new Klass(x, y)`:
        // the class and its arguments are already where `call_class` wants
        // them. They were drained into a Vec and pushed back on every call.
        if !is_attribute_new(&self.stack[receiver_idx + 1..], false) {
            return self.call_class(class, argc, span);
        }
        // `Klass.new()`, `Klass.new({...})` on a plain class with a
        // constructor: the constructor too.
        let value_object = domain::value_objects_declared() && class.is_value_object();
        if !value_object && has_constructor(class) {
            return self.call_class(class, argc, span);
        }
        let args = self.stack.split_off(receiver_idx + 1);
        self.stack.truncate(receiver_idx);
        if value_object {
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
        // No constructor: the fields start as `new Klass()` starts them —
        // every default, `nil` without one — then take the attributes.
        check_attribute_args(class, &args).map_err(|e| RuntimeError::new(e, span))?;
        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
        if matches!(args.as_slice(), [] | [Value::Null]) {
            // No attributes to assign after the defaults: they run as frames
            // above the instance, the call's value, as for `new Klass()`.
            let defaults = self.start_fields(class, &instance, span)?;
            self.push(Value::Instance(instance.clone()));
            if let Some(plan) = defaults {
                self.push_field_defaults(&plan, class, &instance, span)?;
            }
            return Ok(());
        }
        self.init_fields(class, &instance, span)?;
        assign_attributes(&instance, &args);
        self.push(Value::Instance(instance));
        Ok(())
    }

    /// `Klass.new(...)` with labelled arguments, `[class, args…]` on the
    /// stack from `receiver_idx`: the rules of `Interpreter::class_new`. On
    /// a plain class without a constructor, named arguments alone are the
    /// attribute form — the fields started as `new Klass()` starts them, its
    /// defaults that are not literals included, then the attributes
    /// assigned. Anything else is the construction `Klass(k: v)` is. The VM
    /// read `Klass.new` as a member before, whose attribute form left those
    /// defaults nil.
    pub(crate) fn class_new_named(
        &mut self,
        class: &Rc<Class>,
        receiver_idx: usize,
        labels: &[Option<SoliStr>],
        span: Span,
    ) -> Result<(), RuntimeError> {
        use crate::interpreter::executor::access::member::{
            assign_attributes, check_attribute_args, has_constructor,
        };
        let block_label = |label: &Option<SoliStr>| {
            label
                .as_ref()
                .is_some_and(|name| name.as_str() == super::chunk::BLOCK_LABEL)
        };
        if labels.iter().any(block_label) {
            return Err(RuntimeError::new(
                format!("{}.new takes no block", class.name),
                span,
            ));
        }
        let attribute_form = labels.iter().all(Option::is_some)
            && !(domain::value_objects_declared() && class.is_value_object())
            && !has_constructor(class);
        if !attribute_form {
            return self.call_value_named(labels.len(), labels, span);
        }
        let values = self.stack.split_off(receiver_idx + 1);
        self.stack.truncate(receiver_idx);
        let mut pairs = crate::interpreter::value::HashPairs::default();
        for (label, value) in labels.iter().zip(values) {
            if let Some(name) = label {
                pairs.insert(HashKey::String(name.clone()), value);
            }
        }
        let args = [Value::Hash(Rc::new(RefCell::new(pairs)))];
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
        let params: Option<Vec<String>> = match class.find_vm_init_with_class() {
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
            // Level by level, as instance dispatch: a subclass's override
            // beats an ancestor's compiled method.
            let nearest = class.find_instance_method_level(&invariant.method);
            let held = if let Some((InstanceMethodDef::Compiled(method), defining)) = nearest {
                self.push(Value::Instance(instance.clone()));
                self.run_to_completion(|vm| {
                    vm.call_closure_in_class(method, 0, span, Some(defining))
                })?
            } else if let Some((InstanceMethodDef::Source(method), defining)) = nearest {
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
