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
    /// Decimal type with precision (number of decimal places)
    Decimal(u32),
    /// Primitive boolean type
    Bool,
    /// Primitive string type
    String,
    /// Symbol type
    Symbol,
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
    /// Future type (async result)
    Future(Box<Type>),
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
        matches!(self, Type::Int | Type::Float | Type::Decimal(_))
    }

    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            Type::Int | Type::Float | Type::Decimal(_) | Type::Bool | Type::String | Type::Symbol
        )
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
            Type::Decimal(precision) => write!(f, "Decimal({})", precision),
            Type::Bool => write!(f, "Bool"),
            Type::String => write!(f, "String"),
            Type::Symbol => write!(f, "Symbol"),
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
            Type::Future(inner) => write!(f, "Future<{}>", inner),
            Type::Class(class) => write!(f, "{}", class.name),
            Type::Interface(iface) => write!(f, "{}", iface.name),
            Type::Var(id) => write!(f, "?T{}", id),
            Type::Unknown => write!(f, "unknown"),
            Type::Any => write!(f, "Any"),
        }
    }
}

/// Enum type information — the ordered set of variant names, used for `match`
/// exhaustiveness checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumType {
    pub name: String,
    pub variants: Vec<String>,
}

impl EnumType {
    pub fn new(name: String) -> Self {
        Self {
            name,
            variants: Vec::new(),
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

    /// Check if this class extends Model (directly or transitively).
    pub fn extends_model(&self) -> bool {
        if self.name == "Model" {
            return true;
        }
        if let Some(ref super_) = self.superclass {
            return super_.name == "Model" || super_.extends_model();
        }
        false
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

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str) -> ClassType {
        ClassType::new(name.to_string())
    }

    fn class_extending(name: &str, parent: ClassType) -> ClassType {
        let mut c = ClassType::new(name.to_string());
        c.superclass = Some(Box::new(parent));
        c
    }

    fn iface(name: &str) -> InterfaceType {
        InterfaceType::new(name.to_string())
    }

    // ---------- is_numeric / is_primitive ----------

    #[test]
    fn is_numeric_covers_int_float_decimal() {
        assert!(Type::Int.is_numeric());
        assert!(Type::Float.is_numeric());
        assert!(Type::Decimal(2).is_numeric());
    }

    #[test]
    fn is_numeric_rejects_non_numeric() {
        assert!(!Type::Bool.is_numeric());
        assert!(!Type::String.is_numeric());
        assert!(!Type::Symbol.is_numeric());
        assert!(!Type::Null.is_numeric());
        assert!(!Type::Any.is_numeric());
        assert!(!Type::Unknown.is_numeric());
        assert!(!Type::Array(Box::new(Type::Int)).is_numeric());
    }

    #[test]
    fn is_primitive_covers_primitives() {
        assert!(Type::Int.is_primitive());
        assert!(Type::Float.is_primitive());
        assert!(Type::Decimal(0).is_primitive());
        assert!(Type::Bool.is_primitive());
        assert!(Type::String.is_primitive());
        assert!(Type::Symbol.is_primitive());
    }

    #[test]
    fn is_primitive_rejects_compound_and_special() {
        assert!(!Type::Void.is_primitive());
        assert!(!Type::Null.is_primitive());
        assert!(!Type::Any.is_primitive());
        assert!(!Type::Unknown.is_primitive());
        assert!(!Type::Array(Box::new(Type::Int)).is_primitive());
        assert!(!Type::Class(class("Foo")).is_primitive());
        assert!(!Type::Interface(iface("Bar")).is_primitive());
    }

    // ---------- is_assignable_to: identity / Any / Unknown ----------

    #[test]
    fn assignable_identity() {
        assert!(Type::Int.is_assignable_to(&Type::Int));
        assert!(Type::String.is_assignable_to(&Type::String));
    }

    #[test]
    fn assignable_any_is_bidirectional() {
        assert!(Type::Any.is_assignable_to(&Type::Int));
        assert!(Type::Int.is_assignable_to(&Type::Any));
        assert!(Type::Any.is_assignable_to(&Type::Class(class("Foo"))));
        assert!(Type::Class(class("Foo")).is_assignable_to(&Type::Any));
    }

    #[test]
    fn assignable_unknown_is_bidirectional() {
        // Documents the current (debatable) behavior: Unknown silently
        // accepts everything, which is what the type-checker integration
        // tests' bug_unknown_type_name_in_annotation_is_silently_accepted
        // case relies on.
        assert!(Type::Unknown.is_assignable_to(&Type::Int));
        assert!(Type::Int.is_assignable_to(&Type::Unknown));
    }

    // ---------- is_assignable_to: numeric widening ----------

    #[test]
    fn assignable_int_to_float_widens() {
        assert!(Type::Int.is_assignable_to(&Type::Float));
    }

    #[test]
    fn assignable_float_to_int_does_not_narrow() {
        assert!(!Type::Float.is_assignable_to(&Type::Int));
    }

    #[test]
    fn assignable_decimal_does_not_widen_to_float() {
        // Decimal/Float interop is *not* covered by widening.
        assert!(!Type::Decimal(2).is_assignable_to(&Type::Float));
        assert!(!Type::Float.is_assignable_to(&Type::Decimal(2)));
    }

    #[test]
    fn assignable_decimal_to_decimal_only_when_equal_precision() {
        // Decimal(_) values currently compare structurally, so different
        // precisions are NOT assignable to each other.
        assert!(Type::Decimal(2).is_assignable_to(&Type::Decimal(2)));
        assert!(!Type::Decimal(2).is_assignable_to(&Type::Decimal(4)));
    }

    // ---------- is_assignable_to: null ----------

    #[test]
    fn assignable_null_to_class() {
        assert!(Type::Null.is_assignable_to(&Type::Class(class("Foo"))));
    }

    #[test]
    fn assignable_null_to_interface() {
        assert!(Type::Null.is_assignable_to(&Type::Interface(iface("Greeter"))));
    }

    #[test]
    fn assignable_null_to_primitive_rejected() {
        assert!(!Type::Null.is_assignable_to(&Type::Int));
        assert!(!Type::Null.is_assignable_to(&Type::String));
        assert!(!Type::Null.is_assignable_to(&Type::Bool));
    }

    #[test]
    fn assignable_class_to_null_rejected() {
        // Reverse direction is not allowed.
        assert!(!Type::Class(class("Foo")).is_assignable_to(&Type::Null));
    }

    // ---------- is_assignable_to: arrays / hashes ----------

    #[test]
    fn assignable_array_covariance() {
        // Array<Int> is assignable to Array<Float> because Int -> Float widens.
        let int_arr = Type::Array(Box::new(Type::Int));
        let float_arr = Type::Array(Box::new(Type::Float));
        assert!(int_arr.is_assignable_to(&float_arr));
    }

    #[test]
    fn assignable_array_invariant_for_unrelated_element_types() {
        let str_arr = Type::Array(Box::new(Type::String));
        let int_arr = Type::Array(Box::new(Type::Int));
        assert!(!str_arr.is_assignable_to(&int_arr));
    }

    #[test]
    fn assignable_hash_covariance_in_both_axes() {
        let h1 = Type::Hash {
            key_type: Box::new(Type::String),
            value_type: Box::new(Type::Int),
        };
        let h2 = Type::Hash {
            key_type: Box::new(Type::String),
            value_type: Box::new(Type::Float),
        };
        assert!(h1.is_assignable_to(&h2));
    }

    #[test]
    fn assignable_hash_rejected_when_value_unrelated() {
        let h1 = Type::Hash {
            key_type: Box::new(Type::String),
            value_type: Box::new(Type::Int),
        };
        let h2 = Type::Hash {
            key_type: Box::new(Type::String),
            value_type: Box::new(Type::Bool),
        };
        assert!(!h1.is_assignable_to(&h2));
    }

    // ---------- is_assignable_to: function variance ----------

    #[test]
    fn assignable_function_param_contravariance() {
        // f1: (Float) -> Void is assignable to f2: (Int) -> Void
        // because Int (param of f2) is assignable to Float (param of f1).
        let f1 = Type::Function {
            params: vec![Type::Float],
            return_type: Box::new(Type::Void),
        };
        let f2 = Type::Function {
            params: vec![Type::Int],
            return_type: Box::new(Type::Void),
        };
        assert!(f1.is_assignable_to(&f2));
        // …but the reverse is not safe.
        assert!(!f2.is_assignable_to(&f1));
    }

    #[test]
    fn assignable_function_return_covariance() {
        // f1: () -> Int is assignable to f2: () -> Float.
        let f1 = Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int),
        };
        let f2 = Type::Function {
            params: vec![],
            return_type: Box::new(Type::Float),
        };
        assert!(f1.is_assignable_to(&f2));
        assert!(!f2.is_assignable_to(&f1));
    }

