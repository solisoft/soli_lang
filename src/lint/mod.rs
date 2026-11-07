pub mod expressions;
pub mod ignore;
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
    /// `true` while linting code whose `this` is the class itself rather than
    /// an instance: a `static { … }` block, a `static def`, and class-level
    /// statements. A closure stored there (`this.before_action = fn(req) {…}`)
    /// is held by the class, which lives for the whole process, so
    /// `smell/closure-cycle` does not apply. A lambda body resets it: its
    /// `this` is whatever the lambda is later called with.
    pub(crate) class_level_this: bool,
    /// The file is a spec (`tests/`, `*_spec.sl`…): the test DSL's globals
    /// count as builtins for `smell/shadowed-builtin`. Set by `lint()`.
    pub(crate) test_file: bool,
    /// The names the file's auto-loaded neighbours declare, handed in by a
    /// caller that already has them (the language server's project index).
    /// When set, `lint()` does not walk `app/` on disk for them.
    known_names: Option<std::collections::HashSet<String>>,
}

impl Linter {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            file_path: None,
            diagnostics: Vec::new(),
            depth: 0,
            program_names: std::collections::HashSet::new(),
            class_level_this: false,
            test_file: false,
            known_names: None,
        }
    }

    pub fn with_file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// The top-level names declared by the file's auto-loaded neighbours,
    /// precomputed: they count as in scope, and `lint()` skips its own walk of
    /// `app/` (which re-parses every sibling on each call).
    pub fn with_known_names(mut self, names: std::collections::HashSet<String>) -> Self {
        self.known_names = Some(names);
        self
    }

    pub fn lint(mut self, program: &Program) -> Vec<LintDiagnostic> {
        self.test_file = self.is_test_file();
        rules::style::check_line_lengths(&self.source, &mut self.diagnostics);
        rules::docs::check_openapi_docs(
            &self.source,
            self.file_path.as_deref(),
            &mut self.diagnostics,
        );
        rules::props::check_component_props(&program.statements, &mut self.diagnostics);
        rules::scope::collect_program_names(&program.statements, &mut self.program_names);
        self.collect_sibling_definitions();
        rules::shadow::check_top_level(
            &program.statements,
            &self.program_names,
            self.test_file,
            &mut self.diagnostics,
        );

        for stmt in &program.statements {
            self.lint_stmt(stmt);
        }

        let suppressions = suppress::collect_suppressions(&self.source);
        self.diagnostics
            .retain(|d| !suppressions.suppresses(d.span.line, d.rule));

        self.diagnostics
            .sort_by_key(|d| (d.span.line, d.span.column));
        self.diagnostics
    }

    /// Soli MVC auto-loads every `.sl` file in the application's global
    /// directories (`app/models`, `app/controllers`, `app/services`,
    /// `app/policies`, `app/mailers`, `app/middleware`, `app/jobs`,
    /// `app/subscribers` — see [`crate::app_layout`]) into one shared scope,
    /// and the view helpers in `app/helpers` next to it. When we lint one
    /// file, top-level definitions from all of those directories are in scope
    /// too. Helpers are counted for every file: a name the linter cannot see
    /// is a false `smell/undefined-local`, and one it sees too generously only
    /// misses a rare mistake. We find the nearest `app/` ancestor and scan
    /// those directories under its parent, unless the caller already handed
    /// the names in ([`Linter::with_known_names`]).
    fn collect_sibling_definitions(&mut self) {
        if let Some(known) = self.known_names.take() {
            self.program_names.extend(known);
            return;
        }
        let Some(file_path) = &self.file_path else {
            return;
        };
        let path = std::path::Path::new(file_path);

        // Walk up the directory tree to find the nearest ancestor named "app/".
        // If the file is not inside any "app/" tree it's a standalone script —
        // return early so we don't silently pull names from random neighbours.
        let Some(root) = path
            .ancestors()
            .skip(1)
            .find(|d| d.file_name().and_then(|n| n.to_str()) == Some("app"))
            .and_then(|app| app.parent())
        else {
            return;
        };

        let dirs: Vec<&str> = crate::app_layout::APP_GLOBAL_DIRS
            .iter()
            .chain(crate::app_layout::HELPER_DIRS)
            .copied()
            .collect();
        let names = crate::app_layout::top_level_names(root, &dirs, Some(path));
        self.program_names.extend(names);
    }

    /// Whether the file is one `soli test` runs, where the test DSL
    /// (`describe`, `context`, `expect`…) is in scope too.
    fn is_test_file(&self) -> bool {
        self.file_path
            .as_deref()
            .is_some_and(|path| crate::is_test_path(std::path::Path::new(path)))
    }
}

/// The per-scope checks of a function, method or constructor body:
/// `smell/undefined-local` and `smell/shadowed-builtin`.
pub(crate) fn check_scope(
    params: &[crate::ast::stmt::Parameter],
    body: &[crate::ast::Stmt],
    visible_names: &std::collections::HashSet<String>,
    test_file: bool,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    rules::scope::check_undefined_locals(params, body, visible_names, diagnostics);
    rules::shadow::check_shadowed_builtins(params, body, visible_names, test_file, diagnostics);
}

impl Linter {
    fn lint_body(&mut self, stmts: &[crate::ast::Stmt]) {
        rules::smell::check_unreachable_code(stmts, &mut self.diagnostics);
        rules::idiom::check_manual_find_guard(stmts, &mut self.diagnostics);
        for stmt in stmts {
            self.lint_stmt(stmt);
        }
    }
}
