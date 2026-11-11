//! What the language server knows about the whole application, not only the
//! open file: the top-level declarations of every auto-loaded file, and the
//! members of every class, each with its owner and its class's superclass.
//!
//! The index is built once per application root, lazily, on the first request
//! that needs it; afterwards only the file that changed is parsed again (an
//! open buffer on each change, a file on disk on save or on a watched-file
//! event). A buffer that does not parse keeps its last good entry, so a
//! half-typed line does not make its classes disappear.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tower_lsp_server::ls_types::Range;

use crate::app_layout::{APP_GLOBAL_DIRS, HELPER_DIRS};
use crate::ast::expr::{Argument, ExprKind};
use crate::ast::stmt::{ClassDecl, Stmt, StmtKind};
use crate::lexer::TokenKind;

use super::util::{doc_comment_above, range_of};

/// More files than this and the rest of the tree is left out: the index is
/// built on the server's single thread, and a monorepo under `app/` would
/// stall typing.
const MAX_FILES: usize = 5_000;

/// Which environment a file's top-level names live in at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    /// The application's shared global environment (models, controllers…).
    Global,
    /// The views' helper environment (`app/helpers`).
    Helper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexedKind {
    Class,
    Module,
    Function,
    Constant,
    /// An instance method.
    Method,
    /// A `static def`.
    StaticMethod,
    /// A declared field (`title: String`).
    Field,
    /// A model scope: `scope("published", fn() { … })`, called on the class.
    Scope,
    /// A model relation: `has_many("comments")`, read on an instance.
    Relation,
}

impl IndexedKind {
    pub fn label(self) -> &'static str {
        match self {
            IndexedKind::Class => "class",
            IndexedKind::Module => "module",
            IndexedKind::Function => "function",
            IndexedKind::Constant => "constant",
            IndexedKind::Method => "method",
            IndexedKind::StaticMethod => "static method",
            IndexedKind::Field => "field",
            IndexedKind::Scope => "scope",
            IndexedKind::Relation => "relation",
        }
    }

    /// Reached on the class itself (`Post.published`) rather than on an
    /// instance (`post.title`).
    pub fn is_static_member(self) -> bool {
        matches!(self, IndexedKind::StaticMethod | IndexedKind::Scope)
    }
}

