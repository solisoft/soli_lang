//! Soli-specific idiom rules. These flag code that is *correct* but not
//! idiomatic — the patterns CLAUDE.md calls out as fluency tells: comparing
//! against `null`/`""` instead of `.nil?`/`.blank?`, and chained `==`/`!=`
//! membership tests instead of `.includes?`.

use crate::ast::expr::{BinaryOp, Expr, ExprKind};
use crate::ast::stmt::{Stmt, StmtKind};
use crate::lint::{LintDiagnostic, Severity};
use crate::span::Span;

/// `x == null` / `x != null` → prefer `x.nil?` / `x.present?`.
///
/// Called on every `Binary` node. Only equality/inequality against a bare
/// `null` literal is flagged; `??` and `&.` already cover the safe-navigation
/// cases, so we leave those alone.
pub fn check_nil_comparison(
    left: &Expr,
    operator: BinaryOp,
    right: &Expr,
    span: Span,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let (op_str, suggestion) = match operator {
        BinaryOp::Equal => ("==", ".nil?"),
        BinaryOp::NotEqual => ("!=", ".present?"),
        _ => return,
    };
    if !is_null(left) && !is_null(right) {
        return;
    }
    // `null == null` is degenerate; don't bother.
    if is_null(left) && is_null(right) {
        return;
    }
    diagnostics.push(LintDiagnostic {
        rule: "idiom/nil-comparison",
        message: format!(
            "prefer `{suggestion}` over `{op_str} null` — it reads better and \
             handles the nil case directly"
        ),
        span,
        severity: Severity::Warning,
    });
}

/// `x == ""` / `x != ""` → prefer `x.blank?` / `x.present?`.
///
/// `.blank?` folds nil and empty-string into one check, so it's the idiomatic
/// emptiness test in Soli (see CLAUDE.md "concise defaults and guards").
pub fn check_prefer_blank(
    left: &Expr,
    operator: BinaryOp,
    right: &Expr,
    span: Span,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let suggestion = match operator {
        BinaryOp::Equal => ".blank?",
        BinaryOp::NotEqual => ".present?",
        _ => return,
    };
    if !is_empty_string(left) && !is_empty_string(right) {
        return;
    }
    diagnostics.push(LintDiagnostic {
        rule: "idiom/prefer-blank",
        message: format!(
            "prefer `{suggestion}` over comparing to an empty string — `.blank?` \
             also covers the nil case"
        ),
        span,
        severity: Severity::Warning,
    });
}

/// Chained membership tests:
///   `x == "a" || x == "b" || x == "c"`   → `["a","b","c"].includes?(x)`
///   `x != "a" && x != "b" && x != "c"`   → `unless [...].includes?(x)`
///
/// `operands` is the flattened list of leaves of an `||` (or `&&`) chain.
/// `is_or` selects which equality operator (`==` for `||`, `!=` for `&&`) the
/// leaves must use. We only flag chains of 3+ comparisons of the *same* value
/// against literals — two-way checks read fine as-is.
pub fn check_prefer_includes(
    operands: &[&Expr],
    is_or: bool,
    span: Span,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    const MIN_CHAIN: usize = 3;
    if operands.len() < MIN_CHAIN {
        return;
    }
    let want_op = if is_or {
        BinaryOp::Equal
    } else {
        BinaryOp::NotEqual
    };

    let mut subject: Option<String> = None;
    for operand in operands {
        let Some(key) = comparison_subject(operand, want_op) else {
            return;
        };
        match &subject {
            None => subject = Some(key),
            Some(existing) if *existing != key => return,
            _ => {}
        }
    }

    let suggestion = if is_or {
        "[...].includes?(x)"
    } else {
        "unless [...].includes?(x)"
    };
    diagnostics.push(LintDiagnostic {
        rule: "idiom/prefer-includes",
        message: format!(
            "prefer `{suggestion}` over a chain of {} comparisons against the same value",
            operands.len()
        ),
        span,
        severity: Severity::Warning,
    });
}

/// Dead nil-guard right after a `.find(...)`:
///   record = User.find(id)
///   return not_found() if record.nil?   # <- never runs: .find raises RecordNotFound
///
/// `Model.find(id)` raises `RecordNotFound` on a miss (which the request
/// handler turns into a 404), so a manual nil-check on its result is
/// unreachable. Use `find_by`/`first_by` when you actually want "or nil".
/// Operates on a statement slice so it can see the binding and the guard that
/// follows it.
pub fn check_manual_find_guard(stmts: &[Stmt], diagnostics: &mut Vec<LintDiagnostic>) {
    for pair in stmts.windows(2) {
        let Some(bound) = binding_from_find(&pair[0]) else {
            continue;
        };
        let Some((guard_var, guard_span)) = nil_guard(&pair[1]) else {
            continue;
        };
        if bound != guard_var {
            continue;
        }
        diagnostics.push(LintDiagnostic {
            rule: "idiom/manual-find-guard",
            message: format!(
                "`{bound}` comes from `.find(...)`, which raises when the record is \
                 missing — this nil-check never runs. Drop it, or use `find_by`/`first_by` \
                 if you want a nil result"
            ),
            span: guard_span,
            severity: Severity::Warning,
        });
    }
}

