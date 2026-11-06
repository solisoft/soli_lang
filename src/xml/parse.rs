//! The one parse loop: quick-xml's `NsReader` events, checked against a
//! [`ParsePolicy`] and fed to a [`Sink`].
//!
//! The loop owns everything the policy decides — DOCTYPE refusal, entity and
//! character references, line-ending and attribute-value normalisation, the
//! namespace scope, the root-element rules and every limit — so a sink only
//! builds. [`TreeSink`] builds an [`XmlTree`]; the streaming lane's sink
//! builds one subtree per match from the same events. The loop is a
//! [`EventLoop::step`] state machine rather than one call, so a pull reader
//! can stop between events.
//!
//! What the loop guarantees to a sink:
//! - text arrives joined: adjacent text and references are one `text` call,
//!   with references decoded and line endings normalised (CRLF and lone CR
//!   become LF); a CDATA section is its own `cdata` call;
//! - `start_element` carries the element's resolved namespace, its
//!   attributes (decoded, normalised, duplicate-checked) and its declarations;
//!   a self-closing tag is `start_element` (with `self_closing`) then
//!   `end_element`;
//! - nothing outside the policy reaches the sink: refused constructs are
//!   errors, ignored ones (a trailing root under `TrailingContent::Ignore`,
//!   blank text under `strip_blank`) are never delivered.

use std::io::BufRead;
use std::sync::Arc;

use quick_xml::errors::{Error as QxError, IllFormedError};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::NsReader;

use super::error::{XmlError, XmlErrorKind};
use super::escape::{first_forbidden, is_ncname, is_qname, is_xml_char, split_qname};
use super::ns::Bindings;
use super::policy::{
    OnNoRoot, OnUnknownEntity, ParsePolicy, Strictness, TrailingContent, MAX_DEPTH_CEILING,
    MAX_NAME_LEN,
};
use super::tree::{find_duplicate, Attr, Kind, NodeId, NsDecl, XmlDecl, XmlTree, XML_NS};

/// An element start, fully resolved.
#[derive(Debug, Clone)]
pub struct StartTag {
    pub qname: Box<str>,
    /// Byte offset of the local name in `qname`.
    pub local_start: u16,
    pub ns_uri: Option<Arc<str>>,
    pub attrs: Vec<Attr>,
    pub ns_decls: Vec<NsDecl>,
    pub self_closing: bool,
}

/// What the loop feeds. Every method may refuse (a node limit, a stop).
pub trait Sink {
    /// `<?xml …?>`, before anything else.
    fn declaration(&mut self, _decl: XmlDecl) -> Result<(), XmlError> {
        Ok(())
    }
    /// An element opens. `scope` is the namespace scope *including* the
    /// element's own declarations (a streaming sink copies it onto a match).
    fn start_element(&mut self, tag: StartTag, scope: &NsScope) -> Result<(), XmlError>;
    /// The element opened last closes.
    fn end_element(&mut self) -> Result<(), XmlError>;
    /// A joined text run (never empty).
    fn text(&mut self, text: String) -> Result<(), XmlError>;
    /// One CDATA section.
    fn cdata(&mut self, text: String) -> Result<(), XmlError>;
    fn comment(&mut self, text: String) -> Result<(), XmlError>;
    fn pi(&mut self, target: String, data: String) -> Result<(), XmlError>;
}

/// The namespace bindings in scope while parsing. Lookups are one hash
/// probe and [`NsScope::in_scope`] is linear in the live bindings, however
/// many a hostile document piles up (the policy's `max_ns_bindings` caps
/// them).
#[derive(Debug, Clone)]
pub struct NsScope {
    bindings: Bindings,
    marks: Vec<usize>,
    xml: Arc<str>,
}

impl Default for NsScope {
    fn default() -> Self {
        NsScope {
            bindings: Bindings::default(),
            marks: Vec::new(),
            xml: Arc::from(XML_NS),
        }
    }
}

impl NsScope {
    /// The URI `prefix` (`None` = default) is bound to, `None` when unbound
    /// (or, for the default namespace, undeclared with `xmlns=""`).
    pub fn lookup(&self, prefix: Option<&str>) -> Option<&Arc<str>> {
        if prefix == Some("xml") {
            return Some(&self.xml);
        }
        self.bindings.get(prefix).filter(|uri| !uri.is_empty())
    }

