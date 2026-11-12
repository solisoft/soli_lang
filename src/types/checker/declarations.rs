//! Declaration type checking: classes, interfaces, and functions.

use crate::ast::*;
use crate::types::type_repr::{
    ClassType, EnumType, FieldInfo, InterfaceType, MethodInfo, MethodSignature, Type,
};

use super::TypeChecker;

impl TypeChecker {
    pub(crate) fn declare_class(&mut self, decl: &ClassDecl) {
        let mut class_type = ClassType::new(decl.name.clone());
        class_type.is_module = decl.is_module;
        class_type.decl_span = (!self.declaring_project).then_some(decl.span);
        class_type.type_params = decl.type_params.clone();
        class_type.constructor_params = decl.constructor.as_ref().map(|ctor| {
            ctor.params
                .iter()
                .filter(|p| !p.is_block_param)
                .map(|p| self.resolve_type(&p.type_annotation))
                .collect()
        });

        // Set superclass. A parent this pass never saw declared is recorded by
        // name rather than dropped: a served application loads every
        // auto-loaded directory into one environment, so the parent is usually
        // in a sibling file (`class UserPolicy < ApplicationPolicy`) or
        // installed by the framework (`class ApplicationController <
        // Controller`). Forgetting it made the class look like a root with
        // only its own members, and every inherited call read as
        // `Cannot access member`.
        if let Some(ref superclass_name) = decl.superclass {
            match self.env.get_class(superclass_name) {
                Some(super_class) => class_type.superclass = Some(Box::new(super_class.clone())),
                None => class_type.unresolved_superclass = Some(superclass_name.clone()),
            }
        }

        // Set interfaces
        class_type.interfaces = decl.interfaces.clone();

        // Add fields
        for field in &decl.fields {
            let ty = if let Some(ref ta) = field.type_annotation {
                self.resolve_type(ta)
            } else {
                Type::Any
            };
            class_type.fields.insert(
                field.name.clone(),
                FieldInfo {
                    name: field.name.clone(),
                    ty,
                    is_private: matches!(field.visibility, Visibility::Private),
                    is_protected: matches!(field.visibility, Visibility::Protected),
                    is_static: field.is_static,
                },
            );
        }

        // An instance variable a method or the constructor assigns is a
        // field, declared or not: both engines create it on the first
        // `@name = …`, and `obj.name` reads it from outside. Its type is
        // whatever was assigned, so `Any`.
        for name in assigned_instance_variables(decl) {
            class_type.fields.entry(name.clone()).or_insert(FieldInfo {
                name,
                ty: Type::Any,
                is_private: false,
                is_protected: false,
                is_static: false,
            });
        }

        // Add methods
        for method in &decl.methods {
            let params: Vec<(String, Type)> = method
                .params
                .iter()
                .map(|p| (p.name.clone(), self.resolve_type(&p.type_annotation)))
                .collect();
            // No annotation means unknown, not `Void`. Annotations are
            // optional in Soli and most application code omits them, so
            // defaulting to `Void` made every use of a returned value an
            // error: `poses + Job.pour_organisation(...)` read as "cannot add
            // Int and Void", and `this._porteur().nil?` as "cannot access
            // member on Void". Only an explicit `-> Void` means it returns
            // nothing. `check_class_stmt` already types the body this way;
            // this table disagreed with it.
            let return_type = method
                .return_type
                .as_ref()
                .map(|t| self.resolve_type(t))
                .unwrap_or(Type::Any);

            class_type.methods.insert(
                method.name.clone(),
                MethodInfo {
                    name: method.name.clone(),
                    params,
                    return_type,
                    is_private: matches!(method.visibility, Visibility::Private),
                    is_protected: matches!(method.visibility, Visibility::Protected),
                    is_static: method.is_static,
                },
            );
        }

        if self.job_classes.contains(&decl.name) {
            for name in JOB_FACADE {
                class_type
                    .methods
                    .entry(name.to_string())
                    .or_insert_with(|| MethodInfo {
                        name: name.to_string(),
                        params: Vec::new(),
                        return_type: Type::Any,
                        is_private: false,
                        is_protected: false,
                        is_static: true,
                    });
            }
        }

        self.env.define_class(class_type);
    }

    /// Merge included / extended module members into the classes that mix them
    /// in, after every declaration has been collected.
    ///
    /// Runs as its own pass so declaration order does not matter: a concern
    /// declared below the class that includes it still resolves. Nothing under
    /// `src/types/` read `ClassDecl.includes` before this, so the synthesized
    /// `ClassType` held only the class's own methods and `soli check` rejected
    /// `u.greet()` for `class User { include Greetable }`. `Model` subclasses
    /// were unaffected only because their members resolve as `Any` — which is
    /// why the model-concern path looked fine.
    pub(crate) fn apply_mixin_members(&mut self, decls: &[ClassDecl]) {
        use std::collections::HashMap as StdHashMap;
        let by_name: StdHashMap<&str, &ClassDecl> =
            decls.iter().map(|d| (d.name.as_str(), d)).collect();

        for decl in decls {
            if decl.includes.is_empty() && decl.extends.is_empty() {
                continue;
            }
            let Some(mut class_type) = self.env.get_class(&decl.name).cloned() else {
                continue;
            };

            // Instance methods from `include`, plus `class_methods do` blocks,
            // which become class methods on the includer.
            for module_name in collect_mixin_chain(&by_name, &decl.includes) {
                if let Some(module) = self.env.get_class(&module_name).cloned() {
                    for (name, info) in module.methods {
                        // The class's own methods win, matching the runtime.
                        class_type.methods.entry(name).or_insert(info);
                    }
                }
                if let Some(module_decl) = by_name.get(module_name.as_str()) {
                    for method in &module_decl.concern_class_methods {
                        class_type
                            .methods
                            .entry(method.name.clone())
                            .or_insert_with(|| self.method_info(method, true));
                    }
                }
            }

            // `extend` mixes a module's instance methods in as class methods.
            for module_name in collect_mixin_chain(&by_name, &decl.extends) {
                if let Some(module_decl) = by_name.get(module_name.as_str()) {
                    for method in &module_decl.methods {
                        class_type
                            .methods
                            .entry(method.name.clone())
                            .or_insert_with(|| self.method_info(method, true));
                    }
                }
            }

            self.env.define_class(class_type);
        }
    }

