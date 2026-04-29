//! Scope-sensitive lint rules: detect reads of names that are never
//! assigned in the enclosing function/method body.
//!
//! Soli makes `let` optional — `x = 1` implicitly declares `x`. That's
//! convenient, but it makes a typo like `opts.push(...)` (where `opts`
//! was never assigned) fail only at runtime with "Undefined variable".
//! This rule catches those statically: if a bare name appears in a
//! function body but is never on the LHS of an assignment, never a
//! `let`/`for`/`catch` binding, and not a parameter, and also not
//! defined at program level, it's flagged.

use std::collections::HashSet;

use crate::ast::expr::{Argument, Expr, ExprKind, InterpolatedPart};
use crate::ast::stmt::{ImportDecl, ImportSpecifier, Parameter, Stmt, StmtKind};
use crate::lint::{LintDiagnostic, Severity};

fn insert_import_names(decl: &ImportDecl, out: &mut HashSet<String>) {
    match &decl.specifier {
        ImportSpecifier::All => {}
        ImportSpecifier::Named(items) => {
            for item in items {
                out.insert(item.alias.clone().unwrap_or_else(|| item.name.clone()));
            }
        }
        ImportSpecifier::Namespace(name) => {
            out.insert(name.clone());
        }
    }
}

/// Names the linter should treat as pre-defined, regardless of whether the
/// program explicitly declares them. Covers framework/globals that are
/// injected by the runtime (request-handler params, session helpers,
/// controller-scope helpers, test DSL, etc.) so the rule stays low-noise.
const WELL_KNOWN_GLOBALS: &[&str] = &[
    // Ruby/Soli keywords that appear as expressions
    "this",
    "self",
    "super",
    "true",
    "false",
    "null",
    "nil",
    // Request/controller context
    "req",
    "request",
    "response",
    "params",
    "session",
    "current_user",
    "flash",
    "errors",
    // Common stdlib / builtins (uppercase-classes handled separately)
    "print",
    "println",
    "puts",
    "p",
    "str",
    "int",
    "float",
    "bool",
    "decimal",
    "len",
    "length",
    "size",
    "type",
    "typeof",
    "assert",
    "assert_eq",
    "assert_ne",
    "expect",
    "describe",
    "context",
    "test",
    "it",
    "specify",
    "before_each",
    "after_each",
    "before_all",
    "after_all",
    "with_transaction",
    // Test HTTP helpers
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "head",
    "options",
    "request",
    "login",
    "logout",
    "as_admin",
    "as_guest",
    "as_user",
    "with_token",
    "set_header",
    "set_authorization",
    "clear_authorization",
    "set_cookie",
    "clear_cookies",
    "clear_headers",
    "signed_in",
    "signed_in?",
    "signed_out",
    "signed_out?",
    "create_session",
    "destroy_session",
    // Rendering/session helpers
    "render",
    "render_partial",
    "redirect",
    "h",
    "h!",
    "raw",
    "json",
    "session_get",
    "session_set",
    "session_delete",
    "session_destroy",
    "session_regenerate",
    "session_id",
    "session_driver",
    "session_config",
    "session_configure",
    "session_has",
    // Password / hashing helpers commonly used in tests
    "argon2_hash",
    "argon2_verify",
    "bcrypt_hash",
    "bcrypt_verify",
    "hmac",
    "sha256",
    // Module system
    "import",
    "export",
    // Misc
    "env",
    "getenv",
    "hasenv",
    "dotenv",
    // HTTP response helpers
    "halt",
    // Loop control
    "next",
    // Validation framework
    "validate",
    // Upload helpers
    "find_uploaded_file",
    "detach_all_uploads",
    // JSON helpers
    "json_parse",
    // Controller helpers
    "render_json",
    // Support helpers
    "fmt_blob_size",
    "fmt_blob_date",
    "short_content_kind",
    // Test framework
    "clock",
];

/// Collect top-level names defined in the program. These are names a
/// function body can reference without a local binding (top-level `let`,
/// `const`, `fn`, `class`, and imported symbols).
pub fn collect_program_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match &stmt.kind {
            StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => {
                out.insert(name.clone());
            }
            StmtKind::Function(decl) => {
                out.insert(decl.name.clone());
            }
            StmtKind::Class(decl) => {
                out.insert(decl.name.clone());
                // Nested classes are name-resolved via qualified access, so
                // we don't add them as top-level here.
            }
            StmtKind::Interface(decl) => {
                out.insert(decl.name.clone());
            }
            StmtKind::Import(decl) => insert_import_names(decl, out),
            StmtKind::Export(inner) => collect_program_names(std::slice::from_ref(inner), out),
            _ => {}
        }
    }
}

