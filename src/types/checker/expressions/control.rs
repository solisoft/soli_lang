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

    /// Check await expression.
    pub(crate) fn check_await_expr(&mut self, _expr: &Expr) -> TypeResult<Type> {
        unimplemented!("Await expressions not yet implemented")
    }

    /// Check throw expression.
    pub(crate) fn check_throw_expr(&mut self, _expr: &Expr) -> TypeResult<Type> {
        unimplemented!("Throw expressions not yet implemented")
    }
}
