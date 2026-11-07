//! Language Server Protocol implementation for Soli.

pub mod actions;
pub mod completions;
pub mod folding;
pub mod format;
pub mod goto;
pub mod hover;
pub mod project;
pub mod receiver;
pub mod references;
pub mod rename;
pub mod symbols;
pub mod symbols_lsp;
mod util;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tower_lsp_server::ls_types as lsp_types;
use tower_lsp_server::ls_types::{InitializeParams, ServerCapabilities, TextEdit, Uri};
use tower_lsp_server::{LanguageServer, LspService};

use crate::lsp::project::{Namespace, ProjectIndex};
use crate::lsp::symbols::SymbolTable;

#[derive(Debug)]
pub struct Backend {
    client: tower_lsp_server::Client,
    documents: Arc<Mutex<HashMap<Uri, String>>>,
    symbol_tables: Arc<Mutex<HashMap<Uri, SymbolTable>>>,
    /// The editor's workspace folders, from `initialize`.
    workspace_roots: Arc<Mutex<Vec<PathBuf>>>,
    /// One index per application root (or per directory of loose scripts),
    /// built on first use.
    indexes: Arc<Mutex<HashMap<PathBuf, ProjectIndex>>>,
    /// The client can register `workspace/didChangeWatchedFiles` dynamically.
    watch_files: Arc<AtomicBool>,
}

impl Backend {
    fn new(client: tower_lsp_server::Client) -> Self {
        Self {
            client,
            documents: Arc::new(Mutex::new(HashMap::new())),
            symbol_tables: Arc::new(Mutex::new(HashMap::new())),
            workspace_roots: Arc::new(Mutex::new(Vec::new())),
            indexes: Arc::new(Mutex::new(HashMap::new())),
            watch_files: Arc::new(AtomicBool::new(false)),
        }
    }

    fn get_document(&self, uri: &Uri) -> Option<String> {
        self.documents.lock().unwrap().get(uri).cloned()
    }

    fn update_document(&self, uri: Uri, text: String) {
        self.documents.lock().unwrap().insert(uri, text);
    }

    fn remove_document(&self, uri: &Uri) {
        self.documents.lock().unwrap().remove(uri);
        self.symbol_tables.lock().unwrap().remove(uri);
    }

    fn get_symbol_table(&self, uri: &Uri) -> Option<SymbolTable> {
        self.symbol_tables.lock().unwrap().get(uri).cloned()
    }

    fn update_symbol_table(&self, uri: Uri, table: SymbolTable) {
        self.symbol_tables.lock().unwrap().insert(uri, table);
    }

    /// The root of the index `path` belongs to: its application's root, a
    /// workspace folder holding an `app/` directory, or else its own
    /// directory (loose scripts).
    fn index_root(&self, path: &Path) -> (PathBuf, bool) {
        if let Some(root) = crate::app_layout::app_root(path) {
            return (root, false);
        }
        let roots = self.workspace_roots.lock().unwrap();
        if let Some(root) = roots
            .iter()
            .find(|root| path.starts_with(root) && root.join("app").is_dir())
        {
            return (root.clone(), false);
        }
        let dir = path.parent().unwrap_or(path).to_path_buf();
        (dir, true)
    }

    /// Run `f` on the index `path` belongs to, scanning the application the
    /// first time it is needed.
    fn with_index<R>(&self, path: &Path, f: impl FnOnce(&mut ProjectIndex) -> R) -> R {
        let (root, loose) = self.index_root(path);
        let mut indexes = self.indexes.lock().unwrap();
        let index = indexes.entry(root.clone()).or_insert_with(|| {
            if loose {
                ProjectIndex::loose(&root)
            } else {
                ProjectIndex::scan(&root)
            }
        });
        f(index)
    }

    /// Bring the index up to date with an open buffer: it wins over the file
    /// on disk.
    fn index_buffer(&self, uri: &Uri, text: &str) {
        if let Some(path) = document_path(uri) {
            self.with_index(&path, |index| index.update_file(&path, text));
        }
    }

    /// Re-read `path` from disk in every application index that holds it,
    /// unless an open buffer has it (the buffer wins); a deleted file leaves.
    fn refresh_from_disk(&self, path: &Path, deleted: bool) {
        let open = Uri::from_file_path(path).is_some_and(|uri| self.get_document(&uri).is_some());
        let mut indexes = self.indexes.lock().unwrap();
        for index in indexes.values_mut() {
            if !path.starts_with(&index.root) {
                continue;
            }
            if deleted {
                index.remove_file(path);
            } else if !open {
                index.refresh_from_disk(path);
            }
        }
    }

