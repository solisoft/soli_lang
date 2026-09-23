use std::collections::HashSet;

use crate::ast::expr::{Expr, ExprKind};
use crate::ast::stmt::{ClassDecl, Stmt, StmtKind};
use crate::lint::{LintDiagnostic, Severity};
use crate::span::Span;

pub fn check_unreachable_code(stmts: &[Stmt], diagnostics: &mut Vec<LintDiagnostic>) {
    let mut found_return = false;
    for stmt in stmts {
        if found_return {
            diagnostics.push(LintDiagnostic {
                rule: "smell/unreachable-code",
                message: "unreachable code after return statement".to_string(),
                span: stmt.span,
                severity: Severity::Warning,
            });
            break; // Only report once per block
        }
        if matches!(stmt.kind, StmtKind::Return(_)) {
            found_return = true;
        }
    }
}

pub fn check_empty_catch(catch_block: &Stmt, diagnostics: &mut Vec<LintDiagnostic>) {
    if let StmtKind::Block(stmts) = &catch_block.kind {
        if stmts.is_empty() {
            diagnostics.push(LintDiagnostic {
                rule: "smell/empty-catch",
                message: "empty catch block".to_string(),
                span: catch_block.span,
                severity: Severity::Warning,
            });
        }
    }
}

pub fn check_duplicate_methods(class: &ClassDecl, diagnostics: &mut Vec<LintDiagnostic>) {
    let mut seen = HashSet::new();
    for method in &class.methods {
        if !seen.insert(&method.name) {
            diagnostics.push(LintDiagnostic {
                rule: "smell/duplicate-methods",
                message: format!(
                    "duplicate method '{}' in class '{}'",
                    method.name, class.name
                ),
                span: method.span,
                severity: Severity::Warning,
            });
        }
    }
}

pub fn check_deep_nesting(depth: usize, span: Span, diagnostics: &mut Vec<LintDiagnostic>) {
    if depth > 4 {
        diagnostics.push(LintDiagnostic {
            rule: "smell/deep-nesting",
            message: format!("nesting depth {} exceeds maximum of 4", depth),
            span,
            severity: Severity::Warning,
        });
    }
}

/// `smell/closure-cycle`: a closure stored on `this` (`this.x = fn() {...}`,
/// `@x = |y| ...`, `this.handlers["k"] = fn...`).
///
/// A closure keeps the whole environment it was created in, and a method's
/// environment binds `this`. Stored on the instance, the closure and the
/// instance hold each other, and without a cycle collector neither is ever
/// freed — per call, in a long-lived server. It does not matter whether the
/// body mentions `this`: the captured environment holds it either way.
pub fn check_closure_cycle(
    target: &Expr,
    value: &Expr,
    span: Span,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    if !is_rooted_at_this(target) {
        return;
    }
    let mut value = value;
    while let ExprKind::Grouping(inner) = &value.kind {
        value = inner;
    }
    if matches!(value.kind, ExprKind::Lambda { .. }) {
        diagnostics.push(LintDiagnostic {
            rule: "smell/closure-cycle",
            message: "a closure stored on `this` captures the method's environment, which \
                      holds `this`: the instance and the closure keep each other alive and \
                      are never freed; store a method name or pass the closure per call"
                .to_string(),
            span,
            severity: Severity::Warning,
        });
    }
}

fn is_rooted_at_this(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Member { object, .. }
        | ExprKind::SafeMember { object, .. }
        | ExprKind::Index { object, .. } => {
            matches!(object.kind, ExprKind::This) || is_rooted_at_this(object)
        }
        _ => false,
    }
}

#[cfg(test)]
mod closure_cycle_tests {
    use crate::lexer::Scanner;
    use crate::lint::Linter;
    use crate::parser::Parser;

    fn count(src: &str) -> usize {
        let tokens = Scanner::new(src).scan_tokens().expect("lex");
        let program = Parser::new(tokens).parse().expect("parse");
        Linter::new(src)
            .lint(&program)
            .into_iter()
            .filter(|d| d.rule == "smell/closure-cycle")
            .count()
    }

    #[test]
    fn a_closure_stored_on_this_is_flagged() {
        let src = r#"
class Counter {
    new() {
        this.on_tick = fn() { this.count = 1; };
        @formatter = |x| { x * 2 };
        this.handlers["k"] = (fn(y) y);
    }
}
"#;
        assert_eq!(count(src), 3);
    }

