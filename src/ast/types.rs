//! Type annotation AST nodes.

use crate::span::Span;

/// A type annotation in the source code.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeAnnotation {
    pub kind: TypeKind,
    pub span: Span,
}

impl TypeAnnotation {
    pub fn new(kind: TypeKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// The kinds of types that can be expressed in source.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// Primitive types: Int, Float, Bool, String
    Named(String),
    /// Void type (for functions that don't return a value)
    Void,
    /// Array type: Type[]
    Array(Box<TypeAnnotation>),
    /// Hash type: Hash<K, V>
    Hash {
        key_type: Box<TypeAnnotation>,
        value_type: Box<TypeAnnotation>,
    },
    /// Function type: (A, B) -> C
    Function {
        params: Vec<TypeAnnotation>,
        return_type: Box<TypeAnnotation>,
    },
    /// Nullable type: Type?
    Nullable(Box<TypeAnnotation>),
}

impl std::fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            TypeKind::Named(name) => write!(f, "{}", name),
            TypeKind::Void => write!(f, "Void"),
            TypeKind::Array(inner) => write!(f, "{}[]", inner),
            TypeKind::Hash {
                key_type,
                value_type,
            } => write!(f, "Hash<{}, {}>", key_type, value_type),
            TypeKind::Function {
                params,
                return_type,
            } => {
                write!(f, "(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param)?;
                }
                write!(f, ") -> {}", return_type)
            }
            TypeKind::Nullable(inner) => write!(f, "{}?", inner),
        }
    }
}
