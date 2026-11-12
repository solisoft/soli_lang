//! Type annotation parsing.

use crate::ast::*;
use crate::error::ParserError;
use crate::lexer::TokenKind;
use crate::span::Span;

use super::core::{ParseResult, Parser};

impl Parser {
    pub(crate) fn parse_type(&mut self) -> ParseResult<TypeAnnotation> {
        self.enter_depth("type")?;
        let result = self.parse_union_type();
        self.exit_depth();
        result
    }

    /// `A | B | nil`: members joined by `|` on the line of the type. A `|`
    /// on the next line is code (a block's parameters), and inside `|x: T|`
    /// parameters the `|` closes them (see `no_union_type`).
    fn parse_union_type(&mut self) -> ParseResult<TypeAnnotation> {
        let first = self.parse_type_inner()?;
        if self.no_union_type {
            return Ok(first);
        }
        let mut members = vec![first];
        while self.check(&TokenKind::Pipe)
            && self.peek().span.line == self.previous_span().line
            && starts_a_type(&self.peek_nth(1).kind)
        {
            self.advance();
            members.push(self.parse_type_inner()?);
        }
        if members.len() == 1 {
            return Ok(members.pop().unwrap_or_else(|| unreachable!()));
        }
        let span = members[0].span.merge(&members[members.len() - 1].span);
        Ok(TypeAnnotation::new(TypeKind::Union(members), span))
    }

    fn parse_type_inner(&mut self) -> ParseResult<TypeAnnotation> {
        let start_span = self.current_span();

        let base_type = match &self.peek().kind {
            TokenKind::Int => {
                self.advance();
                TypeAnnotation::new(TypeKind::Named("Int".to_string()), start_span)
            }
            TokenKind::Float => {
                self.advance();
                TypeAnnotation::new(TypeKind::Named("Float".to_string()), start_span)
            }
            TokenKind::Bool => {
                self.advance();
                TypeAnnotation::new(TypeKind::Named("Bool".to_string()), start_span)
            }
            TokenKind::String => {
                self.advance();
                TypeAnnotation::new(TypeKind::Named("String".to_string()), start_span)
            }
            TokenKind::Void => {
                self.advance();
                TypeAnnotation::new(TypeKind::Void, start_span)
            }
            // `nil` / `null` as a type: the member of a union that admits nil.
            TokenKind::Null => {
                self.advance();
                TypeAnnotation::new(TypeKind::Named("Null".to_string()), start_span)
            }
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                if name == "Fn" && self.check(&TokenKind::LeftParen) {
                    self.parse_function_type(start_span)?
                } else if self.type_params.contains(&name) {
                    TypeAnnotation::new(TypeKind::Param(name), start_span)
                } else if self
                    .type_params
                    .iter()
                    .any(|p| name.strip_suffix('?') == Some(p))
                {
                    let base_name = name[..name.len() - 1].to_string();
                    let base = TypeAnnotation::new(TypeKind::Param(base_name), start_span);
                    TypeAnnotation::new(TypeKind::Nullable(Box::new(base)), start_span)
                } else if self.check(&TokenKind::Less)
                    && self.peek().span.start == self.previous_span().end
                {
                    self.parse_type_arguments(name, start_span)?
                } else if name.ends_with('?') {
                    // Handle nullable suffix attached by lexer (e.g., "String?" -> Nullable(Named("String")))
                    let base_name = name[..name.len() - 1].to_string();
                    let base = TypeAnnotation::new(TypeKind::Named(base_name), start_span);
                    TypeAnnotation::new(TypeKind::Nullable(Box::new(base)), start_span)
                } else {
                    TypeAnnotation::new(TypeKind::Named(name), start_span)
                }
            }
            TokenKind::Fn => {
                self.advance();
                self.parse_function_type(start_span)?
            }
            TokenKind::LeftParen => {
                self.advance();
                // Inside parentheses `|` is a union again, even in `|x: …|`.
                let no_union_type = std::mem::replace(&mut self.no_union_type, false);
                let mut params = Vec::new();
                let parsed = (|| {
                    if !self.check(&TokenKind::RightParen) {
                        params.push(self.parse_type()?);
                        while self.match_token(&TokenKind::Comma) {
                            params.push(self.parse_type()?);
                        }
                    }
                    self.expect(&TokenKind::RightParen)
                })();
                self.no_union_type = no_union_type;
                parsed?;
                // `(Int | String)`: a type in parentheses, not a function type.
                if params.len() == 1 && !self.check(&TokenKind::Arrow) {
                    let mut grouped = params.pop().unwrap_or_else(|| unreachable!());
                    grouped.span = start_span.merge(&self.previous_span());
                    return self.type_suffixes(start_span, grouped);
                }
                self.expect(&TokenKind::Arrow)?;
                let return_type = Box::new(self.parse_type()?);
                let span = start_span.merge(&return_type.span);
                TypeAnnotation::new(
                    TypeKind::Function {
                        params,
                        return_type,
                    },
                    span,
                )
            }
            _ => {
                return Err(ParserError::unexpected_token(
                    "type",
                    format!("{}", self.peek().kind),
                    self.current_span(),
                ));
            }
        };

