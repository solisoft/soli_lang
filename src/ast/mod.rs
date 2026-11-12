//! Abstract Syntax Tree for Solilang.

pub mod expr;
pub mod stmt;
pub mod types;
pub mod walk;

pub use expr::{BinaryOp, CompoundOp, Expr, ExprKind, MatchArm, MatchPattern, UnaryOp};
pub use stmt::{
    Body, ClassDecl, ConstructorDecl, EnumDecl, EnumPayloadField, EnumVariantDecl, FieldDecl,
    FunctionDecl, ImportDecl, ImportItem, ImportSpecifier, InterfaceDecl, InterfaceMethod,
    MethodDecl, Parameter, Program, Stmt, StmtKind, Visibility,
};
pub use types::{GenericType, TypeAnnotation, TypeKind};
