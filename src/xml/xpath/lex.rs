//! The XPath tokenizer. Positions are 1-based **character** columns, the
//! numbers error messages show.

use super::{XPathError, MAX_EXPR_BYTES};
use crate::xml::escape::{is_name_char, is_name_start_char};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    /// `/`
    Slash,
    /// `//`
    DSlash,
    /// `|`
    Pipe,
    /// `.`
    Dot,
    /// `..`
    DDot,
    /// `@`
    At,
    /// `*` on its own
    Star,
    /// `[`, `]`, `(`, `)`, `,`
    LBracket,
    RBracket,
    LParen,
    RParen,
    Comma,
    /// `=`, `!=`, `<`, `<=`, `>`, `>=`
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `name`, `p:name`
    Name {
        prefix: Option<String>,
        local: String,
    },
    /// `p:*`
    PrefixStar(String),
    /// `*:name`
    StarLocal(String),
    /// `'…'` or `"…"`
    Literal(String),
    /// Digits only (`12`). A decimal (`1.5`) is lexed as `Decimal`.
    Integer(String),
    Decimal,
    /// `$name`
    Variable(String),
    /// `name::` — an explicit axis.
    Axis(String),
    /// `+`, `-`, anything else the subset has no use for.
    Other(char),
    End,
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub tok: Tok,
    pub col: usize,
}

/// Tokenize `src`. Refuses an expression over [`MAX_EXPR_BYTES`] and an
/// unterminated literal; everything else unexpected becomes [`Tok::Other`]
/// so the parser can name it in context.
pub(crate) fn lex(src: &str) -> Result<Vec<Token>, XPathError> {
    if src.len() > MAX_EXPR_BYTES {
        return Err(XPathError::limit(format!(
            "the expression is longer than {MAX_EXPR_BYTES} bytes"
        )));
    }
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let col = i + 1;
        let next = chars.get(i + 1).copied();
        let push = |out: &mut Vec<Token>, tok| out.push(Token { tok, col });
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
            }
            '/' if next == Some('/') => {
                push(&mut out, Tok::DSlash);
                i += 2;
            }
            '/' => {
                push(&mut out, Tok::Slash);
                i += 1;
            }
            '|' => {
                push(&mut out, Tok::Pipe);
                i += 1;
            }
            '.' if next == Some('.') => {
                push(&mut out, Tok::DDot);
                i += 2;
            }
            '.' if next.is_some_and(|n| n.is_ascii_digit()) => {
                i += 1;
                while chars.get(i).is_some_and(|d| d.is_ascii_digit()) {
                    i += 1;
                }
                push(&mut out, Tok::Decimal);
            }
            '.' => {
                push(&mut out, Tok::Dot);
                i += 1;
            }
            '@' => {
                push(&mut out, Tok::At);
                i += 1;
            }
            '*' => {
                if next == Some(':') && chars.get(i + 2).is_some_and(|n| is_name_start_char(*n)) {
                    let (local, end) = ncname(&chars, i + 2);
                    push(&mut out, Tok::StarLocal(local));
                    i = end;
                } else {
                    push(&mut out, Tok::Star);
                    i += 1;
                }
            }
            '[' => {
                push(&mut out, Tok::LBracket);
                i += 1;
            }
            ']' => {
                push(&mut out, Tok::RBracket);
                i += 1;
            }
            '(' => {
                push(&mut out, Tok::LParen);
                i += 1;
            }
            ')' => {
                push(&mut out, Tok::RParen);
                i += 1;
            }
            ',' => {
                push(&mut out, Tok::Comma);
                i += 1;
            }
            '=' => {
                push(&mut out, Tok::Eq);
                i += 1;
            }
            '!' if next == Some('=') => {
                push(&mut out, Tok::Ne);
                i += 2;
            }
            '<' if next == Some('=') => {
                push(&mut out, Tok::Le);
                i += 2;
            }
            '<' => {
                push(&mut out, Tok::Lt);
                i += 1;
            }
            '>' if next == Some('=') => {
                push(&mut out, Tok::Ge);
                i += 2;
            }
            '>' => {
                push(&mut out, Tok::Gt);
                i += 1;
            }
            '\'' | '"' => {
                let quote = c;
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && chars[end] != quote {
                    end += 1;
                }
                if end >= chars.len() {
                    return Err(XPathError::invalid(col, "unterminated string literal"));
                }
                let text: String = chars[start..end].iter().collect();
                push(&mut out, Tok::Literal(text));
                i = end + 1;
            }
            '0'..='9' => {
                let start = i;
                while chars.get(i).is_some_and(|d| d.is_ascii_digit()) {
                    i += 1;
                }
                if chars.get(i) == Some(&'.') && chars.get(i + 1) != Some(&'.') {
                    i += 1;
                    while chars.get(i).is_some_and(|d| d.is_ascii_digit()) {
                        i += 1;
                    }
                    push(&mut out, Tok::Decimal);
                } else {
                    push(&mut out, Tok::Integer(chars[start..i].iter().collect()));
                }
            }
            '$' => {
                let (name, end) = ncname(&chars, i + 1);
                push(&mut out, Tok::Variable(name));
                i = end.max(i + 1);
            }
            c if is_name_start_char(c) => {
                let (first, end) = ncname(&chars, i);
                i = end;
                if chars.get(i) == Some(&':') {
                    match chars.get(i + 1) {
                        Some(':') => {
                            push(&mut out, Tok::Axis(first));
                            i += 2;
                            continue;
                        }
                        Some('*') => {
                            push(&mut out, Tok::PrefixStar(first));
                            i += 2;
                            continue;
                        }
                        Some(n) if is_name_start_char(*n) => {
                            let (local, end) = ncname(&chars, i + 1);
                            push(
                                &mut out,
                                Tok::Name {
                                    prefix: Some(first),
                                    local,
                                },
                            );
                            i = end;
                            continue;
                        }
                        _ => {}
                    }
                }
                push(
                    &mut out,
                    Tok::Name {
                        prefix: None,
                        local: first,
                    },
                );
            }
            other => {
                push(&mut out, Tok::Other(other));
                i += 1;
            }
        }
    }
    out.push(Token {
        tok: Tok::End,
        col: chars.len() + 1,
    });
    Ok(out)
}

/// An NCName starting at `start` (no colon), and the index after it.
fn ncname(chars: &[char], start: usize) -> (String, usize) {
    let mut end = start;
    if chars.get(end).is_some_and(|c| is_name_start_char(*c)) {
        end += 1;
        while chars
            .get(end)
            .is_some_and(|c| *c != ':' && is_name_char(*c))
        {
            end += 1;
        }
    }
    (chars[start..end].iter().collect(), end)
}
