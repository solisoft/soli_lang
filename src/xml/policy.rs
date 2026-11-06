//! `ParsePolicy`: the per-caller knobs of one shared parser.
//!
//! `Xml.parse`, `SOAP.parse` / `SOAP.call` and the c14n trio all parse with
//! [`super::parse`], but each has promises to keep: SOAP drops unknown
//! entities and ignores trailing roots, c14n allows unbound prefixes and pins
//! its own error strings. Those differences are data here, never `if caller`
//! branches in the parse loop. None of this is exposed to Soli: a Soli caller
//! can only *lower* the limits, through [`ParsePolicy::lowered`].

use crate::interpreter::limits;

/// Longest element, attribute or PI name accepted, in bytes. Fixed, not
/// configurable: no real vocabulary comes near it.
pub const MAX_NAME_LEN: usize = 4096;

/// Hard ceiling on the nesting depth, whatever `SOLI_MAX_XML_DEPTH` says.
/// quick-xml counts open elements in a `u16`, and the tree walkers that are
/// not iterative recurse at most this deep. [`super::EventLoop::new`] clamps
/// any policy to it, so a hand-built policy cannot overflow quick-xml.
pub const MAX_DEPTH_CEILING: usize = 10_000;

/// Default cap on the namespace bindings live at one point of a document
/// (every `xmlns*` of every open element, shadowed ones included). quick-xml
/// caps declarations per element (256) but not their sum down a deep path;
/// real vocabularies stay in the tens.
pub const MAX_NS_BINDINGS: usize = 4096;

/// quick-xml's default cap on the namespace declarations of one start tag,
/// kept for `Xml.parse`.
pub const QUICK_XML_NS_DECLS_PER_ELEMENT: usize =
    quick_xml::name::DEFAULT_MAX_DECLARATIONS_PER_ELEMENT;

/// What to do with `&name;` when `name` is not a predefined entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnUnknownEntity {
    /// Refuse the document.
    Error,
    /// Drop the reference, keep parsing (today's `SOAP.parse`).
    Drop,
}

/// Refuse the construct, or accept it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    Error,
    Allow,
}

/// What to do with content after the root element closes (a second root,
/// non-whitespace text), and with non-whitespace text before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailingContent {
    /// Refuse the document.
    Error,
    /// Skip it: a second root and its subtree are read (so they must still be
    /// well-formed up to the end of the input) but kept out of the tree.
    /// Comments and PIs after the root are still kept.
    Ignore,
}

