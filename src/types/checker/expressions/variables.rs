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
            // In a view, a bare read of a function that takes arguments is a
            // local the caller passed under that name (`items`, `docs_og`):
            // read bare, the function would fail for want of them.
            if self.view
                && !self.view_callee
                && matches!(&ty, Type::Function { params, .. } if !params.is_empty())
            {
                return Ok(Type::Any);
            }
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
        // `post_path(@post)`, `mcp_url()`: a route helper. The router names
        // them when `config/routes.sl` runs, which the checker does not do.
        if self.route_helpers && (name.ends_with("_path") || name.ends_with("_url")) {
            return Ok(Type::Any);
        }
        // In a view, a bare name read is a local the caller passed — a
        // partial's, a component's — and unknown here. A name *called* must
        // still exist (`view_callee`).
        if self.view && !self.view_callee {
            return Ok(Type::Any);
        }
        let suggestion = crate::error::suggest::best_match(name, self.env.visible_names());
        Err(TypeError::UndefinedVariable(
            name.to_string(),
            span,
            suggestion,
        ))
    }

    /// A variable read for its value. A bare
    /// [`BARE_CALL_BUILTINS`](crate::interpreter::builtins::BARE_CALL_BUILTINS)
    /// name is the call, so `clock - start` is a `Float`; a callee is typed by
    /// [`check_variable`](Self::check_variable), so `clock()` still is too.
    pub(crate) fn check_variable_value(&mut self, name: &str, span: Span) -> TypeResult<Type> {
        let ty = self.check_variable(name, span)?;
        if crate::interpreter::builtins::is_bare_call_builtin(name) {
            return Ok(super::access::collapse_zero_arg_method(ty));
        }
        Ok(ty)
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
