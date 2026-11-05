//! `XmlError`: what went wrong, and where.
//!
//! The parser records only a byte offset while it runs. Line and column are
//! computed from the source text **on the error path only**
//! ([`XmlError::locate`]), so a successful parse never pays for line counting.
//! A streaming driver that has no source string at hand fills
//! [`XmlError::position`] itself from the lines it counted.

use std::fmt;

/// Every way the XML core can fail.
///
/// The `Display` text is the message without any `Xml.parse:` style prefix:
/// the binding adds the prefix of the call that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlErrorKind {
    /// A `<!DOCTYPE …>` was found. The message is the caller's policy text
    /// ([`super::ParsePolicy::doctype_message`]), pinned by existing specs.
    Doctype(&'static str),
    /// `&name;` where `name` is not one of the five predefined entities.
    UnknownEntity(String),
    /// A character reference to a character XML forbids (`&#0;`, `&#xFFFE;`).
    ForbiddenCharRef(u32),
    /// A character reference that is not a number (`&#xZZ;`) or out of range.
    InvalidCharRef(String),
    /// A literal character XML forbids in text or in an attribute value.
    ForbiddenChar(u32),
    /// A prefix used in a name with no `xmlns:prefix` in scope.
    UnboundPrefix(String),
    /// Two attributes with the same expanded name on one element.
    DuplicateAttribute(String),
    /// `</found>` closes `<expected>`.
    MismatchedEndTag { expected: String, found: String },
    /// `</name>` with no element open.
    UnmatchedEndTag(String),
    /// End of input with `<name>` still open.
    Unclosed(String),
    /// The input holds no element at all. The message is the policy's.
    NoRoot(&'static str),
    /// A second top-level element.
    SecondRoot(String),
    /// Non-whitespace text (or CDATA, or a reference) outside the root element.
    TextOutsideRoot,
    /// An element, attribute or PI name that is not a valid (Q)Name.
    InvalidName(String),
    /// The input is not UTF-8.
    InvalidUtf8,
    /// `<?xml …?>` somewhere other than the very start.
    MisplacedDeclaration,
    /// A literal `<` inside an attribute value.
    LtInAttributeValue,
    /// Nesting deeper than the limit (the limit is carried).
    DepthExceeded(usize),
    /// More nodes in one tree than the limit (tombstones included).
    NodesExceeded(usize),
    /// Input larger than the limit: `(actual, limit)`; `actual` is `None`
    /// when a stream was cut off before its size was known.
    BytesExceeded(Option<usize>, usize),
    /// More attributes on one element than the limit.
    AttrsExceeded(usize),
    /// More namespace bindings live at once than the policy's
    /// `max_ns_bindings`.
    NamespacesExceeded(usize),
    /// A name longer than [`super::policy::MAX_NAME_LEN`] bytes.
    NameTooLong(usize),
    /// More text directly inside one element (or in one run outside the
    /// root) than the policy's `max_text_run`.
    TextRunExceeded(usize),
    /// Serialised output larger than the limit.
    OutputExceeded(usize),
    /// A comment containing `--` or ending with `-`.
    InvalidComment,
    /// A processing instruction whose target is `xml` (any case) or whose
    /// data contains `?>`.
    InvalidPi(String),
    /// A tree edit that would make a node its own ancestor.
    Cycle,
    /// A tree edit the node kind does not allow (text under a document, a
    /// child under a text node, …). The message says which.
    InvalidOperation(String),
    /// Any other error quick-xml reports, with its own message.
    Syntax(String),
}

/// An XML error: a kind, the byte offset where it was detected (when there is
/// one), and the 1-based line and column once [`XmlError::locate`] ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError {
    pub kind: XmlErrorKind,
    /// Byte offset in the input (after a stripped BOM), if the error has one.
    pub offset: Option<u64>,
    /// `(line, column)`, both 1-based; the column counts characters.
    pub position: Option<(usize, usize)>,
}