/// Entry point: check a function/method body for reads of names that
/// are never assigned. `program_names` is the set of top-level names
/// collected via `collect_program_names`.
pub fn check_undefined_locals(
    params: &[Parameter],
    body: &[Stmt],
    program_names: &HashSet<String>,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let mut defined: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
    collect_assigned_in_stmts(body, &mut defined);

    let mut reported: HashSet<(String, u32, u32)> = HashSet::new();
    for s in body {
        check_stmt(s, &defined, program_names, diagnostics, &mut reported);
    }
}

fn is_likely_global(name: &str) -> bool {
    // Classes / modules are conventionally PascalCase and live at program
    // level. If we don't see them defined here they may come from imports
    // or be builtins — don't flag.
    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return true;
    }
    // Leading underscore: common "intentionally unused / private" marker.
    // Internal names like __foo__ are almost always runtime-injected.
    if name.starts_with('_') {
        return true;
    }
    WELL_KNOWN_GLOBALS.contains(&name)
}

fn flag(
    name: &str,
    span: crate::span::Span,
    diagnostics: &mut Vec<LintDiagnostic>,
    reported: &mut HashSet<(String, u32, u32)>,
) {
    let key = (name.to_string(), span.line as u32, span.column as u32);
    if !reported.insert(key) {
        return;
    }
    diagnostics.push(LintDiagnostic {
        rule: "smell/undefined-local",
        message: format!(
            "variable '{}' is read but never assigned in this scope",
            name
        ),
        span,
        severity: Severity::Warning,
    });
}

fn check_stmt(
    stmt: &Stmt,
    defined: &HashSet<String>,
    program: &HashSet<String>,
    diagnostics: &mut Vec<LintDiagnostic>,
    reported: &mut HashSet<(String, u32, u32)>,
) {
    match &stmt.kind {
        StmtKind::Expression(e) => check_expr(e, defined, program, diagnostics, reported),
        StmtKind::Let { initializer, .. } => {
            if let Some(e) = initializer {
                check_expr(e, defined, program, diagnostics, reported);
            }
        }
        StmtKind::Const { initializer, .. } => {
            check_expr(initializer, defined, program, diagnostics, reported);
        }
        StmtKind::Block(stmts) => {
            for s in stmts {
                check_stmt(s, defined, program, diagnostics, reported);
            }
        }
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            check_expr(condition, defined, program, diagnostics, reported);
            check_stmt(then_branch, defined, program, diagnostics, reported);
            if let Some(e) = else_branch {
                check_stmt(e, defined, program, diagnostics, reported);
            }
        }
        StmtKind::While { condition, body } => {
            check_expr(condition, defined, program, diagnostics, reported);
            check_stmt(body, defined, program, diagnostics, reported);
        }
        StmtKind::For { iterable, body, .. } => {
            check_expr(iterable, defined, program, diagnostics, reported);
            check_stmt(body, defined, program, diagnostics, reported);
        }
        StmtKind::Return(e) => {
            if let Some(e) = e {
                check_expr(e, defined, program, diagnostics, reported);
            }
        }
        StmtKind::Throw(e) => check_expr(e, defined, program, diagnostics, reported),
        StmtKind::Try {
            try_block,
            catch_clauses,
            finally_block,
        } => {
            check_stmt(try_block, defined, program, diagnostics, reported);
            for clause in catch_clauses {
                check_stmt(&clause.body, defined, program, diagnostics, reported);
            }
            if let Some(f) = finally_block {
                check_stmt(f, defined, program, diagnostics, reported);
            }
        }
        StmtKind::Function(_) | StmtKind::Class(_) | StmtKind::Interface(_) => {
            // Nested definitions have their own scope; program-level already
            // records their names. Skip their bodies here — the top-level
            // linter walks into them separately.
        }
        StmtKind::Import(_) | StmtKind::Export(_) => {}
    }
}

