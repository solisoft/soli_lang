//! Solilang: A statically-typed, class-based OOP language with pipeline operators.
//!
//! This is the library root that exports all modules.
//!
//! # Execution
//!
//! Solilang uses a tree-walking interpreter for executing programs.

// Allow some clippy lints that are stylistic and not critical
#![allow(clippy::module_inception)]
#![allow(clippy::result_large_err)]
#![allow(clippy::arc_with_non_send_sync)]
#![allow(clippy::only_used_in_recursion)]
#![allow(clippy::type_complexity)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::new_without_default)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::wildcard_in_or_patterns)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::unnecessary_lazy_evaluations)]
#![allow(clippy::len_zero)]
#![allow(clippy::redundant_pattern_matching)]
#![allow(clippy::trim_split_whitespace)]
#![allow(clippy::to_string_in_format_args)]
#![allow(clippy::while_let_on_iterator)]
#![allow(clippy::manual_ok_err)]
#![allow(clippy::unwrap_or_default)]
#![allow(clippy::unnecessary_filter_map)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::redundant_closure)]
#![allow(clippy::unused_enumerate_index)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::let_underscore_future)]
#![allow(clippy::double_ended_iterator_last)]
#![allow(clippy::needless_late_init)]
#![allow(clippy::manual_strip)]
#![allow(clippy::never_loop)]

pub mod ast;
pub mod coverage;
pub mod error;
pub mod interpreter;
pub mod lexer;
pub mod live;
pub mod migration;
pub mod module;
pub mod parser;
pub mod scaffold;
pub mod serve;
pub mod solidb_http;
pub mod span;
pub mod template;
pub mod types;

use error::SolilangError;

/// Run a Solilang program from source code.
pub fn run(source: &str) -> Result<(), SolilangError> {
    run_with_options(source, true)
}

/// Run a Solilang program with optional type checking.
pub fn run_with_type_check(source: &str, type_check: bool) -> Result<(), SolilangError> {
    run_with_options(source, type_check)
}

/// Run a Solilang program with full control over execution options.
pub fn run_with_options(
    source: &str,
    type_check: bool,
) -> Result<(), SolilangError> {
    run_with_path(source, None, type_check)
}

/// Run a Solilang program from a file path with module resolution.
pub fn run_file(
    path: &std::path::Path,
    type_check: bool,
) -> Result<(), SolilangError> {
    let source = std::fs::read_to_string(path).map_err(|e| error::RuntimeError::General {
        message: format!("Failed to read file '{}': {}", path.display(), e),
        span: span::Span::new(0, 0, 1, 1),
    })?;

    run_with_path(&source, Some(path), type_check)
}

/// Run a Solilang program with optional source path for module resolution.
pub fn run_with_path(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
) -> Result<(), SolilangError> {
    // Lexing
    let tokens = lexer::Scanner::new(source).scan_tokens()?;

    // Parsing
    let mut program = parser::Parser::new(tokens).parse()?;

    // Module resolution (if we have imports and a source path)
    if let Some(path) = source_path.filter(|_| has_imports(&program)) {
        let base_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let mut resolver = module::ModuleResolver::new(base_dir);
        program = resolver
            .resolve(program, path)
            .map_err(|e| error::RuntimeError::General {
                message: format!("Module resolution error: {}", e),
                span: span::Span::new(0, 0, 1, 1),
            })?;
    }

    // Type checking (optional)
    if type_check {
        let mut checker = types::TypeChecker::new();
        if let Err(errors) = checker.check(&program) {
            return Err(errors.into_iter().next().unwrap().into());
        }
    }

    // Execute with tree-walking interpreter
    let mut interpreter = interpreter::Interpreter::new();
    interpreter.interpret(&program)?;

    Ok(())
}

/// Run a Solilang program with optional coverage tracking.
pub fn run_with_path_and_coverage(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
    coverage_tracker: Option<&std::rc::Rc<std::cell::RefCell<coverage::CoverageTracker>>>,
    source_file_path: Option<&std::path::Path>,
) -> Result<(), SolilangError> {
    // Lexing
    let tokens = lexer::Scanner::new(source).scan_tokens()?;

    // Parsing
    let mut program = parser::Parser::new(tokens).parse()?;

    // Module resolution (if we have imports and a source path)
    if let Some(path) = source_path.filter(|_| has_imports(&program)) {
        let base_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let mut resolver = module::ModuleResolver::new(base_dir);
        program = resolver
            .resolve(program, path)
            .map_err(|e| error::RuntimeError::General {
                message: format!("Module resolution error: {}", e),
                span: span::Span::new(0, 0, 1, 1),
            })?;
    }

    // Type checking (optional)
    if type_check {
        let mut checker = types::TypeChecker::new();
        if let Err(errors) = checker.check(&program) {
            return Err(errors.into_iter().next().unwrap().into());
        }
    }

    // Execute with tree-walking interpreter
    let mut interpreter = interpreter::Interpreter::new();
    if let (Some(tracker), Some(path)) = (coverage_tracker, source_file_path) {
        interpreter.set_coverage_tracker(tracker.clone());
        interpreter.set_source_path(path.to_path_buf());
    }
    interpreter.interpret(&program)?;

    Ok(())
}

/// Check if a program has any import statements.
fn has_imports(program: &ast::Program) -> bool {
    program
        .statements
        .iter()
        .any(|stmt| matches!(stmt.kind, ast::StmtKind::Import(_)))
}

/// Parse source code into an AST without executing.
pub fn parse(source: &str) -> Result<ast::Program, SolilangError> {
    let tokens = lexer::Scanner::new(source).scan_tokens()?;
    let program = parser::Parser::new(tokens).parse()?;
    Ok(program)
}

/// Type check a program without executing.
pub fn type_check(source: &str) -> Result<(), Vec<error::TypeError>> {
    let tokens = lexer::Scanner::new(source)
        .scan_tokens()
        .map_err(|_| Vec::new())?;
    let program = parser::Parser::new(tokens)
        .parse()
        .map_err(|_| Vec::new())?;

    let mut checker = types::TypeChecker::new();
    checker.check(&program)
}