    /// Run a query against the project the document belongs to, with the
    /// namespace its code runs in (`None` for a view or a file outside the
    /// auto-loaded directories).
    fn with_project<R>(
        &self,
        uri: &Uri,
        f: impl FnOnce(Option<(&ProjectIndex, Option<Namespace>)>) -> R,
    ) -> R {
        let Some(path) = document_path(uri) else {
            return f(None);
        };
        self.with_index(&path, |index| {
            let namespace = index.namespace_of(&path);
            f(Some((index, namespace)))
        })
    }

    /// The names the document's auto-loaded neighbours declare, from the
    /// index, for a file under `app/` — the files `soli lint` gives those
    /// names to. `None` elsewhere: the linter decides on its own.
    fn known_names(&self, path: &Path) -> Option<std::collections::HashSet<String>> {
        let in_app = path
            .ancestors()
            .skip(1)
            .any(|dir| dir.file_name().is_some_and(|name| name == "app"));
        if !in_app {
            return None;
        }
        self.with_index(path, |index| {
            (!index.loose).then(|| index.known_names(path))
        })
    }
}

/// The file a `file://` URI names, canonicalized where it exists so that it
/// matches the paths the index walks (`/var` vs `/private/var` on macOS).
fn document_path(uri: &Uri) -> Option<PathBuf> {
    let path = uri.to_file_path()?.into_owned();
    if let Ok(canonical) = path.canonicalize() {
        return Some(canonical);
    }
    match (
        path.parent().and_then(|p| p.canonicalize().ok()),
        path.file_name(),
    ) {
        (Some(parent), Some(name)) => Some(parent.join(name)),
        _ => Some(path),
    }
}

/// Whether the document is a `.slv` view template.
fn is_template(uri: &Uri) -> bool {
    uri.as_str().ends_with(".slv")
}

/// Whether the document is Soli at all: a `.sl` file, a `.slv` template, or
/// a file with no extension (`Solifile`, an unsaved buffer). A client that
/// cannot single out `.slv` files sends every HTML buffer along (Nova maps
/// them to its `html` syntax); those get nothing from this server — above all
/// no formatting or rename, which would rewrite markup as Soli.
fn is_soli_document(uri: &Uri) -> bool {
    let text = uri.as_str();
    let path = text.split(['?', '#']).next().unwrap_or(text);
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        None => true,
        Some((_, ext)) => ext == "sl" || ext == "slv",
    }
}

/// Whether whole-document Soli features (symbols, formatting, folding, code
/// actions, references, rename) apply: a Soli document that is not a template,
/// whose HTML they would read as code.
fn is_soli_code(uri: &Uri) -> bool {
    is_soli_document(uri) && !is_template(uri)
}

impl LanguageServer for Backend {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> Result<lsp_types::InitializeResult, tower_lsp_server::jsonrpc::Error> {
        log::info!("LSP server initializing...");
        let mut roots: Vec<PathBuf> = params
            .workspace_folders
            .iter()
            .flatten()
            .filter_map(|folder| folder.uri.to_file_path().map(|p| p.into_owned()))
            .collect();
        #[allow(deprecated)]
        if roots.is_empty() {
            roots.extend(
                params
                    .root_uri
                    .as_ref()
                    .and_then(|uri| uri.to_file_path().map(|p| p.into_owned())),
            );
        }
        *self.workspace_roots.lock().unwrap() = roots
            .into_iter()
            .map(|root| root.canonicalize().unwrap_or(root))
            .collect();
        let watch_files = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.did_change_watched_files.as_ref())
            .and_then(|watched| watched.dynamic_registration)
            .unwrap_or(false);
        self.watch_files.store(watch_files, Ordering::Relaxed);

