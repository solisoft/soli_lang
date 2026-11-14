//! Where an application's auto-loaded code lives, in one place.
//!
//! Three lists of auto-loaded directories used to disagree: `soli check` had
//! no `app/jobs` or `app/subscribers`, the duplicate-global check had both,
//! and the linter had `app/helpers` and `app/jobs` but no policies, mailers or
//! subscribers. `soli check`, the duplicate-global check, the linter and the
//! language server all read these now.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Directories a running server loads into the application's one global
/// environment: a declaration in any of them is reachable from all of them
/// with no import.
pub const APP_GLOBAL_DIRS: &[&str] = &[
    "app/models",
    "app/services",
    "app/policies",
    "app/mailers",
    "app/controllers",
    "app/middleware",
    "app/jobs",
    "app/subscribers",
];

/// Directories loaded into the views' helper environment, a namespace of its
/// own (a helper and a model may share a name).
pub const HELPER_DIRS: &[&str] = &["app/helpers"];

/// Directories outside `app/` whose top-level names `soli check` also treats
/// as ambient: configuration and the application's own stdlib.
pub const SUPPORT_DIRS: &[&str] = &["config", "stdlib"];

/// Directory names a project walk never enters.
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "vendor", "tmp"];

/// The root of the Soli application `target` sits in: the nearest directory,
/// `target` itself included, holding `soli.toml` or `config/routes.sl`.
/// `None` outside any application.
pub fn app_root(target: &Path) -> Option<PathBuf> {
    let absolute = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    let start = if absolute.is_dir() {
        absolute.as_path()
    } else {
        absolute.parent()?
    };
    start
        .ancestors()
        .find(|dir| dir.join("soli.toml").is_file() || dir.join("config/routes.sl").is_file())
        .map(Path::to_path_buf)
}

/// The applications under `dir`, `dir` itself excluded: every directory below
/// it holding `soli.toml` or `config/routes.sl`, sorted. A walk stops at an
/// application (one inside another is part of it) and skips what
/// [`files_with_extension`] skips. `soli check examples` needs them: each
/// example is an application of its own, and its models are only known from
/// its root.
pub fn nested_app_roots(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            if path.join("soli.toml").is_file() || path.join("config/routes.sl").is_file() {
                out.push(path);
            } else {
                walk(&path, out);
            }
        }
    }
    let mut roots = Vec::new();
    walk(dir, &mut roots);
    roots.sort();
    roots
}

/// The files under `dir` with extension `ext`, recursively and sorted. Hidden
/// directories and dependency or build trees (`node_modules`, `target`…) are
/// skipped.
pub fn files_with_extension(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files(dir, ext, &mut files);
    files.sort();
    files
}

fn collect_files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_files(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// The top-level names (classes, modules, `def`s, consts, top-level
/// assignments…) declared by the `.sl` files under `dirs` of `root`, leaving
/// out `except`. A file that does not lex or parse contributes nothing: it is
/// its own error, reported when its turn comes.
pub fn top_level_names(root: &Path, dirs: &[&str], except: Option<&Path>) -> HashSet<String> {
    let mut names = HashSet::new();
    for dir in dirs {
        for path in files_with_extension(&root.join(dir), "sl") {
            if except.is_some_and(|except| except == path) {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(program) = crate::parse(&source) else {
                continue;
            };
            crate::lint::rules::scope::collect_program_names(&program.statements, &mut names);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn the_applications_under_a_directory_are_found_without_descending_into_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "shop/soli.toml", "");
        write(root, "shop/plugins/inner/soli.toml", "");
        write(root, "group/blog/config/routes.sl", "");
        write(root, "group/blog/node_modules/x/soli.toml", "");
        write(root, ".hidden/soli.toml", "");
        write(root, "loose/script.sl", "");
        assert_eq!(
            nested_app_roots(root),
            [root.join("group/blog"), root.join("shop")]
        );
    }

    #[test]
    fn the_root_is_the_nearest_directory_with_routes_or_soli_toml() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(&root, "config/routes.sl", "");
        write(&root, "app/models/post.sl", "class Post < Model\nend\n");
        assert_eq!(
            app_root(&root.join("app/models/post.sl")),
            Some(root.clone())
        );
        assert_eq!(app_root(&root.join("app")), Some(root.clone()));
        let loose = tempfile::tempdir().unwrap();
        assert_eq!(app_root(loose.path()), None);
    }

    #[test]
    fn names_come_from_every_listed_directory_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "app/jobs/mail/welcome.sl",
            "def send_welcome\n  1\nend\n",
        );
        write(
            root,
            "app/subscribers/audit.sl",
            "class AuditSubscriber\nend\n",
        );
        write(
            root,
            "app/policies/post_policy.sl",
            "class PostPolicy\nend\n",
        );
        write(root, "app/models/broken.sl", "def broken(\n");
        write(
            root,
            "app/models/node_modules/x.sl",
            "def vendored\n  1\nend\n",
        );
        let names = top_level_names(root, APP_GLOBAL_DIRS, None);
        for name in ["send_welcome", "AuditSubscriber", "PostPolicy"] {
            assert!(names.contains(name), "{name}: {names:?}");
        }
        assert!(!names.contains("vendored"));
        let except = root.join("app/policies/post_policy.sl");
        assert!(!top_level_names(root, APP_GLOBAL_DIRS, Some(&except)).contains("PostPolicy"));
    }

    #[test]
    fn soli_check_autoloads_are_the_app_global_and_support_dirs() {
        let expected: Vec<&str> = APP_GLOBAL_DIRS
            .iter()
            .chain(SUPPORT_DIRS)
            .copied()
            .collect();
        assert_eq!(crate::AUTOLOADED_DIRS, expected.as_slice());
    }
}
