//! Statement parsing: if, while, for, return, blocks.

use crate::ast::*;
use crate::lexer::TokenKind;

use super::core::{ParseResult, Parser};

impl Parser {
    pub(crate) fn statement(&mut self) -> ParseResult<Stmt> {
        if self.check(&TokenKind::Class) {
            self.class_declaration()
        } else if self.check(&TokenKind::Fn) {
            self.function_declaration()
        } else if self.check(&TokenKind::Let) {
            self.let_declaration()
        } else if self.check(&TokenKind::Const) {
            self.const_declaration()
        } else if self.check(&TokenKind::If) {
            self.if_statement()
        } else if self.check(&TokenKind::While) {
            self.while_statement()
        } else if self.check(&TokenKind::For) {
            self.for_statement()
        } else if self.check(&TokenKind::Return) {
            self.return_statement()
        } else if self.check(&TokenKind::Throw) {
            self.throw_statement()
        } else if self.check(&TokenKind::Try) {
            self.try_statement()
        } else if self.check(&TokenKind::LeftBrace) {
            if self.looks_like_hash_literal() {
                self.expression_statement()
            } else {
                self.block_statement()
            }
        } else if self.check(&TokenKind::Interface) {
            self.interface_declaration()
        } else {
            self.expression_statement()
        }
    }