        Ok(lsp_types::InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(lsp_types::TextDocumentSyncCapability::Options(
                    lsp_types::TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(lsp_types::TextDocumentSyncKind::FULL),
                        save: Some(lsp_types::TextDocumentSyncSaveOptions::Supported(true)),
                        ..Default::default()
                    },
                )),
                hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
                completion_provider: Some(lsp_types::CompletionOptions {
                    trigger_characters: Some(vec![".".to_string(), "@".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(lsp_types::OneOf::Left(true)),
                references_provider: Some(lsp_types::OneOf::Left(true)),
                document_symbol_provider: Some(lsp_types::OneOf::Left(true)),
                rename_provider: Some(lsp_types::OneOf::Left(true)),
                code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
                document_formatting_provider: Some(lsp_types::OneOf::Left(true)),
                folding_range_provider: Some(lsp_types::FoldingRangeProviderCapability::Simple(
                    true,
                )),
                ..Default::default()
            },
            // Lets a client show which `soli` it reached and warn about an old one.
            server_info: Some(lsp_types::ServerInfo {
                name: "soli".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            ..Default::default()
        })
    }

    async fn initialized(&self, params: lsp_types::InitializedParams) {
        log::info!("LSP server initialized");
        let _ = params;
        self.client
            .log_message(lsp_types::MessageType::INFO, "Soli LSP server ready")
            .await;
        if self.watch_files.load(Ordering::Relaxed) {
            // Edits made outside the editor (git checkout, a generator) reach
            // the index through these events. Without dynamic registration the
            // index still refreshes on open, change and save.
            let watchers = ["**/*.sl", "**/*.slv"]
                .iter()
                .map(|glob| lsp_types::FileSystemWatcher {
                    glob_pattern: lsp_types::GlobPattern::String(glob.to_string()),
                    kind: None,
                })
                .collect();
            let options = lsp_types::DidChangeWatchedFilesRegistrationOptions { watchers };
            let registration = lsp_types::Registration {
                id: "soli-watched-files".to_string(),
                method: "workspace/didChangeWatchedFiles".to_string(),
                register_options: serde_json::to_value(options).ok(),
            };
            if let Err(error) = self.client.register_capability(vec![registration]).await {
                log::warn!("soli lsp: could not watch files: {error:?}");
            }
        }
    }

    async fn shutdown(&self) -> Result<(), tower_lsp_server::jsonrpc::Error> {
        Ok(())
    }

    async fn did_open(&self, params: lsp_types::DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;

        log::info!("Document opened: {}", uri.as_str());
        self.update_document(uri.clone(), text.clone());

        if is_soli_code(&uri) {
            if let Some(table) = symbols::build_symbol_table(&text) {
                self.update_symbol_table(uri.clone(), table);
            }
            self.index_buffer(&uri, &text);
        }

        if let Err(e) = self.publish_diagnostics(uri).await {
            log::error!("Error publishing diagnostics: {:?}", e);
        }
    }

    async fn did_change(&self, params: lsp_types::DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;

        if let Some(text) = params.content_changes.into_iter().last() {
            self.update_document(uri.clone(), text.text);

            if let Some(text) = self.get_document(&uri) {
                if is_soli_code(&uri) {
                    if let Some(table) = symbols::build_symbol_table(&text) {
                        self.update_symbol_table(uri.clone(), table);
                    }
                    self.index_buffer(&uri, &text);
                }
            }
        }

        if let Err(e) = self.publish_diagnostics(uri).await {
            log::error!("Error publishing diagnostics: {:?}", e);
        }
    }

    async fn did_save(&self, params: lsp_types::DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        match self.get_document(&uri) {
            Some(text) if is_soli_code(&uri) => self.index_buffer(&uri, &text),
            Some(_) => {}
            None => {
                if let Some(path) = document_path(&uri) {
                    self.refresh_from_disk(&path, false);
                }
            }
        }
    }

    async fn did_change_watched_files(&self, params: lsp_types::DidChangeWatchedFilesParams) {
        for change in params.changes {
            let Some(path) = document_path(&change.uri) else {
                continue;
            };
            self.refresh_from_disk(&path, change.typ == lsp_types::FileChangeType::DELETED);
        }
    }

    async fn did_close(&self, params: lsp_types::DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        log::info!("Document closed: {}", uri.as_str());
        self.remove_document(&uri);
        // The buffer may have been closed unsaved: the disk is the truth again.
        if let Some(path) = document_path(&uri) {
            let deleted = !path.exists();
            self.refresh_from_disk(&path, deleted);
        }
    }

    async fn hover(
        &self,
        params: lsp_types::HoverParams,
    ) -> Result<Option<lsp_types::Hover>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        if !is_soli_document(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            let table = self.get_symbol_table(&uri);
            return Ok(self.with_project(&uri, |project| {
                hover::get_hover(&text, pos, table.as_ref(), project)
            }));
        }
        Ok(None)
    }

    async fn completion(
        &self,
        params: lsp_types::CompletionParams,
    ) -> Result<Option<lsp_types::CompletionResponse>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        if !is_soli_document(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            // A template's prose and markup are not code.
            if is_template(&uri) && !completions::in_template_code(&text, pos) {
                return Ok(None);
            }
            let table = self.get_symbol_table(&uri);
            return Ok(self.with_project(&uri, |project| {
                completions::get_completions(&text, pos, table.as_ref(), project)
            }));
        }
        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: lsp_types::GotoDefinitionParams,
    ) -> Result<Option<lsp_types::GotoDefinitionResponse>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        if !is_soli_document(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            let table = self.get_symbol_table(&uri);
            return Ok(self.with_project(&uri, |project| {
                goto::goto_definition(&text, pos, table.as_ref(), &uri, project)
            }));
        }
        Ok(None)
    }

    async fn references(
        &self,
        params: lsp_types::ReferenceParams,
    ) -> Result<Option<Vec<lsp_types::Location>>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        // Templates wait for offset mapping: a text scan would count markup.
        if !is_soli_code(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            return Ok(references::find_references(&text, pos, &uri));
        }
        Ok(None)
    }

    async fn rename(
        &self,
        params: lsp_types::RenameParams,
    ) -> Result<Option<lsp_types::WorkspaceEdit>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let new_name = params.new_name;
        // In a template the text scan would rename words in the markup too.
        if !is_soli_code(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            return Ok(rename::rename_symbol(&text, pos, &new_name, &uri));
        }
        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: lsp_types::DocumentSymbolParams,
    ) -> Result<Option<lsp_types::DocumentSymbolResponse>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document.uri;
        if !is_soli_code(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            return Ok(Some(symbols_lsp::get_document_symbols(&text)));
        }
        Ok(None)
    }

    async fn formatting(
        &self,
        params: lsp_types::DocumentFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document.uri;
        if !is_soli_code(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            return Ok(Some(format::format_document(&text)));
        }
        Ok(None)
    }

    async fn code_action(
        &self,
        params: lsp_types::CodeActionParams,
    ) -> Result<Option<Vec<lsp_types::CodeActionOrCommand>>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document.uri;
        let range = params.range;
        if !is_soli_code(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            let actions: Vec<lsp_types::CodeActionOrCommand> =
                actions::get_code_actions(&text, range, &uri)
                    .into_iter()
                    .map(lsp_types::CodeActionOrCommand::CodeAction)
                    .collect();
            return Ok(Some(actions));
        }
        Ok(None)
    }

    async fn folding_range(
        &self,
        params: lsp_types::FoldingRangeParams,
    ) -> Result<Option<Vec<lsp_types::FoldingRange>>, tower_lsp_server::jsonrpc::Error> {
        let uri = params.text_document.uri;
        if !is_soli_code(&uri) {
            return Ok(None);
        }

        if let Some(text) = self.get_document(&uri) {
            return Ok(Some(folding::get_folding_ranges(&text)));
        }
        Ok(None)
    }
}

