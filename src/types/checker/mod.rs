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
    /// The top-level `def`s of the file being checked. The runtime installs
    /// them as globals, so a bare `name = value` anywhere in the file rebinds
    /// the function rather than creating a local (`let name = …` shadows).
    pub(crate) declared_functions: std::collections::HashSet<String>,
    /// For each `def` of the file or the project: the fewest and most
    /// arguments it takes, and the signature it was declared with (a call
    /// reaching another binding of the name is not checked against it).
    pub(crate) function_arities: std::collections::HashMap<String, (usize, usize, Type)>,
    /// Classes the job loader gives `perform_later` and the rest of the
    /// facade (see [`TypeChecker::declare_job_classes`]).
    pub(crate) job_classes: std::collections::HashSet<String>,
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
            declared_functions: std::collections::HashSet::new(),
            function_arities: std::collections::HashMap::new(),
            job_classes: std::collections::HashSet::new(),
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

    /// Name the classes of `app/jobs/*_job.sl`: the loader gives each the
    /// static `perform_now`, `perform_later`, `perform_in`, `perform_at` and
    /// `schedule_cron` it does not define itself. Call it before the classes
    /// are declared.
    pub fn declare_job_classes<I: IntoIterator<Item = String>>(&mut self, names: I) {
        self.job_classes.extend(names);
    }

    /// Declare what the application's other files declare at the top level,
    /// with their real signatures (#693): a sibling controller's helper is
    /// checked for arity and argument types, a service class for its members,
    /// and `class UserPolicy < ApplicationPolicy` resolves its parent across
    /// files instead of answering every member with `Any`.
    ///
    /// `statements` are the top-level classes, modules, enums, interfaces and
    /// `def`s of the project, a name declared once (see
    /// `crate::check::Project`). Model subclasses keep their schemaless
    /// fields as `Any` (`ClassType::extends_model`); a class whose ancestry
    /// leaves the project (`< Controller`) keeps `extends_unknown`. Call it
    /// after [`TypeChecker::declare_ambient`]; the checked file's own
    /// declarations, declared by [`TypeChecker::check`], still win.
    pub fn declare_project(&mut self, statements: &[Stmt]) {
        let mut classes: Vec<ClassDecl> = Vec::new();
        for stmt in statements {
            match &stmt.kind {
                StmtKind::Interface(decl) => self.declare_interface(decl),
                StmtKind::Class(decl) => classes.push((**decl).clone()),
                StmtKind::Enum(decl) => {
                    self.declare_class(&decl.lower_to_class());
                    self.declare_enum(decl);
                }
                _ => {}
            }
        }
        // Parents before children, since a class keeps a copy of its parent;
        // twice, so a signature naming a class declared later resolves.
        let ordered = parents_first(&classes);
        for _ in 0..2 {
            for decl in &ordered {
                self.declare_class(decl);
            }
        }
        self.apply_mixin_members(&classes);
        for stmt in statements {
            if let StmtKind::Function(decl) = &stmt.kind {
                let ty = self.function_type(decl);
                self.note_arity(decl, &ty);
                self.env.define_function(decl.name.clone(), ty);
            }
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
    /// `req`, `params`, `session`, `cookies`, `current_user` are
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

    /// Declare what the migration runner puts around a migration's `up` and
    /// `down`: the `_db` connection, its host and name, and `MigrationDb`.
    pub fn declare_migration_scope(&mut self) {
        for name in crate::types::runtime_globals::migration_globals() {
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

    /// Declare the widget catalogue for a script that opens `eui_window`.
    ///
    /// The view and handler run in workers that load the catalogue (`column`,
    /// `button`, `tw`…), so the calls are real, but the checker had never heard
    /// of them: `b = button("x", "go")`, or a helper returning one, stopped the
    /// script with `Undefined variable 'button'` — while the same call written
    /// inline in a list went through. Declared like the ambient names of an
    /// application (`Any`, a definition of the script still wins), and only
    /// here: in any other script the catalogue does not exist.
    #[cfg(feature = "eui")]
    fn declare_eui_window_catalogue(&mut self, program: &Program) {
        if crate::serve::eui::script::mentions_eui_window(program) {
            self.declare_ambient(crate::scaffold::templates::eui::catalogue_def_names());
        }
    }

    /// Type check a complete program.
    pub fn check(&mut self, program: &Program) -> Result<(), Vec<TypeError>> {
        #[cfg(feature = "eui")]
        self.declare_eui_window_catalogue(program);

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

        // Second pass: check all declarations. Same recovery as a body: a
        // top-level `x = broken(…)` still binds `x`, so the script's later
        // lines do not each report "Undefined variable 'x'".
        self.check_body(&program.statements);

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
            // `Int?` takes `nil` as well as an `Int` wherever it is stored;
            // dropping the `?` refused `f(nil)` for `def f(x: Int?)`.
            TypeKind::Nullable(inner) => match self.resolve_type(inner) {
                inner @ (Type::Nullable(_) | Type::Any | Type::Unknown) => inner,
                inner => Type::Nullable(Box::new(inner)),
            },
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

/// `classes` ordered so a superclass declared among them comes before its
/// subclasses; a cycle (refused at run time) keeps declaration order.
fn parents_first(classes: &[ClassDecl]) -> Vec<&ClassDecl> {
    use std::collections::{HashMap, HashSet};
    let by_name: HashMap<&str, &ClassDecl> = classes.iter().map(|d| (d.name.as_str(), d)).collect();
    let mut placed: HashSet<&str> = HashSet::new();
    let mut out = Vec::with_capacity(classes.len());
    for decl in classes {
        let mut chain = vec![decl];
        let mut seen: HashSet<&str> = HashSet::from([decl.name.as_str()]);
        let mut current = decl;
        while let Some(parent) = current
            .superclass
            .as_deref()
            .and_then(|name| by_name.get(name))
        {
            if !seen.insert(parent.name.as_str()) || placed.contains(parent.name.as_str()) {
                break;
            }
            chain.push(parent);
            current = parent;
        }
        for decl in chain.into_iter().rev() {
            if placed.insert(decl.name.as_str()) {
                out.push(decl);
            }
        }
    }
    out
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(test, feature = "eui"))]
mod eui_window_tests {
    fn check(source: &str) -> Result<(), String> {
        crate::type_check_source_with_ambient(source, None, &[])
            .map(|_| ())
            .map_err(|errs| {
                errs.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
    }

    #[test]
    fn a_window_script_can_call_the_catalogue_anywhere() {
        let source = r#"
def make_button()
  button("y", "go")
end

def handler(event_data)
  {}
end

def view(state)
  label = button("x", "go")
  column({}, [label, make_button()])
end

eui_window("t", "handler", "view", {"title": "t"})
"#;
        let result = check(source);
        assert!(result.is_ok(), "{:?}", result);
    }

    #[test]
    fn a_script_without_a_window_does_not_get_the_catalogue() {
        let source = "def view()\n  label = button(\"x\", \"go\")\n  label\nend\n";
        assert!(check(source).is_err());
    }
}
