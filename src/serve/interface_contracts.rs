//! Classes that `implements` an interface without one of its methods.
//!
//! The type checker refuses such a class, but it runs under `soli run`,
//! `soli check` and the test runner — never on an application's files at
//! boot. So in an app the contract held nothing: a class missing a method of
//! its interface booted, and the first call to it raised in production. This
//! reads the application's files before anything is loaded, so `soli serve`
//! (and the Worker) refuse to boot with the class and the method named, a
//! hot reload warns, and `soli lint` reports it.
//!
//! The question is presence and arity, not types: a method satisfies
//! `fn find(id: Int) -> User` when it is an instance method named `find` that
//! can be called with one argument. Methods come from the class, its
//! superclasses and the modules they `include`. When that chain leaves the
//! application — `< Model`, `< Controller`, a module from a library — a method
//! not found may be inherited from there, so it is not reported. An interface
//! declared outside the application's files is not checked either.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::stmt::{ClassDecl, InterfaceDecl, MethodDecl, Parameter, Program, StmtKind};

/// A class missing a method of an interface it implements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broken {
    pub class: String,
    pub interface: String,
    pub method: String,
    /// The interface's parameter count.
    pub arity: usize,
    /// `Some(n..=m)` when a method of that name exists but cannot take
    /// `arity` arguments; `None` when there is no such method.
    pub found_arity: Option<(usize, usize)>,
    pub file: PathBuf,
    pub line: u32,
}

impl Broken {
    /// The explanation, with the class's place relative to `root`.
    pub fn message(&self, root: &Path) -> String {
        let place = format!(
            "{}:{}",
            self.file
                .strip_prefix(root)
                .unwrap_or(&self.file)
                .to_string_lossy()
                .replace('\\', "/"),
            self.line
        );
        let arguments = |n: usize| {
            if n == 1 {
                "1 argument".to_string()
            } else {
                format!("{n} arguments")
            }
        };
        match self.found_arity {
            None => format!(
                "class `{}` ({place}) implements `{}` but has no method `{}` ({}): \
                 the first call to it would raise",
                self.class,
                self.interface,
                self.method,
                arguments(self.arity)
            ),
            Some((min, max)) => format!(
                "class `{}` ({place}) implements `{}`, whose `{}` takes {}, but its own \
                 `{}` takes {}",
                self.class,
                self.interface,
                self.method,
                arguments(self.arity),
                self.method,
                if min == max {
                    arguments(min)
                } else {
                    format!("{min} to {max} arguments")
                }
            ),
        }
    }
}

/// A class of the application, by qualified name, and the file declaring it.
type Declared<'a> = (String, &'a ClassDecl, &'a Path);

/// What [`collect`] finds in the application's files.
struct Collected<'a> {
    interfaces: HashMap<String, &'a InterfaceDecl>,
    /// Classes and modules by qualified name.
    units: HashMap<String, &'a ClassDecl>,
    /// Every class, in file order.
    order: Vec<Declared<'a>>,
}

/// Parameters a call must pass, and may pass. A block parameter is not one.
fn arity(params: &[Parameter]) -> (usize, usize) {
    let positional: Vec<&Parameter> = params.iter().filter(|p| !p.is_block_param).collect();
    let required = positional
        .iter()
        .filter(|p| p.default_value.is_none())
        .count();
    (required, positional.len())
}

