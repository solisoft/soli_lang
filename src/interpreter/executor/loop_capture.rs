//! Detects whether a loop body creates closures.
//!
//! `while`/`for` reuse a single body environment across iterations as an
//! optimization (avoids a per-iteration allocation). That reuse is only sound
//! when nothing in the body captures the environment: if the body creates a
//! closure (a lambda or nested function) that outlives the iteration, every
//! such closure must capture a *fresh* binding so that, e.g., closures pushed
//! from different iterations don't all observe the final value of a loop-local.
//!
//! This is a conservative over-approximation — a body containing any lambda or
//! nested function declaration forces the per-iteration-environment path, even
//! if the closure doesn't actually capture a loop-local. False positives only
//! cost an allocation (and such bodies already allocate a closure), never
//! correctness.

use crate::ast::expr::{Argument, Expr, ExprKind, InterpolatedPart};
use crate::ast::stmt::{Stmt, StmtKind};

/// Whether any statement in `body` creates a closure (lambda or nested `fn`).
pub(crate) fn body_creates_closures(body: &[Stmt]) -> bool {
    body.iter().any(stmt_creates_closures)
}

pub(crate) fn stmt_creates_closures(stmt: &Stmt) -> bool {
    match &stmt.kind {
        // A nested function declaration is itself a closure.
        StmtKind::Function(_) => true,
        StmtKind::Expression(e) | StmtKind::Throw(e) => expr_creates_closures(e),
        StmtKind::Let { initializer, .. } => {
            initializer.as_ref().is_some_and(expr_creates_closures)
        }
        StmtKind::Const { initializer, .. } => expr_creates_closures(initializer),
        StmtKind::Block(stmts) => body_creates_closures(stmts),
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr_creates_closures(condition)
                || stmt_creates_closures(then_branch)
                || else_branch
                    .as_ref()
                    .is_some_and(|eb| stmt_creates_closures(eb))
        }
        StmtKind::While { condition, body } => {
            expr_creates_closures(condition) || stmt_creates_closures(body)
        }
        StmtKind::For { iterable, body, .. } => {
            expr_creates_closures(iterable) || stmt_creates_closures(body)
        }
        StmtKind::Return(e) => e.as_ref().is_some_and(expr_creates_closures),
        StmtKind::Try {
            try_block,
            catch_clauses,
            finally_block,
        } => {
            stmt_creates_closures(try_block)
                || catch_clauses.iter().any(|c| stmt_creates_closures(&c.body))
                || finally_block
                    .as_ref()
                    .is_some_and(|fb| stmt_creates_closures(fb))
        }
        StmtKind::Export(inner) => stmt_creates_closures(inner),
        // Class declarations carry their own method scopes; interfaces/imports
        // never create capturing closures in the loop's environment.
        StmtKind::Class(_) | StmtKind::Enum(_) | StmtKind::Interface(_) | StmtKind::Import(_) => {
            false
        }
    }
}

fn expr_creates_closures(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Lambda { .. } => true,
        ExprKind::Assign { target, value } => {
            expr_creates_closures(target) || expr_creates_closures(value)
        }
        ExprKind::CompoundAssign { target, value, .. } => {
            expr_creates_closures(target) || expr_creates_closures(value)
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipeline { left, right }
        | ExprKind::LogicalAnd { left, right }
        | ExprKind::LogicalOr { left, right }
        | ExprKind::NullishCoalescing { left, right } => {
            expr_creates_closures(left) || expr_creates_closures(right)
        }
        ExprKind::Unary { operand, .. } => expr_creates_closures(operand),
        ExprKind::Grouping(e)
        | ExprKind::Spread(e)
        | ExprKind::Throw(e)
        | ExprKind::PostfixIncrement(e)
        | ExprKind::PostfixDecrement(e) => expr_creates_closures(e),
        ExprKind::Call { callee, arguments } => {
            expr_creates_closures(callee) || arguments_create_closures(arguments)
        }
        ExprKind::New {
            class_expr,
            arguments,
        } => expr_creates_closures(class_expr) || arguments_create_closures(arguments),
        ExprKind::Member { object, .. } | ExprKind::SafeMember { object, .. } => {
            expr_creates_closures(object)
        }
        ExprKind::QualifiedName { qualifier, .. } => expr_creates_closures(qualifier),
        ExprKind::Index { object, index } => {
            expr_creates_closures(object) || expr_creates_closures(index)
        }
        ExprKind::Array(elems) => elems.iter().any(expr_creates_closures),
        ExprKind::Hash(pairs) => pairs
            .iter()
            .any(|(k, v)| expr_creates_closures(k) || expr_creates_closures(v)),
        ExprKind::Block(stmts) => body_creates_closures(stmts),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr_creates_closures(condition)
                || expr_creates_closures(then_branch)
                || else_branch
                    .as_ref()
                    .is_some_and(|eb| expr_creates_closures(eb))
        }
        ExprKind::Match { expression, arms } => {
            expr_creates_closures(expression)
                || arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(expr_creates_closures)
                        || expr_creates_closures(&arm.body)
                })
        }
        ExprKind::ListComprehension {
            element,
            iterable,
            condition,
            ..
        } => {
            expr_creates_closures(element)
                || expr_creates_closures(iterable)
                || condition.as_ref().is_some_and(|c| expr_creates_closures(c))
        }
        ExprKind::HashComprehension {
            key,
            value,
            iterable,
            condition,
            ..
        } => {
            expr_creates_closures(key)
                || expr_creates_closures(value)
                || expr_creates_closures(iterable)
                || condition.as_ref().is_some_and(|c| expr_creates_closures(c))
        }
        ExprKind::Rescue { expr, fallback } => {
            expr_creates_closures(expr) || expr_creates_closures(fallback)
        }
        ExprKind::InterpolatedString(parts) => parts.iter().any(|part| match part {
            InterpolatedPart::Expression(e) => expr_creates_closures(e),
            InterpolatedPart::Literal(_) => false,
        }),
        // Leaves and constructs that don't contain a capturing closure.
        ExprKind::IntLiteral(_)
        | ExprKind::FloatLiteral(_)
        | ExprKind::DecimalLiteral(_)
        | ExprKind::StringLiteral(_)
        | ExprKind::CommandSubstitution(_)
        | ExprKind::SdqlBlock { .. }
        | ExprKind::BoolLiteral(_)
        | ExprKind::Symbol(_)
        | ExprKind::Null
        | ExprKind::Variable(_)
        | ExprKind::This
        | ExprKind::Super => false,
    }
}

fn arguments_create_closures(arguments: &[Argument]) -> bool {
    arguments.iter().any(|arg| match arg {
        Argument::Positional(e) | Argument::Block(e) => expr_creates_closures(e),
        Argument::Named(named) => expr_creates_closures(&named.value),
    })
}
