//! Solilang: A statically-typed, class-based OOP language with pipeline operators.
//!
//! This is the library root that exports all modules.
//!
//! # Execution
//!
//! Solilang uses a tree-walking interpreter for executing programs.

pub mod ast;
pub mod bundle;
pub mod cdp;
pub mod cleanup;
pub mod compiled_cache;
pub mod coverage;
pub mod db;
pub mod desktop;
pub mod embedding;
pub mod error;
pub mod fmt;
pub mod generation;
pub mod graph;
pub mod inflect;
pub mod interpreter;
pub mod jobs;
pub mod lexer;
pub mod lint;
pub mod live;
#[cfg(feature = "lsp")]
pub mod lsp;
pub mod metrics;
pub mod migration;
pub mod mobile;
pub mod module;
pub mod native;
pub mod parser;
pub mod platform;
pub(crate) mod redaction;
pub mod regex_cache;
pub mod repl_common;
pub mod repl_highlight;
pub mod repl_simple;
pub mod repl_tui;
pub mod scaffold;
pub mod serve;
#[cfg(feature = "solidb-driver")]
pub mod solidb_driver;
pub mod solidb_http;
pub mod span;
pub mod template;
pub mod types;
pub mod update;
pub mod virtual_fs;
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

/// Execute `preamble_files` in order against an existing interpreter, so the
/// names they define (model and service classes) are in scope for the program
/// that follows. Shared by the test runner, `db:seed`, and both migration
/// runners, which differ only in which interpreter they hand over.
fn run_preamble_files(
    interpreter: &mut interpreter::Interpreter,
    preamble_files: &[(std::path::PathBuf, String)],
    type_check: bool,
) -> Result<(), SolilangError> {
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
        inject_job_facade(interpreter, preamble_path);
    }
    Ok(())
}

/// Give an `app/jobs/*_job.sl` class the facade the server injects when it
/// loads the app (`perform_later`, `perform_in`, `perform_at`, `perform_now`,
/// `schedule_cron`). Without it a spec — or a model callback or service a spec
/// drives — failed on `perform_later` while the same call worked in the app.
fn inject_job_facade(interpreter: &mut interpreter::Interpreter, path: &std::path::Path) {
    let in_jobs_dir = path
        .parent()
        .and_then(|dir| dir.file_name())
        .is_some_and(|dir| dir == "jobs");
    let is_job_file = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("_job.sl"));
    if !in_jobs_dir || !is_job_file {
        return;
    }
    let class_name = serve::app_loader::job_class_name_from_path(path);
    let Some(interpreter::value::Value::Class(class)) =
        interpreter.environment.borrow().get(&class_name)
    else {
        return;
    };
    let class_value = interpreter::value::Value::Class(std::rc::Rc::new(
        interpreter::builtins::jobs::inject_facade_methods(&class),
    ));
    interpreter
        .environment
        .borrow_mut()
        .define(class_name.clone(), class_value.clone());
    interpreter::builtins::jobs::register_job_class_in_registry(&class_name, class_value);
}

/// Run a migration file. Same pipeline as [`run_with_options`], but the
/// interpreter also has the SQL schema helpers (`db.execute`, `add_column`, …).
///
/// `preamble_files` are `app/models` / `app/services` (see
/// [`crate::migration::collect_model_preamble_files`]) so a data migration can
/// use the Model API without an `import`, exactly as on the SoliDB path.
pub(crate) fn run_migration_source(
    source: &str,
    preamble_files: &[(std::path::PathBuf, String)],
) -> Result<(), SolilangError> {
    let tokens = lexer::Scanner::new(source).scan_tokens()?;
    let program = parser::Parser::new(tokens).parse()?;
    let mut interpreter = interpreter::Interpreter::new_for_migrations();
    interpreter::builtins::mailer::ensure_prelude(&mut interpreter);
    // Models load before the migration body, and without type-checking: a model
    // class is application code the migration merely borrows, and `soli check`
    // is where it gets checked.
    run_preamble_files(&mut interpreter, preamble_files, false)?;
    interpreter.interpret(&program)?;
    Ok(())
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
    let program = parse_resolve_check(source, source_path, type_check)?;
    execute_program_tree(&program)
}