    /// Every binding in scope, nearest winning, `xml` and undeclared defaults
    /// excluded, in the order the prefixes were first declared. Linear in
    /// the number of live bindings ([`NsScope::binding_count`]).
    pub fn in_scope(&self) -> Vec<NsDecl> {
        self.bindings.in_scope()
    }

    /// Live bindings, shadowed ones included: what `max_ns_bindings` caps.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    fn push(&mut self, decls: &[NsDecl]) {
        self.marks.push(self.bindings.len());
        for decl in decls {
            self.bindings.push(decl.prefix.as_deref(), decl.uri.clone());
        }
    }

    fn pop(&mut self) {
        if let Some(mark) = self.marks.pop() {
            self.bindings.truncate(mark);
        }
    }
}

/// [`EventLoop::step`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Continue,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootState {
    Before,
    Inside,
    After,
}

/// The parse loop over any `BufRead` (a string's bytes, a buffered file).
///
/// `max_bytes` and the text cap are checked once quick-xml has handed over a
/// whole event, so one huge text or tag is buffered before it is refused.
/// [`parse`] checks a string's length first; a driver reading a file must
/// bound the reader itself (a `BufRead` adapter that stops after
/// `max_bytes`) to keep a single oversized event out of memory.
pub struct EventLoop<R: BufRead> {
    reader: NsReader<R>,
    buf: Vec<u8>,
    policy: ParsePolicy,
    scope: NsScope,
    /// The open elements of the kept tree: qname, start offset, and the
    /// bytes of text delivered directly inside it so far (`max_text_run`).
    open: Vec<(Box<str>, u64, usize)>,
    /// Depth inside a trailing root being skipped (`TrailingContent::Ignore`).
    skip_depth: usize,
    state: RootState,
    pending: String,
    pending_start: u64,
    /// A reference was part of the pending run (references are not allowed
    /// outside the root).
    pending_ref: bool,
    seen_event: bool,
    done: bool,
}

fn utf8(bytes: &[u8], offset: u64) -> Result<&str, XmlError> {
    std::str::from_utf8(bytes).map_err(|_| XmlError::at(XmlErrorKind::InvalidUtf8, offset))
}

fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// CRLF and lone CR become LF (XML 1.0 §2.11).
pub(crate) fn normalize_eol(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

fn map_qx_error(err: QxError, offset: u64) -> XmlError {
    let kind = match err {
        QxError::IllFormed(IllFormedError::MismatchedEndTag { expected, found }) => {
            XmlErrorKind::MismatchedEndTag { expected, found }
        }
        QxError::IllFormed(IllFormedError::UnmatchedEndTag(name)) => {
            XmlErrorKind::UnmatchedEndTag(name)
        }
        QxError::IllFormed(IllFormedError::DoubleHyphenInComment) => XmlErrorKind::InvalidComment,
        QxError::Encoding(_) => XmlErrorKind::InvalidUtf8,
        other => XmlErrorKind::Syntax(other.to_string()),
    };
    XmlError::at(kind, offset)
}

/// Resolve the body of a reference (`amp`, `#38`, `#x26`). `Ok(None)` is an
/// unknown entity the policy drops.
fn resolve_reference(
    name: &str,
    policy: &ParsePolicy,
    offset: u64,
) -> Result<Option<char>, XmlError> {
    if let Some(number) = name.strip_prefix('#') {
        if policy.legacy_char_refs {
            return legacy_char_ref(number, policy, offset);
        }
        let code = match number.strip_prefix('x') {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => number.parse::<u32>().ok(),
        }
        .filter(|_| !number.is_empty() && !number.starts_with('+') && !number.starts_with("x+"))
        .ok_or_else(|| XmlError::at(XmlErrorKind::InvalidCharRef(name.to_string()), offset))?;
        return match char::from_u32(code) {
            Some(c) if is_xml_char(c) => Ok(Some(c)),
            Some(_) => Err(XmlError::at(XmlErrorKind::ForbiddenCharRef(code), offset)),
            None if code <= 0x10FFFF => {
                // A surrogate code point.
                Err(XmlError::at(XmlErrorKind::ForbiddenCharRef(code), offset))
            }
            None => Err(XmlError::at(
                XmlErrorKind::InvalidCharRef(name.to_string()),
                offset,
            )),
        };
    }
    let c = match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => {
            return match policy.unknown_entity {
                OnUnknownEntity::Error => Err(XmlError::at(
                    XmlErrorKind::UnknownEntity(name.to_string()),
                    offset,
                )),
                OnUnknownEntity::Drop => Ok(None),
            }
        }
    };
    Ok(Some(c))
}

