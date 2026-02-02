//! Variable type resolution.

use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check variable expression.
    pub(crate) fn check_variable(&mut self, name: &str, span: Span) -> TypeResult<Type> {
        self.env
            .get(name)
            .ok_or_else(|| TypeError::UndefinedVariable(name.to_string(), span))
    }

    /// Check qualified name expression.
    pub(crate) fn check_qualified_name(&mut self, _span: Span) -> TypeResult<Type> {
        // For now, return Unknown for qualified names
        // Full type checking would require runtime evaluation
        Ok(Type::Unknown)
    }

    /// Check grouping expression (parentheses).
    pub(crate) fn check_grouping(&mut self, inner: &Expr) -> TypeResult<Type> {
        self.check_expr(inner)
    }
}
