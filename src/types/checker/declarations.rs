//! Declaration type checking: classes, interfaces, and functions.

use crate::ast::*;
use crate::types::type_repr::{
    ClassType, FieldInfo, InterfaceType, MethodInfo, MethodSignature, Type,
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
            let ty = self.resolve_type(&field.type_annotation);
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
