use std::collections::HashSet;

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
            Stmt::new(StmtKind::Return(None), span()),
            Stmt::new(
                StmtKind::Expression(Expr::new(ExprKind::IntLiteral(1), span())),
                span(),
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
            ),
            Stmt::new(StmtKind::Return(None), span()),
        ];
        let mut d = Vec::new();
        check_unreachable_code(&stmts, &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_unreachable_reports_only_once() {
        let stmts = vec![
            Stmt::new(StmtKind::Return(None), span()),
            Stmt::new(
                StmtKind::Expression(Expr::new(ExprKind::IntLiteral(1), span())),
                span(),
            ),
            Stmt::new(
                StmtKind::Expression(Expr::new(ExprKind::IntLiteral(2), span())),
                span(),
            ),
        ];
        let mut d = Vec::new();
        check_unreachable_code(&stmts, &mut d);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn test_empty_catch_detected() {
        let catch = Stmt::new(StmtKind::Block(vec![]), span());
        let mut d = Vec::new();
        check_empty_catch(&catch, &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "smell/empty-catch");
    }

    #[test]
    fn test_non_empty_catch() {
        let catch = Stmt::new(
            StmtKind::Block(vec![Stmt::new(StmtKind::Return(None), span())]),
            span(),
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