    #[test]
    fn plain_values_and_local_closures_are_not_flagged() {
        let src = r#"
class Counter {
    new() {
        this.count = 0;
        let double = fn(x) { x * 2 };
        this.total = [1, 2].map(fn(x) { x * 2 });
    }
}
"#;
        assert_eq!(count(src), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::expr::{Expr, ExprKind};
    use crate::ast::stmt::{MethodDecl, Visibility};

    fn span() -> Span {
        Span::new(0, 0, 1, 1)
    }

    #[test]
    fn test_unreachable_code_detected() {
        let stmts = vec![
            Stmt::new(StmtKind::Return(None), span(), None),
            Stmt::new(
                StmtKind::Expression(Expr::new(ExprKind::IntLiteral(1), span())),
                span(),
                None,
            ),
        ];
        let mut d = Vec::new();
        check_unreachable_code(&stmts, &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "smell/unreachable-code");
    }

    #[test]
    fn test_no_unreachable_code() {
        let stmts = vec![
            Stmt::new(
                StmtKind::Expression(Expr::new(ExprKind::IntLiteral(1), span())),
                span(),
                None,
            ),
            Stmt::new(StmtKind::Return(None), span(), None),
        ];
        let mut d = Vec::new();
        check_unreachable_code(&stmts, &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_unreachable_reports_only_once() {
        let stmts = vec![
            Stmt::new(StmtKind::Return(None), span(), None),
            Stmt::new(
                StmtKind::Expression(Expr::new(ExprKind::IntLiteral(1), span())),
                span(),
                None,
            ),
            Stmt::new(
                StmtKind::Expression(Expr::new(ExprKind::IntLiteral(2), span())),
                span(),
                None,
            ),
        ];
        let mut d = Vec::new();
        check_unreachable_code(&stmts, &mut d);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn test_empty_catch_detected() {
        let catch = Stmt::new(StmtKind::Block(vec![]), span(), None);
        let mut d = Vec::new();
        check_empty_catch(&catch, &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "smell/empty-catch");
    }

    #[test]
    fn test_non_empty_catch() {
        let catch = Stmt::new(
            StmtKind::Block(vec![Stmt::new(StmtKind::Return(None), span(), None)]),
            span(),
            None,
        );
        let mut d = Vec::new();
        check_empty_catch(&catch, &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_duplicate_methods_detected() {
        let class = ClassDecl {
            name: "Foo".to_string(),
            superclass: None,
            interfaces: vec![],
            fields: vec![],
            methods: vec![
                MethodDecl {
                    visibility: Visibility::Public,
                    is_static: false,
                    name: "bar".to_string(),
                    params: vec![],
                    return_type: None,
                    body: vec![],
                    span: span(),
                },
                MethodDecl {
                    visibility: Visibility::Public,
                    is_static: false,
                    name: "bar".to_string(),
                    params: vec![],
                    return_type: None,
                    body: vec![],
                    span: Span::new(0, 0, 5, 1),
                },
            ],
            constructor: None,
            static_block: None,
            class_statements: vec![],
            nested_classes: vec![],
            is_module: false,
            includes: vec![],
            extends: vec![],
            included_hooks: vec![],
            extended_hooks: vec![],
            concern_class_methods: vec![],
            span: span(),
        };
        let mut d = Vec::new();
        check_duplicate_methods(&class, &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "smell/duplicate-methods");
    }

    #[test]
    fn test_no_duplicate_methods() {
        let class = ClassDecl {
            name: "Foo".to_string(),
            superclass: None,
            interfaces: vec![],
            fields: vec![],
            methods: vec![
                MethodDecl {
                    visibility: Visibility::Public,
                    is_static: false,
                    name: "bar".to_string(),
                    params: vec![],
                    return_type: None,
                    body: vec![],
                    span: span(),
                },
                MethodDecl {
                    visibility: Visibility::Public,
                    is_static: false,
                    name: "baz".to_string(),
                    params: vec![],
                    return_type: None,
                    body: vec![],
                    span: span(),
                },
            ],
            constructor: None,
            static_block: None,
            class_statements: vec![],
            nested_classes: vec![],
            is_module: false,
            includes: vec![],
            extends: vec![],
            included_hooks: vec![],
            extended_hooks: vec![],
            concern_class_methods: vec![],
            span: span(),
        };
        let mut d = Vec::new();
        check_duplicate_methods(&class, &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_deep_nesting_within_limit() {
        let mut d = Vec::new();
        check_deep_nesting(4, span(), &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_deep_nesting_exceeds_limit() {
        let mut d = Vec::new();
        check_deep_nesting(5, span(), &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "smell/deep-nesting");
    }
}