    /// `MethodInfo` for one declared method, with `is_static` forced.
    fn method_info(&mut self, method: &MethodDecl, is_static: bool) -> MethodInfo {
        let params: Vec<(String, Type)> = method
            .params
            .iter()
            .map(|p| (p.name.clone(), self.resolve_type(&p.type_annotation)))
            .collect();
        let return_type = method
            .return_type
            .as_ref()
            .map(|t| self.resolve_type(t))
            .unwrap_or(Type::Any);
        MethodInfo {
            name: method.name.clone(),
            params,
            return_type,
            is_private: matches!(method.visibility, Visibility::Private),
            is_protected: matches!(method.visibility, Visibility::Protected),
            is_static,
        }
    }

    /// Record an enum's variant set so `match` exhaustiveness can be checked.
    pub(crate) fn declare_enum(&mut self, decl: &EnumDecl) {
        let mut enum_type = EnumType::new(decl.name.clone());
        enum_type.variants = decl.variants.iter().map(|v| v.name.clone()).collect();
        self.env.define_enum(enum_type);
    }

    pub(crate) fn declare_interface(&mut self, decl: &InterfaceDecl) {
        let mut iface_type = InterfaceType::new(decl.name.clone());

        for method in &decl.methods {
            let params: Vec<Type> = method
                .params
                .iter()
                .map(|p| self.resolve_type(&p.type_annotation))
                .collect();
            let return_type = method
                .return_type
                .as_ref()
                .map(|t| self.resolve_type(t))
                .unwrap_or(Type::Any);

            iface_type.methods.insert(
                method.name.clone(),
                MethodSignature {
                    name: method.name.clone(),
                    params,
                    return_type,
                },
            );
        }

        self.env.define_interface(iface_type);
    }

    pub(crate) fn declare_function(&mut self, decl: &FunctionDecl) {
        let ty = self.function_type(decl);
        self.note_arity(decl, &ty);
        self.declared_functions.insert(decl.name.clone());
        self.env.define_function(decl.name.clone(), ty);
    }

    /// Remember how many arguments `decl` takes: its parameters without a
    /// default at least, all of them at most (a `&block` parameter is passed
    /// as a block, not counted).
    pub(crate) fn note_arity(&mut self, decl: &FunctionDecl, ty: &Type) {
        let params = decl.params.iter().filter(|p| !p.is_block_param);
        let max = params.clone().count();
        let min = params.filter(|p| p.default_value.is_none()).count();
        let at = (!self.declaring_project).then_some(decl.span);
        self.function_arities
            .insert(decl.name.clone(), (min, max, ty.clone(), at));
    }

    /// The signature of a top-level `def`, for a call to check against.
    pub(crate) fn function_type(&self, decl: &FunctionDecl) -> Type {
        let params: Vec<Type> = decl
            .params
            .iter()
            .map(|p| self.resolve_type(&p.type_annotation))
            .collect();
        let return_type = decl
            .return_type
            .as_ref()
            .map(|t| self.resolve_type(t))
            .unwrap_or(Type::Any);
        Type::Function {
            params,
            return_type: Box::new(return_type),
        }
    }
}

/// What `jobs::inject_facade_methods` adds to a job class.
const JOB_FACADE: [&str; 5] = [
    "perform_now",
    "perform_later",
    "perform_in",
    "perform_at",
    "schedule_cron",
];

/// The names of `@name = …` (and `this.name = …`, `@name ||= …`) in the
/// instance methods and constructor of `decl`.
fn assigned_instance_variables(decl: &ClassDecl) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut on_expr = |expr: &Expr| {
        let target = match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::CompoundAssign { target, .. } => target,
            _ => return,
        };
        if let ExprKind::Member { object, name } = &target.kind {
            if matches!(object.kind, ExprKind::This) && !names.contains(name) {
                names.push(name.clone());
            }
        }
    };
    let bodies = decl
        .methods
        .iter()
        .filter(|m| !m.is_static)
        .map(|m| &*m.body)
        .chain(decl.constructor.iter().map(|c| &*c.body));
    for body in bodies {
        crate::ast::walk::walk_stmts(body, &mut |_| {}, &mut on_expr);
    }
    names
}

/// Every module reachable from `roots`, innermost first, matching the order the
/// runtime mixes them in. Cycles are broken by the `seen` set — a module that
/// includes itself (directly or through a chain) is refused at runtime, but the
/// type checker must not hang on it either.
fn collect_mixin_chain(
    by_name: &std::collections::HashMap<&str, &ClassDecl>,
    roots: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    fn walk(
        by_name: &std::collections::HashMap<&str, &ClassDecl>,
        name: &str,
        seen: &mut std::collections::HashSet<String>,
        out: &mut Vec<String>,
    ) {
        if !seen.insert(name.to_string()) {
            return;
        }
        if let Some(decl) = by_name.get(name) {
            for inner in &decl.includes {
                walk(by_name, inner, seen, out);
            }
        }
        out.push(name.to_string());
    }
    for root in roots {
        walk(by_name, root, &mut seen, &mut out);
    }
    out
}