impl Backend {
    async fn publish_diagnostics(&self, uri: Uri) -> Result<(), tower_lsp_server::jsonrpc::Error> {
        if let Some(text) = self.get_document(&uri) {
            let path = document_path(&uri);
            // Another kind of file a client sends along (an `.html` buffer):
            // not Soli, nothing to say.
            if path.as_ref().is_some_and(|path| {
                !matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    None | Some("sl") | Some("slv")
                )
            }) {
                return Ok(());
            }
            let known = path.as_deref().and_then(|path| self.known_names(path));
            // Built before any `.await`: the error type is not `Send`.
            let diagnostics = diagnostics_for(&text, path.as_deref(), known.as_ref());
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
        }
        Ok(())
    }
}

/// The diagnostics of one buffer. With its path, it goes through
/// [`crate::lint_file_with_names`]: a `.slv` template has its `<% %>` code
/// linted (not its HTML), and a `.sl` file under `app/` gets the path-aware
/// rules (`security/unfiltered-mass-assignment`, the test-file rules…), its
/// neighbours' names coming from the index (`known`) instead of a walk of
/// the disk on each keystroke.
///
/// A file that does not lex or parse has no lint results; the error itself is
/// the diagnostic. A template's findings are reported on their whole line:
/// columns in the extracted code are not the template's.
pub(crate) fn diagnostics_for(
    text: &str,
    path: Option<&Path>,
    known: Option<&std::collections::HashSet<String>>,
) -> Vec<lsp_types::Diagnostic> {
    use tower_lsp_server::ls_types::{Diagnostic, DiagnosticSeverity};

    let template = path.is_some_and(|path| path.extension().is_some_and(|ext| ext == "slv"));
    let linted = match path {
        Some(path) => crate::lint_file_with_names(text, &path.to_string_lossy(), known),
        None => crate::lint(text),
    };
    let mut diagnostics: Vec<Diagnostic> = match linted {
        Err(error) => {
            let mut diagnostic = syntax_error_diagnostic(&error);
            if let Some(line) = error.template_line() {
                diagnostic.range = whole_line(text, line);
            }
            vec![diagnostic]
        }
        Ok(found) => found
            .into_iter()
            .map(|d| Diagnostic {
                range: if template {
                    whole_line(text, d.span.line_usize())
                } else {
                    warning_range(text, d.span)
                },
                severity: Some(DiagnosticSeverity::WARNING),
                message: d.message,
                code: Some(lsp_types::NumberOrString::String(d.rule.to_string())),
                source: Some("soli".to_string()),
                ..Default::default()
            })
            .collect(),
    };
    // What the extraction lets through but the template engine refuses — an
    // `<% if %>` without its `end` — fails at render time: an error here.
    if template
        && diagnostics
            .iter()
            .all(|d| d.severity != Some(DiagnosticSeverity::ERROR))
    {
        if let Err(message) = crate::template::parser::parse_template(text) {
            let line = crate::template::parser::error_line(&message).unwrap_or(1);
            diagnostics.push(Diagnostic {
                range: whole_line(text, line),
                severity: Some(DiagnosticSeverity::ERROR),
                message: format!("Template error: {message}"),
                source: Some("soli".to_string()),
                ..Default::default()
            });
        }
    }
    diagnostics
}