/// What an input without a root element gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnNoRoot {
    /// Raise [`super::XmlErrorKind::NoRoot`] with this message. An input that
    /// ends with its first root still open raises the same error (the c14n
    /// legacy shape: no root element ever *closed*).
    Error(&'static str),
    /// Raise `NoRoot` for an empty input, but an unclosed root raises
    /// `Unclosed` (the `Xml.parse` shape).
    ErrorUnclosedSeparately(&'static str),
    /// Give a tree with no root (SOAP: empty input or plain text is `nil`).
    Empty,
}

/// The knobs of one parse. `Copy`, so a streaming reader can own one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsePolicy {
    /// The message of the DOCTYPE refusal. DOCTYPE is always refused.
    pub doctype_message: &'static str,
    pub unknown_entity: OnUnknownEntity,
    pub unbound_prefix: Strictness,
    /// Two attributes with the same expanded name on one element.
    pub duplicate_attribute: Strictness,
    pub trailing_content: TrailingContent,
    pub no_root: OnNoRoot,
    /// Element nesting limit: depth `max_depth` parses, `max_depth + 1` fails.
    pub max_depth: usize,
    /// Node limit of the tree (document node excluded).
    pub max_nodes: usize,
    /// Input size limit, in bytes (after a BOM).
    pub max_bytes: usize,
    /// Attributes per element (namespace declarations excluded).
    pub max_attrs: usize,
    /// Most text one element may hold directly, in bytes: its text,
    /// references and CDATA added up across comments, PIs and child elements
    /// (what a projection joins into one `_text`). Text outside the root
    /// counts per run. `None`: bounded by `max_bytes` only.
    pub max_text_run: Option<usize>,
    /// Namespace bindings live at one point (see [`MAX_NS_BINDINGS`]).
    pub max_ns_bindings: usize,
    /// Keep whitespace-only text nodes inside the root (`false` is
    /// `strip_blank: true`).
    pub keep_blank_text: bool,
    /// Well-formedness checks quick-xml leaves to its caller: names must be
    /// QNames, comments may not hold `--`, literal forbidden characters in
    /// text and attribute values are refused, the XML declaration must come
    /// first, a PI target may not be `xml`, and `xmlns:p=""` is refused.
    /// On for `Xml.parse`, off where today's lenient parsers must not start
    /// refusing payloads they accepted (SOAP, c14n).
    pub strict_syntax: bool,
    /// Keep a processing instruction's data exactly as quick-xml gives it,
    /// leading whitespace included (`<?p  d?>` has data `"  d"`). The c14n
    /// trio formats PIs from that raw content today; everyone else gets the
    /// data with leading whitespace stripped (`"d"`).
    pub raw_pi_data: bool,
    /// Never refuse a document over an attribute (SOAP, which drops
    /// attributes anyway): an attribute whose value does not decode (`&`
    /// alone, `&#0;`) or whose name is too long is skipped, a syntax error
    /// stops reading the tag's attributes (the ones before it are kept),
    /// and attributes past `max_attrs` are dropped. Namespace declarations
    /// are read best effort under the same rules.
    pub lenient_attributes: bool,
    /// Namespace declarations one start tag may make (quick-xml's
    /// per-element cap, RUSTSEC-2026-0195). `max_ns_bindings` still caps the
    /// declarations live at once, so a lenient caller can lift this one to
    /// keep accepting what its old parser accepted.
    pub max_ns_decls_per_element: usize,
    /// Character references resolve by quick-xml's own rule, as the old SOAP
    /// and c14n parsers did: any code point but NUL, controls and U+FFFE
    /// included. A reference that does not resolve (`&#0;`, a surrogate, past
    /// U+10FFFF, a sign, not a number) is then handled like an unknown entity:
    /// dropped under [`OnUnknownEntity::Drop`], refused under `Error`. Off
    /// (`Xml.parse`): only characters XML allows, every other one refused.
    pub legacy_char_refs: bool,
    /// Duplicate attributes are compared by qname as written (the old c14n
    /// parser), not by expanded name: `p:x` and `q:x` with `p` and `q` bound
    /// to one URI, or an unbound `p:x` beside `x`, are two attributes.
    pub duplicate_by_qname: bool,
    /// The depth limit is checked on start tags only, as the old SOAP and
    /// c14n parsers did: a self-closing element directly inside an element at
    /// `max_depth` is accepted (it cannot nest, so one level past the limit
    /// is the most this allows).
    pub self_closing_past_depth: bool,
    /// Stop reading when the first root element closes (`SOAP.parse`, which
    /// never looked past it): what follows, well-formed or not, is not read.
    /// Wins over `trailing_content`.
    pub stop_after_root: bool,
    /// Non-whitespace text (and references) before the root element is given
    /// to the sink as text of the document node instead of being ignored.
    /// `SOAP.parse` read it as the start of the root's text (`junk<a>x</a>`
    /// gave `"junkx"`); `soap_shape` does the same with it. Only meaningful
    /// with `TrailingContent::Ignore`.
    pub keep_text_before_root: bool,
}

