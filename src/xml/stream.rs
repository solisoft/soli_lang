//! Streaming: read a document of any size and hand over one subtree per
//! match, never the whole tree (`Xml.each`, `Xml.reader`).
//!
//! [`Pull`] drives the shared [`EventLoop`] one [`EventLoop::step`] at a time
//! with a [`StreamSink`]. The sink keeps a stack of the open elements (local
//! name and namespace only) and, when an element matches the [`Matcher`],
//! builds that element and everything inside it as a fresh [`XmlTree`]. The
//! match is a detached element (no document, no parent) carrying every
//! namespace declaration in scope where it stood, so `to_xml`, `find` and
//! `xpath` work on it alone. A match nested inside a match belongs to the
//! outer subtree and is not handed over again.
//!
//! The document is checked as a whole, exactly as [`super::parse`] checks it
//! (DOCTYPE, entities, names, depth, a second root, …): an error past the
//! last match still raises.
//!
//! **Memory.** There is no cap on the file: memory is bounded by the depth
//! times the name size, plus one subtree, plus one event. The [`Bounded`]
//! reader enforces `max_bytes` **before** quick-xml buffers anything: it
//! refuses to hand over more than `max_bytes` bytes counted from a mark,
//! which the driver moves
//! - to the start tag of a match while a subtree is being built (one subtree
//!   is at most `max_bytes` of input),
//! - past every event the sink received otherwise (one event, or one run of
//!   text and references the loop is still joining, is at most `max_bytes`).
//!
//! `max_nodes` caps the nodes of one subtree. Text outside matches is read
//! and dropped.
//!
//! **Errors** carry the byte offset; line and column are computed only on the
//! error path, by reading the input again up to the offset
//! ([`Pull::locate`], for a seekable input such as a file).

use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::sync::Arc;

use super::error::{XmlError, XmlErrorKind};
use super::escape::is_ncname;
use super::parse::{EventLoop, NsScope, Sink, StartTag, Step};
use super::policy::ParsePolicy;
use super::tree::{Attr, Kind, NodeId, NsDecl, XmlDecl, XmlTree};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// How often (in events) [`Pull::next_checked`] asks whether to stop.
const INTERRUPT_EVERY: u32 = 256;

/// The message of the error [`Pull::next_checked`] returns when its
/// `interrupted` callback said stop. The binding replaces it with its own
/// (the request deadline's).
pub const INTERRUPTED: &str = "streaming was interrupted";

// ------------------------------------------------------------------ matcher

/// Which namespace a name test accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NsTest {
    /// Any namespace, or none (`*:name`, `*`, and an unprefixed name when the
    /// table has no `""` key).
    Any,
    /// Exactly this namespace (`None` = no namespace).
    Exact(Option<Arc<str>>),
    /// A prefix the table did not bind: resolved against the root element's
    /// declarations when the root opens.
    Pending(Box<str>),
}

impl NsTest {
    fn accepts(&self, ns: Option<&Arc<str>>) -> bool {
        match self {
            NsTest::Any => true,
            NsTest::Exact(uri) => uri.as_deref() == ns.map(|u| &**u),
            NsTest::Pending(_) => false,
        }
    }
}

/// One step of the chain: a local name (or any) in a namespace test.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NameTest {
    local: Option<Box<str>>,
    ns: NsTest,
}

/// The final step's predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PredOp {
    Exists,
    Eq(Box<str>),
    Ne(Box<str>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttrPred {
    local: Box<str>,
    /// An unprefixed attribute is in no namespace (XPath's rule).
    ns: NsTest,
    op: PredOp,
}

/// One open element, as far as matching needs it.
#[derive(Debug, Clone)]
struct Frame {
    local: Box<str>,
    ns: Option<Arc<str>>,
}

/// A compiled `match` expression: the streaming subset of the query grammar.
///
/// - steps `name`, `p:name`, `*:name`, `*`, `p:*`;
/// - a `/`-separated chain of parent → child steps (`rss/channel/item`,
///   the same as `//rss/channel/item`), with an optional leading `/` (the
///   first step is then the root element) or `//`;
/// - an optional predicate on the **final** step only: `[@a]`, `[@a='v']` or
///   `[@a!='v']` (`[@a!='v']` is false when `a` is missing, as in XPath).
///
/// Names follow the lenient flavour of `find`: an unprefixed name matches the
/// local name in any namespace, unless the table has a `""` key, whose URI it
/// must then be in. A prefix resolves against the table, then against the
/// root element's declarations (`xmlns` names the root's default namespace),
/// and raises when neither binds it. An unprefixed attribute is in no
/// namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matcher {
    steps: Vec<NameTest>,
    absolute: bool,
    pred: Option<AttrPred>,
}

