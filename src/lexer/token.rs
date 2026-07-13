//! Token definitions for the Solilang lexer.

use crate::span::Span;

/// Interpolation in SDBQL query block: #{expression}
#[derive(Debug, Clone, PartialEq)]
pub struct SdqlInterpolation {
    pub expr: String,
    pub start: usize,
    pub end: usize,
}

/// All token types in Solilang.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    IntLiteral(i64),
    FloatLiteral(f64),
    DecimalLiteral(String), // String representation of decimal value (e.g., "19.99")
    StringLiteral(String),
    InterpolatedString(Vec<String>), // Parts for interpolation
    BacktickString(String),          // Command substitution: `command`
    BoolLiteral(bool),
    SymbolLiteral(String), // :name

    // Percent array literals: %w[foo bar] → ["foo", "bar"], %i[foo bar] → [:foo, :bar], %n[1 2.5 3D] → [1, 2.5, 3D]
    StringArrayLiteral(Vec<String>),
    SymbolArrayLiteral(Vec<String>),
    NumberArrayLiteral(Vec<String>), // Raw strings: "1", "2.5", "3.5D"
    DecimalArrayLiteral(Vec<String>),

    // SDBQL query block with #{...} interpolation
    SdqlBlock {
        query: String,
        interpolations: Vec<SdqlInterpolation>,
    },

    // Identifiers and keywords
    Identifier(String),

    // Keywords
    Let,
    Const,
    Fn,
    Return,
    If,
    Else,
    Elsif,
    While,
    For,
    In,
    Class,
    Extends,
    Implements,
    Interface,
    Enum,
    New,
    This,
    SelfKeyword,
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
    Rescue,
    Not,
    Match,
    Case,
    When,
    Do,
    End,
    Unless,
    Then,

    // Module keywords
    Import,
    Export,
    From,
    As,

    // Type keywords
    Int,
    Float,
    Decimal,
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
    LessLess, // <<
    Greater,
    GreaterEqual,
    Bang,
    And,
    Or,
    Pipeline,          // |>
    Pipe,              // |
    PlusPlus,          // ++
    MinusMinus,        // --
    PlusEqual,         // +=
    MinusEqual,        // -=
    StarEqual,         // *=
    SlashEqual,        // /=
    PercentEqual,      // %=
    OrEqual,           // ||=
    AndEqual,          // &&=
    NullishEqual,      // ??=
    NullishCoalescing, // ??
    SafeNavigation,    // &.
    Ampersand,         // &
    Tilde,             // ~ (shorthand for `implements` in class headers)
    DoubleColon,       // ::

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
    Question,
    Arrow,    // ->
    FatArrow, // =>
    Spread,   // ...
    Range,    // ..

    // Special
    Eof,
}

