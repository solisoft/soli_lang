//! Type checking for literals.

use crate::ast::*;
use crate::types::checker::expr::InterpolatedPart;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check literal expressions.
    pub(crate) fn check_literal(&mut self, kind: &ExprKind) -> TypeResult<Type> {
        match kind {
            ExprKind::IntLiteral(_) => Ok(Type::Int),
            ExprKind::FloatLiteral(_) => Ok(Type::Float),
            ExprKind::DecimalLiteral(_) => Ok(Type::Decimal(0)),
            ExprKind::StringLiteral(_) => Ok(Type::String),
            ExprKind::BoolLiteral(_) => Ok(Type::Bool),
            ExprKind::Null => Ok(Type::Null),
            _ => unreachable!("Expected literal expression kind"),
        }
    }

    /// Check interpolated string expression.
    pub(crate) fn check_interpolated_string(
        &mut self,
        parts: &[InterpolatedPart],
    ) -> TypeResult<Type> {
        for part in parts {
            if let InterpolatedPart::Expression(expr) = part {
                self.check_expr(expr)?;
            }
        }
        Ok(Type::String)
    }
}
