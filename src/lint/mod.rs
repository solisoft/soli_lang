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
    pub(crate) file_path: Option<String>,
    diagnostics: Vec<LintDiagnostic>,
    depth: usize,
    /// Names defined at program top-level (top-level lets, consts, fns,
    /// classes, interfaces, and imported symbols). Used by scope-sensitive
    /// rules to distinguish "truly undefined" from "defined elsewhere in
    /// this file". Populated at the start of `lint()`.
    pub(crate) program_names: std::collections::HashSet<String>,
}

impl Linter {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            file_path: None,
            diagnostics: Vec::new(),
            depth: 0,
            program_names: std::collections::HashSet::new(),
        }
    }

    pub fn with_file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    pub fn lint(mut self, program: &Program) -> Vec<LintDiagnostic> {
        rules::style::check_line_lengths(&self.source, &mut self.diagnostics);
        rules::scope::collect_program_names(&program.statements, &mut self.program_names);
        self.collect_sibling_definitions();

        for stmt in &program.statements {
            self.lint_stmt(stmt);
        }

        self.diagnostics
            .sort_by_key(|d| (d.span.line, d.span.column));
        self.diagnostics
    }

    /// Soli MVC auto-loads every `.sl` file in `app/controllers/`,
    /// `app/helpers/`, `app/middleware/`, and `app/models/` into a shared
    /// scope at runtime. So when we lint one file, top-level definitions
    /// in its siblings are effectively in-scope too. Parse each sibling
    /// and merge its program-level names in.
    fn collect_sibling_definitions(&mut self) {
        let Some(file_path) = &self.file_path else {
            return;
        };
        let path = std::path::Path::new(file_path);
        let Some(parent) = path.parent() else {
            return;
        };
        // Only do this for conventional MVC auto-load directories. This
        // keeps the behavior scoped: a script in an arbitrary directory
        // doesn't silently pull names from its neighbors.
        let is_autoload_dir = parent
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| matches!(n, "controllers" | "helpers" | "middleware" | "models"));
        if !is_autoload_dir {
            return;
        }
        let Ok(entries) = std::fs::read_dir(parent) else {
            return;
        };
        for entry in entries.flatten() {
            let sibling = entry.path();
            if sibling == path {
                continue;
            }
            if sibling.extension().and_then(|e| e.to_str()) != Some("sl") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&sibling) else {
                continue;
            };
            let Ok(tokens) = crate::lexer::Scanner::new(&source).scan_tokens() else {
                continue;
            };
            let Ok(sibling_program) = crate::parser::Parser::new(tokens).parse() else {
                continue;
            };
            rules::scope::collect_program_names(
                &sibling_program.statements,
                &mut self.program_names,
            );
        }
    }

    fn lint_body(&mut self, stmts: &[crate::ast::Stmt]) {
        rules::smell::check_unreachable_code(stmts, &mut self.diagnostics);
        for stmt in stmts {
            self.lint_stmt(stmt);
        }
    }
}
