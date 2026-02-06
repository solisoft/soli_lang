//! Expression evaluation.
//!
//! This module has been refactored into focused submodules.

use crate::ast::{Expr, ExprKind};
use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, Value};
use crate::span::Span;

use std::cell::RefCell;
use std::rc::Rc;

use super::{ControlFlow, Interpreter, RuntimeResult};

// Pattern matching is still in the main module as it's a cross-cutting concern
impl Interpreter {
    /// Evaluate an expression.
    /// This is the main dispatch method that delegates to specialized evaluators.
    pub(crate) fn evaluate(&mut self, expr: &Expr) -> RuntimeResult<Value> {
        self.record_coverage(expr.span.line);

        match &expr.kind {
            // Literals
            ExprKind::IntLiteral(n) => Ok(Value::Int(*n)),
            ExprKind::FloatLiteral(n) => Ok(Value::Float(*n)),
            ExprKind::StringLiteral(s) => Ok(Value::String(s.clone())),
            ExprKind::BoolLiteral(b) => Ok(Value::Bool(*b)),
            ExprKind::Null => Ok(Value::Null),

            // Variables
            ExprKind::Variable(name) => self.evaluate_variable(name, expr),

            // Grouping
            ExprKind::Grouping(inner) => self.evaluate(inner),

            // Operators
            ExprKind::Binary {
                left,
                operator,
                right,
            } => self.evaluate_binary(left, *operator, right, expr.span),

            ExprKind::Unary { operator, operand } => {
                self.evaluate_unary(*operator, operand, expr.span)
            }

            ExprKind::LogicalAnd { left, right } => self.evaluate_logical_and(left, right),

            ExprKind::LogicalOr { left, right } => self.evaluate_logical_or(left, right),

            ExprKind::NullishCoalescing { left, right } => {
                self.evaluate_nullish_coalescing(left, right)
            }

            // Calls
            ExprKind::Call { callee, arguments } => {
                self.evaluate_call(callee, arguments, expr.span)
            }

            ExprKind::Pipeline { left, right } => self.evaluate_pipeline(left, right, expr.span),

            // Access
            ExprKind::Member { object, name } => self.evaluate_member(object, name, expr.span),

            ExprKind::QualifiedName { qualifier, name } => {
                self.evaluate_qualified_name(qualifier, name, expr.span)
            }

            ExprKind::Index { object, index } => self.evaluate_index(object, index, expr.span),

            // Control/Keywords
            ExprKind::This => self.evaluate_this(expr),

            ExprKind::Super => self.evaluate_super(expr),

            // Object creation
            ExprKind::New {
                class_expr,
                arguments,
            } => self.evaluate_new(class_expr, arguments, expr.span),

            ExprKind::Array(elements) => self.evaluate_array(elements),

            ExprKind::Hash(pairs) => self.evaluate_hash(pairs),

            // Block
            ExprKind::Block(statements) => {
                let env = Environment::with_enclosing(self.environment.clone());
                match self.execute_block(statements, env)? {
                    ControlFlow::Normal(v) => Ok(v),
                    ControlFlow::Return(v) => Ok(v),
                    ControlFlow::Throw(e) => Err(RuntimeError::General {
                        message: format!("Unhandled exception: {}", e),
                        span: expr.span,
                    }),
                }
            }

            // Assignment
            ExprKind::Assign { target, value } => self.evaluate_assign(target, value),

            // Lambda
            ExprKind::Lambda { params, body, .. } => self.evaluate_lambda(params, body, expr.span),

            // Control flow expressions
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_value = self.evaluate(condition)?;
                if cond_value.is_truthy() {
                    self.evaluate(then_branch)
                } else {
                    match else_branch {
                        Some(else_expr) => self.evaluate(else_expr),
                        None => Ok(Value::Null),
                    }
                }
            }

            // String interpolation
            ExprKind::InterpolatedString(parts) => {
                self.evaluate_interpolated_string(parts, expr.span)
            }

            // Pattern matching
            ExprKind::Match { expression, arms } => {
                self.evaluate_match(expression, arms, expr.span)
            }

            // Comprehensions
            ExprKind::ListComprehension {
                element,
                variable,
                iterable,
                condition,
            } => {
                self.evaluate_list_comprehension(element, variable, iterable, condition.as_deref())
            }

            ExprKind::HashComprehension {
                key,
                value,
                variable,
                iterable,
                condition,
            } => self.evaluate_hash_comprehension(
                key,
                value,
                variable,
                iterable,
                condition.as_deref(),
            ),

            // Await and Spread
            ExprKind::Await(_) => unimplemented!("Await expressions not yet implemented"),
            ExprKind::Spread(_) => unimplemented!("Spread expressions not yet implemented"),

            // Throw expression
            ExprKind::Throw(value) => {
                let error_value = self.evaluate(value)?;
                Err(RuntimeError::General {
                    message: format!("{}", error_value),
                    span: expr.span,
                })
            }
        }
    }

    /// Evaluate assignment expression.
    fn evaluate_assign(&mut self, target: &Expr, value: &Expr) -> RuntimeResult<Value> {
        let new_value = self.evaluate(value)?;

        match &target.kind {
            ExprKind::Variable(name) => {
                // Check if the variable is a const
                if self.environment.borrow().is_const(name) {
                    return Err(RuntimeError::type_error(
                        format!("cannot reassign constant '{}'", name),
                        target.span,
                    ));
                }
                if !self
                    .environment
                    .borrow_mut()
                    .assign(name, new_value.clone())
                {
                    return Err(RuntimeError::undefined_variable(name, target.span));
                }
                Ok(new_value)
            }
            ExprKind::Member { object, name } => {
                let obj_val = self.evaluate(object)?;
                match obj_val {
                    Value::Instance(inst) => {
                        inst.borrow_mut().set(name.clone(), new_value.clone());
                        Ok(new_value)
                    }
                    Value::Class(class) => {
                        // Set static field on class
                        class
                            .static_fields
                            .borrow_mut()
                            .insert(name.clone(), new_value.clone());
                        Ok(new_value)
                    }
                    _ => Err(RuntimeError::type_error(
                        format!("cannot set property on {}", obj_val.type_name()),
                        target.span,
                    )),
                }
            }
            ExprKind::Index { object, index } => {
                self.evaluate_index_assign(object, index, new_value, target.span)
            }
            _ => Err(RuntimeError::type_error(
                "invalid assignment target",
                target.span,
            )),
        }
    }

    /// Evaluate match expression.
    fn evaluate_match(
        &mut self,
        expression: &Expr,
        arms: &Vec<crate::ast::expr::MatchArm>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let input_value = self.evaluate(expression)?;

        for arm in arms {
            if let Some(bindings) = self.match_pattern(&input_value, &arm.pattern)? {
                let env = self.environment.clone();

                for (name, value) in bindings {
                    env.borrow_mut().define(name, value);
                }

                if let Some(guard) = &arm.guard {
                    let guard_value = self.evaluate(guard)?;
                    if !guard_value.is_truthy() {
                        continue;
                    }
                }

                return self.evaluate(&arm.body);
            }
        }

        Err(RuntimeError::type_error(
            "no pattern matched the value",
            span,
        ))
    }

    /// Evaluate list comprehension.
    fn evaluate_list_comprehension(
        &mut self,
        element: &Expr,
        variable: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
    ) -> RuntimeResult<Value> {
        let iter_value = self.evaluate(iterable)?;
        let items: Vec<Value> = match iter_value {
            Value::Array(arr) => arr.borrow().iter().cloned().collect(),
            _ => {
                return Err(RuntimeError::type_error("expected array", iterable.span));
            }
        };

        let mut result = Vec::new();
        for item in items {
            let mut loop_env = Environment::with_enclosing(self.environment.clone());
            loop_env.define(variable.to_string(), item);
            let prev_env =
                std::mem::replace(&mut self.environment, Rc::new(RefCell::new(loop_env)));

            let pass_condition = if let Some(cond) = condition {
                let cond_value = self.evaluate(cond)?;
                if !cond_value.is_truthy() {
                    self.environment = prev_env;
                    continue;
                }
                true
            } else {
                true
            };

            let elem_value = self.evaluate(element)?;
            self.environment = prev_env;

            if pass_condition {
                result.push(elem_value);
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    /// Evaluate hash comprehension.
    fn evaluate_hash_comprehension(
        &mut self,
        key: &Expr,
        value: &Expr,
        variable: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
    ) -> RuntimeResult<Value> {
        let iter_value = self.evaluate(iterable)?;
        let items: Vec<Value> = match iter_value {
            Value::Array(arr) => arr.borrow().iter().cloned().collect(),
            _ => {
                return Err(RuntimeError::type_error("expected array", iterable.span));
            }
        };

        let mut result: indexmap::IndexMap<HashKey, Value> = indexmap::IndexMap::new();
        for item in items {
            let mut loop_env = Environment::with_enclosing(self.environment.clone());
            loop_env.define(variable.to_string(), item);
            let prev_env =
                std::mem::replace(&mut self.environment, Rc::new(RefCell::new(loop_env)));

            let should_include = if let Some(cond) = condition {
                let cond_value = self.evaluate(cond)?;
                if !cond_value.is_truthy() {
                    self.environment = prev_env;
                    continue;
                }
                true
            } else {
                true
            };

            let key_value = self.evaluate(key)?;
            let val_value = self.evaluate(value)?;
            self.environment = prev_env;

            if should_include {
                if let Some(hash_key) = key_value.to_hash_key() {
                    result.insert(hash_key, val_value);
                } else {
                    return Err(RuntimeError::type_error(
                        format!("{} cannot be used as a hash key", key_value.type_name()),
                        key.span,
                    ));
                }
            }
        }

        Ok(Value::Hash(Rc::new(RefCell::new(result))))
    }
}
