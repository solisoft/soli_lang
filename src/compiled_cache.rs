//! Compiled module cache for avoiding repeated compilation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::SolilangError;
use crate::lexer::Scanner;
use crate::module::ModuleResolver;
use crate::parser::Parser;
use crate::types::TypeChecker;
use crate::vm::chunk::CompiledModule;
use crate::vm::Compiler;

fn has_imports(program: &crate::ast::Program) -> bool {
    program
        .statements
        .iter()
        .any(|stmt| matches!(stmt.kind, crate::ast::StmtKind::Import(_)))
}

/// Most compiled modules kept (see `get_or_compile`).
const MODULE_CACHE_MAX: usize = 128;

static MODULE_CACHE: OnceLock<Arc<Mutex<HashMap<String, CompiledModule>>>> = OnceLock::new();

fn get_cache() -> &'static Arc<Mutex<HashMap<String, CompiledModule>>> {
    MODULE_CACHE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub fn get_or_compile(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
) -> Result<CompiledModule, SolilangError> {
    let cache_key = source.to_string();

    if let Some(cached) = get_cache().lock().unwrap().get(&cache_key) {
        return Ok(cached.clone());
    }

    let compiled = compile(source, source_path, type_check)?;

    {
        let mut cache = get_cache().lock().unwrap();
        // Keyed by the whole source, so each edited version of a file is a new
        // entry that the old ones never make room for. Past the cap, start over:
        // a recompile is cheap next to keeping every revision for the process's
        // life.
        if cache.len() >= MODULE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(cache_key, compiled.clone());
    }

    Ok(compiled)
}

fn compile(
    source: &str,
    source_path: Option<&std::path::Path>,
    type_check: bool,
) -> Result<CompiledModule, SolilangError> {
    let tokens = Scanner::new(source).scan_tokens()?;

    let mut program = Parser::new(tokens).parse()?;

    if let Some(path) = source_path.filter(|_| has_imports(&program)) {
        let base_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let mut resolver = ModuleResolver::new(base_dir);
        program =
            resolver
                .resolve(program, path)
                .map_err(|e| crate::error::RuntimeError::General {
                    message: format!("Module resolution error: {}", e),
                    span: crate::span::Span::new(0, 0, 1, 1),
                })?;
    }

    if type_check {
        let mut checker = TypeChecker::new();
        if let Err(errors) = checker.check(&program) {
            return Err(errors.into_iter().next().unwrap().into());
        }
    }

    let module = Compiler::compile(&program).map_err(|e| crate::error::RuntimeError::General {
        message: format!("Compile error: {}", e),
        span: crate::span::Span::new(0, 0, 1, 1),
    })?;

    Ok(module)
}

pub fn clear_cache() {
    if let Some(cache) = MODULE_CACHE.get() {
        cache.lock().unwrap().clear();
    }
}
