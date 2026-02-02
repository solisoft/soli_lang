//! Range class operations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

pub fn register_range_class(env: &mut Environment) {
    // Placeholder for Range class
    // TODO: Implement proper Range class when needed
    let empty_class = Rc::new(Class {
        name: "Range".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    });

    env.define("Range".to_string(), Value::Class(empty_class.clone()));

    let range_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    let mut range_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    range_static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("Range.new", Some(2), {
            let class_ref = empty_class.clone();
            move |args| {
                let start = match args.get(1) {
                    Some(Value::Int(n)) => *n,
                    _ => return Err("Range.new() requires integer start".to_string()),
                };
                let end = match args.get(2) {
                    Some(Value::Int(n)) => *n,
                    _ => return Err("Range.new() requires integer end".to_string()),
                };
                let mut inst = Instance::new(class_ref.clone());
                inst.set("__start".to_string(), Value::Int(start));
                inst.set("__end".to_string(), Value::Int(end));
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            }
        })),
    );

    let range_class = Class {
        name: "Range".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: range_static_methods,
        native_methods: range_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    };

    env.assign("Range", Value::Class(Rc::new(range_class)));
}
