//! Document symbols provider for LSP.

use crate::lsp::symbols::SymbolTable;
use lsp_types::{DocumentSymbol, DocumentSymbolResponse, Position, Range, SymbolKind, SymbolTag};

pub fn get_document_symbols(source: &str) -> DocumentSymbolResponse {
    let Some(table) = crate::lsp::symbols::build_symbol_table(source) else {
        return DocumentSymbolResponse::Flat(Vec::new());
    };

    let mut symbols = Vec::new();

    for scoped in &table.symbols {
        let kind = match scoped.symbol.kind {
            crate::lsp::symbols::SymbolKind::Class => SymbolKind::CLASS,
            crate::lsp::symbols::SymbolKind::Function => SymbolKind::FUNCTION,
            crate::lsp::symbols::SymbolKind::Method => SymbolKind::METHOD,
            crate::lsp::symbols::SymbolKind::Property => SymbolKind::PROPERTY,
            crate::lsp::symbols::SymbolKind::Variable => SymbolKind::VARIABLE,
            crate::lsp::symbols::SymbolKind::Parameter => SymbolKind::VARIABLE,
            crate::lsp::symbols::SymbolKind::Constant => SymbolKind::CONSTANT,
        };

        let range = lsp_range_from_span(scoped.symbol.span);
        let selection_range = Range {
            start: Position {
                line: range.start.line,
                character: range.start.character,
            },
            end: Position {
                line: range.start.line,
                character: range.start.character + scoped.symbol.name.len() as u32,
            },
        };

        let children = get_children_for_symbol(&scoped.symbol, &table);

        let detail = scoped.symbol.type_name.clone();

        symbols.push(DocumentSymbol {
            name: scoped.symbol.name.clone(),
            kind,
            tags: None,
            detail,
            deprecated: None,
            range,
            selection_range,
            children: if children.is_empty() {
                None
            } else {
                Some(children)
            },
        });
    }

    DocumentSymbolResponse::Nested(symbols)
}

fn get_children_for_symbol(
    symbol: &crate::lsp::symbols::Symbol,
    table: &SymbolTable,
) -> Vec<DocumentSymbol> {
    let mut children = Vec::new();

    for scoped in table.symbols.iter() {
        if scoped.symbol.name == symbol.name {
            continue;
        }

        if scoped.symbol.span.start > symbol.span.start && scoped.symbol.span.end <= symbol.span.end
        {
            if scoped.symbol.scope_level == symbol.scope_level + 1 {
                let kind = match scoped.symbol.kind {
                    crate::lsp::symbols::SymbolKind::Class => SymbolKind::CLASS,
                    crate::lsp::symbols::SymbolKind::Function => SymbolKind::FUNCTION,
                    crate::lsp::symbols::SymbolKind::Method => SymbolKind::METHOD,
                    crate::lsp::symbols::SymbolKind::Property => SymbolKind::PROPERTY,
                    crate::lsp::symbols::SymbolKind::Variable => SymbolKind::VARIABLE,
                    crate::lsp::symbols::SymbolKind::Parameter => SymbolKind::VARIABLE,
                    crate::lsp::symbols::SymbolKind::Constant => SymbolKind::CONSTANT,
                };

                let range = lsp_range_from_span(scoped.symbol.span);
                let selection_range = Range {
                    start: range.start,
                    end: Position {
                        line: range.start.line,
                        character: range.start.character + scoped.symbol.name.len() as u32,
                    },
                };

                children.push(DocumentSymbol {
                    name: scoped.symbol.name.clone(),
                    kind,
                    tags: None,
                    detail: scoped.symbol.type_name.clone(),
                    deprecated: None,
                    range,
                    selection_range,
                    children: None,
                });
            }
        }
    }

    children
}

fn lsp_range_from_span(span: crate::span::Span) -> Range {
    Range {
        start: Position {
            line: (span.line.saturating_sub(1)) as u32,
            character: (span.column.saturating_sub(1)) as u32,
        },
        end: Position {
            line: (span.line.saturating_sub(1)) as u32,
            character: ((span.column + (span.end - span.start)) as u32),
        },
    }
}
