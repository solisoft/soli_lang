//! this, super, lambda and control flow type checking.

use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check 'this' expression.
    pub(crate) fn check_this_expr(&self, span: Span) -> TypeResult<Type> {
        if let Some(class_type) = self.env.current_class_type() {
            Ok(Type::Class(class_type.clone()))
        } else {
            Err(TypeError::ThisOutsideClass(span))
        }
    }

    /// Check 'super' expression.
    pub(crate) fn check_super_expr(&self, span: Span) -> TypeResult<Type> {
        if let Some(class_type) = self.env.current_class_type() {
            if let Some(ref superclass) = class_type.superclass {
                Ok(Type::Class(*superclass.clone()))
            } else {
                Err(TypeError::NoSuperclass(class_type.name.clone(), span))
            }
        } else {
            Err(TypeError::SuperOutsideClass(span))
        }
    }

    /// Check lambda expression.
    pub(crate) fn check_lambda_expr(
        &mut self,
        body: &[Stmt],
        params: &[Parameter],
        return_type: &Option<TypeAnnotation>,
    ) -> TypeResult<Type> {
        self.env.push_scope();

        let param_types: Vec<Type> = params
            .iter()
            .map(|param| {
                let t = self.resolve_type(&param.type_annotation);
                self.env.define(param.name.clone(), t.clone());
                t
            })
            .collect();

        let ret_type = return_type
            .as_ref()
            .map(|t| self.resolve_type(t))
            .unwrap_or(Type::Any);

        self.env.set_return_type(Some(ret_type.clone()));

        // Check body statements
        // Note: Implicit return logic is handled in parsing (last expr wrapped in Return)
        // or we rely on Return statements in body.
        for stmt in body {
            if let Err(e) = self.check_stmt(stmt) {
                self.errors.push(e);
            }
        }

        self.env.set_return_type(None);
        self.env.pop_scope();

        // Infer return type from body if not explicit?
        // For now, we just validate against explicit return type (or Any).

        Ok(Type::Function {
            params: param_types,
            return_type: Box::new(ret_type),
        })
    }

    /// Check throw expression.
    ///
    /// `throw` parses to a statement (`StmtKind::Throw`), so this expression
    /// form is currently unreachable from the grammar. Handle it gracefully
    /// anyway — type-check the thrown value and treat the throw as producing
    /// no usable value (`Any`) — rather than panicking, in case a future
    /// grammar change ever routes a throw expression here.
    pub(crate) fn check_throw_expr(&mut self, expr: &Expr) -> TypeResult<Type> {
        self.check_expr(expr)?;
        Ok(Type::Any)
    }
}

#[cfg(test)]
mod throw_check_tests {
    use super::*;

    // `throw` parses to a statement (`StmtKind::Throw`), so `ExprKind::Throw`
    // is unreachable from the grammar. If a throw expression ever reaches the
    // checker it must be handled gracefully (it used to be `unimplemented!()`,
    // which would panic). This pins the graceful path.
    #[test]
    fn throw_expression_type_checks_gracefully() {
        let span = Span::new(0, 0, 1, 0);
        let throw = Expr::new(
            ExprKind::Throw(Box::new(Expr::new(ExprKind::Null, span))),
            span,
        );
        let mut checker = TypeChecker::new();
        let ty = checker
            .check_expr(&throw)
            .expect("throw expression must type-check gracefully, not panic");
        assert_eq!(ty, Type::Any);
    }
}
