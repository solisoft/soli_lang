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
            } => {
                // First check if it's a known method
                if let Ok(method_type) = self.check_hash_method(&key_type, &value_type, name, span)
                {
                    return Ok(method_type);
                }
                // If not a method, treat as property access - return value_type
                // (hash.key returns the value type for any key)
                Ok((*value_type).clone())
            }
            Type::String => self.check_string_method(name, span),
            Type::Any | Type::Unknown => Ok(Type::Any),
            // Primitive types support methods via the OO method dispatch system
            Type::Int | Type::Float | Type::Bool | Type::Null => Ok(Type::Any),
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
            "empty?" | "include?" | "contains" => Ok(Type::Function {
                params: vec![inner_type.clone()],
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
            "length" | "get" | "pop" | "clear" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            "push" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Null),
            }),
            // Universal methods on all types
            "class" | "inspect" | "to_string" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "nil?" | "is_a?" | "blank?" | "present?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
            }),
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
            "length" | "len" | "empty?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Int),
            }),
            "keys" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Array(Box::new(key_type.clone()))),
            }),
            "values" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Array(Box::new(value_type.clone()))),
            }),
            "entries" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Array(Box::new(Type::Array(Box::new(Type::Any))))),
            }),
            "has_key" | "delete" => Ok(Type::Function {
                params: vec![key_type.clone()],
                return_type: Box::new(value_type.clone()),
            }),
            "merge" => Ok(Type::Function {
                params: vec![Type::Hash {
                    key_type: Box::new(key_type.clone()),
                    value_type: Box::new(value_type.clone()),
                }],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(key_type.clone()),
                    value_type: Box::new(value_type.clone()),
                }),
            }),
            "clear" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Null),
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
            // Universal methods on all types
            "class" | "inspect" | "to_string" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "nil?" | "is_a?" | "blank?" | "present?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
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
            "to_i" | "to_int" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Int),
            }),
            "to_f" | "to_float" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Float),
            }),
            "to_s" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "chr" | "insert" | "delete" | "substring" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            // Universal methods on all types
            "class" | "inspect" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "nil?" | "is_a?" | "blank?" | "present?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
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
