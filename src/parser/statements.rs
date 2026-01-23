//! Statement parsing: if, while, for, return, blocks.

use crate::ast::*;
use crate::lexer::TokenKind;

use super::core::{ParseResult, Parser};

impl Parser {
    pub(crate) fn statement(&mut self) -> ParseResult<Stmt> {
        if self.check(&TokenKind::If) {
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
            self.block_statement()
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

        let then_branch = Box::new(self.statement()?);

        let else_branch = if self.match_token(&TokenKind::Else) {
            Some(Box::new(self.statement()?))
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

        let body = Box::new(self.statement()?);
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

        let body = Box::new(self.statement()?);
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

        let try_block = Box::new(self.block_statement()?);

        let catch_var = if self.match_token(&TokenKind::Catch) {
            self.expect(&TokenKind::LeftParen)?;
            let var = self.expect_identifier()?;
            self.expect(&TokenKind::RightParen)?;
            Some(var)
        } else {
            None
        };

        let catch_block = if self.check(&TokenKind::LeftBrace) || catch_var.is_some() {
            Some(Box::new(self.block_statement()?))
        } else {
            None
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
            statements.push(self.declaration()?);
        }

        self.expect(&TokenKind::RightBrace)?;
        Ok(statements)
    }

    fn expression_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        let expr = self.expression()?;

        // Check for postfix if: expr if cond (parentheses optional)
        if self.check(&TokenKind::If) {
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
        if self.check(&TokenKind::Unless) {
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
