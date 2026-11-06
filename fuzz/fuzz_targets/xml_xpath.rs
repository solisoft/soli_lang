#![no_main]
//! Fuzz the XPath subset (`solilang::xml::xpath`). Any expression must
//! compile or be refused cleanly, and a compiled one must evaluate — in both
//! flavours, from several context nodes, with and without a namespace
//! table — to results in document order without duplicates, or stop with a
//! clean error (an unbound prefix, the visit budget). Never a panic, never a
//! stack overflow, never past the budget.

use libfuzzer_sys::fuzz_target;
use solilang::xml::{
    parse, select, Flavour, Item, NodeId, ParsePolicy, Query, XPath, XPathErrorKind, XmlTree,
};

const DOC: &str = concat!(
    r#"<?xml version="1.0"?><!--pre--><feed xmlns="urn:atom" xmlns:x="urn:x" v="1">"#,
    r#"<title>T &amp; t</title><entry id="1" x:k="a"><title>E1</title><x:meta k="w"/>"#,
    r#"tail<![CDATA[cd]]><?pi data?></entry><entry id="2"><title>E2</title><!--c-->"#,
    r#"<entry id="3"><title/></entry></entry><plain xmlns=""><a><a><a>deep</a></a></a></plain>"#,
    r#"</feed><?post?>"#
);

const BUDGET: u64 = 50_000;

fn key(tree: &XmlTree, item: &Item) -> (u32, usize, u8) {
    match *item {
        Item::Node(id) => (tree.order(id), 0, 0),
        Item::Text(id) => (tree.order(id), 0, 1),
        Item::Attr { element, index } => (tree.order(element), index + 1, 0),
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(expr) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(xpath) = XPath::compile(expr) else {
        return;
    };
    let Ok(mut tree) = parse(DOC, &ParsePolicy::xml()) else {
        return;
    };
    // A detached fragment too: absolute paths use its virtual root.
    let entries: Vec<NodeId> = tree
        .descendants(NodeId::DOCUMENT)
        .into_iter()
        .filter(|id| tree.element(*id).is_some_and(|el| el.local_name == "entry"))
        .collect();
    if let Some(last) = entries.last() {
        let _ = tree.detach(*last);
    }
    let mut contexts = vec![NodeId::DOCUMENT];
    contexts.extend(tree.root_element());
    contexts.extend(entries.first().copied());
    contexts.extend(entries.last().copied());

    let tables: [Vec<(String, String)>; 2] = [
        Vec::new(),
        vec![
            (String::new(), "urn:atom".to_string()),
            ("a".to_string(), "urn:atom".to_string()),
            ("x".to_string(), "urn:other".to_string()),
        ],
    ];
    for flavour in [Flavour::Lenient, Flavour::Strict] {
        for table in &tables {
            for &context in &contexts {
                let query = Query {
                    flavour,
                    namespaces: table,
                    max_visits: BUDGET,
                };
                match select(&mut tree, context, &xpath, &query) {
                    Ok(items) => {
                        for pair in items.windows(2) {
                            assert!(
                                key(&tree, &pair[0]) < key(&tree, &pair[1]),
                                "{expr:?}: results out of order or duplicated: {items:?}"
                            );
                        }
                        for item in &items {
                            if let Item::Node(id) = item {
                                assert!(tree.get(*id).is_some(), "{expr:?}: a node from nowhere");
                            }
                            let _ = item.string(&tree);
                        }
                    }
                    Err(err) => assert!(
                        matches!(
                            err.kind,
                            XPathErrorKind::UnboundPrefix(_) | XPathErrorKind::Budget(BUDGET)
                        ),
                        "{expr:?}: unexpected evaluation error {err}"
                    ),
                }
            }
        }
    }
});
