//! Multi-language code-structure extraction for `soli graph`.
//!
//! Given a file's language and source, produce language-agnostic
//! [`Def`]/[`EdgeRef`] records (classes, methods, functions, imports, …). The
//! `soli` crate maps these onto its graph `Node`/`Edge` schema, so all of the
//! storage / embedding / query machinery is reused unchanged.
//!
//! Parsing is tree-sitter based. Each language contributes a grammar plus a
//! small set of tree-sitter queries; call resolution is deliberately left to
//! the caller (best-effort name matching), so this crate only reports *what is
//! defined* and *what is referenced by name*.

mod lang;

pub use lang::Language;

/// A definition extracted from a source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Def {
    /// `class`, `module`, `method`, `function`, `interface`, `enum`, `constant`.
    pub kind: String,
    /// Short name (`authenticate`).
    pub name: String,
    /// Qualified name where the grammar makes it cheap (`User#authenticate`,
    /// `User.authenticate`); otherwise equal to `name`.
    pub qualified_name: String,
    /// 1-based start line.
    pub line: u32,
    /// Readable signature line, when available.
    pub signature: String,
    /// Superclass / extended type, for `inherits` edges.
    pub superclass: Option<String>,
    /// Byte range of the definition in the source (for the embedded snippet).
    pub start_byte: usize,
    pub end_byte: usize,
}

/// A by-name reference extracted from a source file. The caller resolves the
/// target against the project's definitions.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeRef {
    /// `imports`, `inherits`, `calls`, `implements`.
    pub kind: String,
    /// The referenced name/path as written in source.
    pub target: String,
    /// Qualified name of the enclosing definition (empty at file scope), so the
    /// caller can attribute a `calls` edge to the right method/function.
    pub from_qualified: String,
    pub line: u32,
}

/// The result of extracting one file.
#[derive(Debug, Clone, Default)]
pub struct Extraction {
    pub defs: Vec<Def>,
    pub edges: Vec<EdgeRef>,
}

/// Map a file extension to a supported [`Language`], or `None` for extensions
/// we only chunk-embed (templates, config, unknown).
pub fn language_for_extension(ext: &str) -> Option<Language> {
    lang::language_for_extension(ext)
}

/// Extract definitions and by-name edges from one source file.
pub fn extract(language: Language, source: &str) -> Extraction {
    lang::extract(language, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs_extract(ext: &str, src: &str) -> Extraction {
        extract(language_for_extension(ext).expect("known ext"), src)
    }

    fn has_def(out: &Extraction, kind: &str, qn: &str) -> bool {
        out.defs
            .iter()
            .any(|d| d.kind == kind && d.qualified_name == qn)
    }

    fn has_edge(out: &Extraction, kind: &str, target: &str) -> bool {
        out.edges
            .iter()
            .any(|e| e.kind == kind && e.target == target)
    }

    #[test]
    fn ruby_class_method_and_inherits() {
        let out = defs_extract(
            "rb",
            "class User < ApplicationRecord\n  def authenticate(pw)\n  end\nend\n",
        );
        assert!(has_def(&out, "class", "User"));
        assert!(has_def(&out, "method", "User#authenticate"));
        assert!(has_edge(&out, "inherits", "ApplicationRecord"));
    }

    #[test]
    fn python_class_method_and_inherits() {
        let out = defs_extract(
            "py",
            "import os\nclass User(Base):\n    def authenticate(self, pw):\n        pass\n",
        );
        assert!(has_def(&out, "class", "User"));
        assert!(has_def(&out, "method", "User.authenticate"));
        assert!(has_edge(&out, "inherits", "Base"));
        assert!(has_edge(&out, "imports", "os"));
    }

    #[test]
    fn javascript_class_method_function_import() {
        let out = defs_extract(
            "js",
            "import { x } from './util';\nclass User extends Base {\n  authenticate(pw) {}\n}\nfunction helper() {}\n",
        );
        assert!(has_def(&out, "class", "User"));
        assert!(has_def(&out, "method", "User#authenticate"));
        assert!(has_def(&out, "function", "helper"));
        assert!(has_edge(&out, "inherits", "Base"));
        assert!(has_edge(&out, "imports", "./util"));
    }

    #[test]
    fn typescript_interface_and_class() {
        let out = defs_extract(
            "ts",
            "interface Named { name: string }\nclass User implements Named {\n  greet(): void {}\n}\n",
        );
        assert!(has_def(&out, "interface", "Named"));
        assert!(has_def(&out, "class", "User"));
        assert!(has_def(&out, "method", "User#greet"));
    }

    #[test]
    fn rust_struct_impl_trait() {
        let out = defs_extract(
            "rs",
            "use std::io;\nstruct User;\ntrait Auth { fn ok(&self); }\nimpl Auth for User {\n  fn ok(&self) {}\n}\n",
        );
        assert!(has_def(&out, "class", "User"));
        assert!(has_def(&out, "interface", "Auth"));
        assert!(has_def(&out, "method", "User::ok"));
        assert!(has_edge(&out, "implements", "Auth"));
        assert!(has_edge(&out, "imports", "std::io"));
    }

    #[test]
    fn csharp_class_method_inherits() {
        let out = defs_extract(
            "cs",
            "using System;\nclass User : Base, IAuth {\n  public void Login() {}\n}\n",
        );
        assert!(has_def(&out, "class", "User"));
        assert!(has_def(&out, "method", "User.Login"));
        assert!(has_edge(&out, "inherits", "Base"));
        assert!(has_edge(&out, "implements", "IAuth"));
        assert!(has_edge(&out, "imports", "System"));
    }
}
