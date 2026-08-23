//! Core parser struct and helper methods.

use crate::ast::*;
use crate::error::ParserError;
use crate::lexer::{Token, TokenKind};
use crate::metrics::Metrics;
use crate::span::Span;
use std::time::Instant;

pub type ParseResult<T> = Result<T, ParserError>;

/// Maximum nesting depth for recursively-parsed constructs (expressions,
/// statements, patterns, types). Recursive descent on adversarial input
/// (`((((((...))))))`) would otherwise overflow the stack — which panics
/// without unwinding, so `catch_unwind` fault isolation cannot contain it.
/// 64 leaves headroom below the measured overflow threshold on 2 MB stacks
/// in debug builds (where per-level frames are largest); real-world source
/// nests well under half that.
pub(crate) const MAX_PARSE_DEPTH: usize = 64;

/// The parser for Solilang.
pub struct Parser {
    /// Current construct-nesting depth, guarded by [`MAX_PARSE_DEPTH`].
    pub(crate) depth: usize,
    pub(crate) tokens: Vec<Token>,
    pub(crate) current: usize,
    /// When true, trailing `{` blocks are NOT consumed after call expressions.
    /// Set while parsing if/while/for conditions to avoid stealing the statement body.
    pub(crate) no_trailing_brace: bool,
    /// Set while parsing an `if`/`while`/`elsif`/postfix condition. A condition
    /// ends at the end of its line: without this, `if (x < 0)` followed by a body
    /// line beginning `-x` parses as `if ((x < 0) - x)`. (Ruby's rule, and the
    /// same shape as the `rescue` and `[` line checks in `parse_precedence`.)
    pub(crate) condition_context: bool,
    /// When true, a trailing `do … end` block is NOT consumed by a call/member
    /// expression. Set while parsing command-style argument values so the block
    /// binds to the outer command call (`after_transition to: X do … end`)
    /// rather than the argument value (`X`).
    pub(crate) no_trailing_do: bool,
    /// When true, a `rescue` opening a new line ends the current statement so the
    /// enclosing `try`/`begin` body can treat it as a block-form catch clause rather
    /// than a postfix `rescue` modifier. Set only while parsing an end-form try body.
    pub(crate) in_try_body: bool,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            current: 0,
            depth: 0,
            no_trailing_brace: false,
            condition_context: false,
            no_trailing_do: false,
            in_try_body: false,
        }
    }

    /// Enter a nesting level for a recursive construct. Errors once
    /// [`MAX_PARSE_DEPTH`] is exceeded instead of overflowing the stack.
    /// Pair with [`Parser::exit_depth`] after the recursive call returns.
    pub(crate) fn enter_depth(&mut self, what: &str) -> ParseResult<()> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            self.depth -= 1;
            return Err(ParserError::general(
                format!("{what} nested too deeply (max depth {MAX_PARSE_DEPTH})"),
                self.current_span(),
            ));
        }
        Ok(())
    }

    pub(crate) fn exit_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Parse an expression with trailing brace blocks suppressed.
    /// Used for if/while/for conditions where `{` starts the statement body.
    pub(crate) fn expression_no_trailing_brace(&mut self) -> ParseResult<Expr> {
        let old = self.no_trailing_brace;
        self.no_trailing_brace = true;
        let result = self.expression();
        self.no_trailing_brace = old;
        result
    }

    /// Parse a condition: no trailing brace (that is the body), and no operator
    /// continuation onto the next line (that is the body too).
    pub(crate) fn expression_condition(&mut self) -> ParseResult<Expr> {
        let old_brace = self.no_trailing_brace;
        let old_cond = self.condition_context;
        self.no_trailing_brace = true;
        self.condition_context = true;
        let result = self.expression();
        self.no_trailing_brace = old_brace;
        self.condition_context = old_cond;
        result
    }

    /// Parse a condition that may span lines because a delimiter closes it —
    /// the postfix modifiers, where `stmt if cond` has no body to confuse it.
    pub(crate) fn expression_condition_inline(&mut self) -> ParseResult<Expr> {
        let old_cond = self.condition_context;
        self.condition_context = true;
        let result = self.expression();
        self.condition_context = old_cond;
        result
    }

    /// Parse a complete program.
    pub fn parse(&mut self) -> ParseResult<Program> {
        let start = crate::metrics::metrics_enabled().then(Instant::now);
        let mut statements = Vec::new();

        while !self.is_at_end() {
            statements.push(self.declaration()?);
        }

        if let Some(start) = start {
            Metrics::global().record_parsing(start.elapsed());
        }
        Ok(Program::new(statements))
    }

    // ===== Token manipulation =====

    pub(crate) fn advance(&mut self) -> Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        // `saturating_sub`, because `current` can still be 0 here: at EOF the
        // cursor does not move, so a caller that reaches `advance()` while the
        // stream is already at its Eof token computes `0 - 1`. That underflow
        // panicked the parser on hostile input (found by the `parse_program`
        // fuzz target). The scanner always emits a trailing Eof, so index 0 is
        // always a real token to hand back.
        self.tokens[self.current.saturating_sub(1)].clone()
    }

    pub(crate) fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    pub(crate) fn previous(&self) -> &Token {
        // Same underflow as `advance`: nothing precedes the first token.
        &self.tokens[self.current.saturating_sub(1)]
    }

    pub(crate) fn peek_nth(&self, n: usize) -> &Token {
        let index = if self.current + n < self.tokens.len() {
            self.current + n
        } else {
            self.tokens.len() - 1
        };
        &self.tokens[index]
    }

    pub(crate) fn is_at_end(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    pub(crate) fn check(&self, kind: &TokenKind) -> bool {
        if self.is_at_end() {
            false
        } else {
            std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
        }
    }

    pub(crate) fn check_identifier(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Identifier(_))
    }

    pub(crate) fn match_token(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(crate) fn expect(&mut self, kind: &TokenKind) -> ParseResult<Token> {
        if self.check(kind) {
            Ok(self.advance())
        } else {
            Err(ParserError::unexpected_token(
                format!("{}", kind),
                format!("{}", self.peek().kind),
                self.current_span(),
            ))
        }
    }

    pub(crate) fn expect_identifier(&mut self) -> ParseResult<String> {
        match &self.peek().kind {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            TokenKind::New => {
                self.advance();
                Ok("new".to_string())
            }
            TokenKind::Match => {
                self.advance();
                Ok("match".to_string())
            }
            TokenKind::Class => {
                self.advance();
                Ok("class".to_string())
            }
            _ => Err(ParserError::unexpected_token(
                "identifier",
                format!("{}", self.peek().kind),
                self.current_span(),
            )),
        }
    }

    pub(crate) fn current_span(&self) -> Span {
        self.peek().span
    }

    pub(crate) fn previous_span(&self) -> Span {
        self.previous().span
    }
}