fn unsupported(column: usize, what: &str) -> XmlError {
    XmlError::new(XmlErrorKind::Syntax(format!(
        "unsupported match at column {column}: {what}"
    )))
}

/// A tiny cursor over the expression, columns 1-based in characters.
struct Cursor<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn column(&self) -> usize {
        self.src[..self.pos].chars().count() + 1
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if !matches!(c, ' ' | '\t' | '\n' | '\r') {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    /// A run of characters that may belong to a name (`:` and the
    /// expression's punctuation stop it); the caller validates it.
    fn name(&mut self) -> &'a str {
        let rest = self.rest();
        let end = rest
            .char_indices()
            .find(|&(_, c)| c.is_whitespace() || "[]/@=!'\":*()|,<>".contains(c))
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        self.pos += end;
        &rest[..end]
    }
}

impl Matcher {
    /// Compile `expr` with the namespace table `ns` (`""` = the namespace an
    /// unprefixed name must be in). A prefix neither the table nor (later)
    /// the root binds raises when the root opens.
    pub fn compile(expr: &str, ns: &[(String, String)]) -> Result<Matcher, XmlError> {
        if expr.len() > 4096 {
            return Err(unsupported(1, "the expression is longer than 4096 bytes"));
        }
        let table = |prefix: &str| -> Option<Option<Arc<str>>> {
            ns.iter()
                .find(|(p, _)| p == prefix)
                .map(|(_, uri)| (!uri.is_empty()).then(|| Arc::from(uri.as_str())))
        };
        let mut cur = Cursor { src: expr, pos: 0 };
        cur.skip_ws();
        let absolute = if cur.eat("//") { false } else { cur.eat("/") };
        let mut steps = Vec::new();
        let mut pred = None;
        loop {
            if steps.len() >= 64 {
                return Err(unsupported(cur.column(), "more than 64 steps"));
            }
            let column = cur.column();
            if pred.is_some() {
                return Err(unsupported(
                    column,
                    "a predicate is only allowed on the last step",
                ));
            }
            steps.push(Self::step(&mut cur, &table)?);
            if cur.peek() == Some('[') {
                pred = Some(Self::predicate(&mut cur, &table)?);
            }
            cur.skip_ws();
            match cur.peek() {
                None => break,
                Some('/') if cur.rest().starts_with("//") => {
                    return Err(unsupported(
                        cur.column(),
                        "'//' is only allowed at the start of a match",
                    ))
                }
                Some('/') => {
                    cur.pos += 1;
                    if pred.is_some() {
                        return Err(unsupported(
                            column,
                            "a predicate is only allowed on the last step",
                        ));
                    }
                }
                Some('[') => {
                    return Err(unsupported(
                        cur.column(),
                        "only one predicate is allowed, on the last step",
                    ))
                }
                Some(c) => return Err(unsupported(cur.column(), &format!("unexpected '{c}'"))),
            }
        }
        Ok(Matcher {
            steps,
            absolute,
            pred,
        })
    }

