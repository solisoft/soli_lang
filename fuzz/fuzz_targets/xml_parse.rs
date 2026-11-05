#![no_main]
//! Fuzz the XML core (`solilang::xml`). Any input must parse to a tree or give
//! a clean error under every caller's policy — never a panic or a stack
//! overflow — and whatever the strict `Xml.parse` policy accepts must
//! round-trip: serialise it, parse the output, and get the same tree back.

use libfuzzer_sys::fuzz_target;
use solilang::xml::{parse, serialize, soap_shape, to_h, NodeId, ParsePolicy, SerializeOptions};

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

    let Ok(tree) = parse(text, &ParsePolicy::xml()) else {
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
});