/// Lex, parse, resolve imports (when there is a path to resolve them from)
/// and, if asked, type-check: everything before a program runs.
pub fn parse_resolve_check(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
) -> Result<ast::Program, SolilangError> {
    let tokens = lexer::Scanner::new(source).scan_tokens()?;
    let mut program = parser::Parser::new(tokens).parse()?;

    if let Some(path) = source_path.filter(|_| has_imports(&program)) {
        let base_dir = import_base_dir(path);
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
    Ok(program)
}

/// A fresh interpreter with every builtin registered, as a script sees it.
fn script_interpreter() -> interpreter::Interpreter {
    let mut interpreter = interpreter::Interpreter::new();
    interpreter::builtins::mailer::ensure_prelude(&mut interpreter);
    interpreter
}

/// Compile the native kernels of `program` (see [`native`]), given the
/// interpreter whose bindings say which names are builtins.
fn script_kernels(
    program: &ast::Program,
    interpreter: &interpreter::Interpreter,
) -> Option<native::KernelSet> {
    if !native::enabled() {
        return None;
    }
    let env = interpreter.environment.clone();
    let is_builtin = |name: &str| env.borrow().get(name).is_some();
    native::analyze_and_compile(program, &is_builtin)
}

/// Run a parsed, resolved and checked program on the tree-walking
/// interpreter, its typed numeric functions compiled to native kernels.
pub fn execute_program_tree(program: &ast::Program) -> Result<(), SolilangError> {
    #[cfg(feature = "eui")]
    serve::eui::script::remember_script(program);
    let mut interpreter = script_interpreter();
    let kernels = script_kernels(program, &interpreter);
    interpreter.set_kernels(kernels);
    interpreter.interpret(program)?;
    Ok(())
}

/// Which engine runs a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// The bytecode VM, unless the script needs the tree-walker: it uses a
    /// construct the VM hands back at run time ([`tree_walker_reason`]), or it
    /// does not compile to bytecode. The default for `soli script.sl`.
    Auto,
    /// The tree-walking interpreter (`--tree`).
    Tree,
    /// The VM, and an error where it cannot run the script (`--vm`).
    Vm,
}

impl Engine {
    /// `SOLI_ENGINE=tree|vm|auto`, when set and valid.
    pub fn from_env() -> Option<Engine> {
        match std::env::var("SOLI_ENGINE")
            .ok()?
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "tree" | "interpreter" => Some(Engine::Tree),
            "vm" => Some(Engine::Vm),
            "auto" => Some(Engine::Auto),
            _ => None,
        }
    }
}

/// Run the script at `path` on `engine`.
pub fn run_script_file(
    path: &std::path::Path,
    type_check: bool,
    engine: Engine,
) -> Result<(), SolilangError> {
    let source = std::fs::read_to_string(path).map_err(|e| error::RuntimeError::General {
        message: format!("Failed to read file '{}': {}", path.display(), e),
        span: span::Span::new(0, 0, 1, 1),
    })?;
    run_script(&source, Some(path), type_check, engine)
}

/// Run `source` as a script on `engine`.
pub fn run_script(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
    engine: Engine,
) -> Result<(), SolilangError> {
    let program = parse_resolve_check(source, source_path, type_check)?;
    execute_program(&program, engine)
}

/// Run a parsed, resolved and checked program on `engine`.
pub fn execute_program(program: &ast::Program, engine: Engine) -> Result<(), SolilangError> {
    #[cfg(feature = "eui")]
    serve::eui::script::remember_script(program);
    match engine {
        Engine::Tree => execute_program_tree(program),
        Engine::Vm => execute_program_vm(program),
        Engine::Auto => {
            if let Some(reason) = tree_walker_reason(program) {
                log_engine(&format!("tree-walker — the script uses {reason}"));
                return execute_program_tree(program);
            }
            let interpreter = script_interpreter();
            let env = interpreter.environment.clone();
            let zero_arg_builtin = |name: &str| match env.borrow().get(name) {
                Some(Value::NativeFunction(native)) => {
                    native.is_auto_invocable || native.arity == Some(0)
                }
                _ => false,
            };
            if let Some(name) = bare_call_reason(program, &zero_arg_builtin) {
                log_engine(&format!(
                    "tree-walker — the script calls `{name}` without parentheses"
                ));
                return execute_program_tree(program);
            }
            let kernels = script_kernels(program, &interpreter);
            match vm::Compiler::compile_with_kernels(program, kernels) {
                Ok(module) => {
                    log_engine("vm");
                    run_compiled_on_vm(&module, &interpreter)
                }
                // Nothing has run yet, so the tree-walker can take it whole.
                Err(e) => {
                    log_engine(&format!("tree-walker — the VM cannot compile it ({e})"));
                    execute_program_tree(program)
                }
            }
        }
    }
}

/// `SOLI_ENGINE_LOG=1`: say which engine runs the script, and why.
fn log_engine(choice: &str) {
    if std::env::var("SOLI_ENGINE_LOG").is_ok_and(|v| !v.is_empty() && v != "0") {
        eprintln!("engine: {choice}");
    }
}