    fn if_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::If)?;

        // Parentheses are optional around the condition
        let has_paren = self.match_token(&TokenKind::LeftParen);
        let condition = self.expression()?;
        if has_paren {
            self.expect(&TokenKind::RightParen)?;
        }

        let then_branch = self.parse_branch_body()?;

        let else_branch = if self.match_token(&TokenKind::End) {
            None
        } else if self.match_token(&TokenKind::Else) {
            Some(self.parse_else_body()?)
        } else if self.check(&TokenKind::Elsif) {
            // Handle elsif as else { if ... }
            Some(Box::new(self.elsif_statement()?))
        } else {
            None
        };

        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            },
            span,
        ))
    }

    fn elsif_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Elsif)?;

        // Parentheses are optional around the condition
        let has_paren = self.match_token(&TokenKind::LeftParen);
        let condition = self.expression()?;
        if has_paren {
            self.expect(&TokenKind::RightParen)?;
        }

        let then_branch = self.parse_branch_body()?;

        let else_branch = if self.match_token(&TokenKind::End) {
            None
        } else if self.match_token(&TokenKind::Else) {
            Some(self.parse_else_body()?)
        } else if self.check(&TokenKind::Elsif) {
            // Handle chained elsif
            Some(Box::new(self.elsif_statement()?))
        } else {
            None
        };

        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            },
            span,
        ))
    }

    fn while_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::While)?;

        // Parentheses are optional around the condition
        let has_paren = self.match_token(&TokenKind::LeftParen);
        let condition = self.expression()?;
        if has_paren {
            self.expect(&TokenKind::RightParen)?;
        }

        let body = self.parse_block_body()?;
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(StmtKind::While { condition, body }, span))
    }

    fn for_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::For)?;

        // Parentheses are optional around the for clause
        let has_paren = self.match_token(&TokenKind::LeftParen);
        let variable = self.expect_identifier()?;
        self.expect(&TokenKind::In)?;
        let iterable = self.expression()?;
        if has_paren {
            self.expect(&TokenKind::RightParen)?;
        }

        let body = self.parse_block_body()?;
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::For {
                variable,
                iterable,
                body,
            },
            span,
        ))
    }

    fn return_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Return)?;

        let value = if !self.check(&TokenKind::Semicolon) {
            Some(self.expression()?)
        } else {
            None
        };

        self.match_token(&TokenKind::Semicolon);
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(StmtKind::Return(value), span))
    }

    fn throw_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Throw)?;

        let value = self.expression()?;
        self.match_token(&TokenKind::Semicolon);
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(StmtKind::Throw(value), span))
    }

    fn try_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Try)?;

        // After `try`, a `{` always starts a block body, never a hash literal.
        let uses_braces = self.check(&TokenKind::LeftBrace);

        if uses_braces {
            // Brace syntax: try { ... } catch (e) { ... } finally { ... }
            let try_block = Box::new(self.block_statement()?);

            let (catch_var, catch_block) = if self.match_token(&TokenKind::Catch) {
                let var = self.parse_catch_var()?;
                let block = Some(Box::new(self.block_statement()?));
                (var, block)
            } else {
                (None, None)
            };

            let finally_block = if self.match_token(&TokenKind::Finally) {
                Some(Box::new(self.block_statement()?))
            } else {
                None
            };

            let span = start_span.merge(&self.previous_span());
            Ok(Stmt::new(
                StmtKind::Try {
                    try_block,
                    catch_var,
                    catch_block,
                    finally_block,
                },
                span,
            ))
        } else {
            // End syntax: try ... catch e ... finally ... end
            let body_start = self.current_span();
            let mut try_stmts = Vec::new();
            while !self.check(&TokenKind::Catch)
                && !self.check(&TokenKind::Finally)
                && !self.check(&TokenKind::End)
                && !self.is_at_end()
            {
                try_stmts.push(self.statement()?);
            }
            let try_block = Box::new(Stmt::new(
                StmtKind::Block(try_stmts),
                body_start.merge(&self.previous_span()),
            ));

            let (catch_var, catch_block) = if self.match_token(&TokenKind::Catch) {
                let var = self.parse_catch_var()?;
                let catch_start = self.current_span();
                let mut catch_stmts = Vec::new();
                while !self.check(&TokenKind::Finally)
                    && !self.check(&TokenKind::End)
                    && !self.is_at_end()
                {
                    catch_stmts.push(self.statement()?);
                }
                let block = Box::new(Stmt::new(
                    StmtKind::Block(catch_stmts),
                    catch_start.merge(&self.previous_span()),
                ));
                (var, Some(block))
            } else {
                (None, None)
            };

            let finally_block = if self.match_token(&TokenKind::Finally) {
                let finally_start = self.current_span();
                let mut finally_stmts = Vec::new();
                while !self.check(&TokenKind::End) && !self.is_at_end() {
                    finally_stmts.push(self.statement()?);
                }
                Some(Box::new(Stmt::new(
                    StmtKind::Block(finally_stmts),
                    finally_start.merge(&self.previous_span()),
                )))
            } else {
                None
            };

            self.expect(&TokenKind::End)?;
            let span = start_span.merge(&self.previous_span());
            Ok(Stmt::new(
                StmtKind::Try {
                    try_block,
                    catch_var,
                    catch_block,
                    finally_block,
                },
                span,
            ))
        }
    }

    /// Parse a catch variable: `(e)` with parens or bare `e` on the same line.
    fn parse_catch_var(&mut self) -> ParseResult<Option<String>> {
        if self.match_token(&TokenKind::LeftParen) {
            let var = self.expect_identifier()?;
            self.expect(&TokenKind::RightParen)?;
            Ok(Some(var))
        } else {
            // Bare variable: catch e — identifier must be on the same line as catch
            let catch_line = self.previous_span().line;
            let next_line = self.current_span().line;
            if catch_line == next_line
                && !self.check(&TokenKind::LeftBrace)
                && !self.check(&TokenKind::End)
                && !self.check(&TokenKind::Finally)
                && !self.is_at_end()
            {
                Ok(Some(self.expect_identifier()?))
            } else {
                Ok(None)
            }
        }
    }

    /// Check if a `{` at statement position starts a hash literal rather than a block.
    /// Peeks ahead without consuming tokens.
    pub(crate) fn looks_like_hash_literal(&self) -> bool {
        match &self.peek_nth(1).kind {
            // {} is empty hash
            TokenKind::RightBrace => true,
            // { "key": ... } — string key followed by colon/arrow starts a hash
            TokenKind::StringLiteral(_) => matches!(
                &self.peek_nth(2).kind,
                TokenKind::Colon | TokenKind::FatArrow
            ),
            // { 42: ... } — number key followed by colon/arrow starts a hash
            TokenKind::IntLiteral(_) | TokenKind::FloatLiteral(_) => matches!(
                &self.peek_nth(2).kind,
                TokenKind::Colon | TokenKind::FatArrow
            ),
            // { name: ... } or { name => ... } — identifier followed by hash separator
            TokenKind::Identifier(_) => matches!(
                &self.peek_nth(2).kind,
                TokenKind::Colon | TokenKind::FatArrow
            ),
            // Anything else (keywords, nested {, etc.) → block
            _ => false,
        }
    }

    fn block_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        let statements = self.block_statements()?;
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(StmtKind::Block(statements), span))
    }

    pub(crate) fn block_statements(&mut self) -> ParseResult<Vec<Stmt>> {
        self.expect(&TokenKind::LeftBrace)?;

        let mut statements = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            statements.push(self.statement()?);
        }

        self.expect(&TokenKind::RightBrace)?;
        Ok(statements)
    }

    /// Parse an else body, consuming 'end' for multi-line indentation-style bodies.
    /// One-liner else (body on same line as 'else') does NOT consume 'end'.
    fn parse_else_body(&mut self) -> ParseResult<Box<Stmt>> {
        let else_line = self.previous_span().line; // line of the 'else' keyword
        let body_line = self.current_span().line; // line of the first body token
        let is_multiline = body_line != else_line;

        let branch = self.parse_branch_body()?;

        // For multi-line else bodies (indentation-style), consume the closing 'end'
        if is_multiline && !self.check(&TokenKind::Else) && !self.check(&TokenKind::Elsif) {
            self.match_token(&TokenKind::End);
        }

        Ok(branch)
    }

    fn parse_branch_body(&mut self) -> ParseResult<Box<Stmt>> {
        if self.match_token(&TokenKind::End) {
            Ok(Box::new(Stmt::new(
                StmtKind::Block(Vec::new()),
                self.previous_span(),
            )))
        } else if self.check(&TokenKind::LeftBrace) && !self.looks_like_hash_literal() {
            self.advance(); // consume {
            let mut statements = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::RightBrace)?;
            Ok(Box::new(Stmt::new(
                StmtKind::Block(statements),
                self.previous_span(),
            )))
        } else {
            let mut statements = Vec::new();
            while !self.check(&TokenKind::End)
                && !self.check(&TokenKind::Else)
                && !self.check(&TokenKind::Elsif)
                && !self.is_at_end()
            {
                statements.push(self.statement()?);
            }
            if statements.is_empty() {
                Ok(Box::new(Stmt::new(
                    StmtKind::Block(Vec::new()),
                    self.previous_span(),
                )))
            } else {
                Ok(Box::new(Stmt::new(
                    StmtKind::Block(statements),
                    self.previous_span(),
                )))
            }
        }
    }

    /// Parse a block body for `for`/`while` loops.
    /// Unlike `parse_branch_body`, this always consumes the closing `end`
    /// for indentation-style blocks. (`if` needs `parse_branch_body` because
    /// it must inspect `else`/`elsif` before consuming `end`.)
    fn parse_block_body(&mut self) -> ParseResult<Box<Stmt>> {
        if self.check(&TokenKind::LeftBrace) && !self.looks_like_hash_literal() {
            self.advance(); // consume {
            let start_span = self.previous_span();
            let mut statements = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::RightBrace)?;
            Ok(Box::new(Stmt::new(
                StmtKind::Block(statements),
                start_span.merge(&self.previous_span()),
            )))
        } else {
            let start_span = self.current_span();
            let mut statements = Vec::new();
            while !self.check(&TokenKind::End) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::End)?;
            Ok(Box::new(Stmt::new(
                StmtKind::Block(statements),
                start_span.merge(&self.previous_span()),
            )))
        }
    }

    fn expression_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        let expr = self.expression()?;

        // Line of the last token consumed by the expression.
        // Postfix if/unless must be on the same line to avoid stealing
        // a block-level `if`/`unless` from the next line.
        let expr_end_line = self.previous_span().line;

        // Check for postfix if: expr if cond (parentheses optional)
        if self.check(&TokenKind::If) && self.peek().span.line == expr_end_line {
            self.advance(); // consume if
            let has_paren = self.match_token(&TokenKind::LeftParen);
            let cond = self.expression()?;
            if has_paren {
                self.expect(&TokenKind::RightParen)?;
            }

            // Consume optional semicolon for postfix if
            if self.check(&TokenKind::Semicolon) {
                self.advance();
            }

            let span = start_span.merge(&self.previous_span());

            return Ok(Stmt::new(
                StmtKind::If {
                    condition: cond,
                    then_branch: Box::new(Stmt::new(StmtKind::Expression(expr.clone()), expr.span)),
                    else_branch: None,
                },
                span,
            ));
        }

        // Check for postfix unless: expr unless cond (parentheses optional)
        if self.check(&TokenKind::Unless) && self.peek().span.line == expr_end_line {
            self.advance(); // consume unless
            let has_paren = self.match_token(&TokenKind::LeftParen);
            let cond = self.expression()?;
            if has_paren {
                self.expect(&TokenKind::RightParen)?;
            }

            // Consume optional semicolon for postfix unless
            if self.check(&TokenKind::Semicolon) {
                self.advance();
            }

            let condition_expr = Expr::new(
                ExprKind::Unary {
                    operator: crate::ast::expr::UnaryOp::Not,
                    operand: Box::new(cond),
                },
                start_span.merge(&self.previous_span()),
            );

            let span = start_span.merge(&self.previous_span());

            return Ok(Stmt::new(
                StmtKind::If {
                    condition: condition_expr,
                    then_branch: Box::new(Stmt::new(StmtKind::Expression(expr.clone()), expr.span)),
                    else_branch: None,
                },
                span,
            ));
        }

        self.match_token(&TokenKind::Semicolon);
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(StmtKind::Expression(expr), span))
    }
}
