//! Generics in the checker: binding the type parameters of a call from its
//! arguments, and substituting them into a signature.
//!
//! `def first<T>(xs: T[]) -> T` called with an `Int[]` binds `T` to `Int`,
//! so the call is an `Int`. `class Box<T>`'s instance `Box<Int>` reads its
//! `get -> T` as an `Int`. Type parameters are erased at run time; this is
//! the checker's whole share of them.

use std::collections::HashMap;

use super::type_repr::Type;

/// Whether `ty` mentions a type parameter anywhere.
pub fn mentions_param(ty: &Type) -> bool {
    match ty {
        Type::Param(_) => true,
        Type::Array(inner) | Type::Nullable(inner) | Type::Future(inner) => mentions_param(inner),
        Type::Hash {
            key_type,
            value_type,
        } => mentions_param(key_type) || mentions_param(value_type),
        Type::Function {
            params,
            return_type,
        } => params.iter().any(mentions_param) || mentions_param(return_type),
        Type::Union(members) => members.iter().any(mentions_param),
        Type::Class(class) => class.type_args.iter().any(mentions_param),
        _ => false,
    }
}

/// Bind the type parameters `param` mentions from the type of the argument
/// passed for it. A parameter bound twice to different types is bound to
/// their union (`pair(1, "a")` for `def pair<T>(a: T, b: T)` is `Int |
/// String`). `Any` and `Unknown` bind nothing.
pub fn unify(param: &Type, arg: &Type, bindings: &mut HashMap<String, Type>) {
    if matches!(arg, Type::Any | Type::Unknown) {
        return;
    }
    match (param, arg) {
        (Type::Param(name), _) => {
            let bound = match bindings.remove(name) {
                Some(previous) if previous != *arg => Type::union([previous, arg.clone()]),
                _ => arg.clone(),
            };
            bindings.insert(name.clone(), bound);
        }
        (Type::Array(p), Type::Array(a)) | (Type::Future(p), Type::Future(a)) => {
            unify(p, a, bindings)
        }
        (
            Type::Hash {
                key_type: pk,
                value_type: pv,
            },
            Type::Hash {
                key_type: ak,
                value_type: av,
            },
        ) => {
            unify(pk, ak, bindings);
            unify(pv, av, bindings);
        }
        (Type::Nullable(p), Type::Nullable(a)) => unify(p, a, bindings),
        (Type::Nullable(_), Type::Null) => {}
        (Type::Nullable(p), a) => unify(p, a, bindings),
        (
            Type::Function {
                params: pp,
                return_type: pr,
            },
            Type::Function {
                params: ap,
                return_type: ar,
            },
        ) => {
            for (p, a) in pp.iter().zip(ap) {
                unify(p, a, bindings);
            }
            unify(pr, ar, bindings);
        }
        (Type::Class(p), Type::Class(a)) if p.name == a.name => {
            for (p, a) in p.type_args.iter().zip(&a.type_args) {
                unify(p, a, bindings);
            }
        }
        _ => {}
    }
}

/// `ty` with each bound type parameter replaced by its binding. An unbound
/// one stays a parameter — as open as `Any` (a value read from it is `Any`),
/// and still there for a later call to bind (a generic method of a generic
/// class: `Box<Int>`'s `map<U>`).
pub fn substitute(ty: &Type, bindings: &HashMap<String, Type>) -> Type {
    match ty {
        Type::Param(name) => bindings.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Array(inner) => Type::Array(Box::new(substitute(inner, bindings))),
        Type::Future(inner) => Type::Future(Box::new(substitute(inner, bindings))),
        Type::Nullable(inner) => Type::union([substitute(inner, bindings), Type::Null]),
        Type::Hash {
            key_type,
            value_type,
        } => Type::Hash {
            key_type: Box::new(substitute(key_type, bindings)),
            value_type: Box::new(substitute(value_type, bindings)),
        },
        Type::Function {
            params,
            return_type,
        } => Type::Function {
            params: params.iter().map(|p| substitute(p, bindings)).collect(),
            return_type: Box::new(substitute(return_type, bindings)),
        },
        Type::Union(members) => Type::union(members.iter().map(|m| substitute(m, bindings))),
        Type::Class(class) if !class.type_args.is_empty() => {
            let mut class = class.clone();
            class.type_args = class
                .type_args
                .iter()
                .map(|a| substitute(a, bindings))
                .collect();
            Type::Class(class)
        }
        other => other.clone(),
    }
}

/// The instance type a generic class's constructor builds from `args`
/// (`Box.new(1)` is a `Box<Int>`), or the class as it is when nothing binds.
pub fn instance_from_constructor(class: &super::type_repr::ClassType, args: &[Type]) -> Type {
    let mut instance = class.clone();
    if let (false, Some(params)) = (class.type_params.is_empty(), &class.constructor_params) {
        let mut bindings = HashMap::new();
        for (param, arg) in params.iter().zip(args) {
            unify(param, arg, &mut bindings);
        }
        if !bindings.is_empty() {
            instance.type_args = class
                .type_params
                .iter()
                .map(|name| bindings.get(name).cloned().unwrap_or(Type::Any))
                .collect();
        }
    }
    Type::Class(instance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_through_arrays_and_substitutes() {
        let mut bindings = HashMap::new();
        let param = Type::Array(Box::new(Type::Param("T".into())));
        unify(&param, &Type::Array(Box::new(Type::Int)), &mut bindings);
        assert_eq!(bindings.get("T"), Some(&Type::Int));
        assert_eq!(substitute(&Type::Param("T".into()), &bindings), Type::Int);
        assert_eq!(
            substitute(&Type::Param("U".into()), &bindings),
            Type::Param("U".into())
        );
    }

    #[test]
    fn two_bindings_make_a_union() {
        let mut bindings = HashMap::new();
        let t = Type::Param("T".into());
        unify(&t, &Type::Int, &mut bindings);
        unify(&t, &Type::String, &mut bindings);
        assert_eq!(
            bindings.get("T"),
            Some(&Type::Union(vec![Type::Int, Type::String]))
        );
    }
}
