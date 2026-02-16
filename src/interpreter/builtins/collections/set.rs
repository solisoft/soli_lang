//! Set class operations.

use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

pub fn register_set_class(env: &mut Environment) {
    // Placeholder for Set class - using Hash internally with values as keys
    let empty_class = Rc::new(Class {
        name: "Set".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });

    env.define("Set".to_string(), Value::Class(empty_class.clone()));

    let set_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Note: Currently using Hash as underlying storage since sets aren't in original
    // TODO: Implement proper Set class when needed

    let mut set_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    set_static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("Set.new", Some(0), {
            let class_ref = empty_class.clone();
            move |_args| {
                let mut inst = Instance::new(class_ref.clone());
                inst.set(
                    "__value".to_string(),
                    Value::Hash(Rc::new(RefCell::new(IndexMap::new()))),
                );
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            }
        })),
    );

    let set_class = Class {
        name: "Set".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: set_static_methods,
        native_methods: set_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.assign("Set", Value::Class(Rc::new(set_class)));
}
