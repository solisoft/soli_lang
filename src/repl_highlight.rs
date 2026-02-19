use crate::lexer::token::Token;
use crate::lexer::Scanner;
use colored::Colorize;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SyntaxTheme {
    #[default]
    Default,
    Midnight,
    Monokai,
    Github,
}

#[derive(Clone)]
pub struct SyntaxHighlighter {
    pub theme: SyntaxTheme,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        Self {
            theme: SyntaxTheme::Default,
        }
    }

    pub fn highlight(&self, source: &str) -> String {
        let mut scanner = Scanner::new(source);
        let tokens = scanner.scan_tokens();

        match tokens {
            Ok(tokens) => self.render_highlighted(source, &tokens),
            Err(_) => source.to_string(),
        }
    }

    fn render_highlighted(&self, source: &str, tokens: &[Token]) -> String {
        let mut result = String::new();
        let mut last_end = 0;

        for token in tokens {
            if token.kind == crate::lexer::token::TokenKind::Eof {
                break;
            }

            let token_text = &source[token.span.start..token.span.end];
            let highlighted = self.colorize_token(token, token_text);

            result.push_str(&source[last_end..token.span.start]);
            result.push_str(&highlighted);
            last_end = token.span.end;
        }

        result.push_str(&source[last_end..]);
        result
    }

    fn colorize_token(&self, token: &Token, text: &str) -> String {
        use crate::lexer::token::TokenKind::*;

        match &token.kind {
            IntLiteral(_) | FloatLiteral(_) => text.bright_blue().to_string(),
            StringLiteral(_) | InterpolatedString(_) => text.bright_green().to_string(),
            BoolLiteral(true) | BoolLiteral(false) => text.bright_magenta().to_string(),
            Null => text.cyan().to_string(),

            Let | Const | Fn | Return | If | Else | Elsif | While | For | In | Class | Extends
            | Implements | Interface | New | This | Super | Public | Private | Protected
            | Static | Try | Catch | Finally | Throw | Not | Async | Await | Match | Case
            | When | End | Unless | Import | Export | From | As | Int | Float | Bool | String
            | Void => text.bright_yellow().bold().to_string(),

            Plus | Minus | Star | Slash | Percent | Equal | EqualEqual | BangEqual | Less
            | LessEqual | Greater | GreaterEqual | Bang | And | Or | Pipeline | Pipe
            | NullishCoalescing | SafeNavigation | DoubleColon | Arrow | FatArrow | Spread
            | Range => text.red().to_string(),

            LeftParen | RightParen | LeftBrace | RightBrace | LeftBracket | RightBracket
            | Comma | Dot | Colon | Semicolon | Question => text.white().bold().to_string(),

            Identifier(_) => text.white().to_string(),
            _ => text.to_string(),
        }
    }

    pub fn set_theme(&mut self, theme: SyntaxTheme) {
        self.theme = theme;
    }

    pub fn toggle_theme(&mut self) {
        self.theme = match self.theme {
            SyntaxTheme::Default => SyntaxTheme::Midnight,
            SyntaxTheme::Midnight => SyntaxTheme::Monokai,
            SyntaxTheme::Monokai => SyntaxTheme::Github,
            SyntaxTheme::Github => SyntaxTheme::Default,
        };
    }

    pub fn current_theme_name(&self) -> &str {
        match self.theme {
            SyntaxTheme::Default => "default",
            SyntaxTheme::Midnight => "midnight",
            SyntaxTheme::Monokai => "monokai",
            SyntaxTheme::Github => "github",
        }
    }
}

impl Default for SyntaxHighlighter {
    fn default() -> Self {
        Self::new()
    }
}
