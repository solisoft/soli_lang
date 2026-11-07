//! Two files declaring the same top-level name.
//!
//! The application's auto-loaded files share one global environment, and a
//! second `class Notifications` (or `def format_row`, `module Scoped`…) does
//! not reopen or merge with the first: it replaces it. Which copy survives
//! depends on load order, and nothing said a thing — a method of the first
//! class simply stopped existing, or a helper ran someone else's body. This
//! finds those collisions before anything is loaded, so `soli serve` (and the
//! Worker) refuse to boot with both locations named, and `soli lint` reports
//! them.
//!
//! View helpers (`app/helpers`) live in a namespace of their own, checked
//! apart: a helper and a model may share a name, two helpers may not.
//!
//! Top-level `def`s in `app/controllers` are left out: a function-style
//! controller's actions (`def index`, a socket's `def handle`) are captured by
//! the router file by file as each loads, so every controller may have its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::ast::stmt::{Stmt, StmtKind};

/// Directories loaded into the application's global environment.
const APPLICATION_DIRS: &[&str] = crate::app_layout::APP_GLOBAL_DIRS;

/// Directories loaded into the views' helper environment.
const HELPER_DIRS: &[&str] = crate::app_layout::HELPER_DIRS;

/// One top-level declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    /// `class`, `module`, `def`, `enum`, `interface`, `const`.
    pub kind: &'static str,
    pub file: PathBuf,
    pub line: u32,
}

/// A name declared twice in one namespace: where it was first, where again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicate {
    pub first: Declaration,
    pub again: Declaration,
}

impl Duplicate {
    /// The explanation, with both places relative to `root`.
    pub fn message(&self, root: &Path) -> String {
        // `/` on every OS: on Windows a joined path reads `app/models\post.sl`.
        let at = |d: &Declaration| {
            format!(
                "{}:{} ({} {})",
                d.file
                    .strip_prefix(root)
                    .unwrap_or(&d.file)
                    .to_string_lossy()
                    .replace('\\', "/"),
                d.line,
                d.kind,
                d.name
            )
        };
        format!(
            "`{}` is declared twice: {} and {}. The application's files share one \
             namespace, so the one loaded last would silently replace the other — \
             rename one of them",
            self.first.name,
            at(&self.first),
            at(&self.again)
        )
    }
}

/// The top-level names `statements` declare, in order — and the classes
/// nested in them, by their qualified name (`Billing::Invoice`): a module
/// may be declared again (reopened, in another file of its bounded
/// context), the classes in it may not.
pub fn declarations(statements: &[Stmt], file: &Path) -> Vec<Declaration> {
    let mut out = Vec::new();
    for stmt in statements {
        let found = match &stmt.kind {
            StmtKind::Class(decl) => {
                nested_declarations(decl, &decl.name, file, &mut out);
                Some((
                    decl.name.clone(),
                    if decl.is_module { "module" } else { "class" },
                ))
            }
            StmtKind::Function(decl) => Some((decl.name.clone(), "def")),
            StmtKind::Enum(decl) => Some((decl.name.clone(), "enum")),
            StmtKind::Interface(decl) => Some((decl.name.clone(), "interface")),
            StmtKind::Const { name, .. } => Some((name.clone(), "const")),
            StmtKind::Export(inner) => {
                out.extend(declarations(std::slice::from_ref(inner), file));
                None
            }
            _ => None,
        };
        if let Some((name, kind)) = found {
            out.push(Declaration {
                name,
                kind,
                file: file.to_path_buf(),
                line: stmt.span.line,
            });
        }
    }
    out
}

fn nested_declarations(
    decl: &crate::ast::stmt::ClassDecl,
    qualifier: &str,
    file: &Path,
    out: &mut Vec<Declaration>,
) {
    for nested in &decl.nested_classes {
        let name = format!("{qualifier}::{}", nested.name);
        nested_declarations(nested, &name, file, out);
        out.push(Declaration {
            name,
            kind: if nested.is_module { "module" } else { "class" },
            file: file.to_path_buf(),
            line: nested.span.line,
        });
    }
}

/// Every name declared more than once among `declarations`, in the order the
/// second declarations appear. A module declared twice is reopened, not
/// replaced, so that pair is not one.
pub fn duplicates(declarations: Vec<Declaration>) -> Vec<Duplicate> {
    let mut seen: BTreeMap<String, Declaration> = BTreeMap::new();
    let mut found = Vec::new();
    for declaration in declarations {
        match seen.get(&declaration.name) {
            Some(first) if first.kind == "module" && declaration.kind == "module" => {}
            Some(first) => found.push(Duplicate {
                first: first.clone(),
                again: declaration,
            }),
            None => {
                seen.insert(declaration.name.clone(), declaration);
            }
        }
    }
    found
}

