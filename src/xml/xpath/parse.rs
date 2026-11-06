//! The XPath parser: tokens to a small AST, with every subset rule and
//! expression limit checked here, so evaluation never meets a construct it
//! does not handle. Recursion happens only for `not(…)` and `(…)`, capped at
//! [`MAX_NESTING`].

use super::lex::{Tok, Token};
use super::{XPathError, MAX_NESTING, MAX_PREDICATES, MAX_STEPS, MAX_TERMS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Start {
    /// From the context node (or, for `find` on a document, the root element).
    Relative,
    /// `/…` and `//…`: from the root of the context node's tree.
    Root,
}

#[derive(Debug, Clone)]
pub(crate) struct Path {
    pub start: Start,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone)]
pub(crate) enum Step {
    /// `.`
    SelfNode,
    /// `..`
    Parent,
    /// What `//` puts between two steps: descendant-or-self::node().
    DescOrSelf,
    /// `@test` (a final step).
    Attr(NameTest),
    /// A child step with its predicates.
    Child(NodeTest, Vec<Pred>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NameTest {
    /// `*`
    Any,
    /// `name` or `p:name`
    Name {
        prefix: Option<String>,
        local: String,
    },
    /// `p:*`
    Prefix(String),
    /// `*:name`
    Local(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NodeTest {
    Name(NameTest),
    /// `text()`: text and CDATA nodes.
    Text,
    /// `node()`: any child.
    Node,
    /// `comment()`
    Comment,
    /// `processing-instruction()` or `processing-instruction('target')`
    Pi(Option<String>),
}

#[derive(Debug, Clone)]
pub(crate) enum Pred {
    /// `[n]`, 1-based.
    Index(u64),
    /// `[last()]`
    Last,
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn holds(self, left: u64, right: u64) -> bool {
        match self {
            CmpOp::Eq => left == right,
            CmpOp::Ne => left != right,
            CmpOp::Lt => left < right,
            CmpOp::Le => left <= right,
            CmpOp::Gt => left > right,
            CmpOp::Ge => left >= right,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Expr {
    Or(Vec<Expr>),
    And(Vec<Expr>),
    Not(Box<Expr>),
    /// A bare operand: does it select anything?
    Exists(Operand),
    /// `operand = 'lit'` (`eq`) or `operand != 'lit'`.
    Cmp {
        operand: Operand,
        eq: bool,
        literal: String,
    },
    Contains(Operand, String),
    StartsWith(Operand, String),
    Position(CmpOp, u64),
}

#[derive(Debug, Clone)]
pub(crate) enum Operand {
    /// `@a`: the attributes matching.
    Attr(NameTest),
    /// `tag`: the element children matching; their string-values.
    Child(NameTest),
    /// `text()`: the direct text and CDATA children.
    Text,
    /// `.`: the node's own string-value.
    Dot,
}

impl NameTest {
    fn prefix(&self) -> Option<&str> {
        match self {
            NameTest::Name {
                prefix: Some(p), ..
            }
            | NameTest::Prefix(p) => Some(p),
            _ => None,
        }
    }
}

fn add_prefix(test: &NameTest, out: &mut Vec<String>) {
    if let Some(p) = test.prefix() {
        if !out.iter().any(|q| q == p) {
            out.push(p.to_string());
        }
    }
}

impl Operand {
    fn collect_prefixes(&self, out: &mut Vec<String>) {
        if let Operand::Attr(test) | Operand::Child(test) = self {
            add_prefix(test, out);
        }
    }
}

impl Expr {
    fn collect_prefixes(&self, out: &mut Vec<String>) {
        match self {
            Expr::Or(items) | Expr::And(items) => {
                for item in items {
                    item.collect_prefixes(out);
                }
            }
            Expr::Not(inner) => inner.collect_prefixes(out),
            Expr::Exists(op)
            | Expr::Cmp { operand: op, .. }
            | Expr::Contains(op, _)
            | Expr::StartsWith(op, _) => op.collect_prefixes(out),
            Expr::Position(..) => {}
        }
    }
}

impl Path {
    pub fn collect_prefixes(&self, out: &mut Vec<String>) {
        for step in &self.steps {
            match step {
                Step::Attr(test) => add_prefix(test, out),
                Step::Child(test, preds) => {
                    if let NodeTest::Name(name) = test {
                        add_prefix(name, out);
                    }
                    for pred in preds {
                        if let Pred::Expr(expr) = pred {
                            expr.collect_prefixes(out);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    steps: usize,
    /// Tests in the predicate being parsed.
    terms: usize,
    /// `not(…)` / `(…)` depth in the predicate being parsed.
    depth: usize,
}

/// Parse a whole expression: one or more `|`-separated paths.
pub(crate) fn parse(tokens: &[Token]) -> Result<Vec<Path>, XPathError> {
    let mut p = Parser {
        tokens,
        pos: 0,
        steps: 0,
        terms: 0,
        depth: 0,
    };
    let mut paths = vec![p.path()?];
    while *p.peek() == Tok::Pipe {
        p.bump();
        paths.push(p.path()?);
    }
    if *p.peek() != Tok::End {
        return Err(p.unexpected());
    }
    Ok(paths)
}

fn name_is(tok: &Tok, name: &str) -> bool {
    matches!(tok, Tok::Name { prefix: None, local } if local == name)
}

impl<'a> Parser<'a> {
    fn token(&self, offset: usize) -> &'a Token {
        // The lexer always ends the list with `End`; clamp to it.
        let last = self.tokens.len().saturating_sub(1);
        &self.tokens[(self.pos + offset).min(last)]
    }

    fn peek(&self) -> &'a Tok {
        &self.token(0).tok
    }

    fn peek_at(&self, offset: usize) -> &'a Tok {
        &self.token(offset).tok
    }

    fn col(&self) -> usize {
        self.token(0).col
    }

    fn bump(&mut self) {
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
    }

    fn eat(&mut self, tok: Tok, what: &str) -> Result<(), XPathError> {
        if *self.peek() == tok {
            self.bump();
            Ok(())
        } else if matches!(self.peek(), Tok::End) {
            Err(XPathError::invalid(
                self.col(),
                format!("expected {what}, found the end of the expression"),
            ))
        } else {
            Err(match self.unsupported_here() {
                Some(err) => err,
                None => XPathError::invalid(
                    self.col(),
                    format!("expected {what}, found {}", describe(self.peek())),
                ),
            })
        }
    }

    /// The error for a token XPath has but the subset does not, if it is one.
    fn unsupported_here(&self) -> Option<XPathError> {
        let col = self.col();
        let message = match self.peek() {
            Tok::Axis(axis) => format!("axis '{axis}::'"),
            Tok::Variable(name) => format!("variable '${name}'"),
            Tok::Other('+') | Tok::Other('-') => "arithmetic".to_string(),
            Tok::Name {
                prefix: None,
                local,
            } if ["div", "mod"].contains(&local.as_str()) && self.pos > 0 => {
                "arithmetic".to_string()
            }
            Tok::Name { prefix, local } if *self.peek_at(1) == Tok::LParen => {
                let name = match prefix {
                    Some(p) => format!("{p}:{local}"),
                    None => local.clone(),
                };
                format!("function '{name}'")
            }
            Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge => {
                "numeric comparison (only position() compares numbers)".to_string()
            }
            Tok::Integer(_) | Tok::Decimal => "a number here".to_string(),
            _ => return None,
        };
        Some(XPathError::unsupported(col, message))
    }

    fn unexpected(&self) -> XPathError {
        if let Some(err) = self.unsupported_here() {
            return err;
        }
        match self.peek() {
            Tok::End => XPathError::invalid(self.col(), "unexpected end of the expression"),
            other => XPathError::invalid(self.col(), format!("unexpected {}", describe(other))),
        }
    }

    fn count_step(&mut self) -> Result<(), XPathError> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(XPathError::limit(format!("more than {MAX_STEPS} steps")));
        }
        Ok(())
    }

    fn path(&mut self) -> Result<Path, XPathError> {
        match self.peek() {
            Tok::LParen => Err(XPathError::unsupported(
                self.col(),
                "a parenthesised path: for (//a)[1] write at_xpath(\"//a\")",
            )),
            Tok::Slash => {
                self.bump();
                let mut steps = Vec::new();
                if matches!(self.peek(), Tok::End | Tok::Pipe) {
                    // A bare `/` counts as a step, so `/|/|…` is bounded too.
                    self.count_step()?;
                } else {
                    self.rel_path(&mut steps)?;
                }
                Ok(Path {
                    start: Start::Root,
                    steps,
                })
            }
            Tok::DSlash => {
                self.bump();
                let mut steps = vec![Step::DescOrSelf];
                self.rel_path(&mut steps)?;
                Ok(Path {
                    start: Start::Root,
                    steps,
                })
            }
            _ => {
                let mut steps = Vec::new();
                self.rel_path(&mut steps)?;
                Ok(Path {
                    start: Start::Relative,
                    steps,
                })
            }
        }
    }

    fn rel_path(&mut self, steps: &mut Vec<Step>) -> Result<(), XPathError> {
        loop {
            let step = self.step()?;
            self.count_step()?;
            let last_only = match &step {
                Step::Attr(_) => Some("'@attribute'"),
                Step::Child(NodeTest::Text, _) => Some("'text()'"),
                _ => None,
            };
            steps.push(step);
            let descend = match self.peek() {
                Tok::Slash => false,
                Tok::DSlash => true,
                _ => return Ok(()),
            };
            if let Some(what) = last_only {
                return Err(XPathError::unsupported(
                    self.col(),
                    format!("{what} must be the last step of a path"),
                ));
            }
            self.bump();
            if descend {
                steps.push(Step::DescOrSelf);
            }
        }
    }

    fn no_predicate(&self, on: &str) -> Result<(), XPathError> {
        if *self.peek() == Tok::LBracket {
            return Err(XPathError::unsupported(
                self.col(),
                format!("a predicate on {on}"),
            ));
        }
        Ok(())
    }

    fn step(&mut self) -> Result<Step, XPathError> {
        match self.peek() {
            Tok::Dot => {
                self.bump();
                self.no_predicate("'.'")?;
                Ok(Step::SelfNode)
            }
            Tok::DDot => {
                self.bump();
                self.no_predicate("'..'")?;
                Ok(Step::Parent)
            }
            Tok::At => {
                self.bump();
                let test = self.attr_test()?;
                self.no_predicate("an attribute step")?;
                Ok(Step::Attr(test))
            }
            _ => {
                let test = self.node_test()?;
                let mut preds = Vec::new();
                while *self.peek() == Tok::LBracket {
                    if preds.len() == MAX_PREDICATES {
                        return Err(XPathError::limit(format!(
                            "more than {MAX_PREDICATES} predicates on one step"
                        )));
                    }
                    preds.push(self.predicate()?);
                }
                Ok(Step::Child(test, preds))
            }
        }
    }

    /// A name test without parentheses after it, or `None`.
    fn name_test(&mut self) -> Option<NameTest> {
        let test = match self.peek() {
            Tok::Star => NameTest::Any,
            Tok::PrefixStar(p) => NameTest::Prefix(p.clone()),
            Tok::StarLocal(l) => NameTest::Local(l.clone()),
            Tok::Name { prefix, local } if *self.peek_at(1) != Tok::LParen => NameTest::Name {
                prefix: prefix.clone(),
                local: local.clone(),
            },
            _ => return None,
        };
        self.bump();
        Some(test)
    }

    fn attr_test(&mut self) -> Result<NameTest, XPathError> {
        match self.name_test() {
            Some(test) => Ok(test),
            None => match self.unsupported_here() {
                Some(err) => Err(err),
                None => Err(XPathError::invalid(
                    self.col(),
                    "expected an attribute name after '@'",
                )),
            },
        }
    }

    fn node_test(&mut self) -> Result<NodeTest, XPathError> {
        if let Some(test) = self.name_test() {
            return Ok(NodeTest::Name(test));
        }
        let Tok::Name {
            prefix: None,
            local,
        } = self.peek()
        else {
            return Err(self.unexpected());
        };
        let test = match local.as_str() {
            "text" => NodeTest::Text,
            "node" => NodeTest::Node,
            "comment" => NodeTest::Comment,
            "processing-instruction" => NodeTest::Pi(None),
            _ => return Err(self.unexpected()),
        };
        self.bump(); // the name
        self.bump(); // `(`
        let test = match (test, self.peek()) {
            (NodeTest::Pi(None), Tok::Literal(target)) => {
                let target = target.clone();
                self.bump();
                NodeTest::Pi(Some(target))
            }
            (test, _) => test,
        };
        self.eat(Tok::RParen, "')'")?;
        Ok(test)
    }

    fn predicate(&mut self) -> Result<Pred, XPathError> {
        self.bump(); // `[`
        if let (Tok::Integer(digits), Tok::RBracket) = (self.peek(), self.peek_at(1)) {
            let n = digits.parse::<u64>().unwrap_or(u64::MAX);
            self.bump();
            self.bump();
            return Ok(Pred::Index(n));
        }
        if name_is(self.peek(), "last")
            && *self.peek_at(1) == Tok::LParen
            && *self.peek_at(2) == Tok::RParen
            && *self.peek_at(3) == Tok::RBracket
        {
            for _ in 0..4 {
                self.bump();
            }
            return Ok(Pred::Last);
        }
        self.terms = 0;
        self.depth = 0;
        let expr = self.or_expr()?;
        self.eat(Tok::RBracket, "']'")?;
        Ok(Pred::Expr(expr))
    }

    fn or_expr(&mut self) -> Result<Expr, XPathError> {
        let mut items = vec![self.and_expr()?];
        while name_is(self.peek(), "or") {
            self.bump();
            items.push(self.and_expr()?);
        }
        Ok(if items.len() == 1 {
            items.swap_remove(0)
        } else {
            Expr::Or(items)
        })
    }

    fn and_expr(&mut self) -> Result<Expr, XPathError> {
        let mut items = vec![self.test()?];
        while name_is(self.peek(), "and") {
            self.bump();
            items.push(self.test()?);
        }
        Ok(if items.len() == 1 {
            items.swap_remove(0)
        } else {
            Expr::And(items)
        })
    }

    fn nested(&mut self) -> Result<Expr, XPathError> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(XPathError::limit(format!(
                "not(…) or (…) nested more than {MAX_NESTING} deep"
            )));
        }
        let expr = self.or_expr()?;
        self.eat(Tok::RParen, "')'")?;
        self.depth -= 1;
        Ok(expr)
    }

    fn literal(&mut self) -> Result<String, XPathError> {
        match self.peek() {
            Tok::Literal(text) => {
                let text = text.clone();
                self.bump();
                Ok(text)
            }
            Tok::Integer(_) | Tok::Decimal => Err(XPathError::unsupported(
                self.col(),
                "numeric comparison: quote the value to compare it as a string ('1')",
            )),
            _ => Err(XPathError::invalid(
                self.col(),
                format!("expected a quoted string, found {}", describe(self.peek())),
            )),
        }
    }

    fn test(&mut self) -> Result<Expr, XPathError> {
        if *self.peek() == Tok::LParen {
            self.bump();
            return self.nested();
        }
        // `(…)` and `not(…)` only group: the tests inside them count.
        if !(name_is(self.peek(), "not") && *self.peek_at(1) == Tok::LParen) {
            self.terms += 1;
            if self.terms > MAX_TERMS {
                return Err(XPathError::limit(format!(
                    "more than {MAX_TERMS} tests in one predicate"
                )));
            }
        }
        if *self.peek_at(1) == Tok::LParen {
            if let Tok::Name {
                prefix: None,
                local,
            } = self.peek()
            {
                match local.as_str() {
                    "not" => {
                        self.bump();
                        self.bump();
                        return Ok(Expr::Not(Box::new(self.nested()?)));
                    }
                    "contains" | "starts-with" => {
                        let contains = local == "contains";
                        self.bump();
                        self.bump();
                        let operand = self.operand()?;
                        self.eat(Tok::Comma, "','")?;
                        let literal = self.literal()?;
                        self.eat(Tok::RParen, "')'")?;
                        return Ok(if contains {
                            Expr::Contains(operand, literal)
                        } else {
                            Expr::StartsWith(operand, literal)
                        });
                    }
                    "position" => {
                        self.bump();
                        self.bump();
                        self.eat(Tok::RParen, "')'")?;
                        return self.position();
                    }
                    "last" => {
                        return Err(XPathError::unsupported(
                            self.col(),
                            "last() inside an expression: write [last()] on its own",
                        ))
                    }
                    _ => {}
                }
            }
        }
        let operand = self.operand()?;
        let eq = match self.peek() {
            Tok::Eq => true,
            Tok::Ne => false,
            Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge => return Err(self.unexpected()),
            _ => return Ok(Expr::Exists(operand)),
        };
        self.bump();
        let literal = self.literal()?;
        Ok(Expr::Cmp {
            operand,
            eq,
            literal,
        })
    }

    fn position(&mut self) -> Result<Expr, XPathError> {
        let op = match self.peek() {
            Tok::Eq => CmpOp::Eq,
            Tok::Ne => CmpOp::Ne,
            Tok::Lt => CmpOp::Lt,
            Tok::Le => CmpOp::Le,
            Tok::Gt => CmpOp::Gt,
            Tok::Ge => CmpOp::Ge,
            _ => {
                return Err(XPathError::unsupported(
                    self.col(),
                    "position() must be compared with an integer (position() > 1)",
                ))
            }
        };
        self.bump();
        let Tok::Integer(digits) = self.peek() else {
            return Err(XPathError::unsupported(
                self.col(),
                "position() must be compared with an integer (position() > 1)",
            ));
        };
        let n = digits.parse::<u64>().unwrap_or(u64::MAX);
        self.bump();
        Ok(Expr::Position(op, n))
    }

    fn operand(&mut self) -> Result<Operand, XPathError> {
        let operand = match self.peek() {
            Tok::At => {
                self.bump();
                Operand::Attr(self.attr_test()?)
            }
            Tok::Dot => {
                self.bump();
                Operand::Dot
            }
            Tok::DDot => {
                return Err(XPathError::unsupported(
                    self.col(),
                    "'..' inside a predicate",
                ))
            }
            tok if name_is(tok, "text") && *self.peek_at(1) == Tok::LParen => {
                self.bump();
                self.bump();
                self.eat(Tok::RParen, "')'")?;
                Operand::Text
            }
            _ => match self.name_test() {
                Some(test) => Operand::Child(test),
                None => return Err(self.unexpected()),
            },
        };
        match self.peek() {
            Tok::Slash | Tok::DSlash => Err(XPathError::unsupported(
                self.col(),
                "a path inside a predicate (one child step or attribute only)",
            )),
            Tok::LBracket => Err(XPathError::unsupported(
                self.col(),
                "a predicate inside a predicate",
            )),
            _ => Ok(operand),
        }
    }
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Slash => "'/'".into(),
        Tok::DSlash => "'//'".into(),
        Tok::Pipe => "'|'".into(),
        Tok::Dot => "'.'".into(),
        Tok::DDot => "'..'".into(),
        Tok::At => "'@'".into(),
        Tok::Star => "'*'".into(),
        Tok::LBracket => "'['".into(),
        Tok::RBracket => "']'".into(),
        Tok::LParen => "'('".into(),
        Tok::RParen => "')'".into(),
        Tok::Comma => "','".into(),
        Tok::Eq => "'='".into(),
        Tok::Ne => "'!='".into(),
        Tok::Lt => "'<'".into(),
        Tok::Le => "'<='".into(),
        Tok::Gt => "'>'".into(),
        Tok::Ge => "'>='".into(),
        Tok::Name {
            prefix: Some(p),
            local,
        } => format!("'{p}:{local}'"),
        Tok::Name {
            prefix: None,
            local,
        } => format!("'{local}'"),
        Tok::PrefixStar(p) => format!("'{p}:*'"),
        Tok::StarLocal(l) => format!("'*:{l}'"),
        Tok::Literal(_) => "a string".into(),
        Tok::Integer(_) | Tok::Decimal => "a number".into(),
        Tok::Variable(name) => format!("'${name}'"),
        Tok::Axis(axis) => format!("'{axis}::'"),
        Tok::Other(c) => format!("'{c}'"),
        Tok::End => "the end of the expression".into(),
    }
}
