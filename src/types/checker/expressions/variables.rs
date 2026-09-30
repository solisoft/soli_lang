//! Variable type resolution.

use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check variable expression.
    pub(crate) fn check_variable(&mut self, name: &str, span: Span) -> TypeResult<Type> {
        if let Some(ty) = self.env.get(name) {
            return Ok(ty);
        }
        // Inside an instance method, a bare name that is no variable or function
        // is `@name`: a method of the class (or an ancestor) the runtime calls on
        // `this`. When the ancestry leaves this file, the method may be defined
        // where the checker cannot see it.
        if self.env.get("this").is_some() {
            if let Some(class) = self.env.current_class_type() {
                if let Some(method) = class.find_method(name) {
                    return Ok(super::access::collapse_zero_arg_method(Type::Function {
                        params: method.params.iter().map(|(_, t)| t.clone()).collect(),
                        return_type: Box::new(method.return_type.clone()),
                    }));
                }
                if class.has_members_elsewhere() {
                    return Ok(Type::Any);
                }
            }
        }
        Err(TypeError::UndefinedVariable(name.to_string(), span))
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