/// Every interface and class declared in `files`, top-level or nested in a
/// module (`Billing::Invoice`).
fn collect<'a>(files: &'a [(PathBuf, Program, bool)]) -> Collected<'a> {
    let mut found = Collected {
        interfaces: HashMap::new(),
        units: HashMap::new(),
        order: Vec::new(),
    };

    fn add_class<'a>(decl: &'a ClassDecl, name: String, file: &'a Path, found: &mut Collected<'a>) {
        for nested in &decl.nested_classes {
            add_class(nested, format!("{name}::{}", nested.name), file, found);
        }
        found.order.push((name.clone(), decl, file));
        // A module reopened in another file: the first declaration stands in
        // for the name (its methods are what an `include` most often needs).
        found.units.entry(name).or_insert(decl);
    }

    for (path, program, _) in files {
        for stmt in &program.statements {
            let kind = match &stmt.kind {
                StmtKind::Export(inner) => &inner.kind,
                other => other,
            };
            match kind {
                StmtKind::Interface(decl) => {
                    found.interfaces.insert(decl.name.clone(), &**decl);
                }
                StmtKind::Class(decl) => add_class(decl, decl.name.clone(), path, &mut found),
                _ => {}
            }
        }
    }
    found
}

/// The instance methods named `name` reachable from `start` — its own, its
/// included modules', then its superclass's — and whether the chain stayed
/// inside the application (so a method not found really is missing).
fn find_method<'a>(
    start: &'a ClassDecl,
    name: &str,
    units: &HashMap<String, &'a ClassDecl>,
) -> (Option<&'a MethodDecl>, bool) {
    let mut current = Some(start);
    let mut seen = 0;
    while let Some(decl) = current {
        seen += 1;
        if seen > 64 {
            // A cycle (`class A < B`, `class B < A`): its load reports it.
            return (None, false);
        }
        if let Some(method) = decl.methods.iter().find(|m| m.name == name && !m.is_static) {
            return (Some(method), true);
        }
        for module in &decl.includes {
            match units.get(module.as_str()) {
                Some(unit) => {
                    if let Some(method) =
                        unit.methods.iter().find(|m| m.name == name && !m.is_static)
                    {
                        return (Some(method), true);
                    }
                }
                None => return (None, false),
            }
        }
        current = match &decl.superclass {
            None => None,
            Some(parent) => match units.get(parent.as_str()) {
                Some(unit) => Some(*unit),
                None => return (None, false),
            },
        };
    }
    (None, true)
}

/// Every class in `files` missing a method of an interface it implements,
/// in file order.
pub fn find_in_files(files: &[(PathBuf, Program, bool)]) -> Vec<Broken> {
    let Collected {
        interfaces,
        units,
        order,
    } = collect(files);
    let mut found = Vec::new();
    for (class_name, decl, file) in order {
        if decl.is_module {
            continue;
        }
        for interface_name in &decl.interfaces {
            let Some(interface) = interfaces.get(interface_name.as_str()) else {
                continue;
            };
            for wanted in &interface.methods {
                let (wanted_arity, _) = arity(&wanted.params);
                let (method, closed) = find_method(decl, &wanted.name, &units);
                let found_arity = match method {
                    Some(method) => {
                        let (min, max) = arity(&method.params);
                        if (min..=max).contains(&wanted_arity) {
                            continue;
                        }
                        Some((min, max))
                    }
                    None if closed => None,
                    None => continue,
                };
                found.push(Broken {
                    class: class_name.clone(),
                    interface: interface_name.clone(),
                    method: wanted.name.clone(),
                    arity: wanted_arity,
                    found_arity,
                    file: file.to_path_buf(),
                    line: decl.span.line,
                });
            }
        }
    }
    found
}

