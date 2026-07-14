//! Statement AST nodes.

use std::path::PathBuf;

use crate::ast::expr::Expr;
use crate::ast::types::TypeAnnotation;
use crate::span::Span;

/// A statement in the AST.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
    pub source_path: Option<PathBuf>,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span, source_path: Option<PathBuf>) -> Self {
        Self {
            kind,
            span,
            source_path,
        }
    }
}

/// A single catch clause in a try/catch statement.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CatchClause {
    /// Optional type name to match (e.g., "NotFoundError"). None = catch-all.
    pub type_name: Option<String>,
    /// Optional variable to bind the exception value.
    pub var_name: Option<String>,
    /// The catch block body.
    pub body: Box<Stmt>,
}

/// Statement variants.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum StmtKind {
    /// Expression statement: expr;
    Expression(Expr),

    /// Variable declaration: let x: Type = expr;
    Let {
        name: String,
        type_annotation: Option<TypeAnnotation>,
        initializer: Option<Expr>,
    },

    /// Constant declaration: const x: Type = expr;
    Const {
        name: String,
        type_annotation: Option<TypeAnnotation>,
        initializer: Expr,
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

    /// For loop: for (x in iter) { ... } or for (x, i in iter) { ... }
    For {
        variable: String,
        index_variable: Option<String>,
        iterable: Expr,
        body: Box<Stmt>,
    },

    /// Return statement: return expr;
    Return(Option<Expr>),

    /// Throw statement: throw expr;
    Throw(Expr),

    /// Try/Catch/Finally: try { ... } catch TypeName e { ... } catch e { ... } finally { ... }
    Try {
        try_block: Box<Stmt>,
        catch_clauses: Vec<CatchClause>,
        finally_block: Option<Box<Stmt>>,
    },

    /// Function declaration. Boxed: `FunctionDecl`/`ClassDecl` are large
    /// (many `Vec`s + `Option`s), and an unboxed variant sets `size_of::<Stmt>()`
    /// for *every* statement in every parsed body. Boxing keeps `Stmt` small.
    Function(Box<FunctionDecl>),

    /// Class declaration
    Class(Box<ClassDecl>),

    /// Enum declaration
    Enum(Box<EnumDecl>),

    /// Interface declaration
    Interface(Box<InterfaceDecl>),

    /// Import declaration: import "path" or import { items } from "path"
    Import(ImportDecl),

    /// Export declaration: export fn/class/let
    Export(Box<Stmt>),
}

impl StmtKind {
    /// Boxing constructors for the large decl variants — centralize the
    /// `Box::new` so call sites read like the old unboxed form.
    pub fn function(decl: FunctionDecl) -> StmtKind {
        StmtKind::Function(Box::new(decl))
    }
    pub fn class(decl: ClassDecl) -> StmtKind {
        StmtKind::Class(Box::new(decl))
    }
    pub fn enum_decl(decl: EnumDecl) -> StmtKind {
        StmtKind::Enum(Box::new(decl))
    }
    pub fn interface(decl: InterfaceDecl) -> StmtKind {
        StmtKind::Interface(Box::new(decl))
    }
}

/// Size guard — a `Stmt` is allocated per parsed statement in every
/// `Rc<[Stmt]>` function/method body, once per worker. Keep the large decl
/// variants boxed. See the `size_guards` test in `interpreter::value`.
const _: () = assert!(std::mem::size_of::<Stmt>() <= 200);

/// An import specifier (what to import from a module).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ImportSpecifier {
    /// Import all exports: import "module.sl";
    All,
    /// Import specific items: import { foo, bar } from "module.sl";
    Named(Vec<ImportItem>),
    /// Import as namespace: import * as mod from "module.sl";
    Namespace(String),
}

/// A single imported item with optional alias.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
    pub span: Span,
}

/// Import declaration.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImportDecl {
    pub path: String,
    pub specifier: ImportSpecifier,
    pub span: Span,
}

/// Function declaration.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FunctionDecl {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Function parameter.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: TypeAnnotation,
    pub default_value: Option<Expr>,
    pub span: Span,
    pub is_block_param: bool,
}