/// A character reference resolved by quick-xml's rule (`policy.legacy_char_refs`):
/// a decimal or `x`-hex number with no sign, any code point but NUL. One that
/// does not resolve follows `policy.unknown_entity`.
fn legacy_char_ref(
    number: &str,
    policy: &ParsePolicy,
    offset: u64,
) -> Result<Option<char>, XmlError> {
    let (digits, radix) = match number.strip_prefix('x') {
        Some(hex) => (hex, 16),
        None => (number, 10),
    };
    let code = if digits.starts_with(['+', '-']) {
        None
    } else {
        u32::from_str_radix(digits, radix).ok()
    };
    match code.filter(|code| *code != 0).and_then(char::from_u32) {
        Some(c) => Ok(Some(c)),
        None => match policy.unknown_entity {
            OnUnknownEntity::Drop => Ok(None),
            OnUnknownEntity::Error => Err(XmlError::at(
                match code {
                    // NUL or a surrogate: XML forbids the character.
                    Some(code) if code <= 0x10FFFF => XmlErrorKind::ForbiddenCharRef(code),
                    _ => XmlErrorKind::InvalidCharRef(format!("#{number}")),
                },
                offset,
            )),
        },
    }
}

/// Decode an attribute value: references resolved, then attribute-value
/// normalisation after line-ending normalisation — so a literal CRLF becomes
/// **one** space, while `&#10;` stays a line feed.
fn decode_attr_value(raw: &str, policy: &ParsePolicy, offset: u64) -> Result<String, XmlError> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '&' => {
                let end = rest.find(';').ok_or_else(|| {
                    XmlError::at(
                        XmlErrorKind::Syntax(
                            "entity or character reference not closed in an attribute value"
                                .to_string(),
                        ),
                        offset,
                    )
                })?;
                if let Some(c) = resolve_reference(&rest[..end], policy, offset)? {
                    out.push(c);
                }
                rest = &rest[end + 1..];
            }
            '\r' => {
                out.push(' ');
                if rest.starts_with('\n') {
                    rest = &rest[1..];
                }
            }
            '\n' | '\t' => out.push(' '),
            '<' if policy.strict_syntax => {
                return Err(XmlError::at(XmlErrorKind::LtInAttributeValue, offset))
            }
            c if policy.strict_syntax && !is_xml_char(c) => {
                return Err(XmlError::at(XmlErrorKind::ForbiddenChar(c as u32), offset))
            }
            c => out.push(c),
        }
    }
    Ok(out)
}

fn decl_field(
    value: Option<Result<std::borrow::Cow<'_, [u8]>, quick_xml::events::attributes::AttrError>>,
    offset: u64,
) -> Result<Option<Box<str>>, XmlError> {
    match value {
        None => Ok(None),
        Some(Ok(bytes)) => Ok(Some(utf8(&bytes, offset)?.into())),
        Some(Err(err)) => Err(map_qx_error(QxError::InvalidAttr(err), offset)),
    }
}

fn check_name(name: &str, strict: bool, offset: u64) -> Result<(), XmlError> {
    if name.len() > MAX_NAME_LEN {
        return Err(XmlError::at(
            XmlErrorKind::NameTooLong(MAX_NAME_LEN),
            offset,
        ));
    }
    if strict && !is_qname(name) {
        return Err(XmlError::at(
            XmlErrorKind::InvalidName(name.to_string()),
            offset,
        ));
    }
    Ok(())
}

