//! Semantic tokens provider for LSP.

use crate::lsp::symbols::SymbolTable;
use lsp_types::{
    SemanticToken, SemanticTokenType, SemanticTokensPartialResult, SemanticTokensResult,
};

const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::NAMESPACE,
    SemanticTokenType::TYPE,
    SemanticTokenType::CLASS,
    SemanticTokenType::ENUM,
    SemanticTokenType::INTERFACE,
    SemanticTokenType::STRUCT,
    SemanticTokenType::TYPE_PARAMETER,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::ENUM_MEMBER,
    SemanticTokenType::EVENT,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::METHOD,
    SemanticTokenType::MACRO,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::MODIFIER,
    SemanticTokenType::COMMENT,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::REGEXP,
    SemanticTokenType::OPERATOR,
];

pub fn get_semantic_tokens(source: &str) -> SemanticTokensResult {
    let table = match crate::lsp::symbols::build_symbol_table(source) {
        Some(t) => t,
        None => return SemanticTokensResult::Partial(SemanticTokensPartialResult { data: vec![] }),
    };

    let mut tokens: Vec<SemanticToken> = Vec::new();

    for scoped in &table.symbols {
        let token_type = match scoped.symbol.kind {
            crate::lsp::symbols::SymbolKind::Class => SemanticTokenType::CLASS,
            crate::lsp::symbols::SymbolKind::Function => SemanticTokenType::FUNCTION,
            crate::lsp::symbols::SymbolKind::Method => SemanticTokenType::METHOD,
            crate::lsp::symbols::SymbolKind::Property => SemanticTokenType::PROPERTY,
            crate::lsp::symbols::SymbolKind::Variable => SemanticTokenType::VARIABLE,
            crate::lsp::symbols::SymbolKind::Parameter => SemanticTokenType::PARAMETER,
            crate::lsp::symbols::SymbolKind::Constant => SemanticTokenType::VARIABLE,
        };

        let delta_line: u32 = if tokens.is_empty() {
            scoped.symbol.span.line.saturating_sub(1) as u32
        } else {
            let last_line: u32 = tokens.last().map(|t| t.delta_line).unwrap_or(0);
            scoped.symbol.span.line as u32 - last_line
        };

        let delta_start: u32 = if tokens.is_empty() || delta_line > 0 {
            scoped.symbol.span.column.saturating_sub(1) as u32
        } else {
            let last_end: u32 = tokens.last().map(|t| t.delta_start + t.length).unwrap_or(0);
            scoped.symbol.span.column as u32 - last_end
        };

        tokens.push(SemanticToken {
            delta_line,
            delta_start,
            length: (scoped.symbol.span.end - scoped.symbol.span.start) as u32,
            token_type: TOKEN_TYPES
                .iter()
                .position(|t| *t == token_type)
                .unwrap_or(0) as u32,
            token_modifiers_bitset: 0,
        });
    }

    SemanticTokensResult::Partial(SemanticTokensPartialResult { data: tokens })
}