/// If `stmt` binds a variable to the result of a `.find(...)` call, return the
/// variable name. Handles both `let x = M.find(..)` and `x = M.find(..)`.
fn binding_from_find(stmt: &Stmt) -> Option<String> {
    let (name, value) = match &stmt.kind {
        StmtKind::Let {
            name,
            initializer: Some(init),
            ..
        } => (name.clone(), init),
        StmtKind::Expression(expr) => match &expr.kind {
            ExprKind::Assign { target, value } => (simple_path_key(target)?, value.as_ref()),
            _ => return None,
        },
        _ => return None,
    };
    if is_find_call(value) {
        Some(name)
    } else {
        None
    }
}

fn is_find_call(expr: &Expr) -> bool {
    let ExprKind::Call { callee, .. } = &expr.kind else {
        return false;
    };
    matches!(
        &callee.kind,
        ExprKind::Member { name, .. } | ExprKind::SafeMember { name, .. } if name == "find"
    )
}

/// If `stmt` is a nil-guard on a single variable — `if x.nil? ...`,
/// `if x == null ...`, postfix `return ... if x.nil?`, etc. — return that
/// variable's name and the guard's span.
fn nil_guard(stmt: &Stmt) -> Option<(String, Span)> {
    let StmtKind::If { condition, .. } = &stmt.kind else {
        return None;
    };
    nil_check_var(condition).map(|v| (v, stmt.span))
}