impl<R: BufRead> EventLoop<R> {
    pub fn new(input: R, policy: ParsePolicy) -> Self {
        // quick-xml counts nesting in a `u16`: no policy may go past the
        // ceiling, however it was built.
        let mut policy = policy;
        policy.max_depth = policy.max_depth.min(MAX_DEPTH_CEILING);
        let mut reader = NsReader::from_reader(input);
        let config = reader.config_mut();
        config.check_end_names = true;
        config.check_comments = policy.strict_syntax;
        config.allow_dangling_amp = false;
        config.allow_unmatched_ends = false;
        config.expand_empty_elements = false;
        config.trim_text(false);
        reader
            .resolver_mut()
            .set_max_declarations_per_element(policy.max_ns_decls_per_element);
        EventLoop {
            reader,
            buf: Vec::new(),
            policy,
            scope: NsScope::default(),
            open: Vec::new(),
            skip_depth: 0,
            state: RootState::Before,
            pending: String::new(),
            pending_start: 0,
            pending_ref: false,
            seen_event: false,
            done: false,
        }
    }

    /// Open elements of the kept tree.
    pub fn depth(&self) -> usize {
        self.open.len()
    }

    /// Bytes consumed so far.
    pub fn offset(&self) -> u64 {
        self.reader.buffer_position()
    }

    /// The namespace scope at the current position.
    pub fn scope(&self) -> &NsScope {
        &self.scope
    }

    pub fn policy(&self) -> &ParsePolicy {
        &self.policy
    }

    /// Run to the end of the input.
    pub fn run<S: Sink>(&mut self, sink: &mut S) -> Result<(), XmlError> {
        while self.step(sink)? == Step::Continue {}
        Ok(())
    }

    /// Read and handle one event.
    pub fn step<S: Sink>(&mut self, sink: &mut S) -> Result<Step, XmlError> {
        if self.done {
            return Ok(Step::Done);
        }
        let start = self.reader.buffer_position();
        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        let result = match self.reader.read_event_into(&mut buf) {
            Ok(event) => {
                let end = self.reader.buffer_position();
                if end > self.policy.max_bytes as u64 {
                    Err(XmlError::at(
                        XmlErrorKind::BytesExceeded(None, self.policy.max_bytes),
                        start,
                    ))
                } else {
                    self.handle(event, start, sink).map(|step| {
                        // A self-closing root under `stop_after_root` ends the
                        // parse from inside `start`.
                        if self.done {
                            Step::Done
                        } else {
                            step
                        }
                    })
                }
            }
            Err(err) => Err(map_qx_error(err, self.reader.error_position())),
        };
        self.buf = buf;
        if result.is_err() {
            self.done = true;
        }
        result
    }