    fn step(
        cur: &mut Cursor<'_>,
        table: &dyn Fn(&str) -> Option<Option<Arc<str>>>,
    ) -> Result<NameTest, XmlError> {
        let column = cur.column();
        if cur.eat("*") {
            if cur.eat(":") {
                let local = cur.name();
                if !is_ncname(local) {
                    return Err(unsupported(cur.column(), "expected a name after '*:'"));
                }
                return Ok(NameTest {
                    local: Some(local.into()),
                    ns: NsTest::Any,
                });
            }
            return Ok(NameTest {
                local: None,
                ns: NsTest::Any,
            });
        }
        let first = cur.name();
        if first.is_empty() {
            let what = match cur.peek() {
                None => "expected a step".to_string(),
                Some('@') => "an attribute cannot be matched while streaming".to_string(),
                Some(c) => format!("unexpected '{c}'"),
            };
            return Err(unsupported(column, &what));
        }
        if first == "." || first == ".." {
            return Err(unsupported(
                column,
                "'.' and '..' are not supported while streaming",
            ));
        }
        if cur.peek() == Some('(') {
            return Err(unsupported(
                column,
                &format!("'{first}()' is not supported while streaming"),
            ));
        }
        if !is_ncname(first) {
            return Err(unsupported(column, &format!("invalid name '{first}'")));
        }
        if !cur.eat(":") {
            let ns = match table("") {
                Some(uri) => NsTest::Exact(uri),
                None => NsTest::Any,
            };
            return Ok(NameTest {
                local: Some(first.into()),
                ns,
            });
        }
        let ns = Self::prefix(first, table);
        if cur.eat("*") {
            return Ok(NameTest { local: None, ns });
        }
        let local = cur.name();
        if !is_ncname(local) {
            return Err(unsupported(
                cur.column(),
                &format!("expected a name after '{first}:'"),
            ));
        }
        Ok(NameTest {
            local: Some(local.into()),
            ns,
        })
    }

    fn prefix(prefix: &str, table: &dyn Fn(&str) -> Option<Option<Arc<str>>>) -> NsTest {
        if prefix == "xml" {
            return NsTest::Exact(Some(Arc::from(super::tree::XML_NS)));
        }
        match table(prefix) {
            Some(uri) => NsTest::Exact(uri),
            None => NsTest::Pending(prefix.into()),
        }
    }

    fn predicate(
        cur: &mut Cursor<'_>,
        table: &dyn Fn(&str) -> Option<Option<Arc<str>>>,
    ) -> Result<AttrPred, XmlError> {
        cur.eat("[");
        cur.skip_ws();
        let column = cur.column();
        if !cur.eat("@") {
            return Err(unsupported(
                column,
                "only [@a], [@a='v'] and [@a!='v'] are supported while streaming",
            ));
        }
        let name_column = cur.column();
        let first = cur.name();
        if !is_ncname(first) {
            return Err(unsupported(
                name_column,
                "expected an attribute name after '@'",
            ));
        }
        let (local, ns) = if cur.eat(":") {
            let local = cur.name();
            if !is_ncname(local) {
                return Err(unsupported(
                    cur.column(),
                    &format!("expected a name after '{first}:'"),
                ));
            }
            (local, Self::prefix(first, table))
        } else {
            (first, NsTest::Exact(None))
        };
        cur.skip_ws();
        let op = if cur.eat("!=") {
            PredOp::Ne(Self::literal(cur)?)
        } else if cur.eat("=") {
            PredOp::Eq(Self::literal(cur)?)
        } else {
            PredOp::Exists
        };
        cur.skip_ws();
        if !cur.eat("]") {
            return Err(unsupported(
                cur.column(),
                "only [@a], [@a='v'] and [@a!='v'] are supported while streaming",
            ));
        }
        Ok(AttrPred {
            local: local.into(),
            ns,
            op,
        })
    }

    fn literal(cur: &mut Cursor<'_>) -> Result<Box<str>, XmlError> {
        cur.skip_ws();
        let column = cur.column();
        let quote = match cur.peek() {
            Some(q @ ('\'' | '"')) => q,
            _ => return Err(unsupported(column, "expected a quoted value")),
        };
        cur.pos += 1;
        let rest = cur.rest();
        let Some(end) = rest.find(quote) else {
            return Err(unsupported(column, "unterminated string"));
        };
        cur.pos += end + 1;
        Ok(rest[..end].into())
    }

    /// Bind the prefixes the table left open, against the root element's own
    /// declarations; `xmlns` names the root's default namespace.
    fn resolve_at_root(&mut self, decls: &[NsDecl]) -> Result<(), XmlError> {
        let resolve = |test: &mut NsTest| -> Result<(), XmlError> {
            let NsTest::Pending(prefix) = test else {
                return Ok(());
            };
            let wanted = (&**prefix != "xmlns").then_some(&**prefix);
            match decls.iter().find(|d| d.prefix.as_deref() == wanted) {
                Some(decl) => {
                    *test = NsTest::Exact((!decl.uri.is_empty()).then(|| decl.uri.clone()));
                    Ok(())
                }
                None => Err(XmlError::new(XmlErrorKind::UnboundPrefix(
                    prefix.to_string(),
                ))),
            }
        };
        for step in &mut self.steps {
            resolve(&mut step.ns)?;
        }
        if let Some(pred) = &mut self.pred {
            resolve(&mut pred.ns)?;
        }
        Ok(())
    }

