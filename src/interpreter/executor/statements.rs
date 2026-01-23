//! Statement execution.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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
                    eprintln!("[DEBUG BREAKPOINT] Capturing environment from break() call");
                    let env_json = self.serialize_environment_for_debug();
                    eprintln!("[DEBUG BREAKPOINT] Captured env_json length: {}", env_json.len());
                    return Err(RuntimeError::Breakpoint {
                        span: expr.span,
                        env_json,
                    });
                }
                Ok(ControlFlow::Normal)
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
                Ok(ControlFlow::Normal)
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
                    Ok(ControlFlow::Normal)
                }
            }

            StmtKind::While { condition, body } => {
                while self.evaluate(condition)?.is_truthy() {
                    match self.execute(body)? {
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Normal => {}
                        ControlFlow::Throw(e) => return Ok(ControlFlow::Throw(e)),
                    }
                }
                Ok(ControlFlow::Normal)
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
                Ok(ControlFlow::Normal)
            }

            StmtKind::Class(decl) => {
                self.execute_class(decl)?;
                Ok(ControlFlow::Normal)
            }

            StmtKind::Interface(_) => {
                // Interfaces are handled at type-check time, no runtime effect
                Ok(ControlFlow::Normal)
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
                // Execute try block
                match self.execute(try_block)? {
                    ControlFlow::Normal => {
                        // Try block completed normally
                    }
                    ControlFlow::Return(v) => {
                        // Execute finally if present, then return
                        if let Some(finally_blk) = finally_block {
                            self.execute(finally_blk)?;
                        }
                        return Ok(ControlFlow::Return(v));
                    }
                    ControlFlow::Throw(error) => {
                        // Exception occurred in try block
                        if let Some(catch_blk) = catch_block {
                            // Create new environment for catch block
                            let mut catch_env =
                                Environment::with_enclosing(self.environment.clone());

                            // If catch variable is specified, define it
                            if let Some(var_name) = catch_var {
                                catch_env.define(var_name.clone(), error.clone());
                            }

                            // Execute catch block in new environment
                            let previous = std::mem::replace(
                                &mut self.environment,
                                Rc::new(RefCell::new(catch_env)),
                            );
                            let catch_result = self.execute(catch_blk);
                            self.environment = previous;

                            match catch_result {
                                Ok(ControlFlow::Normal) => {
                                    // Catch block completed normally
                                }
                                Ok(ControlFlow::Return(v)) => {
                                    // Execute finally if present, then return
                                    if let Some(finally_blk) = finally_block {
                                        self.execute(finally_blk)?;
                                    }
                                    return Ok(ControlFlow::Return(v));
                                }
                                Ok(ControlFlow::Throw(new_error)) => {
                                    // Rethrow from catch block
                                    if let Some(finally_blk) = finally_block {
                                        self.execute(finally_blk)?;
                                    }
                                    return Ok(ControlFlow::Throw(new_error));
                                }
                                Err(e) => {
                                    // Error in catch block
                                    if let Some(finally_blk) = finally_block {
                                        self.execute(finally_blk)?;
                                    }
                                    return Err(e);
                                }
                            }
                        } else {
                            // No catch block - rethrow
                            if let Some(finally_blk) = finally_block {
                                self.execute(finally_blk)?;
                            }
                            return Ok(ControlFlow::Throw(error));
                        }
                    }
                }

                // If we get here, try block completed normally (or catch handled exception)
                // Execute finally if present
                if let Some(finally_blk) = finally_block {
                    match self.execute(finally_blk)? {
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Throw(e) => return Ok(ControlFlow::Throw(e)),
                        ControlFlow::Normal => {}
                    }
                }

                Ok(ControlFlow::Normal)
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
                for item in arr.borrow().iter().cloned().collect::<Vec<_>>() {
                    let loop_env = Environment::with_enclosing(self.environment.clone());
                    let prev_env =
                        std::mem::replace(&mut self.environment, Rc::new(RefCell::new(loop_env)));
                    self.environment
                        .borrow_mut()
                        .define(variable.to_string(), item);
                    let result = self.execute(body);
                    self.environment = prev_env;
                    match result? {
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Normal => {}
                        ControlFlow::Throw(e) => return Ok(ControlFlow::Throw(e)),
                    }
                }
                Ok(ControlFlow::Normal)
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
        let extends_model = superclass.as_ref().map_or(false, |sc| {
            sc.name == "Model" || sc.superclass.as_ref().map_or(false, |ssc| ssc.name == "Model")
        });

        // Create environment for methods (with potential super binding)
        let method_env = if superclass.is_some() {
            let env = Environment::with_enclosing(self.environment.clone());
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

        let class = Class {
            name: decl.name.clone(),
            superclass,
            methods,
            static_methods,
            native_static_methods,
            native_methods: HashMap::new(),
            constructor,
        };

        let class_rc = Rc::new(class);
        self.environment
            .borrow_mut()
            .define(decl.name.clone(), Value::Class(class_rc.clone()));

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
                            args.push(self.evaluate(arg)?);
                        }

                        // Call the function with the class
                        self.call_value(callee_val, args, stmt.span)?;
                    }
                }
            }
        }

        Ok(())
    }
}