    fn handle<S: Sink>(
        &mut self,
        event: Event<'_>,
        start: u64,
        sink: &mut S,
    ) -> Result<Step, XmlError> {
        let first = !self.seen_event;
        self.seen_event = true;
        match event {
            Event::Text(e) => {
                let text = utf8(&e, start)?;
                self.push_text(text, start)?;
            }
            Event::GeneralRef(e) => {
                let name = utf8(&e, start)?;
                if self.pending.is_empty() && !self.pending_ref {
                    self.pending_start = start;
                }
                self.pending_ref = true;
                if let Some(c) = resolve_reference(name, &self.policy, start)? {
                    // A reference is not line-ending normalised: `&#13;` is a CR.
                    self.pending.push(c);
                    self.check_run(self.pending.len(), self.pending_start)?;
                }
            }
            Event::CData(e) => {
                self.flush(sink)?;
                let text = normalize_eol(utf8(&e, start)?);
                if self.outside_root() {
                    self.outside_content(start)?;
                } else {
                    self.check_literal(&text, start)?;
                    self.check_run(text.len(), start)?;
                    self.add_direct_text(text.len());
                    sink.cdata(text)?;
                }
            }
            Event::Start(e) => {
                self.flush(sink)?;
                self.start(&e, start, false, sink)?;
            }
            Event::Empty(e) => {
                self.flush(sink)?;
                self.start(&e, start, true, sink)?;
            }
            Event::End(_) => {
                self.flush(sink)?;
                if self.skip_depth > 0 {
                    self.skip_depth -= 1;
                } else {
                    self.open.pop();
                    self.scope.pop();
                    sink.end_element()?;
                    if self.open.is_empty() {
                        self.state = RootState::After;
                        if self.policy.stop_after_root {
                            self.done = true;
                            return Ok(Step::Done);
                        }
                    }
                }
            }
            Event::Comment(e) => {
                self.flush(sink)?;
                if self.skip_depth == 0 {
                    let text = normalize_eol(utf8(&e, start)?);
                    if self.policy.strict_syntax && text.ends_with('-') {
                        return Err(XmlError::at(XmlErrorKind::InvalidComment, start));
                    }
                    self.check_literal(&text, start)?;
                    sink.comment(text)?;
                }
            }
            Event::PI(e) => {
                self.flush(sink)?;
                if self.skip_depth == 0 {
                    let target = utf8(e.target(), start)?.to_string();
                    let data = utf8(e.content(), start)?;
                    let data = if self.policy.raw_pi_data {
                        normalize_eol(data)
                    } else {
                        normalize_eol(data.trim_start_matches(is_xml_space))
                    };
                    if target.len() > MAX_NAME_LEN {
                        return Err(XmlError::at(XmlErrorKind::NameTooLong(MAX_NAME_LEN), start));
                    }
                    if self.policy.strict_syntax
                        && (!is_ncname(&target) || target.eq_ignore_ascii_case("xml"))
                    {
                        return Err(XmlError::at(XmlErrorKind::InvalidPi(target), start));
                    }
                    self.check_literal(&data, start)?;
                    sink.pi(target, data)?;
                }
            }
            Event::Decl(e) => {
                self.flush(sink)?;
                if !first {
                    if self.policy.strict_syntax {
                        return Err(XmlError::at(XmlErrorKind::MisplacedDeclaration, start));
                    }
                } else {
                    let version = match e.version() {
                        Ok(v) => utf8(&v, start)?.into(),
                        Err(err) if self.policy.strict_syntax => {
                            return Err(map_qx_error(err, start))
                        }
                        Err(_) => "1.0".into(),
                    };
                    // The lenient policies never read the declaration before:
                    // a malformed field is left out rather than refused.
                    let strict = self.policy.strict_syntax;
                    let field = |value| match decl_field(value, start) {
                        Err(_) if !strict => Ok(None),
                        other => other,
                    };
                    let decl = XmlDecl {
                        version,
                        encoding: field(e.encoding())?,
                        standalone: field(e.standalone())?,
                    };
                    sink.declaration(decl)?;
                }
            }
            Event::DocType(_) => {
                return Err(XmlError::at(
                    XmlErrorKind::Doctype(self.policy.doctype_message),
                    start,
                ));
            }
            Event::Eof => {
                self.flush(sink)?;
                self.done = true;
                return self.finish(start);
            }
        }
        Ok(Step::Continue)
    }

    fn finish(&mut self, offset: u64) -> Result<Step, XmlError> {
        if self.skip_depth > 0 {
            // A trailing root cut off by the end of input: it was being
            // ignored anyway.
            return Ok(Step::Done);
        }
        if let Some((name, start, _)) = self.open.last() {
            return Err(match self.policy.no_root {
                OnNoRoot::Error(message) => XmlError::at(XmlErrorKind::NoRoot(message), offset),
                _ => XmlError::at(XmlErrorKind::Unclosed(name.to_string()), *start),
            });
        }
        if self.state == RootState::Before {
            match self.policy.no_root {
                OnNoRoot::Error(message) | OnNoRoot::ErrorUnclosedSeparately(message) => {
                    return Err(XmlError::at(XmlErrorKind::NoRoot(message), offset));
                }
                OnNoRoot::Empty => {}
            }
        }
        Ok(Step::Done)
    }

    /// Outside the kept root: before it, after it, or inside a skipped one.
    fn outside_root(&self) -> bool {
        self.open.is_empty() || self.skip_depth > 0
    }

    /// Non-whitespace content outside the root.
    fn outside_content(&self, offset: u64) -> Result<(), XmlError> {
        if self.skip_depth > 0 {
            return Ok(());
        }
        match self.policy.trailing_content {
            TrailingContent::Error => Err(XmlError::at(XmlErrorKind::TextOutsideRoot, offset)),
            TrailingContent::Ignore => Ok(()),
        }
    }