    fn matches(&self, frames: &[Frame], attrs: &[Attr]) -> bool {
        let n = self.steps.len();
        if frames.len() < n || (self.absolute && frames.len() != n) {
            return false;
        }
        let tail = &frames[frames.len() - n..];
        let names_match = self.steps.iter().zip(tail).all(|(test, frame)| {
            test.local.as_deref().is_none_or(|l| l == &*frame.local)
                && test.ns.accepts(frame.ns.as_ref())
        });
        names_match && self.pred.as_ref().is_none_or(|pred| pred.holds(attrs))
    }
}

impl AttrPred {
    fn holds(&self, attrs: &[Attr]) -> bool {
        let found = attrs
            .iter()
            .find(|a| a.local_name() == &*self.local && self.ns.accepts(a.ns_uri.as_ref()));
        match (&self.op, found) {
            (_, None) => false,
            (PredOp::Exists, Some(_)) => true,
            (PredOp::Eq(v), Some(a)) => a.value == *v,
            (PredOp::Ne(v), Some(a)) => a.value != *v,
        }
    }
}

// ----------------------------------------------------------- bounded input

/// A `BufRead` that hands over at most `limit` bytes past a mark, so a
/// single oversized event (a 2 GB text node, a tag with a 2 GB attribute) is
/// refused before quick-xml buffers it. It also skips a UTF-8 BOM, so offsets
/// count from the first byte after it.
pub struct Bounded<R> {
    inner: R,
    consumed: u64,
    mark: u64,
    limit: u64,
    tripped: bool,
    started: bool,
}

impl<R: BufRead> Bounded<R> {
    pub fn new(inner: R, limit: u64) -> Self {
        Bounded {
            inner,
            consumed: 0,
            mark: 0,
            limit,
            tripped: false,
            started: false,
        }
    }

    /// Bytes handed over so far (after the BOM).
    pub fn consumed(&self) -> u64 {
        self.consumed
    }

    /// Count the limit from `at` on.
    pub fn set_mark(&mut self, at: u64) {
        self.mark = at;
    }

    /// The limit refused a read.
    pub fn tripped(&self) -> bool {
        self.tripped
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    fn skip_bom(&mut self) -> io::Result<()> {
        if !self.started {
            self.started = true;
            if self.inner.fill_buf()?.starts_with(UTF8_BOM) {
                self.inner.consume(UTF8_BOM.len());
            }
        }
        Ok(())
    }
}

impl<R: BufRead> Read for Bounded<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(out.len());
        out[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl<R: BufRead> BufRead for Bounded<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.skip_bom()?;
        let used = self.consumed.saturating_sub(self.mark);
        let room = self.limit.saturating_sub(used);
        let available = self.inner.fill_buf()?;
        if available.is_empty() {
            return Ok(available);
        }
        if room == 0 {
            self.tripped = true;
            return Err(io::Error::other("the XML input exceeds the byte limit"));
        }
        let n = usize::try_from(room)
            .unwrap_or(usize::MAX)
            .min(available.len());
        Ok(&available[..n])
    }

    fn consume(&mut self, amount: usize) {
        self.consumed = self.consumed.saturating_add(amount as u64);
        self.inner.consume(amount);
    }
}

/// 1-based `(line, column)` of byte `offset` of a stream, with the rules of
/// [`super::line_col`]: lines end at LF, CRLF or a lone CR, the column counts
/// characters. A leading UTF-8 BOM is skipped (offsets count after it).
pub fn line_col_in<R: Read>(input: R, offset: u64) -> io::Result<(usize, usize)> {
    let mut reader = io::BufReader::new(input);
    if reader.fill_buf()?.starts_with(UTF8_BOM) {
        reader.consume(UTF8_BOM.len());
    }
    let (mut line, mut column) = (1usize, 1usize);
    let mut prev_cr = false;
    let mut left = offset;
    while left > 0 {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let take = usize::try_from(left).unwrap_or(usize::MAX).min(chunk.len());
        for &byte in &chunk[..take] {
            match byte {
                b'\n' if prev_cr => {}
                b'\n' | b'\r' => {
                    line += 1;
                    column = 1;
                }
                b if b & 0xC0 != 0x80 => column += 1,
                _ => {}
            }
            prev_cr = byte == b'\r';
        }
        reader.consume(take);
        left -= take as u64;
    }
    Ok((line, column))
}

