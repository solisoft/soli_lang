//! Class registrations for value-type primitives (Int, Float, Bool, Null,
//! Decimal, Symbol).
//!
//! These classes don't carry methods themselves — primitive dispatch happens
//! in `executor::access::member.rs` and the VM. The Class is registered so
//! that user code can do things like `Int`, `Int.class`, and especially
//! `Int.class_eval do define_method(:foo) { ... } end`. The `primitive` field
//! tags the class so `class_eval` / `define_method` / `alias_method` know to
//! route writes to `executor::calls::user_methods::USER_METHODS`.
//!
//! `String`, `Array`, `Hash` already have classes registered in `collections/`
//! and are tagged with `primitive: Some(...)` directly at their registration
//! sites — we don't re-register them here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, PrimType, Value};

pub fn register_primitive_classes(env: &mut Environment) {
    register("Int", PrimType::Int, env);
    register("Float", PrimType::Float, env);
    register("Bool", PrimType::Bool, env);
    register("Null", PrimType::Null, env);
    register("Decimal", PrimType::Decimal, env);
    register("Symbol", PrimType::Symbol, env);
}

fn register(name: &str, prim: PrimType, env: &mut Environment) {
    let class = Rc::new(Class {
        name: name.to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        primitive: Some(prim),
        ..Default::default()
    });
    env.define(name.to_string(), Value::Class(class));
}
