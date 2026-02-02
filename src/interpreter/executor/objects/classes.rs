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

        // Create instance
        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));

        // Initialize fields from class field declarations (including inherited fields)
        fn initialize_fields(
            interpreter: &mut Interpreter,
            class: &crate::interpreter::value::Class,
            instance: &Rc<std::cell::RefCell<crate::interpreter::value::Instance>>,
        ) -> RuntimeResult<()> {
            // First initialize fields from superclass
            if let Some(ref superclass) = class.superclass {
                initialize_fields(interpreter, superclass, instance)?;
            }
            // Then initialize fields from this class
            for (field_name, field_initializer) in &class.fields {
                let value = if let Some(init_expr) = field_initializer {
                    interpreter.evaluate(init_expr)?
                } else {
                    Value::Null
                };
                instance.borrow_mut().set(field_name.clone(), value);
            }
            Ok(())
        }
        initialize_fields(self, &class, &instance)?;

        // Call constructor if present
        if let Some(ctor) = class.find_constructor() {
            let ctor = &ctor;
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
                }
            }

            let param_names: Vec<String> = ctor.params.iter().map(|p| p.name.clone()).collect();

            // Check for unknown named arguments
            for name in named_args.keys() {
                if !param_names.contains(name) {
                    return Err(RuntimeError::undefined_variable(name.clone(), span));
                }
            }

            // Build constructor arguments
            let mut ctor_args = Vec::new();
            let mut used_params = HashSet::new();

            // Positional arguments first
            for (i, arg_val) in positional_args.iter().enumerate() {
                if i < param_names.len() {
                    ctor_args.push(arg_val.clone());
                    used_params.insert(param_names[i].clone());
                } else {
                    return Err(RuntimeError::wrong_arity(
                        ctor.full_arity(),
                        positional_args.len() + named_args.len(),
                        span,
                    ));
                }
            }

            // Named arguments and defaults
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
                        ctor.arity(),
                        ctor_args.len(),
                        span,
                    ));
                }
            }

            // Create constructor environment
            let ctor_env = Environment::with_enclosing(ctor.closure.clone());
            let mut ctor_env = ctor_env;
            ctor_env.define("this".to_string(), Value::Instance(instance.clone()));

            for (param, value) in ctor.params.iter().zip(ctor_args.iter()) {
                ctor_env.define(param.name.clone(), value.clone());
            }

            // Execute constructor body
            let _ = self.execute_block(&ctor.body, ctor_env);
        }

        Ok(Value::Instance(instance))
    }
}