impl ParsePolicy {
    /// `Xml.parse` / `Xml.parse_file`: strict, limits from `SOLI_MAX_XML_*`.
    pub fn xml() -> Self {
        ParsePolicy {
            doctype_message: "DOCTYPE declarations are not allowed in XML documents",
            unknown_entity: OnUnknownEntity::Error,
            unbound_prefix: Strictness::Error,
            duplicate_attribute: Strictness::Error,
            trailing_content: TrailingContent::Error,
            no_root: OnNoRoot::ErrorUnclosedSeparately("no root element"),
            max_depth: env_depth(),
            max_nodes: env_usize(limits::max_xml_nodes()),
            max_bytes: env_usize(limits::max_xml_bytes()),
            max_attrs: env_usize(limits::max_xml_attrs()),
            max_text_run: None,
            max_ns_bindings: MAX_NS_BINDINGS,
            keep_blank_text: true,
            strict_syntax: true,
            raw_pi_data: false,
            lenient_attributes: false,
            max_ns_decls_per_element: QUICK_XML_NS_DECLS_PER_ELEMENT,
            legacy_char_refs: false,
            duplicate_by_qname: false,
            self_closing_past_depth: false,
            stop_after_root: false,
            keep_text_before_root: false,
        }
    }

    /// `SOAP.parse` / `SOAP.call`: today's lenient parser, with depth 64 and a
    /// 1 MiB text run, and truncated input now refused (bug 6). The legacy
    /// knobs keep what the old quick-xml loop accepted: it read no further
    /// than the root, kept text before it, resolved character references the
    /// quick-xml way and checked depth on start tags only.
    pub fn soap() -> Self {
        ParsePolicy {
            doctype_message:
                "XML parsing error: DOCTYPE declarations are not allowed in SOAP payloads",
            unknown_entity: OnUnknownEntity::Drop,
            unbound_prefix: Strictness::Allow,
            duplicate_attribute: Strictness::Allow,
            trailing_content: TrailingContent::Ignore,
            no_root: OnNoRoot::Empty,
            max_depth: 64.min(env_depth()),
            max_nodes: env_usize(limits::max_xml_nodes()),
            max_bytes: env_usize(limits::max_xml_bytes()),
            max_attrs: env_usize(limits::max_xml_attrs()),
            max_text_run: Some(1024 * 1024),
            max_ns_bindings: MAX_NS_BINDINGS,
            keep_blank_text: true,
            strict_syntax: false,
            raw_pi_data: false,
            lenient_attributes: true,
            max_ns_decls_per_element: MAX_NS_BINDINGS,
            legacy_char_refs: true,
            duplicate_by_qname: false,
            self_closing_past_depth: true,
            stop_after_root: true,
            keep_text_before_root: true,
        }
    }

    /// The c14n trio (`Xml.c14n_exclusive`, `get_element_by_id`,
    /// `get_elements_by_tag`): byte-for-byte today's behaviour, error strings
    /// included.
    pub fn c14n() -> Self {
        ParsePolicy {
            doctype_message:
                "DOCTYPE declarations are not allowed (XXE / entity-expansion defense)",
            unknown_entity: OnUnknownEntity::Error,
            unbound_prefix: Strictness::Allow,
            duplicate_attribute: Strictness::Error,
            trailing_content: TrailingContent::Ignore,
            no_root: OnNoRoot::Error("no document element found"),
            max_depth: 256.min(MAX_DEPTH_CEILING),
            max_nodes: env_usize(limits::max_xml_nodes()),
            max_bytes: env_usize(limits::max_xml_bytes()),
            max_attrs: env_usize(limits::max_xml_attrs()),
            max_text_run: None,
            max_ns_bindings: MAX_NS_BINDINGS,
            keep_blank_text: true,
            strict_syntax: false,
            raw_pi_data: true,
            lenient_attributes: false,
            max_ns_decls_per_element: MAX_NS_BINDINGS,
            legacy_char_refs: true,
            duplicate_by_qname: true,
            self_closing_past_depth: true,
            stop_after_root: false,
            keep_text_before_root: false,
        }
    }