impl TokenKind {
    /// Check if this token is a keyword and return the corresponding kind.
    ///
    /// Note: callers in the scanner skip this lookup entirely for identifiers
    /// carrying a `?`/`!` suffix, since no keyword has one. The `match` below is
    /// lowered by the compiler into a length-bucketed comparison tree — measured
    /// to be as fast as an `ahash`/`phf` table here, with zero startup cost.
    pub fn keyword(ident: &str) -> Option<TokenKind> {
        match ident {
            "let" => Some(TokenKind::Let),
            "const" => Some(TokenKind::Const),
            "fn" | "def" => Some(TokenKind::Fn),
            "return" => Some(TokenKind::Return),
            "if" => Some(TokenKind::If),
            "else" => Some(TokenKind::Else),
            "elsif" => Some(TokenKind::Elsif),
            "while" => Some(TokenKind::While),
            "for" => Some(TokenKind::For),
            "in" => Some(TokenKind::In),
            "class" => Some(TokenKind::Class),
            "extends" => Some(TokenKind::Extends),
            "implements" => Some(TokenKind::Implements),
            "interface" => Some(TokenKind::Interface),
            "enum" => Some(TokenKind::Enum),
            "new" => Some(TokenKind::New),
            "this" => Some(TokenKind::This),
            "self" => Some(TokenKind::SelfKeyword),
            "super" => Some(TokenKind::Super),
            "public" => Some(TokenKind::Public),
            "private" => Some(TokenKind::Private),
            "protected" => Some(TokenKind::Protected),
            "static" => Some(TokenKind::Static),
            "true" => Some(TokenKind::BoolLiteral(true)),
            "false" => Some(TokenKind::BoolLiteral(false)),
            "null" | "nil" => Some(TokenKind::Null),
            "try" | "begin" => Some(TokenKind::Try),
            "catch" => Some(TokenKind::Catch),
            "finally" | "ensure" => Some(TokenKind::Finally),
            "throw" => Some(TokenKind::Throw),
            "rescue" => Some(TokenKind::Rescue),
            "not" => Some(TokenKind::Not),
            "and" => Some(TokenKind::And),
            "or" => Some(TokenKind::Or),
            "match" => Some(TokenKind::Match),
            "case" => Some(TokenKind::Case),
            "when" => Some(TokenKind::When),
            "do" => Some(TokenKind::Do),
            "end" => Some(TokenKind::End),
            "then" => Some(TokenKind::Then),
            "unless" => Some(TokenKind::Unless),
            "import" => Some(TokenKind::Import),
            "export" => Some(TokenKind::Export),
            "from" => Some(TokenKind::From),
            "as" => Some(TokenKind::As),
            "Int" => Some(TokenKind::Int),
            "Float" => Some(TokenKind::Float),
            "Bool" => Some(TokenKind::Bool),
            "Decimal" => Some(TokenKind::Decimal),
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
            TokenKind::DecimalLiteral(s) => write!(f, "{}", s),
            TokenKind::StringLiteral(s) => write!(f, "\"{}\"", s),
            TokenKind::InterpolatedString(parts) => {
                write!(f, "interp\"{}\"", parts.join("...("))
            }
            TokenKind::BacktickString(s) => write!(f, "`{}`", s),
            TokenKind::SdqlBlock { query, .. } => {
                write!(f, "@sdbql{{{}}}...", &query[..query.len().min(30)])
            }
            TokenKind::BoolLiteral(b) => write!(f, "{}", b),
            TokenKind::SymbolLiteral(s) => write!(f, ":{}", s),
            TokenKind::StringArrayLiteral(arr) => write!(f, "%w[{}]", arr.join(" ")),
            TokenKind::SymbolArrayLiteral(arr) => write!(f, "%i[{}]", arr.join(" ")),
            TokenKind::NumberArrayLiteral(arr) => write!(f, "%n[{}]", arr.join(" ")),
            TokenKind::DecimalArrayLiteral(arr) => write!(f, "%d[{}]", arr.join(" ")),
            TokenKind::Identifier(s) => write!(f, "{}", s),
            TokenKind::Let => write!(f, "let"),
            TokenKind::Const => write!(f, "const"),
            TokenKind::Fn => write!(f, "fn"),
            TokenKind::Return => write!(f, "return"),
            TokenKind::If => write!(f, "if"),
            TokenKind::Else => write!(f, "else"),
            TokenKind::Elsif => write!(f, "elsif"),
            TokenKind::While => write!(f, "while"),
            TokenKind::For => write!(f, "for"),
            TokenKind::In => write!(f, "in"),
            TokenKind::Class => write!(f, "class"),
            TokenKind::Extends => write!(f, "extends"),
            TokenKind::Implements => write!(f, "implements"),
            TokenKind::Interface => write!(f, "interface"),
            TokenKind::Enum => write!(f, "enum"),
            TokenKind::New => write!(f, "new"),
            TokenKind::This => write!(f, "this"),
            TokenKind::SelfKeyword => write!(f, "self"),
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
            TokenKind::Rescue => write!(f, "rescue"),
            TokenKind::Not => write!(f, "not"),
            TokenKind::Match => write!(f, "match"),
            TokenKind::Case => write!(f, "case"),
            TokenKind::When => write!(f, "when"),
            TokenKind::Do => write!(f, "do"),
            TokenKind::End => write!(f, "end"),
            TokenKind::Then => write!(f, "then"),
            TokenKind::Unless => write!(f, "unless"),
            TokenKind::Import => write!(f, "import"),
            TokenKind::Export => write!(f, "export"),
            TokenKind::From => write!(f, "from"),
            TokenKind::As => write!(f, "as"),
            TokenKind::Int => write!(f, "Int"),
            TokenKind::Float => write!(f, "Float"),
            TokenKind::Decimal => write!(f, "Decimal"),
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
            TokenKind::LessLess => write!(f, "<<"),
            TokenKind::Greater => write!(f, ">"),
            TokenKind::GreaterEqual => write!(f, ">="),
            TokenKind::Bang => write!(f, "!"),
            TokenKind::And => write!(f, "&&"),
            TokenKind::Or => write!(f, "||"),
            TokenKind::Pipeline => write!(f, "|>"),
            TokenKind::Pipe => write!(f, "|"),
            TokenKind::PlusPlus => write!(f, "++"),
            TokenKind::MinusMinus => write!(f, "--"),
            TokenKind::PlusEqual => write!(f, "+="),
            TokenKind::MinusEqual => write!(f, "-="),
            TokenKind::StarEqual => write!(f, "*="),
            TokenKind::SlashEqual => write!(f, "/="),
            TokenKind::PercentEqual => write!(f, "%="),
            TokenKind::OrEqual => write!(f, "||="),
            TokenKind::AndEqual => write!(f, "&&="),
            TokenKind::NullishEqual => write!(f, "??="),
            TokenKind::NullishCoalescing => write!(f, "??"),
            TokenKind::SafeNavigation => write!(f, "&."),
            TokenKind::Ampersand => write!(f, "&"),
            TokenKind::Tilde => write!(f, "~"),
            TokenKind::DoubleColon => write!(f, "::"),
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
            TokenKind::Question => write!(f, "?"),
            TokenKind::Arrow => write!(f, "->"),
            TokenKind::FatArrow => write!(f, "=>"),
            TokenKind::Spread => write!(f, "..."),
            TokenKind::Range => write!(f, ".."),
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- TokenKind::keyword ----------

    #[test]
    fn keyword_recognises_basic_keywords() {
        assert_eq!(TokenKind::keyword("let"), Some(TokenKind::Let));
        assert_eq!(TokenKind::keyword("const"), Some(TokenKind::Const));
        assert_eq!(TokenKind::keyword("return"), Some(TokenKind::Return));
        assert_eq!(TokenKind::keyword("if"), Some(TokenKind::If));
        assert_eq!(TokenKind::keyword("else"), Some(TokenKind::Else));
        assert_eq!(TokenKind::keyword("class"), Some(TokenKind::Class));
        assert_eq!(TokenKind::keyword("new"), Some(TokenKind::New));
    }

    #[test]
    fn keyword_aliases_fn_and_def() {
        // Both `fn` and `def` should map to the same Fn token so users
        // coming from Ruby can write `def foo() {}`.
        assert_eq!(TokenKind::keyword("fn"), Some(TokenKind::Fn));
        assert_eq!(TokenKind::keyword("def"), Some(TokenKind::Fn));
    }

    #[test]
    fn keyword_aliases_null_and_nil() {
        assert_eq!(TokenKind::keyword("null"), Some(TokenKind::Null));
        assert_eq!(TokenKind::keyword("nil"), Some(TokenKind::Null));
    }

    #[test]
    fn keyword_aliases_try_and_begin() {
        assert_eq!(TokenKind::keyword("try"), Some(TokenKind::Try));
        assert_eq!(TokenKind::keyword("begin"), Some(TokenKind::Try));
    }

    #[test]
    fn keyword_aliases_finally_and_ensure() {
        assert_eq!(TokenKind::keyword("finally"), Some(TokenKind::Finally));
        assert_eq!(TokenKind::keyword("ensure"), Some(TokenKind::Finally));
    }

    #[test]
    fn keyword_bool_literals() {
        assert_eq!(
            TokenKind::keyword("true"),
            Some(TokenKind::BoolLiteral(true))
        );
        assert_eq!(
            TokenKind::keyword("false"),
            Some(TokenKind::BoolLiteral(false))
        );
    }

    #[test]
    fn keyword_word_operators() {
        assert_eq!(TokenKind::keyword("and"), Some(TokenKind::And));
        assert_eq!(TokenKind::keyword("or"), Some(TokenKind::Or));
        assert_eq!(TokenKind::keyword("not"), Some(TokenKind::Not));
    }

    #[test]
    fn keyword_match_machinery() {
        assert_eq!(TokenKind::keyword("match"), Some(TokenKind::Match));
        assert_eq!(TokenKind::keyword("case"), Some(TokenKind::Case));
        assert_eq!(TokenKind::keyword("when"), Some(TokenKind::When));
    }

    #[test]
    fn keyword_class_visibility_modifiers() {
        assert_eq!(TokenKind::keyword("public"), Some(TokenKind::Public));
        assert_eq!(TokenKind::keyword("private"), Some(TokenKind::Private));
        assert_eq!(TokenKind::keyword("protected"), Some(TokenKind::Protected));
        assert_eq!(TokenKind::keyword("static"), Some(TokenKind::Static));
    }

    #[test]
    fn keyword_module_keywords() {
        assert_eq!(TokenKind::keyword("import"), Some(TokenKind::Import));
        assert_eq!(TokenKind::keyword("export"), Some(TokenKind::Export));
        assert_eq!(TokenKind::keyword("from"), Some(TokenKind::From));
        assert_eq!(TokenKind::keyword("as"), Some(TokenKind::As));
    }

    #[test]
    fn keyword_type_names() {
        assert_eq!(TokenKind::keyword("Int"), Some(TokenKind::Int));
        assert_eq!(TokenKind::keyword("Float"), Some(TokenKind::Float));
        assert_eq!(TokenKind::keyword("Decimal"), Some(TokenKind::Decimal));
        assert_eq!(TokenKind::keyword("Bool"), Some(TokenKind::Bool));
        assert_eq!(TokenKind::keyword("String"), Some(TokenKind::String));
        assert_eq!(TokenKind::keyword("Void"), Some(TokenKind::Void));
    }

    #[test]
    fn keyword_returns_none_for_identifiers_and_typos() {
        assert_eq!(TokenKind::keyword("foo"), None);
        // Case-sensitive — keywords are lowercase, type names are PascalCase.
        assert_eq!(TokenKind::keyword("Let"), None);
        assert_eq!(TokenKind::keyword("INT"), None);
        // Empty string is not a keyword.
        assert_eq!(TokenKind::keyword(""), None);
        // Trailing whitespace is not stripped — caller must trim.
        assert_eq!(TokenKind::keyword("let "), None);
    }

    // ---------- Display ----------

    #[test]
    fn display_literals_use_their_natural_form() {
        assert_eq!(TokenKind::IntLiteral(42).to_string(), "42");
        assert_eq!(TokenKind::FloatLiteral(3.5).to_string(), "3.5");
        assert_eq!(
            TokenKind::DecimalLiteral("19.99".into()).to_string(),
            "19.99"
        );
        assert_eq!(TokenKind::StringLiteral("hi".into()).to_string(), "\"hi\"");
        assert_eq!(TokenKind::BoolLiteral(true).to_string(), "true");
        assert_eq!(TokenKind::SymbolLiteral("name".into()).to_string(), ":name");
    }

    #[test]
    fn display_percent_array_literals() {
        assert_eq!(
            TokenKind::StringArrayLiteral(vec!["a".into(), "b".into()]).to_string(),
            "%w[a b]"
        );
        assert_eq!(
            TokenKind::SymbolArrayLiteral(vec!["x".into()]).to_string(),
            "%i[x]"
        );
        assert_eq!(
            TokenKind::NumberArrayLiteral(vec!["1".into(), "2".into()]).to_string(),
            "%n[1 2]"
        );
        assert_eq!(
            TokenKind::DecimalArrayLiteral(vec!["1.5D".into()]).to_string(),
            "%d[1.5D]"
        );
    }

    #[test]
    fn display_keywords_round_trip_via_keyword() {
        // For every keyword Display gives, feeding that back into
        // TokenKind::keyword should yield the original kind. This is a
        // structural invariant and lets us catch missing branches in
        // either direction with a single check.
        //
        // Exception: `And`/`Or` have a dual surface form — the lexer
        // accepts both `&&`/`||` (the operator form, which is also what
        // Display emits) AND the words `and`/`or` via `keyword`. So they
        // don't round-trip via Display and are exercised separately in
        // `and_or_have_dual_surface_forms`.
        for kind in [
            TokenKind::Let,
            TokenKind::Const,
            TokenKind::Fn,
            TokenKind::Return,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::Elsif,
            TokenKind::While,
            TokenKind::For,
            TokenKind::In,
            TokenKind::Class,
            TokenKind::Extends,
            TokenKind::Implements,
            TokenKind::Interface,
            TokenKind::Enum,
            TokenKind::New,
            TokenKind::This,
            TokenKind::SelfKeyword,
            TokenKind::Super,
            TokenKind::Public,
            TokenKind::Private,
            TokenKind::Protected,
            TokenKind::Static,
            TokenKind::Null,
            TokenKind::Try,
            TokenKind::Catch,
            TokenKind::Finally,
            TokenKind::Throw,
            TokenKind::Rescue,
            TokenKind::Not,
            TokenKind::Match,
            TokenKind::Case,
            TokenKind::When,
            TokenKind::Do,
            TokenKind::End,
            TokenKind::Then,
            TokenKind::Unless,
            TokenKind::Import,
            TokenKind::Export,
            TokenKind::From,
            TokenKind::As,
            TokenKind::Int,
            TokenKind::Float,
            TokenKind::Decimal,
            TokenKind::Bool,
            TokenKind::String,
            TokenKind::Void,
        ] {
            let displayed = kind.to_string();
            assert_eq!(
                TokenKind::keyword(&displayed),
                Some(kind.clone()),
                "Display→keyword round trip broke for {kind:?} (displayed as {displayed:?})"
            );
        }
    }

    #[test]
    fn and_or_have_dual_surface_forms() {
        // `and`/`or` (word form) and `&&`/`||` (operator form) both
        // produce the same logical token, but Display only emits the
        // operator form. Pin both directions explicitly.
        assert_eq!(TokenKind::And.to_string(), "&&");
        assert_eq!(TokenKind::Or.to_string(), "||");
        assert_eq!(TokenKind::keyword("and"), Some(TokenKind::And));
        assert_eq!(TokenKind::keyword("or"), Some(TokenKind::Or));
        // Symbol form is not a keyword — it's lexed as an operator.
        assert_eq!(TokenKind::keyword("&&"), None);
        assert_eq!(TokenKind::keyword("||"), None);
    }

    #[test]
    fn display_operators() {
        assert_eq!(TokenKind::Plus.to_string(), "+");
        assert_eq!(TokenKind::EqualEqual.to_string(), "==");
        assert_eq!(TokenKind::BangEqual.to_string(), "!=");
        assert_eq!(TokenKind::Pipeline.to_string(), "|>");
        assert_eq!(TokenKind::Pipe.to_string(), "|");
        assert_eq!(TokenKind::PlusPlus.to_string(), "++");
        assert_eq!(TokenKind::MinusMinus.to_string(), "--");
        assert_eq!(TokenKind::NullishCoalescing.to_string(), "??");
        assert_eq!(TokenKind::NullishEqual.to_string(), "??=");
        assert_eq!(TokenKind::SafeNavigation.to_string(), "&.");
        assert_eq!(TokenKind::DoubleColon.to_string(), "::");
    }

    #[test]
    fn display_delimiters_braces_are_doubled_in_format_string() {
        // Specifically pin that `{{` produces a single `{`.
        assert_eq!(TokenKind::LeftBrace.to_string(), "{");
        assert_eq!(TokenKind::RightBrace.to_string(), "}");
    }

    #[test]
    fn display_delimiters_misc() {
        assert_eq!(TokenKind::LeftParen.to_string(), "(");
        assert_eq!(TokenKind::Arrow.to_string(), "->");
        assert_eq!(TokenKind::FatArrow.to_string(), "=>");
        assert_eq!(TokenKind::Spread.to_string(), "...");
        assert_eq!(TokenKind::Range.to_string(), "..");
        assert_eq!(TokenKind::Eof.to_string(), "EOF");
    }

    #[test]
    fn display_interpolated_string_joins_parts() {
        let kind = TokenKind::InterpolatedString(vec!["a=".into(), "x".into(), " b".into()]);
        // Concrete format is implementation detail — pin the salient bits.
        let s = kind.to_string();
        assert!(s.starts_with("interp\""));
        assert!(s.contains("a=...(x...( b"));
    }

    #[test]
    fn display_sdql_block_truncates_long_queries() {
        let q = "FOR x IN coll RETURN x WITH SOME PADDING TO EXCEED THIRTY".to_string();
        let kind = TokenKind::SdqlBlock {
            query: q.clone(),
            interpolations: vec![],
        };
        let s = kind.to_string();
        assert!(s.starts_with("@sdbql{"));
        // Display caps at 30 chars then appends "}..."
        assert!(s.contains(&q[..30]), "got {s}");
    }

    #[test]
    fn display_sdql_block_keeps_short_query_intact() {
        let q = "FOR x IN c".to_string();
        let kind = TokenKind::SdqlBlock {
            query: q.clone(),
            interpolations: vec![],
        };
        let s = kind.to_string();
        assert!(s.contains(&q));
    }

    #[test]
    fn display_backtick_string_wraps_in_backticks() {
        assert_eq!(
            TokenKind::BacktickString("ls -la".into()).to_string(),
            "`ls -la`"
        );
    }

    // ---------- Token / SdqlInterpolation ----------

    #[test]
    fn token_new_stores_kind_and_span() {
        let span = Span::new(0, 3, 1, 1);
        let t = Token::new(TokenKind::Let, span);
        assert_eq!(t.kind, TokenKind::Let);
        assert_eq!(t.span, span);
    }

    #[test]
    fn token_eof_makes_zero_width_span_at_position() {
        let t = Token::eof(42, 5, 8);
        assert_eq!(t.kind, TokenKind::Eof);
        assert_eq!(t.span.start, 42);
        assert_eq!(t.span.end, 42);
        assert_eq!(t.span.line, 5);
        assert_eq!(t.span.column, 8);
    }
}