    #[test]
    fn assignable_function_arity_mismatch_rejected() {
        let f1 = Type::Function {
            params: vec![Type::Int],
            return_type: Box::new(Type::Void),
        };
        let f2 = Type::Function {
            params: vec![Type::Int, Type::Int],
            return_type: Box::new(Type::Void),
        };
        assert!(!f1.is_assignable_to(&f2));
        assert!(!f2.is_assignable_to(&f1));
    }

    // ---------- is_assignable_to: classes ----------

    #[test]
    fn assignable_class_to_self() {
        let foo = class("Foo");
        assert!(Type::Class(foo.clone()).is_assignable_to(&Type::Class(foo)));
    }

    #[test]
    fn assignable_subclass_to_superclass() {
        let parent = class("Animal");
        let child = class_extending("Dog", parent.clone());
        assert!(Type::Class(child).is_assignable_to(&Type::Class(parent)));
    }

    #[test]
    fn assignable_superclass_not_to_subclass() {
        let parent = class("Animal");
        let child = class_extending("Dog", parent.clone());
        assert!(!Type::Class(parent).is_assignable_to(&Type::Class(child)));
    }

    #[test]
    fn assignable_grandchild_to_grandparent() {
        let gp = class("Animal");
        let parent = class_extending("Mammal", gp.clone());
        let child = class_extending("Dog", parent);
        assert!(Type::Class(child).is_assignable_to(&Type::Class(gp)));
    }

