//! Operator precedence for Pratt parsing.

use crate::lexer::TokenKind;

/// Operator precedence levels (higher = tighter binding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    None = 0,
    Assignment = 1, // =
    Or = 2,         // ||
    And = 3,        // &&
    Equality = 4,   // == !=
    Comparison = 5, // < > <= >=
    Pipeline = 6,   // |>
    Term = 7,       // + -
    Factor = 8,     // * / %
    Unary = 9,      // ! -
    Call = 10,      // . () []
    Primary = 11,
}

impl Precedence {
    pub fn next(self) -> Precedence {
        match self {
            Precedence::None => Precedence::Assignment,
            Precedence::Assignment => Precedence::Or,
            Precedence::Or => Precedence::And,
            Precedence::And => Precedence::Equality,
            Precedence::Equality => Precedence::Comparison,
            Precedence::Comparison => Precedence::Pipeline,
            Precedence::Pipeline => Precedence::Term,
            Precedence::Term => Precedence::Factor,
            Precedence::Factor => Precedence::Unary,
            Precedence::Unary => Precedence::Call,
            Precedence::Call => Precedence::Primary,
            Precedence::Primary => Precedence::Primary,
        }
    }
}

pub fn get_precedence(kind: &TokenKind) -> Precedence {
    match kind {
        TokenKind::Equal => Precedence::Assignment,
        TokenKind::Or => Precedence::Or,
        TokenKind::And => Precedence::And,
        TokenKind::EqualEqual | TokenKind::BangEqual => Precedence::Equality,
        TokenKind::Less | TokenKind::LessEqual | TokenKind::Greater | TokenKind::GreaterEqual => {
            Precedence::Comparison
        }
        TokenKind::Pipeline => Precedence::Pipeline,
        TokenKind::Plus | TokenKind::Minus => Precedence::Term,
        TokenKind::Star | TokenKind::Slash | TokenKind::Percent => Precedence::Factor,
        TokenKind::LeftParen | TokenKind::Dot | TokenKind::LeftBracket => Precedence::Call,
        _ => Precedence::None,
    }
}
