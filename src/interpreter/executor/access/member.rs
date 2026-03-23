//! Member access evaluation (obj.property).

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::Expr;
use crate::error::RuntimeError;
use crate::interpreter::builtins::i18n::helpers as i18n_helpers;
use crate::interpreter::builtins::model::get_relation;
use crate::interpreter::builtins::model::is_translated_field;
use crate::interpreter::builtins::model::relations::RelationType;
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
                    Value::Symbol(ref s) => Self::symbol_member_access(s, name, span),
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
            Value::Symbol(ref _s) => Self::symbol_member_access(_s, name, span),
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

    #[allow(clippy::collapsible_match)]
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
                if inst_ref.fields.is_empty() {
                    return Ok(Value::String(format!("<{} instance>", inst_ref.class.name)));
                }
                let mut s = format!("<{}", inst_ref.class.name);
                let mut first = true;
                for (k, v) in inst_ref.fields.iter() {
                    // Hide _errors when empty
                    if k == "_errors" {
                        if let Value::Array(arr) = v {
                            if arr.borrow().is_empty() {
                                continue;
                            }
                        }
                    }
                    if first {
                        s.push(' ');
                        first = false;
                    } else {
                        s.push_str(",\n ");
                    }
                    s.push_str(k);
                    s.push_str(": ");
                    s.push_str(&Self::inspect_compact(v));
                }
                s.push('>');
                return Ok(Value::String(s));
            }
            "class" => {
                let inst_ref = inst.borrow();
                return Ok(Value::String(inst_ref.class.name.clone()));
            }
            "nil?" => return Ok(Value::Bool(false)),
            "blank?" => return Ok(Value::Bool(false)),
            "present?" => return Ok(Value::Bool(true)),
            // Metaprogramming: respond_to?
            "respond_to?" => {
                let inst_clone = inst.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "respond_to?",
                    Some(1),
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let method_name = match args.first() {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Symbol(s)) => s.clone(),
                            _ => {
                                return Err(
                                    "respond_to? expects a string or symbol argument".to_string()
                                )
                            }
                        };
                        let inst_ref = inst_clone.borrow();
                        let has_method = inst_ref.class.find_method(&method_name).is_some()
                            || inst_ref.class.find_native_method(&method_name).is_some();
                        // Also check universal methods (handled inline, not in class methods)
                        let is_universal = matches!(
                            method_name.as_str(),
                            "inspect"
                                | "class"
                                | "nil?"
                                | "blank?"
                                | "present?"
                                | "respond_to?"
                                | "send"
                                | "instance_variables"
                                | "instance_variable_get"
                                | "instance_variable_set"
                                | "methods"
                                | "method_missing"
                        );
                        Ok(Value::Bool(has_method || is_universal))
                    },
                )));
            }
            // Metaprogramming: send - calls a method by name
            // Usage: obj.send("method_name", arg1, arg2)
            "send" => {
                let inst_clone = inst.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "send",
                    None, // Variable arity
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let method_name = match args.first() {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Symbol(s)) => s.clone(),
                            _ => {
                                return Err(
                                    "send expects a method name as first argument".to_string()
                                )
                            }
                        };
                        let call_args: Vec<Value> = args[1..].to_vec();

                        let inst_ref = inst_clone.borrow();

                        // Check native methods first
                        if let Some(native_method) = inst_ref.class.find_native_method(&method_name)
                        {
                            drop(inst_ref);
                            let mut new_args = vec![Value::Instance(inst_clone.clone())];
                            new_args.extend(call_args.iter().cloned());
                            (native_method.func)(new_args)
                        } else if let Some(method) = inst_ref.class.find_method(&method_name) {
                            // User-defined method - execute it directly
                            drop(inst_ref);
                            let mut bound_env = Environment::with_enclosing(method.closure.clone());
                            bound_env
                                .define("this".to_string(), Value::Instance(inst_clone.clone()));

                            // Bind call arguments to parameters
                            for (i, arg) in call_args.iter().enumerate() {
                                if i < method.params.len() {
                                    bound_env.define(method.params[i].name.clone(), arg.clone());
                                }
                            }

                            let call_env_rc = Rc::new(RefCell::new(bound_env));
                            let env_clone = call_env_rc.borrow().clone();

                            let mut interpreter = Interpreter::default();
                            match interpreter.execute_block(&method.body, env_clone) {
                                Ok(crate::interpreter::executor::ControlFlow::Return(v)) => Ok(v),
                                Ok(crate::interpreter::executor::ControlFlow::Normal(v)) => Ok(v),
                                Ok(crate::interpreter::executor::ControlFlow::Throw(e)) => {
                                    Err(format!("Exception in send: {}", e))
                                }
                                Err(e) => Err(format!("Error in send: {}", e)),
                            }
                        } else if let Some(mm_method) = inst_ref.class.find_method("method_missing")
                        {
                            // Fall back to method_missing
                            drop(inst_ref);
                            let mut mm_args = vec![Value::String(method_name.clone())];
                            mm_args.extend(call_args);

                            let mut bound_env =
                                Environment::with_enclosing(mm_method.closure.clone());
                            bound_env
                                .define("this".to_string(), Value::Instance(inst_clone.clone()));

                            for (i, arg) in mm_args.iter().enumerate() {
                                if i < mm_method.params.len() {
                                    bound_env.define(mm_method.params[i].name.clone(), arg.clone());
                                }
                            }

                            let call_env_rc = Rc::new(RefCell::new(bound_env));
                            let env_clone = call_env_rc.borrow().clone();

                            let mut interpreter = Interpreter::default();
                            match interpreter.execute_block(&mm_method.body, env_clone) {
                                Ok(crate::interpreter::executor::ControlFlow::Return(v)) => Ok(v),
                                Ok(crate::interpreter::executor::ControlFlow::Normal(v)) => Ok(v),
                                Ok(crate::interpreter::executor::ControlFlow::Throw(e)) => {
                                    Err(format!("Exception in method_missing: {}", e))
                                }
                                Err(e) => Err(format!("Error in method_missing: {}", e)),
                            }
                        } else {
                            Err(format!("undefined method `{}`", method_name))
                        }
                    },
                )));
            }
            // Metaprogramming: instance_variables
            "instance_variables" => {
                let inst_ref = inst.borrow();
                let vars: Vec<Value> = inst_ref
                    .fields
                    .keys()
                    .map(|k| Value::String(format!("@{}", k)))
                    .collect();
                return Ok(Value::Array(Rc::new(RefCell::new(vars))));
            }
            // Metaprogramming: instance_variable_get
            "instance_variable_get" => {
                let inst_clone = inst.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "instance_variable_get",
                    Some(1),
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let var_name = match args.first() {
                            Some(Value::String(s)) => {
                                if let Some(stripped) = s.strip_prefix('@') {
                                    stripped.to_string()
                                } else {
                                    s.clone()
                                }
                            }
                            Some(Value::Symbol(s)) => s.clone(),
                            _ => {
                                return Err(
                                    "instance_variable_get expects a string or symbol argument"
                                        .to_string(),
                                )
                            }
                        };
                        let inst_ref = inst_clone.borrow();
                        Ok(inst_ref.get(&var_name).unwrap_or(Value::Null))
                    },
                )));
            }
            // Metaprogramming: instance_variable_set
            // Usage: obj.instance_variable_set("@name", value)
            "instance_variable_set" => {
                let inst_clone = inst.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "instance_variable_set",
                    Some(2),
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let var_name = match args.first() {
                            Some(Value::String(s)) => {
                                if let Some(stripped) = s.strip_prefix('@') {
                                    stripped.to_string()
                                } else {
                                    s.clone()
                                }
                            }
                            Some(Value::Symbol(s)) => s.clone(),
                            _ => {
                                return Err(
                                    "instance_variable_set expects a string or symbol as first argument"
                                        .to_string(),
                                )
                            }
                        };

                        let value = match args.get(1) {
                            Some(v) => v.clone(),
                            None => {
                                return Err(
                                    "instance_variable_set expects a value as second argument"
                                        .to_string(),
                                )
                            }
                        };

                        inst_clone.borrow_mut().set(var_name, value.clone());
                        Ok(value)
                    },
                )));
            }
            // Metaprogramming: define_method - define a method on the instance's class
            // Usage: obj.define_method("method_name", def(args) { body })
            "define_method" => {
                let inst_clone = inst.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "define_method",
                    Some(2),
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let method_name = match args.first() {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Symbol(s)) => s.clone(),
                            _ => {
                                return Err("define_method expects method name as first argument"
                                    .to_string())
                            }
                        };

                        let func = match args.get(1) {
                            Some(Value::Function(f)) => f.clone(),
                            _ => {
                                return Err("define_method expects a function as second argument"
                                    .to_string())
                            }
                        };

                        let class = inst_clone.borrow().class.clone();
                        class.methods.borrow_mut().insert(method_name, func);
                        class.all_methods_cache.borrow_mut().take();

                        Ok(Value::Null)
                    },
                )));
            }
            // Metaprogramming: alias_method - create an alias for an existing method
            // Usage: obj.alias_method("new_name", "old_name")
            "alias_method" => {
                let inst_clone = inst.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "alias_method",
                    Some(2),
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let new_name = match args.first() {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Symbol(s)) => s.clone(),
                            _ => {
                                return Err(
                                    "alias_method expects new name as first argument".to_string()
                                )
                            }
                        };

                        let old_name = match args.get(1) {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Symbol(s)) => s.clone(),
                            _ => {
                                return Err(
                                    "alias_method expects old name as second argument".to_string()
                                )
                            }
                        };

                        let class = inst_clone.borrow().class.clone();
                        let old_method = class.methods.borrow().get(&old_name).cloned();

                        if let Some(method) = old_method {
                            class.methods.borrow_mut().insert(new_name, method);
                            class.all_methods_cache.borrow_mut().take();
                            Ok(Value::Null)
                        } else {
                            Ok(Value::String(format!(
                                "alias_method: method '{}' not found",
                                old_name
                            )))
                        }
                    },
                )));
            }
            // Metaprogramming: instance_eval - execute block with self bound to instance
            // Usage: obj.instance_eval { this.name }
            "instance_eval" => {
                let inst_clone = inst.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "instance_eval",
                    Some(1),
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let block = match args.first() {
                            Some(Value::Function(f)) => f.clone(),
                            _ => {
                                return Err(
                                    "instance_eval expects a block/function argument".to_string()
                                )
                            }
                        };

                        // Create environment with self bound to instance
                        let mut eval_env = Environment::with_enclosing(block.closure.clone());
                        eval_env.define("this".to_string(), Value::Instance(inst_clone.clone()));

                        // Also define 'self' as alias for 'this'
                        eval_env.define("self".to_string(), Value::Instance(inst_clone.clone()));

                        let eval_env_rc = Rc::new(RefCell::new(eval_env));
                        let env_clone = eval_env_rc.borrow().clone();

                        let mut interpreter = Interpreter::default();
                        match interpreter.execute_block(&block.body, env_clone) {
                            Ok(crate::interpreter::executor::ControlFlow::Return(v)) => Ok(v),
                            Ok(crate::interpreter::executor::ControlFlow::Normal(v)) => Ok(v),
                            Ok(crate::interpreter::executor::ControlFlow::Throw(e)) => {
                                Err(format!("Exception in instance_eval: {}", e))
                            }
                            Err(e) => Err(format!("Error in instance_eval: {}", e)),
                        }
                    },
                )));
            }
            // Metaprogramming: methods (lists all accessible method names)
            "methods" => {
                let inst_ref = inst.borrow();
                // Trigger cache building by calling find_method (it calls ensure_methods_cached internally)
                let _ = inst_ref.class.find_method("__methods_dummy__");
                let _ = inst_ref.class.find_native_method("__methods_dummy__");
                let mut method_names: Vec<Value> = Vec::new();
                if let Some(ref cache) = *inst_ref.class.all_methods_cache.borrow() {
                    method_names = cache.keys().map(|k| Value::String(k.clone())).collect();
                }
                if let Some(ref cache) = *inst_ref.class.all_native_methods_cache.borrow() {
                    for k in cache.keys() {
                        if !method_names
                            .iter()
                            .any(|v| matches!(v, Value::String(s) if s == k))
                        {
                            method_names.push(Value::String(k.clone()));
                        }
                    }
                }
                // Add universal methods that are handled inline, not in class methods
                let universal_methods = [
                    "inspect",
                    "class",
                    "nil?",
                    "blank?",
                    "present?",
                    "respond_to?",
                    "send",
                    "instance_variables",
                    "instance_variable_get",
                    "instance_variable_set",
                    "methods",
                    "method_missing",
                ];
                for m in universal_methods {
                    if !method_names
                        .iter()
                        .any(|v| matches!(v, Value::String(s) if s == m))
                    {
                        method_names.push(Value::String(m.to_string()));
                    }
                }
                return Ok(Value::Array(Rc::new(RefCell::new(method_names))));
            }
            _ => {}
        }

        // If we get here, name didn't match any universal method, proceed to field lookup
        let inst_ref = inst.borrow();

        // Check if this is a model subclass and the name is a relation
        if inst_ref.class.is_model_subclass() {
            let class_name = &inst_ref.class.name;
            if let Some(relation) = get_relation(class_name, name) {
                drop(inst_ref);

                // Get the foreign key value based on relation type
                let inst_ref = inst.borrow();
                let fk_value = match relation.relation_type {
                    // For HasMany/HasOne, the FK is on the *related* model,
                    // so we use the owner's _key as the bind value.
                    RelationType::HasMany | RelationType::HasOne | RelationType::Polymorphic => {
                        inst_ref.get("_key")
                    }
                    // For BelongsTo, the FK is on this instance
                    RelationType::BelongsTo => inst_ref.get(&relation.foreign_key),
                };
                drop(inst_ref);

                let fk = match fk_value {
                    Some(Value::String(s)) => s,
                    Some(Value::Int(n)) => n.to_string(),
                    _ => {
                        return Err(RuntimeError::new(
                            format!(
                                "Foreign key '{}' not found or invalid on instance",
                                relation.foreign_key
                            ),
                            span,
                        ));
                    }
                };

                // Build the query based on relation type
                let related_collection = &relation.collection;
                let sdbql = match relation.relation_type {
                    RelationType::HasMany => {
                        format!(
                            "FOR doc IN {} FILTER doc.{} == @fk RETURN doc",
                            related_collection, relation.foreign_key
                        )
                    }
                    RelationType::HasOne => {
                        format!(
                            "FOR doc IN {} FILTER doc.{} == @fk LIMIT 1 RETURN doc",
                            related_collection, relation.foreign_key
                        )
                    }
                    RelationType::BelongsTo => {
                        format!(
                            "FOR doc IN {} FILTER doc._key == @fk LIMIT 1 RETURN doc",
                            related_collection
                        )
                    }
                    RelationType::Polymorphic => {
                        let type_field = relation
                            .polymorphic_type_field
                            .clone()
                            .unwrap_or_else(|| format!("{}_type", relation.name));
                        let type_value = relation
                            .polymorphic_type_value
                            .clone()
                            .unwrap_or_else(|| relation.class_name.clone());
                        format!(
                            "FOR doc IN {} FILTER doc.{} == @fk AND doc.{} == \"{}\" RETURN doc",
                            related_collection, relation.foreign_key, type_field, type_value
                        )
                    }
                };

                let mut bind_vars = std::collections::HashMap::new();
                bind_vars.insert("fk".to_string(), serde_json::Value::String(fk));

                use crate::interpreter::builtins::model::crud::exec_with_auto_collection;
                return match exec_with_auto_collection(sdbql, Some(bind_vars), related_collection) {
                    Ok(results) => {
                        if results.is_empty() {
                            return Ok(Value::Null);
                        }
                        match relation.relation_type {
                            RelationType::HasMany => {
                                // Return array of instances
                                let class = inst.borrow().class.clone();
                                let values: Vec<Value> = results
                                    .iter()
                                    .map(|json| {
                                        crate::interpreter::builtins::model::crud::json_doc_to_instance(&class, json)
                                    })
                                    .collect();
                                Ok(Value::Array(Rc::new(RefCell::new(values))))
                            }
                            _ => {
                                // Return single instance
                                let class = inst.borrow().class.clone();
                                Ok(
                                    crate::interpreter::builtins::model::crud::json_doc_to_instance(
                                        &class,
                                        &results[0],
                                    ),
                                )
                            }
                        }
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("Error fetching relation: {}", e),
                        span,
                    )),
                };
            }
        }

        // Check if this is a translated field
        #[allow(clippy::collapsible_match)]
        let inst_ref = inst.borrow();
        if inst_ref.class.is_model_subclass() {
            let class_name = &inst_ref.class.name;
            if is_translated_field(class_name, name) {
                let locale = i18n_helpers::get_locale();

                // Look up in translated_fields.{field_name}.{locale}
                if let Some(translated_fields) = inst_ref.get("translated_fields") {
                    if let Value::Hash(tf_hash) = translated_fields {
                        let tf_ref = tf_hash.borrow();
                        if let Some(field_translations) = tf_ref.get(
                            &crate::interpreter::value::HashKey::String(name.to_string()),
                        ) {
                            if let Value::Hash(locale_hash) = field_translations {
                                let locale_ref = locale_hash.borrow();
                                if let Some(translated_value) = locale_ref.get(
                                    &crate::interpreter::value::HashKey::String(locale.clone()),
                                ) {
                                    return Ok(translated_value.clone());
                                }
                            }
                        }
                    }
                }

                // Fallback: return raw field value if locale not found
                if let Some(raw_value) = inst_ref.get(name) {
                    return Ok(raw_value);
                }
            }
        }

        drop(inst_ref);

        // First check for field
        let inst_ref = inst.borrow();
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

        // Method not found - check for method_missing
        let class_name = inst_ref.class.name.clone();
        if let Some(mm_method) = inst_ref.class.find_method("method_missing") {
            drop(inst_ref);
            let inst_clone = inst.clone();
            let method_name = name.to_string();

            // Return a function that will call method_missing when invoked
            return Ok(Value::NativeFunction(NativeFunction::new(
                format!("{}.method_missing", class_name),
                None, // Variable arity
                move |args: Vec<Value>| -> Result<Value, String> {
                    // Build the method_missing call arguments: [method_name, ...original_args]
                    let mut mm_args = vec![Value::String(method_name.clone())];
                    mm_args.extend(args);

                    // Build environment for method_missing execution
                    let call_env = Environment::with_enclosing(mm_method.closure.clone());
                    let mut env_inner = call_env;
                    env_inner.define("this".to_string(), Value::Instance(inst_clone.clone()));

                    // Bind method_missing parameters
                    for (i, arg) in mm_args.iter().enumerate() {
                        if i < mm_method.params.len() {
                            env_inner.define(mm_method.params[i].name.clone(), arg.clone());
                        }
                    }
                    let call_env_rc = Rc::new(RefCell::new(env_inner));
                    let env_clone = call_env_rc.borrow().clone();

                    let mut interpreter = Interpreter::default();
                    let result = match interpreter.execute_block(&mm_method.body, env_clone) {
                        Ok(crate::interpreter::executor::ControlFlow::Return(v)) => Ok(v),
                        Ok(crate::interpreter::executor::ControlFlow::Normal(v)) => Ok(v),
                        Ok(crate::interpreter::executor::ControlFlow::Throw(e)) => {
                            Err(format!("Exception in method_missing: {}", e))
                        }
                        Err(e) => Err(format!("Error in method_missing: {}", e)),
                    };
                    result
                },
            )));
        }

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
        // Cache.fetch needs interpreter-level dispatch for block execution
        if class.name == "Cache" && name == "fetch" {
            return Ok(Value::Method(ValueMethod {
                receiver: Box::new(class_val.clone()),
                method_name: "fetch".to_string(),
            }));
        }

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

        // ClassName.new or ClassName.new() — instantiate
        if name == "new" {
            let class_val = class_val.clone();
            return Ok(Value::NativeFunction(NativeFunction::new(
                "new",
                Some(0),
                move |_args| {
                    let class_rc = match &class_val {
                        Value::Class(c) => c.clone(),
                        _ => unreachable!(),
                    };
                    Ok(Value::Instance(Rc::new(RefCell::new(
                        crate::interpreter::value::Instance::new(class_rc),
                    ))))
                },
            )));
        }

        // Universal methods on class values
        match name {
            "inspect" => return Ok(Value::String(format!("<class {}>", class.name))),
            "class" => return Ok(Value::String("class".to_string())),
            "nil?" => return Ok(Value::Bool(false)),
            "blank?" => return Ok(Value::Bool(false)),
            "present?" => return Ok(Value::Bool(true)),
            "to_s" | "to_string" => return Ok(Value::String(format!("<class {}>", class.name))),
            // Metaprogramming: respond_to? (checks if class has static method)
            "respond_to?" => {
                let class_clone = class.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "respond_to?",
                    Some(1),
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let method_name = match args.first() {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Symbol(s)) => s.clone(),
                            _ => {
                                return Err(
                                    "respond_to? expects a string or symbol argument".to_string()
                                )
                            }
                        };
                        let has_method = class_clone.find_static_method(&method_name).is_some()
                            || class_clone
                                .find_native_static_method(&method_name)
                                .is_some();
                        Ok(Value::Bool(has_method))
                    },
                )));
            }
            // Metaprogramming: send (call static method dynamically)
            "send" => {
                let class_clone = class.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "send",
                    None,
                    move |args: Vec<Value>| -> Result<Value, String> {
                        if args.is_empty() {
                            return Err("send expects at least a method name argument".to_string());
                        }
                        let method_name = match &args[0] {
                            Value::String(s) => s.clone(),
                            Value::Symbol(s) => s.clone(),
                            _ => {
                                return Err("send expects method name as first argument".to_string())
                            }
                        };
                        let call_args: Vec<Value> = args[1..].to_vec();

                        // Check static methods
                        if let Some(method) = class_clone.find_static_method(&method_name) {
                            // Build environment for method execution
                            let call_env = Environment::with_enclosing(method.closure.clone());
                            let mut env_inner = call_env;

                            // Bind call arguments to parameters
                            for (i, arg) in call_args.iter().enumerate() {
                                if i < method.params.len() {
                                    env_inner.define(method.params[i].name.clone(), arg.clone());
                                }
                            }
                            let call_env_rc = Rc::new(RefCell::new(env_inner));
                            let env_clone = call_env_rc.borrow().clone();

                            let mut interpreter = Interpreter::default();
                            let result = match interpreter.execute_block(&method.body, env_clone) {
                                Ok(crate::interpreter::executor::ControlFlow::Return(v)) => Ok(v),
                                Ok(crate::interpreter::executor::ControlFlow::Normal(v)) => Ok(v),
                                Ok(crate::interpreter::executor::ControlFlow::Throw(e)) => {
                                    Err(format!("Exception in send: {}", e))
                                }
                                Err(e) => Err(format!("Error in send: {}", e)),
                            };
                            result
                        } else if let Some(native_method) =
                            class_clone.find_native_static_method(&method_name)
                        {
                            (native_method.func)(call_args)
                        } else {
                            Err(format!("undefined method `{}`", method_name))
                        }
                    },
                )));
            }
            // Metaprogramming: class_eval - execute block with self bound to class
            // Usage: MyClass.class_eval { self.some_method }
            "class_eval" => {
                let class_val_clone = class_val.clone();
                return Ok(Value::NativeFunction(NativeFunction::new(
                    "class_eval",
                    Some(1),
                    move |args: Vec<Value>| -> Result<Value, String> {
                        let block = match args.first() {
                            Some(Value::Function(f)) => f.clone(),
                            _ => {
                                return Err(
                                    "class_eval expects a block/function argument".to_string()
                                )
                            }
                        };

                        let class_rc = match &class_val_clone {
                            Value::Class(c) => c.clone(),
                            _ => return Err("class_eval requires a class".to_string()),
                        };

                        // Create environment with self bound to class
                        let mut eval_env = Environment::with_enclosing(block.closure.clone());
                        eval_env.define("this".to_string(), Value::Class(class_rc.clone()));
                        eval_env.define("self".to_string(), Value::Class(class_rc.clone()));

                        let eval_env_rc = Rc::new(RefCell::new(eval_env));
                        let env_clone = eval_env_rc.borrow().clone();

                        let mut interpreter = Interpreter::default();
                        match interpreter.execute_block(&block.body, env_clone) {
                            Ok(crate::interpreter::executor::ControlFlow::Return(v)) => Ok(v),
                            Ok(crate::interpreter::executor::ControlFlow::Normal(v)) => Ok(v),
                            Ok(crate::interpreter::executor::ControlFlow::Throw(e)) => {
                                Err(format!("Exception in class_eval: {}", e))
                            }
                            Err(e) => Err(format!("Error in class_eval: {}", e)),
                        }
                    },
                )));
            }
            // Metaprogramming: methods (lists all static method names)
            "methods" => {
                // Trigger cache building by calling find_method
                let _ = class.find_static_method("__methods_dummy__");
                let _ = class.find_native_static_method("__methods_dummy__");
                let mut method_names: Vec<Value> = Vec::new();
                if let Some(ref cache) = *class.all_methods_cache.borrow() {
                    method_names = cache.keys().map(|k| Value::String(k.clone())).collect();
                }
                return Ok(Value::Array(Rc::new(RefCell::new(method_names))));
            }
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
            "length" | "len" | "size" | "map" | "filter" | "each" | "reduce" | "find" | "any?"
            | "all?" | "sort" | "sort_by" | "reverse" | "uniq" | "compact" | "flatten"
            | "first" | "last" | "empty?" | "includes?" | "contains" | "sample" | "shuffle"
            | "take" | "drop" | "zip" | "sum" | "min" | "max" | "push" | "pop" | "clear"
            | "get" | "to_string" | "to_json" | "join" | "is_a?" => {
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
            "length" | "len" | "size" | "map" | "filter" | "each" | "get" | "fetch" | "invert"
            | "transform_values" | "transform_keys" | "select" | "reject" | "slice" | "except"
            | "compact" | "dig" | "to_string" | "to_json" | "keys" | "values" | "has_key"
            | "delete" | "merge" | "entries" | "clear" | "set" | "empty?" | "is_a?" => {
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
        &mut self,
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
            "where" | "order" | "limit" | "offset" | "includes" | "join" | "select" | "fields"
            | "all" | "first" | "count" | "to_query" | "is_a?" | "pluck" | "sum" | "avg"
            | "min" | "max" | "group_by" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(obj_val),
                method_name: name.to_string(),
            })),
            // exists sets exists_mode on QB and returns new QB (chain with .first to execute)
            "exists" => {
                if let Value::QueryBuilder(qb) = obj_val {
                    let mut new_qb = qb.borrow().clone();
                    new_qb.exists_mode = true;
                    Ok(Value::QueryBuilder(Rc::new(RefCell::new(new_qb))))
                } else {
                    unreachable!()
                }
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
            "length" | "len" | "size" | "to_s" | "to_string" | "to_i" | "to_int" | "to_f" | "to_float"
            | "upcase" | "uppercase" | "downcase" | "lowercase" | "trim" | "strip" | "contains" | "starts_with"
            | "ends_with" | "split" | "index_of" | "substring" | "replace" | "lpad"
            | "rpad" | "join" | "empty?"
            // Ruby-style methods
            | "starts_with?" | "ends_with?" | "includes?" | "chomp" | "lstrip" | "rstrip" | "squeeze"
            | "count" | "gsub" | "sub" | "match" | "scan" | "tr" | "center" | "ljust"
            | "rjust" | "ord" | "chr" | "bytes" | "chars" | "lines" | "bytesize"
            | "capitalize" | "swapcase" | "insert" | "delete" | "delete_prefix"
            | "delete_suffix" | "partition" | "rpartition" | "reverse" | "hex" | "oct"
            | "truncate" | "parse_json" | "to_sym"
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

    fn symbol_member_access(s: &str, name: &str, span: Span) -> RuntimeResult<Value> {
        match name {
            "class" => Ok(Value::String("symbol".to_string())),
            "nil?" => Ok(Value::Bool(false)),
            "blank?" => Ok(Value::Bool(false)),
            "present?" => Ok(Value::Bool(true)),
            "to_s" | "to_string" => Ok(Value::String(s.to_string())),
            "inspect" => Ok(Value::String(format!(":{}", s))),
            "is_a?" => Ok(Value::Method(ValueMethod {
                receiver: Box::new(Value::Symbol(s.to_string())),
                method_name: name.to_string(),
            })),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "symbol".to_string(),
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
            Value::Symbol(s) => format!(":{}", s),
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(n) => format!("{}", n),
            Value::Decimal(d) => format!("Decimal({})", d.0),
            Value::Instance(inst) => Self::inspect_instance_compact(inst),
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

    /// One-line compact representation of an instance, showing only key identifier fields.
    fn inspect_instance_compact(inst: &Rc<RefCell<Instance>>) -> String {
        let inst_ref = inst.borrow();
        if inst_ref.fields.is_empty() {
            return format!("<{} instance>", inst_ref.class.name);
        }
        let mut s = format!("<{}", inst_ref.class.name);
        // Show _key first if present, then a few meaningful fields (skip internal _ fields)
        let mut shown = 0;
        if let Some(key) = inst_ref.fields.get("_key") {
            s.push_str(&format!(" _key: {}", Self::inspect_compact(key)));
            shown += 1;
        }
        for (k, v) in &inst_ref.fields {
            if shown >= 3 {
                s.push_str(", ...");
                break;
            }
            if k == "_key" || k.starts_with('_') {
                continue;
            }
            s.push_str(if shown > 0 { ", " } else { " " });
            s.push_str(k);
            s.push_str(": ");
            let val_str = Self::inspect_compact(v);
            if val_str.len() > 30 {
                s.push_str(&val_str[..27]);
                s.push_str("...");
            } else {
                s.push_str(&val_str);
            }
            shown += 1;
        }
        s.push('>');
        s
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
                let max_display = 10;
                let total = arr.len();
                let items: Vec<String> = arr
                    .iter()
                    .take(max_display)
                    .map(|v| {
                        let compact = Self::inspect_compact(v);
                        if compact.len() <= 80 {
                            format!("{}{}", indent, compact)
                        } else {
                            format!("{}{}", indent, Self::inspect_pretty(v, depth + 1))
                        }
                    })
                    .collect();
                if total > max_display {
                    format!(
                        "[\n{},\n{}... ({} more)\n{}]",
                        items.join(",\n"),
                        indent,
                        total - max_display,
                        closing_indent
                    )
                } else {
                    format!("[\n{}\n{}]", items.join(",\n"), closing_indent)
                }
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
            crate::interpreter::value::HashKey::Symbol(s) => format!(":{}", s),
        }
    }
}
