pub mod expressions;
pub mod rules;
pub mod statements;
pub mod suppress;

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
        rules::props::check_component_props(&program.statements, &mut self.diagnostics);
        rules::scope::collect_program_names(&program.statements, &mut self.program_names);
        self.collect_sibling_definitions();

        for stmt in &program.statements {
            self.lint_stmt(stmt);
        }

        let suppressions = suppress::collect_suppressions(&self.source);
        self.diagnostics
            .retain(|d| !suppressions.suppresses(d.span.line as u32, d.rule));

        self.diagnostics
            .sort_by_key(|d| (d.span.line, d.span.column));
        self.diagnostics
    }

    /// Soli MVC auto-loads every `.sl` file in `app/controllers/`,
    /// `app/helpers/`, `app/middleware/`, `app/models/`, `app/services/`,
    /// and `app/jobs/` into a shared scope at runtime. When we lint one
    /// file, top-level definitions from *all* of those directories are
    /// effectively in-scope too. We find the nearest `app/` ancestor, then
    /// scan every auto-load subdirectory under it.
    fn collect_sibling_definitions(&mut self) {
        let Some(file_path) = &self.file_path else {
            return;
        };
        let path = std::path::Path::new(file_path);

        // Walk up the directory tree to find the nearest ancestor named "app/".
        // If the file is not inside any "app/" tree it's a standalone script —
        // return early so we don't silently pull names from random neighbours.
        let app_dir = {
            let mut dir = path.parent();
            loop {
                match dir {
                    None => break None,
                    Some(d) if d.file_name().and_then(|n| n.to_str()) == Some("app") => {
                        break Some(d.to_path_buf());
                    }
                    Some(d) => dir = d.parent(),
                }
            }
        };
        let Some(app_dir) = app_dir else {
            return;
        };

        const AUTOLOAD_DIRS: &[&str] = &[
            "controllers",
            "helpers",
            "middleware",
            "models",
            "services",
            "jobs",
        ];

        for dir_name in AUTOLOAD_DIRS {
            let dir_path = app_dir.join(dir_name);
            let Ok(entries) = std::fs::read_dir(&dir_path) else {
                continue;
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
    }

    fn lint_body(&mut self, stmts: &[crate::ast::Stmt]) {
        rules::smell::check_unreachable_code(stmts, &mut self.diagnostics);
        rules::idiom::check_manual_find_guard(stmts, &mut self.diagnostics);
        for stmt in stmts {
            self.lint_stmt(stmt);
        }
    }
}
