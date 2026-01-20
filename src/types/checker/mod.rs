//! Type checker for Solilang.

mod declarations;
mod expressions;
mod statements;

use crate::ast::*;
use crate::error::TypeError;
use crate::types::environment::TypeEnvironment;
use crate::types::type_repr::Type;

pub(crate) type TypeResult<T> = Result<T, TypeError>;

/// The type checker verifies type correctness of Solilang programs.
pub struct TypeChecker {
    pub(crate) env: TypeEnvironment,
    pub(crate) errors: Vec<TypeError>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            env: TypeEnvironment::new(),
            errors: Vec::new(),
        }
    }

    /// Type check a complete program.
    pub fn check(&mut self, program: &Program) -> Result<(), Vec<TypeError>> {
        // First pass: collect all class and interface declarations
        for stmt in &program.statements {
            if let StmtKind::Class(decl) = &stmt.kind {
                self.declare_class(decl);
            } else if let StmtKind::Interface(decl) = &stmt.kind {
                self.declare_interface(decl);
            } else if let StmtKind::Function(decl) = &stmt.kind {
                self.declare_function(decl);
            }
        }

        // Second pass: check all declarations
        for stmt in &program.statements {
            if let Err(e) = self.check_stmt(stmt) {
                self.errors.push(e);
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    pub(crate) fn resolve_type(&self, annotation: &TypeAnnotation) -> Type {
        match &annotation.kind {
            TypeKind::Named(name) => match name.as_str() {
                "Int" => Type::Int,
                "Float" => Type::Float,
                "Bool" => Type::Bool,
                "String" => Type::String,
                "Any" => Type::Any,
                _ => {
                    if let Some(class) = self.env.get_class(name) {
                        Type::Class(class.clone())
                    } else if let Some(iface) = self.env.get_interface(name) {
                        Type::Interface(iface.clone())
                    } else {
                        Type::Unknown
                    }
                }
            },
            TypeKind::Void => Type::Void,
            TypeKind::Array(inner) => Type::Array(Box::new(self.resolve_type(inner))),
            TypeKind::Function {
                params,
                return_type,
            } => Type::Function {
                params: params.iter().map(|p| self.resolve_type(p)).collect(),
                return_type: Box::new(self.resolve_type(return_type)),
            },
            TypeKind::Nullable(inner) => {
                // For now, treat nullable as the inner type (simplification)
                self.resolve_type(inner)
            }
            TypeKind::Hash {
                key_type,
                value_type,
            } => Type::Hash {
                key_type: Box::new(self.resolve_type(key_type)),
                value_type: Box::new(self.resolve_type(value_type)),
            },
        }
    }
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}
