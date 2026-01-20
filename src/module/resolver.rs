//! Module resolution for import statements.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{ImportDecl, ImportSpecifier, Program, Stmt, StmtKind};
use crate::lexer::Scanner;
use crate::parser::Parser;

use super::package::Package;

/// A resolved module with its exports.
#[derive(Debug, Clone)]
pub struct ResolvedModule {
    /// Canonical path to the module
    pub path: PathBuf,
    /// Original parsed program (before resolution, contains Export statements)
    pub original_program: Program,
    /// Resolved program (imports resolved, exports unwrapped)
    pub program: Program,
    /// Names exported by this module
    pub exports: HashSet<String>,
}

/// Errors that can occur during module resolution.
#[derive(Debug)]
pub enum ResolveError {
    /// File not found
    NotFound(String),
    /// Circular dependency detected
    CircularDependency(Vec<String>),
    /// Import error (item not exported)
    ImportError(String),
    /// Parse error in module
    ParseError(String),
    /// IO error
    IoError(std::io::Error),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::NotFound(path) => write!(f, "Module not found: {}", path),
            ResolveError::CircularDependency(cycle) => {
                write!(f, "Circular dependency: {}", cycle.join(" -> "))
            }
            ResolveError::ImportError(msg) => write!(f, "Import error: {}", msg),
            ResolveError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            ResolveError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for ResolveError {}

impl From<std::io::Error> for ResolveError {
    fn from(e: std::io::Error) -> Self {
        ResolveError::IoError(e)
    }
}

/// Module resolver that handles import resolution.
pub struct ModuleResolver {
    /// Base directory for resolving relative paths
    base_dir: PathBuf,
    /// Optional package configuration
    package: Option<Package>,
    /// Cache of resolved modules
    cache: HashMap<PathBuf, ResolvedModule>,
    /// Currently resolving stack (for cycle detection)
    resolving: Vec<PathBuf>,
}

impl ModuleResolver {
    /// Create a new module resolver.
    pub fn new(base_dir: &Path) -> Self {
        // Try to find and load package file
        let package = Package::find(base_dir).and_then(|p| Package::load(&p).ok());

        ModuleResolver {
            base_dir: base_dir.to_path_buf(),
            package,
            cache: HashMap::new(),
            resolving: Vec::new(),
        }
    }

    /// Create a module resolver with an explicit package.
    pub fn with_package(base_dir: &Path, package: Package) -> Self {
        ModuleResolver {
            base_dir: base_dir.to_path_buf(),
            package: Some(package),
            cache: HashMap::new(),
            resolving: Vec::new(),
        }
    }

    /// Resolve all imports in a program and return a combined program.
    ///
    /// The returned program contains:
    /// 1. All imported definitions (exported from other modules)
    /// 2. All statements from the main program (with imports removed)
    pub fn resolve(
        &mut self,
        program: Program,
        source_path: &Path,
    ) -> Result<Program, ResolveError> {
        let canonical = self.canonicalize(source_path)?;
        let mut combined_statements = Vec::new();

        // First pass: collect all imports and their resolved modules
        for stmt in &program.statements {
            if let StmtKind::Import(import) = &stmt.kind {
                let module = self.resolve_import(import, &canonical)?;

                // Add the imported definitions to the combined program
                let imported_stmts = self.get_imported_statements(&module, import)?;
                combined_statements.extend(imported_stmts);
            }
        }

        // Second pass: add non-import statements from the main program
        for stmt in program.statements {
            match &stmt.kind {
                StmtKind::Import(_) => {
                    // Skip imports, they've been resolved
                }
                StmtKind::Export(inner) => {
                    // Unwrap exports for the main module (they're still executed)
                    combined_statements.push((**inner).clone());
                }
                _ => {
                    combined_statements.push(stmt);
                }
            }
        }

        Ok(Program::new(combined_statements))
    }

    /// Resolve an import declaration.
    fn resolve_import(
        &mut self,
        import: &ImportDecl,
        from_path: &Path,
    ) -> Result<ResolvedModule, ResolveError> {
        let module_path = self.resolve_path(&import.path, from_path)?;

        // Check for circular dependency
        if self.resolving.contains(&module_path) {
            let cycle: Vec<String> = self
                .resolving
                .iter()
                .map(|p| p.display().to_string())
                .chain(std::iter::once(module_path.display().to_string()))
                .collect();
            return Err(ResolveError::CircularDependency(cycle));
        }

        // Check cache
        if let Some(cached) = self.cache.get(&module_path) {
            return Ok(cached.clone());
        }

        // Read and parse the module
        let content = fs::read_to_string(&module_path)?;
        let tokens = Scanner::new(&content).scan_tokens().map_err(|e| {
            ResolveError::ParseError(format!("in {}: {}", module_path.display(), e))
        })?;
        let program = match Parser::new(tokens).parse() {
            Ok(p) => p,
            Err(e) => {
                return Err(ResolveError::ParseError(format!(
                    "in {}: {}",
                    module_path.display(),
                    e
                )))
            }
        };

        // Track that we're resolving this module
        self.resolving.push(module_path.clone());

        // Recursively resolve imports in the module
        let resolved_program = self.resolve(program.clone(), &module_path)?;

        // Done resolving this module
        self.resolving.pop();

        // Collect exports from the original program
        let exports = self.collect_exports(&program);

        let module = ResolvedModule {
            path: module_path.clone(),
            original_program: program,
            program: resolved_program,
            exports,
        };

        // Cache the result
        self.cache.insert(module_path, module.clone());

        Ok(module)
    }

