//! Binary and unary operator evaluation.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::*;
use crate::error::RuntimeError;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::{Interpreter, RuntimeResult};

impl Interpreter {
    pub(crate) fn evaluate_binary(
        &mut self,
        left: &Expr,
        op: BinaryOp,
        right: &Expr,
        span: Span,
    ) -> RuntimeResult<Value> {
        let left_val = self.evaluate(left)?;
        let right_val = self.evaluate(right)?;

        // Auto-resolve Futures before binary operations
        let left_val = left_val.resolve().map_err(|e| RuntimeError::new(e, span))?;
        let right_val = right_val
            .resolve()
            .map_err(|e| RuntimeError::new(e, span))?;

        match op {
            BinaryOp::Add => self.eval_add(&left_val, &right_val, span),
            BinaryOp::Subtract => self.eval_subtract(&left_val, &right_val, span),
            BinaryOp::Multiply => self.eval_multiply(&left_val, &right_val, span),
            BinaryOp::Divide => self.eval_divide(&left_val, &right_val, span),
            BinaryOp::Modulo => self.eval_modulo(&left_val, &right_val, span),
            BinaryOp::Equal => Ok(Value::Bool(left_val == right_val)),
            BinaryOp::NotEqual => Ok(Value::Bool(left_val != right_val)),
            BinaryOp::Less => self.compare_values(&left_val, &right_val, span, |a, b| a < b),
            BinaryOp::LessEqual => self.compare_values(&left_val, &right_val, span, |a, b| a <= b),
            BinaryOp::Greater => self.compare_values(&left_val, &right_val, span, |a, b| a > b),
            BinaryOp::GreaterEqual => {
                self.compare_values(&left_val, &right_val, span, |a, b| a >= b)
            }
            BinaryOp::Range => self.eval_range(&left_val, &right_val, span),
        }
    }

    fn eval_range(&self, left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(start), Value::Int(end)) => {
                let arr: Vec<Value> = (*start..*end).map(Value::Int).collect();
                Ok(Value::Array(Rc::new(RefCell::new(arr))))
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "range (..) expects two integers, got {} and {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    fn eval_add(&self, left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
            (Value::String(a), b) => Ok(Value::String(format!("{}{}", a, b))),
            (a, Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
            _ => Err(RuntimeError::type_error(
                format!("cannot add {} and {}", left.type_name(), right.type_name()),
                span,
            )),
        }
    }

    fn eval_subtract(&self, left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
            _ => Err(RuntimeError::type_error(
                format!(
                    "cannot subtract {} from {}",
                    right.type_name(),
                    left.type_name()
                ),
                span,
            )),
        }
    }

    fn eval_multiply(&self, left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
            (Value::String(s), Value::Int(n)) | (Value::Int(n), Value::String(s)) => {
                Ok(Value::String(s.repeat(*n as usize)))
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "cannot multiply {} and {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    fn eval_divide(&self, left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Int(a / b))
                }
            }
            (Value::Float(a), Value::Float(b)) => {
                if *b == 0.0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Float(a / b))
                }
            }
            (Value::Int(a), Value::Float(b)) => {
                if *b == 0.0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Float(*a as f64 / b))
                }
            }
            (Value::Float(a), Value::Int(b)) => {
                if *b == 0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Float(a / *b as f64))
                }
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "cannot divide {} by {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    fn eval_modulo(&self, left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Int(a % b))
                }
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 % b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a % *b as f64)),
            _ => Err(RuntimeError::type_error(
                format!(
                    "cannot modulo {} by {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    pub(crate) fn compare_values<F>(
        &self,
        left: &Value,
        right: &Value,
        span: Span,
        cmp: F,
    ) -> RuntimeResult<Value>
    where
        F: Fn(f64, f64) -> bool,
    {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(cmp(*a as f64, *b as f64))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(cmp(*a, *b))),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Bool(cmp(*a as f64, *b))),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(cmp(*a, *b as f64))),
            (Value::String(a), Value::String(b)) => {
                // Lexicographic comparison for strings
                Ok(Value::Bool(
                    match (a.cmp(b), cmp(1.0, 0.0), cmp(0.0, 0.0)) {
                        (std::cmp::Ordering::Less, true, _) => true,     // <
                        (std::cmp::Ordering::Less, false, true) => true, // <=
                        (std::cmp::Ordering::Equal, _, true) => true,    // ==, <=, >=
                        (std::cmp::Ordering::Greater, _, false) if cmp(1.0, 0.0) => false, // < or <=
                        (std::cmp::Ordering::Greater, _, _) if cmp(2.0, 1.0) => true,      // >
                        _ => false,
                    },
                ))
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "cannot compare {} and {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    pub(crate) fn evaluate_unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        span: Span,
    ) -> RuntimeResult<Value> {
        let val = self.evaluate(operand)?;

        match op {
            UnaryOp::Negate => match val {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(n) => Ok(Value::Float(-n)),
                _ => Err(RuntimeError::type_error(
                    format!("cannot negate {}", val.type_name()),
                    span,
                )),
            },
            UnaryOp::Not => Ok(Value::Bool(!val.is_truthy())),
        }
    }

    pub(crate) fn evaluate_logical_and(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> RuntimeResult<Value> {
        let left_val = self.evaluate(left)?;
        if !left_val.is_truthy() {
            Ok(left_val)
        } else {
            self.evaluate(right)
        }
    }

    pub(crate) fn evaluate_logical_or(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> RuntimeResult<Value> {
        let left_val = self.evaluate(left)?;
        if left_val.is_truthy() {
            Ok(left_val)
        } else {
            self.evaluate(right)
        }
    }

    pub(crate) fn evaluate_nullish_coalescing(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> RuntimeResult<Value> {
        let left_val = self.evaluate(left)?;
        if matches!(left_val, Value::Null) {
            self.evaluate(right)
        } else {
            Ok(left_val)
        }
    }
}
