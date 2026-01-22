//! Factory pattern for test data generation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

#[derive(Debug, Clone)]
struct FactoryDefinition {
    #[allow(dead_code)]
    name: String,
    data: Value,
}

#[derive(Debug, Default)]
struct FactoryRegistry {
    definitions: HashMap<String, FactoryDefinition>,
    sequences: HashMap<String, u64>,
}

impl FactoryRegistry {
    fn new() -> Self {
        Self::default()
    }

    fn define(&mut self, name: &str, data: Value) {
        self.definitions.insert(
            name.to_string(),
            FactoryDefinition {
                name: name.to_string(),
                data,
            },
        );
    }

    fn create(&self, name: &str) -> Result<Value, String> {
        if let Some(def) = self.definitions.get(name) {
            Ok(def.data.clone())
        } else {
            Err(format!("Factory '{}' not defined", name))
        }
    }

    fn create_with(&self, _name: &str, overrides: &Value) -> Result<Value, String> {
        Ok(overrides.clone())
    }

    fn create_list(&self, name: &str, count: usize) -> Result<Value, String> {
        let mut items = Vec::new();
        for _ in 0..count {
            items.push(self.create(name)?);
        }
        Ok(Value::Array(Rc::new(RefCell::new(items))))
    }

    fn sequence(&mut self, name: &str) -> u64 {
        let next = *self.sequences.entry(name.to_string()).or_insert(0);
        self.sequences.insert(name.to_string(), next + 1);
        next
    }

    fn clear(&mut self) {
        self.definitions.clear();
        self.sequences.clear();
    }
}

thread_local! {
    static FACTORY_REGISTRY: RefCell<FactoryRegistry> = RefCell::new(FactoryRegistry::new());
}

pub fn register_factories(env: &mut Environment) {
    env.define(
        "Factory.define".to_string(),
        Value::NativeFunction(NativeFunction::new("Factory.define", Some(2), |args| {
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

    env.define(
        "Factory.create".to_string(),
        Value::NativeFunction(NativeFunction::new("Factory.create", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("Factory.create expects name as string".to_string()),
            };
            FACTORY_REGISTRY.with(|registry| registry.borrow().create(&name))
        })),
    );

    env.define(
        "Factory.create_with".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "Factory.create_with",
            Some(2),
            |args| {
                let name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("Factory.create_with expects name as string".to_string()),
                };
                FACTORY_REGISTRY.with(|registry| registry.borrow().create_with(&name, &args[1]))
            },
        )),
    );

    env.define(
        "Factory.create_list".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "Factory.create_list",
            Some(2),
            |args| {
                let name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("Factory.create_list expects name as string".to_string()),
                };
                let count = match &args[1] {
                    Value::Int(n) => *n as usize,
                    _ => return Err("Factory.create_list expects count as integer".to_string()),
                };
                FACTORY_REGISTRY.with(|registry| registry.borrow().create_list(&name, count))
            },
        )),
    );

    env.define(
        "Factory.sequence".to_string(),
        Value::NativeFunction(NativeFunction::new("Factory.sequence", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("Factory.sequence expects name as string".to_string()),
            };
            let value = FACTORY_REGISTRY.with(|registry| registry.borrow_mut().sequence(&name));
            Ok(Value::Int(value as i64))
        })),
    );

    env.define(
        "Factory.clear".to_string(),
        Value::NativeFunction(NativeFunction::new("Factory.clear", Some(0), |_args| {
            FACTORY_REGISTRY.with(|registry| {
                registry.borrow_mut().clear();
            });
            Ok(Value::Null)
        })),
    );
}