    /// Resolve an import path to an absolute file path.
    fn resolve_path(&self, import_path: &str, from_path: &Path) -> Result<PathBuf, ResolveError> {
        // Relative path (starts with . or ..)
        if import_path.starts_with('.') {
            let from_dir = from_path.parent().unwrap_or(Path::new("."));
            let resolved = from_dir.join(import_path);
            return self.find_module_file(&resolved);
        }

        // Check package dependencies
        if let Some(ref pkg) = self.package {
            // Check if import_path matches a dependency name
            let parts: Vec<&str> = import_path.split('/').collect();
            if let Some(dep) = pkg.dependencies.get(parts[0]) {
                match dep {
                    super::package::Dependency::Path(dep_path) => {
                        let mut resolved = self.base_dir.join(dep_path);
                        // If there's a sub-path, append it
                        for part in &parts[1..] {
                            resolved = resolved.join(part);
                        }
                        return self.find_module_file(&resolved);
                    }
                    super::package::Dependency::Version(_) => {
                        return Err(ResolveError::NotFound(format!(
                            "Version-based dependencies not yet supported: {}",
                            import_path
                        )));
                    }
                }
            }
        }

        // Absolute path from base directory
        let resolved = self.base_dir.join(import_path);
        self.find_module_file(&resolved)
    }

    /// Find the actual module file (handles .soli extension).
    fn find_module_file(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        // Try exact path
        if path.exists() && path.is_file() {
            return self.canonicalize(path);
        }

        // Try with .soli extension
        let with_ext = path.with_extension("soli");
        if with_ext.exists() && with_ext.is_file() {
            return self.canonicalize(&with_ext);
        }

        // Try as directory with index.soli
        let index = path.join("index.soli");
        if index.exists() && index.is_file() {
            return self.canonicalize(&index);
        }

        // Try as directory with mod.soli
        let mod_file = path.join("mod.soli");
        if mod_file.exists() && mod_file.is_file() {
            return self.canonicalize(&mod_file);
        }

        Err(ResolveError::NotFound(path.display().to_string()))
    }

    /// Canonicalize a path (resolve symlinks, etc.).
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        path.canonicalize()
            .map_err(|_| ResolveError::NotFound(path.display().to_string()))
    }

    /// Collect exported names from a program.
    fn collect_exports(&self, program: &Program) -> HashSet<String> {
        let mut exports = HashSet::new();

        for stmt in &program.statements {
            if let StmtKind::Export(inner) = &stmt.kind {
                if let Some(name) = get_declaration_name(inner) {
                    exports.insert(name);
                }
            }
        }

        exports
    }

    /// Get the statements to import from a module based on the import specifier.
    fn get_imported_statements(
        &self,
        module: &ResolvedModule,
        import: &ImportDecl,
    ) -> Result<Vec<Stmt>, ResolveError> {
        match &import.specifier {
            ImportSpecifier::All => {
                // Import all exported definitions
                let mut stmts = Vec::new();
                for stmt in &module.original_program.statements {
                    if let StmtKind::Export(inner) = &stmt.kind {
                        stmts.push((**inner).clone());
                    }
                }
                Ok(stmts)
            }

            ImportSpecifier::Named(items) => {
                // Import specific named items
                let mut stmts = Vec::new();
                for item in items {
                    if !module.exports.contains(&item.name) {
                        return Err(ResolveError::ImportError(format!(
                            "'{}' is not exported from '{}'",
                            item.name, import.path
                        )));
                    }

                    // Find the exported statement in the original program
                    for stmt in &module.original_program.statements {
                        if let StmtKind::Export(inner) = &stmt.kind {
                            if let Some(name) = get_declaration_name(inner) {
                                if name == item.name {
                                    if let Some(ref alias) = item.alias {
                                        // Rename the declaration
                                        let renamed = rename_declaration(inner, alias);
                                        stmts.push(renamed);
                                    } else {
                                        stmts.push((**inner).clone());
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                Ok(stmts)
            }

            ImportSpecifier::Namespace(_name) => {
                // Namespace imports create a module object
                // For now, we just import all exports
                // TODO: Implement proper namespace object
                let mut stmts = Vec::new();
                for stmt in &module.original_program.statements {
                    if let StmtKind::Export(inner) = &stmt.kind {
                        stmts.push((**inner).clone());
                    }
                }
                Ok(stmts)
            }
        }
    }
}

/// Get the name declared by a statement.
fn get_declaration_name(stmt: &Stmt) -> Option<String> {
    match &stmt.kind {
        StmtKind::Function(decl) => Some(decl.name.clone()),
        StmtKind::Class(decl) => Some(decl.name.clone()),
        StmtKind::Interface(decl) => Some(decl.name.clone()),
        StmtKind::Let { name, .. } => Some(name.clone()),
        _ => None,
    }
}

/// Rename a declaration.
fn rename_declaration(stmt: &Stmt, new_name: &str) -> Stmt {
    let mut new_stmt = stmt.clone();

    match &mut new_stmt.kind {
        StmtKind::Function(ref mut decl) => {
            decl.name = new_name.to_string();
        }
        StmtKind::Class(ref mut decl) => {
            decl.name = new_name.to_string();
        }
        StmtKind::Interface(ref mut decl) => {
            decl.name = new_name.to_string();
        }
        StmtKind::Let { ref mut name, .. } => {
            *name = new_name.to_string();
        }
        _ => {}
    }

    new_stmt
}

// Module resolution tests would require tempfile crate which is not in dev-dependencies.
// These tests should be moved to integration tests or tempfile should be added.
