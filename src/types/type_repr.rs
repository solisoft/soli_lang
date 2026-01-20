//! Internal type representation for the type checker.

use std::collections::HashMap;
use std::fmt;

/// Internal representation of types in the type system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// Primitive integer type
    Int,
    /// Primitive float type
    Float,
    /// Primitive boolean type
    Bool,
    /// Primitive string type
    String,
    /// Void type (no value)
    Void,
    /// Null type
    Null,
    /// Array type
    Array(Box<Type>),
    /// Hash/Map type
    Hash {
        key_type: Box<Type>,
        value_type: Box<Type>,
    },
    /// Function type
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
    },
    /// Class type
    Class(ClassType),
    /// Interface type
    Interface(InterfaceType),
    /// Type variable (for inference)
    Var(u32),
    /// Unknown type (error recovery)
    Unknown,
    /// Any type (escape hatch)
    Any,
}

impl Type {
    pub fn is_numeric(&self) -> bool {
        matches!(self, Type::Int | Type::Float)
    }

    pub fn is_primitive(&self) -> bool {
        matches!(self, Type::Int | Type::Float | Type::Bool | Type::String)
    }

    /// Check if this type is assignable to another type.
    pub fn is_assignable_to(&self, target: &Type) -> bool {
        if self == target {
            return true;
        }

        match (self, target) {
            // Any is assignable to/from anything
            (Type::Any, _) | (_, Type::Any) => true,
            // Unknown is assignable to/from anything (for error recovery)
            (Type::Unknown, _) | (_, Type::Unknown) => true,
            // Null is assignable to class and interface types
            (Type::Null, Type::Class(_)) | (Type::Null, Type::Interface(_)) => true,
            // Int can be assigned to Float (widening)
            (Type::Int, Type::Float) => true,
            // Array covariance
            (Type::Array(a), Type::Array(b)) => a.is_assignable_to(b),
            // Hash covariance
            (
                Type::Hash {
                    key_type: k1,
                    value_type: v1,
                },
                Type::Hash {
                    key_type: k2,
                    value_type: v2,
                },
            ) => k1.is_assignable_to(k2) && v1.is_assignable_to(v2),
            // Function contravariance in params, covariance in return
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                },
            ) => {
                if p1.len() != p2.len() {
                    return false;
                }
                // Contravariant in parameters
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    if !param2.is_assignable_to(param1) {
                        return false;
                    }
                }
                // Covariant in return type
                r1.is_assignable_to(r2)
            }
            // Class subtyping
            (Type::Class(sub), Type::Class(super_)) => {
                if sub.name == super_.name {
                    return true;
                }
                if let Some(ref parent) = sub.superclass {
                    return Type::Class(*parent.clone()).is_assignable_to(target);
                }
                false
            }
            // Class to interface
            (Type::Class(class), Type::Interface(iface)) => class.interfaces.contains(&iface.name),
            _ => false,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Float"),
            Type::Bool => write!(f, "Bool"),
            Type::String => write!(f, "String"),
            Type::Void => write!(f, "Void"),
            Type::Null => write!(f, "Null"),
            Type::Array(inner) => write!(f, "{}[]", inner),
            Type::Hash {
                key_type,
                value_type,
            } => write!(f, "Hash<{}, {}>", key_type, value_type),
            Type::Function {
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
            Type::Class(class) => write!(f, "{}", class.name),
            Type::Interface(iface) => write!(f, "{}", iface.name),
            Type::Var(id) => write!(f, "?T{}", id),
            Type::Unknown => write!(f, "unknown"),
            Type::Any => write!(f, "Any"),
        }
    }
}

/// Class type information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassType {
    pub name: String,
    pub superclass: Option<Box<ClassType>>,
    pub interfaces: Vec<String>,
    pub fields: HashMap<String, FieldInfo>,
    pub methods: HashMap<String, MethodInfo>,
}

impl ClassType {
    pub fn new(name: String) -> Self {
        Self {
            name,
            superclass: None,
            interfaces: Vec::new(),
            fields: HashMap::new(),
            methods: HashMap::new(),
        }
    }

    pub fn find_field(&self, name: &str) -> Option<&FieldInfo> {
        if let Some(field) = self.fields.get(name) {
            return Some(field);
        }
        if let Some(ref super_) = self.superclass {
            return super_.find_field(name);
        }
        None
    }

    pub fn find_method(&self, name: &str) -> Option<&MethodInfo> {
        if let Some(method) = self.methods.get(name) {
            return Some(method);
        }
        if let Some(ref super_) = self.superclass {
            return super_.find_method(name);
        }
        None
    }
}

/// Field information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Type,
    pub is_private: bool,
    pub is_static: bool,
}

/// Method information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodInfo {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub return_type: Type,
    pub is_private: bool,
    pub is_static: bool,
}

/// Interface type information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceType {
    pub name: String,
    pub methods: HashMap<String, MethodSignature>,
}

impl InterfaceType {
    pub fn new(name: String) -> Self {
        Self {
            name,
            methods: HashMap::new(),
        }
    }
}

/// Method signature (for interfaces).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodSignature {
    pub name: String,
    pub params: Vec<Type>,
    pub return_type: Type,
}