#[derive(Debug, Clone)]
pub struct IndexedSymbol {
    pub name: String,
    pub kind: IndexedKind,
    /// The class or module a member, or a nested class, belongs to.
    pub owner: Option<String>,
    /// A class's superclass (`Post < Model` → `Model`).
    pub superclass: Option<String>,
    pub path: PathBuf,
    /// The name itself, in the file.
    pub range: Range,
    /// `def title(width: Int) -> String`, for functions and methods.
    pub signature: Option<String>,
    /// The `#` comment lines right above the declaration, markers stripped.
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FileIndex {
    pub namespace: Namespace,
    pub symbols: Vec<IndexedSymbol>,
    /// Every top-level name the file binds, as the linter counts them
    /// (`lint::rules::scope::collect_program_names`).
    pub program_names: HashSet<String>,
    /// A controller: its top-level `def`s are actions, not globals.
    pub controller: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectIndex {
    pub root: PathBuf,
    pub files: HashMap<PathBuf, FileIndex>,
    /// Not an application: a directory of loose scripts. Nothing is scanned;
    /// only the open buffers in it are indexed, each on its own.
    pub loose: bool,
}

impl ProjectIndex {
    /// Index every `.sl` file in the application's auto-loaded and helper
    /// directories under `root`.
    pub fn scan(root: &Path) -> Self {
        let mut index = ProjectIndex {
            root: root.to_path_buf(),
            files: HashMap::new(),
            loose: false,
        };
        'dirs: for dir in APP_GLOBAL_DIRS.iter().chain(HELPER_DIRS) {
            for path in crate::app_layout::files_with_extension(&root.join(dir), "sl") {
                if index.files.len() >= MAX_FILES {
                    log::warn!("soli lsp: more than {MAX_FILES} files under app/, the rest are not indexed");
                    break 'dirs;
                }
                index.refresh_from_disk(&path);
            }
        }
        index
    }

    /// The index of the loose scripts in `dir`, empty until one is opened.
    pub fn loose(dir: &Path) -> Self {
        ProjectIndex {
            root: dir.to_path_buf(),
            files: HashMap::new(),
            loose: true,
        }
    }

    /// Every file a project-wide reference or rename reads: the `.sl` code and
    /// `.slv` views under `app/`, plus `config/` and `tests/`. A directory of
    /// loose scripts has no project: only its indexed (open) scripts.
    pub fn source_files(&self) -> Vec<PathBuf> {
        if self.loose {
            let mut files: Vec<PathBuf> = self.files.keys().cloned().collect();
            files.sort();
            return files;
        }
        let mut files = Vec::new();
        for (dir, ext) in [
            ("app", "sl"),
            ("app", "slv"),
            ("config", "sl"),
            ("tests", "sl"),
        ] {
            files.extend(crate::app_layout::files_with_extension(
                &self.root.join(dir),
                ext,
            ));
        }
        files.truncate(MAX_FILES);
        files
    }

    /// The namespace `path` belongs to, or `None` when it is not an
    /// auto-loaded application file (or, loose, a script in the directory).
    pub fn namespace_of(&self, path: &Path) -> Option<Namespace> {
        let relative = path.strip_prefix(&self.root).ok()?;
        if !path.extension().is_some_and(|ext| ext == "sl") {
            return None;
        }
        if self.loose {
            return Some(Namespace::Global);
        }
        let under = |dirs: &[&str]| dirs.iter().any(|dir| relative.starts_with(dir));
        if under(HELPER_DIRS) {
            Some(Namespace::Helper)
        } else if under(APP_GLOBAL_DIRS) {
            Some(Namespace::Global)
        } else {
            None
        }
    }

    /// Re-index `path` from `source` (an open buffer). A source that does not
    /// parse keeps the file's previous entry.
    pub fn update_file(&mut self, path: &Path, source: &str) {
        let Some(namespace) = self.namespace_of(path) else {
            return;
        };
        let controller = path
            .strip_prefix(&self.root)
            .is_ok_and(|relative| relative.starts_with("app/controllers"));
        if let Some(file) = index_source(path, source, namespace, controller) {
            self.files.insert(path.to_path_buf(), file);
        }
    }

    /// Re-index `path` from disk; a file that is gone leaves the index.
    pub fn refresh_from_disk(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(source) => self.update_file(path, &source),
            Err(_) => self.remove_file(path),
        }
    }

    pub fn remove_file(&mut self, path: &Path) {
        self.files.remove(path);
    }

    /// The names the auto-loaded neighbours of `path` declare, for the
    /// linter's `smell/undefined-local` (see `Linter::with_known_names`).
    /// Helpers are included, as the linter's own disk walk does.
    pub fn known_names(&self, path: &Path) -> HashSet<String> {
        self.files
            .iter()
            .filter(|(file, _)| file.as_path() != path)
            .flat_map(|(_, index)| index.program_names.iter().cloned())
            .collect()
    }

    fn symbols(&self) -> impl Iterator<Item = (&FileIndex, &IndexedSymbol)> {
        self.files
            .values()
            .flat_map(|file| file.symbols.iter().map(move |symbol| (file, symbol)))
    }

    /// The top-level declarations named `name` that code in `from` can reach.
    pub fn top_level(&self, name: &str, from: Option<Namespace>) -> Vec<&IndexedSymbol> {
        let mut found: Vec<&IndexedSymbol> = self
            .reachable_top_level(from)
            .into_iter()
            .filter(|symbol| symbol.name == name)
            .collect();
        sort_symbols(&mut found);
        found
    }