/// Why `program` must run on the tree-walker, if it must: it uses a construct
/// the VM hands back to the interpreter at run time (`EngineFallback`).
///
/// Only `soli serve` can take a request back and replay it on the
/// interpreter; a script that hit one halfway would stop with an error after
/// doing half its work. So it is decided before anything runs, from the
/// source. The list mirrors the VM's refusals: class reflection
/// (`vm_classes::is_class_reflection_member`), batch iteration and dynamic
/// finders on models, `method_missing`, model callbacks given as closures and
/// state machines.
pub fn tree_walker_reason(program: &ast::Program) -> Option<String> {
    use ast::expr::ExprKind;
    use ast::stmt::StmtKind;

    let mut reason: Option<String> = None;
    let mut on_stmt = |stmt: &ast::Stmt| {
        if reason.is_some() {
            return;
        }
        if let StmtKind::Class(class) = &stmt.kind {
            if class.methods.iter().any(|m| m.name == "method_missing") {
                reason = Some("`method_missing`".to_string());
            }
        }
    };
    let mut found: Option<String> = None;
    let mut on_expr = |expr: &ast::Expr| {
        if found.is_some() {
            return;
        }
        let (name, receiver) = match &expr.kind {
            ExprKind::Member { object, name } | ExprKind::SafeMember { object, name } => {
                (name.as_str(), Some(object))
            }
            ExprKind::Variable(name) => (name.as_str(), None),
            _ => return,
        };
        let on_a_class = receiver.is_some_and(|object| {
            matches!(&object.kind, ExprKind::Variable(n) if n.starts_with(char::is_uppercase))
        });
        let hit = match name {
            "class_eval" | "instance_eval" | "send" | "methods" | "find_each" | "in_batches"
            | "find_in_batches" | "state_machine" | "after_transition" | "before_transition" => {
                true
            }
            "respond_to?" | "inspect" | "to_s" | "to_string" => on_a_class,
            "before_save" | "after_save" | "before_create" | "after_create" | "before_update"
            | "after_update" | "before_delete" | "after_delete" | "before_validation"
            | "after_validation" => true,
            _ => name.starts_with("find_by_") && on_a_class,
        };
        if hit {
            found = Some(format!("`{name}`"));
        }
    };
    ast::walk::walk_program(program, &mut on_stmt, &mut on_expr);
    reason.or(found)
}

/// A bare name the tree-walker would call and the VM would not: a function
/// with no required parameter, read without `()` (`x = helper`,
/// `print(session_id)`).
///
/// The tree-walker calls it; the VM — and so `soli serve` in production —
/// hands over the function itself. Scripts were written against the
/// tree-walker, so one that relies on this keeps running there. Names the
/// script binds as a variable or a parameter are taken to be those, which
/// misses a name used both ways; the cost of a wrong guess the other way is
/// only speed.
pub fn bare_call_reason(
    program: &ast::Program,
    zero_arg_builtin: &dyn Fn(&str) -> bool,
) -> Option<String> {
    use ast::expr::ExprKind;
    use ast::stmt::StmtKind;
    use std::collections::HashSet;

    let no_required = |params: &[ast::Parameter]| params.iter().all(|p| p.default_value.is_some());

    // Functions the tree-walker would call bare, and names that are plainly
    // variables.
    let callable: std::cell::RefCell<HashSet<String>> = Default::default();
    let variables: std::cell::RefCell<HashSet<String>> = Default::default();
    let mut on_stmt = |stmt: &ast::Stmt| match &stmt.kind {
        StmtKind::Function(decl) => {
            if no_required(&decl.params) {
                callable.borrow_mut().insert(decl.name.clone());
            }
            for p in &decl.params {
                variables.borrow_mut().insert(p.name.clone());
            }
        }
        StmtKind::Let {
            name, initializer, ..
        } => match initializer.as_ref().map(|e| &e.kind) {
            Some(ExprKind::Lambda { params, .. }) if no_required(params) => {
                callable.borrow_mut().insert(name.clone());
            }
            _ => {
                variables.borrow_mut().insert(name.clone());
            }
        },
        StmtKind::For {
            variable,
            index_variable,
            ..
        } => {
            variables.borrow_mut().insert(variable.clone());
            if let Some(index) = index_variable {
                variables.borrow_mut().insert(index.clone());
            }
        }
        StmtKind::Try { catch_clauses, .. } => {
            for clause in catch_clauses {
                if let Some(var) = &clause.var_name {
                    variables.borrow_mut().insert(var.clone());
                }
            }
        }
        _ => {}
    };
    // Where a name is the callee of a call or the target of an assignment,
    // it is not read bare.
    let mut not_bare: HashSet<*const ast::Expr> = HashSet::new();
    let mut bare: Vec<(*const ast::Expr, String)> = Vec::new();
    let mut on_expr = |expr: &ast::Expr| match &expr.kind {
        ExprKind::Call { callee, .. } => {
            not_bare.insert(&**callee as *const ast::Expr);
        }
        ExprKind::Assign { target, value } => {
            not_bare.insert(&**target as *const ast::Expr);
            if let ExprKind::Variable(name) = &target.kind {
                match &value.kind {
                    ExprKind::Lambda { params, .. } if no_required(params) => {
                        callable.borrow_mut().insert(name.clone());
                    }
                    _ => {
                        variables.borrow_mut().insert(name.clone());
                    }
                }
            }
        }
        ExprKind::CompoundAssign { target, .. } => {
            not_bare.insert(&**target as *const ast::Expr);
        }
        ExprKind::Lambda { params, .. } => {
            for p in params {
                variables.borrow_mut().insert(p.name.clone());
            }
        }
        ExprKind::Variable(name) => bare.push((expr as *const ast::Expr, name.clone())),
        _ => {}
    };
    ast::walk::walk_program(program, &mut on_stmt, &mut on_expr);

    bare.into_iter()
        .filter(|(ptr, _)| !not_bare.contains(ptr))
        .map(|(_, name)| name)
        .find(|name| {
            callable.borrow().contains(name)
                || (!variables.borrow().contains(name) && zero_arg_builtin(name))
        })
}

