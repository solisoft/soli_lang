//! Declaration parsing: classes, functions, interfaces, variables.

use crate::ast::*;
use crate::error::ParserError;
use crate::lexer::TokenKind;

use super::core::{ParseResult, Parser};

impl Parser {
    pub(crate) fn declaration(&mut self) -> ParseResult<Stmt> {
        if self.check(&TokenKind::Import) {
            self.import_declaration()
        } else if self.check(&TokenKind::Export) {
            self.export_declaration()
        } else if self.check(&TokenKind::Fn) {
            self.function_declaration()
        } else if self.check(&TokenKind::Class) {
            self.class_declaration()
        } else if self.check(&TokenKind::Interface) {
            self.interface_declaration()
        } else if self.check(&TokenKind::Let) {
            self.let_declaration()
        } else {
            self.statement()
        }
    }

    /// Parse an import declaration.
    /// Syntax:
    ///   import "path";                     -- import all exports
    ///   import { foo, bar } from "path";   -- named imports
    ///   import { foo as f } from "path";   -- aliased import
    ///   import * as mod from "path";       -- namespace import
    pub(crate) fn import_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Import)?;

        // Check what kind of import this is
        let specifier = if self.check(&TokenKind::StringLiteral(String::new())) {
            // import "path";
            ImportSpecifier::All
        } else if self.match_token(&TokenKind::Star) {
            // import * as name from "path";
            self.expect(&TokenKind::As)?;
            let name = self.expect_identifier()?;
            self.expect(&TokenKind::From)?;
            ImportSpecifier::Namespace(name)
        } else if self.match_token(&TokenKind::LeftBrace) {
            // import { items } from "path";
            let mut items = Vec::new();

            if !self.check(&TokenKind::RightBrace) {
                loop {
                    let item_span = self.current_span();
                    let name = self.expect_identifier()?;

                    let alias = if self.match_token(&TokenKind::As) {
                        Some(self.expect_identifier()?)
                    } else {
                        None
                    };

                    items.push(ImportItem {
                        name,
                        alias,
                        span: item_span.merge(&self.previous_span()),
                    });

                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                }
            }

            self.expect(&TokenKind::RightBrace)?;
            self.expect(&TokenKind::From)?;
            ImportSpecifier::Named(items)
        } else {
            return Err(ParserError::general(
                "Expected string path, '{', or '*' after 'import'",
                self.current_span(),
            ));
        };

        // Get the module path
        let path = match &self.peek().kind {
            TokenKind::StringLiteral(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            _ => {
                return Err(ParserError::unexpected_token(
                    "string path",
                    format!("{}", self.peek().kind),
                    self.current_span(),
                ));
            }
        };

        self.expect(&TokenKind::Semicolon)?;
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Import(ImportDecl {
                path,
                specifier,
                span,
            }),
            span,
        ))
    }

    /// Parse an export declaration.
    /// Syntax:
    ///   export fn name() { }
    ///   export class Name { }
    ///   export let name = value;
    ///   export interface Name { }
    pub(crate) fn export_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Export)?;

        // Parse the declaration being exported
        let inner = if self.check(&TokenKind::Fn) {
            self.function_declaration()?
        } else if self.check(&TokenKind::Class) {
            self.class_declaration()?
        } else if self.check(&TokenKind::Interface) {
            self.interface_declaration()?
        } else if self.check(&TokenKind::Let) {
            self.let_declaration()?
        } else {
            return Err(ParserError::general(
                "Expected 'fn', 'class', 'interface', or 'let' after 'export'",
                self.current_span(),
            ));
        };

        let span = start_span.merge(&self.previous_span());
        Ok(Stmt::new(StmtKind::Export(Box::new(inner)), span))
    }

    pub(crate) fn function_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Fn)?;

        let name = self.expect_identifier()?;
        let params = self.parse_parameters()?;

        let return_type = if self.match_token(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.block_statements()?;
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Function(FunctionDecl {
                name,
                params,
                return_type,
                body,
                span,
            }),
            span,
        ))
    }

    pub(crate) fn class_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Class)?;

        let name = self.expect_identifier()?;

        let superclass = if self.match_token(&TokenKind::Extends) {
            Some(self.expect_identifier()?)
        } else {
            None
        };

        let interfaces = if self.match_token(&TokenKind::Implements) {
            let mut interfaces = vec![self.expect_identifier()?];
            while self.match_token(&TokenKind::Comma) {
                interfaces.push(self.expect_identifier()?);
            }
            interfaces
        } else {
            Vec::new()
        };

        self.expect(&TokenKind::LeftBrace)?;

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut constructor = None;

        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let (visibility, is_static) = self.parse_modifiers();

            if self.check(&TokenKind::New) {
                if constructor.is_some() {
                    return Err(ParserError::general(
                        "Class already has a constructor",
                        self.current_span(),
                    ));
                }
                constructor = Some(self.parse_constructor()?);
            } else if self.check(&TokenKind::Fn) {
                methods.push(self.parse_method(visibility, is_static)?);
            } else {
                fields.push(self.parse_field(visibility, is_static)?);
            }
        }

        self.expect(&TokenKind::RightBrace)?;
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Class(ClassDecl {
                name,
                superclass,
                interfaces,
                fields,
                methods,
                constructor,
                span,
            }),
            span,
        ))
    }

    fn parse_modifiers(&mut self) -> (Visibility, bool) {
        let mut visibility = Visibility::Public;
        let mut is_static = false;

        loop {
            if self.match_token(&TokenKind::Public) {
                visibility = Visibility::Public;
            } else if self.match_token(&TokenKind::Private) {
                visibility = Visibility::Private;
            } else if self.match_token(&TokenKind::Protected) {
                visibility = Visibility::Protected;
            } else if self.match_token(&TokenKind::Static) {
                is_static = true;
            } else {
                break;
            }
        }

        (visibility, is_static)
    }

    fn parse_constructor(&mut self) -> ParseResult<ConstructorDecl> {
        let start_span = self.current_span();
        self.expect(&TokenKind::New)?;

        let params = self.parse_parameters()?;
        let body = self.block_statements()?;
        let span = start_span.merge(&self.previous_span());

        Ok(ConstructorDecl { params, body, span })
    }

    fn parse_method(&mut self, visibility: Visibility, is_static: bool) -> ParseResult<MethodDecl> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Fn)?;

        let name = self.expect_identifier()?;
        let params = self.parse_parameters()?;

        let return_type = if self.match_token(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.block_statements()?;
        let span = start_span.merge(&self.previous_span());

        Ok(MethodDecl {
            visibility,
            is_static,
            name,
            params,
            return_type,
            body,
            span,
        })
    }

    fn parse_field(&mut self, visibility: Visibility, is_static: bool) -> ParseResult<FieldDecl> {
        let start_span = self.current_span();
        let name = self.expect_identifier()?;

        self.expect(&TokenKind::Colon)?;
        let type_annotation = self.parse_type()?;

        let initializer = if self.match_token(&TokenKind::Equal) {
            Some(self.expression()?)
        } else {
            None
        };

        self.expect(&TokenKind::Semicolon)?;
        let span = start_span.merge(&self.previous_span());

        Ok(FieldDecl {
            visibility,
            is_static,
            name,
            type_annotation,
            initializer,
            span,
        })
    }

    pub(crate) fn interface_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Interface)?;

        let name = self.expect_identifier()?;
        self.expect(&TokenKind::LeftBrace)?;

        let mut methods = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            methods.push(self.parse_interface_method()?);
        }

        self.expect(&TokenKind::RightBrace)?;
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Interface(InterfaceDecl {
                name,
                methods,
                span,
            }),
            span,
        ))
    }

    fn parse_interface_method(&mut self) -> ParseResult<InterfaceMethod> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Fn)?;

        let name = self.expect_identifier()?;
        let params = self.parse_parameters()?;

        let return_type = if self.match_token(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(&TokenKind::Semicolon)?;
        let span = start_span.merge(&self.previous_span());

        Ok(InterfaceMethod {
            name,
            params,
            return_type,
            span,
        })
    }

    pub(crate) fn let_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Let)?;

        let name = self.expect_identifier()?;

        let type_annotation = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let initializer = if self.match_token(&TokenKind::Equal) {
            Some(self.expression()?)
        } else {
            None
        };

        self.expect(&TokenKind::Semicolon)?;
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Let {
                name,
                type_annotation,
                initializer,
            },
            span,
        ))
    }
}