impl XmlError {
    /// An error with no location (tree edits, serialisation, limits checked
    /// before reading).
    pub fn new(kind: XmlErrorKind) -> Self {
        XmlError {
            kind,
            offset: None,
            position: None,
        }
    }

    /// An error detected at byte `offset` of the input.
    pub fn at(kind: XmlErrorKind, offset: u64) -> Self {
        XmlError {
            kind,
            offset: Some(offset),
            position: None,
        }
    }

    /// Fill [`XmlError::position`] from the source text the offset refers to.
    pub fn locate(mut self, src: &str) -> Self {
        if let Some(offset) = self.offset {
            self.position = Some(line_col(src, offset as usize));
        }
        self
    }

    /// The message without the `line L, column C: ` part.
    pub fn message(&self) -> String {
        self.kind.to_string()
    }

    /// The message quick-xml itself would print for this error, where the
    /// existing SOAP specs pin quick-xml's wording (`expected `</a>`, but
    /// `</b>` was found`). Other kinds give their own message.
    pub fn quick_xml_message(&self) -> String {
        match &self.kind {
            XmlErrorKind::MismatchedEndTag { expected, found } => {
                format!("ill-formed document: expected `</{expected}>`, but `</{found}>` was found")
            }
            XmlErrorKind::UnmatchedEndTag(name) => {
                format!("ill-formed document: close tag `</{name}>` does not match any open tag")
            }
            other => other.to_string(),
        }
    }
}

impl fmt::Display for XmlErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use XmlErrorKind::*;
        match self {
            Doctype(message) => f.write_str(message),
            UnknownEntity(name) => write!(f, "unknown entity '&{name};'"),
            ForbiddenCharRef(code) => {
                write!(f, "character reference to U+{code:04X}, which XML forbids")
            }
            InvalidCharRef(text) => write!(f, "invalid character reference '&{text};'"),
            ForbiddenChar(code) => write!(f, "character U+{code:04X} is not allowed in XML"),
            UnboundPrefix(prefix) => write!(f, "unbound namespace prefix '{prefix}'"),
            DuplicateAttribute(name) => write!(f, "duplicate attribute '{name}'"),
            MismatchedEndTag { expected, found } => {
                write!(f, "expected </{expected}>, found </{found}>")
            }
            UnmatchedEndTag(name) => write!(f, "</{name}> closes no open element"),
            Unclosed(name) => write!(f, "<{name}> is not closed at the end of the input"),
            NoRoot(message) => f.write_str(message),
            SecondRoot(name) => write!(f, "second root element <{name}>"),
            TextOutsideRoot => f.write_str("text outside the root element"),
            InvalidName(name) => write!(f, "invalid XML name '{name}'"),
            InvalidUtf8 => f.write_str("input is not valid UTF-8"),
            MisplacedDeclaration => {
                f.write_str("the XML declaration is only allowed at the start of the document")
            }
            LtInAttributeValue => f.write_str("'<' is not allowed in an attribute value"),
            DepthExceeded(limit) => {
                write!(f, "more than {limit} nested elements (SOLI_MAX_XML_DEPTH)")
            }
            NodesExceeded(limit) => write!(f, "more than {limit} nodes (SOLI_MAX_XML_NODES)"),
            BytesExceeded(Some(actual), limit) => write!(
                f,
                "input of {actual} bytes exceeds the {limit}-byte limit (SOLI_MAX_XML_BYTES)"
            ),
            BytesExceeded(None, limit) => {
                write!(
                    f,
                    "input exceeds the {limit}-byte limit (SOLI_MAX_XML_BYTES)"
                )
            }
            AttrsExceeded(limit) => write!(
                f,
                "more than {limit} attributes on one element (SOLI_MAX_XML_ATTRS)"
            ),
            NamespacesExceeded(limit) => write!(
                f,
                "more than {limit} namespace declarations in scope at once"
            ),
            NameTooLong(limit) => write!(f, "name longer than {limit} bytes"),
            TextRunExceeded(limit) => write!(f, "text longer than {limit} bytes"),
            OutputExceeded(limit) => write!(
                f,
                "output exceeds the {limit}-byte limit (SOLI_MAX_XML_BYTES)"
            ),
            InvalidComment => f.write_str("a comment may not contain '--' or end with '-'"),
            InvalidPi(target) => write!(f, "invalid processing instruction '{target}'"),
            Cycle => f.write_str("would create a cycle"),
            InvalidOperation(message) => f.write_str(message),
            Syntax(message) => f.write_str(message),
        }
    }
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some((line, column)) => write!(f, "line {line}, column {column}: {}", self.kind),
            None => write!(f, "{}", self.kind),
        }
    }
}

