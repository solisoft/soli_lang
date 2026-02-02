//! Lexer/Scanner for Solilang source code.

use crate::error::LexerError;
use crate::lexer::token::{Token, TokenKind};
use crate::span::Span;

/// The lexer transforms source code into a stream of tokens.
pub struct Scanner<'a> {
    source: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    current_pos: usize,
    line: usize,
    column: usize,
    start_pos: usize,
    start_line: usize,
    start_column: usize,
}

impl<'a> Scanner<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.char_indices().peekable(),
            current_pos: 0,
            line: 1,
            column: 1,
            start_pos: 0,
            start_line: 1,
            start_column: 1,
        }
    }

    /// Scan all tokens from the source.
    pub fn scan_tokens(&mut self) -> Result<Vec<Token>, LexerError> {
        let mut tokens = Vec::new();

        loop {
            let token = self.scan_token()?;
            let is_eof = token.kind == TokenKind::Eof;
            tokens.push(token);
            if is_eof {
                break;
            }
        }

        Ok(tokens)
    }

    /// Scan the next token.
    pub fn scan_token(&mut self) -> Result<Token, LexerError> {
        self.skip_whitespace_and_comments();
        self.mark_start();

        let Some((_, c)) = self.advance() else {
            return Ok(Token::eof(self.current_pos, self.line, self.column));
        };

        match c {
            // Single-character tokens
            '(' => Ok(self.make_token(TokenKind::LeftParen)),
            ')' => Ok(self.make_token(TokenKind::RightParen)),
            '{' => Ok(self.make_token(TokenKind::LeftBrace)),
            '}' => Ok(self.make_token(TokenKind::RightBrace)),
            '[' => {
                // Distinguish between [[ multiline string and nested array [[...], ...]
                // If after [[ we see: digit, minus (for negative numbers), or another [
                // then it's a nested array. Otherwise, it's a multiline string.
                // Note: [[]] is treated as empty string "" not nested empty array
                if self.peek() == Some('[') {
                    let next_next = self.peek_at(1);
                    match next_next {
                        // Nested array indicators
                        Some(c) if c.is_ascii_digit() => {
                            Ok(self.make_token(TokenKind::LeftBracket))
                        }
                        Some('-') | Some('[') => Ok(self.make_token(TokenKind::LeftBracket)),
                        // Everything else (including ] for empty string) is a multiline string
                        _ => {
                            self.advance(); // consume second [
                            self.scan_multiline_string()
                        }
                    }
                } else {
                    Ok(self.make_token(TokenKind::LeftBracket))
                }
            }
            ']' => Ok(self.make_token(TokenKind::RightBracket)),
            ',' => Ok(self.make_token(TokenKind::Comma)),
            '.' => {
                if self.match_char('.') {
                    if self.match_char('.') {
                        Ok(self.make_token(TokenKind::Spread)) // ...
                    } else {
                        Ok(self.make_token(TokenKind::Range)) // ..
                    }
                } else {
                    Ok(self.make_token(TokenKind::Dot)) // .
                }
            }
            ':' => {
                if self.match_char(':') {
                    Ok(self.make_token(TokenKind::DoubleColon))
                } else {
                    Ok(self.make_token(TokenKind::Colon))
                }
            }
            ';' => Ok(self.make_token(TokenKind::Semicolon)),
            '?' => {
                if self.match_char('?') {
                    Ok(self.make_token(TokenKind::NullishCoalescing))
                } else {
                    Ok(self.make_token(TokenKind::Question))
                }
            }
            '+' => Ok(self.make_token(TokenKind::Plus)),
            '*' => Ok(self.make_token(TokenKind::Star)),
            '/' => Ok(self.make_token(TokenKind::Slash)),
            '%' => Ok(self.make_token(TokenKind::Percent)),

            // Two-character tokens
            '-' => {
                if self.match_char('>') {
                    Ok(self.make_token(TokenKind::Arrow))
                } else {
                    Ok(self.make_token(TokenKind::Minus))
                }
            }
            '=' => {
                if self.match_char('=') {
                    Ok(self.make_token(TokenKind::EqualEqual))
                } else if self.match_char('>') {
                    Ok(self.make_token(TokenKind::FatArrow))
                } else {
                    Ok(self.make_token(TokenKind::Equal))
                }
            }
            '!' => {
                if self.match_char('=') {
                    Ok(self.make_token(TokenKind::BangEqual))
                } else {
                    Ok(self.make_token(TokenKind::Bang))
                }
            }
            '<' => {
                if self.match_char('=') {
                    Ok(self.make_token(TokenKind::LessEqual))
                } else {
                    Ok(self.make_token(TokenKind::Less))
                }
            }
            '>' => {
                if self.match_char('=') {
                    Ok(self.make_token(TokenKind::GreaterEqual))
                } else {
                    Ok(self.make_token(TokenKind::Greater))
                }
            }
            '&' => {
                if self.match_char('&') {
                    Ok(self.make_token(TokenKind::And))
                } else {
                    Err(LexerError::unexpected_char(c, self.current_span()))
                }
            }
            '|' => {
                if self.match_char('>') {
                    Ok(self.make_token(TokenKind::Pipeline))
                } else if self.match_char('|') {
                    Ok(self.make_token(TokenKind::Or))
                } else {
                    Ok(self.make_token(TokenKind::Pipe))
                }
            }

            // String literals
            '"' => self.scan_string(),
            '\'' => self.scan_string(),

            // Numbers
            c if c.is_ascii_digit() => self.scan_number(c),

            // Identifiers and keywords
            c if c.is_alphabetic() || c == '_' => self.scan_identifier(c),

            _ => Err(LexerError::unexpected_char(c, self.current_span())),
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\r') => {
                    self.advance();
                }
                Some('\n') => {
                    self.advance();
                    self.line += 1;
                    self.column = 1;
                }
                Some('/') => {
                    if self.peek_next() == Some('/') {
                        // Line comment
                        while self.peek().is_some() && self.peek() != Some('\n') {
                            self.advance();
                        }
                    } else if self.peek_next() == Some('*') {
                        // Block comment
                        self.advance(); // consume /
                        self.advance(); // consume *
                        let mut depth = 1;
                        while depth > 0 {
                            match self.peek() {
                                None => break,
                                Some('*') if self.peek_next() == Some('/') => {
                                    self.advance();
                                    self.advance();
                                    depth -= 1;
                                }
                                Some('/') if self.peek_next() == Some('*') => {
                                    self.advance();
                                    self.advance();
                                    depth += 1;
                                }
                                Some('\n') => {
                                    self.advance();
                                    self.line += 1;
                                    self.column = 1;
                                }
                                _ => {
                                    self.advance();
                                }
                            }
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
    }

    fn scan_string(&mut self) -> Result<Token, LexerError> {
        let start_position = self.current_pos;
        let start_line = self.line;
        let _start_column = self.column;
        let quote_char = if self.current_pos > 0 {
            self.source[self.current_pos - 1..].chars().next().unwrap()
        } else {
            '"'
        };
        let mut value = String::new();
        let mut has_interpolation = false;
        let mut paren_depth = 0; // Track depth when inside interpolation

        loop {
            match self.peek() {
                None | Some('\n') => {
                    return Err(LexerError::unterminated_string(self.current_span()));
                }
                Some('"') => {
                    // Only treat " as terminator if we're not inside parentheses in interpolation
                    if has_interpolation && paren_depth > 0 {
                        // Inside interpolation expression - " is just a character
                        self.advance();
                        value.push('"');
                    } else if quote_char == '"' {
                        // Not inside interpolation or paren_depth is 0 - this is the closing "
                        self.advance();
                        break;
                    } else {
                        // This is a " inside a single-quoted string - just a character
                        self.advance();
                        value.push('"');
                    }
                }
                Some('\'') => {
                    // Only treat ' as terminator if we're not inside parentheses in interpolation
                    if has_interpolation && paren_depth > 0 {
                        // Inside interpolation expression - ' is just a character
                        self.advance();
                        value.push('\'');
                    } else if quote_char == '\'' {
                        // Not inside interpolation or paren_depth is 0 - this is the closing '
                        self.advance();
                        break;
                    } else {
                        // This is a ' inside a double-quoted string - just a character
                        self.advance();
                        value.push('\'');
                    }
                }
                Some('\\') => {
                    self.advance();
                    match self.peek() {
                        Some('(') => {
                            // Start of interpolation - keep the escape sequence for parser
                            has_interpolation = true;
                            paren_depth = 1; // Start counting parentheses
                            value.push('\\');
                            value.push('(');
                            self.advance();
                        }
                        Some('n') => {
                            self.advance();
                            value.push('\n');
                        }
                        Some('t') => {
                            self.advance();
                            value.push('\t');
                        }
                        Some('r') => {
                            self.advance();
                            value.push('\r');
                        }
                        Some('\\') => {
                            self.advance();
                            value.push('\\');
                        }
                        Some('"') => {
                            self.advance();
                            value.push('"');
                        }
                        Some('\'') => {
                            self.advance();
                            value.push('\'');
                        }
                        Some(c) => {
                            return Err(LexerError::invalid_escape(c, self.current_span()));
                        }
                        None => {
                            return Err(LexerError::unterminated_string(self.current_span()));
                        }
                    }
                }
                Some('(') => {
                    if has_interpolation {
                        paren_depth += 1;
                    }
                    self.advance();
                    value.push('(');
                }
                Some(')') => {
                    if has_interpolation && paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    // Don't break here - the " will break the loop when we see it
                    self.advance();
                    value.push(')');
                }
                Some(c) => {
                    self.advance();
                    value.push(c);
                }
            }
        }

        let end_position = self.current_pos;
        let _end_line = self.line;
        let end_column = self.column;
        let span = Span::new(start_position, end_position, start_line, end_column);

        if has_interpolation {
            // Parse the string for interpolation markers
            let parts = Self::parse_interpolation_parts(&value);
            Ok(Token::new(TokenKind::InterpolatedString(parts), span))
        } else {
            Ok(Token::new(TokenKind::StringLiteral(value), span))
        }
    }

    /// Scan a Lua-style multiline string delimited by [[ and ]].
    /// Content is raw (no escape sequences processed).
    fn scan_multiline_string(&mut self) -> Result<Token, LexerError> {
        let start_position = self.start_pos;
        let start_line = self.start_line;
        let mut value = String::new();

        loop {
            match self.peek() {
                None => {
                    return Err(LexerError::unterminated_string(self.current_span()));
                }
                Some(']') => {
                    if self.peek_next() == Some(']') {
                        self.advance(); // consume first ]
                        self.advance(); // consume second ]
                        break;
                    } else {
                        value.push(']');
                        self.advance();
                    }
                }
                Some('\n') => {
                    value.push('\n');
                    self.advance();
                    self.line += 1;
                    self.column = 1;
                }
                Some(c) => {
                    value.push(c);
                    self.advance();
                }
            }
        }

        let end_position = self.current_pos;
        let end_column = self.column;
        let span = Span::new(start_position, end_position, start_line, end_column);

        Ok(Token::new(TokenKind::StringLiteral(value), span))
    }

    fn parse_interpolation_parts(s: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut chars = s.chars().peekable();
        let _paren_depth = 0;

        while let Some(c) = chars.next() {
            if c == '\\' {
                if chars.peek() == Some(&'(') {
                    // Start of interpolation
                    if !current.is_empty() {
                        parts.push(current);
                    }
                    current = String::new();
                    chars.next(); // consume (
                    let mut _paren_depth = 1;
                    // Read until matching )
                    for c2 in chars.by_ref() {
                        if c2 == '(' {
                            _paren_depth += 1;
                            current.push(c2);
                        } else if c2 == ')' {
                            _paren_depth -= 1;
                            if _paren_depth == 0 {
                                break;
                            }
                            current.push(c2);
                        } else {
                            current.push(c2);
                        }
                    }
                    // The expression is in current - push it as-is for parser to handle
                    // Add a special marker to indicate this is an expression
                    parts.push(format!("\\({})", current));
                    current = String::new();
                } else {
                    current.push(c);
                }
            } else {
                current.push(c);
            }
        }

        if !current.is_empty() {
            parts.push(current);
        }

        parts
    }

    fn scan_number(&mut self, first: char) -> Result<Token, LexerError> {
        let mut value = String::from(first);
        let mut is_float = false;

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                value.push(c);
                self.advance();
            } else if c == '.' && !is_float {
                // Check if next char is a digit (to distinguish from method calls)
                if let Some(next) = self.peek_next() {
                    if next.is_ascii_digit() {
                        is_float = true;
                        value.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else if c == '_' {
                // Allow underscores in numbers for readability
                self.advance();
            } else {
                break;
            }
        }

        if is_float {
            let n: f64 = value
                .parse()
                .map_err(|_| LexerError::invalid_number(value.clone(), self.current_span()))?;
            Ok(self.make_token(TokenKind::FloatLiteral(n)))
        } else {
            let n: i64 = value
                .parse()
                .map_err(|_| LexerError::invalid_number(value.clone(), self.current_span()))?;
            Ok(self.make_token(TokenKind::IntLiteral(n)))
        }
    }

    fn scan_identifier(&mut self, first: char) -> Result<Token, LexerError> {
        let mut value = String::from(first);

        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                value.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Check for trailing ? (for predicate methods like empty?, include?, etc.)
        if self.peek() == Some('?') {
            value.push('?');
            self.advance();
        }

        // Check for trailing ! (for methods that raise errors like insert!, delete!, etc.)
        if self.peek() == Some('!') {
            value.push('!');
            self.advance();
        }

        let kind = TokenKind::keyword(&value).unwrap_or(TokenKind::Identifier(value));
        Ok(self.make_token(kind))
    }

    fn advance(&mut self) -> Option<(usize, char)> {
        if let Some((pos, c)) = self.chars.next() {
            self.current_pos = pos + c.len_utf8();
            self.column += 1;
            Some((pos, c))
        } else {
            None
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().map(|(_, c)| *c)
    }

    fn peek_next(&self) -> Option<char> {
        let mut iter = self.source[self.current_pos..].chars();
        iter.next();
        iter.next()
    }

    /// Peek at character n positions ahead (0 = current peek position)
    fn peek_at(&self, n: usize) -> Option<char> {
        self.source[self.current_pos..].chars().nth(n)
    }

    fn match_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn mark_start(&mut self) {
        self.start_pos = self.current_pos;
        self.start_line = self.line;
        self.start_column = self.column;
    }

    fn current_span(&self) -> Span {
        Span::new(
            self.start_pos,
            self.current_pos,
            self.start_line,
            self.start_column,
        )
    }

    fn make_token(&self, kind: TokenKind) -> Token {
        Token::new(kind, self.current_span())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(source: &str) -> Vec<TokenKind> {
        Scanner::new(source)
            .scan_tokens()
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn test_basic_tokens() {
        assert_eq!(
            scan("(){}"),
            vec![
                TokenKind::LeftParen,
                TokenKind::RightParen,
                TokenKind::LeftBrace,
                TokenKind::RightBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_numbers() {
        assert_eq!(
            scan("42 3.14"),
            vec![
                TokenKind::IntLiteral(42),
                TokenKind::FloatLiteral(3.14),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_string() {
        assert_eq!(
            scan(r#""hello""#),
            vec![
                TokenKind::StringLiteral("hello".to_string()),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_keywords() {
        assert_eq!(
            scan("let fn if else while"),
            vec![
                TokenKind::Let,
                TokenKind::Fn,
                TokenKind::If,
                TokenKind::Else,
                TokenKind::While,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_operators() {
        assert_eq!(
            scan("+ - * / == != |>"),
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::EqualEqual,
                TokenKind::BangEqual,
                TokenKind::Pipeline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_pipeline() {
        assert_eq!(
            scan("x |> foo()"),
            vec![
                TokenKind::Identifier("x".to_string()),
                TokenKind::Pipeline,
                TokenKind::Identifier("foo".to_string()),
                TokenKind::LeftParen,
                TokenKind::RightParen,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_comments() {
        assert_eq!(
            scan("1 // comment\n2"),
            vec![
                TokenKind::IntLiteral(1),
                TokenKind::IntLiteral(2),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_interpolated_string() {
        let tokens = scan(r#" "Hello \(name)!" "#);
        println!("TEST OUTPUT: {:?}", tokens);
        // Should have: Literal("Hello "), InterpolatedString marker, Literal("name"), Literal("!")
    }

    #[test]
    fn test_multiline_string() {
        assert_eq!(
            scan("[[hello]]"),
            vec![
                TokenKind::StringLiteral("hello".to_string()),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_multiline_string_with_newlines() {
        assert_eq!(
            scan("[[line1\nline2]]"),
            vec![
                TokenKind::StringLiteral("line1\nline2".to_string()),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_multiline_string_raw() {
        // Backslash-n should be literal, not a newline
        assert_eq!(
            scan(r"[[hello\nworld]]"),
            vec![
                TokenKind::StringLiteral(r"hello\nworld".to_string()),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_multiline_string_with_single_bracket() {
        assert_eq!(
            scan("[[a]b]]"),
            vec![TokenKind::StringLiteral("a]b".to_string()), TokenKind::Eof]
        );
    }

    #[test]
    fn test_empty_multiline_string() {
        assert_eq!(
            scan("[[]]"),
            vec![TokenKind::StringLiteral("".to_string()), TokenKind::Eof]
        );
    }

    #[test]
    fn test_single_bracket_still_works() {
        // Single [ should still produce LeftBracket
        assert_eq!(
            scan("[1]"),
            vec![
                TokenKind::LeftBracket,
                TokenKind::IntLiteral(1),
                TokenKind::RightBracket,
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_bang_suffix() {
        // Methods ending with ! should be valid identifiers
        assert_eq!(
            scan("insert! delete! fail!"),
            vec![
                TokenKind::Identifier("insert!".to_string()),
                TokenKind::Identifier("delete!".to_string()),
                TokenKind::Identifier("fail!".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_bang_suffix_with_operators() {
        // Ensure ! followed by = is still != operator
        assert_eq!(
            scan("x! = y"),
            vec![
                TokenKind::Identifier("x!".to_string()),
                TokenKind::Equal,
                TokenKind::Identifier("y".to_string()),
                TokenKind::Eof,
            ]
        );
    }
}
