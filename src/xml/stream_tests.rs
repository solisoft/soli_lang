//! Unit tests of streaming. Every test returns `Result` (no `unwrap` under
//! `src/xml`).

use std::io::{BufReader, Cursor};

use super::*;
use crate::xml::{serialize, SerializeOptions};

type TestResult = Result<(), String>;

fn table(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(p, u)| (p.to_string(), u.to_string()))
        .collect()
}

fn policy() -> ParsePolicy {
    ParsePolicy::xml()
}

/// Every match of `expr` in `input`, as compact XML.
fn stream_with(
    input: &str,
    expr: &str,
    ns: &[(&str, &str)],
    policy: ParsePolicy,
) -> Result<Vec<String>, XmlError> {
    let matcher = Matcher::compile(expr, &table(ns))?;
    let mut pull = Pull::new(input.as_bytes(), matcher, policy);
    let mut out = Vec::new();
    while let Some((tree, id)) = pull.next_match()? {
        out.push(serialize(&tree, id, &SerializeOptions::compact())?);
    }
    Ok(out)
}

/// The next match, the error as a String.
fn step<R: std::io::BufRead>(pull: &mut Pull<R>) -> Result<Option<(XmlTree, NodeId)>, String> {
    pull.next_match().map_err(|e| e.to_string())
}

fn stream(input: &str, expr: &str) -> Result<Vec<String>, String> {
    stream_with(input, expr, &[], policy()).map_err(|e| format!("{expr:?}: {e}"))
}

fn refused(input: &str, expr: &str, policy: ParsePolicy) -> Result<XmlError, String> {
    match stream_with(input, expr, &[], policy) {
        Ok(found) => Err(format!("expected a refusal, got {found:?}")),
        Err(err) => Ok(err),
    }
}

fn compile_error(expr: &str) -> Result<String, String> {
    match Matcher::compile(expr, &[]) {
        Ok(m) => Err(format!("{expr:?} compiled: {m:?}")),
        Err(err) => Ok(err.to_string()),
    }
}

fn check(cond: bool, what: impl std::fmt::Display) -> TestResult {
    if cond {
        Ok(())
    } else {
        Err(what.to_string())
    }
}

fn eq<T: PartialEq + std::fmt::Debug>(got: T, want: T) -> TestResult {
    check(got == want, format!("got {got:?}, want {want:?}"))
}

const FEED: &str = r#"<?xml version="1.0"?>
<rss version="2.0">
  <channel>
    <title>T</title>
    <item id="1" status="active"><title>A</title></item>
    <item id="2"><title>B</title><item id="nested"/></item>
    <item id="3" status="gone"><title>C &amp; D</title></item>
  </channel>
  <item id="top"/>
</rss>"#;