impl std::error::Error for XmlError {}

impl From<XmlError> for String {
    fn from(err: XmlError) -> String {
        err.to_string()
    }
}

/// 1-based `(line, column)` of byte `offset` in `src`. Lines end at LF, CRLF
/// or a lone CR; the column counts characters, not bytes. An offset past the
/// end, or inside a character, is clamped back to a character boundary.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let mut end = offset.min(src.len());
    while end > 0 && !src.is_char_boundary(end) {
        end -= 1;
    }
    let before = &src.as_bytes()[..end];
    let mut line = 1;
    let mut line_start = 0;
    let mut i = 0;
    while i < before.len() {
        match before[i] {
            b'\n' => {
                line += 1;
                line_start = i + 1;
            }
            b'\r' => {
                line += 1;
                if before.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                line_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let column = src[line_start.min(end)..end].chars().count() + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_lines_and_characters() {
        let src = "<a>\n  <b>é</c>";
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 4), (2, 1));
        // "  <b>é" is 5 characters but 6 bytes: the column counts characters.
        let pos = src.find("</c>").unwrap_or(0);
        assert_eq!(line_col(src, pos), (2, 7));
    }

    #[test]
    fn line_col_treats_crlf_and_lone_cr_as_one_line_break() {
        assert_eq!(line_col("a\r\nb", 3), (2, 1));
        assert_eq!(line_col("a\rb", 2), (2, 1));
        assert_eq!(line_col("a\r\n\r\nb", 5), (3, 1));
    }

    #[test]
    fn line_col_clamps_out_of_range_and_mid_character_offsets() {
        assert_eq!(line_col("ab", 99), (1, 3));
        // Offset 1 is inside the two-byte 'é'.
        assert_eq!(line_col("é", 1), (1, 1));
        assert_eq!(line_col("", 0), (1, 1));
    }

    #[test]
    fn display_carries_line_and_column_once_located() {
        let err = XmlError::at(
            XmlErrorKind::MismatchedEndTag {
                expected: "b".into(),
                found: "a".into(),
            },
            12,
        )
        .locate("<a>\n  <b>\n  </a>");
        assert_eq!(
            err.to_string(),
            "line 3, column 3: expected </b>, found </a>"
        );
        assert_eq!(
            err.quick_xml_message(),
            "ill-formed document: expected `</b>`, but `</a>` was found"
        );
    }

    #[test]
    fn limit_messages_name_their_variable() {
        assert_eq!(
            XmlErrorKind::DepthExceeded(256).to_string(),
            "more than 256 nested elements (SOLI_MAX_XML_DEPTH)"
        );
        assert!(XmlErrorKind::NodesExceeded(5)
            .to_string()
            .contains("SOLI_MAX_XML_NODES"));
        assert!(XmlErrorKind::BytesExceeded(Some(10), 5)
            .to_string()
            .contains("SOLI_MAX_XML_BYTES"));
        assert!(XmlErrorKind::AttrsExceeded(5)
            .to_string()
            .contains("SOLI_MAX_XML_ATTRS"));
    }
}