/// Run a parsed, resolved and checked program on the bytecode VM, its typed
/// numeric functions compiled to native kernels.
pub fn execute_program_vm(program: &ast::Program) -> Result<(), SolilangError> {
    #[cfg(feature = "eui")]
    serve::eui::script::remember_script(program);
    let interpreter = script_interpreter();
    let kernels = script_kernels(program, &interpreter);
    let module = vm::Compiler::compile_with_kernels(program, kernels).map_err(|e| {
        error::RuntimeError::General {
            message: format!("Compile error: {}", e),
            span: span::Span::new(0, 0, 1, 1),
        }
    })?;
    run_compiled_on_vm(&module, &interpreter)
}

/// Seed a VM with the full builtin environment, exactly like a production
/// serve worker: the interpreter registered every builtin function and
/// native class (DateTime, Duration, HTTP, …), and its globals are copied
/// across. Keeps `--vm` a faithful simulator of production-mode execution
/// instead of a hand-rolled subset.
fn run_compiled_on_vm(
    module: &vm::chunk::CompiledModule,
    interpreter: &interpreter::Interpreter,
) -> Result<(), SolilangError> {
    let mut vm_instance = vm::Vm::new();
    vm_instance.check_return_types = true;
    let all_globals = interpreter.environment.borrow().get_all_bindings();
    for (name, value) in all_globals {
        vm_instance.globals.insert(name, value);
    }
    vm_instance.execute(&module.main)?;
    Ok(())
}

/// Type-check a program without executing it. Resolves imports (when a path is
/// given) and returns every type error, or any lex/parse/module-resolution
/// failure as a single-element vec. Powers `soli check`.
///
/// On success returns any non-blocking warning messages (e.g. enum match
/// non-exhaustiveness); these never fail the check.
pub fn type_check_source(
    source: &str,
    source_path: Option<&std::path::Path>,
) -> Result<Vec<String>, Vec<SolilangError>> {
    type_check_source_with_ambient(source, source_path, &[])
}

/// The directories a running server loads into **one** environment, so that a
/// declaration in any of them is reachable from all of them with no import.
///
/// `soli check` reads them to build its ambient name set; see
/// [`type_check_source_with_ambient`].
pub const AUTOLOADED_DIRS: &[&str] = &[
    "app/controllers",
    "app/models",
    "app/services",
    "app/policies",
    "app/middleware",
    "app/mailers",
    "config",
    "stdlib",
];

