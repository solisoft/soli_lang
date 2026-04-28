//! Solilang: A statically-typed, class-based OOP language with pipeline operators.
//!
//! This is the library root that exports all modules.
//!
//! # Execution
//!
//! Solilang uses a tree-walking interpreter for executing programs.

pub mod ast;
pub mod compiled_cache;
pub mod coverage;
pub mod error;
pub mod inflect;
pub mod interpreter;
pub mod lexer;
pub mod lint;
pub mod live;
pub mod migration;
pub mod module;
pub mod parser;
pub mod regex_cache;
pub mod repl_common;
pub mod repl_highlight;
pub mod repl_simple;
pub mod repl_tui;
pub mod scaffold;
pub mod serve;
pub mod solidb_http;
pub mod span;
pub mod template;
pub mod types;
pub mod vm;

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
pub fn run_with_options(source: &str, type_check: bool) -> Result<(), SolilangError> {
    run_with_path(source, None, type_check)
}

/// Run a Solilang program from a file path with module resolution.
pub fn run_file(path: &std::path::Path, type_check: bool) -> Result<(), SolilangError> {
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

/// Run a Solilang program through the bytecode VM (faster execution).
pub fn run_file_vm(path: &std::path::Path, type_check: bool) -> Result<(), SolilangError> {
    let source = std::fs::read_to_string(path).map_err(|e| error::RuntimeError::General {
        message: format!("Failed to read file '{}': {}", path.display(), e),
        span: span::Span::new(0, 0, 1, 1),
    })?;

    run_vm(&source, Some(path), type_check)
}

/// Run a Solilang program through the bytecode VM.
pub fn run_vm(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
) -> Result<(), SolilangError> {
    let module = compiled_cache::get_or_compile(source, source_path, type_check)?;

    let mut vm_instance = vm::Vm::new();

    // Register builtins
    use interpreter::value::{NativeFunction, Value as V};
    vm_instance.globals.insert(
        "print".to_string(),
        V::NativeFunction(NativeFunction::new("print", None, |args| {
            let output: Vec<String> = args.iter().map(|a| format!("{}", a)).collect();
            println!("{}", output.join(" "));
            Ok(V::Null)
        })),
    );
    vm_instance.globals.insert(
        "puts".to_string(),
        V::NativeFunction(NativeFunction::new("puts", None, |args| {
            let output: Vec<String> = args.iter().map(|a| format!("{}", a)).collect();
            println!("{}", output.join(" "));
            Ok(V::Null)
        })),
    );
    vm_instance.globals.insert(
        "len".to_string(),
        V::NativeFunction(NativeFunction::new("len", Some(1), |args| match &args[0] {
            V::String(s) => Ok(V::Int(s.len() as i64)),
            V::Array(arr) => Ok(V::Int(arr.borrow().len() as i64)),
            V::Hash(hash) => Ok(V::Int(hash.borrow().len() as i64)),
            _ => Ok(V::Int(0)),
        })),
    );
    vm_instance.globals.insert(
        "str".to_string(),
        V::NativeFunction(NativeFunction::new("str", Some(1), |args| {
            Ok(V::String(format!("{}", args[0])))
        })),
    );
    vm_instance.globals.insert(
        "type_of".to_string(),
        V::NativeFunction(NativeFunction::new("type_of", Some(1), |args| {
            Ok(V::String(args[0].type_name().to_string()))
        })),
    );
    vm_instance.globals.insert(
        "clock".to_string(),
        V::NativeFunction(NativeFunction::new("clock", Some(0), |_args| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            Ok(V::Float(now.as_secs_f64()))
        })),
    );

    // Execute the compiled module
    vm_instance.execute(&module.main)?;

    Ok(())
}

/// Run a Solilang program with optional coverage tracking.
///
/// `preamble_files` is a list of `(path, source)` pairs that are loaded into the
/// interpreter in order before running `source`. Each preamble executes with its
/// own `current_source_path` so coverage hits are attributed back to that file.
///
/// Returns `(assertion_count, result)`. The assertion count reflects assertions that
/// succeeded during this file's test run, even if some tests failed afterwards — so
/// the caller can report meaningful totals regardless of pass/fail status.
pub fn run_with_path_and_coverage(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
    coverage_tracker: Option<&std::sync::Arc<std::sync::Mutex<coverage::CoverageTracker>>>,
    source_file_path: Option<&std::path::Path>,
    preamble_files: &[(std::path::PathBuf, String)],
) -> (i64, Result<(), SolilangError>) {
    interpreter::builtins::test_dsl::clear_test_suites();
    let _ = interpreter::builtins::assertions::get_and_reset_assertion_count();

    let result = run_with_path_and_coverage_inner(
        source,
        source_path,
        type_check,
        coverage_tracker,
        source_file_path,
        preamble_files,
    );

    let assertion_count = interpreter::builtins::assertions::get_and_reset_assertion_count();
    (assertion_count, result)
}

