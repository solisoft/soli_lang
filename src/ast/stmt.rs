//! Statement AST nodes.

use crate::ast::expr::Expr;
use crate::ast::types::TypeAnnotation;
use crate::span::Span;

/// A statement in the AST.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Statement variants.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// Expression statement: expr;
    Expression(Expr),

    /// Variable declaration: let x: Type = expr;
    Let {
        name: String,
        type_annotation: Option<TypeAnnotation>,
        initializer: Option<Expr>,
    },

    /// Block: { statements }
    Block(Vec<Stmt>),

    /// If statement: if (cond) { ... } else { ... }
    If {
        condition: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
    },

    /// While loop: while (cond) { ... }
    While { condition: Expr, body: Box<Stmt> },

    /// For loop: for (x in iter) { ... }
    For {
        variable: String,
        iterable: Expr,
        body: Box<Stmt>,
    },

    /// Return statement: return expr;
    Return(Option<Expr>),

    /// Throw statement: throw expr;
    Throw(Expr),

    /// Try/Catch/Finally: try { ... } catch (e) { ... } finally { ... }
    Try {
        try_block: Box<Stmt>,
        catch_var: Option<String>,
        catch_block: Option<Box<Stmt>>,
        finally_block: Option<Box<Stmt>>,
    },

    /// Function declaration
    Function(FunctionDecl),

    /// Class declaration
    Class(ClassDecl),

    /// Interface declaration
    Interface(InterfaceDecl),

    /// Import declaration: import "path" or import { items } from "path"
    Import(ImportDecl),

    /// Export declaration: export fn/class/let
    Export(Box<Stmt>),
}

/// An import specifier (what to import from a module).
#[derive(Debug, Clone, PartialEq)]
pub enum ImportSpecifier {
    /// Import all exports: import "module.soli";
    All,
    /// Import specific items: import { foo, bar } from "module.soli";
    Named(Vec<ImportItem>),
    /// Import as namespace: import * as mod from "module.soli";
    Namespace(String),
}

/// A single imported item with optional alias.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
    pub span: Span,
}

/// Import declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    pub path: String,
    pub specifier: ImportSpecifier,
    pub span: Span,
}

/// Function declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Function parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: TypeAnnotation,
    pub default_value: Option<Expr>,
    pub span: Span,
}

/// Class declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub name: String,
    pub superclass: Option<String>,
    pub interfaces: Vec<String>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<MethodDecl>,
    pub constructor: Option<ConstructorDecl>,
    /// Class-level statements (e.g., validates(...), before_save(...))
    /// These are executed once when the class is defined.
    pub class_statements: Vec<Stmt>,
    pub span: Span,
}

/// Visibility modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Public,
    Private,
    Protected,
}

/// Field declaration in a class.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub visibility: Visibility,
    pub is_static: bool,
    pub name: String,
    pub type_annotation: TypeAnnotation,
    pub initializer: Option<Expr>,
    pub span: Span,
}

/// Method declaration in a class.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodDecl {
    pub visibility: Visibility,
    pub is_static: bool,
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Constructor declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstructorDecl {
    pub params: Vec<Parameter>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Interface declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDecl {
    pub name: String,
    pub methods: Vec<InterfaceMethod>,
    pub span: Span,
}

/// Method signature in an interface.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceMethod {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub span: Span,
}

/// A complete program.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Stmt>,
}

impl Program {
    pub fn new(statements: Vec<Stmt>) -> Self {
        Self { statements }
    }
}
