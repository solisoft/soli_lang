//! Class instantiation evaluation (new expression).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::expr::Argument;
use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{Instance, Value};
use crate::span::Span;

impl Interpreter {
    /// Evaluate class instantiation: new Class(arguments)
    pub(crate) fn evaluate_new(
        &mut self,
        class_expr: &Expr,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Value> {
        let class_val = self.evaluate(class_expr)?;

        let class = match class_val {
            Value::Class(c) => c,
            _ => {
                return Err(RuntimeError::type_error(
                    format!("expected class name, got {}", class_val.type_name()),
                    span,
                ));
            }
        };

        if class.is_module {
            return Err(RuntimeError::type_error(
                format!("cannot instantiate module {}", class.name),
                span,
            ));
        }

        let mut positional_args = Vec::new();
        let mut named_args = HashMap::new();
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    positional_args.push(self.evaluate(expr)?);
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
                Argument::Block(_) => {
                    return Err(RuntimeError::type_error(
                        "constructor does not support block arguments",
                        span,
                    ));
                }
            }
        }

        self.instantiate_class(&class, positional_args, named_args, span)
    }

    /// Create an instance of `class` and run its constructor — the one path
    /// behind `new Klass(...)`, `Klass(...)` and every other way the
    /// tree-walker instantiates a class, so they cannot disagree.
    ///
    /// Fields first (inherited ones included), then the constructor: the
    /// class's own or the nearest inherited one, else a native constructor
    /// (`__construct__`, e.g. `Error`'s message). Arguments bind positionally,
    /// then by name, then from defaults. With no constructor at all the
    /// arguments are ignored, as they always were.
    pub(crate) fn instantiate_class(
        &mut self,
        class: &Rc<crate::interpreter::value::Class>,
        positional_args: Vec<Value>,
        named_args: HashMap<String, Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if class.is_module {
            return Err(RuntimeError::type_error(
                format!("cannot instantiate module {}", class.name),
                span,
            ));
        }

        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
        self.initialize_instance_fields(class, &instance)?;
        if crate::interpreter::domain::value_objects_declared() && class.is_value_object() {
            return self.construct_value_object(class, instance, positional_args, named_args, span);
        }
        self.run_constructor(&instance, class, positional_args, named_args, span)?;
        Ok(Value::Instance(instance))
    }

    /// A value object: writable while its constructor runs — the class's, or
    /// the arguments bound to the declared fields when it has none — then
    /// frozen, then checked against its invariants.
    fn construct_value_object(
        &mut self,
        class: &Rc<crate::interpreter::value::Class>,
        instance: Rc<RefCell<Instance>>,
        positional_args: Vec<Value>,
        named_args: HashMap<String, Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        {
            let _constructing = crate::interpreter::domain::begin_construction(&instance);
            if class.find_constructor().is_some() {
                self.run_constructor(&instance, class, positional_args, named_args, span)?;
            } else {
                crate::interpreter::domain::bind_fields(&instance, positional_args, &named_args)
                    .map_err(|e| RuntimeError::new(e, span))?;
            }
            crate::interpreter::domain::fill_declared_fields(&instance);
        }
        self.check_invariants(&instance, span)?;
        Ok(Value::Instance(instance))
    }

    /// `Klass.new(...)` on a plain class or a value object. With positional
    /// arguments (other than one hash) it runs the constructor exactly like
    /// `new Klass(...)`. No arguments, one hash, or named arguments only keep
    /// the attribute form on a plain class without a constructor — its
    /// fields started as `new Klass()` starts them, then those attributes
    /// assigned — and build a value object from those fields, its
    /// constructor and invariants included.
    pub(crate) fn class_new(
        &mut self,
        class: &Rc<crate::interpreter::value::Class>,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Value> {
        let mut positional = Vec::new();
        let mut named = HashMap::new();
        for argument in arguments {
            match argument {
                Argument::Positional(expr) => positional.push(self.evaluate(expr)?),
                Argument::Named(arg) => {
                    let value = self.evaluate(&arg.value)?;
                    named.insert(arg.name.clone(), value);
                }
                Argument::Block(_) => {
                    return Err(RuntimeError::new(
                        format!("{}.new takes no block", class.name),
                        span,
                    ));
                }
            }
        }
        use crate::interpreter::executor::access::member::{
            assign_attributes, check_attribute_args, has_constructor, is_attribute_new,
        };
        if !is_attribute_new(&positional, !named.is_empty()) {
            return self.instantiate_class(class, positional, named, span);
        }
        if crate::interpreter::domain::value_objects_declared() && class.is_value_object() {
            if let Some(hash @ Value::Hash(_)) = positional.first() {
                for (name, value) in crate::interpreter::domain::changes_from(Some(hash), "new")
                    .map_err(|e| RuntimeError::new(e, span))?
                {
                    named.entry(name).or_insert(value);
                }
            }
            return self.instantiate_class(class, Vec::new(), named, span);
        }
        // `Klass.new()`, `Klass.new(name: x)`, `Klass.new({...})` on a class
        // with a constructor: the constructor, exactly as `new Klass(...)`.
        if has_constructor(class) {
            return self.instantiate_class(class, positional, named, span);
        }
        let args = if named.is_empty() {
            positional
        } else {
            let mut pairs = crate::interpreter::value::HashPairs::default();
            for (name, value) in named {
                pairs.insert(
                    crate::interpreter::value::HashKey::String(name.into()),
                    value,
                );
            }
            vec![Value::Hash(Rc::new(RefCell::new(pairs)))]
        };
        // The fields start as `new Klass()` starts them — every default,
        // `nil` without one — and then take the attributes.
        check_attribute_args(class, &args).map_err(|e| RuntimeError::new(e, span))?;
        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
        self.initialize_instance_fields(class, &instance)?;
        assign_attributes(&instance, &args);
        Ok(Value::Instance(instance))
    }

    /// `amount.with(cents: 600)` / `amount.with({"cents": 600})`: the value
    /// object rebuilt through its constructor with those changes.
    pub(crate) fn value_object_with(
        &mut self,
        instance: &Rc<RefCell<Instance>>,
        arguments: &[Argument],
        span: Span,
    ) -> RuntimeResult<Value> {
        let mut changes = HashMap::new();
        let mut positional = Vec::new();
        for argument in arguments {
            match argument {
                Argument::Positional(expr) => positional.push(self.evaluate(expr)?),
                Argument::Named(named) => {
                    let value = self.evaluate(&named.value)?;
                    changes.insert(named.name.clone(), value);
                }
                Argument::Block(_) => {
                    return Err(RuntimeError::new("with(...) takes no block", span));
                }
            }
        }
        match positional.as_slice() {
            [] => {}
            [single] => {
                for (name, value) in crate::interpreter::domain::changes_from(Some(single), "with")
                    .map_err(|e| RuntimeError::new(e, span))?
                {
                    changes.entry(name).or_insert(value);
                }
            }
            _ => {
                return Err(RuntimeError::new(
                    "with(...) takes the changes as named arguments or one hash",
                    span,
                ))
            }
        }
        let class = instance.borrow().class.clone();
        let params: Option<Vec<String>> = class
            .find_constructor()
            .map(|ctor| ctor.params.iter().map(|p| p.name.clone()).collect());
        let named = crate::interpreter::domain::with_arguments(
            &instance.borrow(),
            params.as_deref(),
            &changes,
        )
        .map_err(|e| RuntimeError::new(e, span))?;
        self.instantiate_class(&class, Vec::new(), named, span)
    }

    /// Run `instance`'s invariants, its ancestors' first: the first falsy one
    /// throws an `InvariantViolation` naming it.
    pub(crate) fn check_invariants(
        &mut self,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> RuntimeResult<()> {
        let class = instance.borrow().class.clone();
        for invariant in crate::interpreter::domain::invariants_of(&class) {
            let Some(method) = class.find_method(&invariant.method) else {
                continue;
            };
            let held = self.call_function_with_this(
                &method,
                Some(Value::Instance(instance.clone())),
                Vec::new(),
            )?;
            if !held.is_truthy() {
                return Err(RuntimeError::Thrown {
                    value: crate::interpreter::domain::violation_for(&class, &invariant),
                    span,
                });
            }
        }
        Ok(())
    }

    /// Run `class`'s constructor on `instance`: its own or the nearest
    /// inherited one, else a native constructor, else nothing. Shared by
    /// instantiation and `super(...)` in a subclass constructor.
    pub(crate) fn run_constructor(
        &mut self,
        instance: &Rc<RefCell<Instance>>,
        class: &crate::interpreter::value::Class,
        positional_args: Vec<Value>,
        named_args: HashMap<String, Value>,
        span: Span,
    ) -> RuntimeResult<()> {
        if let Some(ctor) = class.find_constructor() {
            let ctor_args =
                self.bind_constructor_args(&ctor, positional_args, &named_args, span)?;
            let mut ctor_env = Environment::with_enclosing(ctor.closure.clone());
            ctor_env.define("this".to_string(), Value::Instance(instance.clone()));
            for (param, value) in ctor.params.iter().zip(ctor_args) {
                ctor_env.define(param.name.clone(), value);
            }
            // Pushes a frame so coverage/stack traces attribute constructor
            // lines to the class's own file.
            self.execute_constructor_body(&ctor, ctor_env)?;
        } else if let Some(init) =
            class.find_native_method(crate::interpreter::value::NATIVE_CONSTRUCTOR)
        {
            if let Some(name) = named_args.keys().next() {
                return Err(RuntimeError::new(
                    format!("{}() takes no named argument '{}'", class.name, name),
                    span,
                ));
            }
            let mut init_args = Vec::with_capacity(positional_args.len() + 1);
            init_args.push(Value::Instance(instance.clone()));
            init_args.extend(positional_args);
            (init.func)(&init_args).map_err(|e| RuntimeError::new(e, span))?;
        }
        Ok(())
    }

    /// A constructor's argument list from positional and named arguments and
    /// the parameters' defaults.
    fn bind_constructor_args(
        &mut self,
        ctor: &crate::interpreter::value::Function,
        positional_args: Vec<Value>,
        named_args: &HashMap<String, Value>,
        span: Span,
    ) -> RuntimeResult<Vec<Value>> {
        let param_names: Vec<&str> = ctor.params.iter().map(|p| p.name.as_str()).collect();
        for name in named_args.keys() {
            if !param_names.contains(&name.as_str()) {
                return Err(RuntimeError::undefined_variable(name.clone(), span));
            }
        }
        if positional_args.len() > param_names.len() {
            return Err(RuntimeError::wrong_arity(
                ctor.full_arity(),
                positional_args.len() + named_args.len(),
                span,
            ));
        }
        let given = positional_args.len();
        let mut ctor_args = positional_args;
        let mut used_params: HashSet<&str> = param_names[..given].iter().copied().collect();
        for (i, param_name) in param_names.iter().enumerate() {
            if used_params.contains(param_name) {
                continue;
            }
            if let Some(named_val) = named_args.get(*param_name) {
                ctor_args.push(named_val.clone());
            } else if let Some(default_expr) = ctor.param_default_value(i) {
                let default_value = self.evaluate(default_expr)?;
                ctor_args.push(default_value);
            } else {
                return Err(RuntimeError::wrong_arity(
                    ctor.arity(),
                    ctor_args.len(),
                    span,
                ));
            }
            used_params.insert(param_name);
        }
        Ok(ctor_args)
    }

    /// Start every declared instance field of `class` and its ancestors on
    /// `instance`, the farthest class first — the one path behind every way
    /// of building an instance (`new Klass(...)`, `Klass(...)`,
    /// `Klass.new(...)`, the attribute form). Each class starts its fields —
    /// a literal default, else `nil` — then evaluates its defaults that are
    /// not literals, in declaration order, before the next class down starts
    /// its own: so a subclass's default wins. Those defaults are evaluated on
    /// the new instance (`@x` and the methods are its own) in the class's
    /// environment, as the VM runs them; they were evaluated in the caller's
    /// environment, reading its `this` and its locals, and in hash-map order.
    pub(crate) fn initialize_instance_fields(
        &mut self,
        class: &crate::interpreter::value::Class,
        instance: &Rc<RefCell<Instance>>,
    ) -> RuntimeResult<()> {
        if let Some(ref superclass) = class.superclass {
            self.initialize_instance_fields(superclass, instance)?;
        }
        // A class the VM compiled keeps its starts in `field_init`; its
        // other defaults are bytecode, which only the VM runs.
        if class.fields.is_empty() {
            if let Some(init) = class.field_init.borrow().as_ref() {
                init.start(&mut instance.borrow_mut());
            }
            return Ok(());
        }
        let mut computed = Vec::new();
        for name in crate::interpreter::executor::access::member::declared_field_order(class) {
            let default = class.fields.get(&name).and_then(Option::as_ref);
            let start = match default {
                None => Value::Null,
                Some(expr) => {
                    match crate::interpreter::executor::access::member::literal_value(expr) {
                        Some(value) => value,
                        None => {
                            computed.push((name.clone(), expr));
                            Value::Null
                        }
                    }
                }
            };
            instance.borrow_mut().set(name, start);
        }
        if computed.is_empty() {
            return Ok(());
        }
        // The class's environment: where its methods were declared.
        let enclosing = class
            .defaults_env
            .clone()
            .or_else(|| class.constructor.as_ref().map(|ctor| ctor.closure.clone()))
            .or_else(|| {
                class
                    .methods
                    .borrow()
                    .values()
                    .next()
                    .map(|method| method.closure.clone())
            })
            .unwrap_or_else(|| self.environment.clone());
        let mut env = Environment::with_enclosing(enclosing);
        env.define("this".to_string(), Value::Instance(instance.clone()));
        let env = Rc::new(RefCell::new(env));
        let previous = std::mem::replace(&mut self.environment, env);
        let mut result = Ok(());
        for (name, expr) in computed {
            match self.evaluate(expr) {
                Ok(value) => instance.borrow_mut().set(name, value),
                Err(error) => {
                    result = Err(error);
                    break;
                }
            }
        }
        self.environment = previous;
        result
    }
}