    #[test]
    fn assignable_unrelated_classes_rejected() {
        let a = class("Cat");
        let b = class("Dog");
        assert!(!Type::Class(a).is_assignable_to(&Type::Class(b)));
    }

    #[test]
    fn assignable_class_to_implemented_interface() {
        let mut c = class("Hi");
        c.interfaces.push("Greeter".to_string());
        assert!(Type::Class(c).is_assignable_to(&Type::Interface(iface("Greeter"))));
    }

    #[test]
    fn assignable_class_to_unimplemented_interface_rejected() {
        let c = class("Hi");
        assert!(!Type::Class(c).is_assignable_to(&Type::Interface(iface("Greeter"))));
    }

    // ---------- ClassType::find_field / find_method / extends_model ----------

    #[test]
    fn class_find_field_walks_superclass() {
        let mut parent = class("Animal");
        parent.fields.insert(
            "name".to_string(),
            FieldInfo {
                name: "name".to_string(),
                ty: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        let child = class_extending("Dog", parent);
        let f = child.find_field("name").expect("inherited field");
        assert_eq!(f.ty, Type::String);
        assert!(child.find_field("missing").is_none());
    }

    #[test]
    fn class_find_method_walks_superclass() {
        let mut parent = class("Animal");
        parent.methods.insert(
            "speak".to_string(),
            MethodInfo {
                name: "speak".to_string(),
                params: vec![],
                return_type: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        let child = class_extending("Dog", parent);
        let m = child.find_method("speak").expect("inherited method");
        assert_eq!(m.return_type, Type::String);
        assert!(child.find_method("missing").is_none());
    }

    #[test]
    fn class_find_field_prefers_own_over_inherited() {
        // A child field with the same name should shadow the parent's.
        let mut parent = class("Animal");
        parent.fields.insert(
            "n".to_string(),
            FieldInfo {
                name: "n".to_string(),
                ty: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        let mut child = class_extending("Dog", parent);
        child.fields.insert(
            "n".to_string(),
            FieldInfo {
                name: "n".to_string(),
                ty: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        assert_eq!(child.find_field("n").unwrap().ty, Type::String);
    }

    #[test]
    fn extends_model_self() {
        let m = class("Model");
        assert!(m.extends_model());
    }

    #[test]
    fn extends_model_via_superclass() {
        let model = class("Model");
        let user = class_extending("User", model);
        assert!(user.extends_model());
    }

    #[test]
    fn extends_model_via_grandparent() {
        let model = class("Model");
        let base = class_extending("BaseUser", model);
        let user = class_extending("User", base);
        assert!(user.extends_model());
    }

    #[test]
    fn extends_model_false_for_unrelated_class() {
        assert!(!class("Foo").extends_model());
        let parent = class("Animal");
        let child = class_extending("Dog", parent);
        assert!(!child.extends_model());
    }

    // ---------- Type Display ----------

    #[test]
    fn display_primitives() {
        assert_eq!(Type::Int.to_string(), "Int");
        assert_eq!(Type::Float.to_string(), "Float");
        assert_eq!(Type::Bool.to_string(), "Bool");
        assert_eq!(Type::String.to_string(), "String");
        assert_eq!(Type::Symbol.to_string(), "Symbol");
        assert_eq!(Type::Void.to_string(), "Void");
        assert_eq!(Type::Null.to_string(), "Null");
        assert_eq!(Type::Any.to_string(), "Any");
        assert_eq!(Type::Unknown.to_string(), "unknown");
    }

    #[test]
    fn display_decimal_includes_precision() {
        assert_eq!(Type::Decimal(2).to_string(), "Decimal(2)");
        assert_eq!(Type::Decimal(0).to_string(), "Decimal(0)");
    }

    #[test]
    fn display_array_uses_brackets_suffix() {
        assert_eq!(Type::Array(Box::new(Type::Int)).to_string(), "Int[]");
        assert_eq!(
            Type::Array(Box::new(Type::Array(Box::new(Type::String)))).to_string(),
            "String[][]"
        );
    }

    #[test]
    fn display_hash_lists_both_types() {
        let h = Type::Hash {
            key_type: Box::new(Type::String),
            value_type: Box::new(Type::Int),
        };
        assert_eq!(h.to_string(), "Hash<String, Int>");
    }

    #[test]
    fn display_function_signature() {
        let f = Type::Function {
            params: vec![Type::Int, Type::String],
            return_type: Box::new(Type::Bool),
        };
        assert_eq!(f.to_string(), "(Int, String) -> Bool");

        let nullary = Type::Function {
            params: vec![],
            return_type: Box::new(Type::Void),
        };
        assert_eq!(nullary.to_string(), "() -> Void");
    }

    #[test]
    fn display_future_and_named_types() {
        assert_eq!(Type::Future(Box::new(Type::Int)).to_string(), "Future<Int>");
        assert_eq!(Type::Class(class("User")).to_string(), "User");
        assert_eq!(Type::Interface(iface("Greeter")).to_string(), "Greeter");
        assert_eq!(Type::Var(7).to_string(), "?T7");
    }
}