/// The `.sl` files under `dir`, recursively, sorted — through `platform::fs`,
/// so the Worker's in-memory app tree is read too.
fn sl_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = crate::platform::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if crate::platform::fs::is_dir(&path) {
            files.extend(sl_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "sl") {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// The declarations of the files under `dirs` of `root`. A file that does not
/// parse, or a protected bundle's serialized AST, is left out: its own load
/// reports it. Controllers' top-level `def`s are their actions (see the module
/// header) and are not counted.
fn declarations_under(root: &Path, dirs: &[&str]) -> Vec<Declaration> {
    let mut out = Vec::new();
    for dir in dirs {
        for path in sl_files(&root.join(dir)) {
            let Ok(bytes) = crate::platform::fs::read(&path) else {
                continue;
            };
            if crate::bundle::is_ast_blob(&bytes) {
                continue;
            }
            let source = String::from_utf8_lossy(&bytes);
            let Ok(tokens) = crate::lexer::Scanner::new(&source).scan_tokens() else {
                continue;
            };
            let Ok(program) = crate::parser::Parser::new(tokens).parse() else {
                continue;
            };
            let actions = *dir == "app/controllers";
            out.extend(
                declarations(&program.statements, &path)
                    .into_iter()
                    .filter(|d| !(actions && d.kind == "def")),
            );
        }
    }
    out
}

/// Every name the application at `root` declares twice: in its global
/// environment, or among its view helpers.
pub fn find_in_app(root: &Path) -> Vec<Duplicate> {
    let mut found = duplicates(declarations_under(root, APPLICATION_DIRS));
    found.extend(duplicates(declarations_under(root, HELPER_DIRS)));
    found
}

/// [`find_in_app`] as a boot check: `Err` lists every collision.
pub fn check_app(root: &Path) -> Result<(), String> {
    let found = find_in_app(root);
    if found.is_empty() {
        return Ok(());
    }
    Err(format!(
        "duplicate top-level names:\n  - {}",
        found
            .iter()
            .map(|d| d.message(root))
            .collect::<Vec<_>>()
            .join("\n  - ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str, file: &str) -> Vec<Declaration> {
        let tokens = crate::lexer::Scanner::new(source).scan_tokens().unwrap();
        let program = crate::parser::Parser::new(tokens).parse().unwrap();
        declarations(&program.statements, Path::new(file))
    }

    #[test]
    fn top_level_declarations_are_named_with_their_kind_and_line() {
        let found = parse(
            "class A\nend\n\nmodule M\nend\ndef helper\n  1\nend\nconst MAX = 3\nx = 1\n",
            "a.sl",
        );
        let kinds: Vec<_> = found
            .iter()
            .map(|d| (d.name.as_str(), d.kind, d.line))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("A", "class", 1),
                ("M", "module", 4),
                ("helper", "def", 6),
                ("MAX", "const", 9),
            ]
        );
    }

    #[test]
    fn a_name_declared_in_two_files_is_a_duplicate() {
        let mut all = parse("class Notifications\nend\n", "app/models/notifications.sl");
        all.extend(parse("def helper\n  1\nend\n", "app/models/a.sl"));
        all.extend(parse(
            "\n\nclass Notifications\nend\n",
            "app/services/notifications.sl",
        ));
        let found = duplicates(all);
        assert_eq!(found.len(), 1);
        let message = found[0].message(Path::new(""));
        assert!(message.contains("app/models/notifications.sl:1 (class Notifications)"));
        assert!(message.contains("app/services/notifications.sl:3 (class Notifications)"));
    }

    #[test]
    fn a_module_reopened_is_not_a_duplicate_but_its_classes_are() {
        let mut all = parse(
            "module Billing\n  class Invoice\n  end\nend\n",
            "app/models/billing/invoice.sl",
        );
        all.extend(parse(
            "module Billing\n  class Customer\n  end\nend\n",
            "app/models/billing/customer.sl",
        ));
        all.extend(parse(
            "module Shipping\n  class Invoice\n  end\nend\n",
            "app/models/shipping.sl",
        ));
        assert!(duplicates(all.clone()).is_empty());

        all.extend(parse(
            "module Billing\n  class Invoice\n  end\nend\n",
            "app/services/billing.sl",
        ));
        let found = duplicates(all);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].again.name, "Billing::Invoice");
    }

    #[test]
    fn a_class_and_a_function_of_the_same_name_collide_too() {
        let mut all = parse("class Report\nend\n", "app/models/report.sl");
        all.extend(parse("def Report\n  1\nend\n", "app/services/x.sl"));
        assert_eq!(duplicates(all).len(), 1);
    }

    #[test]
    fn an_app_is_checked_per_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let write = |path: &str, body: &str| {
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, body).unwrap();
        };
        write("app/models/post.sl", "class Post < Model\nend\n");
        write("app/helpers/format.sl", "def label(x)\n  x\nend\n");
        // A model and a helper may share a name: different environments.
        write("app/services/label.sl", "def label(x)\n  x\nend\n");
        assert!(check_app(dir.path()).is_ok());

        // Function-style controllers each have their own `def index`.
        write(
            "app/controllers/blog_controller.sl",
            "def index(req)\n  1\nend\n",
        );
        write(
            "app/controllers/docs_controller.sl",
            "def index(req)\n  2\nend\n",
        );
        assert!(check_app(dir.path()).is_ok());

        write(
            "app/controllers/posts_controller.sl",
            "class Post\nend\nclass PostsController < Controller\nend\n",
        );
        write("app/helpers/more.sl", "def label(y)\n  y\nend\n");
        let err = check_app(dir.path()).unwrap_err();
        assert!(err.contains("`Post` is declared twice"), "{err}");
        assert!(err.contains("`label` is declared twice"), "{err}");
        assert!(err.contains("app/helpers/format.sl:1"), "{err}");
    }
}