/// The whole of 1-based `line`, as an LSP range.
fn whole_line(text: &str, line: usize) -> lsp_types::Range {
    let index = line.saturating_sub(1);
    let length = text.lines().nth(index).map_or(0, |content| {
        content.trim_end_matches('\r').encode_utf16().count()
    });
    lsp_types::Range::new(
        lsp_types::Position::new(index as u32, 0),
        lsp_types::Position::new(index as u32, length as u32),
    )
}

/// Where a lint finding is: from its line and column to the end of its span,
/// kept on that line. A finding with an empty span covers the identifier at
/// its column, or one character. (The end used to be the column plus the
/// length of the *message*.)
fn warning_range(text: &str, span: crate::span::Span) -> lsp_types::Range {
    let line_index = span.line_usize().saturating_sub(1);
    let line_start: usize = text
        .split_inclusive('\n')
        .take(line_index)
        .map(str::len)
        .sum();
    let line_text = text[line_start..]
        .split('\n')
        .next()
        .unwrap_or("")
        .trim_end_matches('\r');
    let line_end = line_start + line_text.len();
    let start = line_text
        .char_indices()
        .nth(span.column_usize().saturating_sub(1))
        .map_or(line_end, |(i, _)| line_start + i);
    let length = span.end_usize().saturating_sub(span.start_usize());
    let end = if length > 0 {
        let mut end = (start + length).min(line_end);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        end
    } else {
        let word_end = text[start..line_end]
            .char_indices()
            .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
            .map_or(line_end, |(i, _)| start + i);
        if word_end > start {
            word_end
        } else {
            text[start..line_end]
                .chars()
                .next()
                .map_or(start, |c| start + c.len_utf8())
        }
    };
    util::range_of(text, start, end.max(start))
}

/// The lexer or parser error that stopped the lint, as an ERROR at its place.
fn syntax_error_diagnostic(error: &crate::error::SolilangError) -> lsp_types::Diagnostic {
    use crate::error::SolilangError;
    let span = match error {
        SolilangError::Lexer(e) => Some(e.span()),
        SolilangError::Parser(e) => Some(e.span()),
        _ => None,
    };
    let start = span.map_or(lsp_types::Position::new(0, 0), |span| {
        lsp_types::Position::new(
            (span.line_usize().saturating_sub(1)) as u32,
            (span.column_usize().saturating_sub(1)) as u32,
        )
    });
    let end = lsp_types::Position::new(start.line, start.character + 1);
    lsp_types::Diagnostic {
        range: lsp_types::Range::new(start, end),
        severity: Some(lsp_types::DiagnosticSeverity::ERROR),
        message: error.to_string(),
        code: error
            .code()
            .map(|code| lsp_types::NumberOrString::String(code.to_string())),
        source: Some("soli".to_string()),
        ..Default::default()
    }
}

/// Run the Soli LSP server on stdio. Blocks the current thread.
///
/// `tower-lsp-server` is async, so we spin up a single-threaded tokio runtime here
/// rather than requiring the binary's `main` to be async — keeps the rest of
/// the CLI synchronous.
pub fn start_lsp() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime for LSP");

    runtime.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();

        let (service, socket) = LspService::new(Backend::new);

        tower_lsp_server::Server::new(stdin, stdout, socket)
            .serve(service)
            .await;
    });
}