/// Class declaration.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClassDecl {
    pub name: String,
    pub superclass: Option<String>,
    pub interfaces: Vec<String>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<MethodDecl>,
    pub constructor: Option<ConstructorDecl>,
    /// Static initialization block - executed once when the class is defined.
    /// Used for controller configuration (layout, before_action, etc.)
    pub static_block: Option<Vec<Stmt>>,
    /// Class-level statements (e.g., validates(...), before_save(...))
    /// These are executed once when the class is defined.
    pub class_statements: Vec<Stmt>,
    /// Nested classes defined within this class
    pub nested_classes: Vec<ClassDecl>,
    pub span: Span,
}

/// Enum declaration: `enum Name { Variant, Payload(field: Type), def method ... }`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<EnumVariantDecl>,
    /// Instance methods defined in the enum body (the "rich" scope).
    pub methods: Vec<MethodDecl>,
    pub span: Span,
}

/// A single enum variant: a unit variant (`Active`) or one carrying a payload
/// (`Pending(reason: String)`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EnumVariantDecl {
    pub name: String,
    /// Ordered payload fields. Empty for unit variants.
    pub payload: Vec<EnumPayloadField>,
    pub span: Span,
}

/// A field carried by a payload variant: `reason: String` (type optional).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EnumPayloadField {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub span: Span,
}