fn run_with_path_and_coverage_inner(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
    coverage_tracker: Option<&std::sync::Arc<std::sync::Mutex<coverage::CoverageTracker>>>,
    source_file_path: Option<&std::path::Path>,
    preamble_files: &[(std::path::PathBuf, String)],
) -> Result<(), SolilangError> {
    let mut interpreter = interpreter::Interpreter::new();
    if let Some(tracker) = coverage_tracker {
        interpreter.set_coverage_tracker(tracker.clone());
    }

    for (preamble_path, preamble_source) in preamble_files {
        let tokens = lexer::Scanner::new(preamble_source).scan_tokens()?;
        let mut program = parser::Parser::new(tokens).parse()?;

        if has_imports(&program) {
            let base_dir = preamble_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let mut resolver = module::ModuleResolver::new(base_dir);
            program = resolver.resolve(program, preamble_path).map_err(|e| {
                error::RuntimeError::General {
                    message: format!("Module resolution error: {}", e),
                    span: span::Span::new(0, 0, 1, 1),
                }
            })?;
        }

        if type_check {
            let mut checker = types::TypeChecker::new();
            if let Err(errors) = checker.check(&program) {
                return Err(errors.into_iter().next().unwrap().into());
            }
        }

        interpreter.set_source_path(preamble_path.clone());
        interpreter.interpret(&program)?;
    }

    let tokens = lexer::Scanner::new(source).scan_tokens()?;
    let mut program = parser::Parser::new(tokens).parse()?;

    let has_imports = source_path.is_some() && has_imports(&program);
    if let Some(path) = source_path.filter(|_| has_imports) {
        let base_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let mut resolver = module::ModuleResolver::new(base_dir);
        program = resolver
            .resolve(program, path)
            .map_err(|e| error::RuntimeError::General {
                message: format!("Module resolution error: {}", e),
                span: span::Span::new(0, 0, 1, 1),
            })?;
    }

    if type_check {
        let mut checker = types::TypeChecker::new();
        if let Err(errors) = checker.check(&program) {
            return Err(errors.into_iter().next().unwrap().into());
        }
    }

    let test_suites = extract_test_definitions(&program);

    if let Some(path) = source_file_path {
        interpreter.set_source_path(path.to_path_buf());
    }
    interpreter.interpret(&program)?;

    let (failed_count, failed_tests) = execute_test_suites(&mut interpreter, &test_suites)?;

    if failed_count > 0 {
        let error_msg = if failed_tests.len() == 1 {
            format!("Test failed: {}", failed_tests[0])
        } else {
            format!(
                "{} tests failed:\n  - {}",
                failed_count,
                failed_tests.join("\n  - ")
            )
        };
        return Err(SolilangError::Runtime(error::RuntimeError::General {
            message: error_msg,
            span: span::Span::new(0, 0, 1, 1),
        }));
    }

    Ok(())
}

