//! Member access evaluation (obj.property).

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{Function, Instance, NativeFunction, Value, ValueMethod};
use crate::span::Span;

impl Interpreter {
    /// Evaluate safe navigation: object&.name (returns null if object is null)
    pub(crate) fn evaluate_safe_member(
        &mut self,
        object: &Expr,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Value> {
        let obj_val = self.evaluate(object)?;
        if matches!(obj_val, Value::Null) {
            return Ok(Value::Null);
        }
        self.evaluate_member_on_value(obj_val, name, span)
    }

    /// Evaluate member access expression: object.name
    pub(crate) fn evaluate_member(
        &mut self,
        object: &Expr,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Value> {
        let obj_val = self.evaluate(object)?;
        self.evaluate_member_on_value(obj_val, name, span)
    }

    /// Shared member access logic on an already-evaluated value.
    fn evaluate_member_on_value(
        &mut self,
        obj_val: Value,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Value> {
        match obj_val {
            Value::Future(future) => {
                let value = Value::Future(future);
                let resolved = value.resolve().map_err(|e| RuntimeError::new(e, span))?;
                match resolved {
                    Value::Hash(hash) => {
                        let hash_key = crate::interpreter::value::HashKey::String(name.to_string());
                        if let Some(v) = hash.borrow().get(&hash_key) {
                            return Ok(v.clone());
                        }
                        Err(RuntimeError::NoSuchProperty {
                            value_type: "Hash".to_string(),
                            property: name.to_string(),
                            span,
                        })
                    }
                    _ => Err(RuntimeError::NoSuchProperty {
                        value_type: resolved.type_name(),
                        property: name.to_string(),
                        span,
                    }),
                }
            }
            Value::Instance(inst) => self.instance_member_access(inst, name, span),
            Value::Class(ref class) => self.class_member_access(class, name, span, &obj_val),
            Value::Super(ref superclass) => self.super_member_access(superclass, name, span),
            Value::Array(ref _arr) => self.array_member_access(name, span, obj_val),
            Value::Hash(ref hash) => self.hash_member_access(hash, name, span, obj_val.clone()),
            Value::QueryBuilder(_) => self.query_builder_member_access(name, span, obj_val),
            Value::String(ref _s) => self.string_member_access(name, span, obj_val),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: obj_val.type_name().to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn instance_member_access(
        &mut self,
        inst: Rc<RefCell<Instance>>,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Value> {
        let inst_ref = inst.borrow();

        // First check for field
        if let Some(value) = inst_ref.get(name) {
            return Ok(value);
        }

        // Then check for native method
        if let Some(native_method) = inst_ref.class.find_native_method(name) {
            let class_name = inst_ref.class.name.clone();
            let user_arity = native_method.arity.map(|a| a.saturating_sub(1));
            let native_method_clone = native_method.clone();
            drop(inst_ref);
            let instance_clone = inst.clone();
            return Ok(Value::NativeFunction(NativeFunction::new(
                format!("{}.{}", class_name, name),
                user_arity,
                move |args| {
                    let mut new_args = vec![Value::Instance(instance_clone.clone())];
                    new_args.extend(args.iter().cloned());
                    (native_method_clone.func)(new_args)
                },
            )));
        }

        // Then check for user-defined method
        if let Some(method) = inst_ref.class.find_method(name) {
            drop(inst_ref);

            // Bind 'this'
            let mut bound_env = Environment::with_enclosing(method.closure.clone());
            bound_env.define("this".to_string(), Value::Instance(inst.clone()));

            let bound_method = Function {
                name: method.name.clone(),
                params: method.params.clone(),
                body: method.body.clone(),
                closure: Rc::new(RefCell::new(bound_env)),
                is_method: true,
                span: method.span,
                source_path: method.source_path.clone(),
                defining_superclass: None,
                return_type: method.return_type.clone(),
            };
            return Ok(Value::Function(Rc::new(bound_method)));
        }

        let class_name = inst_ref.class.name.clone();
        Err(RuntimeError::NoSuchProperty {
            value_type: class_name,
            property: name.to_string(),
            span,
        })
    }

    fn class_member_access(
        &self,
        class: &crate::interpreter::value::Class,
        name: &str,
        span: Span,
        class_val: &Value,
    ) -> RuntimeResult<Value> {
        // Static method access - search up superclass chain
        if let Some(method) = class.find_static_method(name) {
            return Ok(Value::Function(method));
        }
        if let Some(native_method) = class.find_native_static_method(name) {
            if class.is_model_subclass() {
                let class_val = class_val.clone();
                let method_name = name.to_string();
                let original_func = native_method.func.clone();
                let original_arity = native_method.arity;

                let user_arity = original_arity.map(|a| a.saturating_sub(1));

                let bound_func = NativeFunction::new(
                    Box::leak(format!("bound_{}", method_name).into_boxed_str()),
                    user_arity,
                    move |args| {
                        let mut full_args = vec![class_val.clone()];
                        full_args.extend(args);
                        original_func(full_args)
                    },
                );
                return Ok(Value::NativeFunction(bound_func));
            }
            return Ok(Value::NativeFunction((*native_method).clone()));
        }

        // Check for static field (including inherited static fields)
        fn find_static_field(
            class: &crate::interpreter::value::Class,
            name: &str,
        ) -> Option<Value> {
            if let Some(value) = class.static_fields.borrow().get(name) {
                return Some(value.clone());
            }
            if let Some(ref superclass) = class.superclass {
                return find_static_field(superclass, name);
            }
            None
        }

        if let Some(value) = find_static_field(class, name) {
            return Ok(value);
        }
        Err(RuntimeError::NoSuchProperty {
            value_type: class.name.clone(),
            property: name.to_string(),
            span,
        })
    }

    fn super_member_access(
        &self,
        superclass: &crate::interpreter::value::Class,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Value> {
        // Get 'this' instance for field lookup
        let this_val = self
            .environment
            .borrow()
            .get("this")
            .ok_or_else(|| RuntimeError::type_error("'super' outside of class", span))?;

        let instance = match this_val {
            Value::Instance(inst) => inst,
            _ => return Err(RuntimeError::type_error("'super' outside of class", span)),
        };

        // Check for field in the instance
        if let Some(value) = instance.borrow().get(name) {
            return Ok(value);
        }

        // super.method() - look up method in superclass
        if let Some(method) = superclass.find_method(name) {
            let bound_env = Environment::with_enclosing(method.closure.clone());
            let mut bound_env = bound_env;
            bound_env.define("this".to_string(), Value::Instance(instance.clone()));

            let bound_method = Function {
                name: method.name.clone(),
                params: method.params.clone(),
                body: method.body.clone(),
                closure: Rc::new(RefCell::new(bound_env)),
                is_method: true,
                span: method.span,
                source_path: method.source_path.clone(),
                defining_superclass: None,
                return_type: method.return_type.clone(),
            };
            return Ok(Value::Function(Rc::new(bound_method)));
        }

        // Also check for native methods in superclass
        if let Some(native_method) = superclass.find_native_method(name) {
            let instance_clone = instance.clone();
            let user_arity = native_method.arity.map(|a| a.saturating_sub(1));
            let native_method_clone = native_method.clone();
            return Ok(Value::NativeFunction(NativeFunction::new(
                format!("{}.{}", superclass.name, name),
                user_arity,
                move |args| {
                    let mut new_args = vec![Value::Instance(instance_clone.clone())];
                    new_args.extend(args.iter().cloned());
                    (native_method_clone.func)(new_args)
                },
            )));
        }
        Err(RuntimeError::NoSuchProperty {
            value_type: superclass.name.clone(),
            property: name.to_string(),
            span,
        })
    }

    fn array_member_access(&self, name: &str, span: Span, obj_val: Value) -> RuntimeResult<Value> {
        // Handle array methods: map, filter, each, reduce, find, any?, all?, sort, reverse, uniq, compact, flatten, first, last, empty?, include?, sample, shuffle, take, drop, zip, sum, min, max
        match name {
            "length" | "map" | "filter" | "each" | "reduce" | "find" | "any?" | "all?" | "sort"
            | "sort_by" | "reverse" | "uniq" | "compact" | "flatten" | "first" | "last"
            | "empty?" | "include?" | "sample" | "shuffle" | "take" | "drop" | "zip" | "sum"
            | "min" | "max" | "push" | "pop" | "clear" | "get" | "to_string" | "join" => {
                Ok(Value::Method(ValueMethod {
                    receiver: Box::new(obj_val),
                    method_name: name.to_string(),
                }))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Array".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn hash_member_access(
        &self,
        hash: &Rc<RefCell<indexmap::IndexMap<crate::interpreter::value::HashKey, Value>>>,
        name: &str,
        _span: Span,
        obj_val: Value,
    ) -> RuntimeResult<Value> {
        // First check if it's a known method
        match name {
            "length" | "map" | "filter" | "each" | "get" | "fetch" | "invert"
            | "transform_values" | "transform_keys" | "select" | "reject" | "slice" | "except"
            | "compact" | "dig" | "to_string" | "keys" | "values" | "has_key" | "delete"
            | "merge" | "entries" | "clear" | "set" | "empty?" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(obj_val),
                method_name: name.to_string(),
            })),
            _ => {
                // Try to access as a hash key (dot notation for hash access)
                let hash_key = crate::interpreter::value::HashKey::String(name.to_string());
                if let Some(v) = hash.borrow().get(&hash_key) {
                    return Ok(v.clone());
                }
                Ok(Value::Null)
            }
        }
    }

    fn query_builder_member_access(
        &self,
        name: &str,
        span: Span,
        obj_val: Value,
    ) -> RuntimeResult<Value> {
        // Handle QueryBuilder methods for chaining
        match name {
            "where" | "order" | "limit" | "offset" | "all" | "first" | "count" => {
                Ok(Value::Method(ValueMethod {
                    receiver: Box::new(obj_val),
                    method_name: name.to_string(),
                }))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "QueryBuilder".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn string_member_access(&self, name: &str, span: Span, obj_val: Value) -> RuntimeResult<Value> {
        // Handle string methods and properties
        match name {
            // Core string methods
            "length" | "to_string" | "upcase" | "uppercase" | "downcase" | "lowercase" | "trim" | "contains" | "starts_with"
            | "ends_with" | "split" | "index_of" | "substring" | "replace" | "lpad"
            | "rpad" | "join" | "empty?"
            // Ruby-style methods
            | "starts_with?" | "ends_with?" | "include?" | "chomp" | "lstrip" | "rstrip" | "squeeze"
            | "count" | "gsub" | "sub" | "match" | "scan" | "tr" | "center" | "ljust"
            | "rjust" | "ord" | "chr" | "bytes" | "chars" | "lines" | "bytesize"
            | "capitalize" | "swapcase" | "insert" | "delete" | "delete_prefix"
            | "delete_suffix" | "partition" | "rpartition" | "reverse" | "hex" | "oct"
            | "truncate" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(obj_val),
                method_name: name.to_string(),
            })),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "String".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }
}