    fn check_literal(&self, text: &str, offset: u64) -> Result<(), XmlError> {
        if self.policy.strict_syntax {
            if let Some(c) = first_forbidden(text) {
                return Err(XmlError::at(XmlErrorKind::ForbiddenChar(c as u32), offset));
            }
        }
        Ok(())
    }

    /// `len` more bytes of text directly in the current element (or one run
    /// outside the root) would pass `max_text_run`.
    fn check_run(&self, len: usize, offset: u64) -> Result<(), XmlError> {
        let held = self.open.last().map_or(0, |(_, _, held)| *held);
        match self.policy.max_text_run {
            Some(limit) if held.saturating_add(len) > limit => {
                Err(XmlError::at(XmlErrorKind::TextRunExceeded(limit), offset))
            }
            _ => Ok(()),
        }
    }

    fn add_direct_text(&mut self, len: usize) {
        if self.skip_depth == 0 {
            if let Some((_, _, held)) = self.open.last_mut() {
                *held = held.saturating_add(len);
            }
        }
    }

    fn push_text(&mut self, text: &str, start: u64) -> Result<(), XmlError> {
        if self.pending.is_empty() && !self.pending_ref {
            self.pending_start = start;
        }
        let text = normalize_eol(text);
        self.check_literal(&text, start)?;
        self.pending.push_str(&text);
        self.check_run(self.pending.len(), self.pending_start)
    }

    fn flush<S: Sink>(&mut self, sink: &mut S) -> Result<(), XmlError> {
        let had_ref = std::mem::replace(&mut self.pending_ref, false);
        if self.pending.is_empty() {
            if had_ref && self.outside_root() {
                return self.outside_content(self.pending_start);
            }
            return Ok(());
        }
        let text = std::mem::take(&mut self.pending);
        let blank = text.chars().all(is_xml_space);
        if self.outside_root() {
            if !blank || had_ref {
                if self.state == RootState::Before && self.policy.keep_text_before_root {
                    return sink.text(text);
                }
                return self.outside_content(self.pending_start);
            }
            return Ok(());
        }
        self.add_direct_text(text.len());
        if blank && !self.policy.keep_blank_text {
            return Ok(());
        }
        sink.text(text)
    }

