//! Member/index access type checking.

use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check member access expression: a bare read (`"12".to_i`), or the
    /// callee of a call without arguments (`"12".to_i()`).
    pub(crate) fn check_member_expr(
        &mut self,
        span: Span,
        object: &Expr,
        name: &str,
    ) -> TypeResult<Type> {
        self.check_member_access(span, object, name, false)
    }

    /// The member callee of a call that passes arguments (`"ff".to_i(16)`,
    /// `xs.sort { |a, b| … }`, `x |> s.split(",")`). It keeps the method's
    /// signature, so the arguments are checked against it, where a bare read
    /// of the same method may be typed as the call the runtime makes — see
    /// [`builtin_member_type`].
    pub(crate) fn check_member_callee_with_args(
        &mut self,
        span: Span,
        object: &Expr,
        name: &str,
    ) -> TypeResult<Type> {
        self.check_member_access(span, object, name, true)
    }

    /// [`Self::check_member_callee_with_args`], plus the receiver's type: a
    /// call whose answer depends on its receiver and arguments (an XML query,
    /// `checker::expressions::xml`) needs both, and checking the receiver a
    /// second time would cost a chain of calls exponential time.
    pub(crate) fn check_member_callee_and_receiver(
        &mut self,
        span: Span,
        object: &Expr,
        name: &str,
    ) -> TypeResult<(Type, Type)> {
        let obj_type = self.check_expr(object)?;
        let member = self.member_of_type(span, object, obj_type.clone(), name, true)?;
        Ok((member, obj_type))
    }

    fn check_member_access(
        &mut self,
        span: Span,
        object: &Expr,
        name: &str,
        called_with_args: bool,
    ) -> TypeResult<Type> {
        let obj_type = self.check_expr(object)?;
        self.member_of_type(span, object, obj_type, name, called_with_args)
    }

    /// `object.name`, `object` being of type `obj_type`.
    fn member_of_type(
        &mut self,
        span: Span,
        object: &Expr,
        obj_type: Type,
        name: &str,
        called_with_args: bool,
    ) -> TypeResult<Type> {
        // An unannotated `x = new C(...)` is typed loosely (members of `C`
        // are not checked), but which class built it is known: enough to
        // refuse a private or protected member, and nothing more.
        if matches!(obj_type, Type::Any | Type::Unknown) {
            if let Some(class_name) = self.constructed_class_of(object) {
                if let Some(class_def) = self.env.get_class(&class_name).cloned() {
                    if let Some((field, owner)) = class_def.find_field_with_owner(name) {
                        self.check_member_visibility(
                            object,
                            "field",
                            name,
                            &class_name,
                            owner,
                            (field.is_private, field.is_protected),
                            span,
                        )?;
                    } else if let Some((method, owner)) = class_def.find_method_with_owner(name) {
                        self.check_member_visibility(
                            object,
                            "method",
                            name,
                            &class_name,
                            owner,
                            (method.is_private, method.is_protected),
                            span,
                        )?;
                    }
                }
            }
        }

        match obj_type {
            Type::Future(_inner) => Ok(Type::Any),
            Type::Class(class) => {
                // Look up the class in the environment to get the full definition with methods
                let class_def = self.env.get_class(&class.name);
                if let Some(class_def) = class_def {
                    if let Some((field, owner)) = class_def.find_field_with_owner(name) {
                        self.check_member_visibility(
                            object,
                            "field",
                            name,
                            &class.name,
                            owner,
                            (field.is_private, field.is_protected),
                            span,
                        )?;
                        return Ok(field.ty.clone());
                    }
                    if let Some((method, owner)) = class_def.find_method_with_owner(name) {
                        // Private: only on `self` (`@name`, `this.name`, the bare
                        // `name`). Protected: also from code running in an
                        // instance of the declaring class or a subclass.
                        self.check_member_visibility(
                            object,
                            "method",
                            name,
                            &class.name,
                            owner,
                            (method.is_private, method.is_protected),
                            span,
                        )?;
                        // A native the runtime runs when read bare, though its
                        // signature lists optional parameters (`doc.to_xml`,
                        // `Xml.document`): the bare read is that call, as for
                        // the String/Array/Hash methods (`builtin_member_type`).
                        if !called_with_args
                            && !method.params.is_empty()
                            && crate::types::runtime_globals::runtime_calls_bare(
                                owner,
                                name,
                                method.is_static,
                            )
                        {
                            return Ok(method.return_type.clone());
                        }
                        return Ok(collapse_zero_arg_method(Type::Function {
                            params: method.params.iter().map(|(_, t)| t.clone()).collect(),
                            return_type: Box::new(method.return_type.clone()),
                        }));
                    }
                    // Three cases where a member absent from the declaration
                    // is real anyway. A `Model` subclass carries native static
                    // methods (`where`, `all`, `join`) known only at run time.
                    // A class whose ancestry leaves this file inherits from a
                    // sibling the pass never read, or from the framework
                    // (`Controller`, `Mailer`, `Job`). And a `module`'s `this`
                    // is whatever includes it.
                    if class_def.has_members_elsewhere() {
                        return Ok(Type::Any);
                    }
                }
                // Universal methods available on every value at runtime.
                match name {
                    "class" | "inspect" | "to_s" | "to_string" => return Ok(Type::String),
                    "nil?" | "blank?" | "present?" => return Ok(Type::Bool),
                    "is_a?" => {
                        return Ok(Type::Function {
                            params: vec![Type::String],
                            return_type: Box::new(Type::Bool),
                        })
                    }
                    // Reflection the runtime implements on the class value itself
                    // (see `executor::access::member`'s `Value::Class` arm). The
                    // checker knew nothing about it, so `A.define_method("hi", …)`
                    // was rejected on both engines and needed `--no-type-check`.
                    // The shapes are dynamic by nature, hence `Any`.
                    "define_method" | "alias_method" | "class_eval" | "send" => {
                        return Ok(Type::Any)
                    }
                    // `Klass.new(...)`: the constructor (or, with one hash,
                    // the attribute form) — every class has it at run time.
                    // Its arguments depend on the shape, so `Any`. A bare
                    // `Klass.new` is checked as `Klass.new()` (#655).
                    "new" => return Ok(Type::Any),
                    "methods" => return Ok(Type::Array(Box::new(Type::String))),
                    "respond_to?" => {
                        return Ok(Type::Function {
                            params: vec![Type::String],
                            return_type: Box::new(Type::Bool),
                        })
                    }
                    _ => {}
                }
                let suggestion = crate::error::suggest::best_match(name, class.member_names());
                Err(TypeError::NoSuchMember {
                    type_name: class.name,
                    member: name.to_string(),
                    span,
                    suggestion,
                })
            }
            Type::Array(inner_type) => self
                .check_array_method(&inner_type, name, span)
                .map(|ty| builtin_member_type(ty, "array", name, called_with_args)),
            Type::Hash {
                key_type,
                value_type,
            } => {
                // First check if it's a known method
                if let Ok(method_type) = self.check_hash_method(&key_type, &value_type, name, span)
                {
                    return Ok(builtin_member_type(
                        method_type,
                        "hash",
                        name,
                        called_with_args,
                    ));
                }
                // If not a method, treat as property access - return value_type
                // (hash.key returns the value type for any key)
                Ok((*value_type).clone())
            }
            Type::String => self
                .check_string_method(name, span)
                .map(|ty| builtin_member_type(ty, "string", name, called_with_args)),
            Type::Any | Type::Unknown => Ok(Type::Any),
            // Primitive types support methods via the OO method dispatch system
            Type::Int | Type::Float | Type::Bool | Type::Null | Type::Decimal(_) | Type::Symbol => {
                Ok(Type::Any)
            }
            // Functions support universal methods (nil?, blank?, present?, class, inspect)
            Type::Function { .. } => Ok(Type::Any),
            _ => Err(TypeError::NoSuchMember {
                type_name: format!("{}", obj_type),
                member: name.to_string(),
                span,
                suggestion: None,
            }),
        }
    }

    /// Check array method access.
    fn check_array_method(&self, inner_type: &Type, name: &str, span: Span) -> TypeResult<Type> {
        match name {
            "map" | "filter" | "select" | "each" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "each_with_index" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "reduce" | "fold" | "find" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "index_of" => Ok(Type::Function {
                params: vec![inner_type.clone()],
                return_type: Box::new(Type::Int),
            }),
            "any?" | "all?" | "none?" | "one?" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Bool),
            }),
            "sort" | "reverse" | "uniq" | "compact" | "compact_blank" | "flatten" | "shuffle"
            | "reject"
            // ActiveRecord-style chainables that also work on a materialized
            // array, so `org.contacts.order("name").all()` type-checks the way
            // it already ran. Rejecting them made a working chain unreachable
            // from checked code, which is the default.
            | "all" | "order" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "sort_by" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            // `xs.first(n)` / `xs.last(n)` take a count and answer an array;
            // bare, `builtin_member_type` types them as the element.
            "first" | "last" => Ok(Type::Function {
                params: vec![Type::Int],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "sample" | "min" | "max" | "shift" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(inner_type.clone()),
            }),
            // `delete(value)` / `delete_at(index)` return the removed element.
            "delete" | "delete_at" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(inner_type.clone()),
            }),
            "empty?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
            }),
            "include?" | "contains" | "contains?" | "includes?" | "includes" => Ok(Type::Function {
                params: vec![inner_type.clone()],
                return_type: Box::new(Type::Bool),
            }),
            "take" | "drop" | "slice" => Ok(Type::Function {
                params: vec![Type::Int, Type::Int],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            // `dig`/`values_at` are variadic — `Type::Any` lifts the arity cap.
            "dig" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "values_at" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            // `pluck`/`pick` accept one-or-more keys/indices (variadic).
            "pluck" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(Type::Any))),
            }),
            "pick" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            // Field-keyed aggregates. These take a field *name* rather than a
            // callback so the whole traversal stays in Rust — see
            // `array_ops::sum_by` for why that matters.
            "sum_by" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "group_by" | "index_by" | "count_by" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(Type::Any),
                    value_type: Box::new(Type::Any),
                }),
            }),
            "tally" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(Type::Any),
                    value_type: Box::new(Type::Any),
                }),
            }),
            // An average is a ratio, so it is always a `Float` (or null when
            // there is nothing to average).
            "avg" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Float),
            }),
            "avg_by" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Float),
            }),
            // These select *records*, so they carry the element type through.
            "uniq_by" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "max_by" | "min_by" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(inner_type.clone()),
            }),
            "filter_by" => Ok(Type::Function {
                params: vec![Type::Any, Type::Any],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "find_by" => Ok(Type::Function {
                params: vec![Type::Any, Type::Any],
                return_type: Box::new(inner_type.clone()),
            }),
            // `count` takes an optional value/predicate; `insert`/`unshift`/`rotate`
            // are variadic mutators — all use `Type::Any` to stay arity-flexible.
            "count" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Int),
            }),
            "insert" | "unshift" | "rotate" => Ok(Type::Function {
                params: vec![Type::Any],
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
            // `to_s` is the Ruby-conventional alias of `to_string`; the runtime
            // has always answered it and only the checker did not know.
            "to_s" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "join" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            }),
            "length" | "len" | "size" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Int),
            }),
            "pop" | "clear" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            // `xs.get(i)` takes the index; the runtime does not call it bare.
            "get" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "push" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Null),
            }),
            // variadic - accepts one or more arrays to append in place
            "concat" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            "intersection" | "union" | "difference" => Ok(Type::Function {
                params: vec![Type::Array(Box::new(inner_type.clone()))],
                return_type: Box::new(Type::Array(Box::new(inner_type.clone()))),
            }),
            // Universal methods on all types
            "class" | "inspect" | "to_string" | "to_json" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "is_a?" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Bool),
            }),
            "nil?" | "blank?" | "present?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
            }),
            _ => Err(TypeError::NoSuchMember {
                type_name: format!("{}[]", inner_type),
                member: name.to_string(),
                span,
                suggestion: crate::error::suggest::primitive_method_suggestion("Array", name),
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
            "length" | "len" | "size" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Int),
            }),
            "empty?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
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
            // `has_key` answers whether the key is there; typing it as the
            // value made `if CONSTS.has_key(k)` a "condition of type Hash".
            "has_key" => Ok(Type::Function {
                params: vec![key_type.clone()],
                return_type: Box::new(Type::Bool),
            }),
            "delete" => Ok(Type::Function {
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
            // Both engines accept an optional second argument — the value to
            // return when the key is missing — but the checker declared one
            // parameter, so `config.get("port", 8080)` was rejected at check
            // time despite running correctly. `Type::Any` keeps it
            // arity-flexible, matching how `dig` and `count` are declared.
            "get" | "fetch" => Ok(Type::Function {
                params: vec![key_type.clone(), Type::Any],
                return_type: Box::new(value_type.clone()),
            }),
            "dig" => Ok(Type::Function {
                params: vec![Type::Any], // variadic - accepts multiple keys
                return_type: Box::new(value_type.clone()),
            }),
            // Implemented in both engines and absent from the checker, so both
            // calls failed to compile. (`key`, `has_value?` and `value?` looked
            // missing too, but were only rejecting a probe that passed the wrong
            // value type — the existing declarations are correct and stricter.)
            "assoc" | "rassoc" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(Type::Any))),
            }),
            "invert" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(value_type.clone()),
                    value_type: Box::new(key_type.clone()),
                }),
            }),
            // Closure-driven transforms/filters: take a single function argument.
            "transform_values" | "transform_keys" | "select" | "reject" | "keep_if"
            | "delete_if" | "each_key" | "each_value" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(key_type.clone()),
                    value_type: Box::new(value_type.clone()),
                }),
            }),
            // `slice`/`except` take an array of keys; `compact` takes nothing.
            "slice" | "except" => Ok(Type::Function {
                params: vec![Type::Array(Box::new(key_type.clone()))],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(key_type.clone()),
                    value_type: Box::new(value_type.clone()),
                }),
            }),
            "compact" | "to_h" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(key_type.clone()),
                    value_type: Box::new(value_type.clone()),
                }),
            }),
            // Predicate iterators take a single function argument.
            "all?" | "any?" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Bool),
            }),
            // `values_at`/`fetch_values` accept one-or-more keys (variadic);
            // the `Type::Any` param signals the arity check to allow extra args.
            "values_at" | "fetch_values" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(value_type.clone()))),
            }),
            "has_value?" | "value?" => Ok(Type::Function {
                params: vec![value_type.clone()],
                return_type: Box::new(Type::Bool),
            }),
            "key" => Ok(Type::Function {
                params: vec![value_type.clone()],
                return_type: Box::new(key_type.clone()),
            }),
            "set" => Ok(Type::Function {
                params: vec![key_type.clone(), value_type.clone()],
                return_type: Box::new(value_type.clone()),
            }),
            // Universal methods on all types
            "class" | "inspect" | "to_string" | "to_json" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "is_a?" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Bool),
            }),
            "nil?" | "blank?" | "present?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
            }),
            _ => Err(TypeError::NoSuchMember {
                type_name: format!("Hash({}, {})", key_type, value_type),
                member: name.to_string(),
                span,
                suggestion: crate::error::suggest::primitive_method_suggestion("Hash", name),
            }),
        }
    }

    /// Check string method access.
    fn check_string_method(&self, name: &str, span: Span) -> TypeResult<Type> {
        match name {
            "length" | "len" | "size" | "ord" | "bytesize" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Int),
            }),
            // These take a needle. Declaring them with no parameters made the
            // checker auto-invoke the member, so `s.count("a")` resolved to
            // `Int` and then reported "Cannot call non-function type 'Int'" —
            // `soli check` rejected a call that runs correctly.
            "count" | "index_of" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Int),
            }),
            "casecmp" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Int),
            }),
            "starts_with?" | "ends_with?" | "include?" | "includes?" | "contains" | "contains?"
            | "starts_with" | "ends_with" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Bool),
            }),
            // Takes nothing: both engines refuse `"".empty?("x")`. It sat with
            // the needle-taking predicates above, so the checker accepted that
            // call and typed the bare `s.empty?` as a function.
            "empty?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
            }),
            // `squeeze(chars)` restricts the squeeze to a character set, exactly
            // as Ruby does; declaring it zero-arg made a working call fail.
            "squeeze" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::String),
            }),
            // `uppercase`/`lowercase` are runtime aliases of `upcase`/`downcase`
            // the checker did not know at all, so calling them by their alias
            // failed to compile.
            "chomp" | "chop" | "lstrip" | "rstrip" | "strip" | "capitalize" | "swapcase"
            | "reverse" | "to_string" | "upcase" | "downcase" | "uppercase" | "lowercase"
            | "trim" | "join" | "slugify" | "succ" | "next" | "html_entities" => {
                Ok(Type::Function {
                    params: vec![],
                    return_type: Box::new(Type::String),
                })
            }
            // These take the affix to strip. They were grouped with the
            // zero-argument methods above, so a correct call was rejected.
            "delete_prefix" | "delete_suffix" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            }),
            "ascii_only?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
            }),
            "casecmp?" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Bool),
            }),
            // `camelize` takes an optional bool; `prepend` takes one-or-more
            // strings — `Type::Any` keeps both arity-flexible.
            "camelize" | "prepend" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::String),
            }),
            "to_sym" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            "gsub" | "replace_all" | "sub" | "tr" | "replace" => Ok(Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::String),
            }),
            "hex" | "oct" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            "chars" | "lines" | "bytes" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Array(Box::new(Type::String))),
            }),
            // Same auto-invoke trap as `count`/`index_of` above: each of these
            // takes a pattern or separator, so declaring zero parameters made
            // `soli check` reject working code.
            "scan" | "partition" | "rpartition" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Array(Box::new(Type::String))),
            }),
            // split accepts an optional String delimiter (defaults to " ")
            "split" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Array(Box::new(Type::String))),
            }),
            // Each takes an optional second argument — the pad string, or the
            // omission marker for `truncate`. Declaring only the width made the
            // checker reject `name.ljust(20, ".")`, which runs correctly.
            "center" | "ljust" | "rjust" | "truncate" | "lpad" | "rpad" => Ok(Type::Function {
                params: vec![Type::Int, Type::Any],
                return_type: Box::new(Type::String),
            }),
            // `to_i` takes an optional radix: `"ff".to_i(16)`. Both engines
            // refuse any other argument ("to_i base must be an Int"). Bare,
            // `"12".to_i` is the call — see `builtin_member_type`.
            "to_i" | "to_int" => Ok(Type::Function {
                params: vec![Type::Int],
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
            "chr" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            // These take arguments (`s.insert(1, "x")`, `s.delete("a")`,
            // `s.substring(0, 2)`, `s.match(re)`), and the runtime does not
            // call them when read bare. Declared without parameters, the bare
            // read was typed as a call the runtime never makes.
            "insert" | "delete" | "substring" | "match" => Ok(Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }),
            "parse_json" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Any),
            }),
            "to_h" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Hash {
                    key_type: Box::new(Type::String),
                    value_type: Box::new(Type::Any),
                }),
            }),
            // Universal methods on all types
            "class" | "inspect" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            }),
            "is_a?" => Ok(Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Bool),
            }),
            "nil?" | "blank?" | "present?" => Ok(Type::Function {
                params: vec![],
                return_type: Box::new(Type::Bool),
            }),
            _ => Err(TypeError::NoSuchMember {
                type_name: "String".to_string(),
                member: name.to_string(),
                span,
                suggestion: crate::error::suggest::primitive_method_suggestion("String", name),
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
            // A Future resolves transparently at runtime (`HTTP.get(url)["status"]`),
            // the same way member access on one is permissive above.
            Type::Future(_) => Ok(Type::Any),
            // An instance reads and writes its fields by name — `this[champ]`,
            // `record[field] = value` — as both engines do for any class. A
            // class whose parent is not in this file may also be indexed by
            // something else, as member access is permissive there too.
            Type::Class(class)
                if matches!(idx_type, Type::String | Type::Any | Type::Unknown)
                    || self
                        .env
                        .get_class(&class.name)
                        .is_some_and(|c| c.has_members_elsewhere()) =>
            {
                Ok(Type::Any)
            }
            _ => Err(TypeError::General {
                message: format!("cannot index {}", obj_type),
                span,
            }),
        }
    }
}

