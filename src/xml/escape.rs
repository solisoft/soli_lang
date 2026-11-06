//! Escaping, the filter for characters XML forbids, and name validation.
//!
//! Every byte the XML core writes goes through this module, so the rules live
//! in one place:
//!
//! - text: `&` `<` `>` are escaped, and a CR as `&#13;` (a literal CR would be
//!   turned into LF by the next parser);
//! - attribute values: `&` `<` `>` `"`, plus `&#9;` `&#10;` `&#13;`, so a value
//!   survives attribute-value normalisation when it is read back;
//! - characters XML forbids (C0 controls other than tab, LF and CR, and
//!   U+FFFE / U+FFFF) are **dropped** from written text and attribute values,
//!   as `spreadsheet::push_xml_text` does — output is always well-formed;
//! - [`escape5`] is `SOAP.xml_escape`, byte for byte (it drops nothing).

use std::borrow::Cow;

/// `true` for a character XML 1.0 allows in a document (production `Char`).
/// `&str` cannot hold surrogates, so only the C0 range and U+FFFE/U+FFFF
/// need excluding.
#[inline]
pub fn is_xml_char(c: char) -> bool {
    match c {
        '\t' | '\n' | '\r' => true,
        c if (c as u32) < 0x20 => false,
        '\u{FFFE}' | '\u{FFFF}' => false,
        _ => true,
    }
}

/// The first character XML forbids in `s`, if any. A byte scan first: the
/// common case (no control byte, no `EF BF BE/BF`) never decodes characters.
pub fn first_forbidden(s: &str) -> Option<char> {
    let bytes = s.as_bytes();
    let suspicious = bytes
        .iter()
        .any(|&b| (b < 0x20 && b != b'\t' && b != b'\n' && b != b'\r') || b == 0xEF);
    if !suspicious {
        return None;
    }
    s.chars().find(|&c| !is_xml_char(c))
}

/// `s` without the characters XML forbids. Borrows when there is none.
pub fn drop_forbidden(s: &str) -> Cow<'_, str> {
    if first_forbidden(s).is_none() {
        return Cow::Borrowed(s);
    }
    Cow::Owned(s.chars().filter(|&c| is_xml_char(c)).collect())
}

/// Append `s` escaped as element text: `& < >`, and CR as `&#13;`.
/// Forbidden characters are dropped.
pub fn escape_text_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c if !is_xml_char(c) => {}
            c => out.push(c),
        }
    }
}

/// Append `s` escaped as a double-quoted attribute value: `& < > "`, and
/// `&#9; &#10; &#13;` for tab, LF and CR. Forbidden characters are dropped.
pub fn escape_attr_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if !is_xml_char(c) => {}
            c => out.push(c),
        }
    }
}

/// `s` escaped as element text (see [`escape_text_into`]).
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_text_into(&mut out, s);
    out
}

/// `s` escaped as an attribute value (see [`escape_attr_into`]).
pub fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_attr_into(&mut out, s);
    out
}

/// The five-character escape of `SOAP.xml_escape` and `Xml.escape`:
/// `&` `<` `>` `"` `'` (as `&apos;`). Safe in text and in either kind of
/// quoted attribute. Nothing is dropped, so the bytes match `SOAP.xml_escape`
/// exactly.
pub fn escape5(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// XML 1.0 (5th edition) `NameStartChar`, without `:`.
#[inline]
pub(crate) fn is_name_start_char(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// XML 1.0 (5th edition) `NameChar`, without `:`.
#[inline]
pub(crate) fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// `true` for a namespace-spec `NCName`: a Name with no colon.
pub fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_name_start_char(c) => chars.all(is_name_char),
        _ => false,
    }
}

/// `true` for a `QName`: `NCName` or `NCName:NCName`.
pub fn is_qname(s: &str) -> bool {
    match s.split_once(':') {
        Some((prefix, local)) => is_ncname(prefix) && is_ncname(local),
        None => is_ncname(s),
    }
}

/// Split a qname into `(prefix, local)` at its first colon.
pub fn split_qname(qname: &str) -> (Option<&str>, &str) {
    match qname.split_once(':') {
        Some((prefix, local)) => (Some(prefix), local),
        None => (None, qname),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_escaping_covers_every_special_character() {
        assert_eq!(
            escape_text("a & b < c > d \"q\" 'a'"),
            "a &amp; b &lt; c &gt; d \"q\" 'a'"
        );
        assert_eq!(escape_text("x\ry\nz\tw"), "x&#13;y\nz\tw");
        assert_eq!(escape_text("]]>"), "]]&gt;");
    }

    #[test]
    fn attribute_escaping_protects_whitespace_from_normalisation() {
        assert_eq!(escape_attr("a\"b'c<d>e&f"), "a&quot;b'c&lt;d&gt;e&amp;f");
        assert_eq!(escape_attr("\t\n\r "), "&#9;&#10;&#13; ");
    }

    #[test]
    fn forbidden_characters_are_dropped_from_text_and_attributes() {
        assert_eq!(
            escape_text("a\u{1}b\u{0}c\u{FFFE}d\u{FFFF}e\u{1F}"),
            "abcde"
        );
        assert_eq!(escape_attr("a\u{8}b"), "ab");
        assert_eq!(drop_forbidden("ok\u{B}"), "ok");
        assert!(matches!(drop_forbidden("clean é"), Cow::Borrowed(_)));
        assert_eq!(first_forbidden("é\u{EFFF}x"), None);
        assert_eq!(first_forbidden("a\u{FFFF}"), Some('\u{FFFF}'));
        assert!(is_xml_char('\u{FFFD}'));
        assert!(is_xml_char('\u{10FFFF}'));
    }

    /// `SOAP.xml_escape` as it is written in `builtins/soap.rs` today: five
    /// chained `replace` calls. `escape5` must give the same bytes.
    fn soap_xml_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    #[test]
    fn escape5_is_byte_identical_to_soap_xml_escape() {
        let samples = [
            "",
            "plain",
            "a < b & \"c\"",
            "&amp; already escaped",
            "'single' and \"double\"",
            "<<>>&&''\"\"",
            "control \u{1} and tab\t and CR\r",
            "unicode é ∑ 😀 \u{FFFF}",
        ];
        for sample in samples {
            assert_eq!(
                escape5(sample),
                soap_xml_escape(sample),
                "sample {sample:?}"
            );
        }
        assert_eq!(escape5("a < b & \"c\""), "a &lt; b &amp; &quot;c&quot;");
    }

    #[test]
    fn names_are_validated_as_ncnames_and_qnames() {
        for good in ["a", "_x", "ns:local", "é", "a-b.c_1", "x\u{B7}y", "日本"] {
            assert!(is_qname(good), "{good} should be a QName");
        }
        for bad in [
            "", "1a", "-a", ":a", "a:", "a:b:c", "a b", "a<b", "a\"", ".x", "a:1",
        ] {
            assert!(!is_qname(bad), "{bad} should not be a QName");
        }
        assert!(is_ncname("abc"));
        assert!(!is_ncname("a:b"));
        assert_eq!(split_qname("ram:Name"), (Some("ram"), "Name"));
        assert_eq!(split_qname("Name"), (None, "Name"));
    }
}
