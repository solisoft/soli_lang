//! Language Server Protocol implementation for Soli.

pub mod actions;
pub mod completions;
pub mod folding;
pub mod format;
pub mod goto;
pub mod hover;
pub mod inlay;
pub mod references;
pub mod rename;
pub mod semantic;
pub mod symbols;
pub mod symbols_lsp;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use lsp_types::{InitializeParams, ServerCapabilities, TextEdit};
use tower_lsp::{LanguageServer, LspService};
use url::Url;

use crate::lsp::symbols::SymbolTable;

#[derive(Debug)]
pub struct Backend {
    client: tower_lsp::Client,
    documents: Arc<Mutex<HashMap<Url, String>>>,
    symbol_tables: Arc<Mutex<HashMap<Url, SymbolTable>>>,
}

impl Backend {
    fn get_document(&self, uri: &Url) -> Option<String> {
        self.documents.lock().unwrap().get(uri).cloned()
    }

    fn update_document(&self, uri: Url, text: String) {
        self.documents.lock().unwrap().insert(uri, text);
    }

    fn remove_document(&self, uri: &Url) {
        self.documents.lock().unwrap().remove(uri);
        self.symbol_tables.lock().unwrap().remove(uri);
    }

    fn get_symbol_table(&self, uri: &Url) -> Option<SymbolTable> {
        self.symbol_tables.lock().unwrap().get(uri).cloned()
    }

    fn update_symbol_table(&self, uri: Url, table: SymbolTable) {
        self.symbol_tables.lock().unwrap().insert(uri, table);
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<lsp_types::InitializeResult, tower_lsp::jsonrpc::Error> {
        log::info!("LSP server initializing...");
        let _ = params;

        Ok(lsp_types::InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(lsp_types::TextDocumentSyncCapability::Kind(
                    lsp_types::TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
                completion_provider: Some(lsp_types::CompletionOptions::default()),
                definition_provider: Some(lsp_types::OneOf::Left(true)),
                type_definition_provider: Some(lsp_types::TypeDefinitionProviderCapability::Simple(true)),
                references_provider: Some(lsp_types::OneOf::Left(true)),
                document_symbol_provider: Some(lsp_types::OneOf::Left(true)),
                rename_provider: Some(lsp_types::OneOf::Left(true)),
                code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
                document_formatting_provider: Some(lsp_types::OneOf::Left(true)),
                document_range_formatting_provider: Some(lsp_types::OneOf::Left(true)),
                folding_range_provider: Some(lsp_types::FoldingRangeProviderCapability::Simple(true)),
                inlay_hint_provider: Some(lsp_types::OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, params: lsp_types::InitializedParams) {
        log::info!("LSP server initialized");
        let _ = params;
        self.client.log_message(lsp_types::MessageType::INFO, "Soli LSP server ready").await;
    }

    async fn shutdown(&self) -> Result<(), tower_lsp::jsonrpc::Error> {
        Ok(())
    }

    async fn did_open(&self, params: lsp_types::DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        
        log::info!("Document opened: {}", uri);
        self.update_document(uri.clone(), text.clone());

        if let Some(table) = symbols::build_symbol_table(&text) {
            self.update_symbol_table(uri.clone(), table);
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
                if let Some(table) = symbols::build_symbol_table(&text) {
                    self.update_symbol_table(uri.clone(), table);
                }
            }
        }

        if let Err(e) = self.publish_diagnostics(uri).await {
            log::error!("Error publishing diagnostics: {:?}", e);
        }
    }

    async fn did_close(&self, params: lsp_types::DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        log::info!("Document closed: {}", uri);
        self.remove_document(&uri);
    }

    async fn hover(&self, params: lsp_types::HoverParams) -> Result<Option<lsp_types::Hover>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        
        if let Some(text) = self.get_document(&uri) {
            return Ok(hover::get_hover(&text, pos));
        }
        Ok(None)
    }

    async fn completion(&self, params: lsp_types::CompletionParams) -> Result<Option<lsp_types::CompletionResponse>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        
        if let Some(text) = self.get_document(&uri) {
            let table = self.get_symbol_table(&uri);
            return Ok(completions::get_completions(&text, pos, table.as_ref()));
        }
        Ok(None)
    }

    async fn goto_definition(&self, params: lsp_types::GotoDefinitionParams) -> Result<Option<lsp_types::GotoDefinitionResponse>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        
        if let Some(text) = self.get_document(&uri) {
            if let Some(table) = self.get_symbol_table(&uri) {
                return Ok(goto::goto_definition(&text, pos, &table));
            }
        }
        Ok(None)
    }

    async fn references(&self, params: lsp_types::ReferenceParams) -> Result<Option<Vec<lsp_types::Location>>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        
        if let Some(text) = self.get_document(&uri) {
            if let Some(table) = self.get_symbol_table(&uri) {
                return Ok(references::find_references(&text, pos, &table));
            }
        }
        Ok(None)
    }

