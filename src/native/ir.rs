//! The typed form of a kernel, produced by [`super::eligibility`] and
//! consumed by the lowering.
//!
//! Every name is resolved to a slot and every expression carries its type, so
//! the lowering never has to look anything up: it only picks instructions.

use crate::ast::BinaryOp;

/// The three types a kernel can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ty {
    Int,
    Float,
    Bool,
}

impl Ty {
    /// The annotation name, as `value_matches_type` reads it (case-insensitive).
    pub fn from_annotation(name: &str) -> Option<Ty> {
        match name.to_ascii_lowercase().as_str() {
            "int" => Some(Ty::Int),
            "float" => Some(Ty::Float),
            "bool" => Some(Ty::Bool),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Ty::Int => "Int",
            Ty::Float => "Float",
            Ty::Bool => "Bool",
        }
    }

    /// The name with its article, for messages: "an Int".
    pub fn a(self) -> &'static str {
        match self {
            Ty::Int => "an Int",
            Ty::Float => "a Float",
            Ty::Bool => "a Bool",
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, Ty::Int | Ty::Float)
    }
}

/// Arithmetic operators. `ty` on [`KExpr::Arith`] says which kind runs: an Int
/// operand of a Float operation is converted first, as both engines do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn from_binary(op: BinaryOp) -> Option<CmpOp> {
        Some(match op {
            BinaryOp::Equal => CmpOp::Eq,
            BinaryOp::NotEqual => CmpOp::Ne,
            BinaryOp::Less => CmpOp::Lt,
            BinaryOp::LessEqual => CmpOp::Le,
            BinaryOp::Greater => CmpOp::Gt,
            BinaryOp::GreaterEqual => CmpOp::Ge,
            _ => return None,
        })
    }

    pub fn is_ordering(self) -> bool {
        !matches!(self, CmpOp::Eq | CmpOp::Ne)
    }
}

/// How a comparison is carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpKind {
    /// Both Int: exact signed comparison.
    Int,
    /// At least one Float: both as f64.
    Float,
    /// Both Bool: equality only.
    Bool,
}

#[derive(Debug, Clone)]
pub enum KExpr {
    Int(i64),
    Float(f64),
    Bool(bool),
    Local(u32, Ty),
    Arith {
        op: ArithOp,
        left: Box<KExpr>,
        right: Box<KExpr>,
        ty: Ty,
    },
    Cmp {
        op: CmpOp,
        kind: CmpKind,
        left: Box<KExpr>,
        right: Box<KExpr>,
    },
    Neg(Box<KExpr>, Ty),
    /// `!x`: the negated truthiness of any operand.
    Not(Box<KExpr>),
    /// `a && b` / `a || b`. `ty` is `None` when the operands differ in type,
    /// which is only allowed where the value is read for its truthiness.
    And(Box<KExpr>, Box<KExpr>, Option<Ty>),
    Or(Box<KExpr>, Box<KExpr>, Option<Ty>),
    /// `c ? a : b`, both arms of type `ty`.
    Select {
        cond: Box<KExpr>,
        then: Box<KExpr>,
        otherwise: Box<KExpr>,
        ty: Ty,
    },
    /// A call to another kernel of the same set, by index.
    Call {
        kernel: u32,
        args: Vec<KExpr>,
        ty: Ty,
    },
    /// `x = e` used as a value: stores, then yields the stored value.
    Assign(u32, Box<KExpr>, Ty),
}

impl KExpr {
    /// `None` for an `&&`/`||` whose operands differ: it has no single type.
    pub fn ty(&self) -> Option<Ty> {
        Some(match self {
            KExpr::Int(_) => Ty::Int,
            KExpr::Float(_) => Ty::Float,
            KExpr::Bool(_) | KExpr::Cmp { .. } | KExpr::Not(_) => Ty::Bool,
            KExpr::Local(_, ty)
            | KExpr::Arith { ty, .. }
            | KExpr::Neg(_, ty)
            | KExpr::Select { ty, .. }
            | KExpr::Call { ty, .. }
            | KExpr::Assign(_, _, ty) => *ty,
            KExpr::And(_, _, ty) | KExpr::Or(_, _, ty) => return *ty,
        })
    }
}

#[derive(Debug, Clone)]
pub enum KStmt {
    /// Evaluated for its effects (an assignment, a call that may decline).
    Expr(KExpr),
    If {
        cond: KExpr,
        then: Vec<KStmt>,
        otherwise: Vec<KStmt>,
        /// `false` when the source had no `else`: as the function's last
        /// statement, the missing branch is a fall-through, not an empty body.
        has_else: bool,
    },
    While {
        cond: KExpr,
        body: Vec<KStmt>,
    },
    Break,
    Return(KExpr),
    /// A statement whose value is `null` (`let`, a loop): fine in the middle
    /// of a body, a fall-through at its end.
    Nothing(Box<KStmt>),
}

/// One kernel, typed and resolved.
#[derive(Debug, Clone)]
pub struct KFunc {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    /// Type of every slot; the parameters come first.
    pub slots: Vec<Ty>,
    pub body: Vec<KStmt>,
    /// Kernels this one calls directly (indices into the set).
    pub callees: Vec<u32>,
    /// Source line of the `def`, for the log.
    pub line: u32,
}