    /// Every top-level declaration code in `from` can reach: app-global names
    /// from an app file, helpers too from a helper, a view or any other file.
    /// A controller's top-level `def`s are its actions, not globals.
    pub fn reachable_top_level(&self, from: Option<Namespace>) -> Vec<&IndexedSymbol> {
        self.symbols()
            .filter(|(file, symbol)| {
                let declaration = match symbol.kind {
                    IndexedKind::Class | IndexedKind::Module => true,
                    IndexedKind::Function | IndexedKind::Constant => symbol.owner.is_none(),
                    _ => false,
                };
                declaration
                    && !(file.controller && symbol.kind == IndexedKind::Function)
                    && match file.namespace {
                        Namespace::Global => true,
                        Namespace::Helper => from != Some(Namespace::Global),
                    }
            })
            .map(|(_, symbol)| symbol)
            .collect()
    }

    /// The class or module called `name`.
    pub fn class(&self, name: &str) -> Option<&IndexedSymbol> {
        let mut classes: Vec<&IndexedSymbol> = self
            .symbols()
            .map(|(_, symbol)| symbol)
            .filter(|s| {
                s.name == name && matches!(s.kind, IndexedKind::Class | IndexedKind::Module)
            })
            .collect();
        sort_symbols(&mut classes);
        classes.into_iter().next()
    }

    /// `class` and its ancestors, nearest first, as far as the index knows
    /// them; the last element is the first ancestor it does not declare
    /// (`Model`, `Controller`…), if any.
    pub fn ancestry(&self, class: &str) -> Vec<String> {
        let mut chain = vec![class.to_string()];
        let mut current = class.to_string();
        while let Some(parent) = self.class(&current).and_then(|c| c.superclass.clone()) {
            if chain.contains(&parent) {
                break;
            }
            chain.push(parent.clone());
            current = parent;
        }
        chain
    }

    /// Whether `class` is a model: `Model` is among its ancestors.
    pub fn is_model(&self, class: &str) -> bool {
        self.ancestry(class).iter().any(|name| name == "Model")
    }

    /// The members of `class` and its project ancestors: the static side
    /// (`static def`s, scopes) or the instance side (methods, fields,
    /// relations). A member a subclass redefines is listed once, the
    /// subclass's.
    pub fn members(&self, class: &str, static_side: bool) -> Vec<&IndexedSymbol> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for owner in self.ancestry(class) {
            let mut own: Vec<&IndexedSymbol> = self
                .symbols()
                .map(|(_, symbol)| symbol)
                .filter(|s| {
                    s.owner.as_deref() == Some(owner.as_str())
                        && !matches!(s.kind, IndexedKind::Class | IndexedKind::Module)
                        && s.kind.is_static_member() == static_side
                })
                .collect();
            sort_symbols(&mut own);
            for symbol in own {
                if seen.insert(symbol.name.clone()) {
                    out.push(symbol);
                }
            }
        }
        out
    }

    /// Every member of `class` and its ancestors named `name`, either side.
    pub fn member(&self, class: &str, name: &str) -> Vec<&IndexedSymbol> {
        let mut out: Vec<&IndexedSymbol> = Vec::new();
        for static_side in [true, false] {
            out.extend(
                self.members(class, static_side)
                    .into_iter()
                    .filter(|s| s.name == name),
            );
        }
        out
    }

    /// Every method or scope in the project called `name`, any class.
    pub fn members_named(&self, name: &str) -> Vec<&IndexedSymbol> {
        let mut out: Vec<&IndexedSymbol> = self
            .symbols()
            .map(|(_, symbol)| symbol)
            .filter(|s| {
                s.name == name
                    && s.owner.is_some()
                    && !matches!(s.kind, IndexedKind::Class | IndexedKind::Module)
            })
            .collect();
        sort_symbols(&mut out);
        out
    }

    /// The distinct method names declared anywhere in the project, sorted: the
    /// completion offered when a receiver cannot be resolved.
    pub fn all_method_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .symbols()
            .map(|(_, symbol)| symbol)
            .filter(|s| {
                matches!(
                    s.kind,
                    IndexedKind::Method | IndexedKind::StaticMethod | IndexedKind::Scope
                )
            })
            .map(|s| s.name.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        names.sort();
        names
    }
}

