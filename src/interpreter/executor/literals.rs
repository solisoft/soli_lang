//! Literal expression evaluation.

use crate::ast::ExprKind;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::{Interpreter, RuntimeResult};
use crate::error::RuntimeError;

impl Interpreter {
    /// Evaluate an interpolated string expression.
    pub(crate) fn evaluate_interpolated_string(
        &mut self,
        parts: &Vec<crate::ast::expr::InterpolatedPart>,
        _span: Span,
    ) -> RuntimeResult<Value> {
        let mut result = String::new();
        for part in parts {
            match part {
                crate::ast::expr::InterpolatedPart::Literal(s) => {
                    result.push_str(s);
                }
                crate::ast::expr::InterpolatedPart::Expression(expr) => {
                    let value = self.evaluate(expr)?;
                    result.push_str(&value.to_string());
                }
            }
        }
        Ok(Value::String(result))
    }

    /// Evaluate a literal value from an expression kind.
    /// Used for pattern matching.
    pub(crate) fn evaluate_literal(&self, literal: &ExprKind) -> RuntimeResult<Value> {
        match literal {
            ExprKind::IntLiteral(n) => Ok(Value::Int(*n)),
            ExprKind::FloatLiteral(n) => Ok(Value::Float(*n)),
            ExprKind::DecimalLiteral(s) => {
                use crate::interpreter::value::DecimalValue;
                let decimal: rust_decimal::Decimal = s.parse().map_err(|_| {
                    RuntimeError::type_error("invalid decimal literal", Span::default())
                })?;
                let precision = s.split('.').nth(1).map(|p| p.len() as u32).unwrap_or(0);
                Ok(Value::Decimal(DecimalValue(decimal, precision)))
            }
            ExprKind::StringLiteral(s) => Ok(Value::String(s.clone())),
            ExprKind::BoolLiteral(b) => Ok(Value::Bool(*b)),
            ExprKind::Null => Ok(Value::Null),
            _ => Err(RuntimeError::type_error(
                "expected literal expression",
                Span::default(),
            )),
        }
    }

    /// Compare two values for equality.
    /// Used for pattern matching.
    pub(crate) fn values_equal(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            _ => false,
        }
    }
}
