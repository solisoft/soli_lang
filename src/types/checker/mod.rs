//! Type checker for Solilang.

mod declarations;
mod expressions;
mod statements;

use crate::ast::*;
use crate::error::TypeError;
use crate::types::environment::TypeEnvironment;
use crate::types::type_repr::{ClassType, Type};

pub(crate) type TypeResult<T> = Result<T, TypeError>;

/// The type checker verifies type correctness of Solilang programs.
pub struct TypeChecker {
    pub(crate) env: TypeEnvironment,
    pub(crate) errors: Vec<TypeError>,
    /// Non-blocking diagnostics (e.g. enum match non-exhaustiveness). Surfaced
    /// by `soli check` but never fail the check or block execution.
    pub(crate) warnings: Vec<TypeError>,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut env = TypeEnvironment::new();
        // Register Model as a built-in class so subclasses can inherit from it.
        // Model's methods are native_static_methods resolved at runtime.
        env.define_class(ClassType::new("Model".to_string()));
        Self {
            env,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Type-check a program and return any non-blocking warnings collected
    /// during the pass (errors are returned via [`TypeChecker::check`]).
    pub fn check_collecting_warnings(
        &mut self,
        program: &Program,
    ) -> (Result<(), Vec<TypeError>>, Vec<TypeError>) {
        let result = self.check(program);
        (result, std::mem::take(&mut self.warnings))
    }

    /// Type check a complete program.
    pub fn check(&mut self, program: &Program) -> Result<(), Vec<TypeError>> {
        // First pass: collect all class and interface declarations
        for stmt in &program.statements {
            if let StmtKind::Class(decl) = &stmt.kind {
                self.declare_class(decl);
            } else if let StmtKind::Enum(decl) = &stmt.kind {
                // Register the enum under its lowered-class shape so member
                // access (`Status.Active`, `Status.Pending(...)`) resolves like
                // any class static. The variant set for exhaustiveness is
                // tracked separately (see `declare_enum`).
                self.declare_class(&decl.lower_to_class());
                self.declare_enum(decl);
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
