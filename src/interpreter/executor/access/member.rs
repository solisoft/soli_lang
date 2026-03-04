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
                // Handle the resolved value without recursing on Future
                match resolved {
                    Value::String(s) => self.string_member_access(name, span, Value::String(s)),
                    Value::Array(arr) => self.array_member_access(name, span, Value::Array(arr)),
                    Value::Hash(_) => {
                        // For Hash, clone the Rc to avoid borrow issues
                        let hash_rc = match &resolved {
                            Value::Hash(h) => h.clone(),
                            _ => unreachable!(),
                        };
                        let hash_ref = hash_rc.clone();
                        self.hash_member_access(&hash_ref, name, span, Value::Hash(hash_rc))
                    }
                    Value::Int(n) => Self::int_member_access(n, name, span),
                    Value::Float(n) => Self::float_member_access(n, name, span),
                    Value::Bool(b) => Self::bool_member_access(b, name, span),
                    Value::Null => Self::null_member_access(name, span),
                    Value::Decimal(d) => Self::decimal_member_access(&d, name, span),
                    Value::Instance(inst) => self.instance_member_access(inst, name, span),
                    Value::Class(_) => {
                        if let Value::Class(class) = &resolved {
                            self.class_member_access(class, name, span, &resolved)
                        } else {
                            unreachable!()
                        }
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
            Value::Int(n) => Self::int_member_access(n, name, span),
            Value::Float(n) => Self::float_member_access(n, name, span),
            Value::Bool(b) => Self::bool_member_access(b, name, span),
            Value::Null => Self::null_member_access(name, span),
            Value::Decimal(ref d) => Self::decimal_member_access(d, name, span),
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
        // Universal methods on instances
        match name {
            "inspect" => {
                let inst_ref = inst.borrow();
                return Ok(Value::String(format!("<{} instance>", inst_ref.class.name)));
            }
            "class" => {
                let inst_ref = inst.borrow();
                return Ok(Value::String(inst_ref.class.name.clone()));
            }
            "nil?" => return Ok(Value::Bool(false)),
            "blank?" => return Ok(Value::Bool(false)),
            "present?" => return Ok(Value::Bool(true)),
            _ => {}
        }

        let inst_ref = inst.borrow();

        // First check for field
        if let Some(value) = inst_ref.get(name) {
            return Ok(value);
        }

        // Then check for native method
        if let Some(native_method) = inst_ref.class.find_native_method(name) {
            let class_name = inst_ref.class.name.clone();
            let native_method_clone = native_method.clone();
            drop(inst_ref);
            let instance_clone = inst.clone();
            return Ok(Value::NativeFunction(NativeFunction::new(
                format!("{}.{}", class_name, name),
                native_method.arity,
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

                let bound_func = NativeFunction::new(
                    Box::leak(format!("bound_{}", method_name).into_boxed_str()),
                    original_arity.map(|a| a - 1),
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

        // Universal methods on class values
        match name {
            "inspect" => return Ok(Value::String(format!("<class {}>", class.name))),
            "class" => return Ok(Value::String("class".to_string())),
            "nil?" => return Ok(Value::Bool(false)),
            "blank?" => return Ok(Value::Bool(false)),
            "present?" => return Ok(Value::Bool(true)),
            "to_s" | "to_string" => return Ok(Value::String(format!("<class {}>", class.name))),
            _ => {}
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
            let native_method_clone = native_method.clone();
            return Ok(Value::NativeFunction(NativeFunction::new(
                format!("{}.{}", superclass.name, name),
                native_method.arity,
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
        // Universal methods
        match name {
            "class" => return Ok(Value::String("array".to_string())),
            "nil?" => return Ok(Value::Bool(false)),
            "inspect" => {
                return Ok(Value::String(Self::inspect_value(&obj_val)));
            }
            "blank?" => {
                if let Value::Array(ref arr) = obj_val {
                    return Ok(Value::Bool(arr.borrow().is_empty()));
                }
                unreachable!()
            }
            "present?" => {
                if let Value::Array(ref arr) = obj_val {
                    return Ok(Value::Bool(!arr.borrow().is_empty()));
                }
                unreachable!()
            }
            _ => {}
        }
        match name {
            "length" | "map" | "filter" | "each" | "reduce" | "find" | "any?" | "all?" | "sort"
            | "sort_by" | "reverse" | "uniq" | "compact" | "flatten" | "first" | "last"
            | "empty?" | "include?" | "contains" | "sample" | "shuffle" | "take" | "drop"
            | "zip" | "sum" | "min" | "max" | "push" | "pop" | "clear" | "get" | "to_string"
            | "join" | "is_a?" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(obj_val),
                method_name: name.to_string(),
            })),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Array".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn hash_member_access(
        &self,
        hash: &Rc<RefCell<crate::interpreter::value::HashPairs>>,
        name: &str,
        _span: Span,
        obj_val: Value,
    ) -> RuntimeResult<Value> {
        // Universal methods
        match name {
            "class" => return Ok(Value::String("hash".to_string())),
            "nil?" => return Ok(Value::Bool(false)),
            "inspect" => {
                return Ok(Value::String(Self::inspect_value(&obj_val)));
            }
            "blank?" => return Ok(Value::Bool(hash.borrow().is_empty())),
            "present?" => return Ok(Value::Bool(!hash.borrow().is_empty())),
            _ => {}
        }
        // First check if it's a known method
        match name {
            "length" | "len" | "map" | "filter" | "each" | "get" | "fetch" | "invert"
            | "transform_values" | "transform_keys" | "select" | "reject" | "slice" | "except"
            | "compact" | "dig" | "to_string" | "keys" | "values" | "has_key" | "delete"
            | "merge" | "entries" | "clear" | "set" | "empty?" | "is_a?" => {
                Ok(Value::Method(ValueMethod {
                    receiver: Box::new(obj_val),
                    method_name: name.to_string(),
                }))
            }
            _ => {
                // Try to access as a hash key (dot notation for hash access)
                // Use StrKey for zero-allocation lookup (hashes identically to HashKey::String)
                if let Some(v) = hash.borrow().get(&crate::interpreter::value::StrKey(name)) {
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
        // Universal methods
        match name {
            "class" => return Ok(Value::String("query_builder".to_string())),
            "nil?" => return Ok(Value::Bool(false)),
            "inspect" => return Ok(Value::String(format!("{}", obj_val))),
            "blank?" => return Ok(Value::Bool(false)),
            "present?" => return Ok(Value::Bool(true)),
            _ => {}
        }
        // Handle QueryBuilder methods for chaining
        match name {
            "where" | "order" | "limit" | "offset" | "all" | "first" | "count" | "to_query"
            | "is_a?" => {
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
            // Universal methods
            "class" => return Ok(Value::String("string".to_string())),
            "nil?" => return Ok(Value::Bool(false)),
            "inspect" => {
                if let Value::String(ref s) = obj_val {
                    return Ok(Value::String(format!("\"{}\"", s)));
                }
                unreachable!()
            }
            "blank?" => {
                if let Value::String(ref s) = obj_val {
                    return Ok(Value::Bool(s.trim().is_empty()));
                }
                unreachable!()
            }
            "present?" => {
                if let Value::String(ref s) = obj_val {
                    return Ok(Value::Bool(!s.trim().is_empty()));
                }
                unreachable!()
            }
            _ => {}
        }
        match name {
            // Core string methods
            "length" | "len" | "to_s" | "to_string" | "to_i" | "to_int" | "to_f" | "to_float"
            | "upcase" | "uppercase" | "downcase" | "lowercase" | "trim" | "contains" | "starts_with"
            | "ends_with" | "split" | "index_of" | "substring" | "replace" | "lpad"
            | "rpad" | "join" | "empty?"
            // Ruby-style methods
            | "starts_with?" | "ends_with?" | "include?" | "chomp" | "lstrip" | "rstrip" | "squeeze"
            | "count" | "gsub" | "sub" | "match" | "scan" | "tr" | "center" | "ljust"
            | "rjust" | "ord" | "chr" | "bytes" | "chars" | "lines" | "bytesize"
            | "capitalize" | "swapcase" | "insert" | "delete" | "delete_prefix"
            | "delete_suffix" | "partition" | "rpartition" | "reverse" | "hex" | "oct"
            | "truncate"
            // Universal method with args
            | "is_a?" => Ok(Value::Method(ValueMethod {
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

    fn int_member_access(n: i64, name: &str, span: Span) -> RuntimeResult<Value> {
        match name {
            // Zero-arg methods (return value directly)
            "class" => Ok(Value::String("int".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(false)),
            "present?" => Ok(Value::Bool(true)),
            "to_s" | "to_string" => Ok(Value::String(n.to_string())),
            "to_f" | "to_float" => Ok(Value::Float(n as f64)),
            "inspect" => Ok(Value::String(n.to_string())),
            "abs" => Ok(Value::Int(n.abs())),
            "sqrt" => Ok(Value::Float((n as f64).sqrt())),
            "even?" => Ok(Value::Bool(n % 2 == 0)),
            "odd?" => Ok(Value::Bool(n % 2 != 0)),
            "zero?" => Ok(Value::Bool(n == 0)),
            "positive?" => Ok(Value::Bool(n > 0)),
            "negative?" => Ok(Value::Bool(n < 0)),
            "chr" => {
                if (0..=0x10FFFF).contains(&n) {
                    if let Some(c) = char::from_u32(n as u32) {
                        return Ok(Value::String(c.to_string()));
                    }
                }
                Err(RuntimeError::type_error(
                    format!("{} is not a valid character code", n),
                    span,
                ))
            }
            // Methods with args (return ValueMethod)
            "times" | "upto" | "downto" | "pow" | "gcd" | "lcm" | "between?" | "clamp"
            | "is_a?" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(Value::Int(n)),
                method_name: name.to_string(),
            })),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "int".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn float_member_access(n: f64, name: &str, span: Span) -> RuntimeResult<Value> {
        match name {
            // Zero-arg methods (returned directly)
            "class" => Ok(Value::String("float".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(false)),
            "present?" => Ok(Value::Bool(true)),
            "to_s" | "to_string" => Ok(Value::String(format!("{}", n))),
            "to_i" | "to_int" => Ok(Value::Int(n as i64)),
            "inspect" => Ok(Value::String(format!("{}", n))),
            "abs" => Ok(Value::Float(n.abs())),
            "sqrt" => Ok(Value::Float(n.sqrt())),
            "ceil" => Ok(Value::Int(n.ceil() as i64)),
            "floor" => Ok(Value::Int(n.floor() as i64)),
            "truncate" => Ok(Value::Int(n.trunc() as i64)),
            "zero?" => Ok(Value::Bool(n == 0.0)),
            "positive?" => Ok(Value::Bool(n > 0.0)),
            "negative?" => Ok(Value::Bool(n < 0.0)),
            "infinite?" => Ok(Value::Bool(n.is_infinite())),
            "nan?" => Ok(Value::Bool(n.is_nan())),
            "finite?" => Ok(Value::Bool(n.is_finite())),
            // Methods that support both 0-arg (auto-invoked) and with-arg forms,
            // or methods that always require args — all go through ValueMethod.
            // `round` with 0 args is auto-invoked via is_zero_arg_builtin_method.
            "round" | "between?" | "clamp" | "is_a?" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(Value::Float(n)),
                method_name: name.to_string(),
            })),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "float".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn bool_member_access(b: bool, name: &str, span: Span) -> RuntimeResult<Value> {
        match name {
            "class" => Ok(Value::String("bool".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(!b)),
            "present?" => Ok(Value::Bool(b)),
            "to_s" | "to_string" => Ok(Value::String(b.to_string())),
            "to_i" | "to_int" => Ok(Value::Int(if b { 1 } else { 0 })),
            "inspect" => Ok(Value::String(b.to_string())),
            // Method with args
            "is_a?" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(Value::Bool(b)),
                method_name: name.to_string(),
            })),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "bool".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn null_member_access(name: &str, span: Span) -> RuntimeResult<Value> {
        match name {
            "class" => Ok(Value::String("null".to_string())),
            "nil?" => Ok(Value::Bool(true)),
            "blank?" => Ok(Value::Bool(true)),
            "present?" => Ok(Value::Bool(false)),
            "to_s" | "to_string" => Ok(Value::String(String::new())),
            "to_a" | "to_array" => Ok(Value::Array(Rc::new(RefCell::new(Vec::new())))),
            "to_i" | "to_int" => Ok(Value::Int(0)),
            "to_f" | "to_float" => Ok(Value::Float(0.0)),
            "inspect" => Ok(Value::String("null".to_string())),
            // Method with args
            "is_a?" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(Value::Null),
                method_name: name.to_string(),
            })),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "null".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    fn decimal_member_access(
        d: &crate::interpreter::value::DecimalValue,
        name: &str,
        span: Span,
    ) -> RuntimeResult<Value> {
        use rust_decimal::prelude::*;
        let val = d.0;
        match name {
            // Zero-arg methods
            "class" => Ok(Value::String("decimal".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(false)),
            "present?" => Ok(Value::Bool(true)),
            "to_s" | "to_string" => Ok(Value::String(val.to_string())),
            "to_i" | "to_int" => Ok(Value::Int(val.to_i64().unwrap_or(0))),
            "to_f" | "to_float" => Ok(Value::Float(d.to_f64())),
            "inspect" => Ok(Value::String(format!("Decimal({})", val))),
            "abs" => Ok(Value::Decimal(crate::interpreter::value::DecimalValue(
                val.abs(),
                d.1,
            ))),
            "sqrt" => Ok(Value::Float(d.to_f64().sqrt())),
            "ceil" => Ok(Value::Int(val.ceil().to_i64().unwrap_or(0))),
            "floor" => Ok(Value::Int(val.floor().to_i64().unwrap_or(0))),
            "truncate" => Ok(Value::Int(val.trunc().to_i64().unwrap_or(0))),
            "zero?" => Ok(Value::Bool(val.is_zero())),
            "positive?" => Ok(Value::Bool(val.is_sign_positive() && !val.is_zero())),
            "negative?" => Ok(Value::Bool(val.is_sign_negative() && !val.is_zero())),
            // Methods with args (round supports both 0-arg and 1-arg)
            "round" | "between?" | "clamp" | "is_a?" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(Value::Decimal(d.clone())),
                method_name: name.to_string(),
            })),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "decimal".to_string(),
                property: name.to_string(),
                span,
            }),
        }
    }

    /// Produce a Soli-native inspect string (like Ruby's `p` / `.inspect`).
    /// Pretty-prints with indentation when the compact form exceeds 80 chars.
    fn inspect_value(val: &Value) -> String {
        let compact = Self::inspect_compact(val);
        if compact.len() <= 80 {
            compact
        } else {
            Self::inspect_pretty(val, 0)
        }
    }

    /// Compact single-line inspect (used for short values and leaf nodes).
    fn inspect_compact(val: &Value) -> String {
        match val {
            Value::String(s) => format!("\"{}\"", s),
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(n) => format!("{}", n),
            Value::Decimal(d) => format!("Decimal({})", d.0),
            Value::Array(arr) => {
                let arr = arr.borrow();
                let items: Vec<String> = arr.iter().map(Self::inspect_compact).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Hash(hash) => {
                let hash = hash.borrow();
                let items: Vec<String> = hash
                    .iter()
                    .map(|(k, v)| format!("{}: {}", Self::inspect_key(k), Self::inspect_compact(v)))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            _ => format!("{}", val),
        }
    }

    /// Pretty-print with newlines and indentation.
    fn inspect_pretty(val: &Value, depth: usize) -> String {
        let indent = "  ".repeat(depth + 1);
        let closing_indent = "  ".repeat(depth);
        match val {
            Value::Array(arr) => {
                let arr = arr.borrow();
                if arr.is_empty() {
                    return "[]".to_string();
                }
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| {
                        let compact = Self::inspect_compact(v);
                        if compact.len() <= 80 {
                            format!("{}{}", indent, compact)
                        } else {
                            format!("{}{}", indent, Self::inspect_pretty(v, depth + 1))
                        }
                    })
                    .collect();
                format!("[\n{}\n{}]", items.join(",\n"), closing_indent)
            }
            Value::Hash(hash) => {
                let hash = hash.borrow();
                if hash.is_empty() {
                    return "{}".to_string();
                }
                let items: Vec<String> = hash
                    .iter()
                    .map(|(k, v)| {
                        let key_str = Self::inspect_key(k);
                        let val_compact = Self::inspect_compact(v);
                        if val_compact.len() <= 60 {
                            format!("{}{}: {}", indent, key_str, val_compact)
                        } else {
                            format!(
                                "{}{}: {}",
                                indent,
                                key_str,
                                Self::inspect_pretty(v, depth + 1)
                            )
                        }
                    })
                    .collect();
                format!("{{\n{}\n{}}}", items.join(",\n"), closing_indent)
            }
            _ => Self::inspect_compact(val),
        }
    }

    /// Format a hash key for inspect output.
    fn inspect_key(key: &crate::interpreter::value::HashKey) -> String {
        match key {
            crate::interpreter::value::HashKey::String(s) => format!("\"{}\"", s),
            crate::interpreter::value::HashKey::Int(n) => n.to_string(),
            crate::interpreter::value::HashKey::Decimal(d) => format!("{}", d),
            crate::interpreter::value::HashKey::Bool(b) => b.to_string(),
            crate::interpreter::value::HashKey::Null => "null".to_string(),
        }
    }
}