/// Type-check one file, given the names its *neighbours* declare.
///
/// `serve` loads every auto-loaded directory into one environment, so a helper
/// declared in `app/controllers/a.sl` is callable from `app/controllers/b.sl`
/// with no import. The checker sees one file at a time, so without `ambient`
/// every such call reads as `Undefined variable` — a freshly scaffolded `--eui`
/// application reported 137 of them, all naming five functions declared in a
/// sibling file.
///
/// A project declaration **overrides a builtin of the same name**, because that
/// is what happens at run time: the scaffolded EUI catalogue defines its own
/// three-argument `input`, and the one-argument builtin's signature was
/// rejecting all nine of its call sites. A name the checked file declares
/// itself still wins over both.
pub fn type_check_source_with_ambient(
    source: &str,
    source_path: Option<&std::path::Path>,
    ambient: &[String],
) -> Result<Vec<String>, Vec<SolilangError>> {
    let tokens = lexer::Scanner::new(source)
        .scan_tokens()
        .map_err(|e| vec![e.into()])?;
    let mut program = parser::Parser::new(tokens)
        .parse()
        .map_err(|e| vec![e.into()])?;

    if let Some(path) = source_path.filter(|_| has_imports(&program)) {
        let base_dir = import_base_dir(path);
        let mut resolver = module::ModuleResolver::new(base_dir);
        program = resolver.resolve(program, path).map_err(|e| {
            vec![error::RuntimeError::General {
                message: format!("Module resolution error: {}", e),
                span: span::Span::new(0, 0, 1, 1),
            }
            .into()]
        })?;
    }

    let mut checker = types::TypeChecker::new();
    checker.declare_ambient(ambient);
    if source_path.is_some_and(is_test_path) {
        checker.declare_test_dsl();
    }
    if source_path.is_some_and(|p| is_application_path(p) || is_test_path(p)) {
        checker.declare_request_scope();
    }
    let (result, warnings) = checker.check_collecting_warnings(&program);
    match result {
        Ok(()) => Ok(warnings.into_iter().map(|w| w.to_string()).collect()),
        Err(errs) => Err(errs.into_iter().map(Into::into).collect()),
    }
}

/// Whether a path belongs to an application rather than to a loose script.
///
/// `app/`, `config/` and `stdlib/` are what a server loads to answer requests
/// — see [`AUTOLOADED_DIRS`], plus `app/jobs` and `app/helpers`, which are
/// loaded by their own machinery and are just as much request-scope code. Only
/// there does `req` exist and `render` mean anything.
fn is_application_path(path: &std::path::Path) -> bool {
    path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("app") | Some("config") | Some("stdlib")
        )
    })
}

/// Whether a path is one `soli test` would run, and so may call the test DSL.
///
/// Deliberately a superset of the runner's own rule (every `.sl` under the
/// tests directory) plus the two file-name conventions, because getting it
/// wrong in the generous direction costs an unreported `describe` in an odd
/// place, and in the strict direction costs an error on every spec in the
/// project.
fn is_test_path(path: &std::path::Path) -> bool {
    let in_test_dir = path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("tests") | Some("test") | Some("tests-e2e") | Some("spec")
        )
    });
    let named_like_a_spec = path
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| stem.ends_with("_spec") || stem.ends_with("_test"));
    in_test_dir || named_like_a_spec
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
    // Kernels are tied to the AST they were compiled from, which the module
    // cache does not keep; a script with kernels compiles afresh.
    if native::enabled() {
        let program = parse_resolve_check(source, source_path, type_check)?;
        return execute_program_vm(&program);
    }
    let module = compiled_cache::get_or_compile(source, source_path, type_check)?;
    run_compiled_on_vm(&module, &script_interpreter())
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
    interpreter::builtins::mailer::ensure_prelude(&mut interpreter);

    run_preamble_files(&mut interpreter, preamble_files, type_check)?;

    let tokens = lexer::Scanner::new(source).scan_tokens()?;
    let mut program = parser::Parser::new(tokens).parse()?;

    let has_imports = source_path.is_some() && has_imports(&program);
    if let Some(path) = source_path.filter(|_| has_imports) {
        let base_dir = import_base_dir(path);
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
                        if let Some(mut suite) = extract_suite_from_call(name, arguments, stmt.span)
                        {
                            push_viewport_down(&mut suite);
                            suites.push(suite);
                        }
                    }
                }
            }
        }
    }
    suites
}

/// Hand each nested suite the viewport it did not declare for itself.
///
/// Top-down, and after the whole tree is built: `viewport(...)` can appear
/// anywhere in a `describe` body, including below the nested `describe`s it
/// applies to, and a spec author reading the file will expect it to cover them
/// either way.
fn push_viewport_down(suite: &mut interpreter::builtins::test_dsl::TestSuite) {
    for nested in &mut suite.nested_suites {
        if nested.viewport.is_none() {
            nested.viewport = suite.viewport;
        }
        push_viewport_down(nested);
    }
}