/// Where the checker notes the class an unannotated variable was built from:
/// a name no Soli identifier can take, in the variable's own scope.
fn constructed_key(name: &str) -> String {
    format!("\u{0}new {name}")
}

/// The class named by `new C(...)` or `C.new(...)`, the two spellings of a
/// construction.
fn constructed_class_name(expr: &Expr) -> Option<&str> {
    let class_expr = match &expr.kind {
        ExprKind::New { class_expr, .. } => class_expr,
        ExprKind::Call { callee, .. } => match &callee.kind {
            ExprKind::Member { object, name } if name == "new" => object,
            _ => return None,
        },
        _ => return None,
    };
    match &class_expr.kind {
        ExprKind::Variable(class_name) => Some(class_name),
        _ => None,
    }
}

impl TypeChecker {
    /// Remember (or forget) that variable `name` holds a `new C(...)` (or a
    /// `C.new(...)`).
    pub(crate) fn note_constructed_class(&mut self, name: &str, value: Option<&Expr>) {
        let class = value
            .and_then(constructed_class_name)
            .and_then(|class_name| self.env.get_class(class_name).cloned());
        let noted = class.map(Type::Class).unwrap_or(Type::Unknown);
        self.env.define(constructed_key(name), noted);
    }

    /// The class `object` was built from: `new C(...)` / `C.new(...)` itself,
    /// or a variable noted by [`Self::note_constructed_class`].
    fn constructed_class_of(&self, object: &Expr) -> Option<String> {
        if let ExprKind::New { class_expr, .. } = &object.kind {
            return match &class_expr.kind {
                ExprKind::Variable(class_name) => Some(class_name.clone()),
                _ => None,
            };
        }
        // `c.new(...)` on a variable that is no class is not a construction.
        if let Some(class_name) = constructed_class_name(object) {
            return self
                .env
                .get_class(class_name)
                .map(|_| class_name.to_string());
        }
        match &object.kind {
            ExprKind::Variable(name) => match self.env.get(&constructed_key(name)) {
                Some(Type::Class(class)) => Some(class.name),
                _ => None,
            },
            _ => None,
        }
    }

