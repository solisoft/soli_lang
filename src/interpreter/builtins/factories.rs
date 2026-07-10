//! Factory pattern for test data generation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::{Class, NativeFunction, Value};
use crate::span::Span;

#[derive(Debug, Clone)]
enum FactoryTemplate {
    Static(Value),
    Callable(Value),
}

#[derive(Debug, Clone)]
struct FactoryDefinition {
    template: FactoryTemplate,
}

#[derive(Debug, Default)]
struct FactoryRegistry {
    definitions: HashMap<String, FactoryDefinition>,
    bindings: HashMap<String, Rc<Class>>,
    sequences: HashMap<String, u64>,
    factory_sequences: HashMap<String, u64>,
}

impl FactoryRegistry {
    fn define(&mut self, name: &str, template: Value) {
        let template = if is_callable(&template) {
            FactoryTemplate::Callable(template)
        } else {
            FactoryTemplate::Static(template)
        };
        self.definitions
            .insert(name.to_string(), FactoryDefinition { template });
    }

    fn bind(&mut self, name: &str, class: Rc<Class>) {
        self.bindings.insert(name.to_string(), class);
    }

    fn bound_class(&self, name: &str) -> Option<Rc<Class>> {
        self.bindings.get(name).cloned()
    }

    fn next_factory_sequence(&mut self, name: &str) -> u64 {
        let next = *self.factory_sequences.entry(name.to_string()).or_insert(0);
        self.factory_sequences.insert(name.to_string(), next + 1);
        next
    }

    fn sequence(&mut self, name: &str) -> u64 {
        let next = *self.sequences.entry(name.to_string()).or_insert(0);
        self.sequences.insert(name.to_string(), next + 1);
        next
    }

    fn clear(&mut self) {
        self.definitions.clear();
        self.bindings.clear();
        self.sequences.clear();
        self.factory_sequences.clear();
    }

    fn template_for(&self, name: &str) -> Option<FactoryTemplate> {
        self.definitions
            .get(name)
            .map(|definition| definition.template.clone())
    }
}

fn is_callable(value: &Value) -> bool {
    matches!(
        value,
        Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
    )
}

fn merge_values(base: &Value, overrides: &Value) -> Value {
    match (base, overrides) {
        (Value::Hash(base_pairs), Value::Hash(override_pairs)) => {
            let mut result = (*base_pairs.borrow()).clone();
            for (k, v) in override_pairs.borrow().iter() {
                result.insert(k.clone(), v.clone());
            }
            Value::Hash(Rc::new(RefCell::new(result)))
        }
        _ => overrides.clone(),
    }
}

fn interpolate_value(value: &Value, sequence: u64) -> Value {
    match value {
        Value::String(s) => Value::String(interpolate_string(s, sequence).into()),
        Value::Hash(hash) => {
            let mut pairs = (*hash.borrow()).clone();
            for v in pairs.values_mut() {
                *v = interpolate_value(v, sequence);
            }
            Value::Hash(Rc::new(RefCell::new(pairs)))
        }
        Value::Array(items) => {
            let interpolated = items
                .borrow()
                .iter()
                .map(|item| interpolate_value(item, sequence))
                .collect();
            Value::Array(Rc::new(RefCell::new(interpolated)))
        }
        other => other.clone(),
    }
}

fn interpolate_string(input: &str, sequence: u64) -> String {
    if !input.contains("#{n}") {
        return input.to_string();
    }
    input.replace("#{n}", &sequence.to_string())
}

thread_local! {
    static FACTORY_REGISTRY: RefCell<FactoryRegistry> = RefCell::new(FactoryRegistry::default());
}

pub fn factory_name_from_value(value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::String(s) => Ok(s.to_string()),
        other => Err(RuntimeError::General {
            message: format!(
                "Factory methods expect factory name as string, got {}",
                other.type_name()
            ),
            span: Span::new(0, 0, 1, 1),
        }),
    }
}

fn materialize_template(
    interpreter: &mut Interpreter,
    template: &FactoryTemplate,
    sequence: u64,
    span: Span,
) -> Result<Value, RuntimeError> {
    let base = match template {
        FactoryTemplate::Static(value) => value.clone(),
        FactoryTemplate::Callable(callable) => interpreter
            .call_value(callable.clone(), Vec::new(), span)
            .map_err(|e| RuntimeError::General {
                message: format!("Factory template failed: {}", e),
                span,
            })?,
    };
    Ok(interpolate_value(&base, sequence))
}

pub fn build(
    interpreter: &mut Interpreter,
    name: &str,
    overrides: Option<&Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    FACTORY_REGISTRY.with(|registry| {
        let (template, sequence) = {
            let mut registry = registry.borrow_mut();
            let template = registry
                .template_for(name)
                .ok_or_else(|| RuntimeError::General {
                    message: format!("Factory '{}' not defined", name),
                    span,
                })?;
            let sequence = registry.next_factory_sequence(name);
            (template, sequence)
        };

        let mut value = materialize_template(interpreter, &template, sequence, span)?;
        if let Some(overrides) = overrides {
            value = merge_values(&value, overrides);
        }
        Ok(value)
    })
}

pub fn build_list(
    interpreter: &mut Interpreter,
    name: &str,
    count: usize,
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        items.push(build(interpreter, name, None, span)?);
    }
    Ok(Value::Array(Rc::new(RefCell::new(items))))
}

