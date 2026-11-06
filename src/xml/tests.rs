//! Unit tests of the XML core. Every test returns `Result` (no `unwrap` or
//! `expect` anywhere under `src/xml`: `scripts/lint_unwraps.sh` watches it).

use super::*;

type TestResult = Result<(), String>;

fn xml(input: &str) -> Result<XmlTree, String> {
    parse(input, &ParsePolicy::xml()).map_err(|e| format!("{input:?}: {e}"))
}

fn with(input: &str, policy: &ParsePolicy) -> Result<XmlTree, String> {
    parse(input, policy).map_err(|e| format!("{input:?}: {e}"))
}

fn fails(input: &str, policy: &ParsePolicy) -> Result<XmlError, String> {
    match parse(input, policy) {
        Ok(_) => Err(format!("expected {input:?} to be refused")),
        Err(err) => Ok(err),
    }
}

fn fails_xml(input: &str) -> Result<XmlError, String> {
    fails(input, &ParsePolicy::xml())
}

fn root(tree: &XmlTree) -> Result<NodeId, String> {
    tree.root_element()
        .ok_or_else(|| "no root element".to_string())
}

fn child(tree: &XmlTree, id: NodeId, n: usize) -> Result<NodeId, String> {
    tree.children(id)
        .get(n)
        .copied()
        .ok_or_else(|| format!("no child {n}"))
}

fn compact(tree: &XmlTree, id: NodeId) -> Result<String, String> {
    serialize(tree, id, &SerializeOptions::compact()).map_err(|e| e.to_string())
}

fn ensure(cond: bool, what: &str) -> TestResult {
    if cond {
        Ok(())
    } else {
        Err(what.to_string())
    }
}

/// parse → serialize → parse gives the same tree.
fn round_trips(input: &str) -> TestResult {
    let first = xml(input)?;
    let written = compact(&first, NodeId::DOCUMENT)?;
    let second = xml(&written)?;
    // The serialiser always writes `encoding="UTF-8"` when a declaration had
    // an encoding: compare the declaration by what it says, not its spelling.
    let decl_key = |t: &XmlTree| {
        t.decl().map(|d| {
            (
                d.version.clone(),
                d.encoding.is_some(),
                d.standalone.clone(),
            )
        })
    };
    ensure(
        first.subtree_eq(NodeId::DOCUMENT, &second, NodeId::DOCUMENT)
            && decl_key(&first) == decl_key(&second),
        &format!("round trip changed the tree: {input:?} -> {written:?}"),
    )
}

// ------------------------------------------------------------------ parse rules

