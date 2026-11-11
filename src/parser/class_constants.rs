//! Class constants read by their bare name.
//!
//! ```soli
//! class Foo
//!   const MAX = 10
//!   def get
//!     MAX            # → Foo.MAX
//!   end
//! end
//! Foo::MAX           # 10
//! ```
//!
//! A `const` in a class body was an instance field only: `@MAX` and
//! `Foo.new().MAX` read it, but `Foo.MAX`, `Foo::MAX` and a bare `MAX` in a
//! method did not — the bare name failed as an undefined variable. Two
//! desugarings, made once on the outermost class so a nested class knows its
//! full name:
//!
//! * every instance `const` with a value also becomes a `static const` of the
//!   same name (unless one is declared), so the class reads it;
//! * in the class's methods, constructor and invariants, a bare read of one of
//!   its constants becomes `Class.NAME` (`Outer::Inner.NAME` when nested). A
//!   name the method binds itself — a parameter, a local, a loop or catch
//!   variable — keeps its local meaning, and so does a call `NAME(...)`.
//!
//! Both engines, the type checker and the linter see the rewritten tree, so
//! none of them needs a lookup rule of its own.

use std::collections::HashSet;

use crate::ast::expr::{Argument, Expr, ExprKind, InterpolatedPart};
use crate::ast::stmt::{ClassDecl, MethodDecl, Parameter, Stmt, StmtKind};

/// Apply both desugarings to `class` and, with their qualified names, to the
/// classes nested in it.
pub(crate) fn resolve(class: &mut ClassDecl) {
    let span = class.span;
    resolve_with(
        class,
        Expr::new(ExprKind::Variable(class.name.clone()), span),
    );
}

fn resolve_with(class: &mut ClassDecl, qualifier: Expr) {
    for nested in &mut class.nested_classes {
        let span = nested.span;
        let nested_qualifier = Expr::new(
            ExprKind::QualifiedName {
                qualifier: Box::new(qualifier.clone()),
                name: nested.name.clone(),
            },
            span,
        );
        resolve_with(nested, nested_qualifier);
    }

    let constants: HashSet<String> = class
        .fields
        .iter()
        .filter(|f| f.is_const)
        .map(|f| f.name.clone())
        .collect();
    if constants.is_empty() {
        return;
    }

    let statics: HashSet<String> = class
        .fields
        .iter()
        .filter(|f| f.is_static)
        .map(|f| f.name.clone())
        .collect();
    let promoted: Vec<_> = class
        .fields
        .iter()
        .filter(|f| f.is_const && !f.is_static && f.initializer.is_some())
        .filter(|f| !statics.contains(&f.name))
        .map(|f| {
            let mut field = f.clone();
            field.is_static = true;
            field
        })
        .collect();
    class.fields.extend(promoted);

    for method in class
        .methods
        .iter_mut()
        .chain(class.concern_class_methods.iter_mut())
    {
        rewrite_method(method, &constants, &qualifier);
    }
    if let Some(ctor) = &mut class.constructor {
        let bound = bound_names(&ctor.params, ctor.body.make_mut());
        rewrite_stmts(ctor.body.make_mut(), &constants, &bound, &qualifier);
    }
    for invariant in &mut class.invariants {
        let bound = bound_names(&[], invariant.body.make_mut());
        rewrite_stmts(invariant.body.make_mut(), &constants, &bound, &qualifier);
    }
}

fn rewrite_method(method: &mut MethodDecl, constants: &HashSet<String>, qualifier: &Expr) {
    let bound = bound_names(&method.params, method.body.make_mut());
    for param in &mut method.params {
        if let Some(default) = &mut param.default_value {
            rewrite_expr(default, constants, &bound, qualifier);
        }
    }
    rewrite_stmts(method.body.make_mut(), constants, &bound, qualifier);
}