fn sort_symbols(symbols: &mut [&IndexedSymbol]) {
    symbols.sort_by(|a, b| {
        (&a.path, a.range.start.line, a.range.start.character).cmp(&(
            &b.path,
            b.range.start.line,
            b.range.start.character,
        ))
    });
}

/// Index one file's source, or `None` when it does not lex or parse.
pub fn index_source(
    path: &Path,
    source: &str,
    namespace: Namespace,
    controller: bool,
) -> Option<FileIndex> {
    let tokens = crate::lexer::Scanner::new(source).scan_tokens().ok()?;
    let identifiers = identifier_offsets(&tokens);
    let program = crate::parser::Parser::new(tokens).parse().ok()?;
    let mut program_names = HashSet::new();
    crate::lint::rules::scope::collect_program_names(&program.statements, &mut program_names);
    let mut indexer = Indexer {
        path,
        source,
        identifiers,
        symbols: Vec::new(),
    };
    indexer.statements(&program.statements, None);
    Some(FileIndex {
        namespace,
        symbols: indexer.symbols,
        program_names,
        controller,
    })
}

/// Byte offsets of every identifier token, by spelling.
fn identifier_offsets(tokens: &[crate::lexer::Token]) -> HashMap<String, Vec<(usize, usize)>> {
    let mut offsets: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for token in tokens {
        if let TokenKind::Identifier(name) = &token.kind {
            offsets
                .entry(name.clone())
                .or_default()
                .push((token.span.start as usize, token.span.end as usize));
        }
    }
    offsets
}

struct Indexer<'a> {
    path: &'a Path,
    source: &'a str,
    identifiers: HashMap<String, Vec<(usize, usize)>>,
    symbols: Vec<IndexedSymbol>,
}

