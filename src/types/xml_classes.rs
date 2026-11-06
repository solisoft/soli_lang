//! What the checker knows of `Xml`, `XmlNode`, `XmlDocument` and `XmlReader`
//! (`interpreter::builtins::xml`): every native, with its parameters and the
//! type it answers.
//!
//! Conventions, each one the runtime's:
//!
//! - **Optional parameters** are declared: the checker only refuses *more*
//!   arguments than declared. A method the runtime runs when read bare
//!   (`to_xml`, `Xml.document`) is typed as that call — see
//!   `runtime_globals::runtime_calls_bare`.
//! - **Options, attributes and namespace tables are `Any`.** They are passed
//!   as one Hash *or* as named arguments (`Xml.parse(s, strip_blank: true,
//!   max_depth: 8)`, `add("item", id: 1)`, `xpath("//r:x", r: uri)`), and the
//!   checker counts every named argument as one more argument; an `Any`
//!   parameter is what lifts that count. An `add` content is a String, a
//!   number, a node, an Array of these or (as attributes) a Hash: `Any` too.
//! - **`nil`** has no type of its own here: a node that may be absent
//!   (`parent`, `find`, `reader.next`) is typed `XmlNode`, as a nil-able
//!   class value is everywhere in the checker, and a String that may be
//!   absent (`node["id"]`, `prefix`) is typed `String`.
//! - **Queries** (`find`, `find_all`, `at_xpath`, `xpath`) answer nodes or
//!   Strings depending on the expression, so they are declared `Any` (and
//!   `Any[]`); a literal expression is compiled by the checker and the call
//!   typed from it — see `checker::expressions::xml`.

use crate::types::type_repr::{ClassType, MethodInfo, Type};

/// The node class, as a type.
pub fn node_type() -> Type {
    Type::Class(ClassType::new("XmlNode".to_string()))
}

/// The document class, as a type: it carries its parent, so an
/// `XmlDocument` value is accepted where an `XmlNode` is declared.
pub fn document_type() -> Type {
    let mut document = ClassType::new("XmlDocument".to_string());
    document.superclass = Some(Box::new(ClassType::new("XmlNode".to_string())));
    Type::Class(document)
}

fn reader_type() -> Type {
    Type::Class(ClassType::new("XmlReader".to_string()))
}

fn array_of(inner: Type) -> Type {
    Type::Array(Box::new(inner))
}

fn hash_of(value: Type) -> Type {
    Type::Hash {
        key_type: Box::new(Type::String),
        value_type: Box::new(value),
    }
}

fn add(class: &mut ClassType, is_static: bool, name: &str, params: &[(&str, Type)], ret: Type) {
    class.methods.insert(
        name.to_string(),
        MethodInfo {
            name: name.to_string(),
            params: params
                .iter()
                .map(|(param, ty)| (param.to_string(), ty.clone()))
                .collect(),
            return_type: ret,
            is_private: false,
            is_protected: false,
            is_static,
        },
    );
}

/// The four classes, `XmlNode` before `XmlDocument` (which embeds it).
pub fn xml_classes() -> Vec<ClassType> {
    let node = node_class();
    let mut document = ClassType::new("XmlDocument".to_string());
    document.superclass = Some(Box::new(node.clone()));
    add(&mut document, false, "root", &[], node_type());
    add(
        &mut document,
        false,
        "declaration",
        &[],
        hash_of(Type::String),
    );
    // The name of the DOCTYPE `doctype: "skip"` read past, or nil.
    add(&mut document, false, "doctype", &[], Type::String);
    vec![xml_class(), node, document, reader_class()]
}