fn extract_test_definitions(
    program: &ast::Program,
) -> Vec<interpreter::builtins::test_dsl::TestSuite> {
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
        Argument::Block(_) => return None,
    };
    let suite_name = match &first_arg.kind {
        ast::ExprKind::StringLiteral(s) => s.clone(),
        _ => return None,
    };

    // Second argument should be a lambda (the suite body)
    let second_arg = match &arguments[1] {
        Argument::Positional(expr) => expr,
        Argument::Named(_) => return None,
        Argument::Block(_) => return None,
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
        Argument::Block(_) => return None,
    };
    let test_name = match &first_arg.kind {
        ast::ExprKind::StringLiteral(s) => s.clone(),
        _ => return None,
    };

    // Try to get second argument as either Positional(lambda) or Block
    let test_body = match &arguments[1] {
        Argument::Positional(expr) => match &expr.kind {
            ast::ExprKind::Lambda {
                params,
                return_type,
                body,
            } => create_function_value(params.clone(), return_type.clone(), body.clone(), span),
            _ => return None,
        },
        Argument::Block(block_expr) => {
            // Convert block expression to lambda function
            match &block_expr.kind {
                ast::ExprKind::Lambda {
                    params,
                    return_type,
                    body,
                } => create_function_value(params.clone(), return_type.clone(), body.clone(), span),
                _ => return None,
            }
        }
        Argument::Named(_) => return None,
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
    use std::cell::RefCell;
    use std::rc::Rc;

    // Create an environment with builtins registered
    let mut env = interpreter::environment::Environment::new();
    interpreter::builtins::register_builtins(&mut env, true);

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
        ast::ExprKind::Lambda {
            params,
            return_type,
            body,
        } => create_function_value(params.clone(), return_type.clone(), body.clone(), expr.span),
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
            let rebound = rebind_closure(before_all, &interpreter.environment);
            let _ = interpreter.call_value(rebound, Vec::new(), span::Span::new(0, 0, 1, 1));
        }

        for test in &suite.tests {
            // Run before_each if defined
            if let Some(before_each) = &suite.before_each {
                let rebound = rebind_closure(before_each, &interpreter.environment);
                let _ = interpreter.call_value(rebound, Vec::new(), span::Span::new(0, 0, 1, 1));
            }

            // Rebind test body closure to interpreter's environment so
            // top-level `def` functions (e.g. register_test_user) are accessible.
            let test_body = rebind_closure(&test.body, &interpreter.environment);

            // Execute the test body and track failures
            let result = interpreter.call_value(test_body, Vec::new(), span::Span::new(0, 0, 1, 1));

            if let Err(e) = result {
                failed_count += 1;
                failed_tests.push(format!("{}: {}", test.name, e));
            }

            // Run after_each if defined
            if let Some(after_each) = &suite.after_each {
                let rebound = rebind_closure(after_each, &interpreter.environment);
                let _ = interpreter.call_value(rebound, Vec::new(), span::Span::new(0, 0, 1, 1));
            }
        }

        // Run nested suites
        let (nested_failed, mut nested_errors) =
            execute_test_suites(interpreter, &suite.nested_suites)?;
        failed_count += nested_failed;
        failed_tests.append(&mut nested_errors);

        // Run after_all if defined
        if let Some(after_all) = &suite.after_all {
            let rebound = rebind_closure(after_all, &interpreter.environment);
            let _ = interpreter.call_value(rebound, Vec::new(), span::Span::new(0, 0, 1, 1));
        }
    }
    Ok((failed_count, failed_tests))
}

/// Rebind a test function's closure to use the interpreter's environment,
/// so that top-level definitions (def, let) are accessible inside tests.
fn rebind_closure(
    value: &interpreter::value::Value,
    env: &std::rc::Rc<std::cell::RefCell<interpreter::environment::Environment>>,
) -> interpreter::value::Value {
    use interpreter::value::{Function, Value};
    match value {
        Value::Function(func) => {
            let mut new_func = Function {
                name: func.name.clone(),
                params: func.params.clone(),
                body: func.body.clone(),
                closure: env.clone(),
                is_method: func.is_method,
                span: func.span,
                source_path: func.source_path.clone(),
                defining_superclass: func.defining_superclass.clone(),
                return_type: func.return_type.clone(),
                cached_env: std::cell::RefCell::new(None),
                jit_cache: std::cell::RefCell::new(None),
            };
            new_func.closure = env.clone();
            Value::Function(std::rc::Rc::new(new_func))
        }
        other => other.clone(),
    }
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

/// Lint source code and return diagnostics.
pub fn lint(source: &str) -> Result<Vec<lint::LintDiagnostic>, SolilangError> {
    let tokens = lexer::Scanner::new(source).scan_tokens()?;
    let program = parser::Parser::new(tokens).parse()?;
    Ok(lint::Linter::new(source).lint(&program))
}

/// Lint source with the file path available to path-sensitive rules.
pub fn lint_file(source: &str, path: &str) -> Result<Vec<lint::LintDiagnostic>, SolilangError> {
    let tokens = lexer::Scanner::new(source).scan_tokens()?;
    let program = parser::Parser::new(tokens).parse()?;
    Ok(lint::Linter::new(source)
        .with_file_path(path)
        .lint(&program))
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