// --------------------------------------------------------------------- sink

/// A subtree being built.
struct Capture {
    tree: XmlTree,
    stack: Vec<NodeId>,
    root: NodeId,
}

/// The streaming sink: matches elements and builds each match as a
/// standalone subtree.
pub struct StreamSink {
    matcher: Matcher,
    frames: Vec<Frame>,
    capture: Option<Capture>,
    ready: Option<(XmlTree, NodeId)>,
    node_limit: usize,
    root_seen: bool,
    /// Callbacks received: the driver moves the byte mark past each one.
    events: u64,
}

impl StreamSink {
    pub fn new(matcher: Matcher, node_limit: usize) -> Self {
        StreamSink {
            matcher,
            frames: Vec::new(),
            capture: None,
            ready: None,
            node_limit,
            root_seen: false,
            events: 0,
        }
    }

    fn capturing(&self) -> bool {
        self.capture.is_some()
    }

    fn push_leaf(&mut self, kind: Kind) -> Result<(), XmlError> {
        self.events += 1;
        if let Some(cap) = &mut self.capture {
            let parent = cap.stack.last().copied().unwrap_or(cap.root);
            cap.tree.push_child_unchecked(parent, kind)?;
        }
        Ok(())
    }
}

impl Sink for StreamSink {
    fn declaration(&mut self, _decl: XmlDecl) -> Result<(), XmlError> {
        self.events += 1;
        Ok(())
    }

    fn start_element(&mut self, tag: StartTag, scope: &NsScope) -> Result<(), XmlError> {
        self.events += 1;
        let StartTag {
            qname,
            local_start,
            ns_uri,
            attrs,
            ns_decls,
            self_closing,
        } = tag;
        if let Some(cap) = &mut self.capture {
            let parent = cap.stack.last().copied().unwrap_or(cap.root);
            let id = cap.tree.push_child_unchecked(
                parent,
                Kind::Element {
                    qname,
                    local_start,
                    ns_uri,
                    attrs,
                    ns_decls,
                    self_closing,
                },
            )?;
            cap.stack.push(id);
            return Ok(());
        }
        if !self.root_seen {
            self.root_seen = true;
            self.matcher.resolve_at_root(&ns_decls)?;
        }
        let local: Box<str> = qname.get(local_start as usize..).unwrap_or(&qname).into();
        self.frames.push(Frame {
            local,
            ns: ns_uri.clone(),
        });
        if !self.matcher.matches(&self.frames, &attrs) {
            return Ok(());
        }
        // The match carries every declaration in scope: its own first, as
        // written, then the inherited ones it does not redeclare.
        let mut decls = ns_decls;
        if scope.binding_count() > decls.len() {
            for decl in scope.in_scope() {
                if !decls.iter().any(|own| own.prefix == decl.prefix) {
                    decls.push(decl);
                }
            }
        }
        let mut tree = XmlTree::new(self.node_limit);
        let root = tree.push_detached_unchecked(Kind::Element {
            qname,
            local_start,
            ns_uri,
            attrs,
            ns_decls: decls,
            self_closing,
        })?;
        self.capture = Some(Capture {
            tree,
            stack: vec![root],
            root,
        });
        Ok(())
    }

    fn end_element(&mut self) -> Result<(), XmlError> {
        self.events += 1;
        if let Some(cap) = &mut self.capture {
            cap.stack.pop();
            if !cap.stack.is_empty() {
                return Ok(());
            }
            if let Some(cap) = self.capture.take() {
                self.ready = Some((cap.tree, cap.root));
            }
        }
        self.frames.pop();
        Ok(())
    }

    fn text(&mut self, text: String) -> Result<(), XmlError> {
        self.push_leaf(Kind::Text(text.into()))
    }

    fn cdata(&mut self, text: String) -> Result<(), XmlError> {
        self.push_leaf(Kind::CData(text.into()))
    }