fn xml_class() -> ClassType {
    use Type::{Any, Bool, String};
    let mut xml = ClassType::new("Xml".to_string());
    let s = true;

    // Parsing (§1.2). `Xml.parse` never reads a path; `parse_file` does.
    add(
        &mut xml,
        s,
        "parse",
        &[("xml", String), ("opts", Any)],
        document_type(),
    );
    add(
        &mut xml,
        s,
        "parse_file",
        &[("path", String), ("opts", Any)],
        document_type(),
    );
    add(&mut xml, s, "well_formed?", &[("xml", String)], Bool);
    // A String, a number or `nil` (which gives `""`).
    add(&mut xml, s, "escape", &[("text", Any)], String);

    // Building (§1.7). `build` runs its block with the root element.
    add(
        &mut xml,
        s,
        "document",
        &[("root_name", String), ("attrs", Any)],
        document_type(),
    );
    add(
        &mut xml,
        s,
        "build",
        &[("root_name", String), ("attrs", Any)],
        document_type(),
    );
    add(
        &mut xml,
        s,
        "element",
        &[("name", String), ("content", Any), ("attrs", Any)],
        node_type(),
    );

    // Streaming (§1.10): files only. `each` runs its block per match and
    // answers the count. `opts` is `doctype:` (a third-argument Hash with a
    // `doctype` key is the options).
    let stream = [
        ("path", String),
        ("match", String),
        ("ns", Any),
        ("opts", Any),
    ];
    add(&mut xml, s, "each", &stream, Type::Int);
    add(&mut xml, s, "reader", &stream, reader_type());

    // The canonicalisation statics, older than the tree API.
    add(
        &mut xml,
        s,
        "c14n_exclusive",
        &[("xml", String), ("opts", Any)],
        String,
    );
    add(
        &mut xml,
        s,
        "get_element_by_id",
        &[("xml", String), ("id", String)],
        String,
    );
    add(
        &mut xml,
        s,
        "get_elements_by_tag",
        &[("xml", String), ("local_name", String)],
        array_of(String),
    );
    xml
}

fn node_class() -> ClassType {
    use Type::{Any, Bool, String};
    let mut node = ClassType::new("XmlNode".to_string());
    let i = false;

    // Identity and naming (§1.3). `name`, `local_name`, `prefix` and
    // `namespace` are nil on the kinds that have none.
    add(&mut node, i, "kind", &[], String);
    for predicate in [
        "element?",
        "text?",
        "cdata?",
        "comment?",
        "pi?",
        "document?",
    ] {
        add(&mut node, i, predicate, &[], Bool);
    }
    for name in ["name", "local_name", "prefix", "namespace", "path"] {
        add(&mut node, i, name, &[], String);
    }
    add(&mut node, i, "namespaces", &[], hash_of(String));
    add(&mut node, i, "namespace_declarations", &[], hash_of(String));

    // Navigation (§1.4): fresh Arrays; a missing neighbour is nil.
    for many in ["children", "nodes", "ancestors", "descendants"] {
        add(&mut node, i, many, &[], array_of(node_type()));
    }
    for one in [
        "parent",
        "next_sibling",
        "previous_sibling",
        "next_node",
        "previous_node",
    ] {
        add(&mut node, i, one, &[], node_type());
    }
    add(&mut node, i, "document", &[], document_type());

    // Text and attributes (§1.5). `get` is `node["k"]`; `set` is
    // `node["k"] = v`, and answers the value.
    for text in ["text", "own_text", "inner_xml"] {
        add(&mut node, i, text, &[], String);
    }
    add(&mut node, i, "get", &[("name", String)], String);
    add(
        &mut node,
        i,
        "set",
        &[("name", String), ("value", Any)],
        Any,
    );
    add(
        &mut node,
        i,
        "attribute",
        &[("local_name", String), ("namespace", String)],
        String,
    );
    add(&mut node, i, "attributes", &[], hash_of(String));
    add(&mut node, i, "has_attribute?", &[("name", String)], Bool);

    // Building and mutating (§1.7): each answers the node it inserted or
    // touched. `add` runs its block, if any, with the new child.
    add(
        &mut node,
        i,
        "add",
        &[("name", String), ("content", Any), ("attrs", Any)],
        node_type(),
    );
    for leaf in ["add_text", "add_cdata", "add_comment"] {
        add(&mut node, i, leaf, &[("text", Any)], node_type());
    }
    add(
        &mut node,
        i,
        "add_pi",
        &[("target", String), ("data", Any)],
        node_type(),
    );
    // A node, or a String that becomes a text node.
    for sibling in [
        "append",
        "prepend",
        "insert_before",
        "insert_after",
        "replace",
    ] {
        add(&mut node, i, sibling, &[("node", Any)], node_type());
    }
    add(&mut node, i, "remove", &[], node_type());
    add(
        &mut node,
        i,
        "remove_attribute",
        &[("name", String)],
        String,
    );
    add(&mut node, i, "dup", &[], node_type());

    // Serialisation (§1.8) and the `to_h` projection (§1.9).
    add(&mut node, i, "to_xml", &[("opts", Any)], String);
    add(&mut node, i, "to_s", &[("opts", Any)], String);
    add(&mut node, i, "to_h", &[], hash_of(Any));

    // Queries (§1.6): see the module comment.
    for first in ["find", "at_xpath"] {
        add(&mut node, i, first, &[("expr", String), ("ns", Any)], Any);
    }
    for all in ["find_all", "xpath"] {
        add(
            &mut node,
            i,
            all,
            &[("expr", String), ("ns", Any)],
            array_of(Any),
        );
    }
    node
}