#[test]
fn a_bom_is_stripped() -> TestResult {
    let tree = xml("\u{FEFF}<a>x</a>")?;
    assert_eq!(tree.text(root(&tree)?), "x");
    let tree = parse_bytes(
        b"\xEF\xBB\xBF<?xml version=\"1.0\"?><a/>",
        &ParsePolicy::xml(),
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(tree.decl().map(|d| &*d.version), Some("1.0"));
    Ok(())
}

#[test]
fn invalid_utf8_is_refused_with_a_position() -> TestResult {
    match parse_bytes(b"<a>\n\xFF</a>", &ParsePolicy::xml()) {
        Ok(_) => Err("invalid UTF-8 was accepted".into()),
        Err(err) => {
            assert_eq!(err.kind, XmlErrorKind::InvalidUtf8);
            assert_eq!(err.position, Some((2, 1)));
            Ok(())
        }
    }
}

#[test]
fn line_endings_are_normalised_in_text_comments_cdata_and_pis() -> TestResult {
    let tree = xml("<a>1\r\n2\r3<!--c\r\nd--><![CDATA[e\r\nf]]><?p g\r\nh?></a>")?;
    let a = root(&tree)?;
    assert_eq!(
        tree.kind(child(&tree, a, 0)?),
        &Kind::Text("1\n2\n3".into())
    );
    assert_eq!(
        tree.kind(child(&tree, a, 1)?),
        &Kind::Comment("c\nd".into())
    );
    assert_eq!(tree.kind(child(&tree, a, 2)?), &Kind::CData("e\nf".into()));
    assert_eq!(
        tree.kind(child(&tree, a, 3)?),
        &Kind::Pi {
            target: "p".into(),
            data: "g\nh".into()
        }
    );
    Ok(())
}

#[test]
fn crlf_in_an_attribute_becomes_one_space_before_normalisation() -> TestResult {
    let tree = xml("<a x=\"1\r\n2\" y=\"3\r4\" z=\"5\t6\n7\" w=\"8&#10;9&#13;&#9;\"/>")?;
    let a = root(&tree)?;
    assert_eq!(tree.attribute(a, "x"), Some("1 2"));
    assert_eq!(tree.attribute(a, "y"), Some("3 4"));
    assert_eq!(tree.attribute(a, "z"), Some("5 6 7"));
    // Character references are not normalised away.
    assert_eq!(tree.attribute(a, "w"), Some("8\n9\r\t"));
    Ok(())
}

#[test]
fn entities_and_character_references_are_decoded() -> TestResult {
    let tree = xml("<a t=\"&lt;&gt;&amp;&apos;&quot;&#65;&#x42;\">&lt;&gt;&amp;&apos;&quot;&#67;&#x44;&#x1F600;</a>")?;
    let a = root(&tree)?;
    assert_eq!(tree.attribute(a, "t"), Some("<>&'\"AB"));
    assert_eq!(tree.text(a), "<>&'\"CD😀");
    Ok(())
}

#[test]
fn a_reference_to_a_forbidden_character_is_refused() -> TestResult {
    for input in [
        "<a>&#0;</a>",
        "<a>&#x0;</a>",
        "<a>&#1;</a>",
        "<a>&#xFFFE;</a>",
        "<a>&#xFFFF;</a>",
        "<a>&#xD800;</a>",
        "<a x=\"&#0;\"/>",
    ] {
        let err = fails_xml(input)?;
        ensure(
            matches!(err.kind, XmlErrorKind::ForbiddenCharRef(_)),
            &format!("{input}: {err}"),
        )?;
    }
    for input in [
        "<a>&#xZZ;</a>",
        "<a>&#;</a>",
        "<a>&#x110000;</a>",
        "<a>&#+5;</a>",
    ] {
        let err = fails_xml(input)?;
        ensure(
            matches!(err.kind, XmlErrorKind::InvalidCharRef(_)),
            &format!("{input}: {err}"),
        )?;
    }
    // c14n refuses it too; SOAP drops it, as its old parser did
    // (`legacy_char_refs`).
    ensure(
        parse("<a>&#0;</a>", &ParsePolicy::c14n()).is_err(),
        "c14n accepted &#0;",
    )?;
    let tree = with("<a>&#0;v</a>", &ParsePolicy::soap())?;
    ensure(tree.text(root(&tree)?) == "v", "SOAP kept &#0;")
}

#[test]
fn literal_forbidden_characters_are_refused_by_the_strict_policy_only() -> TestResult {
    let err = fails_xml("<a>x\u{1}y</a>")?;
    assert_eq!(err.kind, XmlErrorKind::ForbiddenChar(1));
    fails_xml("<a b=\"\u{2}\"/>")?;
    let tree = with("<a>x\u{1}y</a>", &ParsePolicy::soap())?;
    assert_eq!(tree.text(root(&tree)?), "x\u{1}y");
    Ok(())
}

#[test]
fn adjacent_text_and_references_are_one_text_node() -> TestResult {
    let tree = xml("<a>x &amp; y &#65; z</a>")?;
    let a = root(&tree)?;
    assert_eq!(tree.children(a).len(), 1);
    assert_eq!(
        tree.kind(child(&tree, a, 0)?),
        &Kind::Text("x & y A z".into())
    );
    Ok(())
}

#[test]
fn cdata_is_kept_as_its_own_node() -> TestResult {
    let tree = xml("<a>x<![CDATA[<b>&amp;</b>]]>y</a>")?;
    let a = root(&tree)?;
    let kinds: Vec<&str> = tree
        .children(a)
        .iter()
        .map(|c| tree.kind(*c).name())
        .collect();
    assert_eq!(kinds, ["text", "cdata", "text"]);
    assert_eq!(tree.text(a), "x<b>&amp;</b>y");
    Ok(())
}

#[test]
fn prolog_and_epilogue_comments_and_pis_are_document_children() -> TestResult {
    let tree =
        xml("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!--a-->\n<?pi x?>\n<r/>\n<!--z-->\n")?;
    let kinds: Vec<&str> = tree
        .children(NodeId::DOCUMENT)
        .iter()
        .map(|c| tree.kind(*c).name())
        .collect();
    assert_eq!(kinds, ["comment", "pi", "element", "comment"]);
    let decl = tree.decl().ok_or("no declaration")?;
    assert_eq!(&*decl.version, "1.0");
    assert_eq!(decl.encoding.as_deref(), Some("UTF-8"));
    assert_eq!(decl.standalone, None);
    Ok(())
}

#[test]
fn pi_data_is_stripped_except_for_c14n() -> TestResult {
    let input = "<a><?p   d e ?></a>";
    for (policy, data) in [
        (ParsePolicy::xml(), "d e "),
        (ParsePolicy::soap(), "d e "),
        (ParsePolicy::c14n(), "   d e "),
    ] {
        let tree = with(input, &policy)?;
        let pi = child(&tree, root(&tree)?, 0)?;
        assert_eq!(tree.text(pi), data);
    }
    Ok(())
}

#[test]
fn whitespace_is_kept_unless_strip_blank() -> TestResult {
    let input = "<a>\n  <b> x </b>\n</a>";
    let tree = xml(input)?;
    assert_eq!(tree.children(root(&tree)?).len(), 3);
    let tree = with(input, &ParsePolicy::xml().strip_blank(true))?;
    let a = root(&tree)?;
    assert_eq!(tree.children(a).len(), 1);
    // Non-blank text is never trimmed.
    assert_eq!(tree.text(a), " x ");
    Ok(())
}

// ------------------------------------------------------------------- refusals

#[test]
fn doctype_is_refused_with_each_callers_message() -> TestResult {
    let laughs = "<?xml version=\"1.0\"?><!DOCTYPE lolz [<!ENTITY lol \"lol\"><!ENTITY lol2 \"&lol;&lol;\">]><lolz>&lol2;</lolz>";
    assert_eq!(
        fails_xml(laughs)?.message(),
        "DOCTYPE declarations are not allowed in XML documents"
    );
    let soap = fails(laughs, &ParsePolicy::soap())?;
    assert_eq!(
        soap_error_message(&soap),
        "XML parsing error: DOCTYPE declarations are not allowed in SOAP payloads"
    );
    assert_eq!(
        fails(laughs, &ParsePolicy::c14n())?.message(),
        "DOCTYPE declarations are not allowed (XXE / entity-expansion defense)"
    );
    Ok(())
}

#[test]
fn an_unknown_entity_is_refused_or_dropped_by_policy() -> TestResult {
    let err = fails_xml("<a>x&nbsp;y</a>")?;
    assert_eq!(err.kind, XmlErrorKind::UnknownEntity("nbsp".into()));
    fails("<a>x&nbsp;y</a>", &ParsePolicy::c14n())?;
    fails_xml("<a b=\"&foo;\"/>")?;
    let tree = with("<a>x&nbsp;y</a>", &ParsePolicy::soap())?;
    assert_eq!(tree.text(root(&tree)?), "xy");
    let tree = with("<a b=\"1&foo;2\"/>", &ParsePolicy::soap())?;
    assert_eq!(tree.attribute(root(&tree)?, "b"), Some("12"));
    Ok(())
}

#[test]
fn an_unbound_prefix_is_refused_or_allowed_by_policy() -> TestResult {
    let err = fails_xml("<p:a/>")?;
    assert_eq!(err.kind, XmlErrorKind::UnboundPrefix("p".into()));
    let err = fails_xml("<a q:b=\"1\"/>")?;
    assert_eq!(err.kind, XmlErrorKind::UnboundPrefix("q".into()));
    for policy in [ParsePolicy::soap(), ParsePolicy::c14n()] {
        let tree = with("<p:a q:b=\"1\"><p:c/></p:a>", &policy)?;
        let a = root(&tree)?;
        let el = tree.element(a).ok_or("not an element")?;
        assert_eq!(el.prefix, Some("p"));
        assert_eq!(el.ns_uri, None);
        assert_eq!(tree.attribute(a, "q:b"), Some("1"));
    }
    Ok(())
}

#[test]
fn a_duplicate_attribute_is_refused_by_expanded_name() -> TestResult {
    let err = fails_xml("<a x=\"1\" x=\"2\"/>")?;
    assert_eq!(err.kind, XmlErrorKind::DuplicateAttribute("x".into()));
    // Different prefixes, same namespace: the same expanded name.
    let err = fails_xml("<a xmlns:p=\"u\" xmlns:q=\"u\" p:x=\"1\" q:x=\"2\"/>")?;
    assert_eq!(err.kind, XmlErrorKind::DuplicateAttribute("q:x".into()));
    fails_xml("<a xmlns:p=\"u\" xmlns:p=\"v\"/>")?;
    // An unprefixed attribute is in no namespace, so it never clashes with a
    // prefixed one.
    xml("<a xmlns:p=\"u\" x=\"1\" p:x=\"2\"/>")?;
    // The large-element path (a hash set beyond 16 attributes).
    let mut many = String::from("<a");
    for i in 0..40 {
        many.push_str(&format!(" a{i}=\"{i}\""));
    }
    many.push_str(" a7=\"again\"/>");
    assert_eq!(
        fails_xml(&many)?.kind,
        XmlErrorKind::DuplicateAttribute("a7".into())
    );
    // SOAP never reads attributes and allows duplicates.
    with("<a x=\"1\" x=\"2\"/>", &ParsePolicy::soap())?;
    fails("<a x=\"1\" x=\"2\"/>", &ParsePolicy::c14n())?;
    Ok(())
}

#[test]
fn mismatched_and_unclosed_tags_are_refused_with_their_position() -> TestResult {
    let err = fails_xml("<a>\n  <b>\n  </a>")?;
    assert_eq!(
        err.kind,
        XmlErrorKind::MismatchedEndTag {
            expected: "b".into(),
            found: "a".into()
        }
    );
    assert_eq!(
        err.to_string(),
        "line 3, column 3: expected </b>, found </a>"
    );
    let err = fails_xml("<a>\n <b>text")?;
    assert_eq!(err.kind, XmlErrorKind::Unclosed("b".into()));
    assert_eq!(err.position, Some((2, 2)));
    let err = fails_xml("<a></a></b>")?;
    assert_eq!(err.kind, XmlErrorKind::UnmatchedEndTag("b".into()));
    // Truncated input is refused by every policy (SOAP bug 6).
    fails("<a><b>1</b>", &ParsePolicy::soap())?;
    let err = fails("<a><b>1</b>", &ParsePolicy::c14n())?;
    assert_eq!(err.message(), "no document element found");
    Ok(())
}

#[test]
fn a_document_needs_exactly_one_root_and_no_text_outside_it() -> TestResult {
    for input in ["", "   ", "<!--only a comment-->", "<?pi?>"] {
        let err = fails_xml(input)?;
        assert_eq!(err.kind, XmlErrorKind::NoRoot("no root element"));
    }
    assert_eq!(
        fails_xml("<a/><b/>")?.kind,
        XmlErrorKind::SecondRoot("b".into())
    );
    for input in [
        "junk<a/>",
        "<a/>junk",
        "<a/>&amp;",
        "&#32;<a/>",
        "<a/><![CDATA[x]]>",
    ] {
        let err = fails_xml(input)?;
        ensure(
            err.kind == XmlErrorKind::TextOutsideRoot,
            &format!("{input}: {err}"),
        )?;
    }
    // Whitespace around the root is fine, and not kept.
    let tree = xml("\n  <a/>\n\n")?;
    assert_eq!(tree.children(NodeId::DOCUMENT).len(), 1);
    Ok(())
}

#[test]
fn soap_and_c14n_ignore_trailing_content() -> TestResult {
    for (policy, expected) in [
        (ParsePolicy::soap(), &["element"][..]),
        (ParsePolicy::c14n(), &["element", "comment"][..]),
    ] {
        let tree = with("<a>1</a><b><c/></b>tail<!--kept-->", &policy)?;
        let kinds: Vec<&str> = tree
            .children(NodeId::DOCUMENT)
            .iter()
            .map(|c| tree.kind(*c).name())
            .collect();
        assert_eq!(kinds, expected);
        // A trailing root cut off by the end of the input is ignored too.
        with("<a/><b>", &policy)?;
    }
    // SOAP: an empty input or plain text has no root, and that is not an error.
    let tree = with("", &ParsePolicy::soap())?;
    assert_eq!(tree.root_element(), None);
    let tree = with("just text", &ParsePolicy::soap())?;
    assert_eq!(tree.root_element(), None);
    assert_eq!(
        fails("", &ParsePolicy::c14n())?.message(),
        "no document element found"
    );
    Ok(())
}

#[test]
fn strict_syntax_checks_names_comments_pis_and_the_declaration() -> TestResult {
    fails_xml("<1a/>")?;
    fails_xml("<a 1b=\"x\"/>")?;
    fails_xml("<a:b:c xmlns:a=\"u\"/>")?;
    assert_eq!(
        fails_xml("<a><!-- x -- y --></a>")?.kind,
        XmlErrorKind::InvalidComment
    );
    assert_eq!(
        fails_xml("<a><!-- x ---></a>")?.kind,
        XmlErrorKind::InvalidComment
    );
    fails_xml("<a><?xml-stylesheet ok?><?XML no?></a>")?;
    assert_eq!(
        fails_xml("<a/><?xml version=\"1.0\"?>")?.kind,
        XmlErrorKind::MisplacedDeclaration
    );
    assert_eq!(
        fails_xml(" <?xml version=\"1.0\"?><a/>")?.kind,
        XmlErrorKind::MisplacedDeclaration
    );
    assert_eq!(
        fails_xml("<a b=\"<\"/>")?.kind,
        XmlErrorKind::LtInAttributeValue
    );
    fails_xml("<a xmlns:p=\"\"/>")?;
    // The lenient policies accept what they accepted before.
    with("<a><!-- x -- y --></a>", &ParsePolicy::soap())?;
    with("<1a/>", &ParsePolicy::c14n())?;
    Ok(())
}

#[test]
fn a_dangling_ampersand_is_refused() -> TestResult {
    fails_xml("<a>x & y</a>")?;
    fails_xml("<a b=\"x & y\"/>")?;
    fails("<a>x & y</a>", &ParsePolicy::soap())?;
    Ok(())
}

// --------------------------------------------------------------------- limits

fn nested(depth: usize) -> String {
    let mut s = String::new();
    for _ in 0..depth {
        s.push_str("<n>");
    }
    for _ in 0..depth {
        s.push_str("</n>");
    }
    s
}

#[test]
fn depth_limit_at_n_and_n_plus_one() -> TestResult {
    let policy = ParsePolicy::xml().lowered(Some(5), None, None);
    with(&nested(5), &policy)?;
    let err = fails(&nested(6), &policy)?;
    assert_eq!(err.kind, XmlErrorKind::DepthExceeded(5));
    assert!(err
        .to_string()
        .contains("more than 5 nested elements (SOLI_MAX_XML_DEPTH)"));
    // A self-closing leaf counts as one level too.
    with("<n><n><n><n><n/></n></n></n></n>", &policy)?;
    fails("<n><n><n><n><n><n/></n></n></n></n></n>", &policy)?;
    // SOAP keeps today's 64.
    with(&nested(64), &ParsePolicy::soap())?;
    let err = fails(&nested(65), &ParsePolicy::soap())?;
    assert_eq!(
        soap_error_message(&err),
        "XML parsing error: nesting depth exceeded 64 levels (likely DoS payload)"
    );
    // A skipped trailing root is depth-limited too (SOAP stops at the end
    // of the first root and never reads it).
    let c14n = ParsePolicy::c14n().lowered(Some(3), None, None);
    with("<a/><n><n><n></n></n></n>", &c14n)?;
    fails("<a/><n><n><n><n></n></n></n></n>", &c14n)?;
    Ok(())
}

#[test]
fn node_limit_at_n_and_n_plus_one() -> TestResult {
    // <a> + 4 × (<b> + its text) = 9 nodes.
    let input = "<a><b>1</b><b>2</b><b>3</b><b>4</b></a>";
    let policy = ParsePolicy::xml().lowered(None, Some(9), None);
    assert_eq!(with(input, &policy)?.node_count(), 9);
    let err = fails(input, &ParsePolicy::xml().lowered(None, Some(8), None))?;
    assert_eq!(err.kind, XmlErrorKind::NodesExceeded(8));
    Ok(())
}

#[test]
fn byte_limit_at_n_and_n_plus_one() -> TestResult {
    let input = "<a>12345</a>"; // 12 bytes
    with(input, &ParsePolicy::xml().lowered(None, None, Some(12)))?;
    let err = fails(input, &ParsePolicy::xml().lowered(None, None, Some(11)))?;
    assert_eq!(err.kind, XmlErrorKind::BytesExceeded(Some(12), 11));
    // The BOM does not count.
    with(
        "\u{FEFF}<a>12345</a>",
        &ParsePolicy::xml().lowered(None, None, Some(12)),
    )?;
    Ok(())
}

#[test]
fn byte_limit_holds_for_a_streamed_reader_too() -> TestResult {
    let mut policy = ParsePolicy::xml();
    policy.max_bytes = 10;
    let mut sink = TreeSink::new(100);
    let input = "<a>0123456789</a>";
    match EventLoop::new(input.as_bytes(), policy).run(&mut sink) {
        Ok(()) => Err("a 17-byte stream passed a 10-byte cap".into()),
        Err(err) => {
            assert_eq!(err.kind, XmlErrorKind::BytesExceeded(None, 10));
            Ok(())
        }
    }
}

#[test]
fn attribute_limit_at_n_and_n_plus_one() -> TestResult {
    let element = |n: usize| {
        let mut s = String::from("<a xmlns:p=\"u\"");
        for i in 0..n {
            s.push_str(&format!(" x{i}=\"1\""));
        }
        s.push_str("/>");
        s
    };
    let mut policy = ParsePolicy::xml();
    policy.max_attrs = 3;
    // Namespace declarations do not count.
    with(&element(3), &policy)?;
    let err = fails(&element(4), &policy)?;
    assert_eq!(err.kind, XmlErrorKind::AttrsExceeded(3));
    Ok(())
}

#[test]
fn name_length_limit_at_n_and_n_plus_one() -> TestResult {
    let name = "a".repeat(MAX_NAME_LEN);
    xml(&format!("<{name}/>"))?;
    xml(&format!("<a {name}=\"1\"/>"))?;
    let long = "a".repeat(MAX_NAME_LEN + 1);
    assert_eq!(
        fails_xml(&format!("<{long}/>"))?.kind,
        XmlErrorKind::NameTooLong(MAX_NAME_LEN)
    );
    fails_xml(&format!("<a {long}=\"1\"/>"))?;
    fails(&format!("<{long}/>"), &ParsePolicy::soap())?;
    Ok(())
}

#[test]
fn text_run_limit_at_n_and_n_plus_one() -> TestResult {
    let mut policy = ParsePolicy::soap();
    policy.max_text_run = Some(10);
    with("<a>12345&amp;6789</a>", &policy)?; // joined run: 10 bytes
    let err = fails("<a>12345&amp;67890</a>", &policy)?;
    assert_eq!(err.kind, XmlErrorKind::TextRunExceeded(10));
    with("<a><![CDATA[0123456789]]></a>", &policy)?;
    fails("<a><![CDATA[0123456789x]]></a>", &policy)?;
    // SOAP's real cap keeps today's message.
    let big = format!("<a>{}</a>", "x".repeat(1024 * 1024 + 1));
    let err = fails(&big, &ParsePolicy::soap())?;
    assert_eq!(
        soap_error_message(&err),
        "XML parsing error: element text exceeded 1048576 bytes (likely DoS payload)"
    );
    with(
        &format!("<a>{}</a>", "x".repeat(1024 * 1024)),
        &ParsePolicy::soap(),
    )?;
    Ok(())
}

#[test]
fn namespace_declarations_per_element_are_capped_by_quick_xml() -> TestResult {
    let mut s = String::from("<a");
    for i in 0..257 {
        s.push_str(&format!(" xmlns:p{i}=\"u{i}\""));
    }
    s.push_str("/>");
    fails_xml(&s)?;
    Ok(())
}

// ----------------------------------------------------------------- namespaces

#[test]
fn namespaces_are_scoped_and_resolved() -> TestResult {
    let input = concat!(
        "<feed xmlns=\"urn:atom\" xmlns:x=\"urn:x\">",
        "<entry x:id=\"1\" plain=\"2\"><title>t</title></entry>",
        "<x:other xmlns=\"\"><inner/></x:other>",
        "<y xmlns:x=\"urn:x2\"><x:z xml:lang=\"fr\"/></y>",
        "</feed>"
    );
    let tree = xml(input)?;
    let feed = root(&tree)?;
    let el = tree.element(feed).ok_or("feed")?;
    assert_eq!(el.ns_uri.map(|u| &**u), Some("urn:atom"));
    assert_eq!(el.local_name, "feed");
    assert_eq!(el.prefix, None);

    let entry = child(&tree, feed, 0)?;
    assert_eq!(
        tree.element(entry).and_then(|e| e.ns_uri).map(|u| &**u),
        Some("urn:atom")
    );
    assert_eq!(tree.attribute_ns(entry, "id", Some("urn:x")), Some("1"));
    assert_eq!(tree.attribute_ns(entry, "plain", None), Some("2"));
    assert_eq!(tree.attribute_ns(entry, "plain", Some("urn:atom")), None);
    let title = child(&tree, entry, 0)?;
    assert_eq!(
        tree.element(title).and_then(|e| e.ns_uri).map(|u| &**u),
        Some("urn:atom")
    );

    let other = child(&tree, feed, 1)?;
    let el = tree.element(other).ok_or("other")?;
    assert_eq!(el.prefix, Some("x"));
    assert_eq!(el.local_name, "other");
    assert_eq!(el.ns_uri.map(|u| &**u), Some("urn:x"));
    let inner = child(&tree, other, 0)?;
    // `xmlns=""` undeclares the default namespace.
    assert_eq!(tree.element(inner).and_then(|e| e.ns_uri), None);
    assert_eq!(tree.lookup_namespace(inner, None), None);

    let y = child(&tree, feed, 2)?;
    let z = child(&tree, y, 0)?;
    assert_eq!(
        tree.element(z).and_then(|e| e.ns_uri).map(|u| &**u),
        Some("urn:x2")
    );
    assert_eq!(tree.attribute_ns(z, "lang", Some(XML_NS)), Some("fr"));
    let scope: Vec<(Option<String>, String)> = tree
        .in_scope_namespaces(z)
        .into_iter()
        .map(|d| (d.prefix.map(String::from), d.uri.to_string()))
        .collect();
    assert_eq!(
        scope,
        [
            (None, "urn:atom".to_string()),
            (Some("x".to_string()), "urn:x2".to_string())
        ]
    );
    // Declarations made on one element only.
    assert_eq!(tree.element(y).map(|e| e.ns_decls.len()), Some(1));
    Ok(())
}

#[test]
fn the_sink_sees_the_scope_including_the_elements_own_declarations() -> TestResult {
    struct Probe(Vec<String>);
    impl Sink for Probe {
        fn start_element(&mut self, tag: StartTag, scope: &NsScope) -> Result<(), XmlError> {
            let uris: Vec<String> = scope
                .in_scope()
                .iter()
                .map(|d| format!("{}={}", d.prefix.as_deref().unwrap_or(""), d.uri))
                .collect();
            self.0.push(format!("{}[{}]", tag.qname, uris.join(",")));
            Ok(())
        }
        fn end_element(&mut self) -> Result<(), XmlError> {
            Ok(())
        }
        fn text(&mut self, _: String) -> Result<(), XmlError> {
            Ok(())
        }
        fn cdata(&mut self, _: String) -> Result<(), XmlError> {
            Ok(())
        }
        fn comment(&mut self, _: String) -> Result<(), XmlError> {
            Ok(())
        }
        fn pi(&mut self, _: String, _: String) -> Result<(), XmlError> {
            Ok(())
        }
    }
    let mut probe = Probe(Vec::new());
    let input = "<a xmlns=\"u\"><p:b xmlns:p=\"v\"/><c/></a>";
    let mut events = EventLoop::new(input.as_bytes(), ParsePolicy::xml());
    // Step by step, as a pull reader would.
    let mut steps = 0;
    while events.step(&mut probe).map_err(|e| e.to_string())? == Step::Continue {
        steps += 1;
    }
    assert!(steps >= 4);
    assert_eq!(probe.0, ["a[=u]", "p:b[=u,p=v]", "c[=u]"]);
    Ok(())
}

// ------------------------------------------------------------- serialisation

#[test]
fn documents_in_canonical_form_round_trip_byte_for_byte() -> TestResult {
    for input in [
        "<a/>",
        "<a></a>",
        "<a b=\"1\" c=\"&lt;&amp;&gt;&quot;'\">x &amp; y &lt; z &gt; w</a>",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\"><channel><title>T</title></channel></rss>",
        "<?xml version=\"1.0\"?>\n<a/>",
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<!--c-->\n<?pi data?>\n<a/>\n<!--z-->",
        "<a>x<![CDATA[<raw>]]>y<!--c--><?p d?></a>",
        "<f xmlns=\"urn:a\" xmlns:x=\"urn:x\"><x:e x:id=\"1\"><t>v</t></x:e><g xmlns=\"\"/></f>",
        "<a t=\"&#9;&#10;&#13;\">&#13;</a>",
        "<a>\n  <b/>\n</a>",
    ] {
        let tree = xml(input)?;
        let written = compact(&tree, NodeId::DOCUMENT)?;
        ensure(written == input, &format!("{input:?} was written as {written:?}"))?;
        round_trips(input)?;
    }
    Ok(())
}

#[test]
fn parse_serialize_parse_gives_the_same_tree() -> TestResult {
    for input in [
        "\u{FEFF}<a>\r\n</a>",
        "<a x='single' y=\"a\r\nb\">&#65;&#x1F600;</a>",
        "<?xml version='1.0' encoding='utf-8'?><a/>",
        "<a><![CDATA[x]]]]><![CDATA[>y]]></a>",
        "<a><![CDATA[a]]>b<![CDATA[]]></a>",
        "<a  b = \"1\"  ></a >",
        "<?xml version=\"1.0\"?><!--c--><a><?p   spaced   data ?></a>",
        "<a xmlns:p=\"u&amp;v\" p:x=\"&quot;\"/>",
    ] {
        round_trips(input)?;
    }
    Ok(())
}

#[test]
fn the_self_closing_bit_is_kept_unless_expand_empty() -> TestResult {
    let tree = xml("<a><b/><c></c></a>")?;
    assert_eq!(compact(&tree, NodeId::DOCUMENT)?, "<a><b/><c></c></a>");
    let opts = SerializeOptions {
        expand_empty: true,
        ..SerializeOptions::compact()
    };
    let written = serialize(&tree, NodeId::DOCUMENT, &opts).map_err(|e| e.to_string())?;
    assert_eq!(written, "<a><b></b><c></c></a>");
    Ok(())
}

#[test]
fn the_declaration_option() -> TestResult {
    let tree = xml("<a><b/></a>")?;
    let a = root(&tree)?;
    let on = SerializeOptions {
        declaration: Some(true),
        ..SerializeOptions::compact()
    };
    let off = SerializeOptions {
        declaration: Some(false),
        ..SerializeOptions::compact()
    };
    let doc_on = serialize(&tree, NodeId::DOCUMENT, &on).map_err(|e| e.to_string())?;
    assert_eq!(
        doc_on,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a><b/></a>"
    );
    let node_on = serialize(&tree, a, &on).map_err(|e| e.to_string())?;
    assert_eq!(
        node_on,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<a><b/></a>"
    );
    // A node never writes one by default, even in a document that has one.
    let with_decl = xml("<?xml version=\"1.0\"?><a/>")?;
    assert_eq!(compact(&with_decl, root(&with_decl)?)?, "<a/>");
    let doc_off = serialize(&with_decl, NodeId::DOCUMENT, &off).map_err(|e| e.to_string())?;
    assert_eq!(doc_off, "<a/>");
    Ok(())
}

#[test]
fn indent_pretty_prints_element_only_content_and_leaves_mixed_content_alone() -> TestResult {
    let tree = xml("<a>\n<b><c>1</c><d/></b><e>mixed <f>x</f> text</e><!--n--></a>")?;
    let opts = SerializeOptions {
        indent: Some(2),
        ..SerializeOptions::compact()
    };
    let written = serialize(&tree, NodeId::DOCUMENT, &opts).map_err(|e| e.to_string())?;
    assert_eq!(
        written,
        "<a>\n  <b>\n    <c>1</c>\n    <d/>\n  </b>\n  <e>mixed <f>x</f> text</e>\n  <!--n-->\n</a>"
    );
    // Indenting a document it produced changes nothing more.
    let again = xml(&written)?;
    let twice = serialize(&again, NodeId::DOCUMENT, &opts).map_err(|e| e.to_string())?;
    assert_eq!(twice, written);
    Ok(())
}

#[test]
fn an_elements_to_xml_is_a_standalone_fragment() -> TestResult {
    let input = concat!(
        "<rsm:Doc xmlns:rsm=\"urn:rsm\" xmlns:ram=\"urn:ram\" xmlns=\"urn:d\" xmlns:unused=\"urn:u\">",
        "<ram:Party ram:id=\"7\"><Name>ACME</Name></ram:Party>",
        "</rsm:Doc>"
    );
    let tree = xml(input)?;
    let party = child(&tree, root(&tree)?, 0)?;
    let fragment = compact(&tree, party)?;
    assert_eq!(
        fragment,
        "<ram:Party xmlns:ram=\"urn:ram\" ram:id=\"7\"><Name xmlns=\"urn:d\">ACME</Name></ram:Party>"
    );
    // The fragment parses on its own to the same names and namespaces.
    let alone = xml(&fragment)?;
    let name = child(&alone, root(&alone)?, 0)?;
    assert_eq!(
        alone.element(name).and_then(|e| e.ns_uri).map(|u| &**u),
        Some("urn:d")
    );
    // inner_xml writes children in the element's scope: nothing re-declared.
    let inner = serialize_children(&tree, party, &SerializeOptions::compact())
        .map_err(|e| e.to_string())?;
    assert_eq!(inner, "<Name>ACME</Name>");
    Ok(())
}

#[test]
fn cdata_holding_the_terminator_is_split() -> TestResult {
    let (mut tree, a) = XmlTree::new_fragment(100, "a", &[]).map_err(|e| e.to_string())?;
    tree.add_cdata(a, "x]]>y]]>").map_err(|e| e.to_string())?;
    let written = compact(&tree, a)?;
    assert_eq!(
        written,
        "<a><![CDATA[x]]]]><![CDATA[>y]]]]><![CDATA[>]]></a>"
    );
    let back = xml(&written)?;
    assert_eq!(back.text(root(&back)?), "x]]>y]]>");
    Ok(())
}

#[test]
fn attribute_values_survive_tab_lf_cr_and_quotes() -> TestResult {
    let (mut tree, a) = XmlTree::new_fragment(100, "a", &[("v", "a\tb\nc\rd\"e'<&>")])
        .map_err(|e| e.to_string())?;
    tree.add_text(a, "t\r").map_err(|e| e.to_string())?;
    let written = compact(&tree, a)?;
    assert_eq!(
        written,
        "<a v=\"a&#9;b&#10;c&#13;d&quot;e'&lt;&amp;&gt;\">t&#13;</a>"
    );
    let back = xml(&written)?;
    let r = root(&back)?;
    assert_eq!(back.attribute(r, "v"), Some("a\tb\nc\rd\"e'<&>"));
    assert_eq!(back.text(r), "t\r");
    Ok(())
}

#[test]
fn output_is_capped() -> TestResult {
    let tree = xml("<a>0123456789</a>")?;
    let mut opts = SerializeOptions::compact();
    opts.max_bytes = 17;
    serialize(&tree, NodeId::DOCUMENT, &opts).map_err(|e| e.to_string())?;
    opts.max_bytes = 16;
    match serialize(&tree, NodeId::DOCUMENT, &opts) {
        Ok(s) => Err(format!("17 bytes passed a 16-byte cap: {s}")),
        Err(err) => {
            assert_eq!(err.kind, XmlErrorKind::OutputExceeded(16));
            Ok(())
        }
    }
}

#[test]
fn a_deep_built_tree_serialises_without_recursion() -> TestResult {
    let (mut tree, mut node) =
        XmlTree::new_fragment(200_000, "n", &[]).map_err(|e| e.to_string())?;
    let top = node;
    for _ in 0..100_000 {
        node = tree
            .add_element(node, "n", &[])
            .map_err(|e| e.to_string())?;
    }
    let written = compact(&tree, top)?;
    assert!(written.starts_with("<n><n>"));
    assert!(written.ends_with("</n></n>"));
    let (copy, id) = tree.dup(top).map_err(|e| e.to_string())?;
    assert!(copy.subtree_eq(id, &tree, top));
    assert_eq!(
        to_h(&tree, node),
        to_h(&copy, NodeId(copy.node_count() as u32))
    );
    let _ = to_h(&tree, top);
    Ok(())
}

// --------------------------------------------------------- building, mutation

fn s<T>(r: Result<T, XmlError>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

#[test]
fn building_a_feed_the_way_the_block_builder_will() -> TestResult {
    let (mut tree, rss) = s(XmlTree::new_document(
        1000,
        Some(("rss", &[("version", "2.0")])),
    ))?;
    let rss = rss.ok_or("no root")?;
    let channel = s(tree.add_element(rss, "channel", &[]))?;
    let title = s(tree.add_element(channel, "title", &[]))?;
    s(tree.add_text(title, "News & views"))?;
    for (id, post) in [("1", "First"), ("2", "Second <b>")] {
        let item = s(tree.add_element(channel, "item", &[("id", id)]))?;
        let t = s(tree.add_element(item, "title", &[]))?;
        s(tree.add_text(t, post))?;
        s(tree.add_cdata(item, "<p>body</p>"))?;
    }
    let written = compact(&tree, NodeId::DOCUMENT)?;
    assert_eq!(
        written,
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<rss version=\"2.0\"><channel><title>News &amp; views</title>",
            "<item id=\"1\"><title>First</title><![CDATA[<p>body</p>]]></item>",
            "<item id=\"2\"><title>Second &lt;b&gt;</title><![CDATA[<p>body</p>]]></item>",
            "</channel></rss>"
        )
    );
    let back = xml(&written)?;
    assert!(back.subtree_eq(NodeId::DOCUMENT, &tree, NodeId::DOCUMENT));
    Ok(())
}

#[test]
fn built_names_are_validated_and_prefixes_resolved() -> TestResult {
    let (mut tree, root_id) = s(XmlTree::new_document(
        1000,
        Some(("feed", &[("xmlns", "urn:atom"), ("xmlns:x", "urn:x")])),
    ))?;
    let feed = root_id.ok_or("no root")?;
    let entry = s(tree.add_element(feed, "entry", &[("x:id", "1")]))?;
    assert_eq!(
        tree.element(entry).and_then(|e| e.ns_uri).map(|u| &**u),
        Some("urn:atom")
    );
    assert_eq!(tree.attribute_ns(entry, "id", Some("urn:x")), Some("1"));
    let other = s(tree.add_element(feed, "x:other", &[]))?;
    assert_eq!(
        tree.element(other).and_then(|e| e.ns_uri).map(|u| &**u),
        Some("urn:x")
    );
    for bad in ["1a", "a b", "", "a:b:c", "<x>", "xmlns:p"] {
        ensure(
            tree.add_element(feed, bad, &[]).is_err(),
            &format!("{bad:?} accepted as a name"),
        )?;
    }
    assert_eq!(
        tree.add_element(feed, "q:a", &[]).map_err(|e| e.kind),
        Err(XmlErrorKind::UnboundPrefix("q".into()))
    );
    ensure(
        tree.add_element(feed, "a", &[("1x", "v")]).is_err(),
        "bad attribute name",
    )?;
    ensure(
        tree.add_element(feed, "a", &[("q:x", "v")]).is_err(),
        "unbound attribute prefix",
    )?;
    ensure(
        tree.add_element(feed, "a", &[("x", "1"), ("x", "2")])
            .is_err(),
        "duplicate attribute",
    )?;
    ensure(tree.add_comment(feed, "a--b").is_err(), "comment with --")?;
    ensure(
        tree.add_comment(feed, "ends-").is_err(),
        "comment ending with -",
    )?;
    ensure(tree.add_pi(feed, "xml", "").is_err(), "PI target xml")?;
    ensure(tree.add_pi(feed, "XmL", "").is_err(), "PI target XmL")?;
    ensure(tree.add_pi(feed, "p", "a?>b").is_err(), "PI data with ?>")?;
    s(tree.add_pi(feed, "xml-stylesheet", "href=\"a.css\""))?;
    Ok(())
}

#[test]
fn forbidden_characters_are_dropped_from_built_text_and_values() -> TestResult {
    let (mut tree, a) = s(XmlTree::new_fragment(100, "a", &[("v", "x\u{1}y\u{FFFF}")]))?;
    s(tree.add_text(a, "row\u{0}\u{1B}[1m data\u{FFFE}"))?;
    s(tree.set_attribute(a, "w", Some("p\u{8}q")))?;
    assert_eq!(tree.attribute(a, "v"), Some("xy"));
    assert_eq!(tree.attribute(a, "w"), Some("pq"));
    assert_eq!(tree.text(a), "row[1m data");
    xml(&compact(&tree, a)?)?;
    Ok(())
}

#[test]
fn a_document_has_one_root_and_no_text() -> TestResult {
    let (mut tree, _) = s(XmlTree::new_document(100, Some(("a", &[]))))?;
    ensure(
        tree.add_element(NodeId::DOCUMENT, "b", &[]).is_err(),
        "second root",
    )?;
    ensure(
        tree.add_text(NodeId::DOCUMENT, "x").is_err(),
        "text in a document",
    )?;
    s(tree.add_comment(NodeId::DOCUMENT, "fine"))?;
    let text = s(tree.create_text("t"))?;
    ensure(
        tree.add_element(text, "x", &[]).is_err(),
        "child under a text node",
    )?;
    ensure(
        tree.append_child(text, NodeId::DOCUMENT).is_err(),
        "document inserted",
    )?;
    Ok(())
}

#[test]
fn moving_inserting_and_the_cycle_refusal() -> TestResult {
    let tree0 = xml("<r><a><b/></a><c/><d/></r>")?;
    let mut tree = tree0.clone();
    let r = root(&tree)?;
    let a = child(&tree, r, 0)?;
    let b = child(&tree, a, 0)?;
    let c = child(&tree, r, 1)?;
    let d = child(&tree, r, 2)?;

    s(tree.append_child(r, a))?; // move to the end
    assert_eq!(compact(&tree, r)?, "<r><c/><d/><a><b/></a></r>");
    s(tree.insert_before(c, d))?;
    assert_eq!(compact(&tree, r)?, "<r><d/><c/><a><b/></a></r>");
    s(tree.insert_after(a, c))?;
    assert_eq!(compact(&tree, r)?, "<r><d/><a><b/></a><c/></r>");
    s(tree.prepend_child(r, c))?;
    assert_eq!(compact(&tree, r)?, "<r><c/><d/><a><b/></a></r>");

    assert_eq!(
        tree.append_child(b, a).map_err(|e| e.kind),
        Err(XmlErrorKind::Cycle)
    );
    assert_eq!(
        tree.append_child(a, a).map_err(|e| e.kind),
        Err(XmlErrorKind::Cycle)
    );
    assert_eq!(
        tree.append_child(b, r).map_err(|e| e.kind),
        Err(XmlErrorKind::Cycle)
    );
    assert_eq!(
        tree.insert_before(b, a).map_err(|e| e.kind),
        Err(XmlErrorKind::Cycle)
    );
    assert_eq!(
        tree.replace(b, a).map_err(|e| e.kind),
        Err(XmlErrorKind::Cycle)
    );
    // A refused edit changed nothing.
    assert_eq!(compact(&tree, r)?, "<r><c/><d/><a><b/></a></r>");

    s(tree.replace(d, b))?;
    assert_eq!(compact(&tree, r)?, "<r><c/><b/><a></a></r>");
    assert_eq!(tree.parent(d), None);
    // insert_before on a node without a parent is refused.
    ensure(
        tree.insert_before(d, c).is_err(),
        "insert_before on a detached node",
    )?;
    ensure(tree.order_dirty(), "edits mark the order dirty")?;
    tree.ensure_order();
    let orders: Vec<u32> = [r, c, b, a].iter().map(|n| tree.order(*n)).collect();
    ensure(
        orders.windows(2).all(|w| w[0] < w[1]),
        "document order after edits",
    )?;
    ensure(
        tree.order(d) > tree.order(a),
        "detached nodes come after the document",
    )?;
    ensure(tree.version() > tree0.version(), "edits bump the version")
}

#[test]
fn replacing_the_root_element() -> TestResult {
    let mut tree = xml("<!--c--><a/>")?;
    let a = root(&tree)?;
    let b = s(tree.create_element("b", &[], None))?;
    s(tree.replace(a, b))?;
    assert_eq!(compact(&tree, NodeId::DOCUMENT)?, "<!--c-->\n<b/>");
    let comment = child(&tree, NodeId::DOCUMENT, 0)?;
    let text = s(tree.create_text("x"))?;
    ensure(
        tree.replace(comment, text).is_err(),
        "text replacing a document child",
    )?;
    let c = s(tree.create_element("c", &[], None))?;
    ensure(
        tree.replace(comment, c).is_err(),
        "a second root by replace",
    )?;
    assert_eq!(compact(&tree, NodeId::DOCUMENT)?, "<!--c-->\n<b/>");
    Ok(())
}

#[test]
fn remove_leaves_a_usable_fragment_counted_as_tombstones() -> TestResult {
    let mut tree = xml("<r><a x=\"1\"><b>t</b></a></r>")?;
    let r = root(&tree)?;
    let a = child(&tree, r, 0)?;
    let before = tree.node_count();
    s(tree.detach(a))?;
    assert_eq!(compact(&tree, r)?, "<r></r>");
    assert_eq!(compact(&tree, a)?, "<a x=\"1\"><b>t</b></a>");
    assert_eq!(tree.parent(a), None);
    assert!(!tree.is_attached(a));
    assert_eq!(tree.fragment_root(child(&tree, a, 0)?), a);
    assert_eq!(tree.node_count(), before);
    assert_eq!(tree.path(child(&tree, a, 0)?), "a/b");
    // dup compacts: only the live document is copied.
    let (copy, doc) = s(tree.dup(NodeId::DOCUMENT))?;
    assert_eq!(doc, NodeId::DOCUMENT);
    assert_eq!(copy.node_count(), 1);
    assert_eq!(compact(&copy, NodeId::DOCUMENT)?, "<r></r>");
    Ok(())
}

#[test]
fn the_node_limit_counts_built_and_removed_nodes() -> TestResult {
    let (mut tree, a) = s(XmlTree::new_fragment(3, "a", &[]))?;
    let b = s(tree.add_element(a, "b", &[]))?;
    s(tree.detach(b))?;
    s(tree.add_text(a, "x"))?;
    assert_eq!(
        tree.add_text(a, "y").map_err(|e| e.kind),
        Err(XmlErrorKind::NodesExceeded(3))
    );
    // A refused set_text leaves the children alone.
    assert_eq!(
        tree.set_text(a, "z").map_err(|e| e.kind),
        Err(XmlErrorKind::NodesExceeded(3))
    );
    assert_eq!(tree.text(a), "x");
    Ok(())
}

#[test]
fn attributes_text_and_names_are_edited_in_place() -> TestResult {
    let mut tree = xml("<r xmlns:p=\"urn:p\"><a x=\"1\">old<b/></a></r>")?;
    let r = root(&tree)?;
    let a = child(&tree, r, 0)?;
    assert_eq!(
        s(tree.set_attribute(a, "x", Some("2")))?,
        Some("1".to_string())
    );
    assert_eq!(s(tree.set_attribute(a, "p:y", Some("3")))?, None);
    assert_eq!(tree.attribute_ns(a, "y", Some("urn:p")), Some("3"));
    assert_eq!(s(tree.remove_attribute(a, "x"))?, Some("2".to_string()));
    assert_eq!(s(tree.remove_attribute(a, "x"))?, None);
    assert_eq!(
        s(tree.set_attribute(a, "p:y", None))?,
        Some("3".to_string())
    );
    ensure(
        tree.set_attribute(a, "xmlns:q", Some("u")).is_err(),
        "xmlns via set",
    )?;
    ensure(
        tree.set_attribute(a, "q:z", Some("u")).is_err(),
        "unbound prefix via set",
    )?;
    ensure(
        tree.set_attribute(a, "1z", Some("u")).is_err(),
        "bad name via set",
    )?;

    let b = child(&tree, a, 1)?;
    s(tree.set_text(a, "new & improved"))?;
    assert_eq!(compact(&tree, a)?, "<a>new &amp; improved</a>");
    assert_eq!(tree.parent(b), None);
    s(tree.set_text(a, ""))?;
    assert_eq!(tree.children(a).len(), 0);

    s(tree.rename(a, "p:renamed"))?;
    assert_eq!(
        tree.element(a).and_then(|e| e.ns_uri).map(|u| &**u),
        Some("urn:p")
    );
    assert_eq!(tree.element(a).map(|e| e.local_name), Some("renamed"));
    ensure(
        tree.rename(a, "z:x").is_err(),
        "rename to an unbound prefix",
    )?;
    ensure(
        tree.rename(a, "bad name").is_err(),
        "rename to an invalid name",
    )?;
    assert_eq!(
        compact(&tree, r)?,
        "<r xmlns:p=\"urn:p\"><p:renamed></p:renamed></r>"
    );
    Ok(())
}

#[test]
fn a_moved_element_keeps_its_namespace_on_output() -> TestResult {
    let mut tree =
        xml("<r><x:a xmlns:x=\"urn:1\"><x:b x:k=\"v\"/></x:a><c xmlns:x=\"urn:2\"/></r>")?;
    let r = root(&tree)?;
    let a = child(&tree, r, 0)?;
    let b = child(&tree, a, 0)?;
    let c = child(&tree, r, 1)?;
    s(tree.append_child(c, b))?;
    assert_eq!(
        compact(&tree, c)?,
        "<c xmlns:x=\"urn:2\"><x:b xmlns:x=\"urn:1\" x:k=\"v\"/></c>"
    );
    // A prefix resolves where the element's output declares it: `x` on the
    // moved `x:b` still means urn:1, so an attribute set there gets urn:1.
    s(tree.set_attribute(b, "x:n", Some("w")))?;
    let written = compact(&tree, c)?;
    assert_eq!(
        written,
        "<c xmlns:x=\"urn:2\"><x:b xmlns:x=\"urn:1\" x:k=\"v\" x:n=\"w\"/></c>"
    );
    let back = xml(&written)?;
    let b2 = child(&back, root(&back)?, 0)?;
    assert_eq!(back.attribute_ns(b2, "n", Some("urn:1")), Some("w"));
    assert_eq!(back.attribute_ns(b2, "k", Some("urn:1")), Some("v"));
    // A default-namespace element moved under no namespace re-declares it.
    let mut tree = xml("<r><a xmlns=\"urn:d\"><b/></a></r>")?;
    let r = root(&tree)?;
    let a = child(&tree, r, 0)?;
    let b = child(&tree, a, 0)?;
    s(tree.append_child(r, b))?;
    assert_eq!(
        compact(&tree, r)?,
        "<r><a xmlns=\"urn:d\"></a><b xmlns=\"urn:d\"/></r>"
    );
    // And a no-namespace element moved under a default namespace undeclares it.
    let mut tree = xml("<r><a xmlns=\"urn:d\"/><b/></r>")?;
    let r = root(&tree)?;
    let a = child(&tree, r, 0)?;
    let b = child(&tree, r, 1)?;
    s(tree.append_child(a, b))?;
    assert_eq!(
        compact(&tree, r)?,
        "<r><a xmlns=\"urn:d\"><b xmlns=\"\"/></a></r>"
    );
    Ok(())
}

#[test]
fn import_copies_from_another_tree_and_dup_compacts() -> TestResult {
    let source = xml("<s xmlns:p=\"urn:p\"><p:item n=\"1\">x</p:item></s>")?;
    let item = child(&source, root(&source)?, 0)?;
    let mut target = xml("<t/>")?;
    let t = root(&target)?;
    let copy = s(target.import(&source, item))?;
    s(target.append_child(t, copy))?;
    assert_eq!(
        compact(&target, t)?,
        "<t><p:item xmlns:p=\"urn:p\" n=\"1\">x</p:item></t>"
    );
    ensure(
        target.import(&source, NodeId::DOCUMENT).is_err(),
        "import a document",
    )?;
    // The source is untouched.
    assert_eq!(source.children(root(&source)?).len(), 1);

    let (dup, id) = s(source.dup(item))?;
    assert_eq!(dup.node_count(), 2);
    assert_eq!(dup.parent(id), None);
    assert!(dup.subtree_eq(id, &source, item));
    assert_eq!(
        compact(&dup, id)?,
        "<p:item xmlns:p=\"urn:p\" n=\"1\">x</p:item>"
    );
    Ok(())
}

// ------------------------------------------------------------------ navigation

#[test]
fn navigation_text_and_paths() -> TestResult {
    let tree = xml("<rss><channel><item>a<b>1</b></item><!--c--><item>b<![CDATA[2]]></item><x/></channel></rss>")?;
    let rss = root(&tree)?;
    let channel = child(&tree, rss, 0)?;
    let items: Vec<NodeId> = tree.element_children(channel).collect();
    assert_eq!(items.len(), 3);
    assert_eq!(tree.children(channel).len(), 4);
    assert_eq!(tree.path(items[1]), "/rss/channel/item[2]");
    assert_eq!(tree.path(items[2]), "/rss/channel/x");
    assert_eq!(
        tree.path(child(&tree, items[0], 0)?),
        "/rss/channel/item[1]/text()"
    );
    assert_eq!(tree.path(NodeId::DOCUMENT), "/");
    assert_eq!(tree.text(items[0]), "a1");
    assert_eq!(tree.own_text(items[0]), "a");
    assert_eq!(tree.text(items[1]), "b2");
    assert_eq!(tree.next_sibling(items[0]), Some(items[1]));
    assert_eq!(
        tree.next_node(items[0]).map(|n| tree.kind(n).name()),
        Some("comment")
    );
    assert_eq!(tree.previous_sibling(items[1]), Some(items[0]));
    assert_eq!(tree.previous_sibling(items[0]), None);
    assert_eq!(tree.previous_node(items[0]), None);
    assert_eq!(tree.ancestors(items[0]), [channel, rss]);
    assert_eq!(tree.parent_element(rss), None);
    assert_eq!(tree.parent(rss), Some(NodeId::DOCUMENT));
    assert_eq!(tree.descendants(rss).len(), 5);
    assert_eq!(tree.name(items[0]), Some("item"));
    assert!(!tree.order_dirty());
    let orders: Vec<u32> = tree
        .descendant_nodes(NodeId::DOCUMENT)
        .iter()
        .map(|n| tree.order(*n))
        .collect();
    ensure(
        orders.windows(2).all(|w| w[0] < w[1]),
        "parsed trees are in document order",
    )
}

// ----------------------------------------------------------------- projections

fn map(pairs: Vec<(&str, Proj)>) -> Proj {
    Proj::Map(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn st(s: &str) -> Proj {
    Proj::Str(s.to_string())
}

#[test]
fn to_h_matches_the_designs_example() -> TestResult {
    let tree = xml("<item id=\"1\"><title>A &amp; B</title><tag>x</tag><tag>y</tag></item>")?;
    let expected = map(vec![(
        "item",
        map(vec![
            ("@id", st("1")),
            ("title", st("A & B")),
            ("tag", Proj::List(vec![st("x"), st("y")])),
        ]),
    )]);
    assert_eq!(to_h(&tree, NodeId::DOCUMENT), expected);
    assert_eq!(to_h(&tree, root(&tree)?), expected);
    assert_eq!(
        expected.to_json_string(),
        "{\"item\":{\"@id\":\"1\",\"title\":\"A & B\",\"tag\":[\"x\",\"y\"]}}"
    );
    Ok(())
}

#[test]
fn to_h_leaves_mixed_text_and_namespaces() -> TestResult {
    let tree = xml("<a xmlns:p=\"u\"><e/><b>  </b><c k=\"v\"/><p:d> x <i/> y </p:d></a>")?;
    let expected = map(vec![(
        "a",
        map(vec![
            ("e", Proj::Null),
            ("b", Proj::Null),
            ("c", map(vec![("@k", st("v"))])),
            ("p:d", map(vec![("i", Proj::Null), ("_text", st("x  y"))])),
        ]),
    )]);
    assert_eq!(to_h(&tree, NodeId::DOCUMENT), expected);
    let e = child(&tree, root(&tree)?, 0)?;
    assert_eq!(to_h(&tree, e), map(vec![("e", Proj::Null)]));
    // A non-element gives nil.
    let tree = xml("<a>t</a>")?;
    assert_eq!(to_h(&tree, child(&tree, root(&tree)?, 0)?), Proj::Null);
    Ok(())
}

fn soap(input: &str) -> Result<Proj, String> {
    Ok(soap_shape(&with(input, &ParsePolicy::soap())?))
}

#[test]
fn soap_shape_keeps_todays_rules() -> TestResult {
    let envelope = concat!(
        "<?xml version=\"1.0\"?>",
        "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">",
        "<soap:Body><m:GetPriceResponse xmlns:m=\"urn:m\" attr=\"dropped\">",
        "<m:Price>1.90</m:Price><m:Tag>a</m:Tag><m:Tag>b</m:Tag><m:Empty></m:Empty>",
        "</m:GetPriceResponse></soap:Body></soap:Envelope>"
    );
    let expected = map(vec![(
        "soap:Envelope",
        map(vec![(
            "soap:Body",
            map(vec![(
                "m:GetPriceResponse",
                map(vec![
                    ("m:Price", st("1.90")),
                    ("m:Tag", Proj::List(vec![st("a"), st("b")])),
                    ("m:Empty", Proj::Null),
                ]),
            )]),
        )]),
    )]);
    assert_eq!(soap(envelope)?, expected);
    // Empty input and plain text are nil; comments and PIs are skipped;
    // trailing roots ignored; unknown entities dropped.
    assert_eq!(soap("")?, Proj::Null);
    assert_eq!(soap("hello")?, Proj::Null);
    assert_eq!(
        soap("<a><!--c--><?p d?><b>1</b></a><z/>")?,
        map(vec![("a", map(vec![("b", st("1"))]))])
    );
    assert_eq!(soap("<a>x&nbsp;y</a>")?, map(vec![("a", st("xy"))]));
    // Text beside children keeps the old key position: before a following
    // non-self-closing child, otherwise last.
    assert_eq!(
        soap("<a>x<b>1</b></a>")?,
        map(vec![("a", map(vec![("_text", st("x")), ("b", st("1"))]))])
    );
    assert_eq!(
        soap("<a><b>1</b>x</a>")?,
        map(vec![("a", map(vec![("b", st("1")), ("_text", st("x"))]))])
    );
    assert_eq!(
        soap("<a>x<b/></a>")?,
        map(vec![(
            "a",
            map(vec![("b", Proj::Null), ("_text", st("x"))])
        )])
    );
    // A lone child named `_text` stays ambiguous, as it was.
    assert_eq!(soap("<a><_text>v</_text></a>")?, map(vec![("a", st("v"))]));
    Ok(())
}

#[test]
fn soap_shape_fixes_the_six_bugs_and_normalises_crlf() -> TestResult {
    // Bug 1: whitespace around entities is kept.
    assert_eq!(
        soap("<a>Tom &amp; Jerry</a>")?,
        map(vec![("a", st("Tom & Jerry"))])
    );
    // Bug 2: CDATA is kept.
    assert_eq!(soap("<a><![CDATA[<x>]]></a>")?, map(vec![("a", st("<x>"))]));
    // Bug 3: a self-closing repeat is appended to the list.
    assert_eq!(
        soap("<a><b>1</b><b/><b>2</b></a>")?,
        map(vec![(
            "a",
            map(vec![("b", Proj::List(vec![st("1"), Proj::Null, st("2")]))])
        )])
    );
    // Bug 4: a self-closing root.
    assert_eq!(soap("<a/>")?, map(vec![("a", Proj::Null)]));
    // Bug 5: mixed-content runs are joined, then trimmed.
    assert_eq!(
        soap("<a> one <b>1</b> two <c/> three </a>")?,
        map(vec![(
            "a",
            map(vec![
                ("_text", st("one  two  three")),
                ("b", st("1")),
                ("c", Proj::Null)
            ])
        )])
    );
    // Bug 6: truncated input raises.
    let err = fails("<a><b>1</b>", &ParsePolicy::soap())?;
    assert!(soap_error_message(&err).starts_with("XML parsing error: "));
    // CRLF is normalised.
    assert_eq!(soap("<a>1\r\n2</a>")?, map(vec![("a", st("1\n2"))]));
    Ok(())
}

#[test]
fn soap_error_messages_keep_quick_xmls_wording() -> TestResult {
    let err = fails("<root><unclosed></root>", &ParsePolicy::soap())?;
    let message = soap_error_message(&err);
    ensure(
        message.contains("expected `</unclosed>`, but `</root>` was found"),
        &message,
    )?;
    ensure(message.starts_with("XML parsing error: "), &message)
}

#[test]
fn proj_json_escapes_strings_and_keeps_order() -> TestResult {
    let value = map(vec![
        ("z", st("q\"uote\n")),
        ("a", Proj::List(vec![Proj::Null, map(vec![])])),
    ]);
    assert_eq!(
        value.to_json_string(),
        "{\"z\":\"q\\\"uote\\n\",\"a\":[null,{}]}"
    );
    Ok(())
}

// ------------------------------------------------------------- review fixes

/// `levels` nested elements each declaring `per_level` prefixes, then
/// `leaves` leaves named and attributed with prefixes from the outermost
/// level (the worst case for a backwards scan of every binding).
fn hostile_scope(levels: usize, per_level: usize, leaves: usize, attrs: usize) -> String {
    let mut s = String::new();
    for l in 0..levels {
        s.push_str("<l");
        for d in 0..per_level {
            s.push_str(&format!(" xmlns:p{l}_{d}=\"u{l}_{d}\""));
        }
        s.push('>');
    }
    for _ in 0..leaves {
        s.push_str("<p0_0:leaf");
        for k in 0..attrs {
            s.push_str(&format!(" p0_{k}:a=\"1\""));
        }
        s.push_str("/>");
    }
    for _ in 0..levels {
        s.push_str("</l>");
    }
    s
}

/// A sink that asks for the in-scope namespaces at the first `budget`
/// leaves, the way the streaming lane copies them onto each match.
struct ScopeProbe {
    inner: TreeSink,
    budget: usize,
    seen: Vec<usize>,
}

impl Sink for ScopeProbe {
    fn start_element(&mut self, tag: StartTag, scope: &NsScope) -> Result<(), XmlError> {
        if tag.self_closing && self.budget > 0 {
            self.budget -= 1;
            self.seen.push(scope.in_scope().len());
        }
        self.inner.start_element(tag, scope)
    }
    fn end_element(&mut self) -> Result<(), XmlError> {
        self.inner.end_element()
    }
    fn text(&mut self, text: String) -> Result<(), XmlError> {
        self.inner.text(text)
    }
    fn cdata(&mut self, text: String) -> Result<(), XmlError> {
        self.inner.cdata(text)
    }
    fn comment(&mut self, text: String) -> Result<(), XmlError> {
        self.inner.comment(text)
    }
    fn pi(&mut self, target: String, data: String) -> Result<(), XmlError> {
        self.inner.pi(target, data)
    }
}

#[test]
fn namespace_lookups_stay_fast_on_a_hostile_scope() -> TestResult {
    // 64 × 250 = 16,000 live bindings and 4,000 leaves of 8 prefixed
    // attributes. With a scan per lookup (and a find per binding in
    // `in_scope`) this took minutes; with one stack per prefix it is
    // milliseconds. The bound is loose enough for an unoptimised build.
    let (levels, per_level, leaves, attrs) = (64, 250, 4000, 8);
    let input = hostile_scope(levels, per_level, leaves, attrs);
    let mut policy = ParsePolicy::xml();
    policy.max_ns_bindings = usize::MAX;
    let started = std::time::Instant::now();

    let mut probe = ScopeProbe {
        inner: TreeSink::new(policy.max_nodes),
        budget: 200,
        seen: Vec::new(),
    };
    EventLoop::new(input.as_bytes(), policy)
        .run(&mut probe)
        .map_err(|e| e.to_string())?;
    assert_eq!(probe.seen, vec![levels * per_level; 200]);
    let tree = probe.inner.into_tree();

    let mut leaf = root(&tree)?;
    for _ in 1..levels {
        leaf = child(&tree, leaf, 0)?;
    }
    let first_leaf = child(&tree, leaf, 0)?;
    assert_eq!(
        tree.element(first_leaf)
            .and_then(|e| e.ns_uri.map(|u| u.to_string())),
        Some("u0_0".to_string())
    );
    assert_eq!(tree.attribute_ns(first_leaf, "a", Some("u0_7")), Some("1"));
    for id in tree.children(leaf).iter().take(200) {
        assert_eq!(tree.in_scope_namespaces(*id).len(), levels * per_level);
    }
    let written = compact(&tree, NodeId::DOCUMENT)?;
    assert_eq!(written.len(), input.len());
    let inner =
        serialize_children(&tree, leaf, &SerializeOptions::compact()).map_err(|e| e.to_string())?;
    assert!(inner.starts_with("<p0_0:leaf p0_0:a=\"1\""));

    let elapsed = started.elapsed();
    ensure(
        elapsed < std::time::Duration::from_secs(5),
        &format!("hostile namespace scope took {elapsed:?}"),
    )
}

#[test]
fn live_namespace_bindings_are_capped() -> TestResult {
    // The default cap refuses the hostile shape outright.
    let err = fails_xml(&hostile_scope(20, 250, 1, 1))?;
    assert_eq!(err.kind, XmlErrorKind::NamespacesExceeded(MAX_NS_BINDINGS));
    assert!(err.to_string().contains("namespace declarations in scope"));
    // N and N + 1, counting shadowed bindings; siblings do not add up.
    let mut policy = ParsePolicy::xml();
    policy.max_ns_bindings = 3;
    with(
        "<a xmlns:p=\"1\" xmlns:q=\"2\"><b xmlns:p=\"3\"/><c xmlns:r=\"3\"/></a>",
        &policy,
    )?;
    let err = fails(
        "<a xmlns:p=\"1\" xmlns:q=\"2\"><b xmlns:r=\"3\"><c xmlns:s=\"4\"/></b></a>",
        &policy,
    )?;
    assert_eq!(err.kind, XmlErrorKind::NamespacesExceeded(3));
    // Every caller has the cap.
    fails(&hostile_scope(20, 250, 1, 1), &ParsePolicy::soap())?;
    fails(&hostile_scope(20, 250, 1, 1), &ParsePolicy::c14n())?;
    Ok(())
}

/// Every element of a reparsed output, in document order, by expanded name,
/// with its attributes by expanded name.
type Expanded = Vec<(
    Option<String>,
    String,
    Vec<(Option<String>, String, String)>,
)>;

fn expanded(tree: &XmlTree, id: NodeId) -> Expanded {
    let mut ids = vec![id];
    ids.extend(tree.descendants(id));
    ids.into_iter()
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

fn attr(qname: &str, ns: &str) -> Attr {
    Attr {
        qname: qname.into(),
        ns_uri: Some(std::sync::Arc::from(ns)),
        value: "1".into(),
    }
}

fn element(qname: &str, ns: Option<&str>, attrs: Vec<Attr>, decls: Vec<(&str, &str)>) -> Kind {
    Kind::Element {
        qname: qname.into(),
        local_start: qname.find(':').map_or(0, |c| (c + 1) as u16),
        ns_uri: ns.map(std::sync::Arc::from),
        attrs,
        ns_decls: decls
            .into_iter()
            .map(|(p, u)| NsDecl {
                prefix: Some(p.into()),
                uri: std::sync::Arc::from(u),
            })
            .collect(),
        self_closing: true,
    }
}

#[test]
fn the_serializer_never_rebinds_a_settled_prefix() -> TestResult {
    // An element whose prefix is inherited (p = urn:A from its own name,
    // urn:B above) and an attribute on it using p for urn:B: the attribute
    // gets a fresh prefix; the element keeps urn:A.
    let mut tree = xml("<r xmlns:p=\"urn:A\"><c xmlns:p=\"urn:B\"/></r>")?;
    let c = child(&tree, root(&tree)?, 0)?;
    let e = s(tree.push_child_unchecked(
        c,
        element("p:e", Some("urn:A"), vec![attr("p:x", "urn:B")], vec![]),
    ))?;
    let written = compact(&tree, e)?;
    let back = xml(&written)?;
    assert_eq!(
        expanded(&back, root(&back)?),
        expanded(&tree, e),
        "{written}"
    );
    // The same element under `c` (where p already means urn:B).
    let written = compact(&tree, c)?;
    let back = xml(&written)?;
    assert_eq!(
        expanded(&back, root(&back)?),
        expanded(&tree, c),
        "{written}"
    );
    // Under `r`, where p is inherited as urn:A and needs no declaration:
    // the attribute must not declare `xmlns:p="urn:B"` on the element.
    let r = root(&tree)?;
    s(tree.push_child_unchecked(
        r,
        element("p:e", Some("urn:A"), vec![attr("p:x", "urn:B")], vec![]),
    ))?;
    let written = compact(&tree, r)?;
    assert!(
        written.contains("<p:e xmlns:ns1=\"urn:B\" ns1:x=\"1\"/>"),
        "{written}"
    );
    let back = xml(&written)?;
    assert_eq!(
        expanded(&back, root(&back)?),
        expanded(&tree, r),
        "{written}"
    );

    // A fresh `ns1` is settled too: a later attribute spelled `ns1:` with
    // another URI gets `ns2`, never a second `xmlns:ns1`.
    let mut tree = xml("<r/>")?;
    let r = root(&tree)?;
    let e = s(tree.push_child_unchecked(
        r,
        element(
            "e",
            None,
            vec![attr("p:x", "urn:A"), attr("ns1:y", "urn:C")],
            vec![("p", "urn:B")],
        ),
    ))?;
    let written = compact(&tree, e)?;
    assert_eq!(
        written,
        "<e xmlns:p=\"urn:B\" xmlns:ns1=\"urn:A\" xmlns:ns2=\"urn:C\" ns1:x=\"1\" ns2:y=\"1\"/>"
    );
    let back = xml(&written)?;
    assert_eq!(expanded(&back, root(&back)?), expanded(&tree, e));

    // Two attributes sharing a prefix for different URIs: the first one
    // (inherited binding) is not rebound by the second.
    let mut tree = xml("<r xmlns:p=\"urn:A\"/>")?;
    let r = root(&tree)?;
    s(tree.push_child_unchecked(
        r,
        element(
            "e",
            None,
            vec![attr("p:x", "urn:A"), attr("p:y", "urn:B")],
            vec![],
        ),
    ))?;
    let written = compact(&tree, r)?;
    let back = xml(&written)?;
    assert_eq!(
        expanded(&back, root(&back)?),
        expanded(&tree, r),
        "{written}"
    );

    // The reviewer's sequence through the public API: the element stays in
    // urn:A after the move and the attribute follows the element's prefix.
    let mut tree = xml("<r xmlns:p=\"urn:A\"><c xmlns:p=\"urn:B\"/></r>")?;
    let r = root(&tree)?;
    let c = child(&tree, r, 0)?;
    let e = s(tree.add_element(r, "p:e", &[]))?;
    s(tree.append_child(c, e))?;
    s(tree.set_attribute(e, "p:x", Some("1")))?;
    s(tree.append_child(r, e))?;
    let written = compact(&tree, r)?;
    let back = xml(&written)?;
    assert_eq!(
        expanded(&back, root(&back)?),
        expanded(&tree, r),
        "{written}"
    );
    let e2 = child(&back, root(&back)?, 1)?;
    assert_eq!(
        back.element(e2)
            .and_then(|el| el.ns_uri.map(|u| u.to_string())),
        Some("urn:A".to_string())
    );
    Ok(())
}

#[test]
fn dup_import_and_detach_keep_the_prefixes_their_output_declares() -> TestResult {
    let source = xml("<r xmlns:p=\"urn:A\" xmlns=\"urn:D\"><p:a q=\"1\"><b/></p:a></r>")?;
    let a = child(&source, root(&source)?, 0)?;
    let check = |tree: &mut XmlTree, a: NodeId| -> TestResult {
        let shown = compact(tree, a)?;
        assert!(shown.starts_with("<p:a xmlns:p=\"urn:A\""), "{shown}");
        let added = s(tree.add_element(a, "p:b", &[]))?;
        assert_eq!(
            tree.lookup_namespace(added, Some("p")).as_deref(),
            Some("urn:A")
        );
        s(tree.set_attribute(a, "p:x", Some("v")))?;
        assert_eq!(tree.attribute_ns(a, "x", Some("urn:A")), Some("v"));
        s(tree.rename(added, "p:c"))?;
        let unprefixed = s(tree.add_element(a, "d", &[]))?;
        // `b` was in urn:D: the default namespace stays what `to_xml` shows.
        let b = child(tree, a, 0)?;
        assert_eq!(tree.lookup_namespace(b, None).as_deref(), Some("urn:D"));
        assert_eq!(tree.lookup_namespace(unprefixed, None), None);
        let names: Vec<String> = tree
            .in_scope_namespaces(a)
            .into_iter()
            .map(|d| format!("{:?}={}", d.prefix, d.uri))
            .collect();
        assert_eq!(names, vec!["Some(\"p\")=urn:A".to_string()]);
        // Unused prefixes stay unbound: only what the output declares.
        ensure(
            tree.add_element(a, "z:q", &[]).is_err(),
            "an unbound prefix on a fragment",
        )?;
        let written = compact(tree, a)?;
        let back = xml(&written)?;
        assert_eq!(
            expanded(&back, root(&back)?),
            expanded(tree, a),
            "{written}"
        );
        Ok(())
    };

    let (mut copy, id) = s(source.dup(a))?;
    check(&mut copy, id)?;

    let mut target = xml("<t/>")?;
    let imported = s(target.import(&source, a))?;
    check(&mut target, imported)?;

    let mut detached = source.clone();
    s(detached.detach(a))?;
    check(&mut detached, a)?;
    Ok(())
}

#[test]
fn soap_skips_bad_attributes_instead_of_refusing() -> TestResult {
    for input in ["<a x=\"AT&T\">v</a>", "<a b>v</a>", "<a x=\"&#0;\">v</a>"] {
        assert_eq!(soap(input)?, map(vec![("a", st("v"))]), "{input}");
        fails_xml(input)?;
        fails(input, &ParsePolicy::c14n())?;
    }
    // The attributes that do decode are kept, declarations included.
    let tree = with(
        "<p:a xmlns:p=\"urn:p\" bad=\"&\" q=\"1\" z=\"&#1;\">v</p:a>",
        &ParsePolicy::soap(),
    )?;
    let a = root(&tree)?;
    assert_eq!(tree.attribute(a, "q"), Some("1"));
    assert_eq!(tree.attribute(a, "bad"), None);
    assert_eq!(
        tree.element(a)
            .and_then(|e| e.ns_uri.map(|u| u.to_string())),
        Some("urn:p".to_string())
    );
    // A declaration that does not decode is skipped: its prefix is unbound,
    // which SOAP allows.
    with("<a xmlns:p=\"&#0;\"><p:b>v</p:b></a>", &ParsePolicy::soap())?;
    // Attributes past the cap are dropped, not refused.
    let mut policy = ParsePolicy::soap();
    policy.max_attrs = 2;
    let tree = with("<a x=\"1\" y=\"2\" z=\"3\">v</a>", &policy)?;
    assert_eq!(tree.element(root(&tree)?).map(|e| e.attrs.len()), Some(2));
    Ok(())
}

#[test]
fn a_policy_deeper_than_the_ceiling_is_clamped() -> TestResult {
    let mut policy = ParsePolicy::xml();
    policy.max_depth = usize::MAX;
    with(&nested(MAX_DEPTH_CEILING), &policy)?;
    let err = fails(&nested(MAX_DEPTH_CEILING + 1), &policy)?;
    assert_eq!(err.kind, XmlErrorKind::DepthExceeded(MAX_DEPTH_CEILING));
    Ok(())
}

#[test]
fn soaps_text_cap_counts_an_elements_text_across_comments_and_children() -> TestResult {
    let mut policy = ParsePolicy::soap();
    policy.max_text_run = Some(10);
    with("<a>12345<!--c-->67890</a>", &policy)?;
    fails("<a>12345<!--c-->678901</a>", &policy)?;
    with("<a>12345<b>xxxxxxxxxx</b>67890</a>", &policy)?;
    let err = fails("<a>12345<b/>6789<![CDATA[01]]></a>", &policy)?;
    assert_eq!(err.kind, XmlErrorKind::TextRunExceeded(10));
    // So the projected `_text` can never pass the cap.
    let tree = with("<a> 1234<b/>5678 </a>", &policy)?;
    assert_eq!(
        soap_shape(&tree),
        map(vec![(
            "a",
            map(vec![("b", Proj::Null), ("_text", st("12345678"))])
        )])
    );
    Ok(())
}

#[test]
fn built_cdata_comments_and_pis_round_trip_their_line_endings() -> TestResult {
    let (mut tree, r) = s(XmlTree::new_document(100, Some(("r", &[]))))?;
    let r = r.ok_or("no root")?;
    let cdata = s(tree.add_cdata(r, "a\r\nb\rc"))?;
    let comment = s(tree.add_comment(r, "x\r\ny"))?;
    let pi = s(tree.add_pi(r, "t", "1\r2"))?;
    assert_eq!(tree.text(cdata), "a\nb\nc");
    assert_eq!(tree.text(comment), "x\ny");
    assert_eq!(tree.text(pi), "1\n2");
    s(tree.set_text(cdata, "\r"))?;
    assert_eq!(tree.text(cdata), "\n");
    let written = compact(&tree, NodeId::DOCUMENT)?;
    let back = xml(&written)?;
    ensure(
        tree.subtree_eq(NodeId::DOCUMENT, &back, NodeId::DOCUMENT),
        &format!("built tree changed on the round trip: {written:?}"),
    )
}

#[test]
fn a_lenient_comment_with_double_hyphens_is_written_well_formed() -> TestResult {
    for policy in [ParsePolicy::soap(), ParsePolicy::c14n()] {
        let tree = with("<a><!--x--y---></a>", &policy)?;
        let written = compact(&tree, NodeId::DOCUMENT)?;
        // The comment body is `x--y-` (it ends at the first `-->`).
        assert_eq!(written, "<a><!--x- -y- --></a>");
        xml(&written)?;
    }
    Ok(())
}

#[test]
fn to_h_keeps_a_child_named_text_beside_the_text() -> TestResult {
    let tree = xml("<a><_text>c</_text>t</a>")?;
    assert_eq!(
        to_h(&tree, NodeId::DOCUMENT),
        map(vec![(
            "a",
            map(vec![("_text", Proj::List(vec![st("c"), st("t")]))])
        )])
    );
    Ok(())
}

#[test]
fn built_elements_are_capped_and_duplicate_checked() -> TestResult {
    let limit =
        usize::try_from(crate::interpreter::limits::max_xml_attrs()).map_err(|e| e.to_string())?;
    let names: Vec<String> = (0..=limit).map(|i| format!("a{i}")).collect();
    let pairs: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "1")).collect();
    let mut tree = XmlTree::new(100);
    let err = tree
        .create_element("e", &pairs, None)
        .err()
        .ok_or("too many attributes accepted")?;
    assert_eq!(err.kind, XmlErrorKind::AttrsExceeded(limit));
    let e = s(tree.create_element("e", &pairs[..limit], None))?;
    let err = tree
        .set_attribute(e, "one_more", Some("1"))
        .err()
        .ok_or("set_attribute past the cap")?;
    assert_eq!(err.kind, XmlErrorKind::AttrsExceeded(limit));
    // A duplicate among many (the hashed path) is still caught.
    let mut many: Vec<(&str, &str)> = pairs[..40].to_vec();
    many.push(("a3", "2"));
    ensure(
        tree.create_element("e", &many, None).is_err(),
        "a duplicate among 41 attributes",
    )
}

// ---- Legacy knobs: what the old SOAP and c14n parsers accepted ----

#[test]
fn legacy_char_refs_resolve_the_quick_xml_way() -> TestResult {
    let mut soap = ParsePolicy::soap();
    soap.unknown_entity = OnUnknownEntity::Drop;
    // Any code point but NUL resolves, controls and U+FFFE included.
    let tree = with("<a>&#1;&#xFFFE;&#x41;&#66;</a>", &soap)?;
    assert_eq!(tree.text(root(&tree)?), "\u{1}\u{FFFE}AB");
    // One that does not resolve is dropped under Drop...
    for bad in [
        "&#0;",
        "&#xD800;",
        "&#x110000;",
        "&#X41;",
        "&#;",
        "&#x;",
        "&#-1;",
        "&#+65;",
    ] {
        let tree = with(&format!("<a>{bad}v</a>"), &soap)?;
        assert_eq!(tree.text(root(&tree)?), "v", "{bad}");
    }
    // ... and refused under Error (c14n), in text and attribute values.
    let c14n = ParsePolicy::c14n();
    let tree = with("<a x=\"&#1;\">&#1;</a>", &c14n)?;
    assert_eq!(tree.attribute(root(&tree)?, "x"), Some("\u{1}"));
    assert_eq!(
        fails("<a>&#0;</a>", &c14n)?.kind,
        XmlErrorKind::ForbiddenCharRef(0)
    );
    assert_eq!(
        fails("<a>&#x110000;</a>", &c14n)?.kind,
        XmlErrorKind::InvalidCharRef("#x110000".into())
    );
    fails("<a x=\"&#xD800;\"/>", &c14n)?;
    // `Xml.parse` keeps the strict rule.
    fails_xml("<a>&#1;</a>")?;
    Ok(())
}

#[test]
fn stop_after_root_reads_nothing_past_the_first_root() -> TestResult {
    let soap = ParsePolicy::soap();
    for input in [
        "<a>1</a></b>",
        "<a>1</a><b></c>",
        "<a>1</a><!DOCTYPE x>",
        "<a>1</a>&#0;",
        "<a>1</a><",
        "<a/></b>",
    ] {
        let tree = with(input, &soap)?;
        assert_eq!(tree.children(NodeId::DOCUMENT).len(), 1, "{input}");
    }
    // c14n reads on, so the same inputs fail there.
    fails("<a>1</a></b>", &ParsePolicy::c14n())?;
    // The event loop says Done at the root's end.
    let mut sink = TreeSink::new(100);
    let mut events = EventLoop::new("<a/>junk</x>".as_bytes(), soap);
    assert_eq!(
        events.step(&mut sink).map_err(|e| e.to_string())?,
        Step::Done
    );
    // Truncated input still fails (bug 6).
    fails("<a><b>1</b>", &soap)?;
    Ok(())
}

#[test]
fn text_before_the_root_is_kept_on_request() -> TestResult {
    let soap = ParsePolicy::soap();
    let tree = with(" pre <!--c--> &amp; more <a>x</a>", &soap)?;
    let texts: Vec<String> = tree
        .children(NodeId::DOCUMENT)
        .iter()
        .filter(|c| tree.kind(**c).name() == "text")
        .map(|c| tree.text(*c))
        .collect();
    assert_eq!(texts, [" pre ", " & more "]);
    assert_eq!(soap_shape(&tree), map(vec![("a", st("pre& morex"))]));
    // Blank text before the root is still not delivered; text after it never.
    let tree = with("  <a>x</a>", &soap)?;
    assert_eq!(tree.children(NodeId::DOCUMENT).len(), 1);
    // Other policies ignore (c14n) or refuse (Xml.parse) it.
    let tree = with("pre<a>x</a>", &ParsePolicy::c14n())?;
    assert_eq!(tree.children(NodeId::DOCUMENT).len(), 1);
    fails_xml("pre<a>x</a>")?;
    Ok(())
}

#[test]
fn soap_text_is_trimmed_piece_by_piece_at_comments_and_pis() -> TestResult {
    assert_eq!(soap("<a>x <!-- c --> y</a>")?, map(vec![("a", st("xy"))]));
    assert_eq!(soap("<a>x <?p d?> y</a>")?, map(vec![("a", st("xy"))]));
    // Inside a piece: entities, CDATA and child elements do not cut.
    assert_eq!(
        soap("<a> x &amp; <![CDATA[ y ]]> z </a>")?,
        map(vec![("a", st("x &  y  z"))])
    );
    assert_eq!(
        soap("<r>a<!--c-->b <c>1</c> d</r>")?,
        map(vec![(
            "r",
            map(vec![("_text", st("ab  d")), ("c", st("1"))])
        )])
    );
    // A piece is trimmed of XML whitespace only; the whole of Unicode's.
    assert_eq!(
        soap("<a>\u{a0}x\u{2003}<!--c-->y\u{a0}</a>")?,
        map(vec![("a", st("x\u{2003}y"))])
    );
    Ok(())
}

#[test]
fn soap_keeps_the_ambiguity_of_a_child_named_text() -> TestResult {
    // Text placed before a child named `_text` forms a list with it, and
    // stands unless more text follows, as with the old parser.
    assert_eq!(
        soap("<r>x<_text>v</_text></r>")?,
        map(vec![("r", Proj::List(vec![st("x"), st("v")]))])
    );
    assert_eq!(soap("<r><_text>v</_text>x</r>")?, map(vec![("r", st("x"))]));
    assert_eq!(
        soap("<r>x<_text>v</_text>y</r>")?,
        map(vec![("r", st("xy"))])
    );
    Ok(())
}

#[test]
fn the_per_element_declaration_cap_is_a_knob() -> TestResult {
    let decls: Vec<String> = (0..300).map(|i| format!("xmlns:p{i}=\"u{i}\"")).collect();
    let input = format!("<a {}/>", decls.join(" "));
    fails_xml(&input)?;
    with(&input, &ParsePolicy::soap())?;
    with(&input, &ParsePolicy::c14n())?;
    let mut policy = ParsePolicy::xml();
    policy.max_ns_decls_per_element = 300;
    with(&input, &policy)?;
    policy.max_ns_decls_per_element = 299;
    fails(&input, &policy)?;
    Ok(())
}

#[test]
fn duplicates_by_qname_and_a_self_closing_leaf_past_the_depth() -> TestResult {
    let c14n = ParsePolicy::c14n();
    // By qname: these are two attributes, as for the old c14n parser.
    with(
        "<a xmlns:p=\"u\" xmlns:q=\"u\" p:x=\"1\" q:x=\"2\"/>",
        &c14n,
    )?;
    with("<a x=\"1\" p:x=\"2\"/>", &c14n)?;
    let err = fails("<a x=\"1\" x=\"2\"/>", &c14n)?;
    assert_eq!(err.kind, XmlErrorKind::DuplicateAttribute("x".into()));
    let mut by_name = c14n;
    by_name.duplicate_by_qname = false;
    fails(
        "<a xmlns:p=\"u\" xmlns:q=\"u\" p:x=\"1\" q:x=\"2\"/>",
        &by_name,
    )?;
    // Depth: start tags count, a self-closing leaf just below does not.
    let policy = ParsePolicy::c14n().lowered(Some(3), None, None);
    with("<n><n><n><leaf/></n></n></n>", &policy)?;
    with("<n><n><n><leaf/><leaf/></n></n></n>", &policy)?;
    fails("<n><n><n><n></n></n></n></n>", &policy)?;
    let mut strict = policy;
    strict.self_closing_past_depth = false;
    fails("<n><n><n><leaf/></n></n></n>", &strict)?;
    Ok(())
}

#[test]
fn a_lenient_policy_ignores_a_malformed_declaration() -> TestResult {
    for policy in [ParsePolicy::soap(), ParsePolicy::c14n()] {
        let tree = with("<?xml version=\"1.0\" encoding?><a/>", &policy)?;
        assert_eq!(tree.decl().and_then(|d| d.encoding.clone()), None);
    }
    fails_xml("<?xml version=\"1.0\" encoding?><a/>")?;
    Ok(())
}