fn check_expr(
    expr: &Expr,
    defined: &HashSet<String>,
    program: &HashSet<String>,
    diagnostics: &mut Vec<LintDiagnostic>,
    reported: &mut HashSet<(String, u32, u32)>,
) {
    match &expr.kind {
        ExprKind::Variable(name) => {
            if defined.contains(name) || program.contains(name) || is_likely_global(name) {
                return;
            }
            flag(name, expr.span, diagnostics, reported);
        }
        ExprKind::Assign { target, value } => {
            // LHS Variable is a binding site; don't flag it. Still recurse
            // into more complex LHS (Member, Index) to check their bases.
            match &target.kind {
                ExprKind::Variable(_) => {}
                _ => check_expr(target, defined, program, diagnostics, reported),
            }
            check_expr(value, defined, program, diagnostics, reported);
        }
        ExprKind::CompoundAssign { target, value, .. } => {
            // `x += 1` reads x as well as writes it — if x is never assigned
            // elsewhere, this still counts as an undefined read.
            check_expr(target, defined, program, diagnostics, reported);
            check_expr(value, defined, program, diagnostics, reported);
        }
        ExprKind::PostfixIncrement(target) | ExprKind::PostfixDecrement(target) => {
            check_expr(target, defined, program, diagnostics, reported);
        }
        ExprKind::Binary { left, right, .. } => {
            check_expr(left, defined, program, diagnostics, reported);
            check_expr(right, defined, program, diagnostics, reported);
        }
        ExprKind::Unary { operand, .. } => {
            check_expr(operand, defined, program, diagnostics, reported);
        }
        ExprKind::Grouping(inner) | ExprKind::Spread(inner) | ExprKind::Await(inner) => {
            check_expr(inner, defined, program, diagnostics, reported);
        }
        ExprKind::Call { callee, arguments } => {
            check_expr(callee, defined, program, diagnostics, reported);
            check_args(arguments, defined, program, diagnostics, reported);
        }
        ExprKind::Pipeline { left, right } => {
            check_expr(left, defined, program, diagnostics, reported);
            check_expr(right, defined, program, diagnostics, reported);
        }
        ExprKind::Member { object, .. }
        | ExprKind::SafeMember { object, .. }
        | ExprKind::QualifiedName {
            qualifier: object, ..
        } => check_expr(object, defined, program, diagnostics, reported),
        ExprKind::Index { object, index } => {
            check_expr(object, defined, program, diagnostics, reported);
            check_expr(index, defined, program, diagnostics, reported);
        }
        ExprKind::New {
            class_expr,
            arguments,
        } => {
            check_expr(class_expr, defined, program, diagnostics, reported);
            check_args(arguments, defined, program, diagnostics, reported);
        }
        ExprKind::Array(elements) => {
            for e in elements {
                check_expr(e, defined, program, diagnostics, reported);
            }
        }
        ExprKind::Hash(pairs) => {
            for (k, v) in pairs {
                check_expr(k, defined, program, diagnostics, reported);
                check_expr(v, defined, program, diagnostics, reported);
            }
        }
        ExprKind::Block(stmts) => {
            for s in stmts {
                check_stmt(s, defined, program, diagnostics, reported);
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            check_expr(condition, defined, program, diagnostics, reported);
            check_expr(then_branch, defined, program, diagnostics, reported);
            if let Some(e) = else_branch {
                check_expr(e, defined, program, diagnostics, reported);
            }
        }
        ExprKind::LogicalAnd { left, right }
        | ExprKind::LogicalOr { left, right }
        | ExprKind::NullishCoalescing { left, right } => {
            check_expr(left, defined, program, diagnostics, reported);
            check_expr(right, defined, program, diagnostics, reported);
        }
        ExprKind::Lambda { params, body, .. } => {
            // Lambdas have their own scope: include captured outer names
            // plus the lambda's own params and its body's assignments.
            let mut inner_defined: HashSet<String> = defined.clone();
            for p in params {
                inner_defined.insert(p.name.clone());
            }
            collect_assigned_in_stmts(body, &mut inner_defined);
            for s in body {
                check_stmt(s, &inner_defined, program, diagnostics, reported);
            }
        }
        ExprKind::Match {
            expression, arms, ..
        } => {
            check_expr(expression, defined, program, diagnostics, reported);
            for arm in arms {
                // Match arms bind pattern variables; conservatively allow
                // any name that appears in the arm body to be a pattern
                // binding by re-collecting the arm's defined set. This
                // avoids false positives on `match x { a => a + 1, ... }`.
                let mut arm_defined = defined.clone();
                collect_pattern_bindings(&arm.body, &mut arm_defined);
                check_expr(&arm.body, &arm_defined, program, diagnostics, reported);
            }
        }
        ExprKind::ListComprehension {
            element,
            variable,
            iterable,
            condition,
            ..
        } => {
            let mut inner_defined = defined.clone();
            inner_defined.insert(variable.clone());
            check_expr(iterable, defined, program, diagnostics, reported);
            check_expr(element, &inner_defined, program, diagnostics, reported);
            if let Some(c) = condition {
                check_expr(c, &inner_defined, program, diagnostics, reported);
            }
        }
        ExprKind::HashComprehension {
            key,
            value,
            variable,
            iterable,
            condition,
            ..
        } => {
            let mut inner_defined = defined.clone();
            inner_defined.insert(variable.clone());
            check_expr(iterable, defined, program, diagnostics, reported);
            check_expr(key, &inner_defined, program, diagnostics, reported);
            check_expr(value, &inner_defined, program, diagnostics, reported);
            if let Some(c) = condition {
                check_expr(c, &inner_defined, program, diagnostics, reported);
            }
        }
        ExprKind::InterpolatedString(parts) => {
            for p in parts {
                if let InterpolatedPart::Expression(e) = p {
                    check_expr(e, defined, program, diagnostics, reported);
                }
            }
        }
        ExprKind::Throw(e) => check_expr(e, defined, program, diagnostics, reported),
        ExprKind::Rescue { expr, fallback } => {
            check_expr(expr, defined, program, diagnostics, reported);
            check_expr(fallback, defined, program, diagnostics, reported);
        }
        ExprKind::IntLiteral(_)
        | ExprKind::FloatLiteral(_)
        | ExprKind::DecimalLiteral(_)
        | ExprKind::StringLiteral(_)
        | ExprKind::BoolLiteral(_)
        | ExprKind::Symbol(_)
        | ExprKind::Null
        | ExprKind::This
        | ExprKind::Super
        | ExprKind::CommandSubstitution(_)
        | ExprKind::SdqlBlock { .. } => {}
    }
}

fn check_args(
    arguments: &[Argument],
    defined: &HashSet<String>,
    program: &HashSet<String>,
    diagnostics: &mut Vec<LintDiagnostic>,
    reported: &mut HashSet<(String, u32, u32)>,
) {
    for arg in arguments {
        match arg {
            Argument::Positional(e) => check_expr(e, defined, program, diagnostics, reported),
            Argument::Named(named) => {
                check_expr(&named.value, defined, program, diagnostics, reported)
            }
            Argument::Block(e) => check_expr(e, defined, program, diagnostics, reported),
        }
    }
}

/// Collect names that any form of assignment/declaration introduces within
/// the statements (recursively through nested blocks). Doesn't descend into
/// nested function/class/lambda bodies — those have their own scope.
pub fn collect_assigned_in_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        collect_assigned_in_stmt(s, out);
    }
}