    /// Apply per-call options (`Xml.parse(s, max_depth: 32, …)`). An option
    /// can only **lower** a limit: a larger value is ignored, never an error,
    /// so the environment stays the ceiling.
    pub fn lowered(
        mut self,
        max_depth: Option<usize>,
        max_nodes: Option<usize>,
        max_bytes: Option<usize>,
    ) -> Self {
        if let Some(depth) = max_depth {
            self.max_depth = self.max_depth.min(depth);
        }
        if let Some(nodes) = max_nodes {
            self.max_nodes = self.max_nodes.min(nodes);
        }
        if let Some(bytes) = max_bytes {
            self.max_bytes = self.max_bytes.min(bytes);
        }
        self
    }

    /// `strip_blank: true`: whitespace-only text nodes are not kept.
    pub fn strip_blank(mut self, strip: bool) -> Self {
        self.keep_blank_text = !strip;
        self
    }
}

fn env_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn env_depth() -> usize {
    env_usize(limits::max_xml_depth()).min(MAX_DEPTH_CEILING)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_call_options_only_lower_limits() {
        let base = ParsePolicy::xml();
        let lowered = base.lowered(Some(3), Some(10), Some(100));
        assert_eq!(lowered.max_depth, 3.min(base.max_depth));
        assert_eq!(lowered.max_nodes, 10.min(base.max_nodes));
        assert_eq!(lowered.max_bytes, 100.min(base.max_bytes));
        let raised = base.lowered(Some(usize::MAX), Some(usize::MAX), Some(usize::MAX));
        assert_eq!(raised, base);
    }

    #[test]
    fn the_three_callers_differ_as_specified() {
        let (xml, soap, c14n) = (ParsePolicy::xml(), ParsePolicy::soap(), ParsePolicy::c14n());
        assert_eq!(soap.unknown_entity, OnUnknownEntity::Drop);
        assert_eq!(xml.unknown_entity, OnUnknownEntity::Error);
        assert_eq!(c14n.unknown_entity, OnUnknownEntity::Error);
        assert_eq!(c14n.unbound_prefix, Strictness::Allow);
        assert_eq!(soap.duplicate_attribute, Strictness::Allow);
        assert_eq!(c14n.duplicate_attribute, Strictness::Error);
        assert_eq!(soap.trailing_content, TrailingContent::Ignore);
        assert_eq!(xml.trailing_content, TrailingContent::Error);
        assert!(soap.max_depth <= 64);
        assert_eq!(c14n.max_depth, 256);
        assert_eq!(soap.max_text_run, Some(1024 * 1024));
        assert!(soap.lenient_attributes && !xml.lenient_attributes && !c14n.lenient_attributes);
        assert_eq!(xml.max_ns_bindings, MAX_NS_BINDINGS);
        // The legacy knobs: on where the old parsers behaved that way.
        assert!(!xml.legacy_char_refs && soap.legacy_char_refs && c14n.legacy_char_refs);
        assert!(soap.stop_after_root && !c14n.stop_after_root && !xml.stop_after_root);
        assert!(soap.keep_text_before_root && !c14n.keep_text_before_root);
        assert!(c14n.duplicate_by_qname && !soap.duplicate_by_qname && !xml.duplicate_by_qname);
        assert!(soap.self_closing_past_depth && c14n.self_closing_past_depth);
        assert!(!xml.self_closing_past_depth);
        assert_eq!(xml.max_ns_decls_per_element, 256);
        assert_eq!(soap.max_ns_decls_per_element, MAX_NS_BINDINGS);
        assert!(xml.max_depth <= MAX_DEPTH_CEILING);
        assert!(!ParsePolicy::xml().strip_blank(true).keep_blank_text);
        assert_eq!(
            soap.doctype_message,
            "XML parsing error: DOCTYPE declarations are not allowed in SOAP payloads"
        );
        assert_eq!(
            c14n.doctype_message,
            "DOCTYPE declarations are not allowed (XXE / entity-expansion defense)"
        );
    }
}
