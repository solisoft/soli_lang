//! Tables of the XPath subset: what compiles, what each construct selects in
//! both flavours, namespaces, detached fragments, the limits and the visit
//! budget. Every test returns `Result` (no `unwrap` under `src/xml`).

use super::*;
use crate::xml::{parse, ParsePolicy};

type TestResult = Result<(), String>;

const RSS: &str = concat!(
    r#"<rss version="2.0"><channel><title>News</title>"#,
    r#"<item id="1"><title>A</title><tag>x</tag><tag>y</tag></item>"#,
    r#"<item id="2" kind="b"><title>B &amp; C</title><!--c--><?p d?></item>"#,
    r#"<item id="3"><title>A</title>tail<![CDATA[cd]]></item>"#,
    r#"</channel></rss>"#
);

const ATOM: &str = concat!(
    r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:x="urn:x"><title>T</title>"#,
    r#"<entry><title>E1</title><x:meta x:k="v" k="w"/></entry>"#,
    r#"<entry><title>E2</title></entry></feed>"#
);

const ATOM_NS: &str = "http://www.w3.org/2005/Atom";

fn doc(xml: &str) -> Result<XmlTree, String> {
    parse(xml, &ParsePolicy::xml()).map_err(|e| format!("{xml:?}: {e}"))
}

