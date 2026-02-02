//! Operator type checking.

use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check binary expression.
    pub(crate) fn check_binary_expr(
        &mut self,
        span: Span,
        left: &Expr,
        operator: &BinaryOp,
        right: &Expr,
    ) -> TypeResult<Type> {
        let left_type = self.check_expr(left)?;
        let right_type = self.check_expr(right)?;

        match operator {
            BinaryOp::Add => {
                if matches!(left_type, Type::String) || matches!(right_type, Type::String) {
                    Ok(Type::String)
                } else if left_type.is_numeric() && right_type.is_numeric() {
                    if matches!(left_type, Type::Float) || matches!(right_type, Type::Float) {
                        Ok(Type::Float)
                    } else {
                        Ok(Type::Int)
                    }
                } else if matches!(left_type, Type::Any | Type::Unknown)
                    || matches!(right_type, Type::Any | Type::Unknown)
                {
                    Ok(Type::Any)
                } else {
                    Err(TypeError::General {
                        message: format!("cannot add {} and {}", left_type, right_type),
                        span,
                    })
                }
            }
            BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide | BinaryOp::Modulo => {
                if left_type.is_numeric() && right_type.is_numeric() {
                    if matches!(left_type, Type::Float) || matches!(right_type, Type::Float) {
                        Ok(Type::Float)
                    } else {
                        Ok(Type::Int)
                    }
                } else if matches!(left_type, Type::Any | Type::Unknown)
                    || matches!(right_type, Type::Any | Type::Unknown)
                {
                    Ok(Type::Any)
                } else {
                    Err(TypeError::General {
                        message: format!(
                            "cannot perform arithmetic on {} and {}",
                            left_type, right_type
                        ),
                        span,
                    })
                }
            }
            BinaryOp::Equal | BinaryOp::NotEqual => Ok(Type::Bool),
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
                if (left_type.is_numeric() && right_type.is_numeric())
                    || (matches!(left_type, Type::String) && matches!(right_type, Type::String))
                    || matches!(left_type, Type::Any | Type::Unknown)
                    || matches!(right_type, Type::Any | Type::Unknown)
                {
                    Ok(Type::Bool)
                } else {
                    Err(TypeError::General {
                        message: format!("cannot compare {} and {}", left_type, right_type),
                        span,
                    })
                }
            }
            BinaryOp::Range => {
                if (matches!(left_type, Type::Int) && matches!(right_type, Type::Int))
                    || matches!(left_type, Type::Any | Type::Unknown)
                    || matches!(right_type, Type::Any | Type::Unknown)
                {
                    Ok(Type::Array(Box::new(Type::Int)))
                } else {
                    Err(TypeError::General {
                        message: format!(
                            "range (..) expects two integers, got {} and {}",
                            left_type, right_type
                        ),
                        span,
                    })
                }
            }
        }
    }

    /// Check unary expression.
    pub(crate) fn check_unary_expr(
        &mut self,
        span: Span,
        operator: &UnaryOp,
        operand: &Expr,
    ) -> TypeResult<Type> {
        let operand_type = self.check_expr(operand)?;
        match operator {
            UnaryOp::Negate => {
                if operand_type.is_numeric() || matches!(operand_type, Type::Any | Type::Unknown) {
                    Ok(operand_type)
                } else {
                    Err(TypeError::General {
                        message: format!("cannot negate {}", operand_type),
                        span,
                    })
                }
            }
            UnaryOp::Not => Ok(Type::Bool),
        }
    }

    /// Check logical AND/OR expression.
    pub(crate) fn check_logical(&mut self, left: &Expr, right: &Expr) -> TypeResult<Type> {
        self.check_expr(left)?;
        self.check_expr(right)?;
        Ok(Type::Bool)
    }

    /// Check nullish coalescing expression.
    pub(crate) fn check_nullish_coalescing(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> TypeResult<Type> {
        self.check_expr(left)?;
        let right_type = self.check_expr(right)?;
        // The result type is the right type (since if left is null, we return right)
        Ok(right_type)
    }
}