    fn start<S: Sink>(
        &mut self,
        e: &BytesStart<'_>,
        offset: u64,
        empty: bool,
        sink: &mut S,
    ) -> Result<(), XmlError> {
        if self.skip_depth > 0 {
            if !empty {
                self.skip_depth += 1;
                if self.skip_depth > self.policy.max_depth {
                    return Err(XmlError::at(
                        XmlErrorKind::DepthExceeded(self.policy.max_depth),
                        offset,
                    ));
                }
            }
            return Ok(());
        }
        if self.state == RootState::After {
            match self.policy.trailing_content {
                TrailingContent::Error => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                    return Err(XmlError::at(XmlErrorKind::SecondRoot(name), offset));
                }
                TrailingContent::Ignore => {
                    if !empty {
                        self.skip_depth = 1;
                    }
                    return Ok(());
                }
            }
        }
        let legacy_leaf = empty
            && self.policy.self_closing_past_depth
            && self.open.len() == self.policy.max_depth;
        if self.open.len() + 1 > self.policy.max_depth && !legacy_leaf {
            return Err(XmlError::at(
                XmlErrorKind::DepthExceeded(self.policy.max_depth),
                offset,
            ));
        }
        let tag = self.build_tag(e, offset, empty)?;
        self.state = RootState::Inside;
        let qname = tag.qname.clone();
        sink.start_element(tag, &self.scope)?;
        if empty {
            self.scope.pop();
            sink.end_element()?;
            if self.open.is_empty() {
                self.state = RootState::After;
                if self.policy.stop_after_root {
                    self.done = true;
                }
            }
        } else {
            self.open.push((qname, offset, 0));
        }
        Ok(())
    }

    /// Resolve one start tag. Pushes the element's declarations on the scope
    /// (the caller pops them when the element closes).
    fn build_tag(
        &mut self,
        e: &BytesStart<'_>,
        offset: u64,
        empty: bool,
    ) -> Result<StartTag, XmlError> {
        let strict = self.policy.strict_syntax;
        let qname = utf8(e.name().as_ref(), offset)?.to_string();
        check_name(&qname, strict, offset)?;
        let (prefix, _) = split_qname(&qname);
        if strict && prefix == Some("xmlns") {
            return Err(XmlError::at(XmlErrorKind::InvalidName(qname), offset));
        }

        let lenient = self.policy.lenient_attributes;
        let mut ns_decls: Vec<NsDecl> = Vec::new();
        let mut raw_attrs: Vec<(String, String)> = Vec::new();
        for attr in e.attributes().with_checks(false) {
            let attr = match attr {
                Ok(attr) => attr,
                // Best effort: keep what came before the bad attribute.
                Err(_) if lenient => break,
                Err(err) => return Err(map_qx_error(QxError::InvalidAttr(err), offset)),
            };
            let decoded = utf8(attr.key.as_ref(), offset).and_then(|key| {
                check_name(key, strict, offset)?;
                let value = decode_attr_value(utf8(&attr.value, offset)?, &self.policy, offset)?;
                Ok((key, value))
            });
            let (key, value) = match decoded {
                Ok(pair) => pair,
                Err(_) if lenient => continue,
                Err(err) => return Err(err),
            };
            let decl_prefix = if key == "xmlns" {
                Some(None)
            } else {
                key.strip_prefix("xmlns:").map(Some)
            };
            match decl_prefix {
                Some(decl_prefix) => {
                    if strict && decl_prefix.is_some() && value.is_empty() {
                        return Err(XmlError::at(
                            XmlErrorKind::Syntax(format!(
                                "namespace prefix '{}' bound to an empty URI",
                                decl_prefix.unwrap_or_default()
                            )),
                            offset,
                        ));
                    }
                    if ns_decls.iter().any(|d| d.prefix.as_deref() == decl_prefix)
                        && self.policy.duplicate_attribute == Strictness::Error
                    {
                        return Err(XmlError::at(
                            XmlErrorKind::DuplicateAttribute(key.to_string()),
                            offset,
                        ));
                    }
                    ns_decls.push(NsDecl {
                        prefix: decl_prefix.map(Box::from),
                        uri: Arc::from(value),
                    });
                }
                None => {
                    if raw_attrs.len() >= self.policy.max_attrs {
                        if lenient {
                            continue;
                        }
                        return Err(XmlError::at(
                            XmlErrorKind::AttrsExceeded(self.policy.max_attrs),
                            offset,
                        ));
                    }
                    raw_attrs.push((key.to_string(), value));
                }
            }
        }

        if self.scope.binding_count().saturating_add(ns_decls.len()) > self.policy.max_ns_bindings {
            return Err(XmlError::at(
                XmlErrorKind::NamespacesExceeded(self.policy.max_ns_bindings),
                offset,
            ));
        }
        self.scope.push(&ns_decls);
        let result = self.resolve_tag(qname, raw_attrs, ns_decls, offset, empty);
        if result.is_err() {
            self.scope.pop();
        }
        result
    }

    fn resolve(&self, prefix: Option<&str>, offset: u64) -> Result<Option<Arc<str>>, XmlError> {
        let uri = self.scope.lookup(prefix).cloned();
        match (prefix, &uri, self.policy.unbound_prefix) {
            (Some(p), None, Strictness::Error) => Err(XmlError::at(
                XmlErrorKind::UnboundPrefix(p.to_string()),
                offset,
            )),
            _ => Ok(uri),
        }
    }

    fn resolve_tag(
        &self,
        qname: String,
        raw_attrs: Vec<(String, String)>,
        ns_decls: Vec<NsDecl>,
        offset: u64,
        empty: bool,
    ) -> Result<StartTag, XmlError> {
        let (prefix, _) = split_qname(&qname);
        let ns_uri = self.resolve(prefix, offset)?;
        let mut attrs = Vec::with_capacity(raw_attrs.len());
        for (key, value) in raw_attrs {
            let ns_uri = match split_qname(&key).0 {
                Some(p) => self.resolve(Some(p), offset)?,
                None => None,
            };
            attrs.push(Attr {
                qname: key.into(),
                ns_uri,
                value: value.into(),
            });
        }
        if self.policy.duplicate_attribute == Strictness::Error {
            let dup = if self.policy.duplicate_by_qname {
                let mut seen = std::collections::HashSet::with_capacity(attrs.len());
                attrs.iter().find(|attr| !seen.insert(&attr.qname))
            } else {
                find_duplicate(&attrs)
            };
            if let Some(dup) = dup {
                return Err(XmlError::at(
                    XmlErrorKind::DuplicateAttribute(dup.qname.to_string()),
                    offset,
                ));
            }
        }
        let local_start = match qname.find(':') {
            Some(colon) => u16::try_from(colon + 1).unwrap_or(0),
            None => 0,
        };
        Ok(StartTag {
            qname: qname.into(),
            local_start,
            ns_uri,
            attrs,
            ns_decls,
            self_closing: empty,
        })
    }
}

