//! `component/props` — validates `props(...)` declarations in component
//! templates: every argument must be a string literal and names must be unique.
//!
//! The missing/unknown-prop checks are cross-file (a caller passes data from a
//! *different* file), so they run at render time in `--dev`, not here. This rule
//! only checks that the declaration itself is well-formed.

use crate::ast::expr::{Argument, ExprKind};
use crate::ast::stmt::{Stmt, StmtKind};
use crate::lint::{LintDiagnostic, Severity};
use std::collections::HashSet;

const RULE: &str = "component/props";

pub fn check_component_props(statements: &[Stmt], diagnostics: &mut Vec<LintDiagnostic>) {
    for stmt in statements {
        let StmtKind::Expression(expr) = &stmt.kind else {
            continue;
        };
        let ExprKind::Call { callee, arguments } = &expr.kind else {
            continue;
        };
        if !matches!(&callee.kind, ExprKind::Variable(name) if name == "props") {
            continue;
        }
        let mut seen: HashSet<String> = HashSet::new();
        for arg in arguments {
            let Argument::Positional(a) = arg else {
                diagnostics.push(LintDiagnostic {
                    rule: RULE,
                    message: "props(...) takes positional string arguments only".to_string(),
                    span: expr.span,
                    severity: Severity::Warning,
                });
                continue;
            };
            match &a.kind {
                ExprKind::StringLiteral(name) => {
                    if !seen.insert(name.clone()) {
                        diagnostics.push(LintDiagnostic {
                            rule: RULE,
                            message: format!("duplicate prop \"{}\" in props(...)", name),
                            span: a.span,
                            severity: Severity::Warning,
                        });
                    }
                }
                _ => diagnostics.push(LintDiagnostic {
                    rule: RULE,
                    message: "props(...) arguments must be string literals, e.g. props(\"title\")"
                        .to_string(),
                    span: a.span,
                    severity: Severity::Warning,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    fn lint_src(src: &str) -> Vec<LintDiagnostic> {
        let tokens = Scanner::new(src).scan_tokens().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut diags = Vec::new();
        check_component_props(&program.statements, &mut diags);
        diags
    }

    #[test]
    fn clean_props_declaration_passes() {
        assert!(lint_src("props(\"title\", \"value\")").is_empty());
    }

    #[test]
    fn duplicate_prop_flagged() {
        let d = lint_src("props(\"title\", \"title\")");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("duplicate"));
        assert_eq!(d[0].rule, "component/props");
    }

    #[test]
    fn non_literal_arg_flagged() {
        let d = lint_src("props(\"title\", x)");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("string literal"));
    }

    #[test]
    fn non_props_call_ignored() {
        assert!(lint_src("render(\"x\", y)").is_empty());
    }
}