#[test]
fn a_name_matches_every_outermost_element_with_that_name() -> TestResult {
    let found = stream(FEED, "item")?;
    eq(found.len(), 4)?;
    eq(
        found[0].as_str(),
        r#"<item id="1" status="active"><title>A</title></item>"#,
    )?;
    // The nested item belongs to its outer match and is not yielded again.
    eq(
        found[1].as_str(),
        r#"<item id="2"><title>B</title><item id="nested"/></item>"#,
    )?;
    eq(
        found[2].as_str(),
        r#"<item id="3" status="gone"><title>C &amp; D</title></item>"#,
    )?;
    eq(found[3].as_str(), r#"<item id="top"/>"#)
}

#[test]
fn a_chain_matches_parent_child_steps_anywhere_and_slash_anchors_it() -> TestResult {
    eq(stream(FEED, "channel/item")?.len(), 3)?;
    eq(stream(FEED, "//channel/item")?.len(), 3)?;
    eq(stream(FEED, "rss/channel/item")?.len(), 3)?;
    eq(stream(FEED, "/rss/channel/item")?.len(), 3)?;
    eq(stream(FEED, "/channel/item")?.len(), 0)?;
    eq(
        stream(FEED, "/rss/item")?,
        vec![r#"<item id="top"/>"#.to_string()],
    )?;
    // Not nested in a match of `item/item`: the inner item is found.
    eq(
        stream(FEED, "item/item")?,
        vec![r#"<item id="nested"/>"#.to_string()],
    )?;
    eq(stream(FEED, "/rss")?.len(), 1)?;
    eq(stream(FEED, "channel/*")?.len(), 4)
}

#[test]
fn the_final_predicate_tests_an_attribute() -> TestResult {
    eq(stream(FEED, "item[@status]")?.len(), 2)?;
    eq(stream(FEED, "item[@status='active']")?.len(), 1)?;
    eq(stream(FEED, "item[ @status = \"gone\" ]")?.len(), 1)?;
    // XPath: `!=` is false when the attribute is missing.
    eq(stream(FEED, "item[@status!='active']")?.len(), 1)?;
    eq(stream(FEED, "channel/item[@id='2']")?.len(), 1)?;
    eq(stream(FEED, "item[@nope]")?.len(), 0)
}

#[test]
fn the_grammar_refuses_what_streaming_cannot_do() -> TestResult {
    for (expr, needle) in [
        ("", "expected a step"),
        ("item[1]", "only [@a]"),
        ("item[last()]", "only [@a]"),
        ("item/text()", "text()"),
        ("channel[@a]/item", "only allowed on the last step"),
        ("a//b", "'//' is only allowed at the start"),
        ("..", "'.' and '..'"),
        ("item/@id", "attribute cannot be matched"),
        ("item[@a='x]", "unterminated string"),
        ("item[@a=x]", "expected a quoted value"),
        ("item[@a][@b]", "only one predicate"),
        ("a|b", "unexpected '|'"),
        ("1a", "invalid name"),
    ] {
        let message = compile_error(expr)?;
        check(
            message.contains(needle) && message.contains("column"),
            format!("{expr:?}: {message}"),
        )?;
    }
    eq(
        compile_error("item[1]")?,
        "unsupported match at column 6: only [@a], [@a='v'] and [@a!='v'] are supported while streaming".to_string(),
    )
}

const ATOM: &str = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:x="urn:x">
  <entry><title>One</title><x:tag>a</x:tag></entry>
  <entry xmlns:y="urn:y"><title>Two</title><y:z/></entry>
  <x:entry/>
</feed>"#;

#[test]
fn an_unprefixed_name_matches_any_namespace_unless_the_table_has_a_default() -> TestResult {
    let atom = "http://www.w3.org/2005/Atom";
    let any = stream_with(ATOM, "entry", &[], policy()).map_err(|e| e.to_string())?;
    eq(any.len(), 3)?;
    let atom_only =
        stream_with(ATOM, "entry", &[("", atom)], policy()).map_err(|e| e.to_string())?;
    eq(atom_only.len(), 2)?;
    let other =
        stream_with(ATOM, "entry", &[("", "urn:other")], policy()).map_err(|e| e.to_string())?;
    eq(other.len(), 0)?;
    eq(stream(ATOM, "*:entry")?.len(), 3)?;
    // A prefix: the table first, then the root's declarations.
    let a = stream_with(ATOM, "a:entry", &[("a", atom)], policy()).map_err(|e| e.to_string())?;
    eq(a.len(), 2)?;
    eq(stream(ATOM, "x:entry")?.len(), 1)?;
    // `x:tag` inside the first entry, and `x:entry`.
    eq(stream(ATOM, "x:*")?.len(), 2)?;
    eq(stream(ATOM, "xmlns:entry")?.len(), 2)?;
    eq(stream(ATOM, "/xmlns:feed/xmlns:entry")?.len(), 2)
}

#[test]
fn a_prefix_nothing_binds_raises_when_the_root_opens() -> TestResult {
    let err = refused(ATOM, "q:entry", policy())?;
    eq(err.kind, XmlErrorKind::UnboundPrefix("q".to_string()))?;
    // `y` is declared below the root: the fallback only reads the root.
    let err = refused(ATOM, "y:z", policy())?;
    eq(err.kind, XmlErrorKind::UnboundPrefix("y".to_string()))
}

#[test]
fn a_match_carries_the_namespaces_in_scope_where_it_stood() -> TestResult {
    let found = stream(ATOM, "entry")?;
    eq(
        found[0].as_str(),
        r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:x="urn:x"><title>One</title><x:tag>a</x:tag></entry>"#,
    )?;
    // Its own declarations first, as written, then the inherited ones.
    eq(
        found[1].as_str(),
        r#"<entry xmlns:y="urn:y" xmlns="http://www.w3.org/2005/Atom" xmlns:x="urn:x"><title>Two</title><y:z/></entry>"#,
    )?;
    let matcher = Matcher::compile("entry", &[]).map_err(|e| e.to_string())?;
    let mut pull = Pull::new(ATOM.as_bytes(), matcher, policy());
    let (tree, id) = step(&mut pull)?.ok_or("no match")?;
    // A detached element: no document, no parent, and its prefixes resolve.
    eq(tree.parent(id), None)?;
    eq(tree.is_attached(id), false)?;
    eq(
        tree.lookup_namespace(id, Some("x")).as_deref(),
        Some("urn:x"),
    )?;
    let reparsed = crate::xml::parse(&found[0], &policy()).map_err(|e| e.to_string())?;
    let root = reparsed.root_element().ok_or("no root")?;
    eq(
        reparsed
            .element(root)
            .and_then(|el| el.ns_uri.map(|u| u.to_string())),
        Some("http://www.w3.org/2005/Atom".to_string()),
    )
}

#[test]
fn an_undeclared_default_is_not_copied_down() -> TestResult {
    let input = r#"<a xmlns="urn:a"><b xmlns=""><c/></b></a>"#;
    eq(stream(input, "c")?, vec!["<c/>".to_string()])
}

#[test]
fn a_match_split_across_buffer_boundaries_is_whole() -> TestResult {
    let mut input = String::from("<root>");
    for i in 0..200 {
        input.push_str(&format!(
            "<item id=\"{i}\"><name>n&amp;{i}</name><![CDATA[x]]><!--c--><?p d?></item>"
        ));
    }
    input.push_str("</root>");
    let matcher = Matcher::compile("item", &[]).map_err(|e| e.to_string())?;
    // A 7-byte buffer splits every tag and every text run.
    let reader = BufReader::with_capacity(7, input.as_bytes());
    let mut pull = Pull::new(reader, matcher, policy());
    let mut n = 0;
    while let Some((tree, id)) = step(&mut pull)? {
        let xml = serialize(&tree, id, &SerializeOptions::compact()).map_err(|e| e.to_string())?;
        eq(
            xml,
            format!("<item id=\"{n}\"><name>n&amp;{n}</name><![CDATA[x]]><!--c--><?p d?></item>"),
        )?;
        n += 1;
    }
    eq(n, 200)
}

#[test]
fn each_match_is_a_fresh_tree_capped_by_max_nodes() -> TestResult {
    let mut input = String::from("<r>");
    for _ in 0..50 {
        input.push_str("<i><a/><b/><c/></i>");
    }
    input.push_str("</r>");
    // 4 nodes per match: a cap of 4 holds every match, 3 does not.
    let mut capped = policy();
    capped.max_nodes = 4;
    eq(
        stream_with(&input, "i", &[], capped)
            .map_err(|e| e.to_string())?
            .len(),
        50,
    )?;
    capped.max_nodes = 3;
    let err = refused(&input, "i", capped)?;
    eq(err.kind, XmlErrorKind::NodesExceeded(3))
}

#[test]
fn max_bytes_caps_one_subtree_not_the_file() -> TestResult {
    let mut input = String::from("<r>");
    for i in 0..1000 {
        input.push_str(&format!("<i n=\"{i:04}\">0123456789</i>"));
    }
    input.push_str("</r>");
    // The file is ~30 KB; each match is 29 bytes.
    let mut small = policy();
    small.max_bytes = 64;
    eq(
        stream_with(&input, "i", &[], small)
            .map_err(|e| e.to_string())?
            .len(),
        1000,
    )?;
    // A match bigger than the cap is refused.
    let big = format!("<r><i>{}</i></r>", "x".repeat(100));
    let err = refused(&big, "i", small)?;
    eq(err.kind, XmlErrorKind::BytesExceeded(None, 64))
}

#[test]
fn a_huge_event_outside_matches_is_refused_before_it_is_buffered() -> TestResult {
    let mut small = policy();
    small.max_bytes = 1024;
    for input in [
        format!("<r><skip>{}</skip><i/></r>", "x".repeat(100_000)),
        format!("<r><skip a=\"{}\"/><i/></r>", "x".repeat(100_000)),
        format!("<r><!--{}--><i/></r>", "x".repeat(100_000)),
        // A run joined from many small events is one run.
        format!("<r><skip>{}</skip><i/></r>", "a&amp;".repeat(20_000)),
    ] {
        let matcher = Matcher::compile("i", &[]).map_err(|e| e.to_string())?;
        let mut pull = Pull::new(input.as_bytes(), matcher, small);
        match pull.next_match() {
            Ok(found) => return Err(format!("expected a refusal, got {}", found.is_some())),
            Err(err) => eq(err.kind, XmlErrorKind::BytesExceeded(None, 1024))?,
        }
        // Refused near the limit, not after reading the 100 KB event.
        check(
            pull.offset() < 4096,
            format!("read {} bytes", pull.offset()),
        )?;
    }
    // Small events add up to far more than the cap without tripping it.
    let many = format!("<r>{}<i/></r>", "<skip>abc</skip>\n".repeat(10_000));
    eq(
        stream_with(&many, "i", &[], small)
            .map_err(|e| e.to_string())?
            .len(),
        1,
    )
}

#[test]
fn hostile_documents_are_refused_while_streaming() -> TestResult {
    let doctype = r#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY a "aaaa">]><r><i>&a;</i></r>"#;
    eq(
        refused(doctype, "i", policy())?.kind,
        XmlErrorKind::Doctype("DOCTYPE declarations are not allowed in XML documents"),
    )?;
    let mut shallow = policy();
    shallow.max_depth = 8;
    let deep = format!("<i>{}{}</i>", "<d>".repeat(20), "</d>".repeat(20));
    eq(
        refused(&deep, "i", shallow)?.kind,
        XmlErrorKind::DepthExceeded(8),
    )?;
    // Depth counts from the document root, not from the match.
    let deep_outside = format!("{}<i/>{}", "<d>".repeat(20), "</d>".repeat(20));
    eq(
        refused(&deep_outside, "i", shallow)?.kind,
        XmlErrorKind::DepthExceeded(8),
    )?;
    eq(
        refused("<r><i>&bogus;</i></r>", "i", policy())?.kind,
        XmlErrorKind::UnknownEntity("bogus".to_string()),
    )?;
    eq(
        refused("<r><i>&#0;</i></r>", "i", policy())?.kind,
        XmlErrorKind::ForbiddenCharRef(0),
    )
}

#[test]
fn an_error_after_the_last_match_still_raises() -> TestResult {
    let input = "<r><i/><i/></r><second/>";
    let matcher = Matcher::compile("i", &[]).map_err(|e| e.to_string())?;
    let mut pull = Pull::new(input.as_bytes(), matcher, policy());
    eq(step(&mut pull)?.is_some(), true)?;
    eq(step(&mut pull)?.is_some(), true)?;
    match pull.next_match() {
        Err(err) => eq(err.kind, XmlErrorKind::SecondRoot("second".to_string()))?,
        Ok(other) => return Err(format!("expected an error, got {}", other.is_some())),
    }
    // The stream is over after an error.
    eq(step(&mut pull)?.is_none(), true)?;
    let truncated = refused("<r><i/><i>", "i", policy())?;
    eq(truncated.kind, XmlErrorKind::Unclosed("i".to_string()))
}

#[test]
fn errors_on_a_seekable_input_get_their_line_and_column() -> TestResult {
    let input = "<r>\n  <i/>\n  <é><i></é>\n</r>";
    let matcher = Matcher::compile("i", &[]).map_err(|e| e.to_string())?;
    let mut pull = Pull::new(Cursor::new(input.as_bytes().to_vec()), matcher, policy());
    eq(step(&mut pull)?.is_some(), true)?;
    let err = match pull.next_match() {
        Err(err) => pull.locate(err),
        Ok(_) => return Err("expected an error".to_string()),
    };
    let expected = crate::xml::parse(input, &policy())
        .err()
        .ok_or("the DOM parse should fail too")?;
    // Same position as the string parser reports.
    eq(err.position, expected.position)?;
    eq(
        err.to_string(),
        "line 3, column 9: expected </i>, found </é>".to_string(),
    )
}

#[test]
fn a_bom_is_skipped_and_offsets_count_after_it() -> TestResult {
    let input = "\u{FEFF}<r>\r\n<i/>\r\n<x></y></r>";
    let matcher = Matcher::compile("i", &[]).map_err(|e| e.to_string())?;
    let mut pull = Pull::new(Cursor::new(input.as_bytes().to_vec()), matcher, policy());
    eq(step(&mut pull)?.is_some(), true)?;
    let err = match pull.next_match() {
        Err(err) => pull.locate(err),
        Ok(_) => return Err("expected an error".to_string()),
    };
    let expected = crate::xml::parse(input, &policy())
        .err()
        .ok_or("the DOM parse should fail too")?;
    eq(err.position, expected.position)?;
    eq(err.position, Some((3, 4)))
}

#[test]
fn line_col_in_matches_line_col_on_strings() -> TestResult {
    let src = "a\r\nb\rc\nd é f\r\n\r\ng";
    for offset in 0..=src.len() {
        if !src.is_char_boundary(offset) {
            continue;
        }
        let streamed = line_col_in(src.as_bytes(), offset as u64).map_err(|e| e.to_string())?;
        eq(streamed, crate::xml::line_col(src, offset))?;
    }
    Ok(())
}

#[test]
fn an_interrupt_stops_a_long_scan() -> TestResult {
    let input = format!("<r>{}</r>", "<skip/>".repeat(10_000));
    let matcher = Matcher::compile("i", &[]).map_err(|e| e.to_string())?;
    let mut pull = Pull::new(input.as_bytes(), matcher, policy());
    let mut asked = 0;
    let result = pull.next_checked(&mut || {
        asked += 1;
        asked > 2
    });
    match result {
        Err(err) => eq(err.kind, XmlErrorKind::Syntax(INTERRUPTED.to_string()))?,
        Ok(_) => return Err("expected an interruption".to_string()),
    }
    eq(asked, 3)?;
    eq(step(&mut pull)?.is_none(), true)
}

#[test]
fn the_document_must_still_be_well_formed_and_have_a_root() -> TestResult {
    eq(stream("<r/>", "i")?.len(), 0)?;
    eq(
        refused("", "i", policy())?.kind,
        XmlErrorKind::NoRoot("no root element"),
    )?;
    eq(
        refused("<r>x</r>junk", "i", policy())?.kind,
        XmlErrorKind::TextOutsideRoot,
    )
}