/// The DOM sink: builds an [`XmlTree`].
pub struct TreeSink {
    tree: XmlTree,
    stack: Vec<NodeId>,
}

impl TreeSink {
    pub fn new(node_limit: usize) -> Self {
        TreeSink {
            tree: XmlTree::new(node_limit),
            stack: Vec::new(),
        }
    }

    pub fn into_tree(self) -> XmlTree {
        self.tree
    }

    fn current(&self) -> NodeId {
        self.stack.last().copied().unwrap_or(NodeId::DOCUMENT)
    }

    fn push(&mut self, kind: Kind) -> Result<NodeId, XmlError> {
        let parent = self.current();
        self.tree.push_child_unchecked(parent, kind)
    }
}

impl Sink for TreeSink {
    fn declaration(&mut self, decl: XmlDecl) -> Result<(), XmlError> {
        self.tree.set_decl_unchecked(decl);
        Ok(())
    }

    fn start_element(&mut self, tag: StartTag, _scope: &NsScope) -> Result<(), XmlError> {
        let id = self.push(Kind::Element {
            qname: tag.qname,
            local_start: tag.local_start,
            ns_uri: tag.ns_uri,
            attrs: tag.attrs,
            ns_decls: tag.ns_decls,
            self_closing: tag.self_closing,
        })?;
        self.stack.push(id);
        Ok(())
    }

    fn end_element(&mut self) -> Result<(), XmlError> {
        self.stack.pop();
        Ok(())
    }

    fn text(&mut self, text: String) -> Result<(), XmlError> {
        self.push(Kind::Text(text.into())).map(|_| ())
    }

    fn cdata(&mut self, text: String) -> Result<(), XmlError> {
        self.push(Kind::CData(text.into())).map(|_| ())
    }

    fn comment(&mut self, text: String) -> Result<(), XmlError> {
        self.push(Kind::Comment(text.into())).map(|_| ())
    }

    fn pi(&mut self, target: String, data: String) -> Result<(), XmlError> {
        self.push(Kind::Pi {
            target: target.into(),
            data: data.into(),
        })
        .map(|_| ())
    }
}

/// Parse `input` into a tree under `policy`. A UTF-8 BOM is skipped; errors
/// carry their line and column.
pub fn parse(input: &str, policy: &ParsePolicy) -> Result<XmlTree, XmlError> {
    let body = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    if body.len() > policy.max_bytes {
        return Err(XmlError::new(XmlErrorKind::BytesExceeded(
            Some(body.len()),
            policy.max_bytes,
        )));
    }
    let mut sink = TreeSink::new(policy.max_nodes);
    EventLoop::new(body.as_bytes(), *policy)
        .run(&mut sink)
        .map_err(|err| err.locate(body))?;
    Ok(sink.into_tree())
}

/// [`parse`] for bytes: invalid UTF-8 is an error at the first bad byte.
pub fn parse_bytes(input: &[u8], policy: &ParsePolicy) -> Result<XmlTree, XmlError> {
    match std::str::from_utf8(input) {
        Ok(text) => parse(text, policy),
        Err(err) => {
            let good = &input[..err.valid_up_to()];
            let text = std::str::from_utf8(good).unwrap_or("");
            Err(XmlError::at(XmlErrorKind::InvalidUtf8, good.len() as u64).locate(text))
        }
    }
}