impl Indexer<'_> {
    /// The range of `name` itself, the first time it is spelled at or after
    /// byte `from` (a declaration's start); the declaration's start if the
    /// lexer saw no such identifier (a keyword-like name).
    fn name_range(&self, name: &str, from: usize) -> Range {
        let (start, end) = self
            .identifiers
            .get(name)
            .and_then(|spans| spans.iter().find(|(start, _)| *start >= from).copied())
            .unwrap_or((from, from));
        range_of(self.source, start, end)
    }

    fn push(
        &mut self,
        name: &str,
        kind: IndexedKind,
        owner: Option<&str>,
        from: usize,
        signature: Option<String>,
    ) {
        let range = self.name_range(name, from);
        self.symbols.push(IndexedSymbol {
            name: name.to_string(),
            kind,
            owner: owner.map(str::to_string),
            superclass: None,
            path: self.path.to_path_buf(),
            doc: doc_comment_above(self.source, range.start.line),
            range,
            signature,
        });
    }

    fn statements(&mut self, statements: &[Stmt], owner: Option<&str>) {
        for stmt in statements {
            let from = stmt.span.start_usize();
            match &stmt.kind {
                StmtKind::Class(decl) => self.class(decl, owner),
                StmtKind::Function(decl) => {
                    let signature =
                        signature("def", &decl.name, &decl.params, decl.return_type.as_ref());
                    self.push(
                        &decl.name,
                        IndexedKind::Function,
                        owner,
                        from,
                        Some(signature),
                    );
                }
                StmtKind::Const { name, .. } => {
                    self.push(name, IndexedKind::Constant, owner, from, None);
                }
                StmtKind::Enum(decl) => {
                    self.push(&decl.name, IndexedKind::Class, owner, from, None);
                }
                StmtKind::Interface(decl) => {
                    self.push(&decl.name, IndexedKind::Class, owner, from, None);
                }
                StmtKind::Export(inner) => self.statements(std::slice::from_ref(inner), owner),
                _ => {}
            }
        }
    }

    fn class(&mut self, decl: &ClassDecl, owner: Option<&str>) {
        let from = decl.span.start_usize();
        let kind = if decl.is_module {
            IndexedKind::Module
        } else {
            IndexedKind::Class
        };
        let range = self.name_range(&decl.name, from);
        self.symbols.push(IndexedSymbol {
            name: decl.name.clone(),
            kind,
            owner: owner.map(str::to_string),
            superclass: decl.superclass.clone(),
            path: self.path.to_path_buf(),
            doc: doc_comment_above(self.source, range.start.line),
            range,
            signature: Some(match &decl.superclass {
                Some(parent) => format!("{} {} < {}", kind.label(), decl.name, parent),
                None => format!("{} {}", kind.label(), decl.name),
            }),
        });
        let class = decl.name.as_str();
        for field in &decl.fields {
            let signature = field
                .type_annotation
                .as_ref()
                .map(|t| format!("{}: {}", field.name, t));
            self.push(
                &field.name,
                IndexedKind::Field,
                Some(class),
                field.span.start_usize(),
                signature,
            );
        }
        for method in decl.methods.iter().chain(&decl.concern_class_methods) {
            let (kind, keyword) = if method.is_static {
                (IndexedKind::StaticMethod, "static def")
            } else {
                (IndexedKind::Method, "def")
            };
            let signature = signature(
                keyword,
                &method.name,
                &method.params,
                method.return_type.as_ref(),
            );
            self.push(
                &method.name,
                kind,
                Some(class),
                method.span.start_usize(),
                Some(signature),
            );
        }
        let declarations = decl
            .class_statements
            .iter()
            .chain(decl.static_block.iter().flatten());
        for stmt in declarations {
            self.dsl_declaration(stmt, class);
        }
        for nested in &decl.nested_classes {
            self.class(nested, Some(class));
        }
    }

    /// `scope("published", …)` and `has_many("comments")` / `has_one` /
    /// `belongs_to` in a class body or its `static { }` block, called bare or
    /// on `this`.
    fn dsl_declaration(&mut self, stmt: &Stmt, class: &str) {
        let StmtKind::Expression(expr) = &stmt.kind else {
            return;
        };
        let ExprKind::Call { callee, arguments } = &expr.kind else {
            return;
        };
        let dsl = match &callee.kind {
            ExprKind::Variable(name) => name.as_str(),
            ExprKind::Member { object, name } if matches!(object.kind, ExprKind::This) => {
                name.as_str()
            }
            _ => return,
        };
        let kind = match dsl {
            "scope" => IndexedKind::Scope,
            "has_many" | "has_one" | "belongs_to" => IndexedKind::Relation,
            _ => return,
        };
        let Some(Argument::Positional(first)) = arguments.first() else {
            return;
        };
        let name = match &first.kind {
            ExprKind::StringLiteral(name) | ExprKind::Symbol(name) => name.clone(),
            _ => return,
        };
        let start = first.span.start_usize();
        let end = first.span.end_usize();
        // The name inside its quotes (or after its `:`).
        let inner_start = self.source[start..end.min(self.source.len())]
            .find(name.as_str())
            .map_or(start, |i| start + i);
        let range = range_of(self.source, inner_start, inner_start + name.len());
        self.symbols.push(IndexedSymbol {
            signature: Some(format!("{dsl}(\"{name}\")")),
            name,
            kind,
            owner: Some(class.to_string()),
            superclass: None,
            path: self.path.to_path_buf(),
            doc: doc_comment_above(self.source, range.start.line),
            range,
        });
    }
}

