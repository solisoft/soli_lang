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
        } else if self.check(&TokenKind::Enum) {
            self.enum_declaration()
        } else if self.check(&TokenKind::Interface) {
            self.interface_declaration()
        } else if self.check(&TokenKind::Let) {
            self.let_declaration()
        } else if self.check(&TokenKind::Const) {
            self.const_declaration()
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

        self.match_token(&TokenKind::Semicolon);
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Import(ImportDecl {
                path,
                specifier,
                span,
            }),
            span,
            None,
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
        Ok(Stmt::new(StmtKind::Export(Box::new(inner)), span, None))
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

        let body = self.parse_function_body()?;
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
            None,
        ))
    }

    pub(crate) fn class_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Class)?;

        let name = self.expect_identifier()?;

        let superclass =
            if self.match_token(&TokenKind::Extends) || self.match_token(&TokenKind::Less) {
                Some(self.expect_identifier()?)
            } else {
                None
            };

        let interfaces =
            if self.match_token(&TokenKind::Implements) || self.match_token(&TokenKind::Tilde) {
                let mut interfaces = vec![self.expect_identifier()?];
                while self.match_token(&TokenKind::Comma) {
                    interfaces.push(self.expect_identifier()?);
                }
                interfaces
            } else {
                Vec::new()
            };

        if self.match_token(&TokenKind::End) {
            let span = start_span.merge(&self.previous_span());
            return Ok(Stmt::new(
                StmtKind::Class(ClassDecl {
                    name,
                    superclass,
                    interfaces,
                    fields: Vec::new(),
                    methods: Vec::new(),
                    constructor: None,
                    static_block: None,
                    class_statements: Vec::new(),
                    nested_classes: Vec::new(),
                    span,
                }),
                span,
                None,
            ));
        }

        // Allow optional opening brace
        self.match_token(&TokenKind::LeftBrace);

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut constructor = None;
        let mut static_block = None;
        let mut class_statements = Vec::new();
        let mut nested_classes = Vec::new();

        while !self.check(&TokenKind::RightBrace)
            && !self.check(&TokenKind::End)
            && !self.is_at_end()
        {
            if self.check(&TokenKind::Static) {
                // Check if this is a static block: static { ... }
                if let Some(next) = self.tokens.get(self.current + 1) {
                    if matches!(next.kind, TokenKind::LeftBrace) {
                        static_block = Some(self.parse_static_block()?);
                        continue;
                    }
                }
            }

            if self.check(&TokenKind::Class) {
                // Detect Ruby-style singleton-class block: class << self ... end
                let next1 = self.tokens.get(self.current + 1).map(|t| &t.kind);
                let next2 = self.tokens.get(self.current + 2).map(|t| &t.kind);
                if matches!(next1, Some(TokenKind::LessLess))
                    && matches!(next2, Some(TokenKind::SelfKeyword))
                {
                    self.parse_singleton_class_block(&mut methods)?;
                    continue;
                }
            }

            let (visibility, is_static, is_const) = self.parse_modifiers();

            if self.check(&TokenKind::New) {
                if constructor.is_some() {
                    return Err(ParserError::general(
                        "Class already has a constructor",
                        self.current_span(),
                    ));
                }
                constructor = Some(self.parse_constructor()?);
            } else if self.check(&TokenKind::Class) {
                // Handle nested class declaration
                let nested_class = self.class_declaration()?;
                if let StmtKind::Class(nested_class_decl) = nested_class.kind {
                    nested_classes.push(nested_class_decl);
                }
            } else if self.check(&TokenKind::Fn) {
                methods.push(self.parse_method(visibility, is_static)?);
            } else if self.is_class_level_statement() {
                // Parse class-level statements like validates(...), before_save(...)
                class_statements.push(self.parse_class_level_statement()?);
            } else {
                fields.push(self.parse_field(visibility, is_static, is_const)?);
            }
        }

        if self.match_token(&TokenKind::End) {
            // Class body ends with 'end'
        } else {
            self.expect(&TokenKind::RightBrace)?;
        }
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Class(ClassDecl {
                name,
                superclass,
                interfaces,
                fields,
                methods,
                constructor,
                static_block,
                class_statements,
                nested_classes,
                span,
            }),
            span,
            None,
        ))
    }

    /// Parse an enum declaration:
    ///   enum Status { Active, Archived, Pending(reason: String) def label() ... end }
    /// The body is a mix of variant declarations (comma/newline separated) and
    /// instance methods (`def`/`fn`). Closes with `}` or `end`.
    pub(crate) fn enum_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Enum)?;
        let name = self.expect_identifier()?;

        // Allow optional opening brace (Ruby `end` form also supported).
        self.match_token(&TokenKind::LeftBrace);

        let mut variants = Vec::new();
        let mut methods = Vec::new();

        while !self.check(&TokenKind::RightBrace)
            && !self.check(&TokenKind::End)
            && !self.is_at_end()
        {
            // Allow stray separators between members.
            if self.match_token(&TokenKind::Comma) || self.match_token(&TokenKind::Semicolon) {
                continue;
            }

            let (visibility, is_static, _is_const) = self.parse_modifiers();

            // A method (the "rich" scope): `def`/`fn` or `static def`.
            if self.check(&TokenKind::Fn) {
                methods.push(self.parse_method(visibility, is_static)?);
                continue;
            }

            // Otherwise a variant: Name or Name(field: Type, ...)
            let variant_span = self.current_span();
            let variant_name = self.expect_identifier()?;
            let payload = if self.match_token(&TokenKind::LeftParen) {
                let mut payload = Vec::new();
                if !self.check(&TokenKind::RightParen) {
                    loop {
                        let field_span = self.current_span();
                        let field_name = self.expect_identifier()?;
                        let type_annotation = if self.match_token(&TokenKind::Colon) {
                            Some(self.parse_type()?)
                        } else {
                            None
                        };
                        payload.push(EnumPayloadField {
                            name: field_name,
                            type_annotation,
                            span: field_span.merge(&self.previous_span()),
                        });
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                        if self.check(&TokenKind::RightParen) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RightParen)?;
                payload
            } else {
                Vec::new()
            };

            variants.push(EnumVariantDecl {
                name: variant_name,
                payload,
                span: variant_span.merge(&self.previous_span()),
            });
        }

        if self.match_token(&TokenKind::End) {
            // Enum body ended with 'end'.
        } else {
            self.expect(&TokenKind::RightBrace)?;
        }
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Enum(EnumDecl {
                name,
                variants,
                methods,
                span,
            }),
            span,
            None,
        ))
    }

    /// Check if the current token starts a class-level statement (e.g., validates(...))
    fn is_class_level_statement(&self) -> bool {
        // Check for identifier followed by left paren
        if let TokenKind::Identifier(name) = &self.peek().kind {
            // List of recognized class-level function names
            let class_level_names = [
                "validates",
                "before_save",
                "after_save",
                "before_create",
                "after_create",
                "before_update",
                "after_update",
                "before_delete",
                "after_delete",
                "has_many",
                "has_one",
                "belongs_to",
                "has_and_belongs_to_many",
                "uploader",
                "scope",
                "attr_accessible",
                "encrypts",
                "enum_field",
                "state_machine",
            ];
            // Bare class-level macros (no parentheses needed)
            let bare_class_level_names = ["soft_delete"];
            if bare_class_level_names.contains(&name.as_str()) {
                return true;
            }

            if class_level_names.contains(&name.as_str()) {
                // Look ahead for left paren, symbol, or string (for no-parens form:
                // belongs_to :user, before_save "method")
                if let Some(next) = self.tokens.get(self.current + 1) {
                    return matches!(
                        next.kind,
                        TokenKind::LeftParen
                            | TokenKind::SymbolLiteral(_)
                            | TokenKind::StringLiteral(_)
                    );
                }
            }
        }
        false
    }

    /// Parse a class-level statement like validates(...), before_save(...), or soft_delete
    fn parse_class_level_statement(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();

        // Check for bare class-level macro (no parentheses): e.g., soft_delete
        // or e.g., belongs_to :user, before_save "method_name"
        if let TokenKind::Identifier(name) = &self.peek().kind {
            let bare_names = ["soft_delete"];
            if bare_names.contains(&name.as_str()) {
                let name = name.clone();
                self.advance(); // consume the identifier
                let span = start_span.merge(&self.previous_span());
                self.match_token(&TokenKind::Semicolon);
                // Wrap as a zero-arg Call so the class executor can process it
                let callee = Expr::new(ExprKind::Variable(name), start_span);
                let call = Expr::new(
                    ExprKind::Call {
                        callee: Box::new(callee),
                        arguments: vec![],
                    },
                    span,
                );
                return Ok(Stmt::new(StmtKind::Expression(call), span, None));
            }

            // Handle no-parens form with symbol argument: belongs_to :user
            // The expression parser can't handle bare-symbol infix, so parse it here.
            if let Some(next) = self.tokens.get(self.current + 1) {
                if matches!(
                    next.kind,
                    TokenKind::SymbolLiteral(_) | TokenKind::StringLiteral(_)
                ) {
                    let callee_name = name.clone();
                    self.advance(); // consume the identifier
                                    // Parse one or more comma-separated positional args, so
                                    // `enum_field :status, Status` works alongside the
                                    // single-arg `belongs_to :user`. (Named args still require
                                    // the parens form.)
                    let mut arguments =
                        vec![crate::ast::expr::Argument::Positional(self.expression()?)];
                    while self.match_token(&TokenKind::Comma) {
                        arguments.push(crate::ast::expr::Argument::Positional(self.expression()?));
                    }
                    // Trailing `do … end` block for declarative DSLs, e.g.
                    // `state_machine :status do … end`.
                    if self.check(&TokenKind::Do) {
                        let block = self.parse_trailing_do_block()?;
                        arguments.push(crate::ast::expr::Argument::Block(block));
                    }
                    let span = start_span.merge(&self.previous_span());
                    let callee = Expr::new(ExprKind::Variable(callee_name), start_span);
                    let call = Expr::new(
                        ExprKind::Call {
                            callee: Box::new(callee),
                            arguments,
                        },
                        span,
                    );
                    self.match_token(&TokenKind::Semicolon);
                    return Ok(Stmt::new(StmtKind::Expression(call), span, None));
                }
            }
        }

        // Parse as an expression (function call)
        let expr = self.expression()?;
        self.match_token(&TokenKind::Semicolon); // Optional semicolon
        let span = start_span.merge(&self.previous_span());
        Ok(Stmt::new(StmtKind::Expression(expr), span, None))
    }

    /// Parse a static { ... } block inside a class.
    fn parse_static_block(&mut self) -> ParseResult<Vec<Stmt>> {
        let _start_span = self.current_span();
        self.expect(&TokenKind::Static)?;
        self.expect(&TokenKind::LeftBrace)?;

        let mut statements = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            statements.push(self.statement()?);
        }

        self.expect(&TokenKind::RightBrace)?;
        Ok(statements)
    }

    /// Parse a Ruby-style `class << self ... end` block inside a class body.
    /// Every method declared in the block is marked static; the methods are
    /// pushed straight into the enclosing class's `methods` vector.
    fn parse_singleton_class_block(&mut self, methods: &mut Vec<MethodDecl>) -> ParseResult<()> {
        self.expect(&TokenKind::Class)?;
        self.expect(&TokenKind::LessLess)?;
        self.expect(&TokenKind::SelfKeyword)?;
        // Optional opening brace; matches the rest of the class body grammar.
        self.match_token(&TokenKind::LeftBrace);

        while !self.check(&TokenKind::End)
            && !self.check(&TokenKind::RightBrace)
            && !self.is_at_end()
        {
            // Allow visibility modifiers; an explicit `static` is redundant
            // here but accepted silently — every method is static anyway.
            let (visibility, _is_static, _is_const) = self.parse_modifiers();

            if !self.check(&TokenKind::Fn) {
                return Err(ParserError::general(
                    "`class << self` blocks may only contain method declarations",
                    self.current_span(),
                ));
            }

            methods.push(self.parse_method(visibility, true)?);
        }

        if !self.match_token(&TokenKind::End) {
            self.expect(&TokenKind::RightBrace)?;
        }
        Ok(())
    }

    fn parse_modifiers(&mut self) -> (Visibility, bool, bool) {
        let mut visibility = Visibility::Public;
        let mut is_static = false;
        let mut is_const = false;

        loop {
            if self.match_token(&TokenKind::Public) {
                visibility = Visibility::Public;
            } else if self.match_token(&TokenKind::Private) {
                visibility = Visibility::Private;
            } else if self.match_token(&TokenKind::Protected) {
                visibility = Visibility::Protected;
            } else if self.match_token(&TokenKind::Static) {
                is_static = true;
            } else if self.match_token(&TokenKind::Const) {
                is_const = true;
            } else {
                break;
            }
        }

        (visibility, is_static, is_const)
    }

    fn parse_constructor(&mut self) -> ParseResult<ConstructorDecl> {
        let start_span = self.current_span();
        self.expect(&TokenKind::New)?;

        let params = self.parse_parameters()?;
        let body = self.parse_constructor_body()?;
        let span = start_span.merge(&self.previous_span());

        Ok(ConstructorDecl { params, body, span })
    }

    fn parse_constructor_body(&mut self) -> ParseResult<Vec<Stmt>> {
        if self.match_token(&TokenKind::End) {
            Ok(Vec::new())
        } else if self.match_token(&TokenKind::Do) {
            let mut statements = Vec::new();
            while !self.check(&TokenKind::End) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::End)?;
            Ok(statements)
        } else if self.check(&TokenKind::LeftBrace) && !self.looks_like_hash_literal() {
            self.advance(); // consume {
            let mut statements = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::RightBrace)?;
            Ok(statements)
        } else {
            let mut statements = Vec::new();
            while !self.check(&TokenKind::End) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            if !statements.is_empty() {
                self.expect(&TokenKind::End)?;
            }
            Ok(statements)
        }
    }

    fn parse_method(&mut self, visibility: Visibility, is_static: bool) -> ParseResult<MethodDecl> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Fn)?;

        // Ruby-style `def self.foo(...)`: the `self.` prefix marks the method
        // static. Combines harmlessly with a leading `static` modifier.
        let mut is_static = is_static;
        if self.check(&TokenKind::SelfKeyword)
            && matches!(
                self.tokens.get(self.current + 1).map(|t| &t.kind),
                Some(TokenKind::Dot)
            )
        {
            self.advance();
            self.advance();
            is_static = true;
        }

        let name = self.expect_identifier()?;
        let params = self.parse_parameters()?;

        let return_type = if self.match_token(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.parse_function_body()?;
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

    fn parse_field(
        &mut self,
        visibility: Visibility,
        is_static: bool,
        is_const: bool,
    ) -> ParseResult<FieldDecl> {
        let start_span = self.current_span();
        let name = self.expect_identifier()?;

        // Type annotation: required for regular fields, optional for const fields
        let type_annotation = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else if !is_const {
            return Err(ParserError::general(
                "expected ':' and type annotation for field declaration",
                self.current_span(),
            ));
        } else {
            None
        };

        // Initializer: required for const fields, optional for regular fields
        let initializer = if self.match_token(&TokenKind::Equal) {
            Some(self.expression()?)
        } else if is_const {
            return Err(ParserError::general(
                "const field must have an initializer",
                self.current_span(),
            ));
        } else {
            None
        };

        self.match_token(&TokenKind::Semicolon);
        let span = start_span.merge(&self.previous_span());

        Ok(FieldDecl {
            visibility,
            is_static,
            is_const,
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
            None,
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

        self.match_token(&TokenKind::Semicolon);
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

        self.match_token(&TokenKind::Semicolon);
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Let {
                name,
                type_annotation,
                initializer,
            },
            span,
            None,
        ))
    }

    pub(crate) fn const_declaration(&mut self) -> ParseResult<Stmt> {
        let start_span = self.current_span();
        self.expect(&TokenKind::Const)?;

        let name = self.expect_identifier()?;

        let type_annotation = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(&TokenKind::Equal)?;
        let initializer = self.expression()?;

        self.match_token(&TokenKind::Semicolon);
        let span = start_span.merge(&self.previous_span());

        Ok(Stmt::new(
            StmtKind::Const {
                name,
                type_annotation,
                initializer,
            },
            span,
            None,
        ))
    }

    pub(crate) fn parse_function_body(&mut self) -> ParseResult<Vec<Stmt>> {
        if self.match_token(&TokenKind::End) {
            Ok(Vec::new())
        } else if self.match_token(&TokenKind::Do) {
            // do...end block
            let mut statements = Vec::new();
            while !self.check(&TokenKind::End) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::End)?;
            Ok(statements)
        } else if self.check(&TokenKind::LeftBrace) && !self.looks_like_hash_literal() {
            self.advance(); // consume {
            let mut statements = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.expect(&TokenKind::RightBrace)?;
            Ok(statements)
        } else {
            let mut statements = Vec::new();
            while !self.check(&TokenKind::End) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            if !statements.is_empty() {
                self.expect(&TokenKind::End)?;
            }
            Ok(statements)
        }
    }
}