fn collect_assigned_in_stmt(stmt: &Stmt, out: &mut HashSet<String>) {
    match &stmt.kind {
        StmtKind::Let {
            name, initializer, ..
        } => {
            out.insert(name.clone());
            if let Some(init) = initializer {
                collect_assigned_in_expr(init, out);
            }
        }
        StmtKind::Const {
            name, initializer, ..
        } => {
            out.insert(name.clone());
            collect_assigned_in_expr(initializer, out);
        }
        StmtKind::Expression(e) => collect_assigned_in_expr(e, out),
        StmtKind::Block(stmts) => collect_assigned_in_stmts(stmts, out),
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_assigned_in_expr(condition, out);
            collect_assigned_in_stmt(then_branch, out);
            if let Some(e) = else_branch {
                collect_assigned_in_stmt(e, out);
            }
        }
        StmtKind::While { condition, body } => {
            collect_assigned_in_expr(condition, out);
            collect_assigned_in_stmt(body, out);
        }
        StmtKind::For {
            variable,
            index_variable,
            iterable,
            body,
        } => {
            out.insert(variable.clone());
            if let Some(idx) = index_variable {
                out.insert(idx.clone());
            }
            collect_assigned_in_expr(iterable, out);
            collect_assigned_in_stmt(body, out);
        }
        StmtKind::Return(e) => {
            if let Some(e) = e {
                collect_assigned_in_expr(e, out);
            }
        }
        StmtKind::Throw(e) => collect_assigned_in_expr(e, out),
        StmtKind::Try {
            try_block,
            catch_clauses,
            finally_block,
        } => {
            collect_assigned_in_stmt(try_block, out);
            for clause in catch_clauses {
                if let Some(var) = &clause.var_name {
                    out.insert(var.clone());
                }
                collect_assigned_in_stmt(&clause.body, out);
            }
            if let Some(f) = finally_block {
                collect_assigned_in_stmt(f, out);
            }
        }
        // Nested definitions introduce their name into this scope.
        StmtKind::Function(decl) => {
            out.insert(decl.name.clone());
        }
        StmtKind::Class(decl) => {
            out.insert(decl.name.clone());
        }
        StmtKind::Interface(decl) => {
            out.insert(decl.name.clone());
        }
        StmtKind::Import(decl) => insert_import_names(decl, out),
        StmtKind::Export(inner) => collect_assigned_in_stmt(inner, out),
    }
}