        self.type_suffixes(start_span, base_type)
    }

    /// `[]` and `?` after a type.
    fn type_suffixes(
        &mut self,
        start_span: Span,
        base_type: TypeAnnotation,
    ) -> ParseResult<TypeAnnotation> {
        // Check for array suffix [] and nullable suffix ?
        //
        // `[]` is a suffix only when written against the type (`Int[]`). After
        // a space or a line break, the `[` starts the next expression: in
        // `def f(x) -> Bool` followed by a body line `["a", "b"].includes?(x)`,
        // the array literal is the body, not part of the return type.
        let mut result = base_type;
        while self.check(&TokenKind::LeftBracket)
            && self.peek().span.start == self.previous_span().end
            && self.peek_nth(1).kind == TokenKind::RightBracket
        {
            self.advance();
            self.expect(&TokenKind::RightBracket)?;
            let span = start_span.merge(&self.previous_span());
            result = TypeAnnotation::new(TypeKind::Array(Box::new(result)), span);
        }

        // Check for nullable suffix ?
        if self.match_token(&TokenKind::Question) {
            let span = start_span.merge(&self.previous_span());
            result = TypeAnnotation::new(TypeKind::Nullable(Box::new(result)), span);
        }

        Ok(result)
    }

    /// `Box<Int>`, `Hash<String, Int>`, `Array<Int>`: the `<` is the
    /// lookahead, written against the name.
    fn parse_type_arguments(
        &mut self,
        name: String,
        start_span: Span,
    ) -> ParseResult<TypeAnnotation> {
        self.expect(&TokenKind::Less)?;
        // Inside the brackets `|` is a union again, even in `|x: …|`.
        let no_union_type = std::mem::replace(&mut self.no_union_type, false);
        let mut args = Vec::new();
        let parsed = (|| {
            args.push(self.parse_type()?);
            while self.match_token(&TokenKind::Comma) {
                args.push(self.parse_type()?);
            }
            self.expect(&TokenKind::Greater)
        })();
        self.no_union_type = no_union_type;
        parsed?;
        let span = start_span.merge(&self.previous_span());
        let kind = match (name.as_str(), args.len()) {
            ("Array", 1) => TypeKind::Array(Box::new(args.remove(0))),
            ("Hash", 2) => {
                let value_type = Box::new(args.remove(1));
                TypeKind::Hash {
                    key_type: Box::new(args.remove(0)),
                    value_type,
                }
            }
            _ => TypeKind::Generic(Box::new(GenericType { name, args })),
        };
        Ok(TypeAnnotation::new(kind, span))
    }

    /// `<T>` / `<K, V>` after a class or `def` name: type parameters, read
    /// only when the brackets close on a list of names — `class B<A` is
    /// inheritance, as it always was.
    pub(crate) fn parse_type_parameters(&mut self) -> Vec<String> {
        if !self.check(&TokenKind::Less) {
            return Vec::new();
        }
        let mut names = Vec::new();
        let mut i = 1;
        loop {
            match &self.peek_nth(i).kind {
                TokenKind::Identifier(name) => names.push(name.clone()),
                _ => return Vec::new(),
            }
            match &self.peek_nth(i + 1).kind {
                TokenKind::Comma => i += 2,
                TokenKind::Greater => {
                    for _ in 0..=i + 1 {
                        self.advance();
                    }
                    return names;
                }
                _ => return Vec::new(),
            }
        }
    }

    fn parse_function_type(&mut self, start_span: Span) -> ParseResult<TypeAnnotation> {
        self.expect(&TokenKind::LeftParen)?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            params.push(self.parse_type()?);
            while self.match_token(&TokenKind::Comma) {
                params.push(self.parse_type()?);
            }
        }
        self.expect(&TokenKind::RightParen)?;
        self.expect(&TokenKind::Arrow)?;
        let return_type = Box::new(self.parse_type()?);
        let span = start_span.merge(&return_type.span);
        Ok(TypeAnnotation::new(
            TypeKind::Function {
                params,
                return_type,
            },
            span,
        ))
    }

    pub(crate) fn parse_parameters(&mut self) -> ParseResult<Vec<Parameter>> {
        // Exact capacity: the AST outlives the parse — shared by every
        // worker for the process's life — so growth slack would be kept.
        self.parse_parameters_grown().map(|mut list| {
            list.shrink_to_fit();
            list
        })
    }

    fn parse_parameters_grown(&mut self) -> ParseResult<Vec<Parameter>> {
        if !self.match_token(&TokenKind::LeftParen) {
            return Ok(Vec::new());
        }

        let mut params = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            params.push(self.parse_parameter()?);
            while self.match_token(&TokenKind::Comma) {
                params.push(self.parse_parameter()?);
            }
        }

        self.expect(&TokenKind::RightParen)?;
        Ok(params)
    }

    fn parse_parameter(&mut self) -> ParseResult<Parameter> {
        let start_span = self.current_span();

        let is_block_param = self.match_token(&TokenKind::Ampersand);

        let name = self.expect_identifier()?;

        let type_annotation = if self.match_token(&TokenKind::Colon) {
            self.parse_type()?
        } else {
            TypeAnnotation::new(
                crate::ast::types::TypeKind::Named("Any".to_string()),
                self.previous_span(),
            )
        };

        let default_value = if self.match_token(&TokenKind::Equal) {
            Some(self.expression()?)
        } else {
            None
        };

        let span = start_span.merge(&self.previous_span());

        Ok(Parameter {
            name,
            type_annotation,
            default_value,
            span,
            is_block_param,
        })
    }
}

/// Whether `kind` can begin a type: what may follow a union's `|`.
fn starts_a_type(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Int
            | TokenKind::Float
            | TokenKind::Bool
            | TokenKind::String
            | TokenKind::Void
            | TokenKind::Null
            | TokenKind::Identifier(_)
            | TokenKind::Fn
            | TokenKind::LeftParen
    )
}
