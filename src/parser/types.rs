//! Type annotation parsing.

use crate::ast::*;
use crate::error::ParserError;
use crate::lexer::TokenKind;
use crate::span::Span;

use super::core::{ParseResult, Parser};

impl Parser {
    pub(crate) fn parse_type(&mut self) -> ParseResult<TypeAnnotation> {
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
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                if name == "Fn" && self.check(&TokenKind::LeftParen) {
                    self.parse_function_type(start_span)?
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

        // Check for array suffix [] and nullable suffix ?
        let mut result = base_type;
        while self.match_token(&TokenKind::LeftBracket) {
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
        })
    }
}