fn collect_assigned_in_expr(expr: &Expr, out: &mut HashSet<String>) {
    match &expr.kind {
        ExprKind::Assign { target, value } => {
            if let ExprKind::Variable(name) = &target.kind {
                out.insert(name.clone());
            }
            collect_assigned_in_expr(value, out);
        }
        ExprKind::CompoundAssign { target, value, .. } => {
            // x += ... does NOT introduce x — it assumes x exists. So
            // we don't add the target name here; but do recurse into value.
            collect_assigned_in_expr(target, out);
            collect_assigned_in_expr(value, out);
        }
        ExprKind::Call { callee, arguments } => {
            collect_assigned_in_expr(callee, out);
            for arg in arguments {
                match arg {
                    Argument::Positional(e) => collect_assigned_in_expr(e, out),
                    Argument::Named(n) => collect_assigned_in_expr(&n.value, out),
                    Argument::Block(e) => collect_assigned_in_expr(e, out),
                }
            }
        }
        ExprKind::Binary { left, right, .. } => {
            collect_assigned_in_expr(left, out);
            collect_assigned_in_expr(right, out);
        }
        ExprKind::LogicalAnd { left, right }
        | ExprKind::LogicalOr { left, right }
        | ExprKind::NullishCoalescing { left, right }
        | ExprKind::Pipeline { left, right } => {
            collect_assigned_in_expr(left, out);
            collect_assigned_in_expr(right, out);
        }
        ExprKind::Unary { operand, .. } => collect_assigned_in_expr(operand, out),
        ExprKind::Grouping(e) | ExprKind::Spread(e) | ExprKind::Await(e) | ExprKind::Throw(e) => {
            collect_assigned_in_expr(e, out);
        }
        ExprKind::Rescue { expr, fallback } => {
            collect_assigned_in_expr(expr, out);
            collect_assigned_in_expr(fallback, out);
        }
        ExprKind::Member { object, .. }
        | ExprKind::SafeMember { object, .. }
        | ExprKind::QualifiedName {
            qualifier: object, ..
        } => collect_assigned_in_expr(object, out),
        ExprKind::Index { object, index } => {
            collect_assigned_in_expr(object, out);
            collect_assigned_in_expr(index, out);
        }
        ExprKind::New {
            class_expr,
            arguments,
        } => {
            collect_assigned_in_expr(class_expr, out);
            for arg in arguments {
                match arg {
                    Argument::Positional(e) => collect_assigned_in_expr(e, out),
                    Argument::Named(n) => collect_assigned_in_expr(&n.value, out),
                    Argument::Block(e) => collect_assigned_in_expr(e, out),
                }
            }
        }
        ExprKind::Array(elements) => {
            for e in elements {
                collect_assigned_in_expr(e, out);
            }
        }
        ExprKind::Hash(pairs) => {
            for (k, v) in pairs {
                collect_assigned_in_expr(k, out);
                collect_assigned_in_expr(v, out);
            }
        }
        ExprKind::Block(stmts) => collect_assigned_in_stmts(stmts, out),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_assigned_in_expr(condition, out);
            collect_assigned_in_expr(then_branch, out);
            if let Some(e) = else_branch {
                collect_assigned_in_expr(e, out);
            }
        }
        ExprKind::Match {
            expression, arms, ..
        } => {
            collect_assigned_in_expr(expression, out);
            for arm in arms {
                // Bindings in the arm pattern are in scope of the arm body,
                // which is its own region. We don't expose them to the outer
                // scope, but the arm body's own assignments would be local
                // to a match expression used as an rvalue — track them here
                // conservatively as visible in the enclosing function.
                collect_assigned_in_expr(&arm.body, out);
            }
        }
        ExprKind::Lambda { .. } => {
            // Lambdas introduce their own scope — don't leak their locals.
        }
        ExprKind::ListComprehension {
            variable, iterable, ..
        } => {
            // The comprehension's own binding is scoped to it. Only the
            // iterable expression's assignments escape.
            collect_assigned_in_expr(iterable, out);
            // Ensure we don't forget the iteration variable — treat it as
            // visible to outer scope to avoid false positives if somehow
            // referenced afterward.
            out.insert(variable.clone());
        }
        ExprKind::HashComprehension {
            variable, iterable, ..
        } => {
            collect_assigned_in_expr(iterable, out);
            out.insert(variable.clone());
        }
        ExprKind::PostfixIncrement(target) | ExprKind::PostfixDecrement(target) => {
            collect_assigned_in_expr(target, out);
        }
        ExprKind::InterpolatedString(parts) => {
            for p in parts {
                if let InterpolatedPart::Expression(e) = p {
                    collect_assigned_in_expr(e, out);
                }
            }
        }
        ExprKind::IntLiteral(_)
        | ExprKind::FloatLiteral(_)
        | ExprKind::DecimalLiteral(_)
        | ExprKind::StringLiteral(_)
        | ExprKind::BoolLiteral(_)
        | ExprKind::Symbol(_)
        | ExprKind::Null
        | ExprKind::This
        | ExprKind::Super
        | ExprKind::Variable(_)
        | ExprKind::CommandSubstitution(_)
        | ExprKind::SdqlBlock { .. } => {}
    }
}