    /// Refuse `receiver.name` on a private or protected member the code being
    /// checked may not reach — the rule the runtime applies. `this.name` (and
    /// `@name`) is always allowed.
    #[allow(clippy::too_many_arguments)]
    fn check_member_visibility(
        &self,
        object: &Expr,
        kind: &str,
        name: &str,
        receiver_class: &str,
        owner: &str,
        (is_private, is_protected): (bool, bool),
        span: Span,
    ) -> TypeResult<()> {
        if !(is_private || is_protected) || matches!(object.kind, ExprKind::This) {
            return Ok(());
        }
        let inside_hierarchy = self.env.get("this").is_some()
            && self
                .env
                .current_class_type()
                .is_some_and(|current| current.is_or_inherits(owner));
        if is_protected && inside_hierarchy {
            return Ok(());
        }
        // `DemoFarm._task(…)`, a static method read on its class: neither
        // engine enforces visibility there, so neither does the checker.
        if matches!(&object.kind, ExprKind::Variable(name) if name == receiver_class) {
            return Ok(());
        }
        Err(TypeError::General {
            message: format!(
                "{} {} '{}' {} for an instance of {}",
                if is_private { "private" } else { "protected" },
                kind,
                name,
                if kind == "method" {
                    "called"
                } else {
                    "accessed"
                },
                receiver_class
            ),
            span,
        })
    }
}

