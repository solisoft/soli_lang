//! Statement execution.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::expr::Argument;
use crate::ast::*;
use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Function, Value};

use super::{ControlFlow, Interpreter, RuntimeResult};

impl Interpreter {
    /// Execute a statement, returning control flow information.
    pub(crate) fn execute(&mut self, stmt: &Stmt) -> RuntimeResult<ControlFlow> {
        self.record_coverage(stmt.span.line);
        match &stmt.kind {
            StmtKind::Expression(expr) => {
                let value = self.evaluate(expr)?;
                // Check for breakpoint marker
                if matches!(value, Value::Breakpoint) {
                    let env_json = self.serialize_environment_for_debug();
                    let mut stack_trace = self.get_stack_trace();
                    // Add the breakpoint location as the first entry
                    let file = self
                        .current_source_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    stack_trace.insert(0, format!("break() at {}:{}", file, expr.span.line));
                    return Err(RuntimeError::Breakpoint {
                        span: expr.span,
                        env_json,
                        stack_trace,
                    });
                }
                Ok(ControlFlow::Normal(value))
            }

            StmtKind::Let {
                name, initializer, ..
            } => {
                let value = if let Some(init) = initializer {
                    self.evaluate(init)?
                } else {
                    Value::Null
                };
                self.environment.borrow_mut().define(name.clone(), value);
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Const {
                name, initializer, ..
            } => {
                let value = self.evaluate(initializer)?;
                self.environment
                    .borrow_mut()
                    .define_const(name.clone(), value);
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Block(statements) => self.execute_block(
                statements,
                Environment::with_enclosing(self.environment.clone()),
            ),

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_value = self.evaluate(condition)?;
                if cond_value.is_truthy() {
                    self.execute(then_branch)
                } else if let Some(else_br) = else_branch {
                    self.execute(else_br)
                } else {
                    Ok(ControlFlow::Normal(Value::Null))
                }
            }

            StmtKind::While { condition, body } => {
                while self.evaluate(condition)?.is_truthy() {
                    match self.execute(body)? {
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Normal(_) => {}
                        ControlFlow::Throw(e) => return Ok(ControlFlow::Throw(e)),
                    }
                }
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::For {
                variable,
                iterable,
                body,
            } => self.execute_for_loop(variable, iterable, body),

            StmtKind::Return(value) => {
                let return_value = if let Some(expr) = value {
                    self.evaluate(expr)?
                } else {
                    Value::Null
                };
                Ok(ControlFlow::Return(return_value))
            }

            StmtKind::Function(decl) => {
                let source_path = self
                    .current_source_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
                let func = Function::from_decl(decl, self.environment.clone(), source_path);
                self.environment
                    .borrow_mut()
                    .define(decl.name.clone(), Value::Function(Rc::new(func)));
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Class(decl) => {
                self.execute_class(decl)?;
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Interface(_) => {
                // Interfaces are handled at type-check time, no runtime effect
                Ok(ControlFlow::Normal(Value::Null))
            }

            StmtKind::Import(import_decl) => {
                // Module imports are resolved before execution by the ModuleResolver
                // If we reach here, it means import resolution hasn't been done yet
                Err(RuntimeError::General {
                    message: format!(
                        "Import '{}' was not resolved. Run module resolution first.",
                        import_decl.path
                    ),
                    span: stmt.span,
                })
            }

            StmtKind::Export(inner) => {
                // Export just executes the inner declaration and marks it as exported
                // The module system tracks what's exported
                self.execute(inner)
            }

            StmtKind::Throw(value) => {
                let error_value = self.evaluate(value)?;
                Ok(ControlFlow::Throw(error_value))
            }

            StmtKind::Try {
                try_block,
                catch_var,
                catch_block,
                finally_block,
            } => {
                let try_result = self.execute(try_block);

                let throw_value = match try_result {
                    Ok(control_flow) => match control_flow {
                        ControlFlow::Normal(_) => None,
                        ControlFlow::Return(v) => {
                            if let Some(finally_blk) = finally_block {
                                self.execute(finally_blk)?;
                            }
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Throw(error) => Some(error),
                    },
                    Err(e) => {
                        let error_value = Value::String(format!("{}", e));
                        Some(error_value)
                    }
                };

                if let Some(error) = throw_value {
                    if let Some(catch_blk) = catch_block {
                        let mut catch_env = Environment::with_enclosing(self.environment.clone());

                        if let Some(var_name) = catch_var {
                            catch_env.define(var_name.clone(), error.clone());
                        }

                        let previous = std::mem::replace(
                            &mut self.environment,
                            Rc::new(RefCell::new(catch_env)),
                        );
                        let catch_result = self.execute(catch_blk);
                        self.environment = previous;

                        match catch_result {
                            Ok(ControlFlow::Normal(_)) => {}
                            Ok(ControlFlow::Return(v)) => {
                                if let Some(finally_blk) = finally_block {
                                    self.execute(finally_blk)?;
                                }
                                return Ok(ControlFlow::Return(v));
                            }
                            Ok(ControlFlow::Throw(new_error)) => {
                                if let Some(finally_blk) = finally_block {
                                    self.execute(finally_blk)?;
                                }
                                return Ok(ControlFlow::Throw(new_error));
                            }
                            Err(e) => {
                                if let Some(finally_blk) = finally_block {
                                    self.execute(finally_blk)?;
                                }
                                return Err(e);
                            }
                        }
                    } else {
                        if let Some(finally_blk) = finally_block {
                            self.execute(finally_blk)?;
                        }
                        return Ok(ControlFlow::Throw(error));
                    }
                }

                if let Some(finally_blk) = finally_block {
                    match self.execute(finally_blk)? {
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Throw(e) => return Ok(ControlFlow::Throw(e)),
                        ControlFlow::Normal(_) => {}
                    }
                }

                Ok(ControlFlow::Normal(Value::Null))
            }
        }
    }

    fn execute_for_loop(
        &mut self,
        variable: &str,
        iterable: &Expr,
        body: &Stmt,
    ) -> RuntimeResult<ControlFlow> {
        let iter_value = self.evaluate(iterable)?;
        match iter_value {
            Value::Array(arr) => {
                // Clone items once outside the loop to avoid holding borrow across loop body
                let items: Vec<Value> = arr.borrow().iter().cloned().collect();
                for item in items {
                    // Create loop environment with variable already defined (avoids extra borrow_mut)
                    let mut loop_env = Environment::with_enclosing(self.environment.clone());
                    loop_env.define(variable.to_string(), item);
                    let prev_env =
                        std::mem::replace(&mut self.environment, Rc::new(RefCell::new(loop_env)));
                    let result = self.execute(body);
                    self.environment = prev_env;
                    match result? {
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Normal(_) => {}
                        ControlFlow::Throw(e) => return Ok(ControlFlow::Throw(e)),
                    }
                }
                Ok(ControlFlow::Normal(Value::Null))
            }
            _ => Err(RuntimeError::type_error(
                format!("cannot iterate over {}", iter_value.type_name()),
                iterable.span,
            )),
        }
    }

    pub(super) fn execute_class(&mut self, decl: &ClassDecl) -> RuntimeResult<()> {
        let superclass = if let Some(ref superclass_name) = decl.superclass {
            match self.environment.borrow().get(superclass_name) {
                Some(Value::Class(class)) => Some(class),
                Some(_) => {
                    return Err(RuntimeError::NotAClass(superclass_name.clone(), decl.span));
                }
                None => {
                    return Err(RuntimeError::undefined_variable(superclass_name, decl.span));
                }
            }
        } else {
            None
        };

        // Check if this class extends Model (directly or indirectly)
        let extends_model = superclass.as_ref().is_some_and(|sc| {
            sc.name == "Model"
                || sc
                    .superclass
                    .as_ref()
                    .is_some_and(|ssc| ssc.name == "Model")
        });

        // Create environment for methods (with potential super binding)
        let method_env = if let Some(ref sc) = superclass {
            let mut env = Environment::with_enclosing(self.environment.clone());
            // Store the superclass for super calls within methods of this class
            env.define(
                "__defining_superclass__".to_string(),
                Value::Class(sc.clone()),
            );
            Rc::new(RefCell::new(env))
        } else {
            self.environment.clone()
        };

        // Collect methods
        let mut methods = HashMap::new();
        let mut static_methods = HashMap::new();

        let source_path = self
            .current_source_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        for method_decl in &decl.methods {
            let func = Function::from_method(method_decl, method_env.clone(), source_path.clone());
            if method_decl.is_static {
                static_methods.insert(method_decl.name.clone(), Rc::new(func));
            } else {
                methods.insert(method_decl.name.clone(), Rc::new(func));
            }
        }

        // Create constructor if present
        let constructor = decl.constructor.as_ref().map(|ctor| {
            Rc::new(Function {
                name: "new".to_string(),
                params: ctor.params.clone(),
                body: ctor.body.clone(),
                closure: method_env.clone(),
                is_method: true,
                span: Some(ctor.span),
                source_path: self
                    .current_source_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string()),
                defining_superclass: None,
            })
        });

        // If extending Model, inherit Model's native static methods
        let native_static_methods = if extends_model {
            if let Some(ref sc) = superclass {
                sc.native_static_methods.clone()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        let mut fields = HashMap::new();
        let mut static_field_initializers = HashMap::new();
        let mut const_fields = HashSet::new();
        let mut static_const_fields = HashSet::new();
        for field in &decl.fields {
            if field.is_static {
                static_field_initializers.insert(field.name.clone(), field.initializer.clone());
                if field.is_const {
                    static_const_fields.insert(field.name.clone());
                }
            } else {
                fields.insert(field.name.clone(), field.initializer.clone());
                if field.is_const {
                    const_fields.insert(field.name.clone());
                }
            }
        }

        let class = Class {
            name: decl.name.clone(),
            superclass,
            methods,
            static_methods,
            native_static_methods,
            native_methods: HashMap::new(),
            constructor,
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            const_fields,
            static_const_fields,
            ..Default::default()
        };

        let class_rc = Rc::new(class);
        self.environment
            .borrow_mut()
            .define(decl.name.clone(), Value::Class(class_rc.clone()));

        // Initialize static fields
        for (field_name, field_initializer) in static_field_initializers {
            let value = if let Some(init_expr) = field_initializer {
                self.evaluate(&init_expr)?
            } else {
                Value::Null
            };
            class_rc
                .static_fields
                .borrow_mut()
                .insert(field_name, value);
        }

        // Execute static block if present
        if let Some(ref static_block) = decl.static_block {
            // Create a temporary "this" context for the static block
            // The static block can access the class via `this` or directly
            let static_env = Rc::new(RefCell::new(Environment::with_enclosing(
                self.environment.clone(),
            )));
            let this_value = Value::Class(class_rc.clone());
            static_env
                .borrow_mut()
                .define("this".to_string(), this_value);

            // Execute each statement in the static block
            for stmt in static_block {
                self.execute_with_env(stmt, static_env.clone())?;
            }
        }

        // Execute class-level statements (validates, callbacks, etc.) for Model subclasses
        if extends_model && !decl.class_statements.is_empty() {
            // Execute each class statement with the class as implicit receiver
            for stmt in &decl.class_statements {
                // For expression statements that are function calls,
                // we need to pass the class as the first argument
                if let crate::ast::StmtKind::Expression(expr) = &stmt.kind {
                    if let crate::ast::ExprKind::Call { callee, arguments } = &expr.kind {
                        // Get the callee value (should be a native function from Model)
                        let callee_val = self.evaluate(callee)?;

                        // Build arguments with class as first argument
                        let mut args = vec![Value::Class(class_rc.clone())];
                        for arg in arguments {
                            match arg {
                                Argument::Positional(expr) => {
                                    args.push(self.evaluate(expr)?);
                                }
                                Argument::Named(_) => {
                                    return Err(RuntimeError::type_error(
                                        "model validation does not support named arguments",
                                        stmt.span,
                                    ));
                                }
                            }
                        }

                        // Call the function with the class
                        self.call_value(callee_val, args, stmt.span)?;
                    }
                }
            }
        }

        // Process nested classes
        if !decl.nested_classes.is_empty() {
            self.execute_nested_classes(&decl.nested_classes, class_rc.clone())?;
        }

        Ok(())
    }

    /// Execute nested class declarations
    fn execute_nested_classes(
        &mut self,
        nested_decls: &[ClassDecl],
        parent_class: Rc<Class>,
    ) -> RuntimeResult<()> {
        for nested_decl in nested_decls {
            // Save the current environment
            let previous_env = self.environment.clone();

            // Create a new environment that inherits from the parent
            let nested_env = Rc::new(RefCell::new(Environment::with_enclosing(
                previous_env.clone(),
            )));

            // Define the parent class in the nested environment
            nested_env.borrow_mut().define(
                parent_class.name.clone(),
                Value::Class(parent_class.clone()),
            );

            // Set the environment to the nested environment
            self.environment = nested_env;

            // Execute the nested class declaration
            self.execute_class(nested_decl)?;

            // After execute_class, the nested class should be in the environment
            // Get it and store it in the parent's nested_classes map
            if let Some(Value::Class(nested_class)) =
                self.environment.borrow().get(&nested_decl.name)
            {
                parent_class
                    .nested_classes
                    .borrow_mut()
                    .insert(nested_decl.name.clone(), nested_class.clone());
            }

            // Restore the previous environment
            self.environment = previous_env;
        }

        Ok(())
    }

    /// Execute a statement with a custom environment (used for static blocks).
    fn execute_with_env(
        &mut self,
        stmt: &Stmt,
        env: Rc<RefCell<Environment>>,
    ) -> RuntimeResult<()> {
        let previous_env = std::mem::replace(&mut self.environment, env);
        let result = self.execute(stmt);
        self.environment = previous_env;
        result?;
        Ok(())
    }
}
