//! Type annotation AST nodes.

use crate::span::Span;

/// A type annotation in the source code.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::new(0, 1, 1, 1)
    }

    fn ann(kind: TypeKind) -> TypeAnnotation {
        TypeAnnotation::new(kind, span())
    }

    fn named(name: &str) -> TypeAnnotation {
        ann(TypeKind::Named(name.to_string()))
    }

    #[test]
    fn new_stores_kind_and_span() {
        let s = Span::new(2, 5, 3, 4);
        let a = TypeAnnotation::new(TypeKind::Void, s);
        assert_eq!(a.kind, TypeKind::Void);
        assert_eq!(a.span, s);
    }

    #[test]
    fn display_named_uses_name() {
        assert_eq!(named("Int").to_string(), "Int");
        assert_eq!(named("MyClass").to_string(), "MyClass");
    }

    #[test]
    fn display_void() {
        assert_eq!(ann(TypeKind::Void).to_string(), "Void");
    }

    #[test]
    fn display_array_appends_brackets() {
        let arr = ann(TypeKind::Array(Box::new(named("Int"))));
        assert_eq!(arr.to_string(), "Int[]");
    }

    #[test]
    fn display_array_nests() {
        let inner = ann(TypeKind::Array(Box::new(named("Float"))));
        let outer = ann(TypeKind::Array(Box::new(inner)));
        assert_eq!(outer.to_string(), "Float[][]");
    }

    #[test]
    fn display_hash_lists_both_types() {
        let h = ann(TypeKind::Hash {
            key_type: Box::new(named("String")),
            value_type: Box::new(named("Int")),
        });
        assert_eq!(h.to_string(), "Hash<String, Int>");
    }

    #[test]
    fn display_function_with_params_and_return() {
        let f = ann(TypeKind::Function {
            params: vec![named("Int"), named("String")],
            return_type: Box::new(named("Bool")),
        });
        assert_eq!(f.to_string(), "(Int, String) -> Bool");
    }

    #[test]
    fn display_function_nullary_keeps_empty_parens() {
        let f = ann(TypeKind::Function {
            params: vec![],
            return_type: Box::new(ann(TypeKind::Void)),
        });
        assert_eq!(f.to_string(), "() -> Void");
    }

    #[test]
    fn display_nullable_appends_question_mark() {
        let n = ann(TypeKind::Nullable(Box::new(named("String"))));
        assert_eq!(n.to_string(), "String?");
    }

    #[test]
    fn display_nullable_array_nests_correctly() {
        // (Int[])? — array first, then nullable wrap.
        let arr = ann(TypeKind::Array(Box::new(named("Int"))));
        let nullable = ann(TypeKind::Nullable(Box::new(arr)));
        assert_eq!(nullable.to_string(), "Int[]?");
    }

    #[test]
    fn display_function_returning_hash() {
        let ret = ann(TypeKind::Hash {
            key_type: Box::new(named("String")),
            value_type: Box::new(named("Int")),
        });
        let f = ann(TypeKind::Function {
            params: vec![named("Int")],
            return_type: Box::new(ret),
        });
        assert_eq!(f.to_string(), "(Int) -> Hash<String, Int>");
    }
}