/// Every name a body binds: parameters (its own and its lambdas'), locals,
/// assignment targets, loop, comprehension and catch variables.
fn bound_names(params: &[Parameter], body: &[Stmt]) -> HashSet<String> {
    // Both callbacks add to it, so it is shared through a `RefCell`.
    let bound: std::cell::RefCell<HashSet<String>> =
        std::cell::RefCell::new(params.iter().map(|p| p.name.clone()).collect());
    crate::ast::walk::walk_stmts(
        body,
        &mut |stmt| match &stmt.kind {
            StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => {
                bound.borrow_mut().insert(name.clone());
            }
            StmtKind::For {
                variable,
                index_variable,
                ..
            } => {
                bound.borrow_mut().insert(variable.clone());
                if let Some(index) = index_variable {
                    bound.borrow_mut().insert(index.clone());
                }
            }
            StmtKind::Try { catch_clauses, .. } => {
                for clause in catch_clauses {
                    if let Some(var) = &clause.var_name {
                        bound.borrow_mut().insert(var.clone());
                    }
                }
            }
            _ => {}
        },
        &mut |expr| match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::CompoundAssign { target, .. } => {
                if let ExprKind::Variable(name) = &target.kind {
                    bound.borrow_mut().insert(name.clone());
                }
            }
            ExprKind::Lambda { params, .. } => {
                bound
                    .borrow_mut()
                    .extend(params.iter().map(|p| p.name.clone()));
            }
            ExprKind::ListComprehension { variable, .. }
            | ExprKind::HashComprehension { variable, .. } => {
                bound.borrow_mut().insert(variable.clone());
            }
            _ => {}
        },
    );
    bound.into_inner()
}

fn rewrite_stmts(
    stmts: &mut [Stmt],
    constants: &HashSet<String>,
    bound: &HashSet<String>,
    qualifier: &Expr,
) {
    for stmt in stmts {
        rewrite_stmt(stmt, constants, bound, qualifier);
    }
}

fn rewrite_stmt(
    stmt: &mut Stmt,
    constants: &HashSet<String>,
    bound: &HashSet<String>,
    qualifier: &Expr,
) {
    let expr = |e: &mut Expr| rewrite_expr(e, constants, bound, qualifier);
    match &mut stmt.kind {
        StmtKind::Expression(e) | StmtKind::Throw(e) => expr(e),
        StmtKind::Let { initializer, .. } => {
            if let Some(e) = initializer {
                expr(e);
            }
        }
        StmtKind::Const { initializer, .. } => expr(initializer),
        StmtKind::Block(stmts) => rewrite_stmts(stmts, constants, bound, qualifier),
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        }
        | StmtKind::Unless {
            condition,
            then_branch,
            else_branch,
        } => {
            expr(condition);
            rewrite_stmt(then_branch, constants, bound, qualifier);
            if let Some(other) = else_branch {
                rewrite_stmt(other, constants, bound, qualifier);
            }
        }
        StmtKind::While { condition, body } => {
            expr(condition);
            rewrite_stmt(body, constants, bound, qualifier);
        }
        StmtKind::For { iterable, body, .. } => {
            expr(iterable);
            rewrite_stmt(body, constants, bound, qualifier);
        }
        StmtKind::Return(value) => {
            if let Some(e) = value {
                expr(e);
            }
        }
        StmtKind::Try {
            try_block,
            catch_clauses,
            finally_block,
        } => {
            rewrite_stmt(try_block, constants, bound, qualifier);
            for clause in catch_clauses {
                rewrite_stmt(&mut clause.body, constants, bound, qualifier);
            }
            if let Some(finally) = finally_block {
                rewrite_stmt(finally, constants, bound, qualifier);
            }
        }
        // A function, class or enum declared inside a method is a scope of
        // its own; it keeps reading names as it did.
        StmtKind::Function(_)
        | StmtKind::Class(_)
        | StmtKind::Enum(_)
        | StmtKind::Export(_)
        | StmtKind::Break
        | StmtKind::Interface(_)
        | StmtKind::Import(_) => {}
    }
}

fn rewrite_args(
    arguments: &mut [Argument],
    constants: &HashSet<String>,
    bound: &HashSet<String>,
    qualifier: &Expr,
) {
    for arg in arguments {
        match arg {
            Argument::Positional(e) | Argument::Block(e) => {
                rewrite_expr(e, constants, bound, qualifier)
            }
            Argument::Named(named) => rewrite_expr(&mut named.value, constants, bound, qualifier),
        }
    }
}