fn owned(ns: &[(&str, &str)]) -> Vec<(String, String)> {
    ns.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn render(tree: &XmlTree, items: &[Item]) -> Vec<String> {
    items
        .iter()
        .map(|item| match (item, item.string(tree)) {
            (_, Some(s)) => format!("={s}"),
            (Item::Node(id), None) => tree.path(*id),
            _ => String::new(),
        })
        .collect()
}

fn eval_with(
    tree: &mut XmlTree,
    context: NodeId,
    expr: &str,
    flavour: Flavour,
    ns: &[(&str, &str)],
    max_visits: u64,
) -> Result<Vec<Item>, XPathError> {
    let xpath = XPath::compile(expr)?;
    let namespaces = owned(ns);
    select(
        tree,
        context,
        &xpath,
        &Query {
            flavour,
            namespaces: &namespaces,
            max_visits,
        },
    )
}

fn run(
    tree: &mut XmlTree,
    context: NodeId,
    expr: &str,
    flavour: Flavour,
    ns: &[(&str, &str)],
) -> Result<Vec<String>, String> {
    let items = eval_with(tree, context, expr, flavour, ns, 1_000_000)
        .map_err(|e| format!("{expr:?}: {e}"))?;
    Ok(render(tree, &items))
}

fn check_table(
    tree: &mut XmlTree,
    context: NodeId,
    flavour: Flavour,
    ns: &[(&str, &str)],
    table: &[(&str, &[&str])],
) -> TestResult {
    let mut failures = Vec::new();
    for (expr, want) in table {
        let got = run(tree, context, expr, flavour, ns)?;
        if got != *want {
            failures.push(format!("{expr:?}: got {got:?}, want {want:?}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn compile_error(expr: &str) -> Result<String, String> {
    match XPath::compile(expr) {
        Ok(_) => Err(format!("{expr:?} compiled; expected a refusal")),
        Err(e) => Ok(e.to_string()),
    }
}

const I1: &str = "/rss/channel/item[1]";
const I2: &str = "/rss/channel/item[2]";
const I3: &str = "/rss/channel/item[3]";

#[test]
fn the_grammar_compiles_every_documented_form() -> TestResult {
    let forms = [
        "/",
        "//a",
        "a/b//c",
        ".",
        "..",
        "./a/../b",
        "@id",
        "a/@*",
        "a/@p:id",
        "a/@*:id",
        "a/@p:*",
        "*",
        "p:*",
        "*:a",
        "p:a",
        "text()",
        "a/node()",
        "a/comment()",
        "a/processing-instruction()",
        "a/processing-instruction('t')",
        "a | b | //c/@d",
        "a[1]",
        "a[last()]",
        "a[@id]",
        "a[@id='1']",
        "a[@id!=\"1\"]",
        "a[b]",
        "a[b='x']",
        "a[text()='x']",
        "a[.='x']",
        "a[not(@id)]",
        "a[(@x or @y) and not(b)]",
        "a[contains(@id, 'x')]",
        "a[starts-with(b, 'x')]",
        "a[position() = 1]",
        "a[position() != 1]",
        "a[position() < 2][position() >= 1][position() <= 2][position() > 0]",
        "a[*:b]",
        "a[p:*]",
        "a[@*]",
        "text()[1]",
        " / a [ @ id = 'x' ] ",
        "or[and]",
        "a-b.c/_d",
    ];
    for form in forms {
        XPath::compile(form).map_err(|e| format!("{form:?}: {e}"))?;
    }
    Ok(())
}

#[test]
fn unsupported_xpath_is_refused_with_its_column() -> TestResult {
    let table = [
        (
            "sum(//a)",
            "unsupported XPath at column 1: function 'sum'",
        ),
        (
            "//a[count(b)]",
            "unsupported XPath at column 5: function 'count'",
        ),
        ("child::a", "unsupported XPath at column 1: axis 'child::'"),
        ("a/descendant::b", "unsupported XPath at column 3: axis 'descendant::'"),
        ("$x", "unsupported XPath at column 1: variable '$x'"),
        ("a + b", "unsupported XPath at column 3: arithmetic"),
        ("a[b div 2]", "unsupported XPath at column 5: arithmetic"),
        (
            "//a[@id=1]",
            "unsupported XPath at column 9: numeric comparison: quote the value to compare it as a string ('1')",
        ),
        (
            "a[@n > 2]",
            "unsupported XPath at column 6: numeric comparison (only position() compares numbers)",
        ),
        ("1", "unsupported XPath at column 1: a number here"),
        (
            "a | (//a)[1]",
            "unsupported XPath at column 5: a parenthesised path: for (//a)[1] write at_xpath(\"//a\")",
        ),
        ("a[1.5]", "unsupported XPath at column 3: a number here"),
        (
            "@id/x",
            "unsupported XPath at column 4: '@attribute' must be the last step of a path",
        ),
        (
            "a/text()/b",
            "unsupported XPath at column 9: 'text()' must be the last step of a path",
        ),
        (
            "a[b/c]",
            "unsupported XPath at column 4: a path inside a predicate (one child step or attribute only)",
        ),
        (
            "a[b[c]]",
            "unsupported XPath at column 4: a predicate inside a predicate",
        ),
        ("a[..]", "unsupported XPath at column 3: '..' inside a predicate"),
        (".[1]", "unsupported XPath at column 2: a predicate on '.'"),
        ("@id[1]", "unsupported XPath at column 4: a predicate on an attribute step"),
        (
            "a[last()-1]",
            "unsupported XPath at column 3: last() inside an expression: write [last()] on its own",
        ),
        (
            "a[position() < last()]",
            "unsupported XPath at column 16: position() must be compared with an integer (position() > 1)",
        ),
        ("'abc", "invalid XPath at column 1: unterminated string literal"),
        ("a[", "invalid XPath at column 3: unexpected end of the expression"),
        ("a[@id='x'", "invalid XPath at column 10: expected ']', found the end of the expression"),
        ("", "invalid XPath at column 1: unexpected end of the expression"),
        ("a b", "invalid XPath at column 3: unexpected 'b'"),
        ("//", "invalid XPath at column 3: unexpected end of the expression"),
        ("a/", "invalid XPath at column 3: unexpected end of the expression"),
        ("@", "invalid XPath at column 2: expected an attribute name after '@'"),
        ("a[contains(@x, b)]", "invalid XPath at column 16: expected a quoted string, found 'b'"),
        ("a#", "invalid XPath at column 2: unexpected '#'"),
    ];
    let mut failures = Vec::new();
    for (expr, want) in table {
        let got = compile_error(expr)?;
        if got != want {
            failures.push(format!("{expr:?}: got {got:?}, want {want:?}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

#[test]
fn expression_limits_are_enforced_at_n_and_n_plus_one() -> TestResult {
    // Length.
    let at = format!("a[@x='{}']", "y".repeat(MAX_EXPR_BYTES - 8));
    XPath::compile(&at).map_err(|e| format!("4096 bytes: {e}"))?;
    let over = format!("a[@x='{}']", "y".repeat(MAX_EXPR_BYTES - 7));
    let err = compile_error(&over)?;
    if err != "XPath too complex: the expression is longer than 4096 bytes" {
        return Err(err);
    }
    // Steps, all branches together.
    let steps = |n: usize| vec!["a"; n].join("/");
    XPath::compile(&steps(MAX_STEPS)).map_err(|e| e.to_string())?;
    if compile_error(&steps(MAX_STEPS + 1))? != "XPath too complex: more than 64 steps" {
        return Err("65 steps".into());
    }
    let union = format!("{} | {}", steps(32), steps(33));
    if compile_error(&union)? != "XPath too complex: more than 64 steps" {
        return Err("65 steps across a union".into());
    }
    // A bare `/` is a step too, so a union of them is bounded.
    XPath::compile(&vec!["/"; MAX_STEPS].join("|")).map_err(|e| e.to_string())?;
    if compile_error(&vec!["/"; MAX_STEPS + 1].join("|"))? != "XPath too complex: more than 64 steps"
    {
        return Err("65 bare slashes".into());
    }
    // Predicates per step.
    let preds = |n: usize| format!("a{}", "[@x]".repeat(n));
    XPath::compile(&preds(MAX_PREDICATES)).map_err(|e| e.to_string())?;
    if compile_error(&preds(MAX_PREDICATES + 1))?
        != "XPath too complex: more than 8 predicates on one step"
    {
        return Err("9 predicates".into());
    }
    // Tests per predicate.
    let terms = |n: usize| format!("a[{}]", vec!["@x"; n].join(" or "));
    XPath::compile(&terms(MAX_TERMS)).map_err(|e| e.to_string())?;
    if compile_error(&terms(MAX_TERMS + 1))?
        != "XPath too complex: more than 16 tests in one predicate"
    {
        return Err("17 terms".into());
    }
    // Nesting.
    let nest = |n: usize| format!("a[{}@x{}]", "(".repeat(n), ")".repeat(n));
    XPath::compile(&nest(MAX_NESTING)).map_err(|e| e.to_string())?;
    if compile_error(&nest(MAX_NESTING + 1))?
        != "XPath too complex: not(…) or (…) nested more than 16 deep"
    {
        return Err("17 levels".into());
    }
    // Deep nesting far past the limit is refused, not a stack overflow.
    compile_error(&nest(2000))?;
    Ok(())
}

#[test]
fn strict_xpath_semantics_table() -> TestResult {
    let mut tree = doc(RSS)?;
    let table: &[(&str, &[&str])] = &[
        ("/", &["/"]),
        (".", &["/"]),
        ("rss", &["/rss"]),
        ("channel", &[]),
        ("/rss/channel/item", &[I1, I2, I3]),
        ("//item", &[I1, I2, I3]),
        ("rss//item", &[I1, I2, I3]),
        ("//item/@id", &["=1", "=2", "=3"]),
        ("//@*", &["=2.0", "=1", "=2", "=b", "=3"]),
        ("//item[@id='2']/title/text()", &["=B & C"]),
        ("//item[2]", &[I2]),
        ("//item[last()]", &[I3]),
        ("//item[4]", &[]),
        ("//item[0]", &[]),
        ("//item[position()>1]", &[I2, I3]),
        ("//item[position()=2]", &[I2]),
        ("//item[position()!=2]", &[I1, I3]),
        ("//item[title='A']", &[I1, I3]),
        ("//item[title!='A']", &[I2]),
        ("//item[tag='y']", &[I1]),
        ("//item[tag!='x']", &[I1]),
        ("//item[tag]", &[I1]),
        ("//item[not(tag)]", &[I2, I3]),
        ("//item[not(@kind)]", &[I1, I3]),
        ("//item[@kind and @id='2']", &[I2]),
        ("//item[@id='1' or @id='3']", &[I1, I3]),
        ("//item[(@id='1' or @id='2') and not(@kind)]", &[I1]),
        ("//item[@missing!='x']", &[]),
        ("//item[contains(title,'&')]", &[I2]),
        ("//item[contains(@missing,'')]", &[I1, I2, I3]),
        ("//item[starts-with(@id,'2')]", &[I2]),
        ("//item[contains(tag,'y')]", &[]),
        ("//item[text()='tail']", &[I3]),
        ("//item[text()='cd']", &[I3]),
        ("//item[text()]", &[I3]),
        ("//item[.='Axy']", &[I1]),
        ("//item[@kind][1]", &[I2]),
        ("//item[1][@kind]", &[]),
        ("//item[@id!='1'][2]", &[I3]),
        ("//title[1]", &["/rss/channel/title", "/rss/channel/item[1]/title", "/rss/channel/item[2]/title", "/rss/channel/item[3]/title"]),
        ("//item[1]/tag[2]", &["/rss/channel/item[1]/tag[2]"]),
        ("//item/..", &["/rss/channel"]),
        ("//title/../@id", &["=1", "=2", "=3"]),
        ("//item[2]/../item[3]", &[I3]),
        ("//comment()", &["/rss/channel/item[2]/comment()"]),
        ("//processing-instruction()", &["/rss/channel/item[2]/processing-instruction()"]),
        ("//processing-instruction('p')", &["/rss/channel/item[2]/processing-instruction()"]),
        ("//processing-instruction('q')", &[]),
        ("//item[3]/node()", &["/rss/channel/item[3]/title", "/rss/channel/item[3]/text()[1]", "/rss/channel/item[3]/text()[2]"]),
        ("//item[3]/text()", &["=tail", "=cd"]),
        ("//item[3]/text()[2]", &["=cd"]),
        ("//*[@*]", &["/rss", I1, I2, I3]),
        ("//item/title | /rss/channel/title", &["/rss/channel/title", "/rss/channel/item[1]/title", "/rss/channel/item[2]/title", "/rss/channel/item[3]/title"]),
        ("//item | //item[1] | //item", &[I1, I2, I3]),
        ("//item/@kind | //item[2]", &[I2, "=b"]),
        ("//*:item[@id='3']", &[I3]),
        ("//item//text()", &["=A", "=x", "=y", "=B & C", "=A", "=tail", "=cd"]),
        ("/rss/./channel/./item[1]/.", &[I1]),
        ("/..", &[]),
    ];
    check_table(&mut tree, NodeId::DOCUMENT, Flavour::Strict, &[], table)
}

#[test]
fn lenient_find_starts_at_the_root_element_on_a_document() -> TestResult {
    let mut tree = doc(RSS)?;
    let table: &[(&str, &[&str])] = &[
        ("channel/title", &["/rss/channel/title"]),
        ("rss", &[]),
        (".", &["/rss"]),
        ("..", &["/"]),
        ("channel/item[@id='2']", &[I2]),
        ("/rss", &["/rss"]),
        ("//item[last()]", &[I3]),
        ("channel/item/@id", &["=1", "=2", "=3"]),
        ("channel/item | /rss/channel", &["/rss/channel", I1, I2, I3]),
    ];
    check_table(&mut tree, NodeId::DOCUMENT, Flavour::Lenient, &[], table)?;
    // From an element, both flavours start at the element.
    let channel = tree
        .element_children(tree.root_element().ok_or("no root")?)
        .next()
        .ok_or("no channel")?;
    for flavour in [Flavour::Lenient, Flavour::Strict] {
        let got = run(&mut tree, channel, "item[2]/title", flavour, &[])?;
        if got != ["/rss/channel/item[2]/title"] {
            return Err(format!("{flavour:?}: {got:?}"));
        }
    }
    Ok(())
}

#[test]
fn namespaces_resolve_per_flavour() -> TestResult {
    let mut tree = doc(ATOM)?;
    let e1 = "/feed/entry[1]";
    let e2 = "/feed/entry[2]";
    let strict: &[(&str, &[&str])] = &[
        ("//entry", &[]),
        ("//xmlns:entry", &[e1, e2]),
        ("//*:entry", &[e1, e2]),
        ("/xmlns:feed/xmlns:entry/xmlns:title/text()", &["=E1", "=E2"]),
        ("//x:*", &["/feed/entry[1]/x:meta"]),
        ("//x:meta/@x:k", &["=v"]),
        ("//x:meta/@k", &["=w"]),
        ("//x:meta/@*:k", &["=v", "=w"]),
        ("//x:meta/@x:*", &["=v"]),
        ("//xmlns:entry[xmlns:title='E2']", &[e2]),
        ("//xmlns:entry[title='E2']", &[]),
        ("//xmlns:entry[*:title='E2']", &[e2]),
    ];
    check_table(&mut tree, NodeId::DOCUMENT, Flavour::Strict, &[], strict)?;

    let lenient: &[(&str, &[&str])] = &[
        ("entry", &[e1, e2]),
        ("entry/title", &["/feed/entry[1]/title", "/feed/entry[2]/title"]),
        ("entry[title='E2']", &[e2]),
        ("//meta", &["/feed/entry[1]/x:meta"]),
        ("entry/x:meta/@k", &["=w"]),
    ];
    check_table(&mut tree, NodeId::DOCUMENT, Flavour::Lenient, &[], lenient)?;

    // The `""` key makes unprefixed tests mean exactly that URI, in both.
    let with_default = [("", ATOM_NS)];
    for flavour in [Flavour::Strict, Flavour::Lenient] {
        let got = run(&mut tree, NodeId::DOCUMENT, "//entry", flavour, &with_default)?;
        if got != [e1, e2] {
            return Err(format!("{flavour:?} with the \"\" key: {got:?}"));
        }
        let got = run(&mut tree, NodeId::DOCUMENT, "//meta", flavour, &with_default)?;
        if !got.is_empty() {
            return Err(format!("{flavour:?}: meta is not in the default namespace: {got:?}"));
        }
    }
    let other = [("", "urn:other")];
    if !run(&mut tree, NodeId::DOCUMENT, "entry", Flavour::Lenient, &other)?.is_empty() {
        return Err("a \"\" key with another URI must match nothing".into());
    }
    // `""` mapped to "" means no namespace.
    let none = [("", "")];
    if !run(&mut tree, NodeId::DOCUMENT, "entry", Flavour::Lenient, &none)?.is_empty() {
        return Err("\"\" => \"\" must mean no namespace".into());
    }

    // The table wins over the root's declarations, and binds any name.
    let a = [("a", ATOM_NS), ("x", "urn:y")];
    let got = run(&mut tree, NodeId::DOCUMENT, "//a:entry/a:title/text()", Flavour::Strict, &a)?;
    if got != ["=E1", "=E2"] {
        return Err(format!("table prefix: {got:?}"));
    }
    if !run(&mut tree, NodeId::DOCUMENT, "//x:meta", Flavour::Strict, &a)?.is_empty() {
        return Err("the table must override the root's x".into());
    }

    // An unknown prefix is refused, even if its step is never reached.
    for flavour in [Flavour::Strict, Flavour::Lenient] {
        match eval_with(&mut tree, NodeId::DOCUMENT, "//nothing/q:entry", flavour, &[], 1000) {
            Err(XPathError {
                kind: XPathErrorKind::UnboundPrefix(p),
            }) if p == "q" => {}
            other => return Err(format!("{flavour:?}: {other:?}")),
        }
    }
    // `xml` is always bound.
    let mut tree = doc(r#"<a xml:lang="fr"/>"#)?;
    let got = run(&mut tree, NodeId::DOCUMENT, "a/@xml:lang", Flavour::Strict, &[])?;
    if got != ["=fr"] {
        return Err(format!("xml:lang: {got:?}"));
    }
    // `xmlns:` with no default namespace on the root is unbound.
    let mut tree = doc("<a/>")?;
    match eval_with(&mut tree, NodeId::DOCUMENT, "xmlns:a", Flavour::Strict, &[], 1000) {
        Err(e) if e.to_string() == "unbound prefix 'xmlns'" => Ok(()),
        other => Err(format!("{other:?}")),
    }
}

#[test]
fn factur_x_seller_and_lines() -> TestResult {
    let ram = "urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100";
    let xml = format!(
        concat!(
            r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:rsm" xmlns:ram="{ram}">"#,
            r#"<rsm:SupplyChainTradeTransaction>"#,
            r#"<ram:IncludedSupplyChainTradeLineItem><ram:Name>Widget</ram:Name></ram:IncludedSupplyChainTradeLineItem>"#,
            r#"<ram:IncludedSupplyChainTradeLineItem><ram:Name>Gadget</ram:Name></ram:IncludedSupplyChainTradeLineItem>"#,
            r#"<ram:ApplicableHeaderTradeAgreement><ram:SellerTradeParty><ram:Name>Soli SARL</ram:Name></ram:SellerTradeParty></ram:ApplicableHeaderTradeAgreement>"#,
            r#"</rsm:SupplyChainTradeTransaction></rsm:CrossIndustryInvoice>"#
        ),
        ram = ram
    );
    let mut tree = doc(&xml)?;
    // With a table (another prefix for the same URI) and with the root fallback.
    for (expr, ns) in [
        ("//r:SellerTradeParty/r:Name/text()", vec![("r", ram)]),
        ("//ram:SellerTradeParty/ram:Name/text()", vec![]),
    ] {
        let got = run(&mut tree, NodeId::DOCUMENT, expr, Flavour::Strict, &ns)?;
        if got != ["=Soli SARL"] {
            return Err(format!("{expr}: {got:?}"));
        }
    }
    let lines = run(
        &mut tree,
        NodeId::DOCUMENT,
        "//ram:IncludedSupplyChainTradeLineItem/ram:Name/text()",
        Flavour::Strict,
        &[],
    )?;
    if lines != ["=Widget", "=Gadget"] {
        return Err(format!("{lines:?}"));
    }
    // `find` ignores namespaces for unprefixed names.
    let got = run(
        &mut tree,
        NodeId::DOCUMENT,
        "SupplyChainTradeTransaction/IncludedSupplyChainTradeLineItem[Name='Gadget']/Name/text()",
        Flavour::Lenient,
        &[],
    )?;
    if got != ["=Gadget"] {
        return Err(format!("lenient: {got:?}"));
    }
    Ok(())
}

#[test]
fn detached_fragments_have_a_virtual_root() -> TestResult {
    let (mut tree, a) = XmlTree::new_fragment(100, "a", &[]).map_err(|e| e.to_string())?;
    let b = tree.add_element(a, "b", &[("k", "1")]).map_err(|e| e.to_string())?;
    tree.add_element(b, "b", &[]).map_err(|e| e.to_string())?;
    tree.add_element(a, "c", &[]).map_err(|e| e.to_string())?;
    let table: &[(&str, &[&str])] = &[
        ("//b", &["a/b", "a/b/b"]),
        ("/a/c", &["a/c"]),
        ("/a", &["a"]),
        ("/", &[]),
        ("..", &[]),
        ("b/..", &["a"]),
        ("//b[@k]/@k", &["=1"]),
        ("/*", &["a"]),
        ("//*", &["a", "a/b", "a/b/b", "a/c"]),
    ];
    for flavour in [Flavour::Strict, Flavour::Lenient] {
        check_table(&mut tree, a, flavour, &[], table)?;
    }
    // A node removed from a parsed document: absolute paths stay in its
    // fragment.
    let mut tree = doc(RSS)?;
    let item = tree.descendants(NodeId::DOCUMENT)[3];
    tree.detach(item).map_err(|e| e.to_string())?;
    let got = run(&mut tree, item, "//title/text()", Flavour::Strict, &[])?;
    if got != ["=A"] {
        return Err(format!("detached: {got:?}"));
    }
    let got = run(&mut tree, NodeId::DOCUMENT, "//item/@id", Flavour::Strict, &[])?;
    if got != ["=2", "=3"] {
        return Err(format!("document after detach: {got:?}"));
    }
    // A dup keeps the prefixes its output declares for the root fallback.
    let mut tree = doc(ATOM)?;
    let entry = tree.descendants(NodeId::DOCUMENT)[2];
    let (mut copy, top) = tree.dup(entry).map_err(|e| e.to_string())?;
    let got = run(&mut copy, top, "xmlns:title/text() | *:meta/@*:k", Flavour::Strict, &[])?;
    if got != ["=E1", "=v", "=w"] {
        return Err(format!("dup: {got:?}"));
    }
    Ok(())
}

#[test]
fn results_follow_document_order_after_edits() -> TestResult {
    let mut tree = doc(RSS)?;
    let channel = tree.descendants(NodeId::DOCUMENT)[1];
    let fresh = tree
        .create_element("item", &[("id", "0")], None)
        .map_err(|e| e.to_string())?;
    tree.prepend_child(channel, fresh).map_err(|e| e.to_string())?;
    let last = tree.descendants(NodeId::DOCUMENT)[4];
    tree.append_child(channel, last).map_err(|e| e.to_string())?;
    let got = run(&mut tree, NodeId::DOCUMENT, "//item/@id | //item[1]", Flavour::Strict, &[])?;
    if got != ["/rss/channel/item[1]", "=0", "=2", "=3", "=1"] {
        return Err(format!("{got:?}"));
    }
    Ok(())
}

/// A chain `depth` elements deep, built (a parse caps depth at 256).
fn chain(depth: usize) -> Result<XmlTree, String> {
    let (mut tree, root) =
        XmlTree::new_document(depth + 10, Some(("a", &[]))).map_err(|e| e.to_string())?;
    let mut node = root.ok_or("no root")?;
    for _ in 1..depth {
        node = tree.add_element(node, "a", &[]).map_err(|e| e.to_string())?;
    }
    tree.add_text(node, "x").map_err(|e| e.to_string())?;
    Ok(tree)
}

#[test]
fn the_visit_budget_bounds_adversarial_queries() -> TestResult {
    let mut tree = chain(5000)?;
    // `//*//*//*` stays linear: each `//` skips contexts already covered.
    let items = eval_with(&mut tree, NodeId::DOCUMENT, "//*//*//*", Flavour::Strict, &[], 60_000)
        .map_err(|e| e.to_string())?;
    if items.len() != 4998 {
        return Err(format!("{} nodes", items.len()));
    }
    // `.` reads every subtree: quadratic, so the budget stops it.
    match eval_with(&mut tree, NodeId::DOCUMENT, "//*[.='y']", Flavour::Strict, &[], 1_000_000) {
        Err(e) if e.kind == XPathErrorKind::Budget(1_000_000) => {}
        other => return Err(format!("expected the budget error, got {other:?}")),
    }
    let err = eval_with(&mut tree, NodeId::DOCUMENT, "//*", Flavour::Strict, &[], 10)
        .err()
        .ok_or("expected a budget error")?;
    if err.to_string() != "evaluation exceeded 10 node visits (SOLI_MAX_XPATH_VISITS)" {
        return Err(err.to_string());
    }
    // Long text copied once per ancestor is charged by its length: 32
    // ancestors over 64 KiB of text cost far more than 32 x 2 visits.
    let mut long = chain(32)?;
    let text = "y".repeat(64 * 1024);
    let leaf = long.descendants(NodeId::DOCUMENT)[31];
    long.add_text(leaf, &text).map_err(|e| e.to_string())?;
    match eval_with(&mut long, NodeId::DOCUMENT, "//*[.='z']", Flavour::Strict, &[], 20_000) {
        Err(e) if e.kind == XPathErrorKind::Budget(20_000) => {}
        other => return Err(format!("long text: expected the budget error, got {other:?}")),
    }
    // The same queries within budget on a small tree.
    let mut small = chain(10)?;
    let got = eval_with(&mut small, NodeId::DOCUMENT, "//*[.='x']", Flavour::Strict, &[], 1000)
        .map_err(|e| e.to_string())?;
    if got.len() != 10 {
        return Err(format!("{} matches", got.len()));
    }
    Ok(())
}

#[test]
fn a_context_from_elsewhere_is_an_error_not_a_panic() -> TestResult {
    let mut tree = doc("<a/>")?;
    let xpath = XPath::compile("//a").map_err(|e| e.to_string())?;
    let query = Query {
        flavour: Flavour::Strict,
        namespaces: &[],
        max_visits: 100,
    };
    match select(&mut tree, NodeId(99), &xpath, &query) {
        Err(XPathError {
            kind: XPathErrorKind::BadContext,
        }) => Ok(()),
        other => Err(format!("{other:?}")),
    }
}
