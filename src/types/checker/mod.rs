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

    /// Declare names that exist at run time but are not in this file.
    ///
    /// `serve` loads `app/controllers/`, `app/models/`, `app/services/` and
    /// `stdlib/` into **one** environment, so a helper declared in one file is
    /// callable from its neighbours with no import — which is what the EUI
    /// catalogue's header means by "one namespace either way". The checker sees
    /// one file at a time, so every such call read as `Undefined variable`: a
    /// freshly scaffolded `--eui` application reported 137 of them, all naming
    /// five functions declared in a sibling file.
    ///
    /// They are declared as `Type::Any` deliberately. What is known here is
    /// that the name will exist; what it *is* would mean checking the file that
    /// declares it, which is a whole-project pass and a bigger change. `Any`
    /// removes the false error without inventing a type, and a name the file
    /// does declare itself still wins — this only fills gaps.
    pub fn declare_ambient<I, S>(&mut self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in names {
            self.env
                .define_function(name.as_ref().to_string(), Type::Any);
        }
    }

    /// Declare the test DSL, for a file `soli test` would run.
    ///
    /// `describe`, `test`, `expect`, `as_guest`, the factories and the browser
    /// verbs exist only when the runtime is about to run tests:
    /// `register_builtins` is called with the flag off when serving.
    /// Seeding them everywhere would accept a spec-shaped call in a controller
    /// that cannot run it, so the caller decides, by path — and a spec that
    /// calls them stops reporting one error per file (the outer `describe`
    /// failed, and its whole body went unchecked behind it).
    pub fn declare_test_dsl(&mut self) {
        for name in crate::types::runtime_globals::test_dsl_globals() {
            if self.env.get(name).is_none() {
                self.env.define(name.clone(), Type::Any);
            }
        }
    }

    /// Declare the names that only exist while a request is served, for a file
    /// that belongs to an application.
    ///
    /// `req`, `params`, `session`, `cookies`, `flash`, `current_user` are
    /// injected into the handler's scope by the server; `render` and
    /// `redirect` are registered builtins deliberately kept out of the general
    /// namespace, because a standalone script calling them cannot work. A
    /// controller, a helper, a middleware and a job are that file; a loose
    /// script is not, and still gets `Undefined variable 'render'`.
    pub fn declare_request_scope(&mut self) {
        for name in crate::types::runtime_globals::request_scope_globals() {
            if self.env.get(name).is_none() {
                self.env.define((*name).to_string(), Type::Any);
            }
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

        // Between the passes: fold `include` / `extend` members into the classes
        // that mix them in. Its own pass so declaration order does not matter.
        let class_decls: Vec<ClassDecl> = program
            .statements
            .iter()
            .filter_map(|stmt| match &stmt.kind {
                StmtKind::Class(decl) => Some((**decl).clone()),
                _ => None,
            })
            .collect();
        self.apply_mixin_members(&class_decls);

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