fn rewrite_expr(
    expr: &mut Expr,
    constants: &HashSet<String>,
    bound: &HashSet<String>,
    qualifier: &Expr,
) {
    let sub = |e: &mut Expr| rewrite_expr(e, constants, bound, qualifier);
    match &mut expr.kind {
        ExprKind::Variable(name) => {
            if constants.contains(name.as_str()) && !bound.contains(name.as_str()) {
                let name = std::mem::take(name);
                let mut object = qualifier.clone();
                object.span = expr.span;
                expr.kind = ExprKind::Member {
                    object: Box::new(object),
                    name,
                };
            }
        }
        ExprKind::IntLiteral(_)
        | ExprKind::FloatLiteral(_)
        | ExprKind::DecimalLiteral(_)
        | ExprKind::StringLiteral(_)
        | ExprKind::CommandSubstitution(_)
        | ExprKind::SdqlBlock { .. }
        | ExprKind::BoolLiteral(_)
        | ExprKind::Symbol(_)
        | ExprKind::Null
        | ExprKind::This
        | ExprKind::Super => {}
        ExprKind::InterpolatedString(parts) => {
            for part in parts {
                if let InterpolatedPart::Expression(e) = part {
                    sub(e);
                }
            }
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipeline { left, right }
        | ExprKind::LogicalAnd { left, right }
        | ExprKind::LogicalOr { left, right }
        | ExprKind::NullishCoalescing { left, right } => {
            sub(left);
            sub(right);
        }
        ExprKind::Unary { operand: inner, .. }
        | ExprKind::Grouping(inner)
        | ExprKind::Member { object: inner, .. }
        | ExprKind::SafeMember { object: inner, .. }
        | ExprKind::QualifiedName {
            qualifier: inner, ..
        }
        | ExprKind::PostfixIncrement(inner)
        | ExprKind::PostfixDecrement(inner)
        | ExprKind::Spread(inner)
        | ExprKind::Throw(inner) => sub(inner),
        ExprKind::Call { callee, arguments } => {
            // `NAME(...)` calls whatever `NAME` meant before: a constant
            // holding a lambda is rare, a function of that name is not ours.
            if !matches!(callee.kind, ExprKind::Variable(_)) {
                sub(callee);
            }
            rewrite_args(arguments, constants, bound, qualifier);
        }
        ExprKind::New {
            class_expr,
            arguments,
        } => {
            sub(class_expr);
            rewrite_args(arguments, constants, bound, qualifier);
        }
        ExprKind::Index { object, index } => {
            sub(object);
            sub(index);
        }
        ExprKind::Array(items) => {
            for item in items {
                sub(item);
            }
        }
        ExprKind::Hash(pairs) => {
            for (k, v) in pairs {
                sub(k);
                sub(v);
            }
        }
        ExprKind::Block(stmts) => rewrite_stmts(stmts, constants, bound, qualifier),
        ExprKind::Assign { target, value } | ExprKind::CompoundAssign { target, value, .. } => {
            sub(target);
            sub(value);
        }
        ExprKind::Lambda { params, body, .. } => {
            for param in params.iter_mut() {
                if let Some(default) = &mut param.default_value {
                    sub(default);
                }
            }
            rewrite_stmts(body.make_mut(), constants, bound, qualifier);
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            sub(condition);
            sub(then_branch);
            if let Some(other) = else_branch {
                sub(other);
            }
        }
        ExprKind::Match { expression, arms } => {
            sub(expression);
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    sub(guard);
                }
                sub(&mut arm.body);
            }
        }
        ExprKind::ListComprehension {
            element,
            iterable,
            condition,
            ..
        } => {
            sub(element);
            sub(iterable);
            if let Some(c) = condition {
                sub(c);
            }
        }
        ExprKind::HashComprehension {
            key,
            value,
            iterable,
            condition,
            ..
        } => {
            sub(key);
            sub(value);
            sub(iterable);
            if let Some(c) = condition {
                sub(c);
            }
        }
        ExprKind::Rescue {
            expr: inner,
            fallback,
        } => {
            sub(inner);
            sub(fallback);
        }
    }
}
