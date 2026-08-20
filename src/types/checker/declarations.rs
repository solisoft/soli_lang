//! Declaration type checking: classes, interfaces, and functions.

use crate::ast::*;
use crate::types::type_repr::{
    ClassType, EnumType, FieldInfo, InterfaceType, MethodInfo, MethodSignature, Type,
};

use super::TypeChecker;

impl TypeChecker {
    pub(crate) fn declare_class(&mut self, decl: &ClassDecl) {
        let mut class_type = ClassType::new(decl.name.clone());

        // Set superclass
        if let Some(ref superclass_name) = decl.superclass {
            if let Some(super_class) = self.env.get_class(superclass_name) {
                class_type.superclass = Some(Box::new(super_class.clone()));
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
                    is_static: field.is_static,
                },
            );
        }

        // Add methods
        for method in &decl.methods {
            let params: Vec<(String, Type)> = method
                .params
                .iter()
                .map(|p| (p.name.clone(), self.resolve_type(&p.type_annotation)))
                .collect();
            let return_type = method
                .return_type
                .as_ref()
                .map(|t| self.resolve_type(t))
                .unwrap_or(Type::Void);

            class_type.methods.insert(
                method.name.clone(),
                MethodInfo {
                    name: method.name.clone(),
                    params,
                    return_type,
                    is_private: matches!(method.visibility, Visibility::Private),
                    is_static: method.is_static,
                },
            );
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
            .unwrap_or(Type::Void);
        MethodInfo {
            name: method.name.clone(),
            params,
            return_type,
            is_private: matches!(method.visibility, Visibility::Private),
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
                .unwrap_or(Type::Void);

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
        let params: Vec<Type> = decl
            .params
            .iter()
            .map(|p| self.resolve_type(&p.type_annotation))
            .collect();
        let return_type = decl
            .return_type
            .as_ref()
            .map(|t| self.resolve_type(t))
            .unwrap_or(Type::Void);

        self.env.define_function(
            decl.name.clone(),
            Type::Function {
                params,
                return_type: Box::new(return_type),
            },
        );
    }
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