    async fn rename(&self, params: lsp_types::RenameParams) -> Result<Option<lsp_types::WorkspaceEdit>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let new_name = params.new_name;
        
        if let Some(text) = self.get_document(&uri) {
            if let Some(table) = self.get_symbol_table(&uri) {
                return Ok(rename::rename_symbol(&text, pos, &new_name, &table));
            }
        }
        Ok(None)
    }

    async fn document_symbol(&self, params: lsp_types::DocumentSymbolParams) -> Result<Option<lsp_types::DocumentSymbolResponse>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document.uri;
        
        if let Some(text) = self.get_document(&uri) {
            return Ok(Some(symbols_lsp::get_document_symbols(&text)));
        }
        Ok(None)
    }

    async fn formatting(&self, params: lsp_types::DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document.uri;
        
        if let Some(text) = self.get_document(&uri) {
            return Ok(Some(format::format_document(&text)));
        }
        Ok(None)
    }

    async fn range_formatting(&self, params: lsp_types::DocumentRangeFormattingParams) -> Result<Option<Vec<TextEdit>>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document.uri;
        let range = params.range;
        
        if let Some(text) = self.get_document(&uri) {
            return Ok(Some(format::format_range(&text, range)));
        }
        Ok(None)
    }

    async fn code_action(&self, params: lsp_types::CodeActionParams) -> Result<Option<Vec<lsp_types::CodeActionOrCommand>>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document.uri;
        let range = params.range;
        
        if let Some(text) = self.get_document(&uri) {
            let actions: Vec<lsp_types::CodeActionOrCommand> = actions::get_code_actions(&text, range)
                .into_iter()
                .map(lsp_types::CodeActionOrCommand::CodeAction)
                .collect();
            return Ok(Some(actions));
        }
        Ok(None)
    }

    async fn folding_range(&self, params: lsp_types::FoldingRangeParams) -> Result<Option<Vec<lsp_types::FoldingRange>>, tower_lsp::jsonrpc::Error> {
        let uri = params.text_document.uri;
        let _ = uri;
        
        if let Some(text) = self.get_document(&uri) {
            return Ok(Some(folding::get_folding_ranges(&text)));
        }
        Ok(None)
    }
}

impl Backend {
    async fn publish_diagnostics(&self, uri: Url) -> Result<(), tower_lsp::jsonrpc::Error> {
        use lsp_types::{Diagnostic, DiagnosticSeverity};
        
        if let Some(text) = self.get_document(&uri) {
            let diagnostics: Vec<Diagnostic> = crate::lint(&text)
                .unwrap_or_default()
                .into_iter()
                .map(|d| {
                    let start = lsp_types::Position::new(
                        (d.span.line.saturating_sub(1)) as u32,
                        (d.span.column.saturating_sub(1)) as u32,
                    );
                    let end = lsp_types::Position::new(
                        (d.span.line.saturating_sub(1)) as u32,
                        (d.span.column + d.message.len()) as u32,
                    );
                    Diagnostic {
                        range: lsp_types::Range::new(start, end),
                        severity: Some(DiagnosticSeverity::WARNING),
                        message: d.message,
                        code: Some(lsp_types::NumberOrString::String(d.rule.to_string())),
                        source: Some("soli".to_string()),
                        ..Default::default()
                    }
                })
                .collect();

            self.client.publish_diagnostics(uri, diagnostics, None).await;
        }
        Ok(())
    }
}

pub fn start_lsp() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let service = LspService::new(|client| Backend {
        client,
        documents: Arc::new(Mutex::new(HashMap::new())),
        symbol_tables: Arc::new(Mutex::new(HashMap::new())),
    });

    tower_lsp::Server::new(stdin, stdout).serve(service);
}
