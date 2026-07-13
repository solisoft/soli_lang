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

            // Postfix `rescue` is normally a newline-insensitive inline modifier
            // (`expr rescue fallback`, possibly split across a continuation line).
            // But directly inside a `try`/`begin` body, a `rescue` that opens a new
            // line is a block-form catch clause, not a modifier on the preceding
            // statement — hand it back to the statement parser. (Ruby's rule.)
            if self.in_try_body
                && self.peek().kind == TokenKind::Rescue
                && self.peek().span.line != self.previous().span.line
            {
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
                // `@foo` is sugar for `this.foo`. `@@foo` (Ruby class vars) is intentionally rejected.
                if let Some(rest) = name.strip_prefix('@') {
                    if rest.starts_with('@') {
                        return Err(ParserError::general(
                            format!("class variables (`{}`) are not supported; use a static field or a module-level constant", name),
                            start_span,
                        ));
                    }
                    if rest.is_empty() {
                        return Err(ParserError::general(
                            "expected identifier after `@` (class variables `@@x` are not supported; use a static field)",
                            start_span,
                        ));
                    }
                    let this_expr = Expr::new(ExprKind::This, start_span);
                    return Ok(Expr::new(
                        ExprKind::Member {
                            object: Box::new(this_expr),
                            name: rest.to_string(),
                        },
                        start_span,
                    ));
                }

                // Command-style calls with named args: greet name: "Alice"
                // Same-line requirement matches the positional command-call branch below
                // and prevents swallowing `name: ...` from the next line.
                if let TokenKind::Identifier(_) = &self.peek().kind {
                    let next = self.peek();
                    if next.span.line == start_span.line
                        && self.peek_nth(1).kind == TokenKind::Colon
                    {
                        // Suppress trailing-do capture while parsing the values so a
                        // `do … end` binds to this command call, not the final value
                        // (`after_transition to: X do … end`).
                        let old_no_do = self.no_trailing_do;
                        self.no_trailing_do = true;
                        let args_result = self.parse_named_arguments_without_parens();
                        self.no_trailing_do = old_no_do;
                        let mut arguments = args_result?;
                        if self.check(&TokenKind::Do) {
                            let block = self.parse_trailing_do_block()?;
                            arguments.push(Argument::Block(block));
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
                }

                // Command-style calls: identifier followed by argument on the SAME LINE
                // e.g., print x, print "hello", puts result
                // Same-line requirement prevents ambiguity with multi-line bodies:
                //   fn foo
                //     bar    ← not a call to foo
                //   end
                let next_line = self.peek().span.line;
                if next_line == start_span.line && self.at_command_arg_start() {
                    let arguments = self.parse_command_arguments()?;

                    // Trailing `do … end` block on a no-paren command call:
                    // `state_machine :status do … end`, `event :pay do … end`,
                    // `after_transition to: X do … end`.
                    if self.check(&TokenKind::Do) {
                        let block = self.parse_trailing_do_block()?;
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
                let body_stmt = Stmt::new(StmtKind::Expression(body_expr), span, None);
                Ok(Expr::new(
                    ExprKind::Lambda {
                        params: vec![param],
                        return_type: None,
                        body: vec![body_stmt],
                    },
                    span,
                ))
            }

            // Primitive type keywords (Int, Float, Bool, Decimal, String) double
            // as expressions referencing the registered class globals, so users
            // can do `Int.class_eval do define_method(:double) { ... } end` or
            // call class methods on them. `Void` is intentionally excluded —
            // it's a return-type marker, not a runtime value.
            TokenKind::Int => Ok(Expr::new(ExprKind::Variable("Int".to_string()), start_span)),
            TokenKind::Float => Ok(Expr::new(
                ExprKind::Variable("Float".to_string()),
                start_span,
            )),
            TokenKind::Bool => Ok(Expr::new(
                ExprKind::Variable("Bool".to_string()),
                start_span,
            )),
            TokenKind::Decimal => Ok(Expr::new(
                ExprKind::Variable("Decimal".to_string()),
                start_span,
            )),
            TokenKind::String => Ok(Expr::new(
                ExprKind::Variable("String".to_string()),
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
            | TokenKind::Match => true,
            TokenKind::RightBrace => false, // Empty hash {}, not block
            TokenKind::LeftBrace => self.is_nested_block_expression(),
            _ => false,
        };

        if is_block {
            // Current token is already past the opening '{'
            // start_span is the span of the '{' token
            let _block_span = start_span.merge(&self.previous_span());
            // Current token should be either '}' (empty block) or the first statement
            // A nested block is its own scope: don't let an enclosing `begin` body's
            // newline-`rescue` rule bleed into it.
            let outer_in_try_body = self.in_try_body;
            self.in_try_body = false;
            let mut statements = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.in_try_body = outer_in_try_body;
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
                | Some(TokenKind::Match) => {
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
            TokenKind::LessLess => self.binary_expr(left, BinaryOp::Shovel, precedence),

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
                    // `foo(args) = value` desugars to `foo(args..., value)`. Used by the
                    // controller DSL for filtered hooks: `this.before_action(:show) = fn(req) {...}`
                    // becomes `this.before_action(:show, fn(req) {...})`.
                    ExprKind::Call { .. } => {
                        let ExprKind::Call {
                            callee,
                            mut arguments,
                        } = left.kind
                        else {
                            unreachable!()
                        };
                        arguments.push(Argument::Positional(value));
                        Ok(Expr::new(ExprKind::Call { callee, arguments }, span))
                    }
                    _ => Err(ParserError::invalid_assignment_target(left.span)),
                }
            }

            // Compound assignment: +=, -=, *=, /=, %=, ||=, &&=, ??=
            TokenKind::PlusEqual
            | TokenKind::MinusEqual
            | TokenKind::StarEqual
            | TokenKind::SlashEqual
            | TokenKind::PercentEqual
            | TokenKind::OrEqual
            | TokenKind::AndEqual
            | TokenKind::NullishEqual => {
                let op = match &token.kind {
                    TokenKind::PlusEqual => CompoundOp::Add,
                    TokenKind::MinusEqual => CompoundOp::Subtract,
                    TokenKind::StarEqual => CompoundOp::Multiply,
                    TokenKind::SlashEqual => CompoundOp::Divide,
                    TokenKind::PercentEqual => CompoundOp::Modulo,
                    TokenKind::OrEqual => CompoundOp::Or,
                    TokenKind::AndEqual => CompoundOp::And,
                    TokenKind::NullishEqual => CompoundOp::Coalesce,
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

            // Postfix rescue: expr rescue fallback
            TokenKind::Rescue => {
                let fallback = self.parse_precedence(Precedence::Assignment)?;
                let span = start_span.merge(&fallback.span);
                Ok(Expr::new(
                    ExprKind::Rescue {
                        expr: Box::new(left),
                        fallback: Box::new(fallback),
                    },
                    span,
                ))
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
                } else if !self.no_trailing_do && self.check(&TokenKind::Do) {
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
                let name_span = self.previous_span();
                let member_span = start_span.merge(&name_span);

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
                // Check for named args without parens: obj.method name: "Bob", age: 30
                // Same-line requirement: don't swallow tokens from the next statement.
                } else if let TokenKind::Identifier(_) = &self.peek().kind {
                    let peeked_line = self.peek().span.line;
                    if peeked_line == name_span.line && self.peek_nth(1).kind == TokenKind::Colon {
                        let arguments = self.parse_named_arguments_without_parens()?;
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
                                arguments,
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
                // Check for trailing do block: obj.method do body end
                } else if !self.no_trailing_do && self.check(&TokenKind::Do) {
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
                            None,
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
                            None,
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
                    // Named argument: a label followed by `:`. The label is an
                    // identifier, or a reserved word usable as a label
                    // (`from`/`in`) — mirroring `parse_command_arg`, so the paren
                    // form `f(from: x)` parses like the command form `f from: x`
                    // (and `soli fmt` can rewrite one to the other).
                    let label = match &self.peek().kind {
                        TokenKind::Identifier(name) => Some(name.clone()),
                        TokenKind::From => Some("from".to_string()),
                        TokenKind::In => Some("in".to_string()),
                        _ => None,
                    };
                    if let Some(name) = label.filter(|_| self.peek_nth(1).kind == TokenKind::Colon)
                    {
                        // This is a named argument
                        self.advance(); // consume label
                        self.advance(); // consume colon
                        let value = self.expression()?;
                        let span = start_span.merge(&value.span);
                        arguments.push(Argument::Named(NamedArgument { name, value, span }));
                        seen_named = true;
                    } else {
                        // Positional argument
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

    /// Parse named arguments without parentheses, for Ruby-style method calls.
    /// Caller must have already verified the lookahead is `Ident :`. Each iteration
    /// requires the same shape; anything else ends the list.
    fn parse_named_arguments_without_parens(&mut self) -> ParseResult<Vec<Argument>> {
        let mut arguments = Vec::new();

        loop {
            let start_span = self.current_span();
            let TokenKind::Identifier(name) = &self.peek().kind else {
                break;
            };
            if self.peek_nth(1).kind != TokenKind::Colon {
                break;
            }
            let name = name.clone();
            self.advance(); // identifier
            self.advance(); // colon
            let value = self.expression()?;
            let span = start_span.merge(&value.span);
            arguments.push(Argument::Named(NamedArgument { name, value, span }));
            if !self.match_token(&TokenKind::Comma) {
                break;
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

    /// Parse a trailing do block: `do body end` or `do |params| body end`
    /// (Ruby-style) as a lambda expression.
    pub(crate) fn parse_trailing_do_block(&mut self) -> ParseResult<Expr> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Do)?;

        let params = if self.match_token(&TokenKind::Pipe) {
            let p = self.parse_lambda_params_list(&TokenKind::Pipe)?;
            self.expect(&TokenKind::Pipe)?;
            p
        } else {
            Vec::new()
        };

        let mut statements = Vec::new();
        while !self.check(&TokenKind::End) && !self.is_at_end() {
            statements.push(self.statement()?);
        }
        self.expect(&TokenKind::End)?;

        let span = start_span.merge(&self.previous_span());

        Ok(Expr::new(
            ExprKind::Lambda {
                params,
                return_type: None,
                body: statements,
            },
            span,
        ))
    }

    /// Parse a trailing brace block: `{ body }` or `{ |params| body }` as a lambda expression.
    fn parse_trailing_brace_block(&mut self) -> ParseResult<Expr> {
        let start_span = self.current_span();
        self.expect(&TokenKind::LeftBrace)?;

        let params = if self.match_token(&TokenKind::Pipe) {
            let p = self.parse_lambda_params_list(&TokenKind::Pipe)?;
            self.expect(&TokenKind::Pipe)?;
            p
        } else {
            Vec::new()
        };

        let mut statements = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            statements.push(self.statement()?);
        }
        self.expect(&TokenKind::RightBrace)?;

        let span = start_span.merge(&self.previous_span());

        Ok(Expr::new(
            ExprKind::Lambda {
                params,
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
        // Parse the scrutinee without letting it swallow the match body's `{`
        // — otherwise `match Status.Active { Status.Active => ... }` mis-reads
        // the body as a Ruby-style block argument to the member access (the
        // arm patterns don't look like a hash literal). Same rule the
        // if/while/for condition parsers use.
        let expression = self.expression_no_trailing_brace()?;

        // Body delimiters: `match x { ... }` or the Ruby-style `match x ... end`
        // (both accepted, like `class`/`if`/`enum`).
        let used_brace = self.match_token(&TokenKind::LeftBrace);

        let mut arms = Vec::new();
        while !self.check(&TokenKind::RightBrace)
            && !self.check(&TokenKind::End)
            && !self.is_at_end()
        {
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

        // Close with the matching delimiter; lenient like `class` (accept
        // whichever closer appears) so a stray form still parses.
        if used_brace && self.check(&TokenKind::RightBrace) {
            self.advance();
        } else if self.match_token(&TokenKind::End) {
            // Ruby-style `end`.
        } else {
            self.expect(&TokenKind::RightBrace)?;
        }
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

            // Enum-variant pattern: `Status.Active` or `Status.Pending(r, ...)`.
            Identifier(enum_name) if self.peek_nth(1).kind == TokenKind::Dot => {
                self.advance(); // enum name
                self.advance(); // '.'
                let variant_name = self.expect_identifier()?;
                let bindings = if self.match_token(&TokenKind::LeftParen) {
                    let mut bindings = Vec::new();
                    if !self.check(&TokenKind::RightParen) {
                        loop {
                            bindings.push(self.parse_match_pattern()?);
                            if !self.match_token(&TokenKind::Comma) {
                                break;
                            }
                            if self.check(&TokenKind::RightParen) {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RightParen)?;
                    bindings
                } else {
                    Vec::new()
                };
                Ok(MatchPattern::EnumVariant {
                    enum_name,
                    variant_name,
                    bindings,
                })
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
                // Symbol args (`event :pay`) and lambda args (`guard fn() {…}`)
                // for declarative block DSLs like `state_machine`.
                | TokenKind::SymbolLiteral(_)
                | TokenKind::Fn
        )
    }

    /// Whether the parser is positioned at the start of a command-style
    /// argument. Broader than [`is_command_arg`] in one case: a reserved word
    /// used as a named-arg label (`from:`, `in:`), which needs two tokens of
    /// lookahead to disambiguate from the keyword's normal use.
    fn at_command_arg_start(&self) -> bool {
        if Self::is_command_arg(&self.peek().kind) {
            return true;
        }
        matches!(self.peek().kind, TokenKind::From | TokenKind::In)
            && matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenKind::Colon)
            )
    }

    /// Parse arguments for command-style calls (without parentheses).
    /// e.g., `print "hello", "world"` or `print x, y`
    /// Also used by class-level DSL statements (`edge from: "users", ...`).
    pub(crate) fn parse_command_arguments(&mut self) -> ParseResult<Vec<Argument>> {
        let mut arguments = Vec::new();

        if !self.at_command_arg_start() {
            return Ok(arguments);
        }

        arguments.push(self.parse_command_arg()?);

        while self.match_token(&TokenKind::Comma) {
            if !self.at_command_arg_start() {
                break;
            }
            arguments.push(self.parse_command_arg()?);
        }

        Ok(arguments)
    }

    /// Parse one command-style argument, recognizing the named form
    /// `label: value` (e.g. `transition from: X, to: Y`). The label is an
    /// identifier — or a reserved word like `from` — immediately followed by
    /// `:`; everything else is positional.
    fn parse_command_arg(&mut self) -> ParseResult<Argument> {
        let label_colon = matches!(
            self.tokens.get(self.current + 1).map(|t| &t.kind),
            Some(TokenKind::Colon)
        )
        .then(|| match &self.peek().kind {
            TokenKind::Identifier(name) => Some(name.clone()),
            // Reserved words usable as named-argument labels (Ruby-style), so
            // `transition from: X` reads naturally even though `from` is a keyword.
            TokenKind::From => Some("from".to_string()),
            TokenKind::In => Some("in".to_string()),
            _ => None,
        })
        .flatten();

        // A trailing `do … end` belongs to the command call, not to an argument
        // value (`after_transition to: X do … end` → block binds to
        // `after_transition`). Suppress do-block capture while parsing the value,
        // restoring the flag before propagating any parse error.
        let old_no_do = self.no_trailing_do;
        self.no_trailing_do = true;

        if let Some(label) = label_colon {
            let start = self.current_span();
            self.advance(); // label
            self.advance(); // colon
            let value = self.expression();
            self.no_trailing_do = old_no_do;
            let value = value?;
            let span = start.merge(&self.previous_span());
            return Ok(Argument::Named(NamedArgument {
                name: label,
                value,
                span,
            }));
        }

        let value = self.expression();
        self.no_trailing_do = old_no_do;
        Ok(Argument::Positional(value?))
    }
}

#[cfg(test)]
mod at_sigil_tests {
    use crate::ast::expr::{Argument, ExprKind};
    use crate::ast::stmt::StmtKind;
    use crate::ast::Program;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    fn parse(src: &str) -> Result<Program, String> {
        let tokens = Scanner::new(src)
            .scan_tokens()
            .map_err(|e| format!("lex: {:?}", e))?;
        Parser::new(tokens)
            .parse()
            .map_err(|e| format!("parse: {:?}", e))
    }

    fn first_expr(program: &Program) -> &ExprKind {
        match &program.statements[0].kind {
            StmtKind::Expression(e) => &e.kind,
            other => panic!("expected expression statement, got {:?}", other),
        }
    }

    // `@foo` must desugar to the same AST as `this.foo` so the rest of the
    // toolchain (linter, interpreter, VM) treats both identically.
    #[test]
    fn at_sigil_desugars_to_this_member() {
        let program = parse("@title").expect("parses");
        match first_expr(&program) {
            ExprKind::Member { object, name } => {
                assert_eq!(name, "title");
                assert!(matches!(object.kind, ExprKind::This));
            }
            other => panic!("expected Member {{ This, \"title\" }}, got {:?}", other),
        }
    }

    #[test]
    fn at_sigil_assignment_becomes_member_assignment() {
        let program = parse("@count = 5").expect("parses");
        match first_expr(&program) {
            ExprKind::Assign { target, .. } => match &target.kind {
                ExprKind::Member { object, name } => {
                    assert_eq!(name, "count");
                    assert!(matches!(object.kind, ExprKind::This));
                }
                other => panic!("expected Member target, got {:?}", other),
            },
            other => panic!("expected Assign, got {:?}", other),
        }
    }

    // Ruby class variables (`@@x`) aren't backed by any Soli feature, so the
    // parser must reject them loudly rather than silently turning them into a
    // field named `"@x"`.
    #[test]
    fn double_at_is_parse_error() {
        let err = parse("@@shared").expect_err("must not parse");
        assert!(
            err.contains("not supported") || err.contains("expected identifier after `@`"),
            "expected class-var rejection, got: {}",
            err
        );
    }

    // `@foo()` must compose the sugar with the postfix call form, producing a
    // call on the desugared member access.
    #[test]
    fn at_sigil_with_call_composes_postfix() {
        let program = parse("@greet()").expect("parses");
        match first_expr(&program) {
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::Member { object, name } => {
                    assert_eq!(name, "greet");
                    assert!(matches!(object.kind, ExprKind::This));
                }
                other => panic!("expected Member callee, got {:?}", other),
            },
            other => panic!("expected Call, got {:?}", other),
        }
    }

    // `foo(args) = value` desugars to `foo(args..., value)`. Powers the controller
    // filtered-hook DSL: `this.before_action(:show, :edit) = fn(req) {...}` →
    // `this.before_action(:show, :edit, fn(req) {...})`.
    #[test]
    fn call_assignment_desugars_to_appended_argument() {
        let program = parse("this.before_action(:show) = 42").expect("parses");
        match first_expr(&program) {
            ExprKind::Call { callee, arguments } => {
                match &callee.kind {
                    ExprKind::Member { object, name } => {
                        assert_eq!(name, "before_action");
                        assert!(matches!(object.kind, ExprKind::This));
                    }
                    other => panic!("expected Member callee, got {:?}", other),
                }
                assert_eq!(arguments.len(), 2, "original arg + appended value");
                match &arguments[1] {
                    crate::ast::expr::Argument::Positional(e) => {
                        assert!(matches!(e.kind, ExprKind::IntLiteral(42)));
                    }
                    other => panic!("expected Positional trailing arg, got {:?}", other),
                }
            }
            other => panic!("expected Call, got {:?}", other),
        }
    }

    // `@foo.bar` must chain — outer Member wraps the desugared `this.foo`.
    #[test]
    fn at_sigil_chains_member_access() {
        let program = parse("@inner.label").expect("parses");
        match first_expr(&program) {
            ExprKind::Member { object, name } => {
                assert_eq!(name, "label");
                match &object.kind {
                    ExprKind::Member {
                        object: inner_obj,
                        name: inner_name,
                    } => {
                        assert_eq!(inner_name, "inner");
                        assert!(matches!(inner_obj.kind, ExprKind::This));
                    }
                    other => panic!("expected inner Member, got {:?}", other),
                }
            }
            other => panic!("expected Member, got {:?}", other),
        }
    }

    // --- State machine DSL surface syntax (§6) ------------------------------

    #[test]
    fn command_call_with_named_args() {
        // `transition from: a, to: b` → a call with two named arguments.
        let program = parse("transition from: a, to: b").expect("parses");
        match first_expr(&program) {
            ExprKind::Call { callee, arguments } => {
                assert!(matches!(callee.kind, ExprKind::Variable(ref n) if n == "transition"));
                assert_eq!(arguments.len(), 2);
                assert!(matches!(&arguments[0], Argument::Named(n) if n.name == "from"));
                assert!(matches!(&arguments[1], Argument::Named(n) if n.name == "to"));
            }
            other => panic!("expected Call, got {:?}", other),
        }
    }

    #[test]
    fn command_call_with_symbol_and_do_block() {
        // `event :pay do … end` → a call with a positional symbol + a block.
        let program = parse("event :pay do\n  transition from: a, to: b\nend").expect("parses");
        match first_expr(&program) {
            ExprKind::Call { callee, arguments } => {
                assert!(matches!(callee.kind, ExprKind::Variable(ref n) if n == "event"));
                assert!(matches!(&arguments[0], Argument::Positional(_)));
                assert!(matches!(arguments.last(), Some(Argument::Block(_))));
            }
            other => panic!("expected Call, got {:?}", other),
        }
    }

    #[test]
    fn named_command_call_with_trailing_do_block() {
        // The `do` binds to the command call, NOT to the final value `b`.
        let program = parse("after_transition to: b do\n  this.x = 1\nend").expect("parses");
        match first_expr(&program) {
            ExprKind::Call { callee, arguments } => {
                assert!(
                    matches!(callee.kind, ExprKind::Variable(ref n) if n == "after_transition")
                );
                assert!(matches!(&arguments[0], Argument::Named(n) if n.name == "to"));
                assert!(matches!(arguments.last(), Some(Argument::Block(_))));
            }
            other => panic!("expected Call, got {:?}", other),
        }
    }

    #[test]
    fn command_call_with_lambda_arg() {
        // `guard fn() { … }` → a call with a positional lambda argument.
        let program = parse("guard fn() { this.total > 0 }").expect("parses");
        match first_expr(&program) {
            ExprKind::Call { callee, arguments } => {
                assert!(matches!(callee.kind, ExprKind::Variable(ref n) if n == "guard"));
                assert!(
                    matches!(&arguments[0], Argument::Positional(e) if matches!(e.kind, ExprKind::Lambda { .. }))
                );
            }
            other => panic!("expected Call, got {:?}", other),
        }
    }
}