/// Zero-arg builtin/class methods auto-invoke on bare member access at
/// runtime (`s.length` evaluates to an Int, not a function), so type them
/// as their return type. The explicit call form (`s.length()`) is accepted
/// by the empty-args member-call leniency in `check_call_expr`.
pub(crate) fn collapse_zero_arg_method(ty: Type) -> Type {
    match ty {
        Type::Function {
            params,
            return_type,
        } if params.is_empty() => *return_type,
        other => other,
    }
}

/// The type of `receiver.name` for a built-in receiver (`receiver_type` is the
/// method registry's name for it: `"string"`, `"array"`, `"hash"`).
///
/// A method without parameters is typed as its return value, as
/// [`collapse_zero_arg_method`] does for every receiver. A method whose
/// signature lists *optional* parameters (`"ff".to_i(16)`, `xs.sort(cmp)`,
/// `s.split(",")`) is called bare too when the runtime's method registry says
/// it is zero-arg for that receiver — the rule both engines apply
/// (`method_registry::is_zero_arg_method`) — so a bare read
/// (`"12".to_i + 1`) is typed as that call. The callee of a call with
/// arguments (`called_with_args`) keeps the signature: its arguments are
/// checked against it.
fn builtin_member_type(ty: Type, receiver_type: &str, name: &str, called_with_args: bool) -> Type {
    use crate::interpreter::executor::calls::method_registry::known_methods;

    let Type::Function {
        params,
        return_type,
    } = ty
    else {
        return ty;
    };
    if params.is_empty() {
        return *return_type;
    }
    let runtime_calls_it_bare = known_methods(receiver_type)
        .iter()
        .any(|m| m.name == name && m.zero_arg);
    if called_with_args || !runtime_calls_it_bare {
        return Type::Function {
            params,
            return_type,
        };
    }
    match (receiver_type, name) {
        // Called with a block it answers the receiver; bare, it answers the
        // `[item, index]` pairs (`xs.each_with_index.map { |pair| … }`).
        ("array", "each_with_index") => Type::Array(Box::new(Type::Array(Box::new(Type::Any)))),
        // With a count they answer an array; bare, the element.
        ("array", "first" | "last") => match *return_type {
            Type::Array(element) => *element,
            other => other,
        },
        _ => *return_type,
    }
}

