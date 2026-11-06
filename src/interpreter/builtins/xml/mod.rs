//! The `Xml`, `XmlNode` and `XmlDocument` classes: Soli's XML parser, tree,
//! builder and serialiser, over the pure-Rust core in `crate::xml`.
//!
//! - `value.rs` — the handle (`Value::Xml` inside an `XmlNode` instance), the
//!   thread-local classes, the method-table helpers and the engine hooks.
//! - `node.rs` — the node methods and the `Xml` statics that build trees.
//! - `query.rs` — `find`, `find_all`, `xpath` and `at_xpath`.
//! - `block.rs` — the block builder (`Xml.build … do`, `node.add … do`), run
//!   by the engines.
//! - `c14n.rs` — exclusive canonicalisation and the `get_element_by_id` /
//!   `get_elements_by_tag` statics, which keep their own parser.

pub mod block;
pub mod c14n;
pub mod node;
pub mod query;
pub mod value;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Value};
use std::rc::Rc;

/// Define `Xml` (the c14n trio plus this module's statics, in one class),
/// `XmlNode` and `XmlDocument`.
pub fn register_xml_builtins(env: &mut Environment) {
    // The c14n module defines its own `Xml` class; take its statics from a
    // scratch environment so `Xml` is defined once, with every static.
    let mut scratch = Environment::new();
    c14n::register_xml_builtins(&mut scratch);
    let mut statics = match scratch.get("Xml") {
        Some(Value::Class(class)) => class.native_static_methods.clone(),
        _ => value::MethodTable::new(),
    };
    node::xml_statics(&mut statics);

    let xml_class = Class {
        name: "Xml".to_string(),
        native_static_methods: statics,
        ..Default::default()
    };
    env.define("Xml".to_string(), Value::Class(Rc::new(xml_class)));
    env.define("XmlNode".to_string(), Value::Class(value::node_class()));
    env.define(
        "XmlDocument".to_string(),
        Value::Class(value::document_class()),
    );
}