/// Stub — we conservatively allow any variable name referenced in a match
/// arm body to be treated as a pattern binding. Real pattern-binding
/// collection would walk arm.pattern; keeping this permissive prevents
/// false positives until that's wired up.
fn collect_pattern_bindings(body: &Expr, out: &mut HashSet<String>) {
    // Walk the arm body and pretend any bare Variable is a binding. This
    // is intentionally over-inclusive for now.
    fn walk(e: &Expr, out: &mut HashSet<String>) {
        if let ExprKind::Variable(name) = &e.kind {
            out.insert(name.clone());
        }
        match &e.kind {
            ExprKind::Binary { left, right, .. }
            | ExprKind::LogicalAnd { left, right }
            | ExprKind::LogicalOr { left, right }
            | ExprKind::NullishCoalescing { left, right }
            | ExprKind::Pipeline { left, right } => {
                walk(left, out);
                walk(right, out);
            }
            ExprKind::Unary { operand, .. } => walk(operand, out),
            ExprKind::Grouping(inner) | ExprKind::Spread(inner) | ExprKind::Await(inner) => {
                walk(inner, out)
            }
            _ => {}
        }
    }
    walk(body, out);
}

#[cfg(test)]
mod tests {
    use crate::lexer::Scanner;
    use crate::lint::Linter;
    use crate::parser::Parser;

    fn lint_rules(src: &str) -> Vec<String> {
        let tokens = Scanner::new(src).scan_tokens().expect("lex");
        let program = Parser::new(tokens).parse().expect("parse");
        Linter::new(src)
            .lint(&program)
            .into_iter()
            .map(|d| d.rule.to_string())
            .collect()
    }

    // `@foo` desugars to `Member { This, "foo" }` at parse time, so the
    // undefined-local scope check (which only fires on bare Variables) should
    // never see it and must stay silent.
    #[test]
    fn at_sigil_does_not_trigger_undefined_local() {
        let src = r#"
class Widget {
    value: Int;

    new(v: Int) {
        this.value = v;
    }

    fn peek() -> Int {
        @value
    }

    fn bump() {
        @value = @value + 1;
    }
}
"#;
        let rules = lint_rules(src);
        assert!(
            !rules.iter().any(|r| r == "smell/undefined-local"),
            "unexpected undefined-local diagnostic on @foo: {:?}",
            rules
        );
    }
}