/// Read a `viewport(...)` declaration out of a suite body.
///
/// Only literals: the body of a `describe` is never executed — it is walked —
/// so there is nothing to evaluate an expression against. `viewport(width())`
/// is silently no viewport, which is the same deal `describe` itself makes
/// about its name.
fn extract_viewport(arguments: &[Argument]) -> Option<interpreter::builtins::browser::Viewport> {
    use interpreter::builtins::browser;

    let positional: Vec<&ast::Expr> = arguments
        .iter()
        .filter_map(|arg| match arg {
            Argument::Positional(expr) => Some(expr),
            _ => None,
        })
        .collect();

    let mut viewport = match (
        positional.first().map(|e| &e.kind),
        positional.get(1).map(|e| &e.kind),
    ) {
        (Some(ast::ExprKind::StringLiteral(name)), _) => browser::viewport_from_name(name).ok()?,
        (Some(ast::ExprKind::IntLiteral(width)), Some(ast::ExprKind::IntLiteral(height))) => {
            browser::viewport_size(*width as f64, *height as f64).ok()?
        }
        _ => return None,
    };

    // A trailing `{"scale": 3, "mobile": true}`, in literal form.
    if let Some(ast::ExprKind::Hash(pairs)) = positional.get(2).map(|e| &e.kind) {
        for (key, value) in pairs {
            let ast::ExprKind::StringLiteral(key) = &key.kind else {
                continue;
            };
            match (key.as_str(), &value.kind) {
                ("scale", ast::ExprKind::IntLiteral(n)) => viewport.scale = *n as f64,
                ("scale", ast::ExprKind::FloatLiteral(f)) => viewport.scale = *f,
                ("mobile", ast::ExprKind::BoolLiteral(b)) => viewport.mobile = *b,
                _ => {}
            }
        }
    }

    Some(viewport)
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

    // Second argument should be a lambda (the suite body) — accept either a
    // positional lambda (`describe("X", fn() { ... })`) or a trailing block
    // (`describe("X") do ... end`).
    let second_arg = match &arguments[1] {
        Argument::Positional(expr) => expr,
        Argument::Block(expr) => expr,
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
        viewport: None,
    };

    // Extract tests and nested suites from the lambda body
    extract_tests_from_block(&suite_body, &mut suite);

    Some(suite)
}

/// Pull the first argument out of a `before_each`/`after_each`/`before_all`/
/// `after_all` call, accepting either a positional lambda
/// (`before_each(fn() { ... })`) or a trailing block (`before_each do ... end`).
fn first_callback_expr(arguments: &[Argument]) -> Option<&ast::Expr> {
    match arguments.first()? {
        Argument::Positional(expr) => Some(expr),
        Argument::Block(expr) => Some(expr),
        Argument::Named(_) => None,
    }
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
                        if let Some(callback) = first_callback_expr(arguments) {
                            suite.before_each = Some(ast_expr_to_value(callback));
                        }
                    } else if name == "after_each" {
                        if let Some(callback) = first_callback_expr(arguments) {
                            suite.after_each = Some(ast_expr_to_value(callback));
                        }
                    } else if name == "before_all" {
                        if let Some(callback) = first_callback_expr(arguments) {
                            suite.before_all = Some(ast_expr_to_value(callback));
                        }
                    } else if name == "after_all" {
                        if let Some(callback) = first_callback_expr(arguments) {
                            suite.after_all = Some(ast_expr_to_value(callback));
                        }
                    } else if name == "viewport" {
                        if let Some(viewport) = extract_viewport(arguments) {
                            suite.viewport = Some(viewport);
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
            } => create_function_value(
                params.clone(),
                return_type.as_deref().cloned(),
                body.clone(),
                span,
            ),
            _ => return None,
        },
        Argument::Block(block_expr) => {
            // Convert block expression to lambda function
            match &block_expr.kind {
                ast::ExprKind::Lambda {
                    params,
                    return_type,
                    body,
                } => create_function_value(
                    params.clone(),
                    return_type.as_deref().cloned(),
                    body.clone(),
                    span,
                ),
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
        } => create_function_value(
            params.clone(),
            return_type.as_deref().cloned(),
            body.clone(),
            expr.span,
        ),
        _ => Value::Null,
    }
}

fn execute_test_suites(
    interpreter: &mut interpreter::Interpreter,
    suites: &[interpreter::builtins::test_dsl::TestSuite],
) -> Result<(i64, Vec<String>), error::RuntimeError> {
    execute_test_suites_in(interpreter, suites, "")
}

