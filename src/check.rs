//! The project view `soli check` reads each file against.
//!
//! The checker sees one file at a time; a running application does not. A
//! served app loads every auto-loaded directory into one environment, and a
//! script imported by an EUI window script runs in workers that load the widget
//! catalogue. This module gathers what a file can reach without naming it —
//! the library side of `soli check`, so the CLI and the corpus test
//! (`tests/check_corpus_test.rs`) read the same view.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The `.sl` files `soli check` takes from `target`: the file itself, or every
/// `.sl` file below a directory, hidden and `_`-prefixed directories skipped
/// (the rule `soli lint` walks by). Sorted, so reports come out in a stable
/// order.
pub fn source_files(target: &Path) -> Vec<PathBuf> {
    if target.is_file() {
        return vec![target.to_path_buf()];
    }
    let mut files = Vec::new();
    collect(target, &mut files);
    files.sort();
    files
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with('.') && !name.starts_with('_') {
                collect(&path, out);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("sl") {
            out.push(path);
        }
    }
}

/// Top-level names declared anywhere in the auto-loaded directories under any
/// of `targets`, for [`crate::type_check_source_with_ambient`].
///
/// A target inside an application — `soli check app`, `soli check
/// app/controllers/posts_controller.sl` — is widened to that application's
/// root (see [`crate::app_layout::app_root`]): its auto-loaded set is what the
/// running server puts in scope, whichever subset of files is being checked.
///
/// Returns nothing for a loose file or a directory that is not a project: the
/// narrower per-file view is right when there is no project to widen it to.
pub fn ambient_names(targets: &[PathBuf]) -> Vec<String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut seen_roots: HashSet<PathBuf> = HashSet::new();
    for target in targets {
        let candidates = [
            target.is_dir().then(|| target.clone()),
            crate::app_layout::app_root(target),
        ];
        for root in candidates.into_iter().flatten() {
            let key = root.canonicalize().unwrap_or_else(|_| root.clone());
            if seen_roots.insert(key) {
                roots.push(root);
            }
        }
    }

    let mut names: HashSet<String> = HashSet::new();
    for target in &roots {
        for dir in crate::AUTOLOADED_DIRS {
            let dir = target.join(dir);
            if !dir.is_dir() {
                continue;
            }
            for path in source_files(&dir) {
                // Parse only. A neighbour that does not parse is its own error,
                // reported when its turn comes; it must not stop this file from
                // being checked.
                let Some(program) = parse_file(&path) else {
                    continue;
                };
                crate::lint::rules::scope::collect_program_names(&program.statements, &mut names);
            }
        }
    }
    let mut names: Vec<String> = names.into_iter().collect();
    names.sort();
    names
}

/// The names `file` can reach without declaring them: the project's
/// (`project`, from [`ambient_names`]), plus the EUI widget catalogue when a
/// script of the same directory opens an `eui_window` and imports `file`,
/// directly or through another import. Such a module — a screen of
/// `examples/pocket-garden/` — runs in the window's workers, which load the
/// catalogue, but does not mention `eui_window` itself, so it read every
/// `button(…)` as undefined.
pub fn file_ambient(file: &Path, project: &[String]) -> Vec<String> {
    let mut names = project.to_vec();
    #[cfg(feature = "eui")]
    if imported_by_a_window_script(file) {
        names.extend(
            crate::scaffold::templates::eui::catalogue_def_names()
                .iter()
                .cloned(),
        );
    }
    names
}

/// Read and check one file against `project` (see [`file_ambient`]).
pub fn check_file(file: &Path, project: &[String]) -> std::io::Result<crate::CheckReport> {
    let source = std::fs::read_to_string(file)?;
    let ambient = file_ambient(file, project);
    Ok(crate::type_check_report(&source, Some(file), &ambient))
}

fn parse_file(path: &Path) -> Option<crate::ast::Program> {
    let source = std::fs::read_to_string(path).ok()?;
    let tokens = crate::lexer::Scanner::new(&source).scan_tokens().ok()?;
    crate::parser::Parser::new(tokens).parse().ok()
}

#[cfg(feature = "eui")]
fn imported_by_a_window_script(file: &Path) -> bool {
    // An application's files get the catalogue through the project instead.
    if crate::app_layout::app_root(file).is_some() {
        return false;
    }
    let Some(dir) = file.parent() else {
        return false;
    };
    let Ok(target) = file.canonicalize() else {
        return false;
    };
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let script = entry.path();
        if script.extension().and_then(|e| e.to_str()) != Some("sl") {
            continue;
        }
        // Cheap test first: only a script naming the builtin can open a window.
        let Ok(source) = std::fs::read_to_string(&script) else {
            continue;
        };
        if !source.contains("eui_window") {
            continue;
        }
        let Some(program) = parse_file(&script) else {
            continue;
        };
        if !crate::serve::eui::script::mentions_eui_window(&program) {
            continue;
        }
        if imports_reach(&script, &program, &target) {
            return true;
        }
    }
    false
}

/// Whether `target` is among the files `script` imports by a relative path,
/// transitively.
#[cfg(feature = "eui")]
fn imports_reach(script: &Path, program: &crate::ast::Program, target: &Path) -> bool {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut pending: Vec<(PathBuf, Vec<String>)> =
        vec![(script.to_path_buf(), relative_imports(program))];
    while let Some((from, imports)) = pending.pop() {
        let base = from.parent().map(Path::to_path_buf).unwrap_or_default();
        for import in imports {
            let Ok(path) = base.join(&import).canonicalize() else {
                continue;
            };
            if path == target {
                return true;
            }
            if seen.insert(path.clone()) {
                if let Some(program) = parse_file(&path) {
                    pending.push((path, relative_imports(&program)));
                }
            }
        }
    }
    false
}

#[cfg(feature = "eui")]
fn relative_imports(program: &crate::ast::Program) -> Vec<String> {
    use crate::ast::stmt::StmtKind;

    program
        .statements
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Import(decl) if decl.path.starts_with('.') => Some(decl.path.clone()),
            _ => None,
        })
        .collect()
}
