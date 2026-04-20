//! Expression parsing using Pratt precedence.

use crate::ast::expr::{Argument, NamedArgument};
use crate::ast::*;
use crate::error::ParserError;
use crate::lexer::TokenKind;

use super::core::{ParseResult, Parser};
use super::precedence::{get_precedence, Precedence};

impl Parser {
    pub(crate) fn expression(&mut self) -> ParseResult<Expr> {
        self.parse_precedence(Precedence::Assignment)
    }

    pub(crate) fn parse_precedence(&mut self, min_precedence: Precedence) -> ParseResult<Expr> {
        let mut left = self.parse_prefix()?;

        while !self.is_at_end() {
            let precedence = get_precedence(&self.peek().kind);
            if precedence < min_precedence {
                break;
            }

            left = self.parse_infix(left, precedence)?;
        }

        Ok(left)
    }

    fn parse_prefix(&mut self) -> ParseResult<Expr> {
        let token = self.advance();
        let start_span = token.span;

        match &token.kind {
            TokenKind::IntLiteral(n) => Ok(Expr::new(ExprKind::IntLiteral(*n), start_span)),
            TokenKind::FloatLiteral(n) => Ok(Expr::new(ExprKind::FloatLiteral(*n), start_span)),
            TokenKind::DecimalLiteral(s) => {
                Ok(Expr::new(ExprKind::DecimalLiteral(s.clone()), start_span))
            }
            TokenKind::StringLiteral(s) => {
                Ok(Expr::new(ExprKind::StringLiteral(s.clone()), start_span))
            }
            TokenKind::InterpolatedString(parts) => {
                self.parse_interpolated_string(parts.clone(), start_span)
            }
            TokenKind::BacktickString(s) => Ok(Expr::new(
                ExprKind::CommandSubstitution(s.clone()),
                start_span,
            )),
            TokenKind::SdqlBlock {
                query,
                interpolations,
            } => Ok(Expr::new(
                ExprKind::SdqlBlock {
                    query: query.clone(),
                    interpolations: interpolations
                        .iter()
                        .map(|i| crate::ast::expr::SdqlInterpolation {
                            expr: i.expr.clone(),
                            start: i.start,
                            end: i.end,
                        })
                        .collect(),
                },
                start_span,
            )),
            TokenKind::BoolLiteral(b) => Ok(Expr::new(ExprKind::BoolLiteral(*b), start_span)),
            TokenKind::SymbolLiteral(s) => Ok(Expr::new(ExprKind::Symbol(s.clone()), start_span)),
            TokenKind::StringArrayLiteral(elements) => {
                let exprs: Vec<Expr> = elements
                    .iter()
                    .map(|s| Expr::new(ExprKind::StringLiteral(s.clone()), start_span))
                    .collect();
                Ok(Expr::new(ExprKind::Array(exprs), start_span))
            }
            TokenKind::SymbolArrayLiteral(elements) => {
                let exprs: Vec<Expr> = elements
                    .iter()
                    .map(|s| Expr::new(ExprKind::Symbol(s.clone()), start_span))
                    .collect();
                Ok(Expr::new(ExprKind::Array(exprs), start_span))
            }
            TokenKind::NumberArrayLiteral(elements) => {
                let exprs: Vec<Expr> = elements
                    .iter()
                    .map(|s| {
                        if s.ends_with('D') || s.ends_with('d') {
                            let value = if let Some(dot_pos) = s[..s.len() - 1].find('.') {
                                let before_dot = &s[..dot_pos];
                                let after_dot = &s[dot_pos + 1..s.len() - 1];
                                let trimmed = after_dot.trim_end_matches('0');
                                if trimmed.is_empty() {
                                    format!("{}.00", before_dot)
                                } else {
                                    format!("{}.{}", before_dot, trimmed)
                                }
                            } else {
                                format!("{}.00", &s[..s.len() - 1])
                            };
                            Expr::new(ExprKind::DecimalLiteral(value), start_span)
                        } else if let Ok(n) = s.parse::<i64>() {
                            Expr::new(ExprKind::IntLiteral(n), start_span)
                        } else if let Ok(n) = s.parse::<f64>() {
                            Expr::new(ExprKind::FloatLiteral(n), start_span)
                        } else {
                            Expr::new(ExprKind::IntLiteral(0), start_span)
                        }
                    })
                    .collect();
                Ok(Expr::new(ExprKind::Array(exprs), start_span))
            }
            TokenKind::Null => Ok(Expr::new(ExprKind::Null, start_span)),

            TokenKind::Identifier(name) => {
                // Command-style calls: identifier followed by argument on the SAME LINE
                // e.g., print x, print "hello", puts result
                // Same-line requirement prevents ambiguity with multi-line bodies:
                //   fn foo
                //     bar    ← not a call to foo
                //   end
                let next = self.peek();
                if next.span.line == start_span.line && Self::is_command_arg(&next.kind) {
                    let arguments = self.parse_command_arguments()?;

                    // Check for trailing block: puts("args") { body } or puts "args" { body }
                    if !self.no_trailing_brace
                        && self.check(&TokenKind::LeftBrace)
                        && !self.looks_like_hash_literal()
                    {
                        let block = self.parse_trailing_brace_block()?;
                        let mut args = arguments;
                        args.push(Argument::Block(block));
                        let span = start_span.merge(&self.previous_span());
                        return Ok(Expr::new(
                            ExprKind::Call {
                                callee: Box::new(Expr::new(
                                    ExprKind::Variable(name.clone()),
                                    start_span,
                                )),
                                arguments: args,
                            },
                            span,
                        ));
                    }

                    let span = start_span.merge(&self.previous_span());
                    return Ok(Expr::new(
                        ExprKind::Call {
                            callee: Box::new(Expr::new(
                                ExprKind::Variable(name.clone()),
                                start_span,
                            )),
                            arguments,
                        },
                        span,
                    ));
                }
                Ok(Expr::new(ExprKind::Variable(name.clone()), start_span))
            }

            TokenKind::This | TokenKind::SelfKeyword => Ok(Expr::new(ExprKind::This, start_span)),
            TokenKind::Super => Ok(Expr::new(ExprKind::Super, start_span)),

            TokenKind::LeftParen => {
                let expr = self.expression()?;
                self.expect(&TokenKind::RightParen)?;
                let span = start_span.merge(&self.previous_span());
                Ok(Expr::new(ExprKind::Grouping(Box::new(expr)), span))
            }

            TokenKind::LeftBracket => self.parse_array(start_span),
            TokenKind::LeftBrace => self.parse_hash(start_span),

            TokenKind::Minus => {
                let operand = self.parse_precedence(Precedence::Unary)?;
                let span = start_span.merge(&operand.span);
                Ok(Expr::new(
                    ExprKind::Unary {
                        operator: UnaryOp::Negate,
                        operand: Box::new(operand),
                    },
                    span,
                ))
            }

            TokenKind::Bang => {
                let operand = self.parse_precedence(Precedence::Unary)?;
                let span = start_span.merge(&operand.span);
                Ok(Expr::new(
                    ExprKind::Unary {
                        operator: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                ))
            }

            TokenKind::Not => {
                let operand = self.parse_precedence(Precedence::Unary)?;
                let span = start_span.merge(&operand.span);
                Ok(Expr::new(
                    ExprKind::Unary {
                        operator: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                ))
            }

            TokenKind::New => {
                let start_span = self.current_span();
                // Note: 'new' has already been consumed by parse_prefix

                // Parse the class name (could be simple name or qualified name)
                let name_span = self.current_span();
                let class_name = self.expect_identifier()?;

                // Check for qualified name (e.g., Outer::Inner)
                let class_expr = if self.check(&TokenKind::DoubleColon) {
                    self.advance(); // consume ::
                    let nested_name = self.expect_identifier()?;
                    let nested_span = name_span.merge(&self.previous_span());
                    Expr::new(
                        ExprKind::QualifiedName {
                            qualifier: Box::new(Expr::new(
                                ExprKind::Variable(class_name),
                                name_span,
                            )),
                            name: nested_name,
                        },
                        nested_span,
                    )
                } else {
                    Expr::new(ExprKind::Variable(class_name), name_span)
                };

                self.expect(&TokenKind::LeftParen)?;
                let arguments = self.parse_arguments()?;
                self.expect(&TokenKind::RightParen)?;
                let span = start_span.merge(&self.previous_span());
                Ok(Expr::new(
                    ExprKind::New {
                        class_expr: Box::new(class_expr),
                        arguments,
                    },
                    span,
                ))
            }

            TokenKind::Fn => self.parse_anonymous_function(start_span),

            TokenKind::Match => self.parse_match_expression(start_span),

            TokenKind::Pipe => self.parse_lambda(start_span),
            TokenKind::Or => self.parse_lambda_empty_params(start_span),
            TokenKind::Arrow => self.parse_stabby_lambda(start_span),

            // &:method_name → |__it| __it.method_name
            TokenKind::Ampersand => {
                // Accept both &:symbol and &:identifier forms
                let method_name = if let TokenKind::SymbolLiteral(s) = &self.peek().kind {
                    let s = s.clone();
                    self.advance();
                    s
                } else {
                    self.expect(&TokenKind::Colon)?;
                    let name = self.expect_identifier()?;
                    if self.match_token(&TokenKind::Question) {
                        format!("{}?", name)
                    } else {
                        name
                    }
                };
                let span = start_span.merge(&self.previous_span());
                let param = crate::ast::stmt::Parameter {
                    name: "__it".to_string(),
                    type_annotation: TypeAnnotation::new(
                        crate::ast::types::TypeKind::Named("Any".to_string()),
                        start_span,
                    ),
                    default_value: None,
                    span: start_span,
                    is_block_param: false,
                };
                let body_expr = Expr::new(
                    ExprKind::Member {
                        object: Box::new(Expr::new(
                            ExprKind::Variable("__it".to_string()),
                            start_span,
                        )),
                        name: method_name,
                    },
                    span,
                );
                let body_stmt = Stmt::new(StmtKind::Expression(body_expr), span);
                Ok(Expr::new(
                    ExprKind::Lambda {
                        params: vec![param],
                        return_type: None,
                        body: vec![body_stmt],
                    },
                    span,
                ))
            }

            // Allow 'await' keyword to be used as a function call: await(future)
            TokenKind::Await => Ok(Expr::new(
                ExprKind::Variable("await".to_string()),
                start_span,
            )),

            _ => Err(ParserError::unexpected_token(
                "expression",
                format!("{}", token.kind),
                token.span,
            )),
        }
    }

    fn parse_array(&mut self, start_span: crate::span::Span) -> ParseResult<Expr> {
        // Check if this is a comprehension: [expr for x in iter if cond]
        // We need to look ahead to see if there's a 'for' after the first expression
        if self.check(&TokenKind::For) {
            return self.parse_list_comprehension(start_span, None);
        }

        let mut elements = Vec::new();
        if !self.check(&TokenKind::RightBracket) {
            loop {
                if self.match_token(&TokenKind::Spread) {
                    // Spread operator: ...expr
                    let expr = self.expression()?;
                    let span = start_span.merge(&expr.span);
                    elements.push(Expr::new(ExprKind::Spread(Box::new(expr)), span));
                } else {
                    // Check if this might be the start of a comprehension
                    let element = self.expression()?;

                    // Check if this is followed by 'for'
                    if self.match_token(&TokenKind::For) {
                        // This is a comprehension!
                        return self.parse_list_comprehension(start_span, Some(element));
                    }

                    elements.push(element);
                }

                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightBracket) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RightBracket)?;
        let span = start_span.merge(&self.previous_span());
        Ok(Expr::new(ExprKind::Array(elements), span))
    }

    fn parse_list_comprehension(
        &mut self,
        start_span: crate::span::Span,
        element: Option<Expr>,
    ) -> ParseResult<Expr> {
        // [element for var in iterable if condition]
        // If element is None, we need to parse it (consumed 'for' already)
        let element = if let Some(e) = element {
            e
        } else {
            self.expect(&TokenKind::For)?; // consume 'for'
            self.expression()?
        };

        // Parse the variable name
        let var_name = if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            self.advance();
            name
        } else {
            return Err(ParserError::unexpected_token(
                "identifier".to_string(),
                format!("{}", self.peek().kind),
                self.current_span(),
            ));
        };

        self.expect(&TokenKind::In)?;
        let iterable = self.expression()?;

        // Parse optional condition
        let condition = if self.match_token(&TokenKind::If) {
            Some(Box::new(self.expression()?))
        } else {
            None
        };

        self.expect(&TokenKind::RightBracket)?;
        let span = start_span.merge(&self.previous_span());

        Ok(Expr::new(
            ExprKind::ListComprehension {
                element: Box::new(element),
                variable: var_name,
                iterable: Box::new(iterable),
                condition,
            },
            span,
        ))
    }

    fn parse_hash(&mut self, start_span: crate::span::Span) -> ParseResult<Expr> {
        // Check if this is a hash comprehension: {key: value for x in iter if cond}
        if self.check(&TokenKind::For) {
            return self.parse_hash_comprehension(start_span, None, None);
        }

        // Check if this is a block expression (starts with statement keyword)
        // Note: At this point, we've already advanced past the '{' in parse_prefix
        // so self.peek() gives us the token AFTER the '{'
        let is_block = match &self.peek().kind {
            TokenKind::Let
            | TokenKind::Const
            | TokenKind::If
            | TokenKind::While
            | TokenKind::For
            | TokenKind::Return
            | TokenKind::Throw
            | TokenKind::Try
            | TokenKind::Fn
            | TokenKind::Class
            | TokenKind::Interface
            | TokenKind::Match
            | TokenKind::Await => true,
            TokenKind::RightBrace => false, // Empty hash {}, not block
            TokenKind::LeftBrace => self.is_nested_block_expression(),
            _ => false,
        };

        if is_block {
            // Current token is already past the opening '{'
            // start_span is the span of the '{' token
            let _block_span = start_span.merge(&self.previous_span());
            // Current token should be either '}' (empty block) or the first statement
            let mut statements = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::RightBrace)?;
            let end_span = self.previous_span();
            let full_span = start_span.merge(&end_span);
            return Ok(Expr::new(ExprKind::Block(statements), full_span));
        }

        let mut pairs = Vec::new();
        if !self.check(&TokenKind::RightBrace) {
            loop {
                let key = self.expression()?;

                // Check if this is a comprehension
                if self.match_token(&TokenKind::For) {
                    // This is a hash comprehension with key already parsed
                    return self.parse_hash_comprehension(start_span, Some(key), None);
                }

                self.expect_hash_separator()?;

                // Convert variable keys to string literals (shorthand syntax: {name: value} => {"name": value})
                let key = if let ExprKind::Variable(name) = &key.kind {
                    Expr::new(ExprKind::StringLiteral(name.clone()), key.span)
                } else {
                    key
                };

                // Shorthand: { name: } or { name:, age: } — value is the variable with the same name as the key
                let value = if let ExprKind::StringLiteral(ref name) = key.kind {
                    if self.check(&TokenKind::Comma) || self.check(&TokenKind::RightBrace) {
                        Expr::new(ExprKind::Variable(name.clone()), key.span)
                    } else {
                        self.expression()?
                    }
                } else {
                    self.expression()?
                };

                // Check if this is a comprehension
                if self.match_token(&TokenKind::For) {
                    // This is a hash comprehension with key and value already parsed
                    return self.parse_hash_comprehension(start_span, None, Some((key, value)));
                }

                pairs.push((key, value));

                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightBrace) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RightBrace)?;
        let span = start_span.merge(&self.previous_span());
        Ok(Expr::new(ExprKind::Hash(pairs), span))
    }

