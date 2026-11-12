//! `smell/shadowed-builtin`: a local (a bare assignment or a parameter) named
//! after a global the runtime installs — `options`, `stream`, `page_url` —
//! in a scope that also calls that global.
//!
//! The call reaches the local, not the builtin: `options("/x", …)` after
//! `options = "…"` calls a String. Worse, the tree-walking engine reads a bare
//! assignment to a name that already exists globally as a *rebinding* of it,
//! so the builtin is gone for the rest of the program, while the VM makes a
//! local — the two engines disagree. The type checker reported some of these
//! as `expected (Any, Any) -> String, found String` and missed the rest.
//!
//! Only a scope that also *uses* the global is reported. Naming a local
//! `post`, `json` or `format` in an action that never calls `post(...)` is
//! everyday Soli, and a rule that fired on every one of them would be turned
//! off. Not reported either:
//! - `let name = …` — the conventions name `let` as the way to shadow a
//!   function on purpose;
//! - a lambda assigned to the name (`format = fn(x) { … }`) — calling it is
//!   the point;
//! - a name the program itself declares (a `def`, a class, a method of the
//!   class, a sibling auto-loaded file's function): that is no builtin here.

use std::collections::HashSet;

use crate::ast::expr::{Expr, ExprKind};
use crate::ast::stmt::{Parameter, Stmt, StmtKind};
use crate::ast::types::{TypeAnnotation, TypeKind};
use crate::lint::{LintDiagnostic, Severity};
use crate::span::Span;

const RULE: &str = "smell/shadowed-builtin";

/// Check one scope — a function, method or constructor body with its
/// parameters, or the program's top level with none. Lambdas and blocks
/// inside it belong to it: their assignments and calls are counted here, and
/// they are not checked again on their own.
pub fn check_shadowed_builtins(
    params: &[Parameter],
    body: &[Stmt],
    declared: &HashSet<String>,
    test_file: bool,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    check_scope(params, body.iter(), declared, test_file, diagnostics);
}

/// [`check_shadowed_builtins`] for the program's top level: everything but
/// the declarations, whose bodies are scopes of their own.
pub fn check_top_level(
    stmts: &[Stmt],
    declared: &HashSet<String>,
    test_file: bool,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    // `declared` holds this file's top-level bare assignments too (they
    // declare a global for the whole app, like `const`): the very bindings
    // this checks. Only a `def`, class, enum or import of the name exempts it.
    let mut assigned_here: HashSet<String> = HashSet::new();
    for stmt in stmts.iter().filter(|stmt| !is_declaration(stmt)) {
        match &stmt.kind {
            StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => {
                assigned_here.insert(name.clone());
            }
            StmtKind::Expression(expr) => {
                if let ExprKind::Assign { target, .. } = &expr.kind {
                    if let ExprKind::Variable(name) = &target.kind {
                        assigned_here.insert(name.clone());
                    }
                }
            }
            _ => {}
        }
    }
    let mut own_declarations = HashSet::new();
    for stmt in stmts.iter().filter(|stmt| is_declaration(stmt)) {
        crate::lint::rules::scope::collect_program_names(
            std::slice::from_ref(stmt),
            &mut own_declarations,
        );
    }
    let declared: HashSet<String> = declared
        .iter()
        .filter(|name| !assigned_here.contains(*name) || own_declarations.contains(*name))
        .cloned()
        .collect();
    let scope = stmts.iter().filter(|stmt| !is_declaration(stmt));
    check_scope(&[], scope, &declared, test_file, diagnostics);
}

fn check_scope<'a>(
    params: &[Parameter],
    body: impl Iterator<Item = &'a Stmt>,
    declared: &HashSet<String>,
    test_file: bool,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    // A parameter is counted only when its annotation says it holds a value:
    // an unannotated parameter the body calls is a callback
    // (`def failure_of(check) … check() …`), named after what it does.
    let mut bindings: Vec<(String, Span, bool)> = params
        .iter()
        .filter(|p| holds_a_value(&p.type_annotation))
        .map(|p| (p.name.clone(), p.span, true))
        .collect();
    let mut called: HashSet<String> = HashSet::new();
    let mut member_of: HashSet<String> = HashSet::new();
    let mut on_expr = |expr: &Expr| match &expr.kind {
        ExprKind::Assign { target, value } => {
            if let ExprKind::Variable(name) = &target.kind {
                if !is_lambda(value) {
                    bindings.push((name.clone(), expr.span, false));
                }
            }
        }
        ExprKind::Call { callee, .. } => {
            if let ExprKind::Variable(name) = &callee.kind {
                called.insert(name.clone());
            }
        }
        ExprKind::Member { object, .. } | ExprKind::SafeMember { object, .. } => {
            if let ExprKind::Variable(name) = &object.kind {
                member_of.insert(name.clone());
            }
        }
        _ => {}
    };
    for stmt in body {
        crate::ast::walk::walk_stmt(stmt, &mut |_| {}, &mut on_expr);
    }

    let mut reported: HashSet<String> = HashSet::new();
    for (name, span, is_param) in bindings {
        if declared.contains(&name) || !is_builtin(&name, test_file) {
            continue;
        }
        let is_class = crate::types::runtime_globals::is_runtime_class(&name);
        let used = called.contains(&name) || (is_class && member_of.contains(&name));
        if !used || !reported.insert(name.clone()) {
            continue;
        }
        let what = if is_param { "parameter" } else { "local" };
        let builtin = if is_class {
            format!("builtin class {name}")
        } else {
            format!("builtin {name}()")
        };
        diagnostics.push(LintDiagnostic {
            rule: RULE,
            message: format!(
                "{what} '{name}' shadows the {builtin}, which this scope also uses: \
                 that use reaches the {what}, not the builtin — rename the {what}"
            ),
            span,
            severity: Severity::Warning,
        });
    }
}

fn is_declaration(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Function(_)
        | StmtKind::Class(_)
        | StmtKind::Enum(_)
        | StmtKind::Interface(_)
        | StmtKind::Import(_) => true,
        StmtKind::Export(inner) => is_declaration(inner),
        _ => false,
    }
}

fn is_builtin(name: &str, test_file: bool) -> bool {
    crate::types::runtime_globals::is_runtime_global(name)
        || (test_file
            && crate::types::runtime_globals::test_dsl_globals()
                .iter()
                .any(|global| global == name))
}

/// Whether a parameter's annotation rules out a function: `String`, `Hash`,
/// `Int[]`. Unannotated (`Any`) and function-typed parameters may be called.
fn holds_a_value(annotation: &TypeAnnotation) -> bool {
    match &annotation.kind {
        TypeKind::Named(name) => !matches!(
            name.as_str(),
            "Any" | "Function" | "Fn" | "Lambda" | "Callable"
        ),
        TypeKind::Array(_) | TypeKind::Hash { .. } => true,
        TypeKind::Nullable(inner) => holds_a_value(inner),
        TypeKind::Union(members) => members.iter().all(holds_a_value),
        TypeKind::Function { .. } | TypeKind::Void => false,
    }
}

fn is_lambda(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Lambda { .. } => true,
        ExprKind::Grouping(inner) => is_lambda(inner),
        _ => false,
    }
}
