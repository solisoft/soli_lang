//! Solilang: A statically-typed, class-based OOP language with pipeline operators.
//!
//! This is the library root that exports all modules.
//!
//! # Execution Modes
//!
//! Solilang supports multiple execution modes:
//! - **Tree-walk interpreter**: Simple, direct AST interpretation
//! - **Bytecode VM**: Faster execution via bytecode compilation
//! - **JIT compilation**: (with `jit` feature) Native code for hot paths

pub mod ast;
pub mod bytecode;
pub mod error;
pub mod interpreter;
pub mod lexer;
pub mod module;
pub mod parser;
pub mod serve;
pub mod span;
pub mod template;
pub mod types;

#[cfg(feature = "jit")]
pub mod jit;

use error::SolilangError;

/// Execution mode for running Solilang programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    /// Tree-walking interpreter (default for compatibility)
    #[default]
    TreeWalk,
    /// Bytecode virtual machine (faster)
    Bytecode,
    /// JIT compilation (fastest, requires `jit` feature)
    #[cfg(feature = "jit")]
    Jit,
}

/// Run a Solilang program from source code using the default execution mode.
pub fn run(source: &str) -> Result<(), SolilangError> {
    run_with_options(source, ExecutionMode::default(), true, false)
}

/// Run a Solilang program with optional type checking.
pub fn run_with_type_check(source: &str, type_check: bool) -> Result<(), SolilangError> {
    run_with_options(source, ExecutionMode::default(), type_check, false)
}

/// Run a Solilang program with bytecode VM.
pub fn run_bytecode(source: &str) -> Result<(), SolilangError> {
    run_with_options(source, ExecutionMode::Bytecode, true, false)
}

/// Run a Solilang program with bytecode VM and optional disassembly output.
pub fn run_bytecode_with_disassembly(source: &str, disassemble: bool) -> Result<(), SolilangError> {
    run_with_options(source, ExecutionMode::Bytecode, true, disassemble)
}

/// Run a Solilang program with full control over execution options.
pub fn run_with_options(
    source: &str,
    mode: ExecutionMode,
    type_check: bool,
    disassemble: bool,
) -> Result<(), SolilangError> {
    run_with_path(source, None, mode, type_check, disassemble)
}

/// Run a Solilang program from a file path with module resolution.
pub fn run_file(
    path: &std::path::Path,
    mode: ExecutionMode,
    type_check: bool,
    disassemble: bool,
) -> Result<(), SolilangError> {
    let source = std::fs::read_to_string(path).map_err(|e| error::RuntimeError::General {
        message: format!("Failed to read file '{}': {}", path.display(), e),
        span: span::Span::new(0, 0, 1, 1),
    })?;

    run_with_path(&source, Some(path), mode, type_check, disassemble)
}

/// Run a Solilang program with optional source path for module resolution.
pub fn run_with_path(
    source: &str,
    source_path: Option<&std::path::Path>,
    mode: ExecutionMode,
    type_check: bool,
    disassemble: bool,
) -> Result<(), SolilangError> {
    // Lexing
    let tokens = lexer::Scanner::new(source).scan_tokens()?;

    // Parsing
    let mut program = parser::Parser::new(tokens).parse()?;

    // Module resolution (if we have imports and a source path)
    if source_path.is_some() && has_imports(&program) {
        let path = source_path.unwrap();
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

    // Execution based on mode
    match mode {
        ExecutionMode::TreeWalk => {
            let mut interpreter = interpreter::Interpreter::new();
            interpreter.interpret(&program)?;
        }
        ExecutionMode::Bytecode => {
            // Compile to bytecode
            let mut compiler = bytecode::Compiler::new();
            let function = compiler.compile(&program)?;

            // Optionally print disassembly
            if disassemble {
                bytecode::print_disassembly(&function);
                println!("---");
            }

            // Execute on VM
            let mut vm = bytecode::VM::new();
            vm.run(function)?;
        }
        #[cfg(feature = "jit")]
        ExecutionMode::Jit => {
            // JIT compilation path
            let mut compiler = bytecode::Compiler::new();
            let function = compiler.compile(&program)?;

            // Run with JIT
            let mut vm = jit::JitVM::new();
            vm.run(function)?;
        }
    }

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

/// Compile source code to bytecode without executing.
pub fn compile(source: &str) -> Result<bytecode::CompiledFunction, SolilangError> {
    let tokens = lexer::Scanner::new(source).scan_tokens()?;
    let program = parser::Parser::new(tokens).parse()?;
    let mut compiler = bytecode::Compiler::new();
    let function = compiler.compile(&program)?;
    Ok(function)
}

/// Disassemble compiled bytecode to a string.
pub fn disassemble(function: &bytecode::CompiledFunction) -> String {
    bytecode::disassemble_function(function)
}
