pub mod expressions;
pub mod rules;
pub mod statements;

use crate::ast::Program;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Warning,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Warning => write!(f, "warning"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LintDiagnostic {
    pub rule: &'static str,
    pub message: String,
    pub span: Span,
    pub severity: Severity,
}

pub struct Linter {
    source: String,
    diagnostics: Vec<LintDiagnostic>,
    depth: usize,
}

impl Linter {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            diagnostics: Vec::new(),
            depth: 0,
        }
    }

    pub fn lint(mut self, program: &Program) -> Vec<LintDiagnostic> {
        rules::style::check_line_lengths(&self.source, &mut self.diagnostics);

        for stmt in &program.statements {
            self.lint_stmt(stmt);
        }

        self.diagnostics
            .sort_by_key(|d| (d.span.line, d.span.column));
        self.diagnostics
    }

    fn lint_body(&mut self, stmts: &[crate::ast::Stmt]) {
        rules::smell::check_unreachable_code(stmts, &mut self.diagnostics);
        for stmt in stmts {
            self.lint_stmt(stmt);
        }
    }
}
