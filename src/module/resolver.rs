//! Module resolution for import statements.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::{ImportDecl, ImportSpecifier, Program, Stmt, StmtKind};
use crate::lexer::Scanner;
use crate::parser::Parser;

use super::lockfile::LockFile;
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
    /// Optional lock file for git dependencies
    lock: Option<LockFile>,
    /// Cache of resolved modules
    cache: HashMap<PathBuf, ResolvedModule>,
    /// Currently resolving stack (for cycle detection)
    resolving: Vec<PathBuf>,
}

impl ModuleResolver {
    /// Create a new module resolver.
    pub fn new(base_dir: &Path) -> Self {
        // Try to find and load package file
        let (package, lock) = match Package::find(base_dir) {
            Some(pkg_path) => {
                let pkg = Package::load(&pkg_path).ok();
                // Try to load lock file from same directory as soli.toml
                let lock_path = pkg_path.with_file_name("soli.lock");
                let lock = LockFile::load(&lock_path).ok();
                (pkg, lock)
            }
            None => (None, None),
        };

        ModuleResolver {
            base_dir: base_dir.to_path_buf(),
            package,
            lock,
            cache: HashMap::new(),
            resolving: Vec::new(),
        }
    }

    /// Create a module resolver with an explicit package.
    pub fn with_package(base_dir: &Path, package: Package) -> Self {
        // Try to load lock file
        let lock_path = base_dir.join("soli.lock");
        let lock = LockFile::load(&lock_path).ok();

        ModuleResolver {
            base_dir: base_dir.to_path_buf(),
            package: Some(package),
            lock,
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
                let (imported_stmts, module_path) = get_imported_statements(&module, import)?;
                for mut imported_stmt in imported_stmts {
                    imported_stmt = set_stmt_source_path(&imported_stmt, module_path.clone());
                    combined_statements.push(imported_stmt);
                }
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

        // Read and parse the module through VFS (or disk fallback). In a
        // protected bundle the imported file is a serialized AST, not source.
        let path_str = module_path.to_string_lossy().to_string();
        let bytes = crate::serve::vfs_read(&path_str).map_err(std::io::Error::other)?;
        let program = if crate::bundle::is_ast_blob(&bytes) {
            crate::bundle::deserialize_program(&bytes).map_err(|e| {
                ResolveError::ParseError(format!("in {}: {}", module_path.display(), e))
            })?
        } else {
            let content = String::from_utf8(bytes).map_err(|e| {
                ResolveError::ParseError(format!(
                    "in {}: not valid UTF-8: {}",
                    module_path.display(),
                    e
                ))
            })?;
            let tokens = Scanner::new(&content).scan_tokens().map_err(|e| {
                ResolveError::ParseError(format!("in {}: {}", module_path.display(), e))
            })?;
            match Parser::new(tokens).parse() {
                Ok(p) => p,
                Err(e) => {
                    return Err(ResolveError::ParseError(format!(
                        "in {}: {}",
                        module_path.display(),
                        e
                    )))
                }
            }
        };

        // Track that we're resolving this module
        self.resolving.push(module_path.clone());

        // Recursively resolve imports in the module
        let resolved_program = self.resolve(program.clone(), &module_path)?;

        // Done resolving this module
        self.resolving.pop();

        // Collect exports from the original program
        let exports = collect_exports(&program);

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
    ///
    /// SEC-076: Every resolved file must stay inside an allowed containment
    /// root: the project `base_dir`, a path-dependency root from `soli.toml`,
    /// or a lockfile cache path. Absolute import strings are rejected
    /// outright (they would otherwise escape `base_dir.join(...)` semantics).
    fn resolve_path(&self, import_path: &str, from_path: &Path) -> Result<PathBuf, ResolveError> {
        // SEC-076: refuse absolute import strings — they bypass `base_dir.join`
        // and let untrusted Soli source read arbitrary readable `.sl` files.
        // `has_root` too: on Windows `/tmp/evil.sl` is rooted but not absolute,
        // so an `is_absolute`-only test let it through this gate. (The
        // canonicalizing `validate_within` check downstream still caught it —
        // this restores the intended first line of defense.)
        let candidate = Path::new(import_path);
        if candidate.is_absolute() || candidate.has_root() {
            return Err(ResolveError::ImportError(format!(
                "Absolute import paths are not allowed: '{}'",
                import_path
            )));
        }

        // Relative path (starts with . or ..)
        if import_path.starts_with('.') {
            let from_dir = from_path.parent().unwrap_or(Path::new("."));
            let resolved = from_dir.join(import_path);
            let found = self.find_module_file(&resolved)?;
            // Containment is anchored on whichever root currently owns
            // `from_path` — the project, a path-dep, or a cached package —
            // so a module inside `~/.soli/packages/foo/` can `./bar` to a
            // sibling but cannot `../../../etc/passwd` out of its own tree.
            let root = self.containment_root_for(from_path);
            return self.validate_within(&found, &root);
        }

        // Check package dependencies
        if let Some(ref pkg) = self.package {
            // Check if import_path matches a dependency name
            let parts: Vec<&str> = import_path.split('/').collect();
            if let Some(dep) = pkg.dependencies.get(parts[0]) {
                let dep_root: PathBuf = match dep {
                    super::package::Dependency::Path(dep_path) => self.base_dir.join(dep_path),
                    super::package::Dependency::Version(_)
                    | super::package::Dependency::Git { .. } => self
                        .lock
                        .as_ref()
                        .and_then(|l| l.packages.get(parts[0]))
                        .map(|e| e.cache_path.clone())
                        .ok_or_else(|| {
                            ResolveError::NotFound(format!(
                                "Package '{}' not installed. Run 'soli install'",
                                import_path
                            ))
                        })?,
                };

                let mut resolved = dep_root.clone();
                for part in &parts[1..] {
                    resolved = resolved.join(part);
                }
                let found = self.find_module_file(&resolved)?;
                // SEC-076: a malicious package shipped via the registry or as
                // a path-dep could include sub-paths like `pkg/../../etc` —
                // verify the resolved file stays inside the dep's own root.
                return self.validate_within(&found, &dep_root);
            }
        }

        // Bare-name fallback: resolve under the project base directory.
        // (The earlier absolute-path check ensures `import_path` is relative,
        // so `base_dir.join(...)` cannot be replaced by an absolute root.)
        let resolved = self.base_dir.join(import_path);
        let found = self.find_module_file(&resolved)?;
        self.validate_within(&found, &self.base_dir)
    }

    /// Pick the containment root that owns `from_path`. Order of preference:
    /// any matching lockfile cache path → any matching path-dependency root
    /// → the project `base_dir`. Roots are canonicalised on the fly so
    /// symlinked paths resolve consistently.
    fn containment_root_for(&self, from_path: &Path) -> PathBuf {
        let canonical_from = from_path
            .canonicalize()
            .unwrap_or_else(|_| from_path.to_path_buf());

        if let Some(lock) = &self.lock {
            for entry in lock.packages.values() {
                if let Ok(canon_cache) = entry.cache_path.canonicalize() {
                    if canonical_from.starts_with(&canon_cache) {
                        return canon_cache;
                    }
                }
            }
        }

        if let Some(pkg) = &self.package {
            for dep in pkg.dependencies.values() {
                if let super::package::Dependency::Path(p) = dep {
                    if let Ok(canon_dep) = self.base_dir.join(p).canonicalize() {
                        if canonical_from.starts_with(&canon_dep) {
                            return canon_dep;
                        }
                    }
                }
            }
        }

        self.base_dir
            .canonicalize()
            .unwrap_or_else(|_| self.base_dir.clone())
    }

    /// Reject `resolved` if it does not sit under `root` after both are
    /// canonicalised. Symlink escapes are caught here because `resolved`
    /// comes back canonicalised from `find_module_file`, so any symlink in
    /// the path or as the target is already resolved.
    fn validate_within(&self, resolved: &Path, root: &Path) -> Result<PathBuf, ResolveError> {
        let canon_root = root
            .canonicalize()
            .map_err(|_| ResolveError::NotFound(root.display().to_string()))?;
        if !resolved.starts_with(&canon_root) {
            return Err(ResolveError::ImportError(format!(
                "Import '{}' resolves outside its allowed root '{}'",
                resolved.display(),
                canon_root.display()
            )));
        }
        Ok(resolved.to_path_buf())
    }

    /// Find the actual module file (handles .sl extension).
    fn find_module_file(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        // Normalize the path to resolve . and .. components
        // This is needed because paths like "../stdlib/file.sl" don't exist as-is
        let normalized = normalize_path(path);

        // Try exact path
        let normalized_str = normalized.to_string_lossy().to_string();
        if crate::serve::vfs_exists(&normalized_str) && normalized.is_file() {
            return self.canonicalize(&normalized);
        }

        // Try with .sl extension
        let with_ext = normalized.with_extension("sl");
        let with_ext_str = with_ext.to_string_lossy().to_string();
        if crate::serve::vfs_exists(&with_ext_str) && with_ext.is_file() {
            return self.canonicalize(&with_ext);
        }

        // Try as directory with index.sl
        let index = normalized.join("index.sl");
        let index_str = index.to_string_lossy().to_string();
        if crate::serve::vfs_exists(&index_str) && index.is_file() {
            return self.canonicalize(&index);
        }

        // Try as directory with mod.sl
        let mod_file = normalized.join("mod.sl");
        let mod_file_str = mod_file.to_string_lossy().to_string();
        if crate::serve::vfs_exists(&mod_file_str) && mod_file.is_file() {
            return self.canonicalize(&mod_file);
        }

        Err(ResolveError::NotFound(path.display().to_string()))
    }

    /// Canonicalize a path (resolve symlinks, etc.).
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        path.canonicalize()
            .map_err(|_| ResolveError::NotFound(path.display().to_string()))
    }
}

/// Normalize a path string by resolving . and .. components.
fn normalize_path(path: &Path) -> PathBuf {
    if let Some(s) = path.to_str() {
        let mut result = Vec::new();
        let starts_with_slash = s.starts_with('/');
        for component in s.split('/') {
            match component {
                "" => {
                    // Keep leading slash marker
                    if starts_with_slash && result.is_empty() {
                        // Don't add empty component for leading slash
                    }
                }
                "." => {}
                ".." => {
                    if !result.is_empty() && result.last() != Some(&"..") {
                        result.pop();
                    } else {
                        result.push("..");
                    }
                }
                _ => result.push(component),
            }
        }
        let normalized_str = if starts_with_slash {
            format!("/{}", result.join("/"))
        } else {
            result.join("/")
        };
        PathBuf::from(normalized_str)
    } else {
        path.to_path_buf()
    }
}

/// Collect exported names from a program.
fn collect_exports(program: &Program) -> HashSet<String> {
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
    module: &ResolvedModule,
    import: &ImportDecl,
) -> Result<(Vec<Stmt>, PathBuf), ResolveError> {
    let module_path = module.path.clone();
    let stmts = match &import.specifier {
        ImportSpecifier::All => {
            // Import all exported definitions
            let mut stmts = Vec::new();
            for stmt in &module.original_program.statements {
                if let StmtKind::Export(inner) = &stmt.kind {
                    stmts.push((**inner).clone());
                }
            }
            stmts
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
            stmts
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
            stmts
        }
    };
    Ok((stmts, module_path))
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
        StmtKind::Function(decl) => {
            decl.name = new_name.to_string();
        }
        StmtKind::Class(decl) => {
            decl.name = new_name.to_string();
        }
        StmtKind::Interface(decl) => {
            decl.name = new_name.to_string();
        }
        StmtKind::Let { name, .. } => {
            *name = new_name.to_string();
        }
        _ => {}
    }