pub fn insert(
    interpreter: &mut Interpreter,
    name: &str,
    overrides: Option<&Value>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let class = FACTORY_REGISTRY.with(|registry| {
        registry
            .borrow()
            .bound_class(name)
            .ok_or_else(|| RuntimeError::General {
                message: format!(
                    "Factory '{}' is not bound to a model — call Factory.bind(name, ModelClass) first",
                    name
                ),
                span,
            })
    })?;

    let attrs = build(interpreter, name, overrides, span)?;
    let class_val = Value::Class(class.clone());
    let native_create =
        class
            .find_native_static_method("create")
            .ok_or_else(|| RuntimeError::General {
                message: "Model.create is not available on bound class".to_string(),
                span,
            })?;

    (native_create.func)(vec![class_val, attrs])
        .map_err(|e| RuntimeError::General { message: e, span })
}

pub fn register_factories(env: &mut Environment) {
    let mut native_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    native_static_methods.insert(
        "define".to_string(),
        Rc::new(NativeFunction::new("Factory.define", Some(2), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("Factory.define expects name as string".to_string()),
            };
            FACTORY_REGISTRY.with(|registry| {
                registry.borrow_mut().define(&name, args[1].clone());
            });
            Ok(Value::Null)
        })),
    );

    let stub = |msg: &'static str| {
        move |_args: Vec<Value>| -> Result<Value, String> { Err(msg.to_string()) }
    };

    native_static_methods.insert(
        "create".to_string(),
        Rc::new(NativeFunction::new(
            "Factory.create",
            Some(1),
            stub("Factory.create is handled by the test interpreter"),
        )),
    );
    native_static_methods.insert(
        "create_with".to_string(),
        Rc::new(NativeFunction::new(
            "Factory.create_with",
            Some(2),
            stub("Factory.create_with is handled by the test interpreter"),
        )),
    );
    native_static_methods.insert(
        "create_list".to_string(),
        Rc::new(NativeFunction::new(
            "Factory.create_list",
            Some(2),
            stub("Factory.create_list is handled by the test interpreter"),
        )),
    );
    native_static_methods.insert(
        "insert".to_string(),
        Rc::new(NativeFunction::new(
            "Factory.insert",
            None,
            stub("Factory.insert is handled by the test interpreter"),
        )),
    );

    native_static_methods.insert(
        "sequence".to_string(),
        Rc::new(NativeFunction::new("Factory.sequence", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("Factory.sequence expects name as string".to_string()),
            };
            let value = FACTORY_REGISTRY.with(|registry| registry.borrow_mut().sequence(&name));
            Ok(Value::Int(value as i64))
        })),
    );

    native_static_methods.insert(
        "bind".to_string(),
        Rc::new(NativeFunction::new("Factory.bind", Some(2), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("Factory.bind expects factory name as string".to_string()),
            };
            let class = match &args[1] {
                Value::Class(class) if class.is_model_subclass() => class.clone(),
                Value::Class(class) => {
                    return Err(format!(
                        "Factory.bind expects a Model subclass, got {}",
                        class.name
                    ));
                }
                other => {
                    return Err(format!(
                        "Factory.bind expects a Model class, got {}",
                        other.type_name()
                    ));
                }
            };
            FACTORY_REGISTRY.with(|registry| {
                registry.borrow_mut().bind(&name, class);
            });
            Ok(Value::Null)
        })),
    );

    native_static_methods.insert(
        "clear".to_string(),
        Rc::new(NativeFunction::new("Factory.clear", Some(0), |_args| {
            FACTORY_REGISTRY.with(|registry| {
                registry.borrow_mut().clear();
            });
            Ok(Value::Null)
        })),
    );

    let factory_class = Class {
        name: "Factory".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    let factory_rc = Rc::new(factory_class);
    env.define("Factory".to_string(), Value::Class(factory_rc.clone()));

    // Dotted aliases for lint/scope and legacy call styles.
    for method in [
        "define",
        "create",
        "create_with",
        "create_list",
        "insert",
        "sequence",
        "bind",
        "clear",
    ] {
        if let Some(native) = factory_rc.find_native_static_method(method) {
            env.define(
                format!("Factory.{}", method),
                Value::NativeFunction((*native).clone()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_string_replaces_sequence_marker() {
        assert_eq!(interpolate_string("user#{n}@test.com", 3), "user3@test.com");
        assert_eq!(interpolate_string("plain", 1), "plain");
    }

    #[test]
    fn interpolate_value_walks_nested_hashes() {
        let mut pairs = crate::interpreter::value::HashPairs::default();
        pairs.insert(
            crate::interpreter::value::HashKey::String("email".into()),
            Value::String("user#{n}@test.com".into()),
        );
        let value = Value::Hash(Rc::new(RefCell::new(pairs)));
        let interpolated = interpolate_value(&value, 2);
        match interpolated {
            Value::Hash(hash) => {
                let email = hash
                    .borrow()
                    .get(&crate::interpreter::value::HashKey::String("email".into()))
                    .cloned()
                    .unwrap();
                assert_eq!(email, Value::String("user2@test.com".into()));
            }
            other => panic!("expected hash, got {:?}", other),
        }
    }
}