    fn is_nested_block_expression(&mut self) -> bool {
        let mut depth = 1;
        let mut i = 1;
        loop {
            match self.tokens.get(self.current + i).map(|t| &t.kind) {
                Some(TokenKind::LeftBrace) => {
                    depth += 1;
                    i += 1;
                }
                Some(TokenKind::RightBrace) => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        return false;
                    }
                }
                Some(TokenKind::Let)
                | Some(TokenKind::Const)
                | Some(TokenKind::If)
                | Some(TokenKind::While)
                | Some(TokenKind::For)
                | Some(TokenKind::Return)
                | Some(TokenKind::Throw)
                | Some(TokenKind::Try)
                | Some(TokenKind::Fn)
                | Some(TokenKind::Class)
                | Some(TokenKind::Interface)
                | Some(TokenKind::Match)
                | Some(TokenKind::Await) => {
                    return true;
                }
                Some(_) | None => {
                    i += 1;
                }
            }
        }
    }

    fn parse_hash_comprehension(
        &mut self,
        start_span: crate::span::Span,
        key: Option<Expr>,
        key_value: Option<(Expr, Expr)>,
    ) -> ParseResult<Expr> {
        // {key_expr: value_expr for var in iterable if condition}
        // or {key: value for var in iterable if condition} (key and value are Variable)

        let (key_expr, value_expr) = if let Some(kv) = key_value {
            (kv.0, kv.1)
        } else if let Some(k) = key {
            // key was parsed but not value - parse value
            self.expect_hash_separator()?;
            let v = self.expression()?;
            (k, v)
        } else {
            // Neither key nor key-value was parsed
            self.expect(&TokenKind::For)?; // consume 'for'
            let k = self.expression()?;
            self.expect_hash_separator()?;
            let v = self.expression()?;
            (k, v)
        };

        // Parse the variable name
        let var_name = if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            self.advance();
            name
        } else {
            return Err(ParserError::unexpected_token(
                "identifier".to_string(),
                format!("{}", self.peek().kind),
                self.current_span(),
            ));
        };

        self.expect(&TokenKind::In)?;
        let iterable = self.expression()?;

        // Parse optional condition
        let condition = if self.match_token(&TokenKind::If) {
            Some(Box::new(self.expression()?))
        } else {
            None
        };

        self.expect(&TokenKind::RightBrace)?;
        let span = start_span.merge(&self.previous_span());

        Ok(Expr::new(
            ExprKind::HashComprehension {
                key: Box::new(key_expr),
                value: Box::new(value_expr),
                variable: var_name,
                iterable: Box::new(iterable),
                condition,
            },
            span,
        ))
    }

    /// Expect either '=>' or ':' as a hash key-value separator
    fn expect_hash_separator(&mut self) -> ParseResult<()> {
        if self.match_token(&TokenKind::FatArrow) || self.match_token(&TokenKind::Colon) {
            Ok(())
        } else {
            Err(ParserError::unexpected_token(
                "'=>' or ':'".to_string(),
                format!("{}", self.peek().kind),
                self.current_span(),
            ))
        }
    }

    fn parse_infix(&mut self, left: Expr, precedence: Precedence) -> ParseResult<Expr> {
        let token = self.advance();
        let start_span = left.span;

        match &token.kind {
            // Binary operators
            TokenKind::Plus => self.binary_expr(left, BinaryOp::Add, precedence),
            TokenKind::Minus => self.binary_expr(left, BinaryOp::Subtract, precedence),
            TokenKind::Star => self.binary_expr(left, BinaryOp::Multiply, precedence),
            TokenKind::Slash => self.binary_expr(left, BinaryOp::Divide, precedence),
            TokenKind::Percent => self.binary_expr(left, BinaryOp::Modulo, precedence),
            TokenKind::EqualEqual => self.binary_expr(left, BinaryOp::Equal, precedence),
            TokenKind::BangEqual => self.binary_expr(left, BinaryOp::NotEqual, precedence),
            TokenKind::Less => self.binary_expr(left, BinaryOp::Less, precedence),
            TokenKind::LessEqual => self.binary_expr(left, BinaryOp::LessEqual, precedence),
            TokenKind::Greater => self.binary_expr(left, BinaryOp::Greater, precedence),
            TokenKind::GreaterEqual => self.binary_expr(left, BinaryOp::GreaterEqual, precedence),
            TokenKind::Range => self.binary_expr(left, BinaryOp::Range, precedence),

            // Ternary operator: cond ? then_expr : else_expr
            TokenKind::Question => {
                let then_expr = self.expression()?;
                self.expect(&TokenKind::Colon)?;
                let else_expr = self.parse_precedence(precedence)?;
                let span = start_span.merge(&else_expr.span);
                Ok(Expr::new(
                    ExprKind::If {
                        condition: Box::new(left),
                        then_branch: Box::new(then_expr),
                        else_branch: Some(Box::new(else_expr)),
                    },
                    span,
                ))
            }

            // Logical operators
            TokenKind::And => {
                let right = self.parse_precedence(precedence.next())?;
                let span = start_span.merge(&right.span);
                Ok(Expr::new(
                    ExprKind::LogicalAnd {
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ))
            }
            TokenKind::Or => {
                let right = self.parse_precedence(precedence.next())?;
                let span = start_span.merge(&right.span);
                Ok(Expr::new(
                    ExprKind::LogicalOr {
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ))
            }

            // Nullish coalescing: a ?? b
            TokenKind::NullishCoalescing => {
                let right = self.parse_precedence(precedence.next())?;
                let span = start_span.merge(&right.span);
                Ok(Expr::new(
                    ExprKind::NullishCoalescing {
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ))
            }

            // Pipeline operator
            TokenKind::Pipeline => {
                let right = self.parse_precedence(precedence.next())?;
                let span = start_span.merge(&right.span);
                Ok(Expr::new(
                    ExprKind::Pipeline {
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ))
            }

            // Assignment
            TokenKind::Equal => {
                let value = self.parse_precedence(Precedence::Assignment)?;
                let span = start_span.merge(&value.span);

                match &left.kind {
                    ExprKind::Variable(_) | ExprKind::Member { .. } | ExprKind::Index { .. } => {
                        Ok(Expr::new(
                            ExprKind::Assign {
                                target: Box::new(left),
                                value: Box::new(value),
                            },
                            span,
                        ))
                    }
                    _ => Err(ParserError::invalid_assignment_target(left.span)),
                }
            }

            // Compound assignment: +=, -=, *=, /=, %=
            TokenKind::PlusEqual
            | TokenKind::MinusEqual
            | TokenKind::StarEqual
            | TokenKind::SlashEqual
            | TokenKind::PercentEqual => {
                let op = match &token.kind {
                    TokenKind::PlusEqual => CompoundOp::Add,
                    TokenKind::MinusEqual => CompoundOp::Subtract,
                    TokenKind::StarEqual => CompoundOp::Multiply,
                    TokenKind::SlashEqual => CompoundOp::Divide,
                    TokenKind::PercentEqual => CompoundOp::Modulo,
                    _ => unreachable!(),
                };
                let value = self.parse_precedence(Precedence::Assignment)?;
                let span = start_span.merge(&value.span);

                match &left.kind {
                    ExprKind::Variable(_) | ExprKind::Member { .. } | ExprKind::Index { .. } => {
                        Ok(Expr::new(
                            ExprKind::CompoundAssign {
                                target: Box::new(left),
                                operator: op,
                                value: Box::new(value),
                            },
                            span,
                        ))
                    }
                    _ => Err(ParserError::invalid_assignment_target(left.span)),
                }
            }

            // Postfix increment/decrement: x++, x--
            TokenKind::PlusPlus => {
                let span = start_span.merge(&token.span);
                match &left.kind {
                    ExprKind::Variable(_) | ExprKind::Member { .. } | ExprKind::Index { .. } => {
                        Ok(Expr::new(ExprKind::PostfixIncrement(Box::new(left)), span))
                    }
                    _ => Err(ParserError::invalid_assignment_target(left.span)),
                }
            }
            TokenKind::MinusMinus => {
                let span = start_span.merge(&token.span);
                match &left.kind {
                    ExprKind::Variable(_) | ExprKind::Member { .. } | ExprKind::Index { .. } => {
                        Ok(Expr::new(ExprKind::PostfixDecrement(Box::new(left)), span))
                    }
                    _ => Err(ParserError::invalid_assignment_target(left.span)),
                }
            }

            // Call
            TokenKind::LeftParen => {
                let mut arguments = self.parse_arguments()?;
                self.expect(&TokenKind::RightParen)?;

                // Check for trailing block: obj.method(args) |params| body end
                if self.check(&TokenKind::Pipe) {
                    let block = self.parse_trailing_block()?;
                    arguments.push(Argument::Block(block));
                // Check for trailing brace block: obj.method(args) { body }
                } else if !self.no_trailing_brace
                    && self.check(&TokenKind::LeftBrace)
                    && !self.looks_like_hash_literal()
                {
                    let block = self.parse_trailing_brace_block()?;
                    arguments.push(Argument::Block(block));
                // Check for trailing do block: obj.method(args) do body end
                } else if self.check(&TokenKind::Do) {
                    let block = self.parse_trailing_do_block()?;
                    arguments.push(Argument::Block(block));
                }

                let span = start_span.merge(&self.previous_span());
                Ok(Expr::new(
                    ExprKind::Call {
                        callee: Box::new(left),
                        arguments,
                    },
                    span,
                ))
            }

            // Member access
            TokenKind::Dot => {
                let name = self.expect_identifier()?;
                let member_span = start_span.merge(&self.previous_span());

                // Check for trailing block: obj.method |params| body end
                if self.check(&TokenKind::Pipe) {
                    let block = self.parse_trailing_block()?;
                    let span = start_span.merge(&self.previous_span());
                    let member = Expr::new(
                        ExprKind::Member {
                            object: Box::new(left),
                            name,
                        },
                        member_span,
                    );
                    Ok(Expr::new(
                        ExprKind::Call {
                            callee: Box::new(member),
                            arguments: vec![Argument::Block(block)],
                        },
                        span,
                    ))
                // Check for trailing brace block: obj.method { body }
                } else if !self.no_trailing_brace
                    && self.check(&TokenKind::LeftBrace)
                    && !self.looks_like_hash_literal()
                {
                    let block = self.parse_trailing_brace_block()?;
                    let span = start_span.merge(&self.previous_span());
                    let member = Expr::new(
                        ExprKind::Member {
                            object: Box::new(left),
                            name,
                        },
                        member_span,
                    );
                    Ok(Expr::new(
                        ExprKind::Call {
                            callee: Box::new(member),
                            arguments: vec![Argument::Block(block)],
                        },
                        span,
                    ))
                // Check for trailing do block: obj.method do body end
                } else if self.check(&TokenKind::Do) {
                    let block = self.parse_trailing_do_block()?;
                    let span = start_span.merge(&self.previous_span());
                    let member = Expr::new(
                        ExprKind::Member {
                            object: Box::new(left),
                            name,
                        },
                        member_span,
                    );
                    Ok(Expr::new(
                        ExprKind::Call {
                            callee: Box::new(member),
                            arguments: vec![Argument::Block(block)],
                        },
                        span,
                    ))
                } else {
                    Ok(Expr::new(
                        ExprKind::Member {
                            object: Box::new(left),
                            name,
                        },
                        member_span,
                    ))
                }
            }

            // Safe navigation: obj&.field
            TokenKind::SafeNavigation => {
                let name = self.expect_identifier()?;
                let span = start_span.merge(&self.previous_span());
                Ok(Expr::new(
                    ExprKind::SafeMember {
                        object: Box::new(left),
                        name,
                    },
                    span,
                ))
            }

            // Qualified name access (e.g., Outer::Inner)
            TokenKind::DoubleColon => {
                let name = self.expect_identifier()?;
                let span = start_span.merge(&self.previous_span());
                Ok(Expr::new(
                    ExprKind::QualifiedName {
                        qualifier: Box::new(left),
                        name,
                    },
                    span,
                ))
            }

            // Index access
            TokenKind::LeftBracket => {
                let index = self.expression()?;
                self.expect(&TokenKind::RightBracket)?;
                let span = start_span.merge(&self.previous_span());
                Ok(Expr::new(
                    ExprKind::Index {
                        object: Box::new(left),
                        index: Box::new(index),
                    },
                    span,
                ))
            }

            _ => Err(ParserError::unexpected_token(
                "infix operator",
                format!("{}", token.kind),
                token.span,
            )),
        }
    }

    fn binary_expr(
        &mut self,
        left: Expr,
        operator: BinaryOp,
        precedence: Precedence,
    ) -> ParseResult<Expr> {
        let right = self.parse_precedence(precedence.next())?;
        let span = left.span.merge(&right.span);
        Ok(Expr::new(
            ExprKind::Binary {
                left: Box::new(left),
                operator,
                right: Box::new(right),
            },
            span,
        ))
    }

    pub(crate) fn parse_arguments(&mut self) -> ParseResult<Vec<Argument>> {
        let mut arguments = Vec::new();
        let mut seen_named = false;

        if !self.check(&TokenKind::RightParen) {
            loop {
                let start_span = self.current_span();

                // Check for block argument: &identifier or &{ ... }
                if self.check(&TokenKind::Ampersand) {
                    self.advance(); // consume &
                    let block_start = self.current_span();

                    // Check for inline block: &{ ... } or &(...)
                    if self.check(&TokenKind::LeftBrace) {
                        // Inline block: &{ ... }
                        let block_expr = self.parse_trailing_brace_block()?;
                        arguments.push(Argument::Block(block_expr));
                    } else if self.check(&TokenKind::LeftParen) {
                        // Inline parentheses block: &(...)
                        self.advance(); // consume (
                        let params = self.parse_lambda_params_list(&TokenKind::RightParen)?;
                        self.expect(&TokenKind::RightParen)?;
                        let block_expr = self.finish_parsing_lambda(params, block_start)?;
                        let _span = start_span.merge(&block_expr.span);
                        arguments.push(Argument::Block(block_expr));
                    } else if let TokenKind::SymbolLiteral(s) = &self.peek().kind {
                        // &:method shorthand: &:to_s → |__it| __it.to_s
                        let method_name = s.clone();
                        self.advance(); // consume the symbol
                        let span = start_span.merge(&self.previous_span());
                        let param = crate::ast::stmt::Parameter {
                            name: "__it".to_string(),
                            type_annotation: TypeAnnotation::new(
                                crate::ast::types::TypeKind::Named("Any".to_string()),
                                start_span,
                            ),
                            default_value: None,
                            span: start_span,
                            is_block_param: false,
                        };
                        let body_expr = Expr::new(
                            ExprKind::Member {
                                object: Box::new(Expr::new(
                                    ExprKind::Variable("__it".to_string()),
                                    start_span,
                                )),
                                name: method_name,
                            },
                            span,
                        );
                        let body_stmt = crate::ast::Stmt::new(
                            crate::ast::StmtKind::Expression(body_expr),
                            span,
                        );
                        let lambda = Expr::new(
                            ExprKind::Lambda {
                                params: vec![param],
                                return_type: None,
                                body: vec![body_stmt],
                            },
                            span,
                        );
                        arguments.push(Argument::Block(lambda));
                    } else if self.check(&TokenKind::Colon) {
                        // &:method_name shorthand (old syntax): &:identifier
                        self.advance(); // consume :
                        let method_name = self.expect_identifier()?;
                        let method_name = if self.match_token(&TokenKind::Question) {
                            format!("{}?", method_name)
                        } else {
                            method_name
                        };
                        let span = start_span.merge(&self.previous_span());
                        let param = crate::ast::stmt::Parameter {
                            name: "__it".to_string(),
                            type_annotation: TypeAnnotation::new(
                                crate::ast::types::TypeKind::Named("Any".to_string()),
                                start_span,
                            ),
                            default_value: None,
                            span: start_span,
                            is_block_param: false,
                        };
                        let body_expr = Expr::new(
                            ExprKind::Member {
                                object: Box::new(Expr::new(
                                    ExprKind::Variable("__it".to_string()),
                                    start_span,
                                )),
                                name: method_name,
                            },
                            span,
                        );
                        let body_stmt = crate::ast::Stmt::new(
                            crate::ast::StmtKind::Expression(body_expr),
                            span,
                        );
                        let lambda = Expr::new(
                            ExprKind::Lambda {
                                params: vec![param],
                                return_type: None,
                                body: vec![body_stmt],
                            },
                            span,
                        );
                        arguments.push(Argument::Block(lambda));
                    } else if let TokenKind::Identifier(_) = &self.peek().kind {
                        // Block reference: &identifier
                        let name = self.expect_identifier()?;
                        let span = start_span.merge(&self.previous_span());
                        let var_expr = Expr::new(ExprKind::Variable(name), span);
                        arguments.push(Argument::Block(var_expr));
                    } else {
                        return Err(ParserError::general(
                            "invalid block argument, expected identifier or block".to_string(),
                            block_start,
                        ));
                    }
                } else {
                    // Check for named argument: identifier followed by colon
                    if let TokenKind::Identifier(name) = &self.peek().kind {
                        let name = name.clone();
                        // Look ahead to see if next token is colon
                        let next_token = self.peek_nth(1);
                        if next_token.kind == TokenKind::Colon {
                            // This is a named argument
                            self.advance(); // consume identifier
                            self.advance(); // consume colon
                            let value = self.expression()?;
                            let span = start_span.merge(&value.span);
                            arguments.push(Argument::Named(NamedArgument { name, value, span }));
                            seen_named = true;
                        } else {
                            // This is a positional argument
                            let expr = self.expression()?;
                            if seen_named {
                                return Err(ParserError::general(
                                    "positional argument cannot follow named argument".to_string(),
                                    expr.span,
                                ));
                            }
                            arguments.push(Argument::Positional(expr));
                        }
                    } else {
                        // Positional argument (expression starting with literal, etc.)
                        let expr = self.expression()?;
                        if seen_named {
                            return Err(ParserError::general(
                                "positional argument cannot follow named argument".to_string(),
                                expr.span,
                            ));
                        }
                        arguments.push(Argument::Positional(expr));
                    }
                }

                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
        }

        Ok(arguments)
    }

    /// Parse a trailing block: `|params| body end` as a lambda expression.
    fn parse_trailing_block(&mut self) -> ParseResult<Expr> {
        let start_span = self.current_span();
        self.advance(); // consume |
        let params = self.parse_lambda_params_list(&TokenKind::Pipe)?;
        self.expect(&TokenKind::Pipe)?;
        self.finish_parsing_lambda(params, start_span)
    }

    /// Parse a trailing do block: `do body end` as a 0-param lambda expression.
    fn parse_trailing_do_block(&mut self) -> ParseResult<Expr> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Do)?;

        let mut statements = Vec::new();
        while !self.check(&TokenKind::End) && !self.is_at_end() {
            statements.push(self.statement()?);
        }
        self.expect(&TokenKind::End)?;

        let span = start_span.merge(&self.previous_span());

        Ok(Expr::new(
            ExprKind::Lambda {
                params: vec![],
                return_type: None,
                body: statements,
            },
            span,
        ))
    }

    /// Parse a trailing brace block: `{ body }` as a lambda expression.
    fn parse_trailing_brace_block(&mut self) -> ParseResult<Expr> {
        let start_span = self.current_span();
        self.expect(&TokenKind::LeftBrace)?;

        let mut statements = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            statements.push(self.statement()?);
        }
        self.expect(&TokenKind::RightBrace)?;

        let span = start_span.merge(&self.previous_span());

        // Create a lambda expression with the block as body
        Ok(Expr::new(
            ExprKind::Lambda {
                params: vec![],
                return_type: None,
                body: statements,
            },
            span,
        ))
    }

    fn parse_lambda(&mut self, start_span: crate::span::Span) -> ParseResult<Expr> {
        let params = self.parse_lambda_params_list(&TokenKind::Pipe)?;
        self.expect(&TokenKind::Pipe)?;

        self.finish_parsing_lambda(params, start_span)
    }

    fn parse_lambda_empty_params(&mut self, start_span: crate::span::Span) -> ParseResult<Expr> {
        self.finish_parsing_lambda(Vec::new(), start_span)
    }

    fn parse_stabby_lambda(&mut self, start_span: crate::span::Span) -> ParseResult<Expr> {
        // We consumed '->'
        let params = if self.match_token(&TokenKind::Or) {
            // -> || ...
            Vec::new()
        } else if self.match_token(&TokenKind::Pipe) {
            // -> |args| ...
            let p = self.parse_lambda_params_list(&TokenKind::Pipe)?;
            self.expect(&TokenKind::Pipe)?;
            p
        } else if self.match_token(&TokenKind::LeftParen) {
            // -> (args) ...
            let p = self.parse_lambda_params_list(&TokenKind::RightParen)?;
            self.expect(&TokenKind::RightParen)?;
            p
        } else if self.check(&TokenKind::Identifier(String::new())) {
            // -> x, y { body }
            let mut p = Vec::new();
            loop {
                let param_start = self.current_span();
                let name = self.expect_identifier()?;

                // Type annotation
                let type_annotation = if self.match_token(&TokenKind::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };

                p.push(crate::ast::stmt::Parameter {
                    name,
                    type_annotation: type_annotation.unwrap_or(TypeAnnotation::new(
                        crate::ast::types::TypeKind::Named("Any".to_string()),
                        param_start,
                    )),
                    default_value: None,
                    span: param_start.merge(&self.previous_span()),
                    is_block_param: false,
                });

                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            p
        } else {
            // -> { body } (0-arg)
            Vec::new()
        };

        self.finish_parsing_lambda(params, start_span)
    }

    fn parse_lambda_params_list(
        &mut self,
        end_token: &TokenKind,
    ) -> ParseResult<Vec<crate::ast::stmt::Parameter>> {
        let mut params = Vec::new();

        if !self.check(end_token) {
            loop {
                let param_start = self.current_span();
                let name = self.expect_identifier()?;

                // Type annotation
                let type_annotation = if self.match_token(&TokenKind::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };

                params.push(crate::ast::stmt::Parameter {
                    name,
                    type_annotation: type_annotation.unwrap_or(TypeAnnotation::new(
                        crate::ast::types::TypeKind::Named("Any".to_string()),
                        param_start,
                    )),
                    default_value: None,
                    span: param_start.merge(&self.previous_span()),
                    is_block_param: false,
                });

                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
        }

        Ok(params)
    }

    fn parse_anonymous_function(&mut self, start_span: crate::span::Span) -> ParseResult<Expr> {
        self.expect(&TokenKind::LeftParen)?;
        let params = self.parse_lambda_params_list(&TokenKind::RightParen)?;
        self.expect(&TokenKind::RightParen)?;

        self.finish_parsing_lambda(params, start_span)
    }

    fn finish_parsing_lambda(
        &mut self,
        params: Vec<crate::ast::stmt::Parameter>,
        start_span: crate::span::Span,
    ) -> ParseResult<Expr> {
        // Parse return type
        let return_type = if self.match_token(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        // Parse body
        let body = if self.check(&TokenKind::LeftBrace) && !self.looks_like_hash_literal() {
            self.advance(); // consume {
                            // Block body with braces
            let mut statements = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::RightBrace)?;
            statements
        } else if self.match_token(&TokenKind::End) {
            // Empty body with end
            Vec::new()
        } else {
            // Parse first statement, then decide: inline lambda or end-terminated body
            let first = self.statement()?;

            if self.check(&TokenKind::RightParen)
                || self.check(&TokenKind::Comma)
                || self.check(&TokenKind::RightBracket)
                || self.check(&TokenKind::RightBrace)
                || self.is_at_end()
            {
                // Closing delimiter → single-statement inline lambda
                vec![first]
            } else if self.match_token(&TokenKind::End) {
                // end-terminated single-statement body
                vec![first]
            } else {
                // Multi-statement end-terminated body
                let mut statements = vec![first];
                while !self.check(&TokenKind::End) && !self.is_at_end() {
                    statements.push(self.statement()?);
                }
                self.expect(&TokenKind::End)?;
                statements
            }
        };

        let span = start_span.merge(&self.previous_span());
        Ok(Expr::new(
            ExprKind::Lambda {
                params,
                return_type,
                body,
            },
            span,
        ))
    }

    fn parse_match_expression(&mut self, start_span: crate::span::Span) -> ParseResult<Expr> {
        let expression = self.expression()?;

        self.expect(&TokenKind::LeftBrace)?;

        let mut arms = Vec::new();
        while !self.check(&TokenKind::RightBrace) {
            let arm_start = self.current_span();

            let pattern = self.parse_match_pattern()?;

            let guard = if self.match_token(&TokenKind::If) {
                Some(self.expression()?)
            } else {
                None
            };

            self.expect(&TokenKind::FatArrow)?;
            let body = self.expression()?;
            let body_span = body.span;

            arms.push(MatchArm {
                pattern,
                guard,
                body,
                span: arm_start.merge(&body_span),
            });

            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }

        self.expect(&TokenKind::RightBrace)?;
        let span = start_span.merge(&self.previous_span());

        Ok(Expr::new(
            ExprKind::Match {
                expression: Box::new(expression),
                arms,
            },
            span,
        ))
    }

    fn parse_match_pattern(&mut self) -> ParseResult<MatchPattern> {
        use crate::lexer::TokenKind::*;

        let token_kind = self.peek().kind.clone();

        match token_kind {
            Identifier(s) if s == "_" => {
                self.advance();
                Ok(MatchPattern::Wildcard)
            }

            Int | Float | Bool | String | Void if self.peek_nth(1).kind == TokenKind::Colon => {
                let type_name = match self.advance().kind {
                    TokenKind::Int => "Int",
                    TokenKind::Float => "Float",
                    TokenKind::Bool => "Bool",
                    TokenKind::String => "String",
                    TokenKind::Void => "Void",
                    _ => {
                        return Err(ParserError::unexpected_token(
                            "type keyword".to_string(),
                            format!("{}", self.peek().kind),
                            self.current_span(),
                        ))
                    }
                };
                self.advance();
                let var_name = self.expect_identifier()?;
                Ok(MatchPattern::Typed {
                    name: var_name,
                    type_name: type_name.to_string(),
                })
            }

            IntLiteral(n) => {
                self.advance();
                Ok(MatchPattern::Literal(ExprKind::IntLiteral(n)))
            }

            FloatLiteral(n) => {
                self.advance();
                Ok(MatchPattern::Literal(ExprKind::FloatLiteral(n)))
            }

            StringLiteral(s) => {
                self.advance();
                Ok(MatchPattern::Literal(ExprKind::StringLiteral(s)))
            }

            BoolLiteral(b) => {
                self.advance();
                Ok(MatchPattern::Literal(ExprKind::BoolLiteral(b)))
            }

            Null => {
                self.advance();
                Ok(MatchPattern::Literal(ExprKind::Null))
            }

            LeftBracket => {
                self.advance();
                self.parse_array_pattern()
            }

            LeftBrace => {
                self.advance();
                self.parse_hash_pattern()
            }

            Identifier(s) if self.peek_nth(1).kind != TokenKind::Colon => {
                self.advance();
                Ok(MatchPattern::Variable(s))
            }

            Identifier(_) => {
                self.advance();
                if self.check(&TokenKind::LeftBrace) {
                    let type_name = match &self.previous().kind {
                        TokenKind::Identifier(s) => s.clone(),
                        _ => {
                            return Err(ParserError::unexpected_token(
                                "type name".to_string(),
                                format!("{}", self.previous().kind),
                                self.previous().span,
                            ))
                        }
                    };
                    let fields = self.parse_hash_pattern_fields()?;
                    self.expect(&TokenKind::RightBrace)?;
                    Ok(MatchPattern::Destructuring { type_name, fields })
                } else {
                    self.expect(&TokenKind::Colon)?;
                    let type_name = self.expect_identifier()?;
                    Ok(MatchPattern::Destructuring {
                        type_name: type_name.clone(),
                        fields: Vec::new(),
                    })
                }
            }

            _ => Err(ParserError::unexpected_token(
                "pattern".to_string(),
                format!("{}", self.peek().kind),
                self.current_span(),
            )),
        }
    }

    fn parse_hash_pattern_fields(&mut self) -> ParseResult<Vec<(String, MatchPattern)>> {
        let mut fields = Vec::new();

        if self.check(&TokenKind::RightBrace) {
            return Ok(fields);
        }

        loop {
            let field_name = self.expect_identifier()?;
            self.expect(&TokenKind::Colon)?;
            let field_pattern = self.parse_match_pattern()?;
            fields.push((field_name, field_pattern));

            if !self.match_token(&TokenKind::Comma) {
                break;
            }
            if self.check(&TokenKind::RightBrace) {
                break;
            }
        }

        Ok(fields)
    }

    fn parse_array_pattern(&mut self) -> ParseResult<MatchPattern> {
        if self.check(&TokenKind::RightBracket) {
            self.advance();
            return Ok(MatchPattern::Array {
                elements: Vec::new(),
                rest: None,
            });
        }

        let mut elements = Vec::new();
        let mut rest = None;

        loop {
            if self.check(&TokenKind::Spread) {
                self.advance();
                if let TokenKind::Identifier(name) = self.peek().kind.clone() {
                    self.advance();
                    rest = Some(name);
                } else {
                    return Err(ParserError::unexpected_token(
                        "identifier".to_string(),
                        format!("{}", self.peek().kind),
                        self.current_span(),
                    ));
                }
                break;
            }

            elements.push(self.parse_match_pattern()?);

            if !self.match_token(&TokenKind::Comma) {
                break;
            }
            if self.check(&TokenKind::RightBracket) {
                break;
            }
        }

        self.expect(&TokenKind::RightBracket)?;
        Ok(MatchPattern::Array { elements, rest })
    }

    fn parse_hash_pattern(&mut self) -> ParseResult<MatchPattern> {
        if self.check(&TokenKind::RightBrace) {
            self.advance();
            return Ok(MatchPattern::Hash {
                fields: Vec::new(),
                rest: None,
            });
        }

        let mut fields = Vec::new();
        let mut rest = None;

        loop {
            if self.check(&TokenKind::Spread) {
                self.advance();
                if let TokenKind::Identifier(name) = self.peek().kind.clone() {
                    self.advance();
                    rest = Some(name);
                } else {
                    return Err(ParserError::unexpected_token(
                        "identifier".to_string(),
                        format!("{}", self.peek().kind),
                        self.current_span(),
                    ));
                }
                break;
            }

            let field_name = self.expect_identifier()?;
            self.expect(&TokenKind::Colon)?;
            let field_pattern = self.parse_match_pattern()?;
            fields.push((field_name, field_pattern));

            if !self.match_token(&TokenKind::Comma) {
                break;
            }
            if self.check(&TokenKind::RightBrace) {
                break;
            }
        }

        self.expect(&TokenKind::RightBrace)?;
        Ok(MatchPattern::Hash { fields, rest })
    }

    fn parse_interpolated_string(
        &mut self,
        parts: Vec<String>,
        start_span: crate::span::Span,
    ) -> ParseResult<Expr> {
        use crate::ast::expr::InterpolatedPart;

        let mut interpolated_parts = Vec::new();

        for part in parts {
            if part.starts_with("#{") {
                // This is an expression - parse it
                // The format is #{expr}, so we need to extract expr and parse it
                let expr_content = &part[2..part.len() - 1]; // Remove #{ and }
                                                             // Parse the expression from the content
                let expr = self.parse_expression_from_string(expr_content)?;
                interpolated_parts.push(InterpolatedPart::Expression(expr));
            } else {
                interpolated_parts.push(InterpolatedPart::Literal(part));
            }
        }

        let span = start_span.merge(&self.previous_span());
        Ok(Expr::new(
            ExprKind::InterpolatedString(interpolated_parts),
            span,
        ))
    }

    /// Parse an expression from a string content (for interpolated strings)
    fn parse_expression_from_string(&mut self, content: &str) -> ParseResult<Expr> {
        // Fast path: simple identifier (e.g. #{name}) — skip Scanner+Parser
        let trimmed = content.trim();
        if !trimmed.is_empty()
            && trimmed
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && !trimmed.bytes().next().unwrap_or(0).is_ascii_digit()
        {
            let span = self.previous_span();
            return Ok(Expr::new(ExprKind::Variable(trimmed.to_string()), span));
        }

        // General path: full Scanner+Parser for complex expressions
        use crate::lexer::scanner::Scanner;
        let mut scanner = Scanner::new(content);
        let tokens = scanner.scan_tokens()?;

        let mut parser = crate::parser::Parser::new(tokens);
        parser.expression()
    }

    /// Check if a token is a valid first token for a command-style argument.
    /// Allows literals, identifiers, null, and this — but NOT operators,
    /// parens, braces, or keywords that would create ambiguity.
    fn is_command_arg(token_kind: &TokenKind) -> bool {
        matches!(
            token_kind,
            TokenKind::IntLiteral(_)
                | TokenKind::FloatLiteral(_)
                | TokenKind::DecimalLiteral(_)
                | TokenKind::StringLiteral(_)
                | TokenKind::InterpolatedString(_)
                | TokenKind::BacktickString(_)
                | TokenKind::BoolLiteral(_)
                | TokenKind::Identifier(_)
                | TokenKind::Null
                | TokenKind::This
        )
    }

    /// Parse arguments for command-style calls (without parentheses).
    /// e.g., `print "hello", "world"` or `print x, y`
    fn parse_command_arguments(&mut self) -> ParseResult<Vec<Argument>> {
        let mut arguments = Vec::new();

        if !Self::is_command_arg(&self.peek().kind) {
            return Ok(arguments);
        }

        let expr = self.expression()?;
        arguments.push(Argument::Positional(expr));

        while self.match_token(&TokenKind::Comma) {
            if !Self::is_command_arg(&self.peek().kind) {
                break;
            }
            let expr = self.expression()?;
            arguments.push(Argument::Positional(expr));
        }

        Ok(arguments)
    }
}