fn execute_test_suites_in(
    interpreter: &mut interpreter::Interpreter,
    suites: &[interpreter::builtins::test_dsl::TestSuite],
    prefix: &str,
) -> Result<(i64, Vec<String>), error::RuntimeError> {
    use crate::interpreter::builtins::test_progress;
    let mut failed_count = 0i64;
    let mut failed_tests = Vec::new();

    for suite in suites {
        if test_progress::fail_fast_tripped() {
            break;
        }
        let suite_path = format!("{} {}", prefix, suite.name).trim().to_string();
        let stubs_outside_suite = crate::interpreter::builtins::mock::snapshot_stubs();
        // Run before_all if defined
        if let Some(before_all) = &suite.before_all {
            let rebound = rebind_closure(before_all, &interpreter.environment);
            let _ = interpreter.call_value(rebound, Vec::new(), span::Span::new(0, 0, 1, 1));
        }
        // Stubs from `before_all` belong to the whole suite (nested ones included).
        let suite_stubs = crate::interpreter::builtins::mock::snapshot_stubs();

        for test in &suite.tests {
            if test_progress::fail_fast_tripped()
                || !test_progress::matches_name_filter(&format!("{} {}", suite_path, test.name))
            {
                continue;
            }
            crate::interpreter::builtins::datetime::helpers::unfreeze_datetime();
            // The browser outlives a single test on purpose — relaunching one
            // per test would dominate the runtime — so the errors it collected
            // must be cleared, or the first failing page fails every test after
            // it. Setting the suite's viewport first means the reset restores
            // that rather than whatever the previous test resized to.
            crate::interpreter::builtins::browser::set_active_viewport(suite.viewport);
            crate::interpreter::builtins::browser::reset_browser_state();

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

            // The only place individual `test(...)` blocks are counted. The
            // runner's own tally is per file, so without this the suite could
            // report how many files ran and how many assertions fired, but
            // never how many tests.
            crate::interpreter::builtins::test_progress::record_test(result.is_ok());

            if let Err(e) = result {
                failed_count += 1;
                failed_tests.push(format!("{}: {}", test.name, e));
            }

            // A stub set by the test or `before_each` is scoped to the test.
            crate::interpreter::builtins::mock::restore_stubs(suite_stubs.clone());

            // Run after_each if defined
            if let Some(after_each) = &suite.after_each {
                let rebound = rebind_closure(after_each, &interpreter.environment);
                let _ = interpreter.call_value(rebound, Vec::new(), span::Span::new(0, 0, 1, 1));
            }
        }

        // Run nested suites
        let (nested_failed, mut nested_errors) =
            execute_test_suites_in(interpreter, &suite.nested_suites, &suite_path)?;
        failed_count += nested_failed;
        failed_tests.append(&mut nested_errors);

        // Run after_all if defined
        if let Some(after_all) = &suite.after_all {
            let rebound = rebind_closure(after_all, &interpreter.environment);
            let _ = interpreter.call_value(rebound, Vec::new(), span::Span::new(0, 0, 1, 1));
        }
        crate::interpreter::builtins::mock::restore_stubs(stubs_outside_suite);
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
                kernel: None,
            };
            new_func.closure = env.clone();
            Value::Function(std::rc::Rc::new(new_func))
        }
        other => other.clone(),
    }
}