impl EnumDecl {
    /// Lower this enum into an ordinary class declaration that both execution
    /// engines already know how to run. Pure AST → AST; called on-the-fly at
    /// execution/compile time (the type checker keeps working on the `EnumDecl`).
    ///
    /// - `__variant: String` — instance tag field.
    /// - each **unit** variant → `static const Active = __enum_construct(Name, "Active", {})`
    ///   (a singleton built when the class is defined).
    /// - each **payload** variant → `static def Pending(reason) {
    ///   return __enum_construct(Name, "Pending", { "reason": reason }) }` — a real
    ///   static method, so positional and named construction both work.
    /// - `def variant() { return this.__variant }` — introspection.
    /// - `static const __enum_variants = { "Active": [], "Pending": ["reason"] }` —
    ///   variant → ordered payload field names (drives pattern binding & equality).
    /// - the user's `def`s, copied verbatim.
    pub fn lower_to_class(&self) -> ClassDecl {
        use crate::ast::expr::{Argument, Expr, ExprKind};
        use crate::ast::types::{TypeAnnotation, TypeKind};

        let span = self.span;
        let any_type = || TypeAnnotation::new(TypeKind::Named("Any".to_string()), span);
        // The enum's own type — so variant construction is typed as the enum
        // (enables `.method()` resolution and match-scrutinee typing).
        let enum_type = || TypeAnnotation::new(TypeKind::Named(self.name.clone()), span);
        let var = |name: &str| Expr::new(ExprKind::Variable(name.to_string()), span);
        let str_lit = |s: &str| Expr::new(ExprKind::StringLiteral(s.to_string()), span);

        // __enum_construct(EnumName, "<variant>", <fields-hash>)
        let construct = |variant: &str, fields: Vec<(Expr, Expr)>| {
            Expr::new(
                ExprKind::Call {
                    callee: Box::new(var("__enum_construct")),
                    arguments: vec![
                        Argument::Positional(var(&self.name)),
                        Argument::Positional(str_lit(variant)),
                        Argument::Positional(Expr::new(ExprKind::Hash(fields), span)),
                    ],
                },
                span,
            )
        };

        let mut fields = Vec::new();
        let mut methods = Vec::new();

        // Instance tag field.
        fields.push(FieldDecl {
            visibility: Visibility::Public,
            is_static: false,
            is_const: false,
            name: "__variant".to_string(),
            type_annotation: Some(TypeAnnotation::new(
                TypeKind::Named("String".to_string()),
                span,
            )),
            initializer: None,
            span,
        });

        for variant in &self.variants {
            if variant.payload.is_empty() {
                // Unit variant → static const singleton.
                fields.push(FieldDecl {
                    visibility: Visibility::Public,
                    is_static: true,
                    is_const: true,
                    name: variant.name.clone(),
                    type_annotation: Some(enum_type()),
                    initializer: Some(construct(&variant.name, Vec::new())),
                    span: variant.span,
                });
            } else {
                // Payload variant → static constructor method.
                let params: Vec<Parameter> = variant
                    .payload
                    .iter()
                    .map(|field| Parameter {
                        name: field.name.clone(),
                        type_annotation: field.type_annotation.clone().unwrap_or_else(any_type),
                        default_value: None,
                        span: field.span,
                        is_block_param: false,
                    })
                    .collect();
                let hash_fields: Vec<(Expr, Expr)> = variant
                    .payload
                    .iter()
                    .map(|field| (str_lit(&field.name), var(&field.name)))
                    .collect();
                methods.push(MethodDecl {
                    visibility: Visibility::Public,
                    is_static: true,
                    name: variant.name.clone(),
                    params,
                    return_type: Some(enum_type()),
                    body: vec![Stmt::new(
                        StmtKind::Return(Some(construct(&variant.name, hash_fields))),
                        variant.span,
                        None,
                    )],
                    span: variant.span,
                });
            }
        }

        // static const __enum_variants = { "<variant>": [<field names>], ... }
        let variants_meta: Vec<(Expr, Expr)> = self
            .variants
            .iter()
            .map(|variant| {
                let names: Vec<Expr> = variant.payload.iter().map(|f| str_lit(&f.name)).collect();
                (
                    str_lit(&variant.name),
                    Expr::new(ExprKind::Array(names), span),
                )
            })
            .collect();
        fields.push(FieldDecl {
            visibility: Visibility::Public,
            is_static: true,
            is_const: true,
            name: "__enum_variants".to_string(),
            type_annotation: None,
            initializer: Some(Expr::new(ExprKind::Hash(variants_meta), span)),
            span,
        });

        // static def parse(value) { return __enum_from(EnumName, value) }
        // Rebuilds an enum value from its stored DB/JSON shape. (`from` is a
        // reserved keyword, so the factory is named `parse`.)
        methods.push(MethodDecl {
            visibility: Visibility::Public,
            is_static: true,
            name: "parse".to_string(),
            params: vec![Parameter {
                name: "value".to_string(),
                type_annotation: any_type(),
                default_value: None,
                span,
                is_block_param: false,
            }],
            return_type: Some(enum_type()),
            body: vec![Stmt::new(
                StmtKind::Return(Some(Expr::new(
                    ExprKind::Call {
                        callee: Box::new(var("__enum_from")),
                        arguments: vec![
                            Argument::Positional(var(&self.name)),
                            Argument::Positional(var("value")),
                        ],
                    },
                    span,
                ))),
                span,
                None,
            )],
            span,
        });

        // def variant() { return this.__variant }
        methods.push(MethodDecl {
            visibility: Visibility::Public,
            is_static: false,
            name: "variant".to_string(),
            params: Vec::new(),
            return_type: Some(TypeAnnotation::new(
                TypeKind::Named("String".to_string()),
                span,
            )),
            body: vec![Stmt::new(
                StmtKind::Return(Some(Expr::new(
                    ExprKind::Member {
                        object: Box::new(Expr::new(ExprKind::This, span)),
                        name: "__variant".to_string(),
                    },
                    span,
                ))),
                span,
                None,
            )],
            span,
        });

        // User-defined behaviour, copied verbatim.
        methods.extend(self.methods.iter().cloned());

        ClassDecl {
            name: self.name.clone(),
            superclass: None,
            interfaces: Vec::new(),
            fields,
            methods,
            constructor: None,
            static_block: None,
            class_statements: Vec::new(),
            nested_classes: Vec::new(),
            span,
        }
    }
}

/// Visibility modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Visibility {
    #[default]
    Public,
    Private,
    Protected,
}

/// Field declaration in a class.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FieldDecl {
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_const: bool,
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub initializer: Option<Expr>,
    pub span: Span,
}

/// Method declaration in a class.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConstructorDecl {
    pub params: Vec<Parameter>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Interface declaration.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InterfaceDecl {
    pub name: String,
    pub methods: Vec<InterfaceMethod>,
    pub span: Span,
}

/// Method signature in an interface.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InterfaceMethod {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub span: Span,
}

/// A complete program.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Program {
    pub statements: Vec<Stmt>,
}

impl Program {
    pub fn new(statements: Vec<Stmt>) -> Self {
        Self { statements }
    }
}
