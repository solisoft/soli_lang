//! Statement type checking.

use crate::ast::*;
use crate::error::TypeError;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    pub(crate) fn check_stmt(&mut self, stmt: &Stmt) -> TypeResult<()> {
        match &stmt.kind {
            StmtKind::Expression(expr) => {
                self.check_expr(expr)?;
                Ok(())
            }

            StmtKind::Let {
                name,
                type_annotation,
                initializer,
            } => {
                let declared_type = type_annotation.as_ref().map(|t| self.resolve_type(t));
                let init_type = if let Some(init) = initializer {
                    Some(self.check_expr(init)?)
                } else {
                    None
                };

                let var_type = match (declared_type, init_type) {
                    (Some(decl), Some(init)) => {
                        if !init.is_assignable_to(&decl) {
                            return Err(TypeError::mismatch(
                                format!("{}", decl),
                                format!("{}", init),
                                stmt.span,
                            ));
                        }
                        decl
                    }
                    (Some(decl), None) => decl,
                    (None, Some(init)) => init,
                    (None, None) => Type::Unknown,
                };

                self.env.define(name.clone(), var_type);
                Ok(())
            }

            StmtKind::Const {
                name,
                type_annotation,
                initializer,
            } => {
                let declared_type = type_annotation.as_ref().map(|t| self.resolve_type(t));
                let init_type = self.check_expr(initializer)?;

                let const_type = match declared_type {
                    Some(decl) => {
                        if !init_type.is_assignable_to(&decl) {
                            return Err(TypeError::mismatch(
                                format!("{}", decl),
                                format!("{}", init_type),
                                stmt.span,
                            ));
                        }
                        decl
                    }
                    None => init_type,
                };

                self.env.define(name.clone(), const_type);
                Ok(())
            }

            StmtKind::Block(statements) => {
                self.env.push_scope();
                for s in statements {
                    self.check_stmt(s)?;
                }
                self.env.pop_scope();
                Ok(())
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_type = self.check_expr(condition)?;
                if !matches!(cond_type, Type::Bool | Type::Any | Type::Unknown) {
                    return Err(TypeError::mismatch(
                        "Bool",
                        format!("{}", cond_type),
                        condition.span,
                    ));
                }
                self.check_stmt(then_branch)?;
                if let Some(else_br) = else_branch {
                    self.check_stmt(else_br)?;
                }
                Ok(())
            }

            StmtKind::While { condition, body } => {
                let cond_type = self.check_expr(condition)?;
                if !matches!(cond_type, Type::Bool | Type::Any | Type::Unknown) {
                    return Err(TypeError::mismatch(
                        "Bool",
                        format!("{}", cond_type),
                        condition.span,
                    ));
                }
                self.check_stmt(body)?;
                Ok(())
            }

            StmtKind::For {
                variable,
                index_variable,
                iterable,
                body,
            } => {
                let iter_type = self.check_expr(iterable)?;
                let elem_type = match iter_type {
                    Type::Array(inner) => *inner,
                    Type::Any => Type::Any,
                    _ => {
                        return Err(TypeError::General {
                            message: format!("cannot iterate over {}", iter_type),
                            span: iterable.span,
                        });
                    }
                };

                self.env.push_scope();
                self.env.define(variable.clone(), elem_type);
                if let Some(idx_var) = index_variable {
                    self.env.define(idx_var.clone(), Type::Int);
                }
                self.check_stmt(body)?;
                self.env.pop_scope();
                Ok(())
            }

            StmtKind::Return(value) => {
                let return_type = if let Some(expr) = value {
                    self.check_expr(expr)?
                } else {
                    Type::Void
                };

                if let Some(expected) = self.env.return_type() {
                    if !return_type.is_assignable_to(expected) {
                        return Err(TypeError::mismatch(
                            format!("{}", expected),
                            format!("{}", return_type),
                            stmt.span,
                        ));
                    }
                }
                Ok(())
            }

            StmtKind::Function(decl) => {
                // Resolve parameter types and return type
                let param_types: Vec<Type> = decl
                    .params
                    .iter()
                    .map(|p| self.resolve_type(&p.type_annotation))
                    .collect();
                let return_type = decl
                    .return_type
                    .as_ref()
                    .map(|t| self.resolve_type(t))
                    .unwrap_or(Type::Any);

                // Register function in the OUTER scope so callers (and recursion) can see it
                let func_type = Type::Function {
                    params: param_types.clone(),
                    return_type: Box::new(return_type.clone()),
                };
                self.env.define(decl.name.clone(), func_type);

                // Now push inner scope for the body
                self.env.push_scope();

                // Define parameters
                for (param, ty) in decl.params.iter().zip(param_types.iter()) {
                    self.env.define(param.name.clone(), ty.clone());
                }

                self.env.set_return_type(Some(return_type));

                // Check body
                for s in &decl.body {
                    self.check_stmt(s)?;
                }

                self.env.set_return_type(None);
                self.env.pop_scope();
                Ok(())
            }

            StmtKind::Class(decl) => self.check_class_stmt(decl),

            StmtKind::Interface(_) => {
                // Already handled in first pass
                Ok(())
            }

            StmtKind::Import(_) => {
                // Import type checking is handled during module resolution
                Ok(())
            }

            StmtKind::Export(inner) => {
                // Check the inner statement
                self.check_stmt(inner)
            }

            StmtKind::Throw(value) => {
                // throw expressions can throw any type
                self.check_expr(value)?;
                Ok(())
            }

            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                self.check_stmt(try_block)?;

                for clause in catch_clauses {
                    if let Some(ref var_name) = clause.var_name {
                        self.env.push_scope();
                        self.env.define(var_name.clone(), Type::Any);
                        self.check_stmt(&clause.body)?;
                        self.env.pop_scope();
                    } else {
                        self.check_stmt(&clause.body)?;
                    }
                }

                if let Some(finally_blk) = finally_block {
                    self.check_stmt(finally_blk)?;
                }

                Ok(())
            }
        }
    }

    fn check_class_stmt(&mut self, decl: &ClassDecl) -> TypeResult<()> {
        self.env.set_current_class(Some(decl.name.clone()));

        // Check methods
        for method in &decl.methods {
            self.env.push_scope();

            // Define 'this'
            if let Some(class_type) = self.env.get_class(&decl.name) {
                self.env
                    .define("this".to_string(), Type::Class(class_type.clone()));
            }

            // Define parameters
            for param in &method.params {
                let ty = self.resolve_type(&param.type_annotation);
                self.env.define(param.name.clone(), ty);
            }

            // Set return type (Any if not annotated)
            let return_type = method
                .return_type
                .as_ref()
                .map(|t| self.resolve_type(t))
                .unwrap_or(Type::Any);
            self.env.set_return_type(Some(return_type));

            for s in &method.body {
                self.check_stmt(s)?;
            }

            self.env.set_return_type(None);
            self.env.pop_scope();
        }

        // Check constructor
        if let Some(ref ctor) = decl.constructor {
            self.env.push_scope();

            if let Some(class_type) = self.env.get_class(&decl.name) {
                self.env
                    .define("this".to_string(), Type::Class(class_type.clone()));
            }

            for param in &ctor.params {
                let ty = self.resolve_type(&param.type_annotation);
                self.env.define(param.name.clone(), ty);
            }

            self.env.set_return_type(Some(Type::Void));

            for s in &ctor.body {
                self.check_stmt(s)?;
            }

            self.env.set_return_type(None);
            self.env.pop_scope();
        }

        // Verify interface implementation
        self.check_interface_implementation(decl);

        self.env.set_current_class(None);
        Ok(())
    }

    fn check_interface_implementation(&mut self, decl: &ClassDecl) {
        for iface_name in &decl.interfaces {
            if let Some(iface) = self.env.get_interface(iface_name).cloned() {
                if let Some(class) = self.env.get_class(&decl.name) {
                    for (method_name, sig) in &iface.methods {
                        if let Some(method) = class.find_method(method_name) {
                            // Check signature compatibility
                            let method_params: Vec<Type> =
                                method.params.iter().map(|(_, t)| t.clone()).collect();
                            if method_params != sig.params || method.return_type != sig.return_type
                            {
                                self.errors.push(TypeError::General {
                                    message: format!(
                                        "method '{}' does not match interface signature",
                                        method_name
                                    ),
                                    span: decl.span,
                                });
                            }
                        } else {
                            self.errors.push(TypeError::General {
                                message: format!(
                                    "class '{}' does not implement method '{}' from interface '{}'",
                                    decl.name, method_name, iface_name
                                ),
                                span: decl.span,
                            });
                        }
                    }
                }
            }
        }
    }
}