/// [`find_in_files`] over the application at `root`.
pub fn find_in_app(root: &Path) -> Vec<Broken> {
    find_in_files(&crate::serve::global_names::parsed_under(
        root,
        crate::app_layout::APP_GLOBAL_DIRS,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(sources: &[(&str, &str)]) -> Vec<(PathBuf, Program, bool)> {
        sources
            .iter()
            .map(|(path, source)| {
                let tokens = crate::lexer::Scanner::new(source).scan_tokens().unwrap();
                let program = crate::parser::Parser::new(tokens).parse().unwrap();
                (PathBuf::from(path), program, false)
            })
            .collect()
    }

    const REPO: (&str, &str) = (
        "app/services/repository.sl",
        "interface Repository {\n  fn find(id: Int) -> String\n  fn count() -> Int\n}\n",
    );

    #[test]
    fn a_complete_class_passes() {
        let all = files(&[
            REPO,
            (
                "app/services/memory_repo.sl",
                "class MemoryRepo implements Repository\n  def find(id)\n    id\n  end\n  \
                 def count\n    0\n  end\nend\n",
            ),
        ]);
        assert!(find_in_files(&all).is_empty());
    }

    #[test]
    fn a_missing_method_is_named_with_the_class_and_its_place() {
        let all = files(&[
            REPO,
            (
                "app/services/memory_repo.sl",
                "\nclass MemoryRepo implements Repository\n  def find(id)\n    id\n  end\nend\n",
            ),
        ]);
        let found = find_in_files(&all);
        assert_eq!(found.len(), 1);
        let message = found[0].message(Path::new(""));
        assert!(
            message.contains("`MemoryRepo` (app/services/memory_repo.sl:2)"),
            "{message}"
        );
        assert!(
            message.contains("no method `count` (0 arguments)"),
            "{message}"
        );
    }

    #[test]
    fn a_wrong_arity_is_reported_and_defaults_count_as_optional() {
        let all = files(&[
            REPO,
            (
                "app/services/repos.sl",
                "class Wrong implements Repository\n  def find\n    1\n  end\n  def count\n    0\n  end\nend\n\
                 class Defaults implements Repository\n  def find(id, scope = nil)\n    1\n  end\n  \
                 def count(&block)\n    0\n  end\nend\n",
            ),
        ]);
        let found = find_in_files(&all);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].class, "Wrong");
        assert_eq!(found[0].found_arity, Some((0, 0)));
        assert!(found[0]
            .message(Path::new(""))
            .contains("whose `find` takes 1 argument, but its own `find` takes 0 arguments"));
    }

    #[test]
    fn methods_come_from_superclasses_and_included_modules() {
        let all = files(&[
            REPO,
            (
                "app/services/repos.sl",
                "module Counting\n  def count\n    0\n  end\nend\n\
                 class Base\n  def find(id)\n    id\n  end\nend\n\
                 class Child < Base implements Repository\n  include Counting\nend\n",
            ),
        ]);
        assert!(find_in_files(&all).is_empty(), "{:?}", find_in_files(&all));
    }

    #[test]
    fn a_chain_that_leaves_the_app_is_not_judged() {
        // `count` may come from Model; an interface from elsewhere is unknown.
        let all = files(&[
            REPO,
            (
                "app/models/user.sl",
                "class User < Model implements Repository\n  def find(id)\n    id\n  end\nend\n\
                 class Other implements Exportable\nend\n",
            ),
        ]);
        assert!(find_in_files(&all).is_empty());
    }

    #[test]
    fn a_static_method_does_not_satisfy_an_instance_contract() {
        let all = files(&[
            REPO,
            (
                "app/services/repo.sl",
                "class StaticRepo implements Repository\n  static def find(id)\n    id\n  end\n  \
                 def count\n    0\n  end\nend\n",
            ),
        ]);
        assert_eq!(find_in_files(&all).len(), 1);
    }

    #[test]
    fn the_boot_check_refuses_an_app_with_a_broken_contract() {
        let dir = tempfile::tempdir().unwrap();
        let write = |path: &str, body: &str| {
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, body).unwrap();
        };
        write(REPO.0, REPO.1);
        write(
            "app/services/memory_repo.sl",
            "class MemoryRepo implements Repository\n  def find(id)\n    id\n  end\nend\n",
        );
        let err = crate::serve::global_names::check_app(dir.path()).unwrap_err();
        assert!(err.starts_with("classes missing a method"), "{err}");
        assert!(err.contains("no method `count`"), "{err}");
        assert_eq!(find_in_app(dir.path()).len(), 1);
    }
}