/// The variable being nil-checked by `expr`, if `expr` is `x.nil?` or
/// `x == null` / `null == x`.
fn nil_check_var(expr: &Expr) -> Option<String> {
    match &expr.kind {
        // x.nil? — parses as a zero-arg member access or a no-arg call.
        ExprKind::Member { object, name } if name == "nil?" => simple_path_key(object),
        ExprKind::Call { callee, arguments } if arguments.is_empty() => match &callee.kind {
            ExprKind::Member { object, name } if name == "nil?" => simple_path_key(object),
            _ => None,
        },
        // x == null / null == x
        ExprKind::Binary {
            left,
            operator: BinaryOp::Equal,
            right,
        } => {
            if is_null(right) {
                simple_path_key(left)
            } else if is_null(left) {
                simple_path_key(right)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// For a `subject <op> literal` (or `literal <op> subject`) binary, return the
/// subject's path key. `None` if the node isn't that shape.
fn comparison_subject(expr: &Expr, op: BinaryOp) -> Option<String> {
    let ExprKind::Binary {
        left,
        operator,
        right,
    } = &expr.kind
    else {
        return None;
    };
    if *operator != op {
        return None;
    }
    if let (Some(key), true) = (simple_path_key(left), is_literal(right)) {
        return Some(key);
    }
    if let (true, Some(key)) = (is_literal(left), simple_path_key(right)) {
        return Some(key);
    }
    None
}

/// A stable string key for a "simple value path" — a bare variable, `this`,
/// or a dotted member chain rooted at one of those. Returns `None` for
/// anything with side effects (calls, indexing, arithmetic) so we never claim
/// two different expressions are "the same value".
fn simple_path_key(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Variable(name) => Some(name.clone()),
        ExprKind::This => Some("this".to_string()),
        ExprKind::Member { object, name } | ExprKind::SafeMember { object, name } => {
            Some(format!("{}.{}", simple_path_key(object)?, name))
        }
        _ => None,
    }
}

fn is_literal(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::StringLiteral(_)
            | ExprKind::IntLiteral(_)
            | ExprKind::FloatLiteral(_)
            | ExprKind::BoolLiteral(_)
    )
}

fn is_null(expr: &Expr) -> bool {
    matches!(expr.kind, ExprKind::Null)
}

fn is_empty_string(expr: &Expr) -> bool {
    matches!(&expr.kind, ExprKind::StringLiteral(s) if s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str) -> Expr {
        Expr::new(ExprKind::Variable(name.to_string()), Span::new(0, 0, 1, 1))
    }
    fn string(s: &str) -> Expr {
        Expr::new(
            ExprKind::StringLiteral(s.to_string()),
            Span::new(0, 0, 1, 1),
        )
    }
    fn null() -> Expr {
        Expr::new(ExprKind::Null, Span::new(0, 0, 1, 1))
    }
    fn eq(l: Expr, r: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                left: Box::new(l),
                operator: BinaryOp::Equal,
                right: Box::new(r),
            },
            Span::new(0, 0, 1, 1),
        )
    }

    #[test]
    fn flags_eq_null() {
        let mut d = Vec::new();
        check_nil_comparison(
            &var("x"),
            BinaryOp::Equal,
            &null(),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "idiom/nil-comparison");
    }

    #[test]
    fn ignores_non_null_eq() {
        let mut d = Vec::new();
        check_nil_comparison(
            &var("x"),
            BinaryOp::Equal,
            &string("y"),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn flags_empty_string_eq() {
        let mut d = Vec::new();
        check_prefer_blank(
            &var("x"),
            BinaryOp::Equal,
            &string(""),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "idiom/prefer-blank");
    }

    #[test]
    fn ignores_nonempty_string_eq() {
        let mut d = Vec::new();
        check_prefer_blank(
            &var("x"),
            BinaryOp::Equal,
            &string("hi"),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn flags_three_way_or_chain() {
        let mut d = Vec::new();
        let a = eq(var("s"), string("a"));
        let b = eq(var("s"), string("b"));
        let c = eq(var("s"), string("c"));
        let operands = vec![&a, &b, &c];
        check_prefer_includes(&operands, true, Span::new(0, 0, 1, 1), &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "idiom/prefer-includes");
    }

    #[test]
    fn ignores_two_way_chain() {
        let mut d = Vec::new();
        let a = eq(var("s"), string("a"));
        let b = eq(var("s"), string("b"));
        let operands = vec![&a, &b];
        check_prefer_includes(&operands, true, Span::new(0, 0, 1, 1), &mut d);
        assert!(d.is_empty());
    }

    fn call(callee: Expr, args: Vec<Expr>) -> Expr {
        use crate::ast::expr::Argument;
        Expr::new(
            ExprKind::Call {
                callee: Box::new(callee),
                arguments: args.into_iter().map(Argument::Positional).collect(),
            },
            Span::new(0, 0, 1, 1),
        )
    }
    fn member(object: Expr, name: &str) -> Expr {
        Expr::new(
            ExprKind::Member {
                object: Box::new(object),
                name: name.to_string(),
            },
            Span::new(0, 0, 1, 1),
        )
    }
    fn let_stmt(name: &str, init: Expr) -> Stmt {
        Stmt::new(
            StmtKind::Let {
                name: name.to_string(),
                type_annotation: None,
                initializer: Some(init),
            },
            Span::new(0, 0, 1, 1),
            None,
        )
    }
    fn if_stmt(condition: Expr) -> Stmt {
        let ret = Stmt::new(StmtKind::Return(None), Span::new(0, 0, 2, 1), None);
        Stmt::new(
            StmtKind::If {
                condition,
                then_branch: Box::new(ret),
                else_branch: None,
            },
            Span::new(0, 0, 2, 1),
            None,
        )
    }

    #[test]
    fn flags_nil_guard_after_find() {
        // user = User.find(id); if user.nil? { return }
        let find = call(member(var("User"), "find"), vec![var("id")]);
        let stmts = vec![let_stmt("user", find), if_stmt(member(var("user"), "nil?"))];
        let mut d = Vec::new();
        check_manual_find_guard(&stmts, &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "idiom/manual-find-guard");
    }

    #[test]
    fn ignores_guard_after_find_by() {
        let find_by = call(member(var("User"), "find_by"), vec![var("id")]);
        let stmts = vec![
            let_stmt("user", find_by),
            if_stmt(member(var("user"), "nil?")),
        ];
        let mut d = Vec::new();
        check_manual_find_guard(&stmts, &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn ignores_guard_on_other_var() {
        let find = call(member(var("User"), "find"), vec![var("id")]);
        let stmts = vec![
            let_stmt("user", find),
            if_stmt(member(var("other"), "nil?")),
        ];
        let mut d = Vec::new();
        check_manual_find_guard(&stmts, &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn ignores_mixed_subjects() {
        let mut d = Vec::new();
        let a = eq(var("s"), string("a"));
        let b = eq(var("t"), string("b"));
        let c = eq(var("s"), string("c"));
        let operands = vec![&a, &b, &c];
        check_prefer_includes(&operands, true, Span::new(0, 0, 1, 1), &mut d);
        assert!(d.is_empty());
    }
}