/// Check if a program has any import statements.
/// The directory a script's relative imports resolve from. `Path::parent`
/// of a bare file name is `""`, not `.`, and resolving against `""` found
/// nothing: `soli tool.sl` failed on `import "./lib/math.sl"` that
/// `soli ./tool.sl` loaded.
pub(crate) fn import_base_dir(path: &std::path::Path) -> &std::path::Path {
    path.parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."))
}

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
///
/// `.slv` templates are pre-processed: only the embedded `<% %>` / `<%= %>`
/// code is extracted (HTML stripped, line numbers preserved) before lexing, so
/// the linter runs rules on the Soli code without tripping over markup.
/// `style/empty-block` is dropped for templates because control-flow bodies
/// that contain only HTML legitimately have no Soli statements.
pub fn lint_file(source: &str, path: &str) -> Result<Vec<lint::LintDiagnostic>, SolilangError> {
    if !path.ends_with(".slv") {
        let tokens = lexer::Scanner::new(source).scan_tokens()?;
        let program = parser::Parser::new(tokens).parse()?;
        return Ok(lint::Linter::new(source)
            .with_file_path(path)
            .lint(&program));
    }

    // Template: extract only the embedded code. A genuine malformed-template
    // problem (e.g. an unclosed `<% %>` tag) is surfaced as a Template error.
    let code = template::parser::extract_lintable_code(source).map_err(SolilangError::Template)?;

    // The extracted code can still trip the core parser on constructs the
    // template engine itself parses differently — e.g. `if (a || b) && c`, a
    // known core-parser quirk where `(…)` is taken as the whole condition.
    // Such templates are valid and render fine, so don't turn that into a
    // user-facing parse error: skip linting the file instead.
    let Ok(tokens) = lexer::Scanner::new(&code).scan_tokens() else {
        return Ok(Vec::new());
    };
    let Ok(program) = parser::Parser::new(tokens).parse() else {
        return Ok(Vec::new());
    };

    let mut diagnostics = lint::Linter::new(&code).with_file_path(path).lint(&program);
    // HTML-only control-flow bodies (`<% if x %>…markup…<% end %>`) extract to
    // empty blocks; that isn't a real empty block in the template.
    diagnostics.retain(|d| d.rule != "style/empty-block");

    // `<%= %>` already HTML-escapes its output, so wrapping the expression in
    // an escape helper escapes twice and the page shows literal `&#x27;` /
    // `&amp;` for any value with a special character.
    if let Ok(redundant) = template::parser::redundant_escape_helpers(source) {
        for (line, helper) in redundant {
            diagnostics.push(lint::LintDiagnostic {
                rule: "idiom/redundant-template-escape",
                message: format!(
                    "`<%= {helper}(...) %>` escapes twice — `<%= %>` already HTML-escapes its \
                     output; write the expression bare, or use `<%- %>` for pre-escaped HTML"
                ),
                span: span::Span::new(0, 0, line, 1),
                severity: lint::Severity::Warning,
            });
        }
    }
    Ok(diagnostics)
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

#[cfg(test)]
mod suite_extraction_tests {
    use super::*;

    fn suites(source: &str) -> Vec<interpreter::builtins::test_dsl::TestSuite> {
        let tokens = lexer::Scanner::new(source)
            .scan_tokens()
            .expect("the spec must tokenize");
        let program = parser::Parser::new(tokens)
            .parse()
            .expect("the spec must parse");
        extract_test_definitions(&program)
    }

    #[test]
    fn a_suite_declares_its_viewport_by_preset() {
        let extracted = suites(
            r#"describe("Checkout", fn() {
                 viewport("mobile")
                 test("shows the menu", fn() { assert(true) })
               })"#,
        );
        let viewport = extracted[0].viewport.expect("the declaration must stick");
        assert_eq!(viewport.width, 390);
        assert!(viewport.mobile);
    }

    #[test]
    fn a_suite_can_declare_an_explicit_size_and_options() {
        let extracted = suites(
            r#"describe("Kiosk", fn() {
                 viewport(1024, 768, {"scale": 2, "mobile": true})
                 test("fits", fn() { assert(true) })
               })"#,
        );
        let viewport = extracted[0].viewport.expect("the declaration must stick");
        assert_eq!((viewport.width, viewport.height), (1024, 768));
        assert_eq!(viewport.scale, 2.0);
        assert!(viewport.mobile);
    }

    #[test]
    fn a_nested_suite_inherits_unless_it_says_otherwise() {
        let extracted = suites(
            r#"describe("Dashboard", fn() {
                 context("on a desktop", fn() {
                   viewport("wide")
                   test("shows the sidebar", fn() { assert(true) })
                 })
                 context("on a phone", fn() {
                   context("deeply", fn() {
                     test("still a phone", fn() { assert(true) })
                   })
                   test("hides the sidebar", fn() { assert(true) })
                 })
                 // Declared *below* the nested suites on purpose: reading order
                 // must not decide what a nested suite inherits.
                 viewport("mobile")
               })"#,
        );
        let outer = &extracted[0];
        assert_eq!(outer.viewport.map(|v| v.width), Some(390));

        let desktop = &outer.nested_suites[0];
        assert_eq!(desktop.viewport.map(|v| v.width), Some(1920));

        let phone = &outer.nested_suites[1];
        assert_eq!(phone.viewport.map(|v| v.width), Some(390));
        assert_eq!(phone.nested_suites[0].viewport.map(|v| v.width), Some(390));
    }

    #[test]
    fn a_suite_without_a_declaration_has_no_viewport_of_its_own() {
        let extracted = suites(
            r#"describe("Api", fn() {
                 test("answers", fn() { assert(true) })
               })"#,
        );
        assert!(extracted[0].viewport.is_none());
    }

    #[test]
    fn a_declaration_that_cannot_be_read_statically_is_not_half_applied() {
        // Nothing evaluates a `describe` body, so a computed size has no value
        // to read; falling back to a guessed one would be worse than the
        // default.
        let extracted = suites(
            r#"describe("Computed", fn() {
                 viewport(chosen_width, 800)
                 test("runs", fn() { assert(true) })
               })"#,
        );
        assert!(extracted[0].viewport.is_none());
    }
}
