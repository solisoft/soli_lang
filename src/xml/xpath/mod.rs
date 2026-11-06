//! The XPath subset behind `find`, `find_all`, `xpath` and `at_xpath`: a
//! tokenizer, a parser and an evaluator over [`XmlTree`], in plain Rust.
//!
//! # Grammar
//!
//! ```text
//! Expr      := Path ('|' Path)*
//! Path      := '/' RelPath? | '//' RelPath | RelPath
//! RelPath   := Step (('/' | '//') Step)*
//! Step      := '.' | '..' | '@' AttrTest | NodeTest Pred*
//! NodeTest  := QName | '*' | NCName ':*' | '*:' NCName
//!            | 'text()' | 'node()' | 'comment()' | 'processing-instruction(' Literal? ')'
//! AttrTest  := QName | '*' | NCName ':*' | '*:' NCName
//! Pred      := '[' (Integer | 'last()' | OrExpr) ']'
//! OrExpr    := AndExpr ('or' AndExpr)*
//! AndExpr   := Test ('and' Test)*
//! Test      := 'not(' OrExpr ')' | '(' OrExpr ')'
//!            | Operand (('=' | '!=') Literal)?
//!            | 'contains(' Operand ',' Literal ')' | 'starts-with(' Operand ',' Literal ')'
//!            | 'position()' ('=' | '!=' | '<' | '>' | '<=' | '>=') Integer
//! Operand   := '@' AttrTest | NameTest | 'text()' | '.'      # one child step only
//! Literal   := "'" … "'" | '"' … '"'
//! ```
//!
//! Everything else (explicit `axis::`, arithmetic, variables, numbers other
//! than `[n]` and `position()`, nested paths inside a predicate, any other
//! function) is refused when the expression is compiled, naming the column.
//! `@attr` and `text()` may only end a path.
//!
//! # Semantics
//!
//! XPath 1.0, on the child, attribute, parent, self and
//! descendant-or-self axes (`a//b` is `a/descendant-or-self::node()/b`, so
//! `//item[1]` is every `item` that is the first `item` of its parent).
//! `[n]`, `last()` and `position()` count within the set one step selects
//! from one context node, after the predicates before them. `=` and `!=` are
//! existential string comparisons; `contains` and `starts-with` read the
//! first value of their operand (none = `""`). Results come in document
//! order, without duplicates, across `|` too.
//!
//! Two [`Flavour`]s share it all and differ in two ways only:
//!
//! | | [`Flavour::Lenient`] (`find`) | [`Flavour::Strict`] (`xpath`) |
//! |---|---|---|
//! | relative path from the document node | starts at the root element | starts at the document node |
//! | unprefixed element test | any namespace (or the `""` key's URI) | no namespace (or the `""` key's URI) |
//!
//! Unprefixed **attribute** tests match attributes in no namespace in both.
//! A prefix resolves through the caller's table, then the root element's
//! bindings, then `xmlns` stands for the root's default namespace; anything
//! else is [`XPathErrorKind::UnboundPrefix`]. `xml` is always bound.
//!
//! # Cost
//!
//! A step reads each context node's children (or, after `//`, its subtree)
//! once; a bitset keeps any node from entering a set twice. Predicates read
//! attributes and direct children, `.` reads the subtree. Every node read is
//! a *visit*, and an evaluation that goes past [`Query::max_visits`] stops
//! with [`XPathErrorKind::Budget`]. The parser recurses only into
//! `not(…)`/`(…)`, at most [`MAX_NESTING`] deep; the evaluator never recurses
//! per tree level.

mod eval;
mod lex;
mod parse;
#[cfg(test)]
mod tests;

use std::fmt;

use super::tree::{Kind, NodeId, XmlTree};

/// Longest expression accepted, in bytes.
pub const MAX_EXPR_BYTES: usize = 4096;
/// Most steps in one expression, every `|` branch together.
pub const MAX_STEPS: usize = 64;
/// Most predicates on one step.
pub const MAX_PREDICATES: usize = 8;
/// Most tests (operands of `and`/`or`, `not`, comparisons) in one predicate.
pub const MAX_TERMS: usize = 16;
/// Deepest `not(…)` / `(…)` nesting in one predicate.
pub const MAX_NESTING: usize = 16;

/// Which method family an expression is evaluated for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flavour {
    /// `find` / `find_all`: ElementTree-like.
    Lenient,
    /// `xpath` / `at_xpath`: XPath 1.0.
    Strict,
}