    fn comment(&mut self, text: String) -> Result<(), XmlError> {
        self.push_leaf(Kind::Comment(text.into()))
    }

    fn pi(&mut self, target: String, data: String) -> Result<(), XmlError> {
        self.push_leaf(Kind::Pi {
            target: target.into(),
            data: data.into(),
        })
    }
}

// --------------------------------------------------------------------- pull

/// A pull cursor: [`Pull::next_match`] reads until the next match closes and
/// returns it as a tree of its own (the match is the detached element whose
/// id comes with it), or `None` at the end of the document.
pub struct Pull<R: BufRead> {
    events: EventLoop<Bounded<R>>,
    sink: StreamSink,
    max_bytes: usize,
    finished: bool,
}

impl<R: BufRead> Pull<R> {
    /// Stream `input` under `policy` (its `max_bytes` caps one subtree and
    /// one event, not the input; `max_nodes` caps one subtree).
    pub fn new(input: R, matcher: Matcher, policy: ParsePolicy) -> Self {
        let max_bytes = policy.max_bytes;
        let mut loop_policy = policy;
        // The loop's own check counts from the start of the input; the
        // bounded reader counts from the mark instead.
        loop_policy.max_bytes = usize::MAX;
        Pull {
            events: EventLoop::new(Bounded::new(input, max_bytes as u64), loop_policy),
            sink: StreamSink::new(matcher, policy.max_nodes),
            max_bytes,
            finished: false,
        }
    }

    /// The next match, or `None` once the document has been read to its end
    /// (and found well-formed). After an error, `None`.
    pub fn next_match(&mut self) -> Result<Option<(XmlTree, NodeId)>, XmlError> {
        self.next_checked(&mut || false)
    }

    /// [`Pull::next_match`], asking `interrupted` every so often while it reads; a
    /// `true` answer ends the stream with the error [`INTERRUPTED`].
    pub fn next_checked(
        &mut self,
        interrupted: &mut dyn FnMut() -> bool,
    ) -> Result<Option<(XmlTree, NodeId)>, XmlError> {
        let mut steps: u32 = 0;
        while !self.finished {
            steps = steps.wrapping_add(1);
            if steps.is_multiple_of(INTERRUPT_EVERY) && interrupted() {
                self.finished = true;
                return Err(XmlError::new(XmlErrorKind::Syntax(INTERRUPTED.to_string())));
            }
            let before = self.events.input_mut().consumed();
            let seen = self.sink.events;
            let was_capturing = self.sink.capturing();
            let step = self.events.step(&mut self.sink);
            let input = self.events.input_mut();
            match step {
                Ok(step) => {
                    if self.sink.capturing() {
                        if !was_capturing {
                            input.set_mark(before);
                        }
                    } else if self.sink.events != seen {
                        input.set_mark(input.consumed());
                    }
                    if step == Step::Done {
                        self.finished = true;
                    }
                    if let Some(found) = self.sink.ready.take() {
                        return Ok(Some(found));
                    }
                }
                Err(err) => {
                    self.finished = true;
                    self.sink.capture = None;
                    if input.tripped() {
                        return Err(XmlError::at(
                            XmlErrorKind::BytesExceeded(None, self.max_bytes),
                            before,
                        ));
                    }
                    return Err(err);
                }
            }
        }
        Ok(None)
    }

    /// Bytes read so far (after a BOM).
    pub fn offset(&mut self) -> u64 {
        self.events.input_mut().consumed()
    }

    /// Stop reading: later calls give `None`.
    pub fn finish(&mut self) {
        self.finished = true;
        self.sink.capture = None;
    }
}

impl<R: BufRead + Seek> Pull<R> {
    /// Fill `err`'s line and column by reading the input again from its
    /// start up to the error's offset (the error path only). The stream is
    /// over afterwards.
    pub fn locate(&mut self, mut err: XmlError) -> XmlError {
        self.finish();
        let (Some(offset), None) = (err.offset, err.position) else {
            return err;
        };
        let inner = self.events.input_mut().get_mut();
        if inner.seek(SeekFrom::Start(0)).is_ok() {
            if let Ok(position) = line_col_in(&mut *inner, offset) {
                err.position = Some(position);
            }
        }
        err
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