fn reader_class() -> ClassType {
    let mut reader = ClassType::new("XmlReader".to_string());
    // All three run when read bare: `while (item = reader.next)`.
    add(&mut reader, false, "next", &[], node_type());
    add(&mut reader, false, "count", &[], Type::Int);
    add(&mut reader, false, "close", &[], Type::Null);
    reader
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every native the runtime registers on the four classes has a
    /// signature here — not the parameterless `Any` that
    /// `absorb_runtime_globals` gives a verb nobody modelled — and nothing is
    /// modelled that the runtime does not have. `new` (which refuses) and the
    /// hidden constructor are left to the runtime.
    #[test]
    fn every_native_is_modelled_and_nothing_else() {
        let classes = xml_classes();
        for global in crate::types::runtime_globals::runtime_globals() {
            let Some(class) = classes.iter().find(|c| c.name == global.name) else {
                continue;
            };
            let verbs = global.class_verbs.as_ref().expect("a class");
            let mut runtime: Vec<&str> = verbs
                .iter()
                .map(String::as_str)
                .filter(|v| !matches!(*v, "new" | "__construct__"))
                .collect();
            runtime.sort_unstable();
            let mut modelled: Vec<&str> = class.methods.keys().map(String::as_str).collect();
            modelled.sort_unstable();
            assert_eq!(modelled, runtime, "{}", global.name);
        }
        for name in ["Xml", "XmlNode", "XmlDocument", "XmlReader"] {
            assert!(
                crate::types::runtime_globals::runtime_globals()
                    .iter()
                    .any(|g| g.name == name),
                "{name} is no runtime global"
            );
        }
    }

    /// A zero-parameter signature here is a native the runtime runs bare,
    /// and a native it runs bare either takes no parameter here or is typed
    /// as its call when read bare (`runtime_calls_bare`).
    #[test]
    fn zero_parameter_signatures_are_the_natives_run_bare() {
        use crate::types::runtime_globals::runtime_calls_bare;
        for class in xml_classes() {
            for (name, method) in &class.methods {
                if method.params.is_empty() {
                    assert!(
                        runtime_calls_bare(&class.name, name, method.is_static),
                        "{}.{name} is declared without parameters but is not run bare",
                        class.name
                    );
                }
            }
        }
        assert!(runtime_calls_bare("XmlNode", "to_xml", false));
        assert!(runtime_calls_bare("Xml", "document", true));
        assert!(!runtime_calls_bare("Xml", "parse", true));
        assert!(!runtime_calls_bare("XmlNode", "get", false));
    }
}
