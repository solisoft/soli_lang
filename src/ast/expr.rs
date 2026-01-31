//! Expression AST nodes.

use crate::ast::stmt::{Parameter, Stmt};
use crate::ast::types::TypeAnnotation;
use crate::span::Span;

/// An expression in the AST.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// A named argument in a function call: `name: value`
#[derive(Debug, Clone, PartialEq)]
pub struct NamedArgument {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

/// An argument in a function call (positional or named)
#[derive(Debug, Clone, PartialEq)]
pub enum Argument {
    Positional(Expr),
    Named(NamedArgument),
}

/// All expression variants.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// Integer literal: 42
    IntLiteral(i64),
    /// Float literal: 3.14
    FloatLiteral(f64),
    /// String literal: "hello"
    StringLiteral(String),
    /// Interpolated string: "Hello \(name)!"
    InterpolatedString(Vec<InterpolatedPart>),
    /// Boolean literal: true, false
    BoolLiteral(bool),
    /// Null literal
    Null,

    /// Variable reference: foo
    Variable(String),

    /// Binary operation: a + b
    Binary {
        left: Box<Expr>,
        operator: BinaryOp,
        right: Box<Expr>,
    },

    /// Unary operation: -x, !x
    Unary {
        operator: UnaryOp,
        operand: Box<Expr>,
    },

    /// Grouping expression: (expr)
    Grouping(Box<Expr>),

    /// Function call: foo(a, b) or foo(named: value)
    Call {
        callee: Box<Expr>,
        arguments: Vec<Argument>,
    },

    /// Pipeline expression: x |> foo()
    Pipeline { left: Box<Expr>, right: Box<Expr> },

    /// Member access: obj.field
    Member { object: Box<Expr>, name: String },

    /// Qualified name: Outer::Inner (for nested class access)
    QualifiedName { qualifier: Box<Expr>, name: String },

    /// Array index: arr[index]
    Index { object: Box<Expr>, index: Box<Expr> },

    /// this reference
    This,

    /// super reference (for method calls)
    Super,

    /// Object instantiation: new ClassName(args) or new Outer::Inner(args)
    /// class_expr can be a Variable or QualifiedName expression
    New {
        class_expr: Box<Expr>,
        arguments: Vec<Argument>,
    },

    /// Array literal: [1, 2, 3]
    Array(Vec<Expr>),

    /// Hash literal: { "key" => "value", ... }
    Hash(Vec<(Expr, Expr)>),

    /// Block expression: { statements }
    Block(Vec<Stmt>),

    /// Assignment expression: x = 5
    Assign { target: Box<Expr>, value: Box<Expr> },

    /// Logical and: a && b
    LogicalAnd { left: Box<Expr>, right: Box<Expr> },

    /// Logical or: a || b
    LogicalOr { left: Box<Expr>, right: Box<Expr> },

    /// Nullish coalescing: a ?? b (returns b if a is null, else a)
    NullishCoalescing { left: Box<Expr>, right: Box<Expr> },

    /// Lambda/anonymous function: |x, y| { stmt; }
    Lambda {
        params: Vec<Parameter>,
        return_type: Option<TypeAnnotation>,
        body: Vec<Stmt>,
    },

    /// Ternary/conditional expression: cond ? then_expr : else_expr
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },

    /// Pattern matching: match x { pattern => expr, ... }
    Match {
        expression: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    /// List comprehension: [expr for x in iter if cond]
    ListComprehension {
        element: Box<Expr>,
        variable: String,
        iterable: Box<Expr>,
        condition: Option<Box<Expr>>,
    },

    /// Hash comprehension: {key: expr for x in iter if cond}
    HashComprehension {
        key: Box<Expr>,
        value: Box<Expr>,
        variable: String,
        iterable: Box<Expr>,
        condition: Option<Box<Expr>>,
    },

    /// Await expression: await expr
    Await(Box<Expr>),

    /// Spread expression: ...expr (for arrays/hashes)
    Spread(Box<Expr>),

    /// Throw expression: throw expr
    Throw(Box<Expr>),
}

/// Part of an interpolated string.
#[derive(Debug, Clone, PartialEq)]
pub enum InterpolatedPart {
    /// Literal text
    Literal(String),
    /// Expression to interpolate: \(expr)
    Expression(Expr),
}

/// A single arm in a match expression.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

/// Patterns for match expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchPattern {
    /// Wildcard: _
    Wildcard,
    /// Variable binding: name
    Variable(String),
    /// Type-annotated variable: name: Type
    Typed { name: String, type_name: String },
    /// Literal pattern: 42, "hello", true, null
    Literal(ExprKind),
    /// Array pattern: [a, b, ...rest]
    Array {
        elements: Vec<MatchPattern>,
        rest: Option<String>,
    },
    /// Hash/object pattern: {field: pattern, ...rest}
    Hash {
        fields: Vec<(String, MatchPattern)>,
        rest: Option<String>,
    },
    /// Destructuring pattern: Type { field1, field2 }
    Destructuring {
        type_name: String,
        fields: Vec<(String, MatchPattern)>,
    },
    /// Conjunction (AND) of patterns
    And(Vec<MatchPattern>),
    /// Disjunction (OR) of patterns
    Or(Vec<MatchPattern>),
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Range,
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinaryOp::Add => write!(f, "+"),
            BinaryOp::Subtract => write!(f, "-"),
            BinaryOp::Multiply => write!(f, "*"),
            BinaryOp::Divide => write!(f, "/"),
            BinaryOp::Modulo => write!(f, "%"),
            BinaryOp::Equal => write!(f, "=="),
            BinaryOp::NotEqual => write!(f, "!="),
            BinaryOp::Less => write!(f, "<"),
            BinaryOp::LessEqual => write!(f, "<="),
            BinaryOp::Greater => write!(f, ">"),
            BinaryOp::GreaterEqual => write!(f, ">="),
            BinaryOp::Range => write!(f, ".."),
        }
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
    Not,
}

impl std::fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnaryOp::Negate => write!(f, "-"),
            UnaryOp::Not => write!(f, "!"),
        }
    }
}
