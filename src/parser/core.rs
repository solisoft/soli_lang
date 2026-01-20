//! Core parser struct and helper methods.

use crate::ast::*;
use crate::error::ParserError;
use crate::lexer::{Token, TokenKind};
use crate::span::Span;

pub type ParseResult<T> = Result<T, ParserError>;

/// The parser for Solilang.
pub struct Parser {
    pub(crate) tokens: Vec<Token>,
    pub(crate) current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, current: 0 }
    }

    /// Parse a complete program.
    pub fn parse(&mut self) -> ParseResult<Program> {
        let mut statements = Vec::new();

        while !self.is_at_end() {
            statements.push(self.declaration()?);
        }

        Ok(Program::new(statements))
    }

    // ===== Token manipulation =====

    pub(crate) fn advance(&mut self) -> Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.tokens[self.current - 1].clone()
    }

    pub(crate) fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    pub(crate) fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
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