/// Why an expression was refused or its evaluation stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XPathErrorKind {
    /// Not XPath at all (`a[`, an unterminated literal).
    Invalid { column: usize, message: String },
    /// XPath, but outside the subset (`sum(…)`, `child::a`, `$x`).
    Unsupported { column: usize, message: String },
    /// One of the expression limits (length, steps, predicates, terms,
    /// nesting).
    Limit(String),
    /// A prefix no table entry or root binding resolves.
    UnboundPrefix(String),
    /// More node visits than the budget.
    Budget(u64),
    /// The context node is not a node of the tree.
    BadContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XPathError {
    pub kind: XPathErrorKind,
}

impl XPathError {
    pub(crate) fn invalid(column: usize, message: impl Into<String>) -> Self {
        XPathError {
            kind: XPathErrorKind::Invalid {
                column,
                message: message.into(),
            },
        }
    }

    pub(crate) fn unsupported(column: usize, message: impl Into<String>) -> Self {
        XPathError {
            kind: XPathErrorKind::Unsupported {
                column,
                message: message.into(),
            },
        }
    }

    pub(crate) fn limit(message: impl Into<String>) -> Self {
        XPathError {
            kind: XPathErrorKind::Limit(message.into()),
        }
    }
}

impl fmt::Display for XPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            XPathErrorKind::Invalid { column, message } => {
                write!(f, "invalid XPath at column {column}: {message}")
            }
            XPathErrorKind::Unsupported { column, message } => {
                write!(f, "unsupported XPath at column {column}: {message}")
            }
            XPathErrorKind::Limit(message) => write!(f, "XPath too complex: {message}"),
            XPathErrorKind::UnboundPrefix(prefix) => write!(f, "unbound prefix '{prefix}'"),
            XPathErrorKind::Budget(limit) => write!(
                f,
                "evaluation exceeded {limit} node visits (SOLI_MAX_XPATH_VISITS)"
            ),
            XPathErrorKind::BadContext => write!(f, "the node does not belong to its tree"),
        }
    }
}

impl std::error::Error for XPathError {}

/// A compiled expression. It does not depend on the flavour or the
/// namespace table, so one compilation serves every call.
#[derive(Debug, Clone)]
pub struct XPath {
    paths: Vec<parse::Path>,
    /// Every prefix the expression uses, once each, in first-use order.
    prefixes: Vec<String>,
}

impl XPath {
    /// Compile `src`, or say (with a column) why it is refused.
    pub fn compile(src: &str) -> Result<XPath, XPathError> {
        let tokens = lex::lex(src)?;
        let paths = parse::parse(&tokens)?;
        let mut prefixes = Vec::new();
        for path in &paths {
            path.collect_prefixes(&mut prefixes);
        }
        Ok(XPath { paths, prefixes })
    }

    /// The prefixes the expression uses.
    pub fn prefixes(&self) -> &[String] {
        &self.prefixes
    }
}

/// How to evaluate: the flavour, the caller's prefix table (`""` is the
/// default namespace for unprefixed element tests) and the visit budget.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    pub flavour: Flavour,
    pub namespaces: &'a [(String, String)],
    pub max_visits: u64,
}

/// One result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    /// A node of any kind (elements, `node()`, `comment()`, `..`, `/`).
    Node(NodeId),
    /// The `index`-th attribute of `element` (an `@attr` step): its value.
    Attr { element: NodeId, index: usize },
    /// A text or CDATA node selected by a final `text()` step: its content.
    Text(NodeId),
}

impl Item {
    /// The string an `@attr` or `text()` result stands for, `None` for a
    /// node result.
    pub fn string(&self, tree: &XmlTree) -> Option<String> {
        match *self {
            Item::Node(_) => None,
            Item::Attr { element, index } => tree
                .element(element)
                .and_then(|el| el.attrs.get(index))
                .map(|a| a.value.to_string()),
            Item::Text(id) => match tree.get(id).map(|n| &n.kind) {
                Some(Kind::Text(s)) | Some(Kind::CData(s)) => Some(s.to_string()),
                _ => Some(String::new()),
            },
        }
    }
}

/// Evaluate `xpath` from `context`, in document order without duplicates.
/// Renumbers the tree's document order first if an edit made it stale.
pub fn select(
    tree: &mut XmlTree,
    context: NodeId,
    xpath: &XPath,
    query: &Query<'_>,
) -> Result<Vec<Item>, XPathError> {
    if tree.get(context).is_none() {
        return Err(XPathError {
            kind: XPathErrorKind::BadContext,
        });
    }
    tree.ensure_order();
    eval::evaluate(tree, context, xpath, query)
}
