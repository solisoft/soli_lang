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

use ast::expr::Argument;
use error::SolilangError;
use interpreter::Value;

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
) -> Result<i64, SolilangError> {
    // Clear any previous test suites
    interpreter::builtins::test_dsl::clear_test_suites();

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

    // Extract test definitions from AST
    let test_suites = extract_test_definitions(&program);

    // Execute with tree-walking interpreter
    let mut interpreter = interpreter::Interpreter::new();
    if let (Some(tracker), Some(path)) = (coverage_tracker, source_file_path) {
        interpreter.set_coverage_tracker(tracker.clone());
        interpreter.set_source_path(path.to_path_buf());
    }
    interpreter.interpret(&program)?;

    // Execute collected tests
    let (failed_count, failed_tests) = execute_test_suites(&mut interpreter, &test_suites)?;

    // Get assertion count from thread-local storage
    let assertion_count = interpreter::builtins::assertions::get_and_reset_assertion_count();

    // Return error if any tests failed
    if failed_count > 0 {
        let error_msg = if failed_tests.len() == 1 {
            format!("Test failed: {}", failed_tests[0])
        } else {
            format!("{} tests failed:\n  - {}", failed_count, failed_tests.join("\n  - "))
        };
        return Err(SolilangError::Runtime(error::RuntimeError::General {
            message: error_msg,
            span: span::Span::new(0, 0, 1, 1),
        }));
    }

    Ok(assertion_count)
}

fn extract_test_definitions(program: &ast::Program) -> Vec<interpreter::builtins::test_dsl::TestSuite> {
    let mut suites = Vec::new();
    for stmt in &program.statements {
        if let ast::StmtKind::Expression(expr) = &stmt.kind {
            if let ast::ExprKind::Call { callee, arguments } = &expr.kind {
                // Check if this is a describe call
                if let ast::ExprKind::Variable(name) = &callee.kind {
                    if name == "describe" || name == "context" {
                        if let Some(suite) = extract_suite_from_call(name, arguments, stmt.span) {
                            suites.push(suite);
                        }
                    }
                }
            }
        }
    }
    suites
}

fn extract_suite_from_call(
    _name: &str,
    arguments: &[Argument],
    _span: span::Span,
) -> Option<interpreter::builtins::test_dsl::TestSuite> {
    if arguments.len() < 2 {
        return None;
    }

    // First argument should be the suite name
    let first_arg = match &arguments[0] {
        Argument::Positional(expr) => expr,
        Argument::Named(_) => return None,
    };
    let suite_name = match &first_arg.kind {
        ast::ExprKind::StringLiteral(s) => s.clone(),
        _ => return None,
    };

    // Second argument should be a lambda (the suite body)
    let second_arg = match &arguments[1] {
        Argument::Positional(expr) => expr,
        Argument::Named(_) => return None,
    };
    let suite_body = match &second_arg.kind {
        ast::ExprKind::Lambda { body, .. } => body.clone(),
        _ => return None,
    };

    let mut suite = interpreter::builtins::test_dsl::TestSuite {
        name: suite_name,
        tests: Vec::new(),
        before_each: None,
        after_each: None,
        before_all: None,
        after_all: None,
        nested_suites: Vec::new(),
    };

    // Extract tests and nested suites from the lambda body
    extract_tests_from_block(&suite_body, &mut suite);

    Some(suite)
}

fn extract_tests_from_block(
    statements: &[ast::Stmt],
    suite: &mut interpreter::builtins::test_dsl::TestSuite,
) {
    for stmt in statements {
        if let ast::StmtKind::Expression(expr) = &stmt.kind {
            if let ast::ExprKind::Call { callee, arguments } = &expr.kind {
                if let ast::ExprKind::Variable(name) = &callee.kind {
                        if name == "test" || name == "it" || name == "specify" {
                            if let Some(test) = extract_test_from_call(arguments, stmt.span) {
                                suite.tests.push(test);
                            }
                        } else if name == "describe" || name == "context" {
                            if let Some(nested) = extract_suite_from_call(name, arguments, stmt.span) {
                                suite.nested_suites.push(nested);
                            }
                        } else if name == "before_each" {
                            if let Some(Argument::Positional(callback)) = arguments.first() {
                                suite.before_each = Some(ast_expr_to_value(callback));
                            }
                        } else if name == "after_each" {
                            if let Some(Argument::Positional(callback)) = arguments.first() {
                                suite.after_each = Some(ast_expr_to_value(callback));
                            }
                        } else if name == "before_all" {
                            if let Some(Argument::Positional(callback)) = arguments.first() {
                                suite.before_all = Some(ast_expr_to_value(callback));
                            }
                        } else if name == "after_all" {
                            if let Some(Argument::Positional(callback)) = arguments.first() {
                                suite.after_all = Some(ast_expr_to_value(callback));
                            }
                        }
                }
            }
        }
    }
}