/// `def name(a: Int, b) -> Ret`. An untyped parameter (its type is the
/// parser's implicit `Any`) shows bare.
fn signature(
    keyword: &str,
    name: &str,
    params: &[crate::ast::stmt::Parameter],
    return_type: Option<&crate::ast::types::TypeAnnotation>,
) -> String {
    let params: Vec<String> = params
        .iter()
        .map(|param| {
            let mut text = String::new();
            if param.is_block_param {
                text.push('&');
            }
            text.push_str(&param.name);
            let ty = param.type_annotation.to_string();
            if ty != "Any" {
                text.push_str(": ");
                text.push_str(&ty);
            }
            if param.default_value.is_some() {
                text.push_str(" = …");
            }
            text
        })
        .collect();
    let mut out = format!("{keyword} {name}({})", params.join(", "));
    if let Some(ret) = return_type {
        out.push_str(&format!(" -> {ret}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn fixture() -> (tempfile::TempDir, ProjectIndex) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(&root, "config/routes.sl", "get(\"/\", \"posts#index\")\n");
        write(
            &root,
            "app/models/post.sl",
            "class Post < Model\n  title: String\n  scope(\"published\", fn() { this.where(\"doc.published == true\") })\n  has_many(\"comments\")\n\n  def summary(width: Int) -> String\n    @title\n  end\n\n  static def recent\n    Post.all\n  end\nend\n",
        );
        write(
            &root,
            "app/models/featured_post.sl",
            "class FeaturedPost < Post\n  def badge\n    \"*\"\n  end\nend\n",
        );
        write(
            &root,
            "app/helpers/format.sl",
            "def format_price(cents)\n  \"#{cents / 100}\"\nend\n",
        );
        write(
            &root,
            "app/controllers/posts_controller.sl",
            "def index(req)\n  1\nend\n",
        );
        let index = ProjectIndex::scan(&root);
        (dir, index)
    }

    #[test]
    fn a_class_is_found_at_its_name_with_its_superclass() {
        let (_dir, index) = fixture();
        let post = index.class("Post").unwrap();
        assert!(post.path.ends_with("app/models/post.sl"));
        assert_eq!(post.range.start.line, 0);
        assert_eq!(post.range.start.character, 6);
        assert_eq!(post.superclass.as_deref(), Some("Model"));
        assert!(index.is_model("FeaturedPost"));
        assert_eq!(
            index.ancestry("FeaturedPost"),
            vec!["FeaturedPost", "Post", "Model"]
        );
    }

    #[test]
    fn members_carry_their_owner_and_are_inherited() {
        let (_dir, index) = fixture();
        let names = |static_side| -> Vec<String> {
            index
                .members("FeaturedPost", static_side)
                .into_iter()
                .map(|s| s.name.clone())
                .collect()
        };
        assert_eq!(names(false), vec!["badge", "title", "comments", "summary"]);
        assert_eq!(names(true), vec!["published", "recent"]);
        let summary = &index.member("Post", "summary")[0];
        assert_eq!(
            summary.signature.as_deref(),
            Some("def summary(width: Int) -> String")
        );
        let scope = &index.member("Post", "published")[0];
        assert_eq!(scope.kind, IndexedKind::Scope);
        assert_eq!(scope.range.start.line, 2);
        assert_eq!(scope.range.start.character, 9);
    }

    #[test]
    fn helpers_and_actions_stay_in_their_namespace() {
        let (_dir, index) = fixture();
        assert_eq!(
            index
                .top_level("format_price", Some(Namespace::Helper))
                .len(),
            1
        );
        assert!(index
            .top_level("format_price", Some(Namespace::Global))
            .is_empty());
        assert!(index.top_level("index", None).is_empty());
    }

    #[test]
    fn a_buffer_that_does_not_parse_keeps_the_last_good_entry_and_a_deleted_file_leaves() {
        let (dir, mut index) = fixture();
        let root = dir.path().canonicalize().unwrap();
        let post = root.join("app/models/post.sl");
        index.update_file(&post, "class Post < Model\n  def broken(\n");
        assert!(index.class("Post").is_some());
        index.update_file(&post, "class Article < Model\nend\n");
        assert!(index.class("Post").is_none());
        assert!(index.class("Article").is_some());
        std::fs::remove_file(&post).unwrap();
        index.refresh_from_disk(&post);
        assert!(index.class("Article").is_none());
    }

    #[test]
    fn known_names_leave_out_the_file_itself() {
        let (dir, index) = fixture();
        let root = dir.path().canonicalize().unwrap();
        let names = index.known_names(&root.join("app/models/post.sl"));
        assert!(names.contains("FeaturedPost"));
        assert!(names.contains("format_price"));
        assert!(!names.contains("Post"));
    }
}
