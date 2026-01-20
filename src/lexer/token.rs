//! Token definitions for the Solilang lexer.

use crate::span::Span;

/// All token types in Solilang.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    InterpolatedString(Vec<String>), // Parts for interpolation
    BoolLiteral(bool),

    // Identifiers and keywords
    Identifier(String),

    // Keywords
    Let,
    Fn,
    Return,
    If,
    Else,
    While,
    For,
    In,
    Class,
    Extends,
    Implements,
    Interface,
    New,
    This,
    Super,
    Public,
    Private,
    Protected,
    Static,
    Null,
    Try,
    Catch,
    Finally,
    Throw,
    Async,
    Await,
    Match,

    // Module keywords
    Import,
    Export,
    From,
    As,

    // Type keywords
    Int,
    Float,
    Bool,
    String,
    Void,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Equal,
    EqualEqual,
    BangEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Bang,
    And,
    Or,
    Pipeline, // |>
    Pipe,     // |

    // Delimiters
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Dot,
    Colon,
    Semicolon,
    Arrow,    // ->
    FatArrow, // =>
    Spread,   // ...

    // Special
    Eof,
}

impl TokenKind {
    /// Check if this token is a keyword and return the corresponding kind.
    pub fn keyword(ident: &str) -> Option<TokenKind> {
        match ident {
            "let" => Some(TokenKind::Let),
            "fn" => Some(TokenKind::Fn),
            "return" => Some(TokenKind::Return),
            "if" => Some(TokenKind::If),
            "else" => Some(TokenKind::Else),
            "while" => Some(TokenKind::While),
            "for" => Some(TokenKind::For),
            "in" => Some(TokenKind::In),
            "class" => Some(TokenKind::Class),
            "extends" => Some(TokenKind::Extends),
            "implements" => Some(TokenKind::Implements),
            "interface" => Some(TokenKind::Interface),
            "new" => Some(TokenKind::New),
            "this" => Some(TokenKind::This),
            "super" => Some(TokenKind::Super),
            "public" => Some(TokenKind::Public),
            "private" => Some(TokenKind::Private),
            "protected" => Some(TokenKind::Protected),
            "static" => Some(TokenKind::Static),
            "true" => Some(TokenKind::BoolLiteral(true)),
            "false" => Some(TokenKind::BoolLiteral(false)),
            "null" => Some(TokenKind::Null),
            "try" => Some(TokenKind::Try),
            "catch" => Some(TokenKind::Catch),
            "finally" => Some(TokenKind::Finally),
            "throw" => Some(TokenKind::Throw),
            "async" => Some(TokenKind::Async),
            "await" => Some(TokenKind::Await),
            "match" => Some(TokenKind::Match),
            "import" => Some(TokenKind::Import),
            "export" => Some(TokenKind::Export),
            "from" => Some(TokenKind::From),
            "as" => Some(TokenKind::As),
            "Int" => Some(TokenKind::Int),
            "Float" => Some(TokenKind::Float),
            "Bool" => Some(TokenKind::Bool),
            "String" => Some(TokenKind::String),
            "Void" => Some(TokenKind::Void),
            _ => None,
        }
    }
}

impl std::fmt::Display for TokenKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenKind::IntLiteral(n) => write!(f, "{}", n),
            TokenKind::FloatLiteral(n) => write!(f, "{}", n),
            TokenKind::StringLiteral(s) => write!(f, "\"{}\"", s),
            TokenKind::InterpolatedString(parts) => {
                write!(f, "interp\"{}\"", parts.join("...("))
            }
            TokenKind::BoolLiteral(b) => write!(f, "{}", b),
            TokenKind::Identifier(s) => write!(f, "{}", s),
            TokenKind::Let => write!(f, "let"),
            TokenKind::Fn => write!(f, "fn"),
            TokenKind::Return => write!(f, "return"),
            TokenKind::If => write!(f, "if"),
            TokenKind::Else => write!(f, "else"),
            TokenKind::While => write!(f, "while"),
            TokenKind::For => write!(f, "for"),
            TokenKind::In => write!(f, "in"),
            TokenKind::Class => write!(f, "class"),
            TokenKind::Extends => write!(f, "extends"),
            TokenKind::Implements => write!(f, "implements"),
            TokenKind::Interface => write!(f, "interface"),
            TokenKind::New => write!(f, "new"),
            TokenKind::This => write!(f, "this"),
            TokenKind::Super => write!(f, "super"),
            TokenKind::Public => write!(f, "public"),
            TokenKind::Private => write!(f, "private"),
            TokenKind::Protected => write!(f, "protected"),
            TokenKind::Static => write!(f, "static"),
            TokenKind::Null => write!(f, "null"),
            TokenKind::Try => write!(f, "try"),
            TokenKind::Catch => write!(f, "catch"),
            TokenKind::Finally => write!(f, "finally"),
            TokenKind::Throw => write!(f, "throw"),
            TokenKind::Async => write!(f, "async"),
            TokenKind::Await => write!(f, "await"),
            TokenKind::Match => write!(f, "match"),
            TokenKind::Import => write!(f, "import"),
            TokenKind::Export => write!(f, "export"),
            TokenKind::From => write!(f, "from"),
            TokenKind::As => write!(f, "as"),
            TokenKind::Int => write!(f, "Int"),
            TokenKind::Float => write!(f, "Float"),
            TokenKind::Bool => write!(f, "Bool"),
            TokenKind::String => write!(f, "String"),
            TokenKind::Void => write!(f, "Void"),
            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::Percent => write!(f, "%"),
            TokenKind::Equal => write!(f, "="),
            TokenKind::EqualEqual => write!(f, "=="),
            TokenKind::BangEqual => write!(f, "!="),
            TokenKind::Less => write!(f, "<"),
            TokenKind::LessEqual => write!(f, "<="),
            TokenKind::Greater => write!(f, ">"),
            TokenKind::GreaterEqual => write!(f, ">="),
            TokenKind::Bang => write!(f, "!"),
            TokenKind::And => write!(f, "&&"),
            TokenKind::Or => write!(f, "||"),
            TokenKind::Pipeline => write!(f, "|>"),
            TokenKind::Pipe => write!(f, "|"),
            TokenKind::LeftParen => write!(f, "("),
            TokenKind::RightParen => write!(f, ")"),
            TokenKind::LeftBrace => write!(f, "{{"),
            TokenKind::RightBrace => write!(f, "}}"),
            TokenKind::LeftBracket => write!(f, "["),
            TokenKind::RightBracket => write!(f, "]"),
            TokenKind::Comma => write!(f, ","),
            TokenKind::Dot => write!(f, "."),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Semicolon => write!(f, ";"),
            TokenKind::Arrow => write!(f, "->"),
            TokenKind::FatArrow => write!(f, "=>"),
            TokenKind::Spread => write!(f, "..."),
            TokenKind::Eof => write!(f, "EOF"),
        }
    }
}

/// A token with its kind and source location.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn eof(position: usize, line: usize, column: usize) -> Self {
        Self {
            kind: TokenKind::Eof,
            span: Span::new(position, position, line, column),
        }
    }
}
