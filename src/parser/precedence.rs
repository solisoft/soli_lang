//! Operator precedence for Pratt parsing.

use crate::lexer::TokenKind;

/// Operator precedence levels (higher = tighter binding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    None = 0,
    Assignment = 1,        // =
    Ternary = 2,           // ? :
    NullishCoalescing = 3, // ??
    Or = 4,                // ||
    And = 5,               // &&
    Equality = 6,          // == !=
    Comparison = 7,        // < > <= >=
    Pipeline = 8,          // |>
    Term = 9,              // + -
    Factor = 10,           // * / %
    Unary = 11,            // ! -
    Call = 12,             // . () []
    Primary = 13,
}

impl Precedence {
    pub fn next(self) -> Precedence {
        match self {
            Precedence::None => Precedence::Assignment,
            Precedence::Assignment => Precedence::Ternary,
            Precedence::Ternary => Precedence::NullishCoalescing,
            Precedence::NullishCoalescing => Precedence::Or,
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
        TokenKind::Question => Precedence::Ternary,
        TokenKind::NullishCoalescing => Precedence::NullishCoalescing,
        TokenKind::Or => Precedence::Or,
        TokenKind::And => Precedence::And,
        TokenKind::EqualEqual | TokenKind::BangEqual => Precedence::Equality,
        TokenKind::Less | TokenKind::LessEqual | TokenKind::Greater | TokenKind::GreaterEqual => {
            Precedence::Comparison
        }
        TokenKind::Range => Precedence::Comparison, // .. has same precedence as comparison
        TokenKind::Pipeline => Precedence::Pipeline,
        TokenKind::Plus | TokenKind::Minus => Precedence::Term,
        TokenKind::Star | TokenKind::Slash | TokenKind::Percent => Precedence::Factor,
        TokenKind::LeftParen | TokenKind::Dot | TokenKind::DoubleColon | TokenKind::LeftBracket => {
            Precedence::Call
        }
        _ => Precedence::None,
    }
}