fn extract_test_from_call(
    arguments: &[Argument],
    span: span::Span,
) -> Option<interpreter::builtins::test_dsl::TestDefinition> {
    if arguments.len() < 2 {
        return None;
    }

    let first_arg = match &arguments[0] {
        Argument::Positional(expr) => expr,
        Argument::Named(_) => return None,
    };
    let test_name = match &first_arg.kind {
        ast::ExprKind::StringLiteral(s) => s.clone(),
        _ => return None,
    };

    let second_arg = match &arguments[1] {
        Argument::Positional(expr) => expr,
        Argument::Named(_) => return None,
    };
    let test_body = match &second_arg.kind {
        ast::ExprKind::Lambda { params, return_type, body } => {
            create_function_value(params.clone(), return_type.clone(), body.clone(), span)
        }
        _ => return None,
    };

    Some(interpreter::builtins::test_dsl::TestDefinition {
        name: test_name,
        body: test_body,
    })
}

fn create_function_value(
    params: Vec<ast::stmt::Parameter>,
    return_type: Option<ast::types::TypeAnnotation>,
    body: Vec<ast::Stmt>,
    span: span::Span,
) -> Value {
    use interpreter::value::Function;
    use std::rc::Rc;
    use std::cell::RefCell;

    // Create an environment with builtins registered
    let mut env = interpreter::environment::Environment::new();
    interpreter::builtins::register_builtins(&mut env);

    let decl = ast::FunctionDecl {
        name: "test_fn".to_string(),
        params,
        return_type,
        body,
        span,
    };
    let closure = Rc::new(RefCell::new(env));
    Value::Function(Rc::new(Function::from_decl(&decl, closure, None)))
}

fn ast_expr_to_value(expr: &ast::Expr) -> Value {
    match &expr.kind {
        ast::ExprKind::Lambda { params, return_type, body } => {
            create_function_value(params.clone(), return_type.clone(), body.clone(), expr.span)
        }
        _ => Value::Null,
    }
}

fn execute_test_suites(
    interpreter: &mut interpreter::Interpreter,
    suites: &[interpreter::builtins::test_dsl::TestSuite],
) -> Result<(i64, Vec<String>), error::RuntimeError> {
    let mut failed_count = 0i64;
    let mut failed_tests = Vec::new();

    for suite in suites {
        // Run before_all if defined
        if let Some(before_all) = &suite.before_all {
            let _ = interpreter.call_value(before_all.clone(), Vec::new(), span::Span::new(0, 0, 1, 1));
        }

        for test in &suite.tests {
            // Run before_each if defined
            if let Some(before_each) = &suite.before_each {
                let _ = interpreter.call_value(before_each.clone(), Vec::new(), span::Span::new(0, 0, 1, 1));
            }

            // Execute the test body and track failures
            let result = interpreter.call_value(
                test.body.clone(),
                Vec::new(),
                span::Span::new(0, 0, 1, 1),
            );

            if let Err(e) = result {
                failed_count += 1;
                failed_tests.push(format!("{}: {}", test.name, e));
            }

            // Run after_each if defined
            if let Some(after_each) = &suite.after_each {
                let _ = interpreter.call_value(after_each.clone(), Vec::new(), span::Span::new(0, 0, 1, 1));
            }
        }

        // Run nested suites
        let (nested_failed, mut nested_errors) = execute_test_suites(interpreter, &suite.nested_suites)?;
        failed_count += nested_failed;
        failed_tests.append(&mut nested_errors);

        // Run after_all if defined
        if let Some(after_all) = &suite.after_all {
            let _ = interpreter.call_value(after_all.clone(), Vec::new(), span::Span::new(0, 0, 1, 1));
        }
    }
    Ok((failed_count, failed_tests))
}

/// Check if a program has any import statements.
pub(crate) fn has_imports(program: &ast::Program) -> bool {
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
