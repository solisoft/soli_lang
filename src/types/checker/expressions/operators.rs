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
                } else if let Type::Array(elem) = &left_type {
                    if matches!(&right_type, Type::Array(_) | Type::Any | Type::Unknown) {
                        Ok(Type::Array(elem.clone()))
                    } else {
                        Err(TypeError::General {
                            message: format!("cannot add array and {}", right_type),
                            span,
                        })
                    }
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
            BinaryOp::Subtract => {
                if left_type.is_numeric() && right_type.is_numeric() {
                    if matches!(left_type, Type::Float) || matches!(right_type, Type::Float) {
                        Ok(Type::Float)
                    } else {
                        Ok(Type::Int)
                    }
                } else if let Type::Array(elem) = &left_type {
                    if matches!(&right_type, Type::Array(_) | Type::Any | Type::Unknown) {
                        Ok(Type::Array(elem.clone()))
                    } else {
                        Err(TypeError::General {
                            message: format!("cannot subtract {} from array", right_type),
                            span,
                        })
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
            BinaryOp::Multiply | BinaryOp::Divide | BinaryOp::Modulo => {
                // `"-" * 40` and `40 * "-"` both repeat the string. Arrays do
                // not: `[1, 2] * 2` raises at run time, so it is not admitted
                // here either.
                if matches!(operator, BinaryOp::Multiply)
                    && ((matches!(left_type, Type::String) && matches!(right_type, Type::Int))
                        || (matches!(left_type, Type::Int) && matches!(right_type, Type::String)))
                {
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
                let both_datetime = matches!((&left_type, &right_type),
                    (Type::Class(a), Type::Class(b)) if a.name == "DateTime" && b.name == "DateTime");
                if (left_type.is_numeric() && right_type.is_numeric())
                    || (matches!(left_type, Type::String) && matches!(right_type, Type::String))
                    || both_datetime
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
            BinaryOp::Shovel => {
                // `<<` returns the LHS (array push or HABTM relation getter).
                Ok(left_type.clone())
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

    /// Check a logical `&&` / `||` expression.
    ///
    /// **The result is an operand, not a boolean.** `a || b` answers `a` when
    /// `a` is truthy and `b` otherwise; `&&` is the mirror. So `0 || 7` is `7`
    /// and `getenv("HOST") || "localhost"` is a `String` — which is the
    /// default-value idiom the conventions teach.
    ///
    /// Answering `Bool` unconditionally, as this used to, type-checked the
    /// `let` and then failed at the *use*: `Type mismatch: expected String,
    /// found Bool`, pointing at a line that was not the one at fault. There is
    /// no union type to name here, so two operands that disagree give `Any`.
    /// That is wider than the truth and never wrong, and conditions accept
    /// `Any`, so `if a || b` still checks.
    pub(crate) fn check_logical(&mut self, left: &Expr, right: &Expr) -> TypeResult<Type> {
        let left_type = self.check_expr(left)?;
        let right_type = self.check_expr(right)?;
        Ok(if left_type == right_type {
            left_type
        } else {
            Type::Any
        })
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