    new_stmt
}

/// Set source_path on a statement and all nested statements.
fn set_stmt_source_path(stmt: &Stmt, source_path: PathBuf) -> Stmt {
    let mut new_stmt = stmt.clone();
    new_stmt.source_path = Some(source_path.clone());

    use crate::ast::StmtKind::*;
    match &mut new_stmt.kind {
        Function(decl) => {
            for s in &mut decl.body {
                *s = set_stmt_source_path(s, source_path.clone());
            }
        }
        Class(decl) => {
            for s in &mut decl.class_statements {
                *s = set_stmt_source_path(s, source_path.clone());
            }
            if let Some(ref mut ctor) = decl.constructor {
                for s in &mut ctor.body {
                    *s = set_stmt_source_path(s, source_path.clone());
                }
            }
            for method in &mut decl.methods {
                for s in &mut method.body {
                    *s = set_stmt_source_path(s, source_path.clone());
                }
            }
        }
        Block(stmts) => {
            for s in stmts {
                *s = set_stmt_source_path(s, source_path.clone());
            }
        }
        If {
            then_branch,
            else_branch,
            ..
        } => {
            **then_branch = set_stmt_source_path(then_branch, source_path.clone());
            if let Some(else_stmt) = else_branch {
                *else_branch = Some(Box::new(set_stmt_source_path(
                    else_stmt,
                    source_path.clone(),
                )));
            }
        }
        While { body, .. } => {
            **body = set_stmt_source_path(body, source_path.clone());
        }
        For { body, .. } => {
            **body = set_stmt_source_path(body, source_path.clone());
        }
        Try {
            try_block,
            catch_clauses,
            finally_block,
        } => {
            **try_block = set_stmt_source_path(try_block, source_path.clone());
            for clause in catch_clauses {
                *clause.body = set_stmt_source_path(&clause.body, source_path.clone());
            }
            if let Some(finally) = finally_block {
                *finally_block = Some(Box::new(set_stmt_source_path(finally, source_path.clone())));
            }
        }
        _ => {}
    }

    new_stmt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn rejects_absolute_import_path() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("main.sl");
        write(&from, "");
        write(&tmp.path().join("evil.sl"), "");

        let resolver = ModuleResolver::new(tmp.path());
        let err = resolver
            .resolve_path("/tmp/evil.sl", &from)
            .expect_err("absolute path must be rejected");
        match err {
            ResolveError::ImportError(msg) => {
                assert!(msg.contains("Absolute import"), "{}", msg);
            }
            other => panic!("expected ImportError, got {:?}", other),
        }
    }

    #[test]
    fn rejects_relative_import_escaping_project_root() {
        // Layout:
        //   <root>/proj/main.sl    (the project)
        //   <root>/outside.sl      (outside the project root)
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        write(&proj.join("main.sl"), "");
        write(&tmp.path().join("outside.sl"), "");

        let resolver = ModuleResolver::new(&proj);
        let err = resolver
            .resolve_path("../outside.sl", &proj.join("main.sl"))
            .expect_err("relative escape must be rejected");
        match err {
            ResolveError::ImportError(msg) => {
                assert!(msg.contains("outside its allowed root"), "{}", msg);
            }
            ResolveError::NotFound(_) => panic!("file existed; should be ImportError"),
            other => panic!("expected ImportError, got {:?}", other),
        }
    }

    #[test]
    // As above: gated wholesale, since the API is absent off Unix.
    #[cfg(unix)]
    fn rejects_relative_import_escaping_via_symlink() {
        // Layout:
        //   <root>/proj/main.sl
        //   <root>/proj/escape  -> <root>/outside    (symlink)
        //   <root>/outside/secret.sl
        // Importing "./escape/secret.sl" should be rejected because the
        // canonical path resolves outside the project root.
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        let outside = tmp.path().join("outside");
        write(&proj.join("main.sl"), "");
        write(&outside.join("secret.sl"), "");

        std::os::unix::fs::symlink(&outside, proj.join("escape")).unwrap();

        let resolver = ModuleResolver::new(&proj);
        let err = resolver
            .resolve_path("./escape/secret.sl", &proj.join("main.sl"))
            .expect_err("symlink escape must be rejected");
        match err {
            ResolveError::ImportError(msg) => {
                assert!(msg.contains("outside its allowed root"), "{}", msg);
            }
            other => panic!("expected ImportError, got {:?}", other),
        }
    }

    #[test]
    fn accepts_valid_relative_import_inside_root() {
        // Layout:
        //   <root>/proj/main.sl
        //   <root>/proj/lib/util.sl
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        write(&proj.join("main.sl"), "");
        write(&proj.join("lib/util.sl"), "");

        let resolver = ModuleResolver::new(&proj);
        let resolved = resolver
            .resolve_path("./lib/util.sl", &proj.join("main.sl"))
            .expect("valid relative import must resolve");
        assert!(resolved.ends_with("lib/util.sl"), "{}", resolved.display());
    }

    #[test]
    fn accepts_valid_bare_name_under_base_dir() {
        // Layout:
        //   <root>/proj/main.sl
        //   <root>/proj/helper.sl
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        write(&proj.join("main.sl"), "");
        write(&proj.join("helper.sl"), "");

        let resolver = ModuleResolver::new(&proj);
        let resolved = resolver
            .resolve_path("helper.sl", &proj.join("main.sl"))
            .expect("bare-name import under base must resolve");
        assert!(resolved.ends_with("helper.sl"));
    }
}
