//! Member/index access type checking.

use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check member access expression.
    pub(crate) fn check_member_expr(
        &mut self,
        span: Span,
        object: &Expr,
        name: &str,
    ) -> TypeResult<Type> {
        let obj_type = self.check_expr(object)?;

        match obj_type {
            Type::Future(_inner) => Ok(Type::Any),
            Type::Class(class) => {
                // Look up the class in the environment to get the full definition with methods
                let class_def = self.env.get_class(&class.name);
                if let Some(class_def) = class_def {
                    if let Some(field) = class_def.find_field(name) {
                        return Ok(field.ty.clone());
                    }
                    if let Some(method) = class_def.find_method(name) {
                        return Ok(Type::Function {
                            params: method.params.iter().map(|(_, t)| t.clone()).collect(),
                            return_type: Box::new(method.return_type.clone()),
                        });
                    }
                }
                Err(TypeError::NoSuchMember {
                    type_name: class.name,
                    member: name.to_string(),
                    span,
                })
            }
            Type::Array(inner_type) => self.check_array_method(&inner_type, name, span),
            Type::Hash {
                key_type,
                value_type,
            } => self.check_hash_method(&key_type, &value_type, name, span),
            Type::String => self.check_string_method(name, span),
            Type::Any | Type::Unknown => Ok(Type::Any),
            _ => Err(TypeError::NoSuchMember {
                type_name: format!("{}", obj_type),
                member: name.to_string(),
                span,
            }),
        }
    }

    /// Check array method access.
    fn check_array_method(&self, inner_type: &Type, name: &str, span: Span) -> TypeResult<Type> {
        match name {
            "map" | "filter" | "each" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "reduce" | "find" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "any?" | "all?" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Bool),
            }),
            "sort" | "reverse" | "uniq" | "compact" | "flatten" | "shuffle" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "sort_by" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "first" | "last" | "sample" | "min" | "max" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(inner_type.clone()),
            }),
            "empty?" | "include?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
            }),
            "take" | "drop" => Ok(Type::Function {
                params: vec![Type::Int],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "zip" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(Type::Array(Box::new(Type::Any))))),
            }),
            "sum" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Float),
            }),
            "join" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            }),
            "length" | "get" | "push" | "pop" | "clear" => {
                // These are built-in methods with various signatures
                // For simplicity, return Any
                Ok(Type::Function {
                    params: vec![],
                    return_type: Box::new(Type::Any),
                })
            }
            _ => Err(TypeError::NoSuchMember {
                type_name: format!("{}[]", inner_type),
                member: name.to_string(),
                span,
            }),
        }
    }

    /// Check hash method access.
    fn check_hash_method(
        &self,
        key_type: &Type,
        value_type: &Type,
        name: &str,
        span: Span,
    ) -> TypeResult<Type> {
        match name {
            "length" | "empty?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Int),
            }),
            "map" | "filter" | "each" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "get" | "fetch" => Ok(Type::Function {
                params: vec![key_type.clone()],
                return_type: Box::new(value_type.clone()),
            }),
            "dig" => Ok(Type::Function {
                params: vec![Type::Any], // variadic - accepts multiple keys
                return_type: Box::new(value_type.clone()),
            }),
            "invert" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(value_type.clone()),
                    value_type: Box::new(key_type.clone()),
                }),
            }),
            "transform_values" | "transform_keys" | "select" | "reject" | "slice" | "except"
            | "compact" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(key_type.clone()),
                    value_type: Box::new(value_type.clone()),
                }),
            }),
            _ => Err(TypeError::NoSuchMember {
                type_name: format!("Hash({}, {})", key_type, value_type),
                member: name.to_string(),
                span,
            }),
        }
    }

    /// Check string method access.
    fn check_string_method(&self, name: &str, span: Span) -> TypeResult<Type> {
        match name {
            "length" | "len" | "count" | "ord" | "bytesize" | "index_of" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Int),
            }),
            "starts_with?" | "ends_with?" | "empty?" | "include?" | "contains" | "starts_with"
            | "ends_with" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Bool),
            }),
            "chomp" | "lstrip" | "rstrip" | "squeeze" | "capitalize" | "swapcase" | "reverse"
            | "delete_prefix" | "delete_suffix" | "to_string" | "upcase" | "downcase" | "trim"
            | "join" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "gsub" | "sub" | "tr" | "replace" => Ok(Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::String),
            }),
            "match" | "hex" | "oct" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            "scan" | "chars" | "lines" | "bytes" | "split" | "partition" | "rpartition" => {
                Ok(Type::Function {
                    params: vec![],
                    return_type: Box::new(Type::Array(Box::new(Type::String))),
                })
            }
            "center" | "ljust" | "rjust" | "truncate" | "lpad" | "rpad" => Ok(Type::Function {
                params: vec![Type::Int],
                return_type: Box::new(Type::String),
            }),
            "chr" | "insert" | "delete" | "substring" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            _ => Err(TypeError::NoSuchMember {
                type_name: "String".to_string(),
                member: name.to_string(),
                span,
            }),
        }
    }

    /// Check index expression.
    pub(crate) fn check_index_expr(
        &mut self,
        span: Span,
        object: &Expr,
        index: &Expr,
    ) -> TypeResult<Type> {
        let obj_type = self.check_expr(object)?;
        let idx_type = self.check_expr(index)?;

        match &obj_type {
            Type::Array(inner) => {
                if !matches!(idx_type, Type::Int | Type::Any | Type::Unknown) {
                    return Err(TypeError::mismatch(
                        "Int",
                        format!("{}", idx_type),
                        index.span,
                    ));
                }
                Ok(*inner.clone())
            }
            Type::String => {
                if !matches!(idx_type, Type::Int | Type::Any | Type::Unknown) {
                    return Err(TypeError::mismatch(
                        "Int",
                        format!("{}", idx_type),
                        index.span,
                    ));
                }
                Ok(Type::String)
            }
            Type::Hash {
                key_type,
                value_type,
            } => {
                if !idx_type.is_assignable_to(key_type) {
                    return Err(TypeError::mismatch(
                        format!("{}", key_type),
                        format!("{}", idx_type),
                        index.span,
                    ));
                }
                Ok(*value_type.clone())
            }
            Type::Any | Type::Unknown => Ok(Type::Any),
            _ => Err(TypeError::General {
                message: format!("cannot index {}", obj_type),
                span,
            }),
        }
    }
}