#[cfg(test)]
mod http_class_typing_tests {
    use crate::types::TypeChecker;

    fn check(source: &str) -> Result<(), String> {
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .expect("lexes");
        let program = crate::parser::Parser::new(tokens).parse().expect("parses");
        TypeChecker::new()
            .check(&program)
            .map_err(|errs| errs.into_iter().next().unwrap().to_string())
    }

    /// Every `HTTP.*` verb the runtime registers must type-check with its
    /// trailing options hash, and the Future it returns must be indexable —
    /// `HTTP.get(url, {"timeout": 5})` used to fail with "expected 1, got 2"
    /// and `HTTP.get_json` was not declared at all.
    #[test]
    fn every_http_verb_accepts_a_trailing_options_hash() {
        let opts = r#"{"headers": {"Authorization": "Bearer t"}, "timeout": 5}"#;
        let calls = [
            format!(r#"HTTP.get("https://x", {opts})"#),
            format!(r#"HTTP.post("https://x", {{"a": 1}}, {opts})"#),
            format!(r#"HTTP.put("https://x", "b", {opts})"#),
            format!(r#"HTTP.patch("https://x", "b", {opts})"#),
            format!(r#"HTTP.delete("https://x", {opts})"#),
            format!(r#"HTTP.head("https://x", {opts})"#),
            format!(r#"HTTP.get_json("https://x", {opts})"#),
            format!(r#"HTTP.get_jsonp("https://x", {opts})"#),
            format!(r#"HTTP.post_json("https://x", {{"a": 1}}, {opts})"#),
            format!(r#"HTTP.put_json("https://x", {{"a": 1}}, {opts})"#),
            format!(r#"HTTP.patch_json("https://x", {{"a": 1}}, {opts})"#),
            format!(r#"HTTP.get_all(["https://x"], {opts})"#),
            format!(r#"HTTP.get_all_json(["https://x"], {opts})"#),
            format!(r#"HTTP.request("GET", "https://x", {opts}, "body")"#),
        ];
        for call in &calls {
            check(&format!("r = {call}\nprint(r)\n"))
                .unwrap_or_else(|e| panic!("{call} should type-check: {e}"));
        }
        // The verbs without options still work bare.
        check(r#"print(HTTP.get("https://x"))"#).unwrap();
        check(r#"print(HTTP.get_json("https://x"))"#).unwrap();
    }

    #[test]
    fn indexing_an_http_future_is_permissive() {
        check(
            r#"r = HTTP.get("https://x", {"timeout": 5})
print(r["status"])
print(HTTP.get_json("https://x")["body"]["name"])
"#,
        )
        .expect("indexing the returned Future must type-check");
    }

    /// `{"response": "binary"}` makes the body an Array<Int>, so the verbs
    /// that return a body cannot be typed String.
    #[test]
    fn a_binary_body_is_used_as_an_array() {
        check(
            r#"bytes = HTTP.get("https://x", {"response": "binary"})
print(bytes.length)
total = bytes.map { |b| b + 1 }
print(Base64.encode(bytes))
print(total)
"#,
        )
        .expect("a binary body must type-check as an array");
    }
}

/// A built-in method whose signature lists optional parameters but which the
/// runtime calls when read bare (`method_registry`'s `zero_arg`) — `to_i`,
/// `split`, `sort`, `count`… The checker kept the bare read as a function, so
/// `"12".to_i + 1` failed with "cannot add (Any) -> Int and Int" though both
/// engines answer 13.
#[cfg(test)]
mod optional_arg_builtin_tests {
    use crate::types::TypeChecker;

    fn check(source: &str) -> Result<(), String> {
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .map_err(|e| e.to_string())?;
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| e.to_string())?;
        TypeChecker::new().check(&program).map_err(|errs| {
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    fn accepts(source: &str) {
        if let Err(e) = check(source) {
            panic!("{source:?} should type-check: {e}");
        }
    }

    fn refuses(source: &str) {
        assert!(check(source).is_err(), "{source:?} should be refused");
    }

    /// #679: `xs.last(2)` was refused with "Cannot call non-function type
    /// 'Int'" — the bare element type was called.
    #[test]
    fn first_and_last_take_a_count_and_read_bare_as_the_element() {
        accepts("xs = [1, 2, 3]\nprint(xs.last(2).length + xs.first(1)[0])\n");
        accepts("xs = [1, 2, 3]\nprint(xs.last + xs.first + 1)\n");
        accepts("xs = [1, 2, 3]\nprint(xs.last() + 1)\n");
    }

    #[test]
    fn a_bare_to_i_is_the_int_it_returns() {
        accepts("x = \"12\".to_i + 1\nprint(x)\n");
        accepts("y = \"12\".to_i\nprint(y + 1)\n");
        accepts("print(\"12\".to_int + 1)\n");
        accepts("let n: Int = \"12\".to_i\nprint(n)\n");
    }

    #[test]
    fn to_i_with_a_radix_or_empty_parens_still_type_checks() {
        accepts("print(\"ff\".to_i(16) + 1)\n");
        accepts("print(\"12\".to_i() + 1)\n");
        accepts("s = \"ff\"\nprint(s&.to_i(16))\n");
    }

    /// Both engines refuse it: "to_i base must be an Int".
    #[test]
    fn a_radix_that_is_no_int_is_refused() {
        refuses("print(\"ff\".to_i(\"16\"))\n");
    }

    #[test]
    fn other_string_methods_with_optional_arguments_read_bare() {
        accepts("print(\"a b\".split[0] + \"!\")\n");
        accepts("print(\"a,b\".split(\",\")[0] + \"!\")\n");
        accepts("let s: String = \"a_b\".camelize\nprint(s)\n");
        accepts("let s: String = \"a_b\".camelize(true)\nprint(s)\n");
        accepts("let s: String = \"aab\".squeeze\nprint(s)\n");
        accepts("let s: String = \"aab\".squeeze(\"a\")\nprint(s)\n");
        // The runtime reads the piped-into member bare: `"a,b".split` is
        // already an Array, so both engines raise "Cannot call non-function
        // value".
        refuses("print(\",\" |> \"a,b\".split())\n");
        refuses("print(16 |> \"ff\".to_i())\n");
        // A method the runtime does not call bare still takes the piped value.
        accepts("print(\"a\" |> \"ab\".starts_with?())\n");
    }

    #[test]
    fn empty_takes_no_argument() {
        accepts("let b: Bool = \"\".empty?\nprint(b)\n");
        accepts("print(\"\".empty?())\n");
        refuses("print(\"\".empty?(\"x\"))\n");
    }

    #[test]
    fn array_methods_with_optional_arguments_read_bare() {
        for method in [
            "sort",
            "reverse",
            "uniq",
            "compact",
            "compact_blank",
            "flatten",
            "shuffle",
            "all",
            "rotate",
        ] {
            accepts(&format!("print([2, 1].{method}[0] + 1)\n"));
            accepts(&format!("print([2, 1].{method}()[0] + 1)\n"));
        }
        accepts("print([2, 1].sort(fn(a, b) { a - b })[0] + 1)\n");
        accepts("print([1, 2, 3].rotate(2)[0] + 1)\n");
        accepts("print([[1], [2]].flatten(1).length)\n");
        accepts("print([1, 2].count + 1)\n");
        accepts("print([1, 2].count(fn(x) { x > 1 }) + 1)\n");
        accepts("print([1, 2].count { |x| x > 1 } + 1)\n");
    }

    /// Bare, `each_with_index` answers `[item, index]` pairs, not the
    /// receiver it answers when given a block.
    #[test]
    fn a_bare_each_with_index_is_the_pairs() {
        accepts("print([5, 6].each_with_index[0][1] + 1)\n");
        accepts("print([5, 6].each_with_index.map { |pair| pair[1] })\n");
        accepts("[5, 6].each_with_index do |x, i|\n  print(x + i)\nend\n");
    }

    /// The runtime does not call these bare, so neither does the checker.
    #[test]
    fn methods_the_runtime_does_not_call_bare_stay_functions() {
        accepts("print(\"ab\".insert(1, \"x\"))\n");
        accepts("print(\"ab\".delete(\"a\"))\n");
        accepts("print(\"ab\".substring(0, 1))\n");
        accepts("print(\"ab\".match(\"a\"))\n");
        accepts("print([1, 2].get(0))\n");
        refuses("print(\"ab\".insert + 1)\n");
        refuses("print([1, 2].get + 1)\n");
        // A method that takes a needle is still a function when read bare.
        refuses("print(\"ab\".starts_with? + 1)\n");
    }
}
