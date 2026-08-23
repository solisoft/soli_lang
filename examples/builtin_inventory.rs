//! Dump the full builtin API surface (globals + classes + methods) as JSON.
//!
//! The runtime environment is the authoritative enumerator: this avoids the
//! drift and aliasing pitfalls of grepping the registration call sites.
//!
//! Usage:
//!   cargo run --example builtin_inventory > /tmp/builtin_inventory.json
//!
//! Primitive methods (Int/Float/Bool/Null/Symbol/Decimal) are NOT dumped here —
//! they live in the per-type tables in
//! `src/interpreter/executor/calls/method_registry.rs`, which
//! `scripts/builtin_audit.py` parses directly on each run.
//!
//! KNOWN GAP: methods that exist ONLY as a match arm in
//! `src/interpreter/executor/access/member.rs` — the universal members such as
//! `class` / `nil?` / `present?` / `inspect` / `is_a?`, and the DateTime
//! versions of them — appear in neither this dump nor the registry tables, so
//! they are unaudited. The audit report says so too, rather than counting them
//! as covered. Closing it means teaching the audit to read those match arms.

use solilang::interpreter::builtins::register_builtins;
use solilang::interpreter::environment::Environment;
use solilang::interpreter::value::{Class, Value};
use std::collections::BTreeMap;

#[derive(serde::Serialize)]
struct Inventory {
    globals: Vec<GlobalEntry>,
    classes: Vec<ClassEntry>,
}

#[derive(serde::Serialize)]
struct GlobalEntry {
    name: String,
    arity: Option<usize>,
}

#[derive(serde::Serialize)]
struct ClassEntry {
    name: String,
    is_module: bool,
    primitive: Option<String>,
    superclass: Option<String>,
    instance_methods: Vec<String>,
    static_methods: Vec<String>,
}

fn prim_name(c: &Class) -> Option<String> {
    use solilang::interpreter::value::PrimType::*;
    c.primitive.map(|p| match p {
        Int => "Int".into(),
        Float => "Float".into(),
        Bool => "Bool".into(),
        Null => "Null".into(),
        Decimal => "Decimal".into(),
        String => "String".into(),
        Array => "Array".into(),
        Hash => "Hash".into(),
        Symbol => "Symbol".into(),
    })
}

fn main() {
    let env = std::rc::Rc::new(std::cell::RefCell::new(
        Environment::with_builtins_capacity(),
    ));
    register_builtins(&mut env.borrow_mut(), true);

    // Soli-defined builtins loaded from embedded .sl sources.
    solilang::interpreter::builtins::retry::register_retry_class(&env)
        .expect("retry stdlib must evaluate");
    solilang::interpreter::builtins::template::register_form_builder(&env)
        .expect("form builder stdlib must evaluate");

    let bindings = env.borrow().get_all_bindings();

    let mut globals: Vec<GlobalEntry> = Vec::new();
    let mut class_map: BTreeMap<String, ClassEntry> = BTreeMap::new();

    for (name, value) in bindings.iter() {
        match value {
            Value::NativeFunction(f) => globals.push(GlobalEntry {
                name: name.clone(),
                arity: f.arity,
            }),
            Value::Class(class) => {
                let entry = ClassEntry {
                    name: name.clone(),
                    is_module: class.is_module,
                    primitive: prim_name(class),
                    superclass: class.superclass.as_ref().map(|sc| sc.name.clone()),
                    instance_methods: {
                        let mut m: Vec<String> = class.native_methods.keys().cloned().collect();
                        m.extend(class.methods.borrow().keys().cloned());
                        m.sort();
                        m.dedup();
                        m
                    },
                    static_methods: {
                        let mut m: Vec<String> =
                            class.native_static_methods.keys().cloned().collect();
                        m.extend(class.static_methods.keys().cloned());
                        m.extend(class.vm_static_methods.borrow().keys().cloned());
                        m.extend(class.mixin_static_methods.borrow().keys().cloned());
                        m.sort();
                        m.dedup();
                        m
                    },
                };
                class_map.insert(name.clone(), entry);
                for nested in class.nested_classes.borrow().values() {
                    let nested_entry = ClassEntry {
                        name: format!("{}::{}", name, nested.name),
                        is_module: nested.is_module,
                        primitive: prim_name(nested),
                        superclass: nested.superclass.as_ref().map(|sc| sc.name.clone()),
                        instance_methods: {
                            let mut m: Vec<String> =
                                nested.native_methods.keys().cloned().collect();
                            m.extend(nested.methods.borrow().keys().cloned());
                            m.sort();
                            m.dedup();
                            m
                        },
                        static_methods: {
                            let mut m: Vec<String> =
                                nested.native_static_methods.keys().cloned().collect();
                            m.extend(nested.static_methods.keys().cloned());
                            m.sort();
                            m.dedup();
                            m
                        },
                    };
                    class_map.insert(nested_entry.name.clone(), nested_entry);
                }
            }
            _ => {}
        }
    }

    globals.sort_by(|a, b| a.name.cmp(&b.name));

    let inventory = Inventory {
        globals,
        classes: class_map.into_values().collect(),
    };

    println!("{}", serde_json::to_string_pretty(&inventory).unwrap());
}
