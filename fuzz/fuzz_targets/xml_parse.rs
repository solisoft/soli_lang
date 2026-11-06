#![no_main]
//! Fuzz the XML core (`solilang::xml`). Any input must parse to a tree or give
//! a clean error under every caller's policy — never a panic or a stack
//! overflow — and whatever the strict `Xml.parse` policy accepts must
//! round-trip: serialise it, parse the output, and get the same tree back.
//! The round trip runs on the `doctype: "skip"` tree, a superset of the
//! default policy's.

use libfuzzer_sys::fuzz_target;
use solilang::xml::{
    parse, serialize, soap_shape, to_h, DoctypeMode, NodeId, ParsePolicy, SerializeOptions,
    XmlTree,
};

/// Every element under `id` in document order, by expanded name, with its
/// attributes by expanded name (prefixes may be renamed on output).
fn expanded(
    tree: &XmlTree,
    id: NodeId,
) -> Vec<(
    Option<String>,
    String,
    Vec<(Option<String>, String, String)>,
)> {
    tree.descendants(id)
        .into_iter()
        .filter_map(|n| tree.element(n))
        .map(|el| {
            let mut attrs: Vec<_> = el
                .attrs
                .iter()
                .map(|a| {
                    (
                        a.ns_uri.as_deref().map(str::to_string),
                        a.local_name().to_string(),
                        a.value.to_string(),
                    )
                })
                .collect();
            attrs.sort();
            (
                el.ns_uri.map(|u| u.to_string()),
                el.local_name.to_string(),
                attrs,
            )
        })
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        // Bytes that are not UTF-8 must be refused cleanly too.
        let _ = solilang::xml::parse_bytes(data, &ParsePolicy::xml());
        return;
    };

    // The lenient callers: no panic is the whole contract.
    if let Ok(tree) = parse(text, &ParsePolicy::soap()) {
        let _ = soap_shape(&tree).to_json_string();
    }
    let _ = parse(text, &ParsePolicy::c14n());

    // `doctype: "skip"` accepts everything the default accepts, to the same
    // tree, and reads past a DOCTYPE (internal subset included) without ever
    // declaring an entity. The rest of the run works on its tree: the
    // serialiser never writes a DOCTYPE, so the round trip reparses under the
    // default policy.
    let refused = parse(text, &ParsePolicy::xml());
    let skipped = parse(text, &ParsePolicy::xml().with_doctype(DoctypeMode::Skip));
    if let (Ok(strict), Ok(lenient)) = (&refused, &skipped) {
        assert!(
            strict.subtree_eq(NodeId::DOCUMENT, lenient, NodeId::DOCUMENT),
            "doctype: skip changed a document without a DOCTYPE"
        );
    }
    assert!(
        refused.is_err() || skipped.is_ok(),
        "doctype: skip refused what the default accepts"
    );
    let Ok(tree) = skipped else {
        return;
    };
    let _ = to_h(&tree, NodeId::DOCUMENT);
    let opts = SerializeOptions::compact();
    let written = serialize(&tree, NodeId::DOCUMENT, &opts).expect("a parsed tree serialises");
    let again = parse(&written, &ParsePolicy::xml()).expect("the serialiser's output parses");
    assert!(
        tree.subtree_eq(NodeId::DOCUMENT, &again, NodeId::DOCUMENT),
        "round trip changed the tree: {written:?}"
    );
    // Indented output must parse too (it may drop blank text, so no equality).
    let indented = SerializeOptions {
        indent: Some(2),
        ..opts
    };
    let pretty = serialize(&tree, NodeId::DOCUMENT, &indented).expect("indented output");
    parse(&pretty, &ParsePolicy::xml()).expect("the indented output parses");
    // Every element's standalone fragment parses on its own.
    for node in tree.descendants(NodeId::DOCUMENT) {
        let fragment = serialize(&tree, node, &opts).expect("a fragment serialises");
        parse(&fragment, &ParsePolicy::xml()).expect("a fragment parses on its own");
    }

    // Mutate, then serialise and reparse: moves and attributes set in a new
    // scope must keep every element's and attribute's expanded name, and the
    // output must stay well-formed (no prefix rebound, no duplicate xmlns).
    let mut edited = tree.clone();
    let elements = edited.descendants(NodeId::DOCUMENT);
    let n = elements.len();
    let seed = data.len();
    for (i, &el) in elements.iter().enumerate().take(64) {
        let target = elements[(i * 7 + seed) % n];
        let _ = edited.append_child(target, el);
        let prefixes: Vec<Option<Box<str>>> = tree
            .in_scope_namespaces(el)
            .into_iter()
            .map(|d| d.prefix)
            .take(4)
            .collect();
        for (k, prefix) in prefixes.iter().enumerate() {
            if let Some(p) = prefix {
                let _ = edited.set_attribute(el, &format!("{p}:fz{k}"), Some("v"));
            }
        }
        // A copy of the element can be built on with its own prefixes.
        if let (Ok((mut copy, id)), Some(name)) = (edited.dup(el), edited.name(el)) {
            copy.add_element(id, name, &[])
                .expect("a dup accepts its own element name");
        }
    }
    let written = serialize(&edited, NodeId::DOCUMENT, &opts).expect("an edited tree serialises");
    let again = parse(&written, &ParsePolicy::xml()).expect("an edited tree's output parses");
    assert_eq!(
        expanded(&edited, NodeId::DOCUMENT),
        expanded(&again, NodeId::DOCUMENT),
        "editing changed a namespace: {written:?}"
    );
    for node in edited.descendants(NodeId::DOCUMENT) {
        let fragment = serialize(&edited, node, &opts).expect("an edited fragment serialises");
        parse(&fragment, &ParsePolicy::xml()).expect("an edited fragment parses on its own");
    }
});
