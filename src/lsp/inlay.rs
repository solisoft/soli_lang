//! Inlay hints provider for LSP.

use crate::lsp::symbols::SymbolTable;
use lsp_types::{InlayHint, InlayHintKind, Position};

pub fn get_inlay_hints(source: &str, table: &SymbolTable) -> Vec<InlayHint> {
    let mut hints = Vec::new();
    let _ = source;

    for scoped in table.symbols.iter() {
        match scoped.symbol.kind {
            crate::lsp::symbols::SymbolKind::Parameter => {
                if let Some(type_name) = &scoped.symbol.type_name {
                    hints.push(InlayHint {
                        position: Position {
                            line: (scoped.symbol.span.line.saturating_sub(1)) as u32,
                            character: scoped.symbol.span.end as u32,
                        },
                        label: lsp_types::InlayHintLabel::String(format!(": {}", type_name)),
                        kind: Some(InlayHintKind::TYPE),
                        text_edits: None,
                        data: None,
                        padding_left: Some(false),
                        padding_right: Some(false),
                        tooltip: None,
                    });
                }
            }
            crate::lsp::symbols::SymbolKind::Function => {
                if scoped.symbol.name != "init" {
                    hints.push(InlayHint {
                        position: Position {
                            line: (scoped.symbol.span.line.saturating_sub(1)) as u32,
                            character: 0,
                        },
                        label: lsp_types::InlayHintLabel::String(format!(
                            "fn {}",
                            scoped.symbol.name
                        )),
                        kind: Some(InlayHintKind::TYPE),
                        text_edits: None,
                        data: None,
                        padding_left: Some(false),
                        padding_right: Some(false),
                        tooltip: None,
                    });
                }
            }
            _ => {}
        }
    }

    hints
}
